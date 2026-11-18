#!/usr/bin/env bash
# Customer/month workload (docs/spec/slice-7.md): an explicit population with
# activity, a broadcast target and an allocated target, measured like the
# 100,000-line workload.
set -euo pipefail
[[ "$(uname -m)" == "${IRESIPHI_EXPECTED_ARCH:-x86_64}" ]]
mkdir -p ci-artifacts/customer-month
evidence="$(pwd)/ci-artifacts/customer-month"
fixture="$(mktemp -d)"
python3 scripts/generate_fixture.py "$fixture" --workload customer-month > "$evidence/fixture-manifest.json"
cp "$fixture/policy.json" "$evidence/policy.json"
cargo run --locked --release --example workload -- \
  "$fixture/recipe.iresiphi" "$fixture/bindings.json" "$fixture/policy.json" \
  "$fixture/bundle" > "$evidence/run.json"
python3 - "$evidence" <<'PY'
import json, sys
from decimal import Decimal
from pathlib import Path
root = Path(sys.argv[1])
report = json.loads((root / 'run.json').read_text())
assert report['status'] == 'succeeded', report['status']
expected = json.loads((root / 'fixture-manifest.json').read_text())['expected']
manifest = report['manifest']
population = manifest['populations']['customer_targets']
assert population['members'] == expected['members'], population
assert population['period_source'] == 'source:targets/field:Period', population
activity = population['activity']['sales']
assert activity['matched_records'] == int(expected['transactions']), activity
assert activity['unmatched_records'] == 0, activity
totals = manifest['totals']['by_output']['customer_targets']['AUD']
rows = totals['rows']
assert rows['population'] == rows['resolved'] == expected['members'], rows
assert rows['unresolved'] == 0, rows
columns = totals['columns']
assert Decimal(columns['sales']['population']) == Decimal(expected['sales']), columns['sales']
assert Decimal(columns['transactions']['population']) == Decimal(expected['transactions']), columns['transactions']
allocated = columns['allocated_target']['allocation']
assert allocated['conserved'] is True, allocated
assert Decimal(allocated['assigned_total']) == Decimal(allocated['source_total']) == Decimal(expected['allocated_target']), allocated
check = manifest['allocations']['customer_targets']['allocated_target']['check']
assert check['status'] == 'passed' and check['partitions'] == expected['partitions'], check
# Phase 6: the population's activity spills over limits.target_bytes, and
# allocation keeps one entry per member.
spilled = manifest['sources']['transactions'].get('spilled')
assert spilled and spilled['records'] == int(expected['transactions']), manifest['sources']['transactions']
summary = manifest['allocations']['customer_targets']['allocated_target']
assert summary['entries'] == expected['members'] and summary['entry_bytes'] > 0, summary
print(json.dumps({'peak_rss_after_delivery_bytes': report['peak_rss_after_delivery_bytes'], 'run_bundle_wall_us': report['run_bundle_wall_us']}))
PY
