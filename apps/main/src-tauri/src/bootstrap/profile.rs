//! 启动期配置文件加载与初始化。

use std::sync::Arc;
use tauri_plugin_store::StoreExt;
use tracing::{error, info, warn};

use crate::engine::BurstEngine;

pub fn load_or_init_profile(app: &tauri::AppHandle, engine: &Arc<BurstEngine>) {
    // 历史拼写修正：先把旧 defults.qzh 迁移为 defaults.qzh，再按 activePath 加载。
    crate::commands::profile::migrate_legacy_default_profile(app);

    let active_path: Option<String> = app.store(crate::STORE_PATH).ok().and_then(|store| {
        store
            .get(crate::commands::profile::ACTIVE_PATH_KEY)
            .or_else(|| store.get("activeProfilePath"))
            .and_then(|v| v.as_str().map(|s| s.to_string()))
    });

    let profiles_dir = match crate::commands::profile::profiles_dir(app) {
        Ok(d) => d,
        Err(e) => {
            error!("无法获取应用数据目录: {}", e);
            return;
        }
    };

    let active_path = active_path.map(|path| {
        if std::path::Path::new(&path)
            .file_name()
            .is_some_and(|name| name == "defults.qzh")
        {
            profiles_dir
                .join("defaults.qzh")
                .to_string_lossy()
                .into_owned()
        } else {
            path
        }
    });

    match active_path {
        Some(path) => match load_profile_from_path(&path, &profiles_dir) {
            Ok(mut profile) => {
                // 旧版本写下的、或他人分享的配置可能含当前版本不支持的按键。此处先净化，
                // 结果存进 ProfileNotice，等前端挂载后取走弹窗提醒（此刻窗口还没建，
                // 直接发事件会丢）。
                crate::commands::profile::sanitize_and_record(app, &mut profile);
                engine.set_hotkeys(profile.hotkeys);
                engine.set_rules(profile.rules);
                if let Some(file_name) = std::path::Path::new(&path).file_name() {
                    crate::commands::profile::set_active_path(
                        app,
                        &profiles_dir.join(file_name).to_string_lossy(),
                    );
                }
                info!("已加载配置: {}", path);
            }
            Err(e) => {
                warn!("加载配置失败 ({}): {}，回退到默认配置", path, e);
                if let Err(e2) = crate::commands::profile::create_default_profile(app, engine) {
                    error!("回退默认配置也失败: {}", e2);
                }
            }
        },
        None => {
            if let Err(e) = crate::commands::profile::create_default_profile(app, engine) {
                error!("初始化默认配置失败: {}", e);
            }
        }
    }
}

fn load_profile_from_path(
    path: &str,
    profiles_dir: &std::path::Path,
) -> Result<qzh_profile::Profile, String> {
    let file_name = std::path::Path::new(path)
        .file_name()
        .ok_or("无效文件路径")?
        .to_string_lossy();
    let safe_path = profiles_dir.join(file_name.as_ref());
    qzh_profile::load_from_path(&safe_path).map_err(|e| e.to_string())
}
