use dioxus::prelude::*;
use task_manager_shared::users::UserResponse;

/// The roster.
///
/// Being here is what allows a sign-in at all; it grants no project on its own — that is set per project
/// in Projects setup. An email is the identity and is never edited: every task assignment and every
/// comment author points at it, so a changed address is a new person rather than a rename.
#[component]
pub fn RenderUsers() -> Element {
    let mut users = use_signal(Vec::<UserResponse>::new);
    let mut allowed = use_signal(|| true);
    let mut loading = use_signal(|| true);
    let mut error = use_signal(String::new);
    let mut revision = use_signal(|| 0_u32);

    // `use_resource`, not `use_future`: the latter spawns once and tracks nothing, so bumping `revision`
    // after a write refreshed nothing at all.
    use_resource(move || {
        let _revision = *revision.read();

        async move {
            match crate::api::get_users().await {
                Ok(Some(response)) => {
                    users.set(response.users);
                    allowed.set(true);
                    loading.set(false);
                }
                Ok(None) => {
                    allowed.set(false);
                    loading.set(false);
                }
                Err(err) => {
                    error.set(err.message);
                    loading.set(false);
                }
            }
        }
    });

    if *loading.read() {
        return rsx! {
            div { class: "loading-note", "Loading…" }
        };
    }

    if !*allowed.read() {
        return rsx! {
            div { class: "page-header",
                h1 { class: "page-title", "Users" }
            }
            div { class: "empty-note", "Admins only." }
        };
    }

    let users_ra = users.read().clone();
    let error_text = error.read().clone();

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Users" }
        }

        if !error_text.is_empty() {
            div { class: "error-banner", "{error_text}" }
        }

        RenderCreateUser { on_saved: move |_| revision += 1 }

        div { class: "card",
            div { class: "card-title", "Roster" }
            if users_ra.is_empty() {
                div { class: "empty-note", "Nobody yet." }
            } else {
                div { class: "table-responsive",
                    table { class: "table",
                        thead {
                            tr {
                                th { "Email" }
                                th { "Name" }
                                th { "Admin" }
                                th { "Disabled" }
                                th { "" }
                            }
                        }
                        tbody {
                            for user in users_ra.iter() {
                                RenderUserRow {
                                    key: "{user.email}",
                                    user: user.clone(),
                                    on_saved: move |_| revision += 1,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RenderCreateUser(on_saved: EventHandler<()>) -> Element {
    let mut email = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut admin = use_signal(|| false);
    let mut error = use_signal(String::new);

    let submit = move |_| {
        let (email_value, name_value, admin_value) =
            (email.read().clone(), name.read().clone(), *admin.read());

        error.set(String::new());

        spawn(async move {
            match crate::api::create_user(&email_value, &name_value, admin_value).await {
                Ok(()) => {
                    email.set(String::new());
                    name.set(String::new());
                    admin.set(false);
                    on_saved.call(());
                }
                Err(err) => error.set(err.message),
            }
        });
    };

    let error_text = error.read().clone();
    let can_save = email.read().contains('@');

    rsx! {
        div { class: "card",
            div { class: "card-title", "Add someone" }
            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }
            div { class: "form-row-inline",
                div { class: "form-row", style: "flex: 1 1 240px",
                    label { "Google account email" }
                    input {
                        r#type: "text",
                        value: "{email}",
                        oninput: move |event| email.set(event.value().to_lowercase()),
                    }
                }
                div { class: "form-row", style: "flex: 1 1 180px",
                    label { "Name shown on the board" }
                    input {
                        r#type: "text",
                        value: "{name}",
                        oninput: move |event| name.set(event.value()),
                    }
                }
                div { class: "checkbox-row",
                    input {
                        r#type: "checkbox",
                        id: "new-user-admin",
                        checked: *admin.read(),
                        onchange: move |event| admin.set(event.checked()),
                    }
                    label { r#for: "new-user-admin", "Admin" }
                }
                button {
                    class: "btn btn-primary",
                    disabled: !can_save,
                    onclick: submit,
                    "Add"
                }
            }
            div { class: "field-hint",
                "It must be the Google account they will sign in with. An admin sees every project and may configure the product."
            }
        }
    }
}

#[component]
fn RenderUserRow(user: UserResponse, on_saved: EventHandler<()>) -> Element {
    let mut name = use_signal(|| user.name.clone());
    let mut admin = use_signal(|| user.admin);
    let mut disabled = use_signal(|| user.disabled);

    let email = user.email.clone();

    let save = move |_| {
        let email = email.clone();
        let (name_value, admin_value, disabled_value) =
            (name.read().clone(), *admin.read(), *disabled.read());

        spawn(async move {
            let _ = crate::api::update_user(&email, &name_value, admin_value, disabled_value).await;
            on_saved.call(());
        });
    };

    // An admin by settings has no checkbox to untick — showing one that silently does nothing would be
    // worse than showing none, so it reads as granted-elsewhere instead.
    let admin_from_settings = user.admin_from_settings;

    rsx! {
        tr {
            td { class: "mono", "{user.email}" }
            td {
                input {
                    r#type: "text",
                    value: "{name}",
                    oninput: move |event| name.set(event.value()),
                }
            }
            td {
                if admin_from_settings {
                    span { class: "tag", title: "Granted by the service settings, not by this row", "from settings" }
                } else {
                    input {
                        r#type: "checkbox",
                        checked: *admin.read(),
                        onchange: move |event| admin.set(event.checked()),
                    }
                }
            }
            td {
                input {
                    r#type: "checkbox",
                    checked: *disabled.read(),
                    onchange: move |event| disabled.set(event.checked()),
                }
            }
            td {
                button { class: "btn btn-sm", onclick: save, "Save" }
            }
        }
    }
}
