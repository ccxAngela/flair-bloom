//! Interception 驱动通道。

#![cfg(windows)]

use crate::SIM_MARKER;
use interception_sys::*;
use qzh_profile::key_id::MouseButton;
use std::os::raw::c_uint;
use tracing::{error, info, warn};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, MAPVK_VK_TO_VSC_EX};

pub struct InterceptionBackend {
    ctx: InterceptionContext,
    keyboard_device: InterceptionDevice,
    mouse_device: Option<InterceptionDevice>,
}

// SAFETY: InterceptionContext 是 Interception DLL 内部分配的不透明指针，文档允许跨线程使用
unsafe impl Send for InterceptionBackend {}
// SAFETY: 同上
unsafe impl Sync for InterceptionBackend {}

impl InterceptionBackend {
    pub fn new() -> Option<Self> {
        // SAFETY: interception_create_context 失败返回 null
        let ctx = unsafe { interception_create_context() };
        if ctx.is_null() {
            return None;
        }
        let slots = hardware_ids_in(ctx);
        let Some(keyboard_device) = first_device(&slots, 1..=10) else {
            // SAFETY: ctx 由本次 create 返回，未转交给后端。
            unsafe { interception_destroy_context(ctx) };
            error!("Interception: 没有能读取硬件 ID 的键盘设备");
            return None;
        };
        let mouse_device = first_device(&slots, 11..=20);
        if mouse_device.is_none() {
            warn!("Interception: 没有能读取硬件 ID 的鼠标设备");
        }
        info!(
            "Interception 后端初始化成功，键盘 ID: {}, 鼠标 ID: {:?}",
            keyboard_device, mouse_device
        );
        Some(Self {
            ctx,
            keyboard_device,
            mouse_device,
        })
    }

    pub fn send_key(&self, vk: u32, is_up: bool) -> bool {
        // SAFETY: MapVirtualKeyW 对任意 u32 安全
        let scan_ex = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX) };
        let scan = (scan_ex & 0xFF) as u16;
        if scan == 0 {
            return false;
        }
        let prefix = (scan_ex >> 8) & 0xFF;
        let mut state: u16 = if is_up {
            InterceptionKeyState_INTERCEPTION_KEY_UP as u16
        } else {
            InterceptionKeyState_INTERCEPTION_KEY_DOWN as u16
        };
        if prefix == 0xE0 {
            state |= InterceptionKeyState_INTERCEPTION_KEY_E0 as u16;
        }
        let stroke = InterceptionKeyStroke {
            code: scan,
            state,
            information: SIM_MARKER as c_uint,
        };
        // SAFETY: ctx/keyboard_device 已验证；stroke 在调用期间在栈上
        unsafe {
            interception_send(
                self.ctx,
                self.keyboard_device,
                &stroke as *const InterceptionKeyStroke as *const InterceptionStroke,
                1,
            ) == 1
        }
    }

    /// 注入滚轮事件。Interception WHEEL state=0x0400，rolling 使用 Windows WHEEL_DELTA=120。
    pub fn send_wheel(&self, up: bool) -> bool {
        let Some(mouse_device) = self.mouse_device else {
            return false;
        };
        let rolling: i16 = if up { 120 } else { -120 };
        let stroke = InterceptionMouseStroke {
            state: 0x0400, // INTERCEPTION_MOUSE_WHEEL
            flags: 0,
            rolling,
            x: 0,
            y: 0,
            information: SIM_MARKER as c_uint,
        };
        // SAFETY: ctx/mouse_device 已验证；stroke 在调用期间在栈上。
        // 返回实际写入的 stroke 数：==1 才算成功，失败时上层会回退 SendInput 而非静默丢弃。
        unsafe {
            interception_send(
                self.ctx,
                mouse_device,
                &stroke as *const InterceptionMouseStroke as *const InterceptionStroke,
                1,
            ) == 1
        }
    }

    pub fn send_mouse(&self, button: MouseButton, is_up: bool) -> bool {
        let Some(mouse_device) = self.mouse_device else {
            return false;
        };
        let state: u16 = match (button, is_up) {
            (MouseButton::Left, false) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_LEFT_BUTTON_DOWN as u16
            }
            (MouseButton::Left, true) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_LEFT_BUTTON_UP as u16
            }
            (MouseButton::Right, false) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_RIGHT_BUTTON_DOWN as u16
            }
            (MouseButton::Right, true) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_RIGHT_BUTTON_UP as u16
            }
            (MouseButton::Middle, false) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_MIDDLE_BUTTON_DOWN as u16
            }
            (MouseButton::Middle, true) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_MIDDLE_BUTTON_UP as u16
            }
            (MouseButton::X1, false) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_BUTTON_4_DOWN as u16
            }
            (MouseButton::X1, true) => InterceptionMouseState_INTERCEPTION_MOUSE_BUTTON_4_UP as u16,
            (MouseButton::X2, false) => {
                InterceptionMouseState_INTERCEPTION_MOUSE_BUTTON_5_DOWN as u16
            }
            (MouseButton::X2, true) => InterceptionMouseState_INTERCEPTION_MOUSE_BUTTON_5_UP as u16,
            // WheelUp/WheelDown 由 dispatch 提前路由到 send_wheel，不应到达此处
            (MouseButton::WheelUp | MouseButton::WheelDown, _) => unreachable!(),
        };
        let stroke = InterceptionMouseStroke {
            state,
            flags: 0,
            rolling: 0,
            x: 0,
            y: 0,
            information: SIM_MARKER as c_uint,
        };
        // SAFETY: ctx/mouse_device 已验证；stroke 在调用期间在栈上。
        // 返回实际写入的 stroke 数：==1 才算成功，失败时上层会回退 SendInput 而非静默丢弃。
        unsafe {
            interception_send(
                self.ctx,
                mouse_device,
                &stroke as *const InterceptionMouseStroke as *const InterceptionStroke,
                1,
            ) == 1
        }
    }
}

impl Drop for InterceptionBackend {
    fn drop(&mut self) {
        // SAFETY: ctx 是 new() 中 create_context 返回的唯一句柄
        unsafe { interception_destroy_context(self.ctx) };
        info!("Interception context 已销毁");
    }
}

fn first_device(slots: &[(i32, String)], range: std::ops::RangeInclusive<i32>) -> Option<i32> {
    slots
        .iter()
        .find(|(id, _)| range.contains(id))
        .map(|(id, _)| *id)
}

/// Interception 20 个设备槽各自挂的硬件 ID，空槽不列；槽号划分见 `hidstick::KEYBOARD_SLOTS`。
/// 只读、不需要管理员；驱动没装或没加载时返回 `None`。
///
/// 槽位按设备出现的先后往后分，拔掉的设备不还槽，重启前不回收：每类满 10 个之后再插进来的
/// 设备分不到槽，它的输入会被驱动吞掉（设备管理器里却显示正常）。诊断靠这里看出这件事。
pub fn device_hardware_ids() -> Option<Vec<(i32, String)>> {
    // SDK 独占打开设备，诊断必须复用正在使用的 context，不能把本应用的占用误判为驱动失效。
    let cell = crate::INTERCEPTION_BACKEND.get_or_init(|| std::sync::Mutex::new(None));
    let guard = crate::revive(cell.lock());
    if let Some(backend) = guard.as_ref() {
        return Some(hardware_ids_in(backend.ctx));
    }
    // SAFETY: interception_create_context 失败返回 null
    let ctx = unsafe { interception_create_context() };
    if ctx.is_null() {
        return None;
    }
    let out = hardware_ids_in(ctx);
    // SAFETY: ctx 由上面 create 成功返回，此后不再使用。
    unsafe { interception_destroy_context(ctx) };
    Some(out)
}

fn hardware_ids_in(ctx: InterceptionContext) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    for device in 1..=20 {
        let mut buf = [0u16; 512];
        // SAFETY: ctx 有效；buf 按字节长度传入，驱动最多写这么多，返回写入（或需要）的字节数
        let written = unsafe {
            interception_get_hardware_id(
                ctx,
                device,
                buf.as_mut_ptr().cast(),
                (buf.len() * 2) as c_uint,
            )
        };
        let len = (written as usize / 2).min(buf.len());
        let id = String::from_utf16_lossy(&buf[..len]);
        // 硬件 ID 是多字符串，第一段最具体
        let first = id.split('\0').next().unwrap_or_default();
        if !first.is_empty() {
            out.push((device, first.to_string()));
        }
    }
    out
}

/// 检测 Interception 驱动是否已安装（尝试创建 context）。
pub fn is_driver_installed() -> bool {
    device_hardware_ids().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_selection_skips_empty_slots_and_separates_device_classes() {
        let slots = vec![(4, "keyboard".into()), (13, "mouse".into())];
        assert_eq!(first_device(&slots, 1..=10), Some(4));
        assert_eq!(first_device(&slots, 11..=20), Some(13));
        assert_eq!(first_device(&[], 1..=10), None);
        assert_eq!(first_device(&slots[..1], 11..=20), None);
    }
}
