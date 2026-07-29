use dioxus::prelude::*;
use task_manager_shared::projects::COLUMN_ID_DONE;
use task_manager_shared::tasks::{FindTaskResponse, TaskResponse};

/// One task, shown in full.
///
/// Read-only, like the board: every change to a task arrives through `/mcp`. What this owes the reader is the
/// whole card — text, attributes, thread — for a task that may not be on the board at all, because it belongs
/// to another project or was closed more than seven days ago.
///
/// **Laid out as four fixed areas, not as one scrolling document**: the text top left, the attributes in a
/// column of their own to its right, and the thread across the bottom half. Each half scrolls on its own, so
/// a task with forty comments still shows its text, and a task with a long text still shows that people have
/// been talking about it.
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

    // Its own size class rather than `modal-lg`: this one takes 95% of the window. A dialog sized to its
    // content has no height for the two halves to be halves OF, and a task is the one thing on this side
    // worth the whole screen.
    super::dialog_template_ex(&title, content, rsx! {}, Some("modal-task"))
}

/// "Closed today" / "Closed 3 days ago" — the age, not the timestamp.
///
/// Days rather than hours because the archive window is measured in days, so the number shown and the reason
/// the task will disappear are the same number.
fn closed_how_long_ago(closed_unix_seconds: i64) -> String {
    let now = js_sys::Date::now() as i64 / 1_000;
    let days = (now - closed_unix_seconds).max(0) / (24 * 60 * 60);

    match days {
        0 => "Closed today".to_string(),
        1 => "Closed yesterday".to_string(),
        _ => format!("Closed {days} days ago"),
    }
}

fn render_task(task: &TaskResponse, found: &FindTaskResponse) -> Element {
    // Rendered rather than shown as source: agents write Markdown, and a checklist as literal dashes is
    // markedly harder to read. `markdown::to_html` escapes raw HTML instead of passing it through, which is
    // what makes rendering text this side did not author safe.
    let text_html = markdown::to_html(&task.text);

    rsx! {
        div { class: "task-view",
            div { class: "task-view-top",
                div { class: "task-view-text", dangerous_inner_html: "{text_html}" }
                {render_attributes(task, found)}
            }
            {render_thread(task)}
        }
    }
}

/// Everything about the task that is not its text, in one narrow column.
///
/// A column rather than a row of tags across the top: these are labelled values and there are eight of them
/// at most, which reads as a list. It also keeps them out of the text's way — the text is what the reader
/// came for and it gets the width.
fn render_attributes(task: &TaskResponse, found: &FindTaskResponse) -> Element {
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
        div { class: "task-view-attrs",
            div { class: "task-view-attr",
                div { class: "task-view-attr-label", "Status" }
                span { class: "tag", "{status}" }
            }

            // The type's id, uncoloured. Its colour and icon live on the project's type list, which this
            // response does not carry — and a lookup by id can land on a project that is not on screen, so
            // there is nothing to read them from. Better a plain tag than a wrong colour.
            if let Some(kind) = task.kind.as_ref() {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Type" }
                    span { class: "tag", "{kind}" }
                }
            }

            div { class: "task-view-attr",
                div { class: "task-view-attr-label", "Assignee" }
                div { "{assignee}" }
            }

            if let Some(goal) = task.goal_name.as_ref().or(task.goal_id.as_ref()) {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Goal" }
                    div { "{goal}" }
                }
            }

            if task.blocked {
                div { class: "task-view-attr",
                    span { class: "sticker-blocked-flag", "Blocked" }
                }
            }

            if !task.labels.is_empty() {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Labels" }
                    div { class: "task-view-attr-tags",
                        for label in task.labels.iter() {
                            span { class: "tag", "{label}" }
                        }
                    }
                }
            }

            if !task.depends_on.is_empty() {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Waiting on" }
                    div { class: "mono", "{task.depends_on.join(\", \")}" }
                }
            }

            if !task.blocks.is_empty() {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Blocking" }
                    div { class: "mono", "{task.blocks.join(\", \")}" }
                }
            }

            // Only in Done, and only as an age. The exact timestamp says nothing a reader wants; how long ago
            // it closed is the same number as how close it is to leaving the board.
            if let Some(closed) = task.closed_unix_seconds.map(closed_how_long_ago) {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Closed" }
                    div { title: "Work closed more than seven days ago leaves the board", "{closed}" }
                }
            }

            if found.archived {
                div { class: "field-hint",
                    "Closed more than seven days ago, so it is not on the board any more — it is still here, and still reachable by id."
                }
            }
        }
    }
}

/// The bottom half: what people have said, oldest first.
fn render_thread(task: &TaskResponse) -> Element {
    rsx! {
        div { class: "task-view-bottom",
            div { class: "task-view-thread-header",
                "Comments"
                span { class: "board-column-count", "{task.comments.len()}" }
            }

            if task.comments.is_empty() {
                div { class: "field-hint", "No comments." }
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
}
