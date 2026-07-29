use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.

// Everything the UI needs about the signed-in user.
//
// `name` may be empty — the UI then falls back to the email. `is_admin` is the resolved answer
// (the flag on the user row OR membership of the settings admin list), not the stored flag: the UI
// gates Projects setup and Users on this one value and should not have to know where the right came
// from.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct MeResponse {
    pub email: String,
    pub name: String,
    pub is_admin: bool,
}

// Where to send the browser to sign in.
//
// The URL is built server-side because it carries the `client_id` and the `redirect_uri` from the
// service settings, and the client has no business knowing either.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoogleAuthUrlResponse {
    pub url: String,
}

// What a completed sign-in hands back: the session token every later request carries in its
// `Authorization` header, and that /ws takes as a `token=` query parameter because the browser
// WebSocket API cannot send headers.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct GoogleCallbackResponse {
    pub token: String,
}

// Google hands the browser back to us with a one-time `code`; `state` is the CSRF token we issued
// with the auth URL and expect to see returned unchanged.
#[derive(MyHttpInput)]
// Body, not query, and that is load-bearing. Both values are base64-ish and regularly contain a `+`; a
// `+` in a QUERY value means a space, so sending them as query parameters mangled the state on roughly
// six sign-ins in ten — the ones whose state happened to contain one — and surfaced as a 401 that looked
// random. A JSON body carries the bytes verbatim and has no such rule.
pub struct GoogleCallbackInputModel {
    #[http_body(name: "code", description: "One-time authorization code from Google")]
    pub code: String,
    #[http_body(name: "state", description: "The CSRF token issued with the auth URL")]
    pub state: String,
}
