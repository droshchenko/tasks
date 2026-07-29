use serde::Deserialize;

#[derive(Deserialize)]
struct ProjectChangedPayload {
    #[serde(rename = "projectChanged")]
    project_changed: Option<String>,
    error: Option<String>,
}

/// What the server pushes down `/ws`.
///
/// Deliberately tiny: `ProjectChanged` is an invalidation signal and carries no task data at all, so the
/// client re-reads through the ordinary REST call. With no second copy of the board on this side there
/// is nothing that can drift out of sync — the price is one round trip on a change nobody is racing.
///
/// The project id the server sends with it is read but not kept: a socket watches exactly one board, so
/// the only board it can be told about is the one already on screen. Carrying the id would be data this
/// side never has a reason to compare.
pub enum ServerWsMessage {
    ProjectChanged,
    Error(String),
    Unknown(String),
}

impl ServerWsMessage {
    pub fn parse(raw: &str) -> Self {
        let Ok(payload) = serde_json::from_str::<ProjectChangedPayload>(raw) else {
            return Self::Unknown(raw.to_string());
        };

        if payload.project_changed.is_some() {
            return Self::ProjectChanged;
        }

        if let Some(error) = payload.error {
            return Self::Error(error);
        }

        Self::Unknown(raw.to_string())
    }
}
