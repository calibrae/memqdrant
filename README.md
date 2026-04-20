# memqdrant

MCP server exposing a Qdrant-backed memory palace — typed wings, rooms, and halls instead of a generic blob store.

Cali's Rust daemon. Stdio only. No web UI, no auth, no drama.

## What it is

A single-binary Rust MCP server that:

- Speaks MCP over stdio (drop it into Claude Code, Cursor, or any MCP-compatible client)
- Embeds text with [Ollama](https://ollama.com/) (`nomic-embed-text` by default, 768-dim)
- Stores and retrieves points in [Qdrant](https://qdrant.tech/) with a **structured palace schema** (wing → room → hall)
- Detects near-duplicates before writing (cosine ≥ 0.95 + exact text match)
- Keeps an append-only JSONL write-ahead log for every mutation

It is intentionally opinionated. If you want a generic `(text, metadata)` store, use [`qdrant/mcp-server-qdrant`](https://github.com/qdrant/mcp-server-qdrant) — this project starts from that interface and replaces the untyped metadata with an enum-validated palace schema.

## Inspiration and prior art

- [`qdrant/mcp-server-qdrant`](https://github.com/qdrant/mcp-server-qdrant) — the official Qdrant MCP server (Python, FastMCP). memqdrant borrows its `store` / `find` tool shape, collection configuration, and filter-wrapping pattern.
- [`MemPalace/mempalace`](https://github.com/MemPalace/mempalace) — the wing / room / drawer terminology and the read-tool set (`status`, `taxonomy`, `check_duplicate`) are lifted from MemPalace's 29-tool MCP server. If you want a full palace with an agentic knowledge graph, cross-wing tunnels, and 96.6% R@5 retrieval on LongMemEval, go use MemPalace directly. memqdrant is the minimum-viable single-user flavour of the same idea, Rust-native, Qdrant-backed.

Neither upstream is vendored. Both are linked above; please follow and star their work.

## Tools

| Tool | What it does |
|---|---|
| `palace_store` | File a verbatim memory into a wing/room/hall. Returns a new point ID or the existing one on near-duplicate. |
| `palace_find` | Semantic search. Optional typed filters: `wing`, `category`, `room`, `hall`. |
| `palace_recall` | Fetch by explicit IDs. Cheap — no embedding. |
| `palace_status` | Total point count plus facet breakdown by wing, hall, category. |
| `palace_taxonomy` | Flat facet dump of wing / room / hall / category counts. |
| `palace_check_duplicate` | Probe whether candidate text already exists above the 0.95 cosine threshold. |

Input caps: 32 KB per text body, 100 IDs per recall batch, 1–20 results per find.

## Palace schema

Every point carries:

```
category:    person | career | technical | infrastructure | project-memory | vibe | project
wing:        projects | infrastructure | nexpublica | personal | career | vibe
room:        free-text (project or topic)
hall:        facts | events | decisions | discoveries | preferences
text:        the memory itself, verbatim
timestamp:   RFC3339 UTC
session:     optional conversation identifier
source_file: optional MD path when imported
```

IDs ≥ `1_000_000_000` are reserved for auto-generation (unix-millis). The palace schema enums are defined in [`src/schema.rs`](src/schema.rs).

## Config

All via environment variables:

| Variable | Default |
|---|---|
| `OLLAMA_URL` | `http://localhost:11434` |
| `OLLAMA_MODEL` | `nomic-embed-text` |
| `QDRANT_URL` | `http://localhost:6333` |
| `COLLECTION` | `claude-memory` |
| `MEMQDRANT_WAL` | `~/.memqdrant/wal.jsonl` |
| `RUST_LOG` | `memqdrant=info` |

Logging goes to **stderr only**. Stdout is the MCP transport — anything written there corrupts the JSON-RPC stream.

On startup, memqdrant creates keyword payload indexes on `wing`, `category`, `room`, `hall` if they're missing. Idempotent; required for the facet-based tools. Adding indexes to an existing collection is non-destructive — Qdrant builds them in place and existing points stay.

## Build

```
cargo build --release
```

Release profile is LTO-thin, single codegen unit, stripped. Current binary weighs in around 3.5 MB.

## Register with Claude Code

```
claude mcp add memqdrant -- /path/to/target/release/memqdrant
claude mcp list
```

Override any env var with `-e KEY=VALUE` before the `--`. Example:

```
claude mcp add memqdrant \
  -e COLLECTION=my-palace \
  -e OLLAMA_URL=http://localhost:11434 \
  -- /path/to/target/release/memqdrant
```

## Testing

End-to-end smoke test against a throwaway Qdrant collection:

```
cargo build --release
python3 scripts/smoke.py
```

It creates `memqdrant-test`, boots the binary, round-trips store / find / recall / status / check_duplicate / duplicate-skip / filtered find, and drops the collection. Fails loudly on any mismatch.

Requires live Qdrant and Ollama reachable at their configured URLs.

## Security notes

- Stdio transport, single-user threat model. No network listener, no auth.
- Dependencies audited with `cargo audit` on every build bump.
- Every write goes through a WAL (`~/.memqdrant/wal.jsonl` by default) with content previews truncated to 120 chars.
- `OLLAMA_URL` and `QDRANT_URL` are environment-controlled — anyone who can set env vars on this binary can already execute code as you, so the SSRF surface is accepted.
- MCP tool outputs (including stored text) are echoed back through the protocol; treat them as untrusted input to whatever LLM consumes them. This is a generic MCP concern, not specific to memqdrant.

## Non-goals

- Not a multi-user service.
- No HTTP/SSE transport. Stdio only.
- No web UI. Use the Qdrant dashboard for inspection.
- No knowledge graph, temporal validity windows, or agent diaries. If you want those, run [MemPalace](https://github.com/MemPalace/mempalace).
- No collection migrations. The palace schema is fixed; existing points embedded with `nomic-embed-text` must stay on that model.

## License

MIT — see [LICENSE](LICENSE).

## Credits

- [`qdrant/mcp-server-qdrant`](https://github.com/qdrant/mcp-server-qdrant) (Apache-2.0) for the MCP-over-Qdrant baseline.
- [`MemPalace/mempalace`](https://github.com/MemPalace/mempalace) (MIT) for the palace terminology, read-tool set, and the idea that verbatim beats summarised.
- The [MCP Rust SDK](https://github.com/modelcontextprotocol/rust-sdk) for the server harness.
