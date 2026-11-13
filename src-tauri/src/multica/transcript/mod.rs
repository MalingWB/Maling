//! Multica 执行记录（transcript）实时上报。
//!
//! 目标：执行 multica 远程任务时，把模型文本、工具调用/结果、运行期错误按批上报到
//! `POST /api/daemon/tasks/{taskId}/messages`，并在节点/turn 边界与终态上报 token 用量，
//! 让 multica 既有 Web UI 获得实时执行时间线（**零服务端改动**）。
//!
//! 组成：
//! - [`model`]: 线上 DTO、消息类型与全部硬上限
//! - [`projector`]: `AcpUiEvent` → 消息 的纯投影（文本快照差量、工具 use/result 配对）
//! - [`tool_input`]: 工具输入归一化（multica 三种约定形状 + Codex `apply_patch` 解析）
//! - [`reporter`]: 注册表、500ms 批 flush、seq 分配、终态排空、用量累计
//!
//! 设计约束（见《Multica接入方案》执行记录章节）：
//! - 只在 multica 远程任务执行期间上报；本地会话零开销（一次注册表查找即返回）。
//! - 观测点在 live emitter 的 IPC 压缩之前，因此工具输入/输出取得到完整 `raw`。
//! - 失败即丢、不重试；seq 作用域 = 服务端 task_id，新 task 从 1 开始，无需持久水位。
//! - 终态（完成/失败/取消）先上报 error 行，再排空，最后才写 multica 终态。

pub(crate) mod model;
mod projector;
mod reporter;
mod tool_input;

pub(crate) use model::{DRAIN_TIMEOUT, TranscriptMessage, UsageRow};
pub use reporter::TranscriptRegistry;
pub(crate) use reporter::{accumulate_usage, finish_run, observe, report_error, start_run};
