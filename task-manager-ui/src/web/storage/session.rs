const SESSION_TOKEN_KEY: &str = "task_manager_session_token";
const LAST_PROJECT_KEY: &str = "task_manager_last_project";

fn get_local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn save_session_token(token: &str) {
    if let Some(storage) = get_local_storage() {
        let _ = storage.set_item(SESSION_TOKEN_KEY, token);
    }
}

pub fn get_session_token() -> Option<String> {
    get_local_storage()?
        .get_item(SESSION_TOKEN_KEY)
        .ok()?
        .filter(|itm| !itm.is_empty())
}

pub fn clear_session_token() {
    if let Some(storage) = get_local_storage() {
        let _ = storage.remove_item(SESSION_TOKEN_KEY);
    }
}

/// Which board was open last.
///
/// Remembered so Home opens where it was left rather than on whichever project happens to sort first —
/// on a machine with one board it means the dropdown never has to be touched at all.
pub fn save_last_project(project_id: &str) {
    if let Some(storage) = get_local_storage() {
        let _ = storage.set_item(LAST_PROJECT_KEY, project_id);
    }
}

pub fn get_last_project() -> Option<String> {
    get_local_storage()?
        .get_item(LAST_PROJECT_KEY)
        .ok()?
        .filter(|itm| !itm.is_empty())
}
