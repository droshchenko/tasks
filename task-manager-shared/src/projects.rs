use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.

/// The leftmost column, present in every project and not configurable.
///
/// It is also the fallback: a task whose status matches no column of its project reads as this one.
pub const COLUMN_ID_TODO: &str = "todo";

/// The rightmost column, present in every project and not configurable. `blocked` is derived
/// against it — a dependency counts as satisfied only when the blocking task sits here.
pub const COLUMN_ID_DONE: &str = "done";

/// True for the two column ids that exist in every project and can be neither added nor removed.
pub fn is_anchor_column(column_id: &str) -> bool {
    column_id == COLUMN_ID_TODO || column_id == COLUMN_ID_DONE
}

// One column of a project's board.
//
// `id` is typed in by a person and never renamed afterwards — only deleted. `order` places the
// column between the two anchors; the anchors themselves are not in this list on the wire, they
// are implied by every reader.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ProjectColumnResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub order: i32,
}

// One kind of work a task can be, configured per project.
//
// `color` is a value of the fixed palette (`task_manager_shared::kind_color::KindColor`), carried
// as a string so a palette entry added later cannot fail a read on an older client — an
// unrecognised colour draws as the default swatch.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ProjectKindResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: String,
    // An icon name without extension, or empty. Unknown to this build reads as no icon.
    #[serde(default)]
    pub icon: String,
}

// A project as the UI and the MCP tools see it.
//
// `columns` excludes the two anchors and is ordered. `labels` is not stored anywhere — it is the
// distinct set of labels currently in use on this project's tasks, so it changes as tasks are
// tagged and stops listing a label when the last task drops it.
//
// `prefix_history` is every prefix this project has ever carried. It exists so `RMS-42` keeps
// resolving after a rename: a reader looks for the project holding `RMS` now, and only then falls
// back to history.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ProjectResponse {
    pub id: String,
    pub name: String,
    pub description: String,
    pub prefix: String,
    #[serde(default)]
    pub prefix_history: Vec<String>,
    // Resolved from the column template this project follows, sorted, anchors excluded. Kept on the
    // wire even though columns are now configured per template: every reader — the board, the MCP
    // tools — wants this project's columns, and none of them should have to join a template list.
    pub columns: Vec<ProjectColumnResponse>,
    // Which template those columns came from. Both `None` when the project follows none, in which case
    // `columns` is empty and the board is Todo -> Done.
    pub column_template_id: Option<String>,
    pub column_template_name: Option<String>,
    // Resolved from the task-type template this project follows, for the same reason `columns` is: every
    // reader wants this project's types and none of them should have to join a template list.
    pub kinds: Vec<ProjectKindResponse>,
    pub kind_template_id: Option<String>,
    pub kind_template_name: Option<String>,
    // Emails of the users who may see this project. An admin sees every project without appearing
    // here.
    #[serde(default)]
    pub members: Vec<String>,
    pub tasks_amount: i32,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ProjectsResponse {
    pub projects: Vec<ProjectResponse>,
}

#[derive(MyHttpInput)]
pub struct CreateProjectInputModel {
    #[http_body(name: "name", description: "Project name", trim)]
    pub name: String,
    #[http_body(name: "description", description: "What the project is about", trim)]
    pub description: String,
    // Refused when another project holds this prefix right now. A prefix a project used to hold
    // and renamed away from is also refused, because its historical ids still resolve through it.
    #[http_body(name: "prefix", description: "Task id prefix, e.g. RMS", trim, to_uppercase)]
    pub prefix: String,
}

#[derive(MyHttpInput)]
pub struct UpdateProjectInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "name", description: "Project name", trim)]
    pub name: String,
    #[http_body(name: "description", description: "What the project is about", trim)]
    pub description: String,
    // Renaming is allowed; the previous prefix is kept in history so old ids keep resolving.
    #[http_body(name: "prefix", description: "Task id prefix, e.g. RMS", trim, to_uppercase)]
    pub prefix: String,
}

// Which template a project follows. `column_template_id` empty means "none" — the board is then just
// Todo -> Done, which is a legitimate state and not an error.
//
// There is deliberately no add-column / update-column / delete-column endpoint on a project any more.
// Columns are configured once per template, in Settings, and a project only points at one.
#[derive(MyHttpInput)]
pub struct SetProjectColumnTemplateInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "columnTemplateId", description: "Column template id, or empty for none", trim, to_lowercase)]
    pub column_template_id: String,
}

// Which task-type template a project follows. Empty means "none" — the project then has no task types,
// which is legitimate: a type is optional on a task.
#[derive(MyHttpInput)]
pub struct SetProjectKindTemplateInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "kindTemplateId", description: "Task-type template id, or empty for none", trim, to_lowercase)]
    pub kind_template_id: String,
}

// Membership is edited from the project's side — this is the whole set, replaced wholesale, so the
// caller never has to diff it.
#[derive(MyHttpInput)]
pub struct SetProjectMembersInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "members", description: "Emails of every user who may see this project")]
    pub members: Vec<String>,
}
