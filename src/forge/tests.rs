//! forge 模块集成测试：编译器表驱动用例 + 产物强制过 DSL 校验（§9.1）。

use crate::dsl::{
    EdgeOutcome, NodeDsl, END_NODE, ENTRY_NODE, NEW_ROUND_NODE,
};
use crate::forge::compiler::{compile, precheck};
use crate::forge::spec::{
    ForgeSpec, ForgeStage, ForgeWorkflow, OnFailure, StageOutputKind, StageRole,
    MAX_STAGES_PER_WORKFLOW, SPEC_VERSION,
};

fn base_goal() -> String {
    "<输入>需求与上一阶段产物\n<产出>通过测试的代码\n<边界>不做数据库迁移\n<验收>全部测试通过"
        .to_string()
}

fn stage(role: StageRole) -> ForgeStage {
    ForgeStage {
        id: None,
        role,
        goal: base_goal(),
        artifact: Some("test-result".to_string()),
        output_kind: StageOutputKind::Json,
        schema_hint: Some(serde_json::json!({ "passed": "boolean" })),
        success_condition: Some("$.passed == true".to_string()),
        on_failure: OnFailure::Retry,
        manual_check: false,
    }
}

fn spec_with_stages(stages: Vec<ForgeStage>) -> ForgeSpec {
    ForgeSpec {
        spec_version: SPEC_VERSION.to_string(),
        name: Some("开发测试流程".to_string()),
        intent: "从需求到验收".to_string(),
        source_skills: vec![],
        workflows: vec![ForgeWorkflow {
            name: "开发测试流程".to_string(),
            stages,
        }],
        open_questions: vec![],
    }
}

#[test]
fn compiles_three_stage_linear_workflow() {
    let spec = spec_with_stages(vec![
        stage(StageRole::Dev),
        stage(StageRole::Test),
        stage(StageRole::Accept),
    ]);
    let results = compile(&spec).unwrap();
    assert_eq!(results.len(), 1);
    let dsl = &results[0].workflow;
    assert_eq!(dsl.nodes.len(), 3);
    assert_eq!(dsl.entry, dsl.nodes[0].id());
    // profile 选型：DSL 引用内置 profile 完整 id
    assert_eq!(dsl.nodes[0].profile(), Some("pf-builtin-dev"));
    assert_eq!(dsl.nodes[1].profile(), Some("pf-builtin-test"));
    assert_eq!(dsl.nodes[2].profile(), Some("pf-builtin-accept"));
    // 两条线性 success 边 + 末节点 $end
    let success_edges: Vec<_> = dsl
        .edges
        .iter()
        .filter(|e| e.on == EdgeOutcome::Success)
        .collect();
    assert_eq!(success_edges.len(), 3);
    assert!(success_edges.iter().any(|e| e.to == END_NODE));
    // control 默认值
    assert_eq!(dsl.control.max_attempts, Some(3));
    assert_eq!(dsl.control.max_rounds, Some(2));
    // 作者态 provider/model 为空（走模型绑定解耦）
    assert!(matches!(&dsl.nodes[0], NodeDsl::Worker(w) if w.provider.is_none() && w.model.is_none()));
}

#[test]
fn every_output_passes_authoring_validation() {
    // 全用例产物统一断言过 ValidatedWorkflow（authoring 模式）
    let specs = vec![
        spec_with_stages(vec![stage(StageRole::Dev)]),
        spec_with_stages((0..MAX_STAGES_PER_WORKFLOW).map(|_| stage(StageRole::Test)).collect()),
    ];
    for spec in specs {
        for result in compile(&spec).unwrap() {
            crate::dsl::validate_authoring_workflow(result.workflow).unwrap();
        }
    }
}

#[test]
fn on_failure_new_round_edge() {
    let spec = spec_with_stages(vec![stage(StageRole::Test)]);
    let dsl = &compile(&spec).unwrap()[0].workflow;
    let failure_edges: Vec<_> = dsl.edges.iter().filter(|e| e.on == EdgeOutcome::Failure).collect();
    // Retry 不产生 failure 边
    assert!(failure_edges.is_empty());

    let mut s = stage(StageRole::Test);
    s.on_failure = OnFailure::NewRound;
    let dsl = &compile(&spec_with_stages(vec![s])).unwrap()[0].workflow;
    let failure: Vec<_> = dsl.edges.iter().filter(|e| e.on == EdgeOutcome::Failure).collect();
    assert_eq!(failure.len(), 1);
    assert_eq!(failure[0].to, NEW_ROUND_NODE);
    assert_eq!(failure[0].new_round_entry.as_deref(), Some(ENTRY_NODE));
}

#[test]
fn on_failure_end_edge() {
    let mut s = stage(StageRole::Review);
    s.on_failure = OnFailure::End;
    let dsl = &compile(&spec_with_stages(vec![s])).unwrap()[0].workflow;
    let failure: Vec<_> = dsl
        .edges
        .iter()
        .filter(|e| e.on == EdgeOutcome::Failure && e.to == END_NODE)
        .collect();
    assert_eq!(failure.len(), 1);
}

#[test]
fn on_failure_manual_check_strips_output_contract() {
    // manual_check 与 output 验证互斥：编译器剥离 output/success_condition
    let mut s = stage(StageRole::Dev);
    s.on_failure = OnFailure::ManualCheck;
    let dsl = &compile(&spec_with_stages(vec![s])).unwrap()[0].workflow;
    match &dsl.nodes[0] {
        NodeDsl::Worker(w) => {
            assert!(w.manual_check.unwrap_or(false));
            assert!(w.output.is_none());
            assert!(w.success_condition.is_none());
        }
        other => panic!("expected worker node got {other:?}"),
    }
}

#[test]
fn manual_check_flag_also_strips_output() {
    let mut s = stage(StageRole::Dev);
    s.manual_check = true;
    let dsl = &compile(&spec_with_stages(vec![s])).unwrap()[0].workflow;
    match &dsl.nodes[0] {
        NodeDsl::Worker(w) => {
            assert!(w.manual_check.unwrap_or(false));
            assert!(w.output.is_none());
        }
        other => panic!("expected worker node got {other:?}"),
    }
}

#[test]
fn markdown_output_has_no_contract() {
    let mut s = stage(StageRole::Review);
    s.output_kind = StageOutputKind::Markdown;
    s.success_condition = None;
    let dsl = &compile(&spec_with_stages(vec![s])).unwrap()[0].workflow;
    match &dsl.nodes[0] {
        NodeDsl::Worker(w) => {
            assert!(w.output.is_none());
            assert!(w.success_condition.is_none());
        }
        other => panic!("expected worker node got {other:?}"),
    }
}

#[test]
fn custom_role_yields_pending_profile() {
    let mut s = stage(StageRole::Custom {
        profile_name: "安全审计".to_string(),
    });
    s.role = StageRole::Custom {
        profile_name: "安全审计".to_string(),
    };
    let results = compile(&spec_with_stages(vec![s])).unwrap();
    assert_eq!(results[0].pending_profiles.len(), 1);
    let (_, name, goal) = &results[0].pending_profiles[0];
    assert_eq!(name, "安全审计");
    assert!(goal.contains("<输入>"));
}

#[test]
fn split_spec_compiles_multiple_workflows() {
    let spec = ForgeSpec {
        spec_version: SPEC_VERSION.to_string(),
        name: None, // 多工作流时顶层名可缺省
        intent: "两组独立流程".to_string(),
        source_skills: vec![],
        workflows: vec![
            ForgeWorkflow {
                name: "流程甲".to_string(),
                stages: vec![stage(StageRole::Dev)],
            },
            ForgeWorkflow {
                name: "流程乙".to_string(),
                stages: vec![stage(StageRole::Review)],
            },
        ],
        open_questions: vec![],
    };
    let results = compile(&spec).unwrap();
    assert_eq!(results.len(), 2);
    assert_ne!(results[0].workflow.id, results[1].workflow.id);
}

#[test]
fn unresolved_questions_rejected_before_compile() {
    let mut spec = spec_with_stages(vec![stage(StageRole::Dev)]);
    spec.open_questions = vec!["未定".to_string()];
    let error = compile(&spec).unwrap_err();
    assert_eq!(error.code(), "forge/unresolved-questions");
}

#[test]
fn invalid_success_condition_path_fails_compile() {
    // path 不在 schema 中 → DSL 校验拦截 → forge/compile-failed
    let mut s = stage(StageRole::Test);
    s.success_condition = Some("$.not_declared == true".to_string());
    let error = compile(&spec_with_stages(vec![s])).unwrap_err();
    assert_eq!(error.code(), "forge/compile-failed");
    assert!(error.to_string().contains("not declared in output schema"));
}

#[test]
fn json_schema_output_rejected_as_schema_hint() {
    // DSL 禁止 JSON Schema 形状（须用简化 output shape）
    let mut s = stage(StageRole::Test);
    s.schema_hint = Some(serde_json::json!({
        "type": "object",
        "properties": { "passed": { "type": "boolean" } }
    }));
    let error = compile(&spec_with_stages(vec![s])).unwrap_err();
    assert_eq!(error.code(), "forge/compile-failed");
}

#[test]
fn precheck_matches_compile_outcome() {
    let spec = spec_with_stages(vec![stage(StageRole::Dev)]);
    precheck(&spec).unwrap();
    let mut s = stage(StageRole::Test);
    s.success_condition = Some("$.not_declared == true".to_string());
    let bad = spec_with_stages(vec![s]);
    assert!(precheck(&bad).is_err());
}

#[test]
fn retry_stage_without_explicit_failure_edge_still_valid() {
    // 仅 Retry onFailure、无 failure 边的工作流仍满足 DSL 校验
    let spec = spec_with_stages(vec![stage(StageRole::Dev), stage(StageRole::Test)]);
    let dsl = &compile(&spec).unwrap()[0].workflow;
    crate::dsl::validate_authoring_workflow(dsl.clone()).unwrap();
}

#[test]
fn natural_prefix_goal_elements_pass_validation() {
    // 真机验收回归:模型常以「输入：…；产出：…」自然段落表达四要素
    // (无尖括号标记),语义等价不应被拒(2026-11-19 真机发现)。
    let spec = spec_with_stages(vec![ForgeStage {
        id: None,
        role: StageRole::Dev,
        goal: "输入：用户提供的文章原文；产出：中文 markdown 要点总结；边界：只提炼要点，不做事实真伪判断；验收：要点覆盖文章全部核心信息。".to_string(),
        artifact: Some("要点总结".to_string()),
        output_kind: StageOutputKind::Markdown,
        schema_hint: None,
        success_condition: None,
        on_failure: OnFailure::Retry,
        manual_check: false,
    }]);
    let result = crate::forge::compiler::compile(&spec);
    assert!(result.is_ok(), "natural-prefix goal must compile: {:?}", result.err().map(|e| e.to_string()));
}
