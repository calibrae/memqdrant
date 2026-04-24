#!/usr/bin/env python3
"""A/B benchmark: run the same query set against two memqdrant endpoints
(different backends, aligned collections) and compare top-K recall.

Outputs per-query overlap and rank-correlation, plus an aggregate verdict.
Accept criterion (soft): Overlap@5 ≥ 0.8 across the set.

Usage:
    python ab_benchmark.py \\
        --a http://mista.calii.lan:6335 --a-label ollama \\
        --b http://doppio.calii.lan:6336 --b-label fastembed \\
        [--k 5] [--queries scripts/ab_queries.txt]
"""
from __future__ import annotations

import argparse
import json
import sys
import time
import urllib.error
import urllib.request

DEFAULT_QUERIES = [
    "memqdrant MCP server deploy",
    "mista Qdrant localhost",
    "giorno Ollama nomic-embed-text",
    "macOS Local Network Privacy TCC",
    "sshd-session denied bundle ID",
    "SSH stdio vs HTTP transport MCP",
    "Marianne HASS entity safety rules",
    "Lou torrents qBittorrent narancia",
    "grytti deploy staging production",
    "hermytt terminal WASM",
    "ytt family inbox system",
    "claude memory palace schema wing room hall",
    "Vault secrets mista file backend",
    "Gitea act-runner-rs signed",
    "Apple Developer ID codesign",
    "Fedora 43 doppio hypervisor",
    "libvirt polnareff VM provisioning",
    "DNS rebinding protection rmcp allowed hosts",
    "fucina bundle ID network privacy solved",
    "WireGuard GRE tunnel edge",
    "Cali personality ship fast no fluff",
    "Rust 2024 edition cargo clippy warnings",
    "Ollama nomic-embed-text 768 dimensions cosine",
    "recency half life temporal filtering",
    "GitHub Actions release workflow tag",
]


def http_json(method: str, url: str, body: dict | None = None, headers: dict | None = None, timeout: float = 30.0):
    h = {"Content-Type": "application/json"}
    if headers:
        h.update(headers)
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            raw = r.read().decode()
            resp_hdrs = {k.lower(): v for k, v in r.headers.items()}
            return r.status, raw, resp_hdrs
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(), {k.lower(): v for k, v in e.headers.items()}


class McpClient:
    """Minimal MCP over Streamable HTTP client — initialize + tools/call only."""

    def __init__(self, base_url: str):
        self.base = base_url.rstrip("/") + "/mcp"
        self.session_id: str | None = None
        self._id = 0
        self._initialize()

    def _post(self, body: dict):
        h = {"Accept": "application/json, text/event-stream"}
        if self.session_id:
            h["mcp-session-id"] = self.session_id
        status, raw, hdrs = http_json("POST", self.base, body, headers=h, timeout=120.0)
        if status >= 400:
            raise RuntimeError(f"MCP {body.get('method')} failed: {status} {raw[:300]}")
        sid = hdrs.get("mcp-session-id")
        if sid and not self.session_id:
            self.session_id = sid
        return raw

    def _request(self, method: str, params: dict | None = None):
        self._id += 1
        body: dict = {"jsonrpc": "2.0", "id": self._id, "method": method}
        if params is not None:
            body["params"] = params
        raw = self._post(body)
        # Server may return SSE frames; pick the last `data: ` JSON line.
        payload = None
        for line in raw.splitlines():
            if line.startswith("data: ") and line[6:].strip().startswith("{"):
                payload = json.loads(line[6:])
        if payload is None:
            payload = json.loads(raw)
        if "error" in payload:
            raise RuntimeError(f"{method} error: {payload['error']}")
        return payload.get("result")

    def _notify(self, method: str, params: dict | None = None):
        body: dict = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            body["params"] = params
        self._post(body)

    def _initialize(self):
        self._request(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "ab-bench", "version": "1"},
            },
        )
        self._notify("notifications/initialized")

    def find(self, query: str, limit: int) -> list[dict]:
        result = self._request(
            "tools/call",
            {"name": "palace_find", "arguments": {"query": query, "limit": limit}},
        )
        content = result["content"][0]["text"]
        return json.loads(content)


def overlap_at_k(a: list[int], b: list[int]) -> float:
    if not a or not b:
        return 0.0
    return len(set(a) & set(b)) / float(len(a))


def spearman_on_intersection(a_ranked: list[int], b_ranked: list[int]) -> float | None:
    """Spearman rank correlation computed only on IDs present in both lists.
    Returns None if fewer than 2 common IDs."""
    common = [x for x in a_ranked if x in b_ranked]
    if len(common) < 2:
        return None
    a_rank = {i: r for r, i in enumerate(a_ranked)}
    b_rank = {i: r for r, i in enumerate(b_ranked)}
    n = len(common)
    d2 = sum((a_rank[i] - b_rank[i]) ** 2 for i in common)
    return 1.0 - (6.0 * d2) / (n * (n * n - 1))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--a", required=True, help="Endpoint A URL (e.g. http://mista:6335)")
    ap.add_argument("--b", required=True, help="Endpoint B URL (e.g. http://doppio:6336)")
    ap.add_argument("--a-label", default="A")
    ap.add_argument("--b-label", default="B")
    ap.add_argument("--k", type=int, default=5)
    ap.add_argument("--queries", help="Path to newline-separated query file (blank lines + #comments skipped)")
    args = ap.parse_args()

    if args.queries:
        with open(args.queries) as f:
            qs = [line.strip() for line in f if line.strip() and not line.startswith("#")]
    else:
        qs = DEFAULT_QUERIES

    print(f"A ({args.a_label}): {args.a}")
    print(f"B ({args.b_label}): {args.b}")
    print(f"K: {args.k}, queries: {len(qs)}\n")

    print(f"connecting…")
    a = McpClient(args.a)
    b = McpClient(args.b)

    overlaps: list[float] = []
    corrs: list[float] = []
    header = f"{'overlap@K':>10}  {'spearman':>9}  query"
    print(header)
    print("-" * len(header))

    for q in qs:
        a_hits = a.find(q, args.k)
        b_hits = b.find(q, args.k)
        a_ids = [h["id"] for h in a_hits]
        b_ids = [h["id"] for h in b_hits]
        ov = overlap_at_k(a_ids, b_ids)
        sp = spearman_on_intersection(a_ids, b_ids)
        overlaps.append(ov)
        if sp is not None:
            corrs.append(sp)
        sp_str = f"{sp:+.2f}" if sp is not None else "  -  "
        print(f"{ov:>10.2f}  {sp_str:>9}  {q}")

    avg_ov = sum(overlaps) / len(overlaps)
    avg_sp = (sum(corrs) / len(corrs)) if corrs else float("nan")
    print()
    print(f"mean Overlap@{args.k}: {avg_ov:.3f}")
    print(f"mean Spearman (on overlapping ids): {avg_sp:.3f}")
    pct = sum(1 for v in overlaps if v >= 0.8) / len(overlaps)
    print(f"queries with overlap ≥ 0.8: {pct:.0%} ({sum(1 for v in overlaps if v >= 0.8)}/{len(overlaps)})")
    verdict = "PASS" if avg_ov >= 0.8 else "REVIEW"
    print(f"\nverdict: {verdict}")


if __name__ == "__main__":
    main()
