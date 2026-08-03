use std::time::Duration;

use flurl::{FlUrl, FlUrlMode};

/// What every request identifies itself as. GitHub refuses an API request without a User-Agent, so this
/// is not politeness — it is the difference between working and a 403 with a body nobody reads.
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

const API_BASE: &str = "https://api.github.com";

/// The API version this speaks, pinned. GitHub's REST API is versioned by date and an unpinned client
/// is one that changes behaviour on a day nobody deployed anything.
const API_VERSION: &str = "2022-11-28";

/// How long one call to GitHub may take.
///
/// Two values because they are two different waits: an answer to "what is the head commit" is a few
/// hundred bytes and should be quick, while a zipball of a real repository is a download. FlUrl's
/// default of ten seconds is right for the first and far too short for the second.
const HEAD_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);

/// The most a repository archive may be, compressed.
///
/// Enforced while the body is read rather than trusted from a header, because codeload builds the
/// archive as it sends it and does not always declare a length. This is the only thing standing between
/// a mistyped repository and the service's memory.
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;

/// How many redirects are followed before giving up. GitHub uses exactly one — api.github.com to
/// codeload — and anything past a couple of hops is a loop rather than a route.
const MAX_REDIRECTS: usize = 5;

/// What went wrong, in the shapes the caller has to tell apart.
///
/// The distinction that earns its keep is [`GithubError::NeedsKey`]: a private repository is
/// indistinguishable from one that does not exist — GitHub answers 404 to an unauthorised reader on
/// purpose, so that a 404 does not confirm a repository is there. A mirror showing "needs a key" when
/// the repository is genuinely misspelled is a better wrong answer than "no such repository" when the
/// key simply expired, because the first sends somebody to look at both.
#[derive(Debug)]
pub enum GithubError {
    /// Refused, and a key — or a better key — is what would change that.
    NeedsKey(String),
    /// The API said no, in a way a key would not fix.
    Refused(String),
    /// Never got an answer.
    Transport(String),
    /// The answer was too big to accept.
    TooLarge(String),
}

impl GithubError {
    pub fn message(&self) -> &str {
        match self {
            Self::NeedsKey(message)
            | Self::Refused(message)
            | Self::Transport(message)
            | Self::TooLarge(message) => message,
        }
    }

    pub fn needs_key(&self) -> bool {
        matches!(self, Self::NeedsKey(_))
    }
}

impl std::fmt::Display for GithubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// Which repository, on which branch — the two things every call here needs.
pub struct RepoRef<'s> {
    pub owner: &'s str,
    pub repo: &'s str,
    /// Empty for the repository's default branch.
    pub branch: &'s str,
}

impl RepoRef<'_> {
    /// The ref as a url segment. `HEAD` is GitHub's own spelling of "whatever the default branch is",
    /// which is what lets an unset branch be one route rather than a lookup followed by a route.
    fn git_ref(&self) -> &str {
        if self.branch.is_empty() {
            "HEAD"
        } else {
            self.branch
        }
    }

    /// Reject anything that could not be a GitHub name before it is put in a url.
    ///
    /// The values come from a parsed url and a validated branch, so this is the second line rather than
    /// the first — and it is here rather than only at the edge because this is the function that builds
    /// the request, and a check next to the thing it protects is the one that survives a refactor.
    fn validate(&self) -> Result<(), GithubError> {
        for (what, value) in [("owner", self.owner), ("repository", self.repo)] {
            if value.is_empty()
                || !value
                    .chars()
                    .all(|itm| itm.is_ascii_alphanumeric() || matches!(itm, '-' | '_' | '.'))
            {
                return Err(GithubError::Refused(format!(
                    "'{value}' is not a GitHub {what} name"
                )));
            }
        }

        if self
            .branch
            .contains(['?', '#', ' ', '\\'])
        {
            return Err(GithubError::Refused(format!(
                "'{}' is not a branch or tag name",
                self.branch
            )));
        }

        Ok(())
    }
}

/// The commit the branch points at right now, as a full sha.
///
/// **This is what makes a ten-minute poll cheap.** A repository nobody has touched answers this in a few
/// hundred bytes, and the pull stops there — the archive is only downloaded when the sha differs from
/// the one the mirror already holds. `Accept: application/vnd.github.sha` asks GitHub to send the sha
/// as the whole body rather than a commit object with the sha somewhere in it.
pub async fn resolve_head_commit(
    repo_ref: &RepoRef<'_>,
    key: Option<&str>,
) -> Result<String, GithubError> {
    repo_ref.validate()?;

    let url = format!(
        "{API_BASE}/repos/{}/{}/commits/{}",
        repo_ref.owner,
        repo_ref.repo,
        repo_ref.git_ref()
    );

    let mut response = api_request(&url, key, "application/vnd.github.sha")?
        .set_timeout(HEAD_TIMEOUT)
        .get()
        .await
        .map_err(|err| GithubError::Transport(format!("could not reach GitHub: {err}")))?;

    let status = response.get_status_code();

    if status != 200 {
        let body = response
            .get_body_as_str()
            .await
            .unwrap_or_default()
            .to_string();

        return Err(status_error(status, key.is_some(), &body));
    }

    let sha = response
        .get_body_as_str()
        .await
        .map_err(|err| GithubError::Transport(format!("GitHub's answer did not read: {err}")))?
        .trim()
        .to_string();

    if sha.is_empty() {
        return Err(GithubError::Refused(
            "GitHub answered with no commit — the branch may not exist".to_string(),
        ));
    }

    Ok(sha)
}

/// The whole repository at one ref, as the bytes of a zip.
///
/// **The redirect is followed by hand, and the key is deliberately dropped on the way.** FlUrl's native
/// backend does not follow redirects at all — that is the browser's job under wasm and nobody's here —
/// and api.github.com answers this route with a 302 to codeload.github.com. The url it redirects to is
/// already signed and needs no credential, so forwarding the Authorization header to another host would
/// hand a token to a host that did not ask for it, which is the rule curl adopted for the same reason.
pub async fn download_archive(
    repo_ref: &RepoRef<'_>,
    key: Option<&str>,
) -> Result<Vec<u8>, GithubError> {
    repo_ref.validate()?;

    let mut url = format!(
        "{API_BASE}/repos/{}/{}/zipball/{}",
        repo_ref.owner,
        repo_ref.repo,
        repo_ref.git_ref()
    );

    // The key rides only on the first hop, for the reason in the doc comment above. Whether there ever
    // WAS one is remembered separately: it is what tells "the key cannot see this" from "this is private
    // and you gave none", and dropping the key must not silently turn the first message into the second.
    let had_key = key.is_some();
    let mut key = key;

    for _ in 0..MAX_REDIRECTS {
        let response = api_request(&url, key, "application/vnd.github+json")?
            .set_timeout(DOWNLOAD_TIMEOUT)
            // Explicit, because streaming a body needs a hyper response and the no-hyper mode
            // materialises one instead — which would turn `get_body_as_stream` below into a panic.
            .update_mode(FlUrlMode::Http1Hyper)
            .get()
            .await
            .map_err(|err| GithubError::Transport(format!("could not reach GitHub: {err}")))?;

        let status = response.get_status_code();

        if (300..400).contains(&status) {
            let location = response
                .get_header_case_insensitive("location")
                .ok()
                .flatten()
                .map(str::to_string);

            let Some(location) = location.filter(|itm| !itm.trim().is_empty()) else {
                return Err(GithubError::Refused(
                    "GitHub redirected the download without saying where to".to_string(),
                ));
            };

            url = location;
            key = None;
            continue;
        }

        if status != 200 {
            let mut response = response;

            let body = response
                .get_body_as_str()
                .await
                .unwrap_or_default()
                .to_string();

            return Err(status_error(status, had_key, &body));
        }

        return read_capped(response).await;
    }

    Err(GithubError::Refused(
        "GitHub redirected the download more times than makes sense".to_string(),
    ))
}

/// Read a body, refusing rather than allocating once it passes [`MAX_ARCHIVE_BYTES`].
///
/// Streamed rather than taken whole because the cap has to bite BEFORE the memory is spent: a
/// `Content-Length` would be the cheap check, and codeload does not always send one — it builds the
/// archive as it goes.
async fn read_capped(response: flurl::FlUrlResponse) -> Result<Vec<u8>, GithubError> {
    let mut stream = response.get_body_as_stream();
    let mut bytes: Vec<u8> = Vec::new();

    loop {
        let chunk = stream
            .get_next_chunk()
            .await
            .map_err(|err| GithubError::Transport(format!("the download stopped: {err}")))?;

        let Some(chunk) = chunk else {
            break;
        };

        if bytes.len() + chunk.len() > MAX_ARCHIVE_BYTES {
            return Err(GithubError::TooLarge(format!(
                "that repository is larger than {MAX_ARCHIVE_BYTES} bytes compressed, which is the limit — connect a folder inside it rather than the whole thing"
            )));
        }

        bytes.extend_from_slice(&chunk);
    }

    if bytes.is_empty() {
        return Err(GithubError::Refused(
            "GitHub sent an empty archive".to_string(),
        ));
    }

    Ok(bytes)
}

/// One request, with the four headers every GitHub call needs and the key when there is one.
///
/// `try_new` rather than `new`, and that is not defensive style — it is the difference between an error
/// and a dead pull. `FlUrl::new` PANICS on a url it cannot parse, and one of the urls that comes through
/// here was not written by us: it is the `Location` GitHub sent back on the redirect. A panic inside the
/// spawned pull task would take that connection's refresh down with no state written and no message
/// anywhere, and the mirror would sit on `pulling` for ever.
fn api_request(url: &str, key: Option<&str>, accept: &str) -> Result<FlUrl, GithubError> {
    let mut request = FlUrl::try_new(url)
        .map_err(|err| GithubError::Refused(format!("'{url}' is not a url we can call: {err}")))?
        .with_header("User-Agent", USER_AGENT)
        .with_header("Accept", accept)
        .with_header("X-GitHub-Api-Version", API_VERSION);

    if let Some(key) = key.map(str::trim).filter(|itm| !itm.is_empty()) {
        request = request.with_header("Authorization", format!("Bearer {key}"));
    }

    Ok(request)
}

/// Turn a status code into the sentence a person needs, and into the distinction the mirror needs.
///
/// **404 is the interesting one.** GitHub answers 404 rather than 403 to a reader who may not see a
/// private repository, precisely so that the answer does not confirm the repository exists. So a 404
/// without a key is "this may be private", and a 404 with one is "the key cannot see it, or the name is
/// wrong" — neither of which is the flat "no such repository" the status code literally says.
fn status_error(status: u16, had_key: bool, body: &str) -> GithubError {
    let detail = github_message(body);

    match status {
        401 => GithubError::NeedsKey(format!(
            "GitHub rejected the key{}",
            suffix(&detail)
        )),
        403 => {
            // Rate limiting comes back as 403 too, and it is not a key problem for somebody who has one
            // — it is a wait. Told apart by what GitHub says, since the status cannot.
            if detail.to_lowercase().contains("rate limit") {
                GithubError::Refused(format!("GitHub is rate-limiting this connection{}", suffix(&detail)))
            } else if had_key {
                GithubError::NeedsKey(format!(
                    "the key is not allowed to read this repository{}",
                    suffix(&detail)
                ))
            } else {
                GithubError::NeedsKey(
                    "this repository cannot be read anonymously — give it a key".to_string(),
                )
            }
        }
        404 => {
            if had_key {
                GithubError::NeedsKey(
                    "no such repository, branch or folder — or the key cannot see it".to_string(),
                )
            } else {
                GithubError::NeedsKey(
                    "no such repository, branch or folder — or it is private and needs a key"
                        .to_string(),
                )
            }
        }
        status => GithubError::Refused(format!("GitHub answered {status}{}", suffix(&detail))),
    }
}

/// The `message` out of GitHub's error json, or the body as it came if it is not that shape.
///
/// Truncated, because an error body can be a page: what goes on a screen beside a connection is a line.
fn github_message(body: &str) -> String {
    let body = body.trim();

    if body.is_empty() {
        return String::new();
    }

    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|itm| itm.get("message")?.as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string());

    let message = message.trim();

    if message.chars().count() > 200 {
        let short: String = message.chars().take(200).collect();
        format!("{short}…")
    } else {
        message.to_string()
    }
}

fn suffix(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!(" — {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_ref<'s>(owner: &'s str, repo: &'s str, branch: &'s str) -> RepoRef<'s> {
        RepoRef {
            owner,
            repo,
            branch,
        }
    }

    /// An unset branch is GitHub's own `HEAD`, which is what keeps "the default branch" from needing a
    /// lookup of its own.
    #[test]
    fn an_unset_branch_reads_as_head() {
        assert_eq!(repo_ref("o", "r", "").git_ref(), "HEAD");
        assert_eq!(repo_ref("o", "r", "main").git_ref(), "main");
    }

    #[test]
    fn a_name_that_could_not_be_a_repository_never_reaches_a_url() {
        assert!(repo_ref("o", "r", "main").validate().is_ok());
        assert!(repo_ref("My-Jet_Tools", "fl.url", "").validate().is_ok());

        assert!(repo_ref("o/../etc", "r", "").validate().is_err());
        assert!(repo_ref("o", "r?x=1", "").validate().is_err());
        assert!(repo_ref("", "r", "").validate().is_err());
        assert!(repo_ref("o", "r", "a b").validate().is_err());
    }

    /// The whole point of reading GitHub's body: "Bad credentials" on a screen beats "GitHub answered
    /// 401".
    #[test]
    fn githubs_own_message_is_what_gets_shown() {
        assert_eq!(
            github_message(r#"{"message":"Bad credentials","status":"401"}"#),
            "Bad credentials"
        );

        // Not json, or not that shape — the body itself is still better than nothing.
        assert_eq!(github_message("upstream timeout"), "upstream timeout");
        assert_eq!(github_message("   "), "");
    }

    /// A 404 is never reported as a flat "no such repository": it is the answer GitHub gives to a reader
    /// who may not see a private one.
    #[test]
    fn a_404_is_read_as_possibly_needing_a_key() {
        assert!(status_error(404, false, "").needs_key());
        assert!(status_error(404, true, "").needs_key());
        assert!(status_error(401, true, "").needs_key());

        // Rate limiting arrives as 403 and is not a key problem — it is a wait.
        let limited = status_error(403, true, r#"{"message":"API rate limit exceeded"}"#);
        assert!(!limited.needs_key(), "{}", limited.message());

        // Anything else is passed through with its number, which is what an unexpected status deserves.
        let other = status_error(500, false, "");
        assert!(!other.needs_key());
        assert!(other.message().contains("500"));
    }
}
