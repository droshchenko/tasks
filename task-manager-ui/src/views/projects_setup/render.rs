use dioxus::prelude::*;
use rust_extensions::AsStr;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::ProjectResponse;
use task_manager_shared::users::UserResponse;

/// Projects setup: the one screen where the product is configured.
///
/// It is a *setup* screen, not a board — nothing here creates or moves a task. What it owns is the four
/// things a project is made of: its name and prefix, its columns, its kinds, and who may see it.
#[component]
pub fn RenderProjectsSetup() -> Element {
    let mut projects = use_signal(Vec::<ProjectResponse>::new);
    let mut users = use_signal(Vec::<UserResponse>::new);
    let mut selected = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut loading = use_signal(|| true);
    // Bumped after every successful write, which is what re-reads the list. Cheaper to reason about than
    // patching the local copy, and it cannot disagree with the server.
    let mut revision = use_signal(|| 0_u32);

    use_future(move || async move {
        let _ = *revision.read();

        match crate::api::get_projects().await {
            Ok(response) => {
                let keep = selected.read().clone();

                let next = if response.projects.iter().any(|itm| itm.id == keep) {
                    keep
                } else {
                    response
                        .projects
                        .first()
                        .map(|itm| itm.id.clone())
                        .unwrap_or_default()
                };

                projects.set(response.projects);
                selected.set(next);
                loading.set(false);
            }
            Err(err) => {
                error.set(err.message);
                loading.set(false);
            }
        }
    });

    // The roster feeds the membership checkboxes. Read once — it changes on the Users screen.
    use_future(move || async move {
        if let Ok(Some(response)) = crate::api::get_users().await {
            users.set(response.users);
        }
    });

    let projects_ra = projects.read();
    let selected_id = selected.read().clone();
    let error_text = error.read().clone();

    if *loading.read() {
        return rsx! {
            div { class: "loading-note", "Loading…" }
        };
    }

    let current = projects_ra
        .iter()
        .find(|itm| itm.id == selected_id)
        .cloned();

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Projects setup" }
            if !projects_ra.is_empty() {
                select {
                    value: "{selected_id}",
                    onchange: move |event| selected.set(event.value()),
                    for project in projects_ra.iter() {
                        option { value: "{project.id}", "{project.prefix} · {project.name}" }
                    }
                }
            }
        }

        if !error_text.is_empty() {
            div { class: "error-banner", "{error_text}" }
        }

        RenderCreateProject { on_saved: move |_| revision += 1 }

        if let Some(project) = current {
            RenderProjectDetails {
                key: "{project.id}",
                project: project.clone(),
                users: users.read().clone(),
                on_saved: move |_| revision += 1,
            }
        } else {
            div { class: "empty-note", "No projects yet. Create the first one above." }
        }
    }
}

#[component]
fn RenderCreateProject(on_saved: EventHandler<()>) -> Element {
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut prefix = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut saving = use_signal(|| false);

    let submit = move |_| {
        let (name_value, description_value, prefix_value) = (
            name.read().clone(),
            description.read().clone(),
            prefix.read().clone(),
        );

        saving.set(true);
        error.set(String::new());

        spawn(async move {
            match crate::api::create_project(&name_value, &description_value, &prefix_value).await {
                Ok(()) => {
                    name.set(String::new());
                    description.set(String::new());
                    prefix.set(String::new());
                    saving.set(false);
                    on_saved.call(());
                }
                Err(err) => {
                    saving.set(false);
                    error.set(err.message);
                }
            }
        });
    };

    let can_save = !name.read().trim().is_empty() && !prefix.read().trim().is_empty();
    let is_saving = *saving.read();
    let error_text = error.read().clone();

    rsx! {
        div { class: "card",
            div { class: "card-title", "New project" }
            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }
            div { class: "form-row-inline",
                div { class: "form-row",
                    label { "Name" }
                    input {
                        r#type: "text",
                        value: "{name}",
                        oninput: move |event| name.set(event.value()),
                    }
                }
                div { class: "form-row",
                    label { "Task prefix" }
                    input {
                        r#type: "text",
                        value: "{prefix}",
                        placeholder: "RMS",
                        oninput: move |event| prefix.set(event.value().to_uppercase()),
                    }
                }
                div { class: "form-row", style: "flex: 1 1 260px",
                    label { "Description" }
                    input {
                        r#type: "text",
                        value: "{description}",
                        oninput: move |event| description.set(event.value()),
                    }
                }
                button {
                    class: "btn btn-primary",
                    disabled: !can_save || is_saving,
                    onclick: submit,
                    "Create"
                }
            }
            div { class: "field-hint",
                "The prefix is the first half of every task id on this board — RMS-000042. It cannot be held by two projects at once, and a task id is built from whichever prefix the project carries at the time it is read."
            }
        }
    }
}

#[component]
fn RenderProjectDetails(
    project: ProjectResponse,
    users: Vec<UserResponse>,
    on_saved: EventHandler<()>,
) -> Element {
    rsx! {
        RenderProjectBasics { project: project.clone(), on_saved }
        RenderColumns { project: project.clone(), on_saved }
        RenderKinds { project: project.clone(), on_saved }
        RenderMembers { project: project.clone(), users, on_saved }
    }
}

#[component]
fn RenderProjectBasics(project: ProjectResponse, on_saved: EventHandler<()>) -> Element {
    let mut name = use_signal(|| project.name.clone());
    let mut description = use_signal(|| project.description.clone());
    let mut prefix = use_signal(|| project.prefix.clone());
    let mut error = use_signal(String::new);

    let project_id = project.id.clone();
    let history = project.prefix_history.join(", ");

    let submit = move |_| {
        let project_id = project_id.clone();
        let (name_value, description_value, prefix_value) = (
            name.read().clone(),
            description.read().clone(),
            prefix.read().clone(),
        );

        error.set(String::new());

        spawn(async move {
            match crate::api::update_project(
                &project_id,
                &name_value,
                &description_value,
                &prefix_value,
            )
            .await
            {
                Ok(()) => on_saved.call(()),
                Err(err) => error.set(err.message),
            }
        });
    };

    let error_text = error.read().clone();

    rsx! {
        div { class: "card",
            div { class: "card-title", "Project" }
            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }
            div { class: "form-row-inline",
                div { class: "form-row",
                    label { "Name" }
                    input {
                        r#type: "text",
                        value: "{name}",
                        oninput: move |event| name.set(event.value()),
                    }
                }
                div { class: "form-row",
                    label { "Task prefix" }
                    input {
                        r#type: "text",
                        value: "{prefix}",
                        oninput: move |event| prefix.set(event.value().to_uppercase()),
                    }
                }
                div { class: "form-row", style: "flex: 1 1 260px",
                    label { "Description" }
                    input {
                        r#type: "text",
                        value: "{description}",
                        oninput: move |event| description.set(event.value()),
                    }
                }
                button { class: "btn btn-primary", onclick: submit, "Save" }
            }
            if !history.is_empty() {
                div { class: "field-hint",
                    "Previously: {history}. Old ids written under those prefixes can still be traced with the tasks_resolve_id MCP tool."
                }
            }
        }
    }
}

#[component]
fn RenderColumns(project: ProjectResponse, on_saved: EventHandler<()>) -> Element {
    let mut new_id = use_signal(String::new);
    let mut new_name = use_signal(String::new);
    let mut new_description = use_signal(String::new);
    let mut new_order = use_signal(|| "10".to_string());
    let mut error = use_signal(String::new);

    let project_id = project.id.clone();

    let add = move |_| {
        let project_id = project_id.clone();
        let (id, name, description) = (
            new_id.read().clone(),
            new_name.read().clone(),
            new_description.read().clone(),
        );
        let order = new_order.read().trim().parse::<i32>().unwrap_or(10);

        error.set(String::new());

        spawn(async move {
            match crate::api::add_column(&project_id, &id, &name, &description, order).await {
                Ok(()) => {
                    new_id.set(String::new());
                    new_name.set(String::new());
                    new_description.set(String::new());
                    on_saved.call(());
                }
                Err(err) => error.set(err.message),
            }
        });
    };

    let error_text = error.read().clone();
    let mut ordered = project.columns.clone();
    ordered.sort_by_key(|itm| itm.order);

    rsx! {
        div { class: "card",
            div { class: "card-title", "Columns" }
            div { class: "field-hint",
                "Todo and Done exist in every project and are not listed here. A column id is typed in once and never renamed — tasks point at it as their status. Deleting a column leaves its tasks alone: they read as Todo until a column with that id exists again."
            }
            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }

            div { class: "table-responsive", style: "margin-top: 12px",
                table { class: "table",
                    thead {
                        tr {
                            th { "Order" }
                            th { "Id" }
                            th { "Name" }
                            th { "Description" }
                            th { "" }
                        }
                    }
                    tbody {
                        tr {
                            td { class: "muted", "first" }
                            td { class: "mono", "todo" }
                            td { "Todo" }
                            td { class: "muted", "Always present" }
                            td { }
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
                            td { }
                        }
                    }
                }
            }

            div { class: "form-row-inline", style: "margin-top: 14px",
                div { class: "form-row",
                    label { "Order" }
                    input {
                        r#type: "text",
                        style: "width: 70px",
                        value: "{new_order}",
                        oninput: move |event| new_order.set(event.value()),
                    }
                }
                div { class: "form-row",
                    label { "Id" }
                    input {
                        r#type: "text",
                        value: "{new_id}",
                        placeholder: "in-progress",
                        oninput: move |event| new_id.set(event.value().to_lowercase()),
                    }
                }
                div { class: "form-row",
                    label { "Name" }
                    input {
                        r#type: "text",
                        value: "{new_name}",
                        oninput: move |event| new_name.set(event.value()),
                    }
                }
                div { class: "form-row", style: "flex: 1 1 220px",
                    label { "Description" }
                    input {
                        r#type: "text",
                        value: "{new_description}",
                        oninput: move |event| new_description.set(event.value()),
                    }
                }
                button {
                    class: "btn btn-primary",
                    disabled: new_id.read().trim().is_empty(),
                    onclick: add,
                    "Add column"
                }
            }
        }
    }
}

#[component]
fn RenderColumnRow(
    project_id: String,
    column: task_manager_shared::projects::ProjectColumnResponse,
    on_saved: EventHandler<()>,
) -> Element {
    let mut name = use_signal(|| column.name.clone());
    let mut description = use_signal(|| column.description.clone());
    let mut order = use_signal(|| column.order.to_string());

    let save_project_id = project_id.clone();
    let save_column_id = column.id.clone();

    let save = move |_| {
        let (project_id, column_id) = (save_project_id.clone(), save_column_id.clone());
        let (name_value, description_value) = (name.read().clone(), description.read().clone());
        let order_value = order.read().trim().parse::<i32>().unwrap_or(column.order);

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

    let delete_project_id = project_id.clone();
    let delete_column_id = column.id.clone();

    let delete = move |_| {
        let (project_id, column_id) = (delete_project_id.clone(), delete_column_id.clone());

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

#[component]
fn RenderKinds(project: ProjectResponse, on_saved: EventHandler<()>) -> Element {
    let mut new_id = use_signal(String::new);
    let mut new_name = use_signal(String::new);
    let mut new_description = use_signal(String::new);
    let mut new_color = use_signal(|| KindColor::default().as_str().to_string());
    let mut error = use_signal(String::new);

    let project_id = project.id.clone();

    let add = move |_| {
        let project_id = project_id.clone();
        let (id, name, description, color) = (
            new_id.read().clone(),
            new_name.read().clone(),
            new_description.read().clone(),
            new_color.read().clone(),
        );

        error.set(String::new());

        spawn(async move {
            match crate::api::add_kind(&project_id, &id, &name, &description, &color).await {
                Ok(()) => {
                    new_id.set(String::new());
                    new_name.set(String::new());
                    new_description.set(String::new());
                    on_saved.call(());
                }
                Err(err) => error.set(err.message),
            }
        });
    };

    let error_text = error.read().clone();

    rsx! {
        div { class: "card",
            div { class: "card-title", "Kinds" }
            div { class: "field-hint",
                "A kind is optional on a task. The description is what an agent reads before classifying one, so write the rule for applying it rather than a synonym of the name."
            }
            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }

            div { class: "table-responsive", style: "margin-top: 12px",
                table { class: "table",
                    thead {
                        tr {
                            th { "Id" }
                            th { "Name" }
                            th { "Description" }
                            th { "Colour" }
                            th { "" }
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

            div { class: "form-row-inline", style: "margin-top: 14px",
                div { class: "form-row",
                    label { "Id" }
                    input {
                        r#type: "text",
                        value: "{new_id}",
                        placeholder: "bug",
                        oninput: move |event| new_id.set(event.value().to_lowercase()),
                    }
                }
                div { class: "form-row",
                    label { "Name" }
                    input {
                        r#type: "text",
                        value: "{new_name}",
                        oninput: move |event| new_name.set(event.value()),
                    }
                }
                div { class: "form-row", style: "flex: 1 1 220px",
                    label { "Description" }
                    input {
                        r#type: "text",
                        value: "{new_description}",
                        oninput: move |event| new_description.set(event.value()),
                    }
                }
                div { class: "form-row",
                    label { "Colour" }
                    RenderColorPicker { value: new_color.read().clone(), on_pick: move |value| new_color.set(value) }
                }
                button {
                    class: "btn btn-primary",
                    disabled: new_id.read().trim().is_empty(),
                    onclick: add,
                    "Add kind"
                }
            }
        }
    }
}

#[component]
fn RenderKindRow(
    project_id: String,
    kind: task_manager_shared::projects::ProjectKindResponse,
    on_saved: EventHandler<()>,
) -> Element {
    let mut name = use_signal(|| kind.name.clone());
    let mut description = use_signal(|| kind.description.clone());
    let mut color = use_signal(|| kind.color.clone());

    let save_project_id = project_id.clone();
    let save_kind_id = kind.id.clone();

    let save = move |_| {
        let (project_id, kind_id) = (save_project_id.clone(), save_kind_id.clone());
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

    let delete_project_id = project_id.clone();
    let delete_kind_id = kind.id.clone();

    let delete = move |_| {
        let (project_id, kind_id) = (delete_project_id.clone(), delete_kind_id.clone());

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

#[component]
fn RenderMembers(
    project: ProjectResponse,
    users: Vec<UserResponse>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut chosen = use_signal(|| project.members.clone());
    let mut error = use_signal(String::new);

    let project_id = project.id.clone();

    let save = move |_| {
        let project_id = project_id.clone();
        let members = chosen.read().clone();

        error.set(String::new());

        spawn(async move {
            match crate::api::set_members(&project_id, members).await {
                Ok(()) => on_saved.call(()),
                Err(err) => error.set(err.message),
            }
        });
    };

    let chosen_ra = chosen.read().clone();
    let error_text = error.read().clone();

    rsx! {
        div { class: "card",
            div { class: "card-title", "Who may see this project" }
            div { class: "field-hint",
                "Admins see every project without being listed here. Somebody on no project at all sees an empty Home rather than being locked out."
            }
            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }

            if users.is_empty() {
                div { class: "empty-note", style: "margin-top: 12px",
                    "Nobody is on the roster yet — add people on the Users screen first."
                }
            } else {
                div { style: "margin-top: 12px",
                    for user in users.iter() {
                        div { class: "checkbox-row", key: "{user.email}",
                            input {
                                r#type: "checkbox",
                                id: "member-{user.email}",
                                checked: chosen_ra.contains(&user.email),
                                onchange: {
                                    let email = user.email.clone();
                                    move |event: Event<FormData>| {
                                        let mut next = chosen.read().clone();
                                        if event.checked() {
                                            if !next.contains(&email) {
                                                next.push(email.clone());
                                            }
                                        } else {
                                            next.retain(|itm| itm != &email);
                                        }
                                        chosen.set(next);
                                    }
                                },
                            }
                            label { r#for: "member-{user.email}",
                                if user.name.trim().is_empty() {
                                    "{user.email}"
                                } else {
                                    "{user.name} · {user.email}"
                                }
                                if user.disabled {
                                    span { class: "tag", style: "margin-left: 6px", "disabled" }
                                }
                            }
                        }
                    }
                }
                button { class: "btn btn-primary", style: "margin-top: 8px", onclick: save, "Save members" }
            }
        }
    }
}
