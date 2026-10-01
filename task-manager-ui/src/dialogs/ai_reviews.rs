use dioxus::prelude::*;
use task_manager_shared::ai_reviews::AiReviewSummary;

#[component]
pub fn AiReviews(reviews: Vec<AiReviewSummary>) -> Element {
    if reviews.is_empty() {
        return rsx! {};
    }
    rsx! {
        section { class: "task-decision-history",
            h3 { "AI evaluations" }
            p { class: "field-hint", "Recorded recommendations for their original task context. Review them before changing the task." }
            for review in reviews {
                details { class: "task-decision-card", key: "{review.id}",
                    summary { {format!("Jev · {} priority · {}", review.suggested_priority, review.suggested_kind.as_deref().unwrap_or("keep current type"))} }
                    p { {format!("Type probability: {:.0}%. Distribution confidence: {:.0}%.", review.kind_probability * 100.0, review.kind_confidence * 100.0)} }
                    p { {format!("Urgency: {:.2} / 4. Confidence: {:.0}%.", review.urgency_score, review.urgency_confidence * 100.0)} }
                    p { {format!("Probability that clarification is needed: {:.0}%.", review.needs_human_probability * 100.0)} }
                    if let Some(duplicate) = review.suggested_duplicate.clone() {
                        p { "Possible duplicate: " button { class: "task-view-link", onclick: move |_| super::view_task::show(duplicate.clone()), "{review.suggested_duplicate.clone().unwrap_or_default()}" } }
                    }
                    p { class: "muted", "Model: {review.model} · policy: {review.policy_version} · source: {review.source}" }
                    p { class: "muted", "Requested by {review.who} · {super::task_decisions::decision_time(review.created_unix_seconds)}" }
                    p { class: "muted mono", "Snapshot: {review.input_hash}" }
                }
            }
        }
    }
}
