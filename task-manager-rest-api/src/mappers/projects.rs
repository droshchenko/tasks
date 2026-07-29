use std::collections::BTreeSet;

use rust_extensions::AsStr;
use task_manager_shared::kind_color::KindColor;
use task_manager_shared::projects::{ProjectColumnResponse, ProjectKindResponse, ProjectResponse};

use crate::board::{ColumnModel, KindModel, ProjectModel};
use crate::postgres::{ProjectColumnJsonModel, ProjectDto, ProjectKindJsonModel};

// Conversions are `From` rather than `Into`. The house style says "always an impl, never a standalone
// `map_x_to_y` function", and `From` satisfies that while also giving `Into` for free — and clippy's
// `from_over_into` rejects the other direction, which matters under `-D warnings`.

impl From<&ProjectColumnJsonModel> for ColumnModel {
    fn from(src: &ProjectColumnJsonModel) -> Self {
        Self {
            id: src.id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            order: src.column_order,
        }
    }
}

impl From<&ColumnModel> for ProjectColumnJsonModel {
    fn from(src: &ColumnModel) -> Self {
        Self {
            id: src.id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            column_order: src.order,
        }
    }
}

impl From<&ProjectKindJsonModel> for KindModel {
    fn from(src: &ProjectKindJsonModel) -> Self {
        Self {
            id: src.id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            // A colour this build does not recognise is decoration that failed to load, not a
            // corrupt row — it draws as the default swatch.
            color: KindColor::parse_or_default(&src.color),
        }
    }
}

impl From<&KindModel> for ProjectKindJsonModel {
    fn from(src: &KindModel) -> Self {
        Self {
            id: src.id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            color: src.color.as_str().to_string(),
        }
    }
}

/// Postgres row -> memory. Membership is not in the project row (it has its own table), so it starts
/// empty and the loader fills it.
impl From<&ProjectDto> for ProjectModel {
    fn from(src: &ProjectDto) -> Self {
        let mut columns: Vec<ColumnModel> = src.columns.iter().map(|itm| itm.into()).collect();
        // Sorted once here so every reader is already in board order.
        columns.sort_by_key(|itm| itm.order);

        Self {
            id: src.id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            prefix: src.prefix.to_uppercase(),
            prefix_history: src
                .prefix_history
                .iter()
                .map(|itm| itm.to_uppercase())
                .collect(),
            columns,
            kinds: src.kinds.iter().map(|itm| itm.into()).collect(),
            members: BTreeSet::new(),
            last_task_number: src.last_task_number,
            created: src.created,
        }
    }
}

/// Memory -> Postgres row. Membership is deliberately absent: it lives in `project_members`, and a
/// project row that also carried it would give the same fact two homes.
impl From<&ProjectModel> for ProjectDto {
    fn from(src: &ProjectModel) -> Self {
        Self {
            id: src.id.clone(),
            name: src.name.clone(),
            description: src.description.clone(),
            prefix: src.prefix.clone(),
            prefix_history: src.prefix_history.clone(),
            columns: src.columns.iter().map(|itm| itm.into()).collect(),
            kinds: src.kinds.iter().map(|itm| itm.into()).collect(),
            last_task_number: src.last_task_number,
            created: src.created,
        }
    }
}

/// Memory -> wire.
///
/// The two anchor columns are not part of `columns` — a client adds Todo at the start and Done at the
/// end.
///
/// `tasks_amount` is derived against the board rather than read off the project, so it is passed in.
/// There is no `labels` here on purpose: the UI configures projects and never tags anything, so the
/// label vocabulary is an MCP-only concern and lives on the MCP view instead.
pub fn project_to_response(src: &ProjectModel, tasks_amount: usize) -> ProjectResponse {
    ProjectResponse {
        id: src.id.clone(),
        name: src.name.clone(),
        description: src.description.clone(),
        prefix: src.prefix.clone(),
        prefix_history: src.prefix_history.clone(),
        columns: src
            .columns
            .iter()
            .map(|itm| ProjectColumnResponse {
                id: itm.id.clone(),
                name: itm.name.clone(),
                description: itm.description.clone(),
                order: itm.order,
            })
            .collect(),
        kinds: src
            .kinds
            .iter()
            .map(|itm| ProjectKindResponse {
                id: itm.id.clone(),
                name: itm.name.clone(),
                description: itm.description.clone(),
                color: itm.color.as_str().to_string(),
            })
            .collect(),
        members: src.members.iter().cloned().collect(),
        tasks_amount: tasks_amount as i32,
    }
}
