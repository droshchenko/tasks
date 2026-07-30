use dioxus::prelude::*;
use dioxus_utils::RenderState;
use task_manager_shared::documents::{DocumentIndexEntryResponse, FindDocumentResponse};

use super::DocumentsState;

/// The icons that actually exist under `public/assets/file-types`, keyed by extension.
///
/// An explicit list rather than "try `<ext>.svg` and hope": a missing file renders as a broken-image glyph,
/// which looks like a bug in the tree rather than like an extension nobody has drawn yet. Adding an icon is
/// adding its file and one line here.
const FILE_ICONS: [&str; 6] = ["css", "html", "md", "pdf", "png", "toml"];

const ICON_DIR: &str = "/assets/file-types";

pub fn file_icon(name: &str) -> String {
    match extension(name) {
        Some(extension) if FILE_ICONS.contains(&extension.as_str()) => {
            format!("{ICON_DIR}/{extension}.svg")
        }
        _ => format!("{ICON_DIR}/file.svg"),
    }
}

pub fn folder_icon(expanded: bool) -> String {
    if expanded {
        format!("{ICON_DIR}/folder-open.svg")
    } else {
        format!("{ICON_DIR}/folder.svg")
    }
}

fn extension(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;

    // `.gitignore` is a name, not an extension of nothing.
    if stem.is_empty() {
        return None;
    }

    Some(extension.to_lowercase())
}

/// The project's whole index, loaded the first time it is asked for.
///
/// One request for the lot, unlike the file browser this screen is modelled on: that one walks a real
/// filesystem a folder at a time, where this one is answered from an index the server holds in memory. What is
/// NOT in it is the payloads — that is the whole point of the split.
pub fn get_index<'s>(
    mut cs: Signal<DocumentsState>,
    cs_ra: &'s DocumentsState,
) -> Result<&'s [DocumentIndexEntryResponse], Element> {
    if cs_ra.selected_project.is_empty() {
        return Ok(&[]);
    }

    match cs_ra.index.as_ref() {
        RenderState::None => {
            let project = cs_ra.selected_project.clone();

            spawn(async move {
                cs.write().index.set_loading();

                match crate::api::get_documents(&project).await {
                    Ok(response) => cs.write().index.set_loaded(response.documents),
                    Err(err) => cs.write().index.set_error(err.message),
                }
            });

            Err(render_tree_note("loading…", false))
        }
        RenderState::Loading => Err(render_tree_note("loading…", false)),
        RenderState::Loaded(index) => Ok(index.as_slice()),
        RenderState::Error(err) => Err(render_tree_note(err.as_str(), true)),
    }
}

/// The selected document, text included.
///
/// Which document that is comes from the url, so the state can be holding a different one — the check is
/// against the id rather than against "has anything been loaded", or a click on a second document would show
/// the first one's text under the second one's name.
///
/// A BINARY document is fetched too, and comes back with no content: the response carries its type and size,
/// and the viewer points an `<iframe>` or an `<img>` at the raw endpoint. Nothing here ever holds bytes.
pub fn get_content<'s>(
    mut cs: Signal<DocumentsState>,
    cs_ra: &'s DocumentsState,
    project: &str,
    id: &str,
) -> Result<&'s FindDocumentResponse, Element> {
    if !cs_ra.content_is_for(id) {
        let project = project.to_string();
        let id = id.to_string();

        spawn(async move {
            cs.write().begin_content_load(&id);

            match crate::api::get_document(&project, &id).await {
                Ok(found) => cs.write().content.set_loaded(found),
                Err(err) => cs.write().content.set_error(err.message),
            }
        });

        return Err(render_viewer_note("loading…", false));
    }

    match cs_ra.content.as_ref() {
        RenderState::None | RenderState::Loading => Err(render_viewer_note("loading…", false)),
        RenderState::Loaded(found) => Ok(found),
        RenderState::Error(err) => Err(render_viewer_note(err.as_str(), true)),
    }
}

pub fn render_tree_note(text: &str, failed: bool) -> Element {
    let class = if failed {
        "tree-note failed"
    } else {
        "tree-note"
    };

    rsx! {
        div { class: "{class}", "{text}" }
    }
}

pub fn render_viewer_note(text: &str, failed: bool) -> Element {
    let class = if failed {
        "viewer-note failed"
    } else {
        "viewer-note"
    };

    rsx! {
        div { class: "{class}", "{text}" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_generic_icon_for_an_extension_nobody_has_drawn() {
        assert_eq!(file_icon("spec.pdf"), "/assets/file-types/pdf.svg");
        assert_eq!(file_icon("NOTES.MD"), "/assets/file-types/md.svg");
        assert_eq!(file_icon("logo.png"), "/assets/file-types/png.svg");

        // Only `.png` is drawn — this maps a name to a file, not a kind to a picture.
        assert_eq!(file_icon("logo.jpg"), "/assets/file-types/file.svg");
        assert_eq!(file_icon("Makefile"), "/assets/file-types/file.svg");
        assert_eq!(file_icon(".gitignore"), "/assets/file-types/file.svg");
    }
}
