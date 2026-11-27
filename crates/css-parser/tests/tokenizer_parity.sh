#!/bin/sh
# parity check: tokens of the test css files, typescript tokenizer vs rust tokenizer
# usage: sh crates/css-parser/tests/tokenizer_parity.sh <path to the typescript project>
set -eu
ts=${1:-../css}
cd "$(dirname "$0")/../../.."
rustup run stable cargo build --release -p rust-css-parser --bin tokens >/dev/null 2>&1
tmp=$(mktemp -d)
status=0
find "$ts/test" -name '*.css' | sort | while read -r f; do
    node scripts/parity/tokens.js "$(cd "$ts" && pwd)/dist" "$f" > "$tmp/ts.txt"
    ./target/release/tokens "$f" > "$tmp/rs.txt"
    if ! cmp -s "$tmp/ts.txt" "$tmp/rs.txt"; then echo "DIFF $f"; status=1; fi
done
rm -rf "$tmp"
exit $status
