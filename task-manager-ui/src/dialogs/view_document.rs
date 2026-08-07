use dioxus::prelude::*;
use task_manager_shared::documents::{DocumentReference, parse_document_reference};

/// The block a task or a goal draws for the documents it references.
///
/// **What it has is references, and nothing else.** The board snapshot carries the reference list and not the
/// documents, so this cannot show a size or a version without a request — and it deliberately does not make
/// one per card.
///
/// What a reference DOES carry is enough to draw a useful row, which is the difference a url made: a file in
/// a connected repository is drawn by its file name, with its path as the tooltip, because the reference
/// spells both out. A document of the project's own is still drawn by its id — its path is a row in a table
/// this screen has not fetched.
///
/// A click LEAVES the dialog for the Documents screen, rather than opening a second window over it. That is
/// not a compromise: a document can be a PDF, and a PDF wants the whole pane the browser's viewer needs — a
/// modal over a board is the one place it cannot have that. The selection is a url, so the trip is linkable
/// and the back button comes straight back here.
#[component]
pub fn DocumentRefs(ids: Vec<String>) -> Element {
    rsx! {
        div { class: "task-view-attr",
            div { class: "task-view-attr-label", "Documents" }
            div { class: "doc-refs",
                for reference in ids.iter() {
                    {
                        let reference = reference.clone();
                        let drawn = DrawnRef::of(&reference);

                        rsx! {
                            button {
                                class: "doc-ref",
                                key: "{reference}",
                                title: "{drawn.title}",
                                onclick: move |_| {
                                    // Closed first: the screen underneath is about to be replaced, and a dialog
                                    // left open would hang over the document that was asked for.
                                    super::close();
                                    navigator().push(crate::AppRoute::Documents { selected: reference.clone() });
                                },
                                span { class: "doc-ref-icon", "{drawn.icon}" }
                                span { class: "doc-ref-id", "{drawn.label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One reference as a row: what to show, and what to say about it on hover.
struct DrawnRef {
    icon: &'static str,
    label: String,
    title: String,
}

impl DrawnRef {
    /// **The two kinds are marked differently on purpose.** A file in a connected repository is somebody
    /// else's — it has no history here, and editing it writes into a working copy rather than into this
    /// board — and a reader who cannot tell the two apart on a card learns the difference at the worst
    /// possible moment. The mark is the same one the Documents tree uses for the same thing.
    fn of(reference: &str) -> Self {
        match parse_document_reference(reference) {
            Some(DocumentReference::Mirror { project, path }) => Self {
                icon: "🐙",
                label: task_manager_shared::documents::document_file_name(&path).to_string(),
                title: format!("{path} — in a repository connected to {project}. Open it"),
            },
            Some(DocumentReference::Own { project, id }) => Self {
                icon: "📄",
                label: id,
                title: format!("A document of {project}. Open it"),
            },
            // A bare id, which is what every reference stored before references were urls still is. Drawn
            // exactly as it always was rather than guessed at — the board this card is on is the board it
            // is on, and saying so adds nothing.
            None => Self {
                icon: "📄",
                label: reference.to_string(),
                title: "Open this document".to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name is what a reader is looking for, and a reference to a repository's file carries one — which
    /// is the difference between a row that says `system.md` and a row that says a 25-character id.
    #[test]
    fn a_repositorys_file_is_drawn_by_its_name() {
        let drawn = DrawnRef::of("raw/TM/github/specs/design/system.md");

        assert_eq!(drawn.label, "system.md");
        assert_eq!(drawn.icon, "🐙");
        // The path is on the tooltip, because two repositories can both hold a `README.md`.
        assert!(
            drawn.title.contains("github/specs/design/system.md"),
            "the path is what tells two of them apart: {}",
            drawn.title
        );
    }

    /// A document of the project's own has no name to draw until somebody fetches it, so the id is still
    /// what the row shows — with or without the url around it.
    #[test]
    fn a_document_of_the_projects_own_is_drawn_by_its_id() {
        for spelling in [
            "raw/TM/document/01K2C4Q0S1T2U3V4W5X6Y7Z8",
            // The legacy spelling, still on cards written before references were urls.
            "01K2C4Q0S1T2U3V4W5X6Y7Z8",
        ] {
            let drawn = DrawnRef::of(spelling);

            assert_eq!(drawn.label, "01K2C4Q0S1T2U3V4W5X6Y7Z8");
            assert_eq!(drawn.icon, "📄");
        }
    }
}
