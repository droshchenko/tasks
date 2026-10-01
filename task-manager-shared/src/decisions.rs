use my_http_utils::macros::{MyHttpInput, MyHttpObjectStructure};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DecisionChoice {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub consequence: String,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct DecisionAnswer {
    pub option_id: Option<String>,
    pub text: String,
    pub answered_by: String,
    pub answered_unix_seconds: i64,
    pub source: String,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct TaskDecision {
    pub id: String,
    pub request_key: String,
    pub action: String,
    pub question: String,
    pub options: Vec<DecisionChoice>,
    pub required: bool,
    pub asked_by: String,
    pub asked_unix_seconds: i64,
    pub status: String,
    pub answer: Option<DecisionAnswer>,
    pub cancel_reason: Option<String>,
    pub cancelled_by: Option<String>,
    pub cancelled_unix_seconds: Option<i64>,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct PendingDecision {
    pub task_id: String,
    pub task_title: String,
    pub project: String,
    pub project_name: String,
    pub decision: TaskDecision,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct PendingDecisionsResponse {
    pub items: Vec<PendingDecision>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRequest {
    pub request_key: String,
    pub action: String,
    pub question: String,
    pub options: Vec<DecisionChoice>,
    pub required: bool,
    pub asked_by: String,
}

impl TaskDecision {
    pub fn is_waiting(&self) -> bool {
        self.status == "pending"
    }
    pub fn blocks_completion(&self) -> bool {
        self.required && self.is_waiting()
    }
}

pub fn add_decision(
    history: &mut Vec<TaskDecision>,
    request: DecisionRequest,
    id: String,
    now: i64,
) -> Result<(usize, bool), String> {
    let mut request = request;
    request.request_key = request.request_key.trim().to_string();
    request.action = request.action.trim().to_string();
    request.question = request.question.trim().to_string();
    request.asked_by = request.asked_by.trim().to_string();
    if request.request_key.is_empty() || request.request_key.len() > 200 {
        return Err("a question needs a stable request_key of at most 200 bytes".into());
    }
    for (name, value) in [
        ("action", &request.action),
        ("question", &request.question),
        ("asked_by", &request.asked_by),
    ] {
        if value.is_empty() || value.contains('\0') || value.chars().count() > 4000 {
            return Err(format!(
                "a decision needs a nonempty {name} of at most 4000 characters"
            ));
        }
    }
    if request.options.len() > 12 {
        return Err("a question may offer at most 12 choices".into());
    }
    if request
        .options
        .iter()
        .filter(|choice| choice.recommended)
        .count()
        > 1
    {
        return Err("only one choice may be recommended".into());
    }
    let mut seen = Vec::new();
    for choice in &mut request.options {
        choice.id = choice.id.trim().to_string();
        choice.label = choice.label.trim().to_string();
        choice.consequence = choice.consequence.trim().to_string();
        if choice.id.is_empty()
            || choice.id.len() > 100
            || choice.id.contains('\0')
            || seen.contains(&choice.id)
        {
            return Err("choice ids must be nonempty, unique and at most 100 bytes".into());
        }
        if choice.label.is_empty()
            || choice.label.contains('\0')
            || choice.consequence.contains('\0')
            || choice.label.chars().count() > 500
            || choice.consequence.chars().count() > 2000
        {
            return Err("each choice needs a label of at most 500 characters and a consequence of at most 2000".into());
        }
        seen.push(choice.id.clone());
    }
    if let Some(index) = history
        .iter()
        .position(|item| item.request_key == request.request_key)
    {
        let existing = &history[index];
        if existing.action.trim() == request.action
            && existing.question.trim() == request.question
            && existing
                .options
                .iter()
                .zip(&request.options)
                .all(|(old, new)| {
                    old.id == new.id
                        && old.label.trim() == new.label
                        && old.consequence.trim() == new.consequence
                        && old.recommended == new.recommended
                })
            && existing.options.len() == request.options.len()
            && existing.required == request.required
            && existing.asked_by.trim() == request.asked_by
        {
            return Ok((index, false));
        }
        return Err("this request_key already names a different question; use a new key for a revised question".into());
    }
    history.push(TaskDecision {
        id,
        request_key: request.request_key,
        action: request.action,
        question: request.question,
        options: request.options,
        required: request.required,
        asked_by: request.asked_by,
        asked_unix_seconds: now,
        status: "pending".into(),
        answer: None,
        cancel_reason: None,
        cancelled_by: None,
        cancelled_unix_seconds: None,
    });
    Ok((history.len() - 1, true))
}

pub fn answer_decision(
    history: &mut [TaskDecision],
    id: &str,
    mut answer: DecisionAnswer,
) -> Result<bool, String> {
    let decision = history
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("no decision '{id}' on this task"))?;
    answer.option_id = answer
        .option_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    answer.text = answer.text.trim().to_string();
    if answer.answered_by.trim().is_empty()
        || answer.answered_by.contains('\0')
        || answer.answered_by.len() > 4000
        || answer.text.contains('\0')
        || answer.text.chars().count() > 8000
    {
        return Err("an answer needs an author and at most 8000 characters without NUL".into());
    }
    if let Some(option) = &answer.option_id {
        if !decision.options.iter().any(|choice| &choice.id == option) {
            return Err(format!("'{option}' is not one of this question's choices"));
        }
    } else if answer.text.is_empty() {
        return Err(
            "choose an option or provide a free-text answer; a default is not a human answer"
                .into(),
        );
    }
    if let Some(existing) = &decision.answer {
        if existing.option_id == answer.option_id
            && existing.text == answer.text
            && existing.answered_by == answer.answered_by
            && existing.source == answer.source
        {
            return Ok(false);
        }
        return Err(
            "this question was already answered; ask a new question instead of rewriting history"
                .into(),
        );
    }
    if !decision.is_waiting() {
        return Err("this question was cancelled and cannot be answered".into());
    }
    decision.status = "answered".into();
    decision.answer = Some(answer);
    Ok(true)
}

pub fn cancel_decision(
    history: &mut [TaskDecision],
    id: &str,
    reason: &str,
    who: &str,
    now: i64,
) -> Result<bool, String> {
    let decision = history
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("no decision '{id}' on this task"))?;
    if reason.trim().is_empty()
        || who.trim().is_empty()
        || reason.contains('\0')
        || who.contains('\0')
        || reason.chars().count() > 4000
        || who.len() > 4000
    {
        return Err("cancelling a question needs a reason and author of at most 4000 characters without NUL".into());
    }
    if decision.status == "cancelled" && decision.cancel_reason.as_deref() == Some(reason.trim()) {
        return Ok(false);
    }
    if !decision.is_waiting() {
        return Err("only an unanswered question can be cancelled".into());
    }
    decision.status = "cancelled".into();
    decision.cancel_reason = Some(reason.trim().into());
    decision.cancelled_by = Some(who.trim().into());
    decision.cancelled_unix_seconds = Some(now);
    Ok(true)
}

pub fn validate_decision_history(history: &[TaskDecision]) -> Result<(), String> {
    let mut validated = Vec::new();
    let mut ids = Vec::new();
    for decision in history {
        if decision.id != decision.id.trim()
            || decision.request_key != decision.request_key.trim()
            || decision
                .options
                .iter()
                .any(|choice| choice.id != choice.id.trim())
            || decision
                .answer
                .as_ref()
                .and_then(|answer| answer.option_id.as_deref())
                .map(|id| id != id.trim() || id.is_empty())
                .unwrap_or(false)
        {
            return Err("imported decision and option identifiers must use their canonical spelling without surrounding whitespace".into());
        }
        if decision.id.trim().is_empty()
            || decision.id.contains('\0')
            || decision.id.len() > 200
            || ids.contains(&decision.id)
        {
            return Err("decision ids must be nonempty and unique within a task".into());
        }
        ids.push(decision.id.clone());
        let (_, added) = add_decision(
            &mut validated,
            DecisionRequest {
                request_key: decision.request_key.clone(),
                action: decision.action.clone(),
                question: decision.question.clone(),
                options: decision.options.clone(),
                required: decision.required,
                asked_by: decision.asked_by.clone(),
            },
            decision.id.clone(),
            decision.asked_unix_seconds,
        )?;
        if !added {
            return Err("decision request keys must be unique within a task".into());
        }
        match decision.status.as_str() {
            "pending" if decision.answer.is_none() && decision.cancel_reason.is_none() && decision.cancelled_by.is_none() && decision.cancelled_unix_seconds.is_none() => {},
            "answered" if decision.cancel_reason.is_none() && decision.cancelled_by.is_none() && decision.cancelled_unix_seconds.is_none() => {
                let answer = decision.answer.clone().ok_or("an answered question needs its actual answer")?;
                answer_decision(&mut validated, &decision.id, answer)?;
            },
            "cancelled" if decision.answer.is_none() => {
                cancel_decision(&mut validated, &decision.id, decision.cancel_reason.as_deref().ok_or("a cancelled question needs a reason")?,
                    decision.cancelled_by.as_deref().ok_or("a cancelled question needs an author")?,
                    decision.cancelled_unix_seconds.ok_or("a cancelled question needs a time")?)?;
            },
            _ => return Err("inconsistent decision state: pending, answered and cancelled records must have their matching payloads".into()),
        }
    }
    Ok(())
}

#[derive(MyHttpInput)]
pub struct AnswerDecisionInputModel {
    #[http_body(name: "taskId", description: "The task containing the question")]
    pub task_id: String,
    #[http_body(name: "decisionId", description: "The question to answer")]
    pub decision_id: String,
    #[http_body(name: "optionId", description: "Chosen option id, or empty for a free-text answer")]
    pub option_id: Option<String>,
    #[http_body(name: "text", description: "Free-text answer or an optional note")]
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> DecisionRequest {
        DecisionRequest {
            request_key: "deploy-release".into(),
            action: "Deploy release 2".into(),
            question: "Deploy now?".into(),
            options: vec![DecisionChoice {
                id: "prepare".into(),
                label: "Prepare only".into(),
                consequence: "No rollout".into(),
                recommended: true,
            }],
            required: true,
            asked_by: "AI".into(),
        }
    }
    fn answer() -> DecisionAnswer {
        DecisionAnswer {
            option_id: Some("prepare".into()),
            text: String::new(),
            answered_by: "owner@example.org".into(),
            answered_unix_seconds: 2,
            source: "ui".into(),
        }
    }
    #[test]
    fn question_and_reply_are_idempotent_and_preserve_the_original_choices() {
        let mut history = Vec::new();
        assert_eq!(
            add_decision(&mut history, request(), "d1".into(), 1).unwrap(),
            (0, true)
        );
        assert_eq!(
            add_decision(&mut history, request(), "d2".into(), 3).unwrap(),
            (0, false)
        );
        assert!(history[0].blocks_completion());
        assert!(answer_decision(&mut history, "d1", answer()).unwrap());
        assert!(!answer_decision(&mut history, "d1", answer()).unwrap());
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].options[0].label, "Prepare only");
        assert_eq!(history[0].asked_unix_seconds, 1);
        assert!(!history[0].blocks_completion());
    }
    #[test]
    fn a_recommended_option_does_not_answer_the_question() {
        let mut history = Vec::new();
        add_decision(&mut history, request(), "d1".into(), 1).unwrap();
        let mut reply = answer();
        reply.option_id = None;
        assert!(answer_decision(&mut history, "d1", reply).is_err());
        assert!(history[0].blocks_completion());
        assert!(answer_decision(&mut history, "other-task-id", answer()).is_err());
    }
    #[test]
    fn free_text_is_stored_without_inventing_a_selection_and_history_is_immutable() {
        let mut history = Vec::new();
        add_decision(&mut history, request(), "d1".into(), 1).unwrap();
        let mut reply = answer();
        reply.option_id = None;
        reply.text = "Wait until Monday".into();
        answer_decision(&mut history, "d1", reply).unwrap();
        assert_eq!(history[0].answer.as_ref().unwrap().option_id, None);
        assert!(answer_decision(&mut history, "d1", answer()).is_err());
        let mut revised = request();
        revised.action = "Deploy release 3".into();
        assert!(add_decision(&mut history, revised, "d2".into(), 4).is_err());
    }
    #[test]
    fn cancellation_preserves_the_question_and_rejects_late_replies() {
        let mut history = Vec::new();
        add_decision(&mut history, request(), "d1".into(), 1).unwrap();
        cancel_decision(&mut history, "d1", "Scope changed", "AI", 2).unwrap();
        assert_eq!(history[0].action, "Deploy release 2");
        assert_eq!(history[0].cancel_reason.as_deref(), Some("Scope changed"));
        assert!(!history[0].blocks_completion());
        assert!(answer_decision(&mut history, "d1", answer()).is_err());
    }

    #[test]
    fn imported_history_must_have_consistent_states_and_unique_question_ids() {
        let mut history = Vec::new();
        add_decision(&mut history, request(), "d1".into(), 1).unwrap();
        validate_decision_history(&history).unwrap();
        history[0].status = "answered".into();
        assert!(validate_decision_history(&history).is_err());
        history[0].status = "pending".into();
        history[0].options[0].id = " prepare ".into();
        assert!(validate_decision_history(&history).is_err());
        history[0].options[0].id = "prepare".into();
        history[0].request_key = " deploy-release ".into();
        assert!(validate_decision_history(&history).is_err());
        history[0].request_key = "deploy-release".into();
        answer_decision(&mut history, "d1", answer()).unwrap();
        validate_decision_history(&history).unwrap();
        history.push(history[0].clone());
        assert!(validate_decision_history(&history).is_err());
    }
}
