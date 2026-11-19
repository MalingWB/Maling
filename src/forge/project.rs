//! WorkflowDsl → ForgeSpec 反投影（A3，对话式修改路径）。
//!
//! 现有工作流反投影为 Spec 进入 Forge 会话，修改后重新编译。
//! 无损回写约束（§3.3）：Spec 不表达的字段（executionSlotId、
//! permission_mode、config_options 等）不参与循环，重编译时保留原值；
//! 仅对新增节点查默认绑定。
//!
//! 反投影是编译的逆映射（尽力而为的投影，不是全量逆函数）：
//! - worker 节点（按线性 success 边排序）→ stages；
//! - profile 反查 role（内置五类），非内置 profile 记为 Custom；
//! - failure 边到 `$new-round`/`$end` 还原 onFailure；
//! - manual_check 优先于 output 契约还原；
//! - 无 failure 边 → Retry。
//!
//! AiDynamic 节点不属于 Forge 的平铺模型：反投影时报 `forge/invalid-spec`
//! 上下文错误，由上层引导用户先在编辑器中处理，不做静默丢弃。

use crate::dsl::{EdgeOutcome, JsonConditionDsl, NodeDsl, WorkflowDsl, END_NODE, NEW_ROUND_NODE};
use crate::forge::compiler::CompiledWorkflow;
use crate::forge::spec::{
    ForgeError, ForgeSpec, ForgeStage, ForgeWorkflow, OnFailure, StageOutputKind, StageRole,
    SPEC_VERSION,
};

/// 内置 profile id → StageRole 的反查表（与 compiler.rs 的正向映射互逆）。
fn role_from_profile(profile: Option<&str>) -> StageRole {
    match profile {
        Some("pf-builtin-dev") => StageRole::Dev,
        Some("pf-builtin-test") => StageRole::Test,
        Some("pf-builtin-review") => StageRole::Review,
        Some("pf-builtin-plan") => StageRole::Plan,
        Some("pf-builtin-accept") => StageRole::Accept,
        other => StageRole::Custom {
            profile_name: other
                .map(str::to_string)
                .unwrap_or_else(|| "custom".to_string()),
        },
    }
}

/// JsonConditionDsl → Spec 的表达式文本。
fn condition_text(condition: &JsonConditionDsl) -> Option<String> {
    match condition {
        JsonConditionDsl::Expression { expression } => Some(expression.clone()),
        JsonConditionDsl::PathEquals { path, equals } => {
            Some(format!("{} == {}", path, serde_json::to_string(equals).ok()?))
        }
    }
    .filter(|text| !text.trim().is_empty())
}

/// onFailure 还原：failure 出边目标决定策略；无 failure 边 → Retry。
/// manual_check 置位（且无 output 验证）时优先还原为 ManualCheck。
fn on_failure_for(workflow: &WorkflowDsl, node_id: &str, manual_check: bool) -> OnFailure {
    if manual_check {
        return OnFailure::ManualCheck;
    }
    for edge in &workflow.edges {
        if edge.from != node_id || edge.on != EdgeOutcome::Failure {
            continue;
        }
        if edge.to == NEW_ROUND_NODE {
            return OnFailure::NewRound;
        }
        if edge.to == END_NODE && edge.new_round_entry.is_none() {
            return OnFailure::End;
        }
    }
    OnFailure::Retry
}

/// 从 DSL 还原一个 stage（best effort：goal 缺失时给出最小占位文本，
/// 保证反投影产物可进 Forge 会话继续拷打收敛）。
fn stage_from_worker(
    paths: &crate::storage::GoldBandPaths,
    language: crate::config::DesktopLanguage,
    workflow: &WorkflowDsl,
    node_id: &str,
) -> Result<ForgeStage, ForgeError> {
    let NodeDsl::Worker(worker) = workflow
        .nodes
        .iter()
        .find(|node| node.id() == node_id)
        .ok_or_else(|| ForgeError::InvalidSpec {
            detail: format!("node `{node_id}` not found"),
        })?
    else {
        return Err(ForgeError::InvalidSpec {
            detail: format!("node `{node_id}` is not a worker node"),
        });
    };
    let manual_check = worker.manual_check.unwrap_or(false);
    let (output_kind, artifact, schema_hint, success_condition) = if manual_check {
        (StageOutputKind::None, None, None, None)
    } else if let Some(output) = &worker.output {
        (
            StageOutputKind::Json,
            Some(output.artifact.clone()),
            output.schema.clone(),
            worker.success_condition.as_ref().and_then(condition_text),
        )
    } else {
        // 无 output 契约的节点（markdown 语义）。
        (StageOutputKind::Markdown, None, None, None)
    };
    // custom 角色（非内置 profile 引用）：还原 profileId 与职责正文进
    // Spec——对话中可编辑名称/定义，重编译按 id 原地更新（对话反哺）。
    // profile 解析失败时保留节点引用文本作为名称,不阻塞反投影。
    let role = role_from_profile(worker.profile.as_deref());
    let (profile_id, role_definition) = match &role {
        StageRole::Custom { .. } => {
            let resolved = worker
                .profile
                .as_deref()
                .and_then(|profile_id| {
                    crate::app::profiles::find_profile_by_id(paths, profile_id, language).ok()
                    .flatten()
                });
            match resolved {
                Some(entry) => (Some(entry.id), Some(entry.content)),
                None => (worker.profile.clone(), None),
            }
        }
        _ => (None, None),
    };
    Ok(ForgeStage {
        id: Some(node_id.to_string()),
        role,
        goal: worker
            .goal
            .clone()
            .filter(|goal| !goal.trim().is_empty())
            .unwrap_or_else(|| {
                // goal 是必收敛字段；反投影缺失时以四要素骨架占位，
                // 由 Forge 会话继续收敛。
                "<输入>\n<产出>\n<边界>\n<验收>".to_string()
            }),
        role_definition,
        profile_id,
        artifact,
        output_kind,
        schema_hint,
        success_condition,
        on_failure: on_failure_for(workflow, node_id, manual_check),
        manual_check,
    })
}

/// 按线性 success 链排序 worker 节点：entry 起点沿 success 边遍历。
/// 无法按链还原（分叉/环）时报错，不产出顺序不确定的 Spec。
fn ordered_worker_ids(workflow: &WorkflowDsl) -> Result<Vec<String>, ForgeError> {
    if workflow
        .nodes
        .iter()
        .any(|node| matches!(node, NodeDsl::AiDynamic(_)))
    {
        return Err(ForgeError::InvalidSpec {
            detail: "workflow contains ai-dynamic nodes which forge cannot round-trip".to_string(),
        });
    }
    let mut remaining: std::collections::BTreeSet<String> = workflow
        .nodes
        .iter()
        .map(|node| node.id().to_string())
        .filter(|id| id != &workflow.entry)
        .collect();
    if !workflow.nodes.iter().any(|node| node.id() == workflow.entry) {
        return Err(ForgeError::InvalidSpec {
            detail: format!("entry `{}` is not a worker node", workflow.entry),
        });
    }
    let mut order = vec![workflow.entry.clone()];
    let mut current = workflow.entry.clone();
    while !remaining.is_empty() {
        let next = workflow
            .edges
            .iter()
            .find(|edge| {
                edge.from == current
                    && edge.on == EdgeOutcome::Success
                    && edge.new_round_entry.is_none()
                    && remaining.contains(&edge.to)
            })
            .map(|edge| edge.to.clone());
        match next {
            Some(next) => {
                remaining.remove(&next);
                order.push(next.clone());
                current = next;
            }
            None => {
                return Err(ForgeError::InvalidSpec {
                    detail: format!(
                        "workflow `{}` is not a linear forge chain (branching or cycle at `{current}`)",
                        workflow.id
                    ),
                });
            }
        }
    }
    Ok(order)
}

/// 反投影入口：`WorkflowDsl → ForgeSpec`。
///
/// `name`/`intent` 为 Spec 层必填字段，DSL 不承载，由调用方提供
/// （反投影入口 `forge_from_workflow` 从模板取 name，intent 用空串，
/// 进入 Forge 会话后由对话收敛）。
pub fn from_dsl(
    paths: &crate::storage::GoldBandPaths,
    language: crate::config::DesktopLanguage,
    workflow: &WorkflowDsl,
    name: &str,
    intent: &str,
) -> Result<ForgeSpec, ForgeError> {
    let order = ordered_worker_ids(workflow)?;
    let stages = order
        .iter()
        .map(|node_id| stage_from_worker(paths, language, workflow, node_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ForgeSpec {
        spec_version: SPEC_VERSION.to_string(),
        name: Some(name.to_string()),
        intent: intent.to_string(),
        source_skills: vec![],
        workflows: vec![ForgeWorkflow {
            name: name.to_string(),
            stages,
        }],
        open_questions: vec![],
    })
}

/// 无损回写：以原 DSL 为基底应用反投影 Spec 的对话修改。
///
/// 参与循环的字段（goal/output 契约/成功条件/manual_check/onFailure 边/
/// profile）按重编译结果覆盖；不参与的字段（executionSlotId、
/// permission_mode、config_options、auto_accept、prompt_envelope 等
/// 在 authoring 解耦下本就不在 DSL 节点上或不受编译器管理）保留原值。
/// 节点 id 以原 DSL 为准对齐（Spec.stage.id 保存的是原节点 id）。
pub fn write_back(base: &WorkflowDsl, revised: &CompiledWorkflow) -> WorkflowDsl {
    let mut workflow = base.clone();
    // 编译器产出的节点顺序与原链一致（线性 success 边）；按 id 对齐覆盖。
    let compiled_nodes = &revised.workflow.nodes;
    workflow.nodes = workflow
        .nodes
        .iter()
        .map(|node| {
            let NodeDsl::Worker(worker) = node else {
                return node.clone();
            };
            match compiled_nodes
                .iter()
                .find(|candidate| candidate.id() == worker.id)
            {
                Some(NodeDsl::Worker(compiled)) => NodeDsl::Worker(crate::dsl::WorkerNode {
                    id: worker.id.clone(),
                    execution_slot_id: worker.execution_slot_id.clone(),
                    provider: worker.provider.clone(),
                    model: worker.model.clone(),
                    profile: compiled.profile.clone(),
                    goal: compiled.goal.clone(),
                    output: compiled.output.clone(),
                    success_condition: compiled.success_condition.clone(),
                    permission_mode: worker.permission_mode.clone(),
                    auto_accept: worker.auto_accept,
                    config_options: worker.config_options.clone(),
                    manual_check: compiled.manual_check,
                    prompt_envelope: worker.prompt_envelope,
                }),
                // 新增节点（原链中没有）：保留编译器产物，slot 走上层默认绑定。
                _ => node.clone(),
            }
        })
        .collect();
    // 新增节点追加（原 DSL 中不存在的编译产物）。
    for node in compiled_nodes {
        if !workflow.nodes.iter().any(|existing| existing.id() == node.id()) {
            workflow.nodes.push(node.clone());
        }
    }
    // 边整体替换：失败策略/链路结构是 Spec 表达的字段。
    workflow.edges = revised.workflow.edges.clone();
    if workflow
        .nodes
        .first()
        .is_some_and(|first| first.id() != workflow.entry)
    {
        workflow.entry = revised.workflow.entry.clone();
    }
    workflow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::compiler::compile;
    use crate::forge::spec::MAX_STAGES_PER_WORKFLOW;

    /// 反投影测试的隔离路径(custom 角色的 profile 解析走临时目录)。
    fn test_paths() -> crate::storage::GoldBandPaths {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.keep();
        crate::storage::GoldBandPaths::new(
            camino::Utf8PathBuf::from_path_buf(path.join("repo")).unwrap(),
        )
    }

    fn base_goal() -> String {
        "<输入>需求与上一阶段产物\n<产出>通过测试的代码\n<边界>不做数据库迁移\n<验收>全部测试通过"
            .to_string()
    }

    fn compiled_linear() -> CompiledWorkflow {
        let spec = ForgeSpec {
            spec_version: SPEC_VERSION.to_string(),
            name: Some("开发测试流程".to_string()),
            intent: "从需求到验收".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                name: "开发测试流程".to_string(),
                stages: vec![
                    ForgeStage {
                        id: None,
                        role: StageRole::Dev,

                        role_definition: None,

                        profile_id: None,
                        goal: base_goal(),
                        artifact: Some("test-result".to_string()),
                        output_kind: StageOutputKind::Json,
                        schema_hint: Some(serde_json::json!({ "passed": "boolean" })),
                        success_condition: Some("$.passed == true".to_string()),
                        on_failure: OnFailure::Retry,
                        manual_check: false,
                    },
                    ForgeStage {
                        id: None,
                        role: StageRole::Accept,

                        role_definition: None,

                        profile_id: None,
                        goal: base_goal(),
                        artifact: None,
                        output_kind: StageOutputKind::Markdown,
                        schema_hint: None,
                        success_condition: None,
                        on_failure: OnFailure::NewRound,
                        manual_check: false,
                    },
                ],
            }],
            open_questions: vec![],
        };
        compile(&spec).unwrap().remove(0)
    }

    #[test]
    fn round_trip_preserves_participating_fields() {
        let compiled = compiled_linear();
        let spec = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &compiled.workflow, "开发测试流程", "意图").unwrap();
        // 反投影 Spec 可再次编译（openQuestions 为空、字段齐全）。
        let recompiled = compile(&spec).unwrap().remove(0);
        // 参与循环的字段等值：goal、profile、output 契约、manual_check。
        for (left, right) in compiled
            .workflow
            .nodes
            .iter()
            .zip(recompiled.workflow.nodes.iter())
        {
            let (NodeDsl::Worker(a), NodeDsl::Worker(b)) = (left, right) else {
                panic!("expected worker nodes");
            };
            assert_eq!(a.goal, b.goal);
            assert_eq!(a.profile, b.profile);
            assert_eq!(
                serde_json::to_value(&a.output).unwrap(),
                serde_json::to_value(&b.output).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&a.success_condition).unwrap(),
                serde_json::to_value(&b.success_condition).unwrap()
            );
            assert_eq!(a.manual_check, b.manual_check);
        }
    }

    #[test]
    fn round_trip_failure_strategy_survives() {
        let compiled = compiled_linear();
        let spec = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &compiled.workflow, "开发测试流程", "意图").unwrap();
        assert_eq!(spec.workflows[0].stages[1].on_failure, OnFailure::NewRound);
        let recompiled = compile(&spec).unwrap().remove(0);
        let new_round_edges: Vec<_> = recompiled
            .workflow
            .edges
            .iter()
            .filter(|e| e.on == EdgeOutcome::Failure && e.to == NEW_ROUND_NODE)
            .collect();
        assert_eq!(new_round_edges.len(), 1);
    }

    #[test]
    fn write_back_preserves_non_participating_fields() {
        let compiled = compiled_linear();
        // 在原 DSL 上加“不参与循环”的字段值（模拟用户在编辑器中的配置）。
        let mut base = compiled.workflow.clone();
        if let NodeDsl::Worker(worker) = &mut base.nodes[0] {
            worker.execution_slot_id = Some("slot-kept".to_string());
            worker.permission_mode = Some("plan".to_string());
            worker.config_options.insert("key".to_string(), "value".to_string());
        }
        // 模拟对话修改：goal 变化后重编译。
        let mut revised = compiled.workflow.clone();
        if let NodeDsl::Worker(worker) = &mut revised.nodes[0] {
            worker.goal = Some("<输入>x\n<产出>y\n<边界>z\n<验收>w".to_string());
        }
        let written = write_back(
            &base,
            &CompiledWorkflow {
                name: "样本".to_string(),
                workflow: revised,
                pending_profiles: vec![],
            },
        );
        let NodeDsl::Worker(worker) = &written.nodes[0] else {
            panic!("expected worker");
        };
        // 参与循环的字段被覆盖为新值。
        assert_eq!(worker.goal.as_deref(), Some("<输入>x\n<产出>y\n<边界>z\n<验收>w"));
        // 不参与循环的字段保留原值。
        assert_eq!(worker.execution_slot_id.as_deref(), Some("slot-kept"));
        assert_eq!(worker.permission_mode.as_deref(), Some("plan"));
        assert_eq!(
            worker.config_options.get("key").map(String::as_str),
            Some("value")
        );
    }

    #[test]
    fn ai_dynamic_workflow_rejected() {
        let mut workflow = compiled_linear().workflow;
        workflow.nodes.push(NodeDsl::AiDynamic(crate::dsl::AiDynamicNode {
            id: "dynamic-1".to_string(),
            agent_strategy: crate::dsl::AiDynamicAgentStrategy::Fixed {
                provider: "claude".to_string(),
                model: None,
                permission_mode: None,
                auto_accept: false,
            },
            config_options: Default::default(),
            model_bound_overrides: Default::default(),
            allowed_profiles: vec![],
            global_goal: None,
            control: Default::default(),
            allowed_workflows: vec![],
        }));
        let error = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &workflow, "n", "i").unwrap_err();
        assert_eq!(error.code(), "forge/invalid-spec");
    }

    #[test]
    fn max_stage_limit_enforced_in_round_trip() {
        // 编译产物节点数与 Spec stages 一致，上限在 Spec 校验层兜底。
        let mut spec = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &compiled_linear().workflow, "n", "i").unwrap();
        spec.workflows[0].stages = (0..MAX_STAGES_PER_WORKFLOW + 1)
            .map(|_| ForgeStage {
                id: None,
                role: StageRole::Dev,

                role_definition: None,

                profile_id: None,
                goal: base_goal(),
                artifact: None,
                output_kind: StageOutputKind::None,
                schema_hint: None,
                success_condition: None,
                on_failure: OnFailure::Retry,
                manual_check: false,
            })
            .collect();
        assert_eq!(compile(&spec).unwrap_err().code(), "forge/too-many-stages");
    }
}
