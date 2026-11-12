# Explicit populations and allocation

Recipe format 0.1 can declare two more things: an explicit output population, and an allocation of a coarse quantity onto that population's members. Both are optional. A recipe that declares neither has the same canonical JSON and hash as before. The normative contract is [docs/spec/slice-2.md](spec/slice-2.md). This page describes what the library does.

A recipe that declares a population must list `"population"` in `required_capabilities`, and one that allocates must also list `"allocation"`. Without them, static validation fails with `UNDECLARED_CAPABILITY`.

## Example: targets by customer and month

`examples/customer_month.iresiphi` is the slice-2 acceptance fixture. Three customers (A and B in North with shares 1 and 3, C in South with share 1) each get a row for January and February 2026. Each row shows the customer's sales, the number of transactions, the regional target as context, and the regional target allocated by share.

```sh
cargo run --example customer_month -- /tmp/iresiphi-customer-month
```

The directory must not exist yet. The program writes the three CSV inputs there, then validates and runs the recipe. On the development machine it printed:

```text
EXAMPLE_OUTPUT
```

North's January goal of 900 splits 1:3 between A and B. C is the only South recipient and gets all 300. The target context column repeats each region's goal on every row. Its six-row sum would be 4100, which is not a target total, so the run leaves that column out of the totals and lists it under `excluded`.

From Python, load the same file and bind the CSV files:

```python
import iresiphi
from iresiphi import build as b

recipe = iresiphi.load("examples/customer_month.iresiphi")
root = "/tmp/iresiphi-customer-month"
bindings = {id: b.binding(f"{root}/{id}.csv", "csv") for id in ["customers", "transactions", "targets"]}
with iresiphi.run(recipe, bindings, b.policy([root])) as result:
    for row in result.batch("customer_targets"):
        print(row["CustNo"], row["month"], row["sales"], row["allocated_target"])
```

`iresiphi.build` has builders for every declaration on this page: `population`, `months`, `period_match`, `allocation`, and the `complete` and `allocations` arguments of `output`. `tests/python_support.py` builds the whole fixture with them.

## Populations

```json
"populations": [
  {"id": "customer_month", "recipients": "customers",
   "filter": {"op": "equal", "left": {"op": "field", "source": "customers", "field": "SalesArea"},
              "right": {"op": "literal", "value": {"kind": "string", "value": "North"}}},
   "period": {"field": "month", "calendar": "month", "values": ["2026-01", "2026-02"]}}
]
```

A population is the cross product of the recipient records that pass `filter` and a finite set of periods. `filter` is optional. It reads only fields of the recipient source, without relationship paths or aggregates, and a record is kept only when the predicate is true.

Each member has every field of its recipient record plus the period field, named by `period.field`. That name must not already be a recipient field. A member's identity and grain are the recipient's identity and grain followed by the period field. A recipe addresses members the way it addresses source records. An output anchors at the population ID, relationships start at it, and expressions read `{"op": "field", "source": "customer_month", ...}`.

A population declares its period set in one of two ways, never both:

- `"values"` lists calendar months as `YYYY-MM` text. The list must be non-empty and must not repeat a value. A value that is not a calendar month fails validation with `INVALID_PERIOD`.
- `"from": {"source": "targets", "field": "Period"}` takes the distinct values of a string field of a named source when the run reads it. A null or a value that is not `YYYY-MM` fails the run with `INVALID_PERIOD` and counts the affected records.

Nothing infers a population from observed activity or from an output's grain. A customer with no transactions in a month is still a member for that month, and a transaction for an unknown customer or month never adds one.

Before it creates any member, the run multiplies the number of recipients that passed the filter by the number of periods. If the product exceeds `limits.anchor_rows`, the run fails with `RESOURCE_LIMIT` and subject `population:<id>` without creating any member.

### Joining finer records

A one-to-many relationship from a population joins finer records to members. Its `period` declares the period-derived key:

```json
{"id": "sales", "from": "customer_month", "to": "transactions",
 "from_keys": ["CustNo"], "to_keys": ["buyer_ref"], "cardinality": "one_to_many",
 "period": {"key": "month", "date": "booked_at", "calendar": "month"}}
```

A transaction joins the member whose `CustNo` equals its `buyer_ref` and whose `month` equals the calendar month of `booked_at`. The month is the first seven characters of the date as recorded. Dates are business-local: nothing converts between time zones, so 2026-01-31 is in January. A null date has no month and joins no member. `key` must be the population's period field, `date` must be a date field of the target source, and the relationship must start at a population. A relationship cannot declare both `period` and `validity`.

Output columns aggregate the joined records with the existing per-record aggregates, such as `{"op": "aggregate", "relationship": "sales", "function": "sum", "field": "gross"}`.

### Members without records

By default a member with no joined records gets a null `sum`, `min` and `max` and a `count_records` of 0. An absence of records is not read as zero.

An output can declare an activity complete:

```json
{"id": "customer_targets", "anchor": "customer_month", "complete": ["sales"], ...}
```

Then a `sum` over a member with no `sales` records is 0 at the field's scale. `min` and `max` stay null, because there is no value to choose. A member whose records exist but whose summed values are null still sums to null. `complete` lists one-to-many relationships from the output's population; anything else fails validation.

Completeness is a declared assumption. The run does not check that the transactions file is complete. Explain (`completeness` on the output node) and the run manifest (`populations.<output>.activity.<relationship>.empty_member_rule` and `statement`) say which rule applied: `declared_complete` or `observed_only`.

### Records that match no member

The run counts finer records that join no member and reports them with the warning `POPULATION_UNMATCHED`. It carries the exact count and up to 20 record references. The manifest repeats the count and samples under `populations.<output>.activity.<relationship>` (`matched_records`, `unmatched_records`, `unmatched_samples`). These records never add a member.

### Population outputs

An output anchored at a population has one row per member. It cannot declare `eligibility` (put the recipient subset in the population's `filter`), `detail`, `group_by` or `aggregates`. Each of these fails validation with `INVALID_POPULATION`.

The run keeps a population output's totals per value of its `currency` column, or under `undeclared` if it has none. If the recipient source declares a `currency` field, the output must keep it as its `currency` column. The totals cover only additive columns. Those are numeric columns the member owns (its own fields and aggregates over its records) and allocated columns. A column read along a to-one path, such as the regional goal, is broadcast context. The totals list it under `excluded` with `"transition": "broadcast"` and never sum it. Population outputs appear under `totals.by_output` only, not in the `combined` totals of the review outputs.

## Allocation

```json
"allocations": [
  {"name": "allocated_target",
   "quantity": {"source": "targets", "field": "Goal"},
   "partition": ["Territory", "Period"],
   "via": "region_target",
   "weight": {"op": "field", "source": "customer_month", "field": "Share"},
   "recipient_filter": {"op": "equal", "left": {"op": "field", "source": "customer_month", "field": "SalesArea"},
                        "right": {"op": "literal", "value": {"kind": "string", "value": "North"}}},
   "precision": 18, "scale": 2,
   "residual": "largest_remainder",
   "null_weight": "zero",
   "unallocated": "report"}
]
```

An output whose anchor is a population can declare allocations. Each one adds a column named `name`. Nothing turns a join or a broadcast into an allocation. The fields:

| Field | Meaning |
| --- | --- |
| `quantity` | A decimal field of the coarse source. |
| `partition` | The coarse source's grain keys, in any order. Each coarse record is one partition. |
| `via` | A many-to-one relationship from the population to the coarse source, without `validity` or `period`, whose target keys are the partition keys. It gives each member its partition. |
| `weight` | An Int64 or decimal expression at member grain. It may read member fields, to-one paths from the population and aggregates over the member's records. |
| `recipient_filter` | Optional. A Boolean expression at member grain. Members for which it is false or null are not recipients. |
| `precision`, `scale` | Type of the allocated column. `scale` must be at least the quantity's scale. |
| `residual` | `largest_remainder` or `report`. Required. |
| `null_weight` | Optional. `zero` treats a null weight as 0. Absent or `error` fails the run. |
| `unallocated` | Optional. `report` reports an unallocatable partition's whole quantity as residual. Absent or `error` fails the run. |

Validation rejects anything else with `INVALID_ALLOCATION` or `TYPE_MISMATCH`. General division stays rejected; the share below is the only division in format 0.1.

### Shares and rounding

For each recipient in a partition, the exact share is `quantity × weight ÷ sum of recipient weights in the partition`. The library computes it with arbitrary-precision integers and no binary floating point, then quantizes it to `scale`:

- `largest_remainder` truncates each share toward zero. The remaining units of the last decimal place go one each to recipients in descending order of the discarded remainder; ties go to the recipient with the smaller identity, compared field by field. The assigned amounts sum exactly to the quantity, and the residual is 0.
- `report` rounds each share half-even and reports the residual, which is the quantity minus the assigned sum. The residual can be negative.

Three recipients of equal weight sharing 2.00 at scale 2 get 0.67, 0.67 and 0.66 under `largest_remainder` (A and B win the tie). Under `report` each gets 0.67, and the residual is -0.01. The library splits a negative quantity by magnitude and gives every share its sign.

After every run, the library checks each partition: assigned sum plus residual must equal the quantity exactly. A violation fails the run with `ALLOCATION_CONSERVATION`. The manifest records the check under `allocations.<output>.<name>.check`, and `check_results` lists `allocation.conservation`.

### Edge cases

No edge case has an implicit default. Each one produces the named diagnostic unless the recipe declares the alternative.

| Case | Result | Declared alternative |
| --- | --- | --- |
| A recipient's weight is null | `ALLOCATION_NULL_WEIGHT` fails the run, with the count and references | `"null_weight": "zero"` |
| A recipient's weight is negative | `ALLOCATION_NEGATIVE_WEIGHT` fails the run | None |
| A partition's recipient weights sum to zero | `ALLOCATION_ZERO_WEIGHT` fails the run | `"unallocated": "report"`: the whole quantity is residual and its recipients get null |
| A partition has no recipients, including when the recipient filter removed them all | `ALLOCATION_NO_RECIPIENTS` fails the run | `"unallocated": "report"`, as above |
| A recipient's partition has no quantity: the `via` lookup found no record, or the quantity is null | The allocation is null and the row's `_status` is `unmatched`, never zero. The warning `ALLOCATION_MISSING_QUANTITY` counts these recipients and the run ends `succeeded_with_unresolved` | None needed |
| Two coarse records share partition keys, or a partition key is null | `DUPLICATE_IDENTITY` or `NULL_IDENTITY` fails the run (check `data.identity`), even when the source's declared identity is a different key | None |
| A recipient carries a currency that differs from its partition's quantity | `MIXED_CURRENCY` fails the run | None |

If the quantity source declares a `currency` field, the recipient source must declare one too, or validation fails with `GRAIN_VIOLATION`.

### Filter timing

A recipient filter runs before normalization. It removes members from the denominator, so it changes shares: with only A as a North recipient, A gets all 900 of North's January goal. The member rows stay in the output with a null allocation and the evidence status `not_recipient`.

The output's `filter` runs after allocation and never changes shares. It can read the allocated column. Selecting customer A keeps A's January share of 225, because B's weight is still in North's denominator. The library computes every allocation over the whole population before it selects any row, so no recipient in a denominator is ever pruned.

Explain names each stage. The allocation node `output:<output>/allocation:<name>` has `recipient_filter.applies` = `before_normalization` and a `denominator` statement. The output filter node has `applies` = `after_allocation`.

### Schema, totals and evidence

In the output schema an allocated column has `role` `allocation`, `ownership` `allocated`, `additive` true, its `partition`, and one origin: the quantity field, its source grain as `origin_grain`, and `transition` `allocated`. A broadcast column keeps `ownership` `broadcast`, `additive` false and the origin transition `broadcast`.

The totals for an allocated column hold the usual `population`, `comparable` and `unresolved` sums over the output's rows. Beside them, `allocation` reports `source_total`, `assigned_total` and `residual_total` over every partition, and whether they conserve. The output filter changes the row sums but not these three figures. When the quantity carries a currency, these figures are per currency.

The run report has a fourth evidence list, `allocation_evidence`, with one record per partition: the partition keys, the quantity, recipient count, weight sum, assigned sum, residual, policy, status (`allocated`, `unallocated` or `no_quantity`) and the quantity record's reference. Python reads it as evidence kind `"allocation"`, and a bundle stores it as the `_allocation_evidence` table when the recipe allocates. Each primary evidence record of a population output carries `member` (its identity values) and `allocations.<name>`. That entry holds the recipient's weight, the exact share before quantization (`exact_share`), and the assigned amount. `exact_share` is decimal text when the share terminates within 38 digits. Otherwise it is null, with `exact_share_terminates` false and `exact_share_status` `non_terminating` or `exceeds_38_digits`.

Budgets, cancellation and bundle delivery work as for other runs. Allocation evidence counts toward `output_bytes` and `retained_bytes`.

## Limits

- Periods are calendar months only. There are no weeks, quarters, fiscal calendars or time zones.
- One allocation reads one coarse source. Allocating across several coarse sources in one operation is not supported.
- A population output keeps one row per member. To aggregate members further, declare another population.
- The review outputs from the invoice preview still total every numeric column, including values looked up along a to-one path. Only population outputs exclude broadcast columns, so the two-supplier fixture's totals do not change.
