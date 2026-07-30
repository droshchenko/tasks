use rust_extensions::date_time::DateTimeAsMicroseconds;
use service_sdk::my_telemetry::MyTelemetryContext;
use task_manager_shared::documents::{
    DEFAULT_BINARY_CONTENT_TYPE, DEFAULT_TEXT_CONTENT_TYPE, content_type_for_path,
    normalise_document_path,
};

use crate::app::AppContext;
use crate::documents::DocumentIndexEntry;
use crate::postgres::{
    DocumentDto, DocumentHistoryDto, DocumentTrashDto, DocumentTrashIndexDto, DocumentVersionDto,
};

use super::resolve_project_by_prefix;

/// The longest a TEXT document may be, in characters.
///
/// A constant rather than a setting: a setting needs a deployment to change and a template to carry it, and
/// nobody has an opinion about this number until a write is refused — at which point the message says what
/// the limit is. One MiB of prose is a long specification; a text that does not fit is two documents.
pub const MAX_CONTENT_LEN: usize = 1_000_000;

/// The largest BINARY document, in bytes.
///
/// Higher than the text limit and still deliberately modest, for three reasons that compound: **every version
/// is kept whole**, so a 16 MiB file rewritten five times is 80 MiB of history that nothing prunes; the
/// payload crosses the MCP boundary base64-encoded, which inflates it by a third; and the whole thing is held
/// in memory on both sides of a single `INSERT` — there is no streaming here.
///
/// If real files start bouncing off this, the answer is not a bigger number: it is storing blobs outside
/// Postgres and keeping only a key in the row.
pub const MAX_BINARY_LEN: usize = 16 * 1024 * 1024;

/// What a document actually holds. **Exactly one of the two, never both and never neither.**
///
/// An enum rather than two optional fields threaded through every function, because the invariant is the
/// point: a document is text or it is bytes, and everything downstream — how it is drawn, whether it can be
/// searched, whether a line in it could ever be referenced — follows from which. A row cannot hold an enum,
/// so the two nullable columns are the storage and this is the truth that writes them.
#[derive(Debug, Clone, PartialEq)]
pub enum DocumentBody {
    /// Markdown, or any other text. The kind a person reads in the browser and an agent can diff.
    Text(String),
    /// A file — a PDF, an image, anything whose bytes are not text.
    Binary(Vec<u8>),
}

impl DocumentBody {
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }

    /// The size in BYTES either way — UTF-8 bytes for text, real bytes for a file.
    ///
    /// One unit on purpose: a listing shows sizes of both kinds down one column, and two units would make it
    /// a lie half the time.
    pub fn size_bytes(&self) -> i64 {
        match self {
            Self::Text(text) => text.len() as i64,
            Self::Binary(bytes) => bytes.len() as i64,
        }
    }

    /// Refuse a payload that is too big, or one a text column cannot physically hold.
    ///
    /// The `NUL` check is the one that looks pedantic and is not: Postgres rejects `\0` in a `text` column,
    /// and a JSON string may legally contain one. Without this the write fails inside the driver, which
    /// reaches the caller as "it did not save" with no reason attached — and the fix they would reach for is
    /// to try again. Bytes containing NUL are not a text document; they are a binary one.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Text(text) => {
                let length = text.chars().count();

                if length > MAX_CONTENT_LEN {
                    return Err(format!(
                        "that document is {length} characters — the limit is {MAX_CONTENT_LEN}. A text that does not fit is two documents"
                    ));
                }

                if text.contains('\0') {
                    return Err(
                        "that text contains a NUL byte, which Postgres cannot store in a text column — upload it as binary if it is a file"
                            .to_string(),
                    );
                }
            }
            Self::Binary(bytes) => {
                if bytes.is_empty() {
                    return Err("that file is empty".to_string());
                }

                if bytes.len() > MAX_BINARY_LEN {
                    return Err(format!(
                        "that file is {} bytes — the limit is {MAX_BINARY_LEN}. Every version is kept whole, so a large file is a large history",
                        bytes.len()
                    ));
                }
            }
        }

        Ok(())
    }

    /// The two columns this payload writes: `(content, binary_content)`.
    fn into_columns(self) -> (Option<String>, Option<Vec<u8>>) {
        match self {
            Self::Text(text) => (Some(text), None),
            Self::Binary(bytes) => (None, Some(bytes)),
        }
    }

    /// Read a payload back out of the two columns.
    ///
    /// `binary_content` wins if both are somehow set, and a row with neither reads as empty text. Lenient
    /// rather than fatal on purpose: the invariant is enforced where rows are written, and one odd row must
    /// not make a document unreadable — the same leniency an unknown status or colour gets everywhere else.
    pub fn from_columns(content: Option<String>, binary_content: Option<Vec<u8>>) -> Self {
        match binary_content {
            Some(bytes) => Self::Binary(bytes),
            None => Self::Text(content.unwrap_or_default()),
        }
    }

    /// The text, for a text document; `None` for a file.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.as_str()),
            Self::Binary(_) => None,
        }
    }

}

/// A document as a caller hands it over: what it holds, and what it is.
pub struct NewDocumentContent {
    pub body: DocumentBody,
    /// The MIME type. `None` is worked out from the path — see `content_type_for_path` — which is what makes
    /// `docs/spec.pdf` arrive as `application/pdf` without anybody saying so.
    pub content_type: Option<String>,
}

impl NewDocumentContent {
    /// The content type to store: what the caller said, else what the path implies, else a default that
    /// depends on which kind of payload this is.
    fn resolve_content_type(&self, path: &str) -> String {
        if let Some(declared) = self
            .content_type
            .as_deref()
            .map(str::trim)
            .filter(|itm| !itm.is_empty())
        {
            return declared.to_lowercase();
        }

        if let Some(guessed) = content_type_for_path(path) {
            return guessed.to_string();
        }

        // Nothing said and nothing to guess from. The fallbacks differ because the payloads do: an unknown
        // text is still text, and unknown bytes are a file to be downloaded rather than shown.
        if self.body.is_binary() {
            DEFAULT_BINARY_CONTENT_TYPE.to_string()
        } else {
            DEFAULT_TEXT_CONTENT_TYPE.to_string()
        }
    }
}

/// What a version of a document records having happened to it.
///
/// Five values rather than a bool, because "the payload changed" and "it moved" are the two questions a
/// history is asked and a single flag answers neither. Stored as the string these produce, with no constraint
/// in Postgres — the same leniency every open vocabulary here gets, so an unrecognised value renders as
/// itself instead of failing a read of the history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DocumentEvent {
    /// The first version. There is exactly one of these per document, for ever.
    Created,
    /// The payload was rewritten at the same path.
    Updated,
    /// The path changed; the payload did not.
    Moved,
    /// It went to the trash. The version records the path and payload it had when it went.
    Deleted,
    /// It came back out of the trash, at the path this version records.
    Restored,
}

impl DocumentEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Moved => "moved",
            Self::Deleted => "deleted",
            Self::Restored => "restored",
        }
    }
}

/// Upload a document: create one at this path, or overwrite the one already there.
///
/// **The path is the key, and the id is the identity.** An upload never takes an id: it looks the path up
/// within the project, and either writes a new version of what it found or creates something new. That is
/// what makes "just upload the file again" the whole of the editing story.
///
/// Which is also why moving is a *different* call. If an upload could carry a new path together with a new
/// payload, the history could not tell "somebody rewrote it" from "somebody moved it", and those are the two
/// things a history is for. See [`update_document_path`].
///
/// **A rewrite may change the KIND of a document**, text to binary or back, and that is allowed: it is one
/// document at one path whose payload was replaced, and the history records what each version held. Refusing
/// it would only push somebody into deleting and re-uploading, which loses the thread.
pub async fn upload_document(
    app: &AppContext,
    project_prefix: &str,
    path: &str,
    content: NewDocumentContent,
    who: &str,
) -> Result<DocumentDto, String> {
    let project_id = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?.id.clone()
    };

    let path = normalise_document_path(path)?;
    let who = require_author(who)?;

    content.body.validate()?;

    let content_type = content.resolve_content_type(&path);
    let size = content.body.size_bytes();
    let (text, binary) = content.body.into_columns();

    let ctx = MyTelemetryContext::create_empty();
    let now = DateTimeAsMicroseconds::now();

    let existing = app
        .documents_repo
        .get_by_path(&project_id, &path, &ctx)
        .await;

    let row = match existing {
        Some(existing) => DocumentDto {
            // The id of what was already there. This is the line the whole feature turns on: overwriting a
            // path does not mint a new document, it adds a version to the one that lives there.
            id: existing.id,
            project_id,
            doc_path: path,
            content_type: Some(content_type),
            content: text,
            binary_content: binary,
            content_size: Some(size),
            version: existing.version + 1,
            // Kept, not restamped: a document was created once, and that is a different fact from when it
            // was last written.
            created: existing.created,
            updated: now,
            updated_by: who.clone(),
        },
        None => DocumentDto {
            // Minted here and nowhere else. Sortable, so a listing by id is a listing by age.
            id: rust_extensions::SortableId::generate().to_string(),
            project_id,
            doc_path: path,
            content_type: Some(content_type),
            content: text,
            binary_content: binary,
            content_size: Some(size),
            version: 1,
            created: now,
            updated: now,
            updated_by: who.clone(),
        },
    };

    let event = if row.version == 1 {
        DocumentEvent::Created
    } else {
        DocumentEvent::Updated
    };

    write_version(app, &row, &who, event, &ctx).await;
    app.documents_index.upsert(&row);

    Ok(row)
}

/// Move a document to another path. The payload is untouched and the id does not change.
///
/// Its own call rather than a field on the upload, so the history says which of the two happened — see
/// [`upload_document`]. It is also how a document is renamed, and how a "folder" is renamed: there is no
/// folder to rename, so moving every document under a prefix is the operation, one call each.
///
/// **The content type is left exactly as it was**, even when the new path implies another one. A guess from
/// an extension is how a type is chosen when nobody said; it is not a reason to overwrite what was said.
pub async fn update_document_path(
    app: &AppContext,
    id: &str,
    path: &str,
    who: &str,
) -> Result<DocumentDto, String> {
    let path = normalise_document_path(path)?;
    let who = require_author(who)?;

    let ctx = MyTelemetryContext::create_empty();
    let existing = require_live(app, id, &ctx).await?;

    if existing.doc_path == path {
        return Err(format!(
            "'{path}' is already where that document is — nothing to move"
        ));
    }

    if let Some(taken) = app
        .documents_repo
        .get_by_path(&existing.project_id, &path, &ctx)
        .await
    {
        return Err(format!(
            "'{path}' is taken by document {} — pick another path, or upload over that one if replacing it is what you meant",
            taken.id
        ));
    }

    let row = DocumentDto {
        id: existing.id,
        project_id: existing.project_id,
        doc_path: path,
        content_type: existing.content_type,
        content: existing.content,
        binary_content: existing.binary_content,
        content_size: existing.content_size,
        version: existing.version + 1,
        created: existing.created,
        updated: DateTimeAsMicroseconds::now(),
        updated_by: who.clone(),
    };

    write_version(app, &row, &who, DocumentEvent::Moved, &ctx).await;
    app.documents_index.upsert(&row);

    Ok(row)
}

/// Put a document in the trash.
///
/// Not a deletion: the row moves to a second table, keeping its id, its payload and the last path it had.
/// Which is why nothing prunes the references to it on tasks and goals — restoring is one call away, and a
/// reference quietly removed here would not come back with the document.
///
/// Returns the path it had, which is what a caller wants echoed back: it is what the document was called.
pub async fn delete_document(app: &AppContext, id: &str, who: &str) -> Result<String, String> {
    let who = require_author(who)?;

    let ctx = MyTelemetryContext::create_empty();
    let existing = require_live(app, id, &ctx).await?;

    let now = DateTimeAsMicroseconds::now();
    let version = existing.version + 1;

    // The version is written first, as always — see `write_version`. Recorded with the path and payload it
    // had when it went, so the history answers "what was in it when it was thrown away".
    app.documents_repo
        .upsert_history(
            &DocumentHistoryDto {
                document_id: existing.id.clone(),
                version,
                project_id: existing.project_id.clone(),
                doc_path: existing.doc_path.clone(),
                content_type: existing.content_type.clone(),
                content: existing.content.clone(),
                binary_content: existing.binary_content.clone(),
                content_size: existing.content_size,
                who: who.clone(),
                event: DocumentEvent::Deleted.as_str().to_string(),
                moment: now,
            },
            &ctx,
        )
        .await;

    // Then the trash, and only then the removal from the working table. That order cannot lose the
    // document: if the second write fails it is briefly in both places, which reads as still live and is
    // undone by deleting it again. The other order would drop it from the working table with nowhere to
    // restore it from.
    app.documents_repo
        .upsert_trash(
            &DocumentTrashDto {
                id: existing.id.clone(),
                project_id: existing.project_id.clone(),
                doc_path: existing.doc_path.clone(),
                content_type: existing.content_type,
                content: existing.content,
                binary_content: existing.binary_content,
                content_size: existing.content_size,
                version,
                created: existing.created,
                deleted: now,
                deleted_by: who,
            },
            &ctx,
        )
        .await;

    app.documents_repo.delete_row(&existing.id, &ctx).await;

    app.documents_index
        .remove(&existing.project_id, &existing.id);

    Ok(existing.doc_path)
}

/// Take a document back out of the trash.
///
/// `path` is optional, and the default is the point: with nothing passed it goes back where it was, which is
/// what somebody undoing a mistake means. If that path has been taken since, the call is **refused and says
/// so** rather than picking a name — where a restored document lands is a decision, and guessing it is how a
/// document ends up somewhere nobody looks.
pub async fn restore_document(
    app: &AppContext,
    id: &str,
    path: Option<&str>,
    who: &str,
) -> Result<DocumentDto, String> {
    let who = require_author(who)?;

    let ctx = MyTelemetryContext::create_empty();

    let Some(trashed) = app.documents_repo.get_trashed(id, &ctx).await else {
        // Told apart from a document that is simply live, because "it is not in the trash" is a different
        // thing to hear depending on which.
        return Err(match app.documents_repo.get_by_id(id, &ctx).await {
            Some(live) => format!(
                "document {id} is not in the trash — it is live, at '{}'",
                live.doc_path
            ),
            None => format!("no document {id}, in the trash or out of it"),
        });
    };

    let path = match path.map(str::trim).filter(|itm| !itm.is_empty()) {
        Some(path) => normalise_document_path(path)?,
        None => trashed.doc_path.clone(),
    };

    if let Some(taken) = app
        .documents_repo
        .get_by_path(&trashed.project_id, &path, &ctx)
        .await
    {
        return Err(format!(
            "'{path}' is taken by document {} — pass `path` to restore this one somewhere else",
            taken.id
        ));
    }

    let row = DocumentDto {
        id: trashed.id.clone(),
        project_id: trashed.project_id.clone(),
        doc_path: path,
        content_type: trashed.content_type.clone(),
        content: trashed.content.clone(),
        binary_content: trashed.binary_content.clone(),
        content_size: trashed.content_size,
        // One past the version it was deleted at, so its history is a single unbroken run rather than two.
        version: trashed.version + 1,
        created: trashed.created,
        updated: DateTimeAsMicroseconds::now(),
        updated_by: who.clone(),
    };

    write_version(app, &row, &who, DocumentEvent::Restored, &ctx).await;

    // Last, for the same reason the working row is removed last on the way in: while both exist the document
    // reads as live, which is the harmless half of the two failure modes.
    app.documents_repo.delete_trash_row(&trashed.id, &ctx).await;

    app.documents_index.upsert(&row);

    Ok(row)
}

/// One live document by id, payload included.
pub async fn read_document(app: &AppContext, id: &str) -> Result<DocumentDto, String> {
    let ctx = MyTelemetryContext::create_empty();
    require_live(app, id, &ctx).await
}

/// One live document by where it lives.
///
/// Offered beside the by-id read because a path is what a person says: an agent asked to "read the design
/// doc" has a path and no id, and making it list the project first to translate one into the other would be
/// a round trip for nothing.
pub async fn read_document_by_path(
    app: &AppContext,
    project_prefix: &str,
    path: &str,
) -> Result<DocumentDto, String> {
    let project_id = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?.id.clone()
    };

    let path = normalise_document_path(path)?;

    let ctx = MyTelemetryContext::create_empty();

    app.documents_repo
        .get_by_path(&project_id, &path, &ctx)
        .await
        .ok_or_else(|| format!("no document at '{path}' on {}", project_prefix.to_uppercase()))
}

/// Every live document of one project, by path, WITHOUT the payloads.
///
/// **Served from memory.** The index is the one part of a document that is cached — see
/// `crate::documents::DocumentsIndex` — because it is what the tree is drawn from and what a card counts.
/// The payloads are not, which is the whole point of splitting them.
pub fn list_documents(
    app: &AppContext,
    project_prefix: &str,
) -> Result<Vec<DocumentIndexEntry>, String> {
    let project_id = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?.id.clone()
    };

    Ok(app.documents_index.of_project(&project_id))
}

/// One project's trash, most recently deleted first, WITHOUT the payloads.
///
/// From Postgres rather than from memory, unlike the index: the trash is not drawn anywhere and is asked for
/// rarely, so caching it would be a second copy to keep in step for nothing.
pub async fn list_trash(
    app: &AppContext,
    project_prefix: &str,
) -> Result<Vec<DocumentTrashIndexDto>, String> {
    let project_id = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?.id.clone()
    };

    let ctx = MyTelemetryContext::create_empty();

    let mut rows = app
        .documents_repo
        .get_trash_of_project(&project_id, &ctx)
        .await;

    rows.sort_by(|left, right| {
        right
            .deleted
            .unix_microseconds
            .cmp(&left.deleted.unix_microseconds)
    });

    Ok(rows)
}

/// Every version of one document, oldest first, WITHOUT the payloads.
///
/// Works for a trashed document as well as a live one, and that is deliberate: the history is the reason the
/// id is stable, so it must not stop answering the moment somebody deletes the thing.
pub async fn document_history(
    app: &AppContext,
    id: &str,
) -> Result<Vec<DocumentVersionDto>, String> {
    let ctx = MyTelemetryContext::create_empty();

    let mut rows = app.documents_repo.get_history(id, &ctx).await;

    if rows.is_empty() {
        return Err(format!(
            "no document {id} — nothing has ever been written under that id"
        ));
    }

    rows.sort_by_key(|itm| itm.version);
    Ok(rows)
}

/// One version of one document, payload included — how an old text or an old file is read back.
pub async fn document_version(
    app: &AppContext,
    id: &str,
    version: i64,
) -> Result<DocumentHistoryDto, String> {
    let ctx = MyTelemetryContext::create_empty();

    match app.documents_repo.get_version(id, version, &ctx).await {
        Some(row) => Ok(row),
        None => {
            // The versions that DO exist, named. A caller off by one otherwise has to guess.
            let existing = app.documents_repo.get_history(id, &ctx).await;

            Err(match existing.iter().map(|itm| itm.version).max() {
                Some(highest) => format!(
                    "document {id} has no version {version} — it has versions 1 to {highest}"
                ),
                None => format!("no document {id}"),
            })
        }
    }
}

/// Fill in `content_type` and `content_size` on rows written before those columns existed.
///
/// **A one-time backfill, run at startup, and the reason it exists is that the fallbacks cannot answer for
/// everything.** A missing content type can be recovered from the path — see [`content_type_of`] — but a
/// missing SIZE cannot be recovered from anything except the payload, and the index deliberately never reads
/// one. So every such document showed as `0 B`, which is not a size, it is a gap dressed as a fact.
///
/// It writes NO history row and does not touch `version`: nothing about the document changed. What was always
/// true about it is merely now written down, which is what makes running this twice a no-op.
///
/// Reads the full row — payload included — for the affected documents only. That is the one place in this
/// feature that pulls payloads in bulk, it happens once per deploy over a handful of rows, and it stops being
/// work at all as soon as there are none left.
pub async fn backfill_document_columns(app: &AppContext) {
    let ctx = MyTelemetryContext::create_empty();

    let stale: Vec<String> = app
        .documents_repo
        .get_all_indexed(&ctx)
        .await
        .into_iter()
        .filter(|row| row.content_size.is_none() || row.content_type.is_none())
        .map(|row| row.id)
        .collect();

    if stale.is_empty() {
        return;
    }

    println!(
        "documents: filling in content_type and content_size for {} row(s) written before those columns",
        stale.len()
    );

    for id in stale {
        let Some(row) = app.documents_repo.get_by_id(&id, &ctx).await else {
            continue;
        };

        let body = DocumentBody::from_columns(row.content.clone(), row.binary_content.clone());

        let filled = DocumentDto {
            content_type: Some(content_type_of(row.content_type.as_deref(), &row.doc_path)),
            content_size: Some(body.size_bytes()),
            ..row
        };

        app.documents_repo.upsert(&filled, &ctx).await;
    }
}

/// The payload of a live document row.
pub fn body_of(row: &DocumentDto) -> DocumentBody {
    DocumentBody::from_columns(row.content.clone(), row.binary_content.clone())
}

/// The payload of one history row.
pub fn body_of_version(row: &DocumentHistoryDto) -> DocumentBody {
    DocumentBody::from_columns(row.content.clone(), row.binary_content.clone())
}

/// The content type to report for a row whose stored one may be missing.
///
/// **The path is consulted before the default, and that is not a nicety — it is the fix for a real bug.** Rows
/// written before this column existed have `NULL` in it, and a flat `text/markdown` fallback made every one of
/// them Markdown: `prototype_draft.html` came back as `text/markdown`, so the viewer did not frame it and drew
/// a web page as a wall of markup. The path knew the answer all along.
///
/// Order: what was stored, then what the path implies, then Markdown — which is what a document with a
/// meaningless name and no stored type most likely is, since text was all this feature held at the time.
pub fn content_type_of(stored: Option<&str>, path: &str) -> String {
    if let Some(declared) = stored.map(str::trim).filter(|itm| !itm.is_empty()) {
        return declared.to_lowercase();
    }

    content_type_for_path(path)
        .unwrap_or(DEFAULT_TEXT_CONTENT_TYPE)
        .to_string()
}

/// What a caller wants to change about the documents a task or a goal references.
///
/// Add and remove rather than "here is the new list", the same shape labels use and for the same reason: a
/// caller attaching one document must not have to know — or resend — the four that are already there.
///
/// **Unlike a label, an unknown id is refused.** A label is a word and removing one that is not there is
/// harmless; a document id is minted by the system, so one that names nothing means the caller is working
/// from a stale read or has invented it, and attaching a reference nobody can resolve is worse than
/// refusing.
#[derive(Default)]
pub struct DocumentsPatch {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl DocumentsPatch {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }

    /// Apply the patch to one reference list, or refuse it entirely.
    ///
    /// `project_id` is the project the task or goal is on: every id added is checked to be a live document of
    /// **that** project. References do not cross projects — a document is owned by one board, and a
    /// reference to one somebody cannot see would read as a broken link to them and as a working one to
    /// whoever wrote it.
    ///
    /// **Checked against the in-memory index**, not Postgres: the index holds every live document's
    /// reference, which is exactly what this question needs, and it is the reason the index exists. Only the
    /// "is it in the trash" half — the branch that produces a better message — goes to the database.
    ///
    /// Removal happens after addition, so an id passed to both ends up removed — the order labels use.
    pub async fn apply(
        &self,
        app: &AppContext,
        project_id: &str,
        ids: &mut Vec<String>,
        owner: &str,
    ) -> Result<(), String> {
        for id in &self.add {
            let id = id.trim();

            if id.is_empty() {
                return Err("a document reference needs an id".to_string());
            }

            let Some(entry) = app.documents_index.get(id) else {
                // A trashed document is named as trashed rather than as missing: attaching one is still
                // refused — a reference should point at something a reader can open — but the fix is
                // different, and it is one call away.
                let ctx = MyTelemetryContext::create_empty();

                return Err(match app.documents_repo.get_trashed(id, &ctx).await {
                    Some(trashed) => format!(
                        "document {id} is in the trash (it was at '{}') — restore it before attaching it to {owner}",
                        trashed.doc_path
                    ),
                    None => format!(
                        "no document {id} — list the project's documents and use the `id` each entry reports"
                    ),
                });
            };

            if entry.project_id != project_id {
                return Err(format!(
                    "document {id} belongs to another project, and a reference does not cross projects — {owner} can only point at documents of its own board"
                ));
            }

            if !ids.iter().any(|itm| itm == id) {
                ids.push(id.to_string());
            }
        }

        for id in &self.remove {
            let id = id.trim();

            // Unlike an add, removing an id that is not there is NOT refused. The caller's intent — "this
            // reference should not be here" — is already true, and there is nothing for them to fix.
            ids.retain(|itm| itm != id);
        }

        // Sorted, which for a `SortableId` is also oldest first: two callers attaching the same documents in
        // different orders end up with identical lists, so nothing compares unequal for a reason nobody can
        // see.
        ids.sort();
        ids.dedup();

        Ok(())
    }
}

/// The author of a write, or a refusal.
///
/// Required for the same reason a comment's author is: MCP has no session, so the caller passes one, and a
/// history of anonymous versions would not answer the question it exists to answer.
fn require_author(who: &str) -> Result<String, String> {
    let who = who.trim();

    if who.is_empty() {
        return Err(
            "a document write needs an author — pass `who` as an email, or `AI`".to_string(),
        );
    }

    Ok(super::normalise_actor(who))
}

/// One live document, or a refusal that says which of the two reasons it is.
async fn require_live(
    app: &AppContext,
    id: &str,
    ctx: &MyTelemetryContext,
) -> Result<DocumentDto, String> {
    if let Some(row) = app.documents_repo.get_by_id(id, ctx).await {
        return Ok(row);
    }

    Err(match app.documents_repo.get_trashed(id, ctx).await {
        Some(trashed) => format!(
            "document {id} is in the trash (it was at '{}') — restore it first",
            trashed.doc_path
        ),
        None => format!("no document {id}"),
    })
}

/// Write one version, then the document itself.
///
/// **The order is the point, and it is the same on every path through this module.** History is a second
/// table — the only place in this service where one change touches two — so there is no transaction to hide
/// behind, and one of the two writes has to go first. History does, because the failure it leaves behind is
/// a version nobody is serving: one row too many, which the next attempt overwrites, since history is
/// upserted rather than inserted. The other order loses a version outright, and a history with a hole in it
/// is not a history.
///
/// Memory comes after both, in the callers — the same order `scripts/` writes everything else in: Postgres
/// first, then the copy that is served.
async fn write_version(
    app: &AppContext,
    row: &DocumentDto,
    who: &str,
    event: DocumentEvent,
    ctx: &MyTelemetryContext,
) {
    app.documents_repo
        .upsert_history(
            &DocumentHistoryDto {
                document_id: row.id.clone(),
                version: row.version,
                project_id: row.project_id.clone(),
                doc_path: row.doc_path.clone(),
                content_type: row.content_type.clone(),
                content: row.content.clone(),
                binary_content: row.binary_content.clone(),
                content_size: row.content_size,
                who: who.to_string(),
                event: event.as_str().to_string(),
                moment: row.updated,
            },
            ctx,
        )
        .await;

    app.documents_repo.upsert(row, ctx).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five events have to be distinct strings, or a history cannot tell a rewrite from a move — which is
    /// the one distinction it exists for.
    #[test]
    fn every_event_has_its_own_name() {
        let names = [
            DocumentEvent::Created.as_str(),
            DocumentEvent::Updated.as_str(),
            DocumentEvent::Moved.as_str(),
            DocumentEvent::Deleted.as_str(),
            DocumentEvent::Restored.as_str(),
        ];

        let mut sorted = names.to_vec();
        sorted.sort();
        sorted.dedup();

        assert_eq!(sorted.len(), names.len(), "two events share a name");
    }

    #[test]
    fn an_author_is_required_and_normalised() {
        assert!(require_author("").is_err());
        assert!(require_author("   ").is_err());

        assert_eq!(require_author("  Yuri@MXTM.ai ").unwrap(), "yuri@mxtm.ai");
        // `AI` survives whatever case it arrived in, exactly as a comment's author does — the two must not
        // drift, or a filter on one spelling misses the other.
        assert_eq!(require_author("ai").unwrap(), "AI");
    }

    #[test]
    fn an_empty_patch_is_recognised() {
        assert!(DocumentsPatch::default().is_empty());

        assert!(
            !DocumentsPatch {
                add: vec!["some-id".to_string()],
                ..Default::default()
            }
            .is_empty()
        );

        assert!(
            !DocumentsPatch {
                remove: vec!["some-id".to_string()],
                ..Default::default()
            }
            .is_empty()
        );
    }

    /// Exactly one column each way. This is the invariant the two nullable columns cannot express on their
    /// own, so it is worth a test rather than a comment.
    #[test]
    fn a_payload_writes_exactly_one_column() {
        let (text, binary) = DocumentBody::Text("hello".to_string()).into_columns();
        assert_eq!(text.as_deref(), Some("hello"));
        assert!(binary.is_none());

        let (text, binary) = DocumentBody::Binary(vec![1, 2, 3]).into_columns();
        assert!(text.is_none());
        assert_eq!(binary, Some(vec![1, 2, 3]));
    }

    #[test]
    fn a_payload_round_trips_through_its_columns() {
        for body in [
            DocumentBody::Text("# spec".to_string()),
            DocumentBody::Binary(vec![0, 1, 2, 255]),
        ] {
            let (text, binary) = body.clone().into_columns();
            assert_eq!(DocumentBody::from_columns(text, binary), body);
        }
    }

    /// A row with neither column set must read as something rather than break: writes enforce the invariant,
    /// reads are lenient — the rule the whole service follows.
    #[test]
    fn a_row_with_no_payload_reads_as_empty_text() {
        assert_eq!(
            DocumentBody::from_columns(None, None),
            DocumentBody::Text(String::new())
        );
    }

    /// Bytes for both kinds. Two units would make a listing's size column mean two different things.
    #[test]
    fn size_is_bytes_whichever_kind_it_is() {
        // Two bytes in UTF-8, one character — which is exactly why the unit has to be said out loud.
        assert_eq!(DocumentBody::Text("é".to_string()).size_bytes(), 2);
        assert_eq!(DocumentBody::Binary(vec![0; 10]).size_bytes(), 10);
    }

    /// The check that stops a write failing inside the driver with no reason attached.
    #[test]
    fn a_nul_byte_is_refused_in_text() {
        assert!(DocumentBody::Text("a\0b".to_string()).validate().is_err());
        assert!(DocumentBody::Text("ab".to_string()).validate().is_ok());

        // In BINARY it is perfectly normal — every PDF has them.
        assert!(DocumentBody::Binary(vec![0, 1, 0]).validate().is_ok());
    }

    #[test]
    fn an_oversized_payload_is_refused() {
        let long = "a".repeat(MAX_CONTENT_LEN + 1);
        assert!(DocumentBody::Text(long).validate().is_err());

        let big = vec![0u8; MAX_BINARY_LEN + 1];
        assert!(DocumentBody::Binary(big).validate().is_err());
    }

    #[test]
    fn an_empty_file_is_refused_but_an_empty_text_is_not() {
        assert!(DocumentBody::Binary(Vec::new()).validate().is_err());
        // An empty text document is a real thing somebody created and will fill in.
        assert!(DocumentBody::Text(String::new()).validate().is_ok());
    }

    /// What the caller said wins; then the path; then a default that depends on the kind of payload.
    #[test]
    fn the_content_type_is_declared_then_guessed_then_defaulted() {
        let declared = NewDocumentContent {
            body: DocumentBody::Binary(vec![1]),
            content_type: Some("  Application/PDF ".to_string()),
        };
        assert_eq!(
            declared.resolve_content_type("whatever.bin"),
            "application/pdf",
            "a declared type is taken, lower-cased"
        );

        let guessed = NewDocumentContent {
            body: DocumentBody::Binary(vec![1]),
            content_type: None,
        };
        assert_eq!(
            guessed.resolve_content_type("docs/spec.pdf"),
            "application/pdf"
        );

        let binary_fallback = NewDocumentContent {
            body: DocumentBody::Binary(vec![1]),
            content_type: None,
        };
        assert_eq!(
            binary_fallback.resolve_content_type("docs/thing"),
            DEFAULT_BINARY_CONTENT_TYPE
        );

        let text_fallback = NewDocumentContent {
            body: DocumentBody::Text("x".to_string()),
            content_type: None,
        };
        assert_eq!(
            text_fallback.resolve_content_type("docs/thing"),
            DEFAULT_TEXT_CONTENT_TYPE,
            "unknown text is still text"
        );
    }

    /// A row written before these columns existed has to read as what its PATH says, not as Markdown.
    ///
    /// This is the regression test for the bug it fixes: `.html` with no stored type was coming back
    /// `text/markdown`, so the viewer refused to frame it and showed the markup.
    #[test]
    fn a_row_predating_the_column_falls_back_to_its_path() {
        assert_eq!(
            content_type_of(None, "docs/pbi-004_prototype_draft.html"),
            "text/html",
            "an html document must not read as Markdown just because nothing was stored"
        );
        assert_eq!(content_type_of(None, "docs/spec.pdf"), "application/pdf");
        assert_eq!(content_type_of(None, "docs/notes.md"), DEFAULT_TEXT_CONTENT_TYPE);

        // Nothing stored and nothing in the name: text was all this feature held when such a row was written.
        assert_eq!(content_type_of(None, "Makefile"), DEFAULT_TEXT_CONTENT_TYPE);
        assert_eq!(content_type_of(Some("  "), "a.html"), "text/html");

        // A stored type still wins over the path — it was said on purpose.
        assert_eq!(
            content_type_of(Some("application/pdf"), "a.md"),
            "application/pdf"
        );
    }
}
