use super::*;
use task_manager_shared::decisions::{DecisionRequest, add_decision};

fn question(task: &mut TaskModel, required: bool, at: i64) {
    add_decision(
        &mut task.decisions,
        DecisionRequest {
            request_key: format!("q{}", task.number),
            action: "Prepare a change".into(),
            question: "Which option should be used?".into(),
            options: Vec::new(),
            required,
            asked_by: "agent@example.test".into(),
        },
        format!("q{}", task.number),
        at,
    )
    .unwrap();
}

#[test]
fn dependency_reasons_match_mcp_and_update_when_the_blocker_is_completed() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    board.upsert_task(task("p", 1, COLUMN_ID_TODO, &[]));
    board.upsert_task(task("p", 2, COLUMN_ID_DONE, &[]));
    board.upsert_task(deleted(task("p", 4, COLUMN_ID_TODO, &[])));
    let waiting = task("p", 5, COLUMN_ID_TODO, &[1, 2, 4, 99]);
    let read = board.read();
    let project = read.get_project("p").unwrap();
    let response = crate::mappers::task_to_response(&waiting, &project, &read);
    let previous_revision = response.revision_unix_microseconds;
    assert_eq!(
        response
            .readiness
            .dependencies
            .iter()
            .map(|item| item.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["TM-1", "TM-4", "TM-99"]
    );
    assert_eq!(response.readiness.dependencies[1].status, None);
    let mcp = crate::mcp::TaskView::from_model(&waiting, &project, &read);
    assert_eq!(
        serde_json::to_value(mcp.readiness).unwrap(),
        serde_json::to_value(response.readiness).unwrap()
    );
    drop(read);
    board.upsert_task(task("p", 1, COLUMN_ID_DONE, &[]));
    let reasons = crate::mappers::task_readiness(&waiting, &project, &board.read());
    assert_eq!(reasons.dependencies.len(), 2);
    assert!(
        crate::mappers::task_to_response(&waiting, &project, &board.read())
            .revision_unix_microseconds
            > previous_revision
    );
}

#[test]
fn goal_attention_excludes_finished_deleted_and_optional_questions() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    let epic = goal("p", 10);
    board.upsert_goal(epic.clone());
    let mut required = task("p", 1, COLUMN_ID_TODO, &[]);
    required.goal_number = Some(10);
    question(&mut required, true, 1);
    let mut optional = task("p", 2, COLUMN_ID_TODO, &[]);
    optional.goal_number = Some(10);
    question(&mut optional, false, 2);
    let mut completed = task("p", 3, COLUMN_ID_DONE, &[99]);
    completed.goal_number = Some(10);
    let mut gone = deleted(task("p", 4, COLUMN_ID_TODO, &[99]));
    gone.goal_number = Some(10);
    for task in [required, optional, completed, gone] {
        board.upsert_task(task);
    }
    let waiting = crate::mappers::goal_waiting_tasks(&epic, &board.read());
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].task_id, "TM-1");
    assert_eq!(waiting[0].readiness.required_decisions, 1);
}

#[test]
fn inbox_enforces_membership_and_omits_deleted_or_archived_work() {
    let board = board();
    let mut own = project("p", "TM", &[]);
    own.members.insert("reader@example.test".into());
    board.upsert_project(own);
    board.upsert_project(project("secret", "PRIVATE", &[]));
    let mut archived = project("archive", "OLD", &[]);
    archived.archived_moment = Some(DateTimeAsMicroseconds::new(1));
    board.upsert_project(archived);
    let mut optional = task("p", 1, COLUMN_ID_DONE, &[]);
    question(&mut optional, false, 1);
    let mut required = task("p", 2, COLUMN_ID_TODO, &[]);
    question(&mut required, true, 20);
    let mut gone = deleted(task("p", 3, COLUMN_ID_TODO, &[]));
    question(&mut gone, true, 0);
    let mut secret = task("secret", 1, COLUMN_ID_TODO, &[]);
    question(&mut secret, true, 0);
    let mut old = task("archive", 1, COLUMN_ID_TODO, &[]);
    question(&mut old, true, 0);
    for task in [optional, required, gone, secret, old] {
        board.upsert_task(task);
    }
    let read = board.read();
    let visible = crate::mappers::pending_decisions_for(&read, "reader@example.test", false);
    assert_eq!(
        visible
            .iter()
            .map(|item| item.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["TM-2", "TM-1"]
    );
    assert!(crate::mappers::pending_decisions_for(&read, "nobody@example.test", false).is_empty());
    assert_eq!(
        crate::mappers::pending_decisions_for(&read, "admin@example.test", true).len(),
        3
    );
}

#[test]
fn jev_receipt_identity_tracks_real_context_and_ignores_its_own_log_comment() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    board.upsert_task(task("p", 1, COLUMN_ID_TODO, &[]));
    let index = crate::intelligence::semantic::SemanticIndex::default();
    let first = crate::intelligence::jev::build_request_for(
        &board.read(),
        &index,
        None,
        "TM-1",
        "jev-latest",
    )
    .unwrap();
    let mut updated = board.read().get_task("p", 1).unwrap().as_ref().clone();
    let receipt = crate::intelligence::jev::fixture_review();
    updated.comments.push(crate::board::CommentModel {
        moment: DateTimeAsMicroseconds::new(42_000_000),
        who: receipt.summary.who.clone(),
        text: format!("**Jev evaluation** (`{}`) old receipt", receipt.summary.id),
    });
    updated.ai_reviews.push(receipt);
    board.upsert_task(updated.clone());
    let again = crate::intelligence::jev::build_request_for(
        &board.read(),
        &index,
        None,
        "TM-1",
        "jev-latest",
    )
    .unwrap();
    assert_eq!(first.hash, again.hash);
    updated.comments.push(crate::board::CommentModel {
        moment: DateTimeAsMicroseconds::new(43_000_000),
        who: "owner".into(),
        text: "**Jev evaluation** is not permission. Preserve existing records.".into(),
    });
    board.upsert_task(updated);
    let changed = crate::intelligence::jev::build_request_for(
        &board.read(),
        &index,
        None,
        "TM-1",
        "jev-latest",
    )
    .unwrap();
    assert_ne!(first.hash, changed.hash);
}

#[test]
fn jev_retrieval_never_supplies_candidates_from_another_project() {
    let board = board();
    board.upsert_project(project("p", "TM", &[]));
    board.upsert_project(project("secret", "SECRET", &[]));
    let index = crate::intelligence::semantic::SemanticIndex::default();
    for task in [
        task("p", 1, COLUMN_ID_TODO, &[]),
        task("p", 2, COLUMN_ID_TODO, &[]),
        task("secret", 3, COLUMN_ID_TODO, &[]),
    ] {
        index.upsert(crate::postgres::SemanticVectorDto {
            project_id: task.project_id.clone(),
            task_number: task.number,
            provider_key: "provider".into(),
            content_hash: crate::intelligence::semantic::content_hash(&task),
            model: "embedding-v1".into(),
            dimensions: 2,
            embedding: vec![1.0, 0.0],
        });
        board.upsert_task(task);
    }
    let request = crate::intelligence::jev::build_request_for(
        &board.read(),
        &index,
        Some("provider"),
        "TM-1",
        "jev-latest",
    )
    .unwrap();
    assert_eq!(
        request.duplicates.values().cloned().collect::<Vec<_>>(),
        vec!["TM-2"]
    );
    assert!(!request.payload.to_string().contains("SECRET"));
}

#[test]
fn semantic_fingerprints_track_changes_outside_the_bounded_embedding_excerpt() {
    let mut task = task("p", 1, COLUMN_ID_TODO, &[]);
    task.text = "a".repeat(12000);
    let before = crate::intelligence::semantic::content_hash(&task);
    let excerpt = crate::intelligence::semantic::semantic_text(&task);
    task.text.push_str("changed acceptance criteria");
    assert_eq!(excerpt, crate::intelligence::semantic::semantic_text(&task));
    assert_ne!(before, crate::intelligence::semantic::content_hash(&task));
}

#[test]
fn jev_snapshot_binding_survives_project_prefix_reuse() {
    let board = board();
    board.upsert_project(project("original", "TM", &[]));
    board.upsert_task(task("original", 1, COLUMN_ID_TODO, &[]));
    let index = crate::intelligence::semantic::SemanticIndex::default();
    let first = crate::intelligence::jev::build_request_for(
        &board.read(),
        &index,
        None,
        "TM-1",
        "jev-latest",
    )
    .unwrap();
    board.upsert_project(project("original", "RENAMED", &["TM"]));
    board.upsert_project(project("replacement", "TM", &[]));
    board.upsert_task(task("replacement", 1, COLUMN_ID_TODO, &[]));
    let replacement = crate::intelligence::jev::build_request_for(
        &board.read(),
        &index,
        None,
        "TM-1",
        "jev-latest",
    )
    .unwrap();
    assert_eq!(
        first.payload["state"]["task"],
        replacement.payload["state"]["task"]
    );
    assert_ne!(first.hash, replacement.hash);
}
