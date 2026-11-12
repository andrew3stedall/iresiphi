'use strict';
const native = require('./iresiphi.node');
const lossless = require('lossless-json');
const json = {stringify: lossless.stringify, parse: text => lossless.parse(text, undefined, token => /^-?\d+$/.test(token) ? BigInt(token) : token)};
const copy = value => json.parse(json.stringify(value));
class IresiphiError extends Error {
  constructor(record, report) {
    super(record.message); this.name = 'IresiphiError';
    this.report_id = report?.runId ?? null;
    this.record = {...record, report_id: this.report_id};
    Object.assign(this, this.record); this.report = report;
  }
}
function failure(code, message) {
  return new IresiphiError({code, phase: 'node', message, severity: 'error', subject: null,
    context: null, affected_outputs: [], affected_count: null, count_status: 'unknown', samples: [], samples_truncated: false, values: null, check: 'node', remediation: 'Supply an open handle and valid arguments.'});
}
function document(value) {
  if (typeof value === 'string') return value;
  const seen = new Set();
  function check(v) {
    if (v === null || ['string', 'boolean', 'bigint'].includes(typeof v)) return;
    if (typeof v === 'number' && Number.isSafeInteger(v)) return;
    if (typeof v !== 'object' || (!Array.isArray(v) && Object.getPrototypeOf(v) !== Object.prototype) || seen.has(v))
      throw failure('NON_PORTABLE', 'Use portable data and tagged exact scalar strings.');
    seen.add(v); for (const item of Object.values(v)) check(item); seen.delete(v);
  }
  check(value); return json.stringify(value);
}
function unwrap(text) {
  const result = json.parse(text);
  if (result.error) throw new IresiphiError(result.error);
  return result.value;
}
class Recipe {
  #loaded;
  constructor(input) { this.#loaded = unwrap(native.inspectRecipe(document(input), 'load')); }
  #get() { if (!this.#loaded) throw failure('HANDLE_CLOSED', 'Recipe is closed'); return this.#loaded; }
  get hash() { return this.#get().hash; }
  toJson() { return this.#get().json; }
  canonicalJson() { return this.#get().canonical; }
  validate(bindings, policy, dataChecks = 'none') { return unwrap(native.inspectRecipe(this.toJson(), 'validate', bindings === undefined ? undefined : document(bindings), policy === undefined ? undefined : document(policy), dataChecks)); }
  explain(bindings, policy) { return unwrap(native.inspectRecipe(this.toJson(), 'explain', bindings === undefined ? undefined : document(bindings), policy === undefined ? undefined : document(policy))); }
  close() { this.#loaded = null; }
}
function fromTagged(value) {
  switch (value.kind) {
    case 'null': return null;
    case 'int64': return BigInt(value.value);
    case 'decimal': case 'date': case 'string': case 'boolean': return value.value;
    default: throw failure('INVALID_VALUE', 'Unknown scalar kind');
  }
}
class Run {
  #report;
  constructor(report) { this.#report = report; }
  #get() { if (!this.#report) throw failure('HANDLE_CLOSED', 'Run is closed'); return this.#report; }
  get runId() { return this.#get().run_id; }
  get status() { return this.#get().status; }
  get manifest() { return copy(this.#get().manifest); }
  get diagnostics() { return copy(this.#get().diagnostics); }
  get outputs() { return Object.keys(this.#get().tables); }
  #table(name) { const rows = this.#get().tables[name]; if (!rows) throw failure('UNKNOWN_OUTPUT', `Unknown output ${name}`); return rows; }
  schema(name) { this.#table(name); return copy(this.#get().manifest.schema[name].columns); }
  rowCount(name) { return BigInt(this.#table(name).length); }
  batch(name, offset = 0, rows = 8192) {
    if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(rows) || rows < 1 || rows > 8192) throw failure('INVALID_ARGUMENT', 'Batch rows must be 1..8192; offset must be nonnegative');
    return this.#table(name).slice(offset, offset + rows).map(row => Object.fromEntries(Object.entries(row).map(([k,v]) => [k, fromTagged(v)])));
  }
  *batches(name, rows = 8192) { let offset = 0; while (true) { const batch = this.batch(name, offset, rows); if (!batch.length) return; yield batch; offset += batch.length; } }
  evidence(kind = 'primary') { const key = {primary: 'evidence', candidate: 'candidate_evidence', detail: 'detail_evidence', allocation: 'allocation_evidence'}[kind]; if (!key) throw failure('INVALID_ARGUMENT', 'Unknown evidence kind'); return copy(this.#get()[key]); }
  close() { this.#report = null; }
}
async function run(recipe, bindings, policy, {signal, destination} = {}) {
  const text = recipe.toJson(), b = document(bindings), p = document(policy);
  const token = new native.CancelToken();
  const cancel = () => token.cancel();
  signal?.addEventListener('abort', cancel, {once: true});
  if (signal?.aborted) cancel();
  try {
    let output;
    try { output = await native.execute(text, b, p, destination, token); }
    catch (e) { try { unwrap(e.message); } catch (structured) { if (structured instanceof IresiphiError) throw structured; } throw e; }
    const report = json.parse(output), result = new Run(report);
    if (['failed', 'cancelled'].includes(report.status)) throw new IresiphiError(report.diagnostics.find(d => d.severity === 'error') ?? report.diagnostics[0], result);
    return result;
  } finally { signal?.removeEventListener('abort', cancel); }
}
module.exports = {Recipe, Run, run, IresiphiError};
