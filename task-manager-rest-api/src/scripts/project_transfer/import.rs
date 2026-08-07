use std::io::Read;

use ahash::AHashMap;
use rust_extensions::date_time::DateTimeAsMicroseconds;
use service_sdk::my_telemetry::MyTelemetryContext;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::priority::Priority;

use crate::app::AppContext;
use crate::board::{
    CommentModel, GhActionModel, GoalModel, ProjectModel, SubtaskModel, TaskModel,
    parse_goal_handle, parse_task_handle,
};
use crate::postgres::{GoalDto, TaskDto};

use super::models::*;

/// The most one import may write, counted in goals plus tasks.
///
/// Not a technical ceiling — it is the point past which one HTTP request is the wrong shape for the job.
/// Every card is a Postgres upsert, written one after another inside a single request, and there is no undo:
/// an import that ran for four minutes and then timed out would leave a board half-poured with nothing to
/// roll it back. A real board is a fraction of this.
pub const MAX_IMPORT_CARDS: usize = 2_000;

/// The most an import archive may be, decoded.
///
/// Matched to what an export can produce, so a file this product wrote is a file this product can read back.
pub const MAX_IMPORT_BYTES: usize = 512 * 1024 * 1024;

/// One thing in the file that was not written, and why.
pub struct SkippedImport {
    pub name: String,
    pub reason: String,
}

impl SkippedImport {
    fn new(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reason: reason.into(),
        }
    }
}

/// What an import did.
pub struct ImportOutcome {
    pub goals: usize,
    pub tasks: usize,
    pub comments: usize,
    pub documents: usize,
    pub skipped: Vec<SkippedImport>,
    pub notes: Vec<String>,
}

/// Pour an export into an existing project.
///
/// **Numbers are re-issued, and that is the one thing this cannot preserve.** A task is identified by
/// `(project, number)` out of the receiving project's own counter, so `TM-42` from the file lands as whatever
/// this board's counter hands out next. Every reference inside the file is therefore remapped: a task's goal,
/// its dependencies, and the target of every comment. That is why the file spells them as handles — a bare
/// number would be indistinguishable from one this board already uses.
///
/// **Everything else is kept as it was**: text, status, priority, kind, assignee, labels, checklists, build
/// links, the created / updated / closed / deleted moments, and every comment with its own author and moment.
/// A comment is not re-signed by whoever pressed Import — the thread is a record of who said what, and
/// rewriting it would be a lie about the past.
///
/// **Nothing on this board is touched except its settings.** Import only ever adds cards; the tasks already
/// here keep their numbers and are not looked at. The settings ARE replaced, because the statuses in the file
/// are the source board's column ids and mean nothing unless this project follows the same template — see
/// [`super::models::ProjectFile`].
///
/// **Partial by design**, like every archive on this API: an entry that cannot be written comes back in
/// `skipped` with a reason, and the rest still arrives.
pub async fn import_project(
    app: &AppContext,
    project_prefix: &str,
    archive: &[u8],
    who: &str,
) -> Result<ImportOutcome, String> {
    if archive.len() > MAX_IMPORT_BYTES {
        return Err(format!(
            "that archive is {} bytes — the limit is {MAX_IMPORT_BYTES}",
            archive.len()
        ));
    }

    let project = {
        let board = app.board.read();
        super::super::resolve_project_by_prefix(&board, project_prefix)?
            .as_ref()
            .clone()
    };

    let mut archive = ImportArchive::open(archive)?;

    let project_file = archive.read_yaml::<ProjectFile>(PROJECT_FILE)?;

    if project_file.format != FORMAT {
        return Err(format!(
            "this archive says it is '{}', and this build reads '{FORMAT}'",
            project_file.format
        ));
    }

    let source_prefix = project_file.project.prefix.trim().to_uppercase();

    if source_prefix.is_empty() {
        return Err(format!("{PROJECT_FILE} does not say which project it came from"));
    }

    let goals_file = archive.read_yaml_or_default::<GoalsFile>(GOALS_FILE)?;
    let tasks_file = archive.read_yaml_or_default::<TasksFile>(TASKS_FILE)?;
    let comments_file = archive.read_yaml_or_default::<CommentsFile>(COMMENTS_FILE)?;

    let cards = goals_file.goals.len() + tasks_file.tasks.len();

    if cards > MAX_IMPORT_CARDS {
        return Err(format!(
            "this archive holds {cards} goals and tasks, and the limit for one import is {MAX_IMPORT_CARDS}"
        ));
    }

    let mut skipped: Vec<SkippedImport> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    // Documents first, because the cards point at them: a task's `documents` list is paths in this same
    // archive, and it can only become a list of ids once those ids exist.
    let documents = write_documents(app, &mut archive, &project, who, &mut skipped).await?;

    // Then the numbers — every one of them, before a single card is built. A task's goal and its dependencies
    // can name anything in the file, including something further down it, so the whole map has to exist first.
    let numbers = Numbering::reserve(app, &project, &goals_file, &tasks_file)?;

    let mut comments_by_target = group_comments(&comments_file, &mut skipped);

    let mut goals: Vec<GoalModel> = Vec::with_capacity(goals_file.goals.len());

    for goal in &goals_file.goals {
        match build_goal(&project, goal, &numbers, &mut comments_by_target, &documents) {
            Ok(model) => goals.push(model),
            Err(err) => skipped.push(SkippedImport::new(goal.id.clone(), err)),
        }
    }

    let mut tasks: Vec<TaskModel> = Vec::with_capacity(tasks_file.tasks.len());

    for task in &tasks_file.tasks {
        match build_task(
            &project,
            task,
            &numbers,
            &mut comments_by_target,
            &documents,
            &mut skipped,
        ) {
            Ok(model) => tasks.push(model),
            Err(err) => skipped.push(SkippedImport::new(task.id.clone(), err)),
        }
    }

    // Every comment left over is one whose target never made it into this board — a handle naming a card that
    // is not in the file, or a card that was itself skipped. Said out loud rather than dropped: a thread that
    // silently loses half its notes is worse than one that says which.
    for (target, orphans) in comments_by_target.iter() {
        skipped.push(SkippedImport::new(
            target.clone(),
            format!(
                "{} comment(s) are on something this archive does not carry",
                orphans.len()
            ),
        ));
    }

    let comments = goals
        .iter()
        .map(|itm| itm.comments.len())
        .chain(tasks.iter().map(|itm| itm.comments.len()))
        .sum();

    // Postgres before memory, as everywhere else in this service: a row that failed to write must not be on
    // a board that says it is there.
    let telemetry = MyTelemetryContext::create_empty();

    for goal in &goals {
        let dto: GoalDto = goal.into();
        app.goals_repo.upsert(&dto, &telemetry).await;
    }

    for task in &tasks {
        let dto: TaskDto = task.into();
        app.tasks_repo.upsert(&dto, &telemetry).await;
    }

    // The settings, replaced — and the counter, which moved when the numbers were reserved. One project row,
    // written once, carrying both.
    apply_settings(app, &project, &project_file, &telemetry, &mut notes).await;

    notes.extend(read_notes(app, &project.id, &tasks));

    let goals_written = goals.len();
    let tasks_written = tasks.len();

    // One snapshot swap for the whole import, rather than one per card.
    app.board.upsert_goals_and_tasks(goals, tasks);
    app.notify_project_changed(&project.id).await;

    Ok(ImportOutcome {
        goals: goals_written,
        tasks: tasks_written,
        comments,
        documents: documents.len(),
        skipped,
        notes,
    })
}

/// The zip, and the two ways this feature reads out of it.
struct ImportArchive<'s> {
    zip: zip::ZipArchive<std::io::Cursor<&'s [u8]>>,
}

impl<'s> ImportArchive<'s> {
    fn open(archive: &'s [u8]) -> Result<Self, String> {
        if archive.is_empty() {
            return Err("that archive is empty".to_string());
        }

        let zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
            .map_err(|err| format!("that file is not a zip we can read: {err}"))?;

        Ok(Self { zip })
    }

    fn read_yaml<TModel: serde::de::DeserializeOwned>(
        &mut self,
        name: &str,
    ) -> Result<TModel, String> {
        let mut entry = self
            .zip
            .by_name(name)
            .map_err(|_| format!("this archive has no {name} — it is not a project export"))?;

        let mut text = String::new();

        entry
            .read_to_string(&mut text)
            .map_err(|err| format!("{name} could not be read: {err}"))?;

        serde_yaml::from_str(&text).map_err(|err| format!("{name} could not be understood: {err}"))
    }

    /// The same, for the three files an export with nothing in them may legitimately omit — a project with no
    /// goals writes an empty `goals.yaml`, but a hand-made archive is entitled to leave it out.
    fn read_yaml_or_default<TModel: serde::de::DeserializeOwned + Default>(
        &mut self,
        name: &str,
    ) -> Result<TModel, String> {
        if self.zip.index_for_name(name).is_none() {
            return Ok(TModel::default());
        }

        self.read_yaml(name)
    }

    /// Every entry under `documents/`, as a path within the project and its bytes.
    ///
    /// Read one at a time by the caller rather than collected: a document can be sixteen megabytes and there
    /// can be hundreds of them, so this hands back the names and the caller comes back for each payload.
    fn document_names(&self) -> Vec<String> {
        self.zip
            .file_names()
            .filter(|name| name.starts_with(DOCUMENTS_FOLDER) && !name.ends_with('/'))
            .map(|itm| itm.to_string())
            .collect()
    }

    fn read_entry(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let mut entry = self
            .zip
            .by_name(name)
            .map_err(|err| format!("it could not be found in the archive: {err}"))?;

        let mut bytes = Vec::with_capacity(entry.size() as usize);

        entry
            .read_to_end(&mut bytes)
            .map_err(|err| format!("it did not decompress: {err}"))?;

        Ok(bytes)
    }
}

/// Write every document in the archive, and answer with source path -> new document id.
///
/// Each one goes through `upload_document`, which is the same call the Documents screen and every MCP write
/// make: a path already holding a document gets a NEW VERSION of it rather than a duplicate, with the whole
/// history kept. That is the right behaviour for an import run twice, and it is the reason nothing here has
/// to check what is already on the board.
async fn write_documents(
    app: &AppContext,
    archive: &mut ImportArchive<'_>,
    project: &ProjectModel,
    who: &str,
    skipped: &mut Vec<SkippedImport>,
) -> Result<AHashMap<String, String>, String> {
    let mut written: AHashMap<String, String> = AHashMap::new();

    for name in archive.document_names() {
        let path = &name[DOCUMENTS_FOLDER.len()..];

        // The same rule every document path goes through, which is also the sanitiser: `..` is refused here
        // rather than allowed to name somewhere outside the project.
        let path = match task_manager_shared::documents::normalise_document_path(path) {
            Ok(path) => path,
            Err(problem) => {
                skipped.push(SkippedImport::new(name, problem));
                continue;
            }
        };

        // The reserved root is a connected repository's working copy on disk, not this project's to write:
        // an import that wrote there would edit a git checkout, and the repository is what puts files there.
        if task_manager_shared::github::is_github_path(&path) {
            skipped.push(SkippedImport::new(
                name,
                "it is under `github/`, which is a connected repository — connect the same repository instead",
            ));
            continue;
        }

        let bytes = match archive.read_entry(&name) {
            Ok(bytes) => bytes,
            Err(problem) => {
                skipped.push(SkippedImport::new(name, problem));
                continue;
            }
        };

        if bytes.is_empty() {
            skipped.push(SkippedImport::new(name, "it is empty"));
            continue;
        }

        // Text or bytes decided from the path and confirmed against the content — the same call the zip
        // upload makes, so a `.md` arrives as a document that renders and diffs rather than as a download.
        let content = super::super::NewDocumentContent {
            body: super::super::body_for_entry(&path, bytes),
            content_type: None,
        };

        match super::super::upload_document(app, &project.prefix, &path, content, who).await {
            Ok(row) => {
                written.insert(path, row.id);
            }
            Err(err) => skipped.push(SkippedImport::new(name, err)),
        }
    }

    Ok(written)
}

/// Source handle -> the number it gets on this board.
///
/// Reserved in one go, before anything is built, because a task can name a goal or a dependency that is
/// further down the file than it is.
struct Numbering {
    goals: AHashMap<String, i64>,
    tasks: AHashMap<String, i64>,
}

impl Numbering {
    fn reserve(
        app: &AppContext,
        project: &ProjectModel,
        goals_file: &GoalsFile,
        tasks_file: &TasksFile,
    ) -> Result<Self, String> {
        let amount = (goals_file.goals.len() + tasks_file.tasks.len()) as i64;

        if amount == 0 {
            return Ok(Self {
                goals: AHashMap::new(),
                tasks: AHashMap::new(),
            });
        }

        let reserved = app
            .board
            .reserve_task_numbers(&project.id, amount)
            .ok_or_else(|| {
                format!(
                    "project {} vanished while the import was reserving numbers",
                    project.prefix
                )
            })?;

        let mut numbers = reserved.into_iter();

        let mut goals = AHashMap::with_capacity(goals_file.goals.len());
        let mut tasks = AHashMap::with_capacity(tasks_file.tasks.len());

        // In file order, so the cards land on this board in the order they were written on the other one.
        //
        // **A repeated id is refused, and it has to be**: the map is what every reference in the file
        // resolves through, so two cards spelled `TM-42` would both be built against the second one's number
        // and the second would overwrite the first in Postgres — losing a card and silently re-pointing
        // whatever depended on it. An export cannot produce that; a hand-edited file can.
        for goal in &goals_file.goals {
            let number = numbers.next().expect("one number per card was reserved");

            if goals.insert(normalise_handle(&goal.id), number).is_some() {
                return Err(format!(
                    "'{}' is in {GOALS_FILE} more than once — an id names one goal",
                    goal.id
                ));
            }
        }

        for task in &tasks_file.tasks {
            let number = numbers.next().expect("one number per card was reserved");

            if tasks.insert(normalise_handle(&task.id), number).is_some() {
                return Err(format!(
                    "'{}' is in {TASKS_FILE} more than once — an id names one task",
                    task.id
                ));
            }
        }

        Ok(Self { goals, tasks })
    }
}

/// A handle as it is looked up: upper-cased and with the spaces off.
///
/// The prefix is NOT rewritten to the source's — a file naming `TM-7` in one place and `tm-7` in another is
/// one card either way, and a file whose handles name a third project is a file this import will not resolve,
/// which is exactly what should happen.
fn normalise_handle(src: &str) -> String {
    src.trim().to_uppercase()
}

/// Every comment in the file, grouped by what it hangs on.
///
/// Grouped rather than searched per card: a board can carry thousands of comments, and a scan per task would
/// be quadratic. What is left in the map when every card has been built is the orphans, which the caller
/// reports.
fn group_comments(
    file: &CommentsFile,
    skipped: &mut Vec<SkippedImport>,
) -> AHashMap<String, Vec<CommentModel>> {
    let mut grouped: AHashMap<String, Vec<CommentModel>> = AHashMap::new();

    for comment in &file.comments {
        let moment = match decode_moment(&comment.moment, "a comment's moment") {
            Ok(moment) => moment,
            Err(problem) => {
                skipped.push(SkippedImport::new(comment.target.clone(), problem));
                continue;
            }
        };

        let text = match decode_text(&comment.text_base64, "a comment's text") {
            Ok(text) => text,
            Err(problem) => {
                skipped.push(SkippedImport::new(comment.target.clone(), problem));
                continue;
            }
        };

        grouped
            .entry(normalise_handle(&comment.target))
            .or_default()
            .push(CommentModel {
                moment,
                // Kept exactly as it was. A thread says who said what, and re-signing it with whoever pressed
                // Import would make the record false.
                who: comment.who.clone(),
                text,
            });
    }

    // Oldest first within one card, which is the order a thread is read in and the order every other writer
    // in this service appends in.
    for comments in grouped.values_mut() {
        comments.sort_by_key(|itm| itm.moment.unix_microseconds);
    }

    grouped
}

fn build_goal(
    project: &ProjectModel,
    src: &GoalFileModel,
    numbers: &Numbering,
    comments: &mut AHashMap<String, Vec<CommentModel>>,
    documents: &AHashMap<String, String>,
) -> Result<GoalModel, String> {
    let handle = normalise_handle(&src.id);

    if parse_goal_handle(&handle).is_none() {
        return Err(format!(
            "'{}' is not a goal id — expected something like TM-G1",
            src.id
        ));
    }

    let number = *numbers
        .goals
        .get(&handle)
        .ok_or_else(|| format!("'{}' has no number reserved for it", src.id))?;

    Ok(GoalModel {
        project_id: project.id.clone(),
        number,
        name: decode_text(&src.name_base64, "a goal's name")?,
        description: decode_text(&src.description_base64, "a goal's description")?,
        // An unrecognised colour reads as the default swatch, exactly as one stored by a build that knew a
        // colour this one does not — the same leniency, and for the same reason.
        color: KindColor::parse_or_default(&src.color),
        priority: Priority::parse_or_default(&src.priority),
        subtasks: build_subtasks(&src.subtasks)?,
        documents: resolve_documents(&src.documents, documents),
        comments: comments.remove(&handle).unwrap_or_default(),
        created: decode_moment(&src.created, "a goal's created")?,
        updated: decode_moment(&src.updated, "a goal's updated")?,
        close_moment: decode_optional_moment(src.closed.as_deref(), "a goal's closed")?,
        deleted_moment: decode_optional_moment(src.deleted.as_deref(), "a goal's deleted")?,
    })
}

fn build_task(
    project: &ProjectModel,
    src: &TaskFileModel,
    numbers: &Numbering,
    comments: &mut AHashMap<String, Vec<CommentModel>>,
    documents: &AHashMap<String, String>,
    skipped: &mut Vec<SkippedImport>,
) -> Result<TaskModel, String> {
    let handle = normalise_handle(&src.id);

    if parse_task_handle(&handle).is_none() {
        return Err(format!(
            "'{}' is not a task id — expected something like TM-42",
            src.id
        ));
    }

    let number = *numbers
        .tasks
        .get(&handle)
        .ok_or_else(|| format!("'{}' has no number reserved for it", src.id))?;

    // A goal naming something the archive does not carry leaves the task standalone rather than refusing it:
    // the work is real and the grouping is not worth losing it over. Reported, so nobody has to notice.
    let goal_number = match src.goal.as_deref().map(str::trim).filter(|itm| !itm.is_empty()) {
        None => None,
        Some(goal) => {
            let goal_handle = normalise_handle(goal);

            match numbers.goals.get(&goal_handle) {
                Some(number) => Some(*number),
                None => {
                    skipped.push(SkippedImport::new(
                        src.id.clone(),
                        format!("its goal '{goal}' is not in this archive — imported standalone"),
                    ));
                    None
                }
            }
        }
    };

    // Same leniency, same reason, and one line per lost edge rather than one refusal: a dependency on
    // something that is not in the file cannot be expressed on this board.
    let mut depends_on = Vec::with_capacity(src.depends_on.len());

    for dependency in &src.depends_on {
        let dependency_handle = normalise_handle(dependency);

        match numbers.tasks.get(&dependency_handle) {
            Some(number) => depends_on.push(*number),
            None => skipped.push(SkippedImport::new(
                src.id.clone(),
                format!("it depends on '{dependency}', which is not in this archive — the dependency was dropped"),
            )),
        }
    }

    depends_on.sort_unstable();
    depends_on.dedup();

    let mut gh_actions = Vec::with_capacity(src.gh_actions.len());

    for action in &src.gh_actions {
        gh_actions.push(GhActionModel {
            url: action.url.clone(),
            title: decode_text(&action.title_base64, "a build link's title")?,
            moment: decode_moment(&action.moment, "a build link's moment")?,
        });
    }

    Ok(TaskModel {
        project_id: project.id.clone(),
        number,
        text: decode_text(&src.text_base64, "a task's text")?,
        // Kept verbatim, even when this project's template has no such column. `effective_status` reads an
        // unknown one as Todo and leaves the stored value alone, so pointing the project at the right
        // template afterwards brings every one of these tasks to where it belongs — which rewriting them
        // here would have made impossible. The count is reported.
        status: src.status.trim().to_lowercase(),
        priority: Priority::parse_or_default(&src.priority),
        kind: src
            .kind
            .as_deref()
            .map(|itm| itm.trim().to_lowercase())
            .filter(|itm| !itm.is_empty()),
        goal_number,
        assignee: src
            .assignee
            .as_deref()
            .map(str::trim)
            .filter(|itm| !itm.is_empty())
            .map(super::super::normalise_actor),
        labels: normalise_labels(&src.labels),
        depends_on,
        subtasks: build_subtasks(&src.subtasks)?,
        documents: resolve_documents(&src.documents, documents),
        gh_actions,
        comments: comments.remove(&handle).unwrap_or_default(),
        created: decode_moment(&src.created, "a task's created")?,
        updated: decode_moment(&src.updated, "a task's updated")?,
        close_moment: decode_optional_moment(src.closed.as_deref(), "a task's closed")?,
        deleted_moment: decode_optional_moment(src.deleted.as_deref(), "a task's deleted")?,
    })
}

/// Checklist items, with fresh ids.
///
/// The id is minted here rather than carried: it is a `SortableId` nobody ever sees, and two boards holding
/// the same one for two items that are only coincidentally the same is a collision waiting to be found by
/// whatever ticks one of them.
fn build_subtasks(src: &[SubtaskFileModel]) -> Result<Vec<SubtaskModel>, String> {
    let mut result = Vec::with_capacity(src.len());

    for item in src {
        result.push(SubtaskModel {
            id: rust_extensions::SortableId::generate().to_string(),
            title: decode_text(&item.title_base64, "a checklist item's title")?,
            text: decode_text(&item.text_base64, "a checklist item's text")?,
            done: item.done,
        });
    }

    Ok(result)
}

/// Document references: paths in the file -> ids on this board.
///
/// A path that did not arrive — skipped as noise, refused, or simply not in the archive — drops out of the
/// list. A reference to a document that is not here would resolve to nothing on the screen that draws it,
/// which is a worse answer than one fewer reference.
fn resolve_documents(paths: &[String], documents: &AHashMap<String, String>) -> Vec<String> {
    let mut ids: Vec<String> = paths
        .iter()
        .filter_map(|path| documents.get(path.trim()).cloned())
        .collect();

    // Sorted and de-duplicated, which for a sortable id is also oldest first — the shape every other writer
    // of this list leaves it in.
    ids.sort();
    ids.dedup();
    ids
}

/// Lower-case, trim, drop blanks, de-duplicate, sort — the same normalisation a label gets on every other
/// write, so a board built by import compares equal to one built by hand.
fn normalise_labels(src: &[String]) -> Vec<String> {
    let mut labels: Vec<String> = src
        .iter()
        .map(|itm| itm.trim().to_lowercase())
        .filter(|itm| !itm.is_empty())
        .collect();

    labels.sort();
    labels.dedup();
    labels
}

fn decode_optional_moment(
    src: Option<&str>,
    what: &str,
) -> Result<Option<DateTimeAsMicroseconds>, String> {
    match src.map(str::trim).filter(|itm| !itm.is_empty()) {
        None => Ok(None),
        Some(src) => Ok(Some(decode_moment(src, what)?)),
    }
}

/// Replace the project's settings with the file's, and persist the counter in the same write.
///
/// Both halves have to land in one row: the counter moved when the numbers were reserved, and a project row
/// written from a copy captured before that would undo the reservation. So the project is re-read from
/// memory here, the settings are laid over it, and it is saved once.
async fn apply_settings(
    app: &AppContext,
    project: &ProjectModel,
    file: &ProjectFile,
    telemetry: &MyTelemetryContext,
    notes: &mut Vec<String>,
) {
    // Re-read, so the counter this writes is the one the reservation left behind.
    let Some(current) = app.board.read().get_project(&project.id) else {
        return;
    };

    let mut updated = current.as_ref().clone();

    if let Ok(name) = decode_text(&file.project.name_base64, "the project's name") {
        let name = name.trim().to_string();

        if !name.is_empty() {
            updated.name = name;
        }
    }

    if let Ok(description) = decode_text(&file.project.description_base64, "the project's description") {
        updated.description = description.trim().to_string();
    }

    updated.archive_days = file.project.archive_days;

    {
        let board = app.board.read();

        // A template id is taken only when a template with that id is on this instance. Pointing the project
        // at one that is not here would leave it looking configured while its board had nothing in the
        // middle — the same refusal `set_column_template` makes, for the same reason.
        match file.project.column_template_id.as_deref() {
            None => updated.column_template_id = None,
            Some(id) if board.get_column_template(id).is_some() => {
                updated.column_template_id = Some(id.to_string());
            }
            Some(id) => notes.push(format!(
                "the export follows column template '{id}', which is not on this instance — this project's columns were left as they were"
            )),
        }

        match file.project.kind_template_id.as_deref() {
            None => updated.kind_template_id = None,
            Some(id) if board.get_kind_template(id).is_some() => {
                updated.kind_template_id = Some(id.to_string());
            }
            Some(id) => notes.push(format!(
                "the export follows task-type template '{id}', which is not on this instance — this project's task types were left as they were"
            )),
        }

        // The prefix last, and only when nobody else holds it. Two projects cannot share one, and the common
        // case for this feature — copying a board on the instance the original still lives on — is exactly
        // the case where it is taken.
        let wanted = file.project.prefix.trim().to_uppercase();

        if wanted != updated.prefix {
            if board.is_prefix_free(&wanted, Some(&project.id)) {
                // The prefix being left behind goes into history, so an id written under it can still be
                // traced — the same bookkeeping a rename does.
                if !updated.prefix_history.contains(&updated.prefix) {
                    updated.prefix_history.push(updated.prefix.clone());
                }

                updated.prefix = wanted;
            } else {
                notes.push(format!(
                    "the export came from prefix '{wanted}', which another project holds — this one kept '{}'",
                    updated.prefix
                ));
            }
        }
    }

    let dto: crate::postgres::ProjectDto = (&updated).into();
    app.projects_repo.upsert(&dto, telemetry).await;
    app.board.upsert_project(updated);
}

/// What landed but reads differently here than it did on the board it came from.
///
/// Only the statuses and the kinds, because they are the only two open vocabularies a card carries: a
/// priority is a product-wide enum and an assignee is an email, neither of which can name something this
/// project does not have.
fn read_notes(app: &AppContext, project_id: &str, tasks: &[TaskModel]) -> Vec<String> {
    let Some(project) = app.board.read().get_project(project_id) else {
        return Vec::new();
    };

    let mut unknown_statuses: Vec<String> = tasks
        .iter()
        .map(|itm| itm.status.clone())
        .filter(|status| !project.has_column(status))
        .collect();

    unknown_statuses.sort();
    unknown_statuses.dedup();

    let mut unknown_kinds: Vec<String> = tasks
        .iter()
        .filter_map(|itm| itm.kind.clone())
        .filter(|kind| !project.has_kind(kind))
        .collect();

    unknown_kinds.sort();
    unknown_kinds.dedup();

    let mut notes = Vec::new();

    if !unknown_statuses.is_empty() {
        notes.push(format!(
            "{} has no column for: {} — those tasks kept the status they arrived with and read as Todo until the column exists",
            project.prefix,
            unknown_statuses.join(", ")
        ));
    }

    if !unknown_kinds.is_empty() {
        notes.push(format!(
            "{} has no task type for: {} — those tasks kept the type they arrived with and read as having none",
            project.prefix,
            unknown_kinds.join(", ")
        ));
    }

    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handle is matched however it was spelled — the file is hand-editable, and a lower-cased id in it is
    /// the same card as an upper-cased one everywhere else in this product.
    #[test]
    fn a_handle_is_matched_whatever_case_it_was_written_in() {
        assert_eq!(normalise_handle(" tm-42 "), "TM-42");
        assert_eq!(normalise_handle("TM-G7"), "TM-G7");
    }

    /// A reference to a document that did not arrive drops out rather than pointing at nothing — and what
    /// does arrive comes out sorted and unique, the shape every other writer of this list leaves it in.
    #[test]
    fn a_reference_to_a_document_that_did_not_arrive_is_dropped() {
        let mut documents = AHashMap::new();
        documents.insert("docs/a.md".to_string(), "id-b".to_string());
        documents.insert("docs/b.md".to_string(), "id-a".to_string());

        let ids = resolve_documents(
            &[
                "docs/b.md".to_string(),
                "docs/missing.md".to_string(),
                " docs/a.md ".to_string(),
                "docs/b.md".to_string(),
            ],
            &documents,
        );

        assert_eq!(ids, vec!["id-a".to_string(), "id-b".to_string()]);
    }

    /// Labels arrive in whatever shape the file has them and leave in the one every other write produces,
    /// or two boards holding the same tags would not compare equal.
    #[test]
    fn labels_are_normalised_the_way_every_other_write_normalises_them() {
        let labels = normalise_labels(&[
            " Backend ".to_string(),
            "backend".to_string(),
            "".to_string(),
            "API".to_string(),
        ]);

        assert_eq!(labels, vec!["api".to_string(), "backend".to_string()]);
    }

    /// An absent moment and an empty one are the same fact — a card that was never closed — and neither is
    /// the epoch.
    #[test]
    fn a_moment_that_is_not_there_reads_as_nothing() {
        assert!(decode_optional_moment(None, "x").unwrap().is_none());
        assert!(decode_optional_moment(Some("  "), "x").unwrap().is_none());
        assert!(decode_optional_moment(Some("nonsense"), "x").is_err());

        let moment = decode_optional_moment(Some("2026-08-06T09:15:00.000000Z"), "x")
            .unwrap()
            .expect("a moment");

        assert_eq!(moment.to_rfc3339_utc(), "2026-08-06T09:15:00.000000Z");
    }

    /// Prose survives the round trip with every character YAML would otherwise have an opinion about.
    #[test]
    fn prose_round_trips_through_base64() {
        let text = "# Heading\n\n- item: with a colon\n  \"quoted\"\n\tTabbed\n";

        assert_eq!(decode_text(&encode_text(text), "x").unwrap(), text);
    }

    /// Builds an archive the way the export builds one, so these tests read a real zip rather than a
    /// stand-in for one.
    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

        for (name, bytes) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }

        writer.finish().unwrap().into_inner()
    }

    fn a_project_file() -> Vec<u8> {
        let model = ProjectFile {
            format: FORMAT.to_string(),
            exported: "2026-08-06T09:00:00.000000Z".to_string(),
            project: ProjectFileProject {
                prefix: "TM".to_string(),
                name_base64: encode_text("Task manager"),
                description_base64: encode_text("The board"),
                column_template_id: Some("default".to_string()),
                kind_template_id: None,
                archive_days: Some(14),
            },
            contents: ProjectFileContents {
                goals: 1,
                tasks: 1,
                comments: 1,
                documents: 1,
            },
        };

        serde_yaml::to_string(&model).unwrap().into_bytes()
    }

    /// **The format's own round trip.** Everything the export writes has to come back as what it was — this
    /// is the one test that reads the four files as a set, and it is what would catch a field renamed on one
    /// side only.
    #[test]
    fn what_the_export_writes_is_what_the_import_reads() {
        let tasks = TasksFile {
            tasks: vec![TaskFileModel {
                id: "TM-42".to_string(),
                text_base64: encode_text("Ship it:\n- properly\n"),
                status: "review".to_string(),
                priority: "high".to_string(),
                kind: Some("bug".to_string()),
                goal: Some("TM-G7".to_string()),
                assignee: Some("yuri@mxtm.ai".to_string()),
                labels: vec!["backend".to_string()],
                depends_on: vec!["TM-4".to_string()],
                subtasks: vec![SubtaskFileModel {
                    title_base64: encode_text("first"),
                    text_base64: encode_text(""),
                    done: true,
                }],
                documents: vec!["docs/a.md".to_string()],
                gh_actions: vec![GhActionFileModel {
                    url: "https://github.com/o/r/actions/runs/1".to_string(),
                    title_base64: encode_text("build #1"),
                    moment: "2026-08-06T09:00:00.000000Z".to_string(),
                }],
                created: "2026-08-01T09:00:00.000000Z".to_string(),
                updated: "2026-08-05T09:00:00.000000Z".to_string(),
                closed: None,
                deleted: None,
            }],
        };

        let archive = zip_of(&[
            (PROJECT_FILE, &a_project_file()),
            (
                TASKS_FILE,
                serde_yaml::to_string(&tasks).unwrap().as_bytes(),
            ),
            ("documents/docs/a.md", b"# hello"),
        ]);

        let mut read = ImportArchive::open(&archive).unwrap();

        let project = read.read_yaml::<ProjectFile>(PROJECT_FILE).unwrap();
        assert_eq!(project.format, FORMAT);
        assert_eq!(project.project.prefix, "TM");
        assert_eq!(project.project.archive_days, Some(14));
        assert_eq!(
            decode_text(&project.project.name_base64, "x").unwrap(),
            "Task manager"
        );

        let back = read.read_yaml::<TasksFile>(TASKS_FILE).unwrap();
        let task = &back.tasks[0];

        assert_eq!(task.id, "TM-42");
        assert_eq!(
            decode_text(&task.text_base64, "x").unwrap(),
            "Ship it:\n- properly\n"
        );
        assert_eq!(task.status, "review");
        assert_eq!(task.goal.as_deref(), Some("TM-G7"));
        assert_eq!(task.depends_on, vec!["TM-4".to_string()]);
        assert_eq!(task.documents, vec!["docs/a.md".to_string()]);
        assert_eq!(task.gh_actions.len(), 1);

        assert_eq!(read.document_names(), vec!["documents/docs/a.md".to_string()]);
        assert_eq!(read.read_entry("documents/docs/a.md").unwrap(), b"# hello");
    }

    /// The three list files are the ones a hand-made archive may leave out, and an absent one means "none of
    /// those" rather than "this is not an export".
    #[test]
    fn the_list_files_may_be_absent() {
        let archive = zip_of(&[(PROJECT_FILE, &a_project_file())]);
        let mut read = ImportArchive::open(&archive).unwrap();

        assert!(read.read_yaml_or_default::<GoalsFile>(GOALS_FILE).unwrap().goals.is_empty());
        assert!(read.read_yaml_or_default::<TasksFile>(TASKS_FILE).unwrap().tasks.is_empty());
        assert!(
            read.read_yaml_or_default::<CommentsFile>(COMMENTS_FILE)
                .unwrap()
                .comments
                .is_empty()
        );
        assert!(read.document_names().is_empty());
    }

    /// `project.yaml` is what makes an archive a project export. Without it there is nothing to check the
    /// format against, and the message has to say so rather than reporting an empty import.
    #[test]
    fn an_archive_that_is_not_an_export_is_refused_by_name() {
        let archive = zip_of(&[("readme.md", b"# not an export")]);
        let mut read = ImportArchive::open(&archive).unwrap();

        let problem = read.read_yaml::<ProjectFile>(PROJECT_FILE).unwrap_err();

        assert!(problem.contains(PROJECT_FILE), "{problem}");
        assert!(ImportArchive::open(b"").is_err());
        assert!(ImportArchive::open(b"not a zip at all").is_err());
    }

    /// The receiving board: a project that already has forty cards on it, so a number handed to an imported
    /// task can never be one the file asked for.
    fn a_target_project() -> ProjectModel {
        ProjectModel {
            id: "p1".to_string(),
            name: "Target".to_string(),
            description: String::new(),
            prefix: "NEW".to_string(),
            prefix_history: Vec::new(),
            column_template_id: None,
            columns: Vec::new(),
            kind_template_id: None,
            kinds: Vec::new(),
            members: std::collections::BTreeSet::new(),
            last_task_number: 40,
            archive_days: None,
            archived_moment: None,
            github_connections: Vec::new(),
            created: DateTimeAsMicroseconds::new(0),
        }
    }

    /// The numbering a reservation would have produced, without a board to reserve from.
    fn numbering(goals: &[(&str, i64)], tasks: &[(&str, i64)]) -> Numbering {
        Numbering {
            goals: goals
                .iter()
                .map(|(handle, number)| (handle.to_string(), *number))
                .collect(),
            tasks: tasks
                .iter()
                .map(|(handle, number)| (handle.to_string(), *number))
                .collect(),
        }
    }

    fn a_task_file_model(id: &str) -> TaskFileModel {
        TaskFileModel {
            id: id.to_string(),
            text_base64: encode_text("do the thing"),
            status: "done".to_string(),
            priority: "high".to_string(),
            kind: Some("BUG".to_string()),
            goal: None,
            assignee: Some(" Yuri@MXTM.ai ".to_string()),
            labels: Vec::new(),
            depends_on: Vec::new(),
            subtasks: Vec::new(),
            documents: Vec::new(),
            gh_actions: Vec::new(),
            created: "2026-08-01T09:00:00.000000Z".to_string(),
            updated: "2026-08-05T09:00:00.000000Z".to_string(),
            closed: Some("2026-08-05T09:00:00.000000Z".to_string()),
            deleted: None,
        }
    }

    /// **The heart of an import.** Every reference in the file is a handle from the board it came from, and
    /// every one of them has to come out as a number on THIS board — the goal, the dependencies, and nothing
    /// left pointing at what the file said.
    #[test]
    fn every_reference_is_remapped_onto_this_boards_numbers() {
        let project = a_target_project();
        let numbers = numbering(&[("TM-G7", 41)], &[("TM-42", 42), ("TM-4", 43)]);

        let mut src = a_task_file_model("TM-42");
        src.goal = Some("TM-G7".to_string());
        src.depends_on = vec!["TM-4".to_string()];

        let mut comments = AHashMap::new();
        let mut skipped = Vec::new();

        let task = build_task(
            &project,
            &src,
            &numbers,
            &mut comments,
            &AHashMap::new(),
            &mut skipped,
        )
        .expect("the task should build");

        assert!(skipped.is_empty(), "nothing was lost");
        assert_eq!(task.number, 42, "the number this board handed out");
        assert_eq!(task.goal_number, Some(41), "the goal's NEW number, not G7");
        assert_eq!(task.depends_on, vec![43], "the blocker's NEW number, not 4");
        assert_eq!(task.project_id, "p1");
    }

    /// Everything that is not a reference arrives exactly as it left — including the moments, which is what
    /// makes an imported board read as the same history rather than as forty cards created today.
    #[test]
    fn what_is_not_a_reference_arrives_unchanged() {
        let project = a_target_project();
        let numbers = numbering(&[], &[("TM-42", 42)]);

        let mut src = a_task_file_model("TM-42");
        src.labels = vec![" Backend ".to_string(), "backend".to_string()];

        let task = build_task(
            &project,
            &src,
            &numbers,
            &mut AHashMap::new(),
            &AHashMap::new(),
            &mut Vec::new(),
        )
        .expect("the task should build");

        assert_eq!(task.text, "do the thing");
        assert_eq!(task.status, "done");
        assert_eq!(rust_extensions::AsStr::as_str(&task.priority), "high");
        // Lower-cased, like every other write of one — a kind id is an open vocabulary, not free text.
        assert_eq!(task.kind.as_deref(), Some("bug"));
        assert_eq!(task.assignee.as_deref(), Some("yuri@mxtm.ai"));
        assert_eq!(task.labels, vec!["backend".to_string()]);
        assert_eq!(task.created.to_rfc3339_utc(), "2026-08-01T09:00:00.000000Z");
        assert_eq!(task.updated.to_rfc3339_utc(), "2026-08-05T09:00:00.000000Z");
        assert_eq!(
            task.close_moment.map(|itm| itm.to_rfc3339_utc()),
            Some("2026-08-05T09:00:00.000000Z".to_string())
        );
        assert!(task.deleted_moment.is_none());
    }

    /// A reference to something the archive does not carry cannot be expressed here — the work is still real,
    /// so it arrives without the edge and the loss is reported rather than swallowed.
    #[test]
    fn a_reference_to_something_outside_the_archive_is_dropped_and_said() {
        let project = a_target_project();
        let numbers = numbering(&[], &[("TM-42", 42)]);

        let mut src = a_task_file_model("TM-42");
        src.goal = Some("TM-G9".to_string());
        src.depends_on = vec!["TM-4".to_string()];

        let mut skipped = Vec::new();

        let task = build_task(
            &project,
            &src,
            &numbers,
            &mut AHashMap::new(),
            &AHashMap::new(),
            &mut skipped,
        )
        .expect("the task itself is still worth having");

        assert_eq!(task.goal_number, None);
        assert!(task.depends_on.is_empty());
        assert_eq!(skipped.len(), 2, "the goal and the dependency, one line each");
        assert!(skipped.iter().all(|itm| itm.name == "TM-42"));
    }

    /// A card's thread comes off the map and comes off it ONCE — what is left when every card has been built
    /// is the orphans, and a card that took its comments twice would report the rest of the board as
    /// orphaned.
    #[test]
    fn a_card_takes_its_thread_out_of_the_map() {
        let project = a_target_project();
        let numbers = numbering(&[], &[("TM-42", 42)]);

        let mut comments = AHashMap::new();
        comments.insert(
            "TM-42".to_string(),
            vec![CommentModel {
                moment: DateTimeAsMicroseconds::new(0),
                who: "AI".to_string(),
                text: "did it".to_string(),
            }],
        );
        comments.insert("TM-99".to_string(), Vec::new());

        let task = build_task(
            &project,
            &a_task_file_model("TM-42"),
            &numbers,
            &mut comments,
            &AHashMap::new(),
            &mut Vec::new(),
        )
        .expect("the task should build");

        assert_eq!(task.comments.len(), 1);
        assert_eq!(task.comments[0].who, "AI");
        assert!(!comments.contains_key("TM-42"), "its thread was taken");
        assert!(comments.contains_key("TM-99"), "somebody else's was not");
    }

    /// A goal handle where a task is expected — and the other way round — is a file somebody has edited into
    /// something this board cannot read, and it says so rather than building a card with a nonsense id.
    #[test]
    fn a_card_whose_id_is_not_a_handle_is_refused_by_name() {
        let project = a_target_project();
        let numbers = numbering(&[], &[("TM-42", 42)]);

        for id in ["TM-G7", "not-a-handle", ""] {
            let problem = build_task(
                &project,
                &a_task_file_model(id),
                &numbers,
                &mut AHashMap::new(),
                &AHashMap::new(),
                &mut Vec::new(),
            )
            .unwrap_err();

            assert!(problem.contains(id) || id.is_empty(), "{problem}");
        }
    }

    /// A comment lands on the card it names, and one naming nothing is left in the map for the caller to
    /// report — which is the whole reason this returns a map rather than attaching as it goes.
    #[test]
    fn comments_are_grouped_by_card_and_oldest_first() {
        let file = CommentsFile {
            comments: vec![
                CommentFileModel {
                    target: "TM-42".to_string(),
                    moment: "2026-08-06T10:00:00.000000Z".to_string(),
                    who: "yuri@mxtm.ai".to_string(),
                    text_base64: encode_text("second"),
                },
                CommentFileModel {
                    target: "tm-42".to_string(),
                    moment: "2026-08-06T09:00:00.000000Z".to_string(),
                    who: "AI".to_string(),
                    text_base64: encode_text("first"),
                },
                CommentFileModel {
                    target: "TM-G7".to_string(),
                    moment: "2026-08-06T09:30:00.000000Z".to_string(),
                    who: "AI".to_string(),
                    text_base64: encode_text("on the goal"),
                },
            ],
        };

        let mut skipped = Vec::new();
        let grouped = group_comments(&file, &mut skipped);

        assert!(skipped.is_empty());

        // Both spellings of the handle are one card, and the thread reads oldest first.
        let task = grouped.get("TM-42").expect("the task's thread");
        assert_eq!(task.len(), 2);
        assert_eq!(task[0].text, "first");
        assert_eq!(task[1].text, "second");
        // The author is kept, never re-signed by whoever pressed Import.
        assert_eq!(task[0].who, "AI");

        assert_eq!(grouped.get("TM-G7").expect("the goal's thread").len(), 1);
    }
}
