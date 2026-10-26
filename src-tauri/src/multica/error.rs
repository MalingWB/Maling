//! multica 错误类型与 `CommandErrorVm` 映射（开发设计 2.2.8 / 第 5 章码表）。
//!
//! 后端只返回 `{ code, params }`，**不含任何对客文案**；前端按 `code` 查 i18n 文案。
//! HTTP 错误按状态码映射（client.rs `map_status`）：401/403→auth-failed、
//! 404→task-not-found、409→claim-conflict、其余≥400→network-failed。

// 完整错误码表一次定义：注册/领取冲突/会话恢复等 variant 分 M2-M4 启用。
use serde_json::{Value, json};

/// multica 模块统一错误。
///
/// 命令层经 `command_error`（commands.rs）downcast 后映射为
/// `CommandErrorVm { code: "multica.<reason>", params }`。
#[derive(Debug, thiserror::Error)]
pub enum MulticaError {
    #[error("multica not configured")]
    NotConfigured,
    #[error("multica auth failed: {0}")]
    AuthFailed(String),
    #[error("multica network failed: {0}")]
    NetworkFailed(String),
    #[error("multica register failed: {0}")]
    RegisterFailed(String),
    #[error("multica claim conflict")]
    ClaimConflict,
    #[error("multica task not found")]
    TaskNotFound,
    #[error("multica runtime offline")]
    RuntimeOffline,
    /// 连接地址非法或两参数（base/app）不配套（一 Some 一 None）。
    #[error("multica invalid connection address")]
    InvalidAddress,
    /// 用户在浏览器登录等待期间主动取消（连接弹窗「取消连接」）。非失败：前端静默关闭弹窗。
    #[error("multica connect cancelled by user")]
    ConnectCancelled,
    /// issue 附件中转目录创建失败（M5-bu）：本地临时目录不可写（磁盘满/权限），附件既无法下载
    /// 也无法告知 agent，整体无法继续 → 硬失败并 release 回滚。逐个附件的下载失败是软失败
    /// （局部失败隔离），不进本错误码——以隐藏区段告知 agent，不阻断任务启动。
    #[error("multica attachment staging failed: {0}")]
    AttachmentStageFailed(String),
    // M4-d：保留在错误码表（multica.session-resume-failed），但断点续跑路径不 emit——任何 resume Err
    // 改走 silent fresh-fallback（更稳，无需 fragile 串匹配）。见开发设计 §2.2.8。
    #[allow(dead_code)]
    #[error("multica session resume failed, will rerun")]
    SessionResumeFailed,
    /// 上游拉取决策无对应等待槽（M5-ch）：决策迟到（resolve 已收口或 run 已终态）。
    /// 前端通常已随 settled 事件关闭弹窗，此错误用于兜底（如弹窗与收口竞态的迟到提交）。
    #[error("multica upstream pull decision has no pending slot")]
    UpstreamPullNotPending,
}

impl MulticaError {
    /// 错误码（kebab-case）。命名空间 = 层级归属（远程来源解耦设计 §4.3）：通用层结构错误用
    /// `remote.*`（NotConfigured = 来源未配置）；其余协议/连接域错误保留 `multica.` 前缀。
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotConfigured => "remote.source-not-configured",
            Self::AuthFailed(_) => "multica.auth-failed",
            Self::NetworkFailed(_) => "multica.network-failed",
            Self::RegisterFailed(_) => "multica.register-failed",
            Self::ClaimConflict => "multica.claim-conflict",
            Self::TaskNotFound => "multica.task-not-found",
            Self::RuntimeOffline => "multica.runtime-offline",
            Self::InvalidAddress => "multica.invalid-address",
            Self::ConnectCancelled => "multica.connect-cancelled",
            Self::AttachmentStageFailed(_) => "multica.attachment-stage-failed",
            Self::SessionResumeFailed => "multica.session-resume-failed",
            Self::UpstreamPullNotPending => "multica.upstream-pull-not-pending",
        }
    }

    /// 错误参数（上下文，无对客文案）。
    pub fn params(&self) -> Value {
        match self {
            Self::AuthFailed(msg) | Self::NetworkFailed(msg) | Self::RegisterFailed(msg) => {
                json!({ "message": msg })
            }
            Self::AttachmentStageFailed(msg) => json!({ "message": msg }),
            _ => json!({}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MulticaError;

    #[test]
    fn codes_use_multica_kebab_prefix() {
        assert_eq!(
            MulticaError::NotConfigured.code(),
            "remote.source-not-configured"
        );
        assert_eq!(
            MulticaError::AuthFailed("401".into()).code(),
            "multica.auth-failed"
        );
        assert_eq!(
            MulticaError::NetworkFailed("timeout".into()).code(),
            "multica.network-failed"
        );
        assert_eq!(
            MulticaError::RegisterFailed("x".into()).code(),
            "multica.register-failed"
        );
        assert_eq!(MulticaError::ClaimConflict.code(), "multica.claim-conflict");
        assert_eq!(MulticaError::TaskNotFound.code(), "multica.task-not-found");
        assert_eq!(
            MulticaError::RuntimeOffline.code(),
            "multica.runtime-offline"
        );
        assert_eq!(
            MulticaError::InvalidAddress.code(),
            "multica.invalid-address"
        );
        assert_eq!(
            MulticaError::ConnectCancelled.code(),
            "multica.connect-cancelled"
        );
        assert_eq!(
            MulticaError::AttachmentStageFailed("temp dir unwritable".into()).code(),
            "multica.attachment-stage-failed"
        );
        assert_eq!(
            MulticaError::SessionResumeFailed.code(),
            "multica.session-resume-failed"
        );
        assert_eq!(
            MulticaError::UpstreamPullNotPending.code(),
            "multica.upstream-pull-not-pending"
        );
    }

    #[test]
    fn params_carry_message_context_without_user_copy() {
        assert_eq!(
            MulticaError::AuthFailed("jwt expired".into()).params(),
            serde_json::json!({ "message": "jwt expired" })
        );
        // 无上下文的变体返回空对象，绝不内嵌对客文案。
        assert_eq!(MulticaError::ClaimConflict.params(), serde_json::json!({}));
        assert_eq!(MulticaError::TaskNotFound.params(), serde_json::json!({}));
    }
}
