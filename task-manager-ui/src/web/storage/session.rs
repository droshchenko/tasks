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

/// Which folders of a project's document tree were left open.
///
/// Per project, because a path in one project names nothing in another. Stored rather than derived so a reload
/// comes back to the tree somebody was reading instead of collapsing everything they had opened — which on a
/// deep tree is a dozen clicks to undo.
///
/// Newline-separated, which is safe because a document path cannot contain a newline: it is normalised
/// server-side into slash-separated segments with the whitespace trimmed.
pub fn get_expanded_folders(project_id: &str) -> std::collections::HashSet<String> {
    let Some(storage) = get_local_storage() else {
        return Default::default();
    };

    storage
        .get_item(&expanded_key(project_id))
        .ok()
        .flatten()
        .map(|raw| {
            raw.split('\n')
                .filter(|itm| !itm.is_empty())
                .map(|itm| itm.to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub fn set_expanded_folders(project_id: &str, folders: &std::collections::HashSet<String>) {
    if let Some(storage) = get_local_storage() {
        let joined: Vec<&str> = folders.iter().map(|itm| itm.as_str()).collect();
        let _ = storage.set_item(&expanded_key(project_id), &joined.join("\n"));
    }
}

fn expanded_key(project_id: &str) -> String {
    format!("task_manager_documents_expanded_{project_id}")
}
