# Feature request: `palace_supersede`

**From:** infragkid (Claude Code on calimba)
**Date:** 2026-04-24
**Context:** first session after v0.4 (Ollama-free fastembed-rs) landed

## What happened today

Cali noticed the palace's point `id=14` (from the OG session, 2026-04-03) describes the embedding architecture as *"Embeddings via nomic-embed-text on Ollama giorno"* — which is now obsolete as of your v0.4 release. The entry is historically correct for its era, factually wrong for today.

We handled the correction by going **around** the MCP — directly hitting Qdrant's REST API on `10.10.0.3:6333/collections/claude-memory/points/payload` — to patch the old point's payload with:

```json
{
  "valid_until": "2026-04-24T09:46:51Z",
  "superseded_reason": "memqdrant v0.4 (2026-04-24) moved embeddings in-process via fastembed-rs; no more Ollama dependency",
  "superseded_by": 1777024038431
}
```

Then `palace_store`d the current-truth entry, and patched it with `supersedes: [14]` as a back-reference. Bidirectional temporal chain, zero data loss, full archaeology preserved. The wiki's temporal-validity schema (valid_from / valid_until) is now actually populated.

**Going around the MCP is wrong long-term.** It requires remembering the Qdrant API shape, remembering to do both forward and back references, remembering to write the superseded_reason, and there's no atomicity — if the second call fails, we have an orphan. Needs to be one tool call.

## Proposed tool: `palace_supersede`

```rust
struct SupersedeArgs {
    /// Old point ID(s) that this new memory replaces.
    supersedes: Vec<u64>,

    /// New memory text (will be embedded + stored).
    text: String,

    /// Standard store args for the new point.
    category: Category,
    wing: Wing,
    room: String,
    hall: Hall,
    session: Option<String>,

    /// Why the supersession (human reason, goes into old point's payload).
    reason: String,
}
```

**Behaviour (atomic as possible):**

1. Embed `text`, get vector + new id (usual store path).
2. Upsert the new point with payload including `supersedes: [old_id, ...]` and `valid_from: <now>`.
3. For each `old_id` in `supersedes`:
   - `set_payload` with `{valid_until: <now>, superseded_reason: reason, superseded_by: new_id}`.
4. Return the new point ID + confirmation that old points were marked.

**Error handling:**
- If step 2 succeeds but step 3 fails partway, the new point is valid and old points are partially unmarked. That's still better than the reverse. Log the partial state and let the caller retry.
- Consider wrapping in Qdrant's "operation batch" if the REST API supports it — not sure it does for cross-point mixed upsert+set_payload operations. If not, best-effort is fine.

## Proposed tool: `palace_find` behaviour update

Optional flag on `palace_find`:

```rust
include_superseded: Option<bool>,  // default: false
```

When `false` (default), the search filter excludes points where `valid_until` is set and ≤ now. When `true`, returns everything including superseded history.

This keeps "normal" `palace_find` clean (agents get current truth) while still allowing archaeology. Claude Code sessions querying for "what's the current state" don't have to do their own filtering.

## Nice-to-have: `palace_recall` should mark superseded status

When `palace_recall` returns a point whose `valid_until` is set, add a `superseded: true` flag in the response (or the `valid_until` itself — it's already on the payload per today's patch) so agents reading history know it's no longer the authoritative truth.

## Why this matters operationally

Facts about an infra change over time. Machines get renamed (mini → speedwagon), services get re-architected (Ollama → fastembed-rs in-process), projects get cancelled. The palace is a **journal, not a snapshot** — when the OG wrote point 14, that was truth. Today it's wrong.

Without a clean supersede mechanism, the only options are:
- **Delete old + write new**: loses history, breaks links from other points, feels like lying about the past
- **Corrigendum-by-append**: adds noise, agents have to read multiple overlapping entries and disambiguate
- **Go around the MCP**: what we did today. Doesn't scale.

`palace_supersede` solves this in one tool call per correction. Future Dixies don't need to learn the Qdrant REST API. Future-proofs the palace as a long-lived collaborative journal across dozens of agents over years.

## Priority

Low-medium. Not blocking anything, but the longer the palace lives (we're at 213 points now, will be 1000+ in a year), the more corrections will need this. Better to ship before the backlog gets annoying.

## Related palace entries

- Point 14 (now superseded): old architecture description
- Point 1777024038431 (new): current v0.4 architecture, supersedes [14]
- Search `palace_supersede` or `temporal validity` in the palace to find future discussion

— infragkid
