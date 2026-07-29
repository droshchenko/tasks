use std::rc::Rc;

use dioxus::prelude::*;
use task_manager_shared::projects::{ProjectColumnResponse, ProjectResponse};

/// The columns between Todo and Done: add, rename, reorder, delete.
///
/// Todo and Done are drawn but not editable — they exist in every project by definition. They are shown
/// rather than left out so the order of what you add is visible against the ends it sits between.
#[derive(Clone, Default)]
struct ComponentState {
    new_id: String,
    new_name: String,
    new_description: String,
    new_order: String,
    error: String,
    busy: bool,
}

impl ComponentState {
    fn new() -> Self {
        Self {
            new_order: "10".to_string(),
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
pub fn EditColumnsDialog(project: Rc<ProjectResponse>, on_saved: EventHandler<()>) -> Element {
    let mut cs = use_signal(ComponentState::new);
    let cs_ra = cs.read();

    let add_project_id = project.id.clone();

    let add = move |_| {
        let project_id = add_project_id.clone();
        let ra = cs.read();
        let (id, name, description) = (
            ra.new_id.trim().to_string(),
            ra.new_name.trim().to_string(),
            ra.new_description.trim().to_string(),
        );
        // An unparseable order is 10 rather than an error: the field is a nicety, and refusing the whole
        // add over it would be the wrong trade.
        let order = ra.new_order.trim().parse::<i32>().unwrap_or(10);
        drop(ra);

        cs.write().begin();

        spawn(async move {
            match crate::api::add_column(&project_id, &id, &name, &description, order).await {
                Ok(()) => {
                    cs.write().added();
                    on_saved.call(());
                }
                Err(err) => cs.write().fail(err.message),
            }
        });
    };

    let mut ordered = project.columns.clone();
    ordered.sort_by_key(|itm| itm.order);

    let new_id = cs_ra.new_id.as_str();
    let new_name = cs_ra.new_name.as_str();
    let new_description = cs_ra.new_description.as_str();
    let new_order = cs_ra.new_order.as_str();
    let error = cs_ra.error.as_str();
    let can_add = cs_ra.can_add();

    let content = rsx! {
        div { class: "field-hint",
            "A column id is typed in once and never renamed — tasks point at it as their status. Deleting a column leaves its tasks alone: they read as Todo until a column with that id exists again."
        }
        if !error.is_empty() {
            div { class: "error-banner", style: "margin-top: 10px", "{error}" }
        }

        div { class: "table-responsive", style: "margin-top: 12px",
            table { class: "table",
                thead {
                    tr {
                        th { style: "width: 80px", "Order" }
                        th { "Id" }
                        th { "Name" }
                        th { "Description" }
                        th { style: "width: 150px" }
                    }
                }
                tbody {
                    tr {
                        td { class: "muted", "first" }
                        td { class: "mono", "todo" }
                        td { "Todo" }
                        td { class: "muted", "Always present" }
                        td {}
                    }
                    for column in ordered.iter() {
                        RenderColumnRow {
                            key: "{column.id}",
                            project_id: project.id.clone(),
                            column: column.clone(),
                            on_saved,
                        }
                    }
                    tr {
                        td { class: "muted", "last" }
                        td { class: "mono", "done" }
                        td { "Done" }
                        td { class: "muted", "Always present. A dependency counts as satisfied only here" }
                        td {}
                    }
                }
            }
        }

        div { class: "form-row-inline", style: "margin-top: 16px",
            div { class: "form-row",
                label { "Order" }
                input {
                    r#type: "text",
                    style: "width: 70px",
                    value: "{new_order}",
                    oninput: move |event| cs.write().new_order = event.value(),
                }
            }
            div { class: "form-row",
                label { "Id" }
                input {
                    r#type: "text",
                    placeholder: "in-progress",
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
            button { class: "btn btn-primary", disabled: !can_add, onclick: add, "Add column" }
        }
    };

    // No OK button: every row saves and deletes itself, and adding is its own button. A "Save" here
    // would be a button that does nothing, which is worse than no button.
    super::dialog_template_ex(
        &format!("Columns · {}", project.prefix),
        content,
        rsx! {},
        Some("modal-xl"),
    )
}

#[component]
fn RenderColumnRow(
    project_id: String,
    column: ProjectColumnResponse,
    on_saved: EventHandler<()>,
) -> Element {
    let mut name = use_signal(|| column.name.clone());
    let mut description = use_signal(|| column.description.clone());
    let mut order = use_signal(|| column.order.to_string());

    let save_ids = (project_id.clone(), column.id.clone());
    let fallback_order = column.order;

    let save = move |_| {
        let (project_id, column_id) = save_ids.clone();
        let (name_value, description_value) = (name.read().clone(), description.read().clone());
        let order_value = order.read().trim().parse::<i32>().unwrap_or(fallback_order);

        spawn(async move {
            let _ = crate::api::update_column(
                &project_id,
                &column_id,
                &name_value,
                &description_value,
                order_value,
            )
            .await;
            on_saved.call(());
        });
    };

    let delete_ids = (project_id.clone(), column.id.clone());

    let delete = move |_| {
        let (project_id, column_id) = delete_ids.clone();

        spawn(async move {
            let _ = crate::api::delete_column(&project_id, &column_id).await;
            on_saved.call(());
        });
    };

    rsx! {
        tr {
            td {
                input {
                    r#type: "text",
                    style: "width: 70px",
                    value: "{order}",
                    oninput: move |event| order.set(event.value()),
                }
            }
            td { class: "mono", "{column.id}" }
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
                div { class: "btn-row",
                    button { class: "btn btn-sm", onclick: save, "Save" }
                    button { class: "btn btn-sm btn-danger", onclick: delete, "Delete" }
                }
            }
        }
    }
}
