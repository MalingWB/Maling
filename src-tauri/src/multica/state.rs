//! multica 运行期内存状态（开发设计 2.2.6 / 2.5）。
//!
//! **不进 `DesktopState`**：由 loop_/bridge 共享 `Arc<Mutex<MulticaRuntimeState>>`
//! （作为独立 tauri managed state，非 DesktopState 的 Mutex 池字段）。
//! `runtime_ids` 为缓存（register 幂等取回，丢失下次启动重建），M2 仅内存持有；
//! 待持久化的 pending_issues / task_conversations 在 M4 进库层 StateConfig。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// 运行期内存状态容器。
#[derive(Default)]
pub struct MulticaRuntimeState {
    /// workspace_id → server 分配的 runtime_id（register 幂等取回，内存缓存）。
    pub runtime_ids: HashMap<String, String>,
    /// remote_task_id → 本地 task/run 映射（claim/start 后填充；bridge 归属用）。
    pub active_runs: HashMap<String, ActiveRemoteRun>,
    /// 完成路径认领集（M5-bp）：正在执行 complete+finalize 的 remote_task_id。
    ///
    /// 门控命中时同一终 turn 会触发 `RunCompleted` 与 `AcpTurnFinished` 两个事件，且两者的
    /// 归属查找都发生在各自订阅器回调（任一 finalize 之前）——不加守卫则两个 handler 并发
    /// 跑完整完成路径（重复 complete HTTP + 冗余日志/通知）。认领保证完成路径单飞：先到者
    /// 进入，后到者整体退出；认领随 finalize 终结（成功/失败皆走 finalize，见 bridge）。
    pub completing: HashSet<String>,
    /// 对账进行中标记（状态对账改造，单飞）：batch-status 对账由页面 mount / 30s 周期 /
    /// 手动刷新三处触发，慢响应期间可能再次触发——重复对账虽幂等（值 diff 后写），但重复
    /// 扇出 batch 请求无意义。全局单飞（对账无任务粒度，与 `completing` 的按 id 认领不同）。
    pub reconcile_in_flight: bool,
    /// issue → 最近一次即时上送的 `completion_output` 值（M5-cf 工作流阶段性上传的发送侧
    /// 去重投影：值未变即跳过 PUT，逐 turn 触发下不重发）。
    ///
    /// 权威值在服务端 `issue.completion_output`（后写胜）；本地仅内存缓存，进程重启丢失
    /// 无害（最坏同值重发一次，PUT 幂等）。任务终态时随 [`MulticaRuntimeState::drop_active_run`]
    /// 同生命周期清理（见 bridge `finalize_terminal`）。
    pub issue_output_pushed: HashMap<String, String>,
    /// 等待用户决策的上游拉取槽（M5-ch 决策点）：键 `"{local_run_id}/{node_id}"`。
    ///
    /// 拉取解析器首拉空值时注册（发 pending 事件给前端），命令侧写入决策并 notify；resolve
    /// 结束（任意路径）或 run 终态时移除。与 `issue_output_pushed` 同为 run 级内存态。
    pub upstream_pull_decisions: HashMap<String, UpstreamPullPending>,
}

/// 上游拉取的等待决策（M5-ch）：父 `completion_output` 首拉为空时交还控制权。
#[derive(Debug, Clone)]
pub enum UpstreamPullDecision {
    /// 继续等待（恢复轮询，占用同一总预算）。
    Retry,
    /// 跳过拉取，用户手动提供交付说明文本（已 trim 非空）。
    Manual(String),
    /// 跳过拉取，直接继续执行。
    Skip,
}

/// 单个等待决策的拉取槽：notify 唤醒等待方，decision 为已到达的决策（None = 无人决策）。
pub struct UpstreamPullPending {
    pub notify: Arc<tokio::sync::Notify>,
    pub decision: Option<UpstreamPullDecision>,
}

/// 单个在飞 remote task 的本地映射。
///
/// `local_task_id`/`local_run_id` 为 display id（与 `RuntimeLifecycleEvent` 发出的 task_id/run_id
/// 同形，bridge 据此反向归属：本地 lifecycle 事件 → remote task）。
#[derive(Debug, Clone)]
pub struct ActiveRemoteRun {
    /// 该任务所属 multica workspace（complete/fail 路径不需，但失败回显/重跑需）。
    pub workspace_id: String,
    /// 该任务执行时选定的本地工作区 project_id（绑定模型下沉到任务级：工作区不再绑本地
    /// 目录，每次执行由 composer 下拉选定，start 时随 input.project_id 写入）。
    /// running 行本地深链 + cancel 解析 workspace path 用。
    pub local_project_id: String,
    /// 本地 task display id（事件归属键 = `RunCompleted.task_id`）。
    pub local_task_id: String,
    /// 本地 run display id（事件归属键 = `RunCompleted.run_id`，配 task_id 唯一定位）。
    pub local_run_id: String,
    pub issue_id: Option<String>,
    /// 行标签（claim 时的 thread_name，Issue 3C「最近完成」快照用，避免终态读盘）。
    pub title: Option<String>,
    pub started_at: String,
    /// issue 类型快照（claim 响应的 `issue_kind`，story dev/test 拆分）。终态历史
    /// `RemoteCompletedTask.issue_kind` 的写入源——running 行与终态行的类型徽标都由它供给。
    /// `is_ready` 不快照：领取后无「就绪」语义。
    pub issue_kind: Option<String>,
    /// run 模式快照（发送时 `ConversationCreateInputVm.run_mode` 的字面值，M5-bp 完成门控的
    /// 事实源）。lifecycle 事件不携带模式信息，bridge 的 run/turn 收尾判定只能靠此快照。
    /// 仅内存（重启后 active_runs 丢失由 recover-orphans → auto-retry → re-claim resume
    /// 既有链路兜底，resume 时重新快照）。
    pub run_mode: String,
}

/// 门控命中的 run 模式字面量（`ConversationCreateInputVm.run_mode` 的 "direct" 值，
/// 见 `view_models_conversation.rs` / `scheduled_content_snapshot` 的模式映射）。
pub const REMOTE_RUN_MODE_DIRECT: &str = "direct";

/// 工作流 run 模式字面量（同上，M5-cf 阶段性上传的作用域判定）。
pub const REMOTE_RUN_MODE_WORKFLOW: &str = "workflow";

/// 共享句柄：loop 创建（managed），bridge（M4）取同一份。
pub type SharedMulticaState = Arc<Mutex<MulticaRuntimeState>>;

/// 构造共享运行期状态（main.rs setup 经 `.manage()` 注入）。
pub fn shared_state() -> SharedMulticaState {
    Arc::new(Mutex::new(MulticaRuntimeState::default()))
}

/// 进行中 connect 的取消槽（M5-ay：连接弹窗「取消连接」）。
///
/// [`connect_multica`](crate::commands::connect_multica) 注册当前浏览器登录的
/// `CancellationToken`，[`cancel_multica_connect`](crate::commands::cancel_multica_connect)
/// 触发之；命令结束（任意路径）凭 [`Self::register`] 返回的登记 id 经 [`Self::clear_if_same`]
/// 只清理自己的登记——迟到的清理不误删下一次连接（`CancellationToken` 无相等语义，以单调
/// 登记id 认领）。UI 上连接弹窗互斥（单飞行），注册时若仍有旧 token（异常残留）先取消之。
/// 独立于 [`MulticaRuntimeState`]：这是连接生命周期的一次性槽，不是任务运行态。
#[derive(Default)]
pub struct MulticaConnectCancel(Mutex<MulticaConnectCancelSlot>);

/// 槽内状态：单调递增登记 id + 当前 token（id 凭据 =「clear 时只清自己的」判据）。
#[derive(Default)]
struct MulticaConnectCancelSlot {
    next_id: u64,
    current: Option<(u64, tokio_util::sync::CancellationToken)>,
}

impl MulticaConnectCancel {
    /// 注册本次连接的取消 token，返回登记 id（结束时凭 id 清理）；槽内仍有旧 token
    /// （异常残留）时先取消再覆盖。
    pub fn register(&self, token: &tokio_util::sync::CancellationToken) -> u64 {
        let mut guard = self.0.lock().expect("multica connect cancel slot poisoned");
        guard.next_id += 1;
        let id = guard.next_id;
        if let Some((_, stale)) = guard.current.replace((id, token.clone())) {
            stale.cancel();
        }
        id
    }

    /// 取消当前进行中的连接（若有）；无进行中连接时为幂等 no-op。
    pub fn cancel_current(&self) {
        if let Some((_, token)) = self
            .0
            .lock()
            .expect("multica connect cancel slot poisoned")
            .current
            .as_ref()
        {
            token.cancel();
        }
    }

    /// 命令结束时清理：仅当槽内仍是自己的登记（防迟到清理误删下一次连接的注册）。
    pub fn clear_if_same(&self, id: u64) {
        let mut guard = self.0.lock().expect("multica connect cancel slot poisoned");
        if guard.current.as_ref().map(|(cur, _)| *cur) == Some(id) {
            guard.current = None;
        }
    }
}

impl MulticaRuntimeState {
    /// 写入 workspace → runtime_id 映射（register 成功后调用）。
    pub fn set_runtime_id(&mut self, workspace_id: &str, runtime_id: &str) {
        self.runtime_ids
            .insert(workspace_id.to_string(), runtime_id.to_string());
    }

    /// 取某 workspace 的 runtime_id。
    pub fn runtime_id(&self, workspace_id: &str) -> Option<&str> {
        self.runtime_ids.get(workspace_id).map(String::as_str)
    }

    /// 所有已注册 runtime_id（recover-orphans 遍历用）。
    pub fn runtime_ids(&self) -> Vec<String> {
        self.runtime_ids.values().cloned().collect()
    }

    /// 清空 runtime_id 注册缓存（断开连接时调用）。
    ///
    /// 仅清 register 缓存——重连后 loop 全量/增量 register 会重建。**保留** `active_runs`
    /// （真实在飞本地 run 的 remote 映射；断开后 bridge 上报因无 PAT 失败但不影响本地 run，重连同账号仍有效）。
    pub fn clear_runtime_ids(&mut self) {
        self.runtime_ids.clear();
    }

    /// 清单个 workspace 的 runtime_id 缓存（心跳 404 runtime_not_found 时调用）。
    ///
    /// runtime 行已被服务端删除/失效时，旧 runtime_id 心跳永久 404；清掉后下个 tick
    /// `self_heal_registration` 会重注册取回新 runtime_id（自愈，开发设计 4.1）。
    pub fn clear_runtime_id(&mut self, workspace_id: &str) {
        self.runtime_ids.remove(workspace_id);
    }

    /// 所有已注册 `(workspace_id, runtime_id)` 对（常驻心跳遍历用--需 workspace_id 才能在
    /// 心跳 404 时按 workspace 清缓存触发自愈重注册）。
    pub fn runtime_id_pairs(&self) -> Vec<(String, String)> {
        self.runtime_ids
            .iter()
            .map(|(ws, rid)| (ws.clone(), rid.clone()))
            .collect()
    }

    /// 登记 remote task 的本地映射（M4-c claim+start 后调用）。
    pub fn register_active_run(&mut self, remote_task_id: &str, run: ActiveRemoteRun) {
        self.active_runs.insert(remote_task_id.to_string(), run);
    }

    /// 移除 remote task 映射（终态/取消后调用），返回被移除项供副作用使用。
    ///
    /// 同生命周期清理该 issue 的即时上送去重缓存（`issue_output_pushed`）：任务终态后该键
    /// 不再有消费方，遗留只会让 rerun 后的首个相同块被误判「已发送」而跳过（跳过无害——
    /// 完成路径仍会携带，但清理更诚实）。
    pub fn drop_active_run(&mut self, remote_task_id: &str) -> Option<ActiveRemoteRun> {
        let dropped = self.active_runs.remove(remote_task_id);
        if let Some(run) = &dropped {
            if let Some(issue) = run.issue_id.as_deref().filter(|s| !s.trim().is_empty()) {
                self.issue_output_pushed.remove(issue);
            }
            // M5-ch：run 终态时同生命周期清掉该 run 的拉取决策槽（键前缀 "{local_run_id}/"），
            // 防止 resolver 被中断后槽泄漏（后续同 run id 重跑不得继承旧槽）。
            let prefix = format!("{}/", run.local_run_id);
            self.upstream_pull_decisions
                .retain(|key, _| !key.starts_with(&prefix));
        }
        dropped
    }

    /// 按 (local_task_id, local_run_id) 反查在飞 remote task（bridge 事件归属键）。
    ///
    /// 返回 `(remote_task_id, run 克隆)`（锁内 clone 以便释放锁后再做 async HTTP）。
    pub fn find_active_run_by_local(
        &self,
        local_task_id: &str,
        local_run_id: &str,
    ) -> Option<(String, ActiveRemoteRun)> {
        self.active_runs
            .iter()
            .find(|(_, r)| r.local_task_id == local_task_id && r.local_run_id == local_run_id)
            .map(|(rid, r)| (rid.clone(), r.clone()))
    }

    /// 按 remote_task_id 取在飞映射（cancel 命令用，键即 remote_task_id）。
    pub fn active_run(&self, remote_task_id: &str) -> Option<ActiveRemoteRun> {
        self.active_runs.get(remote_task_id).cloned()
    }

    /// 按本地 task display id 反查在飞映射（M5-cf 拉取解析器用）。
    ///
    /// 一个本地任务同一时刻至多一个在飞 remote run（claim 单飞）；run 维度未知（引擎回调
    /// 只携带 task/node）故按任务级扫描。返回 `(remote_task_id, run 克隆)`。
    pub fn find_active_run_by_task(
        &self,
        local_task_id: &str,
    ) -> Option<(String, ActiveRemoteRun)> {
        self.active_runs
            .iter()
            .find(|(_, r)| r.local_task_id == local_task_id)
            .map(|(rid, r)| (rid.clone(), r.clone()))
    }

    /// 原子认领完成路径（M5-bp 双事件竞态守卫）。
    ///
    /// 返回 false = 已有进行中的完成（另一事件先到认领），调用方须整体退出（complete/
    /// finalize/通知全部由认领方负责）。按 remote_task_id 粒度，不同任务互不阻塞。
    pub fn begin_completion(&mut self, remote_task_id: &str) -> bool {
        self.completing.insert(remote_task_id.to_string())
    }

    /// 终结认领（finalize 时调用，成功/失败皆然；幂等）。之后同任务的后续事件可再认领
    /// （如 server 侧 complete 失败后的迟到重试路径）。
    pub fn end_completion(&mut self, remote_task_id: &str) {
        self.completing.remove(remote_task_id);
    }

    /// 原子认领对账（状态对账改造）：返回 false = 已有对账在飞，调用方整体退出
    /// （结果与广播由在飞方负责）。全局粒度——对账操作无 per-task 语义。
    pub fn begin_reconcile(&mut self) -> bool {
        if self.reconcile_in_flight {
            false
        } else {
            self.reconcile_in_flight = true;
            true
        }
    }

    /// 终结对账认领（命令收尾统一调用，成功/失败皆然；幂等）。
    pub fn end_reconcile(&mut self) {
        self.reconcile_in_flight = false;
    }

    /// 注册上游拉取决策槽（M5-ch）：首拉空值时由拉取解析器调用，返回 notify 句柄供
    /// select 等待。同键已存在时覆盖（同 run 同节点并发 resolve 不存在——claim 单飞；
    /// 覆盖仅作异常残留防御）。
    pub fn register_upstream_pull(
        &mut self,
        local_run_id: &str,
        node_id: &str,
    ) -> Arc<tokio::sync::Notify> {
        let notify = Arc::new(tokio::sync::Notify::new());
        self.upstream_pull_decisions.insert(
            upstream_pull_key(local_run_id, node_id),
            UpstreamPullPending { notify: notify.clone(), decision: None },
        );
        notify
    }

    /// 写入决策并返回该槽的 notify 句柄（锁外 notify_one 唤醒等待方）；无槽 = 决策迟到
    /// （resolve 已结束或 run 已终态），返回 None 由命令侧报结构化错误。
    pub fn decide_upstream_pull(
        &mut self,
        local_run_id: &str,
        node_id: &str,
        decision: UpstreamPullDecision,
    ) -> Option<Arc<tokio::sync::Notify>> {
        let slot = self
            .upstream_pull_decisions
            .get_mut(&upstream_pull_key(local_run_id, node_id))?;
        slot.decision = Some(decision);
        Some(slot.notify.clone())
    }

    /// 取走已到达的决策（None = 尚无人决策）。槽保留——轮询期间迟到的 manual/skip 仍可写入。
    pub fn take_upstream_pull_decision(
        &mut self,
        local_run_id: &str,
        node_id: &str,
    ) -> Option<UpstreamPullDecision> {
        self.upstream_pull_decisions
            .get_mut(&upstream_pull_key(local_run_id, node_id))?
            .decision
            .take()
    }

    /// resolve 结束（任意路径）时移除槽：后续决策命令按无槽报错（前端 settled 事件已收口 UI）。
    pub fn drop_upstream_pull(&mut self, local_run_id: &str, node_id: &str) {
        self.upstream_pull_decisions
            .remove(&upstream_pull_key(local_run_id, node_id));
    }
}

/// 拉取决策槽键：run + node 唯一定位（一个 run 的多个拉取节点可并行等待）。
fn upstream_pull_key(local_run_id: &str, node_id: &str) -> String {
    format!("{local_run_id}/{node_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_run(local: &str) -> ActiveRemoteRun {
        ActiveRemoteRun {
            workspace_id: "ws-1".into(),
            local_project_id: format!("proj-{local}"),
            local_task_id: format!("task-{local}"),
            local_run_id: format!("run-{local}"),
            issue_id: Some(format!("issue-{local}")),
            title: Some(format!("title-{local}")),
            started_at: "2026-08-05T00:00:00".into(),
            issue_kind: Some("dev".into()),
            run_mode: "direct".into(),
        }
    }

    #[test]
    fn set_runtime_id_is_overwritable_and_queryable() {
        let mut state = MulticaRuntimeState::default();
        assert!(state.runtime_id("ws-1").is_none());
        state.set_runtime_id("ws-1", "rt-a");
        assert_eq!(state.runtime_id("ws-1"), Some("rt-a"));
        // 幂等重注册覆盖为同值（server 稳定分配）。
        state.set_runtime_id("ws-1", "rt-a");
        assert_eq!(state.runtime_ids().as_slice(), &["rt-a".to_string()]);
    }

    #[test]
    fn clear_runtime_ids_empties_cache_but_keeps_active_runs() {
        // 断开连接：清 register 缓存，但保留在飞本地 run 的 remote 映射。
        let mut state = MulticaRuntimeState::default();
        state.set_runtime_id("ws-1", "rt-a");
        state.set_runtime_id("ws-2", "rt-b");
        state.register_active_run("remote-1", sample_run("1"));

        state.clear_runtime_ids();

        assert!(state.runtime_ids().is_empty());
        assert!(state.runtime_id("ws-1").is_none());
        // active_runs 保留（断开不改在飞本地 run 的归属映射）。
        assert!(state.active_run("remote-1").is_some());
    }

    #[test]
    fn runtime_ids_returns_all_registered_workspaces() {
        // 常驻心跳源：所有已连接工作空间（与在飞任务解耦，无任务也在线）。
        let mut state = MulticaRuntimeState::default();
        state.set_runtime_id("ws-1", "rt-a");
        state.set_runtime_id("ws-2", "rt-b");
        let mut ids = state.runtime_ids();
        ids.sort();
        assert_eq!(ids, vec!["rt-a".to_string(), "rt-b".to_string()]);

        // 无 active_runs 也照常返回（连接后即持续在线）。
        assert!(state.active_runs.is_empty());
    }

    #[test]
    fn runtime_id_pairs_carries_workspace_for_self_heal() {
        // 心跳遍历需 workspace_id：runtime 行失效 404 时才能按 workspace 清缓存，下 tick 自愈重注册。
        let mut state = MulticaRuntimeState::default();
        state.set_runtime_id("ws-1", "rt-a");
        state.set_runtime_id("ws-2", "rt-b");
        let mut pairs = state.runtime_id_pairs();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![
                ("ws-1".into(), "rt-a".into()),
                ("ws-2".into(), "rt-b".into()),
            ]
        );
    }

    #[test]
    fn clear_runtime_id_singular_drops_one_keeps_rest() {
        // 心跳 404 runtime_not_found：仅清失效那个 workspace 的缓存，其余保留。
        let mut state = MulticaRuntimeState::default();
        state.set_runtime_id("ws-1", "rt-a");
        state.set_runtime_id("ws-2", "rt-b");

        state.clear_runtime_id("ws-1");

        assert!(state.runtime_id("ws-1").is_none(), "失效 workspace 应清掉");
        assert_eq!(
            state.runtime_id("ws-2"),
            Some("rt-b"),
            "其余 workspace 不受影响"
        );
        // 再清不存在的 -> 无副作用。
        state.clear_runtime_id("ws-x");
        assert_eq!(state.runtime_id("ws-2"), Some("rt-b"));
    }

    #[test]
    fn find_active_run_by_local_matches_and_misses() {
        let mut state = MulticaRuntimeState::default();
        state.register_active_run("remote-9", sample_run("9"));
        // 命中：local_task_id + local_run_id 双键匹配。
        let found = state.find_active_run_by_local("task-9", "run-9");
        assert_eq!(found.as_ref().map(|(r, _)| r.as_str()), Some("remote-9"));
        // 串台防护：仅 task_id 匹配但 run_id 不同 → 不命中（多 workspace/多 run 不串台）。
        assert!(
            state
                .find_active_run_by_local("task-9", "run-other")
                .is_none()
        );
        // 完全不命中。
        assert!(state.find_active_run_by_local("task-x", "run-x").is_none());
    }

    #[test]
    fn drop_active_run_returns_and_removes() {
        let mut state = MulticaRuntimeState::default();
        state.register_active_run("remote-9", sample_run("9"));
        let dropped = state.drop_active_run("remote-9");
        assert!(dropped.is_some());
        assert_eq!(dropped.unwrap().local_task_id, "task-9");
        // 已移除 → 再 drop 返回 None。
        assert!(state.drop_active_run("remote-9").is_none());
        assert!(state.active_runs.is_empty());
    }

    #[test]
    fn drop_active_run_clears_issue_push_cache_same_lifecycle() {
        // M5-cf：终态清 issue_output_pushed——rerun 后首个相同块不被误判「已发送」跳过。
        let mut state = MulticaRuntimeState::default();
        state.register_active_run("remote-9", sample_run("9"));
        state
            .issue_output_pushed
            .insert("issue-9".into(), "阶段快照 v1".into());
        state.drop_active_run("remote-9");
        assert!(!state.issue_output_pushed.contains_key("issue-9"));
        // 无 issue 关联的 run（非 issue 来源）drop 时无键可清，不 panic。
        let mut bare = sample_run("8");
        bare.issue_id = None;
        let mut state2 = MulticaRuntimeState::default();
        state2.register_active_run("remote-8", bare);
        state2.drop_active_run("remote-8");
        assert!(state2.issue_output_pushed.is_empty());
    }

    // ---- M5-ch：上游拉取决策槽（首拉空值 → 用户三选一）----

    /// 有界轮询 notify：50ms 内被唤醒 → true（无 reactor 的同步测试用，借临时 runtime）。
    fn notify_wakes_within(notify: &Arc<tokio::sync::Notify>) -> bool {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("test runtime")
            .block_on(async {
                tokio::time::timeout(std::time::Duration::from_millis(50), notify.notified())
                    .await
                    .is_ok()
            })
    }

    #[test]
    fn upstream_pull_slot_roundtrip_manual_decision() {
        // 注册 → 写决策（取回 notify 供锁外唤醒）→ 取走决策（槽保留）→ resolve 收口移除槽。
        let mut state = MulticaRuntimeState::default();
        let notify = state.register_upstream_pull("run-9", "node-1");
        assert!(!notify_wakes_within(&notify), "新槽不得自带唤醒");

        let woken = state
            .decide_upstream_pull(
                "run-9",
                "node-1",
                UpstreamPullDecision::Manual("手动交付说明".into()),
            )
            .expect("已注册槽应命中");
        woken.notify_one();
        assert!(
            notify_wakes_within(&notify),
            "decide 返回的 notify 应唤醒等待方"
        );

        // 取走决策：槽保留（轮询期间迟到的决策仍可写入），再取为 None。
        assert!(matches!(
            state.take_upstream_pull_decision("run-9", "node-1"),
            Some(UpstreamPullDecision::Manual(ref text)) if text == "手动交付说明"
        ));
        assert!(state.take_upstream_pull_decision("run-9", "node-1").is_none());
        // 迟到决策可再写入（槽仍在）：resolve 未收口前决策通道畅通。
        assert!(
            state
                .decide_upstream_pull("run-9", "node-1", UpstreamPullDecision::Skip)
                .is_some()
        );
        assert!(matches!(
            state.take_upstream_pull_decision("run-9", "node-1"),
            Some(UpstreamPullDecision::Skip)
        ));

        state.drop_upstream_pull("run-9", "node-1");
        // 槽移除后：决策无落点（命令侧按 UpstreamPullNotPending 报错）。
        assert!(
            state
                .decide_upstream_pull("run-9", "node-1", UpstreamPullDecision::Retry)
                .is_none()
        );
        assert!(state.take_upstream_pull_decision("run-9", "node-1").is_none());
    }

    #[test]
    fn upstream_pull_slot_keys_scoped_by_run_and_node() {
        // 键 = run/node：同 run 不同节点互不串台；不同 run 同名节点也不串台。
        let mut state = MulticaRuntimeState::default();
        state.register_upstream_pull("run-9", "node-1");
        state.register_upstream_pull("run-9", "node-2");
        state.register_upstream_pull("run-other", "node-1");
        assert_eq!(state.upstream_pull_decisions.len(), 3);

        assert!(
            state
                .decide_upstream_pull("run-9", "node-2", UpstreamPullDecision::Skip)
                .is_some()
        );
        // 只影响 node-2；node-1 与另一 run 的 node-1 均未决策。
        assert!(state.take_upstream_pull_decision("run-9", "node-1").is_none());
        assert!(state.take_upstream_pull_decision("run-other", "node-1").is_none());
        assert!(matches!(
            state.take_upstream_pull_decision("run-9", "node-2"),
            Some(UpstreamPullDecision::Skip)
        ));
    }

    #[test]
    fn drop_active_run_clears_upstream_pull_slots_same_lifecycle() {
        // run 终态 → 前缀清理该 run 全部拉取槽（resolver 被中断后不泄漏；rerun 不继承旧槽）。
        let mut state = MulticaRuntimeState::default();
        state.register_active_run("remote-9", sample_run("9"));
        state.register_upstream_pull("run-9", "node-1");
        state.register_upstream_pull("run-9", "node-2");
        // 其他 run 的槽必须保留（前缀匹配只清自己的）。
        state.register_upstream_pull("run-other", "node-1");
        // 恶意前缀防护：run id "run-9" 不得清掉 "run-9x" 的槽（键含 "/" 分隔）。
        state.register_upstream_pull("run-9x", "node-1");

        state.drop_active_run("remote-9");

        assert!(!state
            .upstream_pull_decisions
            .contains_key("run-9/node-1"));
        assert!(!state
            .upstream_pull_decisions
            .contains_key("run-9/node-2"));
        assert!(state
            .upstream_pull_decisions
            .contains_key("run-other/node-1"));
        assert!(state
            .upstream_pull_decisions
            .contains_key("run-9x/node-1"));
    }

    // ---- M5-bp：完成路径认领（RunCompleted / AcpTurnFinished 双事件竞态单飞）----

    #[test]
    fn begin_completion_claims_exclusively_and_end_releases() {
        let mut state = MulticaRuntimeState::default();
        // 首个认领成功（先到事件进入完成路径）。
        assert!(state.begin_completion("remote-1"));
        // 并发第二事件认领失败 → 整体退出，完成路径单飞。
        assert!(!state.begin_completion("remote-1"));
        // 不同任务互不阻塞（认领按 remote_task_id 粒度）。
        assert!(state.begin_completion("remote-2"));
        // finalize 终结认领（成功/失败皆然）：之后同任务可再认领（重试/新事件）。
        state.end_completion("remote-1");
        assert!(state.begin_completion("remote-1"));
        // 幂等：重复 end 无副作用。
        state.end_completion("remote-1");
        state.end_completion("remote-x");
    }

    // ---- 状态对账改造：对账单飞（mount / 30s 周期 / 手动刷新并发触发）----

    #[test]
    fn begin_reconcile_claims_exclusively_and_end_releases() {
        let mut state = MulticaRuntimeState::default();
        // 首个触发认领成功；在飞期间后续触发整体退出（全局单飞，无任务粒度）。
        assert!(state.begin_reconcile());
        assert!(!state.begin_reconcile());
        // 收尾释放后可再认领；重复 end 幂等。
        state.end_reconcile();
        assert!(state.begin_reconcile());
        state.end_reconcile();
        state.end_reconcile();
    }

    #[test]
    fn active_run_looks_up_by_remote_id() {
        // cancel 命令按 remote_task_id 直查（键即 remote id）。
        let mut state = MulticaRuntimeState::default();
        state.register_active_run("remote-9", sample_run("9"));
        let found = state.active_run("remote-9").expect("已登记应命中");
        assert_eq!(found.local_task_id, "task-9");
        assert_eq!(found.local_run_id, "run-9");
        assert_eq!(found.workspace_id, "ws-1");
        assert!(state.active_run("remote-x").is_none());
    }

    // ---- M5-ay：connect 取消槽（连接弹窗「取消连接」）----

    #[test]
    fn connect_cancel_cancel_current_cancels_registered_token_idempotently() {
        let slot = MulticaConnectCancel::default();
        let token = tokio_util::sync::CancellationToken::new();
        slot.register(&token);

        // 无 token 时 no-op；注册后 cancel_current 触发之（幂等：重复取消无副作用）。
        MulticaConnectCancel::default().cancel_current();
        assert!(!token.is_cancelled());
        slot.cancel_current();
        slot.cancel_current();
        assert!(token.is_cancelled());
    }

    #[test]
    fn connect_cancel_clear_if_same_does_not_remove_next_registration() {
        let slot = MulticaConnectCancel::default();
        let first = tokio_util::sync::CancellationToken::new();
        let first_id = slot.register(&first);

        // 第一连接结束：清掉自己的登记 → 后续 cancel 不影响任何人。
        slot.clear_if_same(first_id);
        slot.cancel_current();
        assert!(!first.is_cancelled());

        // 第二连接注册后，第一连接凭旧 id 的迟到清理已是 no-op，不误删第二连接的登记。
        let second = tokio_util::sync::CancellationToken::new();
        slot.register(&second);
        slot.clear_if_same(first_id);
        slot.cancel_current();
        assert!(second.is_cancelled());
    }

    #[test]
    fn connect_cancel_register_replaces_and_cancels_stale_token() {
        // 异常残留（前次连接未正常收尾）：新注册先取消旧 token，杜绝孤儿登录等待。
        let slot = MulticaConnectCancel::default();
        let stale = tokio_util::sync::CancellationToken::new();
        let fresh = tokio_util::sync::CancellationToken::new();
        slot.register(&stale);
        slot.register(&fresh);

        assert!(stale.is_cancelled(), "被顶替的旧 token 应立即取消");
        assert!(!fresh.is_cancelled(), "新 token 不受影响");
        slot.cancel_current();
        assert!(fresh.is_cancelled());
    }
}
