# Iresiphi

Iresiphi describes how datasets with different structures and levels of detail relate and become specified outputs. The source format, source location, and place of execution are separate concerns.

## Language

**Dataset**:
A collection of data with its own structure and meaning, whether represented as records, nested documents, a table, or extracted text.

**Source**:
A named input to a recipe that describes a dataset and the fields or paths the recipe uses. A source does not prescribe a file format or storage location.

**Source binding**:
The association between a named source and the concrete data used for a run, such as a database query, file, or dataframe.

**Record projection**:
The declared interpretation of a source as records and fields used in calculations, including extraction, nesting, and row identity. It need not change the stored source.

**Relationship**:
A declared connection between records from different sources, with a matching rule and expected cardinality.

**Relationship path**:
The sequence of declared relationships used to interpret a field relative to an output's records. Different paths may give the field different meanings, such as a purchase region versus a customer's current region.

**Feature**:
A selected raw field or a named transformation of fields.

**Grain**:
The entities and dimensions that identify one record, such as customer by month.

**Recipe**:
A reusable specification of source dependencies, relationships, calculations, granularity changes, and a requested output.

**Output**:
The requested fields, structure, population, and grain produced by applying a recipe. Its destination is distinct from where calculations run.

**Execution plan**:
The arrangement of a recipe's operations across execution locations, including any required data transfers.

**Execution policy**:
The constraints on a run's execution locations, data movement, and resource use.

**Lineage**:
The record of source fields, relationships, and operations that produced an output, extended with execution evidence when available.


**Output population**:
The records eligible to appear in an output before final selection, determined by its declared anchor and eligibility rules.

**Output anchor**:
The source records that establish an output's population and identity, independently of whether related records match.

**Source record reference**:
A reference to a particular observation in a bound dataset, distinct from the business identity that may persist across runs.

**Value mapping**:
An explicit correspondence between source values and another vocabulary, such as a supplier's item description and a contracted item identifier.

**Observed detail**:
Source-supplied records at a finer grain linked to a parent record; their presence does not imply allocation of the parent's measures.

**Comparable subset**:
The output records whose required comparisons are resolved, distinct from the full population containing unresolved records.

**Measure ownership**:
The grain at which a measure has its original meaning; repeating it as context at a finer grain does not make it additive there.
