# Contributing to memqdrant

Small project, low ceremony. Open an issue before a large PR so we don't both waste time.

## Scope

memqdrant is deliberately minimal: a single-user, stdio-only, Rust MCP server over Qdrant + Ollama.

Good PRs:

- Bug fixes, failing-test reproductions
- New palace tools that fit the existing schema (wing / room / hall)
- Performance or startup-time improvements
- Docs, examples, additional MCP client configs
- Better error messages surfaced through MCP responses
- Tightening the typed filter surface on `palace_find`

Out of scope (please go build this on top of [MemPalace](https://github.com/MemPalace/mempalace) instead):

- Multi-user auth, SSO, RBAC
- HTTP/SSE transports
- Web UI
- Knowledge graph, tunnels, temporal validity windows
- LLM-based rerankers, extractors, summarisers
- Swapping the embedding model on an existing collection (see "Do not re-embed" below)

## Dev loop

```
cargo build
cargo check
cargo clippy --all-targets
cargo fmt
cargo audit
```

End-to-end smoke test (requires live Qdrant + Ollama):

```
cargo build --release
python3 scripts/smoke.py
```

`scripts/smoke.py` creates a throwaway `memqdrant-test` collection, round-trips every tool, then drops it. Do not aim it at `claude-memory` or any production collection. Ever.

## Style

- Rust 2024, current stable toolchain.
- Match the existing code: terse, no rocket emojis, no ceremonial comments. Comments explain *why*, not *what*.
- Errors via `anyhow` at boundaries, typed via `thiserror` only if variants carry meaningful data.
- Log to stderr via `tracing`. **Never** write to stdout — it's the MCP transport.
- Feature flags only if there's a real alternative implementation behind them.

## Qdrant & Ollama discipline

- **Do not re-embed existing points.** The default `claude-memory` collection is embedded with `nomic-embed-text` (768-dim). Changing models requires a new collection and a reindex of the source material — not a schema migration.
- **Do not swap the dimension or distance** on an existing collection.
- New payload fields are fine — add them to `Payload` in `src/schema.rs` and to `ensure_indexes` in `src/qdrant.rs` if they need to be filterable.
- Auto-generated IDs must stay ≥ `1_000_000_000` to avoid collision with curated and bulk-import ranges.

## PR checklist

- [ ] `cargo build --release` passes
- [ ] `cargo clippy --all-targets` clean
- [ ] `cargo fmt` applied
- [ ] `cargo audit` clean (0 vulns, warnings justified in the PR body)
- [ ] `python3 scripts/smoke.py` passes end-to-end
- [ ] No writes to stdout from any code path
- [ ] No new dependency without a one-line justification in the PR description

## Commit messages

Descriptive, concise, imperative mood. No AI co-author lines.

## License

By submitting a patch you agree that your contribution is licensed under the MIT License (see [LICENSE](LICENSE)).
