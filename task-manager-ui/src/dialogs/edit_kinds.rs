use std::rc::Rc;

use dioxus::prelude::*;
use rust_extensions::AsStr;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::{ProjectKindResponse, ProjectResponse};

/// The kinds of work a task can be, per project.
#[derive(Clone, Default)]
struct ComponentState {
    new_id: String,
    new_name: String,
    new_description: String,
    new_color: String,
    error: String,
    busy: bool,
}

impl ComponentState {
    fn new() -> Self {
        Self {
            new_color: KindColor::default().as_str().to_string(),
            ..Default::default()
        }
    }

    fn can_add(&self) -> bool {
        !self.busy && !self.new_id.trim().is_empty()
    }

    fn begin(&mut self) {
        self.busy = true;
        self.error = String::new();
    }

    fn added(&mut self) {
        self.busy = false;
        self.new_id = String::new();
        self.new_name = String::new();
        self.new_description = String::new();
    }

    fn fail(&mut self, message: String) {
        self.busy = false;
        self.error = message;
    }
}

#[component]
pub fn EditKindsDialog(project: Rc<ProjectResponse>, on_saved: EventHandler<()>) -> Element {
    let mut cs = use_signal(ComponentState::new);
    let cs_ra = cs.read();

    let add_project_id = project.id.clone();

    let add = move |_| {
        let project_id = add_project_id.clone();
        let ra = cs.read();
        let (id, name, description, color) = (
            ra.new_id.trim().to_string(),
            ra.new_name.trim().to_string(),
            ra.new_description.trim().to_string(),
            ra.new_color.clone(),
        );
        drop(ra);

        cs.write().begin();

        spawn(async move {
            match crate::api::add_kind(&project_id, &id, &name, &description, &color).await {
                Ok(()) => {
                    cs.write().added();
                    on_saved.call(());
                }
                Err(err) => cs.write().fail(err.message),
            }
        });
    };

    let new_id = cs_ra.new_id.as_str();
    let new_name = cs_ra.new_name.as_str();
    let new_description = cs_ra.new_description.as_str();
    let new_color = cs_ra.new_color.clone();
    let error = cs_ra.error.as_str();
    let can_add = cs_ra.can_add();

    let content = rsx! {
        div { class: "field-hint",
            "A kind is optional on a task. The description is what an agent reads before classifying one, so write the rule for applying it rather than a synonym of the name."
        }
        if !error.is_empty() {
            div { class: "error-banner", style: "margin-top: 10px", "{error}" }
        }

        if project.kinds.is_empty() {
            div { class: "empty-note", style: "margin-top: 12px", "No kinds yet." }
        } else {
            div { class: "table-responsive", style: "margin-top: 12px",
                table { class: "table",
                    thead {
                        tr {
                            th { "Id" }
                            th { "Name" }
                            th { "Description" }
                            th { style: "width: 150px", "Colour" }
                            th { style: "width: 150px" }
                        }
                    }
                    tbody {
                        for kind in project.kinds.iter() {
                            RenderKindRow {
                                key: "{kind.id}",
                                project_id: project.id.clone(),
                                kind: kind.clone(),
                                on_saved,
                            }
                        }
                    }
                }
            }
        }

        div { class: "form-row-inline", style: "margin-top: 16px",
            div { class: "form-row",
                label { "Id" }
                input {
                    r#type: "text",
                    placeholder: "bug",
                    value: "{new_id}",
                    oninput: move |event| cs.write().new_id = event.value().to_lowercase(),
                }
            }
            div { class: "form-row",
                label { "Name" }
                input {
                    r#type: "text",
                    value: "{new_name}",
                    oninput: move |event| cs.write().new_name = event.value(),
                }
            }
            div { class: "form-row", style: "flex: 1 1 200px",
                label { "Description" }
                input {
                    r#type: "text",
                    value: "{new_description}",
                    oninput: move |event| cs.write().new_description = event.value(),
                }
            }
            div { class: "form-row",
                label { "Colour" }
                RenderColorPicker {
                    value: new_color,
                    on_pick: move |value| cs.write().new_color = value,
                }
            }
            button { class: "btn btn-primary", disabled: !can_add, onclick: add, "Add kind" }
        }
    };

    super::dialog_template_ex(
        &format!("Kinds · {}", project.prefix),
        content,
        rsx! {},
        Some("modal-xl"),
    )
}

#[component]
fn RenderKindRow(
    project_id: String,
    kind: ProjectKindResponse,
    on_saved: EventHandler<()>,
) -> Element {
    let mut name = use_signal(|| kind.name.clone());
    let mut description = use_signal(|| kind.description.clone());
    let mut color = use_signal(|| kind.color.clone());

    let save_ids = (project_id.clone(), kind.id.clone());

    let save = move |_| {
        let (project_id, kind_id) = save_ids.clone();
        let (name_value, description_value, color_value) = (
            name.read().clone(),
            description.read().clone(),
            color.read().clone(),
        );

        spawn(async move {
            let _ = crate::api::update_kind(
                &project_id,
                &kind_id,
                &name_value,
                &description_value,
                &color_value,
            )
            .await;
            on_saved.call(());
        });
    };

    let delete_ids = (project_id.clone(), kind.id.clone());

    let delete = move |_| {
        let (project_id, kind_id) = delete_ids.clone();

        spawn(async move {
            let _ = crate::api::delete_kind(&project_id, &kind_id).await;
            on_saved.call(());
        });
    };

    rsx! {
        tr {
            td { class: "mono", "{kind.id}" }
            td {
                input {
                    r#type: "text",
                    value: "{name}",
                    oninput: move |event| name.set(event.value()),
                }
            }
            td {
                input {
                    r#type: "text",
                    style: "width: 100%",
                    value: "{description}",
                    oninput: move |event| description.set(event.value()),
                }
            }
            td {
                RenderColorPicker {
                    value: color.read().clone(),
                    on_pick: move |value| color.set(value),
                }
            }
            td {
                div { class: "btn-row",
                    button { class: "btn btn-sm", onclick: save, "Save" }
                    button { class: "btn btn-sm btn-danger", onclick: delete, "Delete" }
                }
            }
        }
    }
}

/// Swatches rather than a dropdown of colour names.
///
/// A colour picker that shows the colours is the obvious win over one that spells them, and the palette
/// is fixed and small enough to lay out in full.
#[component]
fn RenderColorPicker(value: String, on_pick: EventHandler<String>) -> Element {
    let picked = KindColor::parse_or_default(&value);

    // Built outside the markup: an rsx format string takes an identifier or a simple expression, not an
    // `if`, so the whole style is assembled first.
    let swatches: Vec<(KindColor, String)> = KindColor::ALL
        .iter()
        .map(|color| {
            let outline = if *color == picked {
                "#172b4d"
            } else {
                "transparent"
            };

            (
                *color,
                format!(
                    "width: 22px; height: 22px; border-radius: 4px; cursor: pointer; background: {}; border: 2px solid {outline};",
                    color.hex()
                ),
            )
        })
        .collect();

    rsx! {
        div { style: "display: flex; gap: 4px;",
            for (color , style) in swatches {
                button {
                    key: "{color.as_str()}",
                    r#type: "button",
                    title: "{color.title()}",
                    style: "{style}",
                    onclick: {
                        let value = color.as_str().to_string();
                        move |_| on_pick.call(value.clone())
                    },
                }
            }
        }
    }
}
