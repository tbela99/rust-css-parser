#!/bin/sh
# Coverage of the rust code: sh scripts/coverage.sh [--html] [--lcov] [--json] [--fail-under-lines N]
#
# the library is mostly tested by the specs (test/specs), through the N-API addon: the addon is built with the coverage
# instrumentation (cargo-llvm-cov), the specs run against it, then the profiles of the specs and of the rust tests
# (cargo test) are merged into one report.
#
# - the instrumented build is in target/coverage (the release build of target/ is not modified)
# - the specs load the instrumented addon from target/coverage/napi (the `coverage` condition of the imports of
#   package.json): extensions/css-parser-napi/dist is not modified
# - reports: a summary in the terminal, and with the options
#   --html: target/coverage/html/index.html
#   --lcov: target/coverage/lcov.info (codecov, coveralls, IDE plugins)
#   --json: target/coverage/coverage-summary.json (the totals, for a badge)
#   --fail-under-lines N: fails when less than N% of the lines are covered
#
# requires cargo-llvm-cov and the llvm tools of the stable toolchain:
#   rustup component add llvm-tools-preview --toolchain stable
#   cargo install cargo-llvm-cov --locked
set -eu
cd "$(dirname "$0")/.."

html=0
lcov=0
json=0
fail_under=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        --html) html=1 ;;
        --lcov) lcov=1 ;;
        --json) json=1 ;;
        --fail-under-lines) shift; fail_under="$1" ;;
        *) echo "unknown option: $1" >&2; exit 1 ;;
    esac
    shift
done

# cargo-llvm-cov is installed in ~/.cargo/bin
PATH="$HOME/.cargo/bin:$PATH"
export PATH

# cargo of the stable toolchain when rustup is installed (its llvm tools), the cargo of the PATH otherwise
cargo() {
    if command -v rustup > /dev/null 2>&1; then
        rustup run stable cargo "$@"
    else
        command cargo "$@"
    fi
}

if ! cargo llvm-cov --version > /dev/null 2>&1; then
    echo 'cargo-llvm-cov is missing: cargo install cargo-llvm-cov --locked' >&2
    exit 1
fi

root=$(pwd)
CARGO_TARGET_DIR="$root/target/coverage"
export CARGO_TARGET_DIR

# the environment of the instrumented build: RUSTFLAGS (through a rustc wrapper) and LLVM_PROFILE_FILE, the profiles
# written by the processes that load the instrumented code (cargo test, node)
eval "$(cargo llvm-cov show-env --sh 2> /dev/null)"
cargo llvm-cov clean --workspace

# the rust tests
cargo test --release --workspace

# the instrumented N-API addon, and the javascript modules of the extension (the build of build-bindings.sh)
cargo build --release -p css-parser-napi
case "$(uname -s)" in
    Darwin) library=libcss_parser_napi.dylib ;;
    Linux) library=libcss_parser_napi.so ;;
    *) echo 'the N-API build supports macOS and Linux.' >&2; exit 1 ;;
esac

napi="$CARGO_TARGET_DIR/napi"
rm -rf "$napi"
mkdir -p "$napi"
cp -R bindings/. "$napi/"
cp extensions/css-parser-napi/backend.js "$napi/"
cp "$CARGO_TARGET_DIR/release/$library" "$napi/css_parser.node"

# the specs, against the instrumented addon
NODE_OPTIONS=--conditions=coverage node_modules/.bin/mocha --timeout 60000 "test/specs/node.spec.js"

# the report: the code of the library and of the N-API extension. the binaries of the library (data generators,
# benchmarks) are not part of it
ignore='(/src/bin/|/build\.rs$|/extensions/css-parser-wasm/)'

cargo llvm-cov report --release --ignore-filename-regex "$ignore" --summary-only

if [ "$html" = 1 ]; then
    cargo llvm-cov report --release --ignore-filename-regex "$ignore" --html --output-dir "$CARGO_TARGET_DIR"
    echo "html report: $CARGO_TARGET_DIR/html/index.html"
fi

if [ "$lcov" = 1 ]; then
    cargo llvm-cov report --release --ignore-filename-regex "$ignore" --lcov --output-path "$CARGO_TARGET_DIR/lcov.info"
    echo "lcov report: $CARGO_TARGET_DIR/lcov.info"
fi

if [ "$json" = 1 ] || [ -n "$fail_under" ]; then
    cargo llvm-cov report --release --ignore-filename-regex "$ignore" --json --summary-only \
        --output-path "$CARGO_TARGET_DIR/coverage.json"
    # the totals, with the names of the istanbul summary (coverage-summary.json of the typescript project)
    node -e '
const fs = require("fs");
const [input, output, failUnder] = process.argv.slice(1);
const totals = JSON.parse(fs.readFileSync(input, "utf8")).data[0].totals;
const pick = (t) => ({ total: t.count, covered: t.covered, pct: Math.round(t.percent * 100) / 100 });
const summary = { total: { lines: pick(totals.lines), functions: pick(totals.functions), regions: pick(totals.regions) } };
if (totals.branches?.count > 0) summary.total.branches = pick(totals.branches);
fs.writeFileSync(output, JSON.stringify(summary, null, 2) + "\n");
console.log(`json summary: ${output}`);
if (failUnder !== "" && summary.total.lines.pct < Number(failUnder)) {
    console.error(`line coverage ${summary.total.lines.pct}% is below ${failUnder}%`);
    process.exit(1);
}
' "$CARGO_TARGET_DIR/coverage.json" "$CARGO_TARGET_DIR/coverage-summary.json" "$fail_under"
fi
