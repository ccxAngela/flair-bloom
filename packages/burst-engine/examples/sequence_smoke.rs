//! Windows 队列真机冒烟：`cargo run -p burst-engine --example sequence_smoke -- sendinput`。
//! 可选 interception、ddsimple、hidstick；捕获并吞掉 E／R 输出，不向前台窗口打字。

#[cfg(windows)]
mod windows {
    use burst_engine::BurstEngine;
    use qzh_profile::{BurstMode, BurstRule, KeyId, OutputStep};
    use std::collections::HashMap;
    use std::ptr::null_mut;
    use std::sync::{mpsc, Mutex, OnceLock};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};
    use win_input::InputMode;
    use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
        UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_KEYUP, WM_QUIT,
        WM_SYSKEYUP,
    };

    #[derive(Clone, Copy)]
    struct Captured {
        key: u32,
        up: bool,
        at: Instant,
    }

    static EVENTS: OnceLock<Mutex<Vec<Captured>>> = OnceLock::new();

    fn captured() -> &'static Mutex<Vec<Captured>> {
        EVENTS.get_or_init(|| Mutex::new(Vec::new()))
    }

    unsafe extern "system" fn capture(code: i32, wparam: WPARAM, lparam: LPARAM) -> isize {
        if code >= 0 {
            // SAFETY: Windows 在 code>=0 时提供有效的 KBDLLHOOKSTRUCT。
            let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
            if [69, 82].contains(&event.vkCode)
                && (event.dwExtraInfo == win_input::SIM_MARKER
                    || win_input::try_consume_injection(KeyId::Keyboard(event.vkCode), up))
            {
                captured()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(Captured {
                        key: event.vkCode,
                        up,
                        at: Instant::now(),
                    });
                return 1;
            }
        }
        // SAFETY: 非测试键继续正常下传，文档允许空 hook 句柄。
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }

    struct Receiver {
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
    }

    impl Receiver {
        fn start() -> Self {
            let (tx, rx) = mpsc::channel();
            let thread = thread::spawn(move || {
                // SAFETY: 回调符合低级键盘 hook 契约，消息队列在就绪通知前创建。
                unsafe {
                    let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(capture), null_mut(), 0);
                    if hook.is_null() {
                        return;
                    }
                    let mut message: MSG = std::mem::zeroed();
                    PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                    if tx.send(GetCurrentThreadId()).is_ok() {
                        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {}
                    }
                    UnhookWindowsHookEx(hook);
                }
            });
            Self {
                thread_id: rx.recv().expect("键盘接收器安装失败"),
                thread: Some(thread),
            }
        }
    }

    impl Drop for Receiver {
        fn drop(&mut self) {
            // SAFETY: 仅通知本例创建的消息线程退出。
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub fn run() {
        let mode_name = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "sendinput".into());
        let mode = InputMode::from_str(&mode_name).expect("未知输入模式");
        win_input::set_resources_dir(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/main/src-tauri/resources"),
        );
        win_input::init_backend(mode);
        assert_eq!(
            win_input::current_mode(),
            mode,
            "目标后端不可用，不能以回退冒充通过"
        );
        let _receiver = Receiver::start();
        let engine = BurstEngine::new();
        let steps = [(69, 100), (69, 150), (82, 200)];
        engine.set_rules(vec![BurstRule {
            id: "sequence-smoke".into(),
            enabled: true,
            trigger_key: KeyId::Keyboard(124),
            steps: steps
                .iter()
                .map(|&(key, interval_ms)| OutputStep {
                    key: KeyId::Keyboard(key),
                    interval_ms,
                })
                .collect(),
            mode: BurstMode::Toggle,
            stop_key: None,
            group: None,
        }]);
        engine.set_global_enabled(true, true);
        engine.on_key_press(KeyId::Keyboard(124));
        engine.on_key_release(KeyId::Keyboard(124));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let downs = captured()
                .lock()
                .unwrap()
                .iter()
                .filter(|event| !event.up)
                .count();
            if downs >= 9 || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        println!(
            "停止前进度：{:?}，故障：{:?}",
            engine.get_rule_progress(),
            engine.output_error()
        );
        engine.set_global_enabled(false, true);
        thread::sleep(Duration::from_millis(80));
        let events = captured().lock().unwrap().clone();
        let downs: Vec<_> = events.iter().filter(|event| !event.up).collect();
        if let Some(first) = events.first() {
            for event in &events {
                println!(
                    "{} {} {}ms",
                    event.key,
                    if event.up { "up" } else { "down" },
                    event.at.duration_since(first.at).as_millis()
                );
            }
        }
        assert!(downs.len() >= 9, "应至少完整观察三轮 E→E→R");
        for (index, event) in downs.iter().enumerate() {
            assert_eq!(event.key, steps[index % 3].0, "步骤顺序错误");
            if index > 0 {
                assert!(
                    event.at.duration_since(downs[index - 1].at)
                        >= Duration::from_millis(u64::from(steps[(index - 1) % 3].1)),
                    "后续步骤提前输出"
                );
            }
        }
        let mut held = HashMap::new();
        for event in &events {
            if event.up {
                assert_eq!(held.remove(&event.key), Some(true), "出现无对应按下的抬起");
            } else {
                assert!(held.insert(event.key, true).is_none(), "点按重叠");
            }
        }
        assert!(held.is_empty(), "全局停止后仍有按键未释放");
        let stopped_count = events.len();
        thread::sleep(Duration::from_millis(250));
        assert_eq!(
            captured().lock().unwrap().len(),
            stopped_count,
            "停止后仍在输出"
        );
        engine.shutdown();
        println!(
            "{mode_name}：三轮 E→E→R 顺序、最短间隔、完整抬起和全局停止通过；输出已由接收器吞掉"
        );
    }
}

fn main() {
    #[cfg(windows)]
    windows::run();
    #[cfg(not(windows))]
    eprintln!("仅支持 Windows");
}
