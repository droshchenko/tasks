const SESSION_TOKEN_KEY: &str = "task_manager_session_token";

fn get_local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// Forget a session token this browser stored before sessions moved into a cookie.
///
/// **The only thing left of the old scheme, and it exists to finish removing it.** The session is now an
/// `HttpOnly` cookie: the browser attaches it to every request by itself, including the ones our code does not
/// make — the `<img>` and `<iframe>` that fetch a document's bytes, and the WebSocket handshake — and script
/// cannot read it, which is what a page somebody else uploaded must not be able to do.
///
/// So nothing writes a token here any more, and nothing reads one. What remains is clearing what an older
/// build left behind, so a stale token does not sit in local storage for a year after it stopped meaning
/// anything.
pub fn clear_session_token() {
    if let Some(storage) = get_local_storage() {
        let _ = storage.remove_item(SESSION_TOKEN_KEY);
    }
}

/// Which folders of a project's document tree were left open.
///
/// Local storage rather than a cookie, unlike the project preference beside it: this can be dozens of paths,
/// and a cookie is sent on every single request — including the ones fetching a document's bytes. The server
/// has no use for it, so there is nothing to be gained by paying that on every round trip.
///
/// Keyed per project, because a path in one project names nothing in another. Newline-separated, which is safe
/// because a document path cannot contain a newline: it is normalised server-side into slash-separated
/// segments with the whitespace trimmed.
pub fn get_expanded_folders(project: &str) -> std::collections::HashSet<String> {
    let Some(storage) = get_local_storage() else {
        return Default::default();
    };

    storage
        .get_item(&expanded_key(project))
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

pub fn set_expanded_folders(project: &str, folders: &std::collections::HashSet<String>) {
    if let Some(storage) = get_local_storage() {
        let joined: Vec<&str> = folders.iter().map(|itm| itm.as_str()).collect();
        let _ = storage.set_item(&expanded_key(project), &joined.join("\n"));
    }
}

fn expanded_key(project: &str) -> String {
    format!("task_manager_documents_expanded_{project}")
}
