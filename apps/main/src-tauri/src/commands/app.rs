//! 协议同意 / 检查更新 / 支持&帮助 / 退出。

use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

use crate::bootstrap::{
    agreement::AGREEMENT_VERSION,
    update::{check_and_download, check_with_fallback, CheckTrigger, UpdateLock},
};

pub(crate) const PENDING_UPDATE_DIR: &str = "pending_update";

/// 产品页的「支持&帮助」一节。
const SUPPORT_URL: &str = "https://app.xwink.fun/flair-bloom/#support";
/// QQ 邮箱网页版。「写邮件」打开它、邮箱由前端先复制好：不走 mailto，没设默认邮件程序就打不开，
/// 设了的多半是启动很慢的 Outlook，点下去像没响应；也不拼写信链接，QQ 邮箱没有公开的写信参数，
/// 「邮我」分享链接（`qm_share?t=qm_mailme&email=`）带明文邮箱、新版 `wx.mail.qq.com/home/index#/compose`
/// （带不带 `to=`）实测都只落到收件箱；地址栏里能预填收件人的写信链接带的是本人会话 `sid` 与草稿 `mailid`，
/// 换个人就失效。
const QQ_MAIL_URL: &str = "https://mail.qq.com/";

/// 用系统默认程序打开外部链接：`support` 是产品页的支持一节，`mail` 是 QQ 邮箱网页版，`membership`
/// 是 xwink 会员服务协议正文，`jx3box` 是剑网三专栏首页的 JX3Box。只认这几个
/// 写死的目标、前端只传名字：没装 opener 插件，也就不给 WebView 开「打开任意网址」的口子。
/// 专栏列表里的链接走 [`super::jx3::open_jx3_link`]，只放行当前数据里有的地址。
#[tauri::command]
pub fn open_link(target: String) -> Result<(), String> {
    let link = match target.as_str() {
        "support" => SUPPORT_URL,
        "membership" => super::license::MEMBERSHIP_TERMS_URL,
        "mail" => QQ_MAIL_URL,
        "jx3box" => super::jx3::JX3BOX_HOME,
        other => return Err(format!("未知链接: {other}")),
    };
    shell_open(link)
}

pub(crate) fn shell_open(link: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        // 走 ShellExecute 交给协议处理器（默认浏览器），锚点原样带过去
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, target) = (wide("open"), wide(link));
        // SAFETY: verb、target 是以 0 结尾的 UTF-16 串，活到调用返回；其余参数按文档可为空
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        // 文档约定：返回值大于 32 才算成功
        if code <= 32 {
            return Err(format!("没有能打开它的程序（错误码 {code}）"));
        }
    }
    #[cfg(not(windows))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener)
            .arg(link)
            .spawn()
            .map_err(|e| format!("无法打开: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn needs_agreement(app: AppHandle) -> Result<bool, String> {
    let store = app
        .store(crate::STORE_PATH)
        .map_err(|e| format!("无法读取存储: {e}"))?;
    let agreed = store
        .get("agreed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let version = store
        .get("agreement_version")
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    Ok(!agreed || version.as_deref() != Some(AGREEMENT_VERSION))
}

#[tauri::command]
pub fn agree_license(app: AppHandle) -> Result<(), String> {
    let store = app
        .store(crate::STORE_PATH)
        .map_err(|e| format!("无法读取存储: {e}"))?;
    store.set("agreed", serde_json::json!(true));
    store.set("agreed_at", serde_json::json!(now_secs()));
    store.set("agreement_version", serde_json::json!(AGREEMENT_VERSION));
    store.set(
        "app_version_at_agree",
        serde_json::json!(env!("CARGO_PKG_VERSION")),
    );
    store.save().map_err(|e| format!("保存协议状态失败: {e}"))?;
    crate::insights::activate(&app);
    Ok(())
}

/// 这次启动该不该按设置自动开全局，判定见 [`crate::bootstrap::startup::should_auto_enable_on_start`]。
/// 面板在启动收尾与同意协议后各问一次，答「该」就走用户点开关的同一条路径打开。
#[tauri::command]
pub fn should_auto_enable_on_start(app: AppHandle) -> bool {
    let need_agreement = crate::bootstrap::agreement::check_agreement(&app);
    crate::bootstrap::startup::should_auto_enable_on_start(&app, need_agreement)
}

/// 浮窗"放大"按钮 / 其它呼出入口：显示主面板并隐藏浮窗。
#[tauri::command]
pub fn show_main_panel(app: AppHandle) {
    crate::enter_panel_mode(&app);
}

/// 主面板"最小化到浮窗"：隐藏主面板并显示常驻浮窗。
#[tauri::command]
pub fn minimize_to_float(app: AppHandle) {
    crate::enter_float_mode(&app);
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    crate::shutdown_and_exit(&app);
}

/// 开机自启开关。取设定值而不是取反：界面上三个启动开关互斥，关掉另一个时必须能明确地
/// 置为 false——取反会在连点里把刚关掉的那个又打开。返回落定后的真实状态。
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let launch = app.autolaunch();
    if enabled {
        launch
            .enable()
            .map_err(|e| format!("启用开机自启失败: {e}"))?;
    } else {
        launch
            .disable()
            .map_err(|e| format!("禁用开机自启失败: {e}"))?;
    }
    Ok(launch.is_enabled().unwrap_or(enabled))
}

/// 「以管理员模式启动」开关：给当前 exe 打上 / 去掉兼容性标志。
///
/// 游戏模式（Interception / DD 驱动）必须以管理员运行，否则每次都要点一次「提权重启」。
/// 与开机自启的互斥由前端保证：`HKCU\Run` 拉起需要提权的程序会被系统直接拦下，
/// 两个都开等于开机根本不启动，而且没有任何提示。
/// 取设定值而不是取反：互斥时前端要能明确地把它关掉。
///
/// 注册表是生效的那一份，settings.json 只记用户意图，供 `bootstrap::startup::apply_run_as_admin`
/// 在标志被更新抹掉后自愈；意图写失败不算切换失败，标志本身已经生效了。
#[tauri::command]
pub fn set_run_as_admin(app: AppHandle, enabled: bool) -> Result<(), String> {
    let exe = current_exe_path()?;
    win_sysinfo::run_as_admin::set_enabled(&exe, enabled)?;
    if let Err(e) = remember_run_as_admin(&app, enabled) {
        warn!("记录管理员启动意图失败: {e}");
    }
    info!(
        "以管理员模式启动：{}",
        if enabled { "已开启" } else { "已关闭" }
    );
    Ok(())
}

fn remember_run_as_admin(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let store = app
        .store(crate::STORE_PATH)
        .map_err(|e| format!("无法读取存储: {e}"))?;
    store.set(
        crate::bootstrap::startup::RUN_AS_ADMIN_KEY,
        serde_json::json!(enabled),
    );
    store.save().map_err(|e| format!("{e}"))
}

/// 当前 exe 路径。注册表里的值名要与资源管理器写入的一致，verbatim 前缀必须去掉。
pub(crate) fn current_exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|path| {
            win_driver::path_util::strip_verbatim(path)
                .to_string_lossy()
                .into_owned()
        })
        .map_err(|e| format!("无法获取程序路径: {e}"))
}

/// 用户主动检查更新：一律走「检查 → 下载 → 弹公告」，与「自动更新」开关无关——
/// 开关只决定启动时要不要自动下载，用户点了就是表达了要更新的意图。
#[tauri::command]
pub async fn check_update(app: AppHandle, lock: State<'_, UpdateLock>) -> Result<(), String> {
    let _guard = lock.acquire().ok_or("更新正在进行中")?;
    check_and_download(&app, CheckTrigger::Manual).await
}

/// 立即安装已下载的更新包。安装器会接管并重启应用，正常路径下本命令不返回。
#[tauri::command]
pub async fn apply_pending_update(
    app: AppHandle,
    lock: State<'_, UpdateLock>,
) -> Result<(), String> {
    let _guard = lock.acquire().ok_or("更新正在进行中")?;
    if try_apply_pending_update(&app).await {
        Ok(())
    } else {
        Err("没有可安装的更新包，或安装未能启动".to_string())
    }
}

/// 检查待安装包并在版本匹配时立即安装（应用将自动重启）。
/// 返回 true 表示安装已触发。
pub async fn try_apply_pending_update(app: &AppHandle) -> bool {
    let dir = match app
        .path()
        .app_local_data_dir()
        .map(|d| d.join(PENDING_UPDATE_DIR))
    {
        Ok(d) => d,
        Err(e) => {
            warn!("无法获取应用数据目录: {}", e);
            return false;
        }
    };

    let installer_path = dir.join("installer");
    let version_path = dir.join("version");

    if !installer_path.exists() || !version_path.exists() {
        return false;
    }

    let saved_version = match std::fs::read_to_string(&version_path) {
        Ok(v) => v.trim().to_string(),
        Err(e) => {
            warn!("读取待安装版本失败: {}", e);
            return false;
        }
    };

    if version_ge(env!("CARGO_PKG_VERSION"), &saved_version) {
        info!(
            "待安装版本 {} 已过期（当前 {}），清理",
            saved_version,
            env!("CARGO_PKG_VERSION")
        );
        let _ = std::fs::remove_dir_all(&dir);
        return false;
    }

    let saved_bytes = match std::fs::read(&installer_path) {
        Ok(b) => b,
        Err(e) => {
            warn!("读取安装包文件失败: {}", e);
            return false;
        }
    };

    let update = match check_with_fallback(app).await {
        Ok(Some(u)) if u.version == saved_version => u,
        Ok(Some(u)) => {
            info!(
                "服务器版本 {} 与已下载版本 {} 不匹配，清理旧安装包",
                u.version, saved_version
            );
            let _ = std::fs::remove_dir_all(&dir);
            return false;
        }
        Ok(None) => {
            info!("服务器端无可用更新，清理待安装包 {}", saved_version);
            let _ = std::fs::remove_dir_all(&dir);
            return false;
        }
        Err(e) => {
            warn!("检查更新失败，待安装包保留下次重试: {}", e);
            return false;
        }
    };

    // 安装包已经在本地，install 不走网络，不需要改写下载地址
    match update.install(saved_bytes) {
        Ok(_) => {
            info!("更新安装完成，应用即将重启");
            let _ = std::fs::remove_dir_all(&dir);
            true
        }
        Err(e) => {
            warn!("安装更新失败: {}", e);
            false
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时钟早于 UNIX epoch")
        .as_secs()
}

fn version_ge(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    parse(a) >= parse(b)
}
