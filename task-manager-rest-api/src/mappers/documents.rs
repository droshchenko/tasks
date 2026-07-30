use task_manager_shared::documents::{DocumentIndexEntryResponse, DocumentResponse};

use crate::postgres::DocumentDto;

/// Postgres -> wire, with the content.
///
/// There is no memory model in between, unlike every other type in this module — a document is never held in
/// `Board`, so the row IS the model and inventing a copy of it would only be a shape to keep in sync.
pub fn document_to_response(src: &DocumentDto) -> DocumentResponse {
    DocumentResponse {
        id: src.id.clone(),
        project_id: src.project_id.clone(),
        path: src.doc_path.clone(),
        content: src.content.clone(),
        version: src.version,
        created_unix_seconds: src.created.unix_microseconds / 1_000_000,
        updated_unix_seconds: src.updated.unix_microseconds / 1_000_000,
        updated_by: src.updated_by.clone(),
    }
}

/// Postgres -> wire, WITHOUT the content.
///
/// The whole reason the index is a separate shape: a project's tree is drawn from paths, and shipping every
/// text to draw a list of names would undo the one decision documents are built around — that a document is
/// read one at a time, on request.
///
/// `size` is the character count of the content that is being left out, which is what a reader wants from a
/// list: whether this is a note or a specification.
pub fn document_to_index_entry(src: &DocumentDto) -> DocumentIndexEntryResponse {
    DocumentIndexEntryResponse {
        id: src.id.clone(),
        project_id: src.project_id.clone(),
        path: src.doc_path.clone(),
        version: src.version,
        size: src.content.chars().count() as i64,
        updated_unix_seconds: src.updated.unix_microseconds / 1_000_000,
        updated_by: src.updated_by.clone(),
    }
}
