use dioxus::prelude::*;
use task_manager_shared::goals::GoalResponse;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::COLUMN_ID_DONE;

/// One goal, shown in full: what it is about, and what has been said about it.
///
/// Read-only, like every other view here — a goal is opened, renamed and closed through `/mcp`. Laid out as
/// the task dialog is, and deliberately with its classes rather than a parallel set: the two are the same
/// shape of thing (a text, some attributes, a thread), and two stylesheets for one shape drift.
///
/// The thread matters more here than on a task. A goal is where a conversation happens and tasks come out of
/// it, so `goals_add_comment` is the record of WHY the work under it looks the way it does — and until this
/// dialog existed there was nowhere in the browser to read it.
#[component]
pub fn ViewGoalDialog(goal: GoalResponse) -> Element {
    let title = format!("{} · {}", goal.id, goal.name);
    let content = render_goal(&goal);

    // The same size class the task dialog uses: two halves need a height to be halves of, and a goal with a
    // long thread is exactly as worth the window as a task with one.
    super::dialog_template_read_only(&title, content, Some("modal-task"))
}

fn render_goal(goal: &GoalResponse) -> Element {
    // Rendered rather than shown as source, and escaped rather than trusted — the same reason and the same
    // call as a task's text: agents write Markdown, and `md_to_html` escapes raw HTML instead of passing it
    // through.
    let description_html = super::md_to_html(&goal.description);
    let has_description = !goal.description.trim().is_empty();

    rsx! {
        div { class: "task-view",
            div { class: "task-view-top",
                // One scrolling column for the description and the checklist under it, exactly as the task
                // dialog arranges the same two things — see the note there for why it is wrapped even when
                // there is no checklist.
                div { class: "task-view-left",
                    if has_description {
                        div { class: "task-view-text md", dangerous_inner_html: "{description_html}" }
                    } else {
                        // Said rather than left blank: a goal with no text is a goal whose shape lives in
                        // its thread, which is normal — the conversation comes first and the summary often
                        // never gets written.
                        div { class: "field-hint",
                            "No description. What this goal is about may be in the comments below."
                        }
                    }

                    if !goal.subtasks.is_empty() {
                        super::Checklist { items: goal.subtasks.clone() }
                    }
                }
                {render_attributes(goal)}
            }
            {render_thread(goal)}
        }
    }
}

/// The right-hand column: the goal's state, its progress, and its colour.
fn render_attributes(goal: &GoalResponse) -> Element {
    let closed = goal.closed_unix_seconds.is_some();
    let hex = KindColor::parse_or_default(&goal.color).hex();
    let priority = task_manager_shared::priority::Priority::parse_or_default(&goal.priority);

    let status = if goal.status == COLUMN_ID_DONE {
        "Closed".to_string()
    } else {
        "Open".to_string()
    };

    // The number the server counted, which includes archived work — not a count of anything on this screen.
    // A goal closes only once every task is done, so by then the oldest of them have aged off the board, and
    // a figure recomputed from a board read would report finished work as half-done.
    let progress = format!("{} of {} done", goal.done_amount, goal.tasks_amount);

    rsx! {
        div { class: "task-view-attrs",
            div { class: "task-view-attr",
                div { class: "task-view-attr-label", "Status" }
                span { class: "tag", "{status}" }
            }

            // Always, Normal included — the same reason as on the task dialog: this is the place a fact is
            // looked up, and a row that vanishes is ambiguous where a row saying "Normal" is not.
            div { class: "task-view-attr",
                div { class: "task-view-attr-label", "Priority" }
                span {
                    class: "sticker-priority",
                    style: "background: {priority.hex()}",
                    "{priority.title()}"
                }
            }

            div { class: "task-view-attr",
                div { class: "task-view-attr-label", "Progress" }
                div { "{progress}" }
            }

            div { class: "task-view-attr",
                div { class: "task-view-attr-label", "Colour" }
                // The swatch itself, not its name: the colour is what the board marks this goal's cards
                // with, and a word for it is one indirection more than a person needs.
                div { class: "task-view-goal", style: "background: {hex}",
                    span { class: "task-view-goal-id", "{goal.id}" }
                }
            }

            // A goal is where a decision is written down, so a document attached to one is usually the
            // decision itself — see the note in the task dialog for why these are ids and not paths.
            if !goal.documents.is_empty() {
                super::DocumentRefs { project: goal.project.clone(), ids: goal.documents.clone() }
            }

            if closed {
                div { class: "task-view-attr",
                    div { class: "task-view-attr-label", "Closed" }
                    div {
                        "The resolution is the last comment below."
                    }
                }
            }
        }
    }
}

/// The bottom half: the conversation, oldest first. The reason the work is shaped the way it is.
fn render_thread(goal: &GoalResponse) -> Element {
    rsx! {
        div { class: "task-view-bottom",
            div { class: "task-view-thread-header",
                "Comments"
                span { class: "board-column-count", "{goal.comments.len()}" }
            }

            if goal.comments.is_empty() {
                div { class: "field-hint", "Nothing has been said about this goal yet." }
            } else {
                div { class: "task-view-thread",
                    for (index , comment) in goal.comments.iter().enumerate() {
                        div { class: "task-view-comment", key: "{index}",
                            div { class: "task-view-comment-who", "{comment.who}" }
                            div { class: "md", dangerous_inner_html: "{super::md_to_html(&comment.text)}" }
                        }
                    }
                }
            }
        }
    }
}
