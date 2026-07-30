use rust_extensions::date_time::DateTimeAsMicroseconds;
use service_sdk::my_telemetry::MyTelemetryContext;
use task_manager_shared::documents::normalise_document_path;

use crate::app::AppContext;
use crate::postgres::{DocumentDto, DocumentHistoryDto, DocumentTrashDto};

use super::resolve_project_by_prefix;

/// The longest a document may be, in characters.
///
/// A constant rather than a setting on purpose: a setting would need a deployment to change and a template
/// to carry it, and nobody has an opinion about this number until a write is refused — at which point the
/// message says what the limit is. One MiB of text is a long specification; a document that does not fit is
/// two documents.
pub const MAX_CONTENT_LEN: usize = 1_000_000;

/// What a version of a document records having happened to it.
///
/// Five values rather than a bool, because "the text changed" and "it moved" are the two questions a history
/// is asked and a single flag answers neither. Stored as the string these produce, with no constraint in
/// Postgres — the same leniency every open vocabulary here gets, so an unrecognised value renders as itself
/// instead of failing a read of the history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DocumentEvent {
    /// The first version. There is exactly one of these per document, for ever.
    Created,
    /// The text was rewritten at the same path.
    Updated,
    /// The path changed; the text did not.
    Moved,
    /// It went to the trash. The version records the path and text it had when it went.
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
/// what makes "just upload the file again" the whole of the editing story — a caller holding a file and a
/// path needs to know nothing else.
///
/// Which is also why moving is a *different* call. If an upload could take a new path together with new
/// text, the history could not tell "somebody rewrote it" from "somebody moved it", and those are the two
/// things a history is for. See [`update_document_path`].
///
/// Returns the document as it now stands.
pub async fn upload_document(
    app: &AppContext,
    project_prefix: &str,
    path: &str,
    content: &str,
    who: &str,
) -> Result<DocumentDto, String> {
    let project_id = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?.id.clone()
    };

    let path = normalise_document_path(path)?;
    let who = require_author(who)?;

    if content.chars().count() > MAX_CONTENT_LEN {
        return Err(format!(
            "that document is {} characters — the limit is {MAX_CONTENT_LEN}. A text that does not fit is two documents",
            content.chars().count()
        ));
    }

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
            content: content.to_string(),
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
            content: content.to_string(),
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

    Ok(row)
}

/// Move a document to another path. The text is untouched and the id does not change.
///
/// Its own call rather than a field on the upload, so the history says which of the two happened — see
/// [`upload_document`]. It is also how a document is renamed, and how a "folder" is renamed: there is no
/// folder to rename, so moving every document under a prefix is the operation, one call each.
///
/// Refused when the destination is taken, rather than overwriting: two documents at one path is the one
/// state the working table must never be in, and silently replacing somebody else's document with this one
/// would lose it from the tree while leaving it in the history, which is the worst of both.
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
        content: existing.content,
        version: existing.version + 1,
        created: existing.created,
        updated: DateTimeAsMicroseconds::now(),
        updated_by: who.clone(),
    };

    write_version(app, &row, &who, DocumentEvent::Moved, &ctx).await;

    Ok(row)
}

/// Put a document in the trash.
///
/// Not a deletion: the row moves to a second table, keeping its id, its text and the last path it had. Which
/// is why nothing prunes the references to it on tasks and goals — restoring is one call away, and a
/// reference quietly removed here would not come back with the document.
///
/// Returns the path it had, which is what a caller wants echoed back: it is what the document was called.
pub async fn delete_document(app: &AppContext, id: &str, who: &str) -> Result<String, String> {
    let who = require_author(who)?;

    let ctx = MyTelemetryContext::create_empty();
    let existing = require_live(app, id, &ctx).await?;

    let now = DateTimeAsMicroseconds::now();
    let version = existing.version + 1;

    // The version is written first, as always — see `write_version`. Recorded at the path and text it had
    // when it went, so the history answers "what was in it when it was thrown away".
    app.documents_repo
        .upsert_history(
            &DocumentHistoryDto {
                document_id: existing.id.clone(),
                version,
                project_id: existing.project_id.clone(),
                doc_path: existing.doc_path.clone(),
                content: existing.content.clone(),
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
                content: existing.content.clone(),
                version,
                created: existing.created,
                deleted: now,
                deleted_by: who,
            },
            &ctx,
        )
        .await;

    app.documents_repo.delete_row(&existing.id, &ctx).await;

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
        content: trashed.content.clone(),
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

    Ok(row)
}

/// One live document by id.
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
        .ok_or_else(|| {
            format!("no document at '{path}' on {}", project_prefix.to_uppercase())
        })
}

/// Every live document of one project, by path.
///
/// Sorted by path, which is what makes a folder tree fall out of a flat list: everything under `docs/` is
/// contiguous, so a reader can group it in one pass.
pub async fn list_documents(
    app: &AppContext,
    project_prefix: &str,
) -> Result<Vec<DocumentDto>, String> {
    let project_id = {
        let board = app.board.read();
        resolve_project_by_prefix(&board, project_prefix)?.id.clone()
    };

    Ok(list_documents_of_project(app, &project_id).await)
}

/// The same list, for a caller that already holds the project id — the HTTP side, which is given one.
pub async fn list_documents_of_project(app: &AppContext, project_id: &str) -> Vec<DocumentDto> {
    let ctx = MyTelemetryContext::create_empty();

    let mut rows = app
        .documents_repo
        .get_all_of_project(project_id, &ctx)
        .await;

    rows.sort_by(|left, right| left.doc_path.cmp(&right.doc_path));
    rows
}

/// One project's trash, most recently deleted first.
///
/// Newest first, unlike the index: the trash is not browsed, it is looked at when something needs restoring,
/// and what needs restoring is almost always what just went in.
pub async fn list_trash(
    app: &AppContext,
    project_prefix: &str,
) -> Result<Vec<DocumentTrashDto>, String> {
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

/// Every version of one document, oldest first.
///
/// Works for a trashed document as well as a live one, and that is deliberate: the history is the reason the
/// id is stable, so it must not stop answering the moment somebody deletes the thing.
pub async fn document_history(
    app: &AppContext,
    id: &str,
) -> Result<Vec<DocumentHistoryDto>, String> {
    let ctx = MyTelemetryContext::create_empty();

    let mut rows = app.documents_repo.get_history(id, &ctx).await;

    if rows.is_empty() {
        return Err(format!("no document {id} — nothing has ever been written under that id"));
    }

    rows.sort_by_key(|itm| itm.version);
    Ok(rows)
}

/// One version of one document, content included — how an old text is read back.
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
    /// Removal happens after addition, so an id passed to both ends up removed — the order labels use.
    pub async fn apply(
        &self,
        app: &AppContext,
        project_id: &str,
        ids: &mut Vec<String>,
        owner: &str,
    ) -> Result<(), String> {
        let ctx = MyTelemetryContext::create_empty();

        for id in &self.add {
            let id = id.trim();

            if id.is_empty() {
                return Err("a document reference needs an id".to_string());
            }

            let Some(document) = app.documents_repo.get_by_id(id, &ctx).await else {
                // A trashed document is named as trashed rather than as missing: attaching one is still
                // refused — a reference should point at something a reader can open — but the fix is
                // different, and it is one call away.
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

            if document.project_id != project_id {
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
        return Err("a document write needs an author — pass `who` as an email, or `AI`".to_string());
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
                content: row.content.clone(),
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
}
