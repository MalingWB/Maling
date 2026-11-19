//! 单个远程任务的执行记录上报器：有界缓冲 + 500ms 批 flush + seq 分配 + 终态排空 + 用量上报。
//!
//! 生命周期与 `(local_task_id, local_run_id)` 绑定（= 一个 remote task 的一次执行）。服务端
//! 侧 seq 作用域是 `task_id`，claim 产生的每个新 task（含 retry/rerun 子任务）从 1 开始；
//! **同一任务终态后的追问 run 例外**（M5-de）：服务端 task_id 不变，追问必须接续上一 run
//! 的 seq 水位（内存快路径 + `StateConfig.remote_transcript_continuity` 跨重启恢复），
//! 用量也必须以历史累计为种子（服务端按 `(task_id, provider, model)` 整体替换）。
//!
//! 失败策略：批次失败即丢弃（协议明确批次原子、不重试；重发同 seq 会产生重复行）；
//! 只有 404（任务已不可见/无权）会让本 run 停止上报，避免无意义的持续请求。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use camino::Utf8Path;
use gold_band::acp::usage::read_prompt_usage_segments;
use gold_band::config::{
    RemoteCompletedTask, RemoteTranscriptContinuity, RemoteTranscriptUsageRow, StateConfig,
};
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{Notify, oneshot};
use tracing::warn;

use crate::multica::client::MulticaClient;
use crate::multica::error::MulticaError;

use super::model::{DRAIN_TIMEOUT, FLUSH_INTERVAL, UsageRow};
use super::projector::Projector;

/// 取当前连接对应的 client。进程内连接信息可能被用户重配，所以每次发送时重新读取。
pub(crate) type ClientProvider = Arc<dyn Fn() -> Option<MulticaClient> + Send + Sync>;

/// 终态任务的追问续报基线（finish_run 排空后捕获，追问 run 懒挂载时消费）。
///
/// `last_seq` = 该任务已分配的最后一个 seq；`usage_seed` = 已上报的累计用量行（追问 run
/// 在其之上继续累计——服务端整体替换语义要求每次上报都是任务级累计总量）。
#[derive(Debug, Clone)]
pub(crate) struct FinishedContinuation {
    pub(crate) remote_task_id: String,
    pub(crate) last_seq: u64,
    pub(crate) usage_seed: Vec<UsageRow>,
}

/// 在飞远程任务的上报器注册表（Tauri managed state）。
///
/// 两层 map 用 `&str` 直接查找，避免每条 live 事件都为主键分配字符串。
#[derive(Default)]
pub struct TranscriptRegistry {
    runs: Mutex<HashMap<String, HashMap<String, Arc<Reporter>>>>,
    /// 终态任务的续报基线（键 = local_task_id）：finish_run 写入（进程内权威），
    /// 追问 run 首个上报事件时同步读取挂载（避免异步挂载丢失追问首拍用户回声）。
    finished: Mutex<HashMap<String, FinishedContinuation>>,
    /// 已判定「无续报资格」的本地任务（非 multica / completed 历史无水位）：
    /// 短路后续事件，避免每条 live 事件都触发一次解析。
    foreign: Mutex<HashSet<String>>,
    /// 正在异步解析归属的任务：解析窗口内的重复事件直接放弃（每任务至多 spawn 一次）。
    resolving: Mutex<HashSet<String>>,
}

impl TranscriptRegistry {
    fn insert(&self, local_task_id: &str, local_run_id: &str, reporter: Arc<Reporter>) {
        let mut runs = self.lock();
        if let Some(previous) = runs
            .entry(local_task_id.to_string())
            .or_default()
            .insert(local_run_id.to_string(), reporter)
        {
            // 同一 run 重复注册（极端竞态）：旧 worker 立即收尾，避免两路写同一 task 的 seq。
            previous.request_finish();
        }
    }

    /// 原子「同 remote 排他 + 注册」：已存在同 remote 的在飞 reporter 时拒绝（返回 false），
    /// 否则注册并接管 `(local_task_id, local_run_id)`。两路 reporter 并发写同一 remote 的
    /// seq 空间会在服务端产生交错行——宁可少报一路。
    fn insert_exclusive(
        &self,
        local_task_id: &str,
        local_run_id: &str,
        remote_task_id: &str,
        reporter: Arc<Reporter>,
    ) -> bool {
        let mut runs = self.lock();
        let same_remote_active = runs
            .values()
            .any(|by_run| by_run.values().any(|r| r.remote_task_id == remote_task_id));
        if same_remote_active {
            return false;
        }
        if let Some(previous) = runs
            .entry(local_task_id.to_string())
            .or_default()
            .insert(local_run_id.to_string(), reporter)
        {
            previous.request_finish();
        }
        true
    }

    fn find(&self, local_task_id: &str, local_run_id: &str) -> Option<Arc<Reporter>> {
        self.lock()
            .get(local_task_id)
            .and_then(|runs| runs.get(local_run_id))
            .cloned()
    }

    fn remove(&self, local_task_id: &str, local_run_id: &str) -> Option<Arc<Reporter>> {
        let mut runs = self.lock();
        let removed = runs
            .get_mut(local_task_id)
            .and_then(|by_run| by_run.remove(local_run_id));
        if runs.get(local_task_id).is_some_and(HashMap::is_empty) {
            runs.remove(local_task_id);
        }
        removed
    }

    /// finish_run 写入续报基线（进程内权威，覆盖旧值——后收尾的 run 水位必然更新）。
    /// 同时清掉 foreign 标记：终态排空竞态窗口内该任务可能已被异步解析误标。
    fn record_finished(&self, local_task_id: &str, continuation: FinishedContinuation) {
        self.finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(local_task_id.to_string(), continuation);
        self.mark_resolved(local_task_id, FollowupResolution::Finished);
    }

    /// 异步解析结果写入：仅在无更新基线时生效（finish_run 的进程内值更新，不会被旧盘值覆盖）。
    fn record_finished_if_absent(&self, local_task_id: &str, continuation: FinishedContinuation) {
        let mut finished = self
            .finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        finished
            .entry(local_task_id.to_string())
            .or_insert(continuation);
        drop(finished);
        self.mark_resolved(local_task_id, FollowupResolution::Finished);
    }

    fn is_known_foreign(&self, local_task_id: &str) -> bool {
        self.foreign
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(local_task_id)
    }

    /// 认领一次解析权（返回 false = 已有在飞解析，本事件放弃）。
    fn begin_resolution(&self, local_task_id: &str) -> bool {
        self.resolving
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(local_task_id.to_string())
    }

    /// 解析收尾：清在飞标记并落地结论（Finished = 有基线，Foreign = 无续报资格）。
    fn mark_resolved(&self, local_task_id: &str, outcome: FollowupResolution) {
        self.resolving
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(local_task_id);
        let mut foreign = self
            .foreign
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match outcome {
            FollowupResolution::Finished => {
                foreign.remove(local_task_id);
            }
            FollowupResolution::Foreign => {
                foreign.insert(local_task_id.to_string());
            }
        }
    }

    /// 懒挂载追问 run 的 reporter（内存基线命中路径；纯内存操作，可在 live 事件热路径同步执行）。
    fn attach_followup(
        &self,
        local_task_id: &str,
        local_run_id: &str,
        client: ClientProvider,
    ) -> Option<Arc<Reporter>> {
        let continuation = {
            let finished = self
                .finished
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            finished.get(local_task_id).cloned()?
        };
        let reporter = Arc::new(Reporter::new(
            continuation.remote_task_id.clone(),
            client,
            // 追问的用户原话走 live userTextDelta 回声事件投影（originalUserText 过滤器放行），
            // 不在此预置 seq=1 初始输入。
            None,
            Some(&continuation),
        ));
        if !self.insert_exclusive(
            local_task_id,
            local_run_id,
            &continuation.remote_task_id,
            reporter.clone(),
        ) {
            return None;
        }
        tauri::async_runtime::spawn(run_worker(reporter.clone()));
        Some(reporter)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, HashMap<String, Arc<Reporter>>>> {
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 归属解析结论：有续报基线 / 无续报资格（非 multica 任务、旧版本收尾无水位）。
enum FollowupResolution {
    Finished,
    Foreign,
}

pub(crate) struct Reporter {
    remote_task_id: String,
    client: ClientProvider,
    state: Mutex<ReporterState>,
    notify: Notify,
}

struct ReporterState {
    project: Projector,
    next_seq: u64,
    usage: UsageAccumulator,
    /// 待刷新用量的 attempt 目录（节点/turn 边界登记，由 worker 统一做文件读取）。
    usage_pending: BTreeSet<String>,
    usage_dirty: bool,
    /// 收到终态信号：做完最后一次 flush + 用量上报后退出。
    closing: bool,
    /// 服务端已不认识该任务（404）：停止一切上报。
    stopped: bool,
    done: Option<oneshot::Sender<()>>,
}

impl Reporter {
    fn new(
        remote_task_id: String,
        client: ClientProvider,
        initial_user_text: Option<&str>,
        continuation: Option<&FinishedContinuation>,
    ) -> Self {
        Self {
            remote_task_id,
            client,
            state: Mutex::new(ReporterState {
                // 初始需求原话在构造期即占据 outbox 最前部 → 首批 drain 分到 seq=1，
                // 保证「任务开始执行前，初始输入是第一条消息」（multica 回放时序要求）。
                project: Projector::new(initial_user_text),
                // seq 作用域 = task_id：claim 的新 task 从 1 开始；终态后追问 run（M5-de）
                // 接续上一 run 水位（服务端 task_id 不变，重叠 seq 会产生交错/重复行）。
                next_seq: continuation.map_or(1, |c| c.last_seq.saturating_add(1)),
                // 用量种子 = 已上报的任务级累计（服务端按三元组整体替换，追问必须报累计总量）。
                usage: UsageAccumulator::seeded(
                    continuation.map(|c| c.usage_seed.as_slice()).unwrap_or(&[]),
                ),
                usage_pending: BTreeSet::new(),
                usage_dirty: false,
                closing: false,
                stopped: false,
                done: None,
            }),
            notify: Notify::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ReporterState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 读取待刷新的 attempt 用量日志（worker 独占，不在 lifecycle 热路径上）。
    ///
    /// 每个 attempt 在每次边界只登记一次，因此这里是「每边界一次小文件读取」，不是轮询式重读。
    /// jsonl 是同步 I/O，放 blocking 线程池执行，不占住 async worker 线程。
    async fn refresh_usage(self: &Arc<Self>, attempt_dirs: &[String]) {
        let dirs = attempt_dirs.to_vec();
        let read = tauri::async_runtime::handle()
            .inner()
            .spawn_blocking(move || {
                let mut accumulated = Vec::with_capacity(dirs.len());
                for dir in &dirs {
                    let segments = read_prompt_usage_segments(Utf8Path::new(dir));
                    if !segments.is_empty() {
                        accumulated.push((dir.clone(), segments));
                    }
                }
                accumulated
            });
        let accumulated = match read.await {
            Ok(accumulated) => accumulated,
            Err(error) => {
                warn!(task = %self.remote_task_id, %error, "multica usage refresh failed");
                Vec::new()
            }
        };
        if accumulated.is_empty() {
            return;
        }
        let mut state = self.lock();
        // closing 也要合并：终态那一拍要带上最后一个边界的用量。
        if state.stopped {
            return;
        }
        let mut merged = false;
        for (dir, segments) in accumulated {
            merged |= state.usage.merge(&dir, segments);
        }
        if merged {
            state.usage_dirty = true;
        }
    }

    fn request_finish(&self) {
        {
            let mut state = self.lock();
            if state.closing {
                return;
            }
            state.closing = true;
        }
        self.notify.notify_one();
    }
}

/// 构造连接 client 提供器（start_run 与追问懒挂载共用；每次发送时重读连接配置）。
fn client_provider(app: &AppHandle) -> ClientProvider {
    let provider_app = app.clone();
    Arc::new(move || {
        let context = crate::multica::bridge::desktop_context(&provider_app)?;
        crate::multica::bridge::multica_client(&context)
    })
}

/// 注册一个新的上报器并启动 flush worker。
///
/// `initial_user_text`：任务初始需求原话（用户的话，非组装后 prompt），作为 seq=1 的
/// 第一条 `user` 消息上报。fresh 传 composer 提交文本；resume 传被续任务 requirement 文件。
pub(crate) fn start_run(
    app: &AppHandle,
    remote_task_id: &str,
    local_task_id: &str,
    local_run_id: &str,
    initial_user_text: Option<&str>,
) {
    let Some(registry) = app.try_state::<TranscriptRegistry>() else {
        return;
    };
    let reporter = Arc::new(Reporter::new(
        remote_task_id.to_string(),
        client_provider(app),
        initial_user_text,
        None,
    ));
    registry.insert(local_task_id, local_run_id, reporter.clone());
    tauri::async_runtime::spawn(run_worker(reporter));
}

/// 观测一个 live 事件（root 分支）。非 multica 任务 / 非上报类型直接返回。
pub(crate) fn observe(
    app: &AppHandle,
    context: &gold_band::app::AcpLiveEventContext,
    event: &gold_band::acp::events::AcpUiEvent,
) {
    if !is_reported_event(event) {
        return;
    }
    // 子 agent 分支不属于本次执行的对外时间线（Gold Band 自己也是折叠展示）；
    // 先于 registry 查做过滤，子分支的高频流式事件不必抢注册表锁。
    if gold_band::acp::branches::event_branch_id(event) != gold_band::acp::branches::ROOT_BRANCH_ID
    {
        return;
    }
    let Some(registry) = app.try_state::<TranscriptRegistry>() else {
        return;
    };
    let Some(reporter) = registry.find(&context.task_id, &context.run_id) else {
        // M5-de：终态后追问 run 懒挂载。首查未命中先试内存基线（本进程内收尾过该任务的
        // run），命中即同步挂载并投影本事件——追问的 userTextDelta 回声是该 run 的第一条
        // 上报事件，异步挂载会丢掉追问首拍。未命中再一次性异步解析（跨重启恢复）。
        let Some(reporter) =
            attach_followup_if_owned(app, &registry, &context.task_id, &context.run_id)
        else {
            return;
        };
        reporter.lock().project.observe(event);
        return;
    };
    reporter.lock().project.observe(event);
}

/// 终态后追问 run 的懒挂载入口：内存基线 → 同步挂载；未命中 → 一次性异步解析归属。
///
/// 热路径成本：非 multica 任务在 registry 首查未命中后，至多多一次 `finished`/`foreign`
/// 集合查找（无磁盘 IO）；解析（含 state.json 读）只在每任务首个未命中事件发生一次。
fn attach_followup_if_owned(
    app: &AppHandle,
    registry: &TranscriptRegistry,
    local_task_id: &str,
    local_run_id: &str,
) -> Option<Arc<Reporter>> {
    if let Some(reporter) =
        registry.attach_followup(local_task_id, local_run_id, client_provider(app))
    {
        return Some(reporter);
    }
    if registry.is_known_foreign(local_task_id) {
        return None;
    }
    spawn_baseline_resolution(app, local_task_id);
    None
}

/// 跨重启的归属解析（spawn 内执行，每任务至多一次）：completed 历史反查 + 持久水位读取。
///
/// 解析窗口内到达的事件放弃（仅影响重启后首个追问的首拍）；结论落地后，后续事件走
/// `attach_followup`（命中基线）或 `foreign` 短路。
fn spawn_baseline_resolution(app: &AppHandle, local_task_id: &str) {
    let Some(registry) = app.try_state::<TranscriptRegistry>() else {
        return;
    };
    if !registry.begin_resolution(local_task_id) {
        return;
    }
    let app = app.clone();
    let local_task_id = local_task_id.to_string();
    tauri::async_runtime::spawn(async move {
        let Some(registry) = app.try_state::<TranscriptRegistry>() else {
            return;
        };
        // 桌面上下文不可用（极端：managed state 缺失）按非 multica 处理，与 observe 的
        // fail-open 一致；state 读取失败同样短路（避免每事件重复 spawn 打磁盘）。
        let baseline = crate::multica::bridge::desktop_context(&app)
            .and_then(|context| context.app().load_state().ok())
            .and_then(|state| resolve_followup_baseline(&state, &local_task_id));
        match baseline {
            Some(continuation) => {
                registry.record_finished_if_absent(&local_task_id, continuation);
            }
            None => registry.mark_resolved(&local_task_id, FollowupResolution::Foreign),
        }
    });
}

/// 纯函数：从 StateConfig 解析追问续报基线。
///
/// 只认该 local task 的**最新** completed 条目（历史最新在前；旧 remote 已被 rerun 顶替，
/// 向其续报会写错任务）。最新条目无持久水位（旧版本收尾的任务）→ None：seq 基线未知，
/// 盲目续报会与服务端既有行交错，宁可不报（与升级前行为一致）。
fn resolve_followup_baseline(
    state: &StateConfig,
    local_task_id: &str,
) -> Option<FinishedContinuation> {
    let newest = state
        .remote_completed_tasks
        .iter()
        .find(|c| c.local_task_id == local_task_id)?;
    let continuity = state
        .remote_transcript_continuity
        .as_ref()?
        .get(&newest.remote_task_id)?;
    Some(FinishedContinuation {
        remote_task_id: newest.remote_task_id.clone(),
        last_seq: continuity.last_seq,
        usage_seed: continuity
            .usage_rows
            .iter()
            .map(|row| UsageRow {
                provider: row.provider.clone(),
                model: row.model.clone(),
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_read_tokens: row.cache_read_tokens,
                cache_write_tokens: row.cache_write_tokens,
                // 恒 0：无权威费用，服务端按费率估算（协议约定）。
                cost_usd_ticks: 0,
            })
            .collect(),
    })
}

/// 纯函数：写入持久水位并清理已淘汰任务的旧条目。
///
/// 当前写入的键无条件保留（首 run 排空先于 finalize 落历史，此刻它还不在 completed 列表）；
/// 其余键以 completed 历史为准——历史有界（最新在前截断），水位随之有界。
fn apply_continuation_write(
    state: &mut StateConfig,
    remote_task_id: &str,
    last_seq: u64,
    usage_rows: &[UsageRow],
) {
    let map = state
        .remote_transcript_continuity
        .get_or_insert_with(HashMap::new);
    map.insert(
        remote_task_id.to_string(),
        RemoteTranscriptContinuity {
            last_seq,
            usage_rows: usage_rows
                .iter()
                .map(|row| RemoteTranscriptUsageRow {
                    provider: row.provider.clone(),
                    model: row.model.clone(),
                    input_tokens: row.input_tokens,
                    output_tokens: row.output_tokens,
                    cache_read_tokens: row.cache_read_tokens,
                    cache_write_tokens: row.cache_write_tokens,
                })
                .collect(),
        },
    );
    let in_history: HashSet<&str> = state
        .remote_completed_tasks
        .iter()
        .map(|c: &RemoteCompletedTask| c.remote_task_id.as_str())
        .collect();
    map.retain(|key, _| key == remote_task_id || in_history.contains(key.as_str()));
}

fn is_reported_event(event: &gold_band::acp::events::AcpUiEvent) -> bool {
    match event.kind.as_str() {
        "textDelta" | "toolCall" | "toolCallUpdate" | "userTextDelta" => true,
        "thoughtDelta" => super::model::REPORT_THINKING,
        _ => false,
    }
}

/// 上报一条运行期错误（失败/取消）。只在 error 行进缓冲后立即唤醒 worker，不等 500ms 节拍。
pub(crate) fn report_error<R: Runtime>(
    app: &AppHandle<R>,
    local_task_id: &str,
    local_run_id: &str,
    message: &str,
) {
    let Some(registry) = app.try_state::<TranscriptRegistry>() else {
        return;
    };
    let Some(reporter) = registry.find(local_task_id, local_run_id) else {
        return;
    };
    {
        let mut state = reporter.lock();
        if state.closing {
            return;
        }
        state.project.push_error(message);
    }
    reporter.notify.notify_one();
}

/// 节点/turn 边界到达：登记该 attempt 的用量待刷新。
///
/// **不做文件读取**——lifecycle 订阅器回调在热路径上，这里只做一次内存登记；jsonl 由 worker
/// 在下一个 500ms 节拍统一读取并累计（同一 attempt 的重复登记被集合去重）。
/// 幂等：同一 `(attempt_dir, turn_seq)` 只累计一次；上报的是**累计总量**（服务端按
/// `(task_id, provider, model)` 整体替换，不是累加）。
pub(crate) fn accumulate_usage<R: Runtime>(
    app: &AppHandle<R>,
    local_task_id: &str,
    local_run_id: &str,
    attempt_dir: &str,
) {
    let Some(registry) = app.try_state::<TranscriptRegistry>() else {
        return;
    };
    let Some(reporter) = registry.find(local_task_id, local_run_id) else {
        return;
    };
    let mut state = reporter.lock();
    if state.closing {
        return;
    }
    state.usage_pending.insert(attempt_dir.to_string());
}

/// 终态排空：把剩余消息与最终用量送出去，最多等 `timeout`。
///
/// 幂等：同 run 重复调用第二次拿不到 reporter，直接返回。
///
/// M5-de：排空后捕获续报基线（seq 高水位 + 累计用量），登记进程内快路径并持久化——
/// 同一任务终态后的追问 run 依赖它续接 seq 与用量（服务端 task_id 不变、用量整体替换）。
pub(crate) async fn finish_run<R: Runtime>(
    app: &AppHandle<R>,
    local_task_id: &str,
    local_run_id: &str,
    timeout: Duration,
) {
    let Some(registry) = app.try_state::<TranscriptRegistry>() else {
        return;
    };
    let Some(reporter) = registry.remove(local_task_id, local_run_id) else {
        return;
    };
    let (done_tx, done_rx) = oneshot::channel();
    {
        let mut state = reporter.lock();
        if state.closing {
            return;
        }
        state.done = Some(done_tx);
        state.closing = true;
    }
    reporter.notify.notify_one();
    if tokio::time::timeout(timeout, done_rx).await.is_err() {
        warn!(
            task = %reporter.remote_task_id,
            "multica transcript drain timed out; remaining messages dropped"
        );
    }
    // 水位登记对排空超时同样生效：next_seq-1 是已分配 seq 的安全上界（协议允许空洞，
    // 不允许重叠）；crash 场景见模块文档遗留说明。
    let continuation = capture_continuation(&reporter);
    registry.record_finished(local_task_id, continuation.clone());
    persist_continuation(app, &continuation);
    let dropped = reporter.lock().project.dropped_messages();
    if dropped > 0 {
        // 背压上限生效（服务端写入跟不上或任务产出行数过大）——只记录，不重发。
        warn!(task = %reporter.remote_task_id, dropped, "multica transcript dropped messages");
    }
}

/// 无在飞映射的追问 turn 边界收尾（M5-de 修复点）。
///
/// Direct 后续追问 / Workflow-AUTO 节点完成后的手动追问在**同一 attempt 内**收尾、不产生
/// 新的 `RunCompleted`（事件定义见 `RuntimeLifecycleEvent::AcpTurnFinished`），常规终态路径
/// （`complete_with_issue_done` / `handle_run_failed`）只认 run 边界——追问 reporter 的排空、
/// 尾部工具行兜底拍与续报水位登记只能挂 turn 边界，否则追问后重启会用首 run 的过期水位
/// attach，服务端 seq 与累计用量整体回退。bridge 订阅器在 `AcpTurnFinished` 无在飞映射时
/// 调用；活跃任务的中段 turn 不走此入口（由 RunCompleted 统一收尾）。
pub(crate) async fn finish_followup_run<R: Runtime>(
    app: &AppHandle<R>,
    local_task_id: &str,
    local_run_id: &str,
) {
    finish_run(app, local_task_id, local_run_id, DRAIN_TIMEOUT).await;
}

/// 捕获续报基线（纯读取）：`last_seq` = 已分配的最后一个 seq（`next_seq - 1`），
/// 用量 = 该任务截至当前的累计行。
fn capture_continuation(reporter: &Reporter) -> FinishedContinuation {
    let state = reporter.lock();
    FinishedContinuation {
        remote_task_id: reporter.remote_task_id.clone(),
        last_seq: state.next_seq.saturating_sub(1),
        usage_seed: state.usage.rows(),
    }
}

/// 持久化续报基线（with_state 原子 RMW；失败仅 warn——内存快路径仍然生效，降级为本会话内续报）。
fn persist_continuation<R: Runtime>(app: &AppHandle<R>, continuation: &FinishedContinuation) {
    let Some(context) = app
        .try_state::<crate::state::DesktopState>()
        .and_then(|desktop| desktop.context().ok())
    else {
        return;
    };
    if let Err(error) = context.app().with_state(|state| {
        apply_continuation_write(
            state,
            &continuation.remote_task_id,
            continuation.last_seq,
            &continuation.usage_seed,
        );
        (true, ())
    }) {
        warn!(
            task = %continuation.remote_task_id,
            %error,
            "multica transcript continuity persist failed"
        );
    }
}

async fn run_worker(reporter: Arc<Reporter>) {
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = reporter.notify.notified() => {}
        }

        let (batch, pending_usage, closing, stopped) = {
            let mut guard = reporter.lock();
            let state = &mut *guard;
            if state.closing {
                // 收尾拍：兜底补出始终没等到实质输入/终态的工具行，再排空。
                state.project.finalize();
            } else {
                // 文本按 flush 节奏成段：不跨批次累积成一个巨块。
                state.project.close_segment();
            }
            let batch = state.project.drain(&mut state.next_seq);
            let pending_usage: Vec<String> = state.usage_pending.iter().cloned().collect();
            state.usage_pending.clear();
            (batch, pending_usage, state.closing, state.stopped)
        };

        if !pending_usage.is_empty() && !stopped {
            reporter.refresh_usage(&pending_usage).await;
        }

        if !batch.is_empty() && !stopped {
            match (reporter.client)() {
                None => {
                    warn!(task = %reporter.remote_task_id, "multica transcript batch dropped: no client");
                }
                Some(client) => {
                    match client
                        .post_task_messages(&reporter.remote_task_id, &batch)
                        .await
                    {
                        Ok(()) => {}
                        Err(MulticaError::TaskNotFound) => {
                            warn!(
                                task = %reporter.remote_task_id,
                                "multica transcript stopped: task not found"
                            );
                            reporter.lock().stopped = true;
                        }
                        Err(error) => {
                            // 单批失败即丢（不重试：重发同 seq 会产生重复行）。
                            warn!(
                                task = %reporter.remote_task_id,
                                count = batch.len(),
                                %error,
                                "multica transcript batch dropped"
                            );
                        }
                    }
                }
            }
        }

        if !stopped {
            report_usage_if_dirty(&reporter).await;
        }

        if closing {
            report_usage(&reporter).await;
            let done = reporter.lock().done.take();
            if let Some(done) = done {
                let _ = done.send(());
            }
            return;
        }
    }
}

async fn report_usage_if_dirty(reporter: &Reporter) {
    let dirty = {
        let mut state = reporter.lock();
        std::mem::take(&mut state.usage_dirty)
    };
    if dirty {
        report_usage(reporter).await;
    }
}

async fn report_usage(reporter: &Reporter) {
    let rows = {
        let state = reporter.lock();
        if state.stopped {
            return;
        }
        state.usage.rows()
    };
    // 无用量行（例如 agent 不输出 usage）就没必要打请求；有行时重报累计值幂等。
    if rows.is_empty() {
        return;
    }
    let Some(client) = (reporter.client)() else {
        return;
    };
    if let Err(error) = client
        .post_task_usage(&reporter.remote_task_id, &rows)
        .await
    {
        warn!(task = %reporter.remote_task_id, %error, "multica usage report dropped");
    }
}

/// token 用量累计器：按 `(provider, model)` 汇总，按 `(attempt_dir, turn_seq)` 去重。
#[derive(Default)]
struct UsageAccumulator {
    seen: HashSet<(String, u64)>,
    totals: BTreeMap<(String, String), TokenSums>,
}

impl UsageAccumulator {
    /// 以历史累计行为种子构造（追问 run，M5-de）：服务端按 `(task_id, provider, model)`
    /// 整体替换，追问 run 的新增用量必须在旧累计之上继续加，否则历史用量被覆盖丢失。
    fn seeded(seed: &[UsageRow]) -> Self {
        let mut accumulator = Self::default();
        accumulator.apply_seed(seed);
        accumulator
    }

    fn apply_seed(&mut self, seed: &[UsageRow]) {
        for row in seed {
            let key = (row.provider.clone(), row.model.clone());
            let totals = self.totals.entry(key).or_default();
            totals.input += row.input_tokens;
            totals.output += row.output_tokens;
            totals.cache_read += row.cache_read_tokens;
            totals.cache_write += row.cache_write_tokens;
        }
    }
}

#[derive(Default, Clone, Copy)]
struct TokenSums {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

impl UsageAccumulator {
    /// 返回是否有新片段被累计。
    fn merge(
        &mut self,
        attempt_dir: &str,
        segments: Vec<gold_band::acp::usage::AcpPromptUsageSegment>,
    ) -> bool {
        let mut merged = false;
        for segment in segments {
            if !self
                .seen
                .insert((attempt_dir.to_string(), segment.turn_seq))
            {
                continue;
            }
            // provider 归一化为 trim+小写：服务端按 (task_id, provider, model) UPSERT 且自动小写
            // provider，若本端按原始大小写分桶，"Anthropic"/"anthropic" 会在服务端落到同一行
            // 先后覆盖 → 少算 token。聚合键必须与服务端归一化一致；model 服务端不归一化，保持原样。
            // provider 为空串 → 服务端从任务 runtime 回填（协议约定），无需本端兜底。
            let key = (
                segment
                    .provider
                    .map(|provider| provider.trim().to_ascii_lowercase())
                    .unwrap_or_default(),
                segment.model.unwrap_or_default(),
            );
            let totals = self.totals.entry(key).or_default();
            totals.input += segment.usage.input_tokens.unwrap_or(0);
            totals.output += segment.usage.output_tokens.unwrap_or(0);
            totals.cache_read += segment.usage.cached_read_tokens.unwrap_or(0);
            totals.cache_write += segment.usage.cached_write_tokens.unwrap_or(0);
            merged = true;
        }
        merged
    }

    fn rows(&self) -> Vec<UsageRow> {
        self.totals
            .iter()
            .filter(|(_, totals)| {
                totals.input + totals.output + totals.cache_read + totals.cache_write > 0
            })
            .map(|((provider, model), totals)| UsageRow {
                provider: provider.clone(),
                model: model.clone(),
                input_tokens: totals.input,
                output_tokens: totals.output,
                cache_read_tokens: totals.cache_read,
                cache_write_tokens: totals.cache_write,
                // 无权威费用：0 让服务端按费率估算（协议约定）。
                cost_usd_ticks: 0,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reporter() -> Arc<Reporter> {
        Arc::new(Reporter::new(
            "task-1".to_string(),
            Arc::new(|| None),
            None,
            None,
        ))
    }

    fn seed_row(provider: &str, model: &str, input: u64) -> UsageRow {
        UsageRow {
            provider: provider.to_string(),
            model: model.to_string(),
            input_tokens: input,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd_ticks: 0,
        }
    }

    fn cont(remote: &str, last_seq: u64, seed: Vec<UsageRow>) -> FinishedContinuation {
        FinishedContinuation {
            remote_task_id: remote.to_string(),
            last_seq,
            usage_seed: seed,
        }
    }

    fn usage_segment(turn_seq: u64, input: u64) -> gold_band::acp::usage::AcpPromptUsageSegment {
        gold_band::acp::usage::AcpPromptUsageSegment {
            turn_id: format!("turn-{turn_seq}"),
            turn_seq,
            elapsed_ms: None,
            provider: Some("anthropic".to_string()),
            model: Some("claude-sonnet-5-5".to_string()),
            usage: gold_band::acp::usage::AcpPromptTokenUsage {
                input_tokens: Some(input),
                output_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
                total_tokens: None,
            },
        }
    }

    fn completed_entry(remote: &str, local: &str) -> RemoteCompletedTask {
        RemoteCompletedTask {
            remote_task_id: remote.to_string(),
            local_task_id: local.to_string(),
            local_run_id: "run-1".to_string(),
            workspace_id: "ws-1".to_string(),
            local_project_id: "proj-1".to_string(),
            issue_ref: None,
            kind: None,
            status: "completed".to_string(),
            title: "t".to_string(),
            completed_at: "2026-11-18T00:00:00Z".to_string(),
            issue_status_category: None,
            issue_status_name: None,
            dpms_required: false,
            dpms_story_id: None,
        }
    }

    #[test]
    fn registry_scopes_runs_by_local_task_and_run() {
        let registry = TranscriptRegistry::default();
        let reporter = reporter();
        registry.insert("task-a", "run-1", reporter.clone());
        assert!(registry.find("task-a", "run-1").is_some());
        assert!(registry.find("task-a", "run-2").is_none());
        assert!(registry.find("task-b", "run-1").is_none());
        assert!(registry.remove("task-a", "run-1").is_some());
        assert!(registry.find("task-a", "run-1").is_none());
        assert!(registry.lock().is_empty());
    }

    #[test]
    fn finish_is_idempotent_for_missing_run() {
        let registry = TranscriptRegistry::default();
        assert!(registry.remove("task-a", "run-1").is_none());
    }

    #[test]
    fn usage_accumulates_and_dedupes_by_attempt_turn() {
        let mut accumulator = UsageAccumulator::default();
        let segment =
            |turn_seq: u64, input: u64, output: u64| gold_band::acp::usage::AcpPromptUsageSegment {
                turn_id: format!("turn-{turn_seq}"),
                turn_seq,
                elapsed_ms: None,
                provider: Some("anthropic".to_string()),
                model: Some("claude-sonnet-5-5".to_string()),
                usage: gold_band::acp::usage::AcpPromptTokenUsage {
                    input_tokens: Some(input),
                    output_tokens: Some(output),
                    cached_read_tokens: Some(0),
                    cached_write_tokens: Some(0),
                    total_tokens: None,
                },
            };
        assert!(accumulator.merge("attempt-1", vec![segment(1, 10, 5)]));
        assert!(!accumulator.merge("attempt-1", vec![segment(1, 10, 5)]));
        assert!(accumulator.merge("attempt-1", vec![segment(2, 20, 7)]));
        let rows = accumulator.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "anthropic");
        assert_eq!(rows[0].input_tokens, 30);
        assert_eq!(rows[0].output_tokens, 12);
        assert_eq!(rows[0].cost_usd_ticks, 0);
    }

    #[test]
    fn usage_rows_split_by_provider_and_model() {
        let mut accumulator = UsageAccumulator::default();
        let segment = |provider: &str, model: &str, input: u64| {
            gold_band::acp::usage::AcpPromptUsageSegment {
                turn_id: "turn".to_string(),
                turn_seq: input,
                elapsed_ms: None,
                provider: Some(provider.to_string()),
                model: Some(model.to_string()),
                usage: gold_band::acp::usage::AcpPromptTokenUsage {
                    input_tokens: Some(input),
                    output_tokens: None,
                    cached_read_tokens: None,
                    cached_write_tokens: None,
                    total_tokens: None,
                },
            }
        };
        accumulator.merge("attempt-1", vec![segment("a", "m1", 1)]);
        accumulator.merge("attempt-2", vec![segment("b", "m2", 2)]);
        let rows = accumulator.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].provider, "a");
        assert_eq!(rows[1].provider, "b");
    }

    /// 服务端按 `(task_id, provider, model)` UPSERT 且自动小写 provider：本端聚合键必须先
    /// 归一化，否则大小写/空白漂移会被拆成多条 entry，服务端同 key 后写胜 → 少算 token。
    #[test]
    fn usage_provider_case_and_space_drift_collapses_into_one_row() {
        let mut accumulator = UsageAccumulator::default();
        let segment = |turn_seq: u64, provider: &str, input: u64| {
            gold_band::acp::usage::AcpPromptUsageSegment {
                turn_id: format!("turn-{turn_seq}"),
                turn_seq,
                elapsed_ms: None,
                provider: Some(provider.to_string()),
                model: Some("claude-sonnet-5-5".to_string()),
                usage: gold_band::acp::usage::AcpPromptTokenUsage {
                    input_tokens: Some(input),
                    output_tokens: None,
                    cached_read_tokens: None,
                    cached_write_tokens: None,
                    total_tokens: None,
                },
            }
        };
        accumulator.merge("attempt-1", vec![segment(1, "Anthropic ", 10)]);
        accumulator.merge("attempt-2", vec![segment(2, "ANTHROPIC", 20)]);
        accumulator.merge("attempt-3", vec![segment(3, "anthropic", 30)]);
        let rows = accumulator.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "anthropic");
        assert_eq!(rows[0].input_tokens, 60);
    }

    /// provider 缺失 → 空串上报，服务端从任务 runtime 回填（协议约定，本端不做兜底猜测）。
    #[test]
    fn usage_without_provider_is_reported_empty_for_server_backfill() {
        let mut accumulator = UsageAccumulator::default();
        let segment = gold_band::acp::usage::AcpPromptUsageSegment {
            turn_id: "turn".to_string(),
            turn_seq: 1,
            elapsed_ms: None,
            provider: None,
            model: Some("claude-sonnet-5-5".to_string()),
            usage: gold_band::acp::usage::AcpPromptTokenUsage {
                input_tokens: Some(5),
                output_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
                total_tokens: None,
            },
        };
        assert!(accumulator.merge("attempt-1", vec![segment]));
        let rows = accumulator.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider, "");
        assert_eq!(rows[0].input_tokens, 5);
    }

    /// finish 协议：closing 后 worker 必须排空退出并回 done 信号（客户端不可用时也不许卡住）。
    #[tokio::test]
    async fn worker_signals_done_after_closing_tick() {
        let reporter = reporter();
        reporter.lock().project.push_error("boom");
        reporter
            .lock()
            .usage_pending
            .insert("attempt-1".to_string());
        let worker = tokio::spawn(run_worker(reporter.clone()));

        let (done_tx, done_rx) = oneshot::channel();
        {
            let mut state = reporter.lock();
            state.done = Some(done_tx);
            state.closing = true;
        }
        reporter.notify.notify_one();

        tokio::time::timeout(Duration::from_secs(2), done_rx)
            .await
            .expect("worker must signal done")
            .expect("done sender must fire");
        worker.await.expect("worker task must not panic");
        // 错误消息占用了 seq 1，排空后水位前进。
        assert_eq!(reporter.lock().next_seq, 2);
    }

    /// 收尾拍必须兜底补出「只有占位、始终没到终态」的工具行，否则该工具在 multica
    /// 执行记录里静默消失。
    #[tokio::test]
    async fn closing_tick_finalizes_placeholder_only_tool_rows() {
        let reporter = reporter();
        {
            let mut state = reporter.lock();
            let event = gold_band::acp::events::normalize_session_update(
                1,
                None,
                &serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call-1",
                    "title": "Bash",
                    "status": "in_progress",
                    "rawInput": {},
                }),
            );
            state.project.observe(&event);
        }
        let worker = tokio::spawn(run_worker(reporter.clone()));

        let (done_tx, done_rx) = oneshot::channel();
        {
            let mut state = reporter.lock();
            state.done = Some(done_tx);
            state.closing = true;
        }
        reporter.notify.notify_one();

        tokio::time::timeout(Duration::from_secs(2), done_rx)
            .await
            .expect("worker must signal done")
            .expect("done sender must fire");
        worker.await.expect("worker task must not panic");
        // 补出的 tool_use 占用 seq 1。
        assert_eq!(reporter.lock().next_seq, 2);
    }

    // ── M5-de：终态后追问 run 续报 ─────────────────────────────────────────────

    /// 核心不变量：追问 run 挂载后 seq 接续上一 run 水位、用量以历史累计为种子继续累加。
    #[test]
    fn followup_run_attaches_with_seq_and_usage_continuity() {
        let registry = TranscriptRegistry::default();
        registry.record_finished(
            "local-1",
            cont(
                "remote-1",
                3,
                vec![seed_row("anthropic", "claude-sonnet-5-5", 30)],
            ),
        );
        let reporter = registry
            .attach_followup("local-1", "run-2", Arc::new(|| None))
            .expect("终态后追问 run 必须挂载 reporter（M5-de）");
        assert_eq!(reporter.lock().next_seq, 4, "seq 接续水位 3 → 从 4 继续");
        let rows = reporter.lock().usage.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_tokens, 30, "用量带历史种子");
        // 追问新增用量累加在种子之上（服务端整体替换 → 必须报任务级累计总量）。
        reporter
            .lock()
            .usage
            .merge("attempt-2", vec![usage_segment(1, 10)]);
        let rows = reporter.lock().usage.rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_tokens, 40, "30 种子 + 10 新增 = 累计 40");
    }

    /// M5-de 修复回归：无在飞映射的追问 turn 边界（`AcpTurnFinished`）必须收尾 reporter
    /// 并推进续报水位。Direct 追问不产生新的 RunCompleted——若边界不收尾，追问后重启会用
    /// 首 run 的过期水位 attach，服务端 seq 与用量整体回退；reporter 也会泄漏（worker
    /// 永不退出、收尾兜底拍缺失）。
    #[tokio::test]
    async fn followup_turn_boundary_finalizes_reporter_and_watermark() {
        let app = tauri::test::mock_app();
        app.manage(TranscriptRegistry::default());
        let registry = app.state::<TranscriptRegistry>();
        // 首 run 收尾登记的基线（seq 水位 2）。
        registry.record_finished("local-1", cont("remote-1", 2, vec![]));
        // 追问首事件懒挂载（同一 local run 复用键，与生产追问路径一致）+ 一条追问回复。
        let reporter = registry
            .attach_followup("local-1", "run-1", Arc::new(|| None))
            .expect("终态后追问必须挂载 reporter");
        {
            let mut guard = reporter.lock();
            guard.project.observe(&gold_band::acp::events::AcpUiEvent {
                id: "e-1".to_string(),
                seq: 1,
                timestamp: "2026-11-18T00:00:00Z".to_string(),
                kind: "textDelta".to_string(),
                session_id: None,
                content: Some("追问后的回复".to_string()),
                title: None,
                tool_call_id: None,
                status: None,
                started_seq: None,
                ended_seq: None,
                started_at: None,
                ended_at: None,
                timing: None,
                raw: None,
            });
        }
        // AcpTurnFinished 边界（无在飞映射）。
        finish_followup_run(app.handle(), "local-1", "run-1").await;
        assert!(
            registry.find("local-1", "run-1").is_none(),
            "追问 turn 收尾后 reporter 必须移除（M5-de 修复）"
        );
        let finished = registry
            .finished
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let continuation = finished
            .get("local-1")
            .expect("收尾必须登记续报基线（重启恢复的事实源）");
        assert_eq!(
            continuation.last_seq, 3,
            "水位必须推进到追问消息之后（seq 3 已分配），否则重启续报回退"
        );
    }

    /// 同 remote 已有在飞 reporter 时拒绝挂载（两路并发写同一 seq 空间 → 服务端交错行）。
    #[test]
    fn followup_attach_refuses_while_same_remote_reporter_active() {
        let registry = TranscriptRegistry::default();
        registry.record_finished("local-1", cont("remote-1", 2, vec![]));
        let active = Arc::new(Reporter::new(
            "remote-1".to_string(),
            Arc::new(|| None),
            None,
            None,
        ));
        registry.insert("local-other", "run-9", active);
        assert!(
            registry
                .attach_followup("local-1", "run-2", Arc::new(|| None))
                .is_none(),
            "同 remote 在飞时不得挂载第二路"
        );
        assert!(
            registry.find("local-1", "run-2").is_none(),
            "拒绝挂载即不得注册"
        );
    }

    /// finish 登记清除 foreign 误标：终态排空竞态窗口内被异步解析标过的任务仍可续报。
    #[test]
    fn record_finished_clears_foreign_marker() {
        let registry = TranscriptRegistry::default();
        registry.mark_resolved("local-1", FollowupResolution::Foreign);
        registry.record_finished("local-1", cont("remote-1", 5, vec![]));
        assert!(
            registry
                .attach_followup("local-1", "run-2", Arc::new(|| None))
                .is_some(),
            "水位登记后 foreign 标记不得遮蔽挂载"
        );
    }

    /// 水位捕获：排空后 next_seq-1 = 已分配 seq 高水位（超时路径同一捕获逻辑，空洞允许）。
    #[test]
    fn continuation_capture_reports_allocated_seq_watermark() {
        let reporter = reporter();
        {
            let mut guard = reporter.lock();
            let state = &mut *guard;
            state.project.push_error("boom");
            let batch = state.project.drain(&mut state.next_seq);
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].seq, 1);
        }
        let continuation = capture_continuation(&reporter);
        assert_eq!(continuation.remote_task_id, "task-1");
        assert_eq!(continuation.last_seq, 1);
    }

    /// 重启恢复解析：只认最新 completed 条目（旧 remote 已被 rerun 顶替，续报会写错任务）；
    /// 最新条目无持久水位（旧版本收尾）→ 不续报（seq 基线未知，宁可少报不可交错）。
    #[test]
    fn resolve_followup_baseline_reads_newest_completed_with_watermark() {
        let mut state = StateConfig::default();
        state.remote_completed_tasks = vec![
            completed_entry("remote-2", "local-9"),
            completed_entry("remote-1", "local-9"),
        ];
        state.remote_transcript_continuity = Some(HashMap::from([(
            "remote-1".to_string(),
            RemoteTranscriptContinuity {
                last_seq: 7,
                usage_rows: vec![],
            },
        )]));
        assert!(
            resolve_followup_baseline(&state, "local-9").is_none(),
            "最新条目无水位 → 不续报（不得回退旧 remote）"
        );
        let continuity = state.remote_transcript_continuity.as_mut().unwrap();
        continuity.insert(
            "remote-2".to_string(),
            RemoteTranscriptContinuity {
                last_seq: 9,
                usage_rows: vec![RemoteTranscriptUsageRow {
                    provider: "anthropic".to_string(),
                    model: "claude-sonnet-5-5".to_string(),
                    input_tokens: 25,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                }],
            },
        );
        let baseline =
            resolve_followup_baseline(&state, "local-9").expect("最新条目有水位 → 可续报");
        assert_eq!(baseline.remote_task_id, "remote-2");
        assert_eq!(baseline.last_seq, 9);
        assert_eq!(baseline.usage_seed[0].input_tokens, 25);
        assert_eq!(baseline.usage_seed[0].cost_usd_ticks, 0);
        assert!(
            resolve_followup_baseline(&state, "local-x").is_none(),
            "非 multica 任务 → None"
        );
    }

    /// 持久化写入：当前键无条件保留（首 run 排空先于 finalize 落历史），
    /// completed 历史外的旧水位清理（历史有界 → 水位有界）。
    #[test]
    fn apply_continuation_write_prunes_stale_keys() {
        let mut state = StateConfig::default();
        state.remote_completed_tasks = vec![completed_entry("remote-keep", "local-a")];
        state.remote_transcript_continuity = Some(HashMap::from([
            (
                "remote-keep".to_string(),
                RemoteTranscriptContinuity {
                    last_seq: 2,
                    usage_rows: vec![],
                },
            ),
            (
                "remote-old".to_string(),
                RemoteTranscriptContinuity {
                    last_seq: 1,
                    usage_rows: vec![],
                },
            ),
        ]));
        apply_continuation_write(
            &mut state,
            "remote-cur",
            5,
            &[seed_row("anthropic", "claude-sonnet-5-5", 10)],
        );
        let map = state.remote_transcript_continuity.as_ref().unwrap();
        let current = map.get("remote-cur").unwrap();
        assert_eq!(current.last_seq, 5);
        assert_eq!(current.usage_rows.len(), 1);
        assert_eq!(current.usage_rows[0].input_tokens, 10);
        assert!(map.contains_key("remote-keep"), "历史内水位保留");
        assert!(!map.contains_key("remote-old"), "历史外旧水位清理");
    }
}
