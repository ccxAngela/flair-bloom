//! tracing 初始化 + panic hook（写崩溃日志 + 弹提示窗口）+ 旧日志清理。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::APP_IDENTIFIER;

pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(base).join(APP_IDENTIFIER).join("logs")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join("Library/Logs")
            .join(APP_IDENTIFIER)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        PathBuf::from(".").join("logs")
    }
}

pub fn cleanup_old_logs(dir: &PathBuf) {
    let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("crash-") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// 初始化 tracing（stdout + rolling file）+ panic hook。
/// 返回的 `_guard` 必须保持存活直到进程退出（防止 writer 线程提前退出）。
pub fn init(dir: &PathBuf) {
    let file_appender = tracing_appender::rolling::daily(dir, "flair-bloom");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    // Leak the guard so the background writer thread lives until process::exit.
    Box::leak(Box::new(guard));

    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().with_writer(std::io::stdout))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    let crash_dir = dir.clone();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if crate::insights::is_worker_thread() {
            tracing::warn!("使用统计观察发生故障，已隔离");
            return;
        }
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let crash_file = crash_dir.join(format!("crash-{ts}.log"));
        let msg = format!(
            "{}\n\nBacktrace:\n{:?}",
            info,
            std::backtrace::Backtrace::force_capture()
        );
        let _ = std::fs::write(&crash_file, &msg);
        eprintln!("PANIC: {info}");
        show_crash_dialog(crash_file);
        prev_hook(info);
    }));
}

static CRASH_DIALOG_SHOWN: AtomicBool = AtomicBool::new(false);

/// 弹崩溃提示窗口（打开日志文件夹 / 关闭）。
/// - 在独立线程展示：panic 可能发生在 LL hook 等敏感线程，原地阻塞会拖垮系统输入；
/// - 进程级去重：连锁 panic 只弹第一次，避免弹窗风暴；
/// - 主线程 panic 后进程随即退出时弹窗可能来不及展示，属可接受降级（崩溃日志仍在）。
fn show_crash_dialog(crash_file: PathBuf) {
    if CRASH_DIALOG_SHOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        // 弹窗自身出错不能再进 panic hook 链路
        let _ = std::panic::catch_unwind(move || {
            const OPEN_LABEL: &str = "打开日志文件夹";
            let result = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title(crate::APP_NAME_CN)
                .set_description(format!(
                    "{}遇到了一个问题并已崩溃。\n\n崩溃日志已保存，如需报告问题请提供：\n{}",
                    crate::APP_NAME_CN,
                    crash_file.display()
                ))
                .set_buttons(rfd::MessageButtons::OkCancelCustom(
                    OPEN_LABEL.to_string(),
                    "关闭".to_string(),
                ))
                .show();
            if matches!(result, rfd::MessageDialogResult::Custom(ref s) if s == OPEN_LABEL) {
                if let Some(dir) = crash_file.parent() {
                    let _ = crate::commands::log::open_dir_in_explorer(dir);
                }
            }
        });
    });
}
