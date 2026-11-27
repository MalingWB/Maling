//! WorkflowDsl → ForgeSpec 反投影（A3，对话式修改路径）。
//!
//! 现有工作流反投影为 Spec 进入 Forge 会话，修改后重新编译。
//! 无损回写约束（§3.3）：Spec 不表达的字段（executionSlotId、
//! permission_mode、config_options 等）不参与循环，重编译时保留原值；
//! 仅对新增节点查默认绑定。
//!
//! 反投影是编译的逆映射（尽力而为的投影，不是全量逆函数）：
//! - worker 节点（按线性 success 边排序）→ stages；
//! - profile 反查 role（内置五类），非内置 profile 记为 Custom（内置角色是只读引用）；
//! - failure 边还原 onFailure：到 `$new-round`/`$end` 的照边还原，指向真实节点的是跳转
//!   （回到本节点即重试），没有 failure 边等价于失败即结束；
//! - manual_check 优先于 output 契约还原。
//!
//! AiDynamic 节点不属于 Forge 的平铺模型：反投影时报 `forge/invalid-spec`
//! 上下文错误，由上层引导用户先在编辑器中处理，不做静默丢弃。

use crate::dsl::{
    EdgeOutcome, JsonConditionDsl, NodeDsl, WorkflowDsl, END_NODE, ENTRY_NODE,
    NEW_ROUND_NODE,
};
use crate::forge::compiler::CompiledWorkflow;
use crate::forge::spec::{
    ForgeError, ForgeSpec, ForgeStage, ForgeWorkflow, OnFailure, StageOutputKind, StageRole,
    SPEC_VERSION,
};

/// 铸造角色显示名的分隔符：显示名是「工作流名 · 角色名」。
///
/// 角色库是用户级的，而每次铸造都会新建一个角色；只叫「发布审计员」的话，工作流
/// 一多就分不清归属、彼此重名。Spec 里的 `profileName` 始终是角色名本身，显示名
/// 只在落库时拼装、反投影时还原。
pub const FORGE_ROLE_NAME_SEPARATOR: &str = " · ";

/// 铸造角色的显示名。`persona` 若已带本工作流前缀（模型照着基线回写了显示名），
/// 先还原成角色名，前缀不会叠加。
pub fn forge_role_name(workflow_name: &str, persona: &str) -> String {
    let persona = forge_role_persona(persona, workflow_name).unwrap_or(persona);
    format!("{workflow_name}{FORGE_ROLE_NAME_SEPARATOR}{persona}")
}

/// 从角色显示名还原角色名：只有带「该工作流 · 」前缀的才是这个工作流铸造出来的
/// 角色；用户自己建的角色（或归属别的工作流的）返回 `None`，调用方原样保留名字。
pub fn forge_role_persona<'a>(stored_name: &'a str, workflow_name: &str) -> Option<&'a str> {
    stored_name
        .strip_prefix(workflow_name)?
        .strip_prefix(FORGE_ROLE_NAME_SEPARATOR)
}

/// 角色是否是这条工作流铸造出来的：显示名带「工作流名 · 」前缀。工作流在对话里
/// 改过名时，角色名里还是改名前的名字，所以 `former_workflow_name`（对应模板当前
/// 的名字）也算。
pub fn is_forged_role_name(
    stored_name: &str,
    workflow_name: &str,
    former_workflow_name: Option<&str>,
) -> bool {
    forge_role_persona(stored_name, workflow_name).is_some()
        || former_workflow_name
            .is_some_and(|former| forge_role_persona(stored_name, former).is_some())
}

/// 内置 profile id → StageRole 的反查表（与 compiler.rs 的正向映射互逆）。
fn role_from_profile(profile: Option<&str>) -> StageRole {
    match profile {
        Some("pf-builtin-dev") => StageRole::Dev,
        Some("pf-builtin-test") => StageRole::Test,
        Some("pf-builtin-review") => StageRole::Review,
        Some("pf-builtin-plan") => StageRole::Plan,
        Some("pf-builtin-accept") => StageRole::Accept,
        Some(id) => StageRole::Custom { profile_name: id.to_owned() },
        None => StageRole::Unassigned,
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

/// onFailure 还原：failure 出边目标决定策略。指向 `$new-round` / `$end` 的照边还原（带上
/// 重开起点）；指向真实节点的是跳转，回到本节点且不沿用会话的即重试；没有 failure 边
/// 等价于失败即结束（`End`）——还原成别的策略会让重新编译改变工作流的行为。
/// manual_check 置位（且无 output 验证）时优先还原为 ManualCheck。
fn on_failure_for(workflow: &WorkflowDsl, node_id: &str, manual_check: bool) -> OnFailure {
    for edge in &workflow.edges {
        if edge.from != node_id || edge.on != EdgeOutcome::Failure {
            continue;
        }
        if edge.to == NEW_ROUND_NODE {
            return match edge
                .new_round_entry
                .as_deref()
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
            {
                None | Some(ENTRY_NODE) => OnFailure::NewRound,
                Some(stage) => OnFailure::NewRoundFrom(stage.to_string()),
            };
        }
        if edge.to == END_NODE {
            return OnFailure::End;
        }
        let continue_session = edge.session == Some(crate::domain::SessionMode::Continue);
        if edge.to == node_id && !continue_session {
            return OnFailure::Retry;
        }
        return OnFailure::Jump {
            stage: edge.to.clone(),
            continue_session,
        };
    }
    if manual_check { OnFailure::ManualCheck } else { OnFailure::End }
}

/// 从 DSL 还原一个 stage（best effort：goal 缺失时给出最小占位文本，
/// 保证反投影产物可进 Forge 会话继续拷打收敛）。
fn stage_from_worker(
    paths: &crate::storage::GoldBandPaths,
    language: crate::config::DesktopLanguage,
    workflow: &WorkflowDsl,
    // 工作流显示名：还原铸造角色的名字时用来识别「工作流名 · 」前缀。
    workflow_name: &str,
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
    // custom 角色（非内置 profile 引用）：还原 profileId/显示名/职责正文进
    // Spec——对话中可编辑名称/定义，重编译按 id 原地更新（对话反哺）。
    // 解析成功时 role 的 profileName 用 profile 显示名（协议语义:名称是人读的,
    // id 由 profileId 字段承载;混用会把 id 写进角色名,2026-11-20 真机发现）。
    // profile 解析失败时保留节点引用文本作为名称,不阻塞反投影。
    let (role, profile_id, role_definition) = match role_from_profile(worker.profile.as_deref()) {
        StageRole::Custom { .. } => {
            let resolved = worker
                .profile
                .as_deref()
                .and_then(|profile_id| {
                    crate::app::profiles::find_profile_by_id(paths, profile_id, language).ok()
                    .flatten()
                });
            match resolved {
                // 五类之外的内置角色是只读引用：带上 profileId 与显示名，但不把内置正文
                // 当作可编辑的 roleDefinition 带进草稿（改了也不会生效）。
                Some(entry) if entry.is_built_in || !entry.forged => (
                    StageRole::Custom {
                        profile_name: entry.name.clone(),
                    },
                    Some(entry.id),
                    None,
                ),
                Some(entry) => (
                    StageRole::Custom {
                        // 铸造出来的角色显示名是「工作流名 · 角色名」；这里还原出角色名，
                        // 重新编译时再拼回去，前缀不会叠加。用户自建的角色原样保留。
                        profile_name: forge_role_persona(&entry.name, workflow_name)
                            .unwrap_or(&entry.name)
                            .to_string(),
                    },
                    Some(entry.id),
                    Some(entry.content),
                ),
                None => (
                    StageRole::Custom {
                        profile_name: worker
                            .profile
                            .clone()
                            .unwrap_or_else(|| "custom".to_string()),
                    },
                    worker.profile.clone(),
                    None,
                ),
            }
        }
        builtin => (builtin, None, None),
    };
    Ok(ForgeStage {
        id: Some(node_id.to_string()),
        role,
        goal: crate::forge::spec::ForgeGoal::Projected { original: worker.goal.clone() },
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

/// Walk the outer success chain to termination, expanding each group in branch order.
/// The save path uses this same check before touching an existing target.
pub(crate) fn ordered_worker_ids(workflow: &WorkflowDsl) -> Result<Vec<String>, ForgeError> {
    use std::collections::{HashMap, HashSet};
    let invalid = || ForgeError::InvalidSpec {
        detail: format!("workflow `{}` cannot round-trip through Forge", workflow.id),
    };
    let worker_count = workflow.nodes.iter().filter(|n| matches!(n, NodeDsl::Worker(_))).count();
    if worker_count > crate::forge::MAX_STAGES_PER_WORKFLOW {
        return Err(ForgeError::TooManyStages { workflow: workflow.id.clone(), count: worker_count, limit: crate::forge::MAX_STAGES_PER_WORKFLOW });
    }
    if workflow.nodes.iter().any(|n| matches!(n, NodeDsl::AiDynamic(_)) || !crate::forge::spec::stage_id_is_valid(n.id()))
        || workflow.edges.iter().any(|e| e.new_round_entry.is_some() && e.to != NEW_ROUND_NODE) {
        return Err(invalid());
    }
    // This also rejects duplicate IDs/outcomes and unsafe member edges before ordering.
    crate::dsl::validate_authoring_workflow(workflow.clone()).map_err(|e| ForgeError::InvalidSpec { detail: e.to_string() })?;
    let by_id: HashMap<_, _> = workflow.nodes.iter().map(|n| (n.id(), n)).collect();
    let successors: HashMap<_, _> = workflow.edges.iter().filter(|e| e.on == EdgeOutcome::Success)
        .map(|e| (e.from.as_str(), e.to.as_str())).collect();
    let mut visited = HashSet::new();
    let mut stages = Vec::with_capacity(worker_count);
    let mut current = workflow.entry.as_str();
    while current != END_NODE {
        if !visited.insert(current) { return Err(invalid()); }
        match by_id.get(current).copied() {
            Some(NodeDsl::Worker(_)) => stages.push(current.to_owned()),
            Some(NodeDsl::ParallelGroup(group)) => {
                for member in group.branches.iter().flat_map(|b| &b.stages) {
                    if !visited.insert(member.as_str()) { return Err(invalid()); }
                    stages.push(member.clone());
                }
            }
            _ => return Err(invalid()),
        }
        current = successors.get(current).copied().unwrap_or(END_NODE);
    }
    if visited.len() != by_id.len() { return Err(invalid()); }
    Ok(stages)
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
        .map(|node_id| stage_from_worker(paths, language, workflow, name, node_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ForgeSpec {
        revision: 0,
        spec_version: SPEC_VERSION.to_string(),
        name: Some(name.to_string()),
        intent: intent.to_string(),
        source_skills: vec![],
        workflows: vec![ForgeWorkflow {
            id: String::new(),
            parallel_groups: workflow.nodes.iter().filter_map(|n| match n {
                NodeDsl::ParallelGroup(g) => Some(crate::forge::spec::ForgeParallelGroup {
                    id: g.id.clone(), branches: g.branches.clone(), max_parallel: g.max_parallel,
                    on_failure: on_failure_for(workflow, &g.id, false),
                }),
                _ => None,
            }).collect(),
            name: name.to_string(),
            stages,
        }],
        open_questions: vec![],
    })
}

/// 无损回写：以原模板 DSL 为基底应用重编译结果。
///
/// 节点按 id（= stage id）对齐：命中的节点只覆盖 Spec 表达的字段（profile、
/// goal、产物契约、成功条件、人工复核），`executionSlotId`（模型绑定的挂载点）
/// 与节点级运行配置保留原值；未命中即新增节点，取编译器产物（slot 走上层默认
/// 绑定）；原链上未命中的节点随 stage 删除。节点顺序、边与 entry 以重编译结果
/// 为准（链路结构由 Spec 表达）；`control` 等模板级可调项保留原值。
pub fn write_back(base: &WorkflowDsl, revised: &CompiledWorkflow) -> Result<WorkflowDsl, ForgeError> {
    ordered_worker_ids(base)?;
    let mut workflow = base.clone();
    workflow.nodes = revised
        .workflow
        .nodes
        .iter()
        .map(|node| {
            let NodeDsl::Worker(compiled) = node else {
                return node.clone();
            };
            let Some(NodeDsl::Worker(existing)) = base
                .nodes
                .iter()
                .find(|existing| existing.id() == compiled.id)
            else {
                return node.clone();
            };
            NodeDsl::Worker(crate::dsl::WorkerNode {
                id: existing.id.clone(),
                execution_slot_id: existing.execution_slot_id.clone(),
                provider: existing.provider.clone(),
                model: existing.model.clone(),
                profile: compiled.profile.clone(),
                goal: compiled.goal.clone(),
                output: compiled.output.clone(),
                success_condition: if compiled.success_condition.as_ref().and_then(condition_text)
                    == existing.success_condition.as_ref().and_then(condition_text) {
                    existing.success_condition.clone()
                } else { compiled.success_condition.clone() },
                permission_mode: existing.permission_mode.clone(),
                auto_accept: existing.auto_accept,
                config_options: existing.config_options.clone(),
                manual_check: if existing.manual_check.unwrap_or(false) == compiled.manual_check.unwrap_or(false) {
                    existing.manual_check
                } else { compiled.manual_check },
                prompt_envelope: existing.prompt_envelope,
            })
        })
        .collect();
    // 边、entry 与节点顺序整体以重编译结果为准。成功边上的 session 策略不由 Spec
    // 表达，起止相同的成功边沿用原值（失败边的 session 由跳转策略表达，以结果为准）。
    workflow.edges = revised
        .workflow
        .edges
        .iter()
        .cloned()
        .filter(|edge| {
            // “失败即结束”在 DSL 里有两种等价写法：显式失败边指向 `$end`，或者没有失败边。
            // Spec 不区分它们：原节点本来没有失败边时保留这种写法，不让一次只改了别处的
            // 对话式修改凭空多出一条边。
            let kept_node = base.nodes.iter().any(|node| node.id() == edge.from);
            let had_failure_edge = base
                .edges
                .iter()
                .any(|base_edge| base_edge.from == edge.from && base_edge.on == EdgeOutcome::Failure);
            !(kept_node && !had_failure_edge && edge.on == EdgeOutcome::Failure && edge.to == END_NODE)
        })
        .map(|mut edge| {
            if let Some(original) = base.edges.iter().find(|base_edge|
                base_edge.from == edge.from && base_edge.to == edge.to && base_edge.on == edge.on) {
                // Success policy is not authored here. Explicit 'new' on an unchanged
                // failure edge is the same policy as the compiler's implicit default.
                if edge.on == EdgeOutcome::Success || (edge.session.is_none() && original.session == Some(crate::domain::SessionMode::New)) {
                    edge.session = original.session;
                }
            }
            edge
        })
        .collect();
    // No outgoing success edge also terminates an outer chain. Keep the original
    // spelling when the revised graph already has another explicit $end (DSL requires one).
    if workflow.edges.iter().filter(|e| e.to == END_NODE).take(2).count() == 2 {
        workflow.edges.retain(|e| {
            let implicit_end = e.on == EdgeOutcome::Success && e.to == END_NODE
                && base.nodes.iter().any(|n| n.id() == e.from)
                && !base.edges.iter().any(|old| old.from == e.from && old.on == EdgeOutcome::Success)
                && !base.nodes.iter().any(|n| matches!(n, NodeDsl::ParallelGroup(g)
                    if g.branches.iter().any(|b| b.stages.contains(&e.from))));
            !implicit_end
        });
    }
    workflow.entry = revised.workflow.entry.clone();
    // The base may carry a different retry budget from the compiler defaults.
    ordered_worker_ids(&workflow)?;
    Ok(workflow)
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::dsl::EdgeDsl;
    use super::*;
    use crate::forge::compiler::compile;
    use crate::forge::spec::MAX_STAGES_PER_WORKFLOW;

    /// 反投影测试的隔离路径(custom 角色的 profile 解析走临时目录)。
    pub(crate) fn test_paths() -> crate::storage::GoldBandPaths {
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
            revision: 0,
            spec_version: SPEC_VERSION.to_string(),
            name: Some("开发测试流程".to_string()),
            intent: "从需求到验收".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                id: String::new(),
                parallel_groups: vec![],
                name: "开发测试流程".to_string(),
                stages: vec![
                    ForgeStage {
                        id: None,
                        role: StageRole::Dev,

                        role_definition: None,

                        profile_id: None,
                        goal: base_goal().into(),
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
                        goal: base_goal().into(),
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
    fn write_back_follows_the_recompiled_chain_and_keeps_matched_nodes() {
        let compiled = compiled_linear();
        let mut base = compiled.workflow.clone();
        base.control.max_rounds = Some(9);
        for (index, slot) in ["slot-1", "slot-2"].iter().enumerate() {
            if let NodeDsl::Worker(worker) = &mut base.nodes[index] {
                worker.execution_slot_id = Some(slot.to_string());
            }
        }
        // 对话修改：最前面插入新阶段、末尾阶段被删除。
        let stage_with = |id: &str, role: StageRole| ForgeStage {
            id: Some(id.to_string()),
            role,
            role_definition: None,
            profile_id: None,
            goal: base_goal().into(),
            artifact: None,
            output_kind: StageOutputKind::None,
            schema_hint: None,
            success_condition: None,
            on_failure: OnFailure::Retry,
            manual_check: false,
        };
        let kept_id = base.nodes[0].id().to_string();
        let revised = compile(&ForgeSpec {
            revision: 0,
            spec_version: SPEC_VERSION.to_string(),
            name: Some("改名后的流程".to_string()),
            intent: "意图".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                id: String::new(),
                parallel_groups: vec![],
                name: "改名后的流程".to_string(),
                stages: vec![stage_with("plan", StageRole::Plan), stage_with(&kept_id, StageRole::Dev)],
            }],
            open_questions: vec![],
        })
        .unwrap()
        .remove(0);

        let written = write_back(&base, &revised).unwrap();

        // 顺序与 entry 跟随重编译链（旧实现把新节点追加在末尾且不更新 entry）。
        let ids: Vec<&str> = written.nodes.iter().map(|node| node.id()).collect();
        assert_eq!(ids, ["plan", kept_id.as_str()]);
        assert_eq!(written.entry, "plan");
        let success = written
            .edges
            .iter()
            .find(|edge| edge.on == EdgeOutcome::Success)
            .expect("success edge");
        assert_eq!((success.from.as_str(), success.to.as_str()), ("plan", kept_id.as_str()));
        // 命中的节点保留 slot；新节点没有 slot（由保存链路分配并补默认绑定）。
        let NodeDsl::Worker(new_node) = &written.nodes[0] else { panic!("worker") };
        assert!(new_node.execution_slot_id.is_none());
        let NodeDsl::Worker(kept_node) = &written.nodes[1] else { panic!("worker") };
        assert_eq!(kept_node.execution_slot_id.as_deref(), Some("slot-1"));
        // 模板级可调项保留。
        assert_eq!(written.control.max_rounds, Some(9));
        assert_eq!(written.id, base.id);
    }

    #[test]
    fn forged_role_names_round_trip_without_stacking_the_prefix() {
        assert_eq!(forge_role_name("发布闸门", "发布复核员"), "发布闸门 · 发布复核员");
        assert_eq!(forge_role_persona("发布闸门 · 发布复核员", "发布闸门"), Some("发布复核员"));
        // 模型照着显示名回写了角色名：不再叠一层。
        assert_eq!(
            forge_role_name("发布闸门", "发布闸门 · 发布复核员"),
            "发布闸门 · 发布复核员"
        );
        // 改名前的名字也算本工作流铸造的角色；别的工作流的不算。
        assert!(is_forged_role_name("样本流程 · 发布复核员", "发布闸门", Some("样本流程")));
        assert!(!is_forged_role_name("样本流程 · 发布复核员", "发布闸门", None));
        // 用户自己建的角色原样保留：没有前缀的，以及名字里恰好带分隔符的。
        for hand_made in ["严格审查员", "Review · strict", "发布闸门"] {
            assert_eq!(forge_role_persona(hand_made, "发布闸门"), None, "{hand_made}");
            assert!(!is_forged_role_name(hand_made, "发布闸门", Some("样本流程")));
        }
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
        ).unwrap();
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

    fn plain_worker(id: &str) -> NodeDsl {
        NodeDsl::Worker(crate::dsl::WorkerNode {
            id: id.to_string(),
            execution_slot_id: None,
            provider: None,
            model: None,
            profile: None,
            goal: Some(base_goal()),
            output: None,
            success_condition: None,
            permission_mode: None,
            auto_accept: false,
            config_options: Default::default(),
            manual_check: None,
            prompt_envelope: Default::default(),
        })
    }

    fn plain_edge(from: &str, to: &str, on: EdgeOutcome, session: Option<crate::domain::SessionMode>, new_round_entry: Option<&str>) -> EdgeDsl {
        EdgeDsl {
            from: from.to_string(),
            to: to.to_string(),
            on,
            session,
            new_round_entry: new_round_entry.map(str::to_string),
        }
    }

    /// 边的可比较投影：起止、结果类型、会话策略与重开起点。
    fn edge_shape(workflow: &WorkflowDsl) -> Vec<[String; 5]> {
        let mut shape: Vec<_> = workflow
            .edges
            .iter()
            .map(|edge| {
                [
                    edge.from.clone(),
                    edge.to.clone(),
                    format!("{:?}", edge.on),
                    edge.session.map(|session| format!("{session:?}")).unwrap_or_default(),
                    edge.new_round_entry.clone().unwrap_or_default(),
                ]
            })
            .collect();
        shape.sort();
        shape
    }

    #[test]
    fn failure_jumps_survive_the_round_trip() {
        // 回归：反投影曾把“失败边指向真实节点”一律记成 retry、丢掉 `$new-round` 的重开
        // 起点，重新编译时这些边随之消失（默认工作流的“审查/测试失败回到开发”就是这样丢的）。
        let base = WorkflowDsl {
            version: "0.1".to_string(),
            id: "repair-loop".to_string(),
            entry: "dev".to_string(),
            control: Default::default(),
            nodes: vec![plain_worker("dev"), plain_worker("test"), plain_worker("accept")],
            edges: vec![
                // 成功边上的 session 策略不由 Spec 表达，回写时必须保留。
                plain_edge("dev", "test", EdgeOutcome::Success, Some(crate::domain::SessionMode::Continue), None),
                plain_edge("test", "accept", EdgeOutcome::Success, None, None),
                plain_edge("test", "dev", EdgeOutcome::Failure, Some(crate::domain::SessionMode::Continue), None),
                plain_edge("accept", END_NODE, EdgeOutcome::Success, None, None),
                plain_edge("accept", NEW_ROUND_NODE, EdgeOutcome::Failure, None, Some("test")),
            ],
        };

        let spec = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &base, "修复回路", "意图").unwrap();

        let strategies: Vec<&OnFailure> = spec.workflows[0].stages.iter().map(|stage| &stage.on_failure).collect();
        // 没有 failure 边 = 失败即结束。
        assert_eq!(strategies[0], &OnFailure::End);
        assert_eq!(
            strategies[1],
            &OnFailure::Jump {
                stage: "dev".to_string(),
                continue_session: true
            }
        );
        assert_eq!(strategies[2], &OnFailure::NewRoundFrom("test".to_string()));

        let revised = compile(&spec).unwrap().remove(0);
        let written = write_back(&base, &revised).unwrap();
        assert_eq!(edge_shape(&written), edge_shape(&base));
    }

    #[test]
    fn a_node_without_a_failure_edge_ends_the_run_when_it_fails() {
        // 运行时语义：没有 failure 边等价于 failure → $end。反投影必须还原成 End，
        // 否则重新编译会给原本“失败即结束”的节点加上重试。
        let base = WorkflowDsl {
            version: "0.1".to_string(),
            id: "linear".to_string(),
            entry: "dev".to_string(),
            control: Default::default(),
            nodes: vec![plain_worker("dev")],
            edges: vec![plain_edge("dev", END_NODE, EdgeOutcome::Success, None, None)],
        };
        let spec = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &base, "线性", "意图").unwrap();
        assert_eq!(spec.workflows[0].stages[0].on_failure, OnFailure::End);

        // 自环失败边即重试；沿用会话的自环是跳回自己。
        let mut retrying = base.clone();
        retrying.edges.push(plain_edge("dev", "dev", EdgeOutcome::Failure, None, None));
        let spec = from_dsl(&test_paths(), crate::config::DesktopLanguage::ZhCn, &retrying, "线性", "意图").unwrap();
        assert_eq!(spec.workflows[0].stages[0].on_failure, OnFailure::Retry);
        let revised = compile(&spec).unwrap().remove(0);
        assert_eq!(edge_shape(&write_back(&retrying, &revised).unwrap()), edge_shape(&retrying));
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
                goal: base_goal().into(),
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
