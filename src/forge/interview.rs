//! openQuestions 收敛判定与 Spec 增量修订合并（A4）。
//!
//! Forge 对话逐轮产出 Spec 增量（见 spec-protocol prompt：只输出本轮
//! 变更的 stage/字段），本模块把增量合并进既有 Spec：
//! - 同 stage（按 id 对齐）整体覆盖；
//! - 新 stage（未命中任何 id，含省略 id）追加，并由系统分配稳定 id；
//! - 删除标记（`openQuestions` 之外，增量中显式列出的 removedStageIds）；
//! - openQuestions 以增量为准整体替换（新一轮未收敛项列表）。
//!
//! 合并后的 Spec 保持增量生长约束（§3.1）：每轮只携带增量，全量状态
//! 由合并方维护，Spec 始终是唯一对话事实源。

use crate::forge::spec::{ForgeError, ForgeSpec, ForgeWorkflow, StageRole};

/// Spec 是否收敛完毕可进入编译（openQuestions 清空 + 校验通过）。
pub fn is_converged(spec: &ForgeSpec) -> bool {
    spec.is_ready_to_compile()
}

/// 一轮对话的 Spec 增量（与 spec-protocol prompt 的输出协议对齐）。
///
/// `spec_version`/`intent` 等顶层字段仅在增量给出时覆盖（Option 语义）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpecDelta {
    /// 本轮要删除的 stage id 列表。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_stage_ids: Vec<String>,
    /// 本轮给出的最新未收敛项（整体替换基线的 openQuestions）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_questions: Option<Vec<crate::forge::spec::ForgeQuestion>>,
    /// 收敛后的工作流名（单工作流时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// 增量 stage：按 id（或序号）对齐基线，未匹配则追加。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<crate::forge::spec::ForgeStage>,
    /// A complete topology replacement for the selected workflow; omission preserves it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_groups: Option<Vec<crate::forge::spec::ForgeParallelGroup>>,
    /// Existing workflows are selected only by ID; a name alone requests a new split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
}

impl SpecDelta {
    /// 解析模型输出的增量 JSON（非法 JSON → `forge/invalid-spec`）。
    ///
    /// 真实模型偶尔漏掉结尾的 `}` / `]`：围栏正常闭合、JSON 却没有闭合（serde 报
    /// EOF）。围栏闭合说明模型认为已经输出完，只差结尾括号——按括号配平补全是
    /// 确定性的，在这里补上；其它解析错误（截断在字符串里、括号不配对等）原样返回。
    pub fn parse(json: &str) -> Result<Self, ForgeError> {
        let invalid = |error: serde_json::Error| ForgeError::InvalidSpec {
            detail: error.to_string(),
        };
        match serde_json::from_str(json) {
            Ok(delta) => Ok(delta),
            Err(error) if error.classify() == serde_json::error::Category::Eof => {
                let Some(closed) = close_unbalanced_json(json) else {
                    return Err(invalid(error));
                };
                serde_json::from_str(&closed).map_err(invalid)
            }
            Err(error) => Err(invalid(error)),
        }
    }
}

/// 补全 JSON 文本末尾缺失的 `}` / `]`：只补字符串与转义之外尚未配平的括号。
/// 文本截断在字符串里、或括号不配对时返回 `None`（那种残缺无法确定原意）。
fn close_unbalanced_json(json: &str) -> Option<String> {
    let mut unclosed: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for character in json.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => unclosed.push('}'),
            '[' => unclosed.push(']'),
            '}' | ']' => {
                if unclosed.pop() != Some(character) {
                    return None;
                }
            }
            _ => {}
        }
    }
    if in_string || unclosed.is_empty() {
        return None;
    }
    let mut closed = json.trim_end().to_string();
    for closer in unclosed.into_iter().rev() {
        closed.push(closer);
    }
    Some(closed)
}

/// 把增量合并进基线 Spec，返回合并后的新 Spec（基线不被修改）。
///
/// 合并规则（§6 A4）：
/// - `removed_stage_ids` 命中的 stage 从目标工作流删除；
/// - 增量 stage 只按 `id` 对齐：命中则整体覆盖（增量 stage 是该 stage 的
///   完整新状态，字段级合并会产生“半新半旧”的混合态，违反 Spec 单一事实源
///   约束）；未命中（含省略 `id`）即新 stage，追加到工作流末尾。基线里每个
///   stage 都带系统 id（见 [`assign_stage_ids`]），模型据基线对齐，不按序号
///   猜测——序号对齐在“只输出变更 stage”的协议下会把修改落到错误的 stage 上；
/// - `open_questions` 整体替换；顶层 `name`/`intent` 覆盖；
/// - 角色以名字为身份：增量没带 `profileId` 时沿用基线值；改了角色名的 stage 如果
///   是原角色唯一的（或全体一起改名的）使用者就保留 `profileId`（给角色改名），
///   否则断开关联（只给这个 stage 换角色），见 [`reconcile_workflow_authority`]；
/// - 合并结果不在此校验（对话中间态允许未收敛，`is_converged` 判定
///   编译时机）。
pub fn merge_delta(base: &ForgeSpec, delta: &SpecDelta) -> Result<ForgeSpec, ForgeError> {
    let mut merged = base.clone();
    if base.workflows.is_empty() {
        return Err(ForgeError::EmptyStages {
            workflow: merged.name.clone().unwrap_or_default(),
        });
    }
    if let Some(name) = delta.name.clone().filter(|value| !value.trim().is_empty()) {
        merged.name = Some(name);
    }
    if let Some(intent) = delta.intent.clone().filter(|value| !value.trim().is_empty()) {
        merged.intent = intent;
    }
    // 单工作流的 Spec：顶层 name 就是这条工作流的名字。首轮骨架是一条占位的工作流，
    // 模型通常只给顶层 name，工作流名要跟着改——否则存档、基线与编译报错里都留着占位名。
    if merged.workflows.len() == 1 && delta.workflow_name.is_none() {
        if let Some(name) = merged.name.clone().filter(|value| !value.trim().is_empty()) {
            merged.workflows[0].name = name;
        }
    }
    if let Some(open_questions) = &delta.open_questions {
        merged.open_questions = open_questions.clone();
    }

    if delta.workflow_id.is_none() && delta.workflow_name.is_none() && delta.stages.is_empty() && delta.removed_stage_ids.is_empty() && delta.parallel_groups.is_none() {
        return Ok(merged);
    }
    let name = delta.workflow_name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let workflow = if let Some(id) = &delta.workflow_id {
        let target = merged.workflows.iter_mut().find(|w| w.id == *id).ok_or_else(|| ForgeError::InvalidSpec { detail: "unknown workflow id".into() })?;
        if let Some(name) = name { target.name = name.into(); }
        target
    } else if let Some(name) = name {
        if merged.workflows.iter().any(|w| w.name == name) { return Err(ForgeError::DuplicateWorkflowName { name: name.into() }); }
        merged.workflows.push(ForgeWorkflow { id: String::new(), name: name.into(), stages: vec![], parallel_groups: vec![] });
        merged.workflows.last_mut().unwrap()
    } else if merged.workflows.len() == 1 {
        &mut merged.workflows[0]
    } else {
        return Err(ForgeError::InvalidSpec { detail: "workflow id required for a multi-workflow draft".into() });
    };

    let baseline = workflow.clone();

    // 删除标记。
    if !delta.removed_stage_ids.is_empty() {
        let removed: std::collections::BTreeSet<&str> = delta
            .removed_stage_ids
            .iter()
            .map(String::as_str)
            .collect();
        workflow.stages.retain(|stage| {
            stage
                .id
                .as_deref()
                .map(|id| !removed.contains(id))
                .unwrap_or(true)
        });
    }

    // 增量 stage 合并：id 对齐 → 覆盖；其余（含省略 id）→ 新 stage 追加。
    for stage in &delta.stages {
        if let Some(existing) = stage.id.as_deref().and_then(|id| workflow.stages.iter_mut().find(|s| s.id.as_deref() == Some(id))) {
            *existing = stage.clone();
        } else {
            workflow.stages.push(stage.clone());
        }
    }
    if let Some(groups) = &delta.parallel_groups { workflow.parallel_groups.clone_from(groups); }
    workflow.validate_groups()?;
    reconcile_workflow_authority(Some(&baseline), workflow)?;
    Ok(merged)
}

/// Reconcile an untrusted authoring document against this workflow's own system baseline.
/// A name selects an existing persona *within this workflow*; it never selects a library entry.
/// Only a whole-group rename retains a binding under a previously unknown name.
pub fn reconcile_workflow_authority(baseline: Option<&ForgeWorkflow>, revised: &mut ForgeWorkflow) -> Result<(), ForgeError> {
    use std::collections::BTreeMap;
    let old = baseline.map_or(&[][..], |workflow| workflow.stages.as_slice());
    let by_id: BTreeMap<_, _> = old.iter().filter_map(|s| Some((s.id.as_deref()?, s))).collect();
    let by_name: BTreeMap<_, _> = old.iter().filter(|s| s.profile_id.is_some())
        .filter_map(|s| Some((s.custom_role_name()?, s))).collect();
    // For each authorized persona, record whether *all remaining* stages select the same
    // destination. This distinguishes a group rename from a split in one linear traversal.
    let mut destinations = BTreeMap::new();
    for next in &revised.stages {
        if let Some(id) = next.id.as_deref().and_then(|id| by_id.get(id)).and_then(|s| s.profile_id.as_deref()) {
            let name = next.custom_role_name();
            let destination = destinations.entry(id).or_insert(name);
            if *destination != name { *destination = None; }
        }
    }
    let mut assignments = Vec::with_capacity(revised.stages.len());
    for stage in &revised.stages {
        let previous = stage.id.as_deref().and_then(|id| by_id.get(id).copied());
        if stage.role == StageRole::Unassigned && previous.is_none_or(|s| s.role != StageRole::Unassigned) {
            return Err(ForgeError::InvalidSpec { detail: "unassigned role must come from the original editor stage".into() });
        }
        if matches!(stage.goal, crate::forge::spec::ForgeGoal::Projected { .. })
            && previous.is_none_or(|s| s.goal != stage.goal) {
            return Err(ForgeError::InvalidSpec { detail: "projected goal does not match the baseline".into() });
        }
        let name = stage.custom_role_name();
        let named = name.and_then(|name| by_name.get(name).copied());
        let whole_group = previous.filter(|s| name.is_some() && s.profile_id.as_deref()
            .and_then(|id| destinations.get(id)).copied().flatten() == name);
        let authorized = named.or(whole_group);
        let binding = authorized.and_then(|s| s.profile_id.clone());
        // Joining another persona adopts its charter, not the previous stage's charter.
        let definition = if name.is_none() {
            None
        } else if named.is_some() && previous.and_then(|s| s.custom_role_name()) != name {
            named.and_then(|s| s.role_definition.clone())
        } else {
            stage.role_definition.clone().filter(|text| !text.trim().is_empty()).or_else(|| authorized.and_then(|s| s.role_definition.clone()))
        };
        if let Some(reference) = authorized.filter(|s| s.role_definition.is_none())
            && (reference.custom_role_name() != name || definition.is_some()) {
            return Err(ForgeError::ReadonlyRole {
                stage: stage.id.clone().unwrap_or_default(),
                role: reference.custom_role_name().unwrap_or_default().to_owned(),
            });
        }
        assignments.push((binding, definition));
    }
    for (stage, (id, definition)) in revised.stages.iter_mut().zip(assignments) {
        stage.profile_id = id;
        stage.role_definition = definition;
    }
    Ok(())
}

/// 为缺少可用 id 的 stage 分配系统 id（Spec 内全局唯一），返回是否有改动。
///
/// stage id 是增量的对齐身份，也是编译后的节点 id（节点按 id 与既有模板节点
/// 对齐，才能保留模型绑定等配置）。模型省略 id 或给出不可作节点 id 的值时由
/// 系统补齐；补齐后的基线随每轮消息注入，模型据此对齐。先出现的保留，
/// 重复出现的视为无效并重新分配。
pub fn assign_stage_ids(spec: &mut ForgeSpec) -> bool {
    // An earlier anonymous stage must not take a later worker's explicit ID (and thereby
    // silently take its slot, role or branch membership), or collide with a structural node.
    let reserved: std::collections::BTreeSet<String> = spec.workflows.iter().flat_map(|w|
        w.stages.iter().filter_map(|s| s.id.clone()).chain(w.parallel_groups.iter().map(|g| g.id.clone()))
    ).collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut changed = false;
    for workflow in &mut spec.workflows {
        for stage in &mut workflow.stages {
            let keep = stage
                .id
                .as_deref()
                .filter(|id| crate::forge::spec::stage_id_is_valid(id))
                .is_some_and(|id| seen.insert(id.to_string()));
            if keep {
                continue;
            }
            let prefix = system_stage_id_prefix(&stage.role);
            let mut ordinal = 1usize;
            let assigned = loop {
                let candidate = format!("{prefix}-{ordinal}");
                ordinal += 1;
                if !reserved.contains(&candidate) && seen.insert(candidate.clone()) {
                    break candidate;
                }
            };
            stage.id = Some(assigned);
            changed = true;
        }
    }
    changed
}

/// 系统 stage id 的角色前缀：`<角色>-<该角色内的序号>`（如 `test-1`、`custom-2`）。
///
/// 节点 id 就是工作流编辑器画布上的节点名，按角色命名比纯序号可读。前缀只在
/// 分配那一刻取自角色，之后角色变了 id 也不变——id 是稳定身份，不是显示名。
fn system_stage_id_prefix(role: &StageRole) -> &'static str {
    match role {
        StageRole::Plan => "plan",
        StageRole::Dev => "dev",
        StageRole::Test => "test",
        StageRole::Review => "review",
        StageRole::Accept => "accept",
        StageRole::Unassigned => "stage",
        StageRole::Custom { .. } => "custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::spec::{
        ForgeQuestion, ForgeStage, OnFailure, StageOutputKind, StageRole, SPEC_VERSION,
    };

    fn base_goal() -> String {
        "<输入>需求\n<产出>代码\n<边界>不做迁移\n<验收>测试通过".to_string()
    }

    fn stage_with(id: Option<&str>, goal: &str) -> ForgeStage {
        ForgeStage {
            id: id.map(str::to_string),
            role: StageRole::Dev,

            role_definition: None,

            profile_id: None,
            goal: goal.into(),
            artifact: Some("result".to_string()),
            output_kind: StageOutputKind::Json,
            schema_hint: Some(serde_json::json!({ "ok": "boolean" })),
            success_condition: Some("$.ok == true".to_string()),
            on_failure: OnFailure::Retry,
            manual_check: false,
        }
    }

    fn base_spec() -> ForgeSpec {
        ForgeSpec {
            revision: 0,
            spec_version: SPEC_VERSION.to_string(),
            name: Some("基线".to_string()),
            intent: "意图".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                id: String::new(),
                parallel_groups: vec![],
                name: "基线".to_string(),
                stages: vec![
                    stage_with(Some("s1"), &base_goal()),
                    stage_with(Some("s2"), &base_goal()),
                ],
            }],
            open_questions: vec![ForgeQuestion::ask("q1")],
        }
    }

    #[test]
    fn group_only_delta_and_id_assignment_preserve_branch_ownership() {
        let mut base = base_spec();
        base.workflows[0].stages = vec![stage_with(None, &base_goal()), stage_with(Some("dev-1"), &base_goal()), stage_with(Some("ui"), &base_goal())];
        let delta = SpecDelta::parse(r#"{"parallelGroups":[{"id":"dev-2","branches":[{"id":"one","stages":["dev-1"]},{"id":"two","stages":["ui"]}],"maxParallel":1,"onFailure":"end"}]}"#).unwrap();
        let mut grouped = merge_delta(&base, &delta).unwrap();
        assign_stage_ids(&mut grouped);
        assert_eq!(grouped.workflows[0].stages[1].id.as_deref(), Some("dev-1"));
        assert_eq!(grouped.workflows[0].stages[0].id.as_deref(), Some("dev-3"));
        let unchanged = merge_delta(&grouped, &SpecDelta { stages: vec![stage_with(Some("ui"), &base_goal())], ..Default::default() }).unwrap();
        assert_eq!(unchanged.workflows[0].parallel_groups[0].branches[0].stages, ["dev-1"]);
        let cleared = merge_delta(&unchanged, &SpecDelta::parse(r#"{"parallelGroups":[]}"#).unwrap()).unwrap();
        assert!(cleared.workflows[0].parallel_groups.is_empty());
        assert!(base.workflows[0].parallel_groups.is_empty());
    }

    #[test]
    fn malformed_group_deltas_leave_the_baseline_untouched() {
        let base = base_spec();
        let group = serde_json::json!({"id":"gate","maxParallel":3,"branches":[{"id":"left","stages":["s1"]},{"id":"right","stages":["s2"]}],"onFailure":"end"});
        for (path, value) in [
            ("/maxParallel", serde_json::json!(0)), ("/maxParallel", serde_json::json!(6)),
            ("/branches", serde_json::json!([{ "id":"left", "stages":["s1"] }])),
            ("/branches/1/stages", serde_json::json!([])), ("/branches/1/stages", serde_json::json!(["s1"])),
            ("/branches/1/stages", serde_json::json!(["missing"])), ("/branches/1/id", serde_json::json!("left")),
            ("/id", serde_json::json!("s2")), ("/onFailure", serde_json::json!({"goto":"s1"})),
            ("/onFailure", serde_json::json!("manual-check")),
        ] {
            let mut bad = group.clone(); *bad.pointer_mut(path).unwrap() = value;
            let delta = SpecDelta::parse(&serde_json::json!({"parallelGroups":[bad]}).to_string()).unwrap();
            assert!(merge_delta(&base, &delta).is_err(), "{path}");
        }
        let delta = SpecDelta::parse(&serde_json::json!({"parallelGroups":[group]}).to_string()).unwrap();
        let valid = merge_delta(&base, &delta).unwrap();
        for policy in [OnFailure::Jump { stage: "s2".into(), continue_session: false }, OnFailure::ManualCheck, OnFailure::NewRound] {
            let mut s1 = valid.workflows[0].stages[0].clone();
            s1.on_failure = policy;
            assert!(merge_delta(&valid, &SpecDelta { stages: vec![s1], ..Default::default() }).is_err());
        }
        assert!(base.workflows[0].parallel_groups.is_empty());
    }

    #[test]
    fn same_stage_overwrites_by_id() {
        let delta = SpecDelta {
            stages: vec![stage_with(Some("s2"), "<输入>a\n<产出>b\n<边界>c\n<验收>d")],
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        assert_eq!(merged.workflows[0].stages.len(), 2);
        assert_eq!(
            merged.workflows[0].stages[1].goal,
            "<输入>a\n<产出>b\n<边界>c\n<验收>d".into()
        );
        // s1 不受影响。
        assert_eq!(merged.workflows[0].stages[0].goal, base_goal().into());
    }

    #[test]
    fn new_stage_appends() {
        let delta = SpecDelta {
            stages: vec![stage_with(Some("s3"), &base_goal())],
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        assert_eq!(merged.workflows[0].stages.len(), 3);
        assert_eq!(merged.workflows[0].stages[2].id.as_deref(), Some("s3"));
    }

    #[test]
    fn delta_without_profile_id_keeps_the_system_assigned_one() {
        let mut base = base_spec();
        base.workflows[0].stages[1] = custom_stage_with("s2", "reviewer", Some("pf-role"));
        let delta = SpecDelta {
            stages: vec![custom_stage_with("s2", "reviewer", None)],
            ..Default::default()
        };

        let merged = merge_delta(&base, &delta).unwrap();

        assert_eq!(
            merged.workflows[0].stages[1].profile_id.as_deref(),
            Some("pf-role")
        );
    }

    fn custom_stage_with(id: &str, name: &str, profile_id: Option<&str>) -> ForgeStage {
        let mut stage = stage_with(Some(id), &base_goal());
        stage.role = StageRole::Custom {
            profile_name: name.to_string(),
        };
        stage.profile_id = profile_id.map(str::to_string);
        stage.role_definition = Some("fixture charter".into());
        stage
    }

    #[test]
    fn an_editor_goal_can_only_be_carried_by_its_original_stage() {
        use crate::forge::spec::ForgeGoal;
        let mut base = base_spec();
        let imported = ForgeGoal::Projected { original: Some("  Arbitrary prose; no headings.  ".into()) };
        base.workflows[0].stages[0].goal = imported.clone();
        let mut original = base.workflows[0].stages[0].clone();
        original.on_failure = OnFailure::End;
        let change = |stage| SpecDelta { stages: vec![stage], ..Default::default() };
        let merged = merge_delta(&base, &change(original.clone())).unwrap();
        assert_eq!(merged.workflows[0].stages[0].goal, imported);
        original.id = Some("other".into());
        assert!(merge_delta(&base, &change(original)).is_err());
    }

    #[test]
    fn model_delta_cannot_assign_system_role_bindings() {
        let mut base = base_spec();
        base.workflows[0].stages = vec![custom_stage_with("known", "reviewer", Some("fixture-baseline"))];
        let delta = SpecDelta {
            stages: vec![custom_stage_with("known", "reviewer", Some("fixture-untrusted")),
                         custom_stage_with("new", "tester", Some("fixture-untrusted"))],
            ..Default::default()
        };
        let merged = merge_delta(&base, &delta).unwrap();
        let bindings: Vec<_> = merged.workflows[0].stages.iter().map(|stage| stage.profile_id.as_deref()).collect();
        assert_eq!(bindings, [Some("fixture-baseline"), None]);
    }

    #[test]
    fn renaming_the_role_for_all_of_its_stages_keeps_the_profile_id() {
        // 两个 stage 共用一个角色（同一个 profileId）。对话里给角色改名：模型把两个
        // stage 一起改——这是改名，profileId 保留，编译时原地改名。
        let mut base = base_spec();
        base.workflows[0].stages = vec![
            custom_stage_with("custom-1", "审计员", Some("pf-audit")),
            custom_stage_with("custom-2", "审计员", Some("pf-audit")),
        ];
        let delta = SpecDelta {
            stages: vec![
                custom_stage_with("custom-1", "质量守护者", Some("pf-audit")),
                custom_stage_with("custom-2", "质量守护者", Some("pf-audit")),
            ],
            ..Default::default()
        };

        let merged = merge_delta(&base, &delta).unwrap();

        let ids: Vec<_> = merged.workflows[0]
            .stages
            .iter()
            .map(|stage| stage.profile_id.clone())
            .collect();
        assert_eq!(ids, [Some("pf-audit".to_string()), Some("pf-audit".to_string())]);
    }

    #[test]
    fn switching_one_stage_to_another_role_detaches_it_from_the_old_role() {
        // 只给 custom-1 换角色（custom-2 还在用「审计员」）：custom-1 断开与原角色的关联，
        // 编译时按新名字另建角色；原角色与 custom-2 不受影响。模型漏带 profileId 也一样。
        let mut base = base_spec();
        base.workflows[0].stages = vec![
            custom_stage_with("custom-1", "审计员", Some("pf-audit")),
            custom_stage_with("custom-2", "审计员", Some("pf-audit")),
        ];
        let delta = SpecDelta {
            stages: vec![custom_stage_with("custom-1", "发布复核员", None)],
            ..Default::default()
        };

        let merged = merge_delta(&base, &delta).unwrap();

        assert_eq!(merged.workflows[0].stages[0].profile_id, None);
        assert_eq!(
            merged.workflows[0].stages[1].profile_id.as_deref(),
            Some("pf-audit")
        );
    }

    #[test]
    fn builtin_roles_carry_no_forged_profile() {
        // 阶段改用内置角色后不再带着原 custom 角色的 profileId：之后再换回 custom 时
        // 按新角色处理，而不是把原角色原地改掉。
        let mut base = base_spec();
        base.workflows[0].stages = vec![custom_stage_with("custom-1", "审计员", Some("pf-audit"))];
        let delta = SpecDelta {
            stages: vec![stage_with(Some("custom-1"), &base_goal())],
            ..Default::default()
        };

        let merged = merge_delta(&base, &delta).unwrap();

        assert_eq!(merged.workflows[0].stages[0].profile_id, None);
    }

    #[test]
    fn stage_without_id_is_a_new_stage_not_a_positional_overwrite() {
        // 协议约定“只输出本轮变更的 stage”：只改了 s2 却没带 id，不能按序号
        // 落到 s1 上（序号对齐会悄悄覆盖错误的 stage），而是作为新 stage 追加。
        let mut delta_stage = stage_with(None, "<输入>a\n<产出>b\n<边界>c\n<验收>d");
        delta_stage.artifact = Some("新产物".to_string());
        let delta = SpecDelta {
            stages: vec![delta_stage],
            ..Default::default()
        };

        let merged = merge_delta(&base_spec(), &delta).unwrap();

        assert_eq!(merged.workflows[0].stages.len(), 3);
        assert_eq!(merged.workflows[0].stages[0].goal, base_goal().into());
        assert_eq!(merged.workflows[0].stages[1].goal, base_goal().into());
        assert_eq!(
            merged.workflows[0].stages[2].artifact.as_deref(),
            Some("新产物")
        );
    }

    #[test]
    fn assign_stage_ids_names_new_stages_after_their_role() {
        let mut spec = base_spec();
        let with_role = |id: Option<&str>, role: StageRole| {
            let mut stage = stage_with(id, &base_goal());
            stage.role = role;
            stage
        };
        spec.workflows[0].stages = vec![
            // 模型给过的有效 id 保留。
            with_role(Some("s1"), StageRole::Dev),
            with_role(None, StageRole::Dev),
            // 不可作节点 id 的视为缺失。
            with_role(Some("阶段/一"), StageRole::Dev),
            with_role(None, StageRole::Custom { profile_name: "审计员".to_string() }),
            with_role(None, StageRole::Test),
            with_role(None, StageRole::Test),
            // 显式身份优先于前方待分配阶段，不能被匿名阶段占用。
            with_role(Some("test-2"), StageRole::Dev),
            with_role(Some("release-review"), StageRole::Review),
        ];

        assert!(assign_stage_ids(&mut spec));

        let ids: Vec<&str> = spec.workflows[0].stages
            .iter()
            .filter_map(|stage| stage.id.as_deref())
            .collect();
        // 节点 id 就是编辑器画布上的节点名：按角色加序号命名，比纯序号可读。
        // 序号按角色分别计，并避开已占用的 id。
        assert_eq!(
            ids,
            [
                "s1", "dev-1", "dev-2", "custom-1", "test-1", "test-3", "test-2",
                "release-review",
            ]
        );
        // 再次分配是幂等的。
        assert!(!assign_stage_ids(&mut spec));
    }

    #[test]
    fn assigned_ids_are_unique_across_workflows() {
        let mut spec = base_spec();
        spec.workflows.push(ForgeWorkflow {
            id: String::new(),
            parallel_groups: vec![],
            name: "流程乙".to_string(),
            stages: vec![stage_with(None, &base_goal()), stage_with(None, &base_goal())],
        });

        assign_stage_ids(&mut spec);

        let ids: Vec<&str> = spec.workflows[1]
            .stages
            .iter()
            .filter_map(|stage| stage.id.as_deref())
            .collect();
        assert_eq!(ids, ["dev-1", "dev-2"]);
    }

    #[test]
    fn top_level_name_names_the_single_workflow() {
        // 真机回归：首轮骨架是一条叫“未命名工作流”的空工作流，模型只给了顶层 name，
        // 工作流名没有跟着改——存档、基线与编译报错里都留着占位名。
        let skeleton = ForgeSpec {
            revision: 0,
            spec_version: crate::forge::SPEC_VERSION.to_string(),
            name: None,
            intent: String::new(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                id: String::new(),
                parallel_groups: vec![],
                name: "未命名工作流".to_string(),
                stages: vec![],
            }],
            open_questions: vec![],
        };
        let delta = SpecDelta {
            name: Some("发布前检查与审计".to_string()),
            intent: Some("发布前先核对再审计".to_string()),
            ..Default::default()
        };

        let merged = merge_delta(&skeleton, &delta).unwrap();

        assert_eq!(merged.name.as_deref(), Some("发布前检查与审计"));
        assert_eq!(merged.workflows[0].name, "发布前检查与审计");
        // 只带 workflowName 的增量不改名；顶层 name 之后变化时单工作流名跟随。
        let renamed = merge_delta(&merged, &SpecDelta { name: Some("发布闸门".to_string()), ..Default::default() }).unwrap();
        assert_eq!(renamed.workflows[0].name, "发布闸门");
        let explicit = SpecDelta { workflow_id: Some(renamed.workflows[0].id.clone()), workflow_name: Some("发布闸门".to_string()), ..Default::default() };
        assert_eq!(merge_delta(&renamed, &explicit).unwrap().workflows[0].name, "发布闸门");
        // 多工作流时各工作流保留自己的名字。
        let mut split_base = renamed.clone();
        split_base.workflows[0].name = "流程甲".to_string();
        split_base.workflows.push(ForgeWorkflow { id: "b".into(), name: "流程乙".to_string(), stages: vec![], parallel_groups: vec![] });
        let split = merge_delta(&split_base, &SpecDelta { name: Some("总览".to_string()), ..Default::default() }).unwrap();
        assert_eq!(split.workflows.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["流程甲", "流程乙"]);
    }

    #[test]
    fn removed_stage_ids_delete() {
        let delta = SpecDelta {
            removed_stage_ids: vec!["s1".to_string()],
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        assert_eq!(merged.workflows[0].stages.len(), 1);
        assert_eq!(merged.workflows[0].stages[0].id.as_deref(), Some("s2"));
    }

    #[test]
    fn open_questions_replaced_wholesale() {
        let delta = SpecDelta {
            open_questions: Some(vec![ForgeQuestion::ask("q2"), ForgeQuestion::ask("q3")]),
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        let questions: Vec<_> = merged
            .open_questions
            .iter()
            .map(|question| question.question.as_str())
            .collect();
        assert_eq!(questions, ["q2", "q3"]);
        assert!(!is_converged(&merged));

        let cleared = SpecDelta {
            open_questions: Some(vec![]),
            ..Default::default()
        };
        let merged = merge_delta(&merged, &cleared).unwrap();
        assert!(merged.open_questions.is_empty());
    }

    #[test]
    fn top_level_fields_overridden_when_present() {
        let delta = SpecDelta {
            name: Some("新名字".to_string()),
            intent: Some("新意图".to_string()),
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        assert_eq!(merged.name.as_deref(), Some("新名字"));
        assert_eq!(merged.intent, "新意图");
    }

    #[test]
    fn base_spec_untouched_after_merge() {
        let base = base_spec();
        let delta = SpecDelta {
            stages: vec![stage_with(Some("s1"), "<输入>a\n<产出>b\n<边界>c\n<验收>d")],
            ..Default::default()
        };
        merge_delta(&base, &delta).unwrap();
        assert_eq!(base.workflows[0].stages[0].goal, base_goal().into());
    }

    #[test]
    fn new_workflow_appended_for_split() {
        let delta = SpecDelta {
            workflow_name: Some("流程乙".to_string()),
            stages: vec![stage_with(Some("s9"), &base_goal())],
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        assert_eq!(merged.workflows.len(), 2);
        assert_eq!(merged.workflows[1].name, "流程乙");
        assert_eq!(merged.workflows[1].stages.len(), 1);
    }

    #[test]
    fn delta_parse_rejects_invalid_json() {
        let error = SpecDelta::parse("{ broken").unwrap_err();
        assert_eq!(error.code(), "forge/invalid-spec");
    }

    #[test]
    fn delta_parse_closes_brackets_the_model_left_unterminated() {
        let valid = serde_json::to_string(&SpecDelta {
            intent: Some("实现登录".to_string()),
            stages: vec![stage_with(None, &base_goal())],
            open_questions: Some(vec![]),
            ..Default::default()
        })
        .unwrap();

        // 真机回归：模型的围栏正常闭合，JSON 却漏了最外层的 `}`（serde 报 EOF）。
        // 围栏闭合说明它认为输出完了，只差结尾括号——按括号配平补全。
        let delta = SpecDelta::parse(valid.strip_suffix('}').unwrap()).unwrap();
        assert_eq!(delta.intent.as_deref(), Some("实现登录"));
        assert_eq!(delta.stages.len(), 1);
        // 漏掉两层结尾同样补全。
        let delta = SpecDelta::parse(valid.strip_suffix("]}").unwrap()).unwrap();
        assert_eq!(delta.stages.len(), 1);

        // 截断在字符串里、括号不配对、或完整值之后还有残缺内容：不猜测。
        assert!(SpecDelta::parse("{\"intent\": \"未闭合").is_err());
        assert!(SpecDelta::parse("{\"stages\": [}").is_err());
        assert!(SpecDelta::parse("{\"intent\": \"a\"} {").is_err());
        // 字符串里的括号与转义引号不参与配平。
        let with_brackets = "{\"intent\": \"含 } 与 [ 和 \\\" 的意图\"}";
        let delta = SpecDelta::parse(with_brackets.strip_suffix('}').unwrap()).unwrap();
        assert_eq!(delta.intent.as_deref(), Some("含 } 与 [ 和 \" 的意图"));
    }

    #[test]
    fn delta_parse_round_trip() {
        let delta = SpecDelta {
            removed_stage_ids: vec!["s1".to_string()],
            open_questions: Some(vec![]),
            name: Some("名".to_string()),
            intent: None,
            stages: vec![stage_with(None, &base_goal())],
            ..Default::default()
        };
        let json = serde_json::to_string(&delta).unwrap();
        assert!(json.contains("\"removedStageIds\""));
        assert!(json.contains("\"openQuestions\""));
        let parsed = SpecDelta::parse(&json).unwrap();
        assert_eq!(parsed.removed_stage_ids, delta.removed_stage_ids);
        assert_eq!(parsed.open_questions, delta.open_questions);
        assert_eq!(parsed.name, delta.name);
        assert_eq!(parsed.stages.len(), 1);
        assert_eq!(parsed.stages[0].goal, delta.stages[0].goal);
    }

    #[test]
    fn empty_base_workflows_rejected() {
        let mut base = base_spec();
        base.workflows.clear();
        let error = merge_delta(&base, &SpecDelta::default()).unwrap_err();
        assert_eq!(error.code(), "forge/empty-stages");
    }
}
