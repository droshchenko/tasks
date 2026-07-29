use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use service_sdk::my_postgres::sql_where::NoneWhereModel;
use service_sdk::my_postgres::{MyPostgres, UpdateConflictType};

service_sdk::macros::use_my_postgres!();

use crate::app::APP_NAME;
use crate::settings::SettingsReader;

pub const TABLE_NAME: &str = "tasks";
pub const PK_NAME: &str = "tasks_pk";

// One comment, inside the task row's `comments` jsonb.
//
// The thread rides on the task rather than living in its own table so that adding a comment is one
// atomic upsert of one row. A second table would mean two writes with no transaction around them,
// and the in-memory copy would have to be reconciled against a half-applied change.
//
// `who` is an email or the literal `claude`, unvalidated on purpose: MCP has no session to derive an
// author from, and an author whose user row was later removed still has to render.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TaskCommentJsonModel {
    pub moment_unix_seconds: i64,
    pub who: String,
    pub text: String,
}

// One task.
//
// The primary key is `(project_id, number)`, and there is deliberately **no** column for the human
// id: the counter is per project and a prefix can move between projects, so two projects can both
// produce `RMS-1`. The displayed id is composed on read from the project's current prefix. For the
// same reason `depends_on` holds numbers — storing `"RMS-7"` would break every dependency in the
// project the moment its prefix changed.
//
// `task_text` and `column_order`-style names avoid Postgres keywords: the schema generator does not
// quote identifiers, and a column it creates but cannot then find in the catalogue gets recreated on
// every startup.
//
// `status` and `kind` are ids of the owning project's configured columns and kinds, with no foreign
// key to either. A column is deleted freely and its tasks keep their stored status, reading as Todo
// from then on — a constraint would have made that impossible instead of merely lenient.
#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Debug)]
pub struct TaskDto {
    #[primary_key(0)]
    pub project_id: String,
    #[primary_key(1)]
    pub number: i64,
    pub task_text: String,
    pub status: String,
    pub kind: Option<String>,
    pub assignee: Option<String>,
    #[sql_type("jsonb")]
    #[json]
    pub labels: Vec<String>,
    #[sql_type("jsonb")]
    #[json]
    pub depends_on: Vec<i64>,
    #[sql_type("jsonb")]
    #[json]
    pub comments: Vec<TaskCommentJsonModel>,
    #[sql_type("timestamp")]
    pub created: DateTimeAsMicroseconds,
    #[sql_type("timestamp")]
    pub updated: DateTimeAsMicroseconds,
}

// Deleting one task. The PK is composite, so both halves are needed.
#[derive(WhereDbModel, Debug)]
pub struct DeleteTaskWhereModel<'s> {
    pub project_id: &'s str,
    pub number: i64,
}

pub struct TasksRepo {
    postgres: MyPostgres,
}

impl TasksRepo {
    pub async fn new(settings_reader: Arc<SettingsReader>) -> Self {
        let postgres = MyPostgres::from_settings(APP_NAME, settings_reader)
            .with_table_schema_verification::<TaskDto>(TABLE_NAME, Some(PK_NAME.into()))
            .build()
            .await;

        Self { postgres }
    }

    /// Every task of every project. Called once at startup — the board is held in memory after that.
    pub async fn get_all(&self, ctx: &MyTelemetryContext) -> Vec<TaskDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_rows(TABLE_NAME, NoneWhereModel::new(), Some(ctx))
            .await
            .expect("tasks: query_rows get_all failed")
    }

    pub async fn upsert(&self, row: &TaskDto, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .insert_or_update_db_entity(
                TABLE_NAME,
                UpdateConflictType::OnPrimaryKeyConstraint(PK_NAME.into()),
                row,
                Some(ctx),
            )
            .await
            .expect("tasks: insert_or_update_db_entity failed");
    }

    pub async fn delete(&self, project_id: &str, number: i64, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .delete(
                TABLE_NAME,
                &DeleteTaskWhereModel { project_id, number },
                Some(ctx),
            )
            .await
            .expect("tasks: delete failed");
    }
}
