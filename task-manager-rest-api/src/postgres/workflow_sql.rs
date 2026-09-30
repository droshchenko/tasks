use super::{GoalDto, TaskDto};
use service_sdk::my_postgres::{
    UpdateConflictType,
    sql::{SqlData, SqlValues, build_insert_or_update_sql},
};

// SDK-generated SQL keeps every user string in bound parameters. A data-modifying CTE makes
// the task, its decisions/comments and affected goal clocks one PostgreSQL transaction.
pub fn task_and_goals_statement(task: &TaskDto, goals: &[GoalDto]) -> SqlData {
    let mut queries = vec![build_insert_or_update_sql(
        task,
        "tasks",
        &UpdateConflictType::OnPrimaryKeyConstraint("tasks_pk".into()),
    )];
    queries.extend(goals.iter().map(|goal| {
        build_insert_or_update_sql(
            goal,
            "goals",
            &UpdateConflictType::OnPrimaryKeyConstraint("goals_pk".into()),
        )
    }));
    let mut values = SqlValues::new();
    let placeholders = regex::Regex::new(r"\$(\d+)").unwrap();
    let mut clauses = Vec::new();
    for (index, query) in queries.into_iter().enumerate() {
        let mut mapping = Vec::new();
        if let SqlValues::Values(parameters) = query.values {
            for parameter in parameters {
                mapping.push(values.push(parameter));
            }
        }
        let sql = placeholders.replace_all(&query.sql, |capture: &regex::Captures<'_>| {
            let original: usize = capture[1].parse().unwrap();
            format!("${}", mapping[original - 1])
        });
        clauses.push(format!("write_{index} AS ({sql} RETURNING 1)"));
    }
    SqlData::new(format!("WITH {} SELECT 1", clauses.join(", ")), values)
}
