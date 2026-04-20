#!/usr/bin/env bash
# Install memqdrant as a systemd service. Idempotent.
# Usage:  sudo ./install.sh /path/to/memqdrant
# Expects the binary to be passed in. Places it at /opt/memqdrant/bin/memqdrant,
# installs the unit file, creates a system user, and reloads systemd.
# Does NOT start the service — review /etc/memqdrant/env first, then:
#   sudo systemctl enable --now memqdrant

set -euo pipefail

if [[ $EUID -ne 0 ]]; then
    echo "run as root" >&2
    exit 1
fi

BIN="${1:-}"
if [[ -z "$BIN" || ! -x "$BIN" ]]; then
    echo "usage: sudo $0 <path-to-memqdrant-binary>" >&2
    exit 2
fi

HERE="$(cd "$(dirname "$0")" && pwd)"

# System user & dirs
if ! id memqdrant >/dev/null 2>&1; then
    useradd --system --shell /usr/sbin/nologin --home-dir /var/lib/memqdrant --create-home memqdrant
fi
install -d -o memqdrant -g memqdrant -m 0750 /var/lib/memqdrant
install -d -m 0755 /opt/memqdrant /opt/memqdrant/bin /etc/memqdrant

# Binary
install -m 0755 "$BIN" /opt/memqdrant/bin/memqdrant

# Env file: copy example if /etc/memqdrant/env doesn't exist; never overwrite.
if [[ ! -f /etc/memqdrant/env ]]; then
    install -m 0640 -o root -g memqdrant "$HERE/env.example" /etc/memqdrant/env
    echo "wrote default /etc/memqdrant/env — review before starting"
fi

# systemd unit
install -m 0644 "$HERE/memqdrant.service" /etc/systemd/system/memqdrant.service
systemctl daemon-reload

cat <<'EOF'

memqdrant installed. Next steps:
  1. Edit /etc/memqdrant/env to taste.
  2. sudo systemctl enable --now memqdrant
  3. sudo systemctl status memqdrant
  4. curl -sS -X POST http://localhost:6334/mcp ... (see README for full MCP handshake)
EOF
