use std::rc::Rc;

use dioxus::prelude::*;
use task_manager_shared::projects::ProjectResponse;

/// The three fields a project *is*: name, description, prefix.
///
/// Columns, kinds and members are collections with their own add/edit/delete, so each has its own dialog
/// rather than a section in this one — three mini-CRUDs stacked in a modal would just rebuild the crowded
/// screen this replaced, inside a box.
#[derive(Clone, Default)]
struct ComponentState {
    name: String,
    description: String,
    prefix: String,
    error: String,
    saving: bool,
}

impl ComponentState {
    fn new(project: Option<&ProjectResponse>) -> Self {
        match project {
            Some(project) => Self {
                name: project.name.clone(),
                description: project.description.clone(),
                prefix: project.prefix.clone(),
                ..Default::default()
            },
            None => Self::default(),
        }
    }

    /// Name and prefix are both required — a project with no prefix could not name a single task.
    fn can_save(&self) -> bool {
        !self.saving && !self.name.trim().is_empty() && !self.prefix.trim().is_empty()
    }

    fn begin_save(&mut self) {
        self.saving = true;
        self.error = String::new();
    }

    fn fail(&mut self, message: String) {
        self.saving = false;
        self.error = message;
    }
}

#[component]
pub fn EditProjectDialog(
    project: Option<Rc<ProjectResponse>>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut cs = use_signal(|| ComponentState::new(project.as_deref()));
    let cs_ra = cs.read();

    let existing = project.clone();

    let submit = move |_| {
        let existing = existing.clone();
        let ra = cs.read();
        let (name, description, prefix) = (
            ra.name.trim().to_string(),
            ra.description.trim().to_string(),
            ra.prefix.trim().to_string(),
        );
        drop(ra);

        cs.write().begin_save();

        spawn(async move {
            let result = match existing.as_deref() {
                Some(project) => {
                    crate::api::update_project(&project.id, &name, &description, &prefix).await
                }
                None => crate::api::create_project(&name, &description, &prefix).await,
            };

            match result {
                Ok(()) => {
                    on_saved.call(());
                    super::close();
                }
                Err(err) => cs.write().fail(err.message),
            }
        });
    };

    let title = match project.as_deref() {
        Some(project) => format!("Project {}", project.prefix),
        None => "New project".to_string(),
    };

    // Renaming a prefix keeps the old one resolving, which is worth saying on the screen where the
    // renaming happens — otherwise it looks like it would break every id already written down.
    let history = project
        .as_deref()
        .map(|project| project.prefix_history.join(", "))
        .unwrap_or_default();

    let name = cs_ra.name.as_str();
    let description = cs_ra.description.as_str();
    let prefix = cs_ra.prefix.as_str();
    let error = cs_ra.error.as_str();
    let can_save = cs_ra.can_save();

    let content = rsx! {
        if !error.is_empty() {
            div { class: "error-banner", "{error}" }
        }
        div { class: "form-row",
            label { "Name" }
            input {
                r#type: "text",
                value: "{name}",
                oninput: move |event| cs.write().name = event.value(),
            }
        }
        div { class: "form-row",
            label { "Task prefix" }
            input {
                r#type: "text",
                value: "{prefix}",
                placeholder: "RMS",
                oninput: move |event| cs.write().prefix = event.value().to_uppercase(),
            }
            div { class: "field-hint",
                "The first half of every task id on this board — RMS-000042. Two projects cannot hold the same prefix at once, and a prefix another project once used is refused too, because its old ids still resolve through it."
            }
        }
        div { class: "form-row",
            label { "Description" }
            textarea {
                rows: "3",
                value: "{description}",
                oninput: move |event| cs.write().description = event.value(),
            }
        }
        if !history.is_empty() {
            div { class: "field-hint",
                "Previously: {history}. Ids written under those prefixes still resolve — the tasks_resolve_id MCP tool finds them."
            }
        }
    };

    let ok_button = rsx! {
        button { class: "btn btn-primary", disabled: !can_save, onclick: submit, "Save" }
    };

    super::dialog_template(&title, content, ok_button)
}
