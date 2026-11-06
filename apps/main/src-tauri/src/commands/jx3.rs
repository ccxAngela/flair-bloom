//! 剑网三专栏：列表由产品站托管，启动时在后台取一次；取不到就依次回落上次取到的本地缓存与随包快照。
//!
//! 应用不直接调 JX3Box：它的接口没有公开文档，一改格式已装的客户端全坏；由站点中转，坏了只改
//! 采集脚本、重发站点。只读取、不上传。契约见 `skills/flair-bloom/references/engineering/architecture.md`。
//! 字段的逐项校验在前端 `panel/jx3/column.ts`（与测试、采集脚本共用），这里只认格式版本与骨架；
//! 前端拿到后再跑一遍，不合格就提示而不渲染。

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};
use tracing::warn;

use crate::commands::profile::now_secs;

pub(crate) const JX3BOX_HOME: &str = "https://www.jx3box.com/";
const REMOTE_URL: &str = "https://app.xwink.fun/flair-bloom/jx3/column.json";
/// 随包快照与站点发布的是同一个文件，只有一个数据来源。
const BUNDLED: &str = include_str!("../../../../site/public/jx3/column.json");
const CACHE_FILE: &str = "jx3-column.json";
/// 与前端 `COLUMN_SCHEMA` 一致。
const SCHEMA: u64 = 1;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnSource {
    Remote,
    Cache,
    Bundled,
}

/// 没用上线上数据的原因，界面据此提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    /// 网络不通或站点出错。
    Offline,
    /// 站点的数据格式比本版应用新，要更新应用。
    NewerSchema,
    /// 站点返回的内容不是认得的格式。
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnResult {
    pub data: Value,
    pub source: ColumnSource,
    /// 线上数据或缓存的取得时间（Unix 秒）；快照没有。
    pub fetched_at: Option<u64>,
    pub fallback: Option<Fallback>,
}

#[derive(Serialize, Deserialize)]
struct Cached {
    fetched_at: u64,
    data: Value,
}

// 持有当前数据，让第三方外链的放行范围与界面展示一致。
#[derive(Default)]
pub struct Jx3State {
    held: Mutex<Option<ColumnResult>>,
    /// 启动预取、打开专栏、刷新可能同时到；排队取，状态里留下的总是最后返回给界面的那份，
    /// 否则慢的预取晚到会盖掉界面正显示的数据，外链放行跟着对不上。
    loading: tauri::async_runtime::Mutex<()>,
}

impl Jx3State {
    fn held(&self) -> Option<ColumnResult> {
        self.held.lock().ok()?.clone()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Ok,
    NewerSchema,
    Invalid,
}

fn verdict(data: &Value) -> Verdict {
    let Some(schema) = data.get("schema").and_then(Value::as_u64) else {
        return Verdict::Invalid;
    };
    if schema > SCHEMA {
        return Verdict::NewerSchema;
    }
    let has_arrays = ["schools", "whitepapers", "macros"]
        .iter()
        .all(|k| data.get(k).is_some_and(Value::is_array));
    if schema == SCHEMA && has_arrays {
        Verdict::Ok
    } else {
        Verdict::Invalid
    }
}

fn bundled() -> Value {
    serde_json::from_str(BUNDLED)
        .expect("随包的 column.json 由 scripts/jx3-column.test.ts 保证合法")
}

/// 按「线上 → 缓存 → 快照」挑一份。`remote` 为 `None` 表示没取到（网络不通、站点出错）。
fn choose(remote: Option<Value>, cache: Option<(u64, Value)>, now: u64) -> ColumnResult {
    let fallback = match remote {
        Some(data) => match verdict(&data) {
            Verdict::Ok => {
                return ColumnResult {
                    data,
                    source: ColumnSource::Remote,
                    fetched_at: Some(now),
                    fallback: None,
                }
            }
            Verdict::NewerSchema => Fallback::NewerSchema,
            Verdict::Invalid => Fallback::Invalid,
        },
        None => Fallback::Offline,
    };
    match cache {
        Some((fetched_at, data)) if verdict(&data) == Verdict::Ok => ColumnResult {
            data,
            source: ColumnSource::Cache,
            fetched_at: Some(fetched_at),
            fallback: Some(fallback),
        },
        _ => ColumnResult {
            data: bundled(),
            source: ColumnSource::Bundled,
            fetched_at: None,
            fallback: Some(fallback),
        },
    }
}

fn link_allowed(data: &Value, url: &str) -> bool {
    if super::app::xwink_url(url).is_some() {
        return true;
    }
    if !url.starts_with("https://") {
        return false;
    }
    ["whitepapers", "macros"].iter().any(|list| {
        data.get(list)
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|e| e.get("url").and_then(Value::as_str) == Some(url))
            })
    })
}

fn cache_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(CACHE_FILE))
}

fn read_cache(app: &AppHandle) -> Option<(u64, Value)> {
    let bytes = std::fs::read(cache_path(app)?).ok()?;
    let cached: Cached = serde_json::from_slice(&bytes).ok()?;
    Some((cached.fetched_at, cached.data))
}

fn write_cache(app: &AppHandle, fetched_at: u64, data: &Value) {
    let Some(path) = cache_path(app) else { return };
    let cached = Cached {
        fetched_at,
        data: data.clone(),
    };
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, serde_json::to_vec(&cached).unwrap_or_default()));
    if let Err(e) = result {
        warn!("写剑网三专栏缓存失败：{e}");
    }
}

async fn fetch_remote() -> Option<Value> {
    let client = super::license::build_http_client(REQUEST_TIMEOUT)
        .inspect_err(|e| warn!("{e}"))
        .ok()?;
    let resp = client
        .get(REMOTE_URL)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .inspect_err(|e| warn!("取剑网三专栏数据失败：{e}"))
        .ok()?;
    resp.json::<Value>()
        .await
        .inspect_err(|e| warn!("剑网三专栏数据不是 JSON：{e}"))
        .ok()
}

/// 取专栏数据并存进状态。`refresh` 为假时已有数据就直接用（排在启动预取后面的打开请求等它回来
/// 拿同一份，不重复联网）；为真时总是联网再取。
async fn load(app: &AppHandle, refresh: bool) -> ColumnResult {
    let state = app.state::<Jx3State>();
    let _queue = state.loading.lock().await;
    if !refresh {
        if let Some(held) = state.held() {
            return held;
        }
    }
    let now = now_secs();
    let result = choose(fetch_remote().await, read_cache(app), now);
    if result.source == ColumnSource::Remote {
        write_cache(app, now, &result.data);
    }
    if let Ok(mut held) = state.held.lock() {
        *held = Some(result.clone());
    }
    result
}

/// 启动时在后台取一次专栏数据：联网说明是「启动时从官网拉取必要信息」，打开专栏也不用等。
pub fn prefetch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        load(&app, false).await;
    });
}

/// 取专栏数据。`refresh` 为假时用启动时取到的那份；启动预取还没回来就等它。
#[tauri::command]
pub async fn get_jx3_column(app: AppHandle, refresh: bool) -> Result<ColumnResult, String> {
    Ok(load(&app, refresh).await)
}

#[tauri::command]
pub fn open_jx3_link(state: State<'_, Jx3State>, url: String) -> Result<(), String> {
    if let Some(link) = super::app::xwink_url(&url) {
        return super::app::shell_open(link.as_str());
    }
    let allowed = state
        .held()
        .is_some_and(|held| link_allowed(&held.data, &url));
    if !allowed {
        return Err("这个链接不在剑网三专栏的列表里".into());
    }
    super::app::shell_open(&url)
}

#[cfg(test)]
#[path = "jx3_tests.rs"]
mod tests;
