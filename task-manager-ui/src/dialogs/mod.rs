use std::rc::Rc;

use dioxus::prelude::*;
use task_manager_shared::column_templates::ColumnTemplateResponse;
use task_manager_shared::documents::UploadArchiveResponse;
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
mod github_connections;
pub use github_connections::*;
mod sync_github;
pub use sync_github::*;
mod land_task;
pub use land_task::*;
mod md;
pub use md::*;
mod message;
pub use message::*;
mod upload_document;
pub use upload_document::*;
mod view_document;
pub use view_document::*;
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
    /// Upload a file into a project's documents — the one dialog here that writes anything but a colour. It
    /// is handed the folders that already exist so it can offer them; creating one is typing a path, because
    /// a folder is not a thing that exists until a document is in it.
    ///
    /// `initial_folder` is where the reader currently is in the tree, so an upload starts there instead of at
    /// the root — the folder they were looking at is overwhelmingly the folder they mean, and it stays fully
    /// editable.
    UploadDocument {
        project: String,
        initial_folder: String,
        folders: Vec<String>,
        on_uploaded: EventHandler<()>,
    },
    /// The GitHub repositories connected to one project, listed and edited in one place.
    ///
    /// **`revision` is what makes the list refresh.** This dialog is the one that performs several
    /// different calls without closing — connect, detach, hand over a key, refresh — so after each one
    /// the router re-opens it with the number bumped, and the dialog reloads on the change. A dialog
    /// that closed after every act would make configuring three repositories nine gestures.
    GithubConnections {
        project: String,
        revision: usize,
        on_saved: EventHandler<()>,
    },
    /// Copy files out of a connected repository into the project's own documents. Handed the mirrors as
    /// the Documents screen already has them — the tree it draws IS the index it loaded, so the dialog
    /// costs no request and cannot disagree with what the reader is looking at.
    SyncGithub {
        project: String,
        mirrors: Vec<MirrorChoice>,
        folders: Vec<String>,
        initial_folder: String,
        on_synced: EventHandler<()>,
    },
    /// One goal, in full: its text and its thread. Handed the whole goal rather than an id, because the
    /// screen that opens it is already holding one — a goal arrives with the board on every push.
    ViewGoal {
        goal: GoalResponse,
    },
    /// A card was dragged into Done and the server will refuse the move without a resolution. Carries the
    /// column it is being dropped into rather than assuming `done`: a project can only have one Done, but
    /// spelling it out keeps the dialog from knowing which id that is.
    LandTask {
        handle: String,
        status: String,
    },
    /// Something the board tried failed, said in the caller's own words. Not a toast: a refused move is a
    /// sentence worth reading — "RMS-42 is part of RMS-G7, which is closed" — and a message that fades is a
    /// message half the readers miss.
    Message {
        title: String,
        text: String,
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
        DialogState::UploadDocument {
            project,
            initial_folder,
            folders,
            on_uploaded,
        } => rsx! {
            UploadDocumentDialog {
                project,
                initial_folder,
                folders,
                on_submit: move |submit: UploadSubmit| {
                    begin_submit();
                    spawn(async move {
                        match submit {
                            UploadSubmit::Document { project, path, bytes, content_type } => {
                                match crate::api::upload_document(&project, &path, &bytes, content_type)
                                    .await
                                {
                                    Ok(_) => {
                                        on_uploaded.call(());
                                        close();
                                    }
                                    Err(err) => submit_failed(err.message),
                                }
                            }
                            UploadSubmit::Archive { project, folder, bytes } => {
                                match crate::api::upload_archive(&project, &folder, &bytes).await {
                                    Ok(response) => {
                                        on_uploaded.call(());

                                        // An archive can half-succeed — a `.DS_Store`, an entry over the size
                                        // limit — and a dialog that just closed would say every file arrived.
                                        // So the skips replace it with a report; a clean unpack closes as any
                                        // other save does.
                                        match archive_report(&response) {
                                            Some(text) => {
                                                open(DialogState::Message {
                                                    title: "Unpacked".to_string(),
                                                    text,
                                                });
                                            }
                                            None => close(),
                                        }
                                    }
                                    Err(err) => submit_failed(err.message),
                                }
                            }
                        }
                    });
                },
            }
        },
        DialogState::GithubConnections {
            project,
            revision,
            on_saved,
        } => rsx! {
            GithubConnectionsDialog {
                project: project.clone(),
                revision,
                on_submit: move |submit: GithubSubmit| {
                    let project = project.clone();
                    begin_submit();
                    spawn(async move {
                        let result = match submit {
                            GithubSubmit::Save { name, url, branch, path, key } => {
                                crate::api::set_github_connection(
                                        &project,
                                        &name,
                                        &url,
                                        &branch,
                                        &path,
                                        key,
                                    )
                                    .await
                            }
                            GithubSubmit::Delete { name } => {
                                crate::api::delete_github_connection(&project, &name).await
                            }
                            GithubSubmit::SetKey { name, key } => {
                                crate::api::set_github_key(&project, &name, &key).await
                            }
                            GithubSubmit::Pull { name } => {
                                crate::api::pull_github_connection(&project, &name).await
                            }
                        };
                        match result {
                            Ok(()) => {
                                on_saved.call(());
                                // Re-opened rather than closed: configuring repositories is several acts
                                // in a row, and the bumped revision is what makes the list show the one
                                // just performed. A pull is asynchronous, so what comes back may still
                                // say `pulling` — pressing Refresh again is how you watch it land.
                                open(DialogState::GithubConnections {
                                    project,
                                    revision: revision + 1,
                                    on_saved,
                                });
                            }
                            Err(err) => submit_failed(err.message),
                        }
                    });
                },
            }
        },
        DialogState::SyncGithub {
            project,
            mirrors,
            folders,
            initial_folder,
            on_synced,
        } => rsx! {
            SyncGithubDialog {
                project,
                mirrors,
                folders,
                initial_folder,
                on_submit: move |submit: SyncGithubSubmit| {
                    begin_submit();
                    spawn(async move {
                        match crate::api::sync_github(
                                &submit.project,
                                &submit.connection,
                                submit.paths,
                                &submit.folder,
                                submit.override_existing,
                            )
                            .await
                        {
                            Ok(response) => {
                                on_synced.call(());
                                // A sync half-succeeds in the ordinary case rather than the exceptional
                                // one — without Override, everything already there is skipped, and that
                                // IS the outcome the reader asked for. Closing silently would say every
                                // file arrived.
                                match sync_report(&response) {
                                    Some(text) => {
                                        open(DialogState::Message {
                                            title: "Synced".to_string(),
                                            text,
                                        });
                                    }
                                    None => close(),
                                }
                            }
                            Err(err) => submit_failed(err.message),
                        }
                    });
                },
            }
        },
        DialogState::Message { title, text } => rsx! {
            MessageDialog { title, text }
        },
        DialogState::LandTask { handle, status } => rsx! {
            LandTaskDialog {
                handle: handle.clone(),
                on_submit: move |comment: String| {
                    let handle = handle.clone();
                    let status = status.clone();
                    begin_submit();
                    spawn(async move {
                        match crate::api::move_task(&handle, &status, Some(&comment)).await {
                            // Nothing to refresh: the move comes back as a WebSocket push carrying the whole
                            // board, which is also why this does not write the task back by hand.
                            Ok(()) => close(),
                            Err(err) => submit_failed(err.message),
                        }
                    });
                },
            }
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

/// What to tell the reader after an archive was unpacked, or `None` when everything in it landed.
///
/// **Silence on a clean unpack, a sentence otherwise.** An upload that half-worked is the one outcome this
/// surface could get wrong without anybody noticing: the tree refreshes either way, and a file that was never
/// written looks exactly like a file nobody put in the zip. So a skip is said out loud, with the reason the
/// server gave, and named by what the entry was called INSIDE the archive — the path it would have had is
/// precisely what does not exist.
///
/// One paragraph rather than lines: the dialog that shows it renders text as text, and a newline there is a
/// space.
fn archive_report(response: &UploadArchiveResponse) -> Option<String> {
    if response.skipped.is_empty() {
        return None;
    }

    let skipped: Vec<String> = response
        .skipped
        .iter()
        .map(|itm| format!("{} — {}", itm.name, itm.reason))
        .collect();

    Some(format!(
        "{} written. {} skipped: {}.",
        count_of(response.documents.len(), "document"),
        skipped.len(),
        skipped.join("; ")
    ))
}

/// What to tell the reader after a sync, or `None` when everything chosen landed.
///
/// **Skips are the normal outcome here, not the exceptional one.** With Override off, every file already
/// in the project is skipped on purpose — which is exactly what was asked for, and exactly what would
/// look like a silent failure if the dialog just closed. So the count is always said when there is one,
/// and the reasons are said for the first few: forty identical "already there" lines are not forty
/// pieces of information.
fn sync_report(response: &UploadArchiveResponse) -> Option<String> {
    if response.skipped.is_empty() {
        return None;
    }

    const REASONS_SHOWN: usize = 5;

    let reasons: Vec<String> = response
        .skipped
        .iter()
        .take(REASONS_SHOWN)
        .map(|itm| format!("{} — {}", itm.name, itm.reason))
        .collect();

    let rest = response.skipped.len().saturating_sub(reasons.len());

    let tail = if rest > 0 {
        format!("; and {rest} more")
    } else {
        String::new()
    };

    Some(format!(
        "{} written. {} skipped: {}{tail}.",
        count_of(response.documents.len(), "document"),
        response.skipped.len(),
        reasons.join("; ")
    ))
}

fn count_of(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
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
