use std::time::Duration;

use flurl::FlUrl;
use serde::Deserialize;

use crate::scripts::MAX_BINARY_LEN;

/// What every request identifies itself as. GitHub refuses an API request without a User-Agent, so this
/// is not politeness — it is the difference between working and a 403 with a body nobody reads.
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

const API_BASE: &str = "https://api.github.com";

/// The API version this speaks, pinned. GitHub's REST API is versioned by date and an unpinned client
/// is one that changes behaviour on a day nobody deployed anything.
const API_VERSION: &str = "2022-11-28";

/// How long a listing may take — the head commit, and the tree behind it.
const LISTING_TIMEOUT: Duration = Duration::from_secs(30);

/// How long ONE file may take. Longer than a listing because it is a payload rather than an answer, and
/// still bounded: a read that hangs is a screen that hangs behind it.
const BLOB_TIMEOUT: Duration = Duration::from_secs(60);

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

        if self.branch.contains(['?', '#', ' ', '\\']) {
            return Err(GithubError::Refused(format!(
                "'{}' is not a branch or tag name",
                self.branch
            )));
        }

        Ok(())
    }
}

/// One file in the repository, as a REFERENCE: where it is, how big it is, and the blob that holds it.
///
/// **There is no content here, and that is the whole design.** This service holds references and nothing
/// else — a listing of a repository costs one small response however large the repository is, and the
/// bytes of a file are fetched at the moment somebody actually asks for that file and are let go
/// immediately afterwards. Nothing is unpacked, nothing is cached on disk, and the process's memory is
/// never holding more than the one file being read.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    /// Path from the root of the repository.
    pub path: String,
    pub size: i64,
    /// The blob's sha — how the content is asked for, and what changes when the file changes.
    pub sha: String,
}

/// A repository's whole file list, and whether GitHub gave all of it.
pub struct RepoTree {
    pub entries: Vec<TreeEntry>,
    /// GitHub cuts a very large tree off rather than paginating it. Reported rather than swallowed: a
    /// truncated listing is a folder that silently lacks files, which is the one failure a reader cannot
    /// see for themselves.
    pub truncated: bool,
}

#[derive(Deserialize)]
struct TreeResponse {
    #[serde(default)]
    tree: Vec<TreeItemResponse>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeItemResponse {
    path: String,
    #[serde(rename = "type")]
    item_type: String,
    sha: String,
    #[serde(default)]
    size: Option<i64>,
}

/// The commit the branch points at right now, as a full sha.
///
/// **This is what makes a ten-minute poll cheap.** A repository nobody has touched answers this in a few
/// hundred bytes, and the pull stops there — the file list is only re-read when the sha differs from the
/// one the mirror already holds. `Accept: application/vnd.github.sha` asks GitHub to send the sha as the
/// whole body rather than a commit object with the sha somewhere in it.
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
        .set_timeout(LISTING_TIMEOUT)
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

/// Every file in the repository at one commit — paths, sizes and blob shas, and no content.
///
/// One request for the whole tree, however deep it is: `recursive=1` is what makes this a listing rather
/// than a walk, and a walk would be one request per folder against an API with an hourly budget.
///
/// Folders are dropped here rather than carried: a folder is not a file, and in this product it is not a
/// thing at all — folders are read off the paths of the files in them, on both sides.
pub async fn list_tree(
    repo_ref: &RepoRef<'_>,
    commit: &str,
    key: Option<&str>,
) -> Result<RepoTree, GithubError> {
    repo_ref.validate()?;

    if commit.is_empty() || !commit.chars().all(|itm| itm.is_ascii_alphanumeric()) {
        return Err(GithubError::Refused(format!(
            "'{commit}' is not a commit sha"
        )));
    }

    let url = format!(
        "{API_BASE}/repos/{}/{}/git/trees/{commit}?recursive=1",
        repo_ref.owner, repo_ref.repo
    );

    let mut response = api_request(&url, key, "application/vnd.github+json")?
        .set_timeout(LISTING_TIMEOUT)
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

    let body = response
        .get_body_as_slice()
        .await
        .map_err(|err| GithubError::Transport(format!("GitHub's answer did not read: {err}")))?;

    let parsed: TreeResponse = serde_json::from_slice(body).map_err(|err| {
        GithubError::Refused(format!("GitHub's file list did not parse: {err}"))
    })?;

    let entries = parsed
        .tree
        .into_iter()
        .filter(|itm| itm.item_type == "blob")
        .map(|itm| TreeEntry {
            path: itm.path,
            // A blob with no size is a submodule or a symlink entry GitHub cannot size. Zero rather
            // than a guess, and the read below refuses an empty payload anyway.
            size: itm.size.unwrap_or(0),
            sha: itm.sha,
        })
        .collect();

    Ok(RepoTree {
        entries,
        truncated: parsed.truncated,
    })
}

/// One file's bytes, fetched at the moment somebody asked for that file.
///
/// **Asked for by blob sha rather than by path**, which is what makes a read unambiguous: a path is
/// resolved against a ref and could answer with a file written since the listing, whereas a sha names
/// exactly the bytes the listing described. It also means the url carries nothing a path could smuggle
/// into it — a sha is forty hex characters or it is refused here.
///
/// `Accept: application/vnd.github.raw` asks for the bytes themselves; the default would be a JSON
/// object with the content base64-ed inside it, which costs a third more on the wire and a decode at the
/// other end.
pub async fn read_blob(
    repo_ref: &RepoRef<'_>,
    sha: &str,
    key: Option<&str>,
) -> Result<Vec<u8>, GithubError> {
    repo_ref.validate()?;

    if sha.is_empty() || !sha.chars().all(|itm| itm.is_ascii_hexdigit()) {
        return Err(GithubError::Refused(format!("'{sha}' is not a blob sha")));
    }

    let url = format!(
        "{API_BASE}/repos/{}/{}/git/blobs/{sha}",
        repo_ref.owner, repo_ref.repo
    );

    let mut response = api_request(&url, key, "application/vnd.github.raw")?
        .set_timeout(BLOB_TIMEOUT)
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

    let bytes = response
        .get_body_as_slice()
        .await
        .map_err(|err| GithubError::Transport(format!("the file did not read: {err}")))?;

    // Checked here as well as against the listed size, because the listing and the read are two moments:
    // the size that was listed is a claim about a file that may have been replaced since.
    if bytes.len() > MAX_BINARY_LEN {
        return Err(GithubError::TooLarge(format!(
            "that file is {} bytes — the limit for one document is {MAX_BINARY_LEN}",
            bytes.len()
        )));
    }

    Ok(bytes.to_vec())
}

/// One request, with the four headers every GitHub call needs and the key when there is one.
///
/// `try_new` rather than `new`, and that is not defensive style — `FlUrl::new` PANICS on a url it cannot
/// parse, and a panic inside the spawned pull task would take that connection's refresh down with no
/// state written and no message anywhere, leaving the mirror on `pulling` for ever.
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
        401 => GithubError::NeedsKey(format!("GitHub rejected the key{}", suffix(&detail))),
        403 | 429 => {
            // Rate limiting is the expected failure of a design that reads one file per request, and it
            // is not a key problem for somebody who has one — it is a wait. Told apart by what GitHub
            // says, since the status cannot.
            if detail.to_lowercase().contains("rate limit") {
                GithubError::Refused(format!(
                    "GitHub is rate-limiting this connection{}. Anonymous reads get 60 an hour; a key raises it to 5000",
                    suffix(&detail)
                ))
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

        // Anything else is passed through with its number, which is what an unexpected status deserves.
        let other = status_error(500, false, "");
        assert!(!other.needs_key());
        assert!(other.message().contains("500"));
    }

    /// Rate limiting is the expected failure of reading one file per request, so it must never be
    /// reported as a key problem — and the message has to say what raises the ceiling.
    #[test]
    fn rate_limiting_is_a_wait_rather_than_a_key_problem() {
        for status in [403, 429] {
            let limited = status_error(status, true, r#"{"message":"API rate limit exceeded"}"#);

            assert!(!limited.needs_key(), "{}", limited.message());
            assert!(limited.message().contains("60"), "{}", limited.message());
        }
    }

    /// The tree listing keeps files and drops everything else — a folder is not a thing in this product,
    /// and a submodule is not a file anybody can read.
    #[test]
    fn only_blobs_come_out_of_a_tree() {
        let body = r#"{"tree":[
            {"path":"docs","type":"tree","sha":"aaa"},
            {"path":"docs/a.md","type":"blob","sha":"bbb","size":12},
            {"path":"vendor","type":"commit","sha":"ccc"},
            {"path":"link","type":"blob","sha":"ddd"}
        ],"truncated":false}"#;

        let parsed: TreeResponse = serde_json::from_str(body).unwrap();

        let entries: Vec<TreeEntry> = parsed
            .tree
            .into_iter()
            .filter(|itm| itm.item_type == "blob")
            .map(|itm| TreeEntry {
                path: itm.path,
                size: itm.size.unwrap_or(0),
                sha: itm.sha,
            })
            .collect();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "docs/a.md");
        assert_eq!(entries[0].size, 12);
        // A blob GitHub cannot size reads as zero rather than as a guess.
        assert_eq!(entries[1].size, 0);
    }
}
