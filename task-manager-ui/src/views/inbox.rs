use dioxus::prelude::*;
use std::collections::BTreeMap;
use task_manager_shared::decisions::PendingDecision;

#[derive(Default)]
struct InboxState {
    items: Vec<PendingDecision>,
    loading: bool,
    loaded: bool,
    error: String,
    answered: std::collections::BTreeSet<(String, String)>,
}

async fn refresh(mut state: Signal<InboxState>) {
    if state.peek().loading {
        return;
    }
    state.write().loading = true;
    let result = crate::api::get_pending_decisions().await;
    let mut state = state.write();
    state.loading = false;
    match result {
        Ok(response) => {
            state.items = response
                .items
                .into_iter()
                .filter(|item| {
                    !state
                        .answered
                        .contains(&(item.task_id.clone(), item.decision.id.clone()))
                })
                .collect();
            state.loaded = true;
            state.error.clear();
        }
        Err(error) => state.error = error.message,
    }
}

#[component]
pub fn RenderInbox() -> Element {
    let mut state = use_signal(InboxState::default);
    let mut project = use_signal(String::new);
    let mut required_only = use_signal(|| false);
    use_future(move || async move {
        loop {
            let hidden = web_sys::window()
                .and_then(|window| window.document())
                .is_some_and(|document| document.hidden());
            if !hidden {
                refresh(state).await;
            }
            dioxus_utils::js::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
    let data = state.read();
    let projects: BTreeMap<String, String> = data
        .items
        .iter()
        .map(|item| (item.project.clone(), item.project_name.clone()))
        .collect();
    let selected = project.read().clone();
    let required = *required_only.read();
    let visible: Vec<_> = data
        .items
        .iter()
        .filter(|item| {
            (selected.is_empty() || item.project == selected)
                && (!required || item.decision.required)
        })
        .cloned()
        .collect();
    let required_count = data
        .items
        .iter()
        .filter(|item| item.decision.required)
        .count();
    rsx! {
        div { class: "inbox-page",
            div { class: "page-header",
                div {
                    h1 { class: "page-title", "Awaiting your answer" }
                    p { class: "page-note", "{data.items.len()} questions · {required_count} required for completion. Refreshed every 30 seconds." }
                }
                button { class: "btn", disabled: data.loading, onclick: move |_| { spawn(refresh(state)); },
                    if data.loading { "Refreshing…" } else { "Refresh" }
                }
            }
            div { class: "inbox-filters",
                label { r#for: "inbox-project", "Project" }
                select { id: "inbox-project", value: "{selected}", onchange: move |event| project.set(event.value()),
                    option { value: "", "All projects" }
                    if !selected.is_empty() && !projects.contains_key(&selected) { option { value: "{selected}", "{selected}" } }
                    for (prefix, name) in projects { option { value: "{prefix}", "{name}" } }
                }
                label { input { r#type: "checkbox", checked: required, onchange: move |event| required_only.set(event.checked()) } "Required only" }
            }
            if !data.error.is_empty() { div { class: "error-banner", role: "alert", "Could not refresh: {data.error}. Showing the last loaded questions." } }
            if visible.is_empty() && data.loaded {
                div { class: "empty-note", if selected.is_empty() && !required { "No questions waiting for you." } else { "No questions match these filters." } }
            }
            div { class: "inbox-list",
                for item in visible {
                    article { class: "inbox-item", key: "{item.task_id}:{item.decision.id}",
                        div { class: "inbox-task-heading",
                            span { class: "tag", "{item.project_name}" }
                            button { class: "task-view-link", onclick: {
                                let id = item.task_id.clone();
                                move |_| crate::dialogs::show_task(id.clone())
                            }, "{item.task_id} · {item.task_title}" }
                        }
                        crate::dialogs::TaskDecisionCard {
                            task_id: item.task_id.clone(), decision: item.decision.clone(),
                            on_answered: {
                                let id = item.task_id.clone();
                                let question = item.decision.id.clone();
                                move |_| {
                                    let mut state = state.write();
                                    state.answered.insert((id.clone(), question.clone()));
                                    state.items.retain(|item| item.task_id != id || item.decision.id != question);
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}
