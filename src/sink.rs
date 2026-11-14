//! Linux local bundles: descriptor-relative, owner-only staging and no-replace publication.
//!
//! A bundle is staged in a run-owned sibling directory (`.iresiphi-<128-bit hex>`, mode
//! 0700, files 0600) of the destination, so staging and destination share a filesystem.
//! The active run holds an exclusive `flock` on its staging directory. Publication is a
//! single `renameat2(RENAME_NOREPLACE)`; filesystems without that primitive are refused.
//! This is atomic visibility, not an fsync-backed durability guarantee.
use crate::{
    cancel::{Cancellation, RunPhase},
    model::{Error, Policy, Result, Type, Value},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const STAGE_PREFIX: &str = ".iresiphi-";
const SCHEMA_FILE: &str = "schema.json";
const MANIFEST_FILE: &str = "manifest.json";

fn fail(code: &str, message: impl Into<String>) -> Error {
    Error::new(code, "delivery", message)
}
fn io_error(error: std::io::Error) -> Error {
    fail("OUTPUT_IO", error.to_string())
}
fn name(bytes: &[u8]) -> Result<CString> {
    CString::new(bytes).map_err(|_| fail("POLICY_DENIED", "path contains NUL"))
}
fn directory_at(parent: i32, name: &CStr) -> Result<File> {
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
/// Open `directory` below an approved output root without following any symlink.
fn approved_directory(directory: &Path, policy: &Policy) -> Result<File> {
    if !directory.is_absolute()
        || directory
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(fail(
            "POLICY_DENIED",
            "output must be an absolute path without parent traversal",
        ));
    }
    let approved = policy.allowed_output_roots.iter().any(|root| {
        root.is_absolute()
            && directory.starts_with(root)
            && !root.components().any(|c| matches!(c, Component::ParentDir))
    });
    if !approved {
        return Err(fail("POLICY_DENIED", "output is outside approved roots"));
    }
    let mut handle = directory_at(libc::AT_FDCWD, &name(b"/")?)?;
    for component in directory.components() {
        if let Component::Normal(part) = component {
            handle = directory_at(handle.as_raw_fd(), &name(part.as_bytes())?)?;
        }
    }
    Ok(handle)
}
fn approved_parent(destination: &Path, policy: &Policy) -> Result<(File, CString)> {
    let parent = destination
        .parent()
        .ok_or_else(|| fail("POLICY_DENIED", "output needs a parent"))?;
    if destination
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(fail(
            "POLICY_DENIED",
            "output must be an absolute path without parent traversal",
        ));
    }
    let directory = approved_directory(parent, policy)?;
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
fn entries(directory: &File) -> Result<Vec<CString>> {
    let fd = unsafe { libc::dup(directory.as_raw_fd()) };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let stream = unsafe { libc::fdopendir(fd) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(io_error(error));
    }
    unsafe { libc::rewinddir(stream) };
    let mut names = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        let entry_name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if entry_name.to_bytes() != b"." && entry_name.to_bytes() != b".." {
            names.push(entry_name.to_owned());
        }
    }
    unsafe { libc::closedir(stream) };
    Ok(names)
}
fn stage_name(bytes: &[u8]) -> bool {
    bytes
        .strip_prefix(STAGE_PREFIX.as_bytes())
        .is_some_and(|s| s.len() == 32 && s.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}
/// Only files a stage itself creates; anything else marks a directory as not ours.
fn staged_file_name(bytes: &[u8]) -> bool {
    bytes == SCHEMA_FILE.as_bytes()
        || bytes == MANIFEST_FILE.as_bytes()
        || bytes
            .strip_prefix(b"table-")
            .and_then(|s| s.strip_suffix(b".csv"))
            .is_some_and(|n| !n.is_empty() && n.iter().all(u8::is_ascii_digit))
}

/// A path that could not be removed, with the operating-system error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Leftover {
    pub path: PathBuf,
    pub error: String,
}
/// Cleanup evidence for one delivery attempt, recorded in the run manifest.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DeliveryCleanup {
    /// The run-owned staging directory, if one was created.
    pub staging: Option<PathBuf>,
    /// The staging directory became the committed bundle.
    pub committed: bool,
    /// Owned paths that could not be removed after failure or cancellation.
    pub leftovers: Vec<Leftover>,
}
#[derive(Debug, Clone)]
pub struct Published {
    pub bytes: usize,
    pub cleanup: DeliveryCleanup,
}
#[derive(Debug, Clone)]
pub struct DeliveryFailure {
    pub error: Error,
    pub cleanup: DeliveryCleanup,
}

struct Stage {
    parent: File,
    parent_path: PathBuf,
    directory: File,
    name: CString,
    files: Vec<CString>,
    done: bool,
}
impl Stage {
    fn path(&self) -> PathBuf {
        self.parent_path
            .join(std::ffi::OsStr::from_bytes(self.name.to_bytes()))
    }
    fn create(parent: File, parent_path: &Path) -> Result<Self> {
        let mut random = [0u8; 16];
        File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(io_error)?;
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let stage_name = name(format!("{STAGE_PREFIX}{suffix}").as_bytes())?;
        // Exclusive: mkdirat fails rather than adopting an existing directory.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), stage_name.as_ptr(), 0o700) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let opened = directory_at(parent.as_raw_fd(), &stage_name).and_then(|directory| {
            // The lock marks the stage active for explicit cleanup until this run ends.
            if unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
                || unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) } != 0
            {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            Ok(directory)
        });
        match opened {
            Ok(directory) => Ok(Self {
                parent,
                parent_path: parent_path.to_owned(),
                directory,
                name: stage_name,
                files: Vec::new(),
                done: false,
            }),
            Err(mut error) => {
                if unsafe {
                    libc::unlinkat(parent.as_raw_fd(), stage_name.as_ptr(), libc::AT_REMOVEDIR)
                } != 0
                {
                    error.context = Some(format!(
                        "leftover {}: {}",
                        parent_path
                            .join(std::ffi::OsStr::from_bytes(stage_name.to_bytes()))
                            .display(),
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
        let file = unsafe { File::from_raw_fd(fd) };
        if unsafe { libc::fchmod(file.as_raw_fd(), 0o600) } != 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        Ok(file)
    }
    /// Remove exactly the files this stage created, then the stage itself. Never recursive.
    fn cleanup(&mut self) -> Vec<Leftover> {
        if self.done {
            return Vec::new();
        }
        // Explicit callers receive the evidence; Drop must not retry and obscure it.
        self.done = true;
        let mut leftovers = Vec::new();
        for file in self.files.drain(..) {
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), file.as_ptr(), 0) } != 0 {
                leftovers.push(Leftover {
                    path: self
                        .parent_path
                        .join(std::ffi::OsStr::from_bytes(self.name.to_bytes()))
                        .join(std::ffi::OsStr::from_bytes(file.to_bytes())),
                    error: std::io::Error::last_os_error().to_string(),
                });
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
            leftovers.push(Leftover {
                path: self.path(),
                error: std::io::Error::last_os_error().to_string(),
            });
        }
        leftovers
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
        if self.cancel.load(Ordering::Acquire) {
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
/// The diagnostic code for a failed no-replace rename, by `errno`.
fn publication_code(errno: Option<i32>) -> &'static str {
    match errno {
        Some(libc::EEXIST | libc::ENOTEMPTY) => "DESTINATION_EXISTS",
        Some(libc::EXDEV) => "CROSS_DEVICE",
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP) => "PUBLICATION_UNSUPPORTED",
        _ => "PUBLICATION_FAILED",
    }
}

fn writer_error(message: String) -> Error {
    let code = if message.contains("CANCELLED") {
        "CANCELLED"
    } else if message.contains("RESOURCE_LIMIT") {
        "RESOURCE_LIMIT"
    } else {
        "OUTPUT_IO"
    };
    fail(code, message)
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
    serde_json::to_writer(writer, value).map_err(|e| writer_error(e.to_string()))
}

/// CSV text for one value: `\N` is null and a leading backslash on any non-null
/// representation is doubled, so empty text and literal `\N` stay distinct.
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
    writer
        .write_record(columns)
        .map_err(|e| writer_error(e.to_string()))?;
    for row in rows {
        if cancel.load(Ordering::Acquire) {
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
            .map_err(|e| writer_error(e.to_string()))?;
    }
    writer.flush().map_err(|e| writer_error(e.to_string()))
}

fn schema_columns(schema: &serde_json::Value, table: &str) -> Result<Vec<String>> {
    schema
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
        .collect()
}

/// Publishes escaped CSV tables, the caller-provided complete type schema and a
/// complete-marker manifest as one bundle at a NEW `destination`.
///
/// Cancellation is honoured until the no-replace rename. After a successful rename
/// the run is committed: no cancellation check follows, and a failure while
/// reporting (including a panic in [`Cancellation::entered`]) leaves the complete
/// bundle, whose manifest carries the run ID.
pub fn publish_bundle(
    destination: &Path,
    policy: &Policy,
    tables: &BTreeMap<String, Vec<BTreeMap<String, Value>>>,
    schema: &serde_json::Value,
    manifest: &serde_json::Value,
    cancel: &dyn Cancellation,
) -> std::result::Result<Published, DeliveryFailure> {
    let mut cleanup = DeliveryCleanup::default();
    let early = |error: Error| DeliveryFailure {
        error,
        cleanup: DeliveryCleanup::default(),
    };
    let (parent, destination_name) = approved_parent(destination, policy).map_err(early)?;
    let parent_path = destination.parent().unwrap_or(Path::new("/"));
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
        return Err(early(fail(
            "DESTINATION_EXISTS",
            "destination already exists; inspect its manifest instead of retrying",
        )));
    }
    let access_error = std::io::Error::last_os_error();
    if access_error.raw_os_error() != Some(libc::ENOENT) {
        return Err(early(io_error(access_error)));
    }
    let flag = cancel.flag();
    if flag.load(Ordering::Acquire) {
        return Err(early(fail("CANCELLED", "cancelled before staging")));
    }
    let mut stage = Stage::create(parent, parent_path).map_err(early)?;
    cleanup.staging = Some(stage.path());
    let outcome = (|| {
        cancel.entered(RunPhase::Write);
        let mut total = 0;
        let mut csv_locations = BTreeMap::new();
        for (index, (table, rows)) in tables.iter().enumerate() {
            let columns = schema_columns(schema, table)?;
            let filename = format!("table-{index}.csv");
            write_csv(
                &mut stage, &filename, rows, &columns, &mut total, policy, flag,
            )?;
            csv_locations.insert(table.clone(), filename);
        }
        let companion = serde_json::json!({"tables":schema,"csv":{"encoding":"UTF-8","delimiter":",","quoting":"RFC4180","record_ending":"LF","header_rows":1,"null":"\\N","non_null_backslash_prefix":"double first backslash","files":csv_locations}});
        write_json(
            &mut stage,
            SCHEMA_FILE,
            &companion,
            &mut total,
            policy,
            flag,
        )?;
        let mut complete = manifest
            .as_object()
            .cloned()
            .ok_or_else(|| fail("INVALID_MANIFEST", "manifest must be an object"))?;
        complete.insert("complete".into(), serde_json::Value::Bool(true));
        write_json(
            &mut stage,
            MANIFEST_FILE,
            &complete,
            &mut total,
            policy,
            flag,
        )?;
        if flag.load(Ordering::Acquire) {
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
            let code = publication_code(error.raw_os_error());
            return Err(fail(code, format!("no-replace rename: {error}")));
        }
        Ok(total)
    })();
    match outcome {
        Ok(bytes) => {
            stage.done = true;
            cleanup.committed = true;
            drop(stage);
            // Committed: this hook cannot revoke the bundle, whatever it does.
            cancel.entered(RunPhase::Committed);
            Ok(Published { bytes, cleanup })
        }
        Err(mut error) => {
            cleanup.leftovers = stage.cleanup();
            if !cleanup.leftovers.is_empty() {
                error.context = Some(format!(
                    "cleanup left: {}",
                    cleanup
                        .leftovers
                        .iter()
                        .map(|l| format!("{} ({})", l.path.display(), l.error))
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            Err(DeliveryFailure { error, cleanup })
        }
    }
}

/// A staging-shaped entry that explicit cleanup left in place, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct StagingCleanup {
    pub removed: Vec<PathBuf>,
    pub skipped: Vec<Skipped>,
    pub leftovers: Vec<Leftover>,
}

/// Explicitly remove abandoned run-owned staging (for example after process death)
/// directly inside `directory`, which must lie under an approved output root.
///
/// An entry is removed only when it is a real directory named `.iresiphi-<32 hex>`,
/// owned by this user with no group/other permissions, not locked by an active run,
/// and containing only regular files a stage writes. Nothing is followed or removed
/// recursively; every other entry is reported as skipped.
pub fn cleanup_staging(directory: &Path, policy: &Policy) -> Result<StagingCleanup> {
    let parent = approved_directory(directory, policy)?;
    let uid = unsafe { libc::geteuid() };
    let mut report = StagingCleanup::default();
    for entry in entries(&parent)? {
        if !stage_name(entry.to_bytes()) {
            continue;
        }
        let path = directory.join(std::ffi::OsStr::from_bytes(entry.to_bytes()));
        let mut skip = |reason: &str| {
            report.skipped.push(Skipped {
                path: path.clone(),
                reason: reason.into(),
            })
        };
        let Ok(stage) = directory_at(parent.as_raw_fd(), &entry) else {
            skip("not a directory");
            continue;
        };
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe { libc::fstat(stage.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            skip("unreadable");
            continue;
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_uid != uid || stat.st_mode & 0o077 != 0 {
            skip("not owner-only run staging");
            continue;
        }
        if unsafe { libc::flock(stage.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            skip("active");
            continue;
        }
        let files = entries(&stage)?;
        let recognized = files.iter().all(|file| {
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            staged_file_name(file.to_bytes())
                && unsafe {
                    libc::fstatat(
                        stage.as_raw_fd(),
                        file.as_ptr(),
                        stat.as_mut_ptr(),
                        libc::AT_SYMLINK_NOFOLLOW,
                    )
                } == 0
                && {
                    let stat = unsafe { stat.assume_init() };
                    stat.st_mode & libc::S_IFMT == libc::S_IFREG && stat.st_uid == uid
                }
        });
        if !recognized {
            skip("unrecognized contents");
            continue;
        }
        let mut clean = true;
        for file in &files {
            if unsafe { libc::unlinkat(stage.as_raw_fd(), file.as_ptr(), 0) } != 0 {
                clean = false;
                report.leftovers.push(Leftover {
                    path: path.join(std::ffi::OsStr::from_bytes(file.to_bytes())),
                    error: std::io::Error::last_os_error().to_string(),
                });
            }
        }
        if clean {
            if unsafe { libc::unlinkat(parent.as_raw_fd(), entry.as_ptr(), libc::AT_REMOVEDIR) }
                == 0
            {
                report.removed.push(path);
            } else {
                report.leftovers.push(Leftover {
                    path,
                    error: std::io::Error::last_os_error().to_string(),
                });
            }
        }
    }
    Ok(report)
}

fn decode(text: &str, data_type: &serde_json::Value) -> Result<Value> {
    let invalid = |why: &str| fail("INVALID_BUNDLE", format!("{why}: {text:?}"));
    if text == "\\N" {
        return Ok(Value::Null);
    }
    let text = if text.starts_with("\\\\") {
        &text[1..]
    } else if text.starts_with('\\') {
        return Err(invalid("undoubled leading backslash"));
    } else {
        text
    };
    let data_type: Type = serde_json::from_value(data_type.clone())
        .map_err(|_| invalid("non-null value in a column without a declared type"))?;
    Ok(match data_type {
        Type::String => Value::String { value: text.into() },
        Type::Boolean => Value::Boolean {
            value: match text {
                "true" => true,
                "false" => false,
                _ => return Err(invalid("boolean")),
            },
        },
        Type::Int64 => {
            text.parse::<i64>().map_err(|_| invalid("int64"))?;
            Value::Int64 { value: text.into() }
        }
        Type::Date => {
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| invalid("date"))?;
            Value::Date { value: text.into() }
        }
        Type::Decimal { precision, scale } => {
            crate::scalar::Decimal::parse(text, precision, scale)
                .map_err(|_| invalid("decimal"))?;
            Value::Decimal {
                value: text.into(),
                precision,
                scale,
            }
        }
    })
}

/// Read a committed bundle back into typed tables using its companion schema.
/// A bundle without a complete-marker manifest is refused.
pub fn read_bundle(bundle: &Path) -> Result<BTreeMap<String, Vec<BTreeMap<String, Value>>>> {
    let read = |file: &str| -> Result<serde_json::Value> {
        let bytes = std::fs::read(bundle.join(file)).map_err(io_error)?;
        serde_json::from_slice(&bytes).map_err(|e| fail("INVALID_BUNDLE", e.to_string()))
    };
    if read(MANIFEST_FILE)?.get("complete") != Some(&serde_json::Value::Bool(true)) {
        return Err(fail("INVALID_BUNDLE", "bundle manifest is not complete"));
    }
    let schema = read(SCHEMA_FILE)?;
    let files = schema["csv"]["files"]
        .as_object()
        .ok_or_else(|| fail("INVALID_BUNDLE", "schema lists no files"))?;
    let mut tables = BTreeMap::new();
    for (table, file) in files {
        let file = file
            .as_str()
            .filter(|f| staged_file_name(f.as_bytes()))
            .ok_or_else(|| fail("INVALID_BUNDLE", "invalid table file name"))?;
        let columns = schema["tables"][table]["columns"]
            .as_array()
            .ok_or_else(|| fail("INVALID_BUNDLE", format!("no columns for {table}")))?;
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(true)
            .from_path(bundle.join(file))
            .map_err(|e| fail("INVALID_BUNDLE", e.to_string()))?;
        let headers = reader
            .headers()
            .map_err(|e| fail("INVALID_BUNDLE", e.to_string()))?
            .clone();
        let names: Vec<_> = columns.iter().map(|c| c["name"].as_str()).collect();
        if headers.iter().map(Some).collect::<Vec<_>>() != names {
            return Err(fail(
                "INVALID_BUNDLE",
                format!("header mismatch in {table}"),
            ));
        }
        let mut rows = Vec::new();
        for record in reader.records() {
            let record = record.map_err(|e| fail("INVALID_BUNDLE", e.to_string()))?;
            let mut row = BTreeMap::new();
            for (column, text) in columns.iter().zip(record.iter()) {
                row.insert(
                    column["name"].as_str().unwrap_or_default().to_owned(),
                    decode(text, &column["type"])?,
                );
            }
            rows.push(row);
        }
        tables.insert(table.clone(), rows);
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::publication_code;

    #[test]
    fn a_failed_rename_is_classified_by_errno() {
        for (errno, code) in [
            (libc::EEXIST, "DESTINATION_EXISTS"),
            (libc::ENOTEMPTY, "DESTINATION_EXISTS"),
            (libc::EXDEV, "CROSS_DEVICE"),
            (libc::ENOSYS, "PUBLICATION_UNSUPPORTED"),
            (libc::EINVAL, "PUBLICATION_UNSUPPORTED"),
            (libc::EOPNOTSUPP, "PUBLICATION_UNSUPPORTED"),
            (libc::EACCES, "PUBLICATION_FAILED"),
            (libc::EIO, "PUBLICATION_FAILED"),
        ] {
            assert_eq!(publication_code(Some(errno)), code, "errno {errno}");
        }
        assert_eq!(publication_code(None), "PUBLICATION_FAILED");
    }
}
