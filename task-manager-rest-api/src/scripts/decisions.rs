use rust_extensions::date_time::DateTimeAsMicroseconds;
use task_manager_shared::decisions::{DecisionAnswer, DecisionRequest};

use crate::board::{CommentModel, TaskModel};

#[derive(Default)]
pub enum DecisionPatch {
    #[default]
    None,
    Request(DecisionRequest),
    Answer {
        id: String,
        answer: DecisionAnswer,
    },
    Cancel {
        id: String,
        reason: String,
        who: String,
    },
}

impl DecisionPatch {
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn apply(&self, task: &mut TaskModel, now: DateTimeAsMicroseconds) -> Result<(), String> {
        let seconds = now.unix_microseconds / 1_000_000;
        let comment = match self {
            Self::None => None,
            Self::Request(request) => {
                if request.required
                    && task.status == "done"
                    && !task
                        .decisions
                        .iter()
                        .any(|decision| decision.request_key == request.request_key.trim())
                {
                    return Err(
                        "re-open the task before requesting a required human decision".into(),
                    );
                }
                let (index, changed) = task_manager_shared::decisions::add_decision(
                    &mut task.decisions,
                    request.clone(),
                    rust_extensions::SortableId::generate().to_string(),
                    seconds,
                )?;
                if changed {
                    let decision = &task.decisions[index];
                    let choices = decision
                        .options
                        .iter()
                        .map(|choice| {
                            format!(
                                "- `{}`: **{}** — {}",
                                choice.id, choice.label, choice.consequence
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Some((
                        decision.asked_by.clone(),
                        format!(
                            "**Human decision requested** (`{}`)\n\n**Action:** {}\n\n{}\n\n{}\n\n_Question recorded by {}._",
                            decision.id,
                            decision.action,
                            decision.question,
                            choices,
                            decision.asked_by
                        ),
                    ))
                } else {
                    None
                }
            }
            Self::Answer { id, answer } => {
                let mut answer = answer.clone();
                answer.answered_unix_seconds = seconds;
                if task_manager_shared::decisions::answer_decision(
                    &mut task.decisions,
                    id,
                    answer.clone(),
                )? {
                    let decision = task
                        .decisions
                        .iter()
                        .find(|decision| &decision.id == id)
                        .unwrap();
                    let answer = decision.answer.as_ref().unwrap();
                    let choice = answer
                        .option_id
                        .as_deref()
                        .and_then(|id| decision.options.iter().find(|choice| choice.id == id))
                        .map(|choice| choice.label.as_str())
                        .unwrap_or("Free-text answer");
                    Some((
                        answer.answered_by.clone(),
                        format!(
                            "**Human decision answered** (`{id}`)\n\n**Action:** {}\n\n**Choice:** {choice}\n\n{}\n\n_Answer recorded by {}; source: {}._",
                            decision.action, answer.text, answer.answered_by, answer.source
                        ),
                    ))
                } else {
                    None
                }
            }
            Self::Cancel { id, reason, who } => {
                if task_manager_shared::decisions::cancel_decision(
                    &mut task.decisions,
                    id,
                    reason,
                    who,
                    seconds,
                )? {
                    Some((
                        who.clone(),
                        format!(
                            "**Human decision cancelled** (`{id}`): {reason}\n\n_Cancelled by {who}._"
                        ),
                    ))
                } else {
                    None
                }
            }
        };
        if let Some((who, text)) = comment {
            task.comments.push(CommentModel {
                moment: now,
                who,
                text,
            });
        }
        Ok(())
    }
}
