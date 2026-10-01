use dioxus::prelude::*;
use std::time::Duration;
use task_manager_shared::ai_settings::*;

#[component]
pub fn KnowledgePanel() -> Element {
    let app = consume_context::<Signal<crate::states::AppState>>();
    let mut selected = use_signal(|| crate::web::storage::get_last_project().unwrap_or_default());
    let mut projects = use_resource(move || async move {
        let response = crate::api::get_projects().await?;
        if !response
            .projects
            .iter()
            .any(|p| p.prefix == *selected.peek())
        {
            selected.set(
                response
                    .projects
                    .iter()
                    .find(|p| !p.archived)
                    .or_else(|| response.projects.first())
                    .map(|p| p.prefix.clone())
                    .unwrap_or_default(),
            );
        }
        Ok::<_, crate::models::RequestError>(response)
    });
    if !app.read().is_admin() {
        return rsx! { div { class: "empty-note", "Admins only." } };
    }
    let items = match &*projects.read() {
        Some(Ok(response)) => response.projects.clone(),
        Some(Err(error)) => {
            return rsx! { div { class: "error-banner", "{error.message}" } button { class: "btn", onclick: move |_| projects.restart(), "Retry" } };
        }
        None => return rsx! { div { class: "loading-note", "Loading projects…" } },
    };
    let prefix = selected.read().clone();
    rsx! {
        div { class: "ai-section-heading",
            div { h2 { "Knowledge & indexing" } p { class: "page-note", "Choose the sources agents can retrieve for each project, and manage its task embeddings." } }
            div { class: "ai-project-choice",
                label { r#for: "knowledge-project", "Project" }
                select { id: "knowledge-project", onchange: move |event| selected.set(event.value()),
                    for project in &items {
                        option { value: "{project.prefix}", selected: prefix == project.prefix, "{project.prefix} · {project.name}" }
                    }
                }
            }
        }
        if prefix.is_empty() { div { class: "empty-note", "Create a project in Projects setup first." } }
        else { ProjectKnowledge { key: "{prefix}", project: prefix } }
    }
}

#[component]
fn ProjectKnowledge(project: String) -> Element {
    let load_project = project.clone();
    let mut data = use_resource(move || {
        let project = load_project.clone();
        async move { crate::api::get_knowledge_settings(project).await }
    });
    let initial = match &*data.read() {
        Some(Ok(response)) => response.clone(),
        Some(Err(error)) => {
            return rsx! { div { class: "error-banner", "{error.message}" } button { class: "btn", onclick: move |_| data.restart(), "Retry" } };
        }
        None => return rsx! { div { class: "loading-note", "Loading project knowledge…" } },
    };
    rsx! {
        div { class: "ai-knowledge-layout",
            KnowledgeEditor { key: "sources-{initial.revision}", initial }
            div { class: "ai-knowledge-aside", TaskIndex { project } }
        }
    }
}

#[component]
fn KnowledgeEditor(initial: KnowledgeSettingsResponse) -> Element {
    let mut saved = use_signal(|| initial.clone());
    let mut draft = use_signal(|| initial.config.clone());
    let mut busy = use_signal(|| false);
    let mut feedback = use_signal(String::new);
    let mut failed = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut preview = use_signal(|| None::<KnowledgePreviewResponse>);
    let current = saved.read().clone();
    let values = draft.read().clone();
    let changed = values != current.config;
    let unavailable = *busy.read();
    let wiki_paths = values.wiki_paths.join("\n");
    let prefix = current.project.clone();
    let show_upload = current.config.graph_source == "upload" && !changed;
    let on_save = move |_| {
        if *busy.peek() {
            return;
        }
        let current = saved.peek().clone();
        let config = draft.peek().clone();
        busy.set(true);
        feedback.set(String::new());
        preview.set(None);
        spawn(async move {
            match crate::api::save_knowledge_settings(current.project, current.revision, config)
                .await
            {
                Ok(result) => {
                    draft.set(result.config.clone());
                    saved.set(result);
                    failed.set(false);
                    feedback.set("Sources saved. Refresh the index now, or let automatic refresh pick up the change.".into());
                }
                Err(error) => {
                    failed.set(true);
                    feedback.set(error.message);
                }
            }
            busy.set(false);
        });
    };
    let on_refresh = move |_| {
        if *busy.peek() {
            return;
        }
        let project = saved.peek().project.clone();
        busy.set(true);
        feedback.set(String::new());
        preview.set(None);
        spawn(async move {
            match crate::api::refresh_knowledge(project).await {
                Ok(result) => {
                    draft.set(result.config.clone());
                    saved.set(result);
                    failed.set(false);
                    feedback.set("Knowledge sources refreshed.".into());
                }
                Err(error) => {
                    failed.set(true);
                    feedback.set(error.message);
                }
            }
            busy.set(false);
        });
    };
    let on_reload = move |_| {
        if *busy.peek() {
            return;
        }
        let project = saved.peek().project.clone();
        busy.set(true);
        feedback.set(String::new());
        preview.set(None);
        spawn(async move {
            match crate::api::get_knowledge_settings(project).await {
                Ok(result) => {
                    draft.set(result.config.clone());
                    saved.set(result);
                    failed.set(false);
                }
                Err(error) => {
                    failed.set(true);
                    feedback.set(error.message);
                }
            }
            busy.set(false);
        });
    };
    let on_preview = move |_| {
        if *busy.peek() {
            return;
        }
        let project = saved.peek().project.clone();
        let text = query.peek().clone();
        busy.set(true);
        feedback.set(String::new());
        spawn(async move {
            match crate::api::preview_knowledge(project, text).await {
                Ok(result) => {
                    preview.set(Some(result));
                    failed.set(false);
                }
                Err(error) => {
                    failed.set(true);
                    feedback.set(error.message);
                }
            }
            busy.set(false);
        });
    };
    rsx! {
        section { class: "card ai-provider-card", aria_label: "Project knowledge sources",
            div { class: "ai-card-heading", h3 { "Project sources" } span { class: "tag", "{current.status.state}" } }
            p { class: "field-hint", "Wiki passages and code relationships are added to task preparation, context search and Jev reviews for {prefix} only." }
            div { class: "checkbox-row",
                input { id: "knowledge-enabled", r#type: "checkbox", checked: values.enabled, disabled: unavailable,
                    onchange: move |event| draft.write().enabled = event.checked() }
                label { r#for: "knowledge-enabled", "Use project knowledge" }
            }
            div { class: "form-row",
                label { r#for: "knowledge-connection", "Connected repository" }
                select { id: "knowledge-connection", disabled: unavailable, onchange: move |event| draft.write().connection = event.value(),
                    option { value: "", selected: values.connection.is_empty(), "Choose repository" }
                    for connection in &current.connections {
                        option { value: "{connection.name}", selected: connection.name == values.connection,
                            "{connection.name} · {connection.repository} · {connection.branch}" }
                    }
                }
                if current.connections.is_empty() {
                    div { class: "field-hint", "Connect a repository in " a { href: "/projects-setup", "Projects setup" } " first." }
                }
                if let Some(connection) = current.connections.iter().find(|c| c.name == values.connection) {
                    div { class: "ai-source-meta", "Repository: {connection.state} · {connection.commit}" }
                    div { class: "ai-source-meta", "Connected folder: /{connection.root_path}" }
                }
            }
            div { class: "form-row",
                label { r#for: "knowledge-wiki-paths", "Wiki folders or files" }
                textarea { id: "knowledge-wiki-paths", rows: "4", value: "{wiki_paths}", disabled: unavailable,
                    placeholder: "docs/dev-wiki/wiki\ndocs/dev-wiki/topics\ndocs/decisions",
                    oninput: move |event| draft.write().wiki_paths = event.value().split('\n').map(str::to_string).collect() }
                div { class: "field-hint", "One path per line, relative to the connected repository folder. Markdown and text files are supported." }
            }
            div { class: "form-row",
                label { r#for: "knowledge-graph-source", "Graphify source" }
                select { id: "knowledge-graph-source", disabled: unavailable, onchange: move |event| draft.write().graph_source = event.value(),
                    option { value: "none", selected: values.graph_source == "none", "No code graph" }
                    option { value: "repository", selected: values.graph_source == "repository", "JSON file in repository" }
                    option { value: "upload", selected: values.graph_source == "upload", "Uploaded JSON export" }
                }
            }
            if values.graph_source == "repository" {
                div { class: "form-row", label { r#for: "knowledge-graph-path", "Graph JSON path" }
                    input { id: "knowledge-graph-path", value: "{values.graph_path}", disabled: unavailable, placeholder: "artifacts/graphify/graph.json",
                        oninput: move |event| draft.write().graph_path = event.value() }
                }
            }
            div { class: "checkbox-row",
                input { id: "knowledge-auto-refresh", r#type: "checkbox", checked: values.auto_refresh, disabled: unavailable,
                    onchange: move |event| draft.write().auto_refresh = event.checked() }
                label { r#for: "knowledge-auto-refresh", "Refresh automatically when the connected repository changes" }
            }
            div { class: "ai-card-actions",
                button { class: "btn btn-primary", disabled: unavailable || !changed, onclick: on_save, "Save sources" }
                button { class: "btn", disabled: unavailable || changed || !current.config.enabled, onclick: on_refresh, "Refresh sources" }
                button { class: "btn btn-sm", disabled: unavailable, onclick: on_reload, "Reload" }
            }
            if unavailable { p { class: "field-hint", role: "status", "Working…" } }
            if !feedback.read().is_empty() { div { class: if *failed.read() { "error-banner" } else { "ai-success" }, role: "status", "{feedback}" } }
            if show_upload {
                div { class: "form-row",
                    label { r#for: "knowledge-graph-upload", "Upload Graphify export" }
                    input { id: "knowledge-graph-upload", r#type: "file", accept: ".json,application/json", disabled: unavailable,
                        onchange: move |event| {
                            let Some(file) = event.files().into_iter().next() else { return; };
                            let current = saved.peek().clone(); busy.set(true); feedback.set(String::new()); preview.set(None);
                            spawn(async move {
                                let result = match file.read_bytes().await {
                                    Ok(bytes) if bytes.len() <= 64 * 1024 * 1024 => crate::api::upload_knowledge_graph(current.project, current.revision, bytes.to_vec()).await,
                                    Ok(_) => Err(crate::models::RequestError { message: "Graphify export must be at most 64 MiB.".into() }),
                                    Err(_) => Err(crate::models::RequestError { message: "The selected file could not be read.".into() }),
                                };
                                match result {
                                    Ok(result) => { saved.set(result); failed.set(false); feedback.set("Graph export saved. Check source status and commit below.".into()); }
                                    Err(error) => { failed.set(true); feedback.set(error.message); }
                                }
                                busy.set(false);
                            });
                        }
                    }
                    div { class: "field-hint", "Choose graph.json from Graphify's export folder (up to 64 MiB). Its built_at_commit must match the connected repository before graph evidence is used." }
                }
            }
            div { class: "ai-index-stats",
                div { class: "ai-index-stat", strong { "{current.status.wiki_documents}" } span { "Wiki documents" } }
                div { class: "ai-index-stat", strong { "{current.status.graph_nodes}" } span { "Graph symbols" } }
                div { class: "ai-index-stat", strong { "{current.status.graph_edges}" } span { "Graph relationships" } }
            }
            if !current.status.source_commit.is_empty() { p { class: "ai-source-meta", "Source commit: {current.status.source_commit}" } }
            if !current.status.graph_commit.is_empty() { p { class: "ai-source-meta", "Graph commit: {current.status.graph_commit}" } }
            if let Some(updated) = current.status.updated_unix_seconds { p { class: "ai-source-meta", "Last indexed: {display_time(updated)}" } }
            if !current.status.notice.is_empty() { p { class: "field-hint", role: "status", "{current.status.notice}" } }
            div { class: "form-row",
                label { r#for: "knowledge-preview", "Try a knowledge query" }
                input { id: "knowledge-preview", value: "{query}", disabled: unavailable, placeholder: "A topic or code symbol", oninput: move |event| query.set(event.value()) }
                button { class: "btn", disabled: unavailable || changed || !current.config.enabled || query.read().trim().is_empty(), onclick: on_preview, "Preview context" }
            }
            if let Some(result) = preview.read().as_ref() {
                if !result.notice.is_empty() { p { class: "field-hint", "{result.notice}" } }
                if result.hits.is_empty() { p { class: "empty-note", "No matching knowledge found." } }
                for hit in &result.hits {
                    article { class: "ai-preview-hit",
                        h4 { "{hit.title}" } span { class: "tag", "{hit.source}" }
                        p { "{hit.excerpt}" }
                        crate::dialogs::DocumentRefs { project: prefix.clone(), ids: vec![hit.reference.clone()] }
                        div { class: "ai-source-meta", "Commit: {hit.commit}" }
                    }
                }
            }
        }
    }
}

fn display_time(seconds: i64) -> String {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(seconds as f64 * 1000.0))
        .to_locale_string("en-GB", &wasm_bindgen::JsValue::UNDEFINED)
        .as_string()
        .unwrap_or_default()
}

#[component]
fn TaskIndex(project: String) -> Element {
    let mut data = use_resource(move || {
        let project = project.clone();
        async move { crate::api::get_index_status(project).await }
    });
    let initial = match &*data.read() {
        Some(Ok(response)) => response.clone(),
        Some(Err(error)) => {
            return rsx! { div { class: "error-banner", "{error.message}" } button { class: "btn", onclick: move |_| data.restart(), "Retry index status" } };
        }
        None => return rsx! { div { class: "loading-note", "Loading index status…" } },
    };
    rsx! { TaskIndexControls { initial } }
}

#[component]
fn TaskIndexControls(initial: IndexStatusResponse) -> Element {
    let mut current = use_signal(|| initial.clone());
    let mut generation = use_signal(|| 0_u64);
    let mut force = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(String::new);
    use_future(move || async move {
        loop {
            dioxus_utils::js::sleep(Duration::from_secs(3)).await;
            let state = current.peek().clone();
            if !matches!(state.job.state.as_str(), "running" | "cancelling") || *busy.peek() {
                continue;
            }
            let expected = *generation.peek();
            match crate::api::get_index_status(state.project).await {
                Ok(status) if *generation.peek() == expected => {
                    current.set(status);
                    error.set(String::new());
                }
                Err(failure) if *generation.peek() == expected => error.set(failure.message),
                _ => {}
            }
        }
    });
    let on_start = move |_| {
        if *busy.peek() {
            return;
        }
        let project = current.peek().project.clone();
        let rebuild = *force.peek();
        busy.set(true);
        *generation.write() += 1;
        error.set(String::new());
        spawn(async move {
            match crate::api::start_task_index(project, rebuild).await {
                Ok(status) => current.set(status),
                Err(failure) => error.set(failure.message),
            }
            busy.set(false);
        });
    };
    let on_cancel = move |_| {
        if *busy.peek() {
            return;
        }
        let project = current.peek().project.clone();
        busy.set(true);
        *generation.write() += 1;
        spawn(async move {
            match crate::api::cancel_task_index(project).await {
                Ok(status) => current.set(status),
                Err(failure) => error.set(failure.message),
            }
            busy.set(false);
        });
    };
    let on_refresh = move |_| {
        if *busy.peek() {
            return;
        }
        let project = current.peek().project.clone();
        busy.set(true);
        *generation.write() += 1;
        spawn(async move {
            match crate::api::get_index_status(project).await {
                Ok(status) => {
                    current.set(status);
                    error.set(String::new());
                }
                Err(failure) => error.set(failure.message),
            }
            busy.set(false);
        });
    };
    let status = current.read().clone();
    let running = matches!(status.job.state.as_str(), "running" | "cancelling");
    rsx! {
        section { class: "card ai-provider-card", aria_label: "Task embedding index",
            div { class: "ai-card-heading", h3 { "Task embeddings" } span { class: "tag", "{status.job.state}" } }
            p { class: "field-hint", "Build vectors for tasks, recent comments and human decisions in this project. Indexing sends that text to the configured embeddings provider." }
            if !status.configured {
                p { class: "field-hint", "Configure and enable embeddings in " Link { to: crate::AppRoute::SettingsSection { section: "ai-providers".into() }, "AI providers" } " first. Text search remains available." }
            }
            if !status.model.is_empty() { p { class: "ai-source-meta", "Model: {status.model}" } }
            div { class: "ai-index-stats",
                div { class: "ai-index-stat", strong { "{status.vectors_current}" } span { "Current vectors" } }
                div { class: "ai-index-stat", strong { "{status.vectors_missing}" } span { "Need indexing" } }
            }
            div { class: "checkbox-row",
                input { id: "task-index-force", r#type: "checkbox", checked: *force.read(), disabled: running || *busy.read(), onchange: move |event| force.set(event.checked()) }
                label { r#for: "task-index-force", "Rebuild all vectors" }
            }
            p { class: "field-hint", "Rebuild after a provider changes the version behind a model alias. Provider usage charges may apply." }
            div { class: "ai-card-actions",
                button { class: "btn btn-primary", disabled: *busy.read() || running || !status.configured || status.tasks_total == 0 || (status.vectors_missing == 0 && !*force.read()), onclick: on_start, "Start indexing" }
                if running { button { class: "btn", disabled: *busy.read() || status.job.state == "cancelling", onclick: on_cancel, "Cancel indexing" } }
                button { class: "btn btn-sm", disabled: *busy.read(), onclick: on_refresh, "Refresh status" }
            }
            if status.job.started_unix_seconds.is_some() {
                p { "{status.job.processed} indexed · {status.job.remaining} remaining" }
                progress { value: status.job.processed, max: status.job.total.max(1) }
            }
            if !status.job.message.is_empty() { p { class: "field-hint", role: "status", "{status.job.message}" } }
            if !status.notice.is_empty() { p { class: "field-hint", "{status.notice}" } }
            if !error.read().is_empty() { div { class: "error-banner", role: "status", "{error}" } }
        }
    }
}
