#!/usr/bin/env bash
# Quick tarball deployment for Fedora (no rpmbuild needed).
# Creates opencode-linux-x64.tar.gz with binary + config template.
#
# Usage:
#   ./scripts/build-tarball.sh
#   scp opencode-linux-x64.tar.gz user@fedora-host:/tmp/
#   ssh user@fedora-host
#   tar xzf /tmp/opencode-linux-x64.tar.gz -C ~/
#   ~/.opencode/bin/opencode --version
set -euo pipefail

BINARY="${OPENCODE_BIN:-$HOME/.opencode/bin/opencode}"
CONFIG_DIR="${HOME}/.config/opencode"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTDIR="${REPO_ROOT}/dist"

rm -rf "$OUTDIR"
mkdir -p "$OUTDIR/opencode/.opencode/bin"
mkdir -p "$OUTDIR/opencode/.config/opencode"

cp "$BINARY" "$OUTDIR/opencode/.opencode/bin/opencode"
chmod 755 "$OUTDIR/opencode/.opencode/bin/opencode"

if command -v jq &>/dev/null; then
    jq 'walk(if type == "object" then del(.api_key) else . end)' \
        "$CONFIG_DIR/opencode.json" > "$OUTDIR/opencode/.config/opencode/opencode.json"
else
    cp "$CONFIG_DIR/opencode.json" "$OUTDIR/opencode/.config/opencode/opencode.json"
fi

for dir in rules skills shared-rules; do
    [ -d "$CONFIG_DIR/$dir" ] && cp -a "$CONFIG_DIR/$dir" "$OUTDIR/opencode/.config/opencode/$dir"
done

# Install script
cat > "$OUTDIR/opencode/install.sh" << 'INSTALL_EOF'
#!/usr/bin/env bash
set -e
ROOT="$(cd "$(dirname "$0")" && pwd)"
echo "Installing opencode from $ROOT ..."

# Binary
mkdir -p ~/.opencode/bin
cp "$ROOT/.opencode/bin/opencode" ~/.opencode/bin/opencode
chmod 755 ~/.opencode/bin/opencode

# Config — don't overwrite existing
mkdir -p ~/.config/opencode
for item in opencode.json rules skills shared-rules; do
    if [ -e "$ROOT/.config/opencode/$item" ]; then
        cp -rn "$ROOT/.config/opencode/$item" ~/.config/opencode/ 2>/dev/null || true
    fi
done

echo ""
echo "Done. Run: ~/.opencode/bin/opencode --version"
echo "Add to PATH: export PATH=\"\$HOME/.opencode/bin:\$PATH\""
echo ""
echo "IMPORTANT: Set your API key in ~/.config/opencode/opencode.json"
INSTALL_EOF
chmod +x "$OUTDIR/opencode/install.sh"

cd "$OUTDIR"
tar czf opencode-linux-x64.tar.gz opencode
rm -rf opencode

echo "=== Done ==="
ls -lh "$OUTDIR/opencode-linux-x64.tar.gz"
echo ""
echo "Deploy: scp opencode-linux-x64.tar.gz user@host:/tmp/"
echo "Install: tar xzf /tmp/opencode-linux-x64.tar.gz && ./opencode/install.sh"
