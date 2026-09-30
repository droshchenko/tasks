use dioxus::prelude::*;
use task_manager_shared::decisions::TaskDecision;

#[derive(Default, Clone, PartialEq)]
struct AnswerDraft {
    option: String,
    text: String,
    saving: bool,
    error: String,
}

#[component]
pub fn TaskDecisionHistory(task_id: String, decisions: Vec<TaskDecision>) -> Element {
    if decisions.is_empty() {
        return rsx! {};
    }
    rsx! {
        section { class: "task-decision-history",
            h3 { "Human decisions" }
            for decision in decisions {
                TaskDecisionCard { key: "{decision.id}", task_id: task_id.clone(), decision }
            }
        }
    }
}

#[component]
fn TaskDecisionCard(task_id: String, decision: TaskDecision) -> Element {
    let mut draft = use_signal(AnswerDraft::default);
    let selection = draft.read().option.clone();
    let text = draft.read().text.clone();
    let saving = draft.read().saving;
    let error = draft.read().error.clone();
    let can_answer = !saving && (!selection.is_empty() || !text.trim().is_empty());
    let question_id = decision.id.clone();
    let action_task = task_id.clone();
    let answer = move |_| {
        let state = draft.read().clone();
        if state.saving || (state.option.is_empty() && state.text.trim().is_empty()) {
            return;
        }
        let task_id = action_task.clone();
        let decision_id = question_id.clone();
        draft.write().saving = true;
        draft.write().error.clear();
        spawn(async move {
            let option_id = if state.option.is_empty() {
                None
            } else {
                Some(state.option)
            };
            match crate::api::answer_task_decision(&task_id, &decision_id, option_id, state.text)
                .await
            {
                Err(error) => {
                    draft.write().saving = false;
                    draft.write().error = error.message;
                }
                Ok(()) => match crate::api::find_task(&task_id).await {
                    Ok(found) => super::open(super::DialogState::ViewTask { found }),
                    Err(error) => {
                        draft.write().saving = false;
                        draft.write().error = format!(
                            "Answer saved; the task could not be refreshed: {}",
                            error.message
                        );
                    }
                },
            }
        });
    };
    let chosen = decision
        .answer
        .as_ref()
        .and_then(|answer| answer.option_id.as_deref())
        .and_then(|id| decision.options.iter().find(|option| option.id == id))
        .map(|option| option.label.clone());
    rsx! {
        article { class: "task-decision-card",
            div { class: "task-decision-meta",
                span { class: "tag", "{decision.status}" }
                if decision.required { span { class: "tag", "Required" } }
                span { "Asked by {decision.asked_by} · {decision_time(decision.asked_unix_seconds)}" }
            }
            p { strong { "Action: " } "{decision.action}" }
            p { "{decision.question}" }
            if !decision.options.is_empty() {
                ul {
                    for option in decision.options.iter() {
                        li { strong { "{option.label}" } " — {option.consequence}"
                            if option.recommended { span { class: "muted", " (recommended)" } }
                        }
                    }
                }
            }
            if let Some(answer) = decision.answer.as_ref() {
                div { class: "task-decision-answer",
                    p { strong { "Answer: " } if let Some(label) = chosen { "{label}" } else { "Free text" } }
                    if !answer.text.is_empty() { p { "{answer.text}" } }
                    p { class: "muted", "By {answer.answered_by} · {decision_time(answer.answered_unix_seconds)} · {answer_source_label(&answer.source)}" }
                }
            } else if decision.status == "cancelled" {
                p { class: "muted", "Cancelled: {decision.cancel_reason.clone().unwrap_or_default()}" }
                p { class: "muted", "By {decision.cancelled_by.clone().unwrap_or_default()} · {decision_time(decision.cancelled_unix_seconds.unwrap_or_default())}" }
            } else {
                if !error.is_empty() { div { class: "error-banner", "{error}" } }
                if !decision.options.is_empty() {
                    label { r#for: "decision-choice-{decision.id}", "Your choice" }
                    select { id: "decision-choice-{decision.id}", onchange: move |event| draft.write().option = event.value(),
                        option { value: "", selected: selection.is_empty(), "Choose an option or write an answer" }
                        for option in decision.options.iter() {
                            option { value: "{option.id}", selected: selection == option.id, "{option.label}" }
                        }
                    }
                }
                label { r#for: "decision-text-{decision.id}", "Answer or note" }
                textarea { id: "decision-text-{decision.id}", rows: "3", value: "{text}", oninput: move |event| draft.write().text = event.value() }
                button { class: "btn btn-primary", disabled: !can_answer, onclick: answer,
                    if saving { "Saving…" } else { "Record answer" }
                }
            }
        }
    }
}

fn decision_time(seconds: i64) -> String {
    js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(seconds as f64 * 1000.0))
        .to_locale_string("en-GB", &wasm_bindgen::JsValue::UNDEFINED)
        .as_string()
        .unwrap_or_default()
}

fn answer_source_label(source: &str) -> &'static str {
    match source {
        "ui" => "Answered in the board",
        "agent_reported" => "Reported from agent chat",
        "imported:ui" => "Imported history: original board answer",
        "imported:agent_reported" => "Imported history: original chat report",
        _ => "Imported or legacy history",
    }
}
