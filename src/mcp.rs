use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::embed::Embedder;
use crate::qdrant::{FindFilter, Qdrant};
use crate::schema::{Category, Hall, Memory, Payload, Wing};
use crate::wal::Wal;

const DUPLICATE_THRESHOLD: f32 = 0.95;
/// Upper bound on stored/searched text. nomic-embed-text has ~8k token ctx; 32KB is well above
/// what a sane memory should be, and keeps pathological inputs from flooding Ollama.
const MAX_TEXT_BYTES: usize = 32 * 1024;
/// Cap on a single palace_recall batch. Keeps one tool call from fetching the whole palace.
const MAX_RECALL_IDS: usize = 100;

#[derive(Clone)]
pub struct Palace {
    embedder: Arc<Embedder>,
    qdrant: Arc<Qdrant>,
    wal: Arc<Wal>,
    tool_router: ToolRouter<Palace>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreArgs {
    /// The memory to file. Store verbatim — do not summarise.
    pub text: String,
    /// Palace category.
    pub category: Category,
    /// Palace wing.
    pub wing: Wing,
    /// Room — free-text topic or project (e.g. "memqdrant", "hermytt", "family").
    pub room: String,
    /// Hall — facts / events / decisions / discoveries / preferences.
    pub hall: Hall,
    /// Optional session identifier — the conversation that produced this memory.
    #[serde(default)]
    pub session: Option<String>,
    /// Optional source path if the memory was imported from a markdown file.
    #[serde(default)]
    pub source_file: Option<String>,
}

#[derive(Debug, Serialize)]
struct StoreResult {
    id: u64,
    duplicate_of: Option<u64>,
    score: Option<f32>,
    text: String,
    wing: String,
    room: String,
    hall: String,
    timestamp: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindArgs {
    /// Natural-language query. Embedded with nomic-embed-text before search.
    pub query: String,
    /// Max results. Default 5, max 20.
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub wing: Option<Wing>,
    #[serde(default)]
    pub category: Option<Category>,
    /// Exact-match room filter.
    #[serde(default)]
    pub room: Option<String>,
    #[serde(default)]
    pub hall: Option<Hall>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecallArgs {
    /// Point IDs to fetch verbatim. No embedding needed.
    pub ids: Vec<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CheckDuplicateArgs {
    /// Candidate text. Returns the closest existing memory and whether it's above the duplicate threshold (0.95).
    pub text: String,
}

#[tool_router]
impl Palace {
    pub fn new(embedder: Embedder, qdrant: Qdrant, wal: Wal) -> Self {
        Self {
            embedder: Arc::new(embedder),
            qdrant: Arc::new(qdrant),
            wal: Arc::new(wal),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "File a verbatim memory into the palace. Categorise by wing, room, hall. Returns the new point ID (or the existing one if a near-duplicate is found above 0.95 cosine)."
    )]
    async fn palace_store(
        &self,
        Parameters(args): Parameters<StoreArgs>,
    ) -> Result<CallToolResult, McpError> {
        let res = self.do_store(args).await.map_err(err)?;
        let payload = serde_json::to_value(&res).map_err(err)?;
        Ok(CallToolResult::success(vec![Content::text(
            payload.to_string(),
        )]))
    }

    #[tool(
        description = "Semantic search over the palace. Optional typed filters (wing/category/room/hall) narrow the search before vector comparison."
    )]
    async fn palace_find(
        &self,
        Parameters(args): Parameters<FindArgs>,
    ) -> Result<CallToolResult, McpError> {
        let results = self.do_find(args).await.map_err(err)?;
        let payload = serde_json::to_value(&results).map_err(err)?;
        Ok(CallToolResult::success(vec![Content::text(
            payload.to_string(),
        )]))
    }

    #[tool(
        description = "Fetch palace points by explicit IDs. No embedding — cheap lookup when you already know what you want."
    )]
    async fn palace_recall(
        &self,
        Parameters(args): Parameters<RecallArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.ids.len() > MAX_RECALL_IDS {
            return Err(err(format!(
                "too many ids: {} (max {})",
                args.ids.len(),
                MAX_RECALL_IDS
            )));
        }
        let results = self.qdrant.retrieve(args.ids).await.map_err(err)?;
        let payload = serde_json::to_value(&results).map_err(err)?;
        Ok(CallToolResult::success(vec![Content::text(
            payload.to_string(),
        )]))
    }

    #[tool(
        description = "Palace status: total point count plus breakdown by wing and by hall. Useful for agents orienting themselves before searching."
    )]
    async fn palace_status(&self) -> Result<CallToolResult, McpError> {
        let total = self.qdrant.count(&FindFilter::default()).await.map_err(err)?;
        let wings = self.qdrant.facet("wing").await.map_err(err)?;
        let halls = self.qdrant.facet("hall").await.map_err(err)?;
        let categories = self.qdrant.facet("category").await.map_err(err)?;
        let body = json!({
            "collection": self.qdrant.collection(),
            "total": total,
            "wings": facet_map(&wings),
            "halls": facet_map(&halls),
            "categories": facet_map(&categories),
        });
        Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
    }

    #[tool(
        description = "Faceted taxonomy: value → count for wing, room, hall, category. Same data as palace_status but flatter — good for dump-the-layout queries."
    )]
    async fn palace_taxonomy(&self) -> Result<CallToolResult, McpError> {
        let wings = self.qdrant.facet("wing").await.map_err(err)?;
        let rooms = self.qdrant.facet("room").await.map_err(err)?;
        let halls = self.qdrant.facet("hall").await.map_err(err)?;
        let categories = self.qdrant.facet("category").await.map_err(err)?;
        let body = json!({
            "wings": facet_map(&wings),
            "rooms": facet_map(&rooms),
            "halls": facet_map(&halls),
            "categories": facet_map(&categories),
        });
        Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
    }

    #[tool(
        description = "Check whether candidate text is already in the palace. Returns the closest match and a flag if cosine ≥ 0.95. Call this before palace_store to avoid duplicates."
    )]
    async fn palace_check_duplicate(
        &self,
        Parameters(args): Parameters<CheckDuplicateArgs>,
    ) -> Result<CallToolResult, McpError> {
        let vec = self.embedder.embed(&args.text).await.map_err(err)?;
        let hits = self
            .qdrant
            .search(vec, 1, &FindFilter::default())
            .await
            .map_err(err)?;
        let top = hits.into_iter().next();
        let is_duplicate = top
            .as_ref()
            .and_then(|m| m.score)
            .map(|s| s >= DUPLICATE_THRESHOLD)
            .unwrap_or(false);
        let body = json!({
            "is_duplicate": is_duplicate,
            "threshold": DUPLICATE_THRESHOLD,
            "closest": top,
        });
        Ok(CallToolResult::success(vec![Content::text(body.to_string())]))
    }
}

impl Palace {
    async fn do_store(&self, args: StoreArgs) -> anyhow::Result<StoreResult> {
        if args.text.len() > MAX_TEXT_BYTES {
            anyhow::bail!(
                "text too large: {} bytes (max {})",
                args.text.len(),
                MAX_TEXT_BYTES
            );
        }
        if args.text.trim().is_empty() {
            anyhow::bail!("text is empty");
        }
        let vec = self.embedder.embed(&args.text).await?;

        // Duplicate check — if the top hit is above threshold, skip the write and return the existing ID.
        let existing = self
            .qdrant
            .search(vec.clone(), 1, &FindFilter::default())
            .await?;
        if let Some(top) = existing.first() {
            if top.score.unwrap_or(0.0) >= DUPLICATE_THRESHOLD && top.text == args.text {
                tracing::info!(id = top.id, "skipping store — exact duplicate");
                return Ok(StoreResult {
                    id: top.id,
                    duplicate_of: Some(top.id),
                    score: top.score,
                    text: top.text.clone(),
                    wing: top.wing.clone(),
                    room: top.room.clone(),
                    hall: top.hall.clone(),
                    timestamp: top.timestamp.clone(),
                });
            }
        }

        let id = new_id();
        let timestamp = now_rfc3339();
        let payload = Payload {
            category: args.category.as_str().to_string(),
            wing: args.wing.as_str().to_string(),
            room: args.room.clone(),
            hall: args.hall.as_str().to_string(),
            text: args.text.clone(),
            timestamp: timestamp.clone(),
            session: args.session.clone(),
            source_file: args.source_file.clone(),
        };

        self.wal.log(
            "palace_store",
            &json!({
                "id": id,
                "wing": payload.wing,
                "room": payload.room,
                "hall": payload.hall,
                "category": payload.category,
                "text_preview": preview(&payload.text),
                "session": payload.session,
            }),
        );

        self.qdrant.upsert(id, vec, payload.clone()).await?;

        Ok(StoreResult {
            id,
            duplicate_of: None,
            score: None,
            text: payload.text,
            wing: payload.wing,
            room: payload.room,
            hall: payload.hall,
            timestamp: payload.timestamp,
        })
    }

    async fn do_find(&self, args: FindArgs) -> anyhow::Result<Vec<Memory>> {
        if args.query.len() > MAX_TEXT_BYTES {
            anyhow::bail!("query too large: {} bytes (max {})", args.query.len(), MAX_TEXT_BYTES);
        }
        let limit = args.limit.unwrap_or(5).clamp(1, 20);
        let filter = FindFilter {
            wing: args.wing.map(|w| w.as_str().to_string()),
            category: args.category.map(|c| c.as_str().to_string()),
            room: args.room,
            hall: args.hall.map(|h| h.as_str().to_string()),
        };
        let vec = self.embedder.embed(&args.query).await?;
        self.qdrant.search(vec, limit, &filter).await
    }
}

#[tool_handler]
impl ServerHandler for Palace {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_protocol_version(ProtocolVersion::LATEST)
        .with_instructions(
            "memqdrant — Cali's memory palace over MCP. \
             Every memory has a wing (projects/infrastructure/nexpublica/personal/career/vibe), \
             a room (free-text project or topic), and a hall (facts/events/decisions/discoveries/preferences). \
             Tools: palace_store, palace_find, palace_recall, palace_status, palace_taxonomy, palace_check_duplicate.".to_string(),
        )
    }
}

fn new_id() -> u64 {
    // Unix millis, guaranteed above the 1_000_000_000 floor.
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0) as u64;
    millis.max(1_000_000_000)
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
}

fn format_rfc3339(mut secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    secs %= 86_400;
    let hour = (secs / 3600) as u32;
    let minute = ((secs / 60) % 60) as u32;
    let second = (secs % 60) as u32;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = (y + if m <= 2 { 1 } else { 0 }) as i64;
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn facet_map(items: &[(String, u64)]) -> serde_json::Value {
    let mut m = serde_json::Map::with_capacity(items.len());
    for (k, v) in items {
        m.insert(k.clone(), json!(v));
    }
    serde_json::Value::Object(m)
}

fn preview(s: &str) -> String {
    const MAX: usize = 120;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX).collect();
    format!("{truncated}…")
}

fn err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}
