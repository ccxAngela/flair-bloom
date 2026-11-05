mod core;
mod storage;
mod transport;

use self::core::{Collector, EventName};
use crate::engine::BurstEngine;
use burst_engine::{ActivityObservation, RuleActivity};
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{sync_channel, Receiver, SyncSender},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_store::StoreExt;
use tokio::sync::oneshot;

const ENABLED_KEY: &str = "usageInsightsEnabled";

enum Command {
    Toggle(bool, oneshot::Sender<Result<bool, String>>),
    Mode {
        generation: u64,
        mode: &'static str,
        reason: &'static str,
        occurred_at: i64,
    },
    Shutdown,
}

pub struct InsightsState {
    tx: SyncSender<Command>,
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
}

impl InsightsState {
    pub fn mode_result(app: &AppHandle, mode: &'static str, reason: &'static str) {
        if let Some(state) = app.try_state::<Self>() {
            let generation = state.generation.load(Ordering::SeqCst);
            if state.enabled.load(Ordering::SeqCst) {
                let _ = state.tx.try_send(Command::Mode {
                    generation,
                    mode,
                    reason,
                    occurred_at: chrono::Utc::now().timestamp_millis(),
                });
            }
        }
    }

    pub fn shutdown(&self) {
        let _ = self.tx.try_send(Command::Shutdown);
    }
}

#[tauri::command]
pub fn get_insights_enabled(state: State<InsightsState>) -> bool {
    state.enabled.load(Ordering::SeqCst)
}

#[tauri::command]
pub async fn set_insights_enabled(
    state: State<'_, InsightsState>,
    enabled: bool,
) -> Result<bool, String> {
    let (tx, rx) = oneshot::channel();
    state
        .tx
        .try_send(Command::Toggle(enabled, tx))
        .map_err(|_| "统计设置暂不可用，请稍后重试".to_string())?;
    rx.await
        .map_err(|_| "统计设置暂不可用，请稍后重试".to_string())?
}

pub fn setup(app: &AppHandle, engine: Arc<BurstEngine>) {
    let enabled = app
        .store(crate::STORE_PATH)
        .map(|store| {
            store
                .get(ENABLED_KEY)
                .is_none_or(|value| value.as_bool().unwrap_or(false))
        })
        .unwrap_or(false);
    let enabled_state = Arc::new(AtomicBool::new(enabled));
    let generation = Arc::new(AtomicU64::new(1));
    let (tx, rx) = sync_channel(64);
    app.manage(InsightsState {
        tx,
        enabled: enabled_state.clone(),
        generation: generation.clone(),
    });
    let app = app.clone();
    let status = enabled_state.clone();
    let thread_status = status.clone();
    let result = std::thread::Builder::new()
        .name("usage-insights".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                thread_status.store(false, Ordering::SeqCst);
                return;
            };
            runtime.block_on(run(app, engine, enabled_state, generation, rx));
        });
    if result.is_err() {
        status.store(false, Ordering::SeqCst);
        tracing::warn!("使用统计后台未能启动，不影响按键功能");
    }
}

struct Worker {
    app: AppHandle,
    engine: Arc<BurstEngine>,
    core: Collector,
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    paths: Option<storage::Paths>,
    activity: Option<(Arc<RuleActivity>, Receiver<ActivityObservation>)>,
    activity_generation: u64,
    runtime_reported: bool,
    dirty: bool,
}

impl Worker {
    fn persist(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(paths) = &self.paths {
            if storage::write_json(&paths.queue, &self.core.queue).is_err() {
                return;
            }
        }
        self.dirty = false;
    }

    fn observe(&mut self) {
        if let Some((_, rx)) = &self.activity {
            for observation in rx.try_iter() {
                if self.core.enabled && observation.generation == self.activity_generation {
                    let now = chrono::DateTime::<chrono::Utc>::from(observation.occurred_at)
                        .timestamp_millis();
                    self.core
                        .record(EventName::RuleActivity, BTreeMap::new(), now);
                    self.dirty = true;
                }
            }
        }
        if !self.core.enabled || self.runtime_reported {
            return;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let failure = if self.engine.output_error().is_some() {
            Some("unavailable")
        } else if self.engine.listener_ready() == Some(false) {
            Some(if cfg!(windows) {
                "initialization"
            } else {
                "unsupported"
            })
        } else {
            None
        };
        if let Some(reason) = failure {
            self.core.record(
                EventName::RuntimeFailed,
                BTreeMap::from([("reason".into(), reason.into())]),
                now,
            );
        } else if self.engine.listener_ready() == Some(true) {
            self.core
                .record(EventName::RuntimeReady, BTreeMap::new(), now);
        } else {
            return;
        }
        self.runtime_reported = true;
        self.dirty = true;
    }

    fn toggle(&mut self, enabled: bool) -> Result<bool, String> {
        if enabled != self.core.enabled {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        // 先停观察再写选择，保存失败也不继续采集；只有明确保存开启后才重新获取环境标识。
        if !enabled {
            self.enabled.store(false, Ordering::SeqCst);
            if let Some((activity, _)) = &self.activity {
                self.activity_generation = activity.set_enabled(false);
            }
            self.core
                .toggle(false, None, chrono::Utc::now().timestamp_millis());
            self.dirty = true;
            self.persist();
        }
        let store = self
            .app
            .store(crate::STORE_PATH)
            .map_err(|_| "保存使用统计设置失败".to_string())?;
        store.set(ENABLED_KEY, enabled);
        store
            .save()
            .map_err(|_| "保存使用统计设置失败".to_string())?;
        if enabled && !self.core.enabled {
            let environment =
                storage::environment(self.paths.as_ref().map(|paths| paths.environment.as_path()));
            let now = chrono::Utc::now().timestamp_millis();
            self.core.toggle(true, Some(environment), now);
            self.runtime_reported = false;
            if let Some((activity, rx)) = &self.activity {
                rx.try_iter().for_each(drop);
                self.activity_generation = activity.set_enabled(true);
            }
            self.enabled.store(true, Ordering::SeqCst);
            self.dirty = true;
        }
        self.persist();
        Ok(enabled)
    }

    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Toggle(enabled, reply) => {
                let result = self.toggle(enabled);
                let _ = reply.send(result);
            }
            Command::Mode {
                generation,
                mode,
                reason,
                occurred_at,
            } => {
                if self.core.enabled && generation == self.generation.load(Ordering::SeqCst) {
                    self.core.record(
                        EventName::InputModeResult,
                        mode_fields(mode, reason),
                        occurred_at,
                    );
                    self.dirty = true;
                }
            }
            Command::Shutdown => return true,
        }
        false
    }

    async fn flush(&mut self, client: &reqwest::Client, endpoint: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        let batch = self.core.batch(now);
        if batch.is_empty() {
            self.core.next_flush = now + core::FLUSH_MS;
            return;
        }
        self.persist();
        let jitter = uuid::Uuid::new_v4().as_u128() as u64;
        match transport::deliver(client, endpoint, &batch).await {
            transport::Delivery::Receipt(receipt) => self.core.acknowledge(
                &batch,
                receipt,
                chrono::Utc::now().timestamp_millis(),
                jitter,
            ),
            transport::Delivery::Rejected => self.core.reject_batch(&batch, now),
            transport::Delivery::Retry => self
                .core
                .retry(chrono::Utc::now().timestamp_millis(), jitter),
        }
        self.dirty = true;
        self.persist();
    }
}

pub fn mode_fields(mode: &str, reason: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("mode".into(), mode.into()),
        (
            "result".into(),
            if reason == "none" {
                "success"
            } else {
                "failed"
            }
            .into(),
        ),
        ("reason".into(), reason.into()),
    ])
}

async fn run(
    app: AppHandle,
    engine: Arc<BurstEngine>,
    enabled: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    rx: Receiver<Command>,
) {
    let paths = app.path().app_data_dir().ok().map(storage::Paths::new);
    let is_enabled = enabled.load(Ordering::SeqCst);
    let environment = is_enabled
        .then(|| storage::environment(paths.as_ref().map(|paths| paths.environment.as_path())));
    let queue = if is_enabled {
        storage::read_queue(paths.as_ref().map(|paths| paths.queue.as_path()))
    } else {
        core::QueueFile::default()
    };
    let now = chrono::Utc::now().timestamp_millis();
    let mut collector = Collector::new(is_enabled, environment, queue, now);
    collector.record(EventName::SessionStarted, BTreeMap::new(), now);
    let activity = engine.observe_rule_activity();
    let activity_generation = activity
        .as_ref()
        .map_or(0, |(activity, _)| activity.set_enabled(is_enabled));
    let mut worker = Worker {
        app,
        engine,
        core: collector,
        enabled,
        generation,
        paths,
        activity,
        activity_generation,
        runtime_reported: false,
        dirty: true,
    };
    worker.persist();
    let client = transport::client();
    let endpoint = endpoint();
    let mut saved_at = Instant::now();
    loop {
        let shutdown = match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(command) => worker.command(command),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
        };
        worker.observe();
        if saved_at.elapsed() >= Duration::from_secs(1) {
            worker.persist();
            saved_at = Instant::now();
        }
        if shutdown || chrono::Utc::now().timestamp_millis() >= worker.core.next_flush {
            if let (Some(client), Some(endpoint)) = (&client, &endpoint) {
                worker.flush(client, endpoint).await;
            }
        }
        if shutdown {
            worker.persist();
            break;
        }
    }
}

fn endpoint() -> Option<String> {
    #[cfg(debug_assertions)]
    let base = std::env::var("FLAIR_INSIGHTS_API")
        .unwrap_or_else(|_| "http://localhost:8100/api/v1".into());
    #[cfg(not(debug_assertions))]
    let base = "https://platform.xwink.fun/api/v1".to_string();
    let url = reqwest::Url::parse(&base).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    #[cfg(debug_assertions)]
    if !["localhost", "127.0.0.1", "[::1]"].contains(&url.host_str()?) || url.scheme() != "http" {
        return None;
    }
    Some(format!(
        "{}/insight-reports/flair-bloom/anonymous",
        base.trim_end_matches('/')
    ))
}
