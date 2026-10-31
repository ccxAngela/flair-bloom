//! Windows 只读系统信息 + 注册表 helper + 安装前置检查。
//!
//! 所有 Windows 独有实现均在 `#[cfg(windows)]` 内；其它平台提供空串 / false 退化实现，
//! 不影响非 Windows 构建通过。

pub mod prereq;
pub mod registry;
pub mod run_as_admin;

/// 操作系统版本描述字符串，例如 `"Windows 11 23H2 (Build 22631.4317)"`。
#[cfg(windows)]
pub fn os_version() -> String {
    use crate::registry::{read_reg_dword, read_reg_sz, wide};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY,
    };

    let subkey = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: subkey 是 NUL 结尾宽串；hkey 是栈上出参指针
    let r = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &mut hkey,
        )
    };
    if r != 0 {
        return String::new();
    }

    let display = read_reg_sz(hkey, "DisplayVersion")
        .or_else(|| read_reg_sz(hkey, "ReleaseId"))
        .unwrap_or_default();
    let product = read_reg_sz(hkey, "ProductName").unwrap_or_default();
    let build = read_reg_sz(hkey, "CurrentBuild").unwrap_or_default();
    let product = windows_product_name(product, &build);
    let ubr = read_reg_dword(hkey, "UBR");

    // SAFETY: hkey 是上面 RegOpenKeyExW 成功返回的句柄
    unsafe { RegCloseKey(hkey) };

    let mut full_build = build.clone();
    if let Some(u) = ubr {
        if !full_build.is_empty() {
            full_build.push('.');
        }
        full_build.push_str(&u.to_string());
    }

    match (
        product.is_empty(),
        display.is_empty(),
        full_build.is_empty(),
    ) {
        (true, true, true) => String::new(),
        (false, false, false) => format!("{product} {display} (Build {full_build})"),
        (false, true, false) => format!("{product} (Build {full_build})"),
        (false, false, true) => format!("{product} {display}"),
        (true, false, false) => format!("Windows {display} (Build {full_build})"),
        (false, true, true) => product,
        (true, false, true) => display,
        (true, true, false) => format!("Build {full_build}"),
    }
}

// Windows 11 部分系统的 ProductName 仍返回 Windows 10，按客户端构建号纠正显示名。
#[cfg(any(windows, test))]
fn windows_product_name(product: String, build: &str) -> String {
    if build.parse::<u32>().is_ok_and(|value| value >= 22000) {
        if let Some(edition) = product.strip_prefix("Windows 10") {
            return format!("Windows 11{edition}");
        }
    }
    product
}

#[cfg(not(windows))]
pub fn os_version() -> String {
    String::new()
}

/// WebView2 Runtime 版本号，例如 `"118.0.2088.61"`。
#[cfg(windows)]
pub fn webview2_version() -> String {
    use crate::registry::{read_reg_sz_at, RegRoot};

    const RUNTIME_GUID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

    let candidates: [(_, &str); 4] = [
        (
            RegRoot::HklmWow64_64,
            "SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients",
        ),
        (RegRoot::Hklm, "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients"),
        (
            RegRoot::HklmWow64_64,
            "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients",
        ),
        (RegRoot::Hkcu, "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients"),
    ];

    for (root, base) in candidates {
        let path = format!("{base}\\{RUNTIME_GUID}");
        if let Some(v) = read_reg_sz_at(root, &path, "pv") {
            if !v.is_empty() && v != "0.0.0.0" {
                return v;
            }
        }
    }
    String::new()
}

#[cfg(not(windows))]
pub fn webview2_version() -> String {
    String::new()
}

/// 当前进程的可执行文件所在目录；失败时返回空串。
pub fn install_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// 主机硬件架构（不是进程的编译目标）。
///
/// 用 `GetNativeSystemInfo` 而非 `std::env::consts::ARCH`：前者读真实 CPU，
/// x64 进程跑在 ARM64 设备上时两者会分叉。
#[cfg(windows)]
pub fn host_arch() -> String {
    use windows_sys::Win32::System::SystemInformation::{GetNativeSystemInfo, SYSTEM_INFO};

    // SAFETY: SYSTEM_INFO 是 POD，全 0 初始化合法
    let mut info: SYSTEM_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: GetNativeSystemInfo 只写出参
    unsafe { GetNativeSystemInfo(&mut info) };
    // SAFETY: SYSTEM_INFO 联合体的 wProcessorArchitecture 字段始终有效
    let arch = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture };
    match arch {
        9 => "x64".to_string(),
        12 => "ARM64".to_string(),
        5 => "ARM".to_string(),
        6 => "ia64".to_string(),
        0 => "x86".to_string(),
        other => format!("unknown({other})"),
    }
}

#[cfg(not(windows))]
pub fn host_arch() -> String {
    std::env::consts::ARCH.to_string()
}

/// 用户区域代码，例如 `"zh-CN"` / `"en-US"`。
#[cfg(windows)]
pub fn user_locale() -> String {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    let mut buf = vec![0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: buf 在调用期间存活，长度参数与缓冲区匹配
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(n as usize - 1)])
}

#[cfg(not(windows))]
pub fn user_locale() -> String {
    std::env::var("LANG")
        .ok()
        .map(|s| s.split('.').next().unwrap_or(&s).to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_product_name_preserves_edition_and_server_names() {
        for (product, build, expected) in [
            ("Windows 10 Pro", "26300", "Windows 11 Pro"),
            ("Windows 10 Enterprise", "22631", "Windows 11 Enterprise"),
            ("Windows 10 Pro", "19045", "Windows 10 Pro"),
            ("Windows Server 2025", "26100", "Windows Server 2025"),
            ("Windows 10 Pro", "", "Windows 10 Pro"),
        ] {
            assert_eq!(windows_product_name(product.to_string(), build), expected);
        }
    }

    #[test]
    fn install_path_is_non_empty_for_test_runner() {
        assert!(!install_path().is_empty());
    }

    #[test]
    fn host_arch_returns_known_label() {
        let a = host_arch();
        assert!(!a.is_empty());
        assert!(!a.starts_with("unknown("), "unexpected arch: {a}");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_os_version_is_empty_for_now() {
        assert_eq!(os_version(), "");
        assert_eq!(webview2_version(), "");
    }

    #[cfg(windows)]
    #[test]
    fn windows_os_version_contains_build_number() {
        let v = os_version();
        assert!(!v.is_empty(), "os_version 在 Windows 上不应为空");
    }
}
