#!/usr/bin/env bash
# Build opencode RPM from local installation.
# Requires: rpmbuild (dnf install rpm-build), jq (optional, for api_key stripping)
#
# Usage:
#   ./scripts/build-rpm.sh              # build from ~/.opencode/bin/opencode + ~/.config/opencode/
#   OPENCODE_BIN=/path/to/opencode ./scripts/build-rpm.sh   # custom binary path
set -euo pipefail

BINARY="${OPENCODE_BIN:-$HOME/.opencode/bin/opencode}"
CONFIG_DIR="${HOME}/.config/opencode"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RPM_DIR="${REPO_ROOT}/scripts/rpm"
SRC="${RPM_DIR}/SOURCES/opencode-linux-x64"

echo "=== opencode RPM builder ==="

# ── prerequisites ──
command -v rpmbuild &>/dev/null || { echo "Need rpmbuild: sudo dnf install rpm-build"; exit 1; }
[ -f "$BINARY" ] || { echo "Binary not found: $BINARY"; echo "Build it: cd packages/opencode && bun run build --single --skip-embed-web-ui"; exit 1; }
[ -f "$CONFIG_DIR/opencode.json" ] || { echo "Config not found: $CONFIG_DIR/opencode.json"; exit 1; }

# ── version ──
VERSION=$("$BINARY" --version 2>&1 | head -1 | sed 's/^v//' | tr '-' '_')
echo "Version: $VERSION"

# ── gather sources ──
rm -rf "$SRC"
mkdir -p "$SRC"
cp "$BINARY" "$SRC/opencode"

# Strip api_key from config (safe by default)
if command -v jq &>/dev/null; then
    jq 'walk(if type == "object" then del(.api_key) else . end)' \
        "$CONFIG_DIR/opencode.json" > "$SRC/opencode.json"
    echo "Config copied (api_key stripped via jq)"
else
    cp "$CONFIG_DIR/opencode.json" "$SRC/opencode.json"
    echo "Config copied (jq not found — verify no api_key in output)"
fi

for dir in rules skills shared-rules; do
    [ -d "$CONFIG_DIR/$dir" ] && cp -a "$CONFIG_DIR/$dir" "$SRC/$dir"
done

# ── tarball ──
cd "${RPM_DIR}/SOURCES"
tar czf opencode-linux-x64.tar.gz opencode-linux-x64
echo "Tarball: $(ls -lh opencode-linux-x64.tar.gz | awk '{print $5}')"

# ── build RPM ──
rpmbuild -bb \
    --define "_topdir ${RPM_DIR}" \
    --define "version ${VERSION}" \
    "${RPM_DIR}/SPECS/opencode.spec"

# ── result ──
RPM=$(find "${RPM_DIR}/RPMS" -name 'opencode-*.rpm' | sort -V | tail -1)
if [ -n "$RPM" ]; then
    echo ""
    echo "=== Done ==="
    ls -lh "$RPM"
    echo ""
    echo "Install:  sudo dnf install $RPM"
    echo "Transfer: scp $RPM user@fedora-host:/tmp/"
else
    echo "ERROR: RPM not found"
    exit 1
fi
