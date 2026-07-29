use std::sync::Arc;

use ahash::{AHashMap, AHashSet};

use super::models::{ProjectModel, TaskModel, UserModel};

/// The whole product state, indexed, as one immutable snapshot.
///
/// Every read serves from here and takes no lock — the wrapper hands out an `Arc` of it. Writes
/// clone it, mutate the clone and swap it in, which is why every collection holds `Arc`s: a clone of
/// this struct copies pointers, not tasks.
///
/// Indexes are derived rather than maintained incrementally ([`BoardInner::rebuild_indexes`] runs
/// after every mutation). At this size that is free, and it removes the entire class of bug where an
/// index and its source disagree.
#[derive(Clone)]
pub struct BoardInner {
    projects: AHashMap<String, Arc<ProjectModel>>,
    /// Upper-cased current prefix -> project id. One entry per project: two projects may not hold
    /// the same prefix at the same time, which is exactly what makes this a map and not a multimap.
    prefix_index: AHashMap<String, String>,
    /// Upper-cased prefix -> every project that has *ever* held it, current holder included.
    /// A multimap, because a prefix is free to move on once renamed away from.
    historical_prefix_index: AHashMap<String, Vec<String>>,
    /// project id -> task number -> task.
    tasks: AHashMap<String, AHashMap<i64, Arc<TaskModel>>>,
    /// Lower-cased email -> user.
    users: AHashMap<String, Arc<UserModel>>,
    /// Cached whole lists so a "give me everything" read clones an `Arc` instead of allocating.
    projects_list: Arc<Vec<Arc<ProjectModel>>>,
    users_list: Arc<Vec<Arc<UserModel>>>,
}

impl BoardInner {
    pub fn new() -> Self {
        Self {
            projects: AHashMap::new(),
            prefix_index: AHashMap::new(),
            historical_prefix_index: AHashMap::new(),
            tasks: AHashMap::new(),
            users: AHashMap::new(),
            projects_list: Arc::new(Vec::new()),
            users_list: Arc::new(Vec::new()),
        }
    }

    /// Build a snapshot out of what the startup load read from Postgres.
    ///
    /// The one place the counter invariant lives: each project's `last_task_number` is floored at the
    /// highest number any of its surviving tasks carries. A counter row that is missing or has fallen
    /// behind therefore cannot re-issue a live number — and since the floor is computed from the
    /// tasks that actually exist, a number lost to a crash before its task row was written is simply
    /// never used.
    ///
    /// A task whose project is gone is dropped: there is nothing to compose its handle from and no
    /// board to draw it on. That cannot happen while project deletion is unimplemented, which is
    /// exactly why it is worth handling now rather than when it can.
    pub fn from_loaded(
        projects: Vec<ProjectModel>,
        tasks: Vec<TaskModel>,
        users: Vec<UserModel>,
    ) -> Self {
        let mut result = Self::new();

        let mut highest_number: AHashMap<&str, i64> = AHashMap::new();
        for task in &tasks {
            let entry = highest_number.entry(task.project_id.as_str()).or_insert(0);
            if task.number > *entry {
                *entry = task.number;
            }
        }

        for mut project in projects {
            let floor = highest_number
                .get(project.id.as_str())
                .copied()
                .unwrap_or(0);

            if project.last_task_number < floor {
                project.last_task_number = floor;
            }

            result.put_project(Arc::new(project));
        }

        for task in tasks {
            if result.projects.contains_key(&task.project_id) {
                result.put_task(Arc::new(task));
            }
        }

        for user in users {
            result.put_user(Arc::new(user));
        }

        result.rebuild_indexes();
        result
    }

    // ----------------------------------------------------------------- mutation (crate-internal)

    pub(super) fn put_project(&mut self, project: Arc<ProjectModel>) {
        self.tasks.entry(project.id.clone()).or_default();
        self.projects.insert(project.id.clone(), project);
    }

    pub(super) fn put_task(&mut self, task: Arc<TaskModel>) {
        self.tasks
            .entry(task.project_id.clone())
            .or_default()
            .insert(task.number, task);
    }

    pub(super) fn drop_task(&mut self, project_id: &str, number: i64) {
        if let Some(of_project) = self.tasks.get_mut(project_id) {
            of_project.remove(&number);
        }
    }

    pub(super) fn put_user(&mut self, user: Arc<UserModel>) {
        self.users.insert(user.email.clone(), user);
    }

    /// Recompute everything derived. Called once at the end of every mutation and once after the
    /// startup load.
    pub(super) fn rebuild_indexes(&mut self) {
        self.prefix_index.clear();
        self.historical_prefix_index.clear();

        for project in self.projects.values() {
            self.prefix_index
                .insert(project.prefix.clone(), project.id.clone());

            // The current prefix counts as history too, so `tasks_resolve_id` can report the live
            // holder and the past ones through one index.
            for prefix in std::iter::once(&project.prefix).chain(project.prefix_history.iter()) {
                let owners = self
                    .historical_prefix_index
                    .entry(prefix.clone())
                    .or_default();

                if !owners.contains(&project.id) {
                    owners.push(project.id.clone());
                }
            }
        }

        // Sorted so two identical reads produce identical output — an unordered map would reshuffle
        // between them and read as a change that did not happen.
        for owners in self.historical_prefix_index.values_mut() {
            owners.sort_unstable();
        }

        let mut projects: Vec<Arc<ProjectModel>> = self.projects.values().cloned().collect();
        projects.sort_by_key(|itm| itm.name.to_lowercase());
        self.projects_list = Arc::new(projects);

        let mut users: Vec<Arc<UserModel>> = self.users.values().cloned().collect();
        users.sort_by(|l, r| l.email.cmp(&r.email));
        self.users_list = Arc::new(users);
    }

    // ---------------------------------------------------------------------------------- reading

    pub fn get_project(&self, project_id: &str) -> Option<Arc<ProjectModel>> {
        self.projects.get(project_id).cloned()
    }

    /// The project holding this prefix **right now**. `prefix` must already be upper-cased — a
    /// parsed handle is.
    pub fn get_project_by_prefix(&self, prefix: &str) -> Option<Arc<ProjectModel>> {
        let project_id = self.prefix_index.get(prefix)?;
        self.projects.get(project_id).cloned()
    }

    /// Every project that has ever held this prefix, including the current holder, sorted by id.
    pub fn projects_ever_holding_prefix(&self, prefix: &str) -> Vec<Arc<ProjectModel>> {
        let Some(owners) = self.historical_prefix_index.get(prefix) else {
            return Vec::new();
        };

        owners
            .iter()
            .filter_map(|id| self.projects.get(id).cloned())
            .collect()
    }

    /// Whether a prefix can be taken. Free means nobody holds it as their current prefix — a prefix
    /// only in some project's history is fair game, which is the decision that made task handles
    /// composed-on-read rather than stored.
    pub fn is_prefix_free(&self, prefix: &str, ignoring_project_id: Option<&str>) -> bool {
        match self.prefix_index.get(prefix) {
            None => true,
            Some(holder) => Some(holder.as_str()) == ignoring_project_id,
        }
    }

    pub fn projects(&self) -> Arc<Vec<Arc<ProjectModel>>> {
        self.projects_list.clone()
    }

    pub fn users(&self) -> Arc<Vec<Arc<UserModel>>> {
        self.users_list.clone()
    }

    pub fn get_user(&self, email: &str) -> Option<Arc<UserModel>> {
        self.users.get(&email.trim().to_lowercase()).cloned()
    }

    /// The display name for an assignee value, when there is one.
    ///
    /// `None` for `claude` and for an email with no user row — the caller then shows the raw value,
    /// which is more honest than inventing a name for it.
    pub fn display_name_of(&self, assignee: &str) -> Option<String> {
        let user = self.get_user(assignee)?;

        if user.name.trim().is_empty() {
            None
        } else {
            Some(user.name.clone())
        }
    }

    pub fn get_task(&self, project_id: &str, number: i64) -> Option<Arc<TaskModel>> {
        self.tasks.get(project_id)?.get(&number).cloned()
    }

    /// A project's tasks, oldest first. Not capped: a board is a hand-written list.
    pub fn tasks_of_project(&self, project_id: &str) -> Vec<Arc<TaskModel>> {
        let Some(of_project) = self.tasks.get(project_id) else {
            return Vec::new();
        };

        let mut tasks: Vec<Arc<TaskModel>> = of_project.values().cloned().collect();
        tasks.sort_by_key(|itm| itm.number);
        tasks
    }

    pub fn tasks_amount(&self, project_id: &str) -> usize {
        self.tasks.get(project_id).map(|itm| itm.len()).unwrap_or(0)
    }

    /// The projects this person may see. An admin sees every one without being a member of any.
    pub fn projects_visible_to(&self, email: &str, is_admin: bool) -> Vec<Arc<ProjectModel>> {
        if is_admin {
            return self.projects_list.as_ref().clone();
        }

        let email = email.trim().to_lowercase();

        self.projects_list
            .iter()
            .filter(|project| project.is_member(&email))
            .cloned()
            .collect()
    }

    /// A project's label vocabulary: the distinct labels its tasks currently carry, sorted.
    ///
    /// Not stored anywhere — a label exists exactly as long as a task wears it. That is the whole
    /// reason there is no labels table.
    pub fn labels_of_project(&self, project_id: &str) -> Vec<String> {
        let Some(of_project) = self.tasks.get(project_id) else {
            return Vec::new();
        };

        let unique: AHashSet<&String> = of_project
            .values()
            .flat_map(|task| task.labels.iter())
            .collect();

        let mut labels: Vec<String> = unique.into_iter().cloned().collect();
        labels.sort();
        labels
    }

    // ------------------------------------------------------------------------------- derivations

    /// Whether a task is blocked: any id in `depends_on` naming a task that is not Done.
    ///
    /// A number matching no task counts as **still blocking**. That is deliberate — a mistyped or
    /// deleted blocker keeps the task blocked rather than silently freeing it, which is the failure
    /// mode nobody would notice.
    pub fn is_blocked(&self, task: &TaskModel) -> bool {
        if task.depends_on.is_empty() {
            return false;
        }

        let Some(project) = self.projects.get(&task.project_id) else {
            return true;
        };

        task.depends_on.iter().any(|blocker| {
            match self.get_task(&task.project_id, *blocker) {
                None => true, // unknown blocker blocks
                Some(blocker) => {
                    project.effective_status(&blocker.status)
                        != task_manager_shared::projects::COLUMN_ID_DONE
                }
            }
        })
    }

    /// The reverse edge: numbers of the tasks naming this one in their `depends_on`.
    ///
    /// Nothing stores this. A task says what is in its way; only the rest of the board knows who is
    /// waiting on it, which is the half you need before parking or re-scoping something.
    pub fn blocks(&self, project_id: &str, number: i64) -> Vec<i64> {
        let Some(of_project) = self.tasks.get(project_id) else {
            return Vec::new();
        };

        let mut dependents: Vec<i64> = of_project
            .values()
            .filter(|task| task.depends_on.contains(&number))
            .map(|task| task.number)
            .collect();

        dependents.sort_unstable();
        dependents
    }
}

impl Default for BoardInner {
    fn default() -> Self {
        Self::new()
    }
}
