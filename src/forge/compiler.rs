//! Spec → WorkflowDsl 确定性编译器。
//!
//! 编译规则（实施方案 §3.2）全部为纯函数映射，无 LLM 参与：
//! - stage → worker 节点（作者态 provider/model 置空，executionSlotId 走绑定解耦）；
//! - role → 内置 profile 选型（`custom` 由上层以 goal 创建 User scope profile 后回填）；
//! - onFailure → 失败边映射（retry 不加边、manual-check 置位、new-round/end 加边）；
//! - 末 stage 补 success → `$end`；
//! - 产物 100% 过 `validate_authoring_workflow`（provider 可空模式），
//!   不通过即 `forge/compile-failed`（含错误明细，供 spec-repair 反喂）。

use crate::dsl::{
    EdgeDsl, EdgeOutcome, JsonConditionDsl, NodeDsl, OutputContractDsl, OutputKind, WorkerNode,
    WorkflowControl, WorkflowDsl, END_NODE, ENTRY_NODE, NEW_ROUND_NODE,
};

use crate::forge::spec::{
    ForgeError, ForgeSpec, ForgeStage, ForgeWorkflow, OnFailure, StageOutputKind, StageRole,
};

/// 编译产物：一条工作流 +（custom role 的）新 profile 草案。
///
/// custom role 的 profile 不在编译器内落库（编译器保持纯函数），
/// 由调用方（`create_forge_task` 保存链路）将草案注册为 User scope profile
/// 后，用返回的 `profile_id` 替换占位。
#[derive(Debug, Clone)]
pub struct CompiledWorkflow {
    /// 模板显示名（单工作流 = Spec 顶层名；拆分 = 各工作流名）。
    pub name: String,
    pub workflow: WorkflowDsl,
    /// 需要新创建的 profile：node_id → (显示名, goal 正文)。
    pub pending_profiles: Vec<(String, String, String)>,
}

/// role → 内置 profile id 的映射。
///
/// DSL 节点引用内置 profile 的完整 id（`pf-builtin-<key>`，见
/// `crate::app::profiles::DEFAULT_PROFILE_SEEDS`）；裸 key 无法通过
/// 保存链路的 profile 解析。
fn builtin_profile_id(role: &StageRole) -> Option<String> {
    match role {
        StageRole::Dev => Some("pf-builtin-dev".to_string()),
        StageRole::Test => Some("pf-builtin-test".to_string()),
        StageRole::Review => Some("pf-builtin-review".to_string()),
        StageRole::Plan => Some("pf-builtin-plan".to_string()),
        StageRole::Accept => Some("pf-builtin-accept".to_string()),
        StageRole::Custom { .. } => None,
    }
}

/// stage 标识 → 节点 id：slug 化 + 序号去重。
fn node_id_for(workflow: &ForgeWorkflow, index: usize) -> String {
    let slug = workflow.name.trim().to_ascii_lowercase().chars().map(|c| {
        if c.is_ascii_alphanumeric() { c } else { '-' }
    }).collect::<String>().trim_matches('-').to_string();
    let base = if slug.is_empty() { "stage".to_string() } else { slug };
    format!("{base}-{}", index + 1)
}

/// 编译单条工作流。
fn compile_one(workflow: &ForgeWorkflow, control: WorkflowControl) -> Result<CompiledWorkflow, ForgeError> {
    let id = slug_id(&workflow.name);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut pending_profiles = Vec::new();

    for (index, stage) in workflow.stages.iter().enumerate() {
        let node_id = node_id_for(workflow, index);
        let (profile, pending) = match builtin_profile_id(&stage.role) {
            Some(profile) => (Some(profile), None),
            None => {
                let StageRole::Custom { profile_name } = &stage.role else {
                    unreachable!("builtin_profile_id None implies Custom");
                };
                let placeholder = format!("{node_id}-profile");
                pending_profiles.push((
                    node_id.clone(),
                    profile_name.clone(),
                    stage.goal.clone(),
                ));
                (Some(placeholder), Some(()))
            }
        };
        let _ = pending;

        let (output, success_condition, manual_check) = output_contract_for(stage)?;

        nodes.push(NodeDsl::Worker(WorkerNode {
            id: node_id.clone(),
            execution_slot_id: None,
            provider: None,
            model: None,
            profile,
            goal: Some(stage.goal.clone()),
            output,
            success_condition,
            permission_mode: None,
            auto_accept: false,
            config_options: Default::default(),
            manual_check: Some(manual_check),
            prompt_envelope: Default::default(),
        }));

        // onFailure 边映射
        match stage.on_failure {
            OnFailure::Retry => {
                // 重试由 control.max_attempts 兜住，不加边；
                // 但 worker 至少需要一条 success 出边，循环结束后统一补。
            }
            OnFailure::ManualCheck => {
                // manual_check 与 output 验证互斥（DSL 校验规则），
                // 编译器强制剥离 output/success_condition（output_contract_for 已处理）。
            }
            OnFailure::NewRound => edges.push(EdgeDsl {
                from: node_id.clone(),
                to: NEW_ROUND_NODE.to_string(),
                on: EdgeOutcome::Failure,
                session: None,
                new_round_entry: Some(ENTRY_NODE.to_string()),
            }),
            OnFailure::End => edges.push(EdgeDsl {
                from: node_id.clone(),
                to: END_NODE.to_string(),
                on: EdgeOutcome::Failure,
                session: None,
                new_round_entry: None,
            }),
        }

        // 线性 success 边到下一节点；末节点在循环后补 $end。
        if index + 1 < workflow.stages.len() {
            edges.push(EdgeDsl {
                from: node_id.clone(),
                to: node_id_for(workflow, index + 1),
                on: EdgeOutcome::Success,
                session: None,
                new_round_entry: None,
            });
        }
    }

    // 末节点 success → $end
    if let Some(last) = nodes.last() {
        edges.push(EdgeDsl {
            from: last.id().to_string(),
            to: END_NODE.to_string(),
            on: EdgeOutcome::Success,
            session: None,
            new_round_entry: None,
        });
    }

    let dsl = WorkflowDsl {
        version: "0.1".to_string(),
        id,
        entry: nodes
            .first()
            .map(|node| node.id().to_string())
            .ok_or_else(|| ForgeError::EmptyStages {
                workflow: workflow.name.clone(),
            })?,
        control,
        nodes,
        edges,
    };

    Ok(CompiledWorkflow {
        name: workflow.name.clone(),
        workflow: dsl,
        pending_profiles,
    })
}

/// stage 的 output 契约映射。
///
/// - json：产出 output 契约 + success_condition（path 存在性由 DSL 校验兜底）；
/// - markdown / none：无 output、无 success_condition；
/// - manual_check=true 时强制剥离（与 output 验证互斥，onFailure=ManualCheck
///   的互斥在 validate 阶段由编译入口统一裁决）。
fn output_contract_for(
    stage: &ForgeStage,
) -> Result<(Option<OutputContractDsl>, Option<JsonConditionDsl>, bool), ForgeError> {
    let manual_check_requested = stage.manual_check || stage.on_failure == OnFailure::ManualCheck;
    if manual_check_requested {
        return Ok((None, None, true));
    }
    match stage.output_kind {
        StageOutputKind::Json => {
            let artifact = stage.artifact.clone().unwrap_or_else(|| "artifact".to_string());
            let output = OutputContractDsl {
                kind: OutputKind::Json,
                artifact,
                schema: stage.schema_hint.clone(),
            };
            let success_condition = stage
                .success_condition
                .as_deref()
                .map(str::trim)
                .filter(|expression| !expression.is_empty())
                .map(|expression| JsonConditionDsl::Expression {
                    expression: expression.to_string(),
                });
            Ok((Some(output), success_condition, false))
        }
        StageOutputKind::Markdown | StageOutputKind::None => Ok((None, None, false)),
    }
}

/// 名称 → slug id：ASCII 字母数字保留、其他字符折叠为 `-`；
/// 非 ASCII（中文等）字符保留，避免多字节名称全部塌缩为同一 fallback。
/// 完全折叠为空时回退 `forged-workflow`。
fn slug_id(name: &str) -> String {
    let slug = name
        .trim()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "forged-workflow".to_string()
    } else {
        slug
    }
}

/// 编译入口：Spec 全量校验 → 逐工作流编译 → 逐个过 DSL 校验。
///
/// 编译顺序：先 `ForgeSpec::validate`（收敛完整性），再确定性映射，
/// 最后 `validate_authoring_workflow`（结构合法性）。任何一层失败
/// 返回结构化错误码，供前端与 spec-repair 消费。
pub fn compile(spec: &ForgeSpec) -> Result<Vec<CompiledWorkflow>, ForgeError> {
    spec.validate()?;

    // manual_check 与 json output + success_condition 互斥（DSL 规则）：
    // on_failure = manual-check 的 stage，其 outputKind 必须为 markdown/none，
    // 由 output_contract_for 剥离，不在此报错（宽容策略：语义上人工复核
    // 取代机器验收，剥离优于拒绝）。

    let mut results = Vec::new();
    for workflow in &spec.workflows {
        let control = WorkflowControl {
            max_attempts: Some(3),
            max_rounds: Some(2),
        };
        let name = if spec.workflows.len() == 1 {
            spec.name.clone().unwrap_or_else(|| workflow.name.clone())
        } else {
            workflow.name.clone()
        };
        let compiled = compile_one(workflow, control)?;
        let compiled = CompiledWorkflow {
            name,
            ..compiled
        };
        // 产物 100% 过 DSL 校验
        crate::dsl::validate_authoring_workflow(compiled.workflow.clone()).map_err(|error| {
            ForgeError::CompileFailed {
                detail: error.to_string(),
            }
        })?;
        results.push(compiled);
    }
    Ok(results)
}

/// 校验 Spec 可编译性但不产出（前端"预检"用）。
pub fn precheck(spec: &ForgeSpec) -> Result<(), ForgeError> {
    spec.validate()?;
    for workflow in &spec.workflows {
        let control = WorkflowControl {
            max_attempts: Some(3),
            max_rounds: Some(2),
        };
        let compiled = compile_one(workflow, control)?;
        crate::dsl::validate_authoring_workflow(compiled.workflow).map_err(|error| {
            ForgeError::CompileFailed {
                detail: error.to_string(),
            }
        })?;
    }
    Ok(())
}

/// 供前端判断：编译器当前只产 new-session 边（无 session=continue）。
pub fn uses_continue_session() -> bool {
    false
}
