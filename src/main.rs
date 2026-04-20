mod embed;
mod mcp;
mod qdrant;
mod schema;
mod wal;

use anyhow::Result;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::EnvFilter;

use crate::embed::Embedder;
use crate::mcp::Palace;
use crate::qdrant::Qdrant;
use crate::wal::Wal;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    // Tracing goes to stderr only — stdout is the MCP transport channel.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("memqdrant=info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let ollama_url = env_or("OLLAMA_URL", "http://localhost:11434");
    let ollama_model = env_or("OLLAMA_MODEL", "nomic-embed-text");
    let qdrant_url = env_or("QDRANT_URL", "http://localhost:6333");
    let collection = env_or("COLLECTION", "claude-memory");

    tracing::info!(
        ollama = %ollama_url,
        model = %ollama_model,
        qdrant = %qdrant_url,
        collection = %collection,
        "memqdrant starting"
    );

    let embedder = Embedder::new(ollama_url, ollama_model);
    let qdrant = Qdrant::new(qdrant_url, collection);
    let wal = Wal::from_env();

    if let Err(e) = qdrant.ensure_indexes().await {
        tracing::warn!("ensure_indexes: {e:#}");
    }

    let palace = Palace::new(embedder, qdrant, wal);

    let service = palace.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("mcp serve: {e:?}");
    })?;
    service.waiting().await?;
    Ok(())
}
