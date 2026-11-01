//! 环境诊断与修复。
//!
//! 解决用户在卸载 / 强删驱动文件后陷入"半卸载"状态，以及全新装机环境下
//! 因 Windows 安全策略 / 杀软拦截导致驱动首次安装失败的问题：
//! - DD-HID：sys 没了但 PnP 服务键 / OEM INF 残留 → 重装报 install error
//! - Interception：服务键残留但 sys 缺失 → 安装器复装失败
//! - .qzh 损坏：AES Tag 校验失败时启动会回退默认配置，但损坏文件还在原地占位
//! - 安装前置环境：HVCI 阻断内核驱动加载、SAC 阻断未知信誉 exe、
//!   Defender 实时保护拦截 ddc.exe / ddhid63340.sys、待重启的 PnP 事务等
//! - 外设驱动：扫串口看认不认得出来；Interception 设备槽用满时它的键盘 / 鼠标输入被吞
//!
//! 模块对外暴露三类入口：
//! 1. [`diagnose_environment`]：只读，不提权，列出所有可疑残留与安装前置异常
//! 2. `repair_*`：每类问题一个修复命令，全部走现有 `run_elevated_exe_capture`
//! 3. 修复前自动 `Export-WindowsDriver` / `reg export` 到
//!    `{app_local_data_dir}/repair_backup/<timestamp>/`，留回滚余地
//!
//! 所有 PowerShell 脚本都是幂等的（`Test-Path` / `Get-Service` 先判存再动手），
//! 多次调用不会重复破坏，方便用户反复点击"修复"。

#![allow(dead_code)] // 非 Windows 平台修复函数体为 stub，但类型仍需导出供前端 invoke

use serde::Serialize;
#[allow(unused_imports)]
use tauri::{AppHandle, Manager};
#[cfg(windows)]
use win_sysinfo::{prereq, registry};

// ===== 公共数据结构 =====

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 仅供参考的状态项（如"配置目录可写"）
    Info,
    /// 异常但不影响核心功能（如旧日志未清理）
    Warn,
    /// 影响功能（如驱动残留导致重装失败）
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// 一切正常
    Ok,
    /// PnP / 服务键残留但实际驱动文件不存在
    Orphan,
    /// 应该存在的资源缺失
    Missing,
    /// 文件存在但内容损坏
    Corrupted,
    /// 检测过程异常，状态未知
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticItem {
    /// 稳定的项目 ID，前端用作 key
    pub id: String,
    /// 分类，用于前端分组
    pub category: String,
    /// 用户友好的项目名
    pub label: String,
    pub severity: Severity,
    pub status: ItemStatus,
    /// 详情：路径、OEM INF 编号、错误描述等
    pub detail: String,
    /// 推荐执行的修复 command 名（前端 invoke 用）
    pub recommended_action: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    /// ISO-8601 形式的时间戳
    pub timestamp: String,
    pub items: Vec<DiagnosticItem>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Ok,
    /// 该步骤本就不需要执行（如目标本来就不存在）
    Skipped,
    /// 物理执行失败
    Failed,
    /// 已标记重启删除
    PendingReboot,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairStep {
    pub name: String,
    pub status: StepStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairOutcome {
    /// 整体是否成功（任意 Failed 即视为整体失败）
    pub success: bool,
    /// 是否需要重启
    pub pending_reboot: bool,
    /// 用户可见的总结文案
    pub summary: String,
    /// 每个修复步骤的详细结果，前端按列表展示
    pub steps: Vec<RepairStep>,
    /// 备份目录（若产生备份），出错时用户可手动恢复
    pub backup_dir: Option<String>,
}

// ===== 时间戳 / 备份目录 =====

/// 形如 `20260527-153045` 的时间戳，跨平台、文件系统安全。
fn timestamp_slug() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    // 朴素 UTC 拆分；只用于文件名，不需要时区精确
    let day_secs = 86_400u64;
    let days_since_epoch = secs / day_secs;
    let time_of_day = secs % day_secs;
    let h = time_of_day / 3600;
    let m = (time_of_day / 60) % 60;
    let s = time_of_day % 60;
    let (y, mo, d) = ymd_from_days(days_since_epoch as i64);
    format!("{y:04}{mo:02}{d:02}-{h:02}{m:02}{s:02}")
}

/// 1970-01-01 起 N 天 → (年, 月, 日)。简易实现，足够文件名用。
fn ymd_from_days(days: i64) -> (i32, u32, u32) {
    // Howard Hinnant 的 days_from_civil 反算
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(windows)]
fn ensure_backup_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let base = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("无法获取本地数据目录: {e}"))?
        .join("repair_backup")
        .join(timestamp_slug());
    std::fs::create_dir_all(&base).map_err(|e| format!("创建备份目录失败: {e}"))?;
    Ok(base)
}

// ===== 诊断（只读）=====

#[tauri::command]
pub async fn diagnose_environment(app: AppHandle) -> Result<RepairReport, String> {
    let mut items = Vec::new();

    #[cfg(windows)]
    {
        items.extend(diagnose_install_prerequisites(&app).await);
        items.extend(diagnose_dd_hid(&app));
        items.extend(diagnose_interception(&app));
        let (dd_blocked, dd_detail) = win_input::dd_channel_status();
        items.push(DiagnosticItem {
            id: "ddsimple.channel".into(),
            category: "DD 驱动".into(),
            label: "运行设备与占用".into(),
            severity: if dd_blocked {
                Severity::Warn
            } else {
                Severity::Info
            },
            status: ItemStatus::Unknown,
            detail: dd_detail,
            recommended_action: None,
        });
        items.extend(diagnose_hidstick());
    }

    items.extend(diagnose_profiles(&app));
    items.extend(diagnose_logs(&app));

    #[cfg(not(windows))]
    let _ = &app;

    Ok(RepairReport {
        timestamp: timestamp_slug(),
        items,
    })
}

// ===== 安装前置检查（Windows 安全策略 / 杀软 / 资源 / 架构）=====

#[cfg(windows)]
async fn diagnose_install_prerequisites(app: &AppHandle) -> Vec<DiagnosticItem> {
    let mut out = Vec::new();

    // ---- 资源完整性（先于一切其它检查，缺文件直接出错）----
    let resource_health = collect_install_resources(app);
    let resources_ok = resource_health.ok();
    let resource_issue_text = resource_health
        .issues
        .iter()
        .map(crate::commands::resource_integrity::issue_label)
        .collect::<Vec<_>>();
    out.push(DiagnosticItem {
        id: "prereq.resources".to_string(),
        category: "安装前置检查".to_string(),
        label: "驱动安装资源".to_string(),
        severity: if resources_ok {
            Severity::Info
        } else {
            Severity::Error
        },
        status: if resources_ok {
            ItemStatus::Ok
        } else if resource_health.has_missing() {
            ItemStatus::Missing
        } else {
            ItemStatus::Corrupted
        },
        detail: if resources_ok {
            "驱动安装资源完整，DD-HID INF/CAT/SYS 与原始签名包匹配".to_string()
        } else {
            format!(
                "驱动资源异常：{}\n请覆盖安装新版，或用原始驱动包替换异常文件后重试。",
                resource_issue_text.join("；")
            )
        },
        recommended_action: None,
    });

    // ---- 系统架构 ----
    let arch = win_sysinfo::host_arch();
    let (arch_ok, arch_detail) = prereq::classify_arch_compat(&arch);
    out.push(DiagnosticItem {
        id: "prereq.arch".to_string(),
        category: "安装前置检查".to_string(),
        label: "系统架构".to_string(),
        severity: if arch_ok {
            Severity::Info
        } else {
            Severity::Error
        },
        status: if arch_ok {
            ItemStatus::Ok
        } else {
            ItemStatus::Missing
        },
        detail: arch_detail,
        recommended_action: None,
    });

    // ---- HVCI / 内存完整性 ----
    let hvci = prereq::detect_hvci_active();
    out.push(DiagnosticItem {
        id: "prereq.hvci".to_string(),
        category: "安装前置检查".to_string(),
        label: "内存完整性 (HVCI)".to_string(),
        severity: match hvci {
            Some(true) => Severity::Error,
            _ => Severity::Info,
        },
        status: match hvci {
            Some(true) => ItemStatus::Orphan, // 借用 Orphan 表示"启用了应该关闭的策略"
            Some(false) => ItemStatus::Ok,
            None => ItemStatus::Unknown,
        },
        detail: match hvci {
            Some(true) => "内核隔离已启用，会拒绝加载未通过 HVCI 兼容认证的驱动。\
                请在 Windows 安全中心 → 设备安全性 → 内核隔离详细信息 → 关闭"
                .to_string(),
            Some(false) => "内核隔离未启用".to_string(),
            None => "未读取到 HVCI 策略键，可能为旧版 Windows".to_string(),
        },
        recommended_action: None,
    });

    // ---- Smart App Control ----
    let sac = prereq::detect_sac_state();
    out.push(DiagnosticItem {
        id: "prereq.sac".to_string(),
        category: "安装前置检查".to_string(),
        label: "Smart App Control".to_string(),
        severity: match sac {
            prereq::SacState::Enforce | prereq::SacState::Evaluation => Severity::Error,
            _ => Severity::Info,
        },
        status: match sac {
            prereq::SacState::Enforce | prereq::SacState::Evaluation => ItemStatus::Orphan,
            prereq::SacState::Off => ItemStatus::Ok,
            prereq::SacState::Unknown => ItemStatus::Unknown,
        },
        detail: match sac {
            prereq::SacState::Enforce => "已强制启用，会拦截无强信誉的 ddc.exe。\
                请在 Windows 设置 → 隐私与安全 → 应用控制中关闭"
                .to_string(),
            prereq::SacState::Evaluation => "处于评估模式，仍会拦截未知信誉应用。\
                请在 Windows 设置 → 隐私与安全 → 应用控制中关闭"
                .to_string(),
            prereq::SacState::Off => "未启用".to_string(),
            prereq::SacState::Unknown => "状态未知".to_string(),
        },
        recommended_action: None,
    });

    // ---- 待重启 ----
    let pending = prereq::detect_pending_reboot();
    out.push(DiagnosticItem {
        id: "prereq.pending_reboot".to_string(),
        category: "安装前置检查".to_string(),
        label: "系统待重启".to_string(),
        severity: if pending {
            Severity::Warn
        } else {
            Severity::Info
        },
        status: if pending {
            ItemStatus::Orphan
        } else {
            ItemStatus::Ok
        },
        detail: if pending {
            "系统存在挂起的重启请求（更新 / 驱动事务），先重启再装驱动可避免 install error"
                .to_string()
        } else {
            "无挂起的重启请求".to_string()
        },
        recommended_action: None,
    });

    // ---- Defender 白名单 ----
    // 不提供一键加白：调用 Add-MpPreference 容易被 Defender 自身的"行为分析"或第三方
    // 杀软拦截，反而把 FlairBloom 标红。这里只做检测和文字引导，让用户自己去
    // Windows 安全中心 → 病毒和威胁防护 → 排除项 添加。
    let install_dir = win_sysinfo::install_path();
    let exclusions = prereq::read_defender_exclusion_paths().await;
    let (severity, status, detail) = match &exclusions {
        None => (
            Severity::Warn,
            ItemStatus::Unknown,
            "无法读取 Defender 排除项（可能被组策略限制或 PowerShell 不可用）".to_string(),
        ),
        Some(list) => {
            if install_dir.is_empty() {
                (
                    Severity::Info,
                    ItemStatus::Unknown,
                    "无法定位安装目录，跳过白名单核对".to_string(),
                )
            } else if prereq::is_path_excluded(&install_dir, list) {
                (
                    Severity::Info,
                    ItemStatus::Ok,
                    format!("安装目录已在 Defender 排除列表：{install_dir}"),
                )
            } else {
                (
                    Severity::Warn,
                    ItemStatus::Missing,
                    format!(
                        "安装目录未加入 Defender 排除列表：{install_dir}\n\
                        若驱动安装屡次失败，可在 Windows 安全中心 → 病毒和威胁防护 → \
                        管理设置 → 排除项 中手动添加该目录"
                    ),
                )
            }
        }
    };
    out.push(DiagnosticItem {
        id: "prereq.defender_exclusion".to_string(),
        category: "安装前置检查".to_string(),
        label: "Windows Defender 白名单".to_string(),
        severity,
        status,
        detail,
        recommended_action: None,
    });

    out
}

/// 资源完整性自检（独立于 status.rs 的结果，避免它被缓存）。
#[cfg(windows)]
fn collect_install_resources(
    app: &AppHandle,
) -> crate::commands::resource_integrity::ResourceHealth {
    let resources = match app.path().resource_dir() {
        Ok(d) => d.join("resources"),
        Err(e) => {
            return crate::commands::resource_integrity::ResourceHealth {
                checked: 0,
                issues: vec![crate::commands::resource_integrity::ResourceIssue {
                    rel: "<resource_dir>",
                    path: std::path::PathBuf::new(),
                    kind: crate::commands::resource_integrity::ResourceIssueKind::ReadError(
                        e.to_string(),
                    ),
                }],
            }
        }
    };
    crate::commands::resource_integrity::check_resources(&resources)
}

#[cfg(windows)]
fn diagnose_dd_hid(_app: &AppHandle) -> Vec<DiagnosticItem> {
    let mut out = Vec::new();
    let sys_present = win_driver::dd_hid::dd_hid_sys_installed();
    let service_present = registry::service_key_present("ddhid63340");
    let oem_inf = win_driver::dd_hid::find_dd_hid_oem_inf();
    let driverstore = win_driver::dd_hid::list_dd_hid_driverstore();

    out.push(DiagnosticItem {
        id: "dd_hid.sys".to_string(),
        category: "DD-HID 驱动".to_string(),
        label: "驱动文件 ddhid63340.sys".to_string(),
        severity: Severity::Info,
        status: if sys_present {
            ItemStatus::Ok
        } else {
            ItemStatus::Missing
        },
        detail: if sys_present {
            "已落盘".to_string()
        } else {
            "未安装或已被删除".to_string()
        },
        recommended_action: None,
    });

    out.push(DiagnosticItem {
        id: "dd_hid.service".to_string(),
        category: "DD-HID 驱动".to_string(),
        label: "服务键 ddhid63340".to_string(),
        severity: classify_residue_severity(service_present, sys_present),
        status: classify_residue_status(service_present, sys_present),
        detail: format!(
            "{}\nHKLM\\SYSTEM\\CurrentControlSet\\Services\\ddhid63340",
            residue_detail(service_present, sys_present, "服务键"),
        ),
        recommended_action: if service_present && !sys_present {
            Some("repair_dd_hid_residue".to_string())
        } else {
            None
        },
    });

    let oem_count = oem_inf.len();
    out.push(DiagnosticItem {
        id: "dd_hid.oem_inf".to_string(),
        category: "DD-HID 驱动".to_string(),
        label: "PnP 注册的 OEM INF".to_string(),
        severity: if oem_count > 0 && !sys_present {
            Severity::Error
        } else if oem_count > 1 {
            Severity::Warn
        } else {
            Severity::Info
        },
        status: if oem_count == 0 {
            ItemStatus::Ok
        } else if !sys_present {
            ItemStatus::Orphan
        } else {
            ItemStatus::Ok
        },
        detail: if oem_count == 0 {
            "无残留".to_string()
        } else {
            format!("发现 {oem_count} 项: {}", oem_inf.join(", "))
        },
        recommended_action: if oem_count > 0 && !sys_present {
            Some("repair_dd_hid_residue".to_string())
        } else {
            None
        },
    });

    out.push(DiagnosticItem {
        id: "dd_hid.driverstore".to_string(),
        category: "DD-HID 驱动".to_string(),
        label: "Driver Store 副本".to_string(),
        severity: if !driverstore.is_empty() && !sys_present {
            Severity::Warn
        } else {
            Severity::Info
        },
        status: if driverstore.is_empty() {
            ItemStatus::Ok
        } else if !sys_present {
            ItemStatus::Orphan
        } else {
            ItemStatus::Ok
        },
        detail: if driverstore.is_empty() {
            "无残留".to_string()
        } else {
            format!("发现 {} 项目录副本", driverstore.len())
        },
        recommended_action: if !driverstore.is_empty() && !sys_present {
            Some("repair_dd_hid_residue".to_string())
        } else {
            None
        },
    });

    out
}

#[cfg(windows)]
fn interception_binary_missing(service: &str) -> bool {
    let Some(raw) = registry::read_service_image_path(service) else {
        return false;
    };
    let raw = raw.trim_matches('"');
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from);
    let path = if let Some(relative) = raw
        .strip_prefix(r"\systemroot\")
        .or_else(|| raw.strip_prefix(r"%systemroot%\"))
    {
        root.map(|p| p.join(relative))
    } else if raw.starts_with(r"system32\") {
        root.map(|p| p.join(raw))
    } else {
        let raw = raw.strip_prefix(r"\??\").unwrap_or(raw);
        let path = std::path::PathBuf::from(raw);
        (path.is_absolute() && !raw.contains('%')).then_some(path)
    };
    path.is_some_and(|path| {
        std::fs::metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    })
}

#[cfg(windows)]
fn diagnose_interception(app: &AppHandle) -> Vec<DiagnosticItem> {
    let mut out = Vec::new();
    let health = crate::commands::status::collect_interception_health();
    let warning = health.warning;
    let api_ok = win_input::interception::is_driver_installed();
    // 仅当服务键 ImagePath 指向 Interception 的 keyboard.sys / mouse.sys 才视为残留，
    // 避免把同名第三方服务误判
    let kbd = registry::is_interception_service("keyboard", "keyboard.sys");
    let mouse = registry::is_interception_service("mouse", "mouse.sys");
    let kbd_present_raw = registry::service_key_present("keyboard");
    let mouse_present_raw = registry::service_key_present("mouse");
    let foreign_kbd = kbd_present_raw && !kbd;
    let foreign_mouse = mouse_present_raw && !mouse;
    let missing_binary = (kbd && interception_binary_missing("keyboard"))
        || (mouse && interception_binary_missing("mouse"));

    out.push(DiagnosticItem {
        id: "interception.runtime".to_string(),
        category: "Interception 驱动".to_string(),
        label: "运行时可用性".to_string(),
        severity: if health.issue.is_some()
            && health.status != crate::commands::status::DriverStatus::NotInstalled
        {
            Severity::Error
        } else {
            Severity::Info
        },
        status: if health.issue.is_none() {
            ItemStatus::Ok
        } else {
            ItemStatus::Missing
        },
        detail: format!(
            "当前模式：{}；保存的模式：{}\n{}",
            win_input::current_mode().as_str(),
            crate::commands::status::collect_configured_input_mode(app),
            health.detail
        ),
        recommended_action: None,
    });

    if let Some(detail) = warning {
        out.push(DiagnosticItem {
            id: "interception.capacity".into(),
            category: "Interception 驱动".into(),
            label: "设备与容量提示（不阻止启用）".into(),
            severity: Severity::Warn,
            status: ItemStatus::Unknown,
            detail,
            recommended_action: None,
        });
    }

    let mut detail = format!("keyboard: {} / mouse: {}", yes_no(kbd), yes_no(mouse),);
    if foreign_kbd || foreign_mouse {
        detail.push_str("（检测到同名但非 Interception 的服务键，已跳过，不会清理）");
    }

    out.push(DiagnosticItem {
        id: "interception.services".to_string(),
        category: "Interception 驱动".to_string(),
        label: "keyboard / mouse 服务键".to_string(),
        severity: if missing_binary && !api_ok {
            Severity::Error
        } else {
            Severity::Info
        },
        status: match (kbd || mouse, api_ok) {
            (true, true) => ItemStatus::Ok,
            (true, false) => ItemStatus::Orphan,
            (false, _) => ItemStatus::Missing,
        },
        detail,
        // 通道打不开也可能是首次安装待重启或被占用，只有确证驱动文件缺失才推荐清残留。
        recommended_action: (missing_binary && !api_ok)
            .then(|| "repair_interception_residue".into()),
    });

    out
}

/// 外设驱动：串口扫描 + Interception 槽位。扫描会对每个非转串口芯片的 USB 串口握一次手
/// （开口拉 DTR、发 `ID`），只在用户点诊断时做；外设模式正用着的口不碰。
#[cfg(windows)]
fn diagnose_hidstick() -> Vec<DiagnosticItem> {
    use win_input::hidstick::{scan_ports, slot_of, KEYBOARD_SLOTS, MOUSE_SLOTS};

    const CATEGORY: &str = "外设驱动";
    let in_use = win_input::hidstick_in_use();
    let scans = scan_ports(in_use.as_ref().map(|(p, f)| (p.as_str(), f.as_str())));
    crate::bootstrap::input::remember_verified_sticks(&scans);
    let stick = scans.iter().find(|s| s.is_stick());

    let mut out = vec![DiagnosticItem {
        id: "hidstick.ports".to_string(),
        category: CATEGORY.to_string(),
        label: "串口扫描".to_string(),
        severity: Severity::Info,
        status: if stick.is_some() {
            ItemStatus::Ok
        } else {
            ItemStatus::Missing
        },
        detail: if scans.is_empty() {
            "没有 USB 串口。外设驱动要插原生 USB 口（主板后置口更稳）".to_string()
        } else {
            let lines: Vec<String> = scans.iter().map(|s| s.describe()).collect();
            lines.join("；")
        },
        recommended_action: None,
    }];

    let Some(stick) = stick else {
        return out;
    };
    let (status, severity, detail) = match win_input::interception::device_hardware_ids() {
        None => (
            ItemStatus::Ok,
            Severity::Info,
            "没启用游戏模式驱动（Interception），外设驱动不受设备槽限制".to_string(),
        ),
        Some(slots) => {
            let keyboard = slot_of(&slots, KEYBOARD_SLOTS, stick.vid, stick.pid);
            let mouse = slot_of(&slots, MOUSE_SLOTS, stick.vid, stick.pid);
            let used = |range: std::ops::RangeInclusive<i32>| {
                slots
                    .iter()
                    .filter(|(slot, _)| range.contains(slot))
                    .count()
            };
            let slot_text = |slot: Option<i32>| match slot {
                Some(n) => format!("{n} 号"),
                None => "没分到".to_string(),
            };
            let summary = format!(
                "键盘槽 {}、鼠标槽 {}（键盘槽在用 {}/10，鼠标槽在用 {}/10）",
                slot_text(keyboard),
                slot_text(mouse),
                used(KEYBOARD_SLOTS),
                used(MOUSE_SLOTS),
            );
            if keyboard.is_some() && mouse.is_some() {
                (ItemStatus::Ok, Severity::Info, summary)
            } else {
                let lost = match (keyboard, mouse) {
                    (None, None) => "键盘和鼠标",
                    (None, _) => "键盘",
                    _ => "鼠标",
                };
                (
                    ItemStatus::Missing,
                    Severity::Error,
                    format!(
                        "{summary}。外设驱动的{lost}没分到游戏模式驱动（Interception）的设备槽，\
                         这部分输入会被驱动吞掉，设备管理器里却显示正常。每类槽只有 10 个，\
                         拔插设备、装卸虚拟键盘都会占用，拔掉也不还，重启电脑后清零"
                    ),
                )
            }
        }
    };
    out.push(DiagnosticItem {
        id: "hidstick.slots".to_string(),
        category: CATEGORY.to_string(),
        label: "游戏模式驱动设备槽".to_string(),
        severity,
        status,
        detail,
        recommended_action: None,
    });
    out
}

fn diagnose_profiles(app: &AppHandle) -> Vec<DiagnosticItem> {
    let mut out = Vec::new();
    let profiles_dir = match app.path().app_data_dir() {
        Ok(d) => d.join("profiles"),
        Err(_) => return out,
    };
    if !profiles_dir.exists() {
        return out;
    }
    let mut total = 0usize;
    let mut corrupted = Vec::new();
    let entries = match std::fs::read_dir(&profiles_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("qzh") {
            continue;
        }
        total += 1;
        if !is_profile_readable(&path) {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                corrupted.push(name.to_string());
            }
        }
    }
    out.push(DiagnosticItem {
        id: "profiles.health".to_string(),
        category: "配置文件".to_string(),
        label: "已存配置完整性".to_string(),
        severity: if corrupted.is_empty() {
            Severity::Info
        } else {
            Severity::Warn
        },
        status: if corrupted.is_empty() {
            ItemStatus::Ok
        } else {
            ItemStatus::Corrupted
        },
        detail: if corrupted.is_empty() {
            format!("{total} 份配置全部可解密")
        } else {
            format!(
                "{} / {} 份损坏: {}",
                corrupted.len(),
                total,
                corrupted.join(", ")
            )
        },
        recommended_action: if corrupted.is_empty() {
            None
        } else {
            Some("repair_corrupted_profiles".to_string())
        },
    });
    out
}

fn diagnose_logs(_app: &AppHandle) -> Vec<DiagnosticItem> {
    let dir = crate::log_dir();
    let mut count = 0usize;
    let mut bytes = 0u64;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    count += 1;
                    bytes += meta.len();
                }
            }
        }
    }
    let mb = bytes as f64 / 1024.0 / 1024.0;
    vec![DiagnosticItem {
        id: "logs.size".to_string(),
        category: "日志".to_string(),
        label: "本地日志文件".to_string(),
        severity: if mb > 50.0 {
            Severity::Warn
        } else {
            Severity::Info
        },
        status: ItemStatus::Ok,
        detail: format!("{count} 份, 共 {mb:.1} MB"),
        recommended_action: if mb > 50.0 {
            Some("repair_clean_logs".to_string())
        } else {
            None
        },
    }]
}

// ===== 诊断辅助：纯逻辑分类（可单测）=====

/// 服务键 / OEM INF 这类"PnP 残留"的 severity 判定。
///
/// 残留本身在驱动正常工作时是允许的；只有 sys 文件已经消失却仍留有登记
/// 时才会真正阻塞重装，所以这种状态升级为 Error。
pub(crate) fn classify_residue_severity(residue_present: bool, sys_present: bool) -> Severity {
    match (residue_present, sys_present) {
        (true, false) => Severity::Error,
        (true, true) => Severity::Info,
        (false, _) => Severity::Info,
    }
}

pub(crate) fn classify_residue_status(residue_present: bool, sys_present: bool) -> ItemStatus {
    match (residue_present, sys_present) {
        (true, false) => ItemStatus::Orphan,
        (true, true) => ItemStatus::Ok,
        (false, _) => ItemStatus::Ok,
    }
}

fn residue_detail(residue_present: bool, sys_present: bool, label: &str) -> String {
    match (residue_present, sys_present) {
        (true, false) => format!("{label}存在但驱动文件已缺失（半卸载状态，会阻塞重装）"),
        (true, true) => format!("{label}存在且驱动文件正常"),
        (false, _) => format!("{label}不存在"),
    }
}

fn yes_no(v: bool) -> &'static str {
    if v {
        "存在"
    } else {
        "不存在"
    }
}

fn is_profile_readable(path: &std::path::Path) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return false;
    };
    let Some(header) = qzh_format::header::FileHeader::from_bytes(&data) else {
        return false;
    };
    if data.len() <= qzh_format::header::FileHeader::SIZE {
        return false;
    }
    let aad = header.aad();
    let ciphertext = &data[qzh_format::header::FileHeader::SIZE..];
    crypto::aes::decrypt(ciphertext, &header.nonce, &aad).is_ok()
}

// ===== Windows 注册表辅助（已迁至 win-sysinfo::registry）=====

// ===== 修复：DD-HID 深度清理（find_dd_hid_oem_inf / list_dd_hid_driverstore 已迁至 win-driver）=====

#[tauri::command]
pub async fn repair_dd_hid_residue(app: AppHandle) -> Result<RepairOutcome, String> {
    #[cfg(windows)]
    {
        run_dd_hid_repair(app).await
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅 Windows 平台支持驱动修复".to_string())
    }
}

#[cfg(windows)]
async fn run_dd_hid_repair(app: AppHandle) -> Result<RepairOutcome, String> {
    // 修复前先停连发、经旧后端阻塞释放已按下的目标键，再切回 SendInput，且切换窗口内不启动新规则：
    // 避免修复进行时 DLL 仍持有 sys 句柄，也避免正连发时切后端导致目标键 down/up 跨后端错配卡住。
    crate::commands::engine::leave_driver_backend(
        &app,
        &app.state::<crate::commands::engine::EngineState>().0,
    );

    let backup = ensure_backup_dir(&app)?;
    let oem_inf = win_driver::dd_hid::find_dd_hid_oem_inf();
    let driverstore = win_driver::dd_hid::list_dd_hid_driverstore();
    let service_present_before = registry::service_key_present("ddhid63340");
    let sys_present_before = win_driver::dd_hid::dd_hid_sys_installed();

    let backup_lit = win_driver::powershell::ps_single_quoted(&backup.display().to_string());
    let oem_inf_array = win_driver::powershell::ps_string_array(&oem_inf);

    // 单脚本：备份 → pnputil 标准卸载 → 仅在 PnP 未清干净时移除注册表残留键。
    // 关键设计：不再 takeown / icacls / Remove-Item / PendingFileRenameOperations
    // 强夺 sys 文件——那种"半卸载"状态正是后续重装失败的根因。
    // PnP 子系统在 pnputil /delete-driver /uninstall /force 时会自己处理：
    //   停服务 → 释放设备实例 → 删 sys → 清 Driver Store → 移除 INF。
    // 即便 sys 被 TrustedInstaller 持有也由 PnP 提权完成，无需我们绕过 ACL。
    //
    // 退出码：
    //   0 = 全部完成
    //   2 = 某步骤失败（脚本不再使用 1 / PendingReboot 形式，pending_reboot 由
    //       Rust 侧根据"动了什么残留"来综合判定，更精确）
    let script = format!(
        "$ErrorActionPreference='Continue';\n\
         $backup={backup_lit};\n\
         $hardFail=$false;\n\
         function Backup-RegKey($path,$file){{\n\
             try {{ & reg.exe export $path (Join-Path $backup $file) /y | Out-Null }} catch {{ }}\n\
         }}\n\
         # 1. 备份服务键 + OEM INF + PNF\n\
         Backup-RegKey 'HKLM\\SYSTEM\\CurrentControlSet\\Services\\ddhid63340' 'service_ddhid63340.reg'\n\
         $oemInfList = {oem_inf_array}\n\
         foreach ($oem in $oemInfList) {{\n\
             $src = Join-Path $env:SystemRoot \"INF\\$oem\"\n\
             if (Test-Path -LiteralPath $src) {{\n\
                 try {{ Copy-Item -LiteralPath $src -Destination $backup -Force -ErrorAction Stop }}\n\
                 catch {{ }}\n\
                 $pnf = $src -replace '\\.inf$', '.PNF'\n\
                 if (Test-Path -LiteralPath $pnf) {{ try {{ Copy-Item -LiteralPath $pnf -Destination $backup -Force -ErrorAction Stop }} catch {{ }} }}\n\
             }}\n\
         }}\n\
         # 2. pnputil /delete-driver /uninstall /force（让 PnP 主导，不绕过）\n\
         foreach ($oem in $oemInfList) {{\n\
             try {{ & pnputil.exe /delete-driver $oem /uninstall /force | Out-Null }}\n\
             catch {{ $hardFail=$true }}\n\
             if ($LASTEXITCODE -ne 0) {{ $hardFail=$true }}\n\
         }}\n\
         # 3. pnputil 走完后若服务键仍在（罕见，常因 sys 已被手动删除导致 PnP\n\
         #    无法识别该 INF 归属的设备实例）→ 仅做注册表层面的兜底清理。\n\
         #    这一步只动注册表，不动文件系统、不动 ACL。\n\
         if (Test-Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\ddhid63340') {{\n\
             try {{ Remove-Item -LiteralPath 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\ddhid63340' -Recurse -Force -ErrorAction Stop }}\n\
             catch {{ $hardFail=$true }}\n\
         }}\n\
         if ($hardFail) {{ exit 2 }}\n\
         exit 0",
    );

    let exit = win_driver::powershell::run_script_elevated(&script).await;
    crate::commands::status::emit_status_changed(&app);

    let sys_present_after = win_driver::dd_hid::dd_hid_sys_installed();
    let service_present_after = registry::service_key_present("ddhid63340");
    let oem_inf_after = win_driver::dd_hid::find_dd_hid_oem_inf();
    let driverstore_after = win_driver::dd_hid::list_dd_hid_driverstore();

    let mut steps = Vec::new();
    steps.push(make_step(
        "备份注册表与 INF",
        &backup,
        if backup.exists() {
            StepStatus::Ok
        } else {
            StepStatus::Failed
        },
    ));
    steps.push(removal_step(
        "卸载 OEM INF（pnputil）",
        oem_inf.len(),
        oem_inf_after.len(),
        |before, after| {
            if before == 0 {
                "无 OEM INF 需卸载".to_string()
            } else if after == 0 {
                format!("已卸载 {before} 项")
            } else {
                format!("仍残留 {after} 项 (原 {before} 项)")
            }
        },
    ));
    steps.push(boolean_removal_step(
        "清理服务键",
        service_present_before,
        service_present_after,
        "服务键 ddhid63340",
    ));
    steps.push(removal_step(
        "清理 Driver Store 副本",
        driverstore.len(),
        driverstore_after.len(),
        |before, after| {
            if before == 0 {
                "无 Driver Store 副本".to_string()
            } else if after == 0 {
                format!("已清理 {before} 项目录")
            } else {
                format!("仍残留 {after} 项 (原 {before} 项)")
            }
        },
    ));
    steps.push(pnp_sys_step(sys_present_before, sys_present_after));

    let mut success = matches!(exit, Ok(0));
    // 物理事实优先：即便脚本声称成功，只要服务键 / OEM INF 还在就是失败
    if service_present_after || !oem_inf_after.is_empty() {
        success = false;
    }
    // 只要动过任何残留就建议重启：PnP 内部状态（设备实例缓存、SCM 内存副本）
    // 必须等下次开机才能彻底刷新，立即重装大概率撞 install error
    let touched_anything = oem_inf.len() != oem_inf_after.len()
        || service_present_before != service_present_after
        || driverstore.len() != driverstore_after.len()
        || sys_present_before != sys_present_after;
    let pending_reboot = success && touched_anything;

    let summary = match (&exit, success, pending_reboot) {
        (_, true, true) => "残留已清理，请重启电脑后再尝试安装驱动".to_string(),
        (_, true, false) => "未发现 DD-HID 残留".to_string(),
        (Err(e), false, _) => format!("修复中断: {e}"),
        (_, false, _) => "修复部分完成，仍存在残留，建议重启后重试".to_string(),
    };

    Ok(RepairOutcome {
        success,
        pending_reboot,
        summary,
        steps,
        backup_dir: Some(backup.display().to_string()),
    })
}

// ===== 修复：Interception 残留 =====

#[tauri::command]
pub async fn repair_interception_residue(app: AppHandle) -> Result<RepairOutcome, String> {
    #[cfg(windows)]
    {
        run_interception_repair(app).await
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("仅 Windows 平台支持驱动修复".to_string())
    }
}

#[cfg(windows)]
async fn run_interception_repair(app: AppHandle) -> Result<RepairOutcome, String> {
    let backup = ensure_backup_dir(&app)?;
    // 严格识别：仅当服务键 ImagePath 指向 Interception 自带的 keyboard.sys / mouse.sys
    // 才允许清理，绝不动同名的第三方服务
    let kbd_before = registry::is_interception_service("keyboard", "keyboard.sys");
    let mouse_before = registry::is_interception_service("mouse", "mouse.sys");
    let kbd_raw = registry::service_key_present("keyboard");
    let mouse_raw = registry::service_key_present("mouse");
    let api_before = win_input::interception::is_driver_installed();

    // 仅在驱动 API 不可用、但服务键残留时才动手清理；其它情况保守跳过
    if api_before {
        return Ok(RepairOutcome {
            success: true,
            pending_reboot: false,
            summary: "Interception 驱动运行正常，无需修复".to_string(),
            steps: vec![RepairStep {
                name: "检测驱动状态".to_string(),
                status: StepStatus::Skipped,
                detail: "create_context 成功，驱动正常工作".to_string(),
            }],
            backup_dir: None,
        });
    }
    if !kbd_before && !mouse_before {
        let detail = if kbd_raw || mouse_raw {
            "存在同名服务键但 ImagePath 不指向 Interception，已跳过以保护第三方驱动".to_string()
        } else {
            "keyboard / mouse 服务键均不存在".to_string()
        };
        return Ok(RepairOutcome {
            success: true,
            pending_reboot: false,
            summary: "未发现 Interception 服务键残留".to_string(),
            steps: vec![RepairStep {
                name: "扫描服务键".to_string(),
                status: StepStatus::Skipped,
                detail,
            }],
            backup_dir: None,
        });
    }

    let kbd_missing = kbd_before && interception_binary_missing("keyboard");
    let mouse_missing = mouse_before && interception_binary_missing("mouse");
    if !kbd_missing && !mouse_missing {
        return Ok(RepairOutcome {
            success: false,
            pending_reboot: true,
            summary: "未确证驱动文件缺失，保留服务注册。首次安装请先重启 Windows；如已重启，请退出其他 Interception 程序排除独占，再诊断。".into(),
            steps: vec![RepairStep { name: "检查驱动文件".into(), status: StepStatus::Skipped, detail: "通道不可用不能单独证明服务是残留，未执行删除。".into() }],
            backup_dir: None,
        });
    }

    let backup_lit = win_driver::powershell::ps_single_quoted(&backup.display().to_string());
    // 注意：Windows 自带的 `kbdclass` / `mouclass` 服务名是别的；
    // Interception 注册的是 `keyboard` / `mouse`（小写、无后缀），与系统服务并存。
    // 这里只删确认是 Interception 的 key，由 PowerShell 脚本再次校验 ImagePath
    // 兜底，确保即便诊断与执行之间发生变化也不会误删。
    let targets: Vec<String> = [("keyboard", kbd_missing), ("mouse", mouse_missing)]
        .iter()
        .filter_map(|(n, ok)| if *ok { Some((*n).to_string()) } else { None })
        .collect();
    let target_array = win_driver::powershell::ps_string_array(&targets);
    let script = format!(
        "$ErrorActionPreference='Continue';\n\
         $backup={backup_lit};\n\
         $hardFail=$false;\n\
         foreach ($svc in {target_array}) {{\n\
             $regHive='HKLM:\\SYSTEM\\CurrentControlSet\\Services\\' + $svc\n\
             $regExport='HKLM\\SYSTEM\\CurrentControlSet\\Services\\' + $svc\n\
             if (-not (Test-Path $regHive)) {{ continue }}\n\
             # 二次校验 ImagePath，避免删错\n\
             $img=(Get-ItemProperty -Path $regHive -Name 'ImagePath' -ErrorAction SilentlyContinue).ImagePath\n\
             if (-not $img) {{ continue }}\n\
             $expected = $svc + '.sys'\n\
             if (-not ($img.ToLower().EndsWith('\\' + $expected) -or $img.ToLower().EndsWith($expected))) {{ continue }}\n\
             try {{ & reg.exe export $regExport (Join-Path $backup ('service_' + $svc + '.reg')) /y | Out-Null }} catch {{ }}\n\
             try {{ Remove-Item -LiteralPath $regHive -Recurse -Force -ErrorAction Stop }}\n\
             catch {{ $hardFail=$true }}\n\
         }}\n\
         if ($hardFail) {{ exit 2 }}\n\
         exit 0",
    );

    let exit = win_driver::powershell::run_script_elevated(&script).await;
    crate::commands::status::emit_status_changed(&app);

    let kbd_after = registry::is_interception_service("keyboard", "keyboard.sys");
    let mouse_after = registry::is_interception_service("mouse", "mouse.sys");

    let steps = vec![
        make_step(
            "备份服务键",
            &backup,
            if backup.exists() {
                StepStatus::Ok
            } else {
                StepStatus::Failed
            },
        ),
        boolean_removal_step(
            "删除 keyboard 服务键",
            kbd_missing,
            kbd_after,
            "keyboard 服务键",
        ),
        boolean_removal_step(
            "删除 mouse 服务键",
            mouse_missing,
            mouse_after,
            "mouse 服务键",
        ),
    ];

    let success =
        (!kbd_missing || !kbd_after) && (!mouse_missing || !mouse_after) && matches!(exit, Ok(0));
    let summary = if success {
        "Interception 残留已清理，请重启电脑后再尝试安装".to_string()
    } else {
        match exit {
            Err(e) => format!("修复中断: {e}"),
            _ => "修复部分完成，仍存在残留".to_string(),
        }
    };

    Ok(RepairOutcome {
        success,
        pending_reboot: success, // 删服务键必须重启才能重新加载 Interception
        summary,
        steps,
        backup_dir: Some(backup.display().to_string()),
    })
}

// ===== 修复：损坏的 .qzh 配置 =====

#[tauri::command]
pub async fn repair_corrupted_profiles(app: AppHandle) -> Result<RepairOutcome, String> {
    let profiles_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法获取应用数据目录: {e}"))?
        .join("profiles");
    if !profiles_dir.exists() {
        return Ok(RepairOutcome {
            success: true,
            pending_reboot: false,
            summary: "未发现配置目录".to_string(),
            steps: Vec::new(),
            backup_dir: None,
        });
    }
    let corrupted_dir = profiles_dir.join("corrupted").join(timestamp_slug());

    let mut steps = Vec::new();
    let mut moved = 0usize;
    let entries = std::fs::read_dir(&profiles_dir).map_err(|e| format!("读取配置目录失败: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("qzh") {
            continue;
        }
        if is_profile_readable(&path) {
            continue;
        }
        if moved == 0 {
            std::fs::create_dir_all(&corrupted_dir)
                .map_err(|e| format!("创建隔离目录失败: {e}"))?;
        }
        let name = path
            .file_name()
            .map(|n| n.to_owned())
            .unwrap_or_else(|| std::ffi::OsString::from("unknown.qzh"));
        let target = corrupted_dir.join(&name);
        match std::fs::rename(&path, &target) {
            Ok(()) => {
                moved += 1;
                steps.push(RepairStep {
                    name: format!("隔离 {}", name.to_string_lossy()),
                    status: StepStatus::Ok,
                    detail: target.display().to_string(),
                });
            }
            Err(e) => steps.push(RepairStep {
                name: format!("隔离 {}", name.to_string_lossy()),
                status: StepStatus::Failed,
                detail: format!("移动失败: {e}"),
            }),
        }
    }

    let failed = steps
        .iter()
        .filter(|s| s.status == StepStatus::Failed)
        .count();
    let success = failed == 0;
    // 失败优先：哪怕 moved == 0，只要有 rename 失败也要给出失败文案，
    // 不能再说"未发现损坏配置"——前端会同时收到 error toast，文案必须自洽
    let summary = if failed > 0 && moved == 0 {
        format!("发现损坏配置但全部移动失败（{failed} 份）")
    } else if failed > 0 {
        format!("已隔离 {moved} 份，但有 {failed} 份移动失败")
    } else if moved == 0 {
        "未发现损坏的配置".to_string()
    } else {
        format!("已隔离 {moved} 份损坏配置到 corrupted/ 目录")
    };

    Ok(RepairOutcome {
        success,
        pending_reboot: false,
        summary,
        steps,
        backup_dir: if moved > 0 {
            Some(corrupted_dir.display().to_string())
        } else {
            None
        },
    })
}

// ===== 修复：清理旧日志 =====

#[tauri::command]
pub async fn repair_clean_logs(_app: AppHandle) -> Result<RepairOutcome, String> {
    use std::time::{Duration, SystemTime};
    let dir = crate::log_dir();
    if !dir.exists() {
        return Ok(RepairOutcome {
            success: true,
            pending_reboot: false,
            summary: "无日志目录".to_string(),
            steps: Vec::new(),
            backup_dir: None,
        });
    }
    let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);
    let mut removed = 0usize;
    let mut bytes_freed = 0u64;
    let mut steps = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| format!("读取日志目录失败: {e}"))?
        .flatten()
    {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("crash-") {
            continue; // 崩溃日志保留供反馈
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let size = meta.len();
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
                bytes_freed += size;
            }
        }
    }
    steps.push(RepairStep {
        name: "清理 7 天前的日志".to_string(),
        status: StepStatus::Ok,
        detail: format!(
            "删除 {removed} 份, 释放 {:.1} MB",
            bytes_freed as f64 / 1024.0 / 1024.0
        ),
    });
    Ok(RepairOutcome {
        success: true,
        pending_reboot: false,
        summary: format!("已清理 {removed} 份旧日志"),
        steps,
        backup_dir: None,
    })
}

// ===== Step 构造辅助 =====

#[cfg(windows)]
fn make_step(name: &str, target: &std::path::Path, status: StepStatus) -> RepairStep {
    RepairStep {
        name: name.to_string(),
        status,
        detail: target.display().to_string(),
    }
}

#[cfg(windows)]
fn boolean_removal_step(name: &str, before: bool, after: bool, label: &str) -> RepairStep {
    let (status, detail) = match (before, after) {
        (false, _) => (StepStatus::Skipped, format!("{label}本就不存在")),
        (true, false) => (StepStatus::Ok, format!("{label}已删除")),
        (true, true) => (StepStatus::Failed, format!("{label}仍存在")),
    };
    RepairStep {
        name: name.to_string(),
        status,
        detail,
    }
}

#[cfg(windows)]
fn removal_step(
    name: &str,
    before: usize,
    after: usize,
    detail_fn: impl Fn(usize, usize) -> String,
) -> RepairStep {
    let status = if before == 0 {
        StepStatus::Skipped
    } else if after == 0 {
        StepStatus::Ok
    } else {
        // 残留数 > 0：无论比之前少多少都视为失败，没法保证下次安装走干净
        StepStatus::Failed
    };
    RepairStep {
        name: name.to_string(),
        status,
        detail: detail_fn(before, after),
    }
}

#[cfg(windows)]
fn pnp_sys_step(before: bool, after: bool) -> RepairStep {
    let (status, detail) = match (before, after) {
        (false, _) => (StepStatus::Skipped, "驱动文件本就不存在".to_string()),
        (true, false) => (StepStatus::Ok, "驱动文件已由 PnP 移除".to_string()),
        // pnputil 走完后 sys 仍在：通常是 PnP 还在异步释放设备实例，重启即可
        (true, true) => (
            StepStatus::PendingReboot,
            "驱动文件仍占用，将在重启后由 PnP 完成清理".to_string(),
        ),
    };
    RepairStep {
        name: "驱动文件 ddhid63340.sys".to_string(),
        status,
        detail,
    }
}

// ===== 单元测试：纯逻辑路径（ps_* 测试已迁至 win-driver::powershell）=====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residue_severity_matrix() {
        // 残留 + sys 在 → 正常
        assert_eq!(classify_residue_severity(true, true), Severity::Info);
        // 残留 + sys 缺 → 阻塞重装，升级为 Error
        assert_eq!(classify_residue_severity(true, false), Severity::Error);
        // 无残留 → Info（无论 sys 是否在）
        assert_eq!(classify_residue_severity(false, true), Severity::Info);
        assert_eq!(classify_residue_severity(false, false), Severity::Info);
    }

    #[test]
    fn residue_status_matrix() {
        assert_eq!(classify_residue_status(true, true), ItemStatus::Ok);
        assert_eq!(classify_residue_status(true, false), ItemStatus::Orphan);
        assert_eq!(classify_residue_status(false, true), ItemStatus::Ok);
        assert_eq!(classify_residue_status(false, false), ItemStatus::Ok);
    }

    #[test]
    fn timestamp_slug_format() {
        let s = timestamp_slug();
        // 形如 20260527-153045，长度固定 15
        assert_eq!(s.len(), 15);
        assert!(s.chars().nth(8) == Some('-'));
        assert!(s.chars().filter(|c| c.is_ascii_digit()).count() == 14);
    }

    #[test]
    fn ymd_from_days_known_dates() {
        // 1970-01-01 → 0 days
        assert_eq!(ymd_from_days(0), (1970, 1, 1));
        // 2000-01-01 = 10957 days
        assert_eq!(ymd_from_days(10957), (2000, 1, 1));
        // 2026-05-27（约）：手动算，不要求和今天对齐到秒，只验算法
        // 2026-01-01 是 ymd_from_days(20454)；可以反推单日偏移
        let d_2026_01_01 = ymd_from_days(20454);
        assert_eq!(d_2026_01_01, (2026, 1, 1));
    }
}
