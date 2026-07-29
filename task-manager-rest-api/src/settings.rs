service_sdk::macros::use_settings!();

// Settings live in the settings-service under `/settings/task-manager-mcp/task-manager-rest-api`.
//
// The Google credentials are here rather than in Postgres on purpose: they are what makes the
// service able to authenticate at all, so putting them behind a screen that requires
// authentication would leave a fresh deployment with no way in. Same reason `admins` is here —
// on an empty database nobody is an admin in Postgres, and these emails are what let the first
// person sign in and create everyone else.
#[derive(
    service_sdk::my_settings_reader::SettingsModel,
    SdkSettingsTraits,
    AutoGenerateSettingsTraits,
    Serialize,
    Deserialize,
    Debug,
    Clone,
)]
pub struct SettingsModel {
    pub seq_conn_string: String,
    pub my_telemetry: Option<String>,
    // `SdkSettingsTraits` keys the `PostgresSettings` impl off this exact field name.
    pub postgres_conn_string: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    // Where Google sends the browser back. Must match the redirect URI registered in the Google
    // console byte for byte, so it is configured rather than derived from the request — behind a
    // reverse proxy the request's own host is not the public one.
    pub google_redirect_uri: String,
    // Encrypts the session token and the OAuth `state`. Changing it signs everyone out, which is also
    // the only way to revoke a session — the token is self-contained, so there is nothing else to
    // invalidate.
    pub session_encryption_key: String,
    // Emails that are admins whatever Postgres says. Additive on top of the `admin` flag on a user
    // row, never a replacement for it: this is the emergency door and the bootstrap.
    #[serde(default)]
    pub admins: Vec<String>,
}
