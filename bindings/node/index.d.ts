/** Exact domain values. Decimals and ISO dates are strings, Int64 is bigint. */
export type Scalar = null | boolean | string | bigint;
export type Data = null | boolean | string | number | bigint | Data[] | {[key: string]: Data};
/** Integer metadata in returned records is bigint too. */
export type RecordData = {[key: string]: unknown};
export interface Binding {path: string; adapter: 'json' | 'csv' | 'xlsx' | 'sqlite'; settings?: {[key: string]: Data}}
export type Bindings = {[source: string]: Binding};
export interface Policy {allowed_read_roots: string[]; allowed_output_roots: string[]; limits?: {[key: string]: number | bigint}; diagnostic_values?: boolean; placement?: 'local' | 'prefer_source' | 'require_source'; transfers?: 'none' | 'keys'; evidence?: 'problems' | 'full'}
export interface Diagnostic extends RecordData {code: string; phase: string; message: string; severity: string; remediation: string | null}
export class IresiphiError extends Error {
  readonly code: string; readonly phase: string; readonly record: Diagnostic;
  readonly report?: Run; readonly report_id: string | null;
}
export class Recipe {
  constructor(document: string | {[key: string]: Data});
  readonly hash: string;
  toJson(): string;
  canonicalJson(): string;
  validate(bindings?: Bindings, policy?: Policy, dataChecks?: 'none' | 'full'): RecordData & {valid: boolean};
  explain(bindings?: Bindings, policy?: Policy): RecordData;
  close(): void;
}
export class Run {
  private constructor();
  readonly runId: string; readonly status: string;
  readonly manifest: RecordData; readonly diagnostics: Diagnostic[]; readonly outputs: string[];
  schema(table: string): RecordData[];
  rowCount(table: string): bigint;
  batch(table: string, offset?: number, rows?: number): {[column: string]: Scalar}[];
  batches(table: string, rows?: number): Generator<{[column: string]: Scalar}[]>;
  evidence(kind?: 'primary' | 'candidate' | 'detail' | 'allocation'): RecordData[];
  close(): void;
}
export function run(recipe: Recipe, bindings: Bindings, policy: Policy, settings?: {signal?: AbortSignal; destination?: string}): Promise<Run>;
