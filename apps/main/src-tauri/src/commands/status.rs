//! 会话级稳定状态汇总。
//!
//! UI 启动时拉一次 [`get_app_status`]，之后只通过 `app-status-changed` 事件刷新。
//! 高频/事件驱动的状态（规则、激活规则 ID、global_enabled）仍走独立命令，
//! 避免逼前端轮询整个状态。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const STATUS_CHANGED_EVENT: &str = "app-status-changed";

/// 驱动安装的三态。
///
/// `Installed` / `NotInstalled` 都很直观。`PendingReboot` 是关键的中间态：
/// 卸载（或安装）已发起但未完成 PnP 物理清理 / 注册——此时既不能再装一次
/// （会撞 install error），也不能直接当作"已安装"使用。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum DriverStatus {
    Installed,
    PendingReboot,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppStatus {
    pub elevated: bool,
    pub interception_installed: DriverStatus,
    /// 游戏模式当前不可启用的具体原因；不影响通用模式。
    pub interception_issue: Option<String>,
    pub dd_hid_installed: DriverStatus,
    /// 外设驱动连着没有（插拔监视握过手才算）。外设模式的「驱动已装」。
    pub hid_stick_present: bool,
    pub input_mode: String,
    pub configured_input_mode: String,
    pub scheduler_hp_degraded: bool,
    pub platform: &'static str,
    pub os_family: &'static str,
    pub os_version: String,
    pub webview_version: String,
    pub arch: String,
    pub locale: String,
    pub install_path: String,
    pub log_dir: String,
    pub app_data_dir: String,
    pub autostart_enabled: bool,
    pub run_as_admin: bool,
    pub resources_ok: bool,
    pub missing_resources: Vec<String>,
}

impl AppStatus {
    pub fn collect(app: &AppHandle) -> Self {
        let (resources_ok, missing_resources) = collect_resource_health(app);
        let interception = collect_interception_health();
        let input_mode = collect_input_mode();
        let configured_input_mode = collect_configured_input_mode(app);
        if interception.issue.is_some()
            && (input_mode == "interception" || configured_input_mode == "interception")
        {
            tracing::warn!(
                current_mode = %input_mode,
                configured_mode = %configured_input_mode,
                detail = %interception.detail,
                "游戏模式就绪检测未通过"
            );
        }
        Self {
            elevated: collect_elevated(),
            interception_installed: interception.status,
            interception_issue: interception.issue,
            dd_hid_installed: collect_dd_hid_installed(),
            hid_stick_present: collect_hid_stick_present(),
            input_mode,
            configured_input_mode,
            scheduler_hp_degraded: app
                .state::<crate::commands::engine::EngineState>()
                .0
                .scheduler_hp_degraded(),
            platform: std::env::consts::OS,
            os_family: std::env::consts::FAMILY,
            os_version: win_sysinfo::os_version(),
            webview_version: win_sysinfo::webview2_version(),
            arch: win_sysinfo::host_arch(),
            locale: win_sysinfo::user_locale(),
            install_path: win_sysinfo::install_path(),
            log_dir: crate::log_dir().to_string_lossy().into_owned(),
            app_data_dir: app
                .path()
                .app_data_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            autostart_enabled: collect_autostart_enabled(app),
            run_as_admin: collect_run_as_admin(),
            resources_ok,
            missing_resources,
        }
    }
}

#[tauri::command]
pub fn get_app_status(app: AppHandle) -> AppStatus {
    AppStatus::collect(&app)
}

/// 由命令侧在状态变更后调用：重新采集并向所有窗口广播 `app-status-changed`。
// 非 Windows 编译路径下没有任何调用方（驱动相关命令全部在 cfg(windows) 内），
// 但函数体本身跨平台，留它在这里方便后续接入非 Windows 的状态变化点。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn emit_status_changed(app: &AppHandle) {
    let status = AppStatus::collect(app);
    if let Err(e) = app.emit(STATUS_CHANGED_EVENT, &status) {
        tracing::warn!("emit {} 失败: {}", STATUS_CHANGED_EVENT, e);
    }
}

#[cfg(windows)]
fn collect_elevated() -> bool {
    win_driver::elevation::is_process_elevated()
}

#[cfg(not(windows))]
fn collect_elevated() -> bool {
    false
}

pub(crate) struct InterceptionHealth {
    pub status: DriverStatus,
    pub issue: Option<String>,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub warning: Option<String>,
    #[cfg_attr(not(windows), allow(dead_code))]
    pub detail: String,
}

pub(crate) fn collect_interception_health() -> InterceptionHealth {
    #[cfg(windows)]
    {
        use win_sysinfo::registry;
        let services = registry::is_interception_service("keyboard", "keyboard.sys")
            || registry::is_interception_service("mouse", "mouse.sys");
        let slots = win_input::interception::device_hardware_ids();
        let classes = registry::input_class_indices("KeyboardClass").and_then(|keyboard| {
            registry::input_class_indices("PointerClass").map(|mouse| (keyboard, mouse))
        });
        assess_interception_health(services, slots.as_deref(), classes)
    }
    #[cfg(not(windows))]
    {
        InterceptionHealth {
            status: DriverStatus::NotInstalled,
            issue: None,
            warning: None,
            detail: String::new(),
        }
    }
}

#[cfg(any(windows, test))]
fn assess_interception_health(
    services: bool,
    slots: Option<&[(i32, String)]>,
    classes: Result<(Vec<u32>, Vec<u32>), String>,
) -> InterceptionHealth {
    let readiness = win_input::interception_health::assess_readiness(services, slots, classes);
    InterceptionHealth {
        status: if readiness.runtime_available {
            DriverStatus::Installed
        } else if services {
            DriverStatus::PendingReboot
        } else {
            DriverStatus::NotInstalled
        },
        issue: readiness.issue,
        warning: readiness.warning,
        detail: readiness.detail,
    }
}

#[cfg(windows)]
fn collect_dd_hid_installed() -> DriverStatus {
    let sys = win_driver::dd_hid::dd_hid_sys_installed();
    let service = win_sysinfo::registry::service_key_present("ddhid63340");
    match (sys, service) {
        (true, true) => DriverStatus::Installed,
        (false, false) => DriverStatus::NotInstalled,
        // sys 在但服务键缺 / 服务键在但 sys 缺：都是半卸载残留，重启由 PnP 完成清理
        _ => DriverStatus::PendingReboot,
    }
}

#[cfg(not(windows))]
fn collect_dd_hid_installed() -> DriverStatus {
    DriverStatus::NotInstalled
}

#[cfg(windows)]
fn collect_hid_stick_present() -> bool {
    crate::bootstrap::input::hid_stick_connected()
}

#[cfg(not(windows))]
fn collect_hid_stick_present() -> bool {
    false
}

#[cfg(windows)]
fn collect_input_mode() -> String {
    win_input::current_mode().as_str().to_string()
}

#[cfg(not(windows))]
fn collect_input_mode() -> String {
    "sendinput".to_string()
}

#[cfg(windows)]
pub(crate) fn collect_configured_input_mode(app: &AppHandle) -> String {
    use tauri_plugin_store::StoreExt;

    app.store(crate::STORE_PATH)
        .ok()
        .and_then(|store| {
            store
                .get("input_mode")
                .and_then(|v| v.as_str().and_then(win_input::InputMode::from_str))
        })
        .map(|mode| mode.as_str().to_string())
        .unwrap_or_else(|| "sendinput".to_string())
}

#[cfg(not(windows))]
fn collect_configured_input_mode(_app: &AppHandle) -> String {
    "sendinput".to_string()
}

fn collect_autostart_enabled(app: &AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// 当前 exe 是否被标记为以管理员身份启动。取不到自身路径时按未标记处理。
fn collect_run_as_admin() -> bool {
    crate::commands::app::current_exe_path()
        .is_ok_and(|exe| win_sysinfo::run_as_admin::is_enabled(&exe))
}

/// 资源完整性自检：检查驱动安装器是否齐全且未被换行转换 / 杀软改写。
///
/// Windows 安装包会把这些 exe 落到 `<resource_dir>/resources/`，杀软误删或解压不全
/// 时会让"安装游戏模式驱动 / DDHID"按下去就报错。把缺失项列出来给反馈链路用，
/// 比让用户自己翻日志强得多。其它平台没有这些资源，恒返回 OK。
fn collect_resource_health(app: &AppHandle) -> (bool, Vec<String>) {
    #[cfg(windows)]
    {
        let resources = match app.path().resource_dir() {
            Ok(d) => d.join("resources"),
            Err(_) => return (false, vec!["<resource_dir 不可达>".to_string()]),
        };
        let health = crate::commands::resource_integrity::check_resources(&resources);
        let issues = health
            .issues
            .iter()
            .map(crate::commands::resource_integrity::issue_label)
            .collect::<Vec<_>>();
        (health.ok(), issues)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        (true, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn online_slots(keyboards: i32, mice: i32) -> Vec<(i32, String)> {
        (1..=keyboards)
            .chain(11..11 + mice)
            .map(|n| (n, "device".into()))
            .collect()
    }

    #[test]
    fn registered_driver_without_runtime_requires_restart() {
        let health = assess_interception_health(true, None, Ok((vec![0], vec![0])));
        assert_eq!(health.status, DriverStatus::PendingReboot);
        assert!(health.issue.unwrap().contains("首次安装"));
    }

    #[test]
    fn open_context_without_hardware_ids_is_not_ready() {
        let health = assess_interception_health(true, Some(&[]), Ok((vec![0], vec![0])));
        assert!(health.issue.unwrap().contains("未读取到可用键盘"));
    }

    #[test]
    fn full_keyboard_or_mouse_capacity_is_a_warning() {
        for (keyboard, mouse) in [(10, 1), (1, 10), (10, 10)] {
            let slots = online_slots(keyboard, mouse);
            let health = assess_interception_health(true, Some(&slots), Ok((vec![0], vec![0])));
            assert!(health.issue.is_none());
            assert!(health.warning.is_some());
        }
    }

    #[test]
    fn high_class_index_is_diagnostic_only() {
        let slots = online_slots(1, 1);
        for classes in [(vec![12], vec![0]), (vec![0], vec![10])] {
            let health = assess_interception_health(true, Some(&slots), Ok(classes));
            assert!(health.issue.is_none());
            assert!(health.warning.is_some());
        }
        assert!(
            assess_interception_health(true, Some(&slots), Ok((vec![9], vec![9])))
                .issue
                .is_none()
        );
    }

    #[test]
    fn failed_class_enumeration_does_not_override_runtime_devices() {
        let slots = online_slots(1, 1);
        assert!(
            assess_interception_health(true, Some(&slots), Err("拒绝访问".into()))
                .issue
                .is_none()
        );
    }

    #[test]
    fn event_name_is_stable() {
        // 前端 listen 的事件名硬编码在 PanelApp.tsx，名字一旦改动两端必须同步。
        // 这里钉死字符串，避免后续 typo 把广播打到一个没人监听的频道上。
        assert_eq!(STATUS_CHANGED_EVENT, "app-status-changed");
    }

    fn sample_status() -> AppStatus {
        AppStatus {
            elevated: true,
            interception_installed: DriverStatus::Installed,
            interception_issue: None,
            dd_hid_installed: DriverStatus::PendingReboot,
            hid_stick_present: false,
            input_mode: "dd_hid".to_string(),
            configured_input_mode: "dd_hid".to_string(),
            scheduler_hp_degraded: false,
            platform: "windows",
            os_family: "windows",
            os_version: "Windows 11 23H2 (Build 22631.4317)".to_string(),
            webview_version: "120.0.0.0".to_string(),
            arch: "x64".to_string(),
            locale: "zh-CN".to_string(),
            install_path: r"C:\Program Files\FlairBloom".to_string(),
            log_dir: r"C:\Users\me\AppData\Local\fun.xwink.flairbloom\logs".to_string(),
            app_data_dir: r"C:\Users\me\AppData\Roaming\fun.xwink.flairbloom".to_string(),
            autostart_enabled: false,
            run_as_admin: false,
            resources_ok: true,
            missing_resources: Vec::new(),
        }
    }

    #[test]
    fn serialize_uses_snake_case_for_frontend_consumption() {
        // 前端 AppStatus 字段是 snake_case，这里钉住序列化的实际 key
        let json = serde_json::to_value(sample_status()).unwrap();
        let obj = json.as_object().unwrap();
        for key in [
            "elevated",
            "interception_installed",
            "interception_issue",
            "dd_hid_installed",
            "input_mode",
            "configured_input_mode",
            "scheduler_hp_degraded",
            "platform",
            "os_family",
            "os_version",
            "webview_version",
            "arch",
            "locale",
            "install_path",
            "log_dir",
            "app_data_dir",
            "autostart_enabled",
            "run_as_admin",
            "resources_ok",
            "missing_resources",
        ] {
            assert!(obj.contains_key(key), "缺少键 {key}");
        }
        assert_eq!(obj["input_mode"], "dd_hid");
        assert_eq!(obj["configured_input_mode"], "dd_hid");
        assert_eq!(obj["arch"], "x64");
        assert_eq!(obj["locale"], "zh-CN");
        // 三态序列化为 snake_case 字符串，前端按枚举值匹配
        assert_eq!(obj["interception_installed"], "installed");
        assert_eq!(obj["dd_hid_installed"], "pending_reboot");
        assert!(obj["missing_resources"].is_array());
    }
}
