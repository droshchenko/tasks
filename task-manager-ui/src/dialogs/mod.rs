use std::rc::Rc;

use dioxus::prelude::*;
use task_manager_shared::projects::ProjectResponse;

mod dialog_template;
pub use dialog_template::*;
mod edit_columns;
pub use edit_columns::*;
mod edit_kinds;
pub use edit_kinds::*;
mod edit_members;
pub use edit_members::*;
mod edit_project;
pub use edit_project::*;

/// Which dialog is open, if any.
///
/// A context signal of its own rather than a field of `AppState`. Dioxus subscribes per signal, not per
/// field: opening a dialog would re-run every reactive scope that reads `AppState`, and Home's board
/// re-read is one of them — it reads `board_revision` from there. A separate signal keeps opening a
/// dialog from refetching a board.
///
/// `on_saved` travels with the state so the page that opened the dialog decides what a save means. Every
/// caller so far uses it to reset its `DataState`, which re-reads from the server rather than patching a
/// local copy — a local patch can disagree with what was actually stored.
#[derive(Clone)]
pub enum DialogState {
    None,
    /// `project: None` is a new project. The form is the same either way; only the title and which API
    /// call runs differ, so it is one dialog rather than two nearly identical ones.
    EditProject {
        project: Option<Rc<ProjectResponse>>,
        on_saved: EventHandler<()>,
    },
    EditColumns {
        project: Rc<ProjectResponse>,
        on_saved: EventHandler<()>,
    },
    EditKinds {
        project: Rc<ProjectResponse>,
        on_saved: EventHandler<()>,
    },
    EditMembers {
        project: Rc<ProjectResponse>,
        on_saved: EventHandler<()>,
    },
}

/// Mounted once, at the top of the signed-in shell, so a dialog overlays whatever screen opened it.
#[component]
pub fn RenderDialog() -> Element {
    let state = consume_context::<Signal<DialogState>>().read().clone();

    match state {
        DialogState::None => rsx! {},
        DialogState::EditProject { project, on_saved } => rsx! {
            EditProjectDialog { project, on_saved }
        },
        DialogState::EditColumns { project, on_saved } => rsx! {
            EditColumnsDialog { project, on_saved }
        },
        DialogState::EditKinds { project, on_saved } => rsx! {
            EditKindsDialog { project, on_saved }
        },
        DialogState::EditMembers { project, on_saved } => rsx! {
            EditMembersDialog { project, on_saved }
        },
    }
}

/// Open a dialog from anywhere with a `Signal<DialogState>` in context.
pub fn open(state: DialogState) {
    consume_context::<Signal<DialogState>>().set(state);
}
