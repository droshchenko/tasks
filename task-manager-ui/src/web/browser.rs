/// Write to the browser console.
///
/// Hand-written because `dioxus-utils` has no logging helper — its `js` module is storage, focus, sleep,
/// reload and `GlobalAppSettings`. Used only where something went wrong that the screen cannot usefully
/// show: a WebSocket that dropped, a `/me` that failed on the transport.
pub fn console_log(message: &str) {
    web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(message));
}

/// Leave the app entirely — a full browser navigation, not a route push.
///
/// Two callers, and both genuinely need to leave: the Google consent page is not ours to route to, and
/// signing out has to throw away every signal in the app, including the WebSocket task and the cached
/// `/me`, which a route push would leave running underneath the login screen.
pub fn navigate_to(url: &str) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(url);
    }
}
