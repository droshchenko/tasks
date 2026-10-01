use my_http_utils::macros::MyHttpObjectStructure;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, MyHttpObjectStructure, Clone, Debug, PartialEq)]
pub struct AiReviewSummary {
    pub id: String,
    pub input_hash: String,
    pub requested_model: String,
    pub model: String,
    pub policy_version: String,
    pub who: String,
    pub created_unix_seconds: i64,
    pub source: String,
    pub suggested_kind: Option<String>,
    pub kind_probability: f64,
    pub kind_confidence: f64,
    pub suggested_priority: String,
    pub urgency_score: f64,
    pub urgency_confidence: f64,
    pub needs_human_probability: f64,
    pub suggested_duplicate: Option<String>,
    pub duplicate_probability: f64,
    pub duplicate_confidence: f64,
    pub review_required: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AiReview {
    pub summary: AiReviewSummary,
    pub request_json: String,
    pub response_json: String,
}

pub fn validate_reviews(reviews: &[AiReview]) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for review in reviews {
        let item = &review.summary;
        let metadata = [
            &item.id,
            &item.input_hash,
            &item.requested_model,
            &item.model,
            &item.policy_version,
            &item.who,
            &item.source,
            &item.suggested_priority,
        ];
        if metadata
            .iter()
            .any(|value| value.contains('\0') || value.len() > 4000)
            || item
                .suggested_kind
                .iter()
                .chain(item.suggested_duplicate.iter())
                .any(|value| value.contains('\0') || value.len() > 4000)
        {
            return Err("AI review metadata contains unsupported characters or is too long".into());
        }
        if item.id.is_empty()
            || item.id.len() > 128
            || !seen.insert(&item.id)
            || item.input_hash.len() != 64
            || !item.input_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            || item.who.trim().is_empty()
            || item.model.trim().is_empty()
            || review.request_json.len() > 65536
            || review.response_json.len() > 65536
        {
            return Err("invalid AI review identity, provenance or snapshot size".into());
        }
        for value in [
            item.kind_probability,
            item.kind_confidence,
            item.urgency_confidence,
            item.needs_human_probability,
            item.duplicate_probability,
            item.duplicate_confidence,
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err("invalid AI probability or confidence".into());
            }
        }
        if !item.urgency_score.is_finite() || !(0.0..=4.0).contains(&item.urgency_score) {
            return Err("invalid urgency score".into());
        }
        serde_json::from_str::<serde_json::Value>(&review.request_json)
            .map_err(|_| "invalid AI request snapshot")?;
        serde_json::from_str::<serde_json::Value>(&review.response_json)
            .map_err(|_| "invalid AI response snapshot")?;
    }
    Ok(())
}
