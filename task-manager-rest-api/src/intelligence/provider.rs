use flurl::{FlUrl, body::HttpRequestBody};
use serde_json::Value;
use std::time::Duration;

pub struct EmbeddingConfig {
    pub endpoint: String,
    pub model: String,
    pub key: Option<String>,
}

impl EmbeddingConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(endpoint) = env_value("TASKS_EMBEDDINGS_URL") else {
            return Ok(None);
        };
        if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
            return Err("TASKS_EMBEDDINGS_URL must be an HTTP(S) endpoint".into());
        }
        let model =
            env_value("TASKS_EMBEDDINGS_MODEL").ok_or("TASKS_EMBEDDINGS_MODEL is required")?;
        Ok(Some(Self {
            endpoint,
            model,
            key: env_value("TASKS_EMBEDDINGS_API_KEY"),
        }))
    }
    pub fn provider_key(&self) -> String {
        crate::documents::content_hash(format!("{}\n{}", self.endpoint, self.model).as_bytes())
    }
}

pub fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn post_json(
    endpoint: &str,
    key: Option<&str>,
    payload: &Value,
) -> Result<Value, String> {
    let bytes = serde_json::to_vec(payload).map_err(|_| "could not encode AI request")?;
    if bytes.len() > 512 * 1024 {
        return Err("AI request exceeds the 512 KiB context limit".into());
    }
    tokio::time::timeout(Duration::from_secs(25), async {
        let mut request = FlUrl::new(endpoint);
        if let Some(key) = key {
            request = request.with_header("Authorization", format!("Bearer {key}"));
        }
        let mut response = request
            .post(HttpRequestBody::Json(bytes))
            .await
            .map_err(|_| "AI provider transport failed; no task state was changed".to_string())?;
        if !(200..300).contains(&response.get_status_code()) {
            return Err(format!(
                "AI provider returned HTTP {}; response details are kept out of task history",
                response.get_status_code()
            ));
        }
        let body = response
            .get_body_as_str()
            .await
            .map_err(|_| "AI provider body could not be read".to_string())?;
        if body.len() > 4 * 1024 * 1024 {
            return Err("AI response exceeds the 4 MiB limit".into());
        }
        serde_json::from_str(&body).map_err(|_| "AI provider returned invalid JSON".into())
    })
    .await
    .map_err(|_| "AI provider timed out; retry explicitly if needed".to_string())?
}

pub async fn embed(
    config: &EmbeddingConfig,
    input: &[String],
) -> Result<(String, Vec<Vec<f32>>), String> {
    let response = post_json(
        &config.endpoint,
        config.key.as_deref(),
        &serde_json::json!({"model": config.model, "input": input, "encoding_format": "float"}),
    )
    .await?;
    parse_embeddings(&response, input.len())
}

pub fn parse_embeddings(response: &Value, count: usize) -> Result<(String, Vec<Vec<f32>>), String> {
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
        })
        .ok_or("embedding response has no model identity")?
        .to_string();
    let data = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or("embedding response has no data array")?;
    if data.len() != count {
        return Err("embedding response count differs from the input count".into());
    }
    let mut vectors = vec![None; count];
    let mut dimensions = None;
    for item in data {
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .ok_or("embedding has no index")? as usize;
        if index >= count || vectors[index].is_some() {
            return Err("embedding response has duplicate or invalid indices".into());
        }
        let raw: Vec<f32> = serde_json::from_value(
            item.get("embedding")
                .cloned()
                .ok_or("embedding is missing")?,
        )
        .map_err(|_| "embedding values must be numbers")?;
        let vector = normalize_vector(&raw)?;
        if dimensions.is_some_and(|dimensions| dimensions != vector.len()) {
            return Err("mixed embedding dimensions".into());
        }
        dimensions = Some(vector.len());
        vectors[index] = Some(vector);
    }
    Ok((model, vectors.into_iter().map(Option::unwrap).collect()))
}

pub fn normalize_vector(vector: &[f32]) -> Result<Vec<f32>, String> {
    if vector.is_empty() || vector.len() > 4096 || vector.iter().any(|value| !value.is_finite()) {
        return Err("embedding must have 1..4096 finite coordinates".into());
    }
    let norm = vector
        .iter()
        .map(|value| (*value as f64).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= f64::EPSILON {
        return Err("embedding has zero or invalid length".into());
    }
    Ok(vector
        .iter()
        .map(|value| (*value as f64 / norm) as f32)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vectors_reject_invalid_values_and_restore_provider_order() {
        assert!(normalize_vector(&[0.0, 0.0]).is_err());
        assert!(normalize_vector(&[f32::NAN]).is_err());
        let response = serde_json::json!({"model":"pinned-v1","data":[{"index":1,"embedding":[0,4]},{"index":0,"embedding":[3,0]}]});
        let (_, vectors) = parse_embeddings(&response, 2).unwrap();
        assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        let duplicate = serde_json::json!({"model":"v1","data":[{"index":0,"embedding":[1]},{"index":0,"embedding":[2]}]});
        assert!(parse_embeddings(&duplicate, 2).is_err());
        assert!(parse_embeddings(&response, 1).is_err());
    }

    #[tokio::test]
    async fn transport_posts_json_with_server_side_auth_and_hides_error_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for status in [200, 503] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse::<usize>()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            assert!(headers.contains("authorization: bearer synthetic-test-key"));
                            let body: Value =
                                serde_json::from_slice(&request[end + 4..end + 4 + length])
                                    .unwrap();
                            assert_eq!(body["state"], "synthetic task");
                            break;
                        }
                    }
                }
                let body = if status == 200 {
                    "{\"model\":\"test\"}"
                } else {
                    "private-provider-debug-details"
                };
                let reply = format!(
                    "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            });
            let result = post_json(
                &format!("http://{address}/evaluate"),
                Some("synthetic-test-key"),
                &serde_json::json!({"state":"synthetic task"}),
            )
            .await;
            if status == 200 {
                assert_eq!(result.unwrap()["model"], "test");
            } else {
                let error = result.unwrap_err();
                assert!(error.contains("503"));
                assert!(!error.contains("private-provider"));
            }
            server.await.unwrap();
        }
    }
}
