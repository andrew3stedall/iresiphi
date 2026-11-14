const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const api = require('..');

const field = (source, name, via = []) => ({op:'field', source, field:name, path:via});
const lit = (kind, value, extra = {}) => ({op:'literal', value:{kind, value, ...extra}});
const binary = (op, left, right) => ({op, left, right});
const column = (name, expr) => ({name, expr});
function declaration(eligibility = binary('and', field('orders', 'open'),
  binary('less_equal', lit('date', '2026-01-01'), field('orders', 'booked')))) {
  const projected = (name, kind, extra = {}, nullable = false) => [name, {path:name, data_type:{kind, ...extra}, nullable}];
  return {format_version:'0.1', name:'open orders', sources:[
    {id:'orders', identity:['order_no'], grain:['order_no'], projection:{fields:Object.fromEntries([
      projected('order_no', 'int64'), projected('cust', 'string', {}, true),
      projected('booked', 'date', {}, true), projected('net', 'decimal', {precision:18, scale:2}),
      projected('open', 'boolean')])}},
    {id:'customers', identity:['CustNo'], grain:['CustNo'], projection:{fields:Object.fromEntries([
      projected('CustNo', 'string'), projected('SalesArea', 'string')])}}
  ], relationships:[{id:'order_customer', from:'orders', to:'customers', from_keys:['cust'], to_keys:['CustNo'], cardinality:'many_to_one'}],
  outputs:[{id:'open_orders', anchor:'orders', eligibility, columns:[
    column('order_no', field('orders', 'order_no')), column('currency', lit('string', 'AUD')),
    column('net', field('orders', 'net')), column('area', field('customers', 'SalesArea', ['order_customer']))]}]};
}
function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'iresiphi-sqlite-'));
  t.after(() => fs.rmSync(root, {recursive:true, force:true}));
  fs.copyFileSync(path.join(__dirname, 'fixtures/sales.sqlite'), path.join(root, 'sales.sqlite'));
  fs.writeFileSync(path.join(root, 'customers.csv'), 'CustNo,SalesArea\nA,North\nB,North\nC,South\n');
  return {root, bindings:{orders:{path:path.join(root, 'sales.sqlite'), adapter:'sqlite', settings:{table:'orders'}},
    customers:{path:path.join(root, 'customers.csv'), adapter:'csv'}},
  policy: placement => ({allowed_read_roots:[root], allowed_output_roots:[], placement})};
}

test('SQLite placement preserves acceptance rows, evidence, output counts and recipe identity', async t => {
  const f = fixture(t);
  const recipe = new api.Recipe(declaration());
  t.after(() => recipe.close());
  const original = [recipe.hash, recipe.canonicalJson()];
  const snapshots = [];
  for (const placement of ['local', 'prefer_source', 'require_source']) {
    const policy = f.policy(placement);
    assert.equal(recipe.validate(f.bindings, policy, 'full').valid, true);
    const explanation = recipe.explain(f.bindings, policy).bound;
    assert.equal(explanation.feasible, true);
    assert.equal(explanation.placement.operations[0].placement, placement === 'local' ? 'local' : 'source');
    const result = await api.run(recipe, f.bindings, policy);
    try {
      const rows = result.batch('open_orders');
      assert.deepEqual(rows.map(r => r.order_no), [1n, 3n, 4n, 5n]);
      assert.deepEqual(rows.map(r => r.net), ['60.00', '200.00', '120.50', '10.00']);
      assert.deepEqual(rows.map(r => r.area), ['North', 'North', 'South', null]);
      assert.equal(rows[3]._status, 'unmatched');
      assert.equal(result.status, 'succeeded_with_unresolved');
      const source = result.manifest.sources.orders;
      assert.equal(source.rows, 6n);
      assert.equal(source.scanned_rows, placement === 'local' ? undefined : 4n);
      snapshots.push([rows, result.evidence(), result.evidence('candidate'), result.manifest.counts.open_orders]);
    } finally { result.close(); }
    assert.deepEqual([recipe.hash, recipe.canonicalJson()], original);
  }
  assert.deepEqual(snapshots[0], snapshots[1]);
  assert.deepEqual(snapshots[0], snapshots[2]);
});

test('decimal eligibility falls back or refuses before opening; explain opens nothing', async t => {
  const f = fixture(t);
  const recipe = new api.Recipe(declaration(binary('less',
    lit('decimal', '50.00', {precision:18, scale:2}), field('orders', 'net'))));
  t.after(() => recipe.close());
  const result = await api.run(recipe, f.bindings, f.policy('prefer_source'));
  try {
    assert.equal(result.manifest.sources.orders.scanned_rows, undefined);
    assert.deepEqual(result.batch('open_orders').map(r => r.order_no), [1n, 3n, 4n, 6n]);
  } finally { result.close(); }
  fs.unlinkSync(f.bindings.orders.path);
  const explanation = recipe.explain(f.bindings, f.policy('prefer_source')).bound;
  assert.equal(explanation.feasible, true);
  assert.match(explanation.placement.operations[0].reason, /decimal/);
  assert.equal(recipe.validate(f.bindings, f.policy('require_source')).valid, false);
  assert.equal(recipe.explain(f.bindings, f.policy('require_source')).bound.feasible, false);
  await assert.rejects(api.run(recipe, f.bindings, f.policy('require_source')), {code:'PLACEMENT_INFEASIBLE'});
});

test('co-located SQLite snapshot reduces lookup targets through the public API', async t => {
  const f = fixture(t);
  fs.copyFileSync(path.join(__dirname, 'fixtures/sales-colocated.sqlite'), f.bindings.orders.path);
  f.bindings.customers = {path:f.bindings.orders.path, adapter:'sqlite', settings:{table:'customers'}};
  const doc = declaration();
  doc.sources[1].projection.fields.Since = {path:'Since', data_type:{kind:'date'}, nullable:true};
  const recipe = new api.Recipe(doc);
  t.after(() => recipe.close());
  const original = [recipe.hash, recipe.canonicalJson()];
  const snapshots = [];
  for (const placement of ['local', 'prefer_source', 'require_source']) {
    const policy = {...f.policy(placement), limits:{input_bytes:fs.statSync(f.bindings.orders.path).size}};
    assert.equal(recipe.validate(f.bindings, policy, 'full').valid, true);
    assert.equal(recipe.explain(f.bindings, policy).bound.feasible, true);
    const result = await api.run(recipe, f.bindings, policy);
    try {
      const sources = result.manifest.sources;
      assert.deepEqual(sources.orders.shared_read, ['customers']);
      assert.deepEqual(sources.customers.shared_read, ['orders']);
      assert.equal(sources.orders.fingerprint, sources.customers.fingerprint);
      assert.equal(sources.orders.rows, 7n);
      assert.equal(sources.customers.rows, 5n);
      assert.equal(sources.customers.scanned_rows, placement === 'local' ? undefined : 3n);
      assert.deepEqual(result.batch('open_orders').map(r => r.order_no), [1n,3n,4n,5n]);
      snapshots.push([result.batch('open_orders'),result.evidence(),result.evidence('candidate'),result.manifest.counts]);
    } finally { result.close(); }
  }
  assert.deepEqual(snapshots[0],snapshots[1]);
  assert.deepEqual(snapshots[0],snapshots[2]);
  assert.deepEqual([recipe.hash,recipe.canonicalJson()],original);
});

test('key transfer reduces a SQLite lookup target from CSV orders under policy', async t => {
  const f = fixture(t);
  const orders = path.join(f.root, 'orders.csv');
  fs.writeFileSync(orders, 'order_no,cust,booked,net,open\n1,A,2026-01-05,60.00,true\n2,A,2026-01-20,40.00,false\n' +
    '3,B,2026-01-11,200.00,true\n4,C,2026-02-08,120.50,true\n5,Z,2026-02-09,10.00,true\n' +
    '6,B,2025-12-31,75.25,true\n7,D,2025-11-02,30.00,true\n');
  const crm = path.join(f.root, 'crm.sqlite');
  fs.copyFileSync(path.join(__dirname, 'fixtures/sales-colocated.sqlite'), crm);
  const bindings = {orders:{path:orders, adapter:'csv'}, customers:{path:crm, adapter:'sqlite', settings:{table:'customers'}}};
  const doc = declaration();
  doc.sources[1].projection.fields.Since = {path:'Since', data_type:{kind:'date'}, nullable:true};
  const recipe = new api.Recipe(doc);
  t.after(() => recipe.close());
  const original = [recipe.hash, recipe.canonicalJson()];
  const snapshots = [];
  for (const [placement, transfers, scanned] of [['local', undefined, undefined], ['require_source', undefined, undefined],
    ['prefer_source', 'none', undefined], ['local', 'keys', undefined], ['prefer_source', 'keys', 4n], ['require_source', 'keys', 4n]]) {
    const policy = {...f.policy(placement), ...(transfers ? {transfers} : {})};
    assert.equal(recipe.validate(bindings, policy, 'full').valid, true);
    const bound = recipe.explain(bindings, policy).bound;
    assert.equal(bound.feasible, true);
    assert.deepEqual(bound.transfers.map(x => x.transfer), scanned ? ['relationship:order_customer/keys'] : []);
    const result = await api.run(recipe, bindings, policy);
    try {
      const customers = result.manifest.sources.customers;
      assert.equal(customers.rows, 5n);
      assert.equal(customers.scanned_rows, scanned);
      if (scanned) {
        assert.equal(customers.placement.transfer.keys, 5n);
        assert.equal(customers.placement.reduction, 'transferred_keys');
      }
      assert.deepEqual(result.batch('open_orders').map(r => r.order_no), [1n, 3n, 4n, 5n]);
      snapshots.push([result.batch('open_orders'), result.evidence(), result.evidence('candidate'), result.manifest.counts]);
    } finally { result.close(); }
  }
  for (const snapshot of snapshots.slice(1)) assert.deepEqual(snapshot, snapshots[0]);
  await assert.rejects(api.run(recipe, bindings, {...f.policy('require_source'), transfers:'keys', limits:{transfer_keys:4}}),
    {code:'PLACEMENT_INFEASIBLE'});
  assert.deepEqual([recipe.hash, recipe.canonicalJson()], original);
});
