use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

// Never put `///` doc comments on fields of a struct deriving MyHttpInput or
// MyHttpObjectStructure: the macro's attribute parser panics with `Somehow we got Punct here: =`.

// One document's index entry: everything about it EXCEPT its payload.
//
// The split is the whole reason this type exists, and a PDF is what makes it non-negotiable: the list a
// screen draws must not carry the payloads, or opening the Documents screen would pull every file in the
// project through JSON. A payload arrives one document at a time — text as `content` on
// `DocumentResponse`, bytes from the raw endpoint, which the browser fetches itself.
//
// `path` is the document's name AND its position: folders are derived from it and are not stored anywhere,
// so `docs/design/system.md` puts this entry two levels down without any folder having to exist.
//
// `id` is a `SortableId` and never changes — not when the text is rewritten, not when the path moves. That
// is what makes the history complete: every version of this document is stitched together by this string.
// It is also what a task or a goal stores when it references a document, which is why moving a document
// never breaks a reference.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DocumentIndexEntryResponse {
    pub id: String,
    pub project_id: String,
    pub path: String,
    // Which version is current. Starts at 1 and moves on every write — a rewrite, a move, a delete, a
    // restore. Shown because it is the cheapest possible answer to "has anybody touched this".
    pub version: i64,
    // What the payload IS — a MIME type: `text/markdown`, `application/pdf`. Worked out from the path when
    // nobody says otherwise, so `docs/spec.pdf` arrives as a PDF without being told.
    pub content_type: String,
    // Whether the payload is bytes rather than text. THE discriminator, and it is not derived from
    // `content_type`: which of the two storage columns is filled is the fact, and the MIME type is metadata
    // beside it. A reader uses this to decide between rendering the document and offering it for download.
    pub is_binary: bool,
    // How big the payload is, in BYTES — UTF-8 bytes for text, real bytes for a file. One unit for both, so
    // a listing showing documents of both kinds is comparable down the column.
    pub size: i64,
    pub updated_unix_seconds: i64,
    // Who wrote the current version — an email or the literal `AI`, unvalidated for the same reason a
    // comment's author is: MCP has no session to derive one from.
    pub updated_by: String,
}

// One document in full, content included. What a reader gets when they open one.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DocumentResponse {
    pub id: String,
    pub project_id: String,
    pub path: String,
    pub content_type: String,
    pub is_binary: bool,
    pub size: i64,
    // The text, for a text document — Markdown, rendered by whoever draws it and never trusted as HTML,
    // because an agent wrote it.
    //
    // `None` for a BINARY document, and deliberately not base64: the browser fetches those from the raw
    // endpoint, where the bytes travel as bytes and an `<iframe>` or an `<img>` can be pointed straight at
    // them. base64 exists on this feature only where JSON leaves no choice — an MCP argument — and putting
    // it here as well would cost a third of the size of every file for a path nothing uses.
    pub content: Option<String>,
    pub version: i64,
    pub created_unix_seconds: i64,
    pub updated_unix_seconds: i64,
    pub updated_by: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DocumentsResponse {
    pub documents: Vec<DocumentIndexEntryResponse>,
}

// One document, or the reason there is none.
//
// A miss is prose rather than an empty result, the same rule the whole surface follows: a reference to a
// document that has been deleted has to read as "it is in the trash", never as a blank pane.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct FindDocumentResponse {
    pub document: Option<DocumentResponse>,
    // True when the id names a document that is in the trash. Told apart from "no such id" because they
    // are different facts and only one of them is recoverable — restoring is an MCP call away.
    pub in_trash: bool,
    pub not_found: String,
}

#[derive(MyHttpInput)]
pub struct GetDocumentsInputModel {
    #[http_body(
        name: "project",
        description: "Which project's documents to index, by PREFIX — `RMS`. The handle a person uses for a board everywhere else in this product; the internal id never crosses this boundary"
    )]
    pub project: String,
}

// Upload a document from the browser.
//
// **The one write the browser makes about a document, and the second exception in the whole product to
// "MCP writes, the UI reads"** — the first being a goal's colour. It is here because the alternative is not a
// person using MCP, it is a person unable to upload at all: a PDF on a laptop cannot reach an agent without
// being base64-ed by hand into a tool call.
//
// The payload is base64 in a JSON body rather than multipart. It costs a third of the size on the way up,
// once per upload, and it buys the same request path everything else on this surface already uses — where
// multipart would mean a road through FlUrl's wasm backend that nothing here has travelled.
//
// There is no `who`: the browser HAS a session, so the author is the person signed in. That is strictly better
// than the MCP side, where `who` is an argument because there is nobody to ask.
#[derive(MyHttpInput)]
pub struct UploadDocumentInputModel {
    #[http_body(name: "project", description: "Which project to put it on, by prefix")]
    pub project: String,
    #[http_body(
        name: "path",
        description: "Where it goes — `notes.md`, or `docs/design/system.md`. Uploading to a path that is taken writes a new version of the document already there"
    )]
    pub path: String,
    #[http_body(name: "contentBase64", description: "The file, base64-encoded")]
    pub content_base64: String,
    #[http_body(
        name: "contentType",
        description: "The MIME type the browser reported for the file. Omitted or empty lets the path decide"
    )]
    pub content_type: Option<String>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct UploadDocumentResponse {
    pub id: String,
    pub path: String,
}

#[derive(MyHttpInput)]
pub struct GetDocumentInputModel {
    #[http_body(name: "project", description: "Which project the document belongs to, by prefix")]
    pub project: String,
    #[http_body(name: "id", description: "Which document, by the id its index entry reports")]
    pub id: String,
}

/// What a text document is, when nobody says otherwise. Also what every document written before there was a
/// `content_type` column was.
pub const DEFAULT_TEXT_CONTENT_TYPE: &str = "text/markdown";

/// What a binary document is when neither the caller nor the path says anything more specific. The MIME type
/// that means "bytes, download them".
pub const DEFAULT_BINARY_CONTENT_TYPE: &str = "application/octet-stream";

/// The MIME type a path implies, or `None` when its extension says nothing.
///
/// An explicit table rather than a crate: the list is short, it is the same on both sides of the wire, and
/// what is NOT in it matters as much as what is — an unknown extension has to fall through to a default that
/// depends on whether the payload is text or bytes, which only the caller knows.
///
/// Here in `shared` because both sides ask: the server to decide what to store, the browser to pick an icon.
pub fn content_type_for_path(path: &str) -> Option<&'static str> {
    let name = document_file_name(path);
    let (stem, extension) = name.rsplit_once('.')?;

    // `.gitignore` is a name, not an extension of nothing.
    if stem.is_empty() {
        return None;
    }

    Some(match extension.to_lowercase().as_str() {
        "md" | "markdown" => DEFAULT_TEXT_CONTENT_TYPE,
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "text/plain",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "zip" => "application/zip",
        _ => return None,
    })
}

/// Whether the browser should draw this in a FRAME rather than as text.
///
/// **Decided by the content type and nothing else**, which is the fix for the bug this replaced: HTML is
/// stored as text — it is diffable and searchable, so it belongs in the text column — and a viewer that
/// framed only what was *binary* therefore showed a web page as a wall of markup. What decides how a document
/// is DRAWN is what it is, not which column it came out of.
///
/// PDF and HTML, because those are the two the browser has a viewer for and this side does not.
pub fn is_framed_content_type(content_type: &str) -> bool {
    is_html_content_type(content_type) || content_type == "application/pdf"
}

/// Whether this is an image, which is drawn with an `<img>` rather than framed.
pub fn is_image_content_type(content_type: &str) -> bool {
    content_type.starts_with("image/")
}

/// Whether this is HTML — the one type that must be SANDBOXED wherever it is shown.
///
/// It gets its own function because it is the only content type on this surface with a security consequence:
/// HTML served from our own origin can read `localStorage`, and that is where the session token lives. A
/// document is uploaded by an agent through `/mcp`, so an unsandboxed preview would let whatever wrote it read
/// every viewer's token. Both halves of the defence key off this: the `sandbox` attribute on the frame, and
/// `Content-Security-Policy: sandbox` on the raw response — which is what also covers the frame being opened
/// directly in a tab.
///
/// The file browser this viewer was modelled on deliberately does NOT sandbox, and says why: it serves a
/// developer's own working copy over a local network, where a preview has to behave like the real page. Its
/// comment names the condition for changing that — a different exposure — and this is one: a board on the
/// public internet, behind a sign-in, showing documents somebody else uploaded.
pub fn is_html_content_type(content_type: &str) -> bool {
    content_type == "text/html" || content_type.starts_with("text/html;")
}

/// Where the raw bytes of every document live: `/raw/{prefix}/{path}`.
pub const RAW_ROUTE_PREFIX: &str = "/raw/";

/// The url that serves a document's raw bytes.
///
/// Built rather than fetched, because the two tags that use it — `<img>` and `<iframe>` — make the request
/// themselves; nothing in the client ever reads the bytes. Here in `shared` so the one place that spells the
/// route is the one place both sides read it from.
///
/// **A PATH that mirrors the document tree, not a query, and that is what makes a framed html page work.** A
/// page asks for its own `style.css` with a relative url, which the browser resolves against the address the
/// page came from: served as `/raw/TM/docs/page.html`, `style.css` beside it resolves to `/raw/TM/docs/style.css`
/// and arrives. From a query url it would resolve back onto the api route and arrive as nothing.
///
/// **No token in it.** The session is a cookie, which the browser attaches by itself — so this url is safe to
/// copy, to put in history and to hand to somebody: it opens for them only if they are signed in and on the
/// board.
///
/// Each segment is percent-encoded and the separators are not, so a `?` or a `#` in a document's name cannot
/// truncate the url while `docs/design/system.md` still arrives as three segments.
pub fn raw_document_url(project_prefix: &str, path: &str) -> String {
    let encoded: Vec<String> = path.split(PATH_SEPARATOR).map(percent_encode).collect();

    format!(
        "{RAW_ROUTE_PREFIX}{}/{}",
        percent_encode(project_prefix),
        encoded.join("/")
    )
}

/// Percent-encodes one url value.
///
/// Hand-rolled because `task-manager-shared` has no url dependency and needs one direction only. Strict on
/// purpose — anything outside the unreserved set is escaped, so `&`, `?`, `#`, `%`, spaces and non-ascii
/// cannot change what the url means.
fn percent_encode(src: &str) -> String {
    let mut encoded = String::with_capacity(src.len());

    for byte in src.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

/// Split `/raw/{prefix}/{path}` back into its two halves, percent-decoded.
///
/// `None` for anything that is not this route, which is what lets a request fall through to whatever owns it.
/// A project prefix can hold no `/` — it is validated to letters, digits and `_` where it is set — so the
/// first separator after the route prefix is unambiguously the end of it.
///
/// Beside the builder on purpose: a url that is written in one place and read in another is a url that drifts,
/// and the round-trip test below is only possible with both halves here.
pub fn parse_raw_document_url(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix(RAW_ROUTE_PREFIX)?;
    let (prefix, document_path) = rest.split_once(PATH_SEPARATOR)?;

    if prefix.is_empty() || document_path.is_empty() {
        return None;
    }

    let decoded: Vec<String> = document_path
        .split(PATH_SEPARATOR)
        .map(|segment| percent_decode(segment))
        .collect();

    Some((percent_decode(prefix), decoded.join("/")))
}

/// Reverses [`percent_encode`]. Anything that is not a well-formed escape is passed through as itself, which
/// is the lenient half of the pair: a url nobody built with the encoder still names something rather than
/// failing to parse.
fn percent_decode(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok();

            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                decoded.push(byte);
                at += 3;
                continue;
            }
        }

        decoded.push(bytes[at]);
        at += 1;
    }

    String::from_utf8_lossy(&decoded).to_string()
}

/// `1.2 KB`, `340 B` — enough to tell a note from a specification, which is all a listing needs a size for.
pub fn render_size(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    if bytes < 0 {
        return "0 B".to_string();
    }

    let as_float = bytes as f64;

    if as_float < KB {
        return format!("{bytes} B");
    }

    if as_float < MB {
        return format!("{:.1} KB", as_float / KB);
    }

    format!("{:.1} MB", as_float / MB)
}

/// The longest a document path may be. Generous enough for a deep folder tree, short enough that a path is
/// still something a person can read in a list.
pub const MAX_PATH_LEN: usize = 400;

/// The separator, and the only one. A document path is not a filesystem path: it is normalised to forward
/// slashes so the same document is named the same way whichever machine an agent is running on.
pub const PATH_SEPARATOR: char = '/';

/// Normalise a document path, or say why it is not one.
///
/// Here rather than on the server because both sides need the same answer: the server validates every write
/// with it, and the browser splits the result into folders — a client that disagreed about what a path is
/// would draw a tree the server does not have.
///
/// What it accepts, and why each rule is a rule:
///
/// * **A leading slash is dropped.** `/docs/a.md` and `docs/a.md` are one document. An agent handed a path
///   by a person will write either, and treating them as two documents at the same place is the single
///   easiest way to end up with a silent duplicate.
/// * **Backslashes become slashes**, for the same reason: an agent on Windows types the separator its shell
///   uses, and the document it means is the same document.
/// * **Empty segments collapse.** `docs//a.md` is `docs/a.md` — a doubled separator is a typo, not a folder
///   with no name.
/// * **`.` and `..` are refused.** This is not a walk over a filesystem; there is no current directory to be
///   relative to, and accepting `..` would mean two spellings of one path that only a resolver could tell
///   apart.
/// * **A trailing slash is refused.** A path names a document, and `docs/` names a folder — which does not
///   exist as a thing at all, since folders are derived from paths.
///
/// **Case is preserved and significant**: `README.md` and `readme.md` are two documents. A path is the name
/// a person gave the document, and folding case would silently rewrite it.
pub fn normalise_document_path(src: &str) -> Result<String, String> {
    let src = src.trim().replace('\\', "/");

    if src.is_empty() {
        return Err("a document needs a path, e.g. `notes.md` or `docs/design/system.md`".to_string());
    }

    if src.len() > MAX_PATH_LEN {
        return Err(format!(
            "that path is {} characters — the limit is {MAX_PATH_LEN}",
            src.len()
        ));
    }

    if src.ends_with(PATH_SEPARATOR) {
        return Err(format!(
            "'{src}' ends in a slash, so it names a folder rather than a document — folders are not stored, they come from the paths of the documents in them"
        ));
    }

    let mut segments: Vec<&str> = Vec::new();

    for segment in src.split(PATH_SEPARATOR) {
        let segment = segment.trim();

        // A doubled separator, or space around one. Dropped rather than refused: it is a typo with exactly
        // one sensible reading.
        if segment.is_empty() {
            continue;
        }

        if segment == "." || segment == ".." {
            return Err(format!(
                "'{src}' contains '{segment}' — a document path is not relative to anything, so write it out in full"
            ));
        }

        segments.push(segment);
    }

    if segments.is_empty() {
        return Err(format!("'{src}' has no name in it"));
    }

    Ok(segments.join("/"))
}

/// The folders a path sits in, outermost first. Empty for a document at the top level.
///
/// Derived, never stored — which is the whole model: a folder exists exactly as long as some document's path
/// mentions it, and an empty folder is not a thing that can exist.
pub fn document_folders(path: &str) -> Vec<&str> {
    let mut segments: Vec<&str> = path.split(PATH_SEPARATOR).collect();
    segments.pop();
    segments
}

/// The last segment of a path — what a reader sees as the document's name in its folder.
pub fn document_file_name(path: &str) -> &str {
    path.rsplit(PATH_SEPARATOR).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_survives_untouched() {
        assert_eq!(
            normalise_document_path("docs/design/system.md").unwrap(),
            "docs/design/system.md"
        );
        assert_eq!(normalise_document_path("notes.md").unwrap(), "notes.md");
    }

    /// The case this exists for: a person says `/path/document.md`, an agent passes it verbatim, and it has
    /// to be the same document as `path/document.md` rather than a second one at the same place.
    #[test]
    fn the_spellings_of_one_path_all_normalise_to_it() {
        for src in [
            "/docs/a.md",
            "docs/a.md",
            "  docs/a.md  ",
            "docs//a.md",
            "docs / a.md",
            "\\docs\\a.md",
        ] {
            assert_eq!(
                normalise_document_path(src).unwrap(),
                "docs/a.md",
                "{src:?} should normalise to docs/a.md"
            );
        }
    }

    /// Case is part of the name. Folding it would quietly rewrite what somebody called their document.
    #[test]
    fn case_is_significant() {
        assert_eq!(normalise_document_path("README.md").unwrap(), "README.md");
        assert_ne!(
            normalise_document_path("README.md").unwrap(),
            normalise_document_path("readme.md").unwrap()
        );
    }

    #[test]
    fn what_is_not_a_path_is_refused() {
        for src in [
            "",
            "   ",
            "/",
            "docs/",
            "docs/../a.md",
            "./a.md",
            "..",
        ] {
            assert!(
                normalise_document_path(src).is_err(),
                "{src:?} should not be accepted as a path"
            );
        }
    }

    #[test]
    fn an_over_long_path_is_refused() {
        let long = format!("{}.md", "a".repeat(MAX_PATH_LEN));
        assert!(normalise_document_path(&long).is_err());
    }

    #[test]
    fn folders_come_from_the_path_and_nothing_else() {
        assert_eq!(document_folders("docs/design/system.md"), vec!["docs", "design"]);
        assert_eq!(document_folders("notes.md"), Vec::<&str>::new());
    }

    /// The separators survive as separators and everything else is escaped — which is the whole trick: the
    /// path has to keep its shape so a framed page's relative links resolve, while a `?` in a name must not
    /// start a query string.
    #[test]
    fn a_raw_url_keeps_its_separators_and_escapes_the_rest() {
        assert_eq!(
            raw_document_url("TM", "docs/design/system.md"),
            "/raw/TM/docs/design/system.md"
        );

        assert_eq!(
            raw_document_url("TM", "docs/a b?c.md"),
            "/raw/TM/docs/a%20b%3Fc.md"
        );

        // Two bytes in utf-8, so two escapes.
        assert_eq!(raw_document_url("TM", "é.md"), "/raw/TM/%C3%A9.md");
    }

    /// The two halves have to agree, or a url the client builds is a url the server cannot read.
    #[test]
    fn a_raw_url_round_trips() {
        for (prefix, path) in [
            ("TM", "docs/design/system.md"),
            ("RMS", "notes.md"),
            ("TM", "docs/a b?c.md"),
            ("TM", "é.md"),
            ("TM", "docs/100% done.md"),
        ] {
            let url = raw_document_url(prefix, path);

            assert_eq!(
                parse_raw_document_url(&url),
                Some((prefix.to_string(), path.to_string())),
                "{url} does not round-trip"
            );
        }
    }

    #[test]
    fn what_is_not_a_raw_url_falls_through() {
        assert_eq!(parse_raw_document_url("/api/documents/v1/list"), None);
        assert_eq!(parse_raw_document_url("/raw/"), None);
        assert_eq!(parse_raw_document_url("/raw/TM"), None, "a project is not a document");
        assert_eq!(parse_raw_document_url("/raw/TM/"), None);
    }

    #[test]
    fn a_size_reads_like_a_size() {
        assert_eq!(render_size(340), "340 B");
        assert_eq!(render_size(1536), "1.5 KB");
        assert_eq!(render_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(render_size(0), "0 B");
    }

    #[test]
    fn a_content_type_is_read_off_the_extension() {
        assert_eq!(content_type_for_path("docs/spec.pdf"), Some("application/pdf"));
        assert_eq!(content_type_for_path("a/b/notes.MD"), Some("text/markdown"));
        assert_eq!(content_type_for_path("logo.png"), Some("image/png"));

        // Nothing to read: the caller's declaration or a kind-dependent default decides.
        assert_eq!(content_type_for_path("Makefile"), None);
        assert_eq!(content_type_for_path(".gitignore"), None);
        assert_eq!(content_type_for_path("build.sh"), None);
    }

    /// The bug this pins: HTML is TEXT, so a viewer that framed only binary payloads showed a web page as
    /// markup. Framing is decided by the type, and `text/html` is framed while `text/markdown` is not.
    #[test]
    fn html_is_framed_even_though_it_is_text() {
        assert!(is_framed_content_type("text/html"));
        assert!(is_framed_content_type("application/pdf"));

        assert!(!is_framed_content_type("text/markdown"));
        assert!(!is_framed_content_type("text/plain"));
        assert!(!is_framed_content_type("image/png"), "an image is drawn, not framed");
        assert!(!is_framed_content_type("application/zip"));
    }

    #[test]
    fn an_image_is_drawn_rather_than_framed() {
        assert!(is_image_content_type("image/png"));
        assert!(is_image_content_type("image/svg+xml"));
        assert!(!is_image_content_type("application/pdf"));
    }

    /// A charset parameter must not smuggle HTML past the sandbox check — `text/html; charset=utf-8` is HTML.
    #[test]
    fn html_is_recognised_with_a_charset_on_it() {
        assert!(is_html_content_type("text/html"));
        assert!(is_html_content_type("text/html; charset=utf-8"));

        assert!(!is_html_content_type("text/plain"));
        // Not a prefix match on `text/htm`: a type has to BE html to be treated as html.
        assert!(!is_html_content_type("text/htmlish"));
    }

    #[test]
    fn the_name_is_the_last_segment() {
        assert_eq!(document_file_name("docs/design/system.md"), "system.md");
        assert_eq!(document_file_name("notes.md"), "notes.md");
    }
}
