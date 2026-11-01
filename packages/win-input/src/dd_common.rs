//! ddxoft DD SDK 的 FFI 装载层，供 [`crate::ddsimple`] 使用。
//!
//! 设计要点：
//! - DLL 在运行时通过 `LoadLibraryW` 加载，避免编译期链接到不存在的导入库；
//! - DD 协议要求首次调用 `DD_btn(0)`，返回 `1` 才表示内核驱动已就绪；
//! - DD-HID 63340 的 X1/X2 侧键需要使用专用状态字节补丁；
//! - DDSimple 的驱动路径使用 `MOUSE_INPUT_DATA.ButtonFlags`，侧键可直传 64/128/256/512。

#![cfg(windows)]

use qzh_profile::key_id::MouseButton;
use std::ffi::c_void;
use std::os::raw::c_int;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, warn};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

type DdBtnFn = unsafe extern "C" fn(c_int) -> c_int;
type DdKeyFn = unsafe extern "C" fn(c_int, c_int) -> c_int;
type DdTodcFn = unsafe extern "C" fn(c_int) -> c_int;
type DdWhlFn = unsafe extern "C" fn(c_int) -> c_int;

pub(super) fn dd_key_was_submitted(ret: c_int) -> bool {
    // 只有负值是在 DLL 调用驱动前返回的错误哨兵。0 也可能已产生输入；
    // 此时撤销回灌过滤并补发 SendInput，会让同键规则把自身输出当成停止操作。
    ret >= 0
}

fn simple_side_button_flag(button: MouseButton, is_up: bool) -> c_int {
    match (button, is_up) {
        (MouseButton::X1, false) => 64,
        (MouseButton::X1, true) => 128,
        (MouseButton::X2, false) => 256,
        (MouseButton::X2, true) => 512,
        _ => unreachable!(),
    }
}

fn dd_wheel_code(up: bool) -> c_int {
    if up {
        1
    } else {
        2
    }
}

pub struct DdFfi {
    handle: HMODULE,
    dd_btn: DdBtnFn,
    dd_key: DdKeyFn,
    dd_todc: DdTodcFn,
    dd_whl: Option<DdWhlFn>,
    diag_logged: AtomicBool,
    mouse_diag_logged: AtomicBool,
    side_btn_diag_logged: AtomicBool,
    wheel_diag_logged: AtomicBool,
}

// SAFETY: HMODULE 在 64 位 Windows 上是地址不变的内核句柄
unsafe impl Send for DdFfi {}
// SAFETY: 同上
unsafe impl Sync for DdFfi {}

impl DdFfi {
    pub fn load(dll_path: &Path) -> Option<Self> {
        if !dll_path.exists() {
            warn!("DD DLL 不存在：{}", dll_path.display());
            return None;
        }
        let wide: Vec<u16> = dll_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: wide 以 NUL 结尾
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            warn!("LoadLibraryW 失败：{}", dll_path.display());
            return None;
        }
        // SAFETY: handle 有效，符号名 NUL 结尾，类型与 DLL 文档一致
        let dd_btn = match unsafe { resolve::<DdBtnFn>(handle, b"DD_btn\0") } {
            Some(f) => f,
            None => {
                unsafe { FreeLibrary(handle) };
                warn!("DD DLL 缺少 DD_btn 导出");
                return None;
            }
        };
        let dd_key = match unsafe { resolve::<DdKeyFn>(handle, b"DD_key\0") } {
            Some(f) => f,
            None => {
                unsafe { FreeLibrary(handle) };
                warn!("DD DLL 缺少 DD_key 导出");
                return None;
            }
        };
        let dd_todc = match unsafe { resolve::<DdTodcFn>(handle, b"DD_todc\0") } {
            Some(f) => f,
            None => {
                unsafe { FreeLibrary(handle) };
                warn!("DD DLL 缺少 DD_todc 导出");
                return None;
            }
        };
        // SAFETY: dd_btn 已解析，DD_btn(0) 是协议规定的自检调用
        let st = unsafe { dd_btn(0) };
        if st != 1 {
            warn!("DD_btn(0) 返回 {}，驱动未就绪", st);
            unsafe { FreeLibrary(handle) };
            return None;
        }
        // DD_whl 是可选扩展：两个已知 DLL 版本均有此导出，旧版不报错仅降级
        let dd_whl = unsafe { resolve::<DdWhlFn>(handle, b"DD_whl\0") };
        if dd_whl.is_none() {
            warn!("DD DLL 缺少 DD_whl 导出，滚轮将回退 SendInput");
        }
        Some(DdFfi {
            handle,
            dd_btn,
            dd_key,
            dd_todc,
            dd_whl,
            diag_logged: AtomicBool::new(false),
            mouse_diag_logged: AtomicBool::new(false),
            side_btn_diag_logged: AtomicBool::new(false),
            wheel_diag_logged: AtomicBool::new(false),
        })
    }

    pub fn send_key(&self, vk: u32, is_up: bool) -> bool {
        // SAFETY: dd_todc 已解析
        let ddcode = unsafe { (self.dd_todc)(vk as c_int) };
        let first = !self.diag_logged.swap(true, Ordering::SeqCst);
        if ddcode == 0 {
            if first {
                warn!("DD_todc({:#x}) 返回 0，VK 无映射，键已丢弃", vk);
            }
            return false;
        }
        let flag = if is_up { 2 } else { 1 };
        // SAFETY: dd_key 已解析
        let ret = unsafe { (self.dd_key)(ddcode, flag) };
        let os_error = if ret == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        if first {
            info!(
                "DD 首次注入：vk={:#x} ddcode={} flag={} ret={} os_error={}",
                vk, ddcode, flag, ret, os_error
            );
            if ret == 0 {
                warn!("DD_key 返回 0，注入结果不确定：保留回灌过滤，不重复补发 SendInput");
            }
        }
        dd_key_was_submitted(ret)
    }

    /// 注入滚轮事件。`up=true` 时向上，`up=false` 时向下。
    /// 返回 `true` 表示 DD 通道已处理，`false` 表示需回退 SendInput。
    pub fn send_wheel(&self, up: bool) -> bool {
        let Some(dd_whl) = self.dd_whl else {
            return false;
        };
        // DD SDK: 1 = wheel up, 2 = wheel down. Passing -1 sets unrelated
        // MOUSE_INPUT_DATA button bits in DDSimple and can trigger XBUTTON1.
        let delta = dd_wheel_code(up);
        // SAFETY: dd_whl 已解析
        let ret = unsafe { dd_whl(delta) };
        // 反汇编 dd63330.dll 确认：DD_whl 注入路径恒返回固定值（滚轮分支无失败回报，
        // fire-and-forget），ret 不携带成功/失败信息，判 `ret == 1` 之类没有意义。
        // 故 ret 仅用于首次诊断日志，滚轮一律视为已由 DD 通道处理（返回 true，不回退 SendInput）。
        if !self.wheel_diag_logged.swap(true, Ordering::SeqCst) {
            info!("DD 首次滚轮注入：up={} ret={}", up, ret);
        }
        true
    }

    pub fn send_mouse(&self, button: MouseButton, is_up: bool) -> bool {
        match button {
            MouseButton::X1 | MouseButton::X2 => {
                let flag = simple_side_button_flag(button, is_up);
                // SAFETY: dd_btn 已解析；DDSimple 内嵌驱动按 MOUSE_INPUT_DATA.ButtonFlags
                // 解释该字段，静态确认支持 X1/X2 down/up flag。
                let ret = unsafe { (self.dd_btn)(flag) };
                if !self.side_btn_diag_logged.swap(true, Ordering::SeqCst) {
                    info!(
                        "DD Simple 首次侧键注入：button={:?} is_up={} flag={} ret={}",
                        button, is_up, flag, ret
                    );
                }
                true
            }
            _ => {
                let flag: c_int = match (button, is_up) {
                    (MouseButton::Left, false) => 1,
                    (MouseButton::Left, true) => 2,
                    (MouseButton::Right, false) => 4,
                    (MouseButton::Right, true) => 8,
                    (MouseButton::Middle, false) => 16,
                    (MouseButton::Middle, true) => 32,
                    // WheelUp/WheelDown 由 dispatch 提前路由到 send_wheel，不应到达此处
                    (MouseButton::WheelUp | MouseButton::WheelDown, _) => unreachable!(),
                    (MouseButton::X1 | MouseButton::X2, _) => unreachable!(),
                };
                // SAFETY: dd_btn 已解析
                let ret = unsafe { (self.dd_btn)(flag) };
                if !self.mouse_diag_logged.swap(true, Ordering::SeqCst) {
                    info!(
                        "DD 首次鼠标注入：button={:?} is_up={} flag={} ret={}",
                        button, is_up, flag, ret
                    );
                }
                true
            }
        }
    }
}

impl Drop for DdFfi {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle 是 load() 中 LoadLibraryW 返回的唯一句柄
            unsafe { FreeLibrary(self.handle) };
        }
    }
}

unsafe fn resolve<T: Copy>(handle: HMODULE, name_with_nul: &[u8]) -> Option<T> {
    debug_assert!(name_with_nul.last() == Some(&0));
    debug_assert!(std::mem::size_of::<T>() == std::mem::size_of::<*const c_void>());
    // SAFETY: handle 存活、字符串 NUL 结尾
    let p = GetProcAddress(handle, name_with_nul.as_ptr());
    p.map(|f| std::mem::transmute_copy::<_, T>(&f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_side_button_flags_follow_mouse_input_data_values() {
        assert_eq!(simple_side_button_flag(MouseButton::X1, false), 64);
        assert_eq!(simple_side_button_flag(MouseButton::X1, true), 128);
        assert_eq!(simple_side_button_flag(MouseButton::X2, false), 256);
        assert_eq!(simple_side_button_flag(MouseButton::X2, true), 512);
    }

    #[test]
    fn dd_wheel_codes_follow_sdk_values() {
        assert_eq!(dd_wheel_code(true), 1);
        assert_eq!(dd_wheel_code(false), 2);
    }
}
