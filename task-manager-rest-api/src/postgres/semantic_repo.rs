use service_sdk::my_postgres::{MyPostgres, UpdateConflictType, sql_where::NoneWhereModel};
use std::sync::Arc;
use std::time::Duration;
service_sdk::macros::use_my_postgres!();
use crate::{app::APP_NAME, settings::SettingsReader};

#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Debug, Clone)]
pub struct SemanticVectorDto {
    #[primary_key(0)]
    pub project_id: String,
    #[primary_key(1)]
    pub task_number: i64,
    #[primary_key(2)]
    pub provider_key: String,
    pub content_hash: String,
    pub model: String,
    pub dimensions: i32,
    #[sql_type("jsonb")]
    #[json]
    pub embedding: Vec<f32>,
}

pub struct SemanticRepo {
    postgres: MyPostgres,
}
impl SemanticRepo {
    pub async fn new(settings: Arc<SettingsReader>) -> Self {
        let postgres = MyPostgres::from_settings(APP_NAME, settings)
            .with_table_schema_verification::<SemanticVectorDto>(
                "semantic_vectors",
                Some("semantic_vectors_pk".into()),
            )
            .build()
            .await;
        Self { postgres }
    }
    pub async fn get_all(
        &self,
        ctx: &MyTelemetryContext,
    ) -> Result<Vec<SemanticVectorDto>, String> {
        self.postgres
            .with_retries(2, Duration::from_secs(1))
            .query_rows("semantic_vectors", NoneWhereModel::new(), Some(ctx))
            .await
            .map_err(|_| "semantic cache could not be loaded".into())
    }
    pub async fn upsert(
        &self,
        row: &SemanticVectorDto,
        ctx: &MyTelemetryContext,
    ) -> Result<(), String> {
        self.postgres
            .with_retries(2, Duration::from_secs(1))
            .insert_or_update_db_entity(
                "semantic_vectors",
                UpdateConflictType::OnPrimaryKeyConstraint("semantic_vectors_pk".into()),
                row,
                Some(ctx),
            )
            .await
            .map_err(|_| "semantic cache could not be saved".to_string())?;
        Ok(())
    }
}
