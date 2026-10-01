use service_sdk::my_postgres::{MyPostgres, UpdateConflictType, sql_where::NoneWhereModel};
use std::{sync::Arc, time::Duration};
service_sdk::macros::use_my_postgres!();
use crate::{app::APP_NAME, settings::SettingsReader};

#[derive(SelectDbEntity, InsertDbEntity, UpdateDbEntity, TableSchema, Clone)]
pub struct AppSettingDto {
    #[primary_key(0)]
    pub id: String,
    pub value: String,
    pub revision: i64,
    pub updated_by: String,
}

impl std::fmt::Debug for AppSettingDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppSettingDto")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

pub struct AppSettingsRepo {
    postgres: MyPostgres,
}

impl AppSettingsRepo {
    pub async fn new(settings: Arc<SettingsReader>) -> Self {
        let postgres = MyPostgres::from_settings(APP_NAME, settings)
            .with_table_schema_verification::<AppSettingDto>(
                "app_settings",
                Some("app_settings_pk".into()),
            )
            .build()
            .await;
        Self { postgres }
    }

    pub async fn get_all(&self) -> Result<Vec<AppSettingDto>, String> {
        self.postgres
            .with_retries(2, Duration::from_secs(1))
            .query_rows("app_settings", NoneWhereModel::new(), None)
            .await
            .map_err(|_| "application settings could not be loaded".into())
    }

    pub async fn upsert(&self, row: &AppSettingDto) -> Result<(), String> {
        self.postgres
            .with_retries(2, Duration::from_secs(1))
            .insert_or_update_db_entity(
                "app_settings",
                UpdateConflictType::OnPrimaryKeyConstraint("app_settings_pk".into()),
                row,
                None,
            )
            .await
            .map_err(|_| "application settings could not be saved".to_string())?;
        Ok(())
    }
}
