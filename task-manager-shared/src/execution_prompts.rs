use my_http_utils::macros::MyHttpObjectStructure;
use serde::{Deserialize, Serialize};

pub const MAX_PROMPT_CHARS: usize = 4000;

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ExecutionPrompt {
    pub target: String,
    pub text: String,
    #[serde(default)]
    pub requires_analysis_documents: bool,
}

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct ResolvedExecutionPrompt {
    pub scope: String,
    pub template_id: String,
    pub target: String,
    pub text: String,
    pub version: String,
    #[serde(default)]
    pub requires_analysis_documents: bool,
}

// Descriptions define the vocabulary; these fragments describe how to do the work.
pub fn validate_prompts(
    prompts: &[ExecutionPrompt],
    targets: &[&str],
) -> Result<Vec<ExecutionPrompt>, String> {
    let mut result = Vec::<ExecutionPrompt>::new();
    let mut seen = Vec::new();
    for prompt in prompts {
        let target = prompt.target.trim().to_lowercase();
        if !targets.contains(&target.as_str()) {
            return Err(format!(
                "prompt target '{target}' does not exist in this template"
            ));
        }
        if seen.contains(&target) {
            return Err(format!("prompt target '{target}' is listed twice"));
        }
        seen.push(target.clone());
        let text = prompt.text.trim();
        if text.contains('\0') || text.chars().count() > MAX_PROMPT_CHARS {
            return Err(format!(
                "prompt '{target}' must contain at most {MAX_PROMPT_CHARS} characters and no NUL"
            ));
        }
        if !text.is_empty() || prompt.requires_analysis_documents {
            result.push(ExecutionPrompt {
                target,
                text: text.to_string(),
                requires_analysis_documents: prompt.requires_analysis_documents,
            });
        }
    }
    result.sort_by(|left, right| left.target.cmp(&right.target));
    Ok(result)
}

pub fn prompt_text<'a>(prompts: &'a [ExecutionPrompt], target: &str) -> &'a str {
    prompts
        .iter()
        .find(|item| item.target == target)
        .map(|item| item.text.as_str())
        .unwrap_or("")
}

pub fn set_prompt(prompts: &mut Vec<ExecutionPrompt>, target: &str, text: String) {
    if let Some(item) = prompts.iter_mut().find(|item| item.target == target) {
        item.text = text;
    } else {
        prompts.push(ExecutionPrompt {
            target: target.to_string(),
            text,
            requires_analysis_documents: false,
        });
    }
}

pub fn requires_analysis(prompts: &[ExecutionPrompt], target: &str) -> bool {
    prompts
        .iter()
        .find(|item| item.target == target)
        .map(|item| item.requires_analysis_documents)
        .unwrap_or(false)
}

pub fn set_analysis_requirement(prompts: &mut Vec<ExecutionPrompt>, target: &str, required: bool) {
    if let Some(item) = prompts.iter_mut().find(|item| item.target == target) {
        item.requires_analysis_documents = required;
    } else {
        prompts.push(ExecutionPrompt {
            target: target.into(),
            text: String::new(),
            requires_analysis_documents: required,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_are_valid_without_becoming_removable_columns() {
        let prompts = vec![ExecutionPrompt {
            target: " DONE ".into(),
            text: " Report evidence ".into(),
            requires_analysis_documents: false,
        }];
        let result = validate_prompts(&prompts, &["todo", "in-development", "done"]).unwrap();
        assert_eq!(result[0].target, "done");
        assert_eq!(result[0].text, "Report evidence");
        assert_eq!(prompt_text(&result, "in-development"), "");
    }

    #[test]
    fn unknown_or_duplicate_targets_are_rejected() {
        let prompt = ExecutionPrompt {
            target: "bug".into(),
            text: "Reproduce the defect".into(),
            requires_analysis_documents: false,
        };
        assert!(validate_prompts(std::slice::from_ref(&prompt), &["analysis"]).is_err());
        assert!(validate_prompts(&[prompt.clone(), prompt], &["bug"]).is_err());
    }

    #[test]
    fn an_analysis_requirement_survives_without_a_prose_prompt_and_legacy_prompts_are_optional() {
        let mut prompts = Vec::new();
        set_analysis_requirement(&mut prompts, "in-analytics", true);
        let validated = validate_prompts(&prompts, &["in-analytics"]).unwrap();
        assert!(requires_analysis(&validated, "in-analytics"));
        let legacy: ExecutionPrompt =
            serde_json::from_str(r#"{"target":"bug","text":"Reproduce"}"#).unwrap();
        assert!(!legacy.requires_analysis_documents);
    }
}
