//! 更新锁 + 静默更新 + 启动期待安装包检测。
//!
//! `UpdateLock` 由 lib.rs `.manage()` 注入，`commands/app.rs` 的 `check_update`
//! 命令通过 `State<UpdateLock>` 获取它。

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;
use tracing::{info, warn};

pub const DEFAULT_GITHUB_PROXY: &str = "https://gh-proxy.com/";
pub const UPDATE_DOWNLOAD_PROGRESS_EVENT: &str = "update-download-progress";
pub const UPDATE_DOWNLOAD_FAILED_EVENT: &str = "update-download-failed";
/// 检查到新版本但按用户设置不自动下载，交由前端在标题栏提示。
pub const UPDATE_AVAILABLE_EVENT: &str = "update-available";

/// settings.json 中「自动更新」开关的键名。缺省视为开启。
pub const AUTO_UPDATE_KEY: &str = "autoUpdate";

/// 读取「自动更新」开关。读不到 store 或未设置时默认开启——新装用户应当自动拿到修复。
pub fn auto_update_enabled(app: &tauri::AppHandle) -> bool {
    use tauri_plugin_store::StoreExt;
    app.store(crate::STORE_PATH)
        .ok()
        .and_then(|store| store.get(AUTO_UPDATE_KEY).and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

/// 一次更新检查的来源，决定要不要下载、以及「已是最新」要不要出声。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckTrigger {
    /// 启动时自动检查。是否下载取决于「自动更新」开关；无更新时保持安静。
    Startup,
    /// 用户主动触发（菜单「检查更新」或标题栏的可用更新提示）。
    /// 一律下载——用户已经表达了要更新的意图，开关只管「自动」那一档。
    Manual,
}

const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// 保证同一时刻只有一个更新任务在运行。
pub struct UpdateLock(pub AtomicBool);

impl UpdateLock {
    pub fn acquire(&self) -> Option<UpdateLockGuard<'_>> {
        if self
            .0
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(UpdateLockGuard(&self.0))
        } else {
            None
        }
    }
}

pub struct UpdateLockGuard<'a>(&'a AtomicBool);

impl Drop for UpdateLockGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 启动时的更新检查流程：先尝试应用待安装包，再按「自动更新」开关决定是否后台下载。
pub async fn check_for_updates(app: tauri::AppHandle) {
    if crate::commands::app::try_apply_pending_update(&app).await {
        return;
    }

    let lock = app.state::<UpdateLock>();
    let _guard = match lock.acquire() {
        Some(g) => g,
        None => return,
    };

    if let Err(e) = check_and_download(&app, CheckTrigger::Startup).await {
        warn!("启动期更新检查失败: {}", e);
    }
}

/// 检查更新，并按 `trigger` 与「自动更新」开关决定是否下载。
///
/// 启动检查与菜单「检查更新」共用这一条流程：两边曾经各写一份几乎相同的实现，
/// 改一处漏一处（代理、进度事件、落盘路径都得同步），现在只留这一份。
pub async fn check_and_download(
    app: &tauri::AppHandle,
    trigger: CheckTrigger,
) -> Result<(), String> {
    let mut update = match check_with_fallback(app).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            info!("已是最新版本");
            // 启动检查保持安静：没更新时弹「已是最新」只会打扰人
            if trigger == CheckTrigger::Manual {
                let _ = app.emit("update-not-available", ());
            }
            return Ok(());
        }
        Err(e) => {
            warn!("检查更新失败: {}", e);
            return Err(format!("检查更新失败: {e}"));
        }
    };

    let version = update.version.clone();
    let notes = update.body.clone();
    info!("发现新版本: {}", version);

    let download = trigger == CheckTrigger::Manual || auto_update_enabled(app);
    if !download {
        info!("自动更新已关闭，仅提示可用更新");
        let _ = app.emit(
            UPDATE_AVAILABLE_EVENT,
            serde_json::json!({ "version": version, "notes": notes }),
        );
        return Ok(());
    }

    let direct_url = proxy_github_download_url(app, &mut update);
    // silent=true 时前端只画进度条不弹 toast：启动期自动下载不该主动打断用户
    let _ = app.emit(
        "update-downloading",
        serde_json::json!({
            "version": version,
            "silent": trigger == CheckTrigger::Startup,
        }),
    );

    let bytes = download_with_fallback(app, &mut update, &version, direct_url).await?;

    save_pending_update(app, &version, &bytes)?;
    let _ = app.emit(
        "update-ready",
        serde_json::json!({ "version": version, "notes": notes }),
    );
    Ok(())
}

/// 先走代理下载，失败了用 `direct_url`（改写前的 GitHub 原始地址）再试一次。
///
/// 代理是第三方免费服务，限流或抽风都会让下载失败，而 GitHub 直连在国内虽慢却仍可用，
/// 比「有新版本但装不上」强。失败事件只在两条路都走不通时才发给前端——回退成功还弹一个
/// 「下载失败」只会吓人。
async fn download_with_fallback(
    app: &tauri::AppHandle,
    update: &mut tauri_plugin_updater::Update,
    version: &str,
    direct_url: Option<tauri::Url>,
) -> Result<Vec<u8>, String> {
    let first = match download_update(app, update, version).await {
        Ok(bytes) => return Ok(bytes),
        Err(e) => e,
    };

    let Some(direct_url) = direct_url else {
        emit_update_download_failed(app, version, &first.to_string());
        return Err(format!("下载更新失败: {first}"));
    };

    warn!("代理下载失败，回退 GitHub 直连: {}", first);
    update.download_url = direct_url;
    download_update(app, update, version).await.map_err(|e| {
        emit_update_download_failed(app, version, &e.to_string());
        format!("下载更新失败: {e}")
    })
}

/// 把下载好的安装包落盘，等待用户点「重启并更新」或下次启动时安装。
pub fn save_pending_update(
    app: &tauri::AppHandle,
    version: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("无法获取应用数据目录: {e}"))?
        .join(crate::commands::app::PENDING_UPDATE_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建更新目录: {e}"))?;
    std::fs::write(dir.join("installer"), bytes).map_err(|e| format!("保存安装包失败: {e}"))?;
    std::fs::write(dir.join("version"), version).map_err(|e| format!("保存版本信息失败: {e}"))?;
    Ok(())
}

/// 逐个端点检查更新，第一个读通的说了算；全失败才返回错误。
///
/// 不把两条端点一起丢给插件自己轮：它在「HTTP 200 但响应体不是 JSON」时是
/// `res.json().await?`，直接从 check 返回、不会 continue 到下一条（tauri-plugin-updater
/// 的 check 循环）。而代理限流最常见的形态恰恰是 200 加一张 HTML 错误页，那样第二条
/// 直连端点永远轮不上。一个端点一个 updater，任何失败形态都能落到下一条。
pub async fn check_with_fallback(
    app: &tauri::AppHandle,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let mut last_error = None;
    for endpoint in configured_update_endpoints(app)? {
        let newer = std::sync::Arc::new(AtomicBool::new(false));
        let updater = match build_updater_for(app, endpoint.clone(), newer.clone()) {
            Ok(updater) => updater,
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        };
        match updater.check().await {
            Ok(update) => {
                if let Some(update) = &update {
                    crate::insights::receive_services(app, &endpoint, &update.raw_json);
                }
                return Ok(update.filter(|_| newer.load(Ordering::SeqCst)));
            }
            Err(e) => {
                warn!("更新端点 {} 不可用: {}", endpoint, e);
                last_error = Some(format!("{e}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "没有可用的更新端点".to_string()))
}

fn build_updater_for(
    app: &tauri::AppHandle,
    endpoint: tauri::Url,
    newer: std::sync::Arc<AtomicBool>,
) -> Result<tauri_plugin_updater::Updater, String> {
    let mut manifest = endpoint;
    manifest.query_pairs_mut().append_pair("updates", "1");
    app.updater_builder()
        .version_comparator(move |current, release| {
            newer.store(release.version > current, Ordering::SeqCst);
            // 同版本也读取服务配置；下载决策仍只接受严格更新的版本。
            true
        })
        .timeout(Duration::from_secs(12))
        .endpoints(vec![manifest])
        .map_err(|e| format!("{e}"))?
        .build()
        .map_err(|e| format!("{e}"))
}

/// 客户端只配置第一方入口，发布清单镜像由服务端解析。
pub(crate) fn configured_update_endpoints(
    app: &tauri::AppHandle,
) -> Result<Vec<tauri::Url>, String> {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("FLAIR_BOOTSTRAP_URL") {
        let url = tauri::Url::parse(&value).map_err(|_| "无效的本地配置入口".to_string())?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("开发配置入口只允许本机 HTTP".into());
        }
        return Ok(vec![url]);
    }
    let config = app.config();
    let endpoints = config
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("endpoints"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| "缺少 updater.endpoints 配置".to_string())?;
    endpoints
        .iter()
        .filter_map(|endpoint| endpoint.as_str())
        .map(|url| tauri::Url::parse(url).map_err(|e| format!("无效的更新地址: {e}")))
        .collect()
}

pub async fn download_update(
    app: &tauri::AppHandle,
    update: &tauri_plugin_updater::Update,
    version: &str,
) -> Result<Vec<u8>, tauri_plugin_updater::Error> {
    let handle = app.clone();
    let mut downloaded = 0u64;
    let mut last_total = None;
    let mut last_percent = None;
    let mut last_emit = Instant::now() - PROGRESS_EMIT_INTERVAL;

    emit_update_download_progress(app, version, 0, None, false);

    let result = update
        .download(
            |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                last_total = total;
                let percent =
                    total.and_then(|total| downloaded.saturating_mul(100).checked_div(total));
                let percent_changed = match (percent, last_percent) {
                    (Some(current), Some(last)) => current > last,
                    (Some(_), None) => true,
                    _ => false,
                };
                let now = Instant::now();
                if percent_changed || now.duration_since(last_emit) >= PROGRESS_EMIT_INTERVAL {
                    emit_update_download_progress(&handle, version, downloaded, total, false);
                    last_emit = now;
                    if percent.is_some() {
                        last_percent = percent;
                    }
                }
            },
            || {
                info!("update downloaded");
            },
        )
        .await;

    // 失败事件由 download_with_fallback 在最后一条路也走不通时才发
    result.inspect(|bytes| {
        let final_size = bytes.len() as u64;
        emit_update_download_progress(
            app,
            version,
            final_size,
            last_total.or(Some(final_size)),
            true,
        );
    })
}

/// 把安装包地址改写成代理地址，返回改写前的原始地址供下载失败时回退。
/// 未改写（非 GitHub 地址，或本来就是代理地址）时返回 `None`。
pub fn proxy_github_download_url(
    app: &tauri::AppHandle,
    update: &mut tauri_plugin_updater::Update,
) -> Option<tauri::Url> {
    let proxy_prefix = github_proxy_prefix(app)?;

    let proxied = proxy_github_url(&update.download_url, &proxy_prefix);
    if proxied == update.download_url {
        return None;
    }
    info!("update download routed through GitHub proxy: {proxy_prefix}");
    Some(std::mem::replace(&mut update.download_url, proxied))
}

fn emit_update_download_failed(app: &tauri::AppHandle, version: &str, message: &str) {
    let _ = app.emit(
        UPDATE_DOWNLOAD_FAILED_EVENT,
        serde_json::json!({ "version": version, "message": message }),
    );
}

fn emit_update_download_progress(
    app: &tauri::AppHandle,
    version: &str,
    downloaded: u64,
    total: Option<u64>,
    done: bool,
) {
    let percent = total.and_then(|total| {
        if total == 0 {
            None
        } else {
            Some(((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 100.0))
        }
    });
    let _ = app.emit(
        UPDATE_DOWNLOAD_PROGRESS_EVENT,
        serde_json::json!({
            "version": version,
            "downloaded": downloaded,
            "total": total,
            "percent": percent,
            "done": done,
        }),
    );
}

fn github_proxy_prefix(_app: &tauri::AppHandle) -> Option<String> {
    Some(DEFAULT_GITHUB_PROXY.to_string())
}

fn normalize_proxy_prefix(prefix: &str) -> String {
    if prefix.ends_with('/') {
        prefix.to_string()
    } else {
        format!("{prefix}/")
    }
}

fn proxy_github_url(url: &tauri::Url, proxy_prefix: &str) -> tauri::Url {
    let Some(host) = url.host_str() else {
        return url.clone();
    };
    if proxy_host(proxy_prefix).is_some_and(|proxy_host| host.eq_ignore_ascii_case(&proxy_host))
        || !is_github_host(host)
    {
        return url.clone();
    }

    let proxied = format!("{}{}", normalize_proxy_prefix(proxy_prefix), url.as_str());
    match tauri::Url::parse(&proxied) {
        Ok(url) => url,
        Err(e) => {
            warn!("invalid GitHub proxy URL: {}", e);
            url.clone()
        }
    }
}

fn proxy_host(proxy_prefix: &str) -> Option<String> {
    tauri::Url::parse(proxy_prefix)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_string()))
}

fn is_github_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("github.com")
        || host.eq_ignore_ascii_case("www.github.com")
        || host.eq_ignore_ascii_case("api.github.com")
        || host.eq_ignore_ascii_case("codeload.github.com")
        || host.ends_with(".githubusercontent.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxies_github_release_url() {
        let url =
            tauri::Url::parse("https://github.com/x-wink/flair-bloom/releases/download/v1/a.zip")
                .unwrap();

        assert_eq!(
            proxy_github_url(&url, "https://gh-proxy.com/").as_str(),
            "https://gh-proxy.com/https://github.com/x-wink/flair-bloom/releases/download/v1/a.zip"
        );
    }

    #[test]
    fn leaves_non_github_url_unchanged() {
        let url = tauri::Url::parse("https://example.com/latest.json").unwrap();

        assert_eq!(proxy_github_url(&url, "https://gh-proxy.com/"), url);
    }

    #[test]
    fn does_not_proxy_proxy_url_again() {
        let url = tauri::Url::parse(
            "https://gh-proxy.com/https://github.com/x-wink/flair-bloom/releases/latest/download/latest.json",
        )
        .unwrap();

        assert_eq!(proxy_github_url(&url, "https://gh-proxy.com/"), url);
    }

    #[test]
    fn configured_endpoint_is_the_single_first_party_bootstrap() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        let endpoints = config["plugins"]["updater"]["endpoints"]
            .as_array()
            .expect("updater.endpoints 必须存在");

        assert_eq!(
            endpoints,
            &serde_json::json!(["https://platform.xwink.fun/api/v1/client-bootstrap/flair-bloom"])
                .as_array()
                .unwrap()
                .clone()
        );
    }
}
