use task_manager_shared::execution_prompts::ResolvedExecutionPrompt;

use crate::board::{BoardInner, ProjectModel, TaskModel};

pub fn resolve_execution_prompts(
    board: &BoardInner,
    project: &ProjectModel,
    task: &TaskModel,
    target_status: Option<&str>,
) -> Result<Vec<ResolvedExecutionPrompt>, String> {
    let status = target_status
        .map(|status| status.trim().to_lowercase())
        .unwrap_or_else(|| project.effective_status(&task.status));
    if !project.has_column(&status) {
        return Err(format!("'{status}' is not a column of {}", project.prefix));
    }
    let mut result = Vec::new();
    if let Some(template) = project
        .column_template_id
        .as_deref()
        .and_then(|id| board.get_column_template(id))
    {
        if let Some(prompt) = template
            .prompts
            .iter()
            .find(|prompt| prompt.target == status)
        {
            result.push(resolve("column", &template.id, prompt));
        }
    }
    if let Some(kind) = project.effective_kind(task.kind.as_deref()) {
        if let Some(template) = project
            .kind_template_id
            .as_deref()
            .and_then(|id| board.get_kind_template(id))
        {
            if let Some(prompt) = template.prompts.iter().find(|prompt| prompt.target == kind) {
                result.push(resolve("task-type", &template.id, prompt));
            }
        }
    }
    Ok(result)
}

fn resolve(
    scope: &str,
    template_id: &str,
    prompt: &task_manager_shared::execution_prompts::ExecutionPrompt,
) -> ResolvedExecutionPrompt {
    let version = crate::documents::content_hash(
        format!(
            "{scope}\n{template_id}\n{}\n{}\n{}",
            prompt.target, prompt.text, prompt.requires_analysis_documents
        )
        .as_bytes(),
    );
    ResolvedExecutionPrompt {
        scope: scope.into(),
        template_id: template_id.into(),
        target: prompt.target.clone(),
        text: prompt.text.clone(),
        version,
        requires_analysis_documents: prompt.requires_analysis_documents,
    }
}

pub fn validate_analysis_transition(
    board: &BoardInner,
    project: &ProjectModel,
    before: &TaskModel,
    after: &TaskModel,
) -> Result<(), String> {
    if !after.analysis_documents.is_empty() {
        return Ok(());
    }
    let leaving_stage =
        project.effective_status(&before.status) != project.effective_status(&after.status);
    let landing = after.status == "done" && before.status != "done";
    let old = resolve_execution_prompts(board, project, before, None)?;
    let new = resolve_execution_prompts(board, project, after, None)?;
    if old.iter().chain(new.iter()).any(|prompt| {
        prompt.requires_analysis_documents
            && ((prompt.scope == "column"
                && prompt.target != "done"
                && leaving_stage
                && prompt.target == project.effective_status(&before.status))
                || (prompt.scope == "column" && prompt.target == "done" && landing)
                || (prompt.scope == "task-type" && landing))
    }) {
        return Err("attach links to the analysis files with add_analysis_documents before leaving this stage or completing this task type".into());
    }
    Ok(())
}
