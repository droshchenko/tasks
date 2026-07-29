use task_manager_shared::auth::MeResponse;

/// Where the shell is in working out whether anybody is signed in.
///
/// Three states rather than an `Option`, because "we have not asked yet" and "we asked and nobody is"
/// have to look different: the first shows a spinner, the second shows the login screen. Collapsing
/// them flashes the login form on every page load.
#[derive(Clone, PartialEq)]
pub enum SignedIn {
    Unknown,
    No,
    Yes(MeResponse),
}

#[derive(Clone, PartialEq)]
pub struct AppState {
    pub signed_in: SignedIn,
    /// True once the WebSocket task has been spawned, so a re-render does not open a second one.
    pub ws_started: bool,
    /// Bumped by the WebSocket when the open board changes. Views watch it and re-read — that is the
    /// whole invalidation mechanism on this side.
    pub board_revision: u64,
    pub ws_live: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            signed_in: SignedIn::Unknown,
            ws_started: false,
            board_revision: 0,
            ws_live: false,
        }
    }
}

impl AppState {
    pub fn me(&self) -> Option<&MeResponse> {
        match &self.signed_in {
            SignedIn::Yes(me) => Some(me),
            SignedIn::Unknown | SignedIn::No => None,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.me().map(|me| me.is_admin).unwrap_or(false)
    }
}
