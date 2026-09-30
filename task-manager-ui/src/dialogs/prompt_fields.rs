use dioxus::prelude::*;
use task_manager_shared::execution_prompts::{ExecutionPrompt, prompt_text, requires_analysis};

#[component]
pub fn ExecutionPromptEditor(
    prompts: Vec<ExecutionPrompt>,
    targets: Vec<String>,
    on_edit: EventHandler<(String, String)>,
    on_requirement: EventHandler<(String, bool)>,
) -> Element {
    rsx! {
        details { class: "execution-prompt-editor",
            summary { "Agent execution prompts" }
            div { class: "field-hint", "Only the current column and task type are sent to an agent. Keep each instruction short. Descriptions above remain definitions for people." }
            for target in targets.iter() {
                div { class: "form-row", key: "{target}",
                    label { r#for: "execution-prompt-{target}", "{target}" }
                    textarea {
                        id: "execution-prompt-{target}", rows: "4",
                        maxlength: "4000",
                        value: prompt_text(&prompts, target),
                        placeholder: "Instructions for this stage or task type",
                        oninput: {
                            let target = target.clone();
                            move |event: Event<FormData>| on_edit.call((target.clone(), event.value()))
                        },
                    }
                    label { class: "checkbox-label",
                        input { r#type: "checkbox", checked: requires_analysis(&prompts, target),
                            onchange: { let target = target.clone(); move |event: Event<FormData>| on_requirement.call((target.clone(), event.checked())) },
                        }
                        if target == "done" {
                            "Require analysis file links before completing the task"
                        } else {
                            "Require analysis file links before leaving this column or completing this task type"
                        }
                    }
                }
            }
        }
    }
}
