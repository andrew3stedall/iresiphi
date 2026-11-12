const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const api = require('..');
const golden = JSON.parse(fs.readFileSync('../../tests/fixtures/two_suppliers.parity.json', 'utf8'));
const root = path.resolve('test/fixtures');
const bindings = Object.fromEntries(['a','b','mapping','rates','components','enrichment'].map(id => [id, {path: path.join(root, id === 'enrichment' ? 'enrichment.xlsx' : `${id}.${['a','b'].includes(id) ? 'json' : 'csv'}`), adapter: id === 'enrichment' ? 'xlsx' : ['a','b'].includes(id) ? 'json' : 'csv', ...(id === 'enrichment' ? {settings: {sheet: 'Review', header_row: 1}} : {settings:{}})}]));
const policy = {allowed_read_roots: [root], allowed_output_roots: [os.tmpdir()]};
const recipe = () => new api.Recipe(golden.canonical_json);
const normalize = value => JSON.parse(JSON.stringify(value, (_, v) => typeof v === 'bigint' ? Number(v) : v)); // Only bounded fixture metadata, never domain scalars.
const sorted = rows => [...rows].sort((a,b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));

test('shared two-supplier golden: rows, schema, totals, reconciliation, diagnostics, evidence', async () => {
  const r = recipe();
  assert.equal(r.validate(bindings, policy, 'full').valid, true);
  assert.ok(r.explain(bindings, policy));
  const result = await api.run(r, bindings, policy);
  assert.equal(result.status, golden.expected.status);
  assert.deepEqual(normalize(result.manifest.totals), golden.expected.totals);
  assert.deepEqual(normalize(result.manifest.reconciliation), golden.expected.reconciliation);
  for (const [name, expected] of Object.entries(golden.expected.tables)) {
    assert.deepEqual(normalize(result.schema(name)), golden.expected.schema[name].columns);
    const plain = expected.map(row => Object.fromEntries(Object.entries(row).map(([key, v]) => [key, v.kind === 'null' ? null : v.value])));
    assert.deepEqual(sorted(result.batch(name)), sorted(plain));
  }
  const diagnostics = result.diagnostics.map(({code,severity,subject,affected_count}) => ({code,severity,subject,affected_count}));
  assert.deepEqual(sorted(normalize(diagnostics)), sorted(golden.expected.diagnostics));
  for (const [kind, count] of Object.entries(golden.expected.evidence_counts)) assert.equal(result.evidence(kind).length, count);
  const owned = result.batch('a_review');
  result.close(); result.close();
  assert.equal(owned.length, 2);
  assert.throws(() => result.status, {code: 'HANDLE_CLOSED'});
});
test('structured core errors preserve failed reports', async () => {
  assert.throws(() => new api.Recipe('{}'), e => e instanceof api.IresiphiError && !!e.code && !!e.phase && !!e.record.remediation);
  const missing = {...bindings}; delete missing.rates;
  await assert.rejects(api.run(recipe(), missing, policy), e => {
    assert.equal(e.code, golden.errors.missing_binding.code);
    assert.equal(e.phase, golden.errors.missing_binding.phase);
    assert.equal(e.report.status, 'failed'); assert.equal(e.report_id, e.report.runId); assert.equal(e.record.report_id, e.report.runId); return true;
  });
});
test('pre-aborted and active AbortSignals preserve cancelled reports and keep JS responsive', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'iresiphi-node-cancel-'));
  try {
    const invoice = JSON.parse(fs.readFileSync(path.join(root, 'a.json'), 'utf8'));
    invoice.lines = Array.from({length: 2000}, (_, i) => ({...invoice.lines[0], id: String(i)}));
    const input = path.join(directory, 'a.json');
    fs.writeFileSync(input, JSON.stringify(invoice));
    const b = {...bindings, a:{...bindings.a, path:input}};
    const p = {...policy, allowed_read_roots:[root,directory]};
    for (const pre of [true, false]) {
      const controller = new AbortController();
      if (pre) controller.abort();
      const pending = api.run(recipe(), b, p, {signal: controller.signal});
      assert.ok(pending instanceof Promise);
      let ticked = pre;
      if (!pre) setTimeout(() => { ticked = true; controller.abort(); }, 10);
      await assert.rejects(pending, e => {
        assert.equal(e.code, 'CANCELLED'); assert.equal(e.report.status, 'cancelled');
        assert.equal(e.report.manifest.complete, false); assert.deepEqual(e.report.outputs, []); return true;
      });
      assert.equal(ticked, true);
    }
  } finally { fs.rmSync(directory, {recursive:true, force:true}); }
});
test('started runs own their recipe and bindings; bundle delivery uses core sink', async () => {
  const r = recipe(); const b = structuredClone(bindings);
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'iresiphi-node-'));
  try {
    const pending = api.run(r, b, policy, {destination: path.join(directory, 'bundle')});
    r.close(); delete b.a;
    const result = await pending;
    assert.ok(result.status.startsWith('succeeded'));
    assert.ok(fs.existsSync(path.join(directory, 'bundle', 'manifest.json')));
  } finally { fs.rmSync(directory, {recursive: true, force: true}); }
});
test('38-digit decimals, i64 bounds, ISO dates round trip through save and native execution', async () => {
  const doc = JSON.parse(golden.canonical_json);
  const values = {
    dec: {kind:'decimal', value:'12345678901234567890123456789012345678', precision:38, scale:0},
    fraction: {kind:'decimal', value:'0.12345678901234567890123456789012345678', precision:38, scale:38},
    min: {kind:'int64', value:'-9223372036854775808'}, max: {kind:'int64', value:'9223372036854775807'},
    date: {kind:'date', value:'2024-02-29'}
  };
  doc.outputs = [{id:'exact', anchor:'a', columns:Object.entries(values).map(([name,value]) => ({name,expr:{op:'literal',value}}))}];
  const r = new api.Recipe(doc);
  const result = await api.run(new api.Recipe(r.toJson()), bindings, policy);
  const row = result.batch('exact')[0];
  assert.equal(row.dec, values.dec.value); assert.equal(row.fraction, values.fraction.value);
  assert.equal(row.min, -9223372036854775808n); assert.equal(row.max, 9223372036854775807n);
  assert.equal(row.date, '2024-02-29');
  assert.throws(() => new api.Recipe({...doc, annotations:{float: 0.1}}), {code:'NON_PORTABLE'});
});
test('invalid bound arguments reject with structured errors', async () => {
  await assert.rejects(api.run(recipe(), bindings, {}), e => e instanceof api.IresiphiError && e.code === 'INVALID_POLICY');
});
test('duplicate identity and unsupported division match shared error golden', async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'iresiphi-node-errors-'));
  try {
    fs.writeFileSync(path.join(directory, 'mapping.csv'), 'id,supplier,raw,item\nm1,A,service A,shared\nm1,B,service B,shared\n');
    const b = {...bindings, mapping:{...bindings.mapping, path:path.join(directory, 'mapping.csv')}};
    await assert.rejects(api.run(recipe(), b, {...policy, allowed_read_roots:[root,directory]}), error => {
      const expected = golden.errors.duplicate_identity;
      for (const field of ['code','phase','check','subject']) assert.equal(error[field], expected[field]);
      return true;
    });
    const doc = JSON.parse(golden.canonical_json);
    const output = doc.outputs.find(o => o.id === 'a_review');
    output.columns.find(c => c.name === 'variance').expr = {op:'divide', left:{op:'field',source:'a',field:'billed'}, right:{op:'field',source:'a',field:'quantity'}};
    const expected = golden.errors.division;
    if (expected.stage === 'load') assert.throws(() => new api.Recipe(doc), {code:expected.code,phase:expected.phase});
    else {
      const report = new api.Recipe(doc).validate();
      assert.equal(report.valid, false);
      const diagnostic = report.checks.find(c => c.diagnostic)?.diagnostic;
      assert.equal(diagnostic.code, expected.code); assert.equal(diagnostic.phase, expected.phase);
    }
  } finally { fs.rmSync(directory,{recursive:true,force:true}); }
});
test('argument diagnostics retain Python-compatible codes and phases', async () => {
  await assert.rejects(api.run(recipe(), {}, {}), {code:'INVALID_POLICY', phase:'policy'});
  await assert.rejects(api.run(recipe(), {a:{}}, policy), {code:'INVALID_BINDING', phase:'binding'});
});
