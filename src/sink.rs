//! Linux local bundles: descriptor-relative staging and no-replace publication.
use crate::model::{Error, Policy, Result, Value};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};

fn fail(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, "delivery", message)
}
fn io_error(error: std::io::Error) -> Error {
    fail("OUTPUT_IO", error.to_string())
}
fn name(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).map_err(|_| fail("POLICY_DENIED", "path contains NUL"))
}
fn directory_at(parent: i32, name: &CString) -> Result<File> {
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn approved_parent(destination: &Path, policy: &Policy) -> Result<(File, CString)> {
    if !destination.is_absolute()
        || destination
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(fail(
            "POLICY_DENIED",
            "output must be an absolute path without parent traversal",
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| fail("POLICY_DENIED", "output needs a parent"))?;
    let approved = policy.allowed_output_roots.iter().any(|root| {
        root.is_absolute()
            && parent.starts_with(root)
            && !root.components().any(|c| matches!(c, Component::ParentDir))
    });
    if !approved {
        return Err(fail("POLICY_DENIED", "output is outside approved roots"));
    }
    let mut directory = directory_at(libc::AT_FDCWD, &name(b"/")?)?;
    for component in parent.components() {
        if let Component::Normal(part) = component {
            directory = directory_at(directory.as_raw_fd(), &name(part.as_bytes())?)?;
        }
    }
    Ok((
        directory,
        name(
            destination
                .file_name()
                .ok_or_else(|| fail("POLICY_DENIED", "output needs a name"))?
                .as_bytes(),
        )?,
    ))
}

struct Stage {
    parent: File,
    directory: File,
    name: CString,
    files: Vec<CString>,
    committed: bool,
}
impl Stage {
    fn create(parent: File) -> Result<Self> {
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(io_error)?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let stage_name = name(format!(".iresiphi-{suffix}").as_bytes())?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), stage_name.as_ptr(), 0o700) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        match directory_at(parent.as_raw_fd(), &stage_name) {
            Ok(directory) => Ok(Self {
                parent,
                directory,
                name: stage_name,
                files: Vec::new(),
                committed: false,
            }),
            Err(mut error) => {
                if unsafe {
                    libc::unlinkat(parent.as_raw_fd(), stage_name.as_ptr(), libc::AT_REMOVEDIR)
                } != 0
                {
                    error.context = Some(format!(
                        "leftover {}: {}",
                        stage_name.to_string_lossy(),
                        std::io::Error::last_os_error()
                    ));
                }
                Err(error)
            }
        }
    }
    fn file(&mut self, filename: &str) -> Result<File> {
        let filename = name(filename.as_bytes())?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                filename.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        self.files.push(filename);
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn cleanup(&mut self) -> Vec<String> {
        if self.committed {
            return Vec::new();
        }
        let mut errors = Vec::new();
        for file in self.files.drain(..) {
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), file.as_ptr(), 0) } != 0 {
                errors.push(format!(
                    "{}/{}: {}",
                    self.name.to_string_lossy(),
                    file.to_string_lossy(),
                    std::io::Error::last_os_error()
                ));
            }
        }
        if unsafe {
            libc::unlinkat(
                self.parent.as_raw_fd(),
                self.name.as_ptr(),
                libc::AT_REMOVEDIR,
            )
        } != 0
        {
            errors.push(format!(
                "{}: {}",
                self.name.to_string_lossy(),
                std::io::Error::last_os_error()
            ));
        }
        // Explicit caller receives cleanup evidence; Drop must not retry and obscure it.
        self.committed = true;
        errors
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

struct BudgetWriter<'a> {
    file: File,
    total: &'a mut usize,
    limit: usize,
    cancel: &'a AtomicBool,
}
impl Write for BudgetWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("CANCELLED"));
        }
        if bytes.len() > self.limit.saturating_sub(*self.total) {
            return Err(std::io::Error::other("RESOURCE_LIMIT"));
        }
        let count = self.file.write(bytes)?;
        *self.total += count;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
fn write_json(
    stage: &mut Stage,
    filename: &str,
    value: &impl serde::Serialize,
    total: &mut usize,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<()> {
    let writer = BudgetWriter {
        file: stage.file(filename)?,
        total,
        limit: policy.limits.output_bytes,
        cancel,
    };
    serde_json::to_writer(writer, value).map_err(|e| {
        let message = e.to_string();
        let code = if message.contains("CANCELLED") {
            "CANCELLED"
        } else if message.contains("RESOURCE_LIMIT") {
            "RESOURCE_LIMIT"
        } else {
            "OUTPUT_IO"
        };
        fail(code, message)
    })
}

fn csv_text(value: &Value) -> String {
    let text = match value {
        Value::Null => return "\\N".into(),
        Value::Boolean { value } => value.to_string(),
        Value::Int64 { value }
        | Value::String { value }
        | Value::Decimal { value, .. }
        | Value::Date { value } => value.clone(),
    };
    if text.starts_with('\\') {
        format!("\\{text}")
    } else {
        text
    }
}

fn write_csv(
    stage: &mut Stage,
    filename: &str,
    rows: &[BTreeMap<String, Value>],
    columns: &[String],
    total: &mut usize,
    policy: &Policy,
    cancel: &AtomicBool,
) -> Result<()> {
    let budget = BudgetWriter {
        file: stage.file(filename)?,
        total,
        limit: policy.limits.output_bytes,
        cancel,
    };
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(budget);
    let convert = |message: String| {
        fail(
            if message.contains("CANCELLED") {
                "CANCELLED"
            } else if message.contains("RESOURCE_LIMIT") {
                "RESOURCE_LIMIT"
            } else {
                "OUTPUT_IO"
            },
            message,
        )
    };
    writer
        .write_record(columns)
        .map_err(|e| convert(e.to_string()))?;
    for row in rows {
        if cancel.load(Ordering::Relaxed) {
            return Err(fail("CANCELLED", "cancelled writing CSV"));
        }
        if row.len() != columns.len() || columns.iter().any(|column| !row.contains_key(column)) {
            return Err(fail(
                "INVALID_SCHEMA",
                "output row does not match declared columns",
            ));
        }
        writer
            .write_record(columns.iter().map(|column| csv_text(&row[column])))
            .map_err(|e| convert(e.to_string()))?;
    }
    writer.flush().map_err(|e| convert(e.to_string()))
}

/// Publishes escaped CSV tables and caller-provided complete type schema.
/// No cancellation check occurs after the successful atomic commit.
pub fn publish_bundle(
    destination: &Path,
    policy: &Policy,
    tables: &BTreeMap<String, Vec<BTreeMap<String, Value>>>,
    schema: &serde_json::Value,
    manifest: &serde_json::Value,
    cancel: &AtomicBool,
) -> Result<usize> {
    let (parent, destination_name) = approved_parent(destination, policy)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let exists = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            destination_name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if exists == 0 {
        return Err(fail("DESTINATION_EXISTS", "destination already exists"));
    }
    let access_error = std::io::Error::last_os_error();
    if access_error.raw_os_error() != Some(libc::ENOENT) {
        return Err(io_error(access_error));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(fail("CANCELLED", "cancelled before staging"));
    }
    let mut stage = Stage::create(parent)?;
    let outcome = (|| {
        let mut total = 0;
        let mut csv_locations = BTreeMap::new();
        for (index, (table, rows)) in tables.iter().enumerate() {
            let columns = schema
                .get(table)
                .and_then(|table| table.get("columns"))
                .and_then(|columns| columns.as_array())
                .ok_or_else(|| {
                    fail(
                        "INVALID_SCHEMA",
                        format!("missing columns for output {table}"),
                    )
                })?
                .iter()
                .map(|column| {
                    column
                        .get("name")
                        .and_then(|name| name.as_str())
                        .map(str::to_owned)
                        .ok_or_else(|| fail("INVALID_SCHEMA", "column lacks a name"))
                })
                .collect::<Result<Vec<_>>>()?;
            let filename = format!("table-{index}.csv");
            write_csv(
                &mut stage, &filename, rows, &columns, &mut total, policy, cancel,
            )?;
            csv_locations.insert(table.clone(), filename);
        }
        let companion = serde_json::json!({"tables":schema,"csv":{"encoding":"UTF-8","delimiter":",","record_ending":"LF","null":"\\N","non_null_backslash_prefix":"double first backslash","files":csv_locations}});
        write_json(
            &mut stage,
            "schema.json",
            &companion,
            &mut total,
            policy,
            cancel,
        )?;
        let mut complete = manifest
            .as_object()
            .cloned()
            .ok_or_else(|| fail("INVALID_MANIFEST", "manifest must be an object"))?;
        complete.insert("complete".into(), serde_json::Value::Bool(true));
        complete.insert(
            "delivery".into(),
            serde_json::json!({"encoding":"csv", "schema":"schema.json"}),
        );
        write_json(
            &mut stage,
            "manifest.json",
            &complete,
            &mut total,
            policy,
            cancel,
        )?;
        if cancel.load(Ordering::Relaxed) {
            return Err(fail("CANCELLED", "cancelled before commit"));
        }
        let rc = unsafe {
            libc::renameat2(
                stage.parent.as_raw_fd(),
                stage.name.as_ptr(),
                stage.parent.as_raw_fd(),
                destination_name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if rc != 0 {
            let error = std::io::Error::last_os_error();
            return Err(fail(
                if error.raw_os_error() == Some(libc::EEXIST) {
                    "DESTINATION_EXISTS"
                } else {
                    "PUBLICATION_FAILED"
                },
                error.to_string(),
            ));
        }
        stage.committed = true;
        Ok(total)
    })();
    match outcome {
        Ok(bytes) => Ok(bytes),
        Err(mut error) => {
            let leftovers = stage.cleanup();
            if !leftovers.is_empty() {
                error.context = Some(format!(
                    "cleanup under {}: {}",
                    destination.parent().unwrap_or(Path::new("/")).display(),
                    leftovers.join("; ")
                ));
            }
            Err(error)
        }
    }
}
