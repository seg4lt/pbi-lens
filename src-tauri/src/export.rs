use crate::pbix::{parse_report, read_entry};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::{self, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

const EXPORT_FORMAT: &str = "pbi-lens-pbix-anatomy";
const EXPORT_FORMAT_VERSION: u32 = 1;
const MAX_RAW_TEXT_ENTRY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RAW_TEXT_TOTAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RawTextEntry {
    name: String,
    content: Value,
    truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkippedEntry {
    name: String,
    size: u64,
    reason: String,
}

pub fn default_export_path(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("report");
    input.with_file_name(format!("{stem}.pbilens.json"))
}

pub fn export_report(input: &Path, force: bool) -> Result<PathBuf, String> {
    let output = default_export_path(input);
    if output.exists() && !force {
        return Err(format!(
            "Export already exists: {}. Pass --force to replace it.",
            output.display()
        ));
    }
    let report = parse_report(input)?;
    let mut raw_entries = Vec::new();
    let mut skipped_entries = Vec::new();
    let mut raw_total = 0usize;
    for entry in &report.entries {
        if entry.size > MAX_RAW_TEXT_ENTRY_BYTES {
            skipped_entries.push(SkippedEntry {
                name: entry.name.clone(),
                size: entry.size,
                reason: "Entry exceeds the per-file text export limit.".into(),
            });
            continue;
        }
        if raw_total >= MAX_RAW_TEXT_TOTAL_BYTES {
            skipped_entries.push(SkippedEntry {
                name: entry.name.clone(),
                size: entry.size,
                reason: "Export reached the aggregate decoded-text safety limit.".into(),
            });
            continue;
        }
        match read_entry(input, &entry.name) {
            Ok(content) if content.kind == "Text" => {
                raw_total = raw_total.saturating_add(content.content.len());
                let value = serde_json::from_str(&content.content)
                    .unwrap_or_else(|_| Value::String(content.content));
                raw_entries.push(RawTextEntry {
                    name: entry.name.clone(),
                    content: value,
                    truncated: content.truncated,
                });
            }
            Ok(_) => skipped_entries.push(SkippedEntry {
                name: entry.name.clone(),
                size: entry.size,
                reason: "Binary package entry; manifest metadata is retained.".into(),
            }),
            Err(error) => skipped_entries.push(SkippedEntry {
                name: entry.name.clone(),
                size: entry.size,
                reason: format!("Could not decode entry: {error}"),
            }),
        }
    }

    let mut report_value = serde_json::to_value(&report)
        .map_err(|error| format!("Could not serialize extracted report anatomy: {error}"))?;
    if let Some(object) = report_value.as_object_mut() {
        object.remove("path");
    }
    let mut document = json!({
        "format": EXPORT_FORMAT,
        "formatVersion": EXPORT_FORMAT_VERSION,
        "generator": {
            "name": "PBI Lens",
            "version": env!("CARGO_PKG_VERSION")
        },
        "source": {
            "fileName": report.name,
            "fileType": report.kind,
            "size": report.size
        },
        "coverage": {
            "normalizedAnatomy": true,
            "decodedPackageTextEntries": raw_entries.len(),
            "skippedPackageEntries": skipped_entries.len(),
            "importedTableRows": "not exported",
            "liveModelValues": "not available in PBIX/PBIT packages",
            "credentialPolicy": "runner credentials are never part of report exports"
        },
        "report": report_value,
        "decodedPackageEntries": raw_entries,
        "skippedPackageEntries": skipped_entries
    });
    redact_sensitive_keys(&mut document);
    write_json_atomically(&output, &document, force)?;
    Ok(output)
}

fn write_json_atomically(output: &Path, value: &Value, force: bool) -> Result<(), String> {
    let parent = output
        .parent()
        .ok_or_else(|| "Export path has no parent directory.".to_string())?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("report.pbilens.json");
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Could not create temporary export: {error}"))?;
    let mut writer = BufWriter::new(file);
    let result = serde_json::to_writer_pretty(&mut writer, value)
        .map_err(|error| format!("Could not encode export JSON: {error}"))
        .and_then(|_| {
            writer
                .write_all(b"\n")
                .and_then(|_| writer.flush())
                .map_err(|error| format!("Could not finish export: {error}"))
        });
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if output.exists() {
        if !force {
            let _ = fs::remove_file(&temporary);
            return Err(format!("Export already exists: {}", output.display()));
        }
        fs::remove_file(output)
            .map_err(|error| format!("Could not replace existing export: {error}"))?;
    }
    fs::rename(&temporary, output).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("Could not publish export: {error}")
    })
}

fn redact_sensitive_keys(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let normalized = key.to_ascii_lowercase().replace(['-', '_', ' '], "");
                if [
                    "password",
                    "pwd",
                    "secret",
                    "clientsecret",
                    "accesstoken",
                    "refreshtoken",
                    "authorization",
                    "credential",
                    "credentials",
                    "customdata",
                    "role",
                    "roles",
                ]
                .iter()
                .any(|sensitive| normalized.contains(sensitive))
                {
                    *child = Value::String("[REDACTED]".into());
                } else {
                    redact_sensitive_keys(child);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                redact_sensitive_keys(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_export_name_beside_the_source() {
        assert_eq!(
            default_export_path(Path::new("/reports/example.pbix")),
            PathBuf::from("/reports/example.pbilens.json")
        );
    }

    #[test]
    fn redacts_sensitive_fields_but_preserves_anatomy() {
        let mut value = json!({
            "serverUrl": "asazure://packaged-server",
            "clientSecret": "do-not-export",
            "nested": { "CUSTOMDATA": "identity", "table": "Fact" }
        });
        redact_sensitive_keys(&mut value);
        assert_eq!(value["clientSecret"], "[REDACTED]");
        assert_eq!(value["nested"]["CUSTOMDATA"], "[REDACTED]");
        assert_eq!(value["nested"]["table"], "Fact");
        assert_eq!(value["serverUrl"], "asazure://packaged-server");
    }
}
