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
    pub visuals: Vec<Visual>,
}
#[derive(Serialize)]
pub struct Visual {
    pub visual_type: String,
    pub visual_type_label: String,
    pub title: String,
    pub x_pct: f64,
    pub y_pct: f64,
    pub w_pct: f64,
    pub h_pct: f64,
    pub z_index: i64,
    pub fields: Vec<String>,
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
        } else if (normalized == "datamashup" || normalized.ends_with("/datamashup"))
            && item.size() <= 64 * 1024 * 1024
        {
            let mut bytes = Vec::with_capacity(item.size() as usize);
            if item.read_to_end(&mut bytes).is_ok() {
                mashup = Some(bytes);
            }
        }
    }
    let pages = layout
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .map(parse_pages)
        .unwrap_or_default();
    let (mut tables, mut relationships) = schema
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .map(parse_model)
        .unwrap_or_default();
    let mut sources = connections
        .as_deref()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .map(parse_sources)
        .unwrap_or_default();
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
    let bookmark_hidden = bookmark_hidden_visuals(&root);
    let sections = root
        .get("sections")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    sections
        .into_iter()
        .map(|s| {
            let section_name = text(&s, "name");
            let hidden_visuals = bookmark_hidden.get(&section_name);
            let width = num(&s, "width", 1280.0);
            let height = num(&s, "height", 720.0);
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
                            if hidden_visuals
                                .is_some_and(|items| items.contains(&text(&config, "name")))
                            {
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
                visuals,
            }
        })
        .collect()
}

fn bookmark_hidden_visuals(root: &Value) -> HashMap<String, HashSet<String>> {
    let config = embedded_json(root.get("config"));
    let mut candidates: HashMap<String, (usize, HashSet<String>)> = HashMap::new();
    let Some(bookmarks) = config.get("bookmarks").and_then(Value::as_array) else {
        return HashMap::new();
    };

    for bookmark in bookmarks {
        let state = bookmark.get("explorationState").unwrap_or(&Value::Null);
        let section = text(state, "activeSection");
        let Some(visuals) = state
            .pointer(&format!("/sections/{section}/visualContainers"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let hidden = visuals
            .iter()
            .filter_map(|(name, value)| {
                (value
                    .pointer("/singleVisual/display/mode")
                    .and_then(Value::as_str)
                    == Some("hidden"))
                .then(|| name.clone())
            })
            .collect::<HashSet<_>>();
        if hidden.is_empty() {
            continue;
        }
        // A PBIX stores bookmark-controlled alternatives in the page alongside the
        // currently useful visuals. Rendering every alternative at once creates
        // stacked maps, tables, and popup panels. The state hiding the most layers
        // is the safest neutral preview; ties retain the report author's first state.
        let score = hidden.len();
        if candidates
            .get(&section)
            .is_none_or(|(best, _)| score > *best)
        {
            candidates.insert(section, (score, hidden));
        }
    }

    candidates
        .into_iter()
        .map(|(section, (_, hidden))| (section, hidden))
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
    let mut strings = Vec::new();
    collect_named(&config, &mut strings);
    collect_named(&query, &mut strings);
    collect_named(&data, &mut strings);
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
        visual_type_label: label_type(&visual_type),
        visual_type,
        title,
        x_pct: x / page_w * 100.0,
        y_pct: y / page_h * 100.0,
        w_pct: w / page_w * 100.0,
        h_pct: h / page_h * 100.0,
        z_index: num(v, "z", 0.0) as i64 + offset_z,
        fields,
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
