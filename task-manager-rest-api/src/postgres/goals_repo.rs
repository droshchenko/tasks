use std::sync::Arc;
use std::time::Duration;

use service_sdk::my_postgres::sql_where::NoneWhereModel;
use service_sdk::my_postgres::{MyPostgres, UpdateConflictType};

service_sdk::macros::use_my_postgres!();

use crate::app::APP_NAME;
use crate::settings::SettingsReader;

pub const TABLE_NAME: &str = "goals";
pub const PK_NAME: &str = "goals_pk";

// A goal — a container for tasks, an epic.
//
// It does not hold its tasks: the task row carries the goal id. That direction is what makes deleting a
// goal harmless, and it is also the direction every read wants, since a task is drawn far more often than
// a goal is listed.
#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Debug)]
pub struct GoalDto {
    #[primary_key(0)]
    pub id: String,
    #[db_index(id: 0, index_name: "goals_project_idx", is_unique: false, order: "ASC")]
    pub project_id: String,
    pub name: String,
    pub description: String,
    #[sql_type("timestamp")]
    pub created: DateTimeAsMicroseconds,
}

#[derive(WhereDbModel, Debug)]
pub struct GoalByIdWhereModel<'s> {
    pub id: &'s str,
}

pub struct GoalsRepo {
    postgres: MyPostgres,
}

impl GoalsRepo {
    pub async fn new(settings_reader: Arc<SettingsReader>) -> Self {
        let postgres = MyPostgres::from_settings(APP_NAME, settings_reader)
            .with_table_schema_verification::<GoalDto>(TABLE_NAME, Some(PK_NAME.into()))
            .build()
            .await;

        Self { postgres }
    }

    /// Every goal. Called once at startup — after that the answer lives in memory.
    pub async fn get_all(&self, ctx: &MyTelemetryContext) -> Vec<GoalDto> {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .query_rows(TABLE_NAME, NoneWhereModel::new(), Some(ctx))
            .await
            .expect("goals: query_rows get_all failed")
    }

    pub async fn upsert(&self, row: &GoalDto, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .insert_or_update_db_entity(
                TABLE_NAME,
                UpdateConflictType::OnPrimaryKeyConstraint(PK_NAME.into()),
                row,
                Some(ctx),
            )
            .await
            .expect("goals: insert_or_update_db_entity failed");
    }

    pub async fn delete(&self, id: &str, ctx: &MyTelemetryContext) {
        self.postgres
            .with_retries(3, Duration::from_secs(1))
            .delete(TABLE_NAME, &GoalByIdWhereModel { id }, Some(ctx))
            .await
            .expect("goals: delete failed");
    }
}
