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
python3 scripts/generate_fixture.py "$fixture" --lines 100000 > "$evidence/fixture-manifest.json"
cp "$fixture/policy.json" "$evidence/policy.json"
# The recorded fixture exceeds default evidence/retention admission. Preserve
# that expected failure, then measure an explicitly raised policy as documented.
if cargo run --locked --release --example workload -- \
  "$fixture/recipe.iresiphi" "$fixture/bindings.json" "$fixture/policy.json" \
  "$fixture/default-bundle" > "$evidence/default-policy.json"; then
  echo "Default admission behavior changed; review the workload policy." >&2
  exit 1
fi
python3 - "$fixture/policy.json" <<'PY'
import json, sys
from pathlib import Path
path = Path(sys.argv[1])
policy = json.loads(path.read_text())
policy['limits'].update(output_bytes=1073741824, retained_bytes=1073741824)
path.write_text(json.dumps(policy))
PY
cp "$fixture/policy.json" "$evidence/raised-policy.json"
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
default = json.loads((root / 'default-policy.json').read_text())
assert default['status'] == 'failed', default
assert any(d['code'] == 'RESOURCE_LIMIT' for d in default['diagnostics']), default
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
