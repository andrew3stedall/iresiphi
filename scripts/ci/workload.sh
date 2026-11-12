#!/usr/bin/env bash
set -euo pipefail
[[ "$(uname -m)" == x86_64 ]]
mkdir -p ci-artifacts/workload
evidence="$(pwd)/ci-artifacts/workload"
{
  git rev-parse HEAD
  uname -a
  cat /etc/os-release
  lscpu
  free -b
  rustc -Vv
  cargo -V
  python3 -V
} > "$evidence/environment.txt"
fixture="$(mktemp -d)"
python3 scripts/generate_fixture.py "$fixture" --lines 100000 --byte-limit 1073741824 > "$evidence/fixture-manifest.json"
cp "$fixture/policy.json" "$evidence/policy.json"
cargo run --locked --release --example workload -- \
  "$fixture/recipe.iresiphi" "$fixture/bindings.json" "$fixture/policy.json" \
  "$fixture/bundle" > "$evidence/run.json"
IRESIPHI_CANCEL_AFTER_MS=50 cargo run --locked --release --example workload -- \
  "$fixture/recipe.iresiphi" "$fixture/bindings.json" "$fixture/policy.json" \
  "$fixture/cancelled-bundle" > "$evidence/cancellation.json"
python3 - "$evidence" <<'PY'
import json, sys
from decimal import Decimal
from pathlib import Path
root = Path(sys.argv[1])
report = json.loads((root / 'run.json').read_text())
assert report['status'] == 'succeeded_with_unresolved', report
expected = json.loads((root / 'fixture-manifest.json').read_text())['expected']
manifest = report['manifest']
assert manifest['resolved_rows'] == 99999, manifest
assert manifest['unresolved_rows'] == 1, manifest
columns = manifest['totals']['combined']['AUD']['columns']
for column, subset, key in [
    ('billed', 'population', 'full_billed'),
    ('billed', 'comparable', 'comparable_billed'),
    ('billed', 'unresolved', 'unresolved_billed'),
    ('expected', 'comparable', 'expected'),
    ('variance', 'comparable', 'variance'),
]:
    assert Decimal(columns[column][subset]) == Decimal(expected[key]), (column, subset)
cancelled = json.loads((root / 'cancellation.json').read_text())
assert cancelled['status'] == 'cancelled', cancelled
assert cancelled['cancel_to_return_us'] is not None, cancelled
PY
