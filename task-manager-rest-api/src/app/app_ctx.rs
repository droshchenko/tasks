use std::sync::Arc;

use encryption::aes::AesKey;
use task_manager_shared::goals::GoalResponse;
use task_manager_shared::tasks::TaskResponse;
use task_manager_shared::ws::{BoardSnapshot, ServerWsPayload};

use crate::board::Board;
use crate::postgres::{
    ColumnTemplatesRepo, GoalsRepo, KindTemplatesRepo, ProjectMembersRepo, ProjectsRepo, TasksRepo,
    UsersRepo,
};
use crate::settings::SettingsReader;
use crate::subscribers::ProjectSubscribers;

// Reported to MCP clients on `initialize` — `McpMiddleware::new` takes both as `&'static str`.
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `encryption::aes::AesKey` requires — it panics on anything else.
const SESSION_KEY_LEN: usize = 48;

pub struct AppContext {
    // Postgres is where the data is durable. Nothing reads through these repos except the startup
    // load and the write half of `scripts/` — every read serves from memory.
    pub projects_repo: ProjectsRepo,
    pub column_templates_repo: ColumnTemplatesRepo,
    pub kind_templates_repo: KindTemplatesRepo,
    pub goals_repo: GoalsRepo,
    pub project_members_repo: ProjectMembersRepo,
    pub tasks_repo: TasksRepo,
    pub users_repo: UsersRepo,

    // The state every read actually serves from.
    pub board: Board,

    // The Homes to tell when a board changes.
    pub subscribers: ProjectSubscribers,

    // Encrypts and reads the session token and the OAuth `state`. There is no session store: the token
    // carries the session, so this key is the whole of it.
    pub session_key: AesKey,

    pub settings_reader: Arc<SettingsReader>,
}

impl AppContext {
    pub async fn new(settings_reader: Arc<SettingsReader>) -> Self {
        // Read once at startup rather than per request: rotating the key is a deploy, and re-deriving it
        // on every call would only add work.
        //
        // The length is checked here rather than left to `AesKey::new`, which panics. Without this the
        // service would start happily on a misconfigured key and die on the first sign-in — the failure
        // would surface hours later, in the one place nobody is watching.
        let session_encryption_key = settings_reader
            .get_settings()
            .await
            .session_encryption_key
            .clone();

        if session_encryption_key.len() != SESSION_KEY_LEN {
            panic!(
                "settings: session_encryption_key must be exactly {SESSION_KEY_LEN} bytes, got {}",
                session_encryption_key.len()
            );
        }

        let session_key = AesKey::new(session_encryption_key.as_bytes());

        Self {
            projects_repo: ProjectsRepo::new(settings_reader.clone()).await,
            column_templates_repo: ColumnTemplatesRepo::new(settings_reader.clone()).await,
            kind_templates_repo: KindTemplatesRepo::new(settings_reader.clone()).await,
            goals_repo: GoalsRepo::new(settings_reader.clone()).await,
            project_members_repo: ProjectMembersRepo::new(settings_reader.clone()).await,
            tasks_repo: TasksRepo::new(settings_reader.clone()).await,
            users_repo: UsersRepo::new(settings_reader.clone()).await,
            board: Board::new(),
            subscribers: ProjectSubscribers::new(),
            session_key,
            settings_reader,
        }
    }

    /// Whether this email is an admin, resolved the way every caller needs it: the flag on the user
    /// row **or** membership of the `admins` list in settings.
    ///
    /// The settings list is additive rather than an alternative, which is what makes an empty
    /// database recoverable — no row there says admin, yet those addresses still get in and create
    /// the roster.
    pub async fn is_admin_in_settings(&self, email: &str) -> bool {
        let email = email.trim().to_lowercase();

        self.settings_reader
            .get_settings()
            .await
            .admins
            .iter()
            .any(|itm| itm.trim().to_lowercase() == email)
    }

    /// Push the board to every Home watching it.
    ///
    /// **The board itself, not a signal to go and re-read it.** A re-read empties the screen for as long as
    /// the round trip takes, and what the reader sees in that moment is a spinner where their board was —
    /// on a board that repaints every time an agent touches anything, that is most of the time. A whole
    /// snapshot cannot drift out of step with the server the way a delta could, so this keeps the one
    /// property the invalidation signal was chosen for.
    ///
    /// Called from `scripts/` after the change is in Postgres **and** in memory.
    ///
    /// The snapshot matches `/api/tasks/v1/list` exactly — same mapper, same archive filter — because the
    /// client has one way of reading a board and it must not matter which door the board came through.
    pub async fn notify_project_changed(&self, project_id: &str) {
        // Built while the board lock is held and sent after it is dropped: `parking_lot`'s guard is `!Send`,
        // so holding one across an `.await` does not compile — which is the compiler enforcing the thing we
        // want anyway, since one stalled socket must not hold up every other reader of the board.
        let prepared = {
            let board = self.board.read();

            board.get_project(project_id).map(|project| {
                let tasks: Vec<TaskResponse> = board
                    .tasks_of_project(&project.id)
                    .iter()
                    .filter(|task| !board.is_archived(task))
                    .map(|task| crate::mappers::task_to_response(task, &project, &board))
                    .collect();

                // Live goals only, and their counters computed here — the same read `/api/goals/v1/list`
                // does. Archived tasks are deliberately absent from `tasks` above while being counted in
                // these numbers, which is why the client is told to take the counters as given.
                let goals: Vec<GoalResponse> = board
                    .goals_of_project(&project.id)
                    .iter()
                    .filter(|goal| !board.is_goal_archived(goal))
                    .map(|goal| {
                        let (tasks_amount, done_amount) =
                            board.goal_progress(&goal.project_id, goal.number);

                        crate::mappers::goal_to_response(
                            goal,
                            &project.prefix,
                            tasks_amount,
                            done_amount,
                        )
                    })
                    .collect();

                let members: Vec<String> = project.members.iter().cloned().collect();

                (tasks, goals, members)
            })
        };

        let (payload, members) = match prepared {
            Some((tasks, goals, members)) => (
                ServerWsPayload::board(BoardSnapshot {
                    project_id: project_id.to_string(),
                    tasks,
                    goals,
                }),
                members,
            ),
            // No project in memory to build a board from. The signal still goes out, and a client that gets
            // one without a snapshot re-reads — which is also what the whole protocol did before snapshots.
            None => (ServerWsPayload::project_changed(project_id), Vec::new()),
        };

        let Ok(payload) = serde_json::to_string(&payload) else {
            return;
        };

        self.subscribers
            .push_to_watchers(project_id, &payload, &members)
            .await;
    }
}
