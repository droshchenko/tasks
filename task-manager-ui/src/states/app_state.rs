use task_manager_shared::auth::MeResponse;
use task_manager_shared::ws::BoardSnapshot;

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
    /// Bumped by the WebSocket on every push about the open board. Views watch it, which is what makes two
    /// pushes carrying identical tasks still count as two pushes.
    pub board_revision: u64,
    /// The board the last push carried, when it carried one. `None` means the push was a bare signal and the
    /// view has to re-read — see [`AppState::board_invalidated`].
    ///
    /// The socket lands it here rather than writing into Home's own state because the socket is older than
    /// any screen and outlives all of them: it has no way to reach a component's signal, and Home watching
    /// this is the same shape as Home watching the revision counter it replaces.
    pub board_push: Option<BoardSnapshot>,
    pub ws_live: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            signed_in: SignedIn::Unknown,
            ws_started: false,
            board_revision: 0,
            board_push: None,
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

    /// A push that carried the board. The view replaces what it shows with this — no request, no spinner.
    ///
    /// One method for the two fields because they are one fact: a snapshot without the bump would not wake a
    /// view that is already showing an equal board, and a bump without the snapshot means the opposite thing.
    pub fn board_pushed(&mut self, snapshot: BoardSnapshot) {
        self.board_revision += 1;
        self.board_push = Some(snapshot);
    }

    /// A push that only said "it changed". Clearing the snapshot is the point: leaving the previous one in
    /// place would have the view re-apply a board that is now known to be out of date.
    pub fn board_invalidated(&mut self) {
        self.board_revision += 1;
        self.board_push = None;
    }
}
