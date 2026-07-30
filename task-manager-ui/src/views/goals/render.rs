use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_utils::{DataState, RenderState};
use task_manager_shared::goals::GoalResponse;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::{COLUMN_ID_DONE, COLUMN_ID_TODO, ProjectResponse};
use task_manager_shared::task_id::task_id_display;
use task_manager_shared::tasks::TaskResponse;

use crate::states::AppState;

/// The same work as Home, seen from the other end.
///
/// Home answers "what is on the board"; this answers "how is each goal going". Read-only for the same
/// reason Home is: goals are opened, renamed and closed through `/mcp`, and this screen owes the reader an
/// accurate picture rather than controls.
#[component]
pub fn RenderGoals() -> Element {
    let app_state = consume_context::<Signal<AppState>>();

    let mut cs = use_signal(ComponentState::default);

    // A push carries the goals and the live board together, so both are replaced without a request and
    // without emptying anything first. The per-goal lists are dropped instead of replaced: they include
    // archived work, which is deliberately not in a snapshot, so the only honest thing to do with them is
    // to fetch again — and only for the goals that are actually open.
    use_effect(move || {
        let app_ra = app_state.read();
        let _revision = app_ra.board_revision;
        let push = app_ra.board_push.clone();
        drop(app_ra);

        match push {
            Some(snapshot) if snapshot.project_id == cs.peek().selected => {
                let mut write = cs.write();
                write.goals.set_loaded(snapshot.goals);
                write.tasks.set_loaded(snapshot.tasks);
                write.goal_tasks.clear();
            }
            Some(_) => {}
            None if cs.peek().goals.has_value() => {
                let mut write = cs.write();
                write.goals.reset();
                write.tasks.reset();
                write.goal_tasks.clear();
            }
            None => {}
        }
    });

    // The selection is remembered in the same place Home remembers it, so switching tabs keeps you on the
    // board you were looking at rather than on whichever project sorts first.
    use_effect(move || {
        let project_id = cs.read().selected.clone();

        if !project_id.is_empty() {
            crate::web::storage::save_last_project(&project_id);
            crate::web::watch_project(&project_id);
        }
    });

    let cs_ra = cs.read();

    let projects = match get_projects(cs, &cs_ra) {
        Ok(projects) => projects,
        Err(element) => return element,
    };

    if projects.is_empty() {
        return rsx! {
            div { class: "page-header",
                h1 { class: "page-title", "Goals" }
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

    let header = rsx! {
        RenderHeader { projects: projects.to_vec(), cs }
    };

    let goals: Vec<GoalResponse> = match get_goals(cs, &cs_ra) {
        Ok(goals) => goals.to_vec(),
        Err(element) => {
            return rsx! {
                div { class: "goals-page",
                    {header}
                    {element}
                }
            };
        }
    };

    // The board read as well, and only for the tasks that belong to no goal — the Backlog group. Live work
    // only there, on purpose: loose tasks have no epic to be history of.
    let tasks: Vec<TaskResponse> = match get_tasks(cs, &cs_ra) {
        Ok(tasks) => tasks.to_vec(),
        Err(element) => {
            return rsx! {
                div { class: "goals-page",
                    {header}
                    {element}
                }
            };
        }
    };

    let loose: Vec<TaskResponse> = tasks
        .iter()
        .filter(|task| task.goal.is_none())
        .cloned()
        .collect();

    let current = current.clone();
    let expanded = cs_ra.expanded.clone();
    let picking_color = cs_ra.picking_color.clone();
    // Copied out rather than cloned: the map holds `DataState`s, which do not clone, and what a child
    // needs is the three-way answer rather than the loader's own state.
    let goal_tasks: HashMap<String, GoalTasks> = cs_ra
        .goal_tasks
        .iter()
        .map(|(id, state)| (id.clone(), GoalTasks::of(state)))
        .collect();
    drop(cs_ra);

    rsx! {
        div { class: "goals-page",
            {header}

            if goals.is_empty() && loose.is_empty() {
                div { class: "empty-note",
                    "Nothing here yet. Goals are opened through MCP — ask an agent to open one."
                }
            }

            div { class: "goals-list",
                for goal in goals.iter() {
                    RenderGoal {
                        key: "{goal.id}",
                        goal: goal.clone(),
                        project: current.clone(),
                        open: expanded.contains(&goal.id),
                        tasks: goal_tasks.get(&goal.id).cloned(),
                        picking_color: picking_color.as_deref() == Some(goal.id.as_str()),
                        cs,
                    }
                }

                // Last, and drawn as a goal without being one: work that belongs to no epic still has to be
                // visible, or this screen would quietly hide part of the board.
                if !loose.is_empty() {
                    RenderBacklog {
                        tasks: loose,
                        project: current.clone(),
                        open: expanded.iter().any(|itm| itm == BACKLOG),
                        cs,
                    }
                }
            }
        }
    }
}

/// The work under one expanded goal, in the three states it can be in — the PROP form of the `DataState`
/// the component state holds.
///
/// It exists only because `DataState` is neither `Clone` nor `PartialEq` and a component prop has to be
/// both. The request itself goes through the `DataState` like every other one; this is what gets copied
/// out of it on the way into the child.
#[derive(Clone, PartialEq)]
enum GoalTasks {
    Loading,
    Loaded(Vec<TaskResponse>),
    Failed(String),
}

impl GoalTasks {
    /// `None` and `Loading` are the same thing to a reader — an entry exists, so a request is on its way.
    fn of(state: &DataState<Vec<TaskResponse>>) -> Self {
        match state.as_ref() {
            RenderState::None | RenderState::Loading => Self::Loading,
            RenderState::Loaded(tasks) => Self::Loaded(tasks.clone()),
            RenderState::Error(err) => Self::Failed(err.clone()),
        }
    }
}

/// The key the Backlog group is remembered under. Not a goal id — no goal can be called this, because a
/// handle always contains a `-`.
const BACKLOG: &str = "backlog";

#[derive(Default)]
struct ComponentState {
    projects: DataState<Vec<ProjectResponse>>,
    selected: String,
    goals: DataState<Vec<GoalResponse>>,
    tasks: DataState<Vec<TaskResponse>>,
    /// Which groups are open. Kept across a repaint, so a push does not fold up what somebody was reading.
    expanded: Vec<String>,
    /// Goal id -> its whole task list, archived work included. Filled when a goal is first expanded and
    /// dropped on every push, since a snapshot cannot carry archived work. A `DataState` per goal, like
    /// every other request on this screen — [`GoalTasks`] is only the shape it takes as a prop.
    goal_tasks: HashMap<String, DataState<Vec<TaskResponse>>>,
    /// Which goal's palette is open, if any. One at a time: two open palettes ask a question nobody asked.
    picking_color: Option<String>,
}

impl ComponentState {
    fn select(&mut self, project_id: String) {
        if self.selected == project_id {
            return;
        }

        self.selected = project_id;
        // Reset rather than clear: the next render sees `None` and loads, which is the same path a first
        // visit takes. See `get_goals`.
        self.goals.reset();
        self.tasks.reset();
        self.goal_tasks.clear();
        self.expanded.clear();
        self.picking_color = None;
    }

    fn toggle(&mut self, key: &str) {
        if let Some(at) = self.expanded.iter().position(|itm| itm == key) {
            self.expanded.remove(at);
        } else {
            self.expanded.push(key.to_string());
        }
    }
}

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
                        let remembered = crate::web::storage::get_last_project();

                        let initial = remembered
                            .filter(|id| response.projects.iter().any(|itm| &itm.id == id))
                            .or_else(|| response.projects.first().map(|itm| itm.id.clone()))
                            .unwrap_or_default();

                        let mut write = cs.write();
                        write.selected = initial;
                        write.projects.set_loaded(response.projects);
                        drop(write);

                        // The socket starts here for the same reason Home starts it there: nothing can be
                        // subscribed to until a project is known. It has to be started by THIS screen too —
                        // somebody who opens /goals directly would otherwise have no channel for changes at
                        // all, and every change arrives that way. Idempotent, so both doing it is fine.
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

fn get_goals(
    mut cs: Signal<ComponentState>,
    cs_ra: &ComponentState,
) -> Result<&[GoalResponse], Element> {
    if cs_ra.selected.is_empty() {
        return Ok(&[]);
    }

    match cs_ra.goals.as_ref() {
        RenderState::None => {
            let project_id = cs_ra.selected.clone();

            spawn(async move {
                cs.write().goals.set_loading();

                match crate::api::get_goals(&project_id).await {
                    Ok(response) => cs.write().goals.set_loaded(response.goals),
                    Err(err) => cs.write().goals.set_error(err.message),
                }
            });

            Err(render_loading())
        }
        RenderState::Loading => Err(render_loading()),
        RenderState::Loaded(goals) => Ok(goals.as_slice()),
        RenderState::Error(err) => Err(render_error(err)),
    }
}

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
        div { class: "error-note", "{message}" }
    }
}

#[component]
fn RenderHeader(projects: Vec<ProjectResponse>, cs: Signal<ComponentState>) -> Element {
    let selected_id = cs.read().selected.clone();
    let mut cs = cs;

    rsx! {
        div { class: "page-header",
            h1 { class: "page-title", "Goals" }
            div { class: "project-picker",
                // `selected` on the option rather than `value` on the select, or a remembered project is
                // restored into the state and not shown in the control.
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
            }
        }
    }
}

/// One goal, folded or open.
#[component]
fn RenderGoal(
    goal: GoalResponse,
    project: ProjectResponse,
    open: bool,
    tasks: Option<GoalTasks>,
    picking_color: bool,
    cs: Signal<ComponentState>,
) -> Element {
    let mut cs = cs;

    let closed = goal.closed_unix_seconds.is_some();
    let hex = KindColor::parse_or_default(&goal.color).hex();

    // Counters as the server sent them. NOT recomputed from the list below: that list is absent until the
    // goal is expanded, and the numbers count archived work which a board read leaves out.
    let done = goal.done_amount;
    let total = goal.tasks_amount;

    let percent = if total > 0 { done * 100 / total } else { 0 };

    let goal_id = goal.id.clone();
    let project_id = project.id.clone();
    let already_loaded = tasks.is_some();

    let for_dialog = goal.clone();
    let swatch_goal = goal.id.clone();
    let picker_project = project.id.clone();
    let picker_goal = goal.id.clone();

    rsx! {
        div {
            class: if closed { "goal-card closed" } else { "goal-card" },
            // The goal's colour on its own edge as well as on its cards, so the two are recognisably the
            // same goal when the screens sit side by side.
            style: "border-left: 3px solid {hex}",

            div {
                class: "goal-head",
                // Fetching from the handler, not from the render body: opening a goal IS the moment its
                // work is wanted, and a write to a signal during render is the one thing the design
                // patterns forbid. A list already in hand is not fetched again — a push clears it, which is
                // what makes the next expansion ask.
                onclick: move |_| {
                    let opening = {
                        let mut write = cs.write();
                        write.toggle(&goal_id);
                        write.expanded.iter().any(|itm| itm == &goal_id)
                    };

                    if !opening || already_loaded {
                        return;
                    }

                    let goal_id = goal_id.clone();
                    let project_id = project_id.clone();

                    spawn(async move {
                        cs.write()
                            .goal_tasks
                            .entry(goal_id.clone())
                            .or_default()
                            .set_loading();

                        let loaded = crate::api::get_goal_tasks(&project_id, &goal_id).await;

                        let mut write = cs.write();
                        let state = write.goal_tasks.entry(goal_id).or_default();

                        match loaded {
                            Ok(response) => state.set_loaded(response.tasks),
                            Err(err) => state.set_error(err.message),
                        }
                    });
                },

                span { class: "goal-caret", if open { "▾" } else { "▸" } }
                img { class: "goal-icon", src: asset!("/public/assets/images/goal.svg"), alt: "" }

                // `stop_propagation` because the whole head is the fold/unfold target: without it, changing
                // a colour would also open or close the goal, which is two things for one click.
                button {
                    class: "goal-swatch",
                    style: "background: {hex}",
                    title: "Colour this goal",
                    onclick: move |event| {
                        event.stop_propagation();

                        let mut write = cs.write();
                        write.picking_color = if write.picking_color.as_deref() == Some(swatch_goal.as_str()) {
                            None
                        } else {
                            Some(swatch_goal.clone())
                        };
                    },
                }

                div { class: "goal-head-text",
                    div { class: "goal-name", "{goal.name}" }
                }

                div { class: "goal-meta",
                    span { class: "goal-id", "{goal.id}" }
                    if closed {
                        span { class: "goal-closed-flag", "Closed" }
                    }
                    span { class: "goal-progress-text", "{done} / {total}" }
                    div { class: "goal-progress",
                        div { class: "goal-progress-fill", style: "width: {percent}%" }
                    }
                    if goal.comments.len() > 0 {
                        span { class: "goal-comments", title: "{goal.comments.len()} notes on the thread",
                            "💬 {goal.comments.len()}"
                        }
                    }

                    // The description and the thread live behind this, not in the head: a head that carried
                    // the text would either truncate it — which reads as a broken sentence — or make every
                    // row a different height. `stop_propagation` for the same reason as the swatch: the head
                    // itself folds the goal, and one click must do one thing.
                    button {
                        class: "sticker-view",
                        title: "Read this goal and its comments",
                        onclick: move |event| {
                            event.stop_propagation();
                            crate::dialogs::open(crate::dialogs::DialogState::ViewGoal {
                                goal: for_dialog.clone(),
                            });
                        },
                        "👁"
                    }
                }
            }

            if picking_color {
                div {
                    class: "goal-palette",
                    onclick: move |event| event.stop_propagation(),
                    crate::dialogs::RenderColorPicker {
                        value: goal.color.clone(),
                        on_pick: move |color: String| {
                            let project_id = picker_project.clone();
                            let goal_id = picker_goal.clone();

                            // Closed first, then the request: the answer comes back as a WebSocket push
                            // carrying the whole board, so there is nothing here to wait for and nothing to
                            // write back by hand.
                            cs.write().picking_color = None;

                            spawn(async move {
                                if let Err(err) = crate::api::set_goal_color(&project_id, &goal_id, &color).await {
                                    crate::web::console_log(
                                        format!("recolouring {goal_id} failed: {}", err.message).as_str(),
                                    );
                                }
                            });
                        },
                    }
                }
            }

            if open {
                div { class: "goal-body",
                    match &tasks {
                        Some(GoalTasks::Loaded(tasks)) if tasks.is_empty() => rsx! {
                            div { class: "empty-note",
                                "No tasks under this goal yet — it is still being talked about."
                            }
                        },
                        Some(GoalTasks::Loaded(tasks)) => rsx! {
                            for task in tasks.iter() {
                                RenderGoalTask { key: "{task.id}", task: task.clone(), project: project.clone() }
                            }
                        },
                        Some(GoalTasks::Failed(err)) => rsx! {
                            div { class: "error-note", "{err}" }
                        },
                        Some(GoalTasks::Loading) | None => rsx! {
                            div { class: "loading-note", "Loading…" }
                        },
                    }
                }
            }
        }
    }
}

/// Tasks that belong to no goal, drawn like a goal so nothing on the board is invisible from this screen.
///
/// Not in the database and never will be: it is a group, not a container. Live work only — loose tasks are
/// nobody's epic, so there is no history of them to show.
#[component]
fn RenderBacklog(
    tasks: Vec<TaskResponse>,
    project: ProjectResponse,
    open: bool,
    cs: Signal<ComponentState>,
) -> Element {
    let mut cs = cs;

    let done = tasks
        .iter()
        .filter(|task| task.status == COLUMN_ID_DONE)
        .count();

    rsx! {
        div { class: "goal-card backlog",
            div {
                class: "goal-head",
                onclick: move |_| cs.write().toggle(BACKLOG),

                span { class: "goal-caret", if open { "▾" } else { "▸" } }

                div { class: "goal-head-text",
                    div { class: "goal-name", "Backlog" }
                    div { class: "goal-note", "not part of any goal" }
                }

                div { class: "goal-meta",
                    span { class: "goal-progress-text", "{done} / {tasks.len()}" }
                }
            }

            if open {
                div { class: "goal-body",
                    for task in tasks.iter() {
                        RenderGoalTask { key: "{task.id}", task: task.clone(), project: project.clone() }
                    }
                }
            }
        }
    }
}

/// One task under a goal: a line rather than a card.
///
/// A line, because what this screen is for is the shape of a goal — twenty cards would bury it. What is on
/// the line is what a person asks about a task without opening it: what kind of work it is, where it sits,
/// whether anybody has said anything, and who has it. The whole task is one click away, in the same dialog
/// the board opens.
#[component]
fn RenderGoalTask(task: TaskResponse, project: ProjectResponse) -> Element {
    let title = task_manager_shared::task_title::task_title(&task.text);

    let kind = task
        .kind
        .as_ref()
        .and_then(|kind_id| project.kinds.iter().find(|itm| &itm.id == kind_id));

    let kind_hex = kind
        .map(|itm| KindColor::parse_or_default(&itm.color).hex())
        .unwrap_or("");

    // Both anchors exist in every project but are not in `columns`, which holds the middle only — so a task
    // in Todo or Done would otherwise show a raw id where every other row shows a name.
    let status_name = match task.status.as_str() {
        COLUMN_ID_TODO => "Todo".to_string(),
        COLUMN_ID_DONE => "Done".to_string(),
        stored => project
            .columns
            .iter()
            .find(|column| column.id == stored)
            .map(|column| column.name.clone())
            .unwrap_or_else(|| stored.to_string()),
    };

    let done = task.status == COLUMN_ID_DONE;

    let assignee = task
        .assignee_name
        .clone()
        .or_else(|| task.assignee.clone());

    let found = crate::api::find_task_locally(&task, &project);

    rsx! {
        div {
            class: if done { "goal-task done" } else { "goal-task" },
            onclick: move |_| {
                crate::dialogs::open(crate::dialogs::DialogState::ViewTask { found: found.clone() });
            },

            span { class: "goal-task-id", "{task_id_display(&task.id)}" }

            if let Some(kind) = kind {
                span {
                    class: "goal-task-kind",
                    style: "background: {kind_hex}",
                    title: "{kind.description}",
                    if crate::web::icon_exists(&kind.icon) {
                        img { class: "sticker-kind-icon", src: "{crate::web::icon_url(&kind.icon)}", alt: "" }
                    }
                    "{kind.name}"
                }
            }

            span { class: "goal-task-title", "{title}" }

            if task.blocked {
                span { class: "sticker-blocked-flag", "Blocked" }
            }

            // Only when there is a thread. A `0` on every line is noise that makes the lines that do have
            // something harder to spot.
            if !task.comments.is_empty() {
                span { class: "goal-task-comments", title: "{task.comments.len()} comments",
                    "💬 {task.comments.len()}"
                }
            }

            span {
                class: if assignee.is_some() { "goal-task-assignee" } else { "goal-task-assignee unassigned" },
                {assignee.clone().unwrap_or_else(|| "Unassigned".to_string())}
            }

            span { class: "goal-task-status", "{status_name}" }
        }
    }
}
