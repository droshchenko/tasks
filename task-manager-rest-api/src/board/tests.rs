use std::collections::BTreeSet;
#[path = "workflow_tests.rs"]
mod workflow_tests;
use std::sync::Arc;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::priority::Priority;
use task_manager_shared::projects::{COLUMN_ID_DONE, COLUMN_ID_TODO};

use super::{
    ARCHIVE_AFTER, Board, BoardInner, ColumnModel, ColumnTemplateModel, GoalModel, KindModel,
    KindTemplateModel, ProjectModel, TaskModel, UserModel,
};

#[test]
fn finishing_the_last_task_completes_the_goal_and_reopening_it_restores_activity() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    let goal = goal("p", 1);
    board.upsert_goal(goal.clone());
    assert_eq!(board.read().goal_state(&goal).0, "todo");
    let mut first = task("p", 2, COLUMN_ID_DONE, &[]);
    first.goal_number = Some(1);
    first.close_moment = Some(DateTimeAsMicroseconds::new(10));
    board.upsert_task(first);
    let mut last = task("p", 3, "in-progress", &[]);
    last.goal_number = Some(1);
    board.upsert_task(last.clone());
    assert_eq!(board.read().goal_state(&goal).0, "in-progress");
    last.status = COLUMN_ID_DONE.into();
    last.close_moment = Some(DateTimeAsMicroseconds::new(20));
    board.upsert_task(last.clone());
    assert_eq!(
        board.read().goal_state(&goal),
        ("done", Some(DateTimeAsMicroseconds::new(20)))
    );
    last.status = "in-progress".into();
    last.close_moment = None;
    board.upsert_task(last);
    assert_eq!(board.read().goal_state(&goal), ("in-progress", None));
    assert!(!board.read().is_goal_archived(&goal));
}

#[test]
fn goal_completion_includes_archived_work_and_excludes_deleted_work() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    let goal = goal("p", 1);
    board.upsert_goal(goal.clone());
    let mut completed = task("p", 2, COLUMN_ID_DONE, &[]);
    completed.goal_number = Some(1);
    completed.close_moment = Some(DateTimeAsMicroseconds::new(1));
    board.upsert_task(completed);
    let mut deleted = task("p", 3, COLUMN_ID_TODO, &[]);
    deleted.goal_number = Some(1);
    deleted.deleted_moment = Some(DateTimeAsMicroseconds::new(2));
    board.upsert_task(deleted);
    assert_eq!(board.read().goal_state(&goal).0, "done");
    assert_eq!(board.read().goal_progress("p", 1), (1, 1));
    let mut added = task("p", 4, COLUMN_ID_TODO, &[]);
    added.goal_number = Some(1);
    board.upsert_task(added);
    assert_eq!(board.read().goal_state(&goal).0, "in-progress");
}

#[test]
fn deleting_the_last_unfinished_task_closes_the_goal_today_instead_of_archiving_it() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    let goal = goal("p", 1);
    board.upsert_goal(goal.clone());
    let now = DateTimeAsMicroseconds::now();
    let mut finished = task("p", 2, COLUMN_ID_DONE, &[]);
    finished.goal_number = Some(1);
    finished.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - 30 * 86400 * 1_000_000,
    ));
    board.upsert_task(finished);
    let mut unfinished = task("p", 3, "in-progress", &[]);
    unfinished.goal_number = Some(1);
    board.upsert_task(unfinished.clone());
    unfinished.deleted_moment = Some(now);
    let changes = crate::scripts::goal_transitions(&board.read(), &unfinished, now);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].close_moment, Some(now));
    assert!(changes[0].auto_completed);
    board.upsert_goals_and_tasks(changes, vec![unfinished]);
    let current = board.read().get_goal("p", 1).unwrap();
    assert_eq!(board.read().goal_state(&current).0, "done");
    assert!(!board.read().is_goal_archived(&current));
}

#[test]
fn detaching_the_last_unfinished_task_updates_both_goal_clocks() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    board.upsert_goal(goal("p", 1));
    board.upsert_goal(goal("p", 4));
    let mut finished = task("p", 2, COLUMN_ID_DONE, &[]);
    finished.goal_number = Some(1);
    finished.close_moment = Some(DateTimeAsMicroseconds::new(1));
    board.upsert_task(finished);
    let mut moving = task("p", 3, "in-progress", &[]);
    moving.goal_number = Some(1);
    board.upsert_task(moving.clone());
    let now = DateTimeAsMicroseconds::now();
    moving.goal_number = Some(4);
    let changes = crate::scripts::goal_transitions(&board.read(), &moving, now);
    assert_eq!(
        changes
            .iter()
            .find(|goal| goal.number == 1)
            .unwrap()
            .close_moment,
        Some(now)
    );
    board.upsert_goals_and_tasks(changes, vec![moving]);
    assert_eq!(
        board
            .read()
            .goal_state(&board.read().get_goal("p", 1).unwrap())
            .0,
        "done"
    );
    assert_eq!(
        board
            .read()
            .goal_state(&board.read().get_goal("p", 4).unwrap())
            .0,
        "in-progress"
    );
}

#[test]
fn preparation_selects_only_current_fragments_and_analysis_is_a_separate_required_output() {
    use task_manager_shared::execution_prompts::ExecutionPrompt;
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    let mut columns = template();
    columns.prompts = vec![
        ExecutionPrompt {
            target: "in-progress".into(),
            text: "Implement the agreed change".into(),
            requires_analysis_documents: true,
        },
        ExecutionPrompt {
            target: "done".into(),
            text: "Report verification".into(),
            requires_analysis_documents: true,
        },
    ];
    let mut kinds = kind_template();
    kinds.prompts = vec![ExecutionPrompt {
        target: "bug".into(),
        text: "Reproduce before fixing".into(),
        requires_analysis_documents: false,
    }];
    board.upsert_column_template(columns);
    board.upsert_kind_template(kinds);
    let mut before = task("p", 2, "in-progress", &[]);
    before.kind = Some("bug".into());
    let state = board.read();
    let project = state.get_project("p").unwrap();
    let selected =
        crate::scripts::resolve_execution_prompts(&state, &project, &before, None).unwrap();
    assert_eq!(selected.len(), 2);
    assert_eq!(selected[0].target, "in-progress");
    assert_eq!(selected[1].target, "bug");
    assert!(!selected.iter().any(|prompt| prompt.target == "done"));
    let mut after = before.clone();
    after.status = "done".into();
    after.documents.push("input specification".into());
    assert!(
        crate::scripts::validate_analysis_transition(&state, &project, &before, &after).is_err()
    );
    after
        .analysis_documents
        .push("raw/TM/github/source/analysis #1.md".into());
    assert!(
        crate::scripts::validate_analysis_transition(&state, &project, &before, &after).is_ok()
    );
}

#[test]
fn required_question_retries_after_completion_and_normalized_answer_history_are_consistent() {
    use crate::scripts::DecisionPatch;
    use task_manager_shared::decisions::{DecisionAnswer, DecisionChoice, DecisionRequest};
    let mut task = task("p", 2, "in-progress", &[]);
    let request = DecisionRequest {
        request_key: "deploy".into(),
        action: "Deploy release 2".into(),
        question: "Proceed?".into(),
        options: vec![DecisionChoice {
            id: "prepare".into(),
            label: "Prepare only".into(),
            consequence: "Keep current deployment".into(),
            recommended: true,
        }],
        required: true,
        asked_by: "AI".into(),
    };
    let now = DateTimeAsMicroseconds::now();
    DecisionPatch::Request(request.clone())
        .apply(&mut task, now)
        .unwrap();
    let id = task.decisions[0].id.clone();
    DecisionPatch::Answer {
        id: id.clone(),
        answer: DecisionAnswer {
            option_id: Some(" prepare ".into()),
            text: "  agreed  ".into(),
            answered_by: "owner@example.org".into(),
            answered_unix_seconds: 0,
            source: "agent_reported".into(),
        },
    }
    .apply(&mut task, now)
    .unwrap();
    assert!(
        task.comments
            .last()
            .unwrap()
            .text
            .contains("**Choice:** Prepare only")
    );
    assert!(
        !task
            .comments
            .last()
            .unwrap()
            .text
            .contains("Free-text answer")
    );
    task.status = "done".into();
    let comments = task.comments.len();
    DecisionPatch::Request(request)
        .apply(&mut task, now)
        .unwrap();
    assert_eq!(task.decisions.len(), 1);
    assert_eq!(task.decisions[0].id, id);
    assert_eq!(task.comments.len(), comments);
}

const TEMPLATE_ID: &str = "tpl";
const KIND_TEMPLATE_ID: &str = "kinds-tpl";

#[tokio::test]
#[ignore = "requires the isolated TASKS_TEST_DATABASE_URL database on port 65433"]
async fn postgres_workflow_is_atomic_and_nullable_migration_preserves_legacy_rows() {
    use service_sdk::my_postgres::{MyPostgres, PostgresSettings, tokio_postgres};
    struct Settings(String);
    #[async_trait::async_trait]
    impl PostgresSettings for Settings {
        async fn get_connection_string(&self) -> String {
            self.0.clone()
        }
    }
    let connection_string =
        std::env::var("TASKS_TEST_DATABASE_URL").expect("isolated test database required");
    assert_eq!(
        connection_string,
        "host=127.0.0.1 port=65433 user=tasks_dev password=tasks_test_only dbname=tasks_workflow_test sslmode=disable"
    );
    let (client, connection) = tokio_postgres::connect(&connection_string, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_task = tokio::spawn(async move {
        connection.await.unwrap();
    });
    client.batch_execute(r#"
        DROP TABLE IF EXISTS tasks, goals, column_templates, kind_templates, semantic_vectors, app_settings;
        CREATE TABLE tasks (
            project_id text NOT NULL, number bigint NOT NULL, task_text text NOT NULL, status text NOT NULL,
            priority text, kind text, goal_number bigint, assignee text, labels jsonb NOT NULL, depends_on jsonb NOT NULL,
            comments jsonb NOT NULL, subtasks jsonb, documents jsonb, gh_actions jsonb,
            created timestamp NOT NULL, updated timestamp NOT NULL, close_moment timestamp, deleted_moment timestamp,
            CONSTRAINT tasks_pk PRIMARY KEY(project_id, number)
        );
        CREATE TABLE goals (
            id text NOT NULL, project_id text NOT NULL, number bigint NOT NULL, name text NOT NULL, description text NOT NULL,
            color text, priority text, comments jsonb NOT NULL, subtasks jsonb, documents jsonb,
            created timestamp NOT NULL, updated timestamp NOT NULL, close_moment timestamp, deleted_moment timestamp,
            CONSTRAINT goals_pk PRIMARY KEY(project_id, number)
        );
        CREATE TABLE column_templates (id text PRIMARY KEY, name text NOT NULL, description text NOT NULL, columns jsonb NOT NULL, created timestamp NOT NULL);
        CREATE TABLE kind_templates (id text PRIMARY KEY, name text NOT NULL, description text NOT NULL, kinds jsonb NOT NULL, created timestamp NOT NULL);
        INSERT INTO tasks(project_id,number,task_text,status,labels,depends_on,comments,created,updated)
            VALUES('legacy',1,'Keep this row','todo','[]','[]','[]',TIMESTAMP '2026-01-01',TIMESTAMP '2026-01-01');
        INSERT INTO goals(id,project_id,number,name,description,comments,created,updated)
            VALUES('legacy:2','legacy',2,'Keep this goal','Legacy goal','[]',TIMESTAMP '2026-01-01',TIMESTAMP '2026-01-01');
        INSERT INTO column_templates VALUES('legacy','Old columns','Keep this template','[]',TIMESTAMP '2026-01-01');
        INSERT INTO kind_templates VALUES('legacy','Old kinds','Keep this template','[]',TIMESTAMP '2026-01-01');
    "#).await.unwrap();
    let _schema =
        MyPostgres::from_settings("tasks-workflow-test", Arc::new(Settings(connection_string)))
            .with_table_schema_verification::<crate::postgres::TaskDto>(
                "tasks",
                Some("tasks_pk".into()),
            )
            .with_table_schema_verification::<crate::postgres::GoalDto>(
                "goals",
                Some("goals_pk".into()),
            )
            .with_table_schema_verification::<crate::postgres::ColumnTemplateDto>(
                "column_templates",
                Some("column_templates_pkey".into()),
            )
            .with_table_schema_verification::<crate::postgres::KindTemplateDto>(
                "kind_templates",
                Some("kind_templates_pkey".into()),
            )
            .with_table_schema_verification::<crate::postgres::SemanticVectorDto>(
                "semantic_vectors",
                Some("semantic_vectors_pk".into()),
            )
            .with_table_schema_verification::<crate::postgres::AppSettingDto>(
                "app_settings",
                Some("app_settings_pk".into()),
            )
            .build()
            .await;
    let legacy = client.query_one("SELECT task_text, decisions IS NULL, analysis_documents IS NULL, ai_reviews IS NULL FROM tasks WHERE project_id='legacy'", &[]).await.unwrap();
    assert_eq!(legacy.get::<_, String>(0), "Keep this row");
    assert!(legacy.get::<_, bool>(1) && legacy.get::<_, bool>(2));
    assert!(legacy.get::<_, bool>(3));
    let configuration =
        crate::intelligence::settings::Configuration::new("synthetic-settings-master", vec![]);
    let provider_input = task_manager_shared::ai_settings::SaveProviderInput {
        provider: "embeddings".into(),
        enabled: true,
        endpoint: "http://127.0.0.1:54321/embeddings".into(),
        model: "fixture-v1".into(),
        api_key: Some("synthetic-settings-key".into()),
        clear_key: false,
        revision: 0,
    };
    let setting = configuration
        .prepare_update(&provider_input, "owner@example.test")
        .unwrap();
    let statement = service_sdk::my_postgres::sql::build_insert_or_update_sql(
        &setting,
        "app_settings",
        &service_sdk::my_postgres::UpdateConflictType::OnPrimaryKeyConstraint(
            "app_settings_pk".into(),
        ),
    );
    client
        .execute(&statement.sql, &statement.values.get_values_to_invoke())
        .await
        .unwrap();
    let stored_setting = client
        .query_one(
            "SELECT value, revision, updated_by FROM app_settings WHERE id='provider:embeddings'",
            &[],
        )
        .await
        .unwrap();
    let stored_value: String = stored_setting.get(0);
    assert!(!stored_value.contains("synthetic-settings-key"));
    let restored = crate::intelligence::settings::Configuration::new(
        "synthetic-settings-master",
        vec![crate::postgres::AppSettingDto {
            id: "provider:embeddings".into(),
            value: stored_value,
            revision: stored_setting.get(1),
            updated_by: stored_setting.get(2),
        }],
    );
    assert_eq!(
        restored.embeddings().unwrap().unwrap().key.as_deref(),
        Some("synthetic-settings-key")
    );
    let vector = crate::postgres::SemanticVectorDto {
        project_id: "p".into(),
        task_number: 2,
        provider_key: "local-fixture".into(),
        content_hash: "hash".into(),
        model: "embedding-test-v1".into(),
        dimensions: 2,
        embedding: vec![0.6, 0.8],
    };
    let vector_sql = service_sdk::my_postgres::sql::build_insert_or_update_sql(
        &vector,
        "semantic_vectors",
        &service_sdk::my_postgres::UpdateConflictType::OnPrimaryKeyConstraint(
            "semantic_vectors_pk".into(),
        ),
    );
    client
        .execute(&vector_sql.sql, &vector_sql.values.get_values_to_invoke())
        .await
        .unwrap();
    let stored_vector: String = client
        .query_one(
            "SELECT embedding::text FROM semantic_vectors WHERE project_id='p'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        serde_json::from_str::<Vec<f32>>(&stored_vector).unwrap(),
        vector.embedding
    );
    assert!(
        client
            .query_one(
                "SELECT auto_completed IS NULL FROM goals WHERE project_id='legacy'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    assert!(
        client
            .query_one(
                "SELECT prompts IS NULL FROM column_templates WHERE id='legacy'",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    let mut changed_task = task("p", 2, COLUMN_ID_DONE, &[]);
    changed_task.text = "Use $1 literally; don't execute it".into();
    changed_task.goal_number = Some(1);
    changed_task
        .ai_reviews
        .push(crate::intelligence::jev::fixture_review());
    changed_task.close_moment = Some(DateTimeAsMicroseconds::now());
    changed_task.analysis_documents = vec!["raw/TM/github/source/analysis #1.md".into()];
    use task_manager_shared::decisions::{DecisionAnswer, DecisionRequest};
    task_manager_shared::decisions::add_decision(
        &mut changed_task.decisions,
        DecisionRequest {
            request_key: "choose".into(),
            action: "Prepare the change".into(),
            question: "Which action?".into(),
            options: Vec::new(),
            required: true,
            asked_by: "AI".into(),
        },
        "decision1".into(),
        1,
    )
    .unwrap();
    task_manager_shared::decisions::answer_decision(
        &mut changed_task.decisions,
        "decision1",
        DecisionAnswer {
            option_id: None,
            text: "Prepare only; preserve $1".into(),
            answered_by: "owner@example.org".into(),
            answered_unix_seconds: 2,
            source: "ui".into(),
        },
    )
    .unwrap();
    let mut changed_goal = goal("p", 1);
    changed_goal.auto_completed = true;
    changed_goal.close_moment = changed_task.close_moment;
    let task_row: crate::postgres::TaskDto = (&changed_task).into();
    let goal_row: crate::postgres::GoalDto = (&changed_goal).into();
    let statement = crate::postgres::task_and_goals_statement(&task_row, &[goal_row]);
    client
        .execute(&statement.sql, &statement.values.get_values_to_invoke())
        .await
        .unwrap();
    let stored = client
        .query_one(
            "SELECT task_text, analysis_documents::text FROM tasks WHERE project_id='p'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(stored.get::<_, String>(0), changed_task.text);
    assert!(stored.get::<_, String>(1).contains("analysis #1.md"));
    let review_json: String = client
        .query_one(
            "SELECT ai_reviews::text FROM tasks WHERE project_id='p'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        serde_json::from_str::<Vec<task_manager_shared::ai_reviews::AiReview>>(&review_json)
            .unwrap(),
        changed_task.ai_reviews
    );
    let decision_json: String = client
        .query_one(
            "SELECT decisions::text FROM tasks WHERE project_id='p'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        serde_json::from_str::<Vec<task_manager_shared::decisions::TaskDecision>>(&decision_json)
            .unwrap(),
        changed_task.decisions
    );
    assert!(
        client
            .query_one("SELECT auto_completed FROM goals WHERE project_id='p'", &[])
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    // A failed goal write must roll back the task write in the same statement.
    client
        .batch_execute(
            "ALTER TABLE goals ADD CONSTRAINT no_bad_goal CHECK (name <> 'fail_this_goal')",
        )
        .await
        .unwrap();
    changed_task.text = "This update must not survive".into();
    changed_goal.name = "fail_this_goal".into();
    let statement = crate::postgres::task_and_goals_statement(
        &(&changed_task).into(),
        &[(&changed_goal).into()],
    );
    assert!(
        client
            .execute(&statement.sql, &statement.values.get_values_to_invoke())
            .await
            .is_err()
    );
    assert_eq!(
        client
            .query_one("SELECT task_text FROM tasks WHERE project_id='p'", &[])
            .await
            .unwrap()
            .get::<_, String>(0),
        "Use $1 literally; don't execute it"
    );
    drop(client);
    connection_task.abort();
}

fn kind_template() -> KindTemplateModel {
    KindTemplateModel {
        prompts: Vec::new(),

        id: KIND_TEMPLATE_ID.to_string(),
        name: "Default".to_string(),
        description: String::new(),
        kinds: vec![KindModel {
            id: "bug".to_string(),
            name: "Bug".to_string(),
            description: String::new(),
            color: KindColor::Red,
            icon: "bug".to_string(),
        }],
        created: DateTimeAsMicroseconds::new(0),
    }
}

fn template() -> ColumnTemplateModel {
    ColumnTemplateModel {
        prompts: Vec::new(),

        id: TEMPLATE_ID.to_string(),
        name: "Default".to_string(),
        description: String::new(),
        columns: vec![ColumnModel {
            id: "in-progress".to_string(),
            name: "In progress".to_string(),
            description: String::new(),
            order: 10,
        }],
        created: DateTimeAsMicroseconds::new(0),
    }
}

/// A board with the fixture template already installed.
///
/// Every test that puts a project in needs it: a project's columns come from its template, so a board
/// without the template gives every project a bare Todo -> Done board.
fn board() -> Board {
    let board = Board::new();
    board.upsert_column_template(template());
    board.upsert_kind_template(kind_template());
    board
}

fn project(id: &str, prefix: &str, history: &[&str]) -> ProjectModel {
    ProjectModel {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        prefix: prefix.to_string(),
        prefix_history: history.iter().map(|itm| itm.to_string()).collect(),
        column_template_id: Some(TEMPLATE_ID.to_string()),
        kind_template_id: Some(KIND_TEMPLATE_ID.to_string()),
        // Both left empty: the board resolves them from the templates. Setting them here would be a lie
        // the very first rebuild overwrites.
        columns: Vec::new(),
        kinds: Vec::new(),
        members: BTreeSet::new(),
        last_task_number: 0,
        // No window of its own, so the tests measure the seven-day default.
        archive_days: None,
        // Live. The archiving tests below put one away by hand — everything else wants a live project.
        archived_moment: None,
        github_connections: Vec::new(),
        created: DateTimeAsMicroseconds::new(0),
    }
}

fn task(project_id: &str, number: i64, status: &str, depends_on: &[i64]) -> TaskModel {
    TaskModel {
        decisions: Vec::new(),
        ai_reviews: Vec::new(),
        analysis_documents: Vec::new(),

        project_id: project_id.to_string(),
        number,
        text: format!("task {number}"),
        status: status.to_string(),
        // Everything the ordering tests do not care about is Normal, so a list that comes back in number
        // order is the tiebreaker working rather than the priority sort being absent.
        priority: Priority::default(),
        kind: None,
        goal_number: None,
        assignee: None,
        labels: Vec::new(),
        depends_on: depends_on.to_vec(),
        // Nothing here reads a checklist — that is the point of it — so every fixture leaves it empty. Nor a
        // document reference: nothing on the board derives anything from one either, and the documents
        // themselves are not in memory at all.
        subtasks: Vec::new(),
        documents: Vec::new(),
        // Nor a build link, for the third time and the same reason: nothing on the board derives anything
        // from one — it is a record hung on the task, not a fact the board reads.
        gh_actions: Vec::new(),
        comments: Vec::new(),
        created: DateTimeAsMicroseconds::new(0),
        updated: DateTimeAsMicroseconds::new(0),
        close_moment: None,
        deleted_moment: None,
    }
}

fn goal(project_id: &str, number: i64) -> GoalModel {
    GoalModel {
        auto_completed: false,

        project_id: project_id.to_string(),
        number,
        name: format!("goal {number}"),
        description: String::new(),
        color: KindColor::default(),
        priority: Priority::default(),
        subtasks: Vec::new(),
        documents: Vec::new(),
        comments: Vec::new(),
        created: DateTimeAsMicroseconds::new(0),
        updated: DateTimeAsMicroseconds::new(0),
        close_moment: None,
        deleted_moment: None,
    }
}

/// The same task, marked deleted. A fixture rather than a field on `task()` because most tests want work that
/// exists, and the ones that do not should say so at the call site.
fn deleted(mut task: TaskModel) -> TaskModel {
    task.deleted_moment = Some(DateTimeAsMicroseconds::new(1));
    task
}

fn user(email: &str, name: &str) -> UserModel {
    UserModel {
        email: email.to_string(),
        name: name.to_string(),
        admin: false,
        disabled: false,
        created: DateTimeAsMicroseconds::new(0),
    }
}

/// The whole point of `blocked`: derived on every read, and true while any dependency is not Done.
#[test]
fn blocked_is_derived_against_the_current_statuses() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, COLUMN_ID_TODO, &[]));
    board.upsert_task(task("p", 2, COLUMN_ID_DONE, &[]));

    let read = board.read();

    // Depends on an open task -> blocked.
    assert!(read.is_blocked(&task("p", 10, COLUMN_ID_TODO, &[1])));
    // Depends only on a Done task -> not blocked.
    assert!(!read.is_blocked(&task("p", 11, COLUMN_ID_TODO, &[2])));
    // One open blocker among Done ones is still enough.
    assert!(read.is_blocked(&task("p", 12, COLUMN_ID_TODO, &[2, 1])));
    // No dependencies -> never blocked.
    assert!(!read.is_blocked(&task("p", 13, COLUMN_ID_TODO, &[])));
}

/// A blocker that does not exist must keep the task blocked. The alternative — treating "not found"
/// as satisfied — frees a task because of a typo, silently.
#[test]
fn an_unknown_blocker_still_blocks() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    assert!(
        board
            .read()
            .is_blocked(&task("p", 1, COLUMN_ID_TODO, &[999]))
    );
}

/// Closing the last blocker unblocks the dependent with nothing written by hand — that is what
/// "derived, never stored" buys.
#[test]
fn closing_the_last_blocker_unblocks_on_the_next_read() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, COLUMN_ID_TODO, &[]));
    board.upsert_task(task("p", 2, COLUMN_ID_TODO, &[1]));

    let dependent = board.read().get_task("p", 2).unwrap();
    assert!(board.read().is_blocked(&dependent));

    board.upsert_task(task("p", 1, COLUMN_ID_DONE, &[]));

    assert!(!board.read().is_blocked(&dependent));
}

/// A task in a deleted column reads as Todo — so it also stops counting as Done, and anything
/// depending on it goes back to blocked. Worth pinning: it is the interaction between two lenient
/// rules, and getting it wrong would mark work unblocked because a column was tidied away.
#[test]
fn a_blocker_whose_column_was_deleted_blocks_again() {
    let board = board();
    // `archived` is not among the project's columns, so a task sitting there reads as Todo.
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, "archived", &[]));

    assert!(board.read().is_blocked(&task("p", 2, COLUMN_ID_TODO, &[1])));
}

#[test]
fn blocks_is_the_reverse_edge_and_is_sorted() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, COLUMN_ID_TODO, &[]));
    board.upsert_task(task("p", 3, COLUMN_ID_TODO, &[1]));
    board.upsert_task(task("p", 2, COLUMN_ID_TODO, &[1]));
    board.upsert_task(task("p", 4, COLUMN_ID_TODO, &[]));

    let read = board.read();

    assert_eq!(read.blocks("p", 1), vec![2, 3]);
    assert!(read.blocks("p", 4).is_empty());
    // The two directions are independent: 2 is blocked by 1 and blocks nothing.
    assert!(read.blocks("p", 2).is_empty());
}

/// A stored status naming a column the project no longer has reads as Todo, and the stored value is
/// left alone — so re-creating the column brings the task back to it.
#[test]
fn an_unknown_status_reads_as_todo_without_being_rewritten() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, "gone", &[]));

    let read = board.read();
    let project = read.get_project("p").unwrap();
    let stored = read.get_task("p", 1).unwrap();

    assert_eq!(project.effective_status(&stored.status), COLUMN_ID_TODO);
    assert_eq!(stored.status, "gone", "the stored value must survive");
    assert_eq!(project.effective_status("in-progress"), "in-progress");
    assert_eq!(project.effective_status(COLUMN_ID_DONE), COLUMN_ID_DONE);
}

#[test]
fn an_unknown_kind_reads_as_no_kind() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let project = board.read().get_project("p").unwrap();

    assert_eq!(project.effective_kind(Some("bug")), Some("bug".to_string()));
    assert_eq!(project.effective_kind(Some("gone")), None);
    assert_eq!(project.effective_kind(None), None);
}

/// The current holder is found by prefix; the history index reports everyone who ever held it. This
/// is what `tasks_resolve_id` reads.
#[test]
fn a_prefix_resolves_to_its_current_holder_and_remembers_the_past_ones() {
    let board = board();
    board.upsert_project(project("a", "TM", &["RMS"]));
    board.upsert_project(project("b", "RMS", &[]));

    let read = board.read();

    assert_eq!(read.get_project_by_prefix("RMS").unwrap().id, "b");
    assert_eq!(read.get_project_by_prefix("TM").unwrap().id, "a");
    assert!(read.get_project_by_prefix("NOPE").is_none());

    let ever: Vec<String> = read
        .projects_ever_holding_prefix("RMS")
        .iter()
        .map(|itm| itm.id.clone())
        .collect();
    assert_eq!(ever, vec!["a".to_string(), "b".to_string()]);
}

/// Free means "nobody holds it *now*". A prefix only in some project's history is takeable — which
/// is precisely why a task's handle is composed on read rather than stored.
#[test]
fn a_prefix_only_in_history_is_free_to_take() {
    let board = board();
    board.upsert_project(project("a", "TM", &["RMS"]));

    let read = board.read();

    assert!(read.is_prefix_free("RMS", None));
    assert!(!read.is_prefix_free("TM", None));
    // A project renaming itself does not collide with its own current prefix.
    assert!(read.is_prefix_free("TM", Some("a")));
}

/// Archiving a project takes it out of the pickers and out of NOTHING the board does.
///
/// Every assertion here is a link that must keep working, written down so a later filter added in the
/// wrong place fails a test instead of silently 404-ing somebody's bookmark. The board deliberately has no
/// opinion about archiving at all: it is the readers — the three dropdowns and the MCP listing — that
/// leave an archived project out, and each of them does it at the point of display.
#[test]
fn an_archived_project_is_hidden_by_its_readers_and_by_nothing_on_the_board() {
    let board = board();

    let mut put_away = project("a", "TM", &["OLD"]);
    put_away.archived_moment = Some(DateTimeAsMicroseconds::new(0));
    put_away.members.insert("yuri@mxtm.ai".to_string());
    board.upsert_project(put_away);

    let read = board.read();

    let found = read.get_project_by_prefix("TM").expect("still resolves");
    assert!(found.is_archived());
    // The direct link by internal id, which is what every per-request auth gate goes through.
    assert!(read.get_project("a").is_some());
    // And the old handle: archiving must not cost `OLD-42` its answer.
    assert_eq!(read.projects_ever_holding_prefix("OLD").len(), 1);

    // THE PREFIX IS STILL HELD. If archiving freed it, a new project could take `TM`, and every link into
    // the archived board would quietly start landing on somebody else's.
    assert!(!read.is_prefix_free("TM", None));

    // Membership is unchanged, so the list endpoint keeps returning it — carrying `archived: true` for the
    // readers to act on. Hiding it here would take it off the setup screen too, which is the one place it
    // can be brought back from.
    assert_eq!(read.projects_visible_to("yuri@mxtm.ai", false).len(), 1);
    assert_eq!(read.projects_visible_to("nobody@mxtm.ai", true).len(), 1);

    // It still follows its template, so the template still cannot be deleted out from under it — a board a
    // link still opens must not lose its columns.
    assert_eq!(read.count_projects_using_template(TEMPLATE_ID), 1);
}

/// A live project answers `is_archived` with false whatever else is true of it — the pairing test for the
/// one above, so neither direction can rot on its own.
#[test]
fn a_project_that_was_never_put_away_is_not_archived() {
    let board = board();
    board.upsert_project(project("a", "TM", &[]));

    assert!(!board.read().get_project("a").unwrap().is_archived());
}

#[test]
fn the_counter_only_moves_forward_and_is_per_project() {
    let board = board();
    board.upsert_project(project("a", "AAA", &[]));
    board.upsert_project(project("b", "BBB", &[]));

    assert_eq!(board.reserve_task_number("a"), Some(1));
    assert_eq!(board.reserve_task_number("a"), Some(2));
    assert_eq!(board.reserve_task_number("b"), Some(1));

    // Deleting is a flag now, so the row does not even leave — which makes the promise easier to keep, not
    // harder. The counter is what guarantees it either way: it only ever moves forward.
    board.upsert_task(deleted(task("a", 2, COLUMN_ID_TODO, &[])));

    assert_eq!(
        board.reserve_task_number("a"),
        Some(3),
        "a deleted number must not be handed out again"
    );

    assert_eq!(board.reserve_task_number("missing"), None);
}

#[test]
fn a_projects_labels_are_the_distinct_labels_its_tasks_carry() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let mut first = task("p", 1, COLUMN_ID_TODO, &[]);
    first.labels = vec!["ui".to_string(), "mt4".to_string()];
    board.upsert_task(first);

    let mut second = task("p", 2, COLUMN_ID_TODO, &[]);
    second.labels = vec!["mt4".to_string()];
    board.upsert_task(second);

    assert_eq!(
        board.read().labels_of_project("p"),
        vec!["mt4".to_string(), "ui".to_string()]
    );

    // Dropping it from the last task carrying it removes the label from the vocabulary — there is no labels
    // table for it to linger in. A DELETED task wears nothing, which is what this checks: the rows are still
    // there and the labels are gone with them.
    let mut first = task("p", 1, COLUMN_ID_TODO, &[]);
    first.labels = vec!["ui".to_string(), "mt4".to_string()];
    board.upsert_task(deleted(first));

    let mut second = task("p", 2, COLUMN_ID_TODO, &[]);
    second.labels = vec!["mt4".to_string()];
    board.upsert_task(deleted(second));

    assert!(board.read().labels_of_project("p").is_empty());
}

#[test]
fn visibility_follows_membership_and_an_admin_sees_everything() {
    let board = board();

    let mut with_member = project("a", "AAA", &[]);
    with_member.members.insert("yuri@mxtm.ai".to_string());
    board.upsert_project(with_member);
    board.upsert_project(project("b", "BBB", &[]));

    let read = board.read();

    let visible: Vec<String> = read
        .projects_visible_to("yuri@mxtm.ai", false)
        .iter()
        .map(|itm| itm.id.clone())
        .collect();
    assert_eq!(visible, vec!["a".to_string()]);

    assert!(read.projects_visible_to("nobody@mxtm.ai", false).is_empty());
    assert_eq!(read.projects_visible_to("nobody@mxtm.ai", true).len(), 2);

    // Case and stray space come from hand-typed membership lists, not from a bug.
    assert_eq!(read.projects_visible_to(" YURI@MXTM.AI ", false).len(), 1);
}

/// An assignee with no user row, and `claude`, have no display name — the caller shows the raw value
/// rather than inventing one.
#[test]
fn a_display_name_resolves_only_for_a_known_user_with_a_name() {
    let board = board();
    board.upsert_user(user("yuri@mxtm.ai", "Yuri"));
    board.upsert_user(user("noname@mxtm.ai", ""));

    let read = board.read();

    assert_eq!(
        read.display_name_of("yuri@mxtm.ai"),
        Some("Yuri".to_string())
    );
    assert_eq!(read.display_name_of("noname@mxtm.ai"), None);
    // The reserved assignee resolves to itself rather than to nothing: it has no user row by design, and
    // a sticker showing whatever case an agent typed would be worse than showing "AI".
    assert_eq!(read.display_name_of("AI"), Some("AI".to_string()));
    assert_eq!(read.display_name_of("ai"), Some("AI".to_string()));
    assert_eq!(read.display_name_of("nobody@x.io"), None);
    assert_eq!(read.display_name_of("stranger@mxtm.ai"), None);
}

/// Closed work leaves the board after the window, and nothing else does. Getting this wrong either buries
/// the board under years of finished tasks or hides work that is still live.
#[test]
fn only_work_closed_longer_ago_than_the_window_is_archived() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let now = DateTimeAsMicroseconds::now();
    let window = ARCHIVE_AFTER.as_micros() as i64;

    let mut just_closed = task("p", 1, COLUMN_ID_DONE, &[]);
    just_closed.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - 60_000_000,
    ));

    let mut long_closed = task("p", 2, COLUMN_ID_DONE, &[]);
    long_closed.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - window - 60_000_000,
    ));

    // A task still being worked on is never archived, however old it is.
    let mut old_and_open = task("p", 3, COLUMN_ID_TODO, &[]);
    old_and_open.created = DateTimeAsMicroseconds::new(0);

    board.upsert_task(just_closed.clone());
    board.upsert_task(long_closed.clone());
    board.upsert_task(old_and_open.clone());

    let read = board.read();

    assert!(
        !read.is_archived(&just_closed),
        "closed a minute ago is live"
    );
    assert!(
        read.is_archived(&long_closed),
        "closed past the window is archived"
    );
    assert!(!read.is_archived(&old_and_open), "open work never archives");
}

/// A task sitting in Done with no close moment must not vanish. It should not happen — the moment is
/// written on the way in — but being lenient means a gap in the data cannot silently swallow work.
#[test]
fn done_without_a_close_moment_stays_visible() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let orphan = task("p", 1, COLUMN_ID_DONE, &[]);
    board.upsert_task(orphan.clone());

    assert!(orphan.close_moment.is_none());
    assert!(!board.read().is_archived(&orphan));
}

/// The archive window is measured against the EFFECTIVE status, so a task whose column was deleted reads
/// as Todo and therefore cannot be archived — even if it still carries an old close moment.
#[test]
fn a_task_whose_column_was_deleted_is_not_archived() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let mut orphaned_column = task("p", 1, "gone", &[]);
    orphaned_column.close_moment = Some(DateTimeAsMicroseconds::new(1));

    board.upsert_task(orphaned_column.clone());

    assert!(!board.read().is_archived(&orphaned_column));
}

/// Columns come from the template, and a project following none has a bare Todo -> Done board.
///
/// The whole point of the indirection, and the case that used to be impossible: a project with no
/// columns configured is legitimate rather than broken.
#[test]
fn a_project_following_no_template_has_no_middle_columns() {
    let board = board();

    let mut without = project("p", "RMS", &[]);
    without.column_template_id = None;
    board.upsert_project(without);

    let project = board.read().get_project("p").unwrap();

    assert!(project.columns.is_empty());
    assert!(project.has_column(COLUMN_ID_TODO));
    assert!(project.has_column(COLUMN_ID_DONE));
    assert!(!project.has_column("in-progress"));
}

/// Editing a template moves every project that follows it, in the same swap.
///
/// This is what the resolved-on-rebuild cache buys, and what would silently rot if a project kept its
/// own copy of the columns instead.
#[test]
fn editing_a_template_changes_every_project_following_it() {
    let board = board();
    board.upsert_project(project("one", "AAA", &[]));
    board.upsert_project(project("two", "BBB", &[]));

    let mut edited = template();
    edited.columns.push(ColumnModel {
        id: "review".to_string(),
        name: "Review".to_string(),
        description: String::new(),
        order: 20,
    });
    board.upsert_column_template(edited);

    let read = board.read();

    for id in ["one", "two"] {
        let project = read.get_project(id).unwrap();

        assert_eq!(
            project
                .columns
                .iter()
                .map(|itm| itm.id.as_str())
                .collect::<Vec<&str>>(),
            vec!["in-progress", "review"],
            "project {id} should follow the edited template"
        );
    }
}

/// A task parked in a column the template no longer has reads as Todo, and its stored status survives —
/// so putting the column back brings it home. Same rule a deleted column always had.
#[test]
fn dropping_a_column_from_a_template_parks_its_tasks_in_todo() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, "in-progress", &[]));

    let mut emptied = template();
    emptied.columns.clear();
    board.upsert_column_template(emptied);

    let read = board.read();
    let project = read.get_project("p").unwrap();
    let stored = read.get_task("p", 1).unwrap();

    assert_eq!(
        stored.status, "in-progress",
        "the stored value must survive"
    );
    assert_eq!(project.effective_status(&stored.status), COLUMN_ID_TODO);

    // Put it back: the task returns to the column it never actually left.
    board.upsert_column_template(template());

    let read = board.read();
    let project = read.get_project("p").unwrap();
    assert_eq!(project.effective_status("in-progress"), "in-progress");
}

/// The count that makes a template refusable to delete, and shows an edit's blast radius.
#[test]
fn a_template_knows_how_many_projects_follow_it() {
    let board = board();
    board.upsert_project(project("one", "AAA", &[]));
    board.upsert_project(project("two", "BBB", &[]));

    let mut alone = project("three", "CCC", &[]);
    alone.column_template_id = None;
    board.upsert_project(alone);

    let read = board.read();

    assert_eq!(read.count_projects_using_template(TEMPLATE_ID), 2);
    assert_eq!(
        read.count_projects_using_template("nothing-follows-this"),
        0
    );
}

/// Tasks and goals draw from ONE counter, so the startup floor has to account for both.
///
/// Without the goals half, a counter row that had fallen behind would hand a task the number a goal
/// already holds — and then `RMS-7` and `RMS-G7` would both exist, which is the single assumption the
/// handles are built on.
#[test]
fn the_counter_floor_counts_goals_as_well_as_tasks() {
    let mut behind = project("p", "RMS", &[]);
    behind.last_task_number = 1;

    let inner = BoardInner::from_loaded(
        vec![behind],
        vec![task("p", 2, COLUMN_ID_TODO, &[])],
        Vec::new(),
        vec![template()],
        vec![kind_template()],
        vec![goal("p", 9)],
    );

    assert_eq!(
        inner.get_project("p").unwrap().last_task_number,
        9,
        "the floor is the highest number in use, whichever kind of thing holds it"
    );
}

/// A goal's progress counts archived work. A goal only closes once every task is done, and by then the
/// oldest of them have aged off the board — a count that skipped those would report finished work as
/// half-finished, and the goal would read as abandoned rather than landed.
#[test]
fn goal_progress_counts_archived_tasks() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_goal(goal("p", 1));

    let now = DateTimeAsMicroseconds::now();
    let window = ARCHIVE_AFTER.as_micros() as i64;

    let mut long_closed = task("p", 2, COLUMN_ID_DONE, &[]);
    long_closed.goal_number = Some(1);
    long_closed.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - window - 60_000_000,
    ));

    let mut open = task("p", 3, COLUMN_ID_TODO, &[]);
    open.goal_number = Some(1);

    // Somebody else's work: it must not be counted at all.
    board.upsert_task(task("p", 4, COLUMN_ID_DONE, &[]));

    board.upsert_task(long_closed.clone());
    board.upsert_task(open);

    let read = board.read();

    assert!(read.is_archived(&long_closed), "the fixture is archived");
    assert_eq!(read.goal_progress("p", 1), (2, 1));
    assert_eq!(read.tasks_of_goal("p", 1).len(), 2);
}

/// The list under a goal and the counter beside it have to agree, so both include archived work — and
/// neither includes a task pointing at a different goal.
#[test]
fn open_tasks_of_a_goal_are_what_blocks_closing_it() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_goal(goal("p", 1));

    let mut done = task("p", 2, COLUMN_ID_DONE, &[]);
    done.goal_number = Some(1);

    let mut open = task("p", 3, "in-progress", &[]);
    open.goal_number = Some(1);

    board.upsert_task(done);
    board.upsert_task(open);

    let read = board.read();
    let blocking = read.open_tasks_of_goal("p", 1);

    assert_eq!(blocking.len(), 1);
    assert_eq!(blocking[0].number, 3);
}

/// A goal archives on the same clock a task does, and an OPEN goal never archives however old it is — an
/// epic that has run for a year is late, not history.
#[test]
fn a_goal_archives_by_its_close_moment_and_the_projects_window() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let now = DateTimeAsMicroseconds::now();
    let window = ARCHIVE_AFTER.as_micros() as i64;

    let mut just_closed = goal("p", 1);
    just_closed.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - 60_000_000,
    ));

    let mut long_closed = goal("p", 2);
    long_closed.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - window - 60_000_000,
    ));

    let ancient_but_open = goal("p", 3);

    let read = board.read();

    assert!(!read.is_goal_archived(&just_closed));
    assert!(read.is_goal_archived(&long_closed));
    assert!(!read.is_goal_archived(&ancient_but_open));
}

/// The window is the project's, not a constant. A project that says two days archives work the default
/// seven-day window would still be showing.
#[test]
fn a_projects_own_archive_window_is_what_counts() {
    let board = board();

    let mut impatient = project("p", "RMS", &[]);
    impatient.archive_days = Some(2);
    board.upsert_project(impatient);

    let now = DateTimeAsMicroseconds::now();
    let three_days = 3 * 24 * 60 * 60 * 1_000_000_i64;

    let mut closed_three_days_ago = task("p", 1, COLUMN_ID_DONE, &[]);
    closed_three_days_ago.close_moment = Some(DateTimeAsMicroseconds::new(
        now.unix_microseconds - three_days,
    ));

    board.upsert_task(closed_three_days_ago.clone());

    assert!(
        board.read().is_archived(&closed_three_days_ago),
        "three days is past a two-day window, even though it is inside the default seven"
    );
}

/// A window that makes no sense reads as the default. Writes validate; the read side is lenient, so one
/// bad row cannot empty a board.
#[test]
fn a_nonsense_archive_window_falls_back_to_the_default() {
    let mut zero = project("p", "RMS", &[]);
    zero.archive_days = Some(0);
    assert_eq!(zero.archive_after(), ARCHIVE_AFTER);

    let mut negative = project("p", "RMS", &[]);
    negative.archive_days = Some(-5);
    assert_eq!(negative.archive_after(), ARCHIVE_AFTER);

    let mut two = project("p", "RMS", &[]);
    two.archive_days = Some(2);
    assert_eq!(two.archive_after().as_secs(), 2 * 24 * 60 * 60);
}

/// The order IS the feature. A column is drawn straight off this list, so the most urgent has to arrive
/// first — and within one priority the board's old oldest-first order has to survive, or cards would
/// shuffle on every repaint for no reason a reader could see.
#[test]
fn tasks_come_back_most_urgent_first_and_oldest_first_within_a_priority() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    // Inserted in an order that has nothing to do with the answer: numbers ascending, priorities scrambled.
    for (number, priority) in [
        (1, Priority::Normal),
        (2, Priority::SuperLow),
        (3, Priority::SuperHigh),
        (4, Priority::Normal),
        (5, Priority::High),
        (6, Priority::Low),
    ] {
        let mut itm = task("p", number, COLUMN_ID_TODO, &[]);
        itm.priority = priority;
        board.upsert_task(itm);
    }

    let numbers: Vec<i64> = board
        .read()
        .tasks_of_project("p")
        .iter()
        .map(|itm| itm.number)
        .collect();

    assert_eq!(numbers, vec![3, 5, 1, 4, 6, 2]);
}

/// A goal is ranked the same way and read the same way. Two rules on one product — one for tasks and
/// another for goals — is the thing this test exists to prevent.
#[test]
fn goals_come_back_most_urgent_first_too() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    for (number, priority) in [
        (1, Priority::Low),
        (2, Priority::SuperHigh),
        (3, Priority::Normal),
    ] {
        let mut itm = goal("p", number);
        itm.priority = priority;
        board.upsert_goal(itm);
    }

    let numbers: Vec<i64> = board
        .read()
        .goals_of_project("p")
        .iter()
        .map(|itm| itm.number)
        .collect();

    assert_eq!(numbers, vec![2, 3, 1]);
}

/// The whole shape of a deletion, in one place: gone from everything derived, still there to be found.
///
/// Each half is a separate way to get this wrong. Leaving deleted work in a count holds a goal open on a card
/// nobody can see; taking it out of `tasks_of_project` would make it unfindable, which is the one thing
/// keeping the row was for.
#[test]
fn a_deleted_task_leaves_every_derived_answer_and_stays_findable() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_goal(goal("p", 1));

    let mut live = task("p", 2, COLUMN_ID_TODO, &[]);
    live.goal_number = Some(1);
    live.labels = vec!["kept".to_string()];
    board.upsert_task(live);

    let mut gone = task("p", 3, COLUMN_ID_TODO, &[]);
    gone.goal_number = Some(1);
    gone.labels = vec!["lost".to_string()];
    board.upsert_task(deleted(gone));

    let read = board.read();

    assert_eq!(
        read.goal_progress("p", 1),
        (1, 0),
        "counted as one, not two"
    );
    assert_eq!(read.tasks_of_goal("p", 1).len(), 1);
    assert_eq!(
        read.open_tasks_of_goal("p", 1).len(),
        1,
        "a deleted task cannot hold a goal open"
    );
    assert_eq!(read.tasks_amount("p"), 1);
    assert_eq!(read.labels_of_project("p"), vec!["kept".to_string()]);

    assert!(
        read.get_task("p", 3).is_none(),
        "hidden from the ordinary lookup"
    );
    assert!(
        read.get_task_including_deleted("p", 3).is_some(),
        "and found by the one search uses"
    );

    // In the snapshot, because the browser is where a search over text happens — the screen hides it there.
    assert_eq!(
        read.tasks_of_project("p").len(),
        2,
        "the snapshot carries deleted work so it can be searched"
    );
}

/// A deleted BLOCKER keeps its dependents blocked, which falls out of the lookup returning `None` — the same
/// place a typo lands. Deleting a blocker is not a statement that the work is done.
#[test]
fn a_deleted_blocker_still_blocks() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    board.upsert_task(deleted(task("p", 1, COLUMN_ID_DONE, &[])));

    let waiting = task("p", 2, COLUMN_ID_TODO, &[1]);
    board.upsert_task(waiting.clone());

    // And a deleted task that depends on something: it must not show up as waiting on it.
    board.upsert_task(deleted(task("p", 3, COLUMN_ID_TODO, &[4])));
    board.upsert_task(task("p", 4, COLUMN_ID_TODO, &[]));

    let read = board.read();

    assert!(
        read.is_blocked(&waiting),
        "a blocker that was deleted is not a blocker that was finished"
    );

    assert!(
        read.blocks("p", 4).is_empty(),
        "a deleted task is not somebody who is waiting on you"
    );
}

/// A deleted goal disappears, and the work that pointed at it reads as standalone rather than pointing at
/// something nobody can open. The tasks themselves survive — whether work outlives its container is a
/// decision, not a side effect.
#[test]
fn a_deleted_goal_disappears_and_its_tasks_do_not() {
    let board = board();
    board.upsert_project(project("p", "RMS", &[]));

    let mut gone = goal("p", 1);
    gone.deleted_moment = Some(DateTimeAsMicroseconds::new(1));
    board.upsert_goal(gone);

    let mut orphan = task("p", 2, COLUMN_ID_TODO, &[]);
    orphan.goal_number = Some(1);
    board.upsert_task(orphan.clone());

    let read = board.read();

    assert!(read.goals_of_project("p").is_empty());
    assert!(read.get_goal("p", 1).is_none());
    assert!(
        read.get_goal_including_deleted("p", 1).is_some(),
        "still findable by id"
    );

    assert!(
        read.effective_goal(&orphan).is_none(),
        "the task reads as standalone rather than pointing at a goal nobody can open"
    );
    assert_eq!(read.tasks_amount("p"), 1, "the task itself is untouched");
}
