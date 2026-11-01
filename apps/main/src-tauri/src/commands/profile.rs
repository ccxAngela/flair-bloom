use crypto::aes;
use qzh_format::header::{FileHeader, MAGIC, VERSION};
use qzh_profile::{
    migrate_profile, Advanced, BurstMode, BurstRule, Hotkeys, KeyId, Profile, ProfileMeta,
    CURRENT_SCHEMA_VERSION, DEFAULT_INTERVAL_MS,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_store::StoreExt;
use tracing::{info, warn};

use super::engine::EngineState;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// 默认配置名（同时是文件名 stem）。受保护：不能改名也不能删除，
/// 用户对它的修改会触发自动 fork（见 [`fork_active_profile`]）。
pub const DEFAULT_PROFILE_NAME: &str = "defaults";

/// 历史拼写错误的旧默认配置名。启动时由 [`migrate_legacy_default_profile`] 一次性
/// 迁移为 [`DEFAULT_PROFILE_NAME`]，迁移后不再使用。
pub const LEGACY_DEFAULT_PROFILE_NAME: &str = "defults";

/// store 中存储「当前激活配置文件绝对路径」的键名。
pub const ACTIVE_PATH_KEY: &str = "activeProfilePath";

pub(crate) fn profiles_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|p| p.join("profiles"))
        .map_err(|e| format!("无法获取应用数据目录: {e}"))
}

pub(crate) fn now_secs() -> u64 {
    // 时钟早于 UNIX epoch 是 invariant 违反（比 1970 还早或被恶意回拨），
    // 静默返回 0 会导致 created_at/updated_at 错乱、list_profiles 排序失真，
    // 故选择显式 panic 让上层 panic hook 记录现场。
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时钟早于 UNIX epoch")
        .as_secs()
}

fn make_id() -> String {
    let n = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{:016x}-{}", now_secs(), n)
}

/// AAD 仅含 magic + version + flags，不含 nonce，所以可预先计算。
fn compute_aad() -> Vec<u8> {
    let mut aad = Vec::with_capacity(7);
    aad.extend_from_slice(MAGIC);
    aad.push(VERSION);
    aad.extend_from_slice(&0u16.to_le_bytes()); // flags = 0
    aad
}

pub(crate) fn write_profile_file_to_path(
    file_path: &Path,
    profile: &Profile,
) -> Result<String, String> {
    let json = serde_json::to_vec(profile).map_err(|e| format!("序列化失败: {e}"))?;
    let aad = compute_aad();
    let (ciphertext, nonce) = aes::encrypt(&json, &aad).map_err(|e| e.to_string())?;
    let header = FileHeader::new(nonce);

    let mut data = header.to_bytes();
    data.extend_from_slice(&ciphertext);

    let tmp_path = file_path.with_extension("qzh.tmp");
    std::fs::write(&tmp_path, &data).map_err(|e| format!("写入临时文件失败: {e}"))?;
    std::fs::rename(&tmp_path, file_path).map_err(|e| format!("替换配置文件失败: {e}"))?;
    Ok(file_path.to_string_lossy().to_string())
}

fn load_meta_from_file(file_path: &Path) -> Option<ProfileMeta> {
    let data = std::fs::read(file_path).ok()?;
    let header = FileHeader::from_bytes(&data)?;
    let aad = header.aad();
    let ciphertext = &data[FileHeader::SIZE..];
    let plaintext = aes::decrypt(ciphertext, &header.nonce, &aad).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&plaintext).ok()?;
    serde_json::from_value::<ProfileMeta>(value.get("meta")?.clone()).ok()
}

/// 读取并完整解密一个 .qzh 文件，返回带迁移与校验的 [`Profile`]。
/// 与 [`load_profile`] 命令的差别：不触发 engine.set_rules，也不更新 active path，
/// 适合 rename/fork/delete 等内部流程消费。
fn read_profile_from_file(file_path: &Path) -> Result<Profile, String> {
    let data = std::fs::read(file_path).map_err(|e| format!("读取文件失败: {e}"))?;
    let header = FileHeader::from_bytes(&data).ok_or("文件格式无效，可能已损坏")?;
    let aad = header.aad();
    let ciphertext = &data[FileHeader::SIZE..];
    let plaintext = aes::decrypt(ciphertext, &header.nonce, &aad).map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|e| format!("解析失败: {e}"))?;
    let version = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let value = if version < CURRENT_SCHEMA_VERSION {
        migrate_profile(value, version).map_err(|e| format!("配置迁移失败: {e}"))?
    } else if version > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "配置版本 {version} 高于当前支持的版本 {CURRENT_SCHEMA_VERSION}，请升级应用"
        ));
    } else {
        value
    };
    let mut profile: Profile =
        serde_json::from_value(value).map_err(|e| format!("反序列化失败: {e}"))?;
    // 加载即适配有效操作下限：旧配置 <10ms 的间隔静默钳到 10ms（管线可持续注入上限），
    // 否则超发会导致停止后下游输入队列排空滞后（「收不住」）。所有读取入口（load/list/
    // rename/delete/fork）都经此函数，故钳位只加这一处。
    profile.clamp_intervals();
    profile.validate().map_err(|e| e.to_string())?;
    Ok(profile)
}

fn profile_path_for_name(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.qzh", sanitize_filename(name)))
}

pub(crate) fn set_active_path(app: &AppHandle, path: &str) {
    if let Ok(store) = app.store(crate::STORE_PATH) {
        store.set(ACTIVE_PATH_KEY, serde_json::json!(path));
        let _ = store.save();
    }
}

/// 把文件里已有、前端不负责的元信息带到要写入的 meta 上。
///
/// - `created_at`：前端传 0 时沿用旧值，新文件取当前时间。
/// - `signature`：一律沿用旧值。前端自动保存每次从头拼 meta、不带署名，照抄才不会把署名清掉；
///   没有权益的用户导入的配置也靠这里原样保留。改署名只走 [`set_profile_signature`]。
fn carry_over_meta(meta: &mut ProfileMeta, old: Option<ProfileMeta>, now: u64) {
    if meta.created_at == 0 {
        meta.created_at = old.as_ref().map_or(now, |m| m.created_at);
    }
    meta.signature = old.map(|m| m.signature).unwrap_or_default();
}

#[tauri::command]
pub fn save_profile(app: AppHandle, name: String, mut profile: Profile) -> Result<String, String> {
    profile.validate().map_err(|e| e.to_string())?;

    let now = now_secs();
    let dir = profiles_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录: {e}"))?;

    let file_path = dir.join(format!("{}.qzh", sanitize_filename(&name)));

    carry_over_meta(&mut profile.meta, load_meta_from_file(&file_path), now);
    profile.meta.updated_at = now;
    profile.meta.app_version = env!("CARGO_PKG_VERSION").to_string();
    profile.schema_version = CURRENT_SCHEMA_VERSION;

    let path = write_profile_file_to_path(&file_path, &profile)?;

    // 自动保存只负责指定配置的持久化，不能重新激活旧快照或打断正在运行的规则。
    Ok(path)
}

/// 改指定配置的水印署名，返回净化后的值。需要 `flair.signature` 权益；界面在没有权益时
/// 本来就不露入口，这里再挡一道只是不让绕过界面直接调命令的写进去。
#[tauri::command]
pub fn set_profile_signature(
    app: AppHandle,
    license: State<super::license::LicenseState>,
    name: String,
    signature: String,
) -> Result<String, String> {
    if !license.grants(crypto::ENT_SIGNATURE) {
        return Err("没有自定义署名的权益".into());
    }
    let path = profile_path_for_name(&profiles_dir(&app)?, &name);
    let mut profile = read_profile_from_file(&path)?;
    profile.meta.signature = qzh_profile::sanitize_signature(&signature);
    profile.meta.updated_at = now_secs();
    write_profile_file_to_path(&path, &profile)?;
    Ok(profile.meta.signature)
}

/// 加载期净化的待提醒结果。启动加载先于窗口创建，事件会丢，故一律先存这里，
/// 前端挂载时用 `take_profile_notice` 取走；运行期切换配置再额外发一次事件即时提醒。
#[derive(Default)]
pub struct ProfileNotice(pub std::sync::Mutex<Option<qzh_profile::SanitizeReport>>);

/// 按当前输入模式净化配置，记日志并登记待提醒。返回是否有改动。
pub(crate) fn sanitize_and_record(app: &AppHandle, profile: &mut Profile) -> bool {
    let caps = crate::commands::engine::current_inject_caps();
    let report = qzh_profile::sanitize_profile(profile, caps);
    if report.is_empty() {
        return false;
    }
    warn!(
        "配置含当前版本不支持的按键：清空热键 {:?}，停用规则 {:?}",
        report.cleared_hotkeys, report.disabled_rules
    );
    if let Some(state) = app.try_state::<ProfileNotice>() {
        if let Ok(mut slot) = state.0.lock() {
            *slot = Some(report);
        }
    }
    true
}

/// 取走待提醒的净化结果。前端挂载时调用一次；取走即清空，不重复弹窗。
#[tauri::command]
pub fn take_profile_notice(state: State<ProfileNotice>) -> Option<qzh_profile::SanitizeReport> {
    state.0.lock().ok().and_then(|mut slot| slot.take())
}

/// 激活一份已在 profiles 目录中的配置文件：读取（解密 / 迁移 / 钳位 / 校验）→ 净化 →
/// 装载引擎 → 更新 `activeProfilePath`。托盘请求交给面板保存后再调用 [`load_profile`]。
pub(crate) fn activate_profile_file(
    app: &AppHandle,
    engine: &crate::engine::BurstEngine,
    path: &Path,
) -> Result<Profile, String> {
    let mut profile = read_profile_from_file(path)?;
    // 配置文件可能来自他人分享、旧版本或手工编辑，完全绕过前端的录入拦截，
    // 不支持的按键必须在装载进引擎之前处理掉。
    if sanitize_and_record(app, &mut profile) {
        let _ = app.emit("profile-sanitized", ());
    }
    engine.set_rules(profile.rules.clone());
    engine.set_hotkeys(profile.hotkeys.clone());
    set_active_path(app, &path.to_string_lossy());
    Ok(profile)
}

#[tauri::command]
pub fn load_profile(
    app: AppHandle,
    path: String,
    state: State<EngineState>,
) -> Result<Profile, String> {
    // 只允许加载 profiles_dir 下的文件，防止路径遍历
    let dir = profiles_dir(&app)?;
    let file_name = Path::new(&path)
        .file_name()
        .ok_or("无效文件路径")?
        .to_string_lossy();
    let safe_path = dir.join(sanitize_filename(&file_name));

    let profile = activate_profile_file(&app, &state.0, &safe_path)?;
    crate::tray::refresh_menu(&app);
    Ok(profile)
}

#[tauri::command]
pub fn list_profiles(app: AppHandle) -> Result<Vec<ProfileEntry>, String> {
    let dir = profiles_dir(&app)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    let read = std::fs::read_dir(&dir).map_err(|e| format!("无法读取配置目录: {e}"))?;
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "qzh") {
            continue;
        }
        match read_profile_from_file(&path) {
            Ok(profile) => {
                let summary = profile_summary(&profile);
                entries.push(ProfileEntry {
                    meta: profile.meta,
                    path: path.to_string_lossy().to_string(),
                    summary,
                });
            }
            Err(e) => {
                warn!("跳过无法读取的配置文件 {}: {}", path.display(), e);
            }
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.meta.updated_at));
    Ok(entries)
}

#[derive(serde::Serialize)]
pub struct ProfileEntry {
    pub meta: ProfileMeta,
    pub path: String,
    pub summary: ProfileSummary,
}

#[derive(serde::Serialize)]
pub struct ProfileSummary {
    pub rules_total: usize,
    pub rules_enabled: usize,
    pub hold_count: usize,
    pub toggle_count: usize,
    pub group_count: usize,
    pub global_toggle: Option<KeyId>,
    pub global_stop: Option<KeyId>,
    pub panel_toggle: Option<KeyId>,
}

fn profile_summary(profile: &Profile) -> ProfileSummary {
    let mut rules_enabled = 0;
    let mut hold_count = 0;
    let mut toggle_count = 0;
    for rule in &profile.rules {
        if rule.enabled {
            rules_enabled += 1;
        }
        match &rule.mode {
            BurstMode::Hold => hold_count += 1,
            BurstMode::Toggle => toggle_count += 1,
        }
    }
    let group_count = profile
        .rules
        .iter()
        .filter_map(|r| r.group.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .len();

    ProfileSummary {
        rules_total: profile.rules.len(),
        rules_enabled,
        hold_count,
        toggle_count,
        group_count,
        global_toggle: profile.hotkeys.global_toggle,
        global_stop: profile.hotkeys.global_stop,
        panel_toggle: profile.hotkeys.panel_toggle,
    }
}

/// 出厂默认规则（不含 id，调用方负责注入）。抽出此函数避免与
/// [`create_default_profile`] 漂移。
fn factory_default_rules() -> Vec<BurstRule> {
    vec![
        BurstRule {
            id: make_id(),
            enabled: false,
            trigger_key: KeyId::Keyboard(0x51), // Q
            target_key: KeyId::Keyboard(0x51),
            mode: BurstMode::Hold,
            stop_key: None,
            interval_ms: DEFAULT_INTERVAL_MS,
            group: None,
        },
        BurstRule {
            id: make_id(),
            enabled: false,
            trigger_key: KeyId::Keyboard(0x46), // F
            target_key: KeyId::Keyboard(0x46),
            mode: BurstMode::Toggle,
            stop_key: None,
            interval_ms: DEFAULT_INTERVAL_MS,
            group: None,
        },
    ]
}

/// 创建并落盘默认配置。供 Tauri command (`init_default_profile`) 与启动时
/// 兜底初始化（`lib.rs` 中加载失败/首启动）共享，避免双份 Profile 字面量与加密落盘代码漂移。
pub(crate) fn create_default_profile(
    app: &AppHandle,
    engine: &crate::engine::BurstEngine,
) -> Result<Profile, String> {
    let now = now_secs();

    let profile = Profile {
        schema_version: CURRENT_SCHEMA_VERSION,
        meta: ProfileMeta {
            name: DEFAULT_PROFILE_NAME.to_string(),
            created_at: now,
            updated_at: now,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            signature: String::new(),
        },
        rules: factory_default_rules(),
        hotkeys: Hotkeys::default(),
        advanced: Advanced::default(),
    };

    let dir = profiles_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录: {e}"))?;
    let file_path = dir.join(format!("{DEFAULT_PROFILE_NAME}.qzh"));

    let rules = profile.rules.clone();
    let hotkeys = profile.hotkeys.clone();
    let path = write_profile_file_to_path(&file_path, &profile)?;
    engine.set_rules(rules);
    engine.set_hotkeys(hotkeys);

    if let Ok(store) = app.store(crate::STORE_PATH) {
        store.set(ACTIVE_PATH_KEY, serde_json::json!(path));
        let _ = store.save();
    }
    crate::tray::refresh_menu(app);

    Ok(profile)
}

#[tauri::command]
pub fn init_default_profile(app: AppHandle, state: State<EngineState>) -> Result<Profile, String> {
    create_default_profile(&app, &state.0)
}

/// 一次性迁移：把历史拼写错误的默认配置 `defults.qzh` 更正为 `defaults.qzh`
/// （同步 meta.name 与 store 的 activeProfilePath）。仅在旧文件存在、且新文件尚不存在时执行，
/// 已迁移或全新安装均为 no-op。任何步骤失败只记日志、不阻断启动（最坏情况退回到重建默认配置）。
pub(crate) fn migrate_legacy_default_profile(app: &AppHandle) {
    let Ok(dir) = profiles_dir(app) else {
        return;
    };
    let legacy = dir.join(format!("{LEGACY_DEFAULT_PROFILE_NAME}.qzh"));
    let renamed = dir.join(format!("{DEFAULT_PROFILE_NAME}.qzh"));
    if !legacy.exists() || renamed.exists() {
        return;
    }

    let mut profile = match read_profile_from_file(&legacy) {
        Ok(p) => p,
        Err(e) => {
            warn!("旧默认配置迁移跳过（无法读取 {}）：{}", legacy.display(), e);
            return;
        }
    };
    profile.meta.name = DEFAULT_PROFILE_NAME.to_string();
    profile.meta.updated_at = now_secs();
    profile.meta.app_version = env!("CARGO_PKG_VERSION").to_string();

    let saved = match write_profile_file_to_path(&renamed, &profile) {
        Ok(p) => p,
        Err(e) => {
            warn!("旧默认配置迁移失败（写入 {}）：{}", renamed.display(), e);
            return;
        }
    };
    if let Err(e) = std::fs::remove_file(&legacy) {
        warn!("旧默认配置迁移：删除旧文件失败 {}：{}", legacy.display(), e);
    }

    // 若当前激活配置正指向旧文件，同步到新路径
    let active_was_legacy = app
        .store(crate::STORE_PATH)
        .ok()
        .and_then(|s| {
            s.get(ACTIVE_PATH_KEY)
                .and_then(|v| v.as_str().map(String::from))
        })
        .map(|p| Path::new(&p) == legacy)
        .unwrap_or(false);
    if active_was_legacy {
        set_active_path(app, &saved);
    }
    info!("已将旧默认配置 defults 迁移为 defaults");
}

#[tauri::command]
pub fn get_active_profile_path(app: AppHandle) -> Result<Option<String>, String> {
    let store = app
        .store(crate::STORE_PATH)
        .map_err(|e| format!("无法读取存储: {e}"))?;
    Ok(store
        .get(ACTIVE_PATH_KEY)
        .and_then(|v| v.as_str().map(|s| s.to_string())))
}

/// 重命名一个配置文件：sanitize 新名 → 改 meta.name 重写新文件 → 删除旧文件 →
/// 如改的是当前激活配置则同步 `activeProfilePath`。返回新文件绝对路径。
#[tauri::command]
pub fn rename_profile(
    app: AppHandle,
    old_name: String,
    new_name: String,
) -> Result<String, String> {
    if old_name == DEFAULT_PROFILE_NAME {
        return Err("默认配置不可重命名".into());
    }
    let trimmed = new_name.trim();
    if trimmed.is_empty() {
        return Err("配置名不能为空".into());
    }
    if trimmed == DEFAULT_PROFILE_NAME {
        return Err("不能使用默认配置名".into());
    }
    let dir = profiles_dir(&app)?;
    let old_path = profile_path_for_name(&dir, &old_name);
    let new_path = profile_path_for_name(&dir, trimmed);

    if !old_path.exists() {
        return Err(format!("配置不存在：{old_name}"));
    }
    if old_path != new_path && new_path.exists() {
        return Err(format!("已存在同名配置：{trimmed}"));
    }

    let mut profile = read_profile_from_file(&old_path)?;
    profile.meta.name = trimmed.to_string();
    profile.meta.updated_at = now_secs();
    profile.meta.app_version = env!("CARGO_PKG_VERSION").to_string();

    let saved_path = write_profile_file_to_path(&new_path, &profile)?;
    if old_path != new_path {
        std::fs::remove_file(&old_path).map_err(|e| format!("删除旧文件失败: {e}"))?;
    }

    // 如果改的是当前激活配置，同步 store
    let was_active = app
        .store(crate::STORE_PATH)
        .ok()
        .and_then(|s| {
            s.get(ACTIVE_PATH_KEY)
                .and_then(|v| v.as_str().map(String::from))
        })
        .map(|p| Path::new(&p) == old_path)
        .unwrap_or(false);
    if was_active {
        set_active_path(&app, &saved_path);
    }
    crate::tray::refresh_menu(&app);
    Ok(saved_path)
}

/// 删除指定配置文件；如删的是当前激活配置，回退到默认配置（必要时重新创建）。
/// 返回删除后引擎应当装载的 [`Profile`]（仅当删的是激活配置时返回 Some，
/// 前端据此刷新 UI；否则返回 None）。
#[tauri::command]
pub fn delete_profile(
    app: AppHandle,
    state: State<EngineState>,
    name: String,
) -> Result<Option<Profile>, String> {
    if name == DEFAULT_PROFILE_NAME {
        return Err("默认配置不可删除".into());
    }
    let dir = profiles_dir(&app)?;
    let target_path = profile_path_for_name(&dir, &name);
    if !target_path.exists() {
        return Err(format!("配置不存在：{name}"));
    }

    let active_path = app.store(crate::STORE_PATH).ok().and_then(|s| {
        s.get(ACTIVE_PATH_KEY)
            .and_then(|v| v.as_str().map(String::from))
    });
    let was_active = active_path
        .as_deref()
        .map(|p| Path::new(p) == target_path)
        .unwrap_or(false);

    std::fs::remove_file(&target_path).map_err(|e| format!("删除配置失败: {e}"))?;

    if !was_active {
        crate::tray::refresh_menu(&app);
        return Ok(None);
    }

    // 删的是激活配置：加载默认配置；若不存在或损坏则重建
    let default_path = dir.join(format!("{DEFAULT_PROFILE_NAME}.qzh"));
    let profile = match activate_profile_file(&app, &state.0, &default_path) {
        Ok(p) => p,
        Err(e) => {
            if default_path.exists() {
                warn!("默认配置损坏将重建: {}", e);
            }
            // create_default_profile 内部会 set_rules + set_active_path
            create_default_profile(&app, &state.0)?
        }
    };
    crate::tray::refresh_menu(&app);
    Ok(Some(profile))
}

/// 基于当前激活配置 fork 出一份副本：选一个不冲突的名字，落盘新文件，
/// 把 `activeProfilePath` 切到新文件。返回新 [`Profile`] 与新路径。
/// 用于「修改默认配置时自动新建一份」的 fork-on-edit 流程。
#[tauri::command]
pub fn fork_active_profile(app: AppHandle, suggested_name: String) -> Result<ForkResult, String> {
    let dir = profiles_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录: {e}"))?;

    let active_path_str = app
        .store(crate::STORE_PATH)
        .ok()
        .and_then(|s| {
            s.get(ACTIVE_PATH_KEY)
                .and_then(|v| v.as_str().map(String::from))
        })
        .ok_or("当前没有激活配置")?;
    let active_path = Path::new(&active_path_str);
    let mut profile = read_profile_from_file(active_path)?;

    let base = {
        let trimmed = suggested_name.trim();
        if trimmed.is_empty() || trimmed == DEFAULT_PROFILE_NAME {
            "我的配置"
        } else {
            trimmed
        }
    };
    let (final_name, final_path) = pick_unique_name(&dir, base);

    let now = now_secs();
    profile.meta.name = final_name.clone();
    profile.meta.created_at = now;
    profile.meta.updated_at = now;
    profile.meta.app_version = env!("CARGO_PKG_VERSION").to_string();
    profile.schema_version = CURRENT_SCHEMA_VERSION;

    let saved_path = write_profile_file_to_path(&final_path, &profile)?;
    set_active_path(&app, &saved_path);
    crate::tray::refresh_menu(&app);

    Ok(ForkResult {
        profile,
        path: saved_path,
    })
}

#[derive(serde::Serialize)]
pub struct ForkResult {
    pub profile: Profile,
    pub path: String,
}

/// 读取外部 .qzh 文件的大小上限（4 MB），与外部配置导入一致，防止大文件导致 OOM。
const QZH_IMPORT_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 导出指定配置为 .qzh 文件（跨设备迁移 / 社区分享）。
/// 读取即完成解密 / 迁移 / 钳位 / 校验，导出内容始终是当前 schema 的规范化副本。
#[tauri::command]
pub fn export_profile(app: AppHandle, name: String, dest_path: String) -> Result<String, String> {
    let dir = profiles_dir(&app)?;
    let src = profile_path_for_name(&dir, &name);
    if !src.exists() {
        return Err(format!("配置不存在：{name}"));
    }
    let mut profile = read_profile_from_file(&src)?;
    profile.schema_version = CURRENT_SCHEMA_VERSION;

    let dest = ensure_qzh_extension(PathBuf::from(dest_path));
    let saved = write_profile_file_to_path(&dest, &profile)?;
    info!("已导出配置「{}」到 {}", name, dest.display());
    Ok(saved)
}

/// 导入外部 .qzh 配置文件：完整读取校验后，以不冲突的名字复制进 profiles 目录，
/// 并立即切换为活跃配置。
#[tauri::command]
pub fn import_qzh_profile(
    app: AppHandle,
    state: State<EngineState>,
    path: String,
) -> Result<Profile, String> {
    let src = Path::new(&path);
    let profile = read_imported_qzh(src)?;
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    let base = pick_import_base_name(&profile.meta.name, &stem);
    save_imported_profile(&app, &state.0, profile, &base)
}

pub(crate) fn read_imported_qzh(src: &Path) -> Result<Profile, String> {
    let size = src
        .metadata()
        .map_err(|e| format!("读取文件信息失败：{e}"))?
        .len();
    if size > QZH_IMPORT_MAX_BYTES {
        return Err(format!("文件过大（{size} 字节），超过 4 MB 上限"));
    }
    read_profile_from_file(src)
}

pub(crate) fn save_imported_profile(
    app: &AppHandle,
    engine: &crate::engine::BurstEngine,
    mut profile: Profile,
    base: &str,
) -> Result<Profile, String> {
    let dir = profiles_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录: {e}"))?;
    let (final_name, final_path) = pick_unique_name(&dir, base);

    let now = now_secs();
    profile.meta.name = final_name;
    profile.meta.created_at = now;
    profile.meta.updated_at = now;
    profile.meta.app_version = env!("CARGO_PKG_VERSION").to_string();
    profile.schema_version = CURRENT_SCHEMA_VERSION;

    write_profile_file_to_path(&final_path, &profile)?;
    // 落盘后一律经 activate_profile_file 激活，别在这里自己 set_rules：
    // 分享来的 .qzh 完全绕过前端录入拦截，净化必须和启动、托盘切换走同一条路。
    let profile = activate_profile_file(app, engine, &final_path)?;
    crate::tray::refresh_menu(app);
    info!("已导入配置「{}」", profile.meta.name);
    Ok(profile)
}

/// 补全 .qzh 扩展名：追加而非替换，避免「我的.配置」被改写成「我的.qzh」。
fn ensure_qzh_extension(mut path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("qzh"))
    {
        path.as_mut_os_string().push(".qzh");
    }
    path
}

/// 选择导入配置的基础名：优先取文件内 meta.name；为空或撞上受保护的默认名时
/// 退回文件名（stem），再兜底「导入配置」。
pub(crate) fn pick_import_base_name(meta_name: &str, stem: &str) -> String {
    let name = meta_name.trim();
    if !name.is_empty() && name != DEFAULT_PROFILE_NAME {
        return name.to_string();
    }
    let stem = stem.trim();
    if !stem.is_empty() && stem != DEFAULT_PROFILE_NAME {
        return stem.to_string();
    }
    "导入配置".to_string()
}

/// 托盘菜单用的轻量配置清单：仅解出 meta（不迁移、不校验），返回（名称, 路径）。
/// 排序与面板配置菜单一致：默认配置最前，其余按创建时间、名称。
pub(crate) fn list_profiles_brief(app: &AppHandle) -> Vec<(String, PathBuf)> {
    let Ok(dir) = profiles_dir(app) else {
        return Vec::new();
    };
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut items: Vec<(ProfileMeta, PathBuf)> = read
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "qzh") {
                return None;
            }
            load_meta_from_file(&path).map(|meta| (meta, path))
        })
        .collect();
    items.sort_by(|a, b| {
        let a_default = a.0.name == DEFAULT_PROFILE_NAME;
        let b_default = b.0.name == DEFAULT_PROFILE_NAME;
        b_default
            .cmp(&a_default)
            .then(a.0.created_at.cmp(&b.0.created_at))
            .then_with(|| a.0.name.cmp(&b.0.name))
    });
    items.into_iter().map(|(m, p)| (m.name, p)).collect()
}

/// 在 `dir` 下挑一个不冲突的文件名：base、base 2、base 3 ...
pub(crate) fn pick_unique_name(dir: &Path, base: &str) -> (String, PathBuf) {
    let first = profile_path_for_name(dir, base);
    if !first.exists() {
        return (base.to_string(), first);
    }
    for i in 2.. {
        let candidate = format!("{base} {i}");
        let path = profile_path_for_name(dir, &candidate);
        if !path.exists() {
            return (candidate, path);
        }
    }
    unreachable!()
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_ascii_control() => '_',
            _ => c,
        })
        .collect()
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
