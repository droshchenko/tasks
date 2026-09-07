use std::sync::Arc;

use mcp_server_middleware::*;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;

use super::DocumentContentView;
use super::documents_tool_calls::project_prefix_of;

// ---------------------------------------------------------------- documents_set_brief

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsSetBriefInput {
    #[property(
        description = "The content hash the brief is filed under — the `content_hash` of the document you just read, 64 hex characters. NOT an id and not a path: a brief describes a TEXT, so filing it by content is what makes every copy of that text briefed at once, and what makes an edited document unbriefed again by itself"
    )]
    pub hash: String,
    #[property(
        description = "WHAT IS IN THE DOCUMENT, written so that the next reader can decide whether to open it WITHOUT opening it. Say what it is (a specification, a runbook, a meeting note, an API contract), what it covers and — where it is not obvious — what it does NOT, the systems, services, tables, endpoints, people and decisions named in it, and the questions somebody could answer from it. Concrete nouns beat adjectives: `settlement-engine retry policy, the four failure classes, and why B-book fills are exempt` is worth ten times `describes retry behaviour`. A few sentences up to 1500 characters — write to the ceiling when the document earns it, because this text is all a future reader gets before choosing"
    )]
    pub brief: String,
    #[property(description = "Who wrote it: an email, or `AI`")]
    pub who: String,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsSetBriefResponse {
    #[property(description = "The hash it was filed under, normalised to lowercase")]
    pub hash: String,
    #[property(description = "The brief as stored, trimmed")]
    pub brief: String,
}

pub struct DocumentsSetBriefHandler {
    app: Arc<AppContext>,
}

impl DocumentsSetBriefHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsSetBriefHandler {
    const FUNC_NAME: &'static str = "documents_set_brief";
    const DESCRIPTION: &'static str = "Record what a document says, so nobody has to read it to find \
out. The brief comes back on every documents_list row and every documents_get, and scanning briefs \
instead of opening documents is how a board of two hundred specifications stays usable.\
\
IT IS FILED BY CONTENT, NOT BY DOCUMENT. The key is the sha256 of the text — `content_hash` on \
anything you read. Three consequences, and all three are the point: the same text stored on two boards \
or living in a connected repository AND synced into a project is briefed once and found by both; an \
edit changes the hash, so the document goes back to being unbriefed and nobody is left reading a brief \
that describes the old text; and a document restored from the trash finds its brief again, because the \
bytes came back with it.\
\
WRITE IT RIGHT AFTER YOU READ SOMETHING. documents_upload and documents_edit hand you the new \
`content_hash` in their response — that is the moment: you have just read or written the text, and it \
costs you nothing to say what is in it. A board where this is habit answers 'which document covers the \
settlement retries' from one documents_list.\
\
WHAT MAKES A BRIEF WORTH HAVING is that it is specific enough to rule a document IN or OUT. Name the \
systems, the endpoints, the tables, the decisions and the people in it. Say what it does not cover when \
somebody would reasonably assume it does. Do not write 'documentation about the API' — write which API, \
which operations, and what a reader would come to it for.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsSetBriefInput, DocumentsSetBriefResponse> for DocumentsSetBriefHandler {
    async fn execute_tool_call(
        &self,
        model: DocumentsSetBriefInput,
    ) -> Result<DocumentsSetBriefResponse, String> {
        let brief =
            crate::scripts::set_brief(&self.app, &model.hash, &model.brief, &model.who).await?;

        Ok(DocumentsSetBriefResponse {
            hash: crate::documents::normalise_content_hash(&model.hash)?,
            brief: brief.text,
        })
    }
}

// ---------------------------------------------------------------- documents_next_without_brief

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsNextWithoutBriefInput {
    #[property(description = "Which board to work through, by prefix, e.g. `RMS`")]
    pub project: String,
    #[property(
        description = "Hashes to step over this round — the ones you have already decided you cannot brief. Without it, a document you skip is the one you are handed again on the next call, and the loop never ends. Add each hash you gave up on and pass the growing list back"
    )]
    pub skip: Option<Vec<String>>,
}

#[derive(ApplyJsonSchema, Debug, Serialize, Deserialize)]
pub struct DocumentsNextWithoutBriefResponse {
    #[property(
        description = "The document to brief, with its text — absent when there is nothing left to read. It is the same shape documents_get returns, cut to the first 64 KB when it is longer: read `truncated`, and when it is true and the opening was not enough, documents_get with `from_line` gives you the rest"
    )]
    pub document: Option<DocumentContentView>,
    #[property(
        description = "The hash to hand to documents_set_brief with what you learned. Empty when there was nothing left"
    )]
    pub content_hash: String,
    #[property(
        description = "How many distinct contents on this board are still unbriefed, INCLUDING this one. It is the reading list left, deduplicated — two copies of one text are one item"
    )]
    pub remaining: i32,
    #[property(
        description = "True when everything readable on the board has a brief. The loop stops here"
    )]
    pub none_left: bool,
}

pub struct DocumentsNextWithoutBriefHandler {
    app: Arc<AppContext>,
}

impl DocumentsNextWithoutBriefHandler {
    pub fn new(app: Arc<AppContext>) -> Self {
        Self { app }
    }
}

impl ToolDefinition for DocumentsNextWithoutBriefHandler {
    const FUNC_NAME: &'static str = "documents_next_without_brief";
    const DESCRIPTION: &'static str = "Hand me the next document nobody has written a brief for, with \
its text. THIS IS A LOOP: call it, read what comes back, call documents_set_brief with the \
`content_hash` and what you learned, call this again. Stop when `none_left` is true.\
\
IT COVERS BOTH KINDS OF DOCUMENT — the project's own and the files of its connected repositories — in \
path order, because a reader works through a board the way it is drawn rather than doing all of one \
kind first.\
\
`remaining` IS THE WORK LEFT, deduplicated by content: two copies of one text are one brief. It does \
not count files (a PDF, an image), which have no text to brief and are never offered here.\
\
IF YOU CANNOT BRIEF WHAT YOU ARE GIVEN — it is generated noise, a lock file, a minified bundle — put \
its hash in `skip` on the next call rather than writing a brief that says nothing. A brief of 'this is \
a generated file' is worth writing once; a brief of 'unclear' is worth nothing to the reader who finds \
it.\
\
WHERE TO START: documents_list reports `without_brief` for the board, and github_refresh reports it \
for a repository it has just re-cloned. Either number above zero is what this tool is for.";
}

#[async_trait::async_trait]
impl McpToolCall<DocumentsNextWithoutBriefInput, DocumentsNextWithoutBriefResponse>
    for DocumentsNextWithoutBriefHandler
{
    async fn execute_tool_call(
        &self,
        model: DocumentsNextWithoutBriefInput,
    ) -> Result<DocumentsNextWithoutBriefResponse, String> {
        let skip = model.skip.unwrap_or_default();

        let next = crate::scripts::next_without_brief(&self.app, &model.project, &skip).await?;

        let document = next.document.map(|row| {
            let view = DocumentContentView::from_dto(
                &row,
                &project_prefix_of(&self.app, &row.project_id),
                // Empty by construction — this is a document nobody has briefed, which is why it is
                // being handed over.
                String::new(),
            );

            // Cut to what a brief actually needs. The shape of a document — its title, its headings, its
            // opening — is in the first pages, and shipping a 900 KB specification whole to produce three
            // sentences spends the context the reading itself needs.
            view.into_slice(None, None, Some(crate::documents::BRIEF_READ_MAX_BYTES as i64))
        });

        let document = match document {
            Some(view) => Some(view?),
            None => None,
        };

        Ok(DocumentsNextWithoutBriefResponse {
            none_left: document.is_none(),
            document,
            content_hash: next.content_hash,
            remaining: next.remaining as i32,
        })
    }
}
