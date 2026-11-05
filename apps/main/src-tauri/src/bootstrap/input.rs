//! 输入后端初始化：启动时先落到 SendInput，避免非管理员进程直接加载驱动后端。

#[cfg(windows)]
use tauri::Manager;
#[cfg(windows)]
use tauri_plugin_store::StoreExt;

pub fn init_input_backend(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        use win_input::{init_backend, set_resources_dir, InputMode};

        if let Ok(dir) = app.path().resource_dir() {
            set_resources_dir(dir.join("resources"));
        }

        let cli_mode = parse_switch_mode_arg()
            .as_deref()
            .and_then(InputMode::from_str);
        init_backend(InputMode::SendInput);

        if let Some(mode) = cli_mode {
            if let Ok(store) = app.store(crate::STORE_PATH) {
                store.set("input_mode", serde_json::json!(mode.as_str()));
                let _ = store.save();
            }
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// 外设驱动连着没有（握过手、或外设模式正连着才算）。只在翻转时发
/// `hid-stick-connected` / `hid-stick-disconnected`，谁听、怎么处理是前端的事；启动时插着的
/// 那一下同样是一次「连上」，面板可能还没开始听，所以面板启动落定后自己再查一次状态。
#[cfg(windows)]
static HID_STICK_CONNECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
static VERIFIED_STICK_PORTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// 用户主动诊断时握手成功的端口加入本会话监视，供非乐鑫设备使用。
#[cfg(windows)]
pub fn remember_verified_sticks(scans: &[win_input::hidstick::PortScan]) {
    let mut verified = VERIFIED_STICK_PORTS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for scan in scans.iter().filter(|scan| scan.is_stick()) {
        if !verified.contains(&scan.name) {
            verified.push(scan.name.clone());
        }
    }
}

#[cfg(windows)]
fn set_hid_stick_connected(app: &tauri::AppHandle, connected: bool) {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    if HID_STICK_CONNECTED.swap(connected, Ordering::SeqCst) == connected {
        return;
    }
    tracing::info!("外设驱动{}", if connected { "已连接" } else { "已断开" });
    let event = if connected {
        "hid-stick-connected"
    } else {
        "hid-stick-disconnected"
    };
    let _ = app.emit(event, ());
}

#[cfg(windows)]
pub fn hid_stick_connected() -> bool {
    HID_STICK_CONNECTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 手动切到外设模式并连上了：连接态以后端为准，插拔监视没认到（比如它握手时口被占着）也算连着，
/// 否则界面显示「未插入」，下次断开也报不出来。
#[cfg(windows)]
pub fn on_hid_stick_backend_up(app: &tauri::AppHandle) {
    set_hid_stick_connected(app, true);
}

/// 外设模式下心跳或下发失败（链路断了，外设驱动未必拔了）。
#[cfg(windows)]
pub fn on_hid_stick_link_lost(app: &tauri::AppHandle) {
    set_hid_stick_connected(app, false);
}

/// 盯外设驱动插拔：后台轮询乐鑫的 USB 串口，新出现的口握一次手，规则见
/// [`win_input::hidstick::StickTracker`]。默认只碰乐鑫，以及本会话主动诊断中握手确认的端口；后台执行，
/// 握手不能拖慢启动，也不能去拉别家设备的 DTR（见 `pick_auto_candidates`）。
/// 轮询而不是 `WM_DEVICECHANGE`：Tauri 不把窗口过程交出来，为它单开一个隐藏窗口不值。
///
/// 外设模式正用着的口不再握手，直接算连着。链路断了（心跳 / 下发失败）而口还在时忘掉它，下一轮
/// 重新握手：是一时的写失败就会重新报「连上」，固件卡死则握不上，要等拔掉再插回。
pub fn start_hid_stick_watcher(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        use win_input::hidstick::{auto_candidate_ports, candidate_ports, probe, StickTracker};
        const POLL_EVERY: std::time::Duration = std::time::Duration::from_millis(1500);

        let app = app.clone();
        let mut tracker = StickTracker::default();
        let spawned = std::thread::Builder::new()
            .name("hidstick-watcher".into())
            .spawn(move || loop {
                if tracker.connected() && !hid_stick_connected() {
                    tracker.forget();
                }
                let in_use = win_input::hidstick_in_use().map(|(port, _)| port);
                let mut candidates = auto_candidate_ports();
                {
                    let present = candidate_ports();
                    let mut verified = VERIFIED_STICK_PORTS
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    verified.retain(|port| present.contains(port));
                    for port in verified.iter() {
                        if !candidates.contains(port) {
                            candidates.push(port.clone());
                        }
                    }
                }
                let flip = tracker.poll(&candidates, |port| {
                    in_use.as_deref() == Some(port) || probe(port).is_some()
                });
                if let Some(connected) = flip {
                    if !connected {
                        // 先让后端回退通用模式，再报断开，界面取到的才是回退后的状态
                        win_input::hidstick_link_down("外设驱动的串口不见了");
                    }
                    set_hid_stick_connected(&app, connected);
                }
                std::thread::sleep(POLL_EVERY);
            });
        if let Err(e) = spawned {
            tracing::warn!("启动外设驱动插拔监视失败: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = app;
}

#[cfg(windows)]
pub fn parse_switch_mode_arg() -> Option<String> {
    for arg in std::env::args() {
        if let Some(v) = arg.strip_prefix("--switch-mode=") {
            return Some(v.to_string());
        }
    }
    None
}

/// 提权重启场景：新提权实例由旧实例用 `--await-pid=<旧PID>` 拉起。必须在单实例插件
/// 初始化之前先等旧进程退出（释放单实例锁），否则新实例会被判定为重复实例而自杀，
/// 表现为「以管理员重启只退出不重启」。带 5 秒超时兜底；旧进程已退出（OpenProcess
/// 失败）则立即返回。
pub fn wait_for_predecessor_exit() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };

        let Some(pid) = std::env::args().find_map(|a| {
            a.strip_prefix("--await-pid=")
                .and_then(|v| v.parse::<u32>().ok())
        }) else {
            return;
        };

        // SAFETY: 参数合法；句柄打开失败返回 null
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return; // 旧进程已退出或无法打开，直接继续启动
        }
        // SAFETY: handle 由上面 OpenProcess 成功返回，等待后立即关闭
        unsafe {
            WaitForSingleObject(handle, 5000);
            CloseHandle(handle);
        }
    }
}
