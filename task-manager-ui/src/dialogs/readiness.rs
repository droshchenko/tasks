use dioxus::prelude::*;
use task_manager_shared::readiness::TaskReadiness;

#[component]
pub fn ReadinessDetails(readiness: TaskReadiness) -> Element {
    rsx! {
        if !readiness.dependencies.is_empty() {
            section { class: "readiness-section",
                h3 { "Blocked by" }
                ul { class: "readiness-list",
                    for item in readiness.dependencies {
                        li { key: "{item.task_id}",
                            button { class: "task-view-link", onclick: {
                                let id = item.task_id.clone();
                                move |_| super::view_task::show(id.clone())
                            }, "{item.task_id}" }
                            if !item.title.is_empty() { span { " · {item.title}" } }
                            if let Some(status) = item.status {
                                div { class: "field-hint", "Current status: {status}. Complete this dependency to unblock the task." }
                                if let Some(assignee) = item.assignee { div { class: "field-hint", "Assigned to {assignee}" } }
                            } else {
                                div { class: "field-hint", "Missing or deleted. Restore this task or correct the dependency reference." }
                            }
                        }
                    }
                }
            }
        }
        if readiness.required_decisions > 0 || readiness.analysis_required {
            section { class: "readiness-section",
                h3 { "Before completion" }
                if readiness.required_decisions > 0 {
                    p { "Awaiting {readiness.required_decisions} required human answer(s). Open the task's Human decisions section to respond." }
                }
                if readiness.analysis_required {
                    p { "Attach the analysis result files required by this stage or task type before its transition." }
                }
            }
        }
    }
}
