//! 按需采集实时显示状态；不调整窗口、系统设置或 WebView。

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

#[derive(Serialize)]
pub struct DisplayEnvironment {
    text_scale_percent: Option<u32>,
    fast_startup_configured: Option<bool>,
    runtime_version: Option<String>,
    monitors: Vec<MonitorInfo>,
    windows: Vec<WindowInfo>,
    saved_appearance: serde_json::Value,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct MonitorInfo {
    name: Option<String>,
    primary: bool,
    size_physical: tauri::PhysicalSize<u32>,
    position_physical: tauri::PhysicalPosition<i32>,
    scale_factor: f64,
}

#[derive(Serialize)]
struct WindowInfo {
    label: String,
    visible: Option<bool>,
    minimized: Option<bool>,
    monitor: Option<String>,
    scale_factor: Option<f64>,
    client_physical: Option<tauri::PhysicalSize<u32>>,
    position_physical: Option<tauri::PhysicalPosition<i32>>,
    webview: Option<WebviewInfo>,
}

#[derive(Serialize)]
struct WebviewInfo {
    zoom_factor: f64,
    rasterization_scale: f64,
    detects_system_scale: bool,
    bounds_mode: i32,
    bounds_physical: tauri::PhysicalSize<i32>,
}

#[tauri::command]
pub async fn get_display_environment(app: AppHandle) -> Result<DisplayEnvironment, String> {
    // WebView 控制器只能在主线程读；收集线程等待回调，避免阻塞 UI。
    tauri::async_runtime::spawn_blocking(move || collect(&app))
        .await
        .map_err(|error| format!("采集显示环境失败：{error}"))
}

fn record<T, E: std::fmt::Display>(
    result: Result<T, E>,
    context: &str,
    errors: &mut Vec<String>,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("{context}: {error}"));
            None
        }
    }
}

fn collect(app: &AppHandle) -> DisplayEnvironment {
    let mut errors = Vec::new();
    let primary = record(app.primary_monitor(), "primary_monitor", &mut errors).flatten();
    let monitors = record(app.available_monitors(), "monitors", &mut errors)
        .unwrap_or_default()
        .into_iter()
        .map(|monitor| MonitorInfo {
            name: monitor.name().cloned(),
            primary: primary
                .as_ref()
                .is_some_and(|p| p.position() == monitor.position()),
            size_physical: *monitor.size(),
            position_physical: *monitor.position(),
            scale_factor: monitor.scale_factor(),
        })
        .collect();
    let mut windows = Vec::new();
    for label in [crate::PANEL_LABEL, crate::FLOAT_LABEL] {
        let Some(window) = app.get_webview_window(label) else {
            errors.push(format!("{label}: window unavailable"));
            continue;
        };
        let context = |field| format!("{label}.{field}");
        windows.push(WindowInfo {
            label: label.to_owned(),
            visible: record(window.is_visible(), &context("visible"), &mut errors),
            minimized: record(window.is_minimized(), &context("minimized"), &mut errors),
            monitor: record(window.current_monitor(), &context("monitor"), &mut errors)
                .flatten()
                .and_then(|m| m.name().cloned()),
            scale_factor: record(window.scale_factor(), &context("scale_factor"), &mut errors),
            client_physical: record(window.inner_size(), &context("client"), &mut errors),
            position_physical: record(window.outer_position(), &context("position"), &mut errors),
            webview: record(read_webview(&window), &context("webview"), &mut errors).flatten(),
        });
    }
    let saved_appearance = record(app.store(crate::STORE_PATH), "settings", &mut errors)
        .map(|store| {
            serde_json::json!({
                "layout": store.get("layout"),
                "panelScale": store.get("panelScale"),
                "panelMaximized": store.get("panelMaximized"),
            })
        })
        .unwrap_or(serde_json::Value::Null);
    DisplayEnvironment {
        text_scale_percent: record(read_text_scale(), "text_scale", &mut errors).flatten(),
        fast_startup_configured: read_fast_startup(),
        runtime_version: record(tauri::webview_version(), "runtime_version", &mut errors),
        monitors,
        windows,
        saved_appearance,
        errors,
    }
}

#[cfg(windows)]
fn read_fast_startup() -> Option<bool> {
    win_sysinfo::registry::read_hklm_dword(
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Power",
        "HiberbootEnabled",
    )
    .map(|value| value != 0)
}

#[cfg(not(windows))]
fn read_fast_startup() -> Option<bool> {
    None
}

#[cfg(windows)]
fn read_text_scale() -> Result<Option<u32>, String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let path = win_sysinfo::registry::wide("Software\\Microsoft\\Accessibility");
    let name = win_sysinfo::registry::wide("TextScaleFactor");
    let mut value = 0u32;
    let mut size = std::mem::size_of_val(&value) as u32;
    // SAFETY: 路径和键名 NUL 结尾，value / size 出参大小正确；只读取当前用户。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(value)),
        ERROR_FILE_NOT_FOUND => Ok(Some(100)),
        _ => Err(format!("RegGetValueW returned {status}")),
    }
}

#[cfg(not(windows))]
fn read_text_scale() -> Result<Option<u32>, String> {
    Ok(None)
}

#[cfg(windows)]
fn read_webview(window: &tauri::WebviewWindow) -> Result<Option<WebviewInfo>, String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Controller3, COREWEBVIEW2_BOUNDS_MODE,
    };
    use windows::core::{Interface, BOOL};
    use windows::Win32::Foundation::RECT;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    window
        .with_webview(move |webview| {
            let result = (|| -> windows::core::Result<WebviewInfo> {
                let controller: ICoreWebView2Controller3 = webview.controller().cast()?;
                let mut zoom_factor = 0.0;
                let mut rasterization_scale = 0.0;
                let mut detects_system_scale = BOOL::default();
                let mut bounds_mode = COREWEBVIEW2_BOUNDS_MODE::default();
                let mut bounds = RECT::default();
                // SAFETY: 回调在控制器所属主线程执行，所有出参在调用期间有效。
                unsafe {
                    controller.ZoomFactor(&mut zoom_factor)?;
                    controller.RasterizationScale(&mut rasterization_scale)?;
                    controller.ShouldDetectMonitorScaleChanges(&mut detects_system_scale)?;
                    controller.BoundsMode(&mut bounds_mode)?;
                    controller.Bounds(&mut bounds)?;
                }
                Ok(WebviewInfo {
                    zoom_factor,
                    rasterization_scale,
                    detects_system_scale: detects_system_scale.as_bool(),
                    bounds_mode: bounds_mode.0,
                    bounds_physical: tauri::PhysicalSize::new(
                        bounds.right - bounds.left,
                        bounds.bottom - bounds.top,
                    ),
                })
            })();
            let _ = sender.send(result.map_err(|error| error.to_string()));
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|error| error.to_string())?
        .map(Some)
}

#[cfg(not(windows))]
fn read_webview(_window: &tauri::WebviewWindow) -> Result<Option<WebviewInfo>, String> {
    Ok(None)
}
