use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Type {
    Boolean,
    Int64,
    String,
    Decimal { precision: u8, scale: u8 },
    Date,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Value {
    Null,
    Boolean {
        value: bool,
    },
    Int64 {
        value: String,
    },
    String {
        value: String,
    },
    Decimal {
        value: String,
        precision: u8,
        scale: u8,
    },
    Date {
        value: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub format_version: String,
    pub name: String,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    #[serde(default)]
    pub features: Vec<Feature>,
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub bundled_definitions: BTreeMap<String, BundledDefinition>,
    #[serde(default)]
    pub annotations: BTreeMap<String, serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub projection: Projection,
    pub identity: Vec<String>,
    pub grain: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Projection {
    #[serde(default)]
    pub array: Option<String>,
    pub fields: BTreeMap<String, Field>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub path: String,
    pub data_type: Type,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub header: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relationship {
    pub id: String,
    pub from: String,
    pub to: String,
    pub from_keys: Vec<String>,
    pub to_keys: Vec<String>,
    pub cardinality: Cardinality,
    #[serde(default)]
    pub validity: Option<Validity>,
    #[serde(default)]
    pub strict: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    OneToOne,
    ManyToOne,
    OneToMany,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validity {
    pub date: String,
    pub valid_from: String,
    pub valid_to: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    pub expr: Expr,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    Literal {
        value: Value,
    },
    Field {
        source: String,
        field: String,
        #[serde(default)]
        path: Vec<String>,
    },
    Feature {
        id: String,
    },
    Add {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Subtract {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Multiply {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// Representable so that documents fail with an explicit diagnostic, but
    /// division is deferred in format 0.1 and always rejected statically.
    Divide {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Equal {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Less {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    LessEqual {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    And {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Or {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Not {
        expr: Box<Expr>,
    },
    IsNull {
        expr: Box<Expr>,
    },
    /// Rate-validity predicate: `valid_from <= date < valid_to`, where a null
    /// `valid_to` is an explicit open-ended upper bound.
    ValidOn {
        date: Box<Expr>,
        valid_from: Box<Expr>,
        valid_to: Box<Expr>,
    },
    Coalesce {
        values: Vec<Expr>,
    },
    Case {
        condition: Box<Expr>,
        then: Box<Expr>,
        otherwise: Box<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        data_type: Type,
    },
    Quantize {
        expr: Box<Expr>,
        precision: u8,
        scale: u8,
        rounding: crate::scalar::Rounding,
    },
    Aggregate {
        relationship: String,
        function: String,
        #[serde(default)]
        field: Option<String>,
        #[serde(default)]
        filter: Option<Box<Expr>>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub id: String,
    pub anchor: String,
    pub columns: Vec<Column>,
    #[serde(default)]
    pub eligibility: Option<Expr>,
    #[serde(default)]
    pub filter: Option<Expr>,
    #[serde(default)]
    pub group_by: Vec<String>,
    #[serde(default)]
    pub detail: bool,
    #[serde(default)]
    pub aggregates: Vec<GroupAggregate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    pub expr: Expr,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub path: PathBuf,
    pub adapter: String,
    #[serde(default)]
    pub settings: BTreeMap<String, serde_json::Value>,
}
pub type Bindings = BTreeMap<String, Binding>;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReference {
    pub source: String,
    pub binding: String,
    pub fingerprint: String,
    pub location: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub fields: BTreeMap<String, Value>,
    pub reference: SourceReference,
    pub raw: BTreeMap<String, serde_json::Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub engine_bytes: usize,
    pub file_bytes: usize,
    pub input_bytes: usize,
    pub xlsx_bytes: usize,
    pub worksheet_cells: usize,
    pub field_bytes: usize,
    pub json_depth: usize,
    pub anchor_rows: usize,
    pub component_rows: usize,
    pub lookup_rows: usize,
    pub candidate_pairs: usize,
    pub output_bytes: usize,
    pub retained_bytes: usize,
    pub batch_rows: usize,
    pub batch_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            engine_bytes: 256 << 20,
            file_bytes: 64 << 20,
            input_bytes: 256 << 20,
            xlsx_bytes: 256 << 20,
            worksheet_cells: 1_000_000,
            field_bytes: 1 << 20,
            json_depth: 64,
            anchor_rows: 100_000,
            component_rows: 1_000_000,
            lookup_rows: 100_000,
            candidate_pairs: 1_000_000,
            output_bytes: 256 << 20,
            retained_bytes: 256 << 20,
            batch_rows: 8192,
            batch_bytes: 16 << 20,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub allowed_read_roots: Vec<PathBuf>,
    pub allowed_output_roots: Vec<PathBuf>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub diagnostic_values: bool,
}
/// A structured diagnostic record (decision #9). `code`, `phase` and `message`
/// are always present; the remaining fields locate and bound the problem. Values
/// are omitted unless `Policy::diagnostic_values` opts in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    /// Stable machine-readable code.
    pub code: String,
    pub phase: String,
    /// Human message. Never contains source data values.
    pub message: String,
    pub context: Option<String>,
    /// `error` (fails the run) or `warning` (recorded; the run continues).
    #[serde(default = "default_severity")]
    pub severity: String,
    /// Stable declaration ID the problem belongs to, such as `source:rates`,
    /// `relationship:a_rate`, `output:a_review` or `expression:<output>.<column>`.
    #[serde(default)]
    pub subject: Option<String>,
    /// ID of the check that established the problem, such as `data.identity`.
    #[serde(default)]
    pub check: Option<String>,
    /// Outputs whose population or values the problem affects.
    #[serde(default)]
    pub affected_outputs: Vec<String>,
    /// Number of affected records when known.
    #[serde(default)]
    pub affected_count: Option<u64>,
    /// `exact` when `affected_count` is the complete count, otherwise `unknown`.
    #[serde(default = "default_count_status")]
    pub count_status: String,
    /// At most [`Error::SAMPLE_CAP`] source references of affected records.
    #[serde(default)]
    pub samples: Vec<SourceReference>,
    /// True when more affected records exist than `samples` lists.
    #[serde(default)]
    pub samples_truncated: bool,
    /// What a user can do about it.
    #[serde(default)]
    pub remediation: Option<String>,
    /// Offending values, present only when the policy opted in to diagnostic values.
    #[serde(default)]
    pub values: Option<Vec<serde_json::Value>>,
}
fn default_severity() -> String {
    "error".into()
}
fn default_count_status() -> String {
    "unknown".into()
}
impl Error {
    /// Default cap on source-reference samples per diagnostic.
    pub const SAMPLE_CAP: usize = 20;
    pub fn new(code: &str, phase: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            phase: phase.into(),
            message: message.into(),
            context: None,
            severity: default_severity(),
            subject: None,
            check: None,
            affected_outputs: Vec::new(),
            affected_count: None,
            count_status: default_count_status(),
            samples: Vec::new(),
            samples_truncated: false,
            remediation: None,
            values: None,
        }
    }
    pub fn warning(mut self) -> Self {
        self.severity = "warning".into();
        self
    }
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }
    pub fn with_check(mut self, check: impl Into<String>) -> Self {
        self.check = Some(check.into());
        self
    }
    pub fn with_outputs(mut self, outputs: Vec<String>) -> Self {
        self.affected_outputs = outputs;
        self
    }
    /// Record an exact affected count and its first [`Error::SAMPLE_CAP`] references.
    pub fn with_affected<'a>(
        mut self,
        count: usize,
        references: impl IntoIterator<Item = &'a SourceReference>,
    ) -> Self {
        self.affected_count = Some(count as u64);
        self.count_status = "exact".into();
        self.samples = references
            .into_iter()
            .take(Self::SAMPLE_CAP)
            .cloned()
            .collect();
        self.samples_truncated = count > self.samples.len();
        self
    }
    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }
    /// Attach values only when `enabled` (the policy's explicit opt-in).
    pub fn with_values(mut self, enabled: bool, values: Vec<serde_json::Value>) -> Self {
        if enabled {
            self.values = Some(values);
        }
        self
    }
    /// Fill the check ID and remediation from the stable code when a producer
    /// did not set them, so every published diagnostic is a complete record.
    pub fn completed(mut self) -> Self {
        if self.check.is_none() {
            self.check = Some(default_check(&self.code, &self.phase).into());
        }
        if self.remediation.is_none() {
            self.remediation = Some(default_remediation(&self.code).into());
        }
        if self.samples.len() > Self::SAMPLE_CAP {
            self.samples.truncate(Self::SAMPLE_CAP);
            self.samples_truncated = true;
        }
        self
    }
}
/// The check that establishes a failure with this code.
pub fn default_check(code: &str, phase: &str) -> String {
    match code {
        "INVALID_VALUE"
        | "INVALID_DECIMAL"
        | "INVALID_DATE"
        | "TYPE_MISMATCH"
        | "NULL_REQUIRED"
        | "UNKNOWN_PATH"
        | "INVALID_JSON"
        | "DUPLICATE_JSON_KEY"
        | "INVALID_CSV"
        | "INVALID_XLSX"
        | "XLSX_QUANTIZATION_REQUIRED"
        | "XLSX_CELL_ERROR"
        | "FORMULA_CACHE_MISSING"
        | "FORMULA_CACHE_UNCERTAIN"
        | "ARITHMETIC_OVERFLOW" => "data.types".into(),
        "NULL_IDENTITY" | "DUPLICATE_IDENTITY" => "data.identity".into(),
        "CARDINALITY_VIOLATION" => "data.relationships".into(),
        "INVALID_INTERVAL" => "data.rate_validity".into(),
        "LOOKUP_AMBIGUOUS" => "lookup.cardinality".into(),
        "LOOKUP_UNMATCHED" => "lookup.coverage".into(),
        "ORPHAN_DETAIL" => "detail.parent".into(),
        "INVALID_PROJECTION" | "INVALID_BINDING" => "source_schema".into(),
        "POLICY_DENIED" | "SOURCE_ACCESS" | "MISSING_BINDING" | "SOURCE_CHANGED" => "access".into(),
        "RESOURCE_LIMIT" => "budget".into(),
        "CANCELLED" => "cancellation".into(),
        _ => phase.into(),
    }
}
/// Generic remediation for a stable code.
pub fn default_remediation(code: &str) -> &'static str {
    match code {
        "UNSUPPORTED_OPERATION"
        | "UNSUPPORTED_TYPE"
        | "UNSUPPORTED_VERSION"
        | "UNSUPPORTED_CAPABILITY"
        | "UNSUPPORTED_ADAPTER" => {
            "Remove or replace the unsupported declaration; this build does not implement it."
        }
        "INVALID_PATH" => {
            "Name the relationship path from the output anchor to the field explicitly; competing routes are never chosen implicitly."
        }
        "GRAIN_VIOLATION" => {
            "Use an explicit aggregate or a separate detail output for finer-grained records."
        }
        "MISSING_REFERENCE" => "Declare the referenced source, field, feature or relationship.",
        "TYPE_MISMATCH" | "PRECISION_OVERFLOW" => {
            "Declare matching types or add an explicit cast or quantization."
        }
        "NULL_IDENTITY" | "DUPLICATE_IDENTITY" => {
            "Correct the source records so every declared identity is present and unique, or declare a different identity."
        }
        "INVALID_INTERVAL" => {
            "Correct the validity interval so valid_from is before valid_to, or leave valid_to null for an open upper bound."
        }
        "CARDINALITY_VIOLATION" => {
            "Remove the duplicate or overlapping candidates, or drop `strict` to quarantine affected lines as ambiguous."
        }
        "LOOKUP_AMBIGUOUS" => {
            "Inspect the candidate evidence and remove duplicate or overlapping lookup records; affected lines keep null dependent values."
        }
        "LOOKUP_UNMATCHED" => {
            "Add the missing lookup records or correct the key values; affected lines keep null dependent values."
        }
        "ORPHAN_DETAIL" => "Correct the detail parent keys or supply the missing parent lines.",
        "INVALID_PROJECTION" | "INVALID_BINDING" => {
            "Align the declared projection and binding settings with the source's actual headers, sheets and paths."
        }
        "POLICY_DENIED" => "Move the file under an allowed root or extend the policy explicitly.",
        "RESOURCE_LIMIT" => {
            "Raise the named policy limit explicitly or reduce the selected population; results are never truncated."
        }
        "CANCELLED" => "Re-run when cancellation is no longer requested.",
        "MISSING_BINDING" | "SOURCE_ACCESS" | "SOURCE_CHANGED" => {
            "Bind every source to a readable, stable local file and run again."
        }
        _ => "Correct the declaration or source data identified by this diagnostic and run again.",
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupAggregate {
    pub name: String,
    pub function: String,
    pub expr: Expr,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledDefinition {
    pub content: serde_json::Value,
    pub content_hash: String,
    #[serde(default)]
    pub provenance: Option<String>,
}

/// In-memory catalog of reusable declarations. Bundling copies a declaration
/// into a recipe so the saved document never refers back to the catalog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub name: String,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
    #[serde(default)]
    pub features: Vec<Feature>,
}
