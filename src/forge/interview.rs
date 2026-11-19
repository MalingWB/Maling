//! openQuestions 收敛判定与 Spec 增量修订合并（A4）。
//!
//! Forge 对话逐轮产出 Spec 增量（见 spec-protocol prompt：只输出本轮
//! 变更的 stage/字段），本模块把增量合并进既有 Spec：
//! - 同 stage（按 id 对齐，无 id 时按序号）覆盖字段；
//! - 新 stage 追加；
//! - 删除标记（`openQuestions` 之外，增量中显式列出的 removedStageIds）；
//! - openQuestions 以增量为准整体替换（新一轮未收敛项列表）。
//!
//! 合并后的 Spec 保持增量生长约束（§3.1）：每轮只携带增量，全量状态
//! 由合并方维护，Spec 始终是唯一对话事实源。

use crate::forge::spec::{ForgeError, ForgeSpec, ForgeWorkflow};

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
    pub open_questions: Option<Vec<String>>,
    /// 收敛后的工作流名（单工作流时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// 增量 stage：按 id（或序号）对齐基线，未匹配则追加。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<crate::forge::spec::ForgeStage>,
    /// 多工作流拆分时的目标工作流名；缺省合并进第一个工作流。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_name: Option<String>,
}

impl SpecDelta {
    /// 解析模型输出的增量 JSON（非法 JSON → `forge/invalid-spec`）。
    pub fn parse(json: &str) -> Result<Self, ForgeError> {
        serde_json::from_str(json).map_err(|error| ForgeError::InvalidSpec {
            detail: error.to_string(),
        })
    }
}

/// 把增量合并进基线 Spec，返回合并后的新 Spec（基线不被修改）。
///
/// 合并规则（§6 A4）：
/// - `removed_stage_ids` 命中的 stage 从目标工作流删除；
/// - 增量 stage 按 `id`（基线 stage 有 id）或序号对齐基线同位 stage，
///   命中则整体覆盖（增量 stage 是该 stage 的完整新状态，字段级合并
///   会产生“半新半旧”的混合态，违反 Spec 单一事实源约束）；
///   未命中则追加到工作流末尾；
/// - `open_questions` 整体替换；顶层 `name`/`intent` 覆盖；
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
    if let Some(open_questions) = &delta.open_questions {
        merged.open_questions = open_questions.clone();
    }

    // 目标工作流：按名匹配，缺省第一个。
    let workflow_name = delta
        .workflow_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let target_name = match workflow_name {
        Some(name) if merged.workflows.iter().any(|w| w.name == name) => name.to_string(),
        Some(name) => {
            // 新工作流（拆分）：追加空工作流承载增量 stages。
            merged.workflows.push(ForgeWorkflow {
                name: name.to_string(),
                stages: vec![],
            });
            name.to_string()
        }
        None => merged.workflows[0].name.clone(),
    };
    let workflow = merged
        .workflows
        .iter_mut()
        .find(|workflow| workflow.name == target_name)
        .expect("target workflow ensured above");

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

    // 增量 stage 合并：id 对齐 → 序号对齐 → 追加。
    for stage in &delta.stages {
        let matched_by_id = stage.id.as_deref().and_then(|id| {
            workflow
                .stages
                .iter()
                .position(|existing| existing.id.as_deref() == Some(id))
        });
        match matched_by_id {
            Some(index) => workflow.stages[index] = stage.clone(),
            None => {
                // 无 id 增量按序号对齐：第 N 个无 id 增量对应第 N 个无 id 基线。
                let ordinal = delta
                    .stages
                    .iter()
                    .position(|candidate| candidate.id.is_none() && std::ptr::eq(candidate, stage))
                    .unwrap_or(0);
                let position = if stage.id.is_none() {
                    workflow
                        .stages
                        .iter()
                        .enumerate()
                        .filter(|(_, existing)| existing.id.is_none())
                        .map(|(index, _)| index)
                        .nth(ordinal)
                } else {
                    None
                };
                match position {
                    Some(index) => workflow.stages[index] = stage.clone(),
                    None => workflow.stages.push(stage.clone()),
                }
            }
        }
    }

    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::spec::{ForgeStage, OnFailure, StageOutputKind, StageRole, SPEC_VERSION};

    fn base_goal() -> String {
        "<输入>需求\n<产出>代码\n<边界>不做迁移\n<验收>测试通过".to_string()
    }

    fn stage_with(id: Option<&str>, goal: &str) -> ForgeStage {
        ForgeStage {
            id: id.map(str::to_string),
            role: StageRole::Dev,

            role_definition: None,

            profile_id: None,
            goal: goal.to_string(),
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
            spec_version: SPEC_VERSION.to_string(),
            name: Some("基线".to_string()),
            intent: "意图".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                name: "基线".to_string(),
                stages: vec![
                    stage_with(Some("s1"), &base_goal()),
                    stage_with(Some("s2"), &base_goal()),
                ],
            }],
            open_questions: vec!["q1".to_string()],
        }
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
            "<输入>a\n<产出>b\n<边界>c\n<验收>d"
        );
        // s1 不受影响。
        assert_eq!(merged.workflows[0].stages[0].goal, base_goal());
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
            open_questions: Some(vec!["q2".to_string(), "q3".to_string()]),
            ..Default::default()
        };
        let merged = merge_delta(&base_spec(), &delta).unwrap();
        assert_eq!(merged.open_questions, vec!["q2".to_string(), "q3".to_string()]);
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
        assert_eq!(base.workflows[0].stages[0].goal, base_goal());
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
    fn delta_parse_round_trip() {
        let delta = SpecDelta {
            removed_stage_ids: vec!["s1".to_string()],
            open_questions: Some(vec![]),
            name: Some("名".to_string()),
            intent: None,
            stages: vec![stage_with(None, &base_goal())],
            workflow_name: None,
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
