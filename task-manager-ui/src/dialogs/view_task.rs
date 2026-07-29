use dioxus::prelude::*;
use task_manager_shared::projects::COLUMN_ID_DONE;
use task_manager_shared::tasks::{FindTaskResponse, TaskResponse};

/// One task, looked up by its id and shown in full.
///
/// Read-only, like the board: every change to a task arrives through `/mcp`. What this owes the reader is
/// the whole card — text, thread, dependencies — for a task that may not be on the board at all, because it
/// belongs to another project or was closed more than seven days ago.
#[component]
pub fn ViewTaskDialog(found: FindTaskResponse) -> Element {
    let Some(task) = found.task.clone() else {
        // A miss carries the server's own message, which says what would fix it — a bad shape, an unknown
        // prefix with the known ones listed, or a number that is not on that board.
        let reason = if found.not_found.is_empty() {
            "Nothing found.".to_string()
        } else {
            found.not_found.clone()
        };

        return super::dialog_template(
            "Not found",
            rsx! {
                div { class: "empty-note", "{reason}" }
            },
            rsx! {},
        );
    };

    let title = format!("{} · {}", task.id, found.project_name);
    let content = render_task(&task, &found);

    super::dialog_template_ex(&title, content, rsx! {}, Some("modal-lg"))
}

fn render_task(task: &TaskResponse, found: &FindTaskResponse) -> Element {
    // Rendered rather than shown as source: agents write Markdown, and a checklist as literal dashes is
    // markedly harder to read. `markdown::to_html` escapes raw HTML instead of passing it through, which is
    // what makes rendering text this side did not author safe.
    let text_html = markdown::to_html(&task.text);

    let status = if task.status == COLUMN_ID_DONE {
        "Done".to_string()
    } else {
        task.status.clone()
    };

    let assignee = task
        .assignee_name
        .clone()
        .or_else(|| task.assignee.clone())
        .unwrap_or_else(|| "Unassigned".to_string());

    rsx! {
        if found.archived {
            div { class: "field-hint",
                "Closed more than seven days ago, so it is not on the board any more — it is still here, and still reachable by id."
            }
        }

        div { class: "task-view-meta",
            span { class: "tag", "{status}" }
            // The type's id, uncoloured. Its colour and icon live on the project's type list, which this
            // response does not carry — and a lookup by id can land on a project that is not on screen, so
            // there is nothing to read them from. Better a plain tag than a wrong colour.
            if let Some(kind) = task.kind.as_ref() {
                span { class: "tag", "{kind}" }
            }
            span { class: "muted", "{assignee}" }
            if task.blocked {
                span { class: "sticker-blocked-flag", "Blocked" }
            }
            for label in task.labels.iter() {
                span { class: "tag", "{label}" }
            }
        }

        div { class: "task-view-text", dangerous_inner_html: "{text_html}" }

        if !task.depends_on.is_empty() {
            div { class: "field-hint", "Waiting on {task.depends_on.join(\", \")}" }
        }
        if !task.blocks.is_empty() {
            div { class: "field-hint", "Blocking {task.blocks.join(\", \")}" }
        }

        if task.comments.is_empty() {
            div { class: "field-hint", style: "margin-top: 12px", "No comments." }
        } else {
            div { class: "task-view-thread",
                for (index , comment) in task.comments.iter().enumerate() {
                    div { class: "task-view-comment", key: "{index}",
                        div { class: "task-view-comment-who", "{comment.who}" }
                        div { dangerous_inner_html: "{markdown::to_html(&comment.text)}" }
                    }
                }
            }
        }
    }
}
