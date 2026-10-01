use serde::{Deserialize, Serialize};

// Prose and moments are spelled the same way in both of this product's transfer formats — see the module
// for why that is deliberate. Re-exported rather than imported so every `use super::models::*;` in this
// folder keeps reaching them under the names it already uses.
pub use crate::scripts::transfer_encoding::{
    decode_moment, decode_text, encode_moment, encode_text,
};

/// What the file says it is, written into `project.yaml` and checked on the way back in.
///
/// The version is on the format, not on the product: a zip made by any build carrying `1` is readable by any
/// build that understands `1`. A future shape that an older service could not apply correctly bumps this, and
/// the import refuses it by name rather than half-applying something it misread.
pub const FORMAT: &str = "task-manager-project/1";

/// The four files an export is made of, and the folder beside them.
///
/// One file per kind of thing rather than one big document: `tasks.yaml` opened on its own is a readable list
/// of tasks, and a diff between two exports says which of the four changed. `documents/` holds the documents
/// as themselves — real bytes at their real paths — so the archive is also just a folder of the project's
/// files, openable by anything.
pub const PROJECT_FILE: &str = "project.yaml";
pub const GOALS_FILE: &str = "goals.yaml";
pub const TASKS_FILE: &str = "tasks.yaml";
pub const COMMENTS_FILE: &str = "comments.yaml";
pub const DOCUMENTS_FILE: &str = "documents.yaml";
pub const DOCUMENTS_FOLDER: &str = "documents/";
pub const BRIEFS_FILE: &str = "briefs.yaml";

/// `documents.yaml` — what each file in `documents/` IS, beside the bytes themselves.
///
/// **It exists so that a document keeps its id across instances**, which is what a reference on a task
/// names it by. The bytes in `documents/` are keyed by path and a path is all they can carry; without this
/// file the receiving board mints a fresh id for every document, and every reference on every card arrives
/// pointing at nothing. That is exactly what used to happen.
///
/// Safe to carry because a `SortableId` is `{unix_micros}-{uuid}` — unique across instances rather than
/// within one — so the same document on two boards is the same id on purpose, and two different documents
/// cannot collide by accident.
///
/// The content type rides along for the same reason: it is what somebody DECLARED, and re-deriving it from
/// the extension on the other side would quietly overwrite that with a guess.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct DocumentsFile {
    #[serde(default)]
    pub documents: Vec<DocumentFileModel>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DocumentFileModel {
    pub id: String,
    // Where it lives, and the key into the `documents/` folder beside this file: `docs/design/system.md`
    // is the entry `documents/docs/design/system.md`. An entry with no line here is still imported — it
    // just gets a fresh id, which is what a hand-made archive gets.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// `briefs.yaml` — what each of the exported documents SAYS, by the hash of its content.
///
/// **Carried because the reading is the expensive part.** The texts travel in `documents/`, so the
/// receiving board hashes them to exactly the same keys — but a brief is a few sentences somebody had to
/// read a whole document to write, and leaving them behind would land a board whose every document reads
/// as unread. Keyed by content rather than by document id for the same reason it is stored that way: an
/// import that renumbers ids does not renumber texts.
///
/// Optional in both directions. An archive without it imports a board with no briefs, which is exactly
/// what every archive written before this existed is.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct BriefsFile {
    #[serde(default)]
    pub briefs: Vec<BriefFileModel>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BriefFileModel {
    pub content_hash: String,
    pub brief: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_by: Option<String>,
}

/// `project.yaml` — what was exported, and the settings of the board it came from.
///
/// **The settings are APPLIED on import, not merely recorded.** Pouring a board into another project and
/// leaving it configured differently is how a task ends up in a column that is not there: the statuses in
/// `tasks.yaml` are the source's column ids, so the receiving project has to follow the same templates for
/// them to mean anything. So the import replaces the target's name, description, archive window and the two
/// template ids with what the file says.
///
/// Two of those replacements are conditional, and both for the same reason — the data model forbids the
/// result, not the intent:
///
/// * a **template id** is taken only when a template with that id exists on this instance. Pointing a project
///   at one that is not here would empty its board rather than configure it;
/// * the **prefix** is taken only when it is free. Two projects cannot hold one prefix, and the common case
///   for this feature — copying a board on the instance the original still lives on — is exactly the case
///   where it is not.
///
/// Either one skipped is reported in `notes` rather than being applied halfway.
#[derive(Serialize, Deserialize, Debug)]
pub struct ProjectFile {
    pub format: String,
    pub exported: String,
    pub project: ProjectFileProject,
    pub contents: ProjectFileContents,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProjectFileProject {
    pub prefix: String,
    pub name_base64: String,
    pub description_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_days: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProjectFileContents {
    pub goals: usize,
    pub tasks: usize,
    pub comments: usize,
    pub documents: usize,
}

/// One checklist item. Identical on a task and on a goal, exactly as it is in memory.
///
/// **The item's id is not exported.** It is a `SortableId` minted where the item was created and never shown
/// to anybody; carrying it over would mean two boards holding the same id for two items that are only
/// coincidentally the same. A fresh one is minted on import.
#[derive(Serialize, Deserialize, Debug)]
pub struct SubtaskFileModel {
    pub title_base64: String,
    #[serde(default)]
    pub text_base64: String,
    #[serde(default)]
    pub done: bool,
}

/// One build a task produced.
#[derive(Serialize, Deserialize, Debug)]
pub struct GhActionFileModel {
    pub url: String,
    pub title_base64: String,
    pub moment: String,
}

/// `goals.yaml`.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct GoalsFile {
    #[serde(default)]
    pub goals: Vec<GoalFileModel>,
}

/// One goal.
///
/// `id` is the handle it had on the board it came from — `RMS-G7`. It is the anchor everything else in the
/// file points at: a task names its goal by it, and a comment names what it is on by it. It is NOT what the
/// goal is called after the import — a number is issued out of the target project's own counter, because two
/// boards both minting from 1 would otherwise collide on the first goal.
#[derive(Serialize, Deserialize, Debug)]
pub struct GoalFileModel {
    pub id: String,
    pub name_base64: String,
    #[serde(default)]
    pub description_base64: String,
    pub color: String,
    pub priority: String,
    #[serde(default)]
    pub subtasks: Vec<SubtaskFileModel>,
    // Documents this goal points at, as the references the board stores — `raw/{project}/document/{id}`
    // and `raw/{project}/github/{repository}/{path}`. Carried verbatim: the ids in them are preserved by
    // `documents.yaml`, so the only thing the import rewrites is the project prefix, which is the one part
    // of a reference that is about WHICH board rather than which document.
    #[serde(default)]
    pub documents: Vec<String>,
    pub created: String,
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<String>,
    #[serde(default)]
    pub auto_completed: bool,
}

/// `tasks.yaml`.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TasksFile {
    #[serde(default)]
    pub tasks: Vec<TaskFileModel>,
}

/// One task.
///
/// Every reference here is a HANDLE as it was on the source board — `goal: RMS-G7`, `depends_on: [RMS-4]` —
/// rather than a bare number. That is what makes the file readable, and it is also what makes a reference
/// checkable: a handle whose prefix is not the export's own names something outside the file, which the
/// import reports rather than quietly reinterpreting as a local number.
#[derive(Serialize, Deserialize, Debug)]
pub struct TaskFileModel {
    pub id: String,
    pub text_base64: String,
    pub status: String,
    pub priority: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub subtasks: Vec<SubtaskFileModel>,
    #[serde(default)]
    pub documents: Vec<String>,
    #[serde(default)]
    pub gh_actions: Vec<GhActionFileModel>,
    pub created: String,
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<String>,
    #[serde(default)]
    pub decisions: Vec<task_manager_shared::decisions::TaskDecision>,
    #[serde(default)]
    pub analysis_documents: Vec<String>,
    #[serde(default)]
    pub ai_reviews: Vec<task_manager_shared::ai_reviews::AiReview>,
}

/// `comments.yaml`.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct CommentsFile {
    #[serde(default)]
    pub comments: Vec<CommentFileModel>,
}

/// One comment, on a task or on a goal.
///
/// **Its own file rather than a list inside each task**, which is what makes the export readable as a
/// conversation: `comments.yaml` is the whole thread of the project in one place, in the order it happened,
/// and it is the file somebody actually wants to read. `on` says what it is attached to, as a handle — and
/// since one counter serves tasks and goals, `RMS-42` and `RMS-G7` are unambiguous about which kind it is.
#[derive(Serialize, Deserialize, Debug)]
pub struct CommentFileModel {
    #[serde(rename = "on")]
    pub target: String,
    pub moment: String,
    pub who: String,
    pub text_base64: String,
}
