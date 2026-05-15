use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokio::sync::Mutex;

/// Local ONNX embedding backend backed by fastembed-rs and
/// `nomic-embed-text-v1.5` (768-dim). Model is fetched on first run into the
/// fastembed cache (default `~/.cache/fastembed`) — set `FASTEMBED_CACHE_DIR`
/// to override (for packaged / read-only deployments).
///
/// The ONNX runtime's CPU arena grows monotonically with cumulative inference
/// work and never shrinks — a long-lived process creeps from ~440 MB cold to
/// multiple GB. To bound that, the embedder optionally **recycles**: when
/// process RSS crosses `FASTEMBED_RECYCLE_RSS_MB` (default 1500, `0` disables),
/// a fresh `TextEmbedding` is built off-lock and swapped in. In-flight embeds
/// keep using the old session until the instant pointer-swap, so inserts and
/// reads are never stalled by a recycle.
#[derive(Clone)]
pub struct Embedder {
    inner: Arc<Mutex<TextEmbedding>>,
    /// RSS threshold in MiB above which the embedder recycles. 0 = disabled.
    recycle_rss_mb: u64,
    /// Single-flight guard — true while a recycle is in progress.
    recycling: Arc<AtomicBool>,
}

/// Build a fresh `TextEmbedding`. Blocking — loads the model from the on-disk
/// cache (or downloads it on the very first run). Call from `spawn_blocking`.
fn build_model() -> Result<TextEmbedding> {
    // INT8 dynamic-quantised variant of nomic-embed-text-v1.5. Output stays 768-dim
    // and same vector space as V15, so collections embedded with V15 stay searchable;
    // resident weights drop ~330 MB and ONNX Runtime arenas shrink with the smaller
    // intermediate tensors. Empirical same-text cosine vs V15: ~0.98-0.99.
    let mut init = InitOptions::new(EmbeddingModel::NomicEmbedTextV15Q);
    if let Ok(dir) = std::env::var("FASTEMBED_CACHE_DIR") {
        init = init.with_cache_dir(dir.into());
    }
    TextEmbedding::try_new(init).context("initialise fastembed NomicEmbedTextV15")
}

/// Current process resident set size in MiB. Linux-only (reads `/proc/self/statm`);
/// returns `None` elsewhere, which disables recycling on non-Linux dev boxes.
fn current_rss_mb() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let page_size = 4096u64; // Linux default; close enough for a threshold check.
    Some(resident_pages * page_size / (1024 * 1024))
}

impl Embedder {
    /// Construct a new embedder. Blocks to load / download the model — do this
    /// once at startup, not per request.
    pub fn new() -> Result<Self> {
        let model = build_model()?;
        let recycle_rss_mb: u64 = std::env::var("FASTEMBED_RECYCLE_RSS_MB")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1500);
        if recycle_rss_mb > 0 {
            tracing::info!(
                recycle_rss_mb,
                "fastembed embedder will recycle when RSS exceeds this threshold"
            );
        } else {
            tracing::info!("fastembed embedder recycling disabled (FASTEMBED_RECYCLE_RSS_MB=0)");
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(model)),
            recycle_rss_mb,
            recycling: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Embed a single string.
    ///
    /// ONNX inference is synchronous and CPU-bound — 50-200 ms+ per call. It
    /// MUST run on `spawn_blocking`, not the async worker: blocking a Tokio
    /// worker thread for that long stalls every other future it is responsible
    /// for, including the Streamable-HTTP MCP sessions' keep-alive and read
    /// tasks. That stall is exactly what made agents "lose MCP mid-insertion".
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // nomic-embed-text-v1.5 expects an instruction prefix for search-corpus
        // documents; without it embeddings are still usable but slightly off
        // from the model card's reference. The Ollama-side pipeline does NOT
        // add the prefix, so we omit it here too — the point is to stay as
        // close as possible to the existing vectors in `claude-memory`.
        let inner = self.inner.clone();
        let text = text.to_string();
        let out = tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            let mut out = guard
                .embed(vec![text.as_str()], None)
                .map_err(|e| anyhow!("fastembed: {e}"))?;
            out.pop()
                .ok_or_else(|| anyhow!("fastembed returned zero embeddings"))
        })
        .await
        .context("fastembed embed task join")?;
        self.maybe_recycle();
        out
    }

    /// Embed a batch of strings, sub-chunked so the ONNX runtime arenas stay
    /// bounded. Empirically a single batch of 256 long sequences peaks at ~6 GB
    /// RSS on an AVX2 worker — well past what a small VM can absorb. We chunk
    /// into groups of `FASTEMBED_BATCH_CHUNK` (default 16, override with the
    /// env var of the same name) and concatenate; per-call ONNX overhead is
    /// dwarfed by the matmul itself, so throughput barely moves.
    ///
    /// Runs on `spawn_blocking` for the same reason as [`Self::embed`] — a
    /// multi-chunk batch can occupy a CPU for seconds, which must never happen
    /// on an async worker thread.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let chunk_size: usize = std::env::var("FASTEMBED_BATCH_CHUNK")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(16);
        let inner = self.inner.clone();
        let texts = texts.to_vec();
        let out = tokio::task::spawn_blocking(move || {
            let mut guard = inner.blocking_lock();
            let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
            for chunk in texts.chunks(chunk_size) {
                let refs: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
                let mut vecs = guard
                    .embed(refs, None)
                    .map_err(|e| anyhow!("fastembed batch: {e}"))?;
                out.append(&mut vecs);
            }
            Ok(out)
        })
        .await
        .context("fastembed embed_batch task join")?;
        self.maybe_recycle();
        out
    }

    /// If process RSS has crossed the recycle threshold and no recycle is
    /// already running, spawn a background task that builds a fresh
    /// `TextEmbedding` off-lock and swaps it in. Cheap to call on every embed:
    /// it's an `/proc` read plus an atomic, and bails immediately when
    /// recycling is disabled or already in flight.
    fn maybe_recycle(&self) {
        if self.recycle_rss_mb == 0 {
            return;
        }
        let Some(rss) = current_rss_mb() else {
            return;
        };
        if rss < self.recycle_rss_mb {
            return;
        }
        // Single-flight: only the task that flips false -> true proceeds.
        if self
            .recycling
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let inner = self.inner.clone();
        let recycling = self.recycling.clone();
        let threshold = self.recycle_rss_mb;
        tokio::spawn(async move {
            tracing::info!(
                rss_mb = rss,
                threshold_mb = threshold,
                "fastembed RSS over threshold — recycling embedder"
            );
            // Build the replacement WITHOUT holding the lock — embeds keep
            // running against the old session for the ~0.5-1s this takes.
            match tokio::task::spawn_blocking(build_model).await {
                Ok(Ok(fresh)) => {
                    // Swap holds the lock only for the assignment — it queues
                    // behind at most one in-flight embed, then returns instantly.
                    {
                        let mut guard = inner.lock().await;
                        *guard = fresh;
                    }
                    let after = current_rss_mb().unwrap_or(0);
                    tracing::info!(rss_mb = after, "fastembed embedder recycled");
                }
                Ok(Err(e)) => {
                    tracing::error!("fastembed recycle: rebuild failed: {e:#}");
                }
                Err(e) => {
                    tracing::error!("fastembed recycle: rebuild task join failed: {e}");
                }
            }
            recycling.store(false, Ordering::Release);
        });
    }
}
