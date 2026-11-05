//! 用户协议版本检查。

use tauri_plugin_store::StoreExt;
use tracing::warn;

pub const AGREEMENT_VERSION: &str = "1.6";

/// 检查是否需要展示协议弹窗。返回 `true` 表示需要（未同意或版本不符）。
pub fn check_agreement(app: &tauri::AppHandle) -> bool {
    match app.store(crate::STORE_PATH) {
        Ok(store) => {
            let agreed = store
                .get("agreed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let version = store
                .get("agreement_version")
                .and_then(|v| v.as_str().map(|s| s.to_string()));
            !agreed || version.as_deref() != Some(AGREEMENT_VERSION)
        }
        Err(e) => {
            warn!("无法读取协议状态: {}", e);
            true
        }
    }
}
