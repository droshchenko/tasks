use encryption::AesEncryptedDataOwned;
use encryption::aes::AesKey;
use rust_extensions::date_time::DateTimeAsMicroseconds;

/// How long a session token is good for.
const SESSION_TTL_HOURS: i64 = 12;

/// How long a sign-in may take between leaving for Google and coming back.
const LOGIN_STATE_TTL_SECONDS: i64 = 600;

/// A session, carried entirely inside the token.
///
/// Nothing is stored server-side: the token *is* the session, protobuf-encoded and encrypted with the
/// key from settings. Three things follow from that, and all three are the reason for it:
///
/// * a restart does not sign anyone out — the key outlives the process;
/// * there is no session table and no session map to keep in sync;
/// * a second instance would accept tokens issued by the first, so nothing here is what pins this
///   service to one instance.
///
/// The cost is that a token cannot be revoked server-side, so `logout` is the client dropping it. Two
/// things make that acceptable: the TTL is short, and every request re-reads the user row — so
/// disabling somebody locks them out immediately, which is the case revocation would actually be for.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SessionToken {
    #[prost(string, tag = "1")]
    pub email: ::prost::alloc::string::String,
    /// Unix seconds. Checked on every parse.
    #[prost(int64, tag = "2")]
    pub expires: i64,
}

impl SessionToken {
    pub fn issue(email: String) -> Self {
        let mut expires = DateTimeAsMicroseconds::now();
        expires.add_hours(SESSION_TTL_HOURS);

        Self {
            email,
            expires: expires.unix_microseconds / 1_000_000,
        }
    }

    pub fn to_token(&self, aes_key: &AesKey) -> String {
        let mut as_bytes = Vec::new();
        prost::Message::encode(self, &mut as_bytes)
            .expect("session token: protobuf encode cannot fail");

        aes_key.encrypt(&as_bytes).as_base_64()
    }

    /// Read a token back.
    ///
    /// `None` for anything that is not a token this service issued and that is still valid — a garbled
    /// string, one encrypted with a different key, or an expired one. The caller turns all of them into
    /// the same 401: from the browser's side they all mean "sign in again".
    pub fn parse(token: &str, aes_key: &AesKey) -> Option<Self> {
        let encrypted = AesEncryptedDataOwned::from_base_64(token).ok()?;
        let decrypted = aes_key.decrypt(&encrypted).ok()?;
        let session: Self = prost::Message::decode(decrypted.as_slice()).ok()?;

        if session.email.trim().is_empty() {
            return None;
        }

        let now = DateTimeAsMicroseconds::now().unix_microseconds / 1_000_000;

        if session.expires <= now {
            return None;
        }

        Some(session)
    }
}

/// The CSRF `state` handed to Google, also carried entirely inside itself.
///
/// A `nonce` so two sign-ins started at the same second are different strings, and an expiry so a
/// state cannot be used a week later.
///
/// Honest limitation: a stateless state cannot detect a replay — there is no record of it having been
/// used. What actually stops a replayed callback is Google: the `code` it accompanies is single-use, so
/// a second attempt with the same pair fails at the token exchange. The state's job here is to prove
/// the callback belongs to a sign-in *this* deployment started, and to bound how long that is true for.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct LoginState {
    #[prost(string, tag = "1")]
    pub nonce: ::prost::alloc::string::String,
    #[prost(int64, tag = "2")]
    pub expires: i64,
}

impl LoginState {
    pub fn issue() -> Self {
        let mut expires = DateTimeAsMicroseconds::now();
        expires.add_seconds(LOGIN_STATE_TTL_SECONDS);

        Self {
            nonce: uuid::Uuid::new_v4().to_string(),
            expires: expires.unix_microseconds / 1_000_000,
        }
    }

    pub fn to_token(&self, aes_key: &AesKey) -> String {
        let mut as_bytes = Vec::new();
        prost::Message::encode(self, &mut as_bytes)
            .expect("login state: protobuf encode cannot fail");

        aes_key.encrypt(&as_bytes).as_base_64()
    }

    pub fn is_valid(token: &str, aes_key: &AesKey) -> bool {
        let Ok(encrypted) = AesEncryptedDataOwned::from_base_64(token) else {
            return false;
        };

        let Ok(decrypted) = aes_key.decrypt(&encrypted) else {
            return false;
        };

        let Ok(state) = <Self as prost::Message>::decode(decrypted.as_slice()) else {
            return false;
        };

        let now = DateTimeAsMicroseconds::now().unix_microseconds / 1_000_000;

        !state.nonce.is_empty() && state.expires > now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // `AesKey::new` panics on any other length — 48 bytes exactly.
    fn key() -> AesKey {
        AesKey::new(b"0123456789abcdef0123456789abcdef0123456789abcdef")
    }

    #[test]
    fn a_session_round_trips_through_its_token() {
        let issued = SessionToken::issue("yuri@mxtm.ai".to_string());
        let token = issued.to_token(&key());

        let parsed = SessionToken::parse(&token, &key()).expect("should parse");

        assert_eq!(parsed.email, "yuri@mxtm.ai");
        assert_eq!(parsed.expires, issued.expires);
    }

    /// The whole security of a stateless token rests on this: a token encrypted with another key must
    /// not read back. If it did, anyone could mint themselves a session for any email.
    #[test]
    fn a_token_from_another_key_does_not_read() {
        let token = SessionToken::issue("yuri@mxtm.ai".to_string()).to_token(&key());
        let other = AesKey::new(b"fedcba9876543210fedcba9876543210fedcba9876543210");

        assert!(SessionToken::parse(&token, &other).is_none());
    }

    #[test]
    fn an_expired_session_does_not_read() {
        let expired = SessionToken {
            email: "yuri@mxtm.ai".to_string(),
            expires: 1,
        };

        let token = expired.to_token(&key());

        assert!(SessionToken::parse(&token, &key()).is_none());
    }

    #[test]
    fn garbage_does_not_read() {
        for token in ["", "not-base64!!", "aGVsbG8="] {
            assert!(
                SessionToken::parse(token, &key()).is_none(),
                "{token:?} should not read"
            );
        }
    }

    #[test]
    fn a_login_state_round_trips_and_an_expired_one_is_refused() {
        let token = LoginState::issue().to_token(&key());
        assert!(LoginState::is_valid(&token, &key()));

        let expired = LoginState {
            nonce: "n".to_string(),
            expires: 1,
        }
        .to_token(&key());
        assert!(!LoginState::is_valid(&expired, &key()));

        assert!(!LoginState::is_valid("nonsense", &key()));
    }

    /// Two sign-ins started in the same second must produce different states, or one browser's callback
    /// would validate against another's.
    #[test]
    fn two_login_states_differ() {
        assert_ne!(LoginState::issue().nonce, LoginState::issue().nonce);
    }
}
