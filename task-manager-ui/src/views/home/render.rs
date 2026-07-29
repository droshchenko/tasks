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
    let mut search = use_signal(String::new);
    // Empty means "any". Kept as ids rather than indexes so a board that changes under a filter cannot
    // silently move it to a different type.
    let mut kind_filter = use_signal(String::new);
    let mut assignee_filter = use_signal(String::new);

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
    let search_text = search.read().clone();
    let kind_wanted = kind_filter.read().clone();
    let assignee_wanted = assignee_filter.read().clone();

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

    // Every filter narrows the same list, so they compose: a type AND an assignee AND some text. A task id
    // is the exception and narrows nothing — the answer to an id may be on another board or off this one
    // entirely, so it goes through the server and opens as a card.
    let needle = if looks_like_a_task_id(search_text.trim()) {
        String::new()
    } else {
        search_text.trim().to_lowercase()
    };

    let visible: Vec<TaskResponse> = tasks_ra
        .iter()
        .filter(|task| needle.is_empty() || matches_text(task, &needle))
        .filter(|task| matches_kind(task, &kind_wanted))
        .filter(|task| matches_assignee(task, &assignee_wanted))
        .cloned()
        .collect();

    // Offered from what is actually ON the board rather than from the roster: an option that matches
    // nothing is a dead end, and the whole point of the list is to narrow to something.
    let assignees = assignees_on_board(&tasks_ra);

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
                    select {
                        value: "{kind_wanted}",
                        onchange: move |event| kind_filter.set(event.value()),
                        option { value: "", "Any type" }
                        for kind in current.kinds.iter() {
                            option { value: "{kind.id}", "{kind.name}" }
                        }
                    }
                    select {
                        value: "{assignee_wanted}",
                        onchange: move |event| assignee_filter.set(event.value()),
                        option { value: "", "Anyone" }
                        option { value: "{UNASSIGNED}", "Unassigned" }
                        for who in assignees.iter() {
                            option { value: "{who.0}", "{who.1}" }
                        }
                    }
                    input {
                        class: "board-search",
                        r#type: "text",
                        placeholder: "Search, or a task id — RMS-42",
                        value: "{search_text}",
                        oninput: move |event| search.set(event.value()),
                        // Enter is what commits a lookup. A search that fired per keystroke would ask the
                        // server about `R`, `RM`, `RMS`… on the way to a handle, and every one of those is
                        // a miss it would then have to explain.
                        onkeydown: move |event| {
                            if event.key() == Key::Enter {
                                let query = search.read().trim().to_string();

                                if looks_like_a_task_id(&query) {
                                    spawn(async move {
                                        match crate::api::find_task(&query).await {
                                            Ok(found) => {
                                                crate::dialogs::open(
                                                    crate::dialogs::DialogState::ViewTask { found },
                                                );
                                            }
                                            Err(err) => error.set(err.message),
                                        }
                                    });
                                }
                            }
                        },
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
                        tasks: visible.clone(),
                    }
                }
            }
        }
    }
}

/// The filter value standing for "nobody is on it".
///
/// A sentinel rather than an empty string, because empty already means "anyone" — and "unassigned" is a
/// thing you genuinely want to filter for: it is the pile nobody has picked up.
const UNASSIGNED: &str = "\u{0}unassigned";

fn matches_kind(task: &TaskResponse, wanted: &str) -> bool {
    if wanted.is_empty() {
        return true;
    }

    task.kind.as_deref() == Some(wanted)
}

fn matches_assignee(task: &TaskResponse, wanted: &str) -> bool {
    if wanted.is_empty() {
        return true;
    }

    match task.assignee.as_deref() {
        None => wanted == UNASSIGNED,
        // Case-insensitive: an address is stored lower-cased and the reserved `AI` is stored as declared,
        // so comparing exactly would depend on which of the two this happens to be.
        Some(assignee) => wanted != UNASSIGNED && assignee.eq_ignore_ascii_case(wanted),
    }
}

/// Everyone with something on this board, as (value, label), sorted by label.
///
/// Deduplicated case-insensitively and labelled with the display name where there is one — the value has to
/// be what is stored, but nobody picks a colleague out of a list of email addresses.
fn assignees_on_board(tasks: &[TaskResponse]) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();

    for task in tasks {
        let Some(assignee) = task.assignee.as_deref() else {
            continue;
        };

        if result
            .iter()
            .any(|(value, _)| value.eq_ignore_ascii_case(assignee))
        {
            continue;
        }

        let label = task
            .assignee_name
            .clone()
            .filter(|itm| !itm.trim().is_empty())
            .unwrap_or_else(|| assignee.to_string());

        result.push((assignee.to_string(), label));
    }

    result.sort_by_key(|itm| itm.1.to_lowercase());
    result
}

/// Whether this looks like a task handle rather than something to search for.
///
/// A shape test, not a parse: `PREFIX-42`. The server does the authoritative parse — it is the only side
/// that knows which prefixes exist and which have moved — so all this decides is which of the two things
/// to do with what was typed.
fn looks_like_a_task_id(query: &str) -> bool {
    let Some((prefix, number)) = query.rsplit_once('-') else {
        return false;
    };

    !prefix.is_empty()
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// Everything on a card that is worth searching: its id, its text, its labels and who has it.
///
/// Not the comments. A thread can be long and mostly discussion, so matching it would return cards whose
/// visible face has nothing to do with what was typed — which reads as a broken filter rather than a
/// thorough one.
fn matches_text(task: &TaskResponse, needle: &str) -> bool {
    task.id.to_lowercase().contains(needle)
        || task.text.to_lowercase().contains(needle)
        || task
            .labels
            .iter()
            .any(|itm| itm.to_lowercase().contains(needle))
        || task
            .assignee
            .as_ref()
            .is_some_and(|itm| itm.to_lowercase().contains(needle))
        || task
            .assignee_name
            .as_ref()
            .is_some_and(|itm| itm.to_lowercase().contains(needle))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, text: &str, labels: &[&str], assignee: Option<&str>) -> TaskResponse {
        TaskResponse {
            id: id.to_string(),
            project_id: "p".to_string(),
            text: text.to_string(),
            status: COLUMN_ID_TODO.to_string(),
            kind: None,
            goal_id: None,
            goal_name: None,
            assignee: assignee.map(|itm| itm.to_string()),
            assignee_name: None,
            labels: labels.iter().map(|itm| itm.to_string()).collect(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            blocked: false,
            comments: Vec::new(),
            created_unix_seconds: 0,
            updated_unix_seconds: 0,
            closed_unix_seconds: None,
        }
    }

    /// Which of the two things the box does is decided here, so the shapes are worth pinning.
    #[test]
    fn a_handle_is_told_apart_from_a_search() {
        assert!(looks_like_a_task_id("RMS-42"));
        assert!(looks_like_a_task_id("RMS-000042"));
        assert!(
            looks_like_a_task_id("TM_2-7"),
            "an underscore is legal in a prefix"
        );

        // Words that happen to contain a dash must NOT be taken for an id, or searching for one of our own
        // type names would fire a lookup instead of filtering.
        assert!(!looks_like_a_task_id("tech-debt"));
        assert!(!looks_like_a_task_id("in-progress-review"));
        assert!(!looks_like_a_task_id("RMS-"));
        assert!(!looks_like_a_task_id("-42"));
        assert!(!looks_like_a_task_id("42"));
        assert!(!looks_like_a_task_id(""));
        assert!(!looks_like_a_task_id("fix the login bug"));
    }

    /// A column id with a dash in it is a plausible thing to type, and it is not an id.
    #[test]
    fn a_prefix_may_not_contain_a_dash() {
        assert!(!looks_like_a_task_id("in-progress-2"));
    }

    #[test]
    fn the_filter_looks_at_the_face_of_a_card() {
        let one = task(
            "RMS-000001",
            "Fix the login redirect",
            &["auth"],
            Some("ann@x.io"),
        );

        assert!(matches_text(&one, "login"));
        assert!(matches_text(&one, "rms-000001"), "the id is searchable too");
        assert!(matches_text(&one, "auth"), "and its labels");
        assert!(matches_text(&one, "ann"), "and who has it");
        assert!(!matches_text(&one, "logout"));
    }

    /// Case must not matter: what gets typed is lower case and what is stored is however it was written.
    #[test]
    fn the_type_filter_is_exact_and_empty_means_any() {
        let mut one = task("RMS-000001", "text", &[], None);
        one.kind = Some("bug".to_string());

        assert!(matches_kind(&one, ""), "empty means any");
        assert!(matches_kind(&one, "bug"));
        assert!(!matches_kind(&one, "feature"));

        let none = task("RMS-000002", "text", &[], None);
        assert!(matches_kind(&none, ""));
        assert!(
            !matches_kind(&none, "bug"),
            "a task with no type matches no type"
        );
    }

    /// "Unassigned" needs its own value: empty already means "anyone", and the pile nobody picked up is
    /// exactly what you want to filter for.
    #[test]
    fn the_assignee_filter_tells_anyone_from_unassigned() {
        let taken = task("RMS-000001", "text", &[], Some("ann@x.io"));
        let free = task("RMS-000002", "text", &[], None);

        assert!(matches_assignee(&taken, ""));
        assert!(matches_assignee(&free, ""));

        assert!(matches_assignee(&taken, "ann@x.io"));
        assert!(!matches_assignee(&free, "ann@x.io"));

        assert!(matches_assignee(&free, UNASSIGNED));
        assert!(!matches_assignee(&taken, UNASSIGNED));
    }

    /// The reserved assignee is stored as `AI` and an address lower-cased, so the compare cannot be exact.
    #[test]
    fn the_assignee_filter_ignores_case() {
        let ai = task("RMS-000001", "text", &[], Some("AI"));

        assert!(matches_assignee(&ai, "AI"));
        assert!(matches_assignee(&ai, "ai"));
    }

    #[test]
    fn the_assignee_list_comes_from_the_board_and_is_deduplicated() {
        let mut named = task("RMS-000001", "text", &[], Some("ann@x.io"));
        named.assignee_name = Some("Ann".to_string());

        let tasks = vec![
            named,
            task("RMS-000002", "text", &[], Some("ANN@x.io")),
            task("RMS-000003", "text", &[], Some("AI")),
            task("RMS-000004", "text", &[], None),
        ];

        let list = assignees_on_board(&tasks);

        assert_eq!(
            list,
            vec![
                ("AI".to_string(), "AI".to_string()),
                ("ann@x.io".to_string(), "Ann".to_string()),
            ],
            "one entry per person, labelled by name where there is one, and nobody for an unassigned task"
        );
    }

    #[test]
    fn the_filter_ignores_case() {
        let one = task("RMS-000001", "Fix the LOGIN redirect", &["Auth"], None);

        assert!(matches_text(&one, "login"));
        assert!(matches_text(&one, "auth"));
    }
}
