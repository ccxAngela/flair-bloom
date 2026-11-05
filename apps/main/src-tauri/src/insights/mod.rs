mod core;
mod lifecycle;
mod storage;
mod transport;

use self::core::{Collector, EventName};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tokio::sync::mpsc::{channel, Receiver, Sender};

const THREAD_NAME: &str = "usage-insights";
enum Command {
    Activate,
    Layout(String),
    Services(serde_json::Value),
    Shutdown(std::sync::mpsc::SyncSender<()>),
}
pub struct InsightsState {
    tx: Sender<Command>,
}
impl InsightsState {
    pub fn shutdown(&self) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        if self.tx.try_send(Command::Shutdown(tx)).is_ok() {
            // 退出总预算固定，断网和后台重试不能延长产品退出。
            let _ = rx.recv_timeout(Duration::from_millis(100));
        }
    }
}
pub fn is_worker_thread() -> bool {
    std::thread::current().name() == Some(THREAD_NAME)
}

pub fn activate(app: &AppHandle) {
    if let Some(state) = app.try_state::<InsightsState>() {
        let _ = state.tx.try_send(Command::Activate);
    }
}

#[tauri::command]
pub fn observe_panel_layout(app: AppHandle, layout: String) {
    if matches!(layout.as_str(), "vertical" | "horizontal") {
        if let Some(state) = app.try_state::<InsightsState>() {
            let _ = state.tx.try_send(Command::Layout(layout));
        }
    }
}

pub fn receive_services(app: &AppHandle, source: &tauri::Url, raw: &serde_json::Value) {
    let services = &raw["services"];
    if service_endpoint(source, services).is_some() {
        if let Some(state) = app.try_state::<InsightsState>() {
            let _ = state.tx.try_send(Command::Services(services.clone()));
        }
    }
}

fn configuration_source_allowed(source: &tauri::Url) -> bool {
    if !source.username().is_empty() || source.password().is_some() {
        return false;
    }
    if cfg!(debug_assertions) {
        source.scheme() == "http"
            && matches!(source.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
    } else {
        source.scheme() == "https"
    }
}

fn service_endpoint(source: &tauri::Url, services: &serde_json::Value) -> Option<String> {
    let path = services["insights"]["anonymous"].as_str()?;
    if services["schemaVersion"] != 1
        || path != "/api/v1/insight-reports/flair-bloom/anonymous"
        || !source.username().is_empty()
        || source.password().is_some()
    {
        return None;
    }
    if !configuration_source_allowed(source) {
        return None;
    }
    source.join(path).ok().map(|url| url.to_string())
}

pub fn setup(app: &AppHandle) {
    let (tx, rx) = channel(64);
    app.manage(InsightsState { tx });
    let app = app.clone();
    if std::thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(run(app, rx));
                }
            }));
            if result.is_err() {
                tracing::warn!("使用统计已停止，产品功能继续运行");
            }
        })
        .is_err()
    {
        tracing::warn!("使用统计后台未能启动");
    }
}

fn persist(paths: Option<&storage::Paths>, collector: &Collector) {
    if let Some(paths) = paths {
        let _ = storage::write_json(&paths.queue, &collector.queue);
    }
}

async fn run(app: AppHandle, mut rx: Receiver<Command>) {
    let mut pending_services = None;
    let mut initial_layout = None;
    if crate::bootstrap::agreement::check_agreement(&app) {
        loop {
            match rx.recv().await {
                Some(Command::Activate) => break,
                Some(Command::Services(value)) => pending_services = Some(value),
                Some(Command::Layout(value)) => initial_layout = Some(value),
                Some(Command::Shutdown(reply)) => {
                    let _ = reply.send(());
                    return;
                }
                None => return,
            }
        }
    }
    let paths = app.path().app_data_dir().ok().map(storage::Paths::new);
    let environment = storage::environment(paths.as_ref().map(|p| p.environment.as_path()));
    let now = chrono::Utc::now().timestamp_millis();
    let mut collector = Collector::new(
        true,
        Some(environment),
        storage::read_queue(paths.as_ref().map(|p| p.queue.as_path())),
        now,
    );
    collector.record(EventName::SessionStarted, BTreeMap::new(), now);
    if let Some(layout) = &initial_layout {
        collector.record(
            EventName::LayoutObserved,
            BTreeMap::from([("layout".into(), layout.clone())]),
            now,
        );
    }
    persist(paths.as_ref(), &collector);
    let source = crate::bootstrap::update::configured_update_endpoints(&app)
        .ok()
        .and_then(|urls| urls.into_iter().next())
        .filter(configuration_source_allowed);
    let config_path = paths
        .as_ref()
        .map(|p| p.queue.with_file_name("services.json"));
    let mut endpoint = config_path
        .as_ref()
        .filter(|path| std::fs::metadata(path).is_ok_and(|meta| meta.len() <= 4096))
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|services| service_endpoint(source.as_ref()?, &services));
    if let Some(services) = pending_services {
        if let Some(url) = source
            .as_ref()
            .and_then(|url| service_endpoint(url, &services))
        {
            endpoint = Some(url);
            if let Some(path) = &config_path {
                let _ = storage::write_json(path, &services);
            }
        }
    }
    let client = transport::client();
    let mut configuration: Option<tokio::task::JoinHandle<Option<serde_json::Value>>> = None;
    let mut next_configuration = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut usage = lifecycle::UsageClock::default();
    let started = Instant::now();
    let mut elapsed = 0;
    let mut layout = initial_layout;
    let mut delivery: Option<tokio::task::JoinHandle<(Vec<core::Event>, transport::Delivery)>> =
        None;
    let checkpoint =
        |collector: &mut Collector, usage: &mut lifecycle::UsageClock, elapsed: &mut u64| {
            let total = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let now = chrono::Utc::now().timestamp_millis();
            for (at, duration) in usage.observe(now, total.saturating_sub(*elapsed)) {
                collector.record(
                    EventName::SessionUsage,
                    BTreeMap::from([("durationMs".into(), duration.to_string())]),
                    at,
                );
            }
            *elapsed = total;
        };
    loop {
        tokio::select! {
            command = rx.recv() => match command {
                Some(Command::Activate) => {},
                Some(Command::Layout(next)) => {
                    if layout.as_ref() != Some(&next) {
                        collector.record(EventName::LayoutObserved, BTreeMap::from([("layout".into(), next.clone())]), chrono::Utc::now().timestamp_millis());
                        layout = Some(next);
                        persist(paths.as_ref(), &collector);
                    }
                }
                Some(Command::Services(services)) => {
                    if let Some(url) = source.as_ref().and_then(|url| service_endpoint(url, &services)) {
                        endpoint = Some(url);
                        if let Some(path) = &config_path { let _ = storage::write_json(path, &services); }
                    }
                }
                Some(Command::Shutdown(reply)) => {
                    checkpoint(&mut collector, &mut usage, &mut elapsed);
                    collector.record(EventName::SessionEnded, BTreeMap::new(), chrono::Utc::now().timestamp_millis());
                    persist(paths.as_ref(), &collector);
                    if let (Some(client), Some(endpoint)) = (&client, &endpoint) {
                        let batch = collector.batch(chrono::Utc::now().timestamp_millis());
                        if !batch.is_empty() {
                            if let Ok(transport::Delivery::Receipt(receipt)) = tokio::time::timeout(
                                Duration::from_millis(50), transport::deliver(client, endpoint, &batch),
                            ).await {
                                collector.acknowledge(&batch, receipt, chrono::Utc::now().timestamp_millis(), 0);
                                persist(paths.as_ref(), &collector);
                            }
                        }
                    }
                    let _ = reply.send(());
                    break;
                }
                None => break,
            },
            _ = tick.tick() => {
                checkpoint(&mut collector, &mut usage, &mut elapsed);
                persist(paths.as_ref(), &collector);
            },
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
        if configuration
            .as_ref()
            .is_some_and(|task| task.is_finished())
        {
            if let Some(task) = configuration.take() {
                if let (Ok(Some(raw)), Some(source)) = (task.await, source.as_ref()) {
                    receive_services(&app, source, &raw);
                }
            }
            next_configuration = Instant::now() + Duration::from_secs(30);
        }
        if endpoint.is_none() && configuration.is_none() && Instant::now() >= next_configuration {
            if let (Some(client), Some(source)) = (&client, &source) {
                let client = client.clone();
                let source = source.clone();
                configuration = Some(tokio::spawn(async move {
                    transport::bootstrap(&client, source.as_str()).await
                }));
            }
        }
        if delivery.as_ref().is_some_and(|task| task.is_finished()) {
            if let Some(task) = delivery.take() {
                if let Ok((batch, result)) = task.await {
                    let now = chrono::Utc::now().timestamp_millis();
                    let jitter = uuid::Uuid::new_v4().as_u128() as u64;
                    match result {
                        transport::Delivery::Receipt(receipt) => {
                            collector.acknowledge(&batch, receipt, now, jitter)
                        }
                        transport::Delivery::Rejected => collector.reject_batch(&batch, now),
                        transport::Delivery::Retry => collector.retry(now, jitter),
                    }
                    persist(paths.as_ref(), &collector);
                } else {
                    break;
                }
            }
        }
        if delivery.is_none() && chrono::Utc::now().timestamp_millis() >= collector.next_flush {
            if let (Some(client), Some(endpoint)) = (&client, &endpoint) {
                let batch = collector.batch(chrono::Utc::now().timestamp_millis());
                if batch.is_empty() {
                    collector.next_flush = chrono::Utc::now().timestamp_millis() + core::FLUSH_MS;
                    continue;
                }
                let client = client.clone();
                let endpoint = endpoint.clone();
                delivery = Some(tokio::spawn(async move {
                    let result = transport::deliver(&client, &endpoint, &batch).await;
                    (batch, result)
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_stay_on_update_origin() {
        let origin = if cfg!(debug_assertions) {
            "http://127.0.0.1:8100"
        } else {
            "https://platform.example"
        };
        let source =
            tauri::Url::parse(&format!("{origin}/api/v1/client-bootstrap/flair-bloom")).unwrap();
        let config = serde_json::json!({"schemaVersion":1,"insights":{"anonymous":"/api/v1/insight-reports/flair-bloom/anonymous"}});
        assert_eq!(
            service_endpoint(&source, &config).as_deref(),
            Some(format!("{origin}/api/v1/insight-reports/flair-bloom/anonymous").as_str())
        );
        for path in [
            "https://evil.example/events",
            "//evil.example/events",
            "/api/v1/insight-reports/anvil/anonymous",
        ] {
            let mut bad = config.clone();
            bad["insights"]["anonymous"] = path.into();
            assert!(service_endpoint(&source, &bad).is_none());
        }
        if cfg!(debug_assertions) {
            assert!(!configuration_source_allowed(
                &tauri::Url::parse(
                    "https://platform.xwink.fun/api/v1/client-bootstrap/flair-bloom"
                )
                .unwrap()
            ));
            assert!(!configuration_source_allowed(
                &tauri::Url::parse("http://remote.example/bootstrap").unwrap()
            ));
        }
    }
}
