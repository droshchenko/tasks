use dioxus::prelude::*;
use task_manager_shared::documents::{document_file_name, normalise_document_path, render_size};

/// What an upload needs, once the reader has chosen it.
#[derive(Clone, PartialEq)]
pub struct UploadSubmit {
    pub project: String,
    pub path: String,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Default)]
struct ComponentState {
    /// The folder the document goes in. Typed, and a dropdown of the folders that exist writes into it — so
    /// choosing an existing one and creating a new one are the same control rather than two that disagree.
    folder: String,
    /// The name it gets. Seeded from the chosen file and then editable: uploading `report (3).pdf` under a
    /// sensible name is the common case, and renaming afterwards is an MCP call.
    name: String,
    file: Option<PickedFile>,
    reading: bool,
}

#[derive(Clone, PartialEq)]
struct PickedFile {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

/// Upload a file into a project's documents.
///
/// **The one dialog in this product that writes something other than a colour**, and it exists because the
/// alternative is not a person using MCP — it is a person unable to upload at all: a PDF on a laptop cannot
/// reach an agent without being base64-ed by hand into a tool call.
///
/// Folders are not created here because folders do not exist: they are read off the paths of the documents in
/// them. So "make a folder" and "choose a folder" are one act — type a path — and the dropdown beside it is a
/// shortcut that fills the same box with one that is already in use.
#[component]
pub fn UploadDocumentDialog(
    project: String,
    folders: Vec<String>,
    on_submit: EventHandler<UploadSubmit>,
) -> Element {
    let mut cs = use_signal(ComponentState::default);
    let cs_ra = cs.read();

    let feedback = super::feedback();

    let folder = cs_ra.folder.clone();
    let name = cs_ra.name.clone();
    let picked = cs_ra.file.clone();
    let reading = cs_ra.reading;

    // What the document will actually be called, worked out the same way the server will work it out — so the
    // reader is shown the answer rather than the inputs to it.
    let full_path = join_path(&folder, &name);
    let path_problem = match full_path.as_deref().map(normalise_document_path) {
        Some(Err(problem)) => Some(problem),
        _ => None,
    };

    let ready = picked.is_some() && path_problem.is_none() && full_path.is_some() && !reading;

    let content = rsx! {
        div { class: "upload-form",
            div { class: "field",
                label { class: "field-label", "Folder" }
                div { class: "upload-folder",
                    input {
                        class: "input",
                        placeholder: "docs/design — leave empty for the top level",
                        value: "{folder}",
                        oninput: move |event| cs.write().folder = event.value(),
                    }
                    // A shortcut into the box beside it, not a second source of truth: picking here types the
                    // folder, and it can then be edited into a new one.
                    if !folders.is_empty() {
                        select {
                            class: "input upload-folder-pick",
                            onchange: move |event| cs.write().folder = event.value(),
                            option { value: "", "— existing folders —" }
                            for existing in folders.iter() {
                                option { value: "{existing}", "{existing}" }
                            }
                        }
                    }
                }
                div { class: "field-hint",
                    "Folders are not stored — a folder exists for as long as a document is in it."
                }
            }

            div { class: "field",
                label { class: "field-label", "File" }
                input {
                    r#type: "file",
                    onchange: move |event| {
                        let Some(file) = event.files().into_iter().next() else {
                            return;
                        };

                        let file_name = file.name();
                        let content_type = file.content_type().filter(|itm| !itm.trim().is_empty());

                        // The name is seeded here rather than when the bytes land, so the box fills the
                        // instant the file is chosen instead of after a large read.
                        {
                            let mut write = cs.write();
                            write.reading = true;

                            if write.name.trim().is_empty() {
                                write.name = document_file_name(&file_name).to_string();
                            }
                        }

                        spawn(async move {
                            match file.read_bytes().await {
                                Ok(bytes) => {
                                    let mut write = cs.write();
                                    write.reading = false;
                                    write.file = Some(PickedFile {
                                        bytes: bytes.to_vec(),
                                        content_type,
                                    });
                                }
                                Err(_) => {
                                    let mut write = cs.write();
                                    write.reading = false;
                                    write.file = None;
                                }
                            }
                        });
                    },
                }

                if reading {
                    div { class: "field-hint", "Reading the file…" }
                } else if let Some(picked) = picked.as_ref() {
                    div { class: "field-hint",
                        "{render_size(picked.bytes.len() as i64)}"
                        if let Some(content_type) = picked.content_type.as_ref() {
                            " · {content_type}"
                        }
                    }
                }
            }

            div { class: "field",
                label { class: "field-label", "Name" }
                input {
                    class: "input",
                    placeholder: "system.md",
                    value: "{name}",
                    oninput: move |event| cs.write().name = event.value(),
                }
            }

            // The path as it will be stored, said out loud. Uploading onto a path that is taken writes a new
            // version of what is there — which is the right behaviour and a surprise if nobody showed you the
            // path first.
            if let Some(problem) = path_problem {
                div { class: "error-note", "{problem}" }
            } else if let Some(full_path) = full_path.as_ref() {
                div { class: "field-hint", "Will be stored as {full_path}" }
            }

            if !feedback.error.is_empty() {
                div { class: "error-note", "{feedback.error}" }
            }
        }
    };

    let ok = rsx! {
        button {
            class: "btn btn-primary",
            disabled: !ready || feedback.saving,
            onclick: move |_| {
                let cs_ra = cs.read();

                let (Some(file), Some(path)) = (
                    cs_ra.file.clone(),
                    join_path(&cs_ra.folder, &cs_ra.name),
                ) else {
                    return;
                };

                drop(cs_ra);

                on_submit.call(UploadSubmit {
                    project: project.clone(),
                    path,
                    bytes: file.bytes,
                    content_type: file.content_type,
                });
            },
            if feedback.saving { "Uploading…" } else { "Upload" }
        }
    };

    super::dialog_template("Upload a document", content, ok)
}

/// A folder and a name into one path. `None` when there is no name — a document has to be called something.
///
/// The server normalises what this produces, so a trailing slash on the folder or a stray space is not an
/// error here: it is written the way it was typed and cleaned up in one place.
fn join_path(folder: &str, name: &str) -> Option<String> {
    let name = name.trim();

    if name.is_empty() {
        return None;
    }

    let folder = folder.trim().trim_matches('/');

    if folder.is_empty() {
        Some(name.to_string())
    } else {
        Some(format!("{folder}/{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_and_a_name_make_a_path() {
        assert_eq!(join_path("docs", "a.md").as_deref(), Some("docs/a.md"));
        assert_eq!(join_path("", "a.md").as_deref(), Some("a.md"));
        assert_eq!(join_path("  ", " a.md ").as_deref(), Some("a.md"));

        // A slash the reader typed either way round is not an error — it is the same folder.
        assert_eq!(join_path("/docs/", "a.md").as_deref(), Some("docs/a.md"));
        assert_eq!(
            join_path("docs/design", "a.md").as_deref(),
            Some("docs/design/a.md")
        );
    }

    #[test]
    fn without_a_name_there_is_no_path() {
        assert_eq!(join_path("docs", ""), None);
        assert_eq!(join_path("docs", "   "), None);
    }
}
