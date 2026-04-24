#!/usr/bin/env python3
"""Side-by-side top-K previews for a set of queries across the A/B endpoints.
Lets a human judge which system is picking the more relevant memory when
overlap disagrees.

Usage:
    python ab_eyeball.py \\
        --a http://mista:6335 --a-label ollama \\
        --b http://doppio:6336 --b-label fastembed \\
        --k 3 --queries <file-or-use-default>
"""
from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

# Reuse the MCP client shape from ab_benchmark.py but inline for standalone run.

DEFAULT_QUERIES = [
    "memqdrant MCP server deploy",
    "macOS Local Network Privacy TCC",
    "mista Qdrant localhost",
    "grytti deploy staging production",
    "Marianne HASS entity safety rules",
    "Cali personality ship fast no fluff",
    "WireGuard GRE tunnel edge",
    "DNS rebinding protection rmcp allowed hosts",
    "recency half life temporal filtering",
    "claude memory palace schema wing room hall",
]


def http_json(method, url, body=None, headers=None, timeout=60.0):
    h = {"Content-Type": "application/json"}
    if headers:
        h.update(headers)
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read().decode(), {k.lower(): v for k, v in r.headers.items()}
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(), {k.lower(): v for k, v in e.headers.items()}


class McpClient:
    def __init__(self, base_url):
        self.base = base_url.rstrip("/") + "/mcp"
        self.session_id = None
        self._id = 0
        self._init()

    def _post(self, body):
        h = {"Accept": "application/json, text/event-stream"}
        if self.session_id:
            h["mcp-session-id"] = self.session_id
        status, raw, hdrs = http_json("POST", self.base, body, headers=h)
        if status >= 400:
            raise RuntimeError(f"{body.get('method')}: {status} {raw[:300]}")
        if not self.session_id:
            self.session_id = hdrs.get("mcp-session-id")
        return raw

    def _call(self, method, params=None):
        self._id += 1
        body = {"jsonrpc": "2.0", "id": self._id, "method": method}
        if params is not None:
            body["params"] = params
        raw = self._post(body)
        payload = None
        for line in raw.splitlines():
            if line.startswith("data: ") and line[6:].strip().startswith("{"):
                payload = json.loads(line[6:])
        if payload is None:
            payload = json.loads(raw)
        if "error" in payload:
            raise RuntimeError(f"{method}: {payload['error']}")
        return payload.get("result")

    def _notify(self, method, params=None):
        body = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            body["params"] = params
        self._post(body)

    def _init(self):
        self._call(
            "initialize",
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "eyeball", "version": "1"},
            },
        )
        self._notify("notifications/initialized")

    def find(self, query, limit):
        r = self._call(
            "tools/call",
            {"name": "palace_find", "arguments": {"query": query, "limit": limit}},
        )
        return json.loads(r["content"][0]["text"])


def preview(text, n=80):
    t = " ".join(text.split())
    return t[:n] + ("…" if len(t) > n else "")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--a", required=True)
    ap.add_argument("--b", required=True)
    ap.add_argument("--a-label", default="A")
    ap.add_argument("--b-label", default="B")
    ap.add_argument("--k", type=int, default=3)
    ap.add_argument("--queries", help="Newline-separated queries file (default: builtin)")
    args = ap.parse_args()

    if args.queries:
        with open(args.queries) as f:
            qs = [line.strip() for line in f if line.strip() and not line.startswith("#")]
    else:
        qs = DEFAULT_QUERIES

    a = McpClient(args.a)
    b = McpClient(args.b)

    for q in qs:
        a_hits = a.find(q, args.k)
        b_hits = b.find(q, args.k)
        print(f"\n=== {q} ===")
        for side, label, hits in (("A", args.a_label, a_hits), ("B", args.b_label, b_hits)):
            print(f"\n  [{side} {label}]")
            for rank, h in enumerate(hits, 1):
                print(f"    {rank}. id={h['id']} score={h.get('score', 0):.3f} {preview(h['text'])}")


if __name__ == "__main__":
    main()
