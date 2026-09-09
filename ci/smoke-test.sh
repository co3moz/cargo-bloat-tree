#!/usr/bin/env bash
#
# End to end check: build a throwaway crate that declares `regex` and
# `serde_json`, run the real pipeline over it, and verify that the crates it
# never declared (regex-automata and friends) are charged to the dependency
# that pulls them in, instead of being listed next to the declared ones.
#
# Usage: ci/smoke-test.sh [path/to/cargo-bloat-tree]

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)

# `python3` exists as a non-working stub on Windows, so probe it for real
python=python3
"$python" -c "" > /dev/null 2>&1 || python=python
"$python" -c "" > /dev/null 2>&1 || {
    echo "this script needs python" >&2
    exit 1
}

bin=${1:-}
if [ -z "$bin" ]; then
    cargo build --release --manifest-path "$root/Cargo.toml"
    # honour CARGO_TARGET_DIR and .cargo/config.toml instead of guessing
    target=$(cargo metadata --format-version 1 --no-deps \
        --manifest-path "$root/Cargo.toml" |
        "$python" -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
    bin="$target/release/cargo-bloat-tree"
fi
[ -x "$bin" ] || bin="$bin.exe"
[ -x "$bin" ] || {
    echo "no cargo-bloat-tree binary at $bin" >&2
    exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/src"
cat > "$work/Cargo.toml" <<'EOF'
[package]
name = "smoke"
version = "0.0.0"
edition = "2021"

[dependencies]
regex = "1"
serde_json = "1"

[workspace]

[profile.release]
debug = 1
EOF
cat > "$work/src/main.rs" <<'EOF'
use regex::Regex;

fn main() {
    let value: serde_json::Value = serde_json::from_str(r#"{"a":[1,2,3]}"#).unwrap();
    let re = Regex::new(r"[a-z]+\d{2,}").unwrap();
    println!("{} {}", value, re.is_match(&value.to_string()));
}
EOF

fail() {
    echo "SMOKE TEST FAILED: $1" >&2
    exit 1
}

echo "== cargo bloat-tree --release =="
"$bin" bloat-tree --release --color never \
    --manifest-path "$work/Cargo.toml" \
    --save-bloat-json "$work/bloat.json" | tee "$work/report.txt"

grep -q 'Direct dependencies' "$work/report.txt" || fail "no summary table"
grep -q 'regex v1\.' "$work/report.txt" || fail "regex is missing from the report"
grep -q 'regex-automata' "$work/report.txt" ||
    fail "regex-automata should show up in the tree under regex"

echo "== only declared dependencies are listed as direct =="
"$bin" bloat-tree --bloat-json "$work/bloat.json" \
    --manifest-path "$work/Cargo.toml" --json > "$work/report.json"
"$python" - "$work/report.json" <<'PY'
import json, sys

report = json.load(open(sys.argv[1]))
direct = sorted(d["name"] for d in report["direct"])
assert direct == ["regex", "serde_json"], f"unexpected direct dependencies: {direct}"

regex = next(d for d in report["direct"] if d["name"] == "regex")
assert regex["total"] > regex["self"], "regex should be charged for its subtree"
assert report["text-section-size"] > 0, "no .text size reported"
print(f"ok: direct={direct}, regex total={regex['total']}B self={regex['self']}B")
PY

echo "== --invert points back at the declared dependency =="
"$bin" bloat-tree --bloat-json "$work/bloat.json" --color never \
    --manifest-path "$work/Cargo.toml" --invert regex-automata > "$work/invert.txt"
cat "$work/invert.txt"
grep -q '\[direct\]' "$work/invert.txt" ||
    fail "--invert did not reach a declared dependency"
grep -q 'regex v1\.' "$work/invert.txt" ||
    fail "--invert did not name regex as the crate pulling regex-automata in"

echo "== reading a piped cargo bloat result =="
(cd "$work" && cargo bloat --release --crates -n 0 --message-format json) |
    "$bin" bloat-tree --bloat-json - --manifest-path "$work/Cargo.toml" \
        --flat --color never > "$work/piped.txt"
grep -q 'Direct dependencies' "$work/piped.txt" || fail "reading the report from stdin is broken"

echo "SMOKE TEST PASSED"
