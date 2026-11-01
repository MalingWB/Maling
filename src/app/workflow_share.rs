//! multica 工作流共享交换（daemon 通道 B 工单；格式转换单一真源，见
//! `.claude/design/multica-workflow-management/码灵对接说明(1).md` §5）。
//!
//! 与角色共享（`profiles::MulticaRoleExchange`）同通道不同资产：multica 只中转**编排模板**，
//! 执行留在码灵。wire envelope 形状（§5.1）：`{id, name, isBuiltIn, workflow}`——字段名与本地
//! `WorkflowTemplate` camelCase 序列化天然对齐，`workflow` 即 `WorkflowDsl` 原生 wire 形态
//! （节点 `type` tag、边 `$end`/`$new-round` 伪节点、`on` kebab 值两端一致）。
//!
//! **modelBindings 永不共享**（ADR-0008 D3：per-device 计算配置属设备私有）：envelope 结构上
//! 没有该字段——剥离由构造保证，序列化不可能带出（multica 发布校验 `MODEL_BINDINGS_PRESENT`
//! 双保险的另一侧）。下载侧同理：payload 恒不带绑定，落库为空绑定，首次打开由客户端提示重绑。

use serde::{Deserialize, Serialize};

use super::{WorkflowDsl, WorkflowTemplate};

/// multica 工作流共享的交换文档载荷（export 工单回传 body 的 `payload`；push-install 详情
/// payload 同形）。wire 字段名即字段名：`isBuiltIn` camelCase（§4.2 示例），与角色交换的
/// snake `is_builtin` 不同——两类 envelope 各自跟随 multica 侧定稿形状，不强求统一。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MulticaWorkflowExchange {
    /// 模板 id（asset key：list 摘要 key = export asset_key = 安装去重锚点，单一 key 空间）。
    pub id: String,
    /// 显示名（安装落库名；builtin 副本规则触发改名时以实际落库名回报）。
    pub name: String,
    /// 内置标记（multica 目录徽标，非门槛；安装侧 builtin id 构成「物化用户副本」语义的一部分）。
    #[serde(default)]
    pub is_built_in: bool,
    /// 编排正文（节点/边/控制；不含任何设备私有配置——见模块注释 modelBindings 规则）。
    pub workflow: WorkflowDsl,
}

/// 本地模板 → multica 交换文档（export 工单回传 body 的 payload）。
///
/// `modelBindings`/`optionalEntryStage`/时间戳不进 envelope：前两者是本地私有（§5.2），
/// 时间戳无 wire 字段（multaca §4.2 envelope 定稿只有四键，未知键由服务端宽松保留是指
/// **接收**第三方 payload 时的容忍，不是码灵要主动多发的理由）。
pub fn workflow_template_to_multica_exchange(
    template: &WorkflowTemplate,
) -> MulticaWorkflowExchange {
    MulticaWorkflowExchange {
        id: template.id.clone(),
        name: template.name.clone(),
        is_built_in: template.is_built_in,
        workflow: template.workflow.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{
        EdgeDsl, EdgeOutcome, END_NODE, NodeDsl, PromptEnvelopeMode, WorkerNode,
    };
    use serde_json::json;

    fn sample_workflow() -> WorkflowDsl {
        WorkflowDsl {
            version: "0.1".to_string(),
            id: "workflow-abc".to_string(),
            entry: "dev".to_string(),
            control: Default::default(),
            nodes: vec![NodeDsl::Worker(WorkerNode {
                id: "dev".to_string(),
                execution_slot_id: None,
                provider: Some("claude-acp".to_string()),
                model: None,
                profile: Some("pf-builtin-dev".to_string()),
                goal: Some("实现需求".to_string()),
                output: None,
                success_condition: None,
                permission_mode: None,
                auto_accept: false,
                config_options: Default::default(),
                manual_check: None,
                prompt_envelope: PromptEnvelopeMode::default(),
            })],
            edges: vec![EdgeDsl {
                from: "dev".to_string(),
                to: END_NODE.to_string(),
                on: EdgeOutcome::Success,
                session: None,
                new_round_entry: None,
            }],
        }
    }

    fn sample_template() -> WorkflowTemplate {
        WorkflowTemplate {
            id: "my-pipeline".to_string(),
            name: "我的流水线".to_string(),
            is_built_in: false,
            optional_entry_stage: None,
            workflow: sample_workflow(),
            model_bindings: Default::default(),
            created_at: "1790000000Z".to_string(),
            updated_at: "1790000000Z".to_string(),
        }
    }

    #[test]
    fn exchange_serializes_canonical_envelope_without_model_bindings() {
        // wire 形态：四键 envelope、isBuiltIn camelCase、workflow 原生 DSL 形态；
        // 永不携带 modelBindings / optionalEntryStage / 时间戳（§4.5/§5.2 剥离规则）。
        let exchange = workflow_template_to_multica_exchange(&sample_template());
        let json = serde_json::to_value(&exchange).unwrap();
        assert_eq!(json["id"], "my-pipeline");
        assert_eq!(json["name"], "我的流水线");
        assert_eq!(json["isBuiltIn"], false);
        assert_eq!(json["workflow"]["entry"], "dev");
        assert_eq!(json["workflow"]["nodes"][0]["type"], "worker");
        assert_eq!(json["workflow"]["nodes"][0]["profile"], "pf-builtin-dev");
        assert_eq!(json["workflow"]["edges"][0]["to"], "$end");
        assert_eq!(json["workflow"]["edges"][0]["on"], "success");
        assert!(json.get("modelBindings").is_none());
        assert!(json.get("optionalEntryStage").is_none());
        assert!(json.get("createdAt").is_none());
        assert!(json.get("updatedAt").is_none());
    }

    #[test]
    fn exchange_round_trips_through_wire_bytes() {
        // 外来 envelope（multica 推送安装 payload）→ 本地 exchange 反序列化（未知键如
        // optionalEntryStage 宽容忽略）；再序列化与原 payload 键集一致（round-trip 稳定）。
        let wire = json!({
            "id": "shared-pipeline",
            "name": "共享流水线",
            "isBuiltIn": true,
            "optionalEntryStage": { "nodeId": "dev", "labelKey": "k", "defaultEnabled": true },
            "workflow": {
                "version": "0.1",
                "id": "task-workflow",
                "entry": "dev",
                "control": { "max_attempts": 10, "max_rounds": 3 },
                "nodes": [
                    { "type": "worker", "id": "dev", "executionSlotId": "slot-1",
                      "provider": null, "profile": "pf-builtin-dev", "goal": "实现",
                      "output": null, "success_condition": null, "manual_check": null,
                      "prompt_envelope": "runtime-managed" }
                ],
                "edges": [ { "from": "dev", "to": "$end", "on": "success" } ]
            }
        });
        let exchange: MulticaWorkflowExchange = serde_json::from_value(wire).unwrap();
        assert_eq!(exchange.id, "shared-pipeline");
        assert!(exchange.is_built_in);
        assert_eq!(exchange.workflow.nodes.len(), 1);
        assert_eq!(exchange.workflow.control.max_attempts, Some(10));
        let back = serde_json::to_value(&exchange).unwrap();
        assert_eq!(back["id"], "shared-pipeline");
        assert_eq!(back["isBuiltIn"], true);
        assert_eq!(back["workflow"]["control"]["max_attempts"], 10);
        // wire 键名锁定（本地 WorkflowDsl 序列化漂移时此处先红）：envelope camelCase 四键、
        // DSL 正文混合命名（executionSlotId 显式 camel，其余 Rust 原名）与 multica §5.1 一致。
        assert!(back["workflow"]["nodes"][0].get("success_condition").is_some());
        assert!(back["workflow"]["nodes"][0].get("prompt_envelope").is_some());
        assert!(back["workflow"]["nodes"][0].get("executionSlotId").is_some());
    }

    #[test]
    fn exchange_rejects_payload_missing_workflow_body() {
        // 结构非法（缺 workflow 正文）：反序列化失败 → 工单按 failed 上报，不落半成品。
        let result: Result<MulticaWorkflowExchange, _> =
            serde_json::from_value(json!({ "id": "x", "name": "n", "isBuiltIn": false }));
        assert!(result.is_err());
    }
}
