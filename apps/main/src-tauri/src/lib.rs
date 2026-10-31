//! Tauri 桥接 + 业务装配入口。
//!
//! 各子系统的初始化逻辑集中在 `bootstrap/` 模块，本文件只做 Builder 装配与 invoke_handler 注册。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tracing::info;

mod bootstrap;
mod commands;
mod engine;
mod tray;

use bootstrap::{
    agreement::{check_agreement, AGREEMENT_VERSION},
    input::{init_input_backend, start_hid_stick_watcher, wait_for_predecessor_exit},
    logging,
    profile::load_or_init_profile,
    startup::apply_run_as_admin,
    update::{check_for_updates, UpdateLock},
};
use commands::{
    app::{
        agree_license, apply_pending_update, check_update, exit_app, minimize_to_float,
        needs_agreement, open_link, set_autostart, set_run_as_admin, should_auto_enable_on_start,
        show_main_panel,
    },
    ddhid_diagnostic::export_dd_hid_diagnostic_report,
    display_environment::get_display_environment,
    driver::{
        install_driver, is_dd_hid_driver_installed, is_driver_installed, is_elevated,
        relaunch_as_admin, uninstall_dd_hid_driver, uninstall_driver,
    },
    engine::{
        get_active_rules, get_global_enabled, get_hotkeys, get_input_mode, get_key_policy,
        get_rule_states, get_rules, relay_key_event, set_global_enabled, set_global_hotkeys,
        set_input_mode, set_rules, EngineState,
    },
    import_profile::{import_external_config, preview_import, scan_import_configs},
    jx3::{get_jx3_column, open_jx3_link, Jx3State},
    license::{activate_license, get_license_status, get_membership_agreement, LicenseState},
    log::{log_from_frontend, open_app_dir},
    profile::{
        delete_profile, export_profile, fork_active_profile, get_active_profile_path,
        import_qzh_profile, init_default_profile, list_profiles, load_profile, rename_profile,
        save_profile, set_profile_signature, take_profile_notice, ProfileNotice,
    },
    repair::{
        diagnose_environment, repair_clean_logs, repair_corrupted_profiles, repair_dd_hid_residue,
        repair_interception_residue,
    },
    sound::{delete_sound_file, import_sound_file, read_sound_file},
    status::get_app_status,
    window::set_panel_aspect,
};
use engine::{start_listener, BurstEngine};

pub const APP_NAME: &str = "FlairBloom";
pub const APP_NAME_CN: &str = "气质花按键助手";
pub const APP_IDENTIFIER: &str = "fun.xwink.flairbloom";
pub(crate) const STORE_PATH: &str = "settings.json";

pub(crate) const PANEL_LABEL: &str = "panel";
pub(crate) const FLOAT_LABEL: &str = "float";

pub(crate) fn show_panel<R: Runtime>(app: &AppHandle<R>) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.show();
        let _ = panel.unminimize();
        let _ = panel.set_focus();
    }
}

/// 进入面板模式：显示主面板、隐藏浮窗。所有"呼出主面板"入口都应走这里。
pub(crate) fn enter_panel_mode<R: Runtime>(app: &AppHandle<R>) {
    show_panel(app);
    if let Some(float) = app.get_webview_window(FLOAT_LABEL) {
        let _ = float.hide();
        // 通知浮窗停止轮询，避免隐藏后仍空转
        let _ = float.emit("float-active", false);
    }
}

/// 进入浮窗模式：隐藏主面板、显示常驻浮窗。所有"隐藏主面板"入口都应走这里。
pub(crate) fn enter_float_mode<R: Runtime>(app: &AppHandle<R>) {
    // 先确保浮窗能显示再隐藏面板；浮窗缺失时保留面板，避免应用整体不可见。
    if let Some(float) = app.get_webview_window(FLOAT_LABEL) {
        let _ = float.show();
        let _ = float.set_focus();
        // 通知浮窗开始轮询激活规则
        let _ = float.emit("float-active", true);
        if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
            let _ = panel.hide();
        }
    } else if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.show();
    }
}

/// 退出应用：先停引擎再退进程。所有"退出应用"入口都应走这里——引擎不停就退，
/// 钩子线程可能在进程收尾期间还在注入按键。
pub(crate) fn shutdown_and_exit<R: Runtime>(app: &AppHandle<R>) {
    app.state::<EngineState>().0.shutdown();
    app.exit(0);
}

pub fn log_dir() -> std::path::PathBuf {
    logging::log_dir()
}

pub fn run() {
    // 提权重启时，新实例先等旧实例退出释放单实例锁，再继续装配（含单实例插件）。
    wait_for_predecessor_exit();

    let dir = logging::log_dir();
    std::fs::create_dir_all(&dir).ok();
    logging::init(&dir);
    logging::cleanup_old_logs(&dir);

    let burst_engine = Arc::new(BurstEngine::new());
    let engine_for_listener = burst_engine.clone();
    let engine_for_tray = burst_engine.clone();
    let engine_for_exit = burst_engine.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            enter_panel_mode(app);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            #[cfg(windows)]
            if let tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
                if let Some(webview) = window.app_handle().get_webview_window(window.label()) {
                    bootstrap::webview_scale::apply(&webview, *scale_factor);
                }
            }
            match (window.label(), event) {
                // 系统级最小化（Win+D、任务栏按钮、Aero Shake）也收进浮窗，与标题栏的最小化按钮
                // 一致。否则面板缩进任务栏、浮窗又不占任务栏位，应用就只剩一个托盘图标，
                // 托盘一折叠用户就找不回来了。没有 Minimized 事件，最小化表现为 Resized。
                (PANEL_LABEL, tauri::WindowEvent::Resized(_)) => {
                    if !window.is_minimized().unwrap_or(false) {
                        return;
                    }
                    let window = window.clone();
                    tauri::async_runtime::spawn(async move {
                        // 先取消最小化：隐藏一个仍处于最小化状态的窗口，下次 show 会再缩回去；
                        // 也让这次 hide 不再触发一轮 Resized + is_minimized。
                        let _ = window.unminimize();
                        enter_float_mode(window.app_handle());
                    });
                }
                // 浮窗被 Alt+F4 关掉 = 关掉应用。浮窗太小放不下确认对话框，而放任它销毁会让
                // 收起状态下的应用只剩托盘图标，且窗口销毁后无法再 show 回来。
                (FLOAT_LABEL, tauri::WindowEvent::CloseRequested { api, .. }) => {
                    api.prevent_close();
                    shutdown_and_exit(window.app_handle());
                }
                _ => {}
            }
        })
        .manage(EngineState(burst_engine.clone()))
        .manage(ProfileNotice::default())
        .manage(Jx3State::default())
        .manage(UpdateLock(AtomicBool::new(false)))
        .invoke_handler(tauri::generate_handler![
            set_global_enabled,
            get_global_enabled,
            set_global_hotkeys,
            set_rules,
            get_rules,
            get_hotkeys,
            get_active_rules,
            get_rule_states,
            relay_key_event,
            get_key_policy,
            take_profile_notice,
            get_input_mode,
            set_input_mode,
            is_driver_installed,
            install_driver,
            uninstall_driver,
            is_dd_hid_driver_installed,
            uninstall_dd_hid_driver,
            is_elevated,
            relaunch_as_admin,
            set_panel_aspect,
            save_profile,
            set_profile_signature,
            get_license_status,
            get_membership_agreement,
            activate_license,
            load_profile,
            list_profiles,
            init_default_profile,
            get_active_profile_path,
            rename_profile,
            delete_profile,
            fork_active_profile,
            export_profile,
            import_qzh_profile,
            needs_agreement,
            agree_license,
            should_auto_enable_on_start,
            open_link,
            get_jx3_column,
            open_jx3_link,
            check_update,
            apply_pending_update,
            exit_app,
            show_main_panel,
            minimize_to_float,
            set_autostart,
            set_run_as_admin,
            log_from_frontend,
            open_app_dir,
            get_app_status,
            get_display_environment,
            diagnose_environment,
            repair_dd_hid_residue,
            repair_interception_residue,
            repair_corrupted_profiles,
            repair_clean_logs,
            export_dd_hid_diagnostic_report,
            scan_import_configs,
            preview_import,
            import_external_config,
            import_sound_file,
            read_sound_file,
            delete_sound_file,
        ])
        .setup(move |app| {
            bootstrap::webview_scale::setup(app.handle());
            app.manage(LicenseState::load(app.handle()));

            // 全局热键回调：同步托盘菜单与前端开关状态
            // 注意：这两个回调均由 WH_KEYBOARD_LL hook 线程同步调用。
            // 在 hook 返回前，WebView2（若聚焦）无法向主线程 STA 投递 COM 消息，
            // 直接在 hook 线程执行 emit/tray 操作会造成死锁并导致 hook 超时被移除。
            // 解决方案：将 UI 操作派发到 Tauri 异步运行时，hook 立即返回。
            {
                let handle = app.handle().clone();
                burst_engine.set_on_global_changed(move |enabled| {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::tray::refresh_menu(&handle);
                        let _ = handle.emit("global-enabled-changed", enabled);
                    });
                });
            }

            // 外设驱动链路断开：win-input 已自行回退 SendInput，这里只报事件。回调可能跑在调度线程上，
            // 同全局开关回调一样派发出去，不在原线程上 emit。
            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                win_input::set_on_hidstick_offline(move || {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        crate::bootstrap::input::on_hid_stick_link_lost(&handle);
                    });
                });
            }

            // 面板显隐热键回调
            {
                let handle = app.handle().clone();
                burst_engine.set_on_panel_toggle(move || {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(win) = handle.get_webview_window(PANEL_LABEL) {
                            let visible = win.is_visible().unwrap_or(false);
                            let minimized = win.is_minimized().unwrap_or(false);
                            if visible && !minimized {
                                enter_float_mode(&handle);
                            } else {
                                enter_panel_mode(&handle);
                            }
                        }
                    });
                });
            }

            let need_agreement = check_agreement(app.handle());
            load_or_init_profile(app.handle(), &burst_engine);
            init_input_backend(app.handle());
            start_hid_stick_watcher(app.handle());
            apply_run_as_admin(app.handle());
            tray::setup_tray(app.handle(), engine_for_tray)?;

            if let Some(panel) = app.get_webview_window("panel") {
                panel.show()?;
                if need_agreement {
                    let _ = panel.emit("show-agreement", AGREEMENT_VERSION);
                }
            }

            start_listener(engine_for_listener);

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                check_for_updates(handle).await;
            });
            commands::jx3::prefetch(app.handle());

            info!("{} started", APP_NAME);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building FlairBloom")
        .run(move |_app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                engine_for_exit.shutdown();
            }
        });
}
