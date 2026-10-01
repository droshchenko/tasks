use dioxus::prelude::*;
use task_manager_shared::ai_settings::{ProviderSettingsResponse, SaveProviderInput};

#[component]
pub fn AiProvidersPanel() -> Element {
    let mut data = use_resource(|| async { crate::api::get_ai_settings().await });
    let settings = match &*data.read() {
        Some(Ok(Some(settings))) => settings.clone(),
        Some(Ok(None)) => return rsx! { div { class: "empty-note", "Admins only." } },
        Some(Err(error)) => {
            return rsx! { div { class: "error-banner", "{error.message}" } button { class: "btn", onclick: move |_| data.restart(), "Retry" } };
        }
        None => return rsx! { div { class: "loading-note", "Loading AI settings…" } },
    };
    rsx! {
        div { class: "ai-section-heading",
            div { h2 { "AI providers" } p { class: "page-note", "Connect search and decision models for this task manager. Changes apply after saving." } }
            button { class: "btn btn-sm", onclick: move |_| data.restart(), "Reload settings" }
        }
        div { class: "ai-provider-grid",
            ProviderCard { key: "embeddings-{settings.embeddings.revision}", initial: settings.embeddings }
            ProviderCard { key: "jev-{settings.jev.revision}", initial: settings.jev }
        }
    }
}

#[derive(Clone, PartialEq)]
struct ProviderDraft {
    enabled: bool,
    endpoint: String,
    model: String,
    key: String,
    clear_key: bool,
}
impl ProviderDraft {
    fn from_saved(saved: &ProviderSettingsResponse) -> Self {
        Self {
            enabled: saved.enabled,
            endpoint: saved.endpoint.clone(),
            model: saved.model.clone(),
            key: String::new(),
            clear_key: false,
        }
    }
    fn dirty(&self, saved: &ProviderSettingsResponse) -> bool {
        self.enabled != saved.enabled
            || self.endpoint != saved.endpoint
            || self.model != saved.model
            || !self.key.is_empty()
            || self.clear_key
    }
}

#[component]
fn ProviderCard(initial: ProviderSettingsResponse) -> Element {
    let mut saved = use_signal(|| initial.clone());
    let mut draft = use_signal(|| ProviderDraft::from_saved(&initial));
    let mut busy = use_signal(|| false);
    let mut feedback = use_signal(String::new);
    let mut failed = use_signal(|| false);
    let mut testing = use_signal(|| false);
    let current = saved.read().clone();
    let values = draft.read().clone();
    let is_embeddings = current.provider == "embeddings";
    let title = if is_embeddings {
        "Embeddings"
    } else {
        "Jev decisions"
    };
    let description = if is_embeddings {
        "Find related tasks by meaning. Uses an OpenAI-compatible embeddings endpoint."
    } else {
        "Evaluate task type, urgency, missing human input and possible duplicates through TypeSafe Jev."
    };
    let id = current.provider.clone();
    let changed = values.dirty(&current);
    let unavailable = *busy.read();
    let status = if current.configured {
        "Configured"
    } else if current.enabled {
        "Needs configuration"
    } else {
        "Disabled"
    };
    let key_hint = if values.clear_key {
        "The saved key will be removed when you save."
    } else if current.has_key {
        "A key is stored on the server. Leave this field empty to keep it."
    } else {
        "Enter a key supplied by your provider."
    };
    let on_save = move |_| {
        if *busy.peek() {
            return;
        }
        let current = saved.peek().clone();
        let values = draft.peek().clone();
        busy.set(true);
        testing.set(false);
        feedback.set(String::new());
        spawn(async move {
            let input = SaveProviderInput {
                provider: current.provider.clone(),
                enabled: values.enabled,
                endpoint: values.endpoint,
                model: values.model,
                api_key: (!values.key.trim().is_empty()).then_some(values.key),
                clear_key: values.clear_key,
                revision: current.revision,
            };
            match crate::api::save_ai_provider(input).await {
                Ok(response) => {
                    let result = if current.provider == "embeddings" {
                        response.embeddings
                    } else {
                        response.jev
                    };
                    draft.set(ProviderDraft::from_saved(&result));
                    saved.set(result);
                    failed.set(false);
                    feedback.set("Settings saved.".into());
                }
                Err(error) => {
                    failed.set(true);
                    feedback.set(error.message);
                }
            }
            busy.set(false);
        });
    };
    let on_test = move |_| {
        if *busy.peek() {
            return;
        }
        let provider = saved.peek().provider.clone();
        busy.set(true);
        testing.set(true);
        feedback.set(String::new());
        spawn(async move {
            match crate::api::test_ai_provider(provider).await {
                Ok(result) => {
                    failed.set(!result.success);
                    feedback.set(format!(
                        "{}{} ({} ms)",
                        result.message,
                        if result.model.is_empty() {
                            String::new()
                        } else {
                            format!(" Model: {}.", result.model)
                        },
                        result.elapsed_ms
                    ));
                }
                Err(error) => {
                    failed.set(true);
                    feedback.set(error.message);
                }
            }
            testing.set(false);
            busy.set(false);
        });
    };

    rsx! {
        section { class: "card ai-provider-card", aria_label: "{title} settings",
            div { class: "ai-card-heading", h3 { "{title}" } span { class: "tag", "{status}" } }
            p { class: "field-hint", "{description}" }
            if current.source == "environment" && current.enabled { p { class: "field-hint", "Using server environment values. Saving here creates an explicit Settings configuration." } }
            div { class: "checkbox-row",
                input { id: "ai-{id}-enabled", r#type: "checkbox", checked: values.enabled, disabled: unavailable,
                    onchange: move |event| draft.write().enabled = event.checked() }
                label { r#for: "ai-{id}-enabled", "Enabled" }
            }
            if is_embeddings {
                div { class: "form-row",
                    label { r#for: "ai-{id}-endpoint", "Embeddings endpoint" }
                    input { id: "ai-{id}-endpoint", r#type: "url", value: "{values.endpoint}", disabled: unavailable,
                        placeholder: "https://provider.example/v1/embeddings", autocomplete: "off",
                        oninput: move |event| draft.write().endpoint = event.value() }
                }
            } else {
                p { class: "field-hint", "Provider: api.typesafe.ai" }
            }
            div { class: "form-row",
                label { r#for: "ai-{id}-model", "Model" }
                input { id: "ai-{id}-model", value: "{values.model}", disabled: unavailable, autocomplete: "off",
                    placeholder: if is_embeddings { "Your embedding model" } else { "jev-latest" },
                    oninput: move |event| draft.write().model = event.value() }
            }
            div { class: "form-row",
                label { r#for: "ai-{id}-key", if is_embeddings { "API key (optional for local providers)" } else { "API key" } }
                input { id: "ai-{id}-key", r#type: "password", value: "{values.key}", disabled: unavailable || values.clear_key,
                    autocomplete: "new-password", placeholder: if current.has_key { "Stored key — leave blank to keep" } else { "Provider API key" },
                    oninput: move |event| draft.write().key = event.value() }
                div { class: "field-hint", "{key_hint}" }
                if current.has_key {
                    div { class: "checkbox-row",
                        input { id: "ai-{id}-remove-key", r#type: "checkbox", checked: values.clear_key, disabled: unavailable,
                            onchange: move |event| { let mut edit = draft.write(); edit.clear_key = event.checked(); if edit.clear_key { edit.key.clear(); } } }
                        label { r#for: "ai-{id}-remove-key", "Remove saved key on save" }
                    }
                }
            }
            if !current.notice.is_empty() && current.enabled { p { class: "field-hint", "{current.notice}" } }
            div { class: "ai-card-actions",
                button { class: "btn btn-primary", disabled: unavailable || !changed, onclick: on_save,
                    if unavailable && !*testing.read() { "Saving…" } else { "Save" } }
                button { class: "btn", disabled: unavailable || changed || !current.configured, onclick: on_test,
                    if *testing.read() { "Testing…" } else { "Test connection" } }
            }
            p { class: "field-hint", if changed { "Save changes before testing the connection." }
                else { "Connection tests use synthetic input and may incur a provider charge." } }
            if !feedback.read().is_empty() {
                div { class: if *failed.read() { "error-banner" } else { "ai-success" }, role: "status", "{feedback}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_password_preserves_saved_key_and_draft_changes_disable_testing() {
        let saved = ProviderSettingsResponse {
            has_key: true,
            configured: true,
            ..Default::default()
        };
        let mut draft = ProviderDraft::from_saved(&saved);
        assert!(draft.key.is_empty());
        assert!(!draft.dirty(&saved));
        draft.clear_key = true;
        assert!(draft.dirty(&saved));
        draft.clear_key = false;
        draft.model = "new-model".into();
        assert!(draft.dirty(&saved));
    }
}
