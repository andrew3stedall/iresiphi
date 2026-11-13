import {Recipe, run, IresiphiError, Scalar} from '..';
async function example(document: string) {
  const recipe = new Recipe(document);
  const hash: string = recipe.hash;
  const signal = new AbortController().signal;
  const result = await run(recipe, {orders: {path: '/data/sales.sqlite', adapter: 'sqlite', settings: {table: 'orders'}}}, {allowed_read_roots: ['/data'], allowed_output_roots: [], placement: 'prefer_source'}, {signal});
  const value: Scalar = result.batch('review')[0].amount;
  const count: bigint = result.rowCount('review');
  const status: string = result.status;
  for (const batch of result.batches('review', 10)) console.log(batch, value, count, status, hash);
  result.close(); recipe.close();
}
function error(e: IresiphiError) { console.log(e.code, e.phase, e.record, e.report?.status); }
void example; void error;
// Domain integer transport must never be typed as a JavaScript number.
// @ts-expect-error numbers are excluded from scalar results
const rounded: Scalar = 9223372036854775807;
void rounded;
