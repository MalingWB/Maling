//! Forge Spec（铸造草稿）数据模型与校验。
//!
//! Spec 是 Forge 对话收敛结论的结构化载体，也是编译为 `WorkflowDsl`
//! 的唯一输入（见 docs/gold-band/开发计划/工作流管理/工作流铸造实施方案.md §3）。
//! Spec 经 serde 反序列化，非法 JSON 直接报 `forge/invalid-spec`；
//! 必收敛字段（name、goal 四要素、successCondition）在此层校验，
//! 保证浅显或缺失的 stage 不会流入编译。

use serde::{Deserialize, Serialize};

/// Spec 格式版本。
pub const SPEC_VERSION: &str = "0.1";

/// 单条工作流的 stages 数量上限（D3/D4：典型 2~6，上限 10）。
pub const MAX_STAGES_PER_WORKFLOW: usize = 10;

/// Forge 结构化错误码。后端只返回错误码与结构化上下文，
/// 对客文案由前端按码处理。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", content = "context", rename_all = "kebab-case")]
pub enum ForgeError {
    /// JSON 反序列化失败或字段类型非法。
    #[error("invalid forge spec: {detail}")]
    InvalidSpec { detail: String },
    /// 单条工作流 stages 超过上限。
    #[error("too many stages: {count} > {limit}")]
    TooManyStages { workflow: String, count: usize, limit: usize },
    /// Spec 顶层 name 缺失或为空。
    #[error("missing forge spec name")]
    MissingName,
    /// stage 的 goal 缺失四要素（输入/产出/边界/验收）之一。
    #[error("stage `{stage}` goal missing element `{element}`")]
    MissingGoalElement { stage: String, element: String },
    /// stage 缺失成功条件。
    #[error("stage `{stage}` missing success condition")]
    MissingSuccessCondition { stage: String },
    /// stages 为空。
    #[error("workflow `{workflow}` has no stages")]
    EmptyStages { workflow: String },
    /// 编译产物未通过 WorkflowDsl 校验（附校验错误明细）。
    #[error("compile failed: {detail}")]
    CompileFailed { detail: String },
    /// 未收敛项仍存在，尚不可编译。
    #[error("spec has unresolved questions: {items}")]
    UnresolvedQuestions { items: String },
}

impl ForgeError {
    /// 错误码字符串（前端按此分支处理）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSpec { .. } => "forge/invalid-spec",
            Self::TooManyStages { .. } => "forge/too-many-stages",
            Self::MissingName => "forge/missing-name",
            Self::MissingGoalElement { .. } => "forge/missing-goal-element",
            Self::MissingSuccessCondition { .. } => "forge/missing-success-condition",
            Self::EmptyStages { .. } => "forge/empty-stages",
            Self::CompileFailed { .. } => "forge/compile-failed",
            Self::UnresolvedQuestions { .. } => "forge/unresolved-questions",
        }
    }
}

impl From<serde_json::Error> for ForgeError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidSpec {
            detail: error.to_string(),
        }
    }
}

/// goal 四要素标记：Forge 对话的必收敛项（§3.1）。
/// goal 文本需覆盖输入、产出、边界（不做什么）、验收（怎么判断完成）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GoalElement {
    Input,
    Output,
    Boundary,
    Acceptance,
}

impl GoalElement {
    pub const ALL: [GoalElement; 4] = [
        GoalElement::Input,
        GoalElement::Output,
        GoalElement::Boundary,
        GoalElement::Acceptance,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Boundary => "boundary",
            Self::Acceptance => "acceptance",
        }
    }

    /// goal 段落的中文要素名(行首前缀形式检测用)。
    pub fn chinese_name(&self) -> &'static str {
        match self {
            Self::Input => "输入",
            Self::Output => "产出",
            Self::Boundary => "边界",
            Self::Acceptance => "验收",
        }
    }
}

/// stage 的失败策略（§3.2 边映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OnFailure {
    /// 同节点重试（不产生额外边，由 control.max_attempts 兜住）。
    Retry,
    /// 人工复核：manual_check = true（与 output 验证互斥，见编译规则）。
    ManualCheck,
    /// 失败开启新一轮：边指向 `$new-round`，重入 entry。
    NewRound,
    /// 失败即终止：边指向 `$end`。
    End,
}

/// stage 角色：决定编译时的内置 profile 选型；
/// `custom` 时以 goal 生成 User scope 新 profile。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageRole {
    Dev,
    Test,
    Review,
    Plan,
    Accept,
    #[serde(rename_all = "camelCase")]
    Custom { profile_name: String },
}

/// 产物种类。`OutputKind::Json` 为 DSL 唯一受支持的 output 契约，
/// markdown 与 none 映射为不设置 output/success_condition。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageOutputKind {
    Json,
    Markdown,
    None,
}

/// 单个阶段（编译为一个 worker 节点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeStage {
    /// stage 标识（Spec 内稳定，用于增量修订对齐；缺省按序生成）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub role: StageRole,
    /// 覆盖四要素的阶段目标（输入/产出/边界/验收）。
    pub goal: String,
    /// custom 角色的完整职责正文（角色的稳定人格：职责/方法/边界），
    /// 与 goal（本阶段任务）分离；编译时作为 User scope profile 的
    /// content。缺省回退 goal 正文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_definition: Option<String>,
    /// custom 角色已铸造的 profile id：一经创建即携带，重编译按 id
    /// 原地更新（名称/正文），不再新建；反投影时从工作流节点还原。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// 产物名（outputKind 为 none 时忽略）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    pub output_kind: StageOutputKind,
    /// 简化产物 schema 草案（非 JSON Schema，遵循 DSL 简化形状）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_hint: Option<serde_json::Value>,
    /// 成功条件表达式（如 `$.passed == true`，path 须存在于 schema_hint）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_condition: Option<String>,
    pub on_failure: OnFailure,
    #[serde(default)]
    pub manual_check: bool,
}

/// 单条工作流（拆分时 Spec 顶层含多个）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeWorkflow {
    /// 工作流显示名（必收敛字段）。
    pub name: String,
    pub stages: Vec<ForgeStage>,
}

/// Forge Spec 顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSpec {
    pub spec_version: String,
    /// Spec 顶层名（单工作流时必收敛；多工作流拆分时可缺省）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 一句话流程意图（试运行样例需求、锚定轮确认）。
    pub intent: String,
    /// 来源 skill 引用（第二阶段 skill 入口填充）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_skills: Vec<String>,
    pub workflows: Vec<ForgeWorkflow>,
    /// 未收敛项，驱动多轮对话；编译前必须为空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_questions: Vec<String>,
}

/// goal 四要素的最小证据检测。
///
/// Spec 输出协议要求 goal 以四段结构表达；此函数检测结构标记而非语义
/// （语义深浅由 Forge 对话负责，结构缺失在此拦截）。每要素接受两种等价
/// 引导形式：协议示例的 `<输入>` 标记，或行首 `输入：`/`输入:` 前缀
/// （模型常以自然段落表达,语义等价不应被拒）。
fn missing_goal_elements(goal: &str) -> Vec<GoalElement> {
    GoalElement::ALL
        .into_iter()
        .filter(|element| !goal_has_element(goal, *element))
        .collect()
}

fn goal_has_element(goal: &str, element: GoalElement) -> bool {
    if goal.contains(goal_element_marker(&element)) {
        return true;
    }
    // 行首(或分号后)的「要素名：/要素名:」前缀,容忍前后空白。
    let name = element.chinese_name();
    for separator in ["：", ":"] {
        let needle = format!("{name}{separator}");
        let mut search_from = 0usize;
        while let Some(offset) = goal[search_from..].find(&needle) {
            let absolute = search_from + offset;
            let at_element_start = goal[..absolute]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || c == ';' || c == '；');
            if at_element_start {
                return true;
            }
            search_from = absolute + needle.len();
        }
    }
    false
}

fn goal_element_marker(element: &GoalElement) -> &'static str {
    match element {
        GoalElement::Input => "<输入>",
        GoalElement::Output => "<产出>",
        GoalElement::Boundary => "<边界>",
        GoalElement::Acceptance => "<验收>",
    }
}

impl ForgeSpec {
    /// 从 JSON 文本解析并做全量收敛校验。
    pub fn parse_and_validate(json: &str) -> Result<Self, ForgeError> {
        let spec: ForgeSpec = serde_json::from_str(json)?;
        spec.validate()?;
        Ok(spec)
    }

    /// 全量校验：必收敛字段、stages 上限、四要素、openQuestions 清空。
    pub fn validate(&self) -> Result<(), ForgeError> {
        if self.workflows.is_empty() {
            return Err(ForgeError::EmptyStages {
                workflow: self.name.clone().unwrap_or_default(),
            });
        }
        if self.workflows.len() == 1 && is_blank(self.name.as_deref()) {
            return Err(ForgeError::MissingName);
        }
        for workflow in &self.workflows {
            if is_blank(Some(&workflow.name)) {
                return Err(ForgeError::MissingName);
            }
            if workflow.stages.is_empty() {
                return Err(ForgeError::EmptyStages {
                    workflow: workflow.name.clone(),
                });
            }
            if workflow.stages.len() > MAX_STAGES_PER_WORKFLOW {
                return Err(ForgeError::TooManyStages {
                    workflow: workflow.name.clone(),
                    count: workflow.stages.len(),
                    limit: MAX_STAGES_PER_WORKFLOW,
                });
            }
            for (index, stage) in workflow.stages.iter().enumerate() {
                let stage_id = stage
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("#{}", index + 1));
                if is_blank(Some(&stage.goal)) {
                    return Err(ForgeError::MissingGoalElement {
                        stage: stage_id,
                        element: "goal".to_string(),
                    });
                }
                if let Some(element) = missing_goal_elements(&stage.goal).first().copied() {
                    return Err(ForgeError::MissingGoalElement {
                        stage: stage_id.clone(),
                        element: element.as_str().to_string(),
                    });
                }
                match stage.output_kind {
                    StageOutputKind::Json => {
                        if is_blank(stage.success_condition.as_deref()) {
                            return Err(ForgeError::MissingSuccessCondition { stage: stage_id });
                        }
                    }
                    StageOutputKind::Markdown | StageOutputKind::None => {}
                }
            }
        }
        if !self.open_questions.is_empty() {
            return Err(ForgeError::UnresolvedQuestions {
                items: self.open_questions.join("; "),
            });
        }
        Ok(())
    }

    /// 是否已达可编译状态（openQuestions 清空 + 全部校验通过）。
    pub fn is_ready_to_compile(&self) -> bool {
        self.open_questions.is_empty() && self.validate().is_ok()
    }
}

fn is_blank(value: Option<&str>) -> bool {
    value.map(str::trim).map(str::is_empty).unwrap_or(true)
}

/// 导出 Spec 的 JSON Schema 片段，供 spec-protocol prompt 与前端校验引用。
/// 常量内嵌（无运行时 schema 库依赖），保持与 `ForgeSpec` 手工同步，
/// 由 `tests.rs` 断言关键字段与 serde 序列化形状一致。
pub fn spec_json_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ForgeSpec",
        "type": "object",
        "required": ["specVersion", "intent", "workflows"],
        "properties": {
            "specVersion": { "const": SPEC_VERSION },
            "name": { "type": "string" },
            "intent": { "type": "string", "minLength": 1 },
            "sourceSkills": { "type": "array", "items": { "type": "string" } },
            "workflows": {
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "object",
                    "required": ["name", "stages"],
                    "properties": {
                        "name": { "type": "string", "minLength": 1 },
                        "stages": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_STAGES_PER_WORKFLOW,
                            "items": { "$ref": "#/$defs/stage" }
                        }
                    }
                }
            },
            "openQuestions": { "type": "array", "items": { "type": "string" } }
        },
        "$defs": {
            "stage": {
                "type": "object",
                "required": ["role", "goal", "outputKind", "onFailure"],
                "properties": {
                    "id": { "type": "string" },
                    "role": {
                        "oneOf": [
                            { "enum": ["dev", "test", "review", "plan", "accept"] },
                            {
                                "type": "object",
                                "required": ["custom"],
                                "properties": {
                                    "custom": { "type": "object", "required": ["profileName"], "properties": { "profileName": { "type": "string" } } }
                                }
                            }
                        ]
                    },
                    "goal": { "type": "string", "minLength": 1 },
                    "roleDefinition": { "type": "string", "description": "custom 角色的完整职责正文(职责/方法/边界),与 goal 分离;缺省回退 goal" },
                    "profileId": { "type": "string", "description": "已铸造角色的 profile id;由系统回填,模型原样携带,不得新建或篡改" },
                    "artifact": { "type": "string" },
                    "outputKind": { "enum": ["json", "markdown", "none"] },
                    "schemaHint": {},
                    "successCondition": { "type": "string" },
                    "onFailure": { "enum": ["retry", "manual-check", "new-round", "end"] },
                    "manualCheck": { "type": "boolean" }
                }
            }
        }
    })
}

#[cfg(test)]
mod spec_tests {
    use super::*;

    fn base_goal() -> String {
        "<输入>需求与上一阶段产物\n<产出>通过测试的代码\n<边界>不做数据库迁移\n<验收>全部测试通过"
            .to_string()
    }

    fn stage(role: StageRole, goal: String) -> ForgeStage {
        ForgeStage {
            id: None,
            role,
            role_definition: None,
            profile_id: None,
            goal,
            artifact: Some("test-result".to_string()),
            output_kind: StageOutputKind::Json,
            schema_hint: Some(serde_json::json!({ "passed": "boolean" })),
            success_condition: Some("$.passed == true".to_string()),
            on_failure: OnFailure::Retry,
            manual_check: false,
        }
    }

    fn workflow_with(stages: Vec<ForgeStage>) -> ForgeWorkflow {
        ForgeWorkflow {
            name: "流程".to_string(),
            stages,
        }
    }

    fn spec_with(workflows: Vec<ForgeWorkflow>) -> ForgeSpec {
        ForgeSpec {
            spec_version: SPEC_VERSION.to_string(),
            name: Some("总名".to_string()),
            intent: "意图".to_string(),
            source_skills: vec![],
            workflows,
            open_questions: vec![],
        }
    }

    fn valid_spec() -> ForgeSpec {
        spec_with(vec![workflow_with(vec![stage(
            StageRole::Dev,
            base_goal(),
        )])])
    }

    #[test]
    fn valid_spec_passes_validation() {
        valid_spec().validate().unwrap();
    }

    #[test]
    fn parse_rejects_invalid_json() {
        let error = ForgeSpec::parse_and_validate("{ not json").unwrap_err();
        assert_eq!(error.code(), "forge/invalid-spec");
    }

    #[test]
    fn empty_workflows_rejected() {
        let error = spec_with(vec![]).validate().unwrap_err();
        assert!(matches!(error, ForgeError::EmptyStages { .. }));
        assert_eq!(error.code(), "forge/empty-stages");
    }

    #[test]
    fn missing_top_level_name_rejected_for_single_workflow() {
        let mut spec = valid_spec();
        spec.name = None;
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code(), "forge/missing-name");
    }

    #[test]
    fn missing_workflow_name_rejected() {
        let spec = spec_with(vec![ForgeWorkflow {
            name: "  ".to_string(),
            stages: vec![stage(StageRole::Dev, base_goal())],
        }]);
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code(), "forge/missing-name");
    }

    #[test]
    fn empty_stages_rejected() {
        let spec = spec_with(vec![workflow_with(vec![])]);
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code(), "forge/empty-stages");
    }

    #[test]
    fn too_many_stages_rejected() {
        let stages: Vec<_> = (0..MAX_STAGES_PER_WORKFLOW + 1)
            .map(|_| stage(StageRole::Test, base_goal()))
            .collect();
        let spec = spec_with(vec![workflow_with(stages)]);
        let error = spec.validate().unwrap_err();
        assert!(matches!(
            error,
            ForgeError::TooManyStages { count: 11, limit: 10, .. }
        ));
        assert_eq!(error.code(), "forge/too-many-stages");
    }

    #[test]
    fn stage_limit_boundary_passes() {
        let stages: Vec<_> = (0..MAX_STAGES_PER_WORKFLOW)
            .map(|_| stage(StageRole::Test, base_goal()))
            .collect();
        spec_with(vec![workflow_with(stages)]).validate().unwrap();
    }

    #[test]
    fn goal_missing_each_element_reports_that_element() {
        for element in GoalElement::ALL {
            let goal = GoalElement::ALL
                .into_iter()
                .filter(|candidate| candidate != &element)
                .map(|candidate| goal_element_marker(&candidate))
                .collect::<Vec<_>>()
                .join("\n");
            let error = spec_with(vec![workflow_with(vec![stage(StageRole::Dev, goal)])])
                .validate()
                .unwrap_err();
            assert!(
                matches!(
                    &error,
                    ForgeError::MissingGoalElement { element: e, .. } if e == element.as_str()
                ),
                "expected missing element {} got {error:?}",
                element.as_str()
            );
            assert_eq!(error.code(), "forge/missing-goal-element");
        }
    }

    #[test]
    fn blank_goal_rejected() {
        let error = spec_with(vec![workflow_with(vec![stage(
            StageRole::Dev,
            "   ".to_string(),
        )])])
        .validate()
        .unwrap_err();
        assert_eq!(error.code(), "forge/missing-goal-element");
    }

    #[test]
    fn json_output_requires_success_condition() {
        let mut s = stage(StageRole::Test, base_goal());
        s.success_condition = None;
        let error = spec_with(vec![workflow_with(vec![s])])
            .validate()
            .unwrap_err();
        assert_eq!(error.code(), "forge/missing-success-condition");
    }

    #[test]
    fn markdown_output_allows_missing_success_condition() {
        let mut s = stage(StageRole::Review, base_goal());
        s.output_kind = StageOutputKind::Markdown;
        s.success_condition = None;
        spec_with(vec![workflow_with(vec![s])]).validate().unwrap();
    }

    #[test]
    fn open_questions_block_validation() {
        let mut spec = valid_spec();
        spec.open_questions = vec!["产物 schema 未定".to_string()];
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code(), "forge/unresolved-questions");
        assert!(!spec.is_ready_to_compile());
        spec.open_questions.clear();
        assert!(spec.is_ready_to_compile());
    }

    #[test]
    fn serde_camel_case_round_trip() {
        let mut spec = valid_spec();
        // skip_serializing_if 字段需非空才参与序列化，验证 camelCase 键名。
        spec.source_skills = vec!["code-review".to_string()];
        spec.open_questions = vec![];
        let json = serde_json::to_string(&spec).unwrap();
        assert!(json.contains("\"specVersion\""));
        assert!(json.contains("\"sourceSkills\""));
        let parsed: ForgeSpec = serde_json::from_str(&json).unwrap();
        parsed.validate().unwrap();
    }

    #[test]
    fn role_custom_serializes_kebab_with_profile_name() {
        let s = stage(
            StageRole::Custom {
                profile_name: "我的角色".to_string(),
            },
            base_goal(),
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"custom\""));
        assert!(json.contains("\"profileName\""));
    }

    #[test]
    fn exported_schema_matches_serde_shape() {
        let schema = spec_json_schema();
        assert_eq!(schema["properties"]["specVersion"]["const"], SPEC_VERSION);
        let stages_schema = &schema["properties"]["workflows"]["items"]["properties"]["stages"];
        assert_eq!(stages_schema["maxItems"], MAX_STAGES_PER_WORKFLOW);
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "intent"));
        assert!(serde_json::to_string(&OnFailure::ManualCheck)
            .unwrap()
            .contains("manual-check"));
        assert!(serde_json::to_string(&StageOutputKind::Json)
            .unwrap()
            .contains("json"));
        let role_enum = &schema["$defs"]["stage"]["properties"]["role"]["oneOf"][0]["enum"];
        assert!(role_enum
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("dev")));
    }
}
