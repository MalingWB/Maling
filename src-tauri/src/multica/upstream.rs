//! `$upstream.completion-output` 令牌的桌面侧解析器（M5-cf 拉取链路）。
//!
//! 链路：本地任务 → 在飞 multica 映射（workspace + 自身 issue）→ `GET /api/issues/{self}`
//! 解析 `parent_issue_id` → 有限退避轮询 `GET /api/issues/{parent}` 直至 `completion_output`
//! 非空（阶段性快照——父未 done 也可得，内网确认 2026-10-24：字段非空即下发、PUT 无状态
//! 门槛）→ 按当前界面语言渲染「上游交付说明 + 父状态行」模板。渲染结果由引擎注入首条
//! prompt 的隐式隐藏区段（「上游交付说明」），正文中的令牌随之移除（AGENTS.md system/user
//! prompt 划分：随执行变化的交付数据属 user prompt 区，不内联进 system prompt 角色正文）。
//!
//! 引擎在 run 专用线程同步调用（`App::resolve_upstream_output`）；本侧经
//! `tauri::async_runtime::block_on` 桥到异步客户端——阻塞上限 = 轮询总预算（15 分钟）。
//! 快照语义：节点启动时的值，父后续覆盖不自动刷新（再次进入该节点重新解析）。

use std::time::Duration;

use gold_band::app::upstream::{UpstreamOutputResolver, UpstreamResolveError};
use gold_band::prompts::{RUNTIME_REMOTE_TASK_UPSTREAM_PULL, prompt_by_language, render};
use serde_json::json;
use tauri::AppHandle;
use tracing::{info, warn};

use crate::multica::bridge::{desktop_context, multica_client, shared_multica_state};

/// 有限退避轮询参数（用户决策 2026-10-24：有限退避轮询）：15s 起、×1.5 退避、间隔上限
/// 60s、总预算 15 分钟。集中定义为模块常量（AGENTS：业务常量集中定义）。
const POLL_INITIAL_INTERVAL: Duration = Duration::from_secs(15);
const POLL_MAX_INTERVAL: Duration = Duration::from_secs(60);
const POLL_BACKOFF_FACTOR: f32 = 1.5;
const POLL_BUDGET: Duration = Duration::from_secs(15 * 60);

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
        let Some(run) = shared
            .lock()
            .expect("multica state lock")
            .find_active_run_by_task(task_id)
            .map(|(_, run)| run)
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

        // 有限退避轮询：值存在即返回（不比较版本——「拿到当前快照」即目的）。
        let mut interval = POLL_INITIAL_INTERVAL;
        let started = std::time::Instant::now();
        loop {
            match client.get_issue(&workspace_id, &parent).await {
                Ok(detail) => {
                    if let Some(output) = detail
                        .completion_output
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        info!(
                            task = task_id,
                            node = node_id,
                            issue = %parent,
                            waited_secs = started.elapsed().as_secs(),
                            "multica upstream pull resolved"
                        );
                        return render_upstream_block(
                            context.config.desktop_language,
                            &detail.status,
                            output,
                        );
                    }
                }
                // 轮询期网络抖动不立即失败（等待窗口内重试是常态；预算耗尽由 Timeout 收口）。
                Err(e) => warn!(
                    task = task_id,
                    node = node_id,
                    issue = %parent,
                    %e,
                    "multica upstream poll fetch failed (will retry within budget)"
                ),
            }
            if started.elapsed() + interval >= POLL_BUDGET {
                warn!(
                    task = task_id,
                    node = node_id,
                    issue = %parent,
                    budget_secs = POLL_BUDGET.as_secs(),
                    "multica upstream pull timed out"
                );
                return Err(UpstreamResolveError::Timeout);
            }
            tokio::time::sleep(interval).await;
            interval = std::cmp::min(
                interval.mul_f32(POLL_BACKOFF_FACTOR),
                POLL_MAX_INTERVAL,
            );
        }
    }
}

impl UpstreamOutputResolver for MulticaUpstreamResolver {
    fn resolve_upstream_output(&self, task_id: &str, node_id: &str) -> Result<String, UpstreamResolveError> {
        tauri::async_runtime::block_on(self.resolve(task_id, node_id))
    }
}

/// 渲染替换文本（复用 remote_task_* 模板管线；语言取当前界面语言，角色缺语言降级英文）。
fn render_upstream_block(
    language: gold_band::config::DesktopLanguage,
    parent_status: &str,
    parent_output: &str,
) -> Result<String, UpstreamResolveError> {
    let template = prompt_by_language(language, RUNTIME_REMOTE_TASK_UPSTREAM_PULL);
    render(
        template,
        json!({ "parent_status": parent_status, "parent_output": parent_output }),
    )
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
    fn render_upstream_block_interpolates_status_and_output() {
        for language in [
            gold_band::config::DesktopLanguage::ZhCn,
            gold_band::config::DesktopLanguage::En,
        ] {
            let block = render_upstream_block(
                language,
                "in_progress",
                "包名：gold-band_0.19.0_x64.msi\n部署环境：内网测试环境 http://t.example.com",
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
    fn render_upstream_block_locks_staged_snapshot_caveat_both_languages() {
        // 阶段性快照口径是安全阀：拉到的值可能是父未完成时的中间态，执行方必须知道会被覆盖。
        let zh = render_upstream_block(
            gold_band::config::DesktopLanguage::ZhCn,
            "done",
            "最终交付",
        )
        .expect("zh render");
        assert!(zh.contains("阶段性快照"));
        assert!(zh.contains("done"));

        let en = render_upstream_block(
            gold_band::config::DesktopLanguage::En,
            "done",
            "final deliverable",
        )
        .expect("en render");
        assert!(en.contains("staged snapshot"));
        assert!(en.contains("done"));
    }
}
