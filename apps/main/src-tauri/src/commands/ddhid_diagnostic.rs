//! Unified driver and display diagnostic report export.

#[allow(unused_imports)]
use crate::commands::repair::{DiagnosticItem, ItemStatus, RepairReport, Severity};
#[allow(unused_imports)]
use std::fmt::Write as _;
use tauri::AppHandle;
#[allow(unused_imports)]
use tauri::Manager;

#[allow(dead_code)]
const DIAGNOSE_SCRIPT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../scripts/diagnose-environment.ps1"
));

#[tauri::command]
pub async fn export_dd_hid_diagnostic_report(app: AppHandle) -> Result<String, String> {
    #[cfg(windows)]
    {
        export_report_windows(app).await
    }

    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅 Windows 平台支持导出环境诊断报告".to_string())
    }
}

#[cfg(windows)]
async fn export_report_windows(app: AppHandle) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let diagnostics_dir = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join(crate::APP_IDENTIFIER))
        .join("diagnostics");
    std::fs::create_dir_all(&diagnostics_dir).map_err(|e| format!("创建诊断目录失败: {e}"))?;

    let script_path = diagnostics_dir.join("diagnose-environment.ps1");
    write_script_with_bom(&script_path)?;
    let script_path_for_task = script_path.clone();

    let resources_root = app
        .path()
        .resource_dir()
        .ok()
        .map(|dir| win_driver::path_util::strip_verbatim(dir.join("resources")));
    let driver_dir = resources_root
        .as_ref()
        .map(|dir| dir.join("ddhid-driver"))
        .unwrap_or_default();
    let out_file = default_report_path(&app)?;
    let evidence_file = diagnostics_dir.join(format!("ddhid-evidence-{}.txt", timestamp_slug()));
    let evidence_file_for_task = evidence_file.clone();

    let app_report = crate::commands::repair::diagnose_environment(app.clone()).await?;

    let output = tauri::async_runtime::spawn_blocking(move || {
        Command::new("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script_path_for_task)
            .arg("-DriverDir")
            .arg(&driver_dir)
            .arg("-OutFile")
            .arg(&evidence_file_for_task)
            .arg("-EvidenceOnly")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .output()
    })
    .await
    .map_err(|e| format!("诊断任务异常: {e}"))?
    .map_err(|e| format!("启动 PowerShell 诊断失败: {e}"))?;

    let mut report = String::new();
    append_header(&mut report, &out_file, resources_root.as_deref());
    append_app_diagnosis(&mut report, &app_report);
    push_section(&mut report, "Live app display environment (JSON)");
    match crate::commands::display_environment::get_display_environment(app.clone()).await {
        Ok(display) => match serde_json::to_string_pretty(&display) {
            Ok(json) => {
                let _ = writeln!(report, "{json}");
            }
            Err(error) => {
                let _ = writeln!(report, "serialization_error: {error}");
            }
        },
        Err(error) => {
            let _ = writeln!(report, "collection_error: {error}");
        }
    }
    if let Some(resources_root) = resources_root.as_deref() {
        append_resource_files(&mut report, resources_root);
    } else {
        push_section(&mut report, "Packaged resource files");
        let _ = writeln!(report, "resource_dir: <unavailable>");
    }
    append_script_evidence(&mut report, &evidence_file, &output);
    let _ = std::fs::remove_file(&evidence_file);
    let _ = std::fs::remove_file(&script_path);

    write_text_with_bom(&out_file, &report)?;
    Ok(out_file.display().to_string())
}

#[cfg(windows)]
fn append_header(
    out: &mut String,
    out_file: &std::path::Path,
    resources_root: Option<&std::path::Path>,
) {
    let _ = writeln!(out, "FlairBloom environment diagnostic report");
    let _ = writeln!(out, "generated_at_slug: {}", timestamp_slug());
    let _ = writeln!(out, "app_name: {}", crate::APP_NAME);
    let _ = writeln!(out, "app_version: {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "output_file: {}", out_file.display());
    let _ = writeln!(
        out,
        "resource_dir: {}",
        resources_root
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<unavailable>".to_string())
    );
}

#[cfg(windows)]
fn append_app_diagnosis(out: &mut String, report: &RepairReport) {
    push_section(out, "App diagnosis summary");
    let _ = writeln!(out, "scan_time: {}", report.timestamp);
    if report.items.is_empty() {
        let _ = writeln!(out, "<no diagnosis items>");
        return;
    }
    for item in &report.items {
        append_diagnosis_item(out, item);
    }
}

#[cfg(windows)]
fn append_diagnosis_item(out: &mut String, item: &DiagnosticItem) {
    let _ = writeln!(out);
    let _ = writeln!(out, "[{}] {}", item.id, item.label);
    let _ = writeln!(out, "category: {}", item.category);
    let _ = writeln!(out, "severity: {}", severity_label(item.severity));
    let _ = writeln!(out, "status: {}", status_label(item.status));
    let _ = writeln!(out, "detail: {}", item.detail);
    let _ = writeln!(
        out,
        "recommended_action: {}",
        item.recommended_action.as_deref().unwrap_or("<none>")
    );
}

#[cfg(windows)]
fn append_resource_files(out: &mut String, resources_root: &std::path::Path) {
    use crate::commands::resource_integrity::{sha256_file_hex, EXPECTED_RESOURCES};

    push_section(out, "Packaged resource files");
    let _ = writeln!(out, "resource_dir: {}", resources_root.display());
    for expected in EXPECTED_RESOURCES {
        let path = resources_root.join(expected.rel);
        let _ = writeln!(out);
        let _ = writeln!(out, "-- {}", path.display());
        let _ = writeln!(out, "label: {}", expected.label);
        let _ = writeln!(out, "expected_size: {}", expected.size);
        let _ = writeln!(out, "expected_sha256: {}", expected.sha256);
        match std::fs::metadata(&path) {
            Ok(meta) => {
                let _ = writeln!(out, "exists: true");
                let _ = writeln!(out, "actual_size: {}", meta.len());
                let _ = writeln!(out, "size_match: {}", meta.len() == expected.size);
                match sha256_file_hex(&path) {
                    Ok(hash) => {
                        let _ = writeln!(out, "actual_sha256: {hash}");
                        let _ = writeln!(out, "sha256_match: {}", hash == expected.sha256);
                    }
                    Err(e) => {
                        let _ = writeln!(out, "hash_error: {e}");
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = writeln!(out, "exists: false");
            }
            Err(e) => {
                let _ = writeln!(out, "metadata_error: {e}");
            }
        }
    }
}

#[cfg(windows)]
fn append_script_evidence(
    out: &mut String,
    evidence_file: &std::path::Path,
    output: &std::process::Output,
) {
    push_section(out, "Windows evidence");
    let _ = writeln!(out, "collector_exit_code: {:?}", output.status.code());
    if !output.status.success() {
        let stdout = decode_output(&output.stdout);
        let stderr = decode_output(&output.stderr);
        let _ = writeln!(out, "collector_error:");
        let _ = writeln!(
            out,
            "{}",
            trim_error_detail(format!("stdout:\n{stdout}\n\nstderr:\n{stderr}"))
        );
    }

    match std::fs::read(evidence_file) {
        Ok(bytes) => {
            let text = decode_output(&bytes);
            if text.is_empty() {
                let _ = writeln!(out, "<empty evidence file>");
            } else {
                let _ = writeln!(out, "{text}");
            }
        }
        Err(e) => {
            let _ = writeln!(out, "evidence_file_read_error: {e}");
        }
    }
}

#[cfg(windows)]
fn push_section(out: &mut String, name: &str) {
    let _ = writeln!(out);
    let _ = writeln!(out, "==== {name} ====");
}

#[cfg(windows)]
fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}

#[cfg(windows)]
fn status_label(status: ItemStatus) -> &'static str {
    match status {
        ItemStatus::Ok => "ok",
        ItemStatus::Orphan => "orphan",
        ItemStatus::Missing => "missing",
        ItemStatus::Corrupted => "corrupted",
        ItemStatus::Unknown => "unknown",
    }
}

#[cfg(windows)]
fn write_script_with_bom(path: &std::path::Path) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(3 + DIAGNOSE_SCRIPT.len());
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(DIAGNOSE_SCRIPT.trim_start_matches('\u{feff}').as_bytes());
    std::fs::write(path, bytes).map_err(|e| format!("写入诊断脚本失败: {e}"))
}

#[cfg(windows)]
fn write_text_with_bom(path: &std::path::Path, text: &str) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(3 + text.len());
    bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    bytes.extend_from_slice(text.as_bytes());
    std::fs::write(path, bytes).map_err(|e| format!("写入诊断报告失败: {e}"))
}

#[cfg(windows)]
fn default_report_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let desktop = app
        .path()
        .desktop_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    let desktop = win_driver::path_util::strip_verbatim(desktop);
    std::fs::create_dir_all(&desktop).map_err(|e| format!("创建诊断输出目录失败: {e}"))?;
    Ok(desktop.join(format!("flair-environment-{}.txt", timestamp_slug())))
}

#[cfg(windows)]
fn decode_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches('\u{feff}')
        .trim()
        .to_string()
}

#[cfg(windows)]
fn trim_error_detail(mut text: String) -> String {
    const MAX_LEN: usize = 3000;
    if text.len() > MAX_LEN {
        text.truncate(MAX_LEN);
        text.push_str("\n...");
    }
    text
}

#[cfg(windows)]
fn timestamp_slug() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let days_since_epoch = secs / 86_400;
    let time_of_day = secs % 86_400;
    let h = time_of_day / 3600;
    let m = (time_of_day / 60) % 60;
    let s = time_of_day % 60;
    let (y, mo, d) = ymd_from_days(days_since_epoch as i64);
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

#[cfg(windows)]
fn ymd_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
