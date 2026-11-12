# Node.js bindings

The source-only `bindings/node` package supports Node **22 and 24** on Linux
**glibc x86_64 and aarch64**. Nothing is published to npm. Browser, Deno, Bun,
Windows and macOS are outside this contract.

Install Rust 1.94.0, a C compiler, and Node 22 or 24. From the repository root:

```sh
cd bindings/node
npm ci
npm run build
npm test
npm run typecheck
```

This builds the development-profile napi-rs addon. Its standalone Cargo.lock
pins the addon dependency graph; no Cargo workspace is added. The root crate's
lockfile, Python wheel build and sdist include list are unchanged. Local ARM
builds are development evidence; hosted CI tests both architectures at the same
commit. See [CI rules](ci.md).

Run this example from `bindings/node` after building (the fixture is included):

```js
const fs = require('node:fs');
const path = require('node:path');
const {Recipe, run} = require('.');
(async () => {
  const golden = JSON.parse(fs.readFileSync('../../tests/fixtures/two_suppliers.parity.json'));
  const recipe = new Recipe(golden.canonical_json);
  const root = path.resolve('test/fixtures');
  const bindings = Object.fromEntries(['a', 'b', 'mapping', 'rates', 'components', 'enrichment'].map(id => {
    const adapter = id === 'enrichment' ? 'xlsx' : ['a', 'b'].includes(id) ? 'json' : 'csv';
    return [id, {path: path.join(root, `${id}.${adapter}`), adapter,
      settings: adapter === 'xlsx' ? {sheet: 'Review', header_row: 1} : {}}];
  }));
  const controller = new AbortController();
  const result = await run(recipe, bindings, {
    allowed_read_roots: [root], allowed_output_roots: [root]
  }, {signal: controller.signal});
  console.log(recipe.hash, result.status, result.batch('a_review'));
  result.close(); recipe.close();
})().catch(error => { console.error(error.record, error.report?.status); process.exitCode = 1; });
```

Use `destination` alongside `signal` to deliver a bundle to a new directory.
`validate(bindings, policy, 'full')` and `explain(bindings, policy)` use the core's
bound checks. Runs use napi-rs workers and return promises. Abort requests the
core cancellation token; cancellation is cooperative. Rejected runs carry
`error.record`, `error.report` and `error.report_id`, including cancelled reports.

Recipe input is JSON text or portable declarative data using the Rust tagged
literal format. Decimal values are strings with declared precision and scale;
Int64 literals are tagged decimal integer strings. Output Int64 values are
`bigint`, decimal values are exact strings (including trailing zeros), and dates
are ISO strings. Metadata integers are also `bigint`, parsed without a double.
Safe integer JavaScript numbers are accepted for structural fields such as scale
and limits; fractional numbers, callbacks and live objects are rejected. Use
`toJson()` to save a recipe without converting its exact literals.

Recipe and run handles own their data. `close()` is idempotent and further reads
raise `HANDLE_CLOSED`. Started runs snapshot their arguments; closing the recipe
does not cancel them. Batches are copies, limited to 8192 rows per call. Memory
reports and evidence are materialized in full at completion; use bundle delivery
for outputs where copying a complete report is too costly. No streaming or
zero-copy transport is claimed. Bound validate/explain calls are synchronous.
