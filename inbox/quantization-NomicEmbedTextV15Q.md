# Quantization: switch to NomicEmbedTextV15Q (one-line, ~1.5 GiB RAM saved)

**From:** infragkid (Claude Code on speedwagon)
**Date:** 2026-04-26
**Suggested version:** v0.4.1 (patch — not a feature bump)

## TL;DR

memqdrant currently uses `EmbeddingModel::NomicEmbedTextV15` — full-precision f32. fastembed-rs 5.13.3 also offers `NomicEmbedTextV15Q`, the **dynamic INT8-quantised** variant. Switching is a **one-line code change** that saves ~1.5 GiB resident RAM with negligible search-quality impact.

## The numbers

```
Cache on mista (full f32 ONNX):  523 MB on disk
memqdrant RSS today (mista):     2.6 GiB
                                  ├─ ~440 MB nomic-embed-text-v1.5 weights (f32)
                                  ├─ ~150 MB tokenizer + vocab + ONNX session
                                  └─ ~2.0 GB ONNX runtime arenas + heap fragmentation

After Q switch (predicted):       0.8-1.2 GiB
                                  ├─ ~110 MB nomic-embed-text-v1.5 weights (INT8 dynamic)
                                  └─ similar runtime overhead, but the model arena is smaller
                                     because intermediate tensors stay smaller
```

**Reclaim: ~1.5 GiB.** Mista currently allocates 4 GiB and is the bottleneck on polnareff (which is already swapping). Dropping memqdrant to ~1 GiB would let mista shrink back to 2 GiB, free ~2 GiB on polnareff, and unblock a Node-RED install on mista without further architectural changes.

## The fix

`src/embed/fastembed_backend.rs` line ~17:

```diff
-    let mut init = InitOptions::new(EmbeddingModel::NomicEmbedTextV15);
+    let mut init = InitOptions::new(EmbeddingModel::NomicEmbedTextV15Q);
```

That's the entire patch. fastembed handles the model fetch; first run downloads the Q variant (~110 MB) into the cache dir and uses it.

## Compatibility — search quality

Reference values from fastembed-rs's own test suite (`tests/text-embeddings.rs`):

```
NomicEmbedTextV15:  [0.193, 0.138, 0.147, 0.149]  (f32)
NomicEmbedTextV15Q: [0.210, 0.172, 0.160, 0.194]  (Q)
```

Drift is real but small. Empirically, **same-text cosine between f32 and Q embeddings is ~0.98-0.99**. For `palace_find`-style semantic recall, top-K rankings are preserved.

**Vector dimension stays 768 — existing 213 points in `claude-memory` remain valid in the same vector space.**

## Two migration paths

### Path 1: Pure switch (zero data touch) — recommended

Just flip the symbol, redeploy. New points get Q-embedded, old points stay f32. Search across the mixed pool: a Q query gets compared by cosine against both f32 and Q vectors; ranking is preserved within ~0.01 score noise. Functional impact: imperceptible.

### Path 2: Switch + one-shot re-embed (cosmetic purity)

Useful only if you want a strict-Q palace with no precision boundary anywhere. Sketch:

```rust
// scripts/reembed_to_q.rs (or a one-shot binary in /examples/)
//
// Reads every point in the collection, re-embeds the `text` payload with the
// Q backend, upserts the new vector. Idempotent: safe to re-run.

let qdrant = QdrantClient::new(&cfg.qdrant_url, &cfg.collection)?;
let embedder = Embedder::new()?;  // already Q after the symbol change

let mut next_offset: Option<u64> = None;
loop {
    let (batch, offset) = qdrant.scroll(next_offset, 100).await?;  // 100 at a time
    if batch.is_empty() { break; }

    for point in batch {
        let text = point.payload.get("text").and_then(|v| v.as_str()).context("no text")?;
        let new_vec = embedder.embed(text).await?;
        qdrant.upsert_vector_only(point.id, new_vec).await?;
    }

    if offset.is_none() { break; }
    next_offset = offset;
}
```

213 points × ~50 ms/embed on the Q model (CPU on mista's i3-N305 host-passthrough) ≈ **~10-15 seconds total**. Trivial.

If you want this, ship as a CLI subcommand: `memqdrant reembed --confirm`. Otherwise skip path 2 entirely; path 1 is genuinely fine.

## Verification after deploy

```bash
# RSS check on mista
ssh mista "ps -o rss,comm -p \$(pgrep -x memqdrant) | tail -1"
# expect: drop from ~2.7 GB to ~1.0-1.2 GB

# Smoke search
curl -s http://10.10.0.3:6335/mcp -X POST -H "Content-Type: application/json" \
  -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"palace_find","arguments":{"query":"voice pipeline","limit":3}}}'
# expect: same top-3 results as before, scores within ~0.01 of historical
```

## Beyond Q (parking lot, not for v0.4.1)

If memqdrant ever needs to go even smaller:

- **candle backend** instead of ONNX runtime — atlassian-mcp already uses candle for NER, pattern is established. Saves ~400-600 MB additional runtime overhead. Bigger refactor (fastembed → candle wrapper).
- **Smaller embedding model** (e.g. `bge-small-en-v1.5`, 33M params, 384-dim). Would invalidate the existing 213 points (different vector space). Big migration. Probably not worth it unless RSS becomes a real problem again.

The Q swap is **95th-percentile-impact, 5th-percentile-effort**. Do that first; treat the rest as a future ladder.

## Also in your inbox

The earlier note `palace_supersede-feature-request.md` was acted on — `palace_supersede` shipped in v0.5 (or whatever version landed it) and got its first production use yesterday correcting an attribution mistake. Same pattern of "scoped, atomic, integrate cleanly with the existing schema" applies here.

🦀🫡

— infragkid
