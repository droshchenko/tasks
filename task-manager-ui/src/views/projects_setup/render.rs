use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_utils::{DataState, RenderState};
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::ProjectResponse;

use crate::dialogs::DialogState;

/// Projects setup: every project in one table, edited through dialogs.
///
/// A table rather than a dropdown-plus-four-cards, which is what this was: the dropdown hid every project
/// but one, so "which prefixes are taken" and "which project has nobody on it" were questions you had to
/// click through the list to answer. They are now columns.
#[component]
pub fn RenderProjectsSetup() -> Element {
    let data = use_signal(DataState::<Vec<ProjectResponse>>::default);

    render_table(data)
}

/// Opening the editor and reloading afterwards are the same two lines everywhere, so they live in one
/// place. `data.write().reset()` puts the `DataState` back to `None`, and the next render re-reads —
/// the server is the only thing that says what was actually stored.
fn open(
    data: Signal<DataState<Vec<ProjectResponse>>>,
    make: impl FnOnce(EventHandler<()>) -> DialogState,
) {
    let mut data = data;

    crate::dialogs::open(make(EventHandler::new(move |_| data.write().reset())));
}

fn render_table(data: Signal<DataState<Vec<ProjectResponse>>>) -> Element {
    let data_ra = data.read();

    let projects = match get_projects(data, &data_ra) {
        Ok(projects) => projects,
        Err(element) => return element,
    };

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Projects setup" }
            div { class: "page-actions",
                button {
                    class: "btn btn-primary",
                    onclick: move |_| {
                        open(data, |on_saved| DialogState::EditProject { project: None, on_saved });
                    },
                    "New project"
                }
            }
        }

        if projects.is_empty() {
            div { class: "empty-note", "No projects yet. Create the first one." }
        } else {
            div { class: "table-responsive",
                table { class: "table",
                    thead {
                        tr {
                            th { "Prefix" }
                            th { "Name" }
                            th { "Description" }
                            th { class: "num", "Tasks" }
                            th { "Column template" }
                            th { "Task types" }
                            th { class: "num", "Members" }
                            th { style: "width: 230px" }
                        }
                    }
                    tbody {
                        for project in projects.iter() {
                            RenderRow { key: "{project.id}", project: project.clone(), data }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RenderRow(project: ProjectResponse, data: Signal<DataState<Vec<ProjectResponse>>>) -> Element {
    // Shared by all four buttons, and cheap: an `Rc` is what the dialog state carries anyway.
    let project = Rc::new(project);

    // The columns a project actually configured, in order — Todo and Done are left out because every
    // project has them and listing them in every row would say nothing.
    let mut ordered = project.columns.clone();
    ordered.sort_by_key(|itm| itm.order);

    let columns = ordered
        .iter()
        .map(|itm| itm.name.as_str())
        .collect::<Vec<&str>>()
        .join(" → ");

    let for_edit = project.clone();
    let for_kinds = project.clone();
    let for_members = project.clone();

    rsx! {
        tr {
            td { class: "mono", "{project.prefix}" }
            td { "{project.name}" }
            td { class: "muted", "{project.description}" }
            td { class: "num", "{project.tasks_amount}" }
            td {
                // The template's NAME, then what it resolves to. Which template a project follows is the
                // thing you change; the column list is the thing you check afterwards.
                if let Some(template) = project.column_template_name.as_ref() {
                    div { "{template}" }
                    div { class: "field-hint", "{columns}" }
                } else {
                    span { class: "muted", "Todo → Done" }
                }
            }
            td {
                if project.kinds.is_empty() {
                    span { class: "muted", "—" }
                } else {
                    for kind in project.kinds.iter() {
                        RenderKindTag { key: "{kind.id}", name: kind.name.clone(), color: kind.color.clone() }
                    }
                }
            }
            td { class: "num", "{project.members.len()}" }
            td {
                div { class: "btn-row",
                    button {
                        class: "btn btn-sm",
                        onclick: move |_| {
                            let project = for_edit.clone();
                            open(data, |on_saved| DialogState::EditProject { project: Some(project), on_saved });
                        },
                        "Edit"
                    }
                    button {
                        class: "btn btn-sm",
                        onclick: move |_| {
                            let project = for_kinds.clone();
                            open(data, |on_saved| DialogState::EditKinds { project, on_saved });
                        },
                        "Task types"
                    }
                    button {
                        class: "btn btn-sm",
                        onclick: move |_| {
                            let project = for_members.clone();
                            open(data, |on_saved| DialogState::EditMembers { project, on_saved });
                        },
                        "Members"
                    }
                }
            }
        }
    }
}

/// A kind as it appears on a sticker, so the table and the board agree on what a kind looks like.
#[component]
fn RenderKindTag(name: String, color: String) -> Element {
    let hex = KindColor::parse_or_default(&color).hex();

    rsx! {
        span { class: "sticker-kind", style: "background: {hex}; margin-right: 4px", "{name}" }
    }
}

fn get_projects(
    mut data: Signal<DataState<Vec<ProjectResponse>>>,
    data_ra: &DataState<Vec<ProjectResponse>>,
) -> Result<&[ProjectResponse], Element> {
    match data_ra.as_ref() {
        RenderState::None => {
            spawn(async move {
                data.write().set_loading();

                match crate::api::get_projects().await {
                    Ok(response) => data.write().set_loaded(response.projects),
                    Err(err) => data.write().set_error(err.message),
                }
            });

            Err(rsx! {
                div { class: "loading-note", "Loading…" }
            })
        }
        RenderState::Loading => Err(rsx! {
            div { class: "loading-note", "Loading…" }
        }),
        RenderState::Loaded(projects) => Ok(projects.as_slice()),
        RenderState::Error(err) => Err(rsx! {
            div { class: "error-banner", "{err}" }
        }),
    }
}
