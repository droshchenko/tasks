use std::sync::Arc;

use encryption::aes::AesKey;

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
}
