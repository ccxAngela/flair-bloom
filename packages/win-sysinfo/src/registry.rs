//! Windows 注册表辅助：读取注册表值、检查键是否存在、服务键判断。

/// 把 `&str` 编码为 NUL 结尾的 UTF-16 宽字串。
#[cfg(windows)]
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 注册表根键枚举。
#[cfg(windows)]
#[derive(Clone, Copy)]
pub enum RegRoot {
    /// HKLM，默认视图（32 位进程会被重定向到 WOW6432Node）
    Hklm,
    /// HKLM，强制 64 位视图
    HklmWow64_64,
    /// HKCU
    Hkcu,
}

/// 读取指定根键 + 子键路径下的 REG_SZ / REG_EXPAND_SZ 值。
#[cfg(windows)]
pub fn read_reg_sz_at(root: RegRoot, subkey: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        KEY_WOW64_64KEY,
    };

    let (hroot, flags) = match root {
        RegRoot::Hklm => (HKEY_LOCAL_MACHINE, KEY_READ),
        RegRoot::HklmWow64_64 => (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_64KEY),
        RegRoot::Hkcu => (HKEY_CURRENT_USER, KEY_READ),
    };
    let wsub = wide(subkey);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: 出参 hkey 有效，wsub NUL 结尾
    let r = unsafe { RegOpenKeyExW(hroot, wsub.as_ptr(), 0, flags, &mut hkey) };
    if r != 0 {
        return None;
    }
    let v = read_reg_sz(hkey, name);
    // SAFETY: hkey 是上面成功返回的句柄
    unsafe { RegCloseKey(hkey) };
    v
}

/// 从已打开的键句柄读取 REG_SZ / REG_EXPAND_SZ 值。
#[cfg(windows)]
pub(crate) fn read_reg_sz(
    hkey: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegQueryValueExW, REG_EXPAND_SZ, REG_SZ};

    let wname = wide(name);
    let mut ty: u32 = 0;
    let mut len: u32 = 0;
    // 第一次：探测长度
    // SAFETY: hkey 有效；wname NUL 结尾；data 传 null 仅探测大小
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            wname.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if r != 0 || (ty != REG_SZ && ty != REG_EXPAND_SZ) || len == 0 {
        return None;
    }

    let cap = (len as usize).div_ceil(2);
    let mut buf = vec![0u16; cap];
    let mut len2 = len;
    // SAFETY: buf 容量 >= len2 字节
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            wname.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut len2,
        )
    };
    if r != 0 {
        return None;
    }
    let chars = (len2 as usize).div_ceil(2);
    let slice = &buf[..chars.min(buf.len())];
    let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    Some(String::from_utf16_lossy(&slice[..end]))
}

/// 从已打开的键句柄读取 REG_DWORD 值。
#[cfg(windows)]
pub(crate) fn read_reg_dword(
    hkey: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegQueryValueExW, REG_DWORD};

    let wname = wide(name);
    let mut ty: u32 = 0;
    let mut data: u32 = 0;
    let mut len: u32 = std::mem::size_of::<u32>() as u32;
    // SAFETY: data 是栈上 u32，len 与之匹配
    let r = unsafe {
        RegQueryValueExW(
            hkey,
            wname.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            &mut data as *mut u32 as *mut u8,
            &mut len,
        )
    };
    if r == 0 && ty == REG_DWORD {
        Some(data)
    } else {
        None
    }
}

/// 读取 HKLM 下指定子键的 DWORD 值。
///
/// 用于 HVCI / SAC / Pending Reboot 等只看一个 DWORD 标志的检测。
#[cfg(windows)]
pub fn read_hklm_dword(subkey: &str, name: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };
    let wpath = wide(subkey);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: wpath NUL 结尾；hkey 是栈上出参指针
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wpath.as_ptr(), 0, KEY_READ, &mut hkey) };
    if r != 0 {
        return None;
    }
    let v = read_reg_dword(hkey, name);
    // SAFETY: hkey 上面 open 成功
    unsafe { RegCloseKey(hkey) };
    v
}

/// HKLM 子键是否存在（仅判存，不关心内容）。
#[cfg(windows)]
pub fn hklm_subkey_present(subkey: &str) -> bool {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };
    let wpath = wide(subkey);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: wpath NUL 结尾；hkey 是栈上出参指针
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wpath.as_ptr(), 0, KEY_READ, &mut hkey) };
    if r != 0 {
        return false;
    }
    // SAFETY: 上面 RegOpenKeyExW 成功
    unsafe { RegCloseKey(hkey) };
    true
}

/// `SYSTEM\CurrentControlSet\Services\<name>` 服务键是否存在。
#[cfg(windows)]
pub fn service_key_present(name: &str) -> bool {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };
    let path = format!("SYSTEM\\CurrentControlSet\\Services\\{name}");
    let wpath = wide(&path);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: wpath NUL 结尾；hkey 是栈上出参指针
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wpath.as_ptr(), 0, KEY_READ, &mut hkey) };
    if r != 0 {
        return false;
    }
    // SAFETY: 上面 RegOpenKeyExW 成功
    unsafe { RegCloseKey(hkey) };
    true
}

/// 读取服务键 `ImagePath`（REG_SZ / REG_EXPAND_SZ），返回小写形式。
#[cfg(windows)]
pub fn read_service_image_path(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
    };
    let path = format!("SYSTEM\\CurrentControlSet\\Services\\{name}");
    let wpath = wide(&path);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: wpath NUL 结尾；hkey 是栈上出参指针
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, wpath.as_ptr(), 0, KEY_READ, &mut hkey) };
    if r != 0 {
        return None;
    }
    let value_name = wide("ImagePath");
    let mut buf: [u16; 1024] = [0; 1024];
    let mut size: u32 = (buf.len() * 2) as u32;
    let mut ty: u32 = 0;
    // SAFETY: hkey 已 open，buf/size/ty 都是栈上出参
    let q = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        )
    };
    // SAFETY: hkey 上面 open 成功
    unsafe { RegCloseKey(hkey) };
    if q != 0 {
        return None;
    }
    let chars = (size as usize).saturating_div(2);
    let trimmed: Vec<u16> = buf
        .iter()
        .take(chars)
        .copied()
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&trimmed).to_lowercase())
}

/// 服务键名 + 期望的驱动文件名后缀同时满足，才视为 Interception 服务。
#[cfg(windows)]
pub fn is_interception_service(name: &str, expected_sys: &str) -> bool {
    if !service_key_present(name) {
        return false;
    }
    match read_service_image_path(name) {
        Some(p) => {
            let needle = format!("\\{expected_sys}");
            p.ends_with(&needle) || p.ends_with(expected_sys)
        }
        None => false,
    }
}

/// 读取当前内核键鼠类设备编号。编号可因拔插持续增长，不能用当前在线设备数量代替最高编号。
#[cfg(windows)]
pub fn input_class_indices(class: &str) -> Result<Vec<u32>, String> {
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE,
    };
    let path = wide(&format!(r"HARDWARE\DEVICEMAP\{class}"));
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: path NUL 结尾，key 是栈上出参。
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(Vec::new());
    }
    if result != ERROR_SUCCESS {
        return Err(format!("无法读取 {class} 设备编号（错误码 {result}）"));
    }
    let mut indices = Vec::new();
    let mut index = 0;
    let result = loop {
        let mut name = [0u16; 1024];
        let mut len = name.len() as u32;
        // SAFETY: key 有效；name 容量由 len 指定，其余字段不读取。
        let result = unsafe {
            RegEnumValueW(
                key,
                index,
                name.as_mut_ptr(),
                &mut len,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if result == ERROR_NO_MORE_ITEMS {
            break Ok(indices);
        }
        if result != ERROR_SUCCESS {
            break Err(format!("无法枚举 {class} 设备编号（错误码 {result}）"));
        }
        let name = String::from_utf16_lossy(&name[..len as usize]);
        if let Some(id) = input_class_index(&name, class) {
            indices.push(id);
        }
        index += 1;
    };
    // SAFETY: key 来自成功的 RegOpenKeyExW。
    unsafe { RegCloseKey(key) };
    result
}

#[cfg(any(windows, test))]
fn input_class_index(name: &str, class: &str) -> Option<u32> {
    let prefix = format!(r"\Device\{class}");
    name.strip_prefix(&prefix)?.parse().ok()
}

/// 在指定根键下写入 REG_SZ 值，子键不存在则创建。
#[cfg(windows)]
pub fn write_reg_sz(root: RegRoot, subkey: &str, name: &str, value: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_SET_VALUE, KEY_WOW64_64KEY, REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    let (hroot, flags) = match root {
        RegRoot::Hklm => (HKEY_LOCAL_MACHINE, KEY_SET_VALUE),
        RegRoot::HklmWow64_64 => (HKEY_LOCAL_MACHINE, KEY_SET_VALUE | KEY_WOW64_64KEY),
        RegRoot::Hkcu => (HKEY_CURRENT_USER, KEY_SET_VALUE),
    };
    let wsub = wide(subkey);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: wsub NUL 结尾；hkey 是栈上出参；class / security / disposition 传 null 表示默认
    let r = unsafe {
        RegCreateKeyExW(
            hroot,
            wsub.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            flags,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };
    if r != 0 {
        return Err(format!("打开注册表项失败（错误码 {r}）"));
    }

    let wname = wide(name);
    let wvalue = wide(value);
    let bytes = std::mem::size_of_val(wvalue.as_slice()) as u32;
    // SAFETY: hkey 有效；wname / wvalue 均 NUL 结尾；bytes 就是 wvalue 的字节数
    let r = unsafe {
        RegSetValueExW(
            hkey,
            wname.as_ptr(),
            0,
            REG_SZ,
            wvalue.as_ptr() as *const u8,
            bytes,
        )
    };
    // SAFETY: hkey 是上面成功返回的句柄
    unsafe { RegCloseKey(hkey) };
    if r != 0 {
        return Err(format!("写入注册表值失败（错误码 {r}）"));
    }
    Ok(())
}

/// 删除指定根键下的一个值。值或子键本就不存在时视为成功。
#[cfg(windows)]
pub fn delete_reg_value(root: RegRoot, subkey: &str, name: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_SET_VALUE, KEY_WOW64_64KEY,
    };

    let (hroot, flags) = match root {
        RegRoot::Hklm => (HKEY_LOCAL_MACHINE, KEY_SET_VALUE),
        RegRoot::HklmWow64_64 => (HKEY_LOCAL_MACHINE, KEY_SET_VALUE | KEY_WOW64_64KEY),
        RegRoot::Hkcu => (HKEY_CURRENT_USER, KEY_SET_VALUE),
    };
    let wsub = wide(subkey);
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: wsub NUL 结尾；hkey 是栈上出参
    let r = unsafe { RegOpenKeyExW(hroot, wsub.as_ptr(), 0, flags, &mut hkey) };
    if r == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    if r != 0 {
        return Err(format!("打开注册表项失败（错误码 {r}）"));
    }

    let wname = wide(name);
    // SAFETY: hkey 有效；wname NUL 结尾
    let r = unsafe { RegDeleteValueW(hkey, wname.as_ptr()) };
    // SAFETY: hkey 是上面成功返回的句柄
    unsafe { RegCloseKey(hkey) };
    if r != 0 && r != ERROR_FILE_NOT_FOUND {
        return Err(format!("删除注册表值失败（错误码 {r}）"));
    }
    Ok(())
}

#[cfg(test)]
mod input_class_tests {
    use super::*;

    #[test]
    fn class_indices_include_overflow_even_with_few_devices() {
        assert_eq!(
            input_class_index(r"\Device\KeyboardClass10", "KeyboardClass"),
            Some(10)
        );
        assert_eq!(
            input_class_index(r"\Device\PointerClass12", "PointerClass"),
            Some(12)
        );
        assert_eq!(
            input_class_index(r"\Device\KeyboardClass9", "KeyboardClass"),
            Some(9)
        );
        assert_eq!(
            input_class_index(r"\Device\PointerClass0", "KeyboardClass"),
            None
        );
        assert_eq!(
            input_class_index(r"\Device\KeyboardClassX", "KeyboardClass"),
            None
        );
    }
}
