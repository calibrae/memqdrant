//! Per-tool token baselines for `mcp-gain`.
//!
//! Each entry is the *estimated* number of tokens an agent would have spent if
//! it had to do the same job without memqdrant — typically some flavour of
//! `ssh mista 'curl http://127.0.0.1:6333/... ' | jq …` plus, for queries, a
//! second roundtrip to embed the query text via Ollama or fastembed.
//!
//! The numbers are deliberately conservative on the first pass; recalibrate
//! after a few weeks of `usage.jsonl` data once we know the real average
//! response sizes per tool.
//!
//! Baseline assumptions used to ground these estimates:
//!
//!   * SSH banner + bash command echo + jq pretty-printed Qdrant payload
//!     leaks ~250 tokens of chrome alone for any read.
//!   * For `palace_find` / `palace_check_duplicate`, the agent would also have
//!     to embed the query text — a separate `curl http://giorno:11434/api/embed`
//!     dance with a 768-element float array round-trip in the response. That
//!     alone is ~1500 tokens of agent context if it ever lands in the chat.
//!   * Multi-result reads (find, recall) carry the per-record cost of raw
//!     Qdrant point JSON (vectors included by default) — order of magnitude
//!     larger than the trimmed `Memory` we hand back.
//!
//! Source tag is bumped whenever this table is recalibrated, so old log
//! entries can be filtered against the right generation if we ever do a
//! historical re-render.

pub const SOURCE: &str = "estimate@v1";

pub const BASELINES: &[(&str, u32)] = &[
    // Read-side: embed the query, hit Qdrant, slice JSON, format.
    ("palace_find", 1800),
    ("palace_check_duplicate", 1700),
    // Read-side without embedding: still SSH + curl + jq.
    ("palace_recall", 600),
    ("palace_status", 350),
    ("palace_taxonomy", 450),
    // Write-side: SSH + Ollama embed + Qdrant POST + ack noise.
    ("palace_store", 1900),
    ("palace_supersede", 2200),
    // Self-call: zero by definition (the gain summary is the gain).
    ("palace_gain", 0),
];

pub fn header() -> String {
    format!("palazzo gain — {SOURCE}")
}
