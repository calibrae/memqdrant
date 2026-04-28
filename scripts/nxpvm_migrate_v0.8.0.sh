#!/usr/bin/env bash
# Migrate nxpvm: memqdrant v0.4.0  →  palazzo v0.8.0 (fastembed).
#
# Run this on nxpvm itself (sudo capable user). Idempotent up to the point of
# stopping memqdrant — once the rename is in place we don't roll back without
# manual cleanup.
#
# Pre-flight assumptions:
#   - /opt/memqdrant/bin/memqdrant exists (current install)
#   - /etc/memqdrant/env exists (env vars)
#   - /var/lib/memqdrant has wal.jsonl and any state we care to preserve
#   - systemd unit memqdrant.service is running
#   - Qdrant is reachable from this host (used by both versions, no migration)
set -euo pipefail

VER=v0.8.0
ARTIFACT=palazzo-fastembed-${VER}-x86_64-unknown-linux-gnu.tar.gz
SHA256=59639b6501debd31b93eafdc3aae8fe413103156ec5847e0ca985c8cdbfe0f21
URL=https://github.com/calibrae/palazzo/releases/download/${VER}/${ARTIFACT}

WORK=$(mktemp -d /tmp/palazzo-migrate.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

echo "==> 1. download + verify artifact"
curl -fsSL "$URL"          -o "$WORK/$ARTIFACT"
curl -fsSL "$URL.sha256"   -o "$WORK/$ARTIFACT.sha256"
( cd "$WORK" && echo "$SHA256  $ARTIFACT" | sha256sum -c - )
tar -xzf "$WORK/$ARTIFACT" -C "$WORK"
BIN=$(find "$WORK" -maxdepth 3 -type f -name palazzo | head -1)
[ -x "$BIN" ] || { echo "binary not found in archive"; exit 1; }
"$BIN" --version 2>&1 || true

echo "==> 2. create palazzo system user (idempotent)"
id palazzo >/dev/null 2>&1 || sudo useradd --system --home-dir /var/lib/palazzo --shell /usr/sbin/nologin palazzo

echo "==> 3. stop and disable memqdrant"
sudo systemctl stop memqdrant.service || true
sudo systemctl disable memqdrant.service || true

echo "==> 4. clone state directories memqdrant -> palazzo"
for src in /opt/memqdrant /etc/memqdrant /var/lib/memqdrant; do
  dst=${src/memqdrant/palazzo}
  if [ -e "$src" ] && [ ! -e "$dst" ]; then
    sudo cp -a "$src" "$dst"
    echo "    cloned $src -> $dst"
  else
    echo "    skip $src (missing or dst exists)"
  fi
done

echo "==> 5. install new binary into /opt/palazzo/bin"
sudo install -d /opt/palazzo/bin
sudo install -m 0755 "$BIN" /opt/palazzo/bin/palazzo

echo "==> 6. rewrite env file (MEMQDRANT_* -> PALAZZO_*, paths)"
if [ -f /etc/palazzo/env ]; then
  sudo sed -i \
    -e 's|MEMQDRANT_|PALAZZO_|g' \
    -e 's|memqdrant=info|palazzo=info|g' \
    -e 's|/var/lib/memqdrant|/var/lib/palazzo|g' \
    -e 's|/opt/memqdrant|/opt/palazzo|g' \
    -e 's|/etc/memqdrant|/etc/palazzo|g' \
    /etc/palazzo/env
  echo "--- /etc/palazzo/env ---"
  sudo cat /etc/palazzo/env
fi

echo "==> 7. fix ownership"
sudo chown -R palazzo:palazzo /var/lib/palazzo /etc/palazzo

echo "==> 8. install systemd unit"
sudo tee /etc/systemd/system/palazzo.service >/dev/null <<'UNIT'
[Unit]
Description=palazzo MCP memory palace
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=palazzo
Group=palazzo
EnvironmentFile=/etc/palazzo/env
ExecStart=/opt/palazzo/bin/palazzo serve
Restart=on-failure
RestartSec=3

# hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictSUIDSGID=yes
LockPersonality=yes
# ONNX runtime needs writable+executable pages, so MemoryDenyWriteExecute is off.
MemoryDenyWriteExecute=no
RestrictRealtime=yes
RestrictNamespaces=yes
SystemCallArchitectures=native
ReadWritePaths=/var/lib/palazzo

[Install]
WantedBy=multi-user.target
UNIT

echo "==> 9. start palazzo"
sudo systemctl daemon-reload
sudo systemctl enable --now palazzo.service
sleep 2
sudo systemctl status palazzo.service --no-pager | head -20 || true

echo "==> 10. health probe"
PORT=$(sudo grep -E '^PALAZZO_BIND|^PALAZZO_PORT|^PORT=' /etc/palazzo/env | head -1 | cut -d= -f2 | tr -d '"' | sed 's|.*:||')
PORT=${PORT:-8089}
sleep 2
curl -fsS "http://127.0.0.1:${PORT}/health" 2>/dev/null && echo "  health ok" || echo "  (no /health endpoint — check journalctl -u palazzo -n 50)"

echo
echo "==> done. Devs reregister with:"
echo "    claude mcp add --transport http palazzo http://10.17.0.142:${PORT}/mcp"
echo
echo "Old memqdrant tree left in place at /opt/memqdrant /etc/memqdrant /var/lib/memqdrant — remove once verified."
