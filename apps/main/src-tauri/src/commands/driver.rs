//! 驱动管理：Interception / DD-HID 安装卸载 + 提权重启。
//!
//! 业务逻辑委托给 `win-driver` crate，本层仅负责 Tauri 桥接与状态广播。

use serde::Serialize;
#[allow(unused_imports)]
use tauri::{AppHandle, Manager};

/// 卸载结果。`pending_reboot=true` 表示驱动文件已标记为重启删除、卸载在逻辑上
/// 已完成，但物理文件要等下次开机才消失。
#[derive(Debug, Clone, Serialize)]
pub struct UninstallOutcome {
    pub message: String,
    pub pending_reboot: bool,
}

#[cfg(windows)]
fn resource_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let raw = app
        .path()
        .resource_dir()
        .map_err(|e| format!("无法获取资源目录: {e}"))?
        .join("resources");
    Ok(win_driver::path_util::strip_verbatim(raw))
}

#[tauri::command]
pub fn is_driver_installed() -> bool {
    #[cfg(windows)]
    {
        win_input::interception::is_driver_installed()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[tauri::command]
pub async fn install_driver(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        let health = crate::commands::status::collect_interception_health();
        if health.status != crate::commands::status::DriverStatus::NotInstalled {
            return Err(health
                .issue
                .unwrap_or_else(|| "游戏模式驱动已安装，无需重复安装。".into()));
        }
        let res_dir = resource_dir(&app)?;
        let health = crate::commands::resource_integrity::check_resources(&res_dir);
        if !health.issues.is_empty() {
            let details = health
                .issues
                .iter()
                .map(crate::commands::resource_integrity::issue_label)
                .collect::<Vec<_>>()
                .join("；");
            return Err(format!(
                "驱动资源文件校验失败，拒绝安装以防提权执行被篡改文件：{details}"
            ));
        }
        let result = win_driver::interception::install(&res_dir).await;
        if let Err(ref e) = result {
            tracing::error!("Interception 驱动安装失败：{e}");
        }
        if result.is_ok() {
            crate::commands::engine::switch_input_backend(
                &app,
                &app.state::<crate::commands::engine::EngineState>().0,
                win_input::InputMode::SendInput,
            );
        }
        crate::commands::status::emit_status_changed(&app);
        result
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅 Windows 平台支持安装驱动".to_string())
    }
}

#[tauri::command]
pub async fn uninstall_driver(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        // 切回 SendInput 前先停连发、经旧后端阻塞释放已按下的目标键，且切换窗口内不启动新规则，
        // 避免目标键 down/up 跨新旧后端错配卡住（统一走 switch_input_backend）。
        crate::commands::engine::leave_driver_backend(
            &app,
            &app.state::<crate::commands::engine::EngineState>().0,
        );
        let res_dir = resource_dir(&app)?;
        let result = win_driver::interception::uninstall(&res_dir).await;
        if let Err(ref e) = result {
            tracing::error!("Interception 驱动卸载失败：{e}");
        }
        crate::commands::status::emit_status_changed(&app);
        result
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅 Windows 平台支持卸载驱动".to_string())
    }
}

/// DD-HID 已永久停用，此处只为「诊断修复」展示存量安装状态、引导用户卸载。
#[tauri::command]
pub fn is_dd_hid_driver_installed() -> bool {
    win_driver::dd_hid::dd_hid_sys_installed()
}

#[tauri::command]
pub async fn uninstall_dd_hid_driver(app: AppHandle) -> Result<UninstallOutcome, String> {
    #[cfg(windows)]
    {
        // 切回 SendInput 前先停连发、经旧后端阻塞释放已按下的目标键，且切换窗口内不启动新规则，
        // 避免目标键 down/up 跨新旧后端错配卡住（统一走 switch_input_backend）。
        crate::commands::engine::leave_driver_backend(
            &app,
            &app.state::<crate::commands::engine::EngineState>().0,
        );
        let res_dir = resource_dir(&app)?;
        let (pending_reboot, exe_result) = win_driver::dd_hid::uninstall(&res_dir).await?;
        crate::commands::status::emit_status_changed(&app);

        if pending_reboot {
            return Ok(UninstallOutcome {
                message: "驱动卸载已发起，剩余清理需重启电脑由 PnP 完成。\n\
                    请重启电脑后再尝试安装驱动。"
                    .to_string(),
                pending_reboot: true,
            });
        }
        let sys_still_present = win_driver::dd_hid::dd_hid_sys_installed();
        match win_driver::judge::judge_uninstall_result(sys_still_present, exe_result.clone()) {
            Ok(()) => Ok(UninstallOutcome {
                message: "DDHID 驱动已卸载，建议重启电脑后再尝试重新安装。".to_string(),
                pending_reboot: false,
            }),
            Err(e) => {
                let exe_state = match &exe_result {
                    Ok(()) => "ddc.exe 报告成功".to_string(),
                    Err(msg) => format!("ddc.exe 失败: {msg}"),
                };
                tracing::error!(
                    "DD-HID 驱动卸载失败：{e}（{exe_state}，sys 仍存在={sys_still_present}）"
                );
                Err(e)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅 Windows 平台支持卸载 DD-HID 驱动".to_string())
    }
}

#[tauri::command]
pub fn is_elevated() -> bool {
    win_driver::elevation::is_process_elevated()
}

/// 以管理员身份重启自身，携带 `--switch-mode=<id>` 参数，然后退出当前进程。
#[tauri::command]
pub async fn relaunch_as_admin(app: AppHandle, mode: String) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::ERROR_CANCELLED;
        use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};

        let _ =
            win_input::InputMode::from_str(&mode).ok_or_else(|| format!("未知输入模式: {mode}"))?;

        let exe = std::env::current_exe().map_err(|e| format!("无法定位当前可执行文件: {e}"))?;
        let path_wide: Vec<u16> = exe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();
        // 传旧进程 PID，新实例启动时先等它退出释放单实例锁（见 wait_for_predecessor_exit）。
        let pid = std::process::id();
        let params: Vec<u16> = format!("--elevated --switch-mode={mode} --await-pid={pid}\0")
            .encode_utf16()
            .collect();

        let result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            // SAFETY: SHELLEXECUTEINFOW 是 POD，全 0 初始化合法
            let mut sei: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            // SEE_MASK_NOASYNC：本进程在 ShellExecuteEx 后立即退出，必须等提权启动
            // 真正完成再返回，否则 shell 异步拉起会被中止，表现为「只退出不重启」。
            sei.fMask = SEE_MASK_NOASYNC;
            sei.lpVerb = verb.as_ptr();
            sei.lpFile = path_wide.as_ptr();
            sei.lpParameters = params.as_ptr();
            sei.nShow = 1;
            // SAFETY: 所有指针字段的 Vec 在闭包内存活，NUL 结尾宽串
            let ok = unsafe { ShellExecuteExW(&mut sei) };
            if ok == 0 {
                // SAFETY: GetLastError 无参
                let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                return if err == ERROR_CANCELLED {
                    Err("已取消管理员授权".to_string())
                } else {
                    Err(format!("启动管理员实例失败 (Win32 错误码 {err})"))
                };
            }
            Ok(())
        })
        .await
        .map_err(|e| format!("任务异常: {e}"))?;

        result.as_ref().map_err(|e| e.clone())?;

        // 提权实例已由 SEE_MASK_NOASYNC 保证拉起完成，这里短暂延迟仅为让本次 invoke
        // 响应回到前端后再退出。新实例会等本进程退出释放单实例锁后才继续启动。
        let app_clone = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            app_clone.exit(0);
        });
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, mode);
        Err("仅 Windows 平台支持提权重启".to_string())
    }
}
