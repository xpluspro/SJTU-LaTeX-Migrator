use super::{app_state, locale, prepare_items, process_item, MigrateInput, Session, WorkItem};
use serde::Serialize;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use tauri::{async_runtime, Manager, Runtime, WebviewWindow};

// Queue locks protect bookkeeping only. Never hold one across I/O or a worker callback.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct Gate {
    paused: AtomicBool,
    wait: Mutex<()>,
    changed: Condvar,
}

impl Gate {
    fn set(&self, paused: bool) {
        // Pair with the recheck under the mutex to avoid losing a resume notification.
        let _guard = lock(&self.wait);
        self.paused.store(paused, Ordering::Release);
        if !paused {
            self.changed.notify_all();
        }
    }

    fn checkpoint(&self) {
        if self.paused.load(Ordering::Acquire) {
            let _guard = self
                .changed
                .wait_while(lock(&self.wait), |_| self.paused.load(Ordering::Acquire))
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[derive(Clone)]
pub(super) struct Context(Arc<ContextData>);

pub(super) struct ContextData {
    pub(super) batch_id: String,
    pub(super) id: usize,
    pub(super) total: usize,
    gate: Gate,
}

impl Deref for Context {
    type Target = ContextData;
    fn deref(&self) -> &ContextData {
        &self.0
    }
}

impl Context {
    pub(super) fn checkpoint(&self) {
        self.gate.checkpoint();
    }
}

#[derive(Serialize)]
pub(crate) struct Outcome {
    pub(super) status: String,
    pub(super) message: String,
    pub(super) path: Option<String>,
}

impl Outcome {
    fn failed(message: String) -> Self {
        Self {
            status: "failed".into(),
            message,
            path: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Prepared {
    batch_id: String,
    items: Vec<ItemInfo>,
}

#[derive(Serialize)]
struct ItemInfo {
    id: usize,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Queued,
    Running,
    Failed,
    Done,
}

impl Phase {
    fn terminal(self) -> bool {
        matches!(self, Self::Failed | Self::Done)
    }
}

struct Row {
    phase: Phase,
    ctx: Context,
}

struct Schedule {
    rows: Vec<Row>,
    active: usize,
    limit: usize,
}

impl Schedule {
    fn new(batch_id: &str, total: usize, limit: usize) -> Self {
        let rows = (0..total)
            .map(|id| Row {
                phase: Phase::Queued,
                ctx: Context(Arc::new(ContextData {
                    batch_id: batch_id.to_owned(),
                    id,
                    total,
                    gate: Gate::default(),
                })),
            })
            .collect();
        Self {
            rows,
            active: 0,
            limit,
        }
    }

    fn claim(&mut self, id: usize) -> Result<Context, String> {
        let row = self.rows.get_mut(id).ok_or_else(|| locale::translate("队列中没有这个项目".into()))?;
        match row.phase {
            Phase::Running => return Err(locale::translate("任务已在运行".into())),
            Phase::Done => return Err(locale::translate("已成功或跳过的任务不能重试".into())),
            Phase::Queued | Phase::Failed => (),
        }
        if row.ctx.gate.paused.load(Ordering::Acquire) {
            return Err(locale::translate("任务已暂停，请先继续".into()));
        }
        if self.active >= self.limit {
            return Err(locale::translate("已达到本批次并发上限".into()));
        }
        row.phase = Phase::Running;
        self.active += 1;
        Ok(row.ctx.clone())
    }

    fn finish(&mut self, id: usize, failed: bool) {
        let row = &mut self.rows[id];
        debug_assert!(row.phase == Phase::Running);
        row.phase = if failed { Phase::Failed } else { Phase::Done };
        // A pause can race the last checkpoint or a receipt. Never carry it into retry.
        row.ctx.gate.set(false);
        self.active -= 1;
    }

    fn pause(&mut self, id: usize, paused: bool) -> Result<(), String> {
        let row = self.rows.get(id).ok_or_else(|| locale::translate("队列中没有这个项目".into()))?;
        if row.phase == Phase::Done {
            return Err(locale::translate("已完成的任务不能暂停或继续".into()));
        }
        row.ctx.gate.set(paused);
        Ok(())
    }
}

struct Batch {
    id: String,
    source: Option<Session>,
    target: Session,
    items: Vec<WorkItem>,
    schedule: Mutex<Schedule>,
}

// One visible batch, kept after completion for explicit retries. Never take fresh
// site URLs or cookies when running an item, even if configuration has changed.
#[derive(Default)]
pub(super) struct Registry {
    current: Mutex<Catalog>,
}

#[derive(Default)]
struct Catalog {
    preparing: bool,
    generation: u64,
    batch: Option<Arc<Batch>>,
}

impl Catalog {
    fn lookup(&self, batch_id: &str) -> Result<&Arc<Batch>, String> {
        if self.preparing {
            return Err(locale::translate("正在准备新批次".into()));
        }
        self.batch
            .as_ref()
            .filter(|b| b.id == batch_id)
            .ok_or_else(|| locale::translate("无效或已过期的批次编号".into()))
    }
}

struct Preparation<'a> {
    registry: &'a Registry,
    id: String,
}

impl Drop for Preparation<'_> {
    fn drop(&mut self) {
        lock(&self.registry.current).preparing = false;
    }
}

impl Registry {
    fn prepare(&self) -> Result<Preparation<'_>, String> {
        let mut current = lock(&self.current);
        if current.preparing
            || current
                .batch
                .as_ref()
                .is_some_and(|b| lock(&b.schedule).rows.iter().any(|r| !r.phase.terminal()))
        {
            return Err(locale::translate("上一批次尚未结束".into()));
        }
        current.generation = current.generation.checked_add(1).ok_or_else(|| locale::translate("批次编号已用尽".into()))?;
        let id = current.generation.to_string();
        current.preparing = true;
        Ok(Preparation { registry: self, id })
    }

    fn claim(&self, batch_id: &str, id: usize) -> Result<Attempt, String> {
        // Lock order is catalog -> schedule -> gate. Check the batch and reserve
        // the slot together so replacing a terminal batch cannot race a retry.
        let current = lock(&self.current);
        let batch = current.lookup(batch_id)?;
        let ctx = lock(&batch.schedule).claim(id)?;
        Ok(Attempt {
            batch: Arc::clone(batch),
            ctx,
            failed: true,
        })
    }

    fn pause(&self, batch_id: &str, id: usize, paused: bool) -> Result<(), String> {
        let current = lock(&self.current);
        let batch = current.lookup(batch_id)?;
        let mut schedule = lock(&batch.schedule);
        schedule.pause(id, paused)
    }
}

// The blocking closure owns its slot: dropping the IPC future cannot free a
// running worker's slot. A panic, including one before process_item returns,
// drops this guard and makes the item retryable.
struct Attempt {
    batch: Arc<Batch>,
    ctx: Context,
    failed: bool,
}

impl Attempt {
    fn run<R: Runtime>(self, app: &tauri::AppHandle<R>) -> Outcome {
        self.ctx.checkpoint();
        process_item(
            app,
            self.batch.source.as_ref(),
            &self.batch.target,
            &self.batch.items[self.ctx.id],
            &self.ctx,
        )
        .unwrap_or_else(Outcome::failed)
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        lock(&self.batch.schedule).finish(self.ctx.id, self.failed);
    }
}

fn main_only<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    if window.label() != "main" {
        return Err(locale::translate("仅主窗口可以操作任务队列".into()));
    }
    Ok(())
}

fn validate(
    mode: &str,
    names: &[String],
    input: &MigrateInput,
    limit: usize,
) -> Result<(), String> {
    if !(1..=3).contains(&limit) {
        return Err(locale::translate("并发任务数必须在 1 到 3 之间".into()));
    }
    match mode {
        "local" if !names.is_empty() && input.project_ids.is_empty() => Ok(()),
        // Display names, when supplied, are not authority for migration: the
        // immutable WorkItems are resolved from the source session by project ID.
        "migration" if !input.project_ids.is_empty() && !input.workdir.trim().is_empty() => Ok(()),
        "local" | "migration" => Err(locale::translate("任务模式与所选项目不一致".into())),
        _ => Err(locale::translate("未知任务类型".into())),
    }
}

#[tauri::command]
pub(crate) async fn prepare_queue<R: Runtime>(
    window: WebviewWindow<R>,
    mode: String,
    names: Vec<String>,
    input: MigrateInput,
    limit: usize,
) -> Result<Prepared, String> {
    main_only(&window)?;
    validate(&mode, &names, &input, limit)?;
    let app = window.app_handle().clone();
    async_runtime::spawn_blocking(move || {
        let state = app_state(&app);
        let prep = state.queue.prepare()?;
        let (source, target, items) = prepare_items(&app, &mode, names, input).map_err(locale::translate)?;
        if items.is_empty() {
            return Err(locale::translate("没有可执行的项目".into()));
        }
        let prepared = Prepared {
            batch_id: prep.id.clone(),
            items: items
                .iter()
                .enumerate()
                .map(|(id, item)| ItemInfo {
                    id,
                    name: item.name().to_owned(),
                })
                .collect(),
        };
        let batch = Arc::new(Batch {
            id: prep.id.clone(),
            source,
            target,
            schedule: Mutex::new(Schedule::new(&prep.id, items.len(), limit)),
            items,
        });
        // Keep the previous batch if preparation failed. Release retired clients
        // outside the queue lock and on this blocking thread.
        let retired = lock(&state.queue.current).batch.replace(batch);
        drop(prep);
        drop(retired);
        Ok(prepared)
    })
    .await
    .map_err(|_| locale::translate("准备队列的后台任务异常退出".to_owned()))?
}

#[tauri::command]
pub(crate) async fn run_queue_item<R: Runtime>(
    window: WebviewWindow<R>,
    batch_id: String,
    id: usize,
) -> Result<Outcome, String> {
    main_only(&window)?;
    let app = window.app_handle().clone();
    let attempt = app_state(&app).queue.claim(&batch_id, id)?;
    let result = async_runtime::spawn_blocking(move || attempt.run(&app)).await;
    Ok(result
        .unwrap_or_else(|_| Outcome::failed(locale::translate("后台任务异常退出，请先核对目标站的导入结果".into()))))
}

#[tauri::command]
pub(crate) async fn pause_queue_item<R: Runtime>(
    window: WebviewWindow<R>,
    batch_id: String,
    id: usize,
    paused: bool,
) -> Result<(), String> {
    main_only(&window)?;
    app_state(window.app_handle())
        .queue
        .pause(&batch_id, id, paused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn only_supported_limits_and_modes_are_accepted() {
        let names = vec!["local zip".into()];
        let input = MigrateInput {
            project_ids: vec![],
            workdir: String::new(),
        };
        for limit in [0, 1, 2, 3, 4, usize::MAX] {
            assert_eq!(
                validate("local", &names, &input, limit).is_ok(),
                (1..=3).contains(&limit)
            );
        }
        assert!(validate("migration", &names, &input, 1).is_err());
        assert!(validate("unknown", &names, &input, 1).is_err());
    }

    #[test]
    fn admission_enforces_limits_and_distinguishes_queued_and_active_pause() {
        for limit in 1..=3 {
            let mut q = Schedule::new("test", limit + 1, limit);
            q.pause(0, true).unwrap();
            assert!(q.claim(0).is_err());
            assert_eq!(q.active, 0);
            q.pause(0, false).unwrap();
            for id in 0..limit {
                q.claim(id).unwrap();
            }
            q.pause(0, true).unwrap();
            assert!(q.claim(limit).is_err()); // paused worker still owns a slot
            q.finish(0, true); // worker got a receipt/error before its next checkpoint
            let retry = q.claim(0).unwrap();
            assert!(!retry.gate.paused.load(Ordering::Acquire));
            q.finish(0, false);
            assert!(q.claim(0).is_err());
            q.claim(limit).unwrap();
        }
    }

    #[test]
    fn resume_wakes_checkpoint_on_another_thread() {
        let mut q = Schedule::new("test", 1, 1);
        let ctx = q.claim(0).unwrap();
        q.pause(0, true).unwrap();
        let (started, ready) = mpsc::channel();
        let (done, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            started.send(()).unwrap();
            ctx.checkpoint();
            done.send(()).unwrap();
        });
        ready.recv().unwrap();
        assert!(matches!(
            result.recv_timeout(Duration::from_millis(20)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        q.pause(0, false).unwrap();
        result.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn panic_releases_slot_and_retry_keeps_prepared_session() {
        let registry = Registry::default();
        let prep = registry.prepare().unwrap();
        let id = prep.id.clone();
        let batch = Arc::new(Batch {
            id: id.clone(),
            source: None,
            target: Session {
                base: "https://original.invalid".into(),
                http: reqwest::blocking::Client::new(),
                csrf: Mutex::default(),
            },
            items: vec![WorkItem::Local {
                name: "name".into(),
                path: "zip".into(),
            }],
            schedule: Mutex::new(Schedule::new(&id, 1, 1)),
        });
        lock(&registry.current).batch = Some(Arc::clone(&batch));
        assert!(registry.claim(&id, 0).is_err()); // preparation not yet published
        drop(prep);
        assert!(registry.prepare().is_err()); // queued is nonterminal
        assert!(registry.claim("stale", 0).is_err());
        assert!(registry.claim(&id, 1).is_err());
        let attempt = registry.claim(&id, 0).unwrap();
        assert!(registry.claim(&id, 0).is_err());
        assert!(std::thread::spawn(move || {
            let _slot = attempt;
            panic!("worker panic")
        })
        .join()
        .is_err());
        let interrupted_prep = registry.prepare().unwrap();
        assert!(registry.claim(&id, 0).is_err());
        assert_ne!(interrupted_prep.id, id);
        drop(interrupted_prep); // failed replacement leaves original retryable
        let mut retry = registry.claim(&id, 0).unwrap();
        assert!(Arc::ptr_eq(&retry.batch, &batch));
        retry.failed = false;
        drop(retry);
        assert!(registry.claim(&id, 0).is_err());
        assert!(registry.pause(&id, 0, true).is_err());
        assert!(registry.prepare().is_ok());
    }
}
