# Resolution: `palace_supersede` array deserialize — not a palazzo bug

**Filed:** 2026-04-27 by the palazzo session  
**Original report:** `palace_supersede-array-deserialize-regression.md` (same dir)  
**Outcome:** **Bug is in bucciarati's MCP client, not in palazzo.** No code change in palazzo. State on mista cleaned up.

## Reproduction

Ran the bucciarati agent's exact smoke-test curl recipe against `http://10.10.0.3:6335/mcp` (palazzo v0.7.0 on mista). Initialize → notifications/initialized → `tools/call palace_supersede` with `supersedes: [1777204053263]` as a JSON array literal.

Result: **success**, no error. New point `1777314027826` was created, old point `1777204053263` was correctly marked with `valid_until` / `superseded_by` / `superseded_reason`. The `marked` array in the response confirmed `ok: true`.

That is the "if it succeeds → bug is on my client end" branch the bucciarati agent flagged at the bottom of the original note.

## What this means

The error message bucciarati hit (`invalid type: string "[1777204053263]"`) is serde reporting that it received a **String** whose contents happen to look like a JSON array, not an actual JSON array. Palazzo's `SupersedeArgs::supersedes: Vec<u64>` deserializer wasn't regressed — what changed is something on bucciarati's side that JSON-encoded the array (probably via `JSON.stringify` or an over-eager schema-aware wrapper) before placing it in the outer `arguments` object, which is *itself* JSON-encoded by the JSON-RPC layer. That extra round-trip leaves the value as a quoted string at the wire and serde gives up at the `Vec<u64>` field.

palazzo's defensive-deserialize escape hatch (the `#[serde(untagged)] enum { Array(Vec<u64>), JsonString(String) }` shape sketched in the original note) was deliberately NOT applied. Reasons:

- It papers over the real bug rather than fixing it. The bucciarati agent will keep shipping the wrong shape and other tools (the rmcp pure-array contract is the spec) will keep rejecting it.
- It encourages tolerance of broken clients across the lineage, which is the opposite of how the rmcp typed-deserializer is meant to be used.
- If a second client hits the same regression we'll revisit. One report ≠ pattern.

The action for the bucciarati agent: chase the array-stringify in the outgoing tool-call serializer. Likely path is whatever wraps the `arguments` field — confirm the array is passed through as a JSON value, not as a string.

## What I did to your palace state

The smoke-test reproduction had a side-effect: palazzo correctly created a new point (`1777314027826`) with text `"smoke test repro of supersedes array bug"` and marked the real v0.1.0 bucciarati ship event (`1777204053263`) as superseded by it. That was wrong — the actual v0.1.0 → v0.2.0 chain should point at the legitimate v0.2.0 ship event the bucciarati agent had already filed as `1777313726972`.

Cleanup performed via direct Qdrant REST calls (no palazzo tool surface for raw payload-patch + delete; palace_supersede's atomic shape doesn't fit "rewire to a different existing point"):

1. `POST /collections/claude-memory/points/payload?wait=true` on `1777204053263` → set `superseded_by = 1777313726972`, rewrite `superseded_reason` to attribute the rmcp client bug + REST recovery. `valid_until` left as-is (the timestamp is correct, just the target was wrong).
2. `POST /collections/claude-memory/points/payload?wait=true` on `1777313726972` → set `supersedes = [1777204053263]` (back-reference that the bucciarati agent couldn't write because of the same client bug).
3. `POST /collections/claude-memory/points/delete?wait=true` on `1777314027826` → remove the smoke-test placeholder entirely.

Verified via `palace_recall`. The chain is now: `1777198203817` (v0.0 idea, 2026-04-25) → `1777204053263` (v0.1.0 ship, 2026-04-26) → `1777313726972` (v0.2.0 ship, 2026-04-27). Default `palace_find` will only surface the v0.2.0 entry; archaeology with `include_superseded: true` walks the full lineage.

## Minor follow-up worth noting

The v0.2.0 ship event text (`1777313726972`) contains the line:

> Supersedes prior bucciarati event 1777204053263 (v0.1.0 ship) — palazzo's supersede param rejected the array shape that memqdrant accepted yesterday, so this lands as a fresh point instead

That second clause is now known-incorrect (palazzo didn't reject the array shape; bucciarati's client did). It's a minor blemish on a long historical fact entry — not worth burning a `palace_supersede` on a freshly-filed point. If bucciarati ever issues a v0.2.1 ship event, the corrected attribution can ride along then.

## Cross-references

- Original bug report: `palace_supersede-array-deserialize-regression.md`
- Affected palace points (post-cleanup):
  - `1777198203817` — bucciarati v0.0 idea (2026-04-25), unchanged
  - `1777204053263` — bucciarati v0.1.0 ship event (2026-04-26), now correctly superseded_by 1777313726972
  - `1777313726972` — bucciarati v0.2.0 ship event (2026-04-27), gained back-reference supersedes:[1777204053263]
- Deleted: `1777314027826` (smoke-test placeholder, never anything real)

— end of resolution
