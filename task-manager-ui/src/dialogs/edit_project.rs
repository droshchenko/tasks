use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_utils::{DataState, RenderState};
use task_manager_shared::column_templates::ColumnTemplateResponse;
use task_manager_shared::projects::ProjectResponse;

/// What a project *is*: name, description, prefix, and which column template it follows.
///
/// Task types and members are collections with their own dialogs. Columns are not here at all: they are
/// configured once per template under Settings, and a project only picks one — which is why this dialog
/// has a dropdown where it used to have a button to a columns editor.
#[derive(Clone, PartialEq)]
struct ComponentState {
    original: Draft,
    draft: Draft,
    error: String,
    saving: bool,
}

/// The editable shape. `PartialEq` is the whole mechanism behind the Save button.
#[derive(Clone, PartialEq, Default)]
struct Draft {
    name: String,
    description: String,
    prefix: String,
    /// Empty means "follow no template" — a legitimate state whose board is Todo -> Done.
    column_template_id: String,
}

impl ComponentState {
    fn new(project: Option<&ProjectResponse>) -> Self {
        let draft = match project {
            Some(project) => Draft {
                name: project.name.clone(),
                description: project.description.clone(),
                prefix: project.prefix.clone(),
                column_template_id: project.column_template_id.clone().unwrap_or_default(),
            },
            None => Draft::default(),
        };

        Self {
            original: draft.clone(),
            draft,
            error: String::new(),
            saving: false,
        }
    }

    fn is_changed(&self) -> bool {
        self.draft != self.original
    }

    /// Name and prefix are both required — a project with no prefix could not name a single task.
    fn can_save(&self) -> bool {
        !self.saving
            && self.is_changed()
            && !self.draft.name.trim().is_empty()
            && !self.draft.prefix.trim().is_empty()
    }

    /// Whether the template changed, which decides if the second request is worth making at all.
    fn template_changed(&self) -> bool {
        self.draft.column_template_id != self.original.column_template_id
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
    // Its own signal rather than a field of `ComponentState`: `DataState` is neither `Clone` nor
    // `PartialEq`, and the state struct must be both — comparing it against `original` is what lights the
    // Save button. The template list is loaded data, not part of the edit, so it does not belong in the
    // comparison anyway.
    let templates_state = use_signal(DataState::<Vec<ColumnTemplateResponse>>::default);
    let cs_ra = cs.read();

    let existing = project.clone();

    let submit = move |_| {
        let existing = existing.clone();
        let ra = cs.read();
        let draft = ra.draft.clone();
        let template_changed = ra.template_changed();
        drop(ra);

        let name = draft.name.trim().to_string();
        let description = draft.description.trim().to_string();
        let prefix = draft.prefix.trim().to_string();
        let template_id = draft.column_template_id.clone();

        cs.write().begin_save();

        spawn(async move {
            // Two calls, because the template is its own endpoint: the basics, then the assignment, and
            // only when it actually moved. A new project always needs the second one if a template was
            // picked, since create has nowhere to carry it.
            let saved_id = match existing.as_deref() {
                Some(project) => {
                    match crate::api::update_project(&project.id, &name, &description, &prefix)
                        .await
                    {
                        Ok(()) => Some(project.id.clone()),
                        Err(err) => {
                            cs.write().fail(err.message);
                            return;
                        }
                    }
                }
                None => match crate::api::create_project(&name, &description, &prefix).await {
                    // Create does not return the new id, so a template picked for a brand-new project is
                    // assigned by finding it back by prefix. Prefixes are unique, which is what makes
                    // that safe.
                    Ok(()) => find_by_prefix(&prefix).await,
                    Err(err) => {
                        cs.write().fail(err.message);
                        return;
                    }
                },
            };

            let assign = existing.is_none() && !template_id.is_empty() || template_changed;

            if assign
                && let Some(project_id) = saved_id
                && let Err(err) = crate::api::set_column_template(&project_id, &template_id).await
            {
                // The basics DID save. Saying so matters: the person would otherwise re-enter a name that
                // is already stored and hit "prefix already used" by themselves.
                cs.write().fail(format!(
                    "The project was saved, but its column template was not: {}",
                    err.message
                ));
                on_saved.call(());
                return;
            }

            on_saved.call(());
            super::close();
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

    let name = cs_ra.draft.name.clone();
    let description = cs_ra.draft.description.clone();
    let prefix = cs_ra.draft.prefix.clone();
    let selected_template = cs_ra.draft.column_template_id.clone();
    let error = cs_ra.error.clone();
    let can_save = cs_ra.can_save();
    let templates = read_templates(templates_state);

    let content = rsx! {
        if !error.is_empty() {
            div { class: "error-banner", "{error}" }
        }
        div { class: "form-row",
            label { "Name" }
            input {
                r#type: "text",
                value: "{name}",
                oninput: move |event| cs.write().draft.name = event.value(),
            }
        }
        div { class: "form-row",
            label { "Task prefix" }
            input {
                r#type: "text",
                value: "{prefix}",
                placeholder: "RMS",
                oninput: move |event| cs.write().draft.prefix = event.value().to_uppercase(),
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
                oninput: move |event| cs.write().draft.description = event.value(),
            }
        }
        div { class: "form-row",
            label { "Columns" }
            select {
                value: "{selected_template}",
                onchange: move |event| cs.write().draft.column_template_id = event.value(),
                option { value: "", "No template — Todo → Done only" }
                for template in templates.iter() {
                    option { value: "{template.id}", "{template.name}" }
                }
            }
            div { class: "field-hint",
                "Columns are configured under Settings → Column templates and shared between projects. Changing the template here moves every task parked in a column the new one does not have: it reads as Todo, and comes back if that column returns."
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

/// The templates to choose from, loading them on first render.
///
/// An empty list on failure rather than an error: not being able to list templates must not stop somebody
/// renaming a project, and the dropdown then simply offers only "no template".
fn read_templates(
    mut state: Signal<DataState<Vec<ColumnTemplateResponse>>>,
) -> Vec<ColumnTemplateResponse> {
    let loaded = match state.read().as_ref() {
        RenderState::Loaded(templates) => Some(templates.clone()),
        RenderState::None => None,
        _ => Some(Vec::new()),
    };

    if let Some(templates) = loaded {
        return templates;
    }

    spawn(async move {
        state.write().set_loading();

        match crate::api::get_column_templates().await {
            Ok(response) => state
                .write()
                .set_loaded(response.map(|itm| itm.templates).unwrap_or_default()),
            Err(_) => state.write().set_loaded(Vec::new()),
        }
    });

    Vec::new()
}

/// Find a just-created project by its prefix, which is unique.
///
/// Only needed because create answers with an empty body. Returning the new id from the server would be
/// better and is the obvious follow-up; this keeps the whole thing to one dialog in the meantime.
async fn find_by_prefix(prefix: &str) -> Option<String> {
    crate::api::get_projects()
        .await
        .ok()?
        .projects
        .into_iter()
        .find(|itm| itm.prefix == prefix)
        .map(|itm| itm.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectResponse {
        ProjectResponse {
            id: "p".to_string(),
            name: "Project".to_string(),
            description: String::new(),
            prefix: "RMS".to_string(),
            prefix_history: Vec::new(),
            columns: Vec::new(),
            column_template_id: Some("tpl".to_string()),
            column_template_name: Some("Development".to_string()),
            kinds: Vec::new(),
            members: Vec::new(),
            tasks_amount: 0,
        }
    }

    #[test]
    fn save_is_offered_only_once_something_differs() {
        let mut cs = ComponentState::new(Some(&project()));
        assert!(!cs.can_save());

        cs.draft.name = "Renamed".to_string();
        assert!(cs.can_save());

        cs.draft.name = "Project".to_string();
        assert!(!cs.is_changed());
    }

    /// Picking another template is a change on its own, and is what decides whether the second request
    /// runs at all.
    #[test]
    fn changing_only_the_template_is_a_saveable_change() {
        let mut cs = ComponentState::new(Some(&project()));
        assert!(!cs.template_changed());

        cs.draft.column_template_id = "other".to_string();

        assert!(cs.is_changed());
        assert!(cs.can_save());
        assert!(cs.template_changed());
    }

    /// Choosing "no template" is a real choice, not a cleared field.
    #[test]
    fn clearing_the_template_is_a_change() {
        let mut cs = ComponentState::new(Some(&project()));
        cs.draft.column_template_id = String::new();

        assert!(cs.template_changed());
        assert!(cs.can_save());
    }

    #[test]
    fn a_project_needs_a_name_and_a_prefix() {
        let mut cs = ComponentState::new(None);
        assert!(
            !cs.can_save(),
            "an untouched new project has nothing to save"
        );

        cs.draft.name = "Project".to_string();
        assert!(!cs.can_save(), "still no prefix");

        cs.draft.prefix = "RMS".to_string();
        assert!(cs.can_save());

        cs.draft.name = "   ".to_string();
        assert!(!cs.can_save());
    }
}
