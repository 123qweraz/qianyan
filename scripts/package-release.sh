#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."

NAME="qianyan-ime-linux-x86_64"
VERSION=$(date +%Y%m%d_%H%M%S)
OUT="release/${NAME}_${VERSION}"
mkdir -p "$OUT"

cargo build --release

cp target/release/qianyan-ime "$OUT/"
cp target/release/qianyan-ime-gui "$OUT/"
cp -r data dicts configs sounds picture "$OUT/"
cp qianyan-ime.desktop "$OUT/"
cp scripts/install/install.sh "$OUT/"
strip "$OUT/qianyan-ime" "$OUT/qianyan-ime-gui" 2>/dev/null || true

cd release
tar czf "${NAME}_${VERSION}.tar.gz" "$(basename "$OUT")"
rm -rf "$(basename "$OUT")"

echo "=== Done ==="
echo "release/${NAME}_${VERSION}.tar.gz"
