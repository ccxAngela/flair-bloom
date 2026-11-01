//! Windows 输入注入子系统：SendInput / Interception / DDSimple / 外设驱动（外设模式）四档后端。
//!
//! 统一入口：[`key_down`] / [`key_up`]（跨平台 stub）与 [`dispatch`]（Windows only）。
//! 非 Windows 平台提供空实现，不影响编译。

pub mod interception_health;

#[cfg(windows)]
mod dd_common;
#[cfg(windows)]
pub mod ddsimple;
#[cfg(any(test, windows))]
pub mod hidstick;
#[cfg(windows)]
pub mod interception;

#[cfg(windows)]
use ddsimple::DdSimpleBackend;
#[cfg(windows)]
use hidstick::{HidSend, HidStickBackend};
#[cfg(windows)]
use interception::InterceptionBackend;
use qzh_profile::key_id::KeyId;
#[cfg(any(test, windows))]
use qzh_profile::key_id::MouseButton;
#[cfg(any(test, windows))]
use std::collections::{HashMap, VecDeque};
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::atomic::AtomicBool;
#[cfg(any(test, windows))]
use std::sync::{Mutex, OnceLock};
#[cfg(any(test, windows))]
use std::time::{Duration, Instant};
#[cfg(windows)]
use tracing::{info, warn};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN,
    MOUSEEVENTF_XUP, MOUSEINPUT,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{XBUTTON1, XBUTTON2};

/// 写入 SendInput 的 dwExtraInfo，hook 据此过滤程序自身模拟的按键，消除竞态。
pub const SIM_MARKER: usize = 0x5148_5844;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputEvent {
    pub key: KeyId,
    pub is_up: bool,
}

impl InputEvent {
    pub fn down(key: KeyId) -> Self {
        Self { key, is_up: false }
    }

    pub fn up(key: KeyId) -> Self {
        Self { key, is_up: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchResult {
    Sent,
    Noop,
    Failed,
    FallbackSent,
}

impl DispatchResult {
    pub fn was_sent(self) -> bool {
        matches!(self, Self::Sent | Self::FallbackSent)
    }
}

#[cfg(any(test, windows))]
const SIM_TTL: Duration = Duration::from_millis(50);
#[cfg(any(test, windows))]
const RELAY_TTL: Duration = Duration::from_millis(200);

#[cfg(any(test, windows))]
type PendingMap = HashMap<(KeyId, bool), VecDeque<Instant>>;
#[cfg(any(test, windows))]
static PENDING_INJECTIONS: OnceLock<Mutex<PendingMap>> = OnceLock::new();
#[cfg(any(test, windows))]
static RELAY_INJECTIONS: OnceLock<Mutex<PendingMap>> = OnceLock::new();

#[cfg(any(test, windows))]
fn pending_map() -> &'static Mutex<PendingMap> {
    PENDING_INJECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(any(test, windows))]
fn relay_map() -> &'static Mutex<PendingMap> {
    RELAY_INJECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(any(test, windows))]
fn revive<T>(r: std::sync::LockResult<T>) -> T {
    r.unwrap_or_else(|e| e.into_inner())
}

#[cfg(any(test, windows))]
fn drop_expired(queue: &mut VecDeque<Instant>, now: Instant, ttl: Duration) {
    while let Some(&front) = queue.front() {
        if now.duration_since(front) > ttl {
            queue.pop_front();
        } else {
            break;
        }
    }
}

#[cfg(any(test, windows))]
fn record_injection_in(map: &'static Mutex<PendingMap>, key: KeyId, is_up: bool, ttl: Duration) {
    let mut map = revive(map.lock());
    let queue = map.entry((key, is_up)).or_default();
    let now = Instant::now();
    drop_expired(queue, now, ttl);
    queue.push_back(now);
}

/// DD 系列（DD-HID / DDSimple）注入的**唯一**自注入过滤依据：注入前登记 `(key, is_up)`，
/// hook 回灌时 `try_consume_injection` 配平消费。
///
/// 坑（务必知悉，勿重蹈）：DD 驱动把 `ExtraInformation` 写死为 0，注入事件回到 LL hook 时
/// `dwExtraInfo` 恒为 0，`SIM_MARKER` **无法幸存**——故 DD 路径既不设置也不依赖 SIM。
/// 不要再为 DD 加任何「直写 SIM」的旁路（历史上的 `dd_direct` DeviceIoControl 旁路已移除，
/// 缘由见 `ddsimple.rs` 顶部注释）。
///
/// 这套时间窗口队列是**尽力而为、不保证 100%**：对同键规则（`trigger == target`）无法可靠
/// 区分「用户真实按键」与「自身注入回灌」，高负载 / 丢事件时会偶发误判（连发自停或停不掉）。
/// DD 同键 Toggle / Hold 均接受该不可靠性，由全局关闭与正常退出时的停连发、抬键清理兜底。
/// 彻底消除需 Raw Input 按来源区分 DD 虚拟设备与物理设备，非当前范围。
#[cfg(any(test, windows))]
fn record_injection(key: KeyId, is_up: bool) {
    record_injection_in(pending_map(), key, is_up, SIM_TTL);
}

#[cfg(any(test, windows))]
fn record_relay_injection(key: KeyId, is_up: bool) {
    record_injection_in(relay_map(), key, is_up, RELAY_TTL);
}

#[cfg(windows)]
fn try_send_dd_key(key: KeyId, is_up: bool, send: impl FnOnce() -> bool) -> bool {
    record_injection(key, is_up);
    if send() {
        record_relay_injection(key, is_up);
        true
    } else {
        // 只有明确未提交给驱动时才撤销登记，让调用方安全回退。
        try_consume_injection(key, is_up);
        false
    }
}

#[cfg(any(test, windows))]
fn try_consume_from(
    map: &'static Mutex<PendingMap>,
    key: KeyId,
    is_up: bool,
    ttl: Duration,
) -> bool {
    let mut map = revive(map.lock());
    let Some(queue) = map.get_mut(&(key, is_up)) else {
        return false;
    };
    drop_expired(queue, Instant::now(), ttl);
    queue.pop_front().is_some()
}

/// hook 端调用：若该 (KeyId, is_up) 对应有未过期的 sim 记录，pop 一条并返回 true。
#[cfg(any(test, windows))]
pub fn try_consume_injection(key: KeyId, is_up: bool) -> bool {
    try_consume_from(pending_map(), key, is_up, SIM_TTL)
}

#[cfg(all(not(windows), not(test)))]
pub fn try_consume_injection(_key: KeyId, _is_up: bool) -> bool {
    false
}

/// WebView relay 端调用：若该事件是应用刚注入后由 DOM 回灌的模拟事件，pop 一条并返回 true。
#[cfg(any(test, windows))]
pub fn try_consume_relay_injection(key: KeyId, is_up: bool) -> bool {
    try_consume_from(relay_map(), key, is_up, RELAY_TTL)
}

#[cfg(all(not(windows), not(test)))]
pub fn try_consume_relay_injection(_key: KeyId, _is_up: bool) -> bool {
    false
}

/// 清空注入队列（引擎重置规则或关闭时调用）。
#[cfg(any(test, windows))]
pub fn clear_pending_injections() {
    if let Some(lock) = PENDING_INJECTIONS.get() {
        revive(lock.lock()).clear();
    }
}

#[cfg(all(not(windows), not(test)))]
pub fn clear_pending_injections() {}

#[cfg(any(test, windows))]
pub fn clear_relay_injections() {
    if let Some(lock) = RELAY_INJECTIONS.get() {
        revive(lock.lock()).clear();
    }
}

#[cfg(all(not(windows), not(test)))]
pub fn clear_relay_injections() {}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    #[default]
    SendInput,
    Interception,
    #[serde(rename = "ddsimple")]
    DdSimple,
    #[serde(rename = "hidstick")]
    HidStick,
}

#[cfg(windows)]
impl InputMode {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "sendinput" => Some(Self::SendInput),
            "interception" => Some(Self::Interception),
            "ddsimple" | "dd_simple" => Some(Self::DdSimple),
            "hidstick" => Some(Self::HidStick),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SendInput => "sendinput",
            Self::Interception => "interception",
            Self::DdSimple => "ddsimple",
            Self::HidStick => "hidstick",
        }
    }

    /// 是否限制 Toggle 目标键与启动 / 停止键相同。
    /// DD 自注入只能靠时间窗过滤，同键启停可能误判；产品接受此不可靠性，由全局关闭与正常退出清理兜底。
    /// 外设驱动同样依赖时间窗过滤，目前仍保留同键限制。
    pub fn requires_distinct_target_for_toggle(&self) -> bool {
        matches!(self, Self::HidStick)
    }

    /// 两档驱动要管理员权限；SendInput 与外设驱动（普通串口）不要。
    pub fn requires_admin(&self) -> bool {
        matches!(self, Self::Interception | Self::DdSimple)
    }
}

#[cfg(windows)]
static INTERCEPTION_BACKEND: OnceLock<Mutex<Option<InterceptionBackend>>> = OnceLock::new();
#[cfg(windows)]
#[cfg(windows)]
static DD_SIMPLE_BACKEND: OnceLock<Mutex<Option<DdSimpleBackend>>> = OnceLock::new();
#[cfg(windows)]
static HIDSTICK_BACKEND: OnceLock<Mutex<Option<HidStickBackend>>> = OnceLock::new();
#[cfg(windows)]
type OfflineListener = Box<dyn Fn() + Send + Sync>;
#[cfg(windows)]
static HIDSTICK_OFFLINE_LISTENER: OnceLock<OfflineListener> = OnceLock::new();
#[cfg(windows)]
static CURRENT_MODE: OnceLock<std::sync::atomic::AtomicU8> = OnceLock::new();
#[cfg(windows)]
static RESOURCES_DIR: OnceLock<PathBuf> = OnceLock::new();

#[cfg(windows)]
static DD_KEY_DOWN_LOGGED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static DD_KEY_UP_LOGGED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static DD_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static INTERCEPTION_MOUSE_FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
static HIDSTICK_UNMAPPED_LOGGED: AtomicBool = AtomicBool::new(false);
/// 外设模式下按下改走了 SendInput 的键（外设驱动拒收或键不在 HID 键盘页）。它们的松开也必须走
/// SendInput：松开发给外设驱动会回 OK，系统却永远收不到这次抬起，键就卡住了。
#[cfg(windows)]
static HIDSTICK_SENDINPUT_DOWN: OnceLock<Mutex<std::collections::HashSet<KeyId>>> = OnceLock::new();

#[cfg(any(test, windows))]
const MODE_SENDINPUT: u8 = 0;
#[cfg(any(test, windows))]
const MODE_INTERCEPTION: u8 = 1;
#[cfg(any(test, windows))]
const MODE_DD_SIMPLE: u8 = 3;
#[cfg(any(test, windows))]
const MODE_HIDSTICK: u8 = 4;

#[cfg(windows)]
fn u8_to_mode(v: u8) -> InputMode {
    match v {
        MODE_INTERCEPTION => InputMode::Interception,
        MODE_DD_SIMPLE => InputMode::DdSimple,
        MODE_HIDSTICK => InputMode::HidStick,
        _ => InputMode::SendInput,
    }
}

#[cfg(any(test, windows))]
fn is_wheel_button(button: MouseButton) -> bool {
    matches!(button, MouseButton::WheelUp | MouseButton::WheelDown)
}

#[cfg(any(test, windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchRoute {
    SendInput,
    Noop,
    InterceptionKeyboard(u32),
    InterceptionWheel { up: bool },
    InterceptionMouse(MouseButton),
    DdSimpleKeyboard(u32),
    DdSimpleWheel { up: bool },
    DdSimpleMouse(MouseButton),
    HidStickKeyboard(u32),
    HidStickWheel { up: bool },
    HidStickMouse(MouseButton),
}

#[cfg(any(test, windows))]
fn resolve_route(mode: u8, key: KeyId, is_up: bool) -> DispatchRoute {
    match (mode, key) {
        (MODE_INTERCEPTION, KeyId::Keyboard(vk)) => DispatchRoute::InterceptionKeyboard(vk),
        (MODE_INTERCEPTION, KeyId::Mouse(btn)) if is_wheel_button(btn) => {
            if is_up {
                DispatchRoute::Noop
            } else {
                DispatchRoute::InterceptionWheel {
                    up: matches!(btn, MouseButton::WheelUp),
                }
            }
        }
        (MODE_INTERCEPTION, KeyId::Mouse(btn)) => DispatchRoute::InterceptionMouse(btn),
        (MODE_DD_SIMPLE, KeyId::Keyboard(vk)) => DispatchRoute::DdSimpleKeyboard(vk),
        (MODE_DD_SIMPLE, KeyId::Mouse(btn)) if is_wheel_button(btn) => {
            if is_up {
                DispatchRoute::Noop
            } else {
                DispatchRoute::DdSimpleWheel {
                    up: matches!(btn, MouseButton::WheelUp),
                }
            }
        }
        (MODE_DD_SIMPLE, KeyId::Mouse(btn)) => DispatchRoute::DdSimpleMouse(btn),
        (MODE_HIDSTICK, KeyId::Keyboard(vk)) => DispatchRoute::HidStickKeyboard(vk),
        (MODE_HIDSTICK, KeyId::Mouse(btn)) if is_wheel_button(btn) => {
            if is_up {
                DispatchRoute::Noop
            } else {
                DispatchRoute::HidStickWheel {
                    up: matches!(btn, MouseButton::WheelUp),
                }
            }
        }
        (MODE_HIDSTICK, KeyId::Mouse(btn)) => DispatchRoute::HidStickMouse(btn),
        _ => DispatchRoute::SendInput,
    }
}

/// 注册资源目录（供 DD DLL 定位）。
#[cfg(windows)]
pub fn set_resources_dir(dir: PathBuf) {
    let _ = RESOURCES_DIR.set(dir);
}

#[cfg(windows)]
pub fn init_backend(mode: InputMode) {
    let current = CURRENT_MODE.get_or_init(|| std::sync::atomic::AtomicU8::new(MODE_SENDINPUT));
    DD_KEY_DOWN_LOGGED.store(false, std::sync::atomic::Ordering::SeqCst);
    DD_KEY_UP_LOGGED.store(false, std::sync::atomic::Ordering::SeqCst);
    DD_FALLBACK_LOGGED.store(false, std::sync::atomic::Ordering::SeqCst);
    INTERCEPTION_MOUSE_FALLBACK_LOGGED.store(false, std::sync::atomic::Ordering::SeqCst);
    HIDSTICK_UNMAPPED_LOGGED.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some(set) = HIDSTICK_SENDINPUT_DOWN.get() {
        revive(set.lock()).clear();
    }
    // 外设驱动独占一个串口，切去任何一档（含重新选外设模式）都先关掉旧连接
    release_hidstick();
    release_inactive_driver_backends(mode);

    match mode {
        InputMode::Interception => {
            let backend_cell = INTERCEPTION_BACKEND.get_or_init(|| Mutex::new(None));
            let mut guard = revive(backend_cell.lock());
            // 切换前已停连发并释放按键；重建 context，避免复用拔插前的设备编号。
            *guard = None;
            *guard = InterceptionBackend::new();
            if guard.is_some() {
                current.store(MODE_INTERCEPTION, std::sync::atomic::Ordering::SeqCst);
                info!("输入后端已切换为 Interception 驱动模式");
            } else {
                current.store(MODE_SENDINPUT, std::sync::atomic::Ordering::SeqCst);
                warn!("Interception 驱动未安装，降级为 SendInput 模式");
            }
        }
        InputMode::DdSimple => {
            let Some(dir) = RESOURCES_DIR.get() else {
                warn!("DD Simple 切换失败：资源目录未注册");
                current.store(MODE_SENDINPUT, std::sync::atomic::Ordering::SeqCst);
                return;
            };
            let cell = DD_SIMPLE_BACKEND.get_or_init(|| Mutex::new(None));
            let mut guard = revive(cell.lock());
            if guard.is_none() {
                *guard = DdSimpleBackend::new(dir);
            }
            if guard.is_some() {
                current.store(MODE_DD_SIMPLE, std::sync::atomic::Ordering::SeqCst);
                info!("输入后端已切换为 DD Simple 模式");
            } else {
                current.store(MODE_SENDINPUT, std::sync::atomic::Ordering::SeqCst);
                warn!("DD Simple 加载失败，降级为 SendInput 模式");
            }
        }
        InputMode::HidStick => {
            let cell = HIDSTICK_BACKEND.get_or_init(|| Mutex::new(None));
            // 插拔监视可能正握着同一个口（启动时它在后台握第一轮），打不开就稍等再试一次
            let opened = HidStickBackend::open(hidstick_went_offline).or_else(|_| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                HidStickBackend::open(hidstick_went_offline)
            });
            match opened {
                Ok(backend) => {
                    *revive(cell.lock()) = Some(backend);
                    current.store(MODE_HIDSTICK, std::sync::atomic::Ordering::SeqCst);
                    info!("输入后端已切换为外设模式");
                }
                Err(e) => {
                    current.store(MODE_SENDINPUT, std::sync::atomic::Ordering::SeqCst);
                    warn!("外设模式不可用（{e}），降级为 SendInput 模式");
                }
            }
        }
        InputMode::SendInput => {
            current.store(MODE_SENDINPUT, std::sync::atomic::Ordering::SeqCst);
            info!("输入后端已切换为 SendInput 模式");
        }
    }
}

/// 只读探测 DD 设备是否被占用，不加载 SDK、不安装驱动、不发送按键。
#[cfg(windows)]
pub fn dd_channel_status() -> (bool, String) {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
        ERROR_SHARING_VIOLATION, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    };
    let cell = DD_SIMPLE_BACKEND.get_or_init(|| Mutex::new(None));
    // 探测期间持锁，避免临时独占句柄与本应用正在初始化的 SDK 竞争。
    let backend = revive(cell.lock());
    if backend.is_some() {
        return (false, "DD 通道由本应用使用。切换至其他模式或正常退出会卸载本应用加载的 SDK；SDK 内部和系统驱动状态不等同于 DLL 引用状态。".into());
    }
    let path: Vec<u16> = r"\\.\dd63330".encode_utf16().chain(Some(0)).collect();
    // SAFETY: path NUL 结尾；仅打开设备，不发 IOCTL；share=0 与随包 SDK 一致。
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0x80000000,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        // SAFETY: handle 来自成功的 CreateFileW，探测结束立即归还。
        unsafe { CloseHandle(handle) };
        return (
            false,
            "DD 设备可打开，探测句柄已释放；尚未加载 SDK，不能据此保证 DD 初始化或游戏兼容性。"
                .into(),
        );
    }
    // SAFETY: 紧随失败的 CreateFileW 读取当前线程错误。
    match unsafe { GetLastError() } {
        ERROR_SHARING_VIOLATION => (true, "DD 设备被占用。请正常退出本应用及其他使用 DD 的程序后重试；卸载 DLL 不保证第三方 SDK 内部的句柄已经关闭。".into()),
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => (false, "未发现 DD 63330 运行设备；SDK 尚未初始化时可能正常，不表示需要清理 DDHID 63340。".into()),
        ERROR_ACCESS_DENIED => (true, "DD 设备访问被拒绝。请检查管理员权限、其他 DD 程序占用和系统安全策略。".into()),
        code => (true, format!("DD 设备探测失败（Win32 错误码 {code}），无法确认是否被占用。")),
    }
}

/// 调用方须先停止调度并经旧后端释放按键，再释放不再使用的驱动通道。
#[cfg(windows)]
fn release_inactive_driver_backends(next: InputMode) {
    if next != InputMode::DdSimple {
        if let Some(cell) = DD_SIMPLE_BACKEND.get() {
            if revive(cell.lock()).take().is_some() {
                info!("DD Simple DLL 已卸载，不再保留本应用的后端引用");
            }
        }
    }
    if next != InputMode::Interception {
        if let Some(cell) = INTERCEPTION_BACKEND.get() {
            if revive(cell.lock()).take().is_some() {
                info!("Interception context 已释放");
            }
        }
    }
}

/// 停调度、释放按键后调用。OnceLock 中的静态后端不会在正常退出时自动析构。
#[cfg(windows)]
pub fn release_backends() {
    release_hidstick();
    release_inactive_driver_backends(InputMode::SendInput);
    if let Some(mode) = CURRENT_MODE.get() {
        mode.store(MODE_SENDINPUT, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(windows)]
fn release_hidstick() {
    // 先出锁再 drop：drop 要 join 心跳线程，最多等一次回复超时
    let old = HIDSTICK_BACKEND
        .get()
        .and_then(|lock| revive(lock.lock()).take());
    drop(old);
}

/// 释放已断开的后端，关掉它占着的口：不关的话外设驱动插回来常拿到同一个口名，却因旧句柄还在
/// 而打不开，插拔监视握不上手，「插回再问」就不弹。只放已离线的那个，免得期间用户已经切回外设
/// 模式、新开的后端被误放。
#[cfg(windows)]
fn release_offline_hidstick() {
    let old = HIDSTICK_BACKEND.get().and_then(|lock| {
        let mut guard = revive(lock.lock());
        match guard.as_ref() {
            Some(backend) if !backend.is_online() => guard.take(),
            _ => None,
        }
    });
    drop(old);
}

/// 外设驱动断开（拔出、写失败、心跳无应答）时调用，可能在调度线程或心跳线程上。回退模式后
/// 另起线程释放后端：调用方可能正持有后端的锁，在这里放会死锁。
#[cfg(windows)]
fn hidstick_went_offline() {
    if let Some(current) = CURRENT_MODE.get() {
        let _ = current.compare_exchange(
            MODE_HIDSTICK,
            MODE_SENDINPUT,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        );
    }
    clear_pending_injections();
    warn!("外设模式已回退为 SendInput 模式");
    let _ = std::thread::Builder::new()
        .name("hidstick-release".into())
        .spawn(release_offline_hidstick);
    if let Some(listener) = HIDSTICK_OFFLINE_LISTENER.get() {
        listener();
    }
}

/// 外面先发现外设驱动没了（插拔监视看到它的口消失）：走与心跳失败同一条断开路径，
/// 让「回退通用模式」先于「已断开」事件落定，界面拿到的状态才是回退后的。
#[cfg(windows)]
pub fn hidstick_link_down(reason: &str) {
    if let Some(lock) = HIDSTICK_BACKEND.get() {
        if let Some(backend) = revive(lock.lock()).as_ref() {
            backend.link_lost(reason);
        }
    }
}

/// 注册外设驱动断开后的回调（只认第一次注册）。回调跑在调度或心跳线程上，必须立即返回，
/// UI 通知应派发到异步运行时。
#[cfg(windows)]
pub fn set_on_hidstick_offline(f: impl Fn() + Send + Sync + 'static) {
    let _ = HIDSTICK_OFFLINE_LISTENER.set(Box::new(f));
}

/// 外设模式正连着的外设驱动 `(口名, 固件版本)`。链路已断的不算：它占着的口马上会被释放，
/// 插拔监视要能重新握手。
#[cfg(windows)]
pub fn hidstick_in_use() -> Option<(String, String)> {
    let lock = HIDSTICK_BACKEND.get()?;
    let guard = revive(lock.lock());
    guard
        .as_ref()
        .filter(|b| b.is_online())
        .map(|b| (b.port_name().to_string(), b.firmware().to_string()))
}

#[cfg(windows)]
pub fn current_mode() -> InputMode {
    let v = CURRENT_MODE
        .get()
        .map(|a| a.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(MODE_SENDINPUT);
    u8_to_mode(v)
}

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    SendInput,
}

#[cfg(not(windows))]
impl InputMode {
    pub fn requires_distinct_target_for_toggle(&self) -> bool {
        false
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(_s: &str) -> Option<Self> {
        Some(Self::SendInput)
    }

    pub fn as_str(&self) -> &'static str {
        "sendinput"
    }
}

#[cfg(windows)]
unsafe fn send_kbd_via_sendinput(vk: u32, flags: u32) -> bool {
    // SAFETY: MapVirtualKeyW 对任意 u32 安全
    let scan_ex = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX);
    let scan = (scan_ex & 0xFF) as u16;
    let prefix = (scan_ex >> 8) & 0xFF;
    let (w_vk, w_scan, scan_flags) = if scan == 0 || prefix == 0xE1 {
        (vk as u16, 0u16, 0u32)
    } else {
        let ext = if prefix == 0xE0 {
            KEYEVENTF_EXTENDEDKEY
        } else {
            0
        };
        (0u16, scan, KEYEVENTF_SCANCODE | ext)
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: w_vk,
                wScan: w_scan,
                dwFlags: scan_flags | flags,
                time: 0,
                dwExtraInfo: SIM_MARKER,
            },
        },
    };
    // SAFETY: input 是栈上完整初始化的 INPUT_KEYBOARD
    SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1
}

#[cfg(windows)]
unsafe fn send_wheel_via_sendinput(up: bool) -> bool {
    // WHEEL_DELTA = 120 per notch；向下用补码表示负值
    let mouse_data: u32 = if up { 120u32 } else { (-120i32) as u32 };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data,
                dwFlags: MOUSEEVENTF_WHEEL,
                time: 0,
                dwExtraInfo: SIM_MARKER,
            },
        },
    };
    // SAFETY: input 是栈上完整初始化的 INPUT_MOUSE
    SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1
}

#[cfg(windows)]
unsafe fn send_mouse_via_sendinput(button: MouseButton, is_up: bool) -> bool {
    let (dw_flags, mouse_data) = match (button, is_up) {
        (MouseButton::Left, false) => (MOUSEEVENTF_LEFTDOWN, 0),
        (MouseButton::Left, true) => (MOUSEEVENTF_LEFTUP, 0),
        (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTUP, 0),
        (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEUP, 0),
        (MouseButton::X1, false) => (MOUSEEVENTF_XDOWN, XBUTTON1 as i32),
        (MouseButton::X1, true) => (MOUSEEVENTF_XUP, XBUTTON1 as i32),
        (MouseButton::X2, false) => (MOUSEEVENTF_XDOWN, XBUTTON2 as i32),
        (MouseButton::X2, true) => (MOUSEEVENTF_XUP, XBUTTON2 as i32),
        // WheelUp/WheelDown 由 dispatch 提前处理，不应到达此处
        (MouseButton::WheelUp | MouseButton::WheelDown, _) => unreachable!(),
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: 0,
                dy: 0,
                mouseData: mouse_data as u32,
                dwFlags: dw_flags,
                time: 0,
                dwExtraInfo: SIM_MARKER,
            },
        },
    };
    // SAFETY: input 是栈上完整初始化的 INPUT_MOUSE
    SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1
}

pub fn key_down(key: KeyId) {
    let _ = key_event(InputEvent::down(key));
}

pub fn key_up(key: KeyId) {
    let _ = key_event(InputEvent::up(key));
}

pub fn key_event(event: InputEvent) -> DispatchResult {
    #[cfg(windows)]
    {
        dispatch(event.key, event.is_up)
    }
    #[cfg(not(windows))]
    {
        let _ = event;
        DispatchResult::Noop
    }
}

pub fn key_events(events: &[InputEvent]) -> Vec<DispatchResult> {
    events.iter().copied().map(key_event).collect()
}

#[cfg(windows)]
fn send_via_sendinput(key: KeyId, is_up: bool) -> DispatchResult {
    // SAFETY: 各 send_*_via_sendinput 的 Safety 契约由调用方保证
    let sent = match key {
        KeyId::Keyboard(vk) => unsafe {
            let flags = if is_up { KEYEVENTF_KEYUP } else { 0 };
            send_kbd_via_sendinput(vk, flags)
        },
        KeyId::Mouse(MouseButton::WheelUp | MouseButton::WheelDown) => {
            if !is_up {
                let up = matches!(key, KeyId::Mouse(MouseButton::WheelUp));
                unsafe { send_wheel_via_sendinput(up) }
            } else {
                return DispatchResult::Noop;
            }
        }
        KeyId::Mouse(btn) => unsafe { send_mouse_via_sendinput(btn, is_up) },
    };
    if sent {
        record_relay_injection(key, is_up);
        DispatchResult::Sent
    } else {
        DispatchResult::Failed
    }
}

#[cfg(windows)]
fn dispatch(key: KeyId, is_up: bool) -> DispatchResult {
    let mode = CURRENT_MODE
        .get()
        .map(|a| a.load(std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(MODE_SENDINPUT);
    match resolve_route(mode, key, is_up) {
        DispatchRoute::SendInput => send_via_sendinput(key, is_up),
        DispatchRoute::Noop => DispatchResult::Noop,
        DispatchRoute::InterceptionKeyboard(vk) => {
            if let Some(lock) = INTERCEPTION_BACKEND.get() {
                if let Some(backend) = revive(lock.lock()).as_ref() {
                    if backend.send_key(vk, is_up) {
                        record_relay_injection(key, is_up);
                        return DispatchResult::Sent;
                    }
                    return DispatchResult::Failed;
                }
            }
            send_via_sendinput(key, is_up)
        }
        DispatchRoute::InterceptionWheel { up } => {
            if let Some(lock) = INTERCEPTION_BACKEND.get() {
                if let Some(backend) = revive(lock.lock()).as_ref() {
                    if backend.send_wheel(up) {
                        record_relay_injection(key, false);
                        return DispatchResult::Sent;
                    }
                    if !INTERCEPTION_MOUSE_FALLBACK_LOGGED
                        .swap(true, std::sync::atomic::Ordering::SeqCst)
                    {
                        warn!("Interception 未识别鼠标设备，滚轮回退 SendInput");
                    }
                }
            }
            let result = if unsafe { send_wheel_via_sendinput(up) } {
                DispatchResult::FallbackSent
            } else {
                DispatchResult::Failed
            };
            if result.was_sent() {
                record_relay_injection(key, false);
            }
            result
        }
        DispatchRoute::InterceptionMouse(btn) => {
            if let Some(lock) = INTERCEPTION_BACKEND.get() {
                if let Some(backend) = revive(lock.lock()).as_ref() {
                    if backend.send_mouse(btn, is_up) {
                        record_relay_injection(key, is_up);
                        return DispatchResult::Sent;
                    }
                    if !INTERCEPTION_MOUSE_FALLBACK_LOGGED
                        .swap(true, std::sync::atomic::Ordering::SeqCst)
                    {
                        warn!("Interception 模式但未识别鼠标设备，鼠标连发回退 SendInput");
                    }
                }
            }
            send_via_sendinput(key, is_up)
        }
        DispatchRoute::DdSimpleKeyboard(vk) => {
            let mut backend_seen = false;
            if let Some(lock) = DD_SIMPLE_BACKEND.get() {
                if let Some(backend) = revive(lock.lock()).as_ref() {
                    backend_seen = true;
                    log_dd_route("DD Simple", is_up, key);
                    // dd63330 驱动注入时把 ExtraInformation 写死为 0（内核键盘注入函数
                    // VA 0x140003d13 显式清零），回到 LL hook 时 dwExtraInfo 恒为 0，SIM_MARKER
                    // 无法幸存。故一律先登记 PENDING_INJECTIONS，由时间窗口队列过滤自身注入回灌，
                    // 否则 hook 会把注入误判为物理按键。
                    if try_send_dd_key(key, is_up, || backend.send_key(vk, is_up)) {
                        return DispatchResult::Sent;
                    }
                }
            }
            if !backend_seen && !DD_FALLBACK_LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                warn!("当前模式 DD Simple 但后端不存在，回退 SendInput");
            }
            send_via_sendinput(key, is_up)
        }
        DispatchRoute::DdSimpleWheel { up } => {
            if let Some(lock) = DD_SIMPLE_BACKEND.get() {
                if let Some(backend) = revive(lock.lock()).as_ref() {
                    log_dd_route("DD Simple", false, key);
                    // 同键盘：dd63330 驱动把 ExtraInformation 写死为 0，SIM_MARKER 无法幸存，
                    // 一律登记 PENDING_INJECTIONS 兜底自注入回灌。
                    record_injection(key, false);
                    if backend.send_wheel(up) {
                        record_relay_injection(key, false);
                        return DispatchResult::Sent;
                    }
                    try_consume_injection(key, false);
                }
            }
            if !DD_FALLBACK_LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                warn!("当前模式 DD Simple 但滚轮回退 SendInput");
            }
            let result = if unsafe { send_wheel_via_sendinput(up) } {
                DispatchResult::FallbackSent
            } else {
                DispatchResult::Failed
            };
            if result.was_sent() {
                record_relay_injection(key, false);
            }
            result
        }
        DispatchRoute::DdSimpleMouse(btn) => {
            let mut backend_seen = false;
            if let Some(lock) = DD_SIMPLE_BACKEND.get() {
                if let Some(backend) = revive(lock.lock()).as_ref() {
                    backend_seen = true;
                    log_dd_route("DD Simple", is_up, key);
                    // 同键盘/滚轮：dd63330 驱动把 ExtraInformation 写死为 0，SIM_MARKER 不可用，
                    // 一律登记 PENDING_INJECTIONS 兜底自注入回灌。
                    record_injection(key, is_up);
                    if backend.send_mouse(btn, is_up) {
                        record_relay_injection(key, is_up);
                        return DispatchResult::Sent;
                    }
                    try_consume_injection(key, is_up);
                }
            }
            if !backend_seen && !DD_FALLBACK_LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                warn!("当前模式 DD Simple 但后端不存在，回退 SendInput");
            }
            send_via_sendinput(key, is_up)
        }
        DispatchRoute::HidStickKeyboard(vk) => {
            dispatch_hidstick(key, is_up, |backend| backend.send_key(vk, is_up))
        }
        DispatchRoute::HidStickWheel { up } => {
            dispatch_hidstick(key, false, |backend| backend.send_wheel(up))
        }
        DispatchRoute::HidStickMouse(btn) => {
            dispatch_hidstick(key, is_up, |backend| backend.send_mouse(btn, is_up))
        }
    }
}

/// 外设驱动敲出的事件回到钩子时就是真硬件事件，与 DD 一样先登记 `PENDING_INJECTIONS` 再发；
/// 没发出去就撤销登记、这一条改走 SendInput，不让按键被直接丢掉。按下走了 SendInput 的键，
/// 松开也走 SendInput（见 [`HIDSTICK_SENDINPUT_DOWN`]）。
#[cfg(windows)]
fn dispatch_hidstick(
    key: KeyId,
    is_up: bool,
    send: impl FnOnce(&HidStickBackend) -> HidSend,
) -> DispatchResult {
    let fallback_downs = HIDSTICK_SENDINPUT_DOWN.get_or_init(|| Mutex::new(Default::default()));
    if is_up && revive(fallback_downs.lock()).remove(&key) {
        return fallback_via_sendinput(key, is_up);
    }
    if let Some(lock) = HIDSTICK_BACKEND.get() {
        if let Some(backend) = revive(lock.lock()).as_ref() {
            record_injection(key, is_up);
            match send(backend) {
                HidSend::Sent => {
                    record_relay_injection(key, is_up);
                    return DispatchResult::Sent;
                }
                HidSend::Unmapped => {
                    if !HIDSTICK_UNMAPPED_LOGGED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        warn!("{key:?} 不在 HID 键盘页，外设模式下改走 SendInput");
                    }
                }
                HidSend::Failed => {}
            }
            try_consume_injection(key, is_up);
        }
    }
    let result = fallback_via_sendinput(key, is_up);
    // 滚轮只有「按下」没有松开，不记
    if !is_up && result.was_sent() && !matches!(key, KeyId::Mouse(b) if is_wheel_button(b)) {
        revive(fallback_downs.lock()).insert(key);
    }
    result
}

#[cfg(windows)]
fn fallback_via_sendinput(key: KeyId, is_up: bool) -> DispatchResult {
    match send_via_sendinput(key, is_up) {
        DispatchResult::Sent => DispatchResult::FallbackSent,
        other => other,
    }
}

#[cfg(windows)]
fn log_dd_route(backend_name: &str, is_up: bool, key: KeyId) {
    let logged = if is_up {
        &DD_KEY_UP_LOGGED
    } else {
        &DD_KEY_DOWN_LOGGED
    };
    if !logged.swap(true, std::sync::atomic::Ordering::SeqCst) {
        let dir = if is_up { "key_up" } else { "key_down" };
        info!("{} 路由到 {} 后端（key={:?}）", dir, backend_name, key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(button: MouseButton) -> KeyId {
        KeyId::Mouse(button)
    }

    #[test]
    fn pending_injection_is_consumed_once_and_keeps_direction() {
        let key = KeyId::Keyboard(0x51);

        record_injection(key, false);

        assert!(!try_consume_injection(key, true));
        assert!(try_consume_injection(key, false));
        assert!(!try_consume_injection(key, false));
    }

    #[cfg(windows)]
    #[test]
    fn dd_zero_return_keeps_delayed_hook_and_relay_events_filtered() {
        let key = KeyId::Keyboard(0x87);
        let submitted = try_send_dd_key(key, false, || dd_common::dd_key_was_submitted(0));
        assert!(submitted, "不能重试已经可能产生按键的 DD 调用");
        assert!(try_consume_injection(key, false));
        assert!(try_consume_relay_injection(key, false));
        assert!(!try_consume_injection(key, false));
    }

    #[cfg(windows)]
    #[test]
    fn dd_hook_can_consume_injection_before_sdk_call_returns() {
        let key = KeyId::Keyboard(0x85);
        assert!(try_send_dd_key(key, true, || {
            assert!(try_consume_injection(key, true));
            dd_common::dd_key_was_submitted(0)
        }));
        assert!(!try_consume_injection(key, true));
        assert!(try_consume_relay_injection(key, true));
    }

    #[cfg(windows)]
    #[test]
    fn dd_pre_submission_failure_allows_fallback_without_stale_filter() {
        let key = KeyId::Keyboard(0x86);
        let submitted = try_send_dd_key(key, false, || dd_common::dd_key_was_submitted(-1));
        assert!(!submitted);
        assert!(!try_consume_injection(key, false));
        assert!(!try_consume_relay_injection(key, false));
        assert!(!dd_common::dd_key_was_submitted(-2));
    }

    #[test]
    fn relay_injection_is_consumed_once_and_keeps_direction() {
        let key = KeyId::Keyboard(0x45);

        record_relay_injection(key, false);

        assert!(!try_consume_relay_injection(key, true));
        assert!(try_consume_relay_injection(key, false));
        assert!(!try_consume_relay_injection(key, false));
    }

    #[cfg(windows)]
    #[test]
    fn ddsimple_mode_uses_canonical_string_and_legacy_alias() {
        assert_eq!(InputMode::from_str("ddsimple"), Some(InputMode::DdSimple));
        assert_eq!(InputMode::from_str("dd_simple"), Some(InputMode::DdSimple));
        assert_eq!(InputMode::DdSimple.as_str(), "ddsimple");
    }

    #[cfg(windows)]
    #[test]
    fn non_sendinput_modes_require_admin() {
        assert!(!InputMode::SendInput.requires_admin());
        assert!(InputMode::Interception.requires_admin());
        assert!(InputMode::DdSimple.requires_admin());
        assert!(!InputMode::HidStick.requires_admin());
    }

    #[cfg(windows)]
    #[test]
    fn hidstick_mode_round_trips_its_name() {
        assert_eq!(InputMode::from_str("hidstick"), Some(InputMode::HidStick));
        assert_eq!(InputMode::HidStick.as_str(), "hidstick");
        assert_eq!(u8_to_mode(MODE_HIDSTICK), InputMode::HidStick);
    }

    #[cfg(windows)]
    #[test]
    fn only_hidstick_requires_distinct_target_for_toggle() {
        assert!(!InputMode::DdSimple.requires_distinct_target_for_toggle());
        assert!(InputMode::HidStick.requires_distinct_target_for_toggle());
        assert!(!InputMode::SendInput.requires_distinct_target_for_toggle());
        assert!(!InputMode::Interception.requires_distinct_target_for_toggle());
    }

    #[test]
    fn interception_wheel_down_routes_to_wheel_backend() {
        assert_eq!(
            resolve_route(MODE_INTERCEPTION, mouse(MouseButton::WheelDown), false),
            DispatchRoute::InterceptionWheel { up: false }
        );
    }

    #[test]
    fn interception_wheel_up_routes_to_wheel_backend() {
        assert_eq!(
            resolve_route(MODE_INTERCEPTION, mouse(MouseButton::WheelUp), false),
            DispatchRoute::InterceptionWheel { up: true }
        );
    }

    #[test]
    fn interception_wheel_release_is_noop() {
        assert_eq!(
            resolve_route(MODE_INTERCEPTION, mouse(MouseButton::WheelDown), true),
            DispatchRoute::Noop
        );
        assert_eq!(
            resolve_route(MODE_INTERCEPTION, mouse(MouseButton::WheelUp), true),
            DispatchRoute::Noop
        );
    }

    #[test]
    fn interception_regular_mouse_routes_to_mouse_backend() {
        assert_eq!(
            resolve_route(MODE_INTERCEPTION, mouse(MouseButton::Left), false),
            DispatchRoute::InterceptionMouse(MouseButton::Left)
        );
        assert_eq!(
            resolve_route(MODE_INTERCEPTION, mouse(MouseButton::X2), true),
            DispatchRoute::InterceptionMouse(MouseButton::X2)
        );
    }

    #[test]
    fn ddsimple_keyboard_routes_to_simple_backend() {
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, KeyId::Keyboard(0x51), false),
            DispatchRoute::DdSimpleKeyboard(0x51)
        );
    }

    #[test]
    fn ddsimple_wheel_routes_to_simple_backend() {
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, mouse(MouseButton::WheelDown), false),
            DispatchRoute::DdSimpleWheel { up: false }
        );
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, mouse(MouseButton::WheelUp), false),
            DispatchRoute::DdSimpleWheel { up: true }
        );
    }

    #[test]
    fn ddsimple_wheel_release_is_noop() {
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, mouse(MouseButton::WheelDown), true),
            DispatchRoute::Noop
        );
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, mouse(MouseButton::WheelUp), true),
            DispatchRoute::Noop
        );
    }

    #[test]
    fn ddsimple_regular_mouse_routes_to_simple_backend() {
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, mouse(MouseButton::Left), false),
            DispatchRoute::DdSimpleMouse(MouseButton::Left)
        );
        assert_eq!(
            resolve_route(MODE_DD_SIMPLE, mouse(MouseButton::X2), true),
            DispatchRoute::DdSimpleMouse(MouseButton::X2)
        );
    }

    #[test]
    fn hidstick_routes_keyboard_mouse_and_wheel() {
        assert_eq!(
            resolve_route(MODE_HIDSTICK, KeyId::Keyboard(0x51), true),
            DispatchRoute::HidStickKeyboard(0x51)
        );
        assert_eq!(
            resolve_route(MODE_HIDSTICK, mouse(MouseButton::X1), false),
            DispatchRoute::HidStickMouse(MouseButton::X1)
        );
        assert_eq!(
            resolve_route(MODE_HIDSTICK, mouse(MouseButton::WheelUp), false),
            DispatchRoute::HidStickWheel { up: true }
        );
        assert_eq!(
            resolve_route(MODE_HIDSTICK, mouse(MouseButton::WheelDown), true),
            DispatchRoute::Noop
        );
    }

    #[test]
    fn sendinput_mode_uses_sendinput_route() {
        assert_eq!(
            resolve_route(MODE_SENDINPUT, KeyId::Keyboard(0x51), false),
            DispatchRoute::SendInput
        );
        assert_eq!(
            resolve_route(MODE_SENDINPUT, mouse(MouseButton::WheelDown), false),
            DispatchRoute::SendInput
        );
    }
}
