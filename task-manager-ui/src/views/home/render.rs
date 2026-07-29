use dioxus::prelude::*;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::{COLUMN_ID_DONE, COLUMN_ID_TODO, ProjectResponse};
use task_manager_shared::tasks::TaskResponse;

use crate::states::AppState;

/// The board.
///
/// **Read-only, deliberately.** Nothing here is dragged, clicked to edit or double-clicked: every change
/// to a task arrives through `/mcp`. What this screen owes the reader is an accurate picture, which is
/// why it repaints from a WebSocket push rather than hoping somebody reloads.
#[component]
pub fn RenderHome() -> Element {
    let app_state = consume_context::<Signal<AppState>>();

    let mut projects = use_signal(Vec::<ProjectResponse>::new);
    let mut tasks = use_signal(Vec::<TaskResponse>::new);
    let mut selected = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut loading = use_signal(|| true);

    // The project list is read once. It changes only through Projects setup, which is a page navigation
    // away — and any change to it also bumps the board revision, which re-reads below.
    use_future(move || async move {
        match crate::api::get_projects().await {
            Ok(response) => {
                let remembered = crate::web::storage::get_last_project();

                let initial = remembered
                    .filter(|id| response.projects.iter().any(|itm| &itm.id == id))
                    .or_else(|| response.projects.first().map(|itm| itm.id.clone()))
                    .unwrap_or_default();

                projects.set(response.projects);
                selected.set(initial);
                loading.set(false);
            }
            Err(err) => {
                error.set(err.message);
                loading.set(false);
            }
        }
    });

    // Re-reads on two triggers: the person picked another board, or the server said this one changed.
    // Reading `board_revision` inside the future is what subscribes to it.
    use_future(move || async move {
        let _ = app_state.read().board_revision;
        let project_id = selected.read().clone();

        if project_id.is_empty() {
            tasks.set(Vec::new());
            return;
        }

        match crate::api::get_tasks(&project_id).await {
            Ok(response) => {
                tasks.set(response.tasks);
                error.set(String::new());
            }
            Err(err) => error.set(err.message),
        }
    });

    // Tell the socket which board to push about. Sent on every change of selection, which is also what
    // makes switching projects not need a reconnect.
    use_effect(move || {
        let project_id = selected.read().clone();

        if !project_id.is_empty() {
            crate::web::storage::save_last_project(&project_id);
            crate::views::home::watch_project(&project_id);
        }
    });

    let projects_ra = projects.read();
    let selected_id = selected.read().clone();
    let error_text = error.read().clone();
    let ws_live = app_state.read().ws_live;

    if *loading.read() {
        return rsx! {
            div { class: "loading-note", "Loading…" }
        };
    }

    if projects_ra.is_empty() {
        return rsx! {
            div { class: "page-header",
                h1 { class: "page-title", "Home" }
            }
            div { class: "empty-note",
                "You are not on any project yet. Ask an admin to add you to one."
            }
        };
    }

    let current = projects_ra
        .iter()
        .find(|itm| itm.id == selected_id)
        .or_else(|| projects_ra.first());

    let Some(current) = current else {
        return rsx! {
            div { class: "empty-note", "No project selected." }
        };
    };

    let tasks_ra = tasks.read();

    // A flex column filling what is left of the window, so the board below it can be full height and each
    // of its columns can scroll on its own.
    rsx! {
        div { class: "board-page",
            div { class: "page-header",
                h1 { class: "page-title", "Home" }
                div { class: "project-picker",
                    select {
                        value: "{selected_id}",
                        onchange: move |event| selected.set(event.value()),
                        for project in projects_ra.iter() {
                            option { value: "{project.id}", "{project.prefix} · {project.name}" }
                        }
                    }
                    span {
                        class: if ws_live { "ws-dot live" } else { "ws-dot" },
                        title: if ws_live {
                            "Live — the board repaints when it changes"
                        } else {
                            "Not live — reload to see changes"
                        },
                    }
                }
            }

            if !current.description.trim().is_empty() {
                p { class: "page-note", "{current.description}" }
            }

            if !error_text.is_empty() {
                div { class: "error-banner", "{error_text}" }
            }

            div { class: "board",
                for column in board_columns(current) {
                    RenderColumn {
                        key: "{column.id}",
                        column: column.clone(),
                        project: current.clone(),
                        tasks: tasks_ra.clone(),
                    }
                }
            }
        }
    }
}

/// One column as the board draws it, anchors included.
#[derive(Clone, PartialEq)]
pub struct BoardColumn {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// The project's configured columns with the two anchors put back at either end.
///
/// The wire model leaves `todo` and `done` out — they exist in every project by definition — so every
/// reader adds them, and this is where this one does it.
fn board_columns(project: &ProjectResponse) -> Vec<BoardColumn> {
    let mut result = Vec::with_capacity(project.columns.len() + 2);

    result.push(BoardColumn {
        id: COLUMN_ID_TODO.to_string(),
        name: "Todo".to_string(),
        description: String::new(),
    });

    let mut middle: Vec<&task_manager_shared::projects::ProjectColumnResponse> =
        project.columns.iter().collect();
    middle.sort_by_key(|itm| itm.order);

    for column in middle {
        result.push(BoardColumn {
            id: column.id.clone(),
            name: column.name.clone(),
            description: column.description.clone(),
        });
    }

    result.push(BoardColumn {
        id: COLUMN_ID_DONE.to_string(),
        name: "Done".to_string(),
        description: String::new(),
    });

    result
}

#[component]
fn RenderColumn(
    column: BoardColumn,
    project: ProjectResponse,
    tasks: Vec<TaskResponse>,
) -> Element {
    // The server has already folded an unknown status into `todo`, so a plain comparison is enough here —
    // the leniency lives in one place rather than being re-implemented per client.
    let in_column: Vec<&TaskResponse> =
        tasks.iter().filter(|itm| itm.status == column.id).collect();

    rsx! {
        div { class: "board-column",
            div { class: "board-column-header",
                span { class: "board-column-name", "{column.name}" }
                span { class: "board-column-count", "{in_column.len()}" }
            }
            if !column.description.trim().is_empty() {
                div { class: "board-column-description", "{column.description}" }
            }
            // The only part that scrolls. The header and the description stay in place, so which column
            // you are looking at is still answerable once the cards have moved.
            div { class: "board-column-body",
                for task in in_column {
                    RenderSticker { key: "{task.id}", task: task.clone(), project: project.clone() }
                }
            }
        }
    }
}

#[component]
fn RenderSticker(task: TaskResponse, project: ProjectResponse) -> Element {
    let kind = task
        .kind
        .as_ref()
        .and_then(|kind_id| project.kinds.iter().find(|itm| &itm.id == kind_id));

    let kind_color = kind
        .map(|itm| KindColor::parse_or_default(&itm.color))
        .unwrap_or_default();

    // Rendered rather than shown as source: agents write Markdown, and a checklist as literal dashes is
    // markedly harder to read. `markdown::to_html` escapes raw HTML instead of passing it through, which
    // is what makes rendering text this side did not author safe.
    let text_html = markdown::to_html(&task.text);

    let assignee = task
        .assignee_name
        .clone()
        .or_else(|| task.assignee.clone())
        .unwrap_or_else(|| "Unassigned".to_string());

    let border = kind
        .map(|_| format!("border-left-color: {}", kind_color.hex()))
        .unwrap_or_default();

    // Only shown in Done, and only as "how long ago" — the exact timestamp is noise on a sticker, while
    // the age is the thing that matters, because it says how close the task is to leaving the board.
    let closed_note = task.closed_unix_seconds.map(closed_how_long_ago);

    rsx! {
        div {
            class: if task.blocked { "sticker blocked" } else { "sticker" },
            style: "{border}",

            div { class: "sticker-top",
                span { class: "sticker-id", "{task.id}" }
                if let Some(kind) = kind {
                    span {
                        class: "sticker-kind",
                        style: "background: {kind_color.hex()}",
                        title: "{kind.description}",
                        // Only when this build actually has the file: a name stored before an icon was
                        // renamed away draws as no icon rather than as a broken image.
                        if crate::web::icon_exists(&kind.icon) {
                            img { class: "sticker-kind-icon", src: "{crate::web::icon_url(&kind.icon)}", alt: "" }
                        }
                        "{kind.name}"
                    }
                }
                if task.blocked {
                    span { class: "sticker-blocked-flag", "Blocked" }
                }
            }

            div { class: "sticker-text", dangerous_inner_html: "{text_html}" }

            if !task.depends_on.is_empty() {
                div { class: "sticker-deps", "Waiting on {task.depends_on.join(\", \")}" }
            }

            div { class: "sticker-bottom",
                span { class: "sticker-assignee", "{assignee}" }
                if let Some(closed) = closed_note {
                    span { title: "Work closed more than seven days ago leaves the board", "{closed}" }
                }
                if !task.comments.is_empty() {
                    span { "💬 {task.comments.len()}" }
                }
                for label in task.labels.iter() {
                    span { class: "tag", "{label}" }
                }
            }
        }
    }
}

/// "Closed today" / "Closed 3 days ago" — the age, not the timestamp.
///
/// Days rather than hours because the archive window is measured in days, so the number on the sticker and
/// the reason it will disappear are the same number.
fn closed_how_long_ago(closed_unix_seconds: i64) -> String {
    let now = js_sys::Date::now() as i64 / 1_000;
    let days = (now - closed_unix_seconds).max(0) / (24 * 60 * 60);

    match days {
        0 => "Closed today".to_string(),
        1 => "Closed yesterday".to_string(),
        _ => format!("Closed {days} days ago"),
    }
}
