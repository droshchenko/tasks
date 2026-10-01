use super::{provider, semantic, settings::JEV_ENDPOINT};
use crate::app::AppContext;
use parking_lot::RwLock;
use std::{collections::HashMap, sync::Arc, time::Instant};
use task_manager_shared::ai_settings::{
    IndexJobResponse, IndexStatusResponse, ProviderTestResponse,
};

#[derive(Default)]
pub struct IndexJobs {
    jobs: RwLock<HashMap<String, Job>>,
}

struct Job {
    id: String,
    cancelled: bool,
    response: IndexJobResponse,
}

fn now() -> i64 {
    rust_extensions::date_time::DateTimeAsMicroseconds::now().unix_microseconds / 1_000_000
}

impl IndexJobs {
    fn current(&self, project: &str) -> IndexJobResponse {
        self.jobs
            .read()
            .get(project)
            .map(|job| job.response.clone())
            .unwrap_or_else(|| IndexJobResponse {
                state: "idle".into(),
                ..Default::default()
            })
    }

    fn begin(&self, project: &str, total: i32) -> Result<String, String> {
        let mut jobs = self.jobs.write();
        if jobs
            .values()
            .any(|job| matches!(job.response.state.as_str(), "running" | "cancelling"))
        {
            return Err(
                "An indexing job is already running. Wait for it to finish or cancel it.".into(),
            );
        }
        let id = uuid::Uuid::new_v4().to_string();
        jobs.insert(
            project.into(),
            Job {
                id: id.clone(),
                cancelled: false,
                response: IndexJobResponse {
                    state: "running".into(),
                    total,
                    remaining: total,
                    started_unix_seconds: Some(now()),
                    ..Default::default()
                },
            },
        );
        Ok(id)
    }

    fn cancelled(&self, project: &str, id: &str) -> bool {
        self.jobs
            .read()
            .get(project)
            .is_none_or(|job| job.id != id || job.cancelled)
    }

    fn update(&self, project: &str, id: &str, apply: impl FnOnce(&mut IndexJobResponse)) {
        if let Some(job) = self
            .jobs
            .write()
            .get_mut(project)
            .filter(|job| job.id == id)
        {
            apply(&mut job.response);
        }
    }

    fn finish(&self, project: &str, id: &str, state: &str, message: &str) {
        self.update(project, id, |job| {
            job.state = state.into();
            job.message = message.into();
            job.finished_unix_seconds = Some(now());
        });
    }

    fn cancel(&self, project: &str) {
        if let Some(job) = self
            .jobs
            .write()
            .get_mut(project)
            .filter(|job| job.response.state == "running")
        {
            job.cancelled = true;
            job.response.state = "cancelling".into();
            job.response.message = "Stopping after the current provider request.".into();
        }
    }
}

pub fn index_status(app: &AppContext, prefix: &str) -> Result<IndexStatusResponse, String> {
    let config = app.configuration.embeddings();
    let (config, notice) = match config {
        Ok(config) => (config, String::new()),
        Err(error) => (None, error),
    };
    let board = app.board.read();
    let project = crate::scripts::resolve_project_by_prefix(&board, prefix)?;
    let tasks: Vec<_> = board
        .tasks_of_project(&project.id)
        .iter()
        .filter(|task| !task.is_deleted())
        .cloned()
        .collect();
    let current = config.as_ref().map_or(0, |config| {
        let provider = config.provider_key();
        tasks
            .iter()
            .filter(|task| {
                app.semantic_index
                    .get(&project.id, task.number, &provider)
                    .is_some_and(|row| row.content_hash == semantic::content_hash(task))
            })
            .count()
    });
    Ok(IndexStatusResponse {
        project: project.prefix.clone(),
        configured: config.is_some(),
        tasks_total: tasks.len() as i32,
        vectors_current: current as i32,
        vectors_missing: (tasks.len() - current) as i32,
        model: config.map_or_else(String::new, |config| config.model),
        notice,
        job: app.index_jobs.current(&project.id),
    })
}

pub fn start_index(
    app: Arc<AppContext>,
    prefix: &str,
    force: bool,
) -> Result<IndexStatusResponse, String> {
    let status = index_status(&app, prefix)?;
    if !status.configured {
        return Err("Configure and enable embeddings in Settings before indexing.".into());
    }
    let project_id = crate::scripts::resolve_project_by_prefix(&app.board.read(), prefix)?
        .id
        .clone();
    let total = if force {
        status.tasks_total
    } else {
        status.vectors_missing
    };
    let job_id = app.index_jobs.begin(&project_id, total)?;
    let revision = app.configuration.revision("provider:embeddings");
    let prefix = status.project.clone();
    let worker = app.clone();
    tokio::spawn(async move {
        run_index(worker, prefix, project_id, job_id, force, revision, total).await;
    });
    index_status(&app, &status.project)
}

async fn run_index(
    app: Arc<AppContext>,
    prefix: String,
    project_id: String,
    id: String,
    force: bool,
    revision: i64,
    total: i32,
) {
    let mut after = 0;
    let max_batches = total.max(0) as usize / 16 + 32;
    for _ in 0..max_batches {
        if app.index_jobs.cancelled(&project_id, &id) {
            app.index_jobs.finish(
                &project_id,
                &id,
                "cancelled",
                "Indexing cancelled. Saved vectors are retained.",
            );
            return;
        }
        if app.configuration.revision("provider:embeddings") != revision {
            app.index_jobs.finish(
                &project_id,
                &id,
                "cancelled",
                "Provider settings changed. Start indexing again with the new configuration.",
            );
            return;
        }
        let batch =
            match semantic::index_project(&app, &prefix, 16, force, if force { after } else { 0 })
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    app.index_jobs.finish(&project_id, &id, "error", &error);
                    return;
                }
            };
        app.index_jobs.update(&project_id, &id, |job| {
            job.processed += batch.indexed;
            job.remaining = batch.remaining;
        });
        if batch.remaining == 0 {
            app.index_jobs
                .finish(&project_id, &id, "completed", "Task indexing completed.");
            return;
        }
        if batch.indexed == 0 || (force && batch.next_after.is_none_or(|next| next <= after)) {
            app.index_jobs.finish(&project_id, &id, "partial", "Some tasks changed while indexing. Run indexing again to refresh the remaining tasks.");
            return;
        }
        after = batch.next_after.unwrap_or(after);
        tokio::task::yield_now().await;
    }
    app.index_jobs.finish(&project_id, &id, "partial", "The board kept changing during indexing. Saved vectors are available; run indexing again for the remaining tasks.");
}

pub fn cancel_index(app: &AppContext, prefix: &str) -> Result<IndexStatusResponse, String> {
    let project_id = crate::scripts::resolve_project_by_prefix(&app.board.read(), prefix)?
        .id
        .clone();
    app.index_jobs.cancel(&project_id);
    index_status(app, prefix)
}

pub async fn test_provider(app: &AppContext, name: &str) -> Result<ProviderTestResponse, String> {
    let started = Instant::now();
    let result = match name {
        "embeddings" => {
            let config = app
                .configuration
                .embeddings()?
                .ok_or("Save and enable embeddings before testing the connection.")?;
            provider::embed(&config, &["Synthetic connection check.".into()]).await
                .map(|(model, vectors)| (model, format!("Connection verified; received a {}-dimension embedding for synthetic text.", vectors[0].len())))
        }
        "jev" => {
            let config = app
                .configuration
                .jev()?
                .ok_or("Save and enable Jev before testing the connection.")?;
            let payload = serde_json::json!({"model":config.model,"state":{"synthetic":true,"purpose":"connection_check"},
                "questions":{"connection":{"type":"noul","instructions":"Does the supplied state explicitly set synthetic to true?"}}});
            provider::post_json(JEV_ENDPOINT, Some(&config.key), &payload).await.and_then(|response| {
                let model = response["model"].as_str().filter(|model| !model.is_empty()).ok_or("Jev returned no model identity")?;
                let answer = &response["answers"]["connection"];
                let value = answer["noul"].as_f64().filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
                if answer["type"].as_str() != Some("noul") || value.is_none() { return Err("Jev returned an invalid connection-check response".into()); }
                Ok((model.to_string(), "Connection verified using a synthetic decision. No task was evaluated or changed.".into()))
            })
        }
        _ => return Err("Unknown AI provider.".into()),
    };
    Ok(match result {
        Ok((model, message)) => ProviderTestResponse {
            success: true,
            model,
            message,
            elapsed_ms: started.elapsed().as_millis() as i64,
        },
        Err(message) => ProviderTestResponse {
            success: false,
            model: String::new(),
            message,
            elapsed_ms: started.elapsed().as_millis() as i64,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jobs_are_exclusive_cancellable_and_old_workers_cannot_overwrite_new_runs() {
        let jobs = IndexJobs::default();
        let first = jobs.begin("one", 32).unwrap();
        assert!(jobs.begin("two", 16).is_err());
        jobs.cancel("one");
        assert!(jobs.cancelled("one", &first));
        assert_eq!(jobs.current("one").state, "cancelling");
        jobs.finish("one", &first, "cancelled", "Stopped");
        let next = jobs.begin("one", 16).unwrap();
        jobs.finish("one", &first, "error", "Old error");
        assert_eq!(jobs.current("one").state, "running");
        jobs.finish("one", &next, "completed", "Finished");
        assert_eq!(jobs.current("one").state, "completed");
        assert_eq!(jobs.current("other-project").state, "idle");
    }
}
