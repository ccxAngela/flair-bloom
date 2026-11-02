//! 配置扫描、预览与导入。.qzh 直通既有读取逻辑，其余格式由解析器转换。
//!
//! 扩展方式：在 [`PARSERS`] 中追加新的 [`ParserDef`] 条目即可支持新格式。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Manager, State};

use qzh_profile::{
    Advanced, BurstMode, BurstRule, Hotkeys, KeyId, MouseButton, Profile, ProfileMeta,
    CURRENT_SCHEMA_VERSION, MAX_INTERVAL_MS, MAX_RULES, MIN_EFFECTIVE_INTERVAL_MS,
};

use super::engine::EngineState;
use super::profile::{now_secs, profiles_dir, save_imported_profile};

static IMPORT_SEQ: AtomicU64 = AtomicU64::new(0);

mod celestial;
mod jijian;
mod qzh;
mod shortcuts;
mod xiaoji;

fn make_id() -> String {
    let n = IMPORT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{:016x}-imp{n}", now_secs())
}

// ─── 公共结果类型 ─────────────────────────────────────────────────────────────

/// 扫描到的一份配置。
#[derive(Debug, Serialize)]
pub struct FoundConfig {
    /// 文件绝对路径。
    pub path: String,
    /// 来源软件名称，如「丐帮高手」。
    pub source_app: String,
    /// 用于列表展示的简短说明（含文件所在目录名）。
    pub display_name: String,
    /// 多档位格式的来源档位，普通单配置为 None。
    pub source_profile_index: Option<usize>,
}

/// 解析后的导入预览，不含完整规则列表，用于在 UI 做确认展示。
#[derive(Debug, Serialize)]
pub struct ImportPreview {
    /// 来源软件名称。
    pub source_app: String,
    /// 建议的配置文件名（取自所在目录名，可由用户修改）。
    pub suggested_name: String,
    /// 将会生成的规则条数（已截断到 MAX_RULES）。
    pub rule_count: usize,
    /// 因超出上限而丢弃的规则条数。
    pub skipped_count: usize,
    /// 连发间隔（ms）。
    pub interval_ms: u32,
    pub interval_max_ms: u32,
    pub warnings: Vec<String>,
    /// 检测到的全局开启键。
    pub global_toggle: Option<KeyId>,
    /// 检测到的全局停止键（与开启键相同时为 None）。
    pub global_stop: Option<KeyId>,
    /// 检测到的面板显隐键。
    pub panel_toggle: Option<KeyId>,
}

// ─── 解析器定义（可扩展注册表）────────────────────────────────────────────────

type ProfileVariants = fn(&str) -> Vec<(usize, String)>;
type ProfileSelector = fn(&str, usize) -> Result<String, String>;

struct ParserDef {
    /// 该格式的配置文件名（大小写不敏感匹配）。
    config_filenames: &'static [&'static str],
    /// 来源软件名称。
    source_app: &'static str,
    /// 快速检测：返回 true 则认为该文件属于此格式。
    detect: fn(data: &str) -> bool,
    /// 解析为预览信息。
    parse_preview: fn(data: &str, dir_name: &str) -> ImportPreview,
    /// 解析为完整规则列表（用于实际导入）。
    parse_rules: fn(data: &str) -> Vec<BurstRule>,
    /// 提取全局开关键。
    parse_hotkeys: fn(data: &str) -> Hotkeys,
    profiles: Option<ProfileVariants>,
    select_profile: Option<ProfileSelector>,
}

/// 解析器注册表。新增格式支持：在此追加一个 `ParserDef`。
static PARSERS: &[ParserDef] = &[
    ParserDef {
        config_filenames: &["config.json"],
        source_app: "丐帮高手",
        detect: gaibang_detect,
        parse_preview: gaibang_parse_preview,
        parse_rules: gaibang_parse_rules,
        parse_hotkeys: gaibang_parse_hotkeys,
        profiles: None,
        select_profile: None,
    },
    ParserDef {
        config_filenames: &["config.json"],
        source_app: celestial::SOURCE,
        detect: celestial::detect,
        parse_preview: celestial::parse_preview,
        parse_rules: celestial::parse_rules,
        parse_hotkeys: celestial::parse_hotkeys,
        profiles: Some(celestial::profiles),
        select_profile: Some(celestial::select_profile),
    },
    ParserDef {
        config_filenames: &["config.ini"],
        source_app: "极简按键",
        detect: jijian::detect,
        parse_preview: jijian::parse_preview,
        parse_rules: jijian::parse_rules,
        parse_hotkeys: jijian::parse_hotkeys,
        profiles: None,
        select_profile: None,
    },
    ParserDef {
        config_filenames: &["config.json"],
        source_app: "小叽按键",
        detect: xiaoji::detect,
        parse_preview: xiaoji::parse_preview,
        parse_rules: xiaoji::parse_rules,
        parse_hotkeys: xiaoji::parse_hotkeys,
        profiles: Some(xiaoji::profiles),
        select_profile: Some(xiaoji::select_profile),
    },
];

// ─── 丐帮高手解析器 ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct GaibangConfig {
    #[serde(default = "default_delay_us")]
    delay_us: u64,
    #[serde(default)]
    exclude: Vec<u32>,
    #[serde(default)]
    hk_toggle: u32,
    #[serde(default)]
    hk_stop: u32,
    #[serde(default)]
    hk_game: u32,
}

fn default_delay_us() -> u64 {
    50_000
}

/// 将 Win32 VK 码映射为 KeyId；鼠标按钮单独映射，其余均视为键盘键。
fn vk_to_key_id(vk: u32) -> KeyId {
    match vk {
        1 => KeyId::Mouse(MouseButton::Left),
        2 => KeyId::Mouse(MouseButton::Right),
        4 => KeyId::Mouse(MouseButton::Middle),
        5 => KeyId::Mouse(MouseButton::X1),
        6 => KeyId::Mouse(MouseButton::X2),
        k => KeyId::Keyboard(k),
    }
}

/// 「可连发」VK 码范围：字符键、功能键、数字键盘、OEM 标点。
fn burstable_vks() -> impl Iterator<Item = u32> {
    (1u32..=2) // 鼠标左/右键
        .chain(4..=6) // 鼠标中/侧键
        .chain(0x30..=0x39) // 数字行 0-9
        .chain(0x41..=0x5A) // 字母 A-Z
        .chain(0x60..=0x6B) // 数字键盘 0-9, *, +
        .chain(0x6D..=0x6F) // 数字键盘 -, ., /（跳过 0x6C VK_SEPARATOR，非标准键）
        .chain(0x70..=0x7B) // F1-F12
        .chain(0xBA..=0xC0) // OEM 标点 ;=,-./ `
        .chain(0xDB..=0xDE) // OEM 标点 [{ \| ]} '"
}

fn gaibang_detect(data: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(data)
        .map(|v| v.get("exclude").is_some() && v.get("delay_us").is_some())
        .unwrap_or(false)
}

fn parse_gaibang_cfg(data: &str) -> Option<GaibangConfig> {
    serde_json::from_str(data).ok()
}

fn gaibang_active_vks(cfg: &GaibangConfig) -> Vec<u32> {
    let mut excluded: HashSet<u32> = cfg.exclude.iter().copied().collect();
    for hk in [cfg.hk_toggle, cfg.hk_stop, cfg.hk_game] {
        if hk != 0 {
            excluded.insert(hk);
        }
    }
    burstable_vks().filter(|k| !excluded.contains(k)).collect()
}

fn gaibang_interval(cfg: &GaibangConfig) -> u32 {
    // 钳到有效操作下限（10ms）而非结构下限（1ms）：管线可持续注入上限，
    // 更快会在下游输入队列堆积、停止后才排空。预览与实际导入一致。
    ((cfg.delay_us / 1000) as u32).clamp(MIN_EFFECTIVE_INTERVAL_MS, MAX_INTERVAL_MS)
}

fn gaibang_parse_preview(data: &str, dir_name: &str) -> ImportPreview {
    let cfg = parse_gaibang_cfg(data).unwrap_or(GaibangConfig {
        delay_us: default_delay_us(),
        exclude: vec![],
        hk_toggle: 0,
        hk_stop: 0,
        hk_game: 0,
    });
    let active = gaibang_active_vks(&cfg);
    let rule_count = active.len().min(MAX_RULES);
    let skipped_count = active.len().saturating_sub(MAX_RULES);
    let interval_ms = gaibang_interval(&cfg);
    let global_toggle = (cfg.hk_toggle != 0).then(|| vk_to_key_id(cfg.hk_toggle));
    let global_stop =
        (cfg.hk_stop != 0 && cfg.hk_stop != cfg.hk_toggle).then(|| vk_to_key_id(cfg.hk_stop));
    let panel_toggle = (cfg.hk_game != 0).then(|| vk_to_key_id(cfg.hk_game));
    ImportPreview {
        source_app: "丐帮高手".to_string(),
        suggested_name: dir_name.to_string(),
        rule_count,
        skipped_count,
        interval_ms,
        interval_max_ms: interval_ms,
        warnings: Vec::new(),
        global_toggle,
        global_stop,
        panel_toggle,
    }
}

fn gaibang_parse_rules(data: &str) -> Vec<BurstRule> {
    let Some(cfg) = parse_gaibang_cfg(data) else {
        return vec![];
    };
    let interval_ms = gaibang_interval(&cfg);
    gaibang_active_vks(&cfg)
        .into_iter()
        .take(MAX_RULES)
        .map(|vk| BurstRule {
            id: make_id(),
            enabled: true,
            trigger_key: vk_to_key_id(vk),
            steps: vec![qzh_profile::profile::OutputStep {
                key: vk_to_key_id(vk),
                interval_ms,
            }],
            mode: BurstMode::Hold,
            stop_key: None,
            group: None,
        })
        .collect()
}

fn gaibang_parse_hotkeys(data: &str) -> Hotkeys {
    let Some(cfg) = parse_gaibang_cfg(data) else {
        return Hotkeys::default();
    };
    let global_toggle = (cfg.hk_toggle != 0).then(|| vk_to_key_id(cfg.hk_toggle));
    let global_stop =
        (cfg.hk_stop != 0 && cfg.hk_stop != cfg.hk_toggle).then(|| vk_to_key_id(cfg.hk_stop));
    let panel_toggle = (cfg.hk_game != 0).then(|| vk_to_key_id(cfg.hk_game));
    Hotkeys {
        global_toggle,
        global_stop,
        panel_toggle,
    }
}

// ─── 工具函数 ─────────────────────────────────────────────────────────────────

fn read_import(
    path: &Path,
    source_profile_index: Option<usize>,
) -> Result<(&'static ParserDef, String), String> {
    let filename = path
        .file_name()
        .ok_or("文件名无效")?
        .to_string_lossy()
        .to_lowercase();
    if !PARSERS
        .iter()
        .any(|p| p.config_filenames.contains(&filename.as_str()))
    {
        return Err("不是支持的配置文件名".into());
    }
    let size = path
        .metadata()
        .map_err(|e| format!("读取文件信息失败：{e}"))?
        .len();
    if size > IMPORT_MAX_BYTES {
        return Err(format!("文件过大（{size} 字节），超过 4 MB 上限"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败：{e}"))?;
    if bytes.len() as u64 > IMPORT_MAX_BYTES {
        return Err("文件超过 4 MB 上限".into());
    }
    let raw = if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return Err("UTF-16 配置长度无效".into());
        }
        let wide: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&wide).map_err(|e| format!("配置编码无效：{e}"))?
    } else if filename.ends_with(".ini") {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        String::from_utf8(bytes).map_err(|e| format!("配置不是 UTF-8：{e}"))?
    };
    let data = raw.trim_start_matches('\u{feff}');
    let parser = PARSERS
        .iter()
        .find(|p| p.config_filenames.contains(&filename.as_str()) && (p.detect)(data))
        .ok_or("无法识别的配置格式，目前支持：丐帮高手、衍天高手、极简按键、小叽按键")?;
    let data = if let Some(index) = source_profile_index {
        parser.select_profile.ok_or("该配置格式不支持方案档位")?(data, index)?
    } else {
        data.to_string()
    };
    Ok((parser, data))
}

/// 扫描目录及一级子目录中的已知配置文件。
fn scan_dir(dir: &Path) -> Vec<FoundConfig> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // 直接在该目录下的文件
        if path.is_file() {
            found.extend(to_found_configs(&path));
            for config_dir in shortcuts::config_dirs(&path) {
                if let Ok(config_files) = std::fs::read_dir(config_dir) {
                    for file in config_files.flatten().filter(|f| f.path().is_file()) {
                        found.extend(to_found_configs(&file.path()));
                    }
                }
            }
        }
        // 进入一级子目录
        if path.is_dir() {
            let Ok(sub) = std::fs::read_dir(&path) else {
                continue;
            };
            for sub_entry in sub.flatten() {
                let sub_path = sub_entry.path();
                if sub_path.is_file() {
                    found.extend(to_found_configs(&sub_path));
                }
            }
        }
    }
    found
}

fn to_found_configs(path: &Path) -> Vec<FoundConfig> {
    if qzh::matches(path) {
        return qzh::preview(path)
            .map(|preview| {
                vec![FoundConfig {
                    path: path.to_string_lossy().into_owned(),
                    source_app: preview.source_app,
                    display_name: preview.suggested_name,
                    source_profile_index: None,
                }]
            })
            .unwrap_or_default();
    }
    let Ok((parser, data)) = read_import(path, None) else {
        return Vec::new();
    };
    let dir_name = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if let Some(profiles) = parser.profiles {
        let variants = profiles(&data);
        if !variants.is_empty() {
            return variants
                .into_iter()
                .map(|(index, name)| FoundConfig {
                    path: path.to_string_lossy().into_owned(),
                    source_app: parser.source_app.into(),
                    display_name: name,
                    source_profile_index: Some(index),
                })
                .collect();
        }
    }
    vec![FoundConfig {
        path: path.to_string_lossy().to_string(),
        source_app: parser.source_app.to_string(),
        display_name: if dir_name.is_empty() {
            path.to_string_lossy().to_string()
        } else {
            dir_name.to_string()
        },
        source_profile_index: None,
    }]
}

/// 默认扫描目录：桌面 + 下载（跨平台）。
fn default_scan_dirs(app: &AppHandle) -> Vec<PathBuf> {
    // 使用系统已知目录，覆盖 OneDrive 与用户重定向的桌面／下载。
    let dirs: Vec<PathBuf> = [app.path().desktop_dir(), app.path().download_dir()]
        .into_iter()
        .flatten()
        .collect();

    #[cfg(windows)]
    let dirs = {
        let mut dirs = dirs;
        if let Ok(appdata) = app.path().config_dir() {
            dirs.push(appdata.join("TheCelestialDiviner"));
        }
        if let Some(public) = std::env::var_os("PUBLIC") {
            dirs.push(PathBuf::from(public).join("Desktop"));
        }
        dirs
    };

    dirs
}

// ─── Tauri 命令 ───────────────────────────────────────────────────────────────

/// 扫描指定目录（为空则使用默认位置），排除已在管理目录中的配置。
#[tauri::command]
pub fn scan_import_configs(app: AppHandle, dirs: Vec<String>) -> Vec<FoundConfig> {
    let targets: Vec<PathBuf> = if dirs.is_empty() {
        default_scan_dirs(&app)
    } else {
        dirs.into_iter().map(PathBuf::from).collect()
    };
    let managed_dir = profiles_dir(&app).ok();
    scan_targets(targets, managed_dir.as_deref())
}

fn scan_targets(targets: Vec<PathBuf>, managed_dir: Option<&Path>) -> Vec<FoundConfig> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let managed_dir = managed_dir.and_then(|dir| std::fs::canonicalize(dir).ok());
    for dir in targets {
        for item in scan_dir(&dir) {
            let path =
                std::fs::canonicalize(&item.path).unwrap_or_else(|_| PathBuf::from(&item.path));
            if managed_dir
                .as_ref()
                .is_some_and(|dir| path.starts_with(dir))
            {
                continue;
            }
            let key = (path, item.source_profile_index);
            if seen.insert(key) {
                result.push(item);
            }
        }
    }
    result
}

/// 导入文件大小上限（4 MB），防止大文件导致 OOM。
const IMPORT_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 解析指定文件，返回导入预览。路径可来自 [`scan_import_configs`] 或用户手动输入。
#[tauri::command]
pub fn preview_import(
    path: String,
    source_profile_index: Option<usize>,
) -> Result<ImportPreview, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在：{path}"));
    }
    if qzh::matches(p) {
        return qzh::preview(p);
    }
    let (parser, data) = read_import(p, source_profile_index)?;
    let dir_name = p
        .parent()
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty() && n != "." && n != "/" && n != "\\")
        .unwrap_or_else(|| "导入配置".to_string());
    Ok((parser.parse_preview)(&data, &dir_name))
}

/// 导入为新的配置文件，立即切换为活跃配置；.qzh 保留整套配置内容。
#[tauri::command]
pub fn import_external_config(
    app: AppHandle,
    state: State<EngineState>,
    path: String,
    profile_name: String,
    source_profile_index: Option<usize>,
) -> Result<Profile, String> {
    let trimmed = profile_name.trim();
    if trimmed.is_empty() {
        return Err("配置名不能为空".into());
    }

    let p = Path::new(&path);
    let profile = if qzh::matches(p) {
        qzh::read(p)?
    } else {
        convert_profile(p, source_profile_index)?
    };
    save_imported_profile(&app, &state.0, profile, trimmed)
}

fn convert_profile(p: &Path, source_profile_index: Option<usize>) -> Result<Profile, String> {
    let (parser, data) = read_import(p, source_profile_index)?;

    let rules = (parser.parse_rules)(&data);
    if rules.is_empty() {
        return Err("没有可导入的规则，请检查预览中的跳过说明。".into());
    }
    if rules.len() > MAX_RULES {
        return Err(format!("规则数量 {} 超过上限 {MAX_RULES}", rules.len()));
    }
    for (i, rule) in rules.iter().enumerate() {
        rule.validate()
            .map_err(|error| format!("第 {} 条规则：{error}", i + 1))?;
    }

    let hotkeys = (parser.parse_hotkeys)(&data);
    let now = now_secs();
    let mut profile = Profile {
        schema_version: CURRENT_SCHEMA_VERSION,
        meta: ProfileMeta {
            name: String::new(),
            created_at: now,
            updated_at: now,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            signature: String::new(),
        },
        rules,
        hotkeys,
        advanced: Advanced::default(),
    };
    // 兜底适配有效操作下限：任何解析器产出的 <10ms 间隔在落盘前钳到 10ms，
    // 与加载旧配置（read_profile_from_file）行为一致。
    profile.clamp_intervals();

    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::super::profile::write_profile_file_to_path;
    use super::*;
    use serde_json::json;

    #[test]
    fn scanner_finds_each_source_profile_once_and_preview_selects_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let data = json!({"Version":20,"GlobalSwitch":{},"ActiveProfile":0,"Profiles":[
            {"K:65:0":{"Targets":[{"VirtualKey":66,"Mode":"Hold"}]}},
            {"K:67:0":{"Targets":[{"VirtualKey":68,"Mode":"Toggle"}]}}
        ],"ProfileNames":["长按","挂机"]});
        std::fs::write(&path, format!("\u{feff}{data}")).unwrap();
        let found = scan_targets(vec![dir.path().to_path_buf(); 2], None);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].source_profile_index, Some(1));
        let preview = preview_import(path.display().to_string(), Some(1)).unwrap();
        assert_eq!(preview.source_app, celestial::SOURCE);
        assert_eq!(preview.rule_count, 1);
        let (_, selected) = read_import(&path, Some(1)).unwrap();
        assert_eq!(
            celestial::parse_rules(&selected)[0].trigger_key,
            KeyId::Keyboard(67)
        );
    }

    #[test]
    fn gaibang_parser_still_handles_its_signature_without_matching_celestial() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CONFIG.JSON");
        std::fs::write(
            &path,
            r#"{"delay_us":50000,"exclude":[65],"hk_toggle":120}"#,
        )
        .unwrap();
        let (parser, data) = read_import(&path, None).unwrap();
        assert_eq!(parser.source_app, "丐帮高手");
        let preview = (parser.parse_preview)(&data, "test");
        assert_eq!(preview.interval_ms, 50);
        assert_eq!(preview.global_toggle, Some(KeyId::Keyboard(120)));
        assert!(read_import(&path, Some(0)).is_err());
    }

    #[test]
    fn oversized_and_unrelated_files_are_rejected_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(IMPORT_MAX_BYTES + 1)
            .unwrap();
        assert!(preview_import(path.display().to_string(), None)
            .unwrap_err()
            .contains("超过 4 MB"));
        std::fs::write(&path, r#"{"Version":20,"unrelated":true}"#).unwrap();
        assert!(to_found_configs(&path).is_empty());
    }

    fn native_profile() -> Profile {
        Profile {
            schema_version: CURRENT_SCHEMA_VERSION,
            meta: ProfileMeta {
                name: "分享配置".into(),
                created_at: 1,
                updated_at: 2,
                app_version: "0.7.1".into(),
                signature: "测试署名".into(),
            },
            rules: vec![BurstRule {
                id: "native-rule".into(),
                enabled: false,
                trigger_key: KeyId::Keyboard(65),
                steps: vec![qzh_profile::profile::OutputStep {
                    key: KeyId::Keyboard(66),
                    interval_ms: 50,
                }],
                mode: BurstMode::Toggle,
                stop_key: Some(KeyId::Keyboard(67)),
                group: Some("互斥组".into()),
            }],
            hotkeys: Hotkeys {
                global_toggle: Some(KeyId::Keyboard(120)),
                global_stop: Some(KeyId::Keyboard(121)),
                panel_toggle: Some(KeyId::Keyboard(122)),
            },
            advanced: Advanced {
                log_level: "debug".into(),
            },
        }
    }

    #[test]
    fn native_adapter_preserves_complete_profile_and_does_not_change_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("分享.QZH");
        let profile = native_profile();
        write_profile_file_to_path(&path, &profile).unwrap();
        let original = std::fs::read(&path).unwrap();
        let preview = preview_import(path.display().to_string(), None).unwrap();
        assert_eq!(preview.source_app, "气质花");
        assert_eq!(preview.suggested_name, "分享配置");
        assert_eq!(preview.rule_count, 1);
        assert_eq!(preview.interval_ms, 50);
        assert_eq!(preview.global_stop, profile.hotkeys.global_stop);
        assert_eq!(
            serde_json::to_value(qzh::read(&path).unwrap()).unwrap(),
            serde_json::to_value(&profile).unwrap()
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);

        let mut empty = profile;
        empty.rules.clear();
        write_profile_file_to_path(&path, &empty).unwrap();
        assert_eq!(
            preview_import(path.display().to_string(), None)
                .unwrap()
                .rule_count,
            0
        );
        assert!(qzh::read(&path).unwrap().rules.is_empty());
    }

    #[test]
    fn scanner_discovers_native_files_outside_managed_directory_once() {
        let dir = tempfile::tempdir().unwrap();
        let managed = dir.path().join("profiles");
        let portable = dir.path().join("分享");
        std::fs::create_dir(&managed).unwrap();
        std::fs::create_dir(&portable).unwrap();
        let profile = native_profile();
        for path in [
            dir.path().join("桌面.QZH"),
            portable.join("下载.qzh"),
            managed.join("已生效.qzh"),
        ] {
            write_profile_file_to_path(&path, &profile).unwrap();
        }
        std::fs::write(dir.path().join("损坏.qzh"), b"invalid").unwrap();
        let found = scan_targets(
            vec![dir.path().to_path_buf(), portable, managed.clone()],
            Some(&managed),
        );
        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|f| f.source_app == "气质花"
            && f.display_name == "分享配置"
            && f.source_profile_index.is_none()));

        let oversized = dir.path().join("过大.qzh");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(IMPORT_MAX_BYTES + 1)
            .unwrap();
        assert!(preview_import(oversized.display().to_string(), None)
            .unwrap_err()
            .contains("超过 4 MB"));
        assert!(qzh::read(&oversized).is_err());
        assert!(to_found_configs(&oversized).is_empty());
    }
}
