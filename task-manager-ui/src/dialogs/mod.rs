use std::rc::Rc;

use dioxus::prelude::*;
use task_manager_shared::column_templates::ColumnTemplateResponse;
use task_manager_shared::kind_templates::KindTemplateResponse;
use task_manager_shared::goals::GoalResponse;
use task_manager_shared::projects::ProjectResponse;
use task_manager_shared::tasks::FindTaskResponse;

mod checklist;
pub use checklist::*;
mod dialog_template;
pub use dialog_template::*;
mod edit_column_template;
pub use edit_column_template::*;
mod edit_kind_template;
pub use edit_kind_template::*;
mod edit_members;
pub use edit_members::*;
mod edit_project;
pub use edit_project::*;
mod view_goal;
pub use view_goal::*;
mod view_task;
pub use view_task::*;

/// Which dialog is open, if any.
///
/// A context signal of its own rather than a field of `AppState`. Dioxus subscribes per signal, not per
/// field: opening a dialog would re-run every reactive scope that reads `AppState`, and Home's board
/// re-read is one of them — it reads `board_revision` from there. A separate signal keeps opening a
/// dialog from refetching a board.
///
/// **Every dialog follows one pattern**: it is handed a model, it builds a new model as you edit, its Save
/// lights up only when the two differ, and pressing Save hands the new model out through an
/// `EventHandler`. The handler — owned by the page, not the dialog — makes the request and refreshes.
/// No dialog calls an API and none of them touch a page's state, which is why none of them can leave a
/// half-applied edit behind: nothing is sent until Save, and what is sent is the whole thing.
#[derive(Clone)]
pub enum DialogState {
    None,
    /// `project: None` is a new project. The form is the same either way; only the title and which API
    /// call runs differ, so it is one dialog rather than two nearly identical ones.
    EditProject {
        project: Option<Rc<ProjectResponse>>,
        on_saved: EventHandler<()>,
    },
    EditMembers {
        project: Rc<ProjectResponse>,
        on_saved: EventHandler<()>,
    },
    /// Columns and task types are configured here — once per template, under Settings — and never on a
    /// project, which only points at one of each. `template: None` creates.
    EditColumnTemplate {
        template: Option<Rc<ColumnTemplateResponse>>,
        on_saved: EventHandler<()>,
    },
    EditKindTemplate {
        template: Option<Rc<KindTemplateResponse>>,
        on_saved: EventHandler<()>,
    },
    /// One task, looked up by id. Read-only — it has no `on_saved` because there is nothing to save.
    ViewTask {
        found: FindTaskResponse,
    },
    /// One goal, in full: its text and its thread. Handed the whole goal rather than an id, because the
    /// screen that opens it is already holding one — a goal arrives with the board on every push.
    ViewGoal {
        goal: GoalResponse,
    },
}

/// Mounted once, at the top of the signed-in shell, so a dialog overlays whatever screen opened it.
///
/// This is also where a dialog's result is turned into a request: the submit handlers live here so the
/// dialogs stay pure and every caller gets the same behaviour — save, close on success, report on failure.
#[component]
pub fn RenderDialog() -> Element {
    let state = consume_context::<Signal<DialogState>>().read().clone();

    match state {
        DialogState::None => rsx! {},
        DialogState::EditProject { project, on_saved } => rsx! {
            EditProjectDialog { project, on_saved }
        },
        DialogState::EditMembers { project, on_saved } => rsx! {
            EditMembersDialog { project, on_saved }
        },
        DialogState::ViewTask { found } => rsx! {
            ViewTaskDialog { found }
        },
        DialogState::ViewGoal { goal } => rsx! {
            ViewGoalDialog { goal }
        },
        DialogState::EditKindTemplate { template, on_saved } => rsx! {
            EditKindTemplateDialog {
                template,
                on_submit: move |submit: KindTemplateSubmit| {
                    begin_submit();
                    spawn(async move {
                        match crate::api::save_kind_template(
                                &submit.id,
                                &submit.name,
                                &submit.description,
                                submit.kinds,
                            )
                            .await
                        {
                            Ok(()) => {
                                on_saved.call(());
                                close();
                            }
                            Err(err) => submit_failed(err.message),
                        }
                    });
                },
            }
        },
        DialogState::EditColumnTemplate { template, on_saved } => rsx! {
            EditColumnTemplateDialog {
                template,
                on_submit: move |submit: ColumnTemplateSubmit| {
                    begin_submit();
                    spawn(async move {
                        match crate::api::save_column_template(
                                &submit.id,
                                &submit.name,
                                &submit.description,
                                submit.columns,
                            )
                            .await
                        {
                            Ok(()) => {
                                on_saved.call(());
                                close();
                            }
                            Err(err) => submit_failed(err.message),
                        }
                    });
                },
            }
        },
    }
}

/// How a submit went, for the dialogs whose request is made by the router rather than by themselves.
///
/// Without this a failed save is invisible: the dialog has already disabled its Save button and has no way
/// to learn the request came back, so it sits there looking busy for ever. The router writes the outcome
/// here and the dialog reads it — which keeps the dialog free of API calls without making a failure
/// disappear.
#[derive(Clone, Default, PartialEq)]
pub struct DialogFeedback {
    pub saving: bool,
    pub error: String,
}

pub fn feedback() -> DialogFeedback {
    consume_context::<Signal<DialogFeedback>>().read().clone()
}

fn begin_submit() {
    consume_context::<Signal<DialogFeedback>>().set(DialogFeedback {
        saving: true,
        error: String::new(),
    });
}

fn submit_failed(message: String) {
    consume_context::<Signal<DialogFeedback>>().set(DialogFeedback {
        saving: false,
        error: message,
    });
}

/// Open a dialog from anywhere with a `Signal<DialogState>` in context.
pub fn open(state: DialogState) {
    consume_context::<Signal<DialogFeedback>>().set(DialogFeedback::default());
    consume_context::<Signal<DialogState>>().set(state);
}
