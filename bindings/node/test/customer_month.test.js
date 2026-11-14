const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const api = require('..');
// Shared with the Rust and Python suites (docs/spec/slice-2.md acceptance fixture).
const golden = JSON.parse(fs.readFileSync('../../tests/fixtures/customer_month.parity.json', 'utf8'));
const inputs = {
  customers: 'CustNo,SalesArea,Share\nA,North,1\nB,North,3\nC,South,1\n',
  transactions: 'receipt_id,buyer_ref,booked_at,gross\nt1,A,2026-01-05,60\nt2,A,2026-01-20,40\nt3,B,2026-01-11,200\nt4,C,2026-01-17,50\nt5,B,2026-02-08,120\n',
  targets: 'Territory,Period,Goal\nNorth,2026-01,900\nSouth,2026-01,300\nNorth,2026-02,800\nSouth,2026-02,400\n',
};
const normalize = value => JSON.parse(JSON.stringify(value, (_, v) => typeof v === 'bigint' ? Number(v) : v)); // Only bounded metadata, never domain scalars.
const sorted = rows => [...rows].sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));

test('shared customer/month golden: population, allocation, totals and conservation', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'iresiphi-node-cm-'));
  try {
    const bindings = Object.fromEntries(Object.entries(inputs).map(([id, content]) => {
      const file = path.join(directory, `${id}.csv`);
      fs.writeFileSync(file, content);
      return [id, {path: file, adapter: 'csv', settings: {}}];
    }));
    const policy = {allowed_read_roots: [directory], allowed_output_roots: [directory], evidence: 'full'};
    const recipe = new api.Recipe(golden.canonical_json);
    assert.equal(recipe.hash, golden.recipe_hash);
    assert.equal(recipe.validate(bindings, policy, 'full').valid, true);
    const result = await api.run(recipe, bindings, policy);
    assert.equal(result.status, golden.expected.status);
    assert.deepEqual(normalize(result.manifest.totals), golden.expected.totals);
    assert.deepEqual(normalize(result.manifest.allocations), golden.expected.allocations);
    for (const [name, expected] of Object.entries(golden.expected.tables)) {
      assert.deepEqual(normalize(result.schema(name)), golden.expected.schema[name].columns);
      const plain = expected.map(row => Object.fromEntries(Object.entries(row).map(([key, v]) => [key, v.kind === 'null' ? null : v.kind === 'int64' ? BigInt(v.value) : v.value])));
      assert.deepEqual(sorted(result.batch(name).map(row => JSON.stringify(row, (_, v) => typeof v === 'bigint' ? `${v}n` : v))),
        sorted(plain.map(row => JSON.stringify(row, (_, v) => typeof v === 'bigint' ? `${v}n` : v))));
    }
    assert.equal(result.evidence('allocation').length, golden.expected.evidence_counts.allocation);
    // The fixed acceptance numbers, independent of the golden file.
    const rows = Object.fromEntries(result.batch('customer_targets').map(r => [`${r.CustNo}/${r.month}`, r]));
    assert.equal(rows['A/2026-01'].allocated_target, '225.00');
    assert.equal(rows['B/2026-01'].allocated_target, '675.00');
    assert.equal(rows['C/2026-01'].allocated_target, '300.00');
    assert.equal(rows['A/2026-02'].allocated_target, '200.00');
    assert.equal(rows['B/2026-02'].allocated_target, '600.00');
    assert.equal(rows['C/2026-02'].allocated_target, '400.00');
    assert.equal(rows['A/2026-02'].sales, '0.00');
    assert.equal(rows['A/2026-02'].transactions, 0n);
    assert.equal(rows['A/2026-01'].transactions, 2n);
  } finally { fs.rmSync(directory, {recursive: true, force: true}); }
});
