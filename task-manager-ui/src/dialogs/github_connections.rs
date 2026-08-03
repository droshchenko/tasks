use dioxus::prelude::*;
use dioxus_utils::{DataState, RenderState};
use task_manager_shared::github::{GithubConnectionResponse, GithubMirrorState};

/// What the reader asked the router to do. Four different calls, because they are four different acts:
/// connecting a repository, detaching one, handing over a key, and asking for a refresh.
///
/// A dialog that manages a LIST cannot follow the one-model-in-one-model-out shape the rest of them do —
/// there is no single object to hand back. So it hands back the ACT instead, and the router still owns
/// every request, which is the part of the pattern that matters.
#[derive(Clone, PartialEq)]
pub enum GithubSubmit {
    Save {
        name: String,
        url: String,
        branch: String,
        path: String,
        /// `None` leaves whatever key the server is holding — which is what makes editing a branch safe.
        key: Option<String>,
    },
    Delete {
        name: String,
    },
    SetKey {
        name: String,
        key: String,
    },
    Pull {
        name: String,
    },
}

#[derive(Default)]
struct ComponentState {
    connections: DataState<Vec<GithubConnectionResponse>>,
    /// The form, when one is open. `None` is the list.
    editing: Option<EditForm>,
    /// Which connection the key box is open for, and what has been typed in it. Separate from the form
    /// above because they are separate acts: an admin connects a repository, and anybody on the project
    /// can hand over a key for it afterwards — including after a restart, which is when it matters.
    key_for: Option<String>,
    key_value: String,
}

#[derive(Clone, PartialEq, Default)]
struct EditForm {
    /// The name this form started from, or `None` for a new connection. What tells an edit from a
    /// create — and what makes renaming impossible on purpose: the name is the folder every mirrored
    /// path goes through, and renaming it would move a tree somebody may be reading.
    original_name: Option<String>,
    name: String,
    url: String,
    branch: String,
    path: String,
    key: String,
}

/// The repositories connected to one project.
///
/// **Configuration and credential are separated here as sharply as they are on the server**, and the
/// dialog says so out loud: the repository is a row that survives everything, the key is held in memory
/// and is gone the next time the service restarts. That is not a caveat worth burying — it is the reason
/// a connection can read `needs-key` on a Monday morning having worked all Friday, and somebody looking
/// at this screen needs to know it before they go looking for a bug.
#[component]
pub fn GithubConnectionsDialog(
    project: String,
    revision: usize,
    on_submit: EventHandler<GithubSubmit>,
) -> Element {
    let mut cs = use_signal(ComponentState::default);

    // The router re-opens this dialog with a new revision after every successful call, which is what
    // makes the list below show what was just done. A prop is not a signal, so `use_reactive!` wakes it.
    use_effect(use_reactive!(|revision| {
        let _ = revision;
        cs.write().connections.reset();
    }));

    let cs_ra = cs.read();

    let feedback = super::feedback();

    let connections = match get_connections(cs, &cs_ra, &project) {
        Ok(connections) => connections.to_vec(),
        Err(element) => {
            return super::dialog_template_ex(
                "Connected repositories",
                element,
                rsx! {},
                Some("modal-lg"),
            );
        }
    };

    let editing = cs_ra.editing.clone();
    let key_for = cs_ra.key_for.clone();
    let key_value = cs_ra.key_value.clone();

    drop(cs_ra);

    let content = match editing.as_ref() {
        Some(form) => render_form(cs, form.clone(), &feedback.error),
        None => render_list(
            cs,
            &connections,
            key_for.as_deref(),
            &key_value,
            &feedback.error,
            on_submit,
        ),
    };

    let ok = match editing.as_ref() {
        Some(form) => {
            let ready = !form.name.trim().is_empty() && !form.url.trim().is_empty();
            let form = form.clone();

            rsx! {
                button {
                    class: "btn btn-primary",
                    disabled: !ready || feedback.saving,
                    onclick: move |_| {
                        let key = form.key.trim().to_string();

                        on_submit
                            .call(GithubSubmit::Save {
                                name: form.name.trim().to_string(),
                                url: form.url.trim().to_string(),
                                branch: form.branch.trim().to_string(),
                                path: form.path.trim().to_string(),
                                // An empty box on an EDIT means "leave the key alone", not "forget it" —
                                // the box is always empty when the form opens, because the server will
                                // not tell anybody what it is holding.
                                key: if key.is_empty() && form.original_name.is_some() {
                                    None
                                } else {
                                    Some(key)
                                },
                            });
                    },
                    if feedback.saving { "Connecting…" } else { "Connect" }
                }
            }
        }
        None => rsx! {
            button {
                class: "btn btn-primary",
                disabled: feedback.saving,
                onclick: move |_| {
                    cs.write().editing = Some(EditForm::default());
                },
                "Connect a repository"
            }
        },
    };

    super::dialog_template_ex("Connected repositories", content, ok, Some("modal-lg"))
}

fn render_list(
    mut cs: Signal<ComponentState>,
    connections: &[GithubConnectionResponse],
    key_for: Option<&str>,
    key_value: &str,
    error: &str,
    on_submit: EventHandler<GithubSubmit>,
) -> Element {
    rsx! {
        div { class: "github-connections",
            if connections.is_empty() {
                div { class: "empty-note",
                    "No repository is connected. A connected one appears in this project's documents as the folder "
                    span { class: "mono", "github/<name>/" }
                    " — read-only, refreshed every ten minutes, and copied in only when you sync it."
                }
            } else {
                table { class: "table",
                    thead {
                        tr {
                            th { "Folder" }
                            th { "Repository" }
                            th { "State" }
                            th { style: "width: 210px" }
                        }
                    }
                    tbody {
                        for connection in connections.iter() {
                            RenderRow {
                                key: "{connection.name}",
                                connection: connection.clone(),
                                cs,
                                on_submit,
                            }
                        }
                    }
                }
            }

            // The key box, opened from a row. Outside the table because it is a sentence and a text
            // field, and a row is not a shape that can hold either.
            if let Some(name) = key_for {
                div { class: "form-row",
                    label { "Key for {name}" }
                    input {
                        r#type: "password",
                        placeholder: "a GitHub token with read access",
                        value: "{key_value}",
                        oninput: move |event| cs.write().key_value = event.value(),
                    }
                    div { class: "field-hint",
                        "Held in this server's memory only — never written to the database, and gone when the service restarts. Leave it empty to forget the key being held."
                    }
                    div { class: "page-actions",
                        button {
                            class: "btn btn-primary",
                            onclick: {
                                let name = name.to_string();

                                move |_| {
                                    let key = cs.read().key_value.clone();

                                    let mut write = cs.write();
                                    write.key_for = None;
                                    write.key_value = String::new();
                                    drop(write);

                                    on_submit.call(GithubSubmit::SetKey { name: name.clone(), key });
                                }
                            },
                            "Hand it over"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                let mut write = cs.write();
                                write.key_for = None;
                                write.key_value = String::new();
                            },
                            "Cancel"
                        }
                    }
                }
            }

            if !error.is_empty() {
                div { class: "error-note", "{error}" }
            }
        }
    }
}

#[component]
fn RenderRow(
    connection: GithubConnectionResponse,
    cs: Signal<ComponentState>,
    on_submit: EventHandler<GithubSubmit>,
) -> Element {
    let mut cs = cs;

    let name = connection.name.clone();
    let branch = if connection.branch.is_empty() {
        "default branch".to_string()
    } else {
        connection.branch.clone()
    };

    rsx! {
        tr {
            td { class: "mono", "{connection.name}" }
            td {
                div { "{connection.repo}" }
                div { class: "field-hint",
                    "{branch}"
                    if !connection.path.is_empty() {
                        " · {connection.path}/"
                    }
                }
            }
            td {
                div { "{state_text(&connection)}" }
                if !connection.error.is_empty() {
                    div { class: "field-hint", "{connection.error}" }
                }
            }
            td {
                div { class: "page-actions",
                    button {
                        class: "btn",
                        title: "Read the repository again now, rather than at the next ten-minute tick",
                        onclick: {
                            let name = name.clone();
                            move |_| on_submit.call(GithubSubmit::Pull { name: name.clone() })
                        },
                        "Refresh"
                    }
                    button {
                        class: "btn",
                        title: "Hand the server a key for this repository",
                        onclick: {
                            let name = name.clone();
                            move |_| {
                                let mut write = cs.write();
                                write.key_for = Some(name.clone());
                                write.key_value = String::new();
                            }
                        },
                        "Key"
                    }
                    button {
                        class: "btn",
                        onclick: {
                            let connection = connection.clone();
                            move |_| {
                                cs.write().editing = Some(EditForm {
                                    original_name: Some(connection.name.clone()),
                                    name: connection.name.clone(),
                                    // The repository as `owner/repo`, which the server parses back into
                                    // the same thing it parsed out of whatever was pasted originally.
                                    url: connection.repo.clone(),
                                    branch: connection.branch.clone(),
                                    path: connection.path.clone(),
                                    // Always empty: the server will not say what it is holding, so an
                                    // edit starts from "leave it alone".
                                    key: String::new(),
                                });
                            }
                        },
                        "Edit"
                    }
                    button {
                        class: "btn",
                        title: "Detach the repository. Documents already synced out of it are not touched",
                        onclick: {
                            let name = name.clone();
                            move |_| on_submit.call(GithubSubmit::Delete { name: name.clone() })
                        },
                        "Remove"
                    }
                }
            }
        }
    }
}

fn render_form(mut cs: Signal<ComponentState>, form: EditForm, error: &str) -> Element {
    let editing_existing = form.original_name.is_some();

    let key_placeholder = if editing_existing {
        "leave empty to keep the key being held"
    } else {
        "empty for a public repository"
    };

    rsx! {
        div { class: "github-form",
            div { class: "form-row",
                label { "Folder name" }
                input {
                    r#type: "text",
                    placeholder: "specs",
                    disabled: editing_existing,
                    value: "{form.name}",
                    oninput: move |event| {
                        if let Some(form) = cs.write().editing.as_mut() {
                            form.name = event.value();
                        }
                    },
                }
                div { class: "field-hint",
                    if editing_existing {
                        "The name cannot change — it is the folder every mirrored path goes through, and moving it would move a tree somebody may be reading."
                    } else {
                        "What this appears as in the documents tree, under github/."
                    }
                }
            }

            div { class: "form-row",
                label { "Repository" }
                input {
                    r#type: "text",
                    placeholder: "https://github.com/owner/repo/tree/main/docs",
                    value: "{form.url}",
                    oninput: move |event| {
                        if let Some(form) = cs.write().editing.as_mut() {
                            form.url = event.value();
                        }
                    },
                }
                div { class: "field-hint",
                    "A github.com url, an owner/repo, or an ssh remote. Paste the address bar after browsing into a folder and the branch and folder below fill themselves in."
                }
            }

            div { class: "form-row",
                label { "Branch" }
                input {
                    r#type: "text",
                    placeholder: "empty for the repository's default branch",
                    value: "{form.branch}",
                    oninput: move |event| {
                        if let Some(form) = cs.write().editing.as_mut() {
                            form.branch = event.value();
                        }
                    },
                }
            }

            div { class: "form-row",
                label { "Folder in the repository" }
                input {
                    r#type: "text",
                    placeholder: "docs — empty for the whole repository",
                    value: "{form.path}",
                    oninput: move |event| {
                        if let Some(form) = cs.write().editing.as_mut() {
                            form.path = event.value();
                        }
                    },
                }
                div { class: "field-hint",
                    "Whatever you name here becomes the ROOT of the folder — connect docs/ and its contents sit directly under the connection, not under a docs/ inside it."
                }
            }

            div { class: "form-row",
                label { "Key" }
                input {
                    r#type: "password",
                    placeholder: "{key_placeholder}",
                    value: "{form.key}",
                    oninput: move |event| {
                        if let Some(form) = cs.write().editing.as_mut() {
                            form.key = event.value();
                        }
                    },
                }
                div { class: "field-hint",
                    "Held in this server's memory and written to no table — so it has to be given again after a restart, and a database dump carries no credential. A public repository needs none."
                }
            }

            if !error.is_empty() {
                div { class: "error-note", "{error}" }
            }

            div { class: "page-actions",
                button {
                    class: "btn",
                    onclick: move |_| cs.write().editing = None,
                    "Back to the list"
                }
            }
        }
    }
}

/// What a connection's state says, in a sentence rather than a keyword.
///
/// The vocabulary travels as an open string — an unknown one is drawn as itself rather than swallowed,
/// which is the same leniency every other open vocabulary in this client gets.
fn state_text(connection: &GithubConnectionResponse) -> String {
    let files = if connection.skipped_amount > 0 {
        format!(
            "{} files ({} not mirrored)",
            connection.files_amount, connection.skipped_amount
        )
    } else {
        format!("{} files", connection.files_amount)
    };

    match connection.state.as_str() {
        GithubMirrorState::READY => files,
        GithubMirrorState::PULLING => "reading…".to_string(),
        GithubMirrorState::PENDING => "not read yet".to_string(),
        GithubMirrorState::NEEDS_KEY => {
            if connection.has_key {
                "the key is not enough".to_string()
            } else {
                "needs a key".to_string()
            }
        }
        // A failure keeps whatever the last good read left, so what is still readable is worth saying
        // beside the fact that the last attempt did not work.
        GithubMirrorState::FAILED => format!("failed · {files} still readable"),
        other => other.to_string(),
    }
}

fn get_connections<'s>(
    mut cs: Signal<ComponentState>,
    cs_ra: &'s ComponentState,
    project: &str,
) -> Result<&'s [GithubConnectionResponse], Element> {
    match cs_ra.connections.as_ref() {
        RenderState::None => {
            let project = project.to_string();

            spawn(async move {
                cs.write().connections.set_loading();

                match crate::api::get_github_connections(&project).await {
                    Ok(response) => cs.write().connections.set_loaded(response.connections),
                    Err(err) => cs.write().connections.set_error(err.message),
                }
            });

            Err(rsx! {
                div { class: "loading-note", "Loading…" }
            })
        }
        RenderState::Loading => Err(rsx! {
            div { class: "loading-note", "Loading…" }
        }),
        RenderState::Loaded(connections) => Ok(connections.as_slice()),
        RenderState::Error(err) => Err(rsx! {
            div { class: "error-note", "{err}" }
        }),
    }
}
