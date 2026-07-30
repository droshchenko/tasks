use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::mcp::{DocumentContentView, DocumentVersionView, DocumentView, TrashedDocumentView};

/// The prefix of the project a document belongs to.
///
/// A document row stores the project **id**, but every id a caller sees on this surface is a prefix — so the
/// translation happens once, here. A project that has vanished between the read and this call falls back to
/// the raw id rather than failing: the document is what was asked for, and naming its board oddly is better
/// than refusing to hand it over.
fn project_prefix_of(app: &AppContext, project_id: &str) -> String {
    app.board
        .read()
        .get_project(project_id)
        .map(|itm| itm.prefix.clone())
        .unwrap_or_else(|| project_id.to_string())
}

// -------------------------------------------------------------------------------------------- list

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsListInput {
    #[property(description = "Which project's documents to index, by prefix, e.g. `RMS`")]
    pub project: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsListResponse {
    #[property(
        description = "Every live document of the project, by path. WITHOUT the texts — read one with documents_get. Sorted by path, so everything under one folder is contiguous"
    )]
    pub documents: Vec<DocumentView>,
    #[property(description = "How many there are")]
    pub amount: i32,
}

pub struct DocumentsListHandler {
    app: Arc<AppContext>,
}

impl DocumentsListHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsListHandler {
    const FUNC_NAME: &'static str = "documents_list";
    const DESCRIPTION: &'static str = "The index of a project's documents: what exists, where it \
lives and how big it is. CALL THIS BEFORE UPLOADING — a path that is already taken gets overwritten, \
which is the right behaviour when you meant to edit that document and a lost text when you did not.\
\
It deliberately does NOT return the texts. A document can be a whole specification, and an agent that \
pulled all of them in to find one would have spent the context it needed for the work. Read the paths, \
pick one, then documents_get.\
\
Folders in the paths are not real: there is no such thing as a folder here, they are read off the paths \
of the documents in them. So an empty folder cannot exist, and moving every document out of one is what \
makes it disappear.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsListInput, DocumentsListResponse> for DocumentsListHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsListInput,
    ) -> Result<DocumentsListResponse, String> {
        let rows = crate::scripts::list_documents(&self.app, &model.project).await?;

        let documents: Vec<DocumentView> = rows
            .iter()
            .map(|row| DocumentView::from_dto(row, &project_prefix_of(&self.app, &row.project_id)))
            .collect();

        Ok(DocumentsListResponse {
            amount: documents.len() as i32,
            documents,
        })
    }
}

// --------------------------------------------------------------------------------------------- get

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsGetInput {
    #[property(
        description = "Which document, by id — the stable one from documents_list, or from the `documents` list on a task or a goal. Either this or `project` + `path`"
    )]
    pub id: Option<String>,
    #[property(
        description = "Which project, by prefix. Only needed when reading by `path` instead of by id"
    )]
    pub project: Option<String>,
    #[property(
        description = "Which document, by where it lives — e.g. `docs/design/system.md`. Needs `project` beside it. Use this when a person named a document rather than handing you an id"
    )]
    pub path: Option<String>,
    #[property(
        description = "Read an OLD version instead of the current one, by its version number from documents_history. Requires `id` — a path is where a document lives now, and says nothing about where it lived then. Omit for the current text"
    )]
    pub version: Option<i64>,
}

pub struct DocumentsGetHandler {
    app: Arc<AppContext>,
}

impl DocumentsGetHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsGetHandler {
    const FUNC_NAME: &'static str = "documents_get";
    const DESCRIPTION: &'static str = "Read one document, text included. Name it by `id` — which \
never changes — or by `project` and `path`, which is what to use when a person named the document \
rather than handing you an id.\
\
Pass `version` to read an older text: every version a document has ever had is kept, and \
documents_history lists them. That is what the stable id is FOR — a document that was rewritten and \
moved three times is still one document, and its whole past is reachable through it.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsGetInput, DocumentContentView> for DocumentsGetHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsGetInput,
    ) -> Result<DocumentContentView, String> {
        let id = model.id.as_deref().map(str::trim).filter(|itm| !itm.is_empty());
        let path = model.path.as_deref().map(str::trim).filter(|itm| !itm.is_empty());

        // Asked for an old version but named the document by where it lives. Refused rather than guessed at:
        // a path says where a document is NOW, and version 3 may well have been somewhere else — reading
        // "version 3 of whatever is at this path today" is a question nobody means to ask.
        if model.version.is_some() && id.is_none() {
            return Err(
                "reading an old version needs `id` — a path is where the document lives now, which says nothing about where that version lived. Get the id from documents_list, then pass `version`"
                    .to_string(),
            );
        }

        if let Some(version) = model.version {
            let id = id.expect("checked just above");
            let row = crate::scripts::document_version(&self.app, id, version).await?;

            return Ok(DocumentContentView::from_history(
                &row,
                &project_prefix_of(&self.app, &row.project_id),
            ));
        }

        let row = match (id, path) {
            (Some(id), _) => crate::scripts::read_document(&self.app, id).await?,
            (None, Some(path)) => {
                let project = model.project.as_deref().map(str::trim).filter(|itm| !itm.is_empty()).ok_or_else(|| {
                    "reading by `path` needs `project` beside it — a path is only unique within one board"
                        .to_string()
                })?;

                crate::scripts::read_document_by_path(&self.app, project, path).await?
            }
            (None, None) => {
                return Err(
                    "pass `id`, or `project` and `path` — documents_list reports both for every document"
                        .to_string(),
                );
            }
        };

        Ok(DocumentContentView::from_dto(
            &row,
            &project_prefix_of(&self.app, &row.project_id),
        ))
    }
}

// ------------------------------------------------------------------------------------------ upload

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsUploadInput {
    #[property(description = "Which project to put it on, by prefix, e.g. `RMS`")]
    pub project: String,
    #[property(
        description = "Where it goes — `notes.md`, or `docs/design/system.md` for one inside folders. A leading slash and backslashes are accepted and normalised, so `/docs/a.md` is the same document as `docs/a.md`. IF THIS PATH IS TAKEN, THE DOCUMENT THERE IS OVERWRITTEN with a new version, keeping its id and its whole history — which is how a document is edited. Call documents_list first if you are not certain the path is free"
    )]
    pub path: String,
    #[property(
        description = "The document, as Markdown. Sent whole every time: there is no partial write, and what arrives here becomes the current version in one go. The previous text is not lost — it stays as the previous version"
    )]
    pub content: String,
    #[property(
        description = "Who is writing it: an email, or the literal `AI` when it is you. Recorded on the version, which is what makes the history answer 'who changed this'"
    )]
    pub who: String,
}

pub struct DocumentsUploadHandler {
    app: Arc<AppContext>,
}

impl DocumentsUploadHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsUploadHandler {
    const FUNC_NAME: &'static str = "documents_upload";
    const DESCRIPTION: &'static str = "Put a document into the system: create one at this path, or \
write a new version of the one already there. This is the ONLY way a document is written — there is no \
editing it in the browser, exactly as with everything else on this board.\
\
THE PATH IS THE KEY AND THE ID IS THE IDENTITY. Uploading to a taken path does not make a second \
document: it adds a version to the one that lives there, keeping its id and everything that references \
it. That is what makes 'upload the file again' the whole of the editing story. It is also why moving is \
a different call — documents_update_path — because a history that could not tell a rewrite from a move \
would not answer the question it exists for.\
\
Read documents_list first when you are not sure the path is free. An upload to a path you did not mean \
to touch replaces what a person put there, and while the old text is still in the history, nobody knows \
to go looking for it.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsUploadInput, DocumentView> for DocumentsUploadHandler {
    async fn execute_tool_call(&self, model: DocumentsUploadInput) -> Result<DocumentView, String> {
        let row = crate::scripts::upload_document(
            &self.app,
            &model.project,
            &model.path,
            &model.content,
            &model.who,
        )
        .await?;

        Ok(DocumentView::from_dto(
            &row,
            &project_prefix_of(&self.app, &row.project_id),
        ))
    }
}

// ------------------------------------------------------------------------------------- update_path

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsUpdatePathInput {
    #[property(description = "Which document to move, by id")]
    pub id: String,
    #[property(
        description = "Where it goes. Refused if another document is already there — pick a free path, or upload over that one if replacing it is what you meant"
    )]
    pub path: String,
    #[property(description = "Who is moving it: an email, or `AI`")]
    pub who: String,
}

pub struct DocumentsUpdatePathHandler {
    app: Arc<AppContext>,
}

impl DocumentsUpdatePathHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsUpdatePathHandler {
    const FUNC_NAME: &'static str = "documents_update_path";
    const DESCRIPTION: &'static str = "Move or rename a document. The text is untouched and the id \
does not change, so every task and goal that references it still does.\
\
Its own call rather than a field on the upload, so the history says which of the two happened: 'somebody \
rewrote it' and 'somebody moved it' are the two questions a history is asked, and one field could not \
answer either.\
\
RENAMING A FOLDER IS THIS CALL, ONCE PER DOCUMENT. There is no folder to rename — folders are read off \
the paths — so moving everything under one prefix is the operation, and each move is its own entry in \
its document's history.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsUpdatePathInput, DocumentView> for DocumentsUpdatePathHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsUpdatePathInput,
    ) -> Result<DocumentView, String> {
        let row =
            crate::scripts::update_document_path(&self.app, &model.id, &model.path, &model.who)
                .await?;

        Ok(DocumentView::from_dto(
            &row,
            &project_prefix_of(&self.app, &row.project_id),
        ))
    }
}

// ----------------------------------------------------------------------------------------- history

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsHistoryInput {
    #[property(description = "Which document, by id — including one that is in the trash")]
    pub id: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsHistoryResponse {
    #[property(description = "Which document this is the history of")]
    pub id: String,
    #[property(
        description = "Every version, oldest first, WITHOUT the texts — read one with documents_get and its `version`. Each entry says what was done, where the document was at the time, and by whom"
    )]
    pub versions: Vec<DocumentVersionView>,
}

pub struct DocumentsHistoryHandler {
    app: Arc<AppContext>,
}

impl DocumentsHistoryHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsHistoryHandler {
    const FUNC_NAME: &'static str = "documents_history";
    const DESCRIPTION: &'static str = "Every version a document has ever had: what happened, where it \
was at the time, who did it and when. Nothing is ever pruned, and the run has no gaps.\
\
This is what the stable id is for. A document that was rewritten five times and moved twice is one \
document with one history, and a `moved` entry is what answers 'when did this end up here'.\
\
It answers for a document in the trash as well as a live one — deleting is a version like any other, and \
the whole point of keeping the id is that throwing something away does not end its story. The texts are \
left out; documents_get with a `version` reads the one you want.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsHistoryInput, DocumentsHistoryResponse> for DocumentsHistoryHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsHistoryInput,
    ) -> Result<DocumentsHistoryResponse, String> {
        let rows = crate::scripts::document_history(&self.app, &model.id).await?;

        Ok(DocumentsHistoryResponse {
            id: model.id,
            versions: rows.iter().map(DocumentVersionView::from_dto).collect(),
        })
    }
}

// ------------------------------------------------------------------------------------------ delete

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsDeleteInput {
    #[property(description = "Which document to put in the trash, by id")]
    pub id: String,
    #[property(description = "Who is deleting it: an email, or `AI`")]
    pub who: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsDeleteResponse {
    #[property(
        description = "The id, unchanged — this is what documents_restore takes, so it is worth keeping"
    )]
    pub id: String,
    #[property(description = "The path it had, which is where a restore puts it back by default")]
    pub path: String,
}

pub struct DocumentsDeleteHandler {
    app: Arc<AppContext>,
}

impl DocumentsDeleteHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsDeleteHandler {
    const FUNC_NAME: &'static str = "documents_delete";
    const DESCRIPTION: &'static str = "Put a document in the trash. Its text, its path and its whole \
history are kept, and documents_restore brings it back — this is not destruction, it is taking something \
out of the tree.\
\
The path it was at becomes free immediately, so a document can be replaced by deleting it and uploading \
another there.\
\
REFERENCES TO IT ARE NOT REMOVED. A task or a goal pointing at a deleted document keeps pointing at it, \
on purpose: restoring is one call away, and a reference quietly dropped would not come back with the \
document. A reader who cannot resolve one is told it is in the trash.\
\
The trash is not shown in the browser — documents_trash is how you see what is in it.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsDeleteInput, DocumentsDeleteResponse> for DocumentsDeleteHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsDeleteInput,
    ) -> Result<DocumentsDeleteResponse, String> {
        let path = crate::scripts::delete_document(&self.app, &model.id, &model.who).await?;

        Ok(DocumentsDeleteResponse {
            id: model.id,
            path,
        })
    }
}

// ------------------------------------------------------------------------------------------- trash

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsTrashInput {
    #[property(description = "Which project's trash to look in, by prefix")]
    pub project: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsTrashResponse {
    #[property(
        description = "What is in the trash, most recently deleted first. FLAT — the trash has no folders, only the last path each document had"
    )]
    pub documents: Vec<TrashedDocumentView>,
    #[property(description = "How many there are")]
    pub amount: i32,
}

pub struct DocumentsTrashHandler {
    app: Arc<AppContext>,
}

impl DocumentsTrashHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsTrashHandler {
    const FUNC_NAME: &'static str = "documents_trash";
    const DESCRIPTION: &'static str = "What has been deleted from a project's documents. THE ONLY WAY \
TO SEE THE TRASH — it is deliberately not drawn in the browser: it is not a place to browse, it is a \
list you look at when something needs restoring.\
\
Flat, and newest first: no folders, just the last path each document had, because what needs restoring \
is almost always what just went in.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsTrashInput, DocumentsTrashResponse> for DocumentsTrashHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsTrashInput,
    ) -> Result<DocumentsTrashResponse, String> {
        let rows = crate::scripts::list_trash(&self.app, &model.project).await?;

        let documents: Vec<TrashedDocumentView> =
            rows.iter().map(TrashedDocumentView::from_dto).collect();

        Ok(DocumentsTrashResponse {
            amount: documents.len() as i32,
            documents,
        })
    }
}

// ----------------------------------------------------------------------------------------- restore

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsRestoreInput {
    #[property(description = "Which document to bring back, by id — from documents_trash")]
    pub id: String,
    #[property(
        description = "Where to put it back. OMIT to restore it where it was, which is what undoing a mistake means. If that path has been taken since, the call is refused and names what is there — pass a path then, rather than guessing at one"
    )]
    pub path: Option<String>,
    #[property(description = "Who is restoring it: an email, or `AI`")]
    pub who: String,
}

pub struct DocumentsRestoreHandler {
    app: Arc<AppContext>,
}

impl DocumentsRestoreHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsRestoreHandler {
    const FUNC_NAME: &'static str = "documents_restore";
    const DESCRIPTION: &'static str = "Take a document back out of the trash, with its text, its id \
and its history intact — so every task and goal that referenced it works again.\
\
With no `path` it goes back exactly where it was, which is what undoing a deletion means. If something \
has taken that path since, the call is REFUSED and says what is there: where a restored document lands \
is a decision, and a guessed path is how a document ends up somewhere nobody looks.\
\
Restoring is only possible from here. There is no way to do it in the browser, which does not show the \
trash at all.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsRestoreInput, DocumentView> for DocumentsRestoreHandler {
    async fn execute_tool_call(&self, model: DocumentsRestoreInput) -> Result<DocumentView, String> {
        let row = crate::scripts::restore_document(
            &self.app,
            &model.id,
            model.path.as_deref(),
            &model.who,
        )
        .await?;

        Ok(DocumentView::from_dto(
            &row,
            &project_prefix_of(&self.app, &row.project_id),
        ))
    }
}
