use dioxus::prelude::*;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::{COLUMN_ID_DONE, COLUMN_ID_TODO, ProjectResponse};
use task_manager_shared::tasks::{FindTaskResponse, TaskResponse};

use dioxus_utils::{DataState, RenderState};

use crate::states::AppState;

/// The board.
///
/// **Read-only, deliberately.** Nothing here is dragged, clicked to edit or double-clicked: every change
/// to a task arrives through `/mcp`. What this screen owes the reader is an accurate picture, which is
/// why it repaints from a WebSocket push rather than hoping somebody reloads.
#[component]
pub fn RenderHome() -> Element {
    let app_state = consume_context::<Signal<AppState>>();

    let mut cs = use_signal(ComponentState::default);
    let cs_ra = cs.read();

    // A WebSocket push says "this board changed". Resetting the `DataState` is what re-reads it — an
    // effect rather than a read inside the loader, because `use_effect` IS reactive where `use_future` is
    // not, and this is the whole invalidation mechanism on this side.
    use_effect(move || {
        let _revision = app_state.read().board_revision;

        if cs.peek().tasks.has_value() {
            cs.write().tasks.reset();
        }
    });

    // Side effects of a selection, in an effect because that is what `use_effect` is for and because it IS
    // reactive: remember the choice for the next visit, and tell the socket which board to push about.
    // Sending the subscription on every change is also what makes switching projects not need a reconnect.
    use_effect(move || {
        let project_id = cs.read().selected.clone();

        if !project_id.is_empty() {
            crate::web::storage::save_last_project(&project_id);
            crate::web::watch_project(&project_id);
        }
    });

    // Loaded first, and the one thing the rest of the screen cannot do without. `Err` carries what to show
    // instead — a spinner or the failure — so the body below stays a straight line.
    let projects = match get_projects(cs, &cs_ra) {
        Ok(projects) => projects,
        Err(element) => return element,
    };

    if projects.is_empty() {
        return rsx! {
            div { class: "page-header",
                h1 { class: "page-title", "Home" }
            }
            div { class: "empty-note",
                "You are not on any project yet. Ask an admin to add you to one."
            }
        };
    }

    let selected_id = cs_ra.selected.clone();

    let current = projects
        .iter()
        .find(|itm| itm.id == selected_id)
        .or_else(|| projects.first());

    let Some(current) = current else {
        return rsx! {
            div { class: "empty-note", "No project selected." }
        };
    };

    // Keyed on the selection: picking another board resets this, which is what makes the next render load
    // it. See `ComponentState::select`.
    //
    // While it loads the header stays up — hiding the project picker mid-switch makes the screen jump.
    let tasks_ra: Vec<TaskResponse> = match get_tasks(cs, &cs_ra) {
        Ok(tasks) => tasks.to_vec(),
        Err(element) => {
            return rsx! {
                div { class: "board-page",
                    RenderHeader {
                        projects: projects.to_vec(),
                        current: current.clone(),
                        cs,
                        assignees: Vec::new(),
                    }
                    {element}
                }
            };
        }
    };

    let search_text = cs_ra.search.clone();
    let kind_wanted = cs_ra.kind_filter.clone();
    let assignee_wanted = cs_ra.assignee_filter.clone();

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
            RenderHeader {
                projects: projects.to_vec(),
                current: current.clone(),
                cs,
                assignees,
            }

            if !current.description.trim().is_empty() {
                p { class: "page-note", "{current.description}" }
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

/// Everything this screen holds, in one struct behind one signal — the house shape.
///
/// The two `DataState`s are why the board works at all now. They are read in the RENDER body, which is what
/// subscribes the component to them; the previous version read its dependencies inside `use_future`, which
/// spawns once and tracks nothing, so the board loaded exactly never.
#[derive(Default)]
struct ComponentState {
    projects: DataState<Vec<ProjectResponse>>,
    /// Keyed on `selected`: choosing another board resets this, and the next render loads it.
    tasks: DataState<Vec<TaskResponse>>,
    selected: String,
    search: String,
    /// Empty means "any". Ids rather than indexes, so a board changing under a filter cannot silently move
    /// it to a different type.
    kind_filter: String,
    assignee_filter: String,
}

impl ComponentState {
    /// Switch boards. One method because the two halves are coupled: leaving `tasks` alone would show the
    /// previous project's cards under the new project's name.
    fn select(&mut self, project_id: String) {
        if self.selected == project_id {
            return;
        }

        self.selected = project_id;
        self.tasks.reset();
    }
}

/// The projects this person may see, loading them on first render.
///
/// The socket starts HERE, once the list is in hand — not in the shell on the way past. Until a project is
/// known there is nothing to subscribe to, and the shell was starting it during render, which is a write to
/// a signal in the render body and the one thing `dioxus-design-patterns` §16 says never to do.
fn get_projects(
    mut cs: Signal<ComponentState>,
    cs_ra: &ComponentState,
) -> Result<&[ProjectResponse], Element> {
    match cs_ra.projects.as_ref() {
        RenderState::None => {
            spawn(async move {
                cs.write().projects.set_loading();

                match crate::api::get_projects().await {
                    Ok(response) => {
                        // The remembered project is matched against what actually came back, so a board
                        // somebody lost access to falls through to the first one they can see rather than
                        // leaving the screen on nothing.
                        let remembered = crate::web::storage::get_last_project();

                        let initial = remembered
                            .filter(|id| response.projects.iter().any(|itm| &itm.id == id))
                            .or_else(|| response.projects.first().map(|itm| itm.id.clone()))
                            .unwrap_or_default();

                        let mut write = cs.write();
                        write.selected = initial;
                        write.projects.set_loaded(response.projects);
                        drop(write);

                        // Data is in. Now the sockets.
                        crate::start_ws();
                    }
                    Err(err) => cs.write().projects.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(projects) => Ok(projects.as_slice()),
        RenderState::Error(err) => Err(render_error(err)),
    }
}

/// The selected board's tasks.
///
/// Nothing to load until a project is chosen, which is an empty board rather than a spinner — the picker is
/// already on screen and a spinner there would suggest something is coming.
fn get_tasks(
    mut cs: Signal<ComponentState>,
    cs_ra: &ComponentState,
) -> Result<&[TaskResponse], Element> {
    if cs_ra.selected.is_empty() {
        return Ok(&[]);
    }

    match cs_ra.tasks.as_ref() {
        RenderState::None => {
            let project_id = cs_ra.selected.clone();

            spawn(async move {
                cs.write().tasks.set_loading();

                match crate::api::get_tasks(&project_id).await {
                    Ok(response) => cs.write().tasks.set_loaded(response.tasks),
                    Err(err) => cs.write().tasks.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(tasks) => Ok(tasks.as_slice()),
        RenderState::Error(err) => Err(render_error(err)),
    }
}

fn render_loading() -> Element {
    rsx! {
        div { class: "loading-note", "Loading…" }
    }
}

fn render_error(message: &str) -> Element {
    rsx! {
        div { class: "error-banner", "{message}" }
    }
}

/// The title, the project picker and the filters. Its own component so the loading path and the loaded path
/// draw the same header — otherwise the screen jumps every time a board is switched.
#[component]
fn RenderHeader(
    projects: Vec<ProjectResponse>,
    current: ProjectResponse,
    cs: Signal<ComponentState>,
    assignees: Vec<(String, String)>,
) -> Element {
    let app_state = consume_context::<Signal<AppState>>();
    let ws_live = app_state.read().ws_live;

    let cs_ra = cs.read();
    let selected_id = cs_ra.selected.clone();
    let search_text = cs_ra.search.clone();
    let kind_wanted = cs_ra.kind_filter.clone();
    let assignee_wanted = cs_ra.assignee_filter.clone();
    drop(cs_ra);

    let mut cs = cs;

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Home" }
            div { class: "project-picker",
                // `selected` on the matching option, not `value` on the select: HTML decides a dropdown's
                // shown item from the option's attribute, so the remembered project was restored into the
                // state but the control still displayed the first entry.
                select {
                    onchange: move |event| cs.write().select(event.value()),
                    for project in projects.iter() {
                        option {
                            value: "{project.id}",
                            selected: project.id == selected_id,
                            "{project.prefix} · {project.name}"
                        }
                    }
                }
                select {
                    onchange: move |event| cs.write().kind_filter = event.value(),
                    option { value: "", selected: kind_wanted.is_empty(), "Any type" }
                    for kind in current.kinds.iter() {
                        option {
                            value: "{kind.id}",
                            selected: kind.id == kind_wanted,
                            "{kind.name}"
                        }
                    }
                }
                select {
                    onchange: move |event| cs.write().assignee_filter = event.value(),
                    option { value: "", selected: assignee_wanted.is_empty(), "Anyone" }
                    option {
                        value: "{UNASSIGNED}",
                        selected: assignee_wanted == UNASSIGNED,
                        "Unassigned"
                    }
                    for who in assignees.iter() {
                        option {
                            value: "{who.0}",
                            selected: who.0 == assignee_wanted,
                            "{who.1}"
                        }
                    }
                }
                input {
                    class: "board-search",
                    r#type: "text",
                    placeholder: "Search, or a task id — RMS-42",
                    value: "{search_text}",
                    oninput: move |event| cs.write().search = event.value(),
                    onkeydown: move |event| {
                        if event.key() == Key::Enter {
                            let query = cs.peek().search.trim().to_string();

                            if looks_like_a_task_id(&query) {
                                spawn(async move {
                                    if let Ok(found) = crate::api::find_task(&query).await {
                                        crate::dialogs::open(
                                            crate::dialogs::DialogState::ViewTask { found },
                                        );
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

/// One card, and deliberately almost nothing: its handle and its title.
///
/// The whole task — the text, the thread, who has it, what it waits on — is behind the eye or a double-click
/// on the card, and only there. A column of full cards is a wall of Markdown you have to read to scan, which
/// is the opposite of what a board is for; a column of titles is a list you can take in at a glance.
#[component]
fn RenderSticker(task: TaskResponse, project: ProjectResponse) -> Element {
    let kind = task
        .kind
        .as_ref()
        .and_then(|kind_id| project.kinds.iter().find(|itm| &itm.id == kind_id));

    let kind_color = kind
        .map(|itm| KindColor::parse_or_default(&itm.color))
        .unwrap_or_default();

    // The type is on the card twice, on purpose: as its label beside the handle, which is what you read, and
    // as the colour of the left edge, which is what you see without reading — a column of edges tells you how
    // the work is made up before you have looked at a single card.
    let border = kind
        .map(|_| format!("border-left-color: {}", kind_color.hex()))
        .unwrap_or_default();

    let title = task_manager_shared::task_title::task_title(&task.text);

    let assignee = task
        .assignee_name
        .clone()
        .or_else(|| task.assignee.clone())
        .unwrap_or_else(|| "Unassigned".to_string());

    // Built here rather than fetched: this side already holds the whole task and the project it is on, so
    // the card opens instantly and without a round trip. `archived` is false by definition — a card that is
    // drawn is on the board.
    let found = found_locally(&task, &project);
    let found_on_the_card = found.clone();

    rsx! {
        div {
            class: if task.blocked { "sticker blocked" } else { "sticker" },
            style: "{border}",
            // The whole card, not only the eye. Double rather than single, because a single click on a card
            // is how you select one and this board has no selection — a stray click must not throw a dialog
            // in front of somebody who was only scrolling.
            ondoubleclick: move |_| {
                crate::dialogs::open(crate::dialogs::DialogState::ViewTask {
                    found: found_on_the_card.clone(),
                });
            },

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
                button {
                    class: "sticker-view",
                    title: "View the task",
                    onclick: move |_| {
                        crate::dialogs::open(crate::dialogs::DialogState::ViewTask {
                            found: found.clone(),
                        });
                    },
                    "👁"
                }
            }

            div { class: "sticker-title", "{title}" }

            // Under the title, because the title is what the card is FOR and who has it is the next question
            // — and it is the same question on every card, so it belongs in the same place on every card.
            //
            // The counters say only HOW MANY. What they count is in the dialog, and a card that listed the ids
            // it waits on was one of the things that made a column unreadable.
            div { class: "sticker-bottom",
                span { class: "sticker-assignee", "{assignee}" }
                div { class: "sticker-counters",
                    if !task.comments.is_empty() {
                        span { title: "{task.comments.len()} comments", "💬 {task.comments.len()}" }
                    }
                    if !task.depends_on.is_empty() {
                        span {
                            title: "Waiting on {task.depends_on.len()} task(s): {task.depends_on.join(\", \")}",
                            "⬇ {task.depends_on.len()}"
                        }
                    }
                    if !task.blocks.is_empty() {
                        span {
                            title: "{task.blocks.len()} task(s) waiting on this one: {task.blocks.join(\", \")}",
                            "⬆ {task.blocks.len()}"
                        }
                    }
                }
            }
        }
    }
}

/// The task exactly as a lookup by id would have answered it, assembled from what is already on screen.
fn found_locally(task: &TaskResponse, project: &ProjectResponse) -> FindTaskResponse {
    FindTaskResponse {
        task: Some(task.clone()),
        project_id: project.id.clone(),
        project_prefix: project.prefix.clone(),
        project_name: project.name.clone(),
        archived: false,
        not_found: String::new(),
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
