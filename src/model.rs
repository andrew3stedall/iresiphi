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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub phase: String,
    pub message: String,
    pub context: Option<String>,
}
impl Error {
    pub fn new(code: &str, phase: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            phase: phase.into(),
            message: message.into(),
            context: None,
        }
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
