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
    pub columns: Vec<ProjectColumnResponse>,
    pub kinds: Vec<ProjectKindResponse>,
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

#[derive(MyHttpInput)]
pub struct AddProjectColumnInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    // Typed in by hand and immutable from then on: tasks reference it as their status, and there
    // is no rename — only delete.
    #[http_body(name: "id", description: "Column id, e.g. in-progress", trim, to_lowercase)]
    pub id: String,
    #[http_body(name: "name", description: "Column name", trim)]
    pub name: String,
    #[http_body(name: "description", description: "What sits in this column", trim)]
    pub description: String,
    #[http_body(name: "order", description: "Position between Todo and Done")]
    pub order: i32,
}

#[derive(MyHttpInput)]
pub struct UpdateProjectColumnInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "columnId", description: "Column id")]
    pub column_id: String,
    #[http_body(name: "name", description: "Column name", trim)]
    pub name: String,
    #[http_body(name: "description", description: "What sits in this column", trim)]
    pub description: String,
    #[http_body(name: "order", description: "Position between Todo and Done")]
    pub order: i32,
}

// Deleting a column does NOT move its tasks: their stored status is left alone and they read as
// Todo from then on. Re-creating a column with the same id brings them back to it.
#[derive(MyHttpInput)]
pub struct DeleteProjectColumnInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "columnId", description: "Column id")]
    pub column_id: String,
}

#[derive(MyHttpInput)]
pub struct AddProjectKindInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    // Immutable once created, same as a column id.
    #[http_body(name: "id", description: "Kind id, e.g. bug", trim, to_lowercase)]
    pub id: String,
    #[http_body(name: "name", description: "Kind name", trim)]
    pub name: String,
    #[http_body(name: "description", description: "What qualifies as this kind", trim)]
    pub description: String,
    #[http_body(name: "color", description: "One of the palette values, e.g. red", trim, to_lowercase)]
    pub color: String,
}

#[derive(MyHttpInput)]
pub struct UpdateProjectKindInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "kindId", description: "Kind id")]
    pub kind_id: String,
    #[http_body(name: "name", description: "Kind name", trim)]
    pub name: String,
    #[http_body(name: "description", description: "What qualifies as this kind", trim)]
    pub description: String,
    #[http_body(name: "color", description: "One of the palette values, e.g. red", trim, to_lowercase)]
    pub color: String,
}

// A task pointing at a deleted kind reads as having no kind at all.
#[derive(MyHttpInput)]
pub struct DeleteProjectKindInputModel {
    #[http_body(name: "projectId", description: "Project id")]
    pub project_id: String,
    #[http_body(name: "kindId", description: "Kind id")]
    pub kind_id: String,
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
