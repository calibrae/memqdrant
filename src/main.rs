mod baselines;
mod embed;
mod mcp;
mod qdrant;
mod schema;
mod util;
mod wal;

#[cfg(all(feature = "ollama", feature = "fastembed"))]
compile_error!(
    "features `ollama` and `fastembed` are mutually exclusive — pick one with --features, and --no-default-features if you want fastembed."
);

#[cfg(not(any(feature = "ollama", feature = "fastembed")))]
compile_error!("enable one of the embedding features: `ollama` (default) or `fastembed`.");

use std::path::PathBuf;

use anyhow::{Context, Result};
use mcp_gain::Tracker;
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
    #[cfg(all(feature = "ollama", not(feature = "fastembed")))]
    ollama_url: String,
    #[cfg(all(feature = "ollama", not(feature = "fastembed")))]
    ollama_model: String,
    qdrant_url: String,
    collection: String,
}

impl Config {
    fn from_env() -> Self {
        Self {
            #[cfg(all(feature = "ollama", not(feature = "fastembed")))]
            ollama_url: env_or("OLLAMA_URL", "http://localhost:11434"),
            #[cfg(all(feature = "ollama", not(feature = "fastembed")))]
            ollama_model: env_or("OLLAMA_MODEL", "nomic-embed-text"),
            qdrant_url: env_or("QDRANT_URL", "http://localhost:6333"),
            collection: env_or("COLLECTION", "claude-memory"),
        }
    }

    fn make_palace(&self) -> Result<Palace> {
        let embedder = make_embedder(self)?;
        let qdrant = Qdrant::new(&self.qdrant_url, &self.collection);
        let wal = Wal::from_env();
        let tracker = make_tracker();
        Ok(Palace::new(embedder, qdrant, wal, tracker))
    }

    fn make_qdrant(&self) -> Qdrant {
        Qdrant::new(&self.qdrant_url, &self.collection)
    }
}

/// Where the gain analytics log lives. Defaults to `/var/lib/palazzo/usage.jsonl`
/// (matches the systemd unit's ReadWritePaths) but is overridable for local dev
/// or relocated deployments.
fn usage_log_path() -> PathBuf {
    std::env::var("PALAZZO_USAGE_LOG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/palazzo/usage.jsonl"))
}

fn gain_enabled() -> bool {
    !matches!(
        std::env::var("PALAZZO_GAIN_ENABLED").as_deref(),
        Ok("0" | "false" | "no" | "off")
    )
}

fn make_tracker() -> Tracker {
    Tracker::new(usage_log_path(), gain_enabled(), baselines::BASELINES)
}

const BACKEND: &str = if cfg!(feature = "fastembed") {
    "fastembed:NomicEmbedTextV15"
} else {
    "ollama"
};

#[cfg(all(feature = "ollama", not(feature = "fastembed")))]
fn make_embedder(cfg: &Config) -> Result<Embedder> {
    Ok(Embedder::new(&cfg.ollama_url, &cfg.ollama_model))
}

#[cfg(feature = "fastembed")]
fn make_embedder(_cfg: &Config) -> Result<Embedder> {
    Embedder::new()
}

fn print_help() {
    eprintln!(
        "palazzo {} — MCP server over Qdrant memory palace
Usage:
  palazzo                      Serve MCP over stdio (default)
  palazzo serve [--bind ADDR]  Serve MCP over Streamable HTTP at POST /mcp
                               (default ADDR: 127.0.0.1:6334, override with PALAZZO_BIND)
  palazzo gain [--since-secs N] [--json]
                               Render the token-savings report from PALAZZO_USAGE_LOG.
                               Defaults to all-time text rendering; --json emits the structured Summary.
  palazzo --help               Show this message

Environment:
  OLLAMA_URL    (default http://localhost:11434)
  OLLAMA_MODEL  (default nomic-embed-text)
  QDRANT_URL    (default http://localhost:6333)
  COLLECTION    (default claude-memory)
  PALAZZO_WAL   (default ~/.palazzo/wal.jsonl)
  PALAZZO_BIND  (default 127.0.0.1:6334 in serve mode)
  PALAZZO_ALLOWED_HOSTS (default localhost,127.0.0.1,::1 — set to \"*\" to disable DNS rebinding check)
  PALAZZO_USAGE_LOG (default /var/lib/palazzo/usage.jsonl — gain analytics JSONL)
  PALAZZO_GAIN_ENABLED (default 1; set 0/false/no/off to disable per-call recording)
  RUST_LOG      (default palazzo=info)
",
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    // Tracing goes to stderr only — stdout is the stdio MCP transport channel.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("palazzo=info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => run_stdio().await,
        Some("serve") => run_http(&args[1..]).await,
        Some("gain") => run_gain(&args[1..]),
        Some("--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("palazzo {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown argument: {other}");
            print_help();
            std::process::exit(2);
        }
    }
}

fn run_gain(rest: &[String]) -> Result<()> {
    let mut since_secs: Option<u64> = None;
    let mut as_json = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--since-secs" => {
                since_secs = Some(
                    rest.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("--since-secs requires a value"))?
                        .parse()
                        .context("--since-secs must be a non-negative integer")?,
                );
                i += 2;
            }
            "--json" => {
                as_json = true;
                i += 1;
            }
            other => anyhow::bail!("unknown gain argument: {other}"),
        }
    }
    let since = since_secs.map(|s| chrono::Utc::now() - chrono::Duration::seconds(s as i64));
    let tracker = make_tracker();
    let summary = tracker.summary(since)?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", mcp_gain::render_text(&summary, &baselines::header()));
    }
    Ok(())
}

async fn run_stdio() -> Result<()> {
    let cfg = Config::from_env();
    tracing::info!(
        backend = BACKEND,
        qdrant = %cfg.qdrant_url,
        collection = %cfg.collection,
        mode = "stdio",
        "palazzo starting"
    );

    if let Err(e) = cfg.make_qdrant().ensure_indexes().await {
        tracing::warn!("ensure_indexes: {e:#}");
    }

    let palace = cfg.make_palace()?;
    let service = palace.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("mcp serve: {e:?}");
    })?;
    service.waiting().await?;
    Ok(())
}

async fn run_http(rest: &[String]) -> Result<()> {
    let mut bind = std::env::var("PALAZZO_BIND").unwrap_or_else(|_| "127.0.0.1:6334".into());
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
        backend = BACKEND,
        qdrant = %cfg.qdrant_url,
        collection = %cfg.collection,
        bind = %bind,
        mode = "streamable-http",
        "palazzo starting"
    );

    if let Err(e) = cfg.make_qdrant().ensure_indexes().await {
        tracing::warn!("ensure_indexes: {e:#}");
    }

    let ct = CancellationToken::new();
    let ct_child = ct.child_token();
    let mut http_config = StreamableHttpServerConfig::default().with_cancellation_token(ct_child);
    match std::env::var("PALAZZO_ALLOWED_HOSTS") {
        Ok(raw) if raw.trim() == "*" => {
            tracing::warn!(
                "PALAZZO_ALLOWED_HOSTS=* — DNS rebinding protection DISABLED. Ensure the listener is behind a trusted reverse proxy or firewall."
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
                "Host header allowlist defaults to localhost/127.0.0.1/::1 — set PALAZZO_ALLOWED_HOSTS to accept remote clients."
            );
        }
    }

    let service = StreamableHttpService::new(
        move || cfg.make_palace().map_err(std::io::Error::other),
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
