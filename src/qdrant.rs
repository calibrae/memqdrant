use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::schema::{Memory, Payload};

#[derive(Clone)]
pub struct Qdrant {
    client: reqwest::Client,
    base_url: String,
    collection: String,
}

#[derive(Debug, Default, Clone)]
pub struct FindFilter {
    pub wing: Option<String>,
    pub category: Option<String>,
    pub room: Option<String>,
    pub hall: Option<String>,
}

impl FindFilter {
    fn is_empty(&self) -> bool {
        self.wing.is_none() && self.category.is_none() && self.room.is_none() && self.hall.is_none()
    }

    fn to_qdrant_filter(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let mut must = Vec::new();
        let pairs = [
            ("wing", &self.wing),
            ("category", &self.category),
            ("room", &self.room),
            ("hall", &self.hall),
        ];
        for (key, val) in pairs {
            if let Some(v) = val {
                must.push(json!({"key": key, "match": {"value": v}}));
            }
        }
        Some(json!({ "must": must }))
    }
}

#[derive(Debug, Serialize)]
struct UpsertBody {
    points: Vec<PointUpsert>,
}

#[derive(Debug, Serialize)]
struct PointUpsert {
    id: u64,
    vector: Vec<f32>,
    payload: Payload,
}

#[derive(Debug, Deserialize)]
struct ScoredPoint {
    id: Value,
    score: f32,
    payload: Option<Payload>,
}

#[derive(Debug, Deserialize)]
struct RetrievedPoint {
    id: Value,
    payload: Option<Payload>,
}

#[derive(Debug, Deserialize)]
struct ResultWrapper<T> {
    result: T,
}

impl Qdrant {
    pub fn new(base_url: impl Into<String>, collection: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            collection: collection.into(),
        }
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    fn url(&self, path: &str) -> String {
        format!("{}/collections/{}{}", self.base_url, self.collection, path)
    }

    pub async fn upsert(&self, id: u64, vector: Vec<f32>, payload: Payload) -> Result<()> {
        let url = self.url("/points?wait=true");
        let body = UpsertBody {
            points: vec![PointUpsert {
                id,
                vector,
                payload,
            }],
        };
        let resp = self
            .client
            .put(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("PUT {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("qdrant upsert {}: {}", status, text));
        }
        Ok(())
    }

    pub async fn search(
        &self,
        vector: Vec<f32>,
        limit: u32,
        filter: &FindFilter,
    ) -> Result<Vec<Memory>> {
        let url = self.url("/points/search");
        let mut body = json!({
            "vector": vector,
            "limit": limit,
            "with_payload": true,
        });
        if let Some(f) = filter.to_qdrant_filter() {
            body["filter"] = f;
        }
        let resp: ResultWrapper<Vec<ScoredPoint>> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?
            .error_for_status()
            .context("qdrant search status")?
            .json()
            .await
            .context("qdrant search decode")?;
        Ok(resp
            .result
            .into_iter()
            .filter_map(to_memory_scored)
            .collect())
    }

    pub async fn retrieve(&self, ids: Vec<u64>) -> Result<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let url = self.url("/points");
        let body = json!({
            "ids": ids,
            "with_payload": true,
        });
        let resp: ResultWrapper<Vec<RetrievedPoint>> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?
            .error_for_status()
            .context("qdrant retrieve status")?
            .json()
            .await
            .context("qdrant retrieve decode")?;
        Ok(resp
            .result
            .into_iter()
            .filter_map(to_memory_plain)
            .collect())
    }

    pub async fn count(&self, filter: &FindFilter) -> Result<u64> {
        let url = self.url("/points/count");
        let mut body = json!({ "exact": true });
        if let Some(f) = filter.to_qdrant_filter() {
            body["filter"] = f;
        }
        let resp: ResultWrapper<Value> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?
            .error_for_status()
            .context("qdrant count status")?
            .json()
            .await
            .context("qdrant count decode")?;
        Ok(resp
            .result
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(0))
    }

    /// Facet a single key. Returns (value, count) pairs.
    pub async fn facet(&self, key: &str) -> Result<Vec<(String, u64)>> {
        let url = self.url("/facet");
        let body = json!({ "key": key, "limit": 100, "exact": true });
        let resp: ResultWrapper<Value> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?
            .error_for_status()
            .context("qdrant facet status")?
            .json()
            .await
            .context("qdrant facet decode")?;
        let hits = resp
            .result
            .get("hits")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(hits.len());
        for h in hits {
            let val = h
                .get("value")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let count = h.get("count").and_then(Value::as_u64).unwrap_or(0);
            if let Some(v) = val {
                out.push((v, count));
            }
        }
        Ok(out)
    }

    /// Ensure keyword indexes exist on wing, category, room, hall. Idempotent —
    /// Qdrant accepts re-creation as no-op. Required for the facet API.
    pub async fn ensure_indexes(&self) -> Result<()> {
        let url = self.url("/index?wait=true");
        for field in ["wing", "category", "room", "hall"] {
            let body = json!({ "field_name": field, "field_schema": "keyword" });
            let resp = self.client.put(&url).json(&body).send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(anyhow!("index {field}: {status} {text}"));
            }
        }
        Ok(())
    }
}

fn id_as_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn to_memory_scored(p: ScoredPoint) -> Option<Memory> {
    let id = id_as_u64(&p.id)?;
    let pl = p.payload?;
    Some(Memory {
        id,
        score: Some(p.score),
        text: pl.text,
        category: pl.category,
        wing: pl.wing,
        room: pl.room,
        hall: pl.hall,
        timestamp: pl.timestamp,
        session: pl.session,
        source_file: pl.source_file,
    })
}

fn to_memory_plain(p: RetrievedPoint) -> Option<Memory> {
    let id = id_as_u64(&p.id)?;
    let pl = p.payload?;
    Some(Memory {
        id,
        score: None,
        text: pl.text,
        category: pl.category,
        wing: pl.wing,
        room: pl.room,
        hall: pl.hall,
        timestamp: pl.timestamp,
        session: pl.session,
        source_file: pl.source_file,
    })
}
