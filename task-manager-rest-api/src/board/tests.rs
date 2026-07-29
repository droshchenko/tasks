use std::collections::BTreeSet;

use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::{COLUMN_ID_DONE, COLUMN_ID_TODO};

use super::{Board, ColumnModel, KindModel, ProjectModel, TaskModel, UserModel};

fn project(id: &str, prefix: &str, history: &[&str]) -> ProjectModel {
    ProjectModel {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        prefix: prefix.to_string(),
        prefix_history: history.iter().map(|itm| itm.to_string()).collect(),
        columns: vec![ColumnModel {
            id: "in-progress".to_string(),
            name: "In progress".to_string(),
            description: String::new(),
            order: 10,
        }],
        kinds: vec![KindModel {
            id: "bug".to_string(),
            name: "Bug".to_string(),
            description: String::new(),
            color: KindColor::Red,
        }],
        members: BTreeSet::new(),
        last_task_number: 0,
        created: DateTimeAsMicroseconds::new(0),
    }
}

fn task(project_id: &str, number: i64, status: &str, depends_on: &[i64]) -> TaskModel {
    TaskModel {
        project_id: project_id.to_string(),
        number,
        text: format!("task {number}"),
        status: status.to_string(),
        kind: None,
        assignee: None,
        labels: Vec::new(),
        depends_on: depends_on.to_vec(),
        comments: Vec::new(),
        created: DateTimeAsMicroseconds::new(0),
        updated: DateTimeAsMicroseconds::new(0),
    }
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
    let board = Board::new();
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
    let board = Board::new();
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
    let board = Board::new();
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
    let board = Board::new();
    // `archived` is not among the project's columns, so a task sitting there reads as Todo.
    board.upsert_project(project("p", "RMS", &[]));
    board.upsert_task(task("p", 1, "archived", &[]));

    assert!(board.read().is_blocked(&task("p", 2, COLUMN_ID_TODO, &[1])));
}

#[test]
fn blocks_is_the_reverse_edge_and_is_sorted() {
    let board = Board::new();
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
    let board = Board::new();
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
    let board = Board::new();
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
    let board = Board::new();
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
    let board = Board::new();
    board.upsert_project(project("a", "TM", &["RMS"]));

    let read = board.read();

    assert!(read.is_prefix_free("RMS", None));
    assert!(!read.is_prefix_free("TM", None));
    // A project renaming itself does not collide with its own current prefix.
    assert!(read.is_prefix_free("TM", Some("a")));
}

#[test]
fn the_counter_only_moves_forward_and_is_per_project() {
    let board = Board::new();
    board.upsert_project(project("a", "AAA", &[]));
    board.upsert_project(project("b", "BBB", &[]));

    assert_eq!(board.reserve_task_number("a"), Some(1));
    assert_eq!(board.reserve_task_number("a"), Some(2));
    assert_eq!(board.reserve_task_number("b"), Some(1));

    board.remove_task("a", 2);
    assert_eq!(
        board.reserve_task_number("a"),
        Some(3),
        "a deleted number must not be handed out again"
    );

    assert_eq!(board.reserve_task_number("missing"), None);
}

#[test]
fn a_projects_labels_are_the_distinct_labels_its_tasks_carry() {
    let board = Board::new();
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

    // Dropping it from the last task carrying it removes the label from the vocabulary — there is no
    // labels table for it to linger in.
    board.remove_task("p", 1);
    board.remove_task("p", 2);
    assert!(board.read().labels_of_project("p").is_empty());
}

#[test]
fn visibility_follows_membership_and_an_admin_sees_everything() {
    let board = Board::new();

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
    let board = Board::new();
    board.upsert_user(user("yuri@mxtm.ai", "Yuri"));
    board.upsert_user(user("noname@mxtm.ai", ""));

    let read = board.read();

    assert_eq!(
        read.display_name_of("yuri@mxtm.ai"),
        Some("Yuri".to_string())
    );
    assert_eq!(read.display_name_of("noname@mxtm.ai"), None);
    assert_eq!(read.display_name_of("claude"), None);
    assert_eq!(read.display_name_of("stranger@mxtm.ai"), None);
}
