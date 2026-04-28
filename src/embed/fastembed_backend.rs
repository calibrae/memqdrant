use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokio::sync::Mutex;

/// Local ONNX embedding backend backed by fastembed-rs and
/// `nomic-embed-text-v1.5` (768-dim). Model is fetched on first run into the
/// fastembed cache (default `~/.cache/fastembed`) — set `FASTEMBED_CACHE_DIR`
/// to override (for packaged / read-only deployments).
#[derive(Clone)]
pub struct Embedder {
    inner: Arc<Mutex<TextEmbedding>>,
}

impl Embedder {
    /// Construct a new embedder. Blocks to load / download the model — do this
    /// once at startup, not per request.
    pub fn new() -> Result<Self> {
        // INT8 dynamic-quantised variant of nomic-embed-text-v1.5. Output stays 768-dim
        // and same vector space as V15, so collections embedded with V15 stay searchable;
        // resident weights drop ~330 MB and ONNX Runtime arenas shrink with the smaller
        // intermediate tensors. Empirical same-text cosine vs V15: ~0.98-0.99.
        let mut init = InitOptions::new(EmbeddingModel::NomicEmbedTextV15Q);
        if let Ok(dir) = std::env::var("FASTEMBED_CACHE_DIR") {
            init = init.with_cache_dir(dir.into());
        }
        let model =
            TextEmbedding::try_new(init).context("initialise fastembed NomicEmbedTextV15")?;
        Ok(Self {
            inner: Arc::new(Mutex::new(model)),
        })
    }

    /// Embed a single string. ONNX inference is synchronous and CPU-bound; we
    /// hold a Mutex and run on the current Tokio worker (short enough not to
    /// warrant spawn_blocking for our traffic volume).
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // nomic-embed-text-v1.5 expects an instruction prefix for search-corpus
        // documents; without it embeddings are still usable but slightly off
        // from the model card's reference. The Ollama-side pipeline does NOT
        // add the prefix, so we omit it here too — the point is to stay as
        // close as possible to the existing vectors in `claude-memory`.
        let mut guard = self.inner.lock().await;
        let mut out = guard
            .embed(vec![text], None)
            .map_err(|e| anyhow!("fastembed: {e}"))?;
        out.pop()
            .ok_or_else(|| anyhow!("fastembed returned zero embeddings"))
    }

    /// Embed a batch of strings in one ONNX inference pass. Lets fastembed/ort
    /// pack inputs into one matmul instead of paying per-call overhead — the
    /// per-item cost drops from ~80 ms to ~25 ms amortised on AVX2 hardware.
    /// Used by `palace_store_batch`.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self.inner.lock().await;
        let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        guard
            .embed(refs, None)
            .map_err(|e| anyhow!("fastembed batch: {e}"))
    }
}
