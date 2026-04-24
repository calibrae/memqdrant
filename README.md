# memqdrant

MCP server exposing a Qdrant-backed memory palace — typed wings, rooms, and halls instead of a generic blob store.

Cali's Rust daemon. Stdio only. No web UI, no auth, no drama.

## What it is

A single-binary Rust MCP server that:

- Speaks MCP over **stdio** (run locally) or **Streamable HTTP** (run as a service)
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
| `palace_find` | Semantic search. Optional typed filters: `wing`, `category`, `room`, `hall`, `since`, `until`, `recency_half_life_days`. |
| `palace_recall` | Fetch by explicit IDs. Cheap — no embedding. |
| `palace_status` | Total point count plus facet breakdown by wing, hall, category. |
| `palace_taxonomy` | Flat facet dump of wing / room / hall / category counts. |
| `palace_check_duplicate` | Probe whether candidate text already exists above the 0.95 cosine threshold. |
| `palace_supersede` | Replace one or more existing memories with a corrected version. Marks the old points with `valid_until`, `superseded_by`, `superseded_reason`; default `palace_find` hides them. |

Input caps: 32 KB per text body, 100 IDs per recall batch, 1–20 results per find.

### Temporal filtering on `palace_find`

- `since` / `until` — inclusive RFC3339 second-precision UTC timestamps (e.g. `2026-04-01T00:00:00Z`). Filter memories by when they were stored. Bad format is rejected with an explicit error.
- `recency_half_life_days` (f64) — opt-in recency bias. When set, memqdrant fetches up to 4× the requested limit from Qdrant (capped at 80), re-ranks each hit by `score × exp(-age_days / half_life)`, then returns the top `limit`. Omit or pass `0` for pure cosine. Typical values: `30` (aggressive), `90` (moderate), `365` (gentle — a year-old memory gets half its raw score).

Both knobs work alongside the wing/category/room/hall filters — they compose.

### Temporal validity (`palace_supersede`)

Memories become wrong over time — infra gets renamed, services get rebuilt, decisions get reversed. `palace_supersede` lets you replace an old entry with a corrected one without losing the history:

- The new text is embedded and stored as a fresh point with `supersedes: [<old_id>, ...]`.
- Each old point gets marked with `valid_until = now`, `superseded_by = <new_id>`, and your free-text `reason`.
- Default `palace_find` excludes any point with a past `valid_until` — agents only see current truth. Pass `include_superseded: true` to surface the full timeline for archaeology.
- `palace_recall` always exposes `valid_until` / `superseded_by` / `superseded_reason` on the returned point, so you can tell current-from-stale at a glance.

The palace becomes a journal, not a snapshot — every correction is an append, never a delete.

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
| `MEMQDRANT_BIND` | `127.0.0.1:6334` (only used by `serve`) |
| `RUST_LOG` | `memqdrant=info` |

Logging goes to **stderr only**. Stdout is the MCP transport — anything written there corrupts the JSON-RPC stream.

On startup, memqdrant creates keyword payload indexes on `wing`, `category`, `room`, `hall` if they're missing. Idempotent; required for the facet-based tools. Adding indexes to an existing collection is non-destructive — Qdrant builds them in place and existing points stay.

## Embedding backends

memqdrant ships two backends behind mutually-exclusive cargo features. Pick one at build time.

| Feature | How it embeds | When to use |
|---|---|---|
| `ollama` (default) | HTTP calls to an Ollama server running `nomic-embed-text` | You already run Ollama on your LAN (or localhost). Ops simplicity — no native deps, binary stays small. |
| `fastembed` | Local ONNX inference of `nomic-embed-text-v1.5` via [`fastembed-rs`](https://github.com/Anush008/fastembed-rs) | You want memqdrant self-contained with zero external services. Trade a larger binary (~28 MB) and a ~137 MB one-time model download for one less daemon to keep alive. |

Select the variant via cargo features (release archives publish both per-platform):

```
cargo build --release                                          # ollama (default)
cargo build --release --no-default-features --features fastembed
```

The two backends produce 768-dim vectors that are numerically *close but not bitwise identical* — Ollama uses GGUF FP16, fastembed uses ONNX. Existing points embedded with one backend remain retrievable with the other; expect minor recency/ranking drift on borderline queries.

## Build

```
cargo build --release
```

Release profile is LTO-thin, single codegen unit, stripped. Binary ~8 MB with the `ollama` backend, ~28 MB with `fastembed` (static ONNX runtime).

## Running

memqdrant speaks two transports; pick one.

### stdio (local)

```
memqdrant
```

Stdout is the MCP channel — logging always goes to stderr. This is the default mode when the binary is invoked with no arguments. Best for single-user laptop use: no port to bind, no service to manage.

Register with Claude Code:

```
claude mcp add memqdrant -- /path/to/target/release/memqdrant
claude mcp list
```

Override env vars with `-e KEY=VALUE` before the `--`:

```
claude mcp add memqdrant \
  -e COLLECTION=my-palace \
  -e OLLAMA_URL=http://localhost:11434 \
  -- /path/to/target/release/memqdrant
```

### Streamable HTTP (service)

```
memqdrant serve --bind 0.0.0.0:6334
```

Serves MCP over Streamable HTTP at `POST /mcp`. Useful when the binary lives on a server co-located with Qdrant + Ollama, and your laptop (or multiple clients) connect over the network.

Register with Claude Code as a remote server:

```
claude mcp add --transport http memqdrant http://your-server:6334/mcp
```

Bind address can also be set via `MEMQDRANT_BIND`. Default is `127.0.0.1:6334`.

#### Deploy as a systemd service

The `deploy/` directory contains a hardened systemd unit, an env-file template, and an installer. On Debian / Ubuntu / any systemd host:

```
# On the target host, after placing the binary at e.g. ~/memqdrant
sudo ./deploy/install.sh ~/memqdrant
# Review /etc/memqdrant/env, then:
sudo systemctl enable --now memqdrant
```

The unit runs as a dedicated `memqdrant` user, drops all needless privileges (`ProtectSystem=strict`, `MemoryDenyWriteExecute=true`, `RestrictNamespaces=true`, etc.), and persists the WAL at `/var/lib/memqdrant/wal.jsonl`.

If you expose the service beyond a trusted LAN, put a reverse proxy with TLS + auth (e.g. nginx + basic auth, or an identity-aware proxy) in front of `:6334`. There is no built-in authentication — memqdrant assumes a trusted network.

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
