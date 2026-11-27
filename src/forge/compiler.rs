//! Spec → WorkflowDsl 确定性编译器。
//!
//! 编译规则（实施方案 §3.2）全部为纯函数映射，无 LLM 参与：
//! - stage → worker 节点，节点 id = stage id（作者态 provider/model 置空，
//!   executionSlotId 走绑定解耦）；
//! - role → 内置 profile 选型（`custom` 由上层以 goal 创建 User scope profile 后回填）；
//! - onFailure → 失败边映射（retry 为自环边、manual-check 置位、new-round 及
//!   带重起点的重开指向 `$new-round`、跳转指向目标阶段、end 指向 `$end`）；
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

/// 编译产物的默认重试次数与轮数上限（模板 control，用户可在编辑器调整）。
const DEFAULT_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_MAX_ROUNDS: u32 = 2;

fn default_control() -> WorkflowControl {
    WorkflowControl {
        max_attempts: Some(DEFAULT_MAX_ATTEMPTS),
        max_rounds: Some(DEFAULT_MAX_ROUNDS),
    }
}

/// custom 角色的铸造草案：显示名 + 职责正文 + 既有 profile id。
///
/// `profile_id` 为 Some 时表示该角色已铸造过（Spec 反投影或上一轮编译
/// 回填），调用方按 id 原地更新名称/正文；为 None 时新建 User scope
/// profile。`definition` 缺省回退 stage goal 正文。
#[derive(Debug, Clone)]
pub struct PendingProfile {
    pub node_id: String,
    /// 所属工作流内的 stage 序号（编译后回写 Spec 存档的 profileId 用）。
    pub stage_index: usize,
    pub name: String,
    /// 落库用的职责正文：stage 的 roleDefinition，缺省回退 goal。
    pub definition: String,
    /// stage 显式给出的 roleDefinition。同名的 stage 共用一个角色，职责正文以第一个
    /// 显式给出的为准（都没有时才用 goal）。
    pub role_definition: Option<String>,
    pub profile_id: Option<String>,
}

/// 编译产物：一条工作流 +（custom role 的）profile 铸造草案。
///
/// custom role 的 profile 不在编译器内落库（编译器保持纯函数），
/// 由调用方（`App::forge_compile` 保存链路）铸造/更新 User scope
/// profile 后，用 profile id 替换占位。
#[derive(Debug, Clone)]
pub struct CompiledWorkflow {
    /// 模板显示名（单工作流 = Spec 顶层名；拆分 = 各工作流名）。
    pub name: String,
    pub workflow: WorkflowDsl,
    /// 需要铸造/更新的 custom 角色 profile。
    pub pending_profiles: Vec<PendingProfile>,
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
        StageRole::Unassigned | StageRole::Custom { .. } => None,
    }
}

/// stage → 节点 id。
///
/// 节点 id 就是 stage id：同一 stage 重编译后节点 id 不变，才能与既有模板的
/// 节点对齐并保留模型绑定等配置（工作流改名、插入 stage 都不影响）。stage
/// 没有 id 或 id 不可作节点 id 时（未经存档直接编译的调用方）按位置生成。
fn node_id_for(workflow: &ForgeWorkflow, index: usize, stage: &ForgeStage) -> String {
    if let Some(id) = stage
        .id
        .as_deref()
        .filter(|id| crate::forge::spec::stage_id_is_valid(id))
    {
        return id.to_string();
    }
    generated_node_id_for(workflow, index)
}

/// 按位置生成节点 id：工作流名 slug 化 + 序号。
fn generated_node_id_for(workflow: &ForgeWorkflow, index: usize) -> String {
    let slug = workflow.name.trim().to_ascii_lowercase().chars().map(|c| {
        if c.is_ascii_alphanumeric() { c } else { '-' }
    }).collect::<String>().trim_matches('-').to_string();
    let base = if slug.is_empty() { "stage".to_string() } else { slug };
    format!("{base}-{}", index + 1)
}

/// 编译单条工作流。
///
/// manual_check 与 json output + success_condition 互斥（DSL 规则）：
/// on_failure = manual-check 的 stage，其 outputKind 必须为 markdown/none，
/// 由 output_contract_for 剥离，不在此报错（宽容策略：语义上人工复核
/// 取代机器验收，剥离优于拒绝）。
fn compile_one(workflow: &ForgeWorkflow) -> Result<CompiledWorkflow, ForgeError> {
    let id = slug_id(&workflow.name);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut pending_profiles = Vec::new();
    workflow.validate_groups()?;
    let owners: std::collections::HashMap<_, _> = workflow.parallel_groups.iter().flat_map(|g|
        g.branches.iter().flat_map(move |b| b.stages.iter().map(move |id| (id.as_str(), g)))
    ).collect();
    let mut emitted = std::collections::HashSet::new();
    let mut outer = Vec::new();

    for (index, stage) in workflow.stages.iter().enumerate() {
        let node_id = node_id_for(workflow, index, stage);
        let group = owners.get(node_id.as_str());
        if let Some(group) = group {
            if emitted.insert(group.id.as_str()) {
                outer.push(group.id.clone());
                nodes.push(NodeDsl::ParallelGroup(crate::dsl::ParallelGroupNode {
                    id: group.id.clone(), branches: group.branches.clone(), max_parallel: group.max_parallel,
                }));
                edges.extend(failure_edge(&group.id, &group.on_failure));
            }
        } else {
            outer.push(node_id.clone());
        }
        let profile = if let StageRole::Custom { profile_name } = &stage.role {
            let placeholder = format!("{node_id}-profile");
            let role_definition = stage
                .role_definition
                .clone()
                .filter(|value| !value.trim().is_empty());
            pending_profiles.push(PendingProfile {
                node_id: node_id.clone(),
                stage_index: index,
                name: profile_name.clone(),
                definition: role_definition
                    .clone()
                    .unwrap_or_else(|| stage.goal.worker_text().unwrap_or_default()),
                role_definition,
                profile_id: stage.profile_id.clone(),
            });
            Some(placeholder)
        } else {
            builtin_profile_id(&stage.role)
        };

        let (output, success_condition, manual_check) = output_contract_for(stage)?;

        nodes.push(NodeDsl::Worker(WorkerNode {
            id: node_id.clone(),
            execution_slot_id: None,
            provider: None,
            model: None,
            profile,
            goal: stage.goal.worker_text(),
            output,
            success_condition,
            permission_mode: None,
            auto_accept: false,
            config_options: Default::default(),
            manual_check: Some(manual_check),
            prompt_envelope: Default::default(),
        }));

        // A branch without a retry edge settles on failure; it never routes to the outer end.
        if group.is_none() || stage.on_failure != OnFailure::End {
            edges.extend(failure_edge(&node_id, &stage.on_failure));
        }
    }
    for (index, from) in outer.iter().enumerate() {
        edges.push(EdgeDsl {
            from: from.clone(), to: outer.get(index + 1).map(String::as_str).unwrap_or(END_NODE).into(),
            on: EdgeOutcome::Success, session: None, new_round_entry: None,
        });
    }

    let dsl = WorkflowDsl {
        version: "0.1".to_string(),
        id,
        entry: outer.first().cloned()
            .ok_or_else(|| ForgeError::EmptyStages {
                workflow: workflow.name.clone(),
            })?,
        control: default_control(),
        nodes,
        edges,
    };

    Ok(CompiledWorkflow {
        name: workflow.name.clone(),
        workflow: dsl,
        pending_profiles,
    })
}

fn failure_edge(from: &str, policy: &OnFailure) -> Option<EdgeDsl> {
    let (to, session, new_round_entry) = match policy {
        OnFailure::ManualCheck => return None,
        OnFailure::Retry => (from, None, None),
        OnFailure::End => (END_NODE, None, None),
        OnFailure::NewRound => (NEW_ROUND_NODE, None, Some(ENTRY_NODE.to_owned())),
        OnFailure::NewRoundFrom(entry) => (NEW_ROUND_NODE, None, Some(entry.clone())),
        OnFailure::Jump { stage, continue_session } => (stage.as_str(), continue_session.then_some(crate::domain::SessionMode::Continue), None),
    };
    Some(EdgeDsl { from: from.into(), to: to.into(), on: EdgeOutcome::Failure, session, new_round_entry })
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
/// 编译顺序：先 `ForgeSpec::validate_all`（收敛完整性），再确定性映射，
/// 最后 `validate_authoring_workflow`（结构合法性）。返回**全部**结构化
/// 错误（修正轮按清单一次修复）；[`compile`] 是只取首个错误的快捷方式。
pub fn compile_reporting_all(
    spec: &ForgeSpec,
) -> Result<Vec<CompiledWorkflow>, Vec<ForgeError>> {
    // 两层问题都收集：Spec 层未收敛不阻止逐工作流编译，DSL 层的问题同轮报出。
    let mut errors = spec.validate_all();
    let mut results = Vec::new();
    for workflow in &spec.workflows {
        let compiled = match compile_one(workflow) {
            Ok(compiled) => compiled,
            Err(error) => {
                push_unique(&mut errors, error);
                continue;
            }
        };
        // 模板显示名：单工作流 = Spec 顶层名；拆分 = 各工作流名。
        let name = if spec.workflows.len() == 1 {
            spec.name.clone().unwrap_or_else(|| workflow.name.clone())
        } else {
            workflow.name.clone()
        };
        match crate::dsl::validate_authoring_workflow(compiled.workflow.clone()) {
            Ok(_) => results.push(CompiledWorkflow {
                name,
                ..compiled
            }),
            Err(error) => push_unique(
                &mut errors,
                ForgeError::CompileFailed {
                    detail: format!("workflow `{}`: {error}", workflow.name),
                },
            ),
        }
    }
    if errors.is_empty() {
        Ok(results)
    } else {
        Err(errors)
    }
}

/// 追加错误，同一问题（Spec 层与编译层各报一次）只保留一条。
fn push_unique(errors: &mut Vec<ForgeError>, error: ForgeError) {
    if !errors.contains(&error) {
        errors.push(error);
    }
}

/// 编译入口（只回报首个错误）。
pub fn compile(spec: &ForgeSpec) -> Result<Vec<CompiledWorkflow>, ForgeError> {
    compile_reporting_all(spec).map_err(|mut errors| errors.swap_remove(0))
}

/// 校验 Spec 可编译性但不产出（前端"预检"用）。
pub fn precheck(spec: &ForgeSpec) -> Result<(), ForgeError> {
    compile(spec).map(|_| ())
}
