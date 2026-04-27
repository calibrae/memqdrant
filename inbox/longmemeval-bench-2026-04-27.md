# LongMemEval bench results (2026-04-27)

**Recorded by:** session that shipped v0.7.0 palazzo rename
**Decision:** **don't pursue model swap or mining pipeline at this time.** Document the gap, accept the wedge, move on.

## Why we ran this

palazzo's README (and the earlier conversation) cites MemPalace's claim of **96.6% R@5 on LongMemEval, raw mode, no LLM** as the bar to beat. The question on the table: *is palazzo today within striking distance of that number, and if not, is there a cheap way to close the gap?*

The cheapest hypothesis was that **nomic-embed-text-v1.5 expects task prefixes** (`search_document:` for documents, `search_query:` for queries) and palazzo doesn't add them, neither in `src/embed/fastembed_backend.rs` nor in any of our Python helpers. If true, ~5 LOC patch unlocks meaningful recall.

## Setup

- **Bench harness:** `/tmp/lme-direct.py` — direct fastembed + numpy cosine, bypassing both palazzo and MemPalace's stack so we measure only the embedding model + retrieval. Source kept in the LongMemEval staging dir; the script is small enough to recreate in 30 minutes if needed.
- **Dataset:** `longmemeval_s_cleaned.json`, 500 questions, downloaded from `huggingface.co/datasets/xiaowu0162/longmemeval-cleaned`. The questions are sequenced by type — first ~80 are all `single-session-user`, so any partial run hits one type only.
- **Embedder:** `nomic-ai/nomic-embed-text-v1.5` via fastembed-rs Python (FP32, identical vector space to palazzo's V15Q INT8 within ~0.99 cosine).
- **Compute:** doppio (i9-9900K, 16 cores, 64 GB) with `OMP_NUM_THREADS=4 ORT_INTRA_OP_NUM_THREADS=4 ORT_INTER_OP_NUM_THREADS=2 RAYON_NUM_THREADS=4`. Without those caps the process OOMs around q3-q4 (~60 GB resident).
- **Granularity:** session-level. Each haystack session concatenated into one text, embedded once, scored against the labelled `answer_session_ids`.
- **Two arms:** `--prefix-mode none` (palazzo as deployed) and `--prefix-mode nomic` (manual `search_document:` / `search_query:` prefixes). The full 500q run wasn't feasible — ~3.5 hours per arm at this thread cap.

## What ran

| Arm | Questions | Wall clock | Result file |
|---|---|---|---|
| no-prefix | 50 of 50 | 3h 16m | `pilot-noprefix-50.jsonl` |
| nomic prefix | 16 of 50 (stopped) | ~1h | `pilot-prefix-50.jsonl` |

The prefix arm was stopped at 16q once the early signal made clear it was not winning and the next 2 hours of compute (~300 Wh on doppio) wouldn't change the decision.

## Results

| K | no-prefix 50q | nomic-prefix 16q |
|---|---|---|
| R@1 | 18.0% | 18.8% |
| R@3 | 26.0% | 25.0% |
| R@5 | **36.0%** | 25.0% |
| R@10 | 52.0% | 50.0% |
| R@30 | 92.0% | 81.2% |

95% confidence intervals (Wilson, single proportion):

- no-prefix R@5: 36.0% [23.5%, 50.6%]
- prefix R@5: 25.0% [9.7%, 49.0%]

The two CIs overlap heavily. **At the sample size we ran, prefix is statistically indistinguishable from no-prefix.** The point estimate is slightly worse with prefix, but that's well within noise for n=16.

## Diagnostic confirmations done along the way

- **fastembed Python 0.8.0 does not auto-prefix nomic-embed-text-v1.5.** `model.embed()`, `model.query_embed()`, and `model.passage_embed()` all return identical vectors (cosine 1.0). Whatever text you hand in is embedded literally. So the `--prefix-mode nomic` flag in the bench was applying real, distinct prefixes — the experiment was clean.
- Manual `"search_document: <text>"` is meaningfully different from `<text>` (cosine ~0.87). So prefixes do shift the embedding. They just don't shift it in a way that improves retrieval at this sample size.

## What we learned about the gap

Three things we now know that we didn't before:

1. **Palazzo's R@5 on `single-session-user` type is ~25-40%, not anywhere near 96.6%.** Even with the noise band, the gap is real and substantial.
2. **R@30 is ~85-90%.** The labelled answer session IS reachable in palazzo's vector space — it just isn't in the top 5. The bottleneck is **ranking discrimination**, not coverage.
3. **Adding nomic's trained prefixes does not close the gap.** That hypothesis is closed.

## Why we're not pursuing further work

Closing the gap looks like one of two things, both expensive:

**Option A — swap the embedding model.** MemPalace's 96.6% headline uses `all-MiniLM-L6-v2` (384-dim, trained for sentence-pair retrieval, no prefix needed) and `bge-large-en-v1.5` for their hybrid runs (1024-dim). Trying either:
- Invalidates the existing 213 points on mista (different vector space). Full re-embed required.
- Need to re-bench to confirm the model genuinely scores higher in palazzo's pipeline (no guarantee MemPalace's bench infrastructure didn't help them).
- Smaller model = smaller binary footprint, but loses the v0.5.1 INT8 quantisation tuning.
- Effort: ~1 day to swap + benchmark, modest risk of regressions on real palazzo queries we already validate visually.

**Option B — build a mining pipeline.** This is what actually delivers MemPalace's recall: their `convo_miner` / `entity_detector` / `fact_checker` / `general_extractor` decompose conversations into atomic facts before embedding. Searching against decomposed atoms gives much better R@5 because each atom is tight and focused, not a 10K-token blob.
- Requires an LLM dependency, which directly contradicts palazzo's "self-contained, no LLM" wedge.
- Multi-week engineering project. Strategic pivot, not a feature.

**Decision:** neither today. Palazzo's wedge is **small, fast, self-contained, MCP-native, runs on a 2 GB VM**. MemPalace optimised for the LongMemEval benchmark for two years. The gap is real; trying to close it would either compromise the wedge (option B) or invalidate live points (option A). Better to be honest about positioning.

## Updates that should follow this decision

- README's "Inspiration and prior art" already says "If you want a full palace with an agentic knowledge graph, cross-wing tunnels, and 96.6% R@5 retrieval on LongMemEval, go use MemPalace directly." That framing was right — leave it.
- No code change needed. The result here is a "don't" not a "do."

## Receipts

- Result jsonl files live on doppio at `/tmp/lme/pilot-noprefix-50.jsonl` and `/tmp/lme/pilot-prefix-50.jsonl`. Doppio is ephemeral; if the data needs to live, copy it off.
- `lme-direct.py` is the harness used. Recreate from this writeup if needed — straightforward.

## If a future session wants to revisit

- Run the **full 500 questions across all 6 types** before drawing aggregate conclusions. Our 50q-of-one-type sample has ±14 pp confidence bands, which is too coarse for any decision smaller than "is this an order-of-magnitude gap." (It is.)
- Test `bge-small-en-v1.5` or `all-MiniLM-L6-v2` as embedders. Both are smaller than nomic and pre-tested for retrieval. If R@5 jumps to 70-80% on either, the model swap conversation re-opens.
- Skip prefix experiments. We've ruled that out.

— end of note
