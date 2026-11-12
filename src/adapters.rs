//! Local adapters preserve source observations and enforce admission before publication.
use crate::{
    model::*,
    scalar::{Decimal, Rounding},
};
use calamine::{Data, Reader, Xlsx};
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    fmt,
    fs::File,
    io::{Cursor, Read},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::fs::MetadataExt,
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};

fn err(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, "source", message)
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(err("CANCELLED", "source read cancelled"))
    } else {
        Ok(())
    }
}
fn limit(ok: bool, what: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(err("RESOURCE_LIMIT", what))
    }
}
fn io_error(e: impl fmt::Display) -> Error {
    err("SOURCE_ACCESS", e.to_string())
}

/// Open below an approved root without following any path-component symlink.
fn open_permitted(path: &Path, roots: &[std::path::PathBuf]) -> Result<File> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(io_error)?.join(path)
    };
    if absolute
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(err("POLICY_DENIED", "parent path components are forbidden"));
    }
    for root in roots {
        let root = std::fs::canonicalize(root).map_err(io_error)?;
        let Ok(relative) = absolute.strip_prefix(&root) else {
            continue;
        };
        let root_c = CString::new(root.as_os_str().as_encoded_bytes()).map_err(io_error)?;
        let fd = unsafe {
            libc::open(
                root_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let parts: Vec<_> = relative
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .collect();
        if parts.is_empty() {
            return Err(err("SOURCE_ACCESS", "source must be a regular file"));
        }
        for (index, part) in parts.iter().enumerate() {
            let Component::Normal(name) = part else {
                return Err(err("POLICY_DENIED", "invalid source path"));
            };
            let name = CString::new(name.as_encoded_bytes()).map_err(io_error)?;
            let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
            if index + 1 < parts.len() {
                flags |= libc::O_DIRECTORY;
            }
            let fd = unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(err(
                    "POLICY_DENIED",
                    std::io::Error::last_os_error().to_string(),
                ));
            }
            file = unsafe { File::from_raw_fd(fd) };
        }
        if !file.metadata().map_err(io_error)?.is_file() {
            return Err(err("SOURCE_ACCESS", "source must be a regular file"));
        }
        return Ok(file);
    }
    Err(err(
        "POLICY_DENIED",
        "source lies outside allowed read roots",
    ))
}

fn read_bytes(
    binding: &Binding,
    policy: &Policy,
    total: &mut usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    let mut file = open_permitted(&binding.path, &policy.allowed_read_roots)?;
    let before = file.metadata().map_err(io_error)?;
    limit(
        before.len() <= policy.limits.file_bytes as u64,
        "encoded file bytes",
    )?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        check_cancel(cancel)?;
        let n = file.read(&mut chunk).map_err(io_error)?;
        if n == 0 {
            break;
        }
        limit(
            bytes.len().saturating_add(n) <= policy.limits.file_bytes,
            "encoded file bytes",
        )?;
        *total = total.saturating_add(n);
        limit(
            *total <= policy.limits.input_bytes,
            "aggregate encoded input bytes",
        )?;
        bytes.extend_from_slice(&chunk[..n]);
    }
    let after = file.metadata().map_err(io_error)?;
    if (
        before.len(),
        before.mtime(),
        before.mtime_nsec(),
        before.ctime(),
        before.ctime_nsec(),
    ) != (
        after.len(),
        after.mtime(),
        after.mtime_nsec(),
        after.ctime(),
        after.ctime_nsec(),
    ) {
        return Err(err(
            "SOURCE_CHANGED",
            "source changed while reading; materialization is not certified",
        ));
    }
    Ok(bytes)
}

/// Read immutable materializations. The public executor converts these owned rows to typed batches.
/// This adapter boundary is bounded materialization, not a zero-copy or streaming guarantee.
fn materialize_sources(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Vec<Row>>> {
    let mut result = BTreeMap::new();
    let mut total = 0;
    let mut expanded = 0;
    let mut retained = 0usize;
    for source in &recipe.sources {
        check_cancel(cancel)?;
        let binding = bindings
            .get(&source.id)
            .ok_or_else(|| err("MISSING_BINDING", format!("missing binding {}", source.id)))?;
        let bytes = read_bytes(binding, policy, &mut total, cancel)?;
        let fingerprint = crate::validation::hex(&Sha256::digest(&bytes));
        let reference = SourceReference {
            source: source.id.clone(),
            binding: source.id.clone(),
            fingerprint,
            location: String::new(),
        };
        let max = if recipe
            .outputs
            .iter()
            .any(|o| o.anchor == source.id && !o.detail)
        {
            policy.limits.anchor_rows
        } else if recipe
            .relationships
            .iter()
            .any(|r| r.to == source.id && r.cardinality == Cardinality::OneToMany)
        {
            policy.limits.component_rows
        } else {
            policy.limits.lookup_rows
        };
        let mut admission = policy.clone();
        admission.limits.lookup_rows = max;
        admission.limits.retained_bytes = policy.limits.retained_bytes.saturating_sub(retained);
        let mut rows = match binding.adapter.as_str() {
            "json" => read_json(source, &bytes, &reference, &admission, cancel)?,
            "csv" => read_csv(source, binding, &bytes, &reference, &admission, cancel)?,
            "xlsx" => read_xlsx(
                source,
                binding,
                &bytes,
                &reference,
                &admission,
                &mut expanded,
                cancel,
            )?,
            _ => return Err(err("UNSUPPORTED_CAPABILITY", "unsupported source adapter")),
        };
        let max = if recipe
            .outputs
            .iter()
            .any(|o| o.anchor == source.id && !o.detail)
        {
            policy.limits.anchor_rows
        } else if recipe
            .relationships
            .iter()
            .any(|r| r.to == source.id && r.cardinality == Cardinality::OneToMany)
        {
            policy.limits.component_rows
        } else {
            policy.limits.lookup_rows
        };
        limit(rows.len() <= max, "source row count")?;
        let mut identities = BTreeSet::new();
        let mut batch_bytes = 0usize;
        for (i, row) in rows.iter_mut().enumerate() {
            check_cancel(cancel)?;
            let key: Vec<_> = source
                .identity
                .iter()
                .map(|id| row.fields.get(id).cloned().unwrap_or(Value::Null))
                .collect();
            if key.iter().any(|v| matches!(v, Value::Null)) {
                return Err(err(
                    "NULL_IDENTITY",
                    format!("null identity in {}", source.id),
                ));
            }
            let key = serde_json::to_string(&key).map_err(io_error)?;
            if !identities.insert(key) {
                return Err(err(
                    "DUPLICATE_IDENTITY",
                    format!("duplicate identity in {}", source.id),
                ));
            }
            let size = serde_json::to_vec(row).map_err(io_error)?.len();
            limit(
                size <= policy.limits.batch_bytes,
                "single decoded record bytes",
            )?;
            if i % policy.limits.batch_rows.max(1) == 0 {
                batch_bytes = 0;
            }
            batch_bytes = batch_bytes.saturating_add(size);
            limit(
                batch_bytes <= policy.limits.batch_bytes,
                "decoded batch bytes",
            )?;
            retained = retained.saturating_add(size);
            limit(
                retained <= policy.limits.retained_bytes,
                "retained source materialization bytes",
            )?;
        }
        result.insert(source.id.clone(), rows);
    }
    Ok(result)
}

struct RowCollector<'a> {
    rows: Vec<Row>,
    bytes: usize,
    batch_bytes: usize,
    policy: &'a Policy,
}
impl<'a> RowCollector<'a> {
    fn new(policy: &'a Policy) -> Self {
        Self {
            rows: Vec::new(),
            bytes: 0,
            batch_bytes: 0,
            policy,
        }
    }
    fn push(&mut self, row: Row) -> Result<()> {
        limit(
            self.rows.len() < self.policy.limits.lookup_rows,
            "source row count",
        )?;
        let size = serde_json::to_vec(&row).map_err(io_error)?.len();
        if self.rows.len() % self.policy.limits.batch_rows.max(1) == 0 {
            self.batch_bytes = 0;
        }
        self.batch_bytes = self.batch_bytes.saturating_add(size);
        self.bytes = self.bytes.saturating_add(size);
        limit(
            self.batch_bytes <= self.policy.limits.batch_bytes,
            "decoded batch bytes",
        )?;
        limit(
            self.bytes <= self.policy.limits.retained_bytes,
            "retained source materialization bytes",
        )?;
        self.rows.push(row);
        Ok(())
    }
}

fn value_from_text(text: Option<&str>, field: &Field) -> Result<Value> {
    let Some(text) = text else {
        return if field.nullable {
            Ok(Value::Null)
        } else {
            Err(err("NULL_REQUIRED", "required field is missing or null"))
        };
    };
    let bad = || {
        err(
            "INVALID_VALUE",
            "source value does not satisfy declared type",
        )
    };
    Ok(match field.data_type {
        Type::String => Value::String { value: text.into() },
        Type::Boolean => Value::Boolean {
            value: match text {
                "true" => true,
                "false" => false,
                _ => return Err(bad()),
            },
        },
        Type::Int64 => {
            if text.is_empty() || text.trim() != text {
                return Err(bad());
            }
            let n = text.parse::<i64>().map_err(|_| bad())?;
            Value::Int64 {
                value: n.to_string(),
            }
        }
        Type::Decimal { precision, scale } => {
            let fixed = expand_decimal(text)?;
            let d = Decimal::parse(&fixed, precision, scale)
                .map_err(|e| err("INVALID_DECIMAL", e.to_string()))?;
            Value::Decimal {
                value: d.to_string(),
                precision,
                scale,
            }
        }
        Type::Date => {
            if text.len() != 10 || text.as_bytes()[4] != b'-' || text.as_bytes()[7] != b'-' {
                return Err(bad());
            }
            let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| bad())?;
            if date.format("%Y-%m-%d").to_string() != text {
                return Err(bad());
            }
            Value::Date { value: text.into() }
        }
    })
}

fn expand_decimal(text: &str) -> Result<String> {
    let Some(at) = text.find(['e', 'E']) else {
        return Ok(text.into());
    };
    let exponent = text[at + 1..]
        .parse::<i32>()
        .map_err(|_| err("INVALID_DECIMAL", "invalid exponent"))?;
    if exponent.unsigned_abs() > 1024 {
        return Err(err(
            "INVALID_DECIMAL",
            "decimal exponent exceeds representable range",
        ));
    }
    let mantissa = &text[..at];
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches(['-', '+']);
    let mut split = mantissa.split('.');
    let integer = split.next().unwrap_or("");
    let fraction = split.next().unwrap_or("");
    if integer.is_empty()
        || split.next().is_some()
        || !integer
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(err("INVALID_DECIMAL", "invalid exponent mantissa"));
    }
    let digits = format!("{integer}{fraction}");
    let point = integer.len() as i32 + exponent;
    let value = if point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..point as usize],
            &digits[point as usize..]
        )
    };
    Ok(if negative { format!("-{value}") } else { value })
}

// A validation pass catches duplicate keys even in unprojected subtrees. Parsing into
// serde_json::Value alone would silently overwrite them.
struct UniqueJson;
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<UniqueJson, M::Error> {
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    map.next_value::<UniqueJson>()?;
                }
                Ok(UniqueJson)
            }
            fn visit_seq<S: SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> std::result::Result<UniqueJson, S::Error> {
                while seq.next_element::<UniqueJson>()?.is_some() {}
                Ok(UniqueJson)
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<UniqueJson, E> {
                Ok(UniqueJson)
            }
        }
        d.deserialize_any(V)
    }
}
fn json_preflight(bytes: &[u8], limits: &Limits, cancel: &AtomicBool) -> Result<()> {
    let (mut depth, mut token) = (0usize, 0usize);
    let (mut string, mut escaped) = (false, false);
    for (i, &b) in bytes.iter().enumerate() {
        if i % 65536 == 0 {
            check_cancel(cancel)?;
        }
        if string {
            token += 1;
            if escaped {
                escaped = false
            } else if b == b'\\' {
                escaped = true
            } else if b == b'"' {
                string = false;
                token = 0;
            }
        } else {
            match b {
                b'"' => {
                    string = true;
                    token = 0
                }
                b'{' | b'[' => {
                    depth += 1;
                    token = 0;
                    limit(depth <= limits.json_depth, "JSON nesting depth")?
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    token = 0
                }
                b',' | b':' | b' ' | b'\n' | b'\r' | b'\t' => token = 0,
                _ => token += 1,
            }
        }
        limit(token <= limits.field_bytes, "JSON token bytes")?;
    }
    Ok(())
}
fn read_json(
    source: &Source,
    bytes: &[u8],
    reference: &SourceReference,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<Vec<Row>> {
    json_preflight(bytes, &policy.limits, cancel)?;
    serde_json::from_slice::<UniqueJson>(bytes).map_err(|e| {
        err(
            if e.to_string().contains("duplicate JSON key") {
                "DUPLICATE_JSON_KEY"
            } else {
                "INVALID_JSON"
            },
            e.to_string(),
        )
    })?;
    let root: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| err("INVALID_JSON", e.to_string()))?;
    let records: Vec<&serde_json::Value> = match &source.projection.array {
        Some(path) => root
            .pointer(path)
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                err(
                    "INVALID_PROJECTION",
                    "declared JSON array is missing or not an array",
                )
            })?
            .iter()
            .collect(),
        None => vec![&root],
    };
    let mut rows = RowCollector::new(policy);
    for (index, record) in records.into_iter().enumerate() {
        check_cancel(cancel)?;
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
        row.reference.location = source
            .projection
            .array
            .as_ref()
            .map(|p| format!("{p}/{index}"))
            .unwrap_or_default();
        for (name, field) in &source.projection.fields {
            let raw = if field.header { &root } else { record }
                .pointer(&field.path)
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let text = match &raw {
                serde_json::Value::Null => None,
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n)
                    if matches!(field.data_type, Type::Int64 | Type::Decimal { .. }) =>
                {
                    Some(n.to_string())
                }
                serde_json::Value::Bool(b) if field.data_type == Type::Boolean => {
                    Some(b.to_string())
                }
                _ => {
                    return Err(err(
                        "INVALID_VALUE",
                        "JSON value type differs from declared scalar type",
                    ));
                }
            };
            if matches!(field.data_type, Type::String) && !raw.is_string() && !raw.is_null() {
                return Err(err("INVALID_VALUE", "JSON string required"));
            }
            let value = value_from_text(text.as_deref(), field)?;
            row.raw.insert(name.clone(), raw);
            row.fields.insert(name.clone(), value);
        }
        rows.push(row)?;
    }
    Ok(rows.rows)
}

fn setting_byte(binding: &Binding, key: &str, default: u8) -> Result<u8> {
    match binding.settings.get(key) {
        None => Ok(default),
        Some(serde_json::Value::String(s)) if s.len() == 1 && s.is_ascii() => Ok(s.as_bytes()[0]),
        _ => Err(err(
            "INVALID_BINDING",
            format!("{key} must be one ASCII byte"),
        )),
    }
}
fn read_csv(
    source: &Source,
    binding: &Binding,
    bytes: &[u8],
    reference: &SourceReference,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<Vec<Row>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(setting_byte(binding, "delimiter", b',')?)
        .quote(setting_byte(binding, "quote", b'"')?)
        .has_headers(true)
        .flexible(false)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|e| err("INVALID_CSV", e.to_string()))?
        .clone();
    let mut seen = BTreeSet::new();
    if headers.iter().any(|h| !seen.insert(h)) {
        return Err(err("INVALID_CSV", "duplicate header"));
    }
    let columns: Vec<_> = source
        .projection
        .fields
        .iter()
        .map(|(name, f)| {
            let path = f.path.strip_prefix('/').unwrap_or(&f.path);
            headers
                .iter()
                .position(|h| h == path)
                .map(|i| (name, f, i))
                .ok_or_else(|| err("INVALID_PROJECTION", format!("missing CSV column {path}")))
        })
        .collect::<Result<_>>()?;
    let nulls = binding
        .settings
        .get("null_tokens")
        .and_then(|v| v.as_array())
        .map(|v| v.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["\\N"]);
    let mut rows = RowCollector::new(policy);
    for (index, record) in reader.records().enumerate() {
        check_cancel(cancel)?;
        let record = record.map_err(|e| err("INVALID_CSV", e.to_string()))?;
        for text in &record {
            limit(text.len() <= policy.limits.field_bytes, "CSV field bytes")?;
        }
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
        row.reference.location = format!("record:{}", index + 2);
        for (name, field, col) in &columns {
            let original = &record[*col];
            let decoded = if original.starts_with("\\\\") {
                &original[1..]
            } else {
                original
            };
            let text = if nulls.contains(&original) {
                None
            } else {
                Some(decoded)
            };
            row.fields
                .insert((*name).clone(), value_from_text(text, field)?);
            row.raw
                .insert((*name).clone(), serde_json::Value::String(original.into()));
        }
        rows.push(row)?;
    }
    Ok(rows.rows)
}

fn xml_attr(event: &quick_xml::events::BytesStart<'_>, name: &str) -> Result<Option<String>> {
    for attribute in event.attributes() {
        let a = attribute.map_err(|e| err("INVALID_XLSX", e.to_string()))?;
        if a.key.as_ref() == name {
            return Ok(Some(
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|e| err("INVALID_XLSX", e.to_string()))?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}
fn coordinate(text: &str) -> Result<(usize, usize)> {
    let mut col = 0usize;
    let mut split = 0;
    for b in text.bytes() {
        if b.is_ascii_uppercase() {
            col = col
                .checked_mul(26)
                .and_then(|x| x.checked_add((b - b'A' + 1) as usize))
                .ok_or_else(|| err("RESOURCE_LIMIT", "worksheet coordinate overflow"))?;
            split += 1;
        } else {
            break;
        }
    }
    let row = text[split..]
        .parse::<usize>()
        .map_err(|_| err("INVALID_XLSX", "invalid cell coordinate"))?;
    if col == 0 || row == 0 {
        return Err(err("INVALID_XLSX", "invalid cell coordinate"));
    }
    Ok((row, col))
}
/// Inspect archive actual decompression and sparse-cell extents before calamine allocates a range.
fn xlsx_preflight(
    bytes: &[u8],
    limits: &Limits,
    expanded: &mut usize,
    cancel: &AtomicBool,
) -> Result<bool> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| err("INVALID_XLSX", e.to_string()))?;
    let mut epoch1904 = false;
    for i in 0..archive.len() {
        check_cancel(cancel)?;
        let mut entry = archive
            .by_index(i)
            .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
        limit(
            entry.size() <= limits.xlsx_bytes as u64,
            "XLSX advertised entry bytes",
        )?;
        let name = entry
            .name()
            .map_err(|e| err("INVALID_XLSX", e.to_string()))?
            .into_owned();
        let mut data = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            check_cancel(cancel)?;
            let n = entry.read(&mut chunk).map_err(io_error)?;
            if n == 0 {
                break;
            }
            *expanded = expanded.saturating_add(n);
            limit(
                *expanded <= limits.xlsx_bytes,
                "aggregate actual XLSX decompression bytes",
            )?;
            data.extend_from_slice(&chunk[..n]);
        }
        if !name.ends_with(".xml") {
            continue;
        }
        let worksheet = name.starts_with("xl/worksheets/");
        let workbook = name == "xl/workbook.xml";
        let mut reader = quick_xml::Reader::from_reader(data.as_slice());
        let (mut max_row, mut max_col, mut cells) = (0usize, 0usize, 0usize);
        let mut cell_type = String::new();
        let (mut in_cell, mut formula, mut cache, mut cache_text, mut in_value) =
            (false, false, false, false, false);
        loop {
            check_cancel(cancel)?;
            match reader
                .read_event()
                .map_err(|e| err("INVALID_XLSX", e.to_string()))?
            {
                quick_xml::events::Event::Start(e) | quick_xml::events::Event::Empty(e) => {
                    match e.local_name().as_ref() {
                        "workbookPr" if workbook => {
                            epoch1904 =
                                matches!(xml_attr(&e, "date1904")?.as_deref(), Some("1" | "true"))
                        }
                        "dimension" if worksheet => {
                            if let Some(reference) = xml_attr(&e, "ref")? {
                                let end = reference.rsplit(':').next().unwrap_or(&reference);
                                let (r, c) = coordinate(end)?;
                                limit(
                                    r.checked_mul(c)
                                        .is_some_and(|n| n <= limits.worksheet_cells),
                                    "worksheet declared range cells",
                                )?;
                            }
                        }
                        "c" if worksheet => {
                            in_cell = true;
                            formula = false;
                            cache = false;
                            cache_text = false;
                            in_value = false;
                            cell_type = xml_attr(&e, "t")?.unwrap_or_default();
                            cells += 1;
                            let reference = xml_attr(&e, "r")?.ok_or_else(|| {
                                err(
                                    "INVALID_XLSX",
                                    "cell coordinates required for bounded range preflight",
                                )
                            })?;
                            let (r, c) = coordinate(&reference)?;
                            max_row = max_row.max(r);
                            max_col = max_col.max(c);
                            limit(
                                cells <= limits.worksheet_cells
                                    && max_row
                                        .checked_mul(max_col)
                                        .is_some_and(|n| n <= limits.worksheet_cells),
                                "worksheet cell/range limit",
                            )?;
                        }
                        "f" if in_cell => formula = true,
                        "v" if in_cell => {
                            cache = true;
                            in_value = true;
                        }
                        _ => {}
                    }
                }
                quick_xml::events::Event::Text(e) => {
                    limit(e.len() <= limits.field_bytes, "XLSX XML token bytes")?;
                    if in_value && !e.is_empty() {
                        cache_text = true;
                    }
                }
                quick_xml::events::Event::End(e) => match e.local_name().as_ref() {
                    "v" => in_value = false,
                    "c" => {
                        if formula && (!cache || (!cache_text && cell_type != "str")) {
                            return Err(err(
                                "FORMULA_CACHE_MISSING",
                                "formula requires an unambiguous stored cached value; formulas are not evaluated",
                            ));
                        }
                        in_cell = false;
                    }
                    _ => {}
                },
                quick_xml::events::Event::DocType(_) => {
                    return Err(err("INVALID_XLSX", "XML document types are forbidden"));
                }
                quick_xml::events::Event::Eof => break,
                _ => {}
            }
        }
    }
    Ok(epoch1904)
}
fn cell_label(mut col: u32, row: u32) -> String {
    let mut label = String::new();
    loop {
        label.insert(0, (b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    format!("{label}{}", row + 1)
}
fn excel_date(serial: f64, epoch1904: bool) -> Result<Value> {
    if !serial.is_finite()
        || serial.fract() != 0.0
        || serial < 0.0
        || serial > 3_000_000.0
        || (!epoch1904 && serial == 60.0)
    {
        return Err(err(
            "INVALID_DATE",
            "invalid or fractional Excel date serial",
        ));
    }
    let epoch = chrono::NaiveDate::from_ymd_opt(
        if epoch1904 { 1904 } else { 1899 },
        if epoch1904 { 1 } else { 12 },
        if epoch1904 { 1 } else { 31 },
    )
    .unwrap();
    let days = serial as i64 - if !epoch1904 && serial > 60.0 { 1 } else { 0 };
    let date = epoch
        .checked_add_signed(chrono::Duration::days(days))
        .ok_or_else(|| err("INVALID_DATE", "Excel date out of range"))?;
    let text = date.format("%Y-%m-%d").to_string();
    if text.len() != 10 {
        return Err(err(
            "INVALID_DATE",
            "Excel date outside ISO four-digit year range",
        ));
    }
    Ok(Value::Date { value: text })
}
fn numeric_decimal(number: f64, field: &Field, rounding: &str) -> Result<Value> {
    if !number.is_finite() {
        return Err(err("INVALID_DECIMAL", "nonfinite XLSX numeric cell"));
    }
    let mode = match rounding {
        "half_even" => Rounding::HalfEven,
        "toward_zero" => Rounding::TowardZero,
        _ => {
            return Err(err(
                "INVALID_BINDING",
                "numeric XLSX decimal policy must be half_even or toward_zero",
            ));
        }
    };
    let text = expand_decimal(&number.to_string())?;
    let source_scale = text.split('.').nth(1).map(str::len).unwrap_or(0);
    let Type::Decimal { precision, scale } = field.data_type else {
        unreachable!()
    };
    let digits = text.replace('.', "");
    let mut coefficient = num_bigint::BigInt::parse_bytes(digits.as_bytes(), 10)
        .ok_or_else(|| err("INVALID_DECIMAL", "invalid observed number"))?;
    if source_scale > scale as usize {
        let divisor = num_bigint::BigInt::from(10).pow((source_scale - scale as usize) as u32);
        let remainder = &coefficient % &divisor;
        let twice = if remainder < num_bigint::BigInt::from(0) {
            -&remainder * 2
        } else {
            &remainder * 2
        };
        let negative = coefficient < num_bigint::BigInt::from(0);
        coefficient /= &divisor;
        if mode == Rounding::HalfEven
            && (twice > divisor
                || (twice == divisor && (&coefficient % 2) != num_bigint::BigInt::from(0)))
        {
            coefficient += if negative { -1 } else { 1 };
        }
    } else {
        coefficient *= num_bigint::BigInt::from(10).pow((scale as usize - source_scale) as u32);
    }
    let negative = coefficient < num_bigint::BigInt::from(0);
    let digits = coefficient.to_string().trim_start_matches('-').to_owned();
    let padded = format!("{:0>width$}", digits, width = scale as usize + 1);
    let split = padded.len() - scale as usize;
    let text = if scale == 0 {
        padded
    } else {
        format!("{}.{}", &padded[..split], &padded[split..])
    };
    let text = if negative { format!("-{text}") } else { text };
    let decimal = Decimal::parse(&text, precision, scale)
        .map_err(|e| err("INVALID_DECIMAL", e.to_string()))?;
    Ok(Value::Decimal {
        value: decimal.to_string(),
        precision,
        scale,
    })
}
fn read_xlsx(
    source: &Source,
    binding: &Binding,
    bytes: &[u8],
    reference: &SourceReference,
    policy: &Policy,
    expanded: &mut usize,
    cancel: &AtomicBool,
) -> Result<Vec<Row>> {
    let epoch1904 = xlsx_preflight(bytes, &policy.limits, expanded, cancel)?;
    let sheet = binding
        .settings
        .get("sheet")
        .and_then(|v| v.as_str())
        .ok_or_else(|| err("INVALID_BINDING", "XLSX sheet must be explicitly named"))?;
    let header_row = binding
        .settings
        .get("header_row")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            err(
                "INVALID_BINDING",
                "XLSX one-based header_row must be declared",
            )
        })?;
    if header_row == 0 || header_row > u32::MAX as u64 {
        return Err(err("INVALID_BINDING", "header_row out of range"));
    }
    let header_row = header_row as u32 - 1;
    let mut book: Xlsx<_> = calamine::open_workbook_from_rs(Cursor::new(bytes))
        .map_err(|e: calamine::XlsxError| err("INVALID_XLSX", e.to_string()))?;
    let range = book
        .worksheet_range(sheet)
        .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
    let formulas = book
        .worksheet_formula(sheet)
        .map_err(|e| err("INVALID_XLSX", e.to_string()))?;
    let Some((_, end_col)) = range.end() else {
        return Err(err("INVALID_PROJECTION", "worksheet has no header"));
    };
    let mut headers = BTreeMap::new();
    for col in 0..=end_col {
        if let Some(Data::String(name)) = range.get_value((header_row, col)) {
            if headers.insert(name.clone(), col).is_some() {
                return Err(err("INVALID_XLSX", "duplicate worksheet header"));
            }
        }
    }
    let columns: Vec<_> = source
        .projection
        .fields
        .iter()
        .map(|(name, f)| {
            let header = f.path.strip_prefix('/').unwrap_or(&f.path);
            headers
                .get(header)
                .map(|&col| (name, f, col))
                .ok_or_else(|| {
                    err(
                        "INVALID_PROJECTION",
                        format!("missing XLSX column {header}"),
                    )
                })
        })
        .collect::<Result<_>>()?;
    let mut rows = RowCollector::new(policy);
    let end_row = range.end().unwrap().0;
    for row_index in header_row + 1..=end_row {
        check_cancel(cancel)?;
        let mut row = Row {
            fields: BTreeMap::new(),
            raw: BTreeMap::new(),
            reference: reference.clone(),
        };
        row.reference.location = format!("{sheet}!{}", row_index + 1);
        for (name, field, col) in &columns {
            let cell = range.get_value((row_index, *col)).unwrap_or(&Data::Empty);
            let policy_value = binding
                .settings
                .get("decimal_quantization")
                .and_then(|v| v.get(*name))
                .and_then(|v| v.as_str());
            let value = match cell {
                Data::Empty => value_from_text(None, field)?,
                Data::Error(_) => return Err(err("XLSX_CELL_ERROR", "spreadsheet error cell")),
                Data::String(text) => {
                    limit(text.len() <= policy.limits.field_bytes, "XLSX field bytes")?;
                    value_from_text(Some(text), field)?
                }
                Data::Bool(value) if field.data_type == Type::Boolean => {
                    Value::Boolean { value: *value }
                }
                Data::Int(value) if field.data_type == Type::Int64 => Value::Int64 {
                    value: value.to_string(),
                },
                Data::Float(value) if field.data_type == Type::Int64 => {
                    if !value.is_finite()
                        || value.fract() != 0.0
                        || *value < i64::MIN as f64
                        || *value >= 9223372036854775808.0
                    {
                        return Err(err(
                            "INVALID_VALUE",
                            "XLSX integer is out of range or fractional",
                        ));
                    }
                    Value::Int64 {
                        value: (*value as i64).to_string(),
                    }
                }
                Data::Int(value) if matches!(field.data_type, Type::Decimal { .. }) => {
                    value_from_text(Some(&value.to_string()), field)?
                }
                Data::Float(value) if matches!(field.data_type, Type::Decimal { .. }) => {
                    numeric_decimal(
                        *value,
                        field,
                        policy_value.ok_or_else(|| {
                            err(
                                "XLSX_QUANTIZATION_REQUIRED",
                                "numeric decimal cell requires explicit quantization policy",
                            )
                        })?,
                    )?
                }
                Data::DateTime(value) if field.data_type == Type::Date => {
                    excel_date(value.as_f64(), epoch1904)?
                }
                Data::DateTimeIso(value) if field.data_type == Type::Date => {
                    value_from_text(Some(value), field)?
                }
                Data::Int(value) if field.data_type == Type::Date => {
                    if !binding
                        .settings
                        .get("excel_dates")
                        .and_then(|v| v.as_array())
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some(name.as_str())))
                    {
                        return Err(err(
                            "INVALID_BINDING",
                            "numeric date requires excel_dates field declaration",
                        ));
                    }
                    excel_date(*value as f64, epoch1904)?
                }
                Data::Float(value) if field.data_type == Type::Date => {
                    if !binding
                        .settings
                        .get("excel_dates")
                        .and_then(|v| v.as_array())
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some(name.as_str())))
                    {
                        return Err(err(
                            "INVALID_BINDING",
                            "numeric date requires excel_dates field declaration",
                        ));
                    }
                    excel_date(*value, epoch1904)?
                }
                _ => {
                    return Err(err(
                        "INVALID_VALUE",
                        "XLSX cell type differs from declared field",
                    ));
                }
            };
            let formula = formulas
                .get_value((row_index, *col))
                .filter(|s| !s.is_empty());
            row.raw.insert((*name).clone(),serde_json::json!({"cell":format!("{sheet}!{}",cell_label(*col,row_index)),"observed":cell.to_string(),"cell_type":format!("{:?}",std::mem::discriminant(cell)),"formula":formula,"quantization":policy_value,"date_system":if epoch1904{"1904"}else{"1900"}}));
            row.fields.insert((*name).clone(), value);
        }
        rows.push(row)?;
    }
    Ok(rows.rows)
}

/// One owned typed batch and its source observations. Exact decimals and dates use
/// Arrow Decimal128 and Date32; evidence remains separate from calculation arrays.
pub struct AdapterBatch {
    pub batch: datafusion::arrow::record_batch::RecordBatch,
    pub observations: Vec<Row>,
}

/// Pull delivery over an admitted source materialization. Only the requested batch
/// is converted to Arrow. Input parsing currently materializes the bounded source.
pub struct SourceBatches<'a> {
    cancel: &'a AtomicBool,
    rows: std::vec::IntoIter<Row>,
    fields: Vec<(String, Field)>,
    batch_rows: usize,
}
impl Iterator for SourceBatches<'_> {
    type Item = Result<AdapterBatch>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Err(error) = check_cancel(self.cancel) {
            if self.rows.len() == 0 {
                return None;
            }
            self.rows = Vec::new().into_iter();
            return Some(Err(error));
        }
        let observations: Vec<_> = self.rows.by_ref().take(self.batch_rows).collect();
        if observations.is_empty() {
            return None;
        }
        Some(typed_batch(&self.fields, observations))
    }
}

fn typed_batch(fields: &[(String, Field)], observations: Vec<Row>) -> Result<AdapterBatch> {
    use datafusion::arrow::{
        array::{ArrayRef, BooleanArray, Date32Array, Decimal128Array, Int64Array, StringArray},
        datatypes::{DataType, Field as ArrowField, Schema},
        record_batch::RecordBatch,
    };
    use std::sync::Arc;
    let mut arrays: Vec<ArrayRef> = Vec::new();
    let mut schema = Vec::new();
    for (name, field) in fields {
        let values: Vec<_> = observations
            .iter()
            .map(|r| r.fields.get(name).unwrap_or(&Value::Null))
            .collect();
        let (data_type, array): (DataType, ArrayRef) = match field.data_type {
            Type::Boolean => (
                DataType::Boolean,
                Arc::new(BooleanArray::from(
                    values
                        .iter()
                        .map(|v| match v {
                            Value::Boolean { value } => Some(*value),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )),
            ),
            Type::Int64 => {
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Int64 { value } => value
                            .parse::<i64>()
                            .map(Some)
                            .map_err(|_| err("INVALID_VALUE", "integer transport")),
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                (DataType::Int64, Arc::new(Int64Array::from(values)))
            }
            Type::String => (
                DataType::Utf8,
                Arc::new(StringArray::from(
                    values
                        .iter()
                        .map(|v| match v {
                            Value::String { value } => Some(value.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )),
            ),
            Type::Decimal { precision, scale } => {
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Decimal { value, .. } => Decimal::parse(value, precision, scale)
                            .map(|d| Some(d.coefficient))
                            .map_err(|e| err("INVALID_DECIMAL", e.to_string())),
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                let array = Decimal128Array::from(values)
                    .with_precision_and_scale(precision, scale as i8)
                    .map_err(|e| err("INVALID_SCHEMA", e.to_string()))?;
                (
                    DataType::Decimal128(precision, scale as i8),
                    Arc::new(array),
                )
            }
            Type::Date => {
                let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
                let values = values
                    .iter()
                    .map(|v| match v {
                        Value::Date { value } => {
                            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                                .map(|d| Some(d.signed_duration_since(epoch).num_days() as i32))
                                .map_err(|_| err("INVALID_DATE", "date transport"))
                        }
                        _ => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                (DataType::Date32, Arc::new(Date32Array::from(values)))
            }
        };
        schema.push(ArrowField::new(name, data_type, field.nullable));
        arrays.push(array);
    }
    let batch = RecordBatch::try_new(Arc::new(Schema::new(schema)), arrays)
        .map_err(|e| err("INVALID_SCHEMA", e.to_string()))?;
    Ok(AdapterBatch {
        batch,
        observations,
    })
}

/// Open all requested local sources as owned, typed pull-delivery iterators.
/// Admission/parsing is eager; this is not a streaming parser or whole-process RSS bound.
pub fn read_batches<'a>(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &'a AtomicBool,
) -> Result<BTreeMap<String, SourceBatches<'a>>> {
    let rows = materialize_sources(recipe, bindings, policy, cancel)?;
    rows.into_iter()
        .map(|(id, rows)| {
            let source = recipe
                .sources
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| err("INVALID_SOURCE", "missing source declaration"))?;
            Ok((
                id,
                SourceBatches {
                    cancel,
                    rows: rows.into_iter(),
                    fields: source
                        .projection
                        .fields
                        .iter()
                        .map(|(n, f)| (n.clone(), f.clone()))
                        .collect(),
                    batch_rows: policy.limits.batch_rows.max(1),
                },
            ))
        })
        .collect()
}

/// Compatibility materializer for the local executor; each source crosses the
/// same typed-batch boundary before its observations are retained for evidence.
pub fn read_sources(
    recipe: &Recipe,
    bindings: &Bindings,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<BTreeMap<String, Vec<Row>>> {
    let sources = read_batches(recipe, bindings, policy, cancel)?;
    let mut result = BTreeMap::new();
    for (id, batches) in sources {
        let mut rows = Vec::new();
        for batch in batches {
            check_cancel(cancel)?;
            rows.extend(batch?.observations);
        }
        result.insert(id, rows);
    }
    Ok(result)
}
