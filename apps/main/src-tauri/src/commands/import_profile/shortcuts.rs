//! 只读解析已知按键工具的快捷方式，不解析或运行目标程序。

use std::path::{Path, PathBuf};

pub(super) fn config_dirs(path: &Path) -> Vec<PathBuf> {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !name.ends_with(".lnk")
        || !["极简", "小叽", "xiaojiautokey", "丐帮", "衍天"]
            .iter()
            .any(|known| name.contains(known))
    {
        return Vec::new();
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED, STGM_READ,
        };
        use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_RAWPATH};
        // SAFETY: 配对管理本次线程的 COM 引用，不改变已有 apartment。
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let result = (|| -> windows::core::Result<Vec<PathBuf>> {
            // SAFETY: 仅创建系统 ShellLink 解析器，Load 不执行快捷方式目标，且不调用 Resolve。
            let link: IShellLinkW =
                unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)? };
            let file: IPersistFile = link.cast()?;
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            unsafe { file.Load(PCWSTR(wide.as_ptr()), STGM_READ)? };
            let mut target = vec![0u16; 32768];
            let mut working = vec![0u16; 32768];
            unsafe { link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)? };
            unsafe { link.GetWorkingDirectory(&mut working)? };
            let decode = |buf: &[u16]| {
                PathBuf::from(String::from_utf16_lossy(
                    &buf[..buf.iter().position(|c| *c == 0).unwrap_or(buf.len())],
                ))
            };
            let target = decode(&target);
            let working = decode(&working);
            Ok(target
                .parent()
                .filter(|p| p.is_absolute())
                .map(Path::to_path_buf)
                .into_iter()
                .chain(working.is_absolute().then_some(working))
                .collect())
        })();
        if initialized {
            // SAFETY: 仅归还本次成功初始化（含 S_FALSE）增加的 COM 引用。
            unsafe { CoUninitialize() };
        }
        result.unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

    #[test]
    fn known_shortcut_locates_config_directory_without_running_target() {
        let directory = tempfile::tempdir().unwrap();
        let target_dir = directory.path().join("按键 配置");
        std::fs::create_dir(&target_dir).unwrap();
        let shortcut = directory.path().join("极简按键.lnk");
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        // SAFETY: 测试只创建临时快捷方式，不创建或执行目标程序，成功初始化后配对释放 COM。
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        {
            let link: IShellLinkW =
                unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap() };
            let target = wide(&target_dir.join("synthetic.exe"));
            let working = wide(&target_dir);
            unsafe { link.SetPath(PCWSTR(target.as_ptr())).unwrap() };
            unsafe { link.SetWorkingDirectory(PCWSTR(working.as_ptr())).unwrap() };
            let file: IPersistFile = link.cast().unwrap();
            let shortcut_wide = wide(&shortcut);
            unsafe { file.Save(PCWSTR(shortcut_wide.as_ptr()), true).unwrap() };
        }
        let found = config_dirs(&shortcut);
        if initialized {
            unsafe { CoUninitialize() };
        }
        assert!(found.contains(&target_dir));
        assert!(config_dirs(&directory.path().join("其他工具.lnk")).is_empty());
    }
}
