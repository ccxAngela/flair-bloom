//! 外设驱动真机冒烟：切到外设模式，往前台窗口逐键打一行字，再经引擎调度长按连发 Q 一秒。
//!
//! 会真的敲键：先把光标放进记事本、输入法切英文，运行期间别碰键盘鼠标。
//! `cargo run -p burst-engine --example hidstick_smoke`

#[cfg(windows)]
fn main() {
    use burst_engine::BurstEngine;
    use qzh_profile::{BurstMode, BurstRule, KeyId};
    use std::time::Duration;
    use win_input::{init_backend, key_event, InputEvent, InputMode};

    const LINE: &str = "flair bloom hidstick ok";
    const F13: u32 = 0x7C;
    const Q: u32 = 0x51;
    const ENTER: u32 = 0x0D;
    const HOLD: Duration = Duration::from_millis(1000);
    const INTERVAL_MS: u32 = 50;
    // 打字节奏对齐引擎：按住 10 ms、松开后再等 10 ms。按下松开紧挨着发（约 2 ms 一个键）时
    // 记事本会丢字、回车错位，那是打得比任何键盘都快，不是通道的问题
    const TAP_HOLD: Duration = Duration::from_millis(10);

    init_backend(InputMode::HidStick);
    assert_eq!(
        win_input::current_mode(),
        InputMode::HidStick,
        "没连上外设驱动"
    );

    // 每个键按下 / 松开各走了哪条路：外设驱动发出是 Sent，被拒后改走 SendInput 是 FallbackSent
    let trace = std::cell::RefCell::new(Vec::new());
    let tap = |vk: u32| {
        let down = key_event(InputEvent::down(KeyId::Keyboard(vk)));
        std::thread::sleep(TAP_HOLD);
        let up = key_event(InputEvent::up(KeyId::Keyboard(vk)));
        std::thread::sleep(TAP_HOLD);
        trace.borrow_mut().push((vk, down, up));
        assert!(down.was_sent() && up.was_sent(), "VK {vk:#04X} 没发出去");
    };

    // 1. 分发层：逐键打字（小写字母与空格，避开 Shift，结果好核对）
    for ch in LINE.chars() {
        let vk = match ch {
            ' ' => 0x20,
            c => c.to_ascii_uppercase() as u32,
        };
        tap(vk);
    }
    tap(ENTER);

    // 2. 引擎层：长按 F13 连发 Q。F13 由代码模拟按下，Q 经调度器 → 外设驱动敲出
    let engine = BurstEngine::new();
    engine.set_rules(vec![BurstRule {
        id: "smoke".to_string(),
        enabled: true,
        trigger_key: KeyId::Keyboard(F13),
        steps: vec![qzh_profile::profile::OutputStep {
            key: KeyId::Keyboard(Q),
            interval_ms: INTERVAL_MS,
        }],
        mode: BurstMode::Hold,
        stop_key: None,
        group: None,
    }]);
    engine.set_global_enabled(true, false);
    engine.on_key_press(KeyId::Keyboard(F13));
    std::thread::sleep(HOLD);
    engine.on_key_release(KeyId::Keyboard(F13));
    engine.set_global_enabled(false, true);
    engine.shutdown();
    tap(ENTER);

    init_backend(InputMode::SendInput);
    for (vk, down, up) in trace.borrow().iter() {
        println!("VK {vk:#04X}：按下 {down:?}，松开 {up:?}");
    }
    println!(
        "已打出「{LINE}」并长按连发 Q {} ms（间隔 {INTERVAL_MS} ms，预期约 {} 个 q）",
        HOLD.as_millis(),
        HOLD.as_millis() as u32 / INTERVAL_MS
    );
}

#[cfg(not(windows))]
fn main() {
    eprintln!("仅 Windows");
}
