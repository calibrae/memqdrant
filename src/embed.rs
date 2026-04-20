use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct Embedder {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

#[derive(Serialize)]
struct EmbedReq<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct EmbedResp {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Serialize)]
struct LegacyReq<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct LegacyResp {
    embedding: Vec<f32>,
}

impl Embedder {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Embed a single string. Tries `/api/embed` (Ollama ≥0.1.33) first,
    /// falls back to legacy `/api/embeddings` on 404.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let new_url = format!("{}/api/embed", self.base_url);
        let resp = self
            .client
            .post(&new_url)
            .json(&EmbedReq {
                model: &self.model,
                input: vec![text],
            })
            .send()
            .await
            .with_context(|| format!("POST {new_url}"))?;

        if resp.status().is_success() {
            let body: EmbedResp = resp.json().await.context("decode /api/embed")?;
            return body
                .embeddings
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("empty embeddings from /api/embed"));
        }

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            tracing::debug!("/api/embed 404, falling back to /api/embeddings");
            let legacy_url = format!("{}/api/embeddings", self.base_url);
            let body: LegacyResp = self
                .client
                .post(&legacy_url)
                .json(&LegacyReq {
                    model: &self.model,
                    prompt: text,
                })
                .send()
                .await
                .with_context(|| format!("POST {legacy_url}"))?
                .error_for_status()?
                .json()
                .await
                .context("decode /api/embeddings")?;
            return Ok(body.embedding);
        }

        Err(anyhow!(
            "embed request to {} failed: {} {}",
            new_url,
            resp.status(),
            resp.text().await.unwrap_or_default()
        ))
    }
}
