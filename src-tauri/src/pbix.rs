use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Write as FmtWrite,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Cursor, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};
use zip::ZipArchive;

#[derive(Serialize)]
pub struct ReportFile {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub parse_ms: u128,
    pub pages: Vec<Page>,
    pub visual_count: usize,
    pub report_filters: Vec<FilterInfo>,
    pub dax_queries: Vec<DaxQuery>,
    pub aas_connection: Option<AasConnection>,
    pub bookmarks: Vec<BookmarkInfo>,
    pub mobile_layout_count: usize,
    pub tables: Vec<Table>,
    pub relationships: Vec<Relationship>,
    pub sources: Vec<Source>,
    pub queries: Vec<Query>,
    pub entries: Vec<Entry>,
    pub deep_model: bool,
    pub deep_cache_hit: bool,
    pub deep_error: String,
    pub model_metadata: Value,
}
#[derive(Serialize)]
pub struct Page {
    pub name: String,
    pub display_name: String,
    pub width: f64,
    pub height: f64,
    pub filters: Vec<FilterInfo>,
    pub is_hidden: bool,
    pub is_drillthrough: bool,
    pub interactions: Vec<InteractionInfo>,
    pub visuals: Vec<Visual>,
}
#[derive(Serialize)]
pub struct Visual {
    pub id: String,
    pub visual_type: String,
    pub visual_type_label: String,
    pub title: String,
    pub x_pct: f64,
    pub y_pct: f64,
    pub w_pct: f64,
    pub h_pct: f64,
    pub z_index: i64,
    pub fields: Vec<String>,
    pub is_hidden: bool,
    pub prototype_query: Value,
    pub semantic_query: Value,
    pub data_transforms: Value,
    pub aggregations: Vec<AggregationInfo>,
    pub resolved_filters: Vec<FilterInfo>,
    pub filters: Vec<FilterInfo>,
    pub slicer_selections: Vec<FilterInfo>,
    pub column_labels: Vec<ColumnLabel>,
    pub sync_group: Option<SyncGroupInfo>,
    pub bookmark_target: String,
}
#[derive(Serialize, Clone)]
pub struct FilterInfo {
    pub scope: String,
    pub target: String,
    pub kind: String,
    pub expression: String,
    pub active: bool,
    pub note: String,
}
#[derive(Serialize)]
pub struct AggregationInfo {
    pub field: String,
    pub function_code: i64,
    pub function_name: String,
    pub native_name: String,
    pub display_name: String,
}
#[derive(Serialize)]
pub struct DaxQuery {
    pub name: String,
    pub path: String,
    pub expression: String,
    pub is_default: bool,
}
#[derive(Serialize)]
pub struct AasConnection {
    pub server_url: String,
    pub catalog: String,
    pub cube: String,
    pub connection_type: String,
}
#[derive(Serialize)]
pub struct BookmarkInfo {
    pub name: String,
    pub id: String,
    pub active_page: String,
    pub hidden_visual_count: usize,
    pub filter_count: usize,
    pub state: Value,
}
#[derive(Serialize)]
pub struct InteractionInfo {
    pub source: String,
    pub target: String,
    pub interaction_type: i64,
    pub behavior: String,
}
#[derive(Serialize)]
pub struct ColumnLabel {
    pub query_ref: String,
    pub display_name: String,
}
#[derive(Serialize)]
pub struct SyncGroupInfo {
    pub group_name: String,
    pub field_changes: bool,
    pub filter_changes: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    #[serde(default)]
    pub row_count: Option<u64>,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub expression: String,
}
#[derive(Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub kind: String,
    pub expression: String,
    #[serde(default)]
    pub is_hidden: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub format_string: String,
    #[serde(default)]
    pub display_folder: String,
    #[serde(default)]
    pub cardinality: Option<u64>,
    #[serde(default)]
    pub data_size: Option<u64>,
}
#[derive(Serialize, Deserialize)]
pub struct Relationship {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub cardinality: String,
    #[serde(default)]
    pub cross_filtering: String,
    #[serde(default)]
    pub referential_integrity: bool,
}
#[derive(Serialize)]
pub struct Source {
    pub kind: String,
    pub detail: String,
}
#[derive(Serialize)]
pub struct Query {
    pub name: String,
    pub preview: String,
    pub formula: String,
    pub connectors: Vec<String>,
    pub dependencies: Vec<String>,
    pub has_native_query: bool,
}
#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub compressed_size: u64,
}
#[derive(Serialize)]
pub struct EntryContent {
    pub name: String,
    pub kind: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Serialize, Deserialize)]
pub struct TableRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Deserialize, Serialize)]
struct DeepMetadata {
    #[serde(default)]
    decoder: String,
    #[serde(default)]
    schema_ok: bool,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    tables: Vec<Table>,
    #[serde(default)]
    relationships: Vec<Relationship>,
    #[serde(default)]
    queries: Vec<DeepQuery>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

#[derive(Deserialize, Serialize)]
struct DeepQuery {
    name: String,
    formula: String,
}

struct DeepProcess {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    responses: mpsc::Receiver<String>,
}

static DEEP_PROCESS: OnceLock<Mutex<Option<DeepProcess>>> = OnceLock::new();

pub fn read_entry(path: &Path, entry_name: &str) -> Result<EntryContent, String> {
    let file = File::open(path).map_err(|e| format!("Cannot open report: {e}"))?;
    let mut zip =
        ZipArchive::new(file).map_err(|_| "Not a readable Power BI package.".to_string())?;
    let mut entry = zip
        .by_name(entry_name)
        .map_err(|_| "Package entry no longer exists.".to_string())?;
    let truncated = entry.size() > 4 * 1024 * 1024;
    let mut bytes = Vec::with_capacity(entry.size().min(4 * 1024 * 1024) as usize);
    entry
        .by_ref()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read package entry: {e}"))?;
    if let Some(mut content) = decode_bytes(&bytes) {
        if let Ok(json) = serde_json::from_str::<Value>(&content) {
            content = serde_json::to_string_pretty(&json).unwrap_or(content);
        }
        return Ok(EntryContent {
            name: entry_name.into(),
            kind: "Text".into(),
            content,
            truncated,
        });
    }
    let mut content = String::new();
    for (row, chunk) in bytes.chunks(16).take(1024).enumerate() {
        let _ = write!(content, "{:08x}  ", row * 16);
        for byte in chunk {
            let _ = write!(content, "{:02x} ", byte);
        }
        for _ in chunk.len()..16 {
            content.push_str("   ");
        }
        content.push(' ');
        for byte in chunk {
            content.push(if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            });
        }
        content.push('\n');
    }
    Ok(EntryContent {
        name: entry_name.into(),
        kind: "Binary hex preview".into(),
        content,
        truncated: truncated || bytes.len() > 16 * 1024,
    })
}

pub fn parse_report(path: &Path) -> Result<ReportFile, String> {
    let started = Instant::now();
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "pbix" && ext != "pbit" {
        return Err("Choose a .pbix or .pbit file.".into());
    }
    let size = path
        .metadata()
        .map_err(|e| format!("Cannot read file metadata: {e}"))?
        .len();
    let file = File::open(path).map_err(|e| format!("Cannot open report: {e}"))?;
    let mut zip = ZipArchive::new(file)
        .map_err(|_| "This file is not a readable Power BI package (ZIP container).".to_string())?;
    let mut entries = Vec::with_capacity(zip.len());
    let mut layout = None;
    let mut schema = None;
    let mut connections = None;
    let mut mashup = None;
    let mut diagram_layout = None;
    let mut dax_queries = Vec::new();
    let mut dax_query_metadata = None;
    let mut has_data_model = false;
    for i in 0..zip.len() {
        let mut item = zip
            .by_index(i)
            .map_err(|e| format!("Cannot inspect package: {e}"))?;
        let name = item.name().to_string();
        entries.push(Entry {
            name: name.clone(),
            size: item.size(),
            compressed_size: item.compressed_size(),
        });
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        if normalized == "datamodel" || normalized.ends_with("/datamodel") {
            has_data_model = true;
        }
        if normalized == "report/layout" || normalized.ends_with("/report/layout") {
            layout = read_text(&mut item);
        } else if normalized == "datamodelschema" || normalized.ends_with("/datamodelschema") {
            schema = read_text(&mut item);
        } else if normalized == "connections" || normalized.ends_with("/connections") {
            connections = read_text(&mut item);
        } else if normalized == "diagramlayout" || normalized.ends_with("/diagramlayout") {
            diagram_layout = read_text(&mut item);
        } else if normalized == "daxqueries/.pbi/daxqueries.json" {
            dax_query_metadata = read_text(&mut item);
        } else if normalized.starts_with("daxqueries/") && normalized.ends_with(".dax") {
            if item.size() <= 4 * 1024 * 1024 {
                if let Some(expression) = read_text(&mut item) {
                    dax_queries.push(DaxQuery {
                        name: dax_query_display_name(&name),
                        path: name,
                        expression,
                        is_default: false,
                    });
                }
            }
        } else if (normalized == "datamashup" || normalized.ends_with("/datamashup"))
            && item.size() <= 64 * 1024 * 1024
        {
            let mut bytes = Vec::with_capacity(item.size() as usize);
            if item.read_to_end(&mut bytes).is_ok() {
                mashup = Some(bytes);
            }
        }
    }
    apply_dax_query_metadata(&mut dax_queries, dax_query_metadata.as_deref());
    let layout_json = layout
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let pages = layout_json.clone().map(parse_pages).unwrap_or_default();
    let report_measure_tables = layout_json
        .as_ref()
        .map(parse_report_measure_tables)
        .unwrap_or_default();
    let report_filters = layout_json
        .as_ref()
        .map(|layout| parse_filters(layout.get("filters"), "Report", false))
        .unwrap_or_default();
    let bookmarks = layout_json
        .as_ref()
        .map(parse_bookmarks)
        .unwrap_or_default();
    let mobile_layout_count = layout_json
        .as_ref()
        .map(count_mobile_layouts)
        .unwrap_or_default();
    let diagram_tables = diagram_layout
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .map(|value| parse_diagram_tables(&value))
        .unwrap_or_default();
    let (mut tables, mut relationships) = schema
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .map(parse_model)
        .unwrap_or_default();
    let connections_json = connections
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let aas_connection = connections_json.as_ref().and_then(parse_aas_connection);
    let mut sources = connections_json.map(parse_sources).unwrap_or_default();
    let (mut queries, mashup_sources) = mashup.as_deref().map(parse_mashup).unwrap_or_default();
    let mut known_sources: HashSet<String> = sources.iter().map(|s| s.detail.clone()).collect();
    sources.extend(
        mashup_sources
            .into_iter()
            .filter(|s| known_sources.insert(s.detail.clone())),
    );
    let mut deep_model = false;
    let mut deep_cache_hit = false;
    let mut deep_error = String::new();
    let mut model_metadata = Value::Object(Default::default());
    if has_data_model {
        match read_deep_metadata(path) {
            Ok((deep, cache_hit)) => {
                deep_model = deep.schema_ok && !deep.tables.is_empty();
                deep_cache_hit = cache_hit;
                if !deep.warnings.is_empty() {
                    deep_error = deep.warnings.join("\n");
                }
                if !deep_model && deep_error.is_empty() {
                    deep_error = "Deep decoding did not produce a usable semantic schema.".into();
                }
                if !deep.tables.is_empty() {
                    tables = deep.tables;
                }
                if !deep.relationships.is_empty() {
                    relationships = deep.relationships;
                }
                if !deep.queries.is_empty() {
                    queries = build_queries(
                        deep.queries
                            .into_iter()
                            .map(|query| (query.name, query.formula))
                            .collect(),
                    );
                    let deep_sources = sources_from_queries(&queries);
                    let mut known: HashSet<String> =
                        sources.iter().map(|source| source.detail.clone()).collect();
                    sources.extend(
                        deep_sources
                            .into_iter()
                            .filter(|source| known.insert(source.detail.clone())),
                    );
                }
                let mut metadata = deep.extra;
                metadata.insert("decoder".into(), Value::String(deep.decoder));
                metadata.insert("schema_ok".into(), Value::Bool(deep.schema_ok));
                metadata.insert(
                    "warnings".into(),
                    Value::Array(deep.warnings.into_iter().map(Value::String).collect()),
                );
                model_metadata = Value::Object(metadata);
            }
            Err(error) => deep_error = error,
        }
    }
    merge_report_measure_tables(&mut tables, report_measure_tables);
    merge_diagram_tables(&mut tables, diagram_tables);
    let visual_count = pages.iter().map(|p| p.visuals.len()).sum();
    Ok(ReportFile {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Power BI report")
            .to_string(),
        kind: ext.to_uppercase(),
        size,
        parse_ms: started.elapsed().as_millis(),
        pages,
        visual_count,
        report_filters,
        dax_queries,
        aas_connection,
        bookmarks,
        mobile_layout_count,
        tables,
        relationships,
        sources,
        queries,
        entries,
        deep_model,
        deep_cache_hit,
        deep_error,
        model_metadata,
    })
}

pub fn read_table_rows(
    path: &Path,
    table_name: &str,
    offset: u64,
    limit: u64,
) -> Result<TableRows, String> {
    let limit = limit.clamp(1, 500);
    let value = request_sidecar(serde_json::json!({
        "command": "table",
        "path": path.to_string_lossy(),
        "table_name": table_name,
        "offset": offset,
        "limit": limit,
    }))?;
    serde_json::from_value(value)
        .map_err(|error| format!("Deep decoder returned invalid table data: {error}"))
}

fn read_deep_metadata(path: &Path) -> Result<(DeepMetadata, bool), String> {
    let cache_path = deep_cache_path(path);
    if let Some(cache_path) = cache_path.as_ref() {
        if let Ok(metadata) = read_deep_cache(cache_path) {
            return Ok((metadata, true));
        }
    }
    let value = request_sidecar(serde_json::json!({
        "command": "metadata",
        "path": path.to_string_lossy(),
    }))?;
    let metadata: DeepMetadata = serde_json::from_value(value)
        .map_err(|error| format!("Deep decoder returned invalid metadata: {error}"))?;
    if metadata.schema_ok {
        if let Some(cache_path) = cache_path {
            let _ = write_deep_cache(&cache_path, &metadata);
        }
    }
    Ok((metadata, false))
}

fn deep_cache_path(path: &Path) -> Option<PathBuf> {
    let mut file = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(b"deep-metadata-v3\0");
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let key = format!("{:x}", hasher.finalize());
    Some(
        std::env::temp_dir()
            .join("pbi-lens-cache")
            .join(format!("{key}.json")),
    )
}

fn read_deep_cache(path: &Path) -> Result<DeepMetadata, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("Cannot inspect cache: {error}"))?;
    if metadata.len() > 64 * 1024 * 1024 {
        return Err("Cached metadata exceeds the safety limit.".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("Cannot read cache: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("Cached metadata is invalid: {error}"))
}

fn write_deep_cache(path: &Path, metadata: &DeepMetadata) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Cache path has no parent directory.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| format!("Cannot create cache: {error}"))?;
    let bytes = serde_json::to_vec(metadata)
        .map_err(|error| format!("Cannot encode cached metadata: {error}"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("metadata.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Cannot create cache entry: {error}"))?;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Cannot write cache: {error}"));
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Cannot publish cache: {error}"));
    }
    Ok(())
}

fn request_sidecar(request: Value) -> Result<Value, String> {
    let process = DEEP_PROCESS.get_or_init(|| Mutex::new(None));
    let mut guard = process
        .lock()
        .map_err(|_| "Deep decoder process lock was poisoned.".to_string())?;
    if guard.is_none() {
        *guard = Some(start_sidecar()?);
    }
    match request_process(
        guard.as_mut().expect("decoder process initialized"),
        &request,
        Duration::from_secs(60),
    ) {
        Ok(result) => Ok(result),
        Err(error) => {
            if let Some(mut failed) = guard.take() {
                stop_sidecar(&mut failed);
            }
            // Prepare a clean helper for the next request without repeating an operation
            // that may be expensive or consistently malformed.
            *guard = start_sidecar().ok();
            Err(error)
        }
    }
}

fn start_sidecar() -> Result<DeepProcess, String> {
    let executable = sidecar_path().ok_or_else(|| {
        "Deep model decoder is unavailable; package metadata is still accessible.".to_string()
    })?;
    let mut child = Command::new(&executable)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Cannot start deep model decoder: {error}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Cannot open deep decoder input.".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Cannot open deep decoder output.".to_string())?;
    let (sender, responses) = mpsc::channel();
    if let Err(error) = thread::Builder::new()
        .name("pbi-deep-output".into())
        .spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut response = String::new();
                match stdout.read_line(&mut response) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if sender.send(response).is_err() => break,
                    Ok(_) => {}
                }
            }
        })
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Cannot start deep decoder reader: {error}"));
    }
    Ok(DeepProcess {
        child,
        stdin: BufWriter::new(stdin),
        responses,
    })
}

fn request_process(
    process: &mut DeepProcess,
    request: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    serde_json::to_writer(&mut process.stdin, request)
        .map_err(|error| format!("Cannot encode decoder request: {error}"))?;
    process
        .stdin
        .write_all(b"\n")
        .and_then(|_| process.stdin.flush())
        .map_err(|error| format!("Cannot send request to deep decoder: {error}"))?;
    let response = match process.responses.recv_timeout(timeout) {
        Ok(response) => response,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            stop_sidecar(process);
            return Err(format!(
                "Deep model decoder timed out after {} seconds.",
                timeout.as_secs()
            ));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let status = process.child.try_wait().ok().flatten();
            return Err(format!("Deep decoder stopped unexpectedly ({status:?})."));
        }
    };
    let value: Value = serde_json::from_str(&response)
        .map_err(|error| format!("Deep decoder returned invalid JSON: {error}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!(
            "Deep model decoder could not read this model: {error}"
        ));
    }
    Ok(value)
}

fn stop_sidecar(process: &mut DeepProcess) {
    let _ = process.child.kill();
    let _ = process.child.wait();
}

fn sidecar_path() -> Option<PathBuf> {
    let target_name = format!("pbi-deep-{}", std::env::consts::ARCH);
    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        if let Some(parent) = current.parent() {
            candidates.push(parent.join("pbi-deep"));
            candidates.push(parent.join("pbi-deep-aarch64-apple-darwin"));
            candidates.push(parent.join(&target_name));
        }
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    candidates.push(
        manifest
            .join("binaries")
            .join("pbi-deep-aarch64-apple-darwin"),
    );
    candidates.push(manifest.join("binaries").join("pbi-deep"));
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn read_text<R: Read>(reader: &mut R) -> Option<String> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).ok()?;
    decode_bytes(&bytes)
}

fn decode_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(&[0xff, 0xfe]) {
        return Some(String::from_utf16_lossy(
            &bytes[2..]
                .chunks_exact(2)
                .map(|x| u16::from_le_bytes([x[0], x[1]]))
                .collect::<Vec<_>>(),
        ));
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return Some(String::from_utf16_lossy(
            &bytes[2..]
                .chunks_exact(2)
                .map(|x| u16::from_be_bytes([x[0], x[1]]))
                .collect::<Vec<_>>(),
        ));
    }
    if bytes.len() > 4
        && bytes
            .iter()
            .skip(1)
            .step_by(2)
            .take(64)
            .filter(|&&b| b == 0)
            .count()
            > 20
    {
        return Some(String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|x| u16::from_le_bytes([x[0], x[1]]))
                .collect::<Vec<_>>(),
        ));
    }
    if bytes.len() > 4
        && bytes
            .iter()
            .step_by(2)
            .take(64)
            .filter(|&&b| b == 0)
            .count()
            > 20
    {
        return Some(String::from_utf16_lossy(
            &bytes
                .chunks_exact(2)
                .map(|x| u16::from_be_bytes([x[0], x[1]]))
                .collect::<Vec<_>>(),
        ));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some(s.trim_start_matches('\u{feff}').to_string());
    }
    None
}

fn parse_pages(root: Value) -> Vec<Page> {
    let sections = root
        .get("sections")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    sections
        .into_iter()
        .map(|s| {
            let section_name = text(&s, "name");
            let width = num(&s, "width", 1280.0);
            let height = num(&s, "height", 720.0);
            let filters = parse_filters(s.get("filters"), "Page", false);
            let section_config = embedded_json(s.get("config"));
            let interactions = parse_interactions(&section_config);
            let is_hidden = s.get("visibility").and_then(Value::as_i64) == Some(1);
            let is_drillthrough =
                embedded_json(s.get("filters"))
                    .as_array()
                    .is_some_and(|filters| {
                        filters.iter().any(|filter| {
                            filter.get("howCreated").and_then(Value::as_i64) == Some(5)
                        })
                    });
            let items = s.get("visualContainers").and_then(Value::as_array);
            let mut groups: HashMap<String, (f64, f64, i64, bool, String)> = HashMap::new();
            if let Some(items) = items {
                for v in items {
                    let config = embedded_json(v.get("config"));
                    if config.get("singleVisualGroup").is_some() {
                        let name = text(&config, "name");
                        let hidden = config
                            .pointer("/singleVisualGroup/isHidden")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        groups.insert(
                            name,
                            (
                                num(v, "x", 0.0),
                                num(v, "y", 0.0),
                                num(v, "z", 0.0) as i64,
                                hidden,
                                text(&config, "parentGroupName"),
                            ),
                        );
                    }
                }
            }
            let visuals = items
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| {
                            let config = embedded_json(v.get("config"));
                            if config.get("singleVisualGroup").is_some() {
                                return None;
                            }
                            let mut parent = text(&config, "parentGroupName");
                            let mut offset_x = 0.0;
                            let mut offset_y = 0.0;
                            let mut offset_z = 0;
                            let mut depth = 0;
                            while !parent.is_empty() && depth < 16 {
                                let Some((x, y, z, hidden, next)) = groups.get(&parent) else {
                                    break;
                                };
                                if *hidden {
                                    return None;
                                }
                                offset_x += x;
                                offset_y += y;
                                offset_z += z;
                                parent = next.clone();
                                depth += 1;
                            }
                            Some(parse_visual(v, width, height, offset_x, offset_y, offset_z))
                        })
                        .collect()
                })
                .unwrap_or_default();
            Page {
                name: section_name,
                display_name: nonempty(text(&s, "displayName"), "Untitled page"),
                width,
                height,
                filters,
                is_hidden,
                is_drillthrough,
                interactions,
                visuals,
            }
        })
        .collect()
}

fn parse_visual(
    v: &Value,
    page_w: f64,
    page_h: f64,
    offset_x: f64,
    offset_y: f64,
    offset_z: i64,
) -> Visual {
    let config = embedded_json(v.get("config"));
    let query = embedded_json(v.get("query"));
    let data = embedded_json(v.get("dataTransforms"));
    let visual_type = config
        .pointer("/singleVisual/visualType")
        .and_then(Value::as_str)
        .unwrap_or("visual")
        .to_string();
    let prototype_query = config
        .pointer("/singleVisual/prototypeQuery")
        .cloned()
        .unwrap_or(Value::Null);
    let aggregations = parse_aggregations(&prototype_query, &data);
    let resolved_filters = parse_resolved_filters(&query);
    let filters = parse_filters(
        v.get("filters"),
        "Visual",
        visual_type.eq_ignore_ascii_case("slicer"),
    );
    let slicer_selections = parse_slicer_selections(&config);
    let is_hidden = config
        .pointer("/singleVisual/display/mode")
        .and_then(Value::as_str)
        == Some("hidden");
    let column_labels = parse_column_labels(&config);
    let sync_group = parse_sync_group(&config);
    let bookmark_target = config
        .pointer("/singleVisual/vcObjects/visualLink/0/properties/bookmark/expr/Literal/Value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim_matches('\'')
        .to_string();
    let mut strings = Vec::new();
    collect_bound_fields(&prototype_query, &mut strings);
    if let Some(resolved_query) = query.pointer("/Commands/0/SemanticQueryDataShapeCommand/Query") {
        collect_bound_fields(resolved_query, &mut strings);
    }
    if let Some(selects) = data.get("selects").and_then(Value::as_array) {
        strings.extend(selects.iter().filter_map(|select| {
            select
                .get("queryName")
                .and_then(Value::as_str)
                .map(str::to_string)
        }));
    }
    if strings.is_empty() {
        collect_named(&config, &mut strings);
        collect_named(&query, &mut strings);
        collect_named(&data, &mut strings);
    }
    let fields: Vec<String> = strings
        .into_iter()
        .filter(|s| !s.is_empty() && s.len() < 100)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(20)
        .collect();
    let configured_title = config
        .pointer("/singleVisual/vcObjects/title/0/properties/text/expr/Literal/Value")
        .or_else(|| {
            config.pointer("/singleVisual/objects/title/0/properties/text/expr/Literal/Value")
        })
        .and_then(Value::as_str)
        .map(|s| s.trim_matches('\'').to_string())
        .unwrap_or_default();
    let button_text = config
        .pointer("/singleVisual/objects/text")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|item| {
                item.pointer("/properties/text/expr/Literal/Value")
                    .and_then(Value::as_str)
            })
        })
        .map(|s| s.trim_matches('\'').to_string())
        .unwrap_or_default();
    let textbox_text = textbox_content(&config);
    let title = if visual_type.eq_ignore_ascii_case("shape")
        || visual_type.eq_ignore_ascii_case("basicShape")
    {
        String::new()
    } else if visual_type.eq_ignore_ascii_case("actionButton") && !button_text.is_empty() {
        button_text
    } else if visual_type.eq_ignore_ascii_case("textbox") {
        if !textbox_text.is_empty() {
            textbox_text
        } else if configured_title.eq_ignore_ascii_case("text") {
            String::new()
        } else {
            configured_title
        }
    } else {
        configured_title
    };
    let x = num(v, "x", 0.0) + offset_x;
    let y = num(v, "y", 0.0) + offset_y;
    let w = num(v, "width", 280.0);
    let h = num(v, "height", 160.0);
    Visual {
        id: text(&config, "name"),
        visual_type_label: label_type(&visual_type),
        visual_type,
        title,
        x_pct: x / page_w * 100.0,
        y_pct: y / page_h * 100.0,
        w_pct: w / page_w * 100.0,
        h_pct: h / page_h * 100.0,
        z_index: num(v, "z", 0.0) as i64 + offset_z,
        fields,
        is_hidden,
        prototype_query,
        semantic_query: query,
        data_transforms: data,
        aggregations,
        resolved_filters,
        filters,
        slicer_selections,
        column_labels,
        sync_group,
        bookmark_target,
    }
}

fn textbox_content(config: &Value) -> String {
    [
        "/singleVisual/objects/general/0/properties/paragraphs",
        "/singleVisual/vcObjects/general/0/properties/paragraphs",
    ]
    .into_iter()
    .find_map(|pointer| {
        let paragraphs = config.pointer(pointer)?.as_array()?;
        let text = paragraphs
            .iter()
            .flat_map(|paragraph| {
                paragraph
                    .get("textRuns")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|run| {
                !run.pointer("/textStyle/fontFamily")
                    .and_then(Value::as_str)
                    .is_some_and(|font| font.eq_ignore_ascii_case("wingdings"))
            })
            .filter_map(|run| run.get("value").and_then(Value::as_str))
            .collect::<String>()
            .trim()
            .to_string();
        (!text.is_empty()).then_some(text)
    })
    .unwrap_or_default()
}

fn parse_model(root: Value) -> (Vec<Table>, Vec<Relationship>) {
    let model = root
        .pointer("/model")
        .or_else(|| root.pointer("/model/model"))
        .unwrap_or(&root);
    let tables = model
        .get("tables")
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter()
                .map(|t| {
                    let mut columns = Vec::new();
                    if let Some(cs) = t.get("columns").and_then(Value::as_array) {
                        for c in cs {
                            columns.push(Column {
                                name: text(c, "name"),
                                data_type: text(c, "dataType"),
                                kind: "Column".into(),
                                expression: expression(c),
                                is_hidden: c
                                    .get("isHidden")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                description: text(c, "description"),
                                format_string: text(c, "formatString"),
                                display_folder: text(c, "displayFolder"),
                                cardinality: None,
                                data_size: None,
                            });
                        }
                    }
                    if let Some(ms) = t.get("measures").and_then(Value::as_array) {
                        for m in ms {
                            columns.push(Column {
                                name: text(m, "name"),
                                data_type: text(m, "dataType"),
                                kind: "Measure".into(),
                                expression: expression(m),
                                is_hidden: m
                                    .get("isHidden")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                description: text(m, "description"),
                                format_string: text(m, "formatString"),
                                display_folder: text(m, "displayFolder"),
                                cardinality: None,
                                data_size: None,
                            });
                        }
                    }
                    if let Some(hs) = t.get("hierarchies").and_then(Value::as_array) {
                        for h in hs {
                            columns.push(Column {
                                name: text(h, "name"),
                                data_type: String::new(),
                                kind: "Hierarchy".into(),
                                expression: String::new(),
                                is_hidden: h
                                    .get("isHidden")
                                    .and_then(Value::as_bool)
                                    .unwrap_or(false),
                                description: text(h, "description"),
                                format_string: String::new(),
                                display_folder: text(h, "displayFolder"),
                                cardinality: None,
                                data_size: None,
                            });
                        }
                    }
                    Table {
                        name: nonempty(text(t, "name"), "Unnamed table"),
                        columns,
                        row_count: None,
                        is_hidden: t.get("isHidden").and_then(Value::as_bool).unwrap_or(false),
                        description: text(t, "description"),
                        expression: expression(t),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let relationships = model
        .get("relationships")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .map(|r| Relationship {
                    from_table: text(r, "fromTable"),
                    from_column: text(r, "fromColumn"),
                    to_table: text(r, "toTable"),
                    to_column: text(r, "toColumn"),
                    is_active: r.get("isActive").and_then(Value::as_bool).unwrap_or(true),
                    cardinality: format!(
                        "{}:{}",
                        nonempty(text(r, "fromCardinality"), "M"),
                        nonempty(text(r, "toCardinality"), "1")
                    ),
                    cross_filtering: text(r, "crossFilteringBehavior"),
                    referential_integrity: r
                        .get("relyOnReferentialIntegrity")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    (tables, relationships)
}

/// Report measures are stored in `Report/Layout.config.modelExtensions`, not in
/// the semantic model, for thin/live-connected reports. Those packages often
/// have no DataModel or DataModelSchema at all, so omitting this source makes a
/// report with valid DAX appear to have an empty model.
fn parse_report_measure_tables(root: &Value) -> Vec<Table> {
    let config = embedded_json(root.get("config"));
    let Some(extensions) = config.get("modelExtensions").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut tables: Vec<Table> = Vec::new();
    for entity in extensions
        .iter()
        .filter_map(|extension| extension.get("entities").and_then(Value::as_array))
        .flatten()
    {
        let table_name = nonempty(text(entity, "name"), "Report measures");
        let Some(measures) = entity.get("measures").and_then(Value::as_array) else {
            continue;
        };
        let table = if let Some(table) = tables.iter_mut().find(|table| table.name == table_name) {
            table
        } else {
            tables.push(Table {
                name: table_name,
                columns: Vec::new(),
                row_count: None,
                is_hidden: false,
                description: String::new(),
                expression: String::new(),
            });
            tables
                .last_mut()
                .expect("report measure table was inserted")
        };
        for measure in measures {
            let name = text(measure, "name");
            if name.is_empty() {
                continue;
            }
            table.columns.push(Column {
                name,
                data_type: "DAX".into(),
                kind: "Report measure".into(),
                expression: expression(measure),
                is_hidden: measure
                    .get("hidden")
                    .or_else(|| measure.get("isHidden"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                description: text(measure, "description"),
                format_string: measure
                    .pointer("/formatInformation/formatString")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                display_folder: text(measure, "displayFolder"),
                cardinality: None,
                data_size: None,
            });
        }
    }
    tables.retain(|table| !table.columns.is_empty());
    tables
}

fn merge_report_measure_tables(tables: &mut Vec<Table>, report_tables: Vec<Table>) {
    for mut report_table in report_tables {
        if let Some(table) = tables
            .iter_mut()
            .find(|table| table.name == report_table.name)
        {
            let existing = table
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<HashSet<_>>();
            table.columns.extend(
                report_table
                    .columns
                    .drain(..)
                    .filter(|column| !existing.contains(&column.name)),
            );
        } else {
            tables.push(report_table);
        }
    }
}

fn parse_bookmarks(root: &Value) -> Vec<BookmarkInfo> {
    let config = embedded_json(root.get("config"));
    config
        .get("bookmarks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|bookmark| {
            let state = bookmark
                .get("explorationState")
                .cloned()
                .unwrap_or(Value::Null);
            BookmarkInfo {
                name: nonempty(text(bookmark, "displayName"), "Unnamed bookmark"),
                id: text(bookmark, "name"),
                active_page: text(&state, "activeSection"),
                hidden_visual_count: count_string_value(&state, "mode", "hidden"),
                filter_count: count_json_key(&state, "filter"),
                state,
            }
        })
        .collect()
}

fn count_mobile_layouts(root: &Value) -> usize {
    let config = embedded_json(root.get("config"));
    config
        .get("layouts")
        .or_else(|| root.get("layouts"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

fn count_json_key(value: &Value, key: &str) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key(key))
                + map
                    .values()
                    .map(|value| count_json_key(value, key))
                    .sum::<usize>()
        }
        Value::Array(values) => values.iter().map(|value| count_json_key(value, key)).sum(),
        _ => 0,
    }
}

fn count_string_value(value: &Value, key: &str, expected: &str) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.get(key).and_then(Value::as_str) == Some(expected))
                + map
                    .values()
                    .map(|value| count_string_value(value, key, expected))
                    .sum::<usize>()
        }
        Value::Array(values) => values
            .iter()
            .map(|value| count_string_value(value, key, expected))
            .sum(),
        _ => 0,
    }
}

fn parse_interactions(section_config: &Value) -> Vec<InteractionInfo> {
    section_config
        .get("relationships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|relationship| {
            let interaction_type = relationship
                .get("type")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            InteractionInfo {
                source: text(relationship, "source"),
                target: text(relationship, "target"),
                interaction_type,
                behavior: match interaction_type {
                    1 => "Filter",
                    3 => "No filter",
                    _ => "Other",
                }
                .into(),
            }
        })
        .collect()
}

fn parse_resolved_filters(semantic_query: &Value) -> Vec<FilterInfo> {
    let query = semantic_query
        .pointer("/Commands/0/SemanticQueryDataShapeCommand/Query")
        .unwrap_or(&Value::Null);
    let aliases = query
        .get("From")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| {
            Some((
                source.get("Name")?.as_str()?.to_string(),
                source.get("Entity")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    query
        .get("Where")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|where_clause| {
            let mut expression = render_filter_expression(where_clause);
            let mut ordered_aliases = aliases.iter().collect::<Vec<_>>();
            ordered_aliases.sort_by_key(|(alias, _)| std::cmp::Reverse(alias.len()));
            for (alias, entity) in ordered_aliases {
                expression = expression.replace(&format!("{alias}["), &format!("{entity}["));
            }
            FilterInfo {
                scope: "Cached merged query".into(),
                target: expression.clone(),
                kind: "Resolved Where".into(),
                expression,
                active: true,
                note: "Already merged by Power BI when this visual was last run in Desktop.".into(),
            }
        })
        .collect()
}

fn parse_column_labels(config: &Value) -> Vec<ColumnLabel> {
    config
        .pointer("/singleVisual/columnProperties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(query_ref, properties)| {
            let display_name = properties.get("displayName")?.as_str()?;
            Some(ColumnLabel {
                query_ref: query_ref.clone(),
                display_name: display_name.into(),
            })
        })
        .collect()
}

fn parse_sync_group(config: &Value) -> Option<SyncGroupInfo> {
    let group = config.pointer("/singleVisual/syncGroup")?;
    Some(SyncGroupInfo {
        group_name: text(group, "groupName"),
        field_changes: group
            .get("fieldChanges")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        filter_changes: group
            .get("filterChanges")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_diagram_tables(root: &Value) -> Vec<String> {
    root.get("diagrams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|diagram| diagram.get("nodes").and_then(Value::as_array))
        .flatten()
        .filter_map(|node| node.get("nodeIndex").and_then(Value::as_str))
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn merge_diagram_tables(tables: &mut Vec<Table>, diagram_tables: Vec<String>) {
    let mut existing = tables
        .iter()
        .map(|table| table.name.clone())
        .collect::<HashSet<_>>();
    tables.extend(
        diagram_tables
            .into_iter()
            .filter(|name| existing.insert(name.clone()))
            .map(|name| Table {
                name,
                columns: Vec::new(),
                row_count: None,
                is_hidden: false,
                description: "Table name from DiagramLayout; live model details are stored on the remote model.".into(),
                expression: String::new(),
            }),
    );
}

fn dax_query_display_name(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .replace("%20", " ")
        .replace("%23", "#")
        .trim_end_matches(".dax")
        .to_string()
}

fn apply_dax_query_metadata(queries: &mut [DaxQuery], metadata: Option<&str>) {
    let Some(root) = metadata.and_then(|value| serde_json::from_str::<Value>(value).ok()) else {
        return;
    };
    let order = find_json_key(&root, "tabOrder")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(dax_query_display_name)
        .collect::<Vec<_>>();
    let default = find_json_key(&root, "defaultTab")
        .and_then(Value::as_str)
        .map(dax_query_display_name);
    queries.sort_by_key(|query| {
        order
            .iter()
            .position(|name| name.eq_ignore_ascii_case(&query.name))
            .unwrap_or(usize::MAX)
    });
    if let Some(default) = default {
        for query in queries {
            query.is_default = query.name.eq_ignore_ascii_case(&default);
        }
    }
}

fn find_json_key<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|value| find_json_key(value, key))),
        Value::Array(values) => values.iter().find_map(|value| find_json_key(value, key)),
        _ => None,
    }
}

fn parse_aggregations(prototype_query: &Value, data_transforms: &Value) -> Vec<AggregationInfo> {
    let aliases = prototype_query
        .get("From")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| {
            Some((
                source.get("Name")?.as_str()?.to_string(),
                source.get("Entity")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    prototype_query
        .get("Select")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, selection)| {
            let aggregation = selection.get("Aggregation")?;
            let function_code = aggregation.get("Function")?.as_i64()?;
            let display_name = data_transforms
                .get("selects")
                .and_then(Value::as_array)
                .and_then(|selects| selects.get(index))
                .and_then(|select| select.get("displayName"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(AggregationInfo {
                field: render_query_expression(
                    aggregation.get("Expression").unwrap_or(&Value::Null),
                    &aliases,
                ),
                function_code,
                function_name: match function_code {
                    0 => "Sum",
                    1 => "Average",
                    _ => "Use packaged display label",
                }
                .into(),
                native_name: selection
                    .get("NativeReferenceName")
                    .or_else(|| selection.get("Name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                display_name,
            })
        })
        .collect()
}

fn render_query_expression(node: &Value, aliases: &HashMap<String, String>) -> String {
    if let Some(column) = node.get("Column") {
        let source = column
            .pointer("/Expression/SourceRef/Entity")
            .or_else(|| column.pointer("/Expression/SourceRef/Source"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let entity = aliases.get(source).map(String::as_str).unwrap_or(source);
        return format!("{entity}[{}]", text(column, "Property"));
    }
    if let Some(measure) = node.get("Measure") {
        let source = measure
            .pointer("/Expression/SourceRef/Entity")
            .or_else(|| measure.pointer("/Expression/SourceRef/Source"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let entity = aliases.get(source).map(String::as_str).unwrap_or(source);
        return format!("{entity}[{}]", text(measure, "Property"));
    }
    if let Some(aggregation) = node.get("Aggregation") {
        return render_query_expression(
            aggregation.get("Expression").unwrap_or(&Value::Null),
            aliases,
        );
    }
    compact_json(node)
}

fn parse_filters(value: Option<&Value>, scope: &str, slicer_value_list: bool) -> Vec<FilterInfo> {
    let decoded = embedded_json(value);
    let Some(filters) = decoded.as_array() else {
        return Vec::new();
    };
    filters
        .iter()
        .map(|filter| {
            let body = filter.get("filter").unwrap_or(&Value::Null);
            let active = has_filter_predicate(body);
            FilterInfo {
                scope: scope.into(),
                target: render_filter_expression(filter.get("expression").unwrap_or(&Value::Null)),
                kind: text(filter, "type"),
                expression: if active {
                    render_filter_expression(body)
                } else {
                    "No predicate (inactive placeholder)".into()
                },
                active,
                note: if slicer_value_list {
                    "Restricts this slicer's value list; it does not filter the page.".into()
                } else {
                    String::new()
                },
            }
        })
        .collect()
}

fn parse_slicer_selections(config: &Value) -> Vec<FilterInfo> {
    [
        "/singleVisual/objects/general",
        "/singleVisual/vcObjects/general",
    ]
    .into_iter()
    .filter_map(|pointer| config.pointer(pointer).and_then(Value::as_array))
    .flatten()
    .filter_map(|item| item.pointer("/properties/filter"))
    .map(|selection| {
        let body = selection.get("filter").unwrap_or(selection);
        FilterInfo {
            scope: "Slicer selection".into(),
            target: "Saved selection".into(),
            kind: "Selection".into(),
            expression: render_filter_expression(body),
            active: has_filter_predicate(body),
            note: "Applies to the page through the slicer.".into(),
        }
    })
    .collect()
}

fn has_filter_predicate(node: &Value) -> bool {
    match node {
        Value::Null => false,
        Value::Array(values) => values.iter().any(has_filter_predicate),
        Value::Object(map) => {
            if let Some(where_clause) = map.get("Where") {
                return has_filter_predicate(where_clause);
            }
            map.iter().any(|(key, value)| {
                !matches!(key.as_str(), "Version" | "From") && has_filter_predicate(value)
            })
        }
        Value::String(value) => !value.is_empty(),
        _ => true,
    }
}

fn render_filter_expression(node: &Value) -> String {
    if node.is_null() {
        return String::new();
    }
    if let Some(literal) = node.get("Literal") {
        return literal
            .get("Value")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default();
    }
    if let Some(column) = node.get("Column") {
        let entity = column
            .pointer("/Expression/SourceRef/Entity")
            .or_else(|| column.pointer("/Expression/SourceRef/Source"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        return format!("{entity}[{}]", text(column, "Property"));
    }
    if let Some(measure) = node.get("Measure") {
        let entity = measure
            .pointer("/Expression/SourceRef/Entity")
            .or_else(|| measure.pointer("/Expression/SourceRef/Source"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        return format!("{entity}[{}] (measure)", text(measure, "Property"));
    }
    if let Some(level) = node.get("HierarchyLevel") {
        let entity = level
            .pointer("/Expression/Hierarchy/Expression/SourceRef/Entity")
            .or_else(|| level.pointer("/Expression/Hierarchy/Expression/SourceRef/Source"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        return format!("{entity}[{}]", text(level, "Level"));
    }
    if let Some(aggregation) = node.get("Aggregation") {
        let code = aggregation
            .get("Function")
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        return format!(
            "Aggregation {code}({})",
            render_filter_expression(aggregation.get("Expression").unwrap_or(&Value::Null))
        );
    }
    if node.get("Now").is_some() {
        return "Now()".into();
    }
    if let Some(date_add) = node.get("DateAdd") {
        let units = ["Day", "Week", "Month", "Year", "Hour", "Minute", "Second"];
        let unit = date_add
            .get("TimeUnit")
            .and_then(Value::as_u64)
            .and_then(|index| units.get(index as usize))
            .copied()
            .unwrap_or("Unit");
        return format!(
            "DateAdd({}, {} {unit})",
            render_filter_expression(date_add.get("Expression").unwrap_or(&Value::Null)),
            date_add.get("Amount").and_then(Value::as_i64).unwrap_or(0)
        );
    }
    if let Some(date_span) = node.get("DateSpan") {
        let units = ["Day", "Week", "Month", "Year", "Hour", "Minute", "Second"];
        let unit = date_span
            .get("TimeUnit")
            .and_then(Value::as_u64)
            .and_then(|index| units.get(index as usize))
            .copied()
            .unwrap_or("Unit");
        return format!(
            "TruncTo{unit}({})",
            render_filter_expression(date_span.get("Expression").unwrap_or(&Value::Null))
        );
    }
    if let Some(in_clause) = node.get("In") {
        let expressions = render_expression_list(in_clause.get("Expressions"));
        let values = in_clause
            .get("Values")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|row| render_expression_list(Some(row)))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("{expressions} IN ({values})");
    }
    if let Some(not) = node.get("Not") {
        return format!(
            "NOT ({})",
            render_filter_expression(not.get("Expression").unwrap_or(&Value::Null))
        );
    }
    if let Some(between) = node.get("Between") {
        return format!(
            "{} BETWEEN {} AND {}",
            render_filter_expression(between.get("Expression").unwrap_or(&Value::Null)),
            render_filter_expression(between.get("LowerBound").unwrap_or(&Value::Null)),
            render_filter_expression(between.get("UpperBound").unwrap_or(&Value::Null))
        );
    }
    if let Some(comparison) = node.get("Comparison") {
        let operators = ["=", ">", ">=", "<", "<="];
        let operator = comparison
            .get("ComparisonKind")
            .and_then(Value::as_u64)
            .and_then(|index| operators.get(index as usize))
            .copied()
            .unwrap_or("?");
        return format!(
            "{} {operator} {}",
            render_filter_expression(comparison.get("Left").unwrap_or(&Value::Null)),
            render_filter_expression(comparison.get("Right").unwrap_or(&Value::Null))
        );
    }
    if let Some(arithmetic) = node.get("Arithmetic") {
        let operators = ["+", "-", "*", "/"];
        let operator = arithmetic
            .get("Operator")
            .and_then(Value::as_u64)
            .and_then(|index| operators.get(index as usize))
            .copied()
            .unwrap_or("?");
        return format!(
            "({}) {operator} ({})",
            render_filter_expression(arithmetic.get("Left").unwrap_or(&Value::Null)),
            render_filter_expression(arithmetic.get("Right").unwrap_or(&Value::Null))
        );
    }
    if let Some(scoped) = node.get("ScopedEval") {
        let scope = scoped
            .get("Scope")
            .map(compact_json)
            .unwrap_or_else(|| "[]".into());
        return format!(
            "ScopedEval({}, Scope: {scope})",
            render_filter_expression(scoped.get("Expression").unwrap_or(&Value::Null))
        );
    }
    for operator in ["And", "Or"] {
        if let Some(branch) = node.get(operator) {
            return format!(
                "({}) {operator} ({})",
                render_filter_expression(branch.get("Left").unwrap_or(&Value::Null)),
                render_filter_expression(branch.get("Right").unwrap_or(&Value::Null))
            );
        }
    }
    if let Some(condition) = node.get("Condition") {
        return render_filter_expression(condition);
    }
    if let Some(where_clause) = node.get("Where") {
        return render_expression_list(Some(where_clause));
    }
    if let Some(expression) = node.get("Expression") {
        return render_filter_expression(expression);
    }
    compact_json(node)
}

fn render_expression_list(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .map(render_filter_expression)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" AND "),
        Some(value) => render_filter_expression(value),
        None => String::new(),
    }
}

fn compact_json(value: &Value) -> String {
    let rendered = serde_json::to_string(value).unwrap_or_default();
    if rendered.chars().count() > 400 {
        format!("{}…", rendered.chars().take(400).collect::<String>())
    } else {
        rendered
    }
}

fn parse_sources(root: Value) -> Vec<Source> {
    let mut found = Vec::new();
    collect_sources(&root, "", &mut found);
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|s| seen.insert(s.detail.clone()))
        .take(50)
        .collect()
}

fn parse_aas_connection(root: &Value) -> Option<AasConnection> {
    let connection = root
        .get("Connections")
        .and_then(Value::as_array)?
        .iter()
        .find(|connection| {
            text(connection, "ConnectionType").eq_ignore_ascii_case("analysisServicesDatabaseLive")
        })?;
    let connection_string = connection.get("ConnectionString")?.as_str()?;
    let mut properties = HashMap::new();
    for part in connection_string.split(';') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        properties.insert(
            key.trim().to_ascii_lowercase(),
            value.trim().trim_matches('"').to_string(),
        );
    }
    let server_url = properties.get("data source")?.clone();
    Some(AasConnection {
        server_url,
        catalog: properties
            .get("initial catalog")
            .cloned()
            .unwrap_or_default(),
        cube: properties.get("cube").cloned().unwrap_or_default(),
        connection_type: text(connection, "ConnectionType"),
    })
}
fn parse_mashup(bytes: &[u8]) -> (Vec<Query>, Vec<Source>) {
    if bytes.len() < 12 {
        return Default::default();
    }
    let length = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if length == 0 || 8 + length > bytes.len() {
        return Default::default();
    }
    let mut package = match ZipArchive::new(Cursor::new(&bytes[8..8 + length])) {
        Ok(z) => z,
        Err(_) => return Default::default(),
    };
    let mut formulas = String::new();
    let mut item = match package.by_name("Formulas/Section1.m") {
        Ok(f) => f,
        Err(_) => return Default::default(),
    };
    if item.read_to_string(&mut formulas).is_err() {
        return Default::default();
    }
    let normalized = formulas.replace('\u{00a0}', " ");
    let mut queries = Vec::new();
    for chunk in normalized.split("\nshared ").skip(1) {
        let Some((raw_name, body)) = chunk.split_once(" =") else {
            continue;
        };
        let name = raw_name
            .trim()
            .trim_start_matches("#\"")
            .trim_end_matches('"')
            .to_string();
        let preview = body
            .lines()
            .find(|l| l.trim_start().starts_with("Source ="))
            .map(|l| l.trim().trim_end_matches(',').chars().take(180).collect())
            .unwrap_or_default();
        let formula = body.trim().trim_end_matches(';').to_string();
        let connectors = detect_connectors(&formula);
        let has_native_query = formula.contains("Value.NativeQuery")
            || formula.contains("Odbc.Query")
            || formula.contains("OleDb.Query");
        queries.push(Query {
            name,
            preview,
            formula,
            connectors,
            dependencies: Vec::new(),
            has_native_query,
        });
    }
    let query_names: Vec<String> = queries.iter().map(|q| q.name.clone()).collect();
    for query in &mut queries {
        query.dependencies = query_names
            .iter()
            .filter(|name| **name != query.name)
            .filter(|name| {
                query.formula.contains(&format!("#\"{name}\""))
                    || query.formula.contains(&format!(" {name},"))
                    || query.formula.contains(&format!(" {name}\n"))
            })
            .cloned()
            .collect();
    }
    let mut sources = Vec::new();
    for segment in normalized.split('"').skip(1).step_by(2) {
        let s = segment.trim();
        if s.starts_with("http://") || s.starts_with("https://") {
            sources.push(Source {
                kind: if s.contains("sharepoint") {
                    "SharePoint".into()
                } else {
                    "Web".into()
                },
                detail: s.to_string(),
            });
        } else if s.starts_with("file://")
            || (s.starts_with('/') && s.len() > 3 && (s.contains('/') || s.contains('\\')))
        {
            sources.push(Source {
                kind: "File".into(),
                detail: s.to_string(),
            });
        }
    }
    for (call, kind) in [
        ("Sql.Database(", "SQL Server"),
        ("MySQL.Database(", "MySQL"),
        ("PostgreSQL.Database(", "PostgreSQL"),
        ("Oracle.Database(", "Oracle"),
        ("Snowflake.Databases(", "Snowflake"),
        ("OData.Feed(", "OData"),
        ("SharePoint.Contents(", "SharePoint"),
        ("SharePoint.Files(", "SharePoint"),
        ("File.Contents(", "File"),
        ("Folder.Files(", "Folder"),
    ] {
        for detail in extract_call_arguments(&normalized, call) {
            sources.push(Source {
                kind: kind.into(),
                detail,
            });
        }
    }
    (queries, sources)
}

fn build_queries(items: Vec<(String, String)>) -> Vec<Query> {
    let names: Vec<String> = items.iter().map(|(name, _)| name.clone()).collect();
    items
        .into_iter()
        .map(|(name, formula)| {
            let preview = formula
                .lines()
                .find(|line| line.trim_start().starts_with("Source ="))
                .map(|line| {
                    line.trim()
                        .trim_end_matches(',')
                        .chars()
                        .take(180)
                        .collect()
                })
                .unwrap_or_default();
            let dependencies = names
                .iter()
                .filter(|other| **other != name)
                .filter(|other| {
                    formula.contains(&format!("#\"{other}\""))
                        || formula.contains(&format!(" {other},"))
                        || formula.contains(&format!(" {other}\n"))
                })
                .cloned()
                .collect();
            let connectors = detect_connectors(&formula);
            let has_native_query = formula.contains("Value.NativeQuery")
                || formula.contains("Odbc.Query")
                || formula.contains("OleDb.Query");
            Query {
                name,
                preview,
                formula,
                connectors,
                dependencies,
                has_native_query,
            }
        })
        .collect()
}

fn sources_from_queries(queries: &[Query]) -> Vec<Source> {
    let text = queries
        .iter()
        .map(|query| query.formula.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut sources = Vec::new();
    for segment in text.split('"').skip(1).step_by(2) {
        let value = segment.trim();
        if value.starts_with("http://") || value.starts_with("https://") {
            sources.push(Source {
                kind: if value.contains("sharepoint") {
                    "SharePoint".into()
                } else {
                    "Web".into()
                },
                detail: value.into(),
            });
        }
    }
    for (call, kind) in [
        ("Sql.Database(", "SQL Server"),
        ("MySQL.Database(", "MySQL"),
        ("PostgreSQL.Database(", "PostgreSQL"),
        ("Oracle.Database(", "Oracle"),
        ("Snowflake.Databases(", "Snowflake"),
        ("OData.Feed(", "OData"),
        ("SharePoint.Contents(", "SharePoint"),
        ("SharePoint.Files(", "SharePoint"),
        ("File.Contents(", "File"),
        ("Folder.Files(", "Folder"),
    ] {
        for detail in extract_call_arguments(&text, call) {
            sources.push(Source {
                kind: kind.into(),
                detail,
            });
        }
    }
    let mut seen = HashSet::new();
    sources
        .into_iter()
        .filter(|source| seen.insert(source.detail.clone()))
        .collect()
}

fn detect_connectors(formula: &str) -> Vec<String> {
    let known = [
        "Web.Contents",
        "Sql.Database",
        "Odbc.Query",
        "Odbc.DataSource",
        "OleDb.Query",
        "Excel.Workbook",
        "Csv.Document",
        "Json.Document",
        "SharePoint.Files",
        "SharePoint.Contents",
        "Folder.Files",
        "File.Contents",
        "PostgreSQL.Database",
        "MySQL.Database",
        "Oracle.Database",
        "Snowflake.Databases",
        "OData.Feed",
        "AnalysisServices.Database",
        "Value.NativeQuery",
    ];
    known
        .iter()
        .filter(|name| formula.contains(**name))
        .map(|s| (*s).to_string())
        .collect()
}

fn extract_call_arguments(text: &str, needle: &str) -> Vec<String> {
    let mut results = Vec::new();
    for tail in text.split(needle).skip(1) {
        let args = tail.split(')').next().unwrap_or("");
        let quoted: Vec<&str> = args
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|s| !s.is_empty())
            .take(2)
            .collect();
        if !quoted.is_empty() {
            results.push(quoted.join(" / "));
        }
    }
    results
}
fn collect_sources(v: &Value, parent: &str, out: &mut Vec<Source>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let key = k.to_ascii_lowercase();
                if let Some(s) = val.as_str() {
                    if [
                        "connectionstring",
                        "server",
                        "database",
                        "url",
                        "path",
                        "datasource",
                        "address",
                    ]
                    .iter()
                    .any(|x| key.contains(x))
                        && !s.is_empty()
                    {
                        out.push(Source {
                            kind: if parent.is_empty() {
                                k.clone()
                            } else {
                                parent.into()
                            },
                            detail: s.into(),
                        });
                    }
                }
                collect_sources(val, k, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_sources(x, parent, out);
            }
        }
        _ => {}
    }
}
fn embedded_json(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    }
}
fn collect_named(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let (Some(src), Some(prop)) = (
                m.get("Source").and_then(Value::as_str),
                m.get("Property").and_then(Value::as_str),
            ) {
                out.push(format!("{src}.{prop}"));
            }
            for (k, x) in m {
                if (k == "Property" || k == "Measure") && x.is_string() {
                    out.push(x.as_str().unwrap().into())
                }
                collect_named(x, out);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_named(x, out)
            }
        }
        _ => {}
    }
}

fn collect_bound_fields(query: &Value, out: &mut Vec<String>) {
    let aliases = query
        .get("From")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| {
            Some((
                source.get("Name")?.as_str()?.to_string(),
                source.get("Entity")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    collect_bound_fields_inner(query, &aliases, out);
}

fn collect_bound_fields_inner(
    node: &Value,
    aliases: &HashMap<String, String>,
    out: &mut Vec<String>,
) {
    match node {
        Value::Object(map) => {
            for key in ["Column", "Measure"] {
                if let Some(field) = map.get(key) {
                    let source = field
                        .pointer("/Expression/SourceRef/Entity")
                        .or_else(|| field.pointer("/Expression/SourceRef/Source"))
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    let entity = aliases.get(source).map(String::as_str).unwrap_or(source);
                    let property = field.get("Property").and_then(Value::as_str).unwrap_or("");
                    if !property.is_empty() {
                        out.push(format!("{entity}[{property}]"));
                    }
                }
            }
            if let Some(level) = map.get("HierarchyLevel") {
                let source = level
                    .pointer("/Expression/Hierarchy/Expression/SourceRef/Entity")
                    .or_else(|| level.pointer("/Expression/Hierarchy/Expression/SourceRef/Source"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                let entity = aliases.get(source).map(String::as_str).unwrap_or(source);
                let name = level.get("Level").and_then(Value::as_str).unwrap_or("");
                if !name.is_empty() {
                    out.push(format!("{entity}[{name}]"));
                }
            }
            for value in map.values() {
                collect_bound_fields_inner(value, aliases, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_bound_fields_inner(value, aliases, out);
            }
        }
        _ => {}
    }
}
fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}
fn expression(v: &Value) -> String {
    match v.get("expression") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(lines)) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
fn num(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}
fn nonempty(s: String, fallback: &str) -> String {
    if s.trim().is_empty() {
        fallback.into()
    } else {
        s
    }
}
fn label_type(s: &str) -> String {
    if s.eq_ignore_ascii_case("kpi") {
        return "KPI".into();
    }
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ')
        }
        out.push(c)
    }
    let mut cs = out.chars();
    cs.next()
        .map(|c| c.to_uppercase().collect::<String>() + cs.as_str())
        .unwrap_or_else(|| "Visual".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_report_level_measures_from_stringified_layout_config() {
        let config = serde_json::json!({
            "modelExtensions": [{
                "entities": [{
                    "name": "Fact Stay",
                    "measures": [{
                        "name": "Hospitalizations",
                        "expression": [
                            "SUMX('Fact Stay', ",
                            "IF('Fact Stay'[EDVisitInd] = 0, 1, 0))"
                        ],
                        "hidden": true,
                        "formatInformation": { "formatString": "#,0" }
                    }]
                }]
            }]
        });
        let layout = serde_json::json!({ "config": config.to_string() });

        let tables = parse_report_measure_tables(&layout);

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "Fact Stay");
        assert_eq!(tables[0].columns.len(), 1);
        let measure = &tables[0].columns[0];
        assert_eq!(measure.name, "Hospitalizations");
        assert_eq!(measure.kind, "Report measure");
        assert_eq!(
            measure.expression,
            "SUMX('Fact Stay', \nIF('Fact Stay'[EDVisitInd] = 0, 1, 0))"
        );
        assert!(measure.is_hidden);
        assert_eq!(measure.format_string, "#,0");
    }

    #[test]
    fn merges_report_measures_without_duplicating_model_measures() {
        let mut tables = vec![Table {
            name: "Measures".into(),
            columns: vec![Column {
                name: "Existing".into(),
                data_type: "DAX".into(),
                kind: "Measure".into(),
                expression: "1".into(),
                is_hidden: false,
                description: String::new(),
                format_string: String::new(),
                display_folder: String::new(),
                cardinality: None,
                data_size: None,
            }],
            row_count: None,
            is_hidden: false,
            description: String::new(),
            expression: String::new(),
        }];
        let report_tables = vec![Table {
            name: "Measures".into(),
            columns: vec![
                Column {
                    name: "Existing".into(),
                    data_type: "DAX".into(),
                    kind: "Report measure".into(),
                    expression: "2".into(),
                    is_hidden: false,
                    description: String::new(),
                    format_string: String::new(),
                    display_folder: String::new(),
                    cardinality: None,
                    data_size: None,
                },
                Column {
                    name: "Report only".into(),
                    data_type: "DAX".into(),
                    kind: "Report measure".into(),
                    expression: "3".into(),
                    is_hidden: false,
                    description: String::new(),
                    format_string: String::new(),
                    display_folder: String::new(),
                    cardinality: None,
                    data_size: None,
                },
            ],
            row_count: None,
            is_hidden: false,
            description: String::new(),
            expression: String::new(),
        }];

        merge_report_measure_tables(&mut tables, report_tables);

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].columns.len(), 2);
        assert_eq!(tables[0].columns[0].expression, "1");
        assert_eq!(tables[0].columns[1].name, "Report only");
    }

    #[test]
    fn reads_visual_aggregation_without_trusting_the_stale_name() {
        let query = serde_json::json!({
            "From": [{ "Name": "f", "Entity": "FactSLHospitalization" }],
            "Select": [{
                "Aggregation": {
                    "Expression": {
                        "Column": {
                            "Expression": { "SourceRef": { "Source": "f" } },
                            "Property": "NumberOfDaysInHospital"
                        }
                    },
                    "Function": 1
                },
                "Name": "Sum(FactSLHospitalization.NumberOfDaysInHospital)",
                "NativeReferenceName": "Average of NumberOfDaysInHospital"
            }]
        });

        let data_transforms = serde_json::json!({
            "selects": [{ "displayName": "Average of NumberOfDaysInHospital" }]
        });
        let aggregations = parse_aggregations(&query, &data_transforms);

        assert_eq!(aggregations.len(), 1);
        assert_eq!(
            aggregations[0].field,
            "FactSLHospitalization[NumberOfDaysInHospital]"
        );
        assert_eq!(aggregations[0].function_code, 1);
        assert_eq!(aggregations[0].function_name, "Average");
        assert_eq!(
            aggregations[0].native_name,
            "Average of NumberOfDaysInHospital"
        );
        assert_eq!(
            aggregations[0].display_name,
            "Average of NumberOfDaysInHospital"
        );
    }

    #[test]
    fn resolves_visual_bindings_and_cached_where_aliases() {
        let prototype = serde_json::json!({
            "From": [{ "Name": "f", "Entity": "Fact Stay" }],
            "Select": [{
                "Measure": {
                    "Expression": { "SourceRef": { "Source": "f" } },
                    "Property": "Hospitalizations"
                }
            }]
        });
        let mut fields = Vec::new();
        collect_bound_fields(&prototype, &mut fields);
        assert_eq!(fields, vec!["Fact Stay[Hospitalizations]"]);

        let cached = serde_json::json!({
            "Commands": [{
                "SemanticQueryDataShapeCommand": {
                    "Query": {
                        "From": [{ "Name": "f", "Entity": "Fact Stay" }],
                        "Where": [{
                            "Condition": {
                                "Comparison": {
                                    "ComparisonKind": 0,
                                    "Left": {
                                        "Column": {
                                            "Expression": { "SourceRef": { "Source": "f" } },
                                            "Property": "Active"
                                        }
                                    },
                                    "Right": { "Literal": { "Value": "true" } }
                                }
                            }
                        }]
                    }
                }
            }]
        });
        let filters = parse_resolved_filters(&cached);
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].expression, "Fact Stay[Active] = true");
    }

    #[test]
    fn renders_arithmetic_and_scoped_filter_expressions() {
        let expression = serde_json::json!({
            "ScopedEval": {
                "Expression": {
                    "Arithmetic": {
                        "Operator": 3,
                        "Left": { "Literal": { "Value": "10" } },
                        "Right": { "Literal": { "Value": "2" } }
                    }
                },
                "Scope": ["Visual"]
            }
        });
        let rendered = render_filter_expression(&expression);
        assert!(rendered.contains("(10) / (2)"));
        assert!(rendered.contains("ScopedEval"));
    }

    #[test]
    fn reads_exact_live_connection_values() {
        let connections = serde_json::json!({
            "Connections": [{
                "ConnectionString": "Data Source=asazure://region.asazure.windows.net/server;Initial Catalog=\"Exact Catalog \";Cube=ExactCube;Access Mode=readonly",
                "ConnectionType": "analysisServicesDatabaseLive"
            }]
        });
        let connection = parse_aas_connection(&connections).unwrap();
        assert_eq!(connection.catalog, "Exact Catalog ");
        assert_eq!(connection.cube, "ExactCube");
    }

    #[test]
    fn distinguishes_slicer_value_restrictions_from_saved_selections() {
        let filters = serde_json::json!([{
            "expression": {
                "Column": {
                    "Expression": { "SourceRef": { "Entity": "DimDischargeLocation" } },
                    "Property": "AcuteCareInd"
                }
            },
            "type": "Categorical",
            "filter": {
                "Where": [{
                    "Condition": {
                        "In": {
                            "Expressions": [{
                                "Column": {
                                    "Expression": { "SourceRef": { "Source": "d" } },
                                    "Property": "AcuteCareInd"
                                }
                            }],
                            "Values": [[{ "Literal": { "Value": "true" } }]]
                        }
                    }
                }]
            }
        }]);
        let encoded_filters = Value::String(filters.to_string());
        let parsed = parse_filters(Some(&encoded_filters), "Visual", true);
        let config = serde_json::json!({
            "singleVisual": {
                "objects": {
                    "general": [{
                        "properties": {
                            "filter": {
                                "filter": {
                                    "Where": [{
                                        "Condition": {
                                            "In": {
                                                "Expressions": [{
                                                    "Column": {
                                                        "Expression": { "SourceRef": { "Source": "d" } },
                                                        "Property": "ReportingGroupName"
                                                    }
                                                }],
                                                "Values": [[{ "Literal": { "Value": "'All Communities'" } }]]
                                            }
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        let selections = parse_slicer_selections(&config);

        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].active);
        assert!(parsed[0].note.contains("does not filter the page"));
        assert_eq!(selections.len(), 1);
        assert!(selections[0].note.contains("Applies to the page"));
        assert!(selections[0].expression.contains("All Communities"));
    }

    #[test]
    fn adds_live_model_table_names_from_diagram_layout() {
        let layout = serde_json::json!({
            "diagrams": [{
                "nodes": [
                    { "nodeIndex": "FactSLHospitalization" },
                    { "nodeIndex": "DimSLFacility" },
                    { "nodeIndex": "FactSLHospitalization" }
                ]
            }]
        });
        let names = parse_diagram_tables(&layout);
        let mut tables = Vec::new();

        merge_diagram_tables(&mut tables, names);

        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].name, "DimSLFacility");
        assert_eq!(tables[1].name, "FactSLHospitalization");
        assert!(tables[0].description.contains("DiagramLayout"));
    }

    #[test]
    fn applies_saved_dax_tab_order_and_default_metadata() {
        let mut queries = vec![
            DaxQuery {
                name: "Second".into(),
                path: "DAXQueries/Second.dax".into(),
                expression: "EVALUATE { 2 }".into(),
                is_default: false,
            },
            DaxQuery {
                name: "First".into(),
                path: "DAXQueries/First.dax".into(),
                expression: "EVALUATE { 1 }".into(),
                is_default: false,
            },
        ];
        let metadata = serde_json::json!({
            "state": {
                "tabOrder": ["First", "Second"],
                "defaultTab": "Second"
            }
        });

        apply_dax_query_metadata(&mut queries, Some(&metadata.to_string()));

        assert_eq!(queries[0].name, "First");
        assert_eq!(queries[1].name, "Second");
        assert!(queries[1].is_default);
    }
}
