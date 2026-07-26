#!/usr/bin/env bash
# Run every reference transcript through the Rust CLI and report per-file and
# total parity. Usage: scripts/parity.sh [path-to-libqalculate]
set -uo pipefail

REF="${1:-/root/Project/libqalculate}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QALC="$ROOT/target/debug/qalc"

if [[ ! -x "$QALC" ]]; then
  echo "building qalc..." >&2
  (cd "$ROOT" && cargo build -q -p qalc) || exit 1
fi

total_pass=0
total_all=0
printf '%-24s %8s  %s\n' FILE SCORE BAR
for f in "$REF"/tests/*.batch; do
  line=$("$QALC" --test-file="$f" 2>&1 | tail -1)
  # "<path> - P/T passed"
  score=${line##* - }
  score=${score% passed}
  pass=${score%%/*}
  all=${score##*/}
  [[ "$pass" =~ ^[0-9]+$ ]] || { pass=0; all=0; }
  total_pass=$((total_pass + pass))
  total_all=$((total_all + all))
  pct=0
  [[ $all -gt 0 ]] && pct=$((pass * 20 / all))
  bar=$(printf '#%.0s' $(seq 1 $pct) 2>/dev/null)
  printf '%-24s %8s  %s\n' "$(basename "$f")" "$score" "$bar"
done
echo
pct=0
[[ $total_all -gt 0 ]] && pct=$((total_pass * 100 / total_all))
printf 'TOTAL: %d/%d (%d%%)\n' "$total_pass" "$total_all" "$pct"
