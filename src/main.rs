mod embed;
mod mcp;
mod qdrant;
mod schema;
mod util;
mod wal;

use anyhow::{Context, Result};
use rmcp::{
    ServiceExt,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::embed::Embedder;
use crate::mcp::Palace;
use crate::qdrant::Qdrant;
use crate::wal::Wal;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

struct Config {
    ollama_url: String,
    ollama_model: String,
    qdrant_url: String,
    collection: String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            ollama_url: env_or("OLLAMA_URL", "http://localhost:11434"),
            ollama_model: env_or("OLLAMA_MODEL", "nomic-embed-text"),
            qdrant_url: env_or("QDRANT_URL", "http://localhost:6333"),
            collection: env_or("COLLECTION", "claude-memory"),
        }
    }

    fn make_palace(&self) -> Palace {
        let embedder = Embedder::new(&self.ollama_url, &self.ollama_model);
        let qdrant = Qdrant::new(&self.qdrant_url, &self.collection);
        let wal = Wal::from_env();
        Palace::new(embedder, qdrant, wal)
    }

    fn make_qdrant(&self) -> Qdrant {
        Qdrant::new(&self.qdrant_url, &self.collection)
    }
}

fn print_help() {
    eprintln!(
        "memqdrant {} — MCP server over Qdrant memory palace
Usage:
  memqdrant                      Serve MCP over stdio (default)
  memqdrant serve [--bind ADDR]  Serve MCP over Streamable HTTP at POST /mcp
                                 (default ADDR: 127.0.0.1:6334, override with MEMQDRANT_BIND)
  memqdrant --help               Show this message

Environment:
  OLLAMA_URL    (default http://localhost:11434)
  OLLAMA_MODEL  (default nomic-embed-text)
  QDRANT_URL    (default http://localhost:6333)
  COLLECTION    (default claude-memory)
  MEMQDRANT_WAL (default ~/.memqdrant/wal.jsonl)
  MEMQDRANT_BIND (default 127.0.0.1:6334 in serve mode)
  MEMQDRANT_ALLOWED_HOSTS (default localhost,127.0.0.1,::1 — set to \"*\" to disable DNS rebinding check)
  RUST_LOG      (default memqdrant=info)
",
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    // Tracing goes to stderr only — stdout is the stdio MCP transport channel.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("memqdrant=info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_stdio().await,
        Some("serve") => run_http(&args[1..]).await,
        Some("--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("memqdrant {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown argument: {other}");
            print_help();
            std::process::exit(2);
        }
    }
}

async fn run_stdio() -> Result<()> {
    let cfg = Config::from_env();
    tracing::info!(
        ollama = %cfg.ollama_url,
        model = %cfg.ollama_model,
        qdrant = %cfg.qdrant_url,
        collection = %cfg.collection,
        mode = "stdio",
        "memqdrant starting"
    );

    if let Err(e) = cfg.make_qdrant().ensure_indexes().await {
        tracing::warn!("ensure_indexes: {e:#}");
    }

    let palace = cfg.make_palace();
    let service = palace.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("mcp serve: {e:?}");
    })?;
    service.waiting().await?;
    Ok(())
}

async fn run_http(rest: &[String]) -> Result<()> {
    let mut bind = std::env::var("MEMQDRANT_BIND").unwrap_or_else(|_| "127.0.0.1:6334".into());
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--bind" => {
                bind = rest
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("--bind requires an address"))?
                    .clone();
                i += 2;
            }
            other => anyhow::bail!("unknown serve argument: {other}"),
        }
    }

    let cfg = Config::from_env();
    tracing::info!(
        ollama = %cfg.ollama_url,
        model = %cfg.ollama_model,
        qdrant = %cfg.qdrant_url,
        collection = %cfg.collection,
        bind = %bind,
        mode = "streamable-http",
        "memqdrant starting"
    );

    if let Err(e) = cfg.make_qdrant().ensure_indexes().await {
        tracing::warn!("ensure_indexes: {e:#}");
    }

    let ct = CancellationToken::new();
    let ct_child = ct.child_token();
    let mut http_config = StreamableHttpServerConfig::default().with_cancellation_token(ct_child);
    match std::env::var("MEMQDRANT_ALLOWED_HOSTS") {
        Ok(raw) if raw.trim() == "*" => {
            tracing::warn!(
                "MEMQDRANT_ALLOWED_HOSTS=* — DNS rebinding protection DISABLED. Ensure the listener is behind a trusted reverse proxy or firewall."
            );
            http_config = http_config.disable_allowed_hosts();
        }
        Ok(raw) => {
            let hosts: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            tracing::info!(?hosts, "Host header allowlist");
            http_config = http_config.with_allowed_hosts(hosts);
        }
        Err(_) => {
            tracing::info!(
                "Host header allowlist defaults to localhost/127.0.0.1/::1 — set MEMQDRANT_ALLOWED_HOSTS to accept remote clients."
            );
        }
    }

    let service = StreamableHttpService::new(
        move || Ok(cfg.make_palace()),
        LocalSessionManager::default().into(),
        http_config,
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!("listening on {bind} at POST /mcp");

    let shutdown = ct.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received");
            shutdown.cancel();
        })
        .await?;
    Ok(())
}
