use std::collections::BTreeSet;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::kind_color::KindColor;

/// One column of a project's board, in memory.
///
/// The two anchors (`todo`, `done`) are not represented here — they exist by definition, and a
/// reader adds them at either end. `order` places this column between them.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub order: i32,
}

/// A named set of columns, shared by any number of projects.
///
/// Columns live here rather than on a project. `columns` is kept sorted by `order` on write, so every
/// read is already in board order.
#[derive(Debug, Clone)]
pub struct ColumnTemplateModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub columns: Vec<ColumnModel>,
    pub created: DateTimeAsMicroseconds,
}

/// One kind of work, in memory. Unlike a column id, a kind is optional on a task.
#[derive(Debug, Clone)]
pub struct KindModel {
    pub id: String,
    pub name: String,
    pub description: String,
    pub color: KindColor,
}

/// A project, in memory.
///
/// `members` is a `BTreeSet` rather than a `Vec`: membership is a set with no meaningful order,
/// asked about far more often than it is edited ("may this person see this board?"), and the sorted
/// iteration makes a listing reproducible.
#[derive(Debug, Clone)]
pub struct ProjectModel {
    pub id: String,
    pub name: String,
    pub description: String,
    // Stored upper-cased, which is also how a parsed handle arrives, so a lookup needs no
    // normalisation at the call site.
    pub prefix: String,
    // Every prefix this project has carried before the current one, upper-cased, oldest first.
    pub prefix_history: Vec<String>,
    // Which column template this project follows, and `None` for a project that follows none — whose
    // board is then just Todo → Done. Not an error: creating a project would otherwise require creating
    // a template first.
    pub column_template_id: Option<String>,
    // The columns resolved from that template, sorted by `order`.
    //
    // A CACHE, not the source of truth: `rebuild_indexes` recomputes it from the templates on every
    // write, so it cannot drift — editing a template updates every project that follows it in the same
    // swap. It exists because `has_column`, `effective_status`, the board read and the MCP tools all ask
    // a project for its columns, and threading the template collection through every one of them would
    // spread this indirection across the whole service instead of confining it to one function.
    pub columns: Vec<ColumnModel>,
    pub kinds: Vec<KindModel>,
    pub members: BTreeSet<String>,
    // High-water mark of the project's own task counter. Only ever moves forward — deleting a task
    // does not lower it, which is what keeps a number from being handed out twice.
    pub last_task_number: i64,
    pub created: DateTimeAsMicroseconds,
}

impl ProjectModel {
    /// Whether `column_id` is a column this project actually has, counting the two anchors.
    pub fn has_column(&self, column_id: &str) -> bool {
        task_manager_shared::projects::is_anchor_column(column_id)
            || self.columns.iter().any(|itm| itm.id == column_id)
    }

    /// Whether `kind_id` is a kind this project actually has.
    pub fn has_kind(&self, kind_id: &str) -> bool {
        self.kinds.iter().any(|itm| itm.id == kind_id)
    }

    /// The status a task should be *read* as.
    ///
    /// A stored status naming a column this project no longer has reads as Todo. The stored value is
    /// left alone, so re-creating the column brings the task back to it — which is why this is a
    /// read-side function and not a migration.
    pub fn effective_status(&self, stored: &str) -> String {
        if self.has_column(stored) {
            stored.to_string()
        } else {
            task_manager_shared::projects::COLUMN_ID_TODO.to_string()
        }
    }

    /// The kind a task should be read as: `None` when it has none, and also when it names a kind
    /// this project no longer has. Same leniency as a status, same reason.
    pub fn effective_kind(&self, stored: Option<&str>) -> Option<String> {
        let stored = stored?;

        if self.has_kind(stored) {
            Some(stored.to_string())
        } else {
            None
        }
    }

    /// Whether this email may see the project. Admins bypass this entirely — they are not members.
    pub fn is_member(&self, email: &str) -> bool {
        self.members.contains(&email.trim().to_lowercase())
    }
}

/// One comment on a task's thread.
#[derive(Debug, Clone)]
pub struct CommentModel {
    pub moment: DateTimeAsMicroseconds,
    pub who: String,
    pub text: String,
}

/// A task, in memory.
///
/// Identified by `(project_id, number)`. The handle a person sees is composed from the project's
/// *current* prefix on every read and is not stored — see the README on why. `depends_on` holds
/// numbers for the same reason.
#[derive(Debug, Clone)]
pub struct TaskModel {
    pub project_id: String,
    pub number: i64,
    pub text: String,
    // The stored status. Read it through `ProjectModel::effective_status` — this field can name a
    // column that no longer exists.
    pub status: String,
    pub kind: Option<String>,
    pub assignee: Option<String>,
    // Lower-cased and de-duplicated on write, sorted so a listing is reproducible.
    pub labels: Vec<String>,
    pub depends_on: Vec<i64>,
    pub comments: Vec<CommentModel>,
    pub created: DateTimeAsMicroseconds,
    // Moved by a change to the task itself. A comment does NOT move it: the thread is a separate
    // record from the work.
    pub updated: DateTimeAsMicroseconds,
    // When the task was moved into Done, and `None` whenever it is not there.
    //
    // Separate from `updated` because that moves on every edit, including edits made after the work
    // landed — so it cannot answer "how long ago was this closed", which is what decides whether the task
    // still appears on the board or has aged out into the archive.
    //
    // Cleared when a task is re-opened, so a re-closed task is dated by its latest close rather than its
    // first.
    pub close_moment: Option<DateTimeAsMicroseconds>,
}

/// One person.
#[derive(Debug, Clone)]
pub struct UserModel {
    // Lower-cased. The identity, and the primary key in Postgres.
    pub email: String,
    pub name: String,
    // Only half of "is this person an admin" — the settings admin list is additive on top.
    pub admin: bool,
    pub disabled: bool,
    pub created: DateTimeAsMicroseconds,
}
