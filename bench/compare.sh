#!/usr/bin/env bash
# Throughput benchmark: lolcat-ultra vs the original Ruby lolcat.
#
# The old benchmark (`yes "test line" | head -n 10M | lolcat-ultra -F`) is
# producer-bound: `head` caps the pipeline at ~0.39s no matter how fast the
# consumer is, so lolcat improvements can't move the number. This benchmark
# instead reads pre-generated files, making the tool under test the only
# bottleneck, and reports throughput (lines/s, MB/s) so the two tools can be
# measured on differently sized inputs: the Ruby implementation is ~3 orders
# of magnitude slower, and running it on the full 10M-line corpus would take
# minutes per iteration.
#
# Requires: hyperfine, python3, the Ruby lolcat gem on PATH (or RUBY_LOLCAT=).
set -euo pipefail
cd "$(dirname "$0")/.."

DATA=target/bench
ULTRA=${ULTRA:-target/release/lolcat-ultra}
RUBY_LOLCAT=${RUBY_LOLCAT:-lolcat}

ULTRA_LINES=10000000 # ~100 MB of "test line": ~0.25s per run for lolcat-ultra
RUBY_LINES=50000     # same corpus truncated: a few seconds per run for Ruby

[ -x "$ULTRA" ] || { echo "error: $ULTRA not found — run 'cargo build --release' first" >&2; exit 1; }
command -v hyperfine >/dev/null || { echo "error: hyperfine not installed (brew install hyperfine)" >&2; exit 1; }
command -v "$RUBY_LOLCAT" >/dev/null || { echo "error: Ruby lolcat not on PATH (gem install lolcat)" >&2; exit 1; }

mkdir -p "$DATA"

# ---- corpora ---------------------------------------------------------------
# ascii: the classic short pure-ASCII workload (exercises the fast path)
if [ ! -s "$DATA/ascii.txt" ]; then
  yes "test line" | head -n "$ULTRA_LINES" > "$DATA/ascii.txt"
fi

# utf8: mixed-width lines with multibyte characters (exercises the slow path);
# built by doubling a seed file, so line count is seed_lines * 2^k
if [ ! -s "$DATA/utf8.txt" ]; then
  printf '%s\n' \
    'naïve café résumé — ünïcödé everywhere' \
    'こんにちは世界 rainbow 🌈 test' \
    'a plain ascii line mixed into the corpus' \
    'Ω≈ç√∫˜µ≤≥÷ symbols and emoji 🎉✨' > "$DATA/utf8.txt"
  while [ "$(wc -l < "$DATA/utf8.txt")" -lt 2000000 ]; do
    cat "$DATA/utf8.txt" "$DATA/utf8.txt" > "$DATA/utf8.tmp" && mv "$DATA/utf8.tmp" "$DATA/utf8.txt"
  done
fi

head -n "$RUBY_LINES" "$DATA/ascii.txt" > "$DATA/ascii-small.txt"
head -n "$RUBY_LINES" "$DATA/utf8.txt" > "$DATA/utf8-small.txt"

# ---- measurement -----------------------------------------------------------
# `cat` on the full corpus gives the I/O floor for reference.
hyperfine --warmup 2 --runs 10 --export-json "$DATA/ultra.json" \
  -n "cat/ascii"   "cat $DATA/ascii.txt > /dev/null" \
  -n "ultra/ascii" "$ULTRA -F $DATA/ascii.txt > /dev/null" \
  -n "ultra/utf8"  "$ULTRA -F $DATA/utf8.txt > /dev/null"

hyperfine --warmup 1 --runs 5 --export-json "$DATA/ruby.json" \
  -n "ruby/ascii" "$RUBY_LOLCAT -f $DATA/ascii-small.txt > /dev/null" \
  -n "ruby/utf8"  "$RUBY_LOLCAT -f $DATA/utf8-small.txt > /dev/null"

# ---- report ----------------------------------------------------------------
python3 - "$DATA" <<'PY'
import json, os, sys

data = sys.argv[1]
runs = {}
for f in ("ultra.json", "ruby.json"):
    for r in json.load(open(os.path.join(data, f)))["results"]:
        runs[r["command"]] = r

def corpus(name, path):
    lines = sum(1 for _ in open(path, "rb"))
    return {"lines": lines, "bytes": os.path.getsize(path)}

corpora = {
    "cat/ascii":   corpus("ascii", f"{data}/ascii.txt"),
    "ultra/ascii": corpus("ascii", f"{data}/ascii.txt"),
    "ultra/utf8":  corpus("utf8",  f"{data}/utf8.txt"),
    "ruby/ascii":  corpus("ascii", f"{data}/ascii-small.txt"),
    "ruby/utf8":   corpus("utf8",  f"{data}/utf8-small.txt"),
}

print(f"\n{'command':<12} {'input':>10} {'mean':>9} {'Mlines/s':>9} {'MB/s':>8}")
stats = {}
for name, c in corpora.items():
    r = runs[name]
    lps = c["lines"] / r["mean"]
    mbps = c["bytes"] / r["mean"] / 1e6
    stats[name] = lps
    size = f"{c['lines']/1e6:.2g}M ln"
    print(f"{name:<12} {size:>10} {r['mean']:>8.3f}s {lps/1e6:>9.2f} {mbps:>8.1f}")

for kind in ("ascii", "utf8"):
    print(f"\nspeedup vs Ruby ({kind}): {stats[f'ultra/{kind}'] / stats[f'ruby/{kind}']:,.0f}x")
PY
