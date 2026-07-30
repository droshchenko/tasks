use std::sync::Arc;
use std::time::Duration;

use service_sdk::my_postgres::{MyPostgres, UpdateConflictType};

service_sdk::macros::use_my_postgres!();

use crate::app::APP_NAME;
use crate::settings::SettingsReader;

pub const TABLE_NAME: &str = "documents";
pub const PK_NAME: &str = "documents_pk";

pub const HISTORY_TABLE_NAME: &str = "documents_history";
pub const HISTORY_PK_NAME: &str = "documents_history_pk";

pub const TRASH_TABLE_NAME: &str = "documents_trash";
pub const TRASH_PK_NAME: &str = "documents_trash_pk";

// One document, as it currently stands.
//
// **The primary key is `id` alone, and it is a `SortableId`** — unlike a task or a goal, which are
// `(project_id, number)` out of the project's counter. Three things follow from that, and all three are the
// point of the design:
//
// * a document has no handle of the `RMS-7` shape, so it is never renamed by a prefix moving between
//   projects and there is nothing to compose on read;
// * a reference to a document — from a task, from a goal — is just this string, so it resolves without
//   knowing which project it came from;
// * and because the id is minted once and never reused, the whole history of one document is stitched
//   together by it, however many times its path or its text changed.
//
// `doc_path` rather than `path` because `path` is a geometric TYPE name in Postgres and the schema
// generator does not quote identifiers — the same defensiveness `task_text` was named with. It carries the
// document's name AND its position: folders are derived from it and stored nowhere, which is why moving a
// document is one write to one column.
//
// The `(project_id, doc_path)` index is UNIQUE, and it is the backstop under the path-is-the-key rule: an
// upload finds the document by its path, so two rows at one path would make "which document is at
// docs/a.md" a question with two answers. The application checks first; this is what stops two writes
// racing past that check.
#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Debug)]
pub struct DocumentDto {
    #[primary_key(0)]
    pub id: String,
    #[db_index(id: 0, index_name: "documents_path_idx", is_unique: true, order: "ASC")]
    pub project_id: String,
    #[db_index(id: 1, index_name: "documents_path_idx", is_unique: true, order: "ASC")]
    pub doc_path: String,
    pub content: String,
    // Which version this row is. Starts at 1 and moves on every write of any kind — a rewrite, a move, a
    // delete, a restore — so it doubles as the count of history rows this document has.
    pub version: i64,
    #[sql_type("timestamp")]
    pub created: DateTimeAsMicroseconds,
    #[sql_type("timestamp")]
    pub updated: DateTimeAsMicroseconds,
    // Who wrote this version — an email or the literal `AI`, unvalidated for the same reason a comment's
    // author is: MCP has no session to derive one from, and an author whose user row was later removed
    // still has to render.
    pub updated_by: String,
}

// One version of one document, for ever.
//
// **A separate table, and the one place in this service where two tables are written for one change.**
// Everywhere else state rides on its own row precisely to avoid that — a comment lives inside the task's
// jsonb so appending one is a single atomic upsert. History cannot: it grows without bound and would turn
// every document read into a read of every version it ever had.
//
// So the write order is fixed and load-bearing: **history first, then the working table.** If the second
// write fails, the history says a version exists that nothing is serving — one row too many, which the next
// attempt overwrites, because history is upserted rather than inserted. The other order would lose a
// version outright on the same failure, and a history with a hole in it is not a history.
//
// `content` is stored whole per version rather than as a diff. Documents are read one at a time out of
// Postgres, so nothing here has to walk a chain to answer a question — and a chain of diffs is a structure
// whose failure mode is "the oldest version is unreadable", which is the one thing this table exists to
// prevent.
#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Debug)]
pub struct DocumentHistoryDto {
    #[primary_key(0)]
    pub document_id: String,
    #[primary_key(1)]
    pub version: i64,
    pub project_id: String,
    // The path the document had at THIS version — which is why a move is a version of its own. Without it,
    // the history could say what a document said but never when it moved or who moved it.
    pub doc_path: String,
    pub content: String,
    pub who: String,
    // What happened: see `crate::scripts::DocumentEvent`. A plain string with no constraint, the same
    // leniency every open vocabulary in this service gets — an unrecognised value renders as itself rather
    // than failing a read of the history.
    pub event: String,
    #[sql_type("timestamp")]
    pub moment: DateTimeAsMicroseconds,
}

// A deleted document.
//
// **A second table rather than a flag on the first.** With a flag, every read of the working set — the
// index, the path lookup, the uniqueness check — would have to remember to exclude the deleted, and the one
// that forgot would be a bug nobody notices until a path that looks free refuses to take a document. Moving
// the row out means the working table contains exactly the live documents and no read has a filter to get
// wrong.
//
// The trash is FLAT: it keeps `doc_path` as the last path the document had, and nothing draws a folder tree
// out of it. It is not a place you browse — it is a list you ask for when something needs restoring.
#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Debug)]
pub struct DocumentTrashDto {
    #[primary_key(0)]
    pub id: String,
    pub project_id: String,
    // The path it had when it was deleted. Where a restore puts it back, unless the caller names another
    // one — and where the refusal comes from when something else has taken that path since.
    pub doc_path: String,
    pub content: String,
    // Carried across so the version sequence continues rather than restarting: a restored document's next
    // version is one past the version it was deleted at, and its history stays one unbroken run.
    pub version: i64,
    #[sql_type("timestamp")]
    pub created: DateTimeAsMicroseconds,
    #[sql_type("timestamp")]
    pub deleted: DateTimeAsMicroseconds,
    pub deleted_by: String,
}

// Everything in one project, for the index and the trash list alike.
#[derive(WhereDbModel, Debug)]
pub struct ByProjectWhereModel<'s> {
    pub project_id: &'s str,
}

// One document by id. The whole reason the primary key is the id alone.
#[derive(WhereDbModel, Debug)]
pub struct ByIdWhereModel<'s> {
    pub id: &'s str,
}

// One document by where it lives. What an upload asks before it decides whether it is creating or
// overwriting.
#[derive(WhereDbModel, Debug)]
pub struct ByPathWhereModel<'s> {
    pub project_id: &'s str,
    pub doc_path: &'s str,
}

// Every version of one document.
#[derive(WhereDbModel, Debug)]
pub struct ByDocumentWhereModel<'s> {
    pub document_id: &'s str,
}

// One version of one document.
#[derive(WhereDbModel, Debug)]
pub struct ByVersionWhereModel<'s> {
    pub document_id: &'s str,
    pub version: i64,
}

/// The three tables documents live in, behind one type.
///
/// One repo rather than three because they are one thing: no caller ever wants the history of a document
/// without the document, or the trash without knowing what is live. Keeping them together is also what
/// keeps the write order — history first — in one place instead of at every call site.
///
/// **Unlike every other repo here, this one is on the read path.** Projects, tasks and goals are loaded once
/// at startup and served from `Board` afterwards; documents are not held in memory at all and every read
/// comes here. That is deliberate: a document is a text somebody opens occasionally, and the board is
/// pushed whole down a WebSocket on every change — putting documents in it would ship every text to every
/// screen on every write.
pub struct DocumentsRepo {
    postgres: MyPostgres,
}

impl DocumentsRepo {
    pub async fn new(settings_reader: Arc<SettingsReader>) -> Self {
        let postgres = MyPostgres::from_settings(APP_NAME, settings_reader)
            .with_table_schema_verification::<DocumentDto>(TABLE_NAME, Some(PK_NAME.into()))
            .with_table_schema_verification::<DocumentHistoryDto>(
                HISTORY_TABLE_NAME,
                Some(HISTORY_PK_NAME.into()),
            )
            .with_table_schema_verification::<DocumentTrashDto>(
                TRASH_TABLE_NAME,
                Some(TRASH_PK_NAME.into()),
            )
            .build()
            .await;

        Self { postgres }
    }

    /// Every live document of one project, content included.
    ///
    /// The content comes along because `SELECT` here is by whole row and there is nothing to be gained by a
    /// second shape — the caller that wants an index throws the texts away, and a project's documents are a
    /// hand-written set rather than a data volume. If that ever stops being true, this is the one function to
    /// split.
    pub async fn get_all_of_project(
        &self,
        project_id: &str,
        ctx: &MyTelemetryContext,
    ) -> Vec<DocumentDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_rows(TABLE_NAME, Some(&ByProjectWhereModel { project_id }), Some(ctx))
            .await
            .expect("documents: query_rows get_all_of_project failed")
    }

    /// One live document by id, or `None` — which means it is in the trash or was never there. The caller
    /// tells those apart by asking the trash; see `get_trashed`.
    pub async fn get_by_id(&self, id: &str, ctx: &MyTelemetryContext) -> Option<DocumentDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_single_row(TABLE_NAME, Some(&ByIdWhereModel { id }), Some(ctx))
            .await
            .expect("documents: query_single_row get_by_id failed")
    }

    /// The live document at a path, if a path is taken. What decides whether an upload creates or overwrites.
    pub async fn get_by_path(
        &self,
        project_id: &str,
        doc_path: &str,
        ctx: &MyTelemetryContext,
    ) -> Option<DocumentDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_single_row(
                TABLE_NAME,
                Some(&ByPathWhereModel {
                    project_id,
                    doc_path,
                }),
                Some(ctx),
            )
            .await
            .expect("documents: query_single_row get_by_path failed")
    }

    /// Write the current state of a document.
    ///
    /// Always AFTER the matching history row — see the note on [`DocumentHistoryDto`].
    pub async fn upsert(&self, row: &DocumentDto, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .insert_or_update_db_entity(
                TABLE_NAME,
                UpdateConflictType::OnPrimaryKeyConstraint(PK_NAME.into()),
                row,
                Some(ctx),
            )
            .await
            .expect("documents: insert_or_update_db_entity failed");
    }

    /// Take a document out of the working table. Only ever paired with a write to the trash — a document is
    /// never removed from both.
    pub async fn delete_row(&self, id: &str, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .delete(TABLE_NAME, &ByIdWhereModel { id }, Some(ctx))
            .await
            .expect("documents: delete failed");
    }

    /// Append a version.
    ///
    /// Upserted rather than inserted, and that is not laziness: if a previous attempt wrote the history row
    /// and then failed on the working table, the same version number comes round again — an insert would
    /// refuse it and the document would be stuck for good.
    pub async fn upsert_history(&self, row: &DocumentHistoryDto, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .insert_or_update_db_entity(
                HISTORY_TABLE_NAME,
                UpdateConflictType::OnPrimaryKeyConstraint(HISTORY_PK_NAME.into()),
                row,
                Some(ctx),
            )
            .await
            .expect("documents: insert_or_update history failed");
    }

    /// Every version of one document, in whatever order Postgres returns them — the caller sorts.
    pub async fn get_history(
        &self,
        document_id: &str,
        ctx: &MyTelemetryContext,
    ) -> Vec<DocumentHistoryDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_rows(
                HISTORY_TABLE_NAME,
                Some(&ByDocumentWhereModel { document_id }),
                Some(ctx),
            )
            .await
            .expect("documents: query_rows get_history failed")
    }

    /// One version of one document, content included.
    pub async fn get_version(
        &self,
        document_id: &str,
        version: i64,
        ctx: &MyTelemetryContext,
    ) -> Option<DocumentHistoryDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_single_row(
                HISTORY_TABLE_NAME,
                Some(&ByVersionWhereModel {
                    document_id,
                    version,
                }),
                Some(ctx),
            )
            .await
            .expect("documents: query_single_row get_version failed")
    }

    /// Put a document in the trash.
    pub async fn upsert_trash(&self, row: &DocumentTrashDto, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .insert_or_update_db_entity(
                TRASH_TABLE_NAME,
                UpdateConflictType::OnPrimaryKeyConstraint(TRASH_PK_NAME.into()),
                row,
                Some(ctx),
            )
            .await
            .expect("documents: insert_or_update trash failed");
    }

    /// One trashed document by id. How a reference to a deleted document is told from a reference to nothing.
    pub async fn get_trashed(&self, id: &str, ctx: &MyTelemetryContext) -> Option<DocumentTrashDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_single_row(TRASH_TABLE_NAME, Some(&ByIdWhereModel { id }), Some(ctx))
            .await
            .expect("documents: query_single_row get_trashed failed")
    }

    /// Everything in one project's trash.
    pub async fn get_trash_of_project(
        &self,
        project_id: &str,
        ctx: &MyTelemetryContext,
    ) -> Vec<DocumentTrashDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_rows(
                TRASH_TABLE_NAME,
                Some(&ByProjectWhereModel { project_id }),
                Some(ctx),
            )
            .await
            .expect("documents: query_rows get_trash_of_project failed")
    }

    /// Take a document out of the trash — only ever paired with a write back to the working table.
    pub async fn delete_trash_row(&self, id: &str, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .delete(TRASH_TABLE_NAME, &ByIdWhereModel { id }, Some(ctx))
            .await
            .expect("documents: delete trash failed");
    }
}
