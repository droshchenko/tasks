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
    pub project: String,
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
    pub project: String,
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

// Upload a ZIP and unpack it, one document per file inside.
//
// The sibling of `UploadDocumentInputModel`, and it exists because the one-file-at-a-time dialog is the wrong
// shape for what people actually have: a folder of documents. Zipping it and dropping the archive in is one
// gesture; forty uploads is not.
//
// **The archive is never stored.** It is a transport, not a document — what lands is the files inside it, at
// paths built from the folder plus each entry's own path, so the tree inside the zip becomes the tree in the
// project. Which is also why there is no `path` here and a `folder` instead: the caller says WHERE, and the
// archive says what each thing is called.
#[derive(MyHttpInput)]
pub struct UploadArchiveInputModel {
    #[http_body(name: "project", description: "Which project to put it on, by prefix")]
    pub project: String,
    #[http_body(
        name: "folder",
        description: "Which folder to unpack into — `docs/design`, or omitted/empty for the top level. Every path inside the archive is hung under it, so a zip holding `a/b.md` unpacked into `docs` writes `docs/a/b.md`"
    )]
    pub folder: Option<String>,
    #[http_body(name: "contentBase64", description: "The zip itself, base64-encoded")]
    pub content_base64: String,
}

// What came out of an archive: what was written, and what was not.
//
// The skips are a FIRST-CLASS half of this answer rather than an error. A zip out of a real folder carries
// things that are not documents — `__MACOSX`, `.DS_Store`, empty files, something over the size limit — and
// refusing the whole upload because of one of them would be useless. So the entries that could be written are
// written, and the rest are reported with the reason, one line each.
#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct UploadArchiveResponse {
    pub documents: Vec<UploadDocumentResponse>,
    pub skipped: Vec<SkippedArchiveEntryResponse>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct SkippedArchiveEntryResponse {
    // The entry's name AS IT IS INSIDE THE ARCHIVE, not the path it would have been written to — the path it
    // would have had is exactly what does not exist.
    pub name: String,
    pub reason: String,
}

/// Whether a picked file is a ZIP — the thing to unpack rather than to store.
///
/// The name decides, and the browser's content type is only a fallback: a zip is `application/zip` in Chrome,
/// `application/x-zip-compressed` in others and nothing at all in a few, while `.zip` on the end of a name is
/// the same everywhere. Here in `shared` because both sides ask — the dialog to change what it offers, the
/// server to refuse an archive that is not one.
pub fn is_zip_upload(file_name: &str, content_type: Option<&str>) -> bool {
    if document_file_name(file_name)
        .to_lowercase()
        .ends_with(".zip")
    {
        return true;
    }

    let Some(content_type) = content_type else {
        return false;
    };

    matches!(
        content_type.trim().to_lowercase().as_str(),
        "application/zip" | "application/x-zip-compressed" | "multipart/x-zip"
    )
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

/// The MIME type a path implies, or `None` when neither its name nor its extension says anything.
///
/// **A page is more than one file, and this table is what decides whether the other files work.** An HTML
/// document served from `/raw/` asks for its own stylesheet and its own script with relative urls, and a
/// browser with strict MIME checking — which is every browser — refuses to apply a stylesheet that arrives
/// as anything but `text/css`, and refuses to run a script that is not a script type. So an extension
/// missing from here is not a cosmetic gap: it is a page that renders bare with two lines in a console
/// nobody has open. That is the bug this table was widened for, and it is why the fallback around it says
/// "bytes" rather than guessing at text — see [`crate::documents::DEFAULT_BINARY_CONTENT_TYPE`].
///
/// An explicit table rather than a crate: it is the same on both sides of the wire, and what is in it is a
/// decision rather than a lookup — `text/plain` for source files is chosen because a browser shows it and
/// cannot be tricked by it, where a more specific `text/x-…` buys nothing anybody reads.
///
/// It covers everything `my_http_server::WebContentType::detect_by_extension` covers, deliberately: the
/// static middleware and this route serve the same kinds of file, and a browser that is happy with one and
/// not the other would be a difference nobody could explain.
///
/// Here in `shared` because both sides ask: the server to decide what to store and what to send, the
/// browser to pick an icon.
pub fn content_type_for_path(path: &str) -> Option<&'static str> {
    let name = document_file_name(path);

    // By NAME first, for the files that carry their type in the name and no extension at all. A repository
    // is full of them, and a `LICENSE` offered as a download rather than shown is the kind of small wrong
    // answer that makes a mirror feel broken.
    match name.to_lowercase().as_str() {
        "makefile" | "dockerfile" | "license" | "licence" | "notice" | "authors" | "codeowners"
        | "readme" | "changelog" | ".gitignore" | ".gitattributes" | ".dockerignore"
        | ".editorconfig" | ".env" => return Some("text/plain"),
        _ => {}
    }

    let (stem, extension) = name.rsplit_once('.')?;

    // A name that is nothing but an extension — `.gitignore` is a name, not an extension of nothing. The
    // ones worth knowing are matched above, by name.
    if stem.is_empty() {
        return None;
    }

    Some(match extension.to_lowercase().as_str() {
        // Text this product renders as itself.
        "md" | "markdown" => DEFAULT_TEXT_CONTENT_TYPE,
        "txt" | "text" | "log" => "text/plain",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",

        // What a page is made of. The three that broke: a stylesheet, a script and a font are all refused
        // outright by the browser when the type is wrong, unlike an image, which it sniffs and draws.
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" | "cjs" => "text/javascript",
        "map" | "json" => "application/json",
        "wasm" => "application/wasm",
        "xml" | "xsl" | "xsd" => "application/xml",
        "yaml" | "yml" => "application/yaml",

        // Fonts. `font/*` is the modern spelling and what browsers want; the ancient `eot` keeps the
        // vendor type it has always had.
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",

        // Images. `ico` keeps `image/x-icon` rather than the newer `image/vnd.microsoft.icon`, because it
        // is what every browser and every existing favicon already agrees on.
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",

        // Source and configuration, all as `text/plain`. They are read far more often than they are run,
        // and this is a document surface: what matters is that a browser shows them and that nothing here
        // claims they are a script that ought to execute.
        "py" | "rs" | "go" | "rb" | "php" | "pl" | "lua" | "r" | "swift" | "kt" | "kts"
        | "java" | "scala" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "m" | "mm" | "sql"
        | "graphql" | "gql" | "proto" | "sh" | "bash" | "zsh" | "fish" | "bat" | "cmd" | "ps1"
        | "toml" | "ini" | "cfg" | "conf" | "properties" | "env" | "gradle" | "tf" | "tfvars"
        | "vue" | "svelte" | "ts" | "tsx" | "jsx" | "rst" | "adoc" | "asciidoc" | "org" | "tex"
        | "diff" | "patch" | "lock" => "text/plain",

        // Things the browser has a viewer for, or an obvious way to handle.
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",

        _ => return None,
    })
}

/// Whether a content type is text on the wire and therefore needs a charset beside it.
///
/// **Without one the browser guesses**, and its guess for `text/html` and `text/css` is not UTF-8 — it is
/// the document's locale default, which turns every non-Latin character in a mirrored page into mojibake.
/// The types that carry their own encoding declaration (an image, a PDF, a font) must NOT get one: a
/// charset parameter on them is meaningless and some parsers treat the whole type as unknown.
///
/// `application/json` is included though JSON is UTF-8 by definition — saying so costs nothing and stops a
/// proxy in between deciding otherwise.
pub fn content_type_needs_charset(content_type: &str) -> bool {
    let content_type = content_type.trim().to_lowercase();

    content_type.starts_with("text/")
        || matches!(
            content_type.as_str(),
            "application/json" | "application/xml" | "application/yaml" | "image/svg+xml"
        )
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
        return Err(
            "a document needs a path, e.g. `notes.md` or `docs/design/system.md`".to_string(),
        );
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
        for src in ["", "   ", "/", "docs/", "docs/../a.md", "./a.md", ".."] {
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
        assert_eq!(
            document_folders("docs/design/system.md"),
            vec!["docs", "design"]
        );
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
        assert_eq!(
            parse_raw_document_url("/raw/TM"),
            None,
            "a project is not a document"
        );
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
        assert_eq!(
            content_type_for_path("docs/spec.pdf"),
            Some("application/pdf")
        );
        assert_eq!(content_type_for_path("a/b/notes.MD"), Some("text/markdown"));
        assert_eq!(content_type_for_path("logo.png"), Some("image/png"));

        // Nothing to read: the caller's declaration or a kind-dependent default decides.
        assert_eq!(content_type_for_path("prototype.unknownext"), None);
        assert_eq!(content_type_for_path("noextension"), None);
    }

    /// **The three that a browser refuses outright when the type is wrong.** An image with a bad type is
    /// sniffed and drawn anyway; a stylesheet is not applied, a script is not run and a font is not loaded —
    /// which is a page rendering bare with nothing but two console lines to say why.
    #[test]
    fn a_page_gets_the_types_its_own_assets_need() {
        assert_eq!(
            content_type_for_path("design-system/css/design-system.css"),
            Some("text/css")
        );
        assert_eq!(
            content_type_for_path("design-system/js/design-system.js"),
            Some("text/javascript")
        );
        assert_eq!(content_type_for_path("app.mjs"), Some("text/javascript"));
        assert_eq!(
            content_type_for_path("fonts/inter.woff2"),
            Some("font/woff2")
        );
        assert_eq!(content_type_for_path("fonts/inter.ttf"), Some("font/ttf"));
        assert_eq!(content_type_for_path("icons.svg"), Some("image/svg+xml"));
        assert_eq!(content_type_for_path("data.json"), Some("application/json"));
    }

    /// Everything `my_http_server::WebContentType::detect_by_extension` knows, this knows too — the static
    /// middleware and the raw route serve the same kinds of file, and a browser happy with one and not the
    /// other would be a difference nobody could explain. `wasm` is the one that was missing.
    #[test]
    fn it_covers_what_the_static_middleware_covers() {
        for (path, expected) in [
            ("a.png", "image/png"),
            ("a.svg", "image/svg+xml"),
            ("a.css", "text/css"),
            ("a.js", "text/javascript"),
            ("a.html", "text/html"),
            ("a.htm", "text/html"),
            ("a.text", "text/plain"),
            ("a.json", "application/json"),
            ("a.yaml", "application/yaml"),
            ("a.yml", "application/yaml"),
            ("a.wasm", "application/wasm"),
        ] {
            assert_eq!(content_type_for_path(path), Some(expected), "{path}");
        }
    }

    /// Source and configuration read as text rather than downloading — a repository is mostly these, and a
    /// `build.py` offered as a file to save is a mirror that feels broken.
    #[test]
    fn source_files_are_text() {
        for path in [
            "build.py",
            "src/main.rs",
            "run.sh",
            "Cargo.toml",
            "schema.sql",
            "app.ts",
            "notes.rst",
        ] {
            assert_eq!(content_type_for_path(path), Some("text/plain"), "{path}");
        }
    }

    /// The files that carry their type in the NAME and have no extension at all. A repository is full of
    /// them, and they are read constantly.
    #[test]
    fn the_names_that_are_text_without_an_extension() {
        for path in [
            "Makefile",
            "docs/LICENSE",
            "Dockerfile",
            "README",
            ".gitignore",
            ".editorconfig",
        ] {
            assert_eq!(content_type_for_path(path), Some("text/plain"), "{path}");
        }
    }

    /// A charset belongs on text and nowhere else: on an image or a font it is meaningless, and some
    /// parsers treat a type carrying one as unknown.
    #[test]
    fn only_text_asks_for_a_charset() {
        assert!(content_type_needs_charset("text/html"));
        assert!(content_type_needs_charset("text/css"));
        assert!(content_type_needs_charset("text/javascript"));
        assert!(content_type_needs_charset("TEXT/Markdown"));
        assert!(content_type_needs_charset("application/json"));
        // XML and SVG both carry an encoding declaration of their own, and both are still served as text.
        assert!(content_type_needs_charset("image/svg+xml"));

        assert!(!content_type_needs_charset("image/png"));
        assert!(!content_type_needs_charset("font/woff2"));
        assert!(!content_type_needs_charset("application/pdf"));
        assert!(!content_type_needs_charset("application/octet-stream"));
    }

    /// The bug this pins: HTML is TEXT, so a viewer that framed only binary payloads showed a web page as
    /// markup. Framing is decided by the type, and `text/html` is framed while `text/markdown` is not.
    #[test]
    fn html_is_framed_even_though_it_is_text() {
        assert!(is_framed_content_type("text/html"));
        assert!(is_framed_content_type("application/pdf"));

        assert!(!is_framed_content_type("text/markdown"));
        assert!(!is_framed_content_type("text/plain"));
        assert!(
            !is_framed_content_type("image/png"),
            "an image is drawn, not framed"
        );
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

    /// The name is what decides, because the content type a browser reports for a zip is three different
    /// strings depending on which browser it is — and sometimes nothing at all.
    #[test]
    fn a_zip_is_recognised_by_its_name_first() {
        assert!(is_zip_upload("docs.zip", None));
        assert!(is_zip_upload("docs.ZIP", None));
        assert!(is_zip_upload(
            "a/b/docs.zip",
            Some("application/octet-stream")
        ));

        // No name to go on: the reported type is the fallback.
        assert!(is_zip_upload("archive", Some("application/zip")));
        assert!(is_zip_upload(
            "archive",
            Some("application/x-zip-compressed")
        ));

        assert!(!is_zip_upload("notes.md", Some("text/markdown")));
        assert!(!is_zip_upload("zipped", None));
        // `.zip` has to be the extension, not a piece of the name.
        assert!(!is_zip_upload("docs.zip.md", None));
    }

    #[test]
    fn the_name_is_the_last_segment() {
        assert_eq!(document_file_name("docs/design/system.md"), "system.md");
        assert_eq!(document_file_name("notes.md"), "notes.md");
    }
}
