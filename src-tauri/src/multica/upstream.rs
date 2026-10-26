//! `$upstream.completion-output` 令牌的桌面侧解析器（M5-cf 拉取链路，M5-ch 决策点）与完成
//! 输出协议区段解析器（M5-cg 产出侧门禁的渲染端）。
//!
//! # 上游拉取
//!
//! 链路：本地任务 → 在飞 multica 映射（workspace + 自身 issue）→ `GET /api/issues/{self}`
//! 解析 `parent_issue_id` → `GET /api/issues/{parent}`：
//!
//! - **首拉立即执行**：父 `completion_output` 非空即命中（阶段性快照——父未 done 也可得，
//!   内网确认 2026-10-24：字段非空即下发、PUT 无状态门槛）。
//! - **首个「确认父未产出」开 M5-ch 决策点**（拉取成功但 `completion_output` 为空——含
//!   首拉与其后轮询的首个确认）：发 pending 事件把控制权交给用户三选一——继续等待（恢复
//!   轮询）/ 手动提供交付说明 / 跳过拉取直接继续。60 秒无人应答自动回落轮询（无人值守
//!   语义≈旧口径，不挂死）；总预算（15 分钟）不变，耗尽仍结构化 Timeout。拉取**网络失
//!   败**不开决策点（语义是「拉不到」不是「父未产出」），预算内静默重试，首个后续
//!   「确认空」再开。
//! - 命中后按当前界面语言渲染「上游交付说明」模板（parent / manual / skipped 三变体）。
//!   渲染结果由引擎注入首条 prompt 的隐式隐藏区段（「上游交付说明」），正文中的令牌随之
//!   移除（AGENTS.md system/user prompt 划分：随执行变化的交付数据属 user prompt 区，不内联
//!   进 system prompt 角色正文）。
//!
//! 引擎在 run 专用线程同步调用（`App::resolve_upstream_output`）；本侧经
//! `tauri::async_runtime::block_on` 桥到异步客户端——阻塞上限 = 轮询总预算（15 分钟）。
//! 快照语义：节点启动时的值，父后续覆盖不自动刷新（再次进入该节点重新解析）。
//!
//! # 完成输出协议区段（M5-cg）
//!
//! 引擎在节点装配时按节点身份判定 [`CompletionProtocolDesignation`]（sink / 标记节点 /
//! 验收节点 → Final，标记声明 → Staged，其余 → Prohibited）后同步调用
//! [`MulticaCompletionProtocolResolver`]：任务在飞映射关联 issue（非空白）→ 按当前界面语言
//! 渲染 `remote_task_node_protocol` 对应变体，注入首条 prompt 隐式隐藏区段。本地任务 /
//! 非 issue 关联 → `None`（不注入，与 claim 时冻结段同口径）。纯内存映射 + 模板渲染，无 IO。

use std::sync::Arc;
use std::time::Duration;

use gold_band::app::upstream::{
    CompletionProtocolDesignation, CompletionProtocolSectionResolver, UpstreamOutputResolver,
    UpstreamResolveError,
};
use gold_band::config::DesktopLanguage;
use gold_band::prompts::{
    RUNTIME_REMOTE_TASK_NODE_PROTOCOL, RUNTIME_REMOTE_TASK_UPSTREAM_PULL, prompt_by_language,
    render,
};
use gold_band::provider::PromptHiddenSection;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::multica::bridge::{desktop_context, multica_client, shared_multica_state};
use crate::multica::client::IssueDetail;
use crate::multica::state::{ActiveRemoteRun, SharedMulticaState, UpstreamPullDecision};

/// 有限退避轮询参数（用户决策 2026-10-24：有限退避轮询）：15s 起、×1.5 退避、间隔上限
/// 60s、总预算 15 分钟。集中定义为模块常量（AGENTS：业务常量集中定义）。
const POLL_INITIAL_INTERVAL: Duration = Duration::from_secs(15);
const POLL_MAX_INTERVAL: Duration = Duration::from_secs(60);
const POLL_BACKOFF_FACTOR: f32 = 1.5;
const POLL_BUDGET: Duration = Duration::from_secs(15 * 60);

/// 决策点等待窗口（M5-ch）：首拉空值后等用户三选一的时长；窗口内无人应答自动回落轮询
/// （无人值守语义≈旧口径——不挂死、终态仍由总预算收口）。
const DECISION_FALLBACK_AFTER: Duration = Duration::from_secs(60);

/// 「确认父未产出」、开决策点时发的前端事件（远程任务会话内拉取决策卡片）。
pub(crate) const MULTICA_UPSTREAM_PULL_PENDING_EVENT: &str =
    "gold-band://multica-upstream-pull-pending";

/// 决策点收口（任意 resolve 退出路径）时发的前端事件：outcome = resolved / manual /
/// skipped / timeout，前端据此关闭决策卡片。
pub(crate) const MULTICA_UPSTREAM_PULL_SETTLED_EVENT: &str =
    "gold-band://multica-upstream-pull-settled";

/// multica 域解析器（main.rs setup 装入 DesktopState，`app()` 组装时挂到引擎 App）。
pub(crate) struct MulticaUpstreamResolver {
    handle: AppHandle,
}

impl MulticaUpstreamResolver {
    pub(crate) fn new(handle: AppHandle) -> Self {
        Self { handle }
    }

    async fn resolve(&self, task_id: &str, node_id: &str) -> Result<String, UpstreamResolveError> {
        let Some(shared) = shared_multica_state(&self.handle) else {
            return Err(UpstreamResolveError::Network(
                "multica shared state unavailable".into(),
            ));
        };
        // 任务级反查：一个本地任务同一时刻至多一个在飞 remote run（claim 单飞）。
        let Some((_, run)) = shared
            .lock()
            .expect("multica state lock")
            .find_active_run_by_task(task_id)
        else {
            return Err(UpstreamResolveError::Network(format!(
                "no in-flight multica mapping for local task {task_id}"
            )));
        };
        let Some(issue) = run
            .issue_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Err(UpstreamResolveError::Network(format!(
                "multica task for {task_id} has no issue binding"
            )));
        };
        let Some(context) = desktop_context(&self.handle) else {
            return Err(UpstreamResolveError::Network(
                "multica desktop context unavailable".into(),
            ));
        };
        let Some(client) = multica_client(&context) else {
            return Err(UpstreamResolveError::Network(
                "multica client unavailable (disconnected)".into(),
            ));
        };
        let workspace_id = run.workspace_id.clone();

        // 父寻址（claim 响应不带 parent_issue_id，detail 总下发：无父为 null）。
        let parent = client
            .get_issue(&workspace_id, issue)
            .await
            .map_err(|e| UpstreamResolveError::Network(format!("resolve own issue: {e}")))?;
        let Some(parent) = parent
            .parent_issue_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            return Err(UpstreamResolveError::NoParentIssue);
        };

        let started = std::time::Instant::now();
        // 首拉立即执行，观察分类决定后续形态：Produced 快路径返回；ConfirmedNotProduced
        // （拉取成功但父未产出）开 M5-ch 决策点；Unreachable 静默进预算内重试。
        let mut polling;
        let mut notify;
        let mut decision_open;
        let first = client.get_issue(&workspace_id, &parent).await;
        match observe_parent_fetch(&first) {
            ParentFetchObservation::Produced => {
                let detail = first.as_ref().expect("produced implies Ok");
                let output = parent_output(detail).expect("produced implies non-empty output");
                self.settle_pull(&shared, &run, node_id, &parent, "resolved");
                info!(
                    task = task_id,
                    node = node_id,
                    issue = %parent,
                    "multica upstream pull resolved on first fetch"
                );
                return render_upstream_section(
                    context.config.desktop_language,
                    UpstreamSectionSource::Parent {
                        status: &detail.status,
                        output,
                    },
                );
            }
            ParentFetchObservation::ConfirmedNotProduced => {
                notify = self.open_decision_point(&shared, &run, node_id, &parent);
                decision_open = true;
                polling = false;
            }
            ParentFetchObservation::Unreachable => {
                let e = first.as_ref().expect_err("unreachable implies Err");
                warn!(
                    task = task_id,
                    node = node_id,
                    issue = %parent,
                    %e,
                    "multica upstream first fetch failed (will retry within budget)"
                );
                // 不开决策点：本地 Notify 永不唤醒，select 只走轮询 sleep。
                notify = Arc::new(Notify::new());
                decision_open = false;
                polling = true;
            }
        }

        let mut interval = POLL_INITIAL_INTERVAL;
        loop {
            let wait = if polling {
                interval
            } else {
                DECISION_FALLBACK_AFTER
            };
            if started.elapsed() + wait >= POLL_BUDGET {
                self.settle_pull(&shared, &run, node_id, &parent, "timeout");
                warn!(
                    task = task_id,
                    node = node_id,
                    issue = %parent,
                    budget_secs = POLL_BUDGET.as_secs(),
                    "multica upstream pull timed out"
                );
                return Err(UpstreamResolveError::Timeout);
            }
            tokio::select! {
                _ = notify.notified() => {
                    let decision = shared
                        .lock()
                        .expect("multica state lock")
                        .take_upstream_pull_decision(&run.local_run_id, node_id);
                    match decision {
                        Some(UpstreamPullDecision::Manual(text)) => {
                            self.settle_pull(&shared, &run, node_id, &parent, "manual");
                            info!(
                                task = task_id,
                                node = node_id,
                                issue = %parent,
                                "multica upstream pull decided: manual input"
                            );
                            return render_upstream_section(
                                context.config.desktop_language,
                                UpstreamSectionSource::Manual(&text),
                            );
                        }
                        Some(UpstreamPullDecision::Skip) => {
                            self.settle_pull(&shared, &run, node_id, &parent, "skipped");
                            info!(
                                task = task_id,
                                node = node_id,
                                issue = %parent,
                                "multica upstream pull decided: skip"
                            );
                            return render_upstream_section(
                                context.config.desktop_language,
                                UpstreamSectionSource::Skipped,
                            );
                        }
                        // 继续等待：恢复轮询（本次迭代末立即拉取一次）。
                        Some(UpstreamPullDecision::Retry) | None => {
                            polling = true;
                        }
                    }
                }
                _ = tokio::time::sleep(wait) => {
                    if !polling {
                        // 60s 无人应答：自动回落轮询（决策卡片保留，期间决策仍生效）。
                        polling = true;
                        info!(
                            task = task_id,
                            node = node_id,
                            issue = %parent,
                            "multica upstream pull decision window elapsed, falling back to polling"
                        );
                    }
                }
            }
            debug_assert!(polling, "decision branches either return or resume polling");
            let fetch = client.get_issue(&workspace_id, &parent).await;
            match observe_parent_fetch(&fetch) {
                ParentFetchObservation::Produced => {
                    let detail = fetch.as_ref().expect("produced implies Ok");
                    let output = parent_output(detail).expect("produced implies non-empty output");
                    info!(
                        task = task_id,
                        node = node_id,
                        issue = %parent,
                        waited_secs = started.elapsed().as_secs(),
                        "multica upstream pull resolved"
                    );
                    self.settle_pull(&shared, &run, node_id, &parent, "resolved");
                    return render_upstream_section(
                        context.config.desktop_language,
                        UpstreamSectionSource::Parent {
                            status: &detail.status,
                            output,
                        },
                    );
                }
                // 首个「确认父未产出」补开决策点：首拉网络失败时 M5-ch 口径在此兑现——后续
                // 轮询确认父确实未产出，与首拉空值同权，不能干等预算耗尽。
                ParentFetchObservation::ConfirmedNotProduced if !decision_open => {
                    notify = self.open_decision_point(&shared, &run, node_id, &parent);
                    decision_open = true;
                    polling = false;
                }
                ParentFetchObservation::ConfirmedNotProduced => {}
                // 轮询期网络抖动不立即失败（等待窗口内重试是常态；预算耗尽由 Timeout 收口）。
                ParentFetchObservation::Unreachable => {
                    let e = fetch.as_ref().expect_err("unreachable implies Err");
                    warn!(
                        task = task_id,
                        node = node_id,
                        issue = %parent,
                        %e,
                        "multica upstream poll fetch failed (will retry within budget)"
                    );
                }
            }
            interval = std::cmp::min(interval.mul_f32(POLL_BACKOFF_FACTOR), POLL_MAX_INTERVAL);
        }
    }

    /// 开 M5-ch 决策点：注册决策槽 + 发 pending 事件（会话内决策卡片），返回 notify 供
    /// select 等待用户三选一。「只开一次」由调用方 `decision_open` 保证（60s 回落后卡片
    /// 保留、槽仍可迟到决策，无需重复注册）。
    fn open_decision_point(
        &self,
        shared: &SharedMulticaState,
        run: &ActiveRemoteRun,
        node_id: &str,
        parent: &str,
    ) -> Arc<Notify> {
        let notify = shared
            .lock()
            .expect("multica state lock")
            .register_upstream_pull(&run.local_run_id, node_id);
        let _ = self.handle.emit(
            MULTICA_UPSTREAM_PULL_PENDING_EVENT,
            UpstreamPullEventVm::pending(run, node_id, parent),
        );
        notify
    }

    /// 决策点收口（任意退出路径统一调用）：移除决策槽 + 发 settled 事件关闭前端决策卡片。
    fn settle_pull(
        &self,
        shared: &SharedMulticaState,
        run: &ActiveRemoteRun,
        node_id: &str,
        parent: &str,
        outcome: &'static str,
    ) {
        shared
            .lock()
            .expect("multica state lock")
            .drop_upstream_pull(&run.local_run_id, node_id);
        let _ = self.handle.emit(
            MULTICA_UPSTREAM_PULL_SETTLED_EVENT,
            UpstreamPullEventVm::settled(run, node_id, parent, outcome),
        );
    }
}

impl UpstreamOutputResolver for MulticaUpstreamResolver {
    fn resolve_upstream_output(
        &self,
        task_id: &str,
        node_id: &str,
    ) -> Result<String, UpstreamResolveError> {
        tauri::async_runtime::block_on(self.resolve(task_id, node_id))
    }
}

/// 首拉/轮询命中的父交付输出（trim 非空才算产出；值存在即返回，不比较版本——「拿到当前
/// 快照」即目的）。
fn parent_output(detail: &IssueDetail) -> Option<&str> {
    detail
        .completion_output
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 父工作项单次拉取的观察分类（M5-ch 决策点开启口径的纯函数化）。
///
/// 决策点开启的不变量：只在**首次「确认父未产出」**（拉取成功且 `completion_output`
/// trim 为空）上开启——无论该确认来自首拉还是后续轮询。拉取失败不构成确认（「拉不到」
/// ≠「父未产出」），不得开决策点打扰用户。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentFetchObservation {
    /// 拉取成功且父已产出——快路径直接返回，不进入等待循环。
    Produced,
    /// 拉取成功但父未产出——「确认父未产出」，决策点的唯一开启口径。
    ConfirmedNotProduced,
    /// 拉取失败——不构成确认，不开决策点，预算内静默重试。
    Unreachable,
}

impl ParentFetchObservation {
    /// 决策点只在「确认父未产出」上开启。
    fn opens_decision_point(self) -> bool {
        matches!(self, ParentFetchObservation::ConfirmedNotProduced)
    }
}

/// 把一次父工作项拉取结果归类（错误细节由调用侧日志消化，不进入分类值）。
fn observe_parent_fetch<E>(fetch: &Result<IssueDetail, E>) -> ParentFetchObservation {
    match fetch {
        Ok(detail) if parent_output(detail).is_some() => ParentFetchObservation::Produced,
        Ok(_) => ParentFetchObservation::ConfirmedNotProduced,
        Err(_) => ParentFetchObservation::Unreachable,
    }
}

/// 前端拉取决策事件载荷（pending 与 settled 同形：pending 的 `outcome` 为 null）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UpstreamPullEventVm {
    task_id: String,
    run_id: String,
    node_id: String,
    parent_issue_id: String,
    task_title: Option<String>,
    outcome: Option<&'static str>,
}

impl UpstreamPullEventVm {
    fn pending(run: &ActiveRemoteRun, node_id: &str, parent: &str) -> Self {
        Self {
            task_id: run.local_task_id.clone(),
            run_id: run.local_run_id.clone(),
            node_id: node_id.to_string(),
            parent_issue_id: parent.to_string(),
            task_title: run.title.clone(),
            outcome: None,
        }
    }

    fn settled(run: &ActiveRemoteRun, node_id: &str, parent: &str, outcome: &'static str) -> Self {
        Self {
            outcome: Some(outcome),
            ..Self::pending(run, node_id, parent)
        }
    }
}

/// 按节点协议区段标题（对齐 `Gold Band remote task context` 命名惯例；与冻结区段分列，
/// 会话内点开即可审计该节点被判定成的产出身份）。
pub(crate) const REMOTE_TASK_PROTOCOL_SECTION_TITLE: &str = "Gold Band remote task protocol";

/// 完成输出协议区段解析器（M5-cg 产出侧门禁，main.rs setup 装入 DesktopState）。
///
/// 引擎装配节点 prompt（`SessionMode::New`）时同步调用；渲染端只做「任务是否关联 multica
/// issue」的归属判定 + 变体渲染——指定判定（sink / 标记 / 验收）在引擎侧完成（数据与模型
/// 所见同源）。非 multica 任务 / 无在飞映射（本地 run 的常态）→ `None` 静默跳过。
pub(crate) struct MulticaCompletionProtocolResolver {
    handle: AppHandle,
}

impl MulticaCompletionProtocolResolver {
    pub(crate) fn new(handle: AppHandle) -> Self {
        Self { handle }
    }
}

impl CompletionProtocolSectionResolver for MulticaCompletionProtocolResolver {
    fn completion_protocol_section(
        &self,
        task_id: &str,
        designation: CompletionProtocolDesignation,
    ) -> Option<PromptHiddenSection> {
        let shared = shared_multica_state(&self.handle)?;
        let run = shared
            .lock()
            .ok()?
            .find_active_run_by_task(task_id)
            .map(|(_, run)| run)?;
        run.issue_id
            .as_deref()
            .map(str::trim)
            .filter(|issue| !issue.is_empty())?;
        let language = desktop_context(&self.handle)?.config.desktop_language;
        let template = prompt_by_language(language, RUNTIME_REMOTE_TASK_NODE_PROTOCOL);
        let rendered = render(template, json!({ "designation": designation.as_str() }))
            .expect("remote task node protocol renders");
        Some(PromptHiddenSection {
            title: REMOTE_TASK_PROTOCOL_SECTION_TITLE.to_string(),
            content: rendered.trim().to_string(),
        })
    }
}

/// 上游交付说明的来源（M5-ch 三变体：父快照 / 用户手动提供 / 跳过）。
enum UpstreamSectionSource<'a> {
    Parent { status: &'a str, output: &'a str },
    Manual(&'a str),
    Skipped,
}

/// 渲染上游交付说明区段正文（复用 remote_task_* 模板管线；语言取当前界面语言，角色缺语言
/// 降级英文）。
fn render_upstream_section(
    language: DesktopLanguage,
    source: UpstreamSectionSource<'_>,
) -> Result<String, UpstreamResolveError> {
    let template = prompt_by_language(language, RUNTIME_REMOTE_TASK_UPSTREAM_PULL);
    let context = match source {
        UpstreamSectionSource::Parent { status, output } => {
            json!({ "source": "parent", "parent_status": status, "parent_output": output })
        }
        UpstreamSectionSource::Manual(text) => json!({ "source": "manual", "content": text }),
        UpstreamSectionSource::Skipped => json!({ "source": "skipped" }),
    };
    render(template, context)
        .map(|rendered| rendered.trim().to_string())
        .map_err(|e| UpstreamResolveError::Network(format!("upstream pull template render: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_constants_match_approved_finite_backoff_decision() {
        // 用户决策 2026-10-24（有限退避轮询）：15s 起、×1.5、上限 60s、总预算 15 分钟。
        // 常量被无意改动时该测试立即转红，防止等待策略静默漂移。
        assert_eq!(POLL_INITIAL_INTERVAL, Duration::from_secs(15));
        assert_eq!(POLL_MAX_INTERVAL, Duration::from_secs(60));
        assert_eq!(POLL_BACKOFF_FACTOR, 1.5);
        assert_eq!(POLL_BUDGET, Duration::from_secs(15 * 60));
        assert!(POLL_INITIAL_INTERVAL < POLL_MAX_INTERVAL);
        assert!(POLL_MAX_INTERVAL < POLL_BUDGET);
    }

    #[test]
    fn decision_fallback_window_is_bounded_and_within_budget() {
        // M5-ch：决策等待窗口必须远小于总预算（60s << 15min）——无人值守时回落轮询仍留足
        // 轮询预算；窗口 >= 预算会让决策点变相吞掉全部预算。
        assert_eq!(DECISION_FALLBACK_AFTER, Duration::from_secs(60));
        assert!(DECISION_FALLBACK_AFTER < POLL_BUDGET);
    }

    #[test]
    fn decision_point_opens_only_on_confirmed_not_produced() {
        // M5-ch 决策点开启口径：充要条件是「拉取成功但父未产出」（含 trim 后空白值）。
        // 网络失败（「拉不到」）不构成确认，不得开决策点——否则首拉网络故障会向用户弹
        // 出无意义的三选一，且「父未产出」并未被确认。
        let detail = |output: Option<&str>| IssueDetail {
            status: "in_progress".into(),
            parent_issue_id: None,
            completion_output: output.map(str::to_string),
        };
        let produced: Result<IssueDetail, String> = Ok(detail(Some("v0.1 产物")));
        let blank: Result<IssueDetail, String> = Ok(detail(Some("   ")));
        let empty: Result<IssueDetail, String> = Ok(detail(None));
        let unreachable: Result<IssueDetail, String> = Err("connect timeout".into());

        assert_eq!(
            observe_parent_fetch(&produced),
            ParentFetchObservation::Produced
        );
        assert!(!observe_parent_fetch(&produced).opens_decision_point());
        for confirmed_empty in [&empty, &blank] {
            assert_eq!(
                observe_parent_fetch(confirmed_empty),
                ParentFetchObservation::ConfirmedNotProduced,
                "trim-blank completion_output counts as not produced"
            );
            assert!(observe_parent_fetch(confirmed_empty).opens_decision_point());
        }
        assert_eq!(
            observe_parent_fetch(&unreachable),
            ParentFetchObservation::Unreachable
        );
        assert!(!observe_parent_fetch(&unreachable).opens_decision_point());
    }

    #[test]
    fn render_upstream_section_interpolates_status_and_output() {
        for language in [DesktopLanguage::ZhCn, DesktopLanguage::En] {
            let block = render_upstream_section(
                language,
                UpstreamSectionSource::Parent {
                    status: "in_progress",
                    output: "包名：gold-band_0.19.0_x64.msi\n部署环境：内网测试环境 http://t.example.com",
                },
            )
            .expect("render upstream block");
            assert!(block.contains("in_progress"), "status visible in {block}");
            assert!(
                block.contains("gold-band_0.19.0_x64.msi"),
                "parent output interpolated in {block}"
            );
            assert!(
                block.contains("http://t.example.com"),
                "multiline parent output preserved in {block}"
            );
        }
    }

    #[test]
    fn render_upstream_section_locks_staged_snapshot_caveat_both_languages() {
        // 阶段性快照口径是安全阀：拉到的值可能是父未完成时的中间态，执行方必须知道会被覆盖。
        let zh = render_upstream_section(
            DesktopLanguage::ZhCn,
            UpstreamSectionSource::Parent {
                status: "done",
                output: "最终交付",
            },
        )
        .expect("zh render");
        assert!(zh.contains("阶段性快照"));
        assert!(zh.contains("done"));

        let en = render_upstream_section(
            DesktopLanguage::En,
            UpstreamSectionSource::Parent {
                status: "done",
                output: "final deliverable",
            },
        )
        .expect("en render");
        assert!(en.contains("staged snapshot"));
        assert!(en.contains("done"));
    }

    #[test]
    fn render_upstream_section_manual_marks_user_provided_source() {
        // manual：内容插值 + 标注用户手动提供（非父快照），不带阶段性快照口径。
        let zh = render_upstream_section(
            DesktopLanguage::ZhCn,
            UpstreamSectionSource::Manual("包名：gold-band_0.19.0_x64.msi"),
        )
        .expect("zh manual render");
        assert!(zh.contains("手动提供"));
        assert!(zh.contains("gold-band_0.19.0_x64.msi"));
        assert!(
            !zh.contains("阶段性快照"),
            "manual must not carry snapshot caveat"
        );

        let en = render_upstream_section(
            DesktopLanguage::En,
            UpstreamSectionSource::Manual("package: gold-band_0.19.0_x64.msi"),
        )
        .expect("en manual render");
        assert!(en.contains("provided manually by the user"));
        assert!(en.contains("gold-band_0.19.0_x64.msi"));
        assert!(
            !en.contains("staged snapshot"),
            "manual must not carry snapshot caveat"
        );
    }

    #[test]
    fn render_upstream_section_skipped_forbids_guessing() {
        // skipped：跳过说明 + 「不要臆测」防线（缺信息时如实说明而非编造）。
        let zh = render_upstream_section(DesktopLanguage::ZhCn, UpstreamSectionSource::Skipped)
            .expect("zh skip render");
        assert!(zh.contains("跳过"));
        assert!(zh.contains("不要臆测"));

        let en = render_upstream_section(DesktopLanguage::En, UpstreamSectionSource::Skipped)
            .expect("en skip render");
        assert!(en.contains("skipped by the user"));
        assert!(en.contains("instead of guessing"));
    }
}
