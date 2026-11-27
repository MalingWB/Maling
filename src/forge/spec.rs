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

/// stage id 长度上限（与节点 id、attempt 目录名段共用）。
pub const MAX_STAGE_ID_LEN: usize = 64;

/// stage id 是否可直接作 DSL 节点 id：非空、不超长，且只含 ASCII 字母数字、
/// `-`、`_`（节点 id 会成为 attempt 目录名段，保留字 `$end` 等因 `$` 被排除）。
pub fn stage_id_is_valid(candidate: &str) -> bool {
    let length = candidate.chars().count();
    (1..=MAX_STAGE_ID_LEN).contains(&length)
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Forge 结构化错误码。后端只返回错误码与结构化上下文，
/// 对客文案由前端按码处理。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "code", content = "context", rename_all = "kebab-case")]
pub enum ForgeError {
    #[error("forge/turn-unconfirmed")]
    TurnUnconfirmed,
    #[error("forge/revision-conflict")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("forge/target-unavailable")]
    TargetUnavailable { workflow: String },
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
    /// 失败策略指向的阶段不在同一条工作流里。
    #[error("stage `{stage}` onFailure targets unknown stage `{target}`")]
    UnknownFailureTarget { stage: String, target: String },
    /// 同一个角色名对应了不同的已铸造角色（profileId 不一致）。
    #[error("role `{role}` is bound to different profile ids in stages {stages}")]
    RoleIdConflict { role: String, stages: String },
    /// 同一个角色名出现了不同的职责正文。
    #[error("role `{role}` has different roleDefinition in stages {stages}")]
    RoleDefinitionConflict { role: String, stages: String },
    #[error("stage `{stage}` references unavailable role `{role}`")]
    UnavailableRole { stage: String, role: String },
    /// Library roles are read-only references.
    #[error("stage `{stage}` changes read-only role `{role}`")]
    ReadonlyRole { stage: String, role: String },
    /// 工作流名已被另一个工作流模板占用（工作流名唯一）。
    #[error("workflow name `{name}` is already used by another workflow template")]
    DuplicateWorkflowName { name: String },
}

impl ForgeError {
    /// 错误码字符串（前端按此分支处理）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::TurnUnconfirmed => "forge/turn-unconfirmed",
            Self::RevisionConflict { .. } => "forge/revision-conflict",
            Self::TargetUnavailable { .. } => "forge/target-unavailable",
            Self::InvalidSpec { .. } => "forge/invalid-spec",
            Self::TooManyStages { .. } => "forge/too-many-stages",
            Self::MissingName => "forge/missing-name",
            Self::MissingGoalElement { .. } => "forge/missing-goal-element",
            Self::MissingSuccessCondition { .. } => "forge/missing-success-condition",
            Self::EmptyStages { .. } => "forge/empty-stages",
            Self::CompileFailed { .. } => "forge/compile-failed",
            Self::UnresolvedQuestions { .. } => "forge/unresolved-questions",
            Self::UnknownFailureTarget { .. } => "forge/unknown-failure-target",
            Self::RoleIdConflict { .. } => "forge/role-id-conflict",
            Self::RoleDefinitionConflict { .. } => "forge/role-definition-conflict",
            Self::ReadonlyRole { .. } => "forge/readonly-role",
            Self::UnavailableRole { .. } => "forge/unavailable-role",
            Self::DuplicateWorkflowName { .. } => "forge/duplicate-workflow-name",
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
///
/// 线上形态与旧存档兼容：`retry` / `manual-check` / `end` / `new-round` 仍是字符串，
/// 带参数的两种写成对象——`{"new-round": "<阶段 id>"}` 与
/// `{"goto": "<阶段 id>", "continueSession": true}`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "OnFailureWire", into = "OnFailureWire")]
pub enum OnFailure {
    /// 失败后再跑一次本阶段：编译成自环失败边，次数由 `control.max_attempts` 限制。
    /// 运行时规定“没有失败边等价于失败即结束”，所以重试必须落成一条边。
    Retry,
    /// 人工复核：manual_check = true（与 output 验证互斥，见编译规则）。
    ManualCheck,
    /// 失败即终止：边指向 `$end`。
    End,
    /// 失败后从工作流入口重开一轮：边指向 `$new-round`。
    NewRound,
    /// 失败后从指定阶段重开一轮。
    NewRoundFrom(String),
    /// 失败后跳到同一工作流的某个阶段（修复回路）。`continue_session` 为真时
    /// 沿用本阶段的会话，后继阶段带着失败现场继续。
    Jump {
        stage: String,
        continue_session: bool,
    },
}

/// [`OnFailure`] 的线上形态（未标注标签：字符串或对象）。
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum OnFailureWire {
    Simple(SimpleOnFailure),
    NewRound {
        #[serde(rename = "new-round")]
        stage: String,
    },
    Jump {
        goto: String,
        #[serde(default, rename = "continueSession")]
        continue_session: bool,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SimpleOnFailure {
    Retry,
    ManualCheck,
    End,
    NewRound,
}

impl From<OnFailureWire> for OnFailure {
    fn from(wire: OnFailureWire) -> Self {
        match wire {
            OnFailureWire::Simple(SimpleOnFailure::Retry) => Self::Retry,
            OnFailureWire::Simple(SimpleOnFailure::ManualCheck) => Self::ManualCheck,
            OnFailureWire::Simple(SimpleOnFailure::End) => Self::End,
            OnFailureWire::Simple(SimpleOnFailure::NewRound) => Self::NewRound,
            // `{"new-round": "$entry"}` 与不带参数的 `"new-round"` 同义。
            OnFailureWire::NewRound { stage } if stage == crate::dsl::ENTRY_NODE => Self::NewRound,
            OnFailureWire::NewRound { stage } => Self::NewRoundFrom(stage),
            OnFailureWire::Jump {
                goto,
                continue_session,
            } => Self::Jump {
                stage: goto,
                continue_session,
            },
        }
    }
}

impl From<OnFailure> for OnFailureWire {
    fn from(value: OnFailure) -> Self {
        match value {
            OnFailure::Retry => Self::Simple(SimpleOnFailure::Retry),
            OnFailure::ManualCheck => Self::Simple(SimpleOnFailure::ManualCheck),
            OnFailure::End => Self::Simple(SimpleOnFailure::End),
            OnFailure::NewRound => Self::Simple(SimpleOnFailure::NewRound),
            OnFailure::NewRoundFrom(stage) => Self::NewRound { stage },
            OnFailure::Jump {
                stage,
                continue_session,
            } => Self::Jump {
                goto: stage,
                continue_session,
            },
        }
    }
}

/// 待澄清项：问题本身，加上模型给出的推荐答案（有推荐时预览面板可以一键采用）。
///
/// 线上形态是对象 `{question, recommendation}`；只有问题的裸字符串同样接受
/// （旧存档，以及模型没按协议输出对象的时候）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "ForgeQuestionWire")]
pub struct ForgeQuestion {
    pub question: String,
    /// 模型的推荐答案；没有可推荐的开放式问题可以没有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ForgeQuestionWire {
    Text(String),
    Detail {
        question: String,
        #[serde(default)]
        recommendation: Option<String>,
    },
}

impl From<ForgeQuestionWire> for ForgeQuestion {
    fn from(wire: ForgeQuestionWire) -> Self {
        match wire {
            ForgeQuestionWire::Text(question) => Self {
                question,
                recommendation: None,
            },
            ForgeQuestionWire::Detail {
                question,
                recommendation,
            } => Self {
                question,
                recommendation,
            },
        }
    }
}

impl ForgeQuestion {
    pub fn ask(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            recommendation: None,
        }
    }
}

/// stage 角色：决定编译时的内置 profile 选型；
/// `custom` 时以 goal 生成 User scope 新 profile。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageRole {
    /// Projection of an editor worker without a profile; never allocates a persona.
    Unassigned,
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

/// Stable protocol keys; values are in the author's language.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalFields {
    #[serde(default)] pub input: String,
    #[serde(default)] pub output: String,
    #[serde(default)] pub boundary: String,
    #[serde(default)] pub acceptance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ForgeGoal {
    Fields(GoalFields),
    /// Non-authored editor text, including absence. Only its original stage may carry it.
    Projected { #[serde(deserialize_with = "Deserialize::deserialize")] original: Option<String> },
    /// Earlier tagged draft goals remain executable without rewriting their worker text.
    Tagged(String),
}

impl From<String> for ForgeGoal {
    fn from(value: String) -> Self { Self::Tagged(value) }
}
impl From<&str> for ForgeGoal {
    fn from(value: &str) -> Self { Self::Tagged(value.into()) }
}
impl ForgeGoal {
    pub fn worker_text(&self) -> Option<String> {
        match self {
            Self::Fields(f) => Some(format!("input: {}
output: {}
boundary: {}
acceptance: {}", f.input, f.output, f.boundary, f.acceptance)),
            Self::Projected { original } => original.clone(),
            Self::Tagged(text) => Some(text.clone()),
        }
    }
    fn missing_elements(&self) -> Vec<GoalElement> {
        match self {
            Self::Projected { .. } => Vec::new(),
            Self::Fields(f) => GoalElement::ALL.into_iter().zip([&f.input, &f.output, &f.boundary, &f.acceptance])
                .filter_map(|(key, value)| value.trim().is_empty().then_some(key)).collect(),
            Self::Tagged(text) => missing_goal_elements(text),
        }
    }
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
    pub goal: ForgeGoal,
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

impl ForgeStage {
    /// custom 角色的角色名（去首尾空白）；内置角色返回 `None`。
    ///
    /// 角色以名字为身份：同一条工作流里同名的 stage 共用一个角色（一份职责正文、
    /// 一个 profileId）。只给某个 stage 换角色就是给它一个新名字；给角色改名则是
    /// 所有用到它的 stage 一起改名。
    pub fn custom_role_name(&self) -> Option<&str> {
        match &self.role {
            StageRole::Custom { profile_name } => Some(profile_name.trim()),
            _ => None,
        }
    }

    /// 失败策略指向的 stage id（跳转 / 从指定阶段重开）；其余策略没有目标。
    pub fn failure_target(&self) -> Option<&str> {
        match &self.on_failure {
            OnFailure::NewRoundFrom(stage) | OnFailure::Jump { stage, .. } => Some(stage),
            _ => None,
        }
    }
}

/// The group occupies its first member's position in the outer stage list. Only the
/// branch lists define order inside the group; the worker definitions are not duplicated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgeParallelGroup {
    pub id: String,
    pub branches: Vec<crate::dsl::ParallelBranchDsl>,
    pub max_parallel: u32,
    pub on_failure: OnFailure,
}

/// 单条工作流（拆分时 Spec 顶层含多个）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeWorkflow {
    /// Stable task-local identity, allocated by the archive writer.
    #[serde(default)]
    pub id: String,
    /// 工作流显示名（必收敛字段）。
    pub name: String,
    pub stages: Vec<ForgeStage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parallel_groups: Vec<ForgeParallelGroup>,
}

impl ForgeWorkflow {
    /// Validate only structural invariants, also usable on an unfinished interview draft.
    pub(crate) fn validate_groups(&self) -> Result<(), ForgeError> {
        use std::collections::{HashMap, HashSet};
        if self.parallel_groups.is_empty() { return Ok(()); }
        let invalid = |detail: &str| ForgeError::InvalidSpec { detail: detail.into() };
        if self.stages.len() > MAX_STAGES_PER_WORKFLOW {
            return Err(ForgeError::TooManyStages { workflow: self.name.clone(), count: self.stages.len(), limit: MAX_STAGES_PER_WORKFLOW });
        }
        let mut stages = HashSet::new();
        for id in self.stages.iter().filter_map(|s| s.id.as_deref()) {
            if !stages.insert(id) { return Err(invalid("parallel topology cannot reference ambiguous stage ids")); }
        }
        let mut groups = HashSet::new();
        let mut members = HashMap::new();
        for group in &self.parallel_groups {
            if !stage_id_is_valid(&group.id) || stages.contains(group.id.as_str()) || !groups.insert(group.id.as_str()) {
                return Err(invalid("invalid or duplicate parallel group id"));
            }
            if !(1..=crate::dsl::MAX_STATIC_PARALLEL).contains(&group.max_parallel)
                || !(2..=MAX_STAGES_PER_WORKFLOW).contains(&group.branches.len()) {
                return Err(invalid("parallel group size or concurrency outside bounds"));
            }
            let mut branch_ids = HashSet::new();
            for branch in &group.branches {
                if branch.id.trim().is_empty() || !branch_ids.insert(&branch.id) || branch.stages.is_empty() {
                    return Err(invalid("parallel group requires distinct nonempty branches"));
                }
                for (index, id) in branch.stages.iter().enumerate() {
                    if !stage_id_is_valid(id) || !stages.contains(id.as_str())
                        || members.insert(id.as_str(), (&group.id, &branch.id, index)).is_some() {
                        return Err(invalid("parallel stage must identify one worker with one owner"));
                    }
                }
            }
        }
        let outer = |id: &str| groups.contains(id) || (stages.contains(id) && !members.contains_key(id));
        for group in &self.parallel_groups {
            let valid = match &group.on_failure {
                OnFailure::ManualCheck => false,
                OnFailure::Jump { stage, continue_session } => outer(stage) && !continue_session,
                OnFailure::NewRoundFrom(stage) => outer(stage),
                _ => true,
            };
            if !valid { return Err(invalid("invalid outer parallel group failure route")); }
        }
        for stage in &self.stages {
            if let Some((group, branch, index)) = stage.id.as_deref().and_then(|id| members.get(id)) {
                let local = match &stage.on_failure {
                    OnFailure::Retry | OnFailure::End => true,
                    OnFailure::Jump {stage: target, ..} => members.get(target.as_str()).is_some_and(|(g,b,i)| g == group && b == branch && i <= index),
                    _ => false,
                };
                if stage.manual_check || !local { return Err(invalid("branch worker only supports local bounded retry or end")); }
            } else if stage.failure_target().is_some_and(|id| !outer(id))
                || matches!(&stage.on_failure, OnFailure::Jump {stage, continue_session: true} if groups.contains(stage.as_str())) {
                return Err(invalid("invalid outer worker failure route"));
            }
        }
        Ok(())
    }
}

/// Forge Spec 顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSpec {
    /// Expected archive revision on full writes; zero denotes an unpersisted draft.
    #[serde(default)]
    pub revision: u64,
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
    pub open_questions: Vec<ForgeQuestion>,
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
    // Earlier English protocol examples used explicit field labels. These are structure,
    // not a request to infer elements from prose or translate values.
    if goal.lines().any(|line| line.trim_start().split_once(':').is_some_and(|(key, value)|
        key.eq_ignore_ascii_case(element.as_str()) && !value.trim().is_empty())) {
        return true;
    }
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

/// stage 在错误里的称呼：有 id 用 id，没有（未经存档直接编译的调用方）用序号。
fn stage_label(stage: &ForgeStage, index: usize) -> String {
    stage
        .id
        .clone()
        .unwrap_or_else(|| format!("#{}", index + 1))
}

/// 失败策略只能跳到同一条工作流里的 stage。
pub(crate) fn failure_target_errors(workflow: &ForgeWorkflow) -> Vec<ForgeError> {
    workflow
        .stages
        .iter()
        .enumerate()
        .filter_map(|(index, stage)| {
            let target = stage.failure_target()?;
            let known = workflow
                .stages
                .iter()
                .any(|other| other.id.as_deref() == Some(target)) || workflow.parallel_groups.iter().any(|g| g.id == target);
            (!known).then(|| ForgeError::UnknownFailureTarget {
                stage: stage_label(stage, index),
                target: target.to_string(),
            })
        })
        .collect()
}

/// 角色身份约束（同一条工作流内）：角色以名字为身份——同名的 stage 共用一个角色，
/// 所以同一个名字只能对应一个 profileId、一份职责正文。职责正文缺省的 stage 跟随
/// 同名 stage 里给出的那一份，不参与比较。
fn role_identity_errors(workflow: &ForgeWorkflow) -> Vec<ForgeError> {
    struct RoleGroup {
        stages: Vec<String>,
        profile_id: Option<String>,
        definition: Option<String>,
        id_conflict: bool,
        definition_conflict: bool,
    }
    let mut groups: std::collections::BTreeMap<&str, RoleGroup> = std::collections::BTreeMap::new();
    for (index, stage) in workflow.stages.iter().enumerate() {
        let Some(name) = stage.custom_role_name().filter(|name| !name.is_empty()) else {
            continue;
        };
        let group = groups.entry(name).or_insert(RoleGroup {
            stages: Vec::new(),
            profile_id: None,
            definition: None,
            id_conflict: false,
            definition_conflict: false,
        });
        group.stages.push(stage_label(stage, index));
        if let Some(profile_id) = stage.profile_id.as_deref() {
            if group
                .profile_id
                .as_deref()
                .is_some_and(|existing| existing != profile_id)
            {
                group.id_conflict = true;
            } else {
                group.profile_id = Some(profile_id.to_string());
            }
        }
        if let Some(definition) = stage
            .role_definition
            .as_deref()
            .map(str::trim)
            .filter(|definition| !definition.is_empty())
        {
            if group
                .definition
                .as_deref()
                .is_some_and(|existing| existing != definition)
            {
                group.definition_conflict = true;
            } else {
                group.definition = Some(definition.to_string());
            }
        }
    }
    groups
        .into_iter()
        .flat_map(|(role, group)| {
            let stages = group.stages.join(", ");
            let id = group.id_conflict.then(|| ForgeError::RoleIdConflict {
                role: role.to_string(),
                stages: stages.clone(),
            });
            let definition = group.definition_conflict.then(|| ForgeError::RoleDefinitionConflict {
                role: role.to_string(),
                stages,
            });
            id.into_iter().chain(definition)
        })
        .collect()
}

impl ForgeSpec {
    /// 从 JSON 文本解析并做全量收敛校验。
    pub fn parse_and_validate(json: &str) -> Result<Self, ForgeError> {
        let spec: ForgeSpec = serde_json::from_str(json)?;
        spec.validate()?;
        Ok(spec)
    }

    /// 全量校验：必收敛字段、stages 上限、四要素、openQuestions 清空。
    /// 遇到第一个问题即返回。
    pub fn validate(&self) -> Result<(), ForgeError> {
        self.validate_all()
            .into_iter()
            .next()
            .map_or(Ok(()), Err)
    }

    /// 全量校验并收集**全部**问题：修正轮按错误清单一次性修复，3 轮预算
    /// 不会被逐个报错耗尽。空列表即通过。
    pub fn validate_all(&self) -> Vec<ForgeError> {
        let mut errors = Vec::new();
        if self.workflows.is_empty() {
            errors.push(ForgeError::EmptyStages {
                workflow: self.name.clone().unwrap_or_default(),
            });
        }
        if self.workflows.len() == 1 && is_blank(self.name.as_deref()) {
            errors.push(ForgeError::MissingName);
        }
        for workflow in &self.workflows {
            if is_blank(Some(&workflow.name)) {
                errors.push(ForgeError::MissingName);
            }
            if workflow.stages.is_empty() {
                errors.push(ForgeError::EmptyStages {
                    workflow: workflow.name.clone(),
                });
            }
            if workflow.stages.len() > MAX_STAGES_PER_WORKFLOW {
                errors.push(ForgeError::TooManyStages {
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
                for element in stage.goal.missing_elements() {
                    errors.push(ForgeError::MissingGoalElement {
                        stage: stage_id.clone(),
                        element: element.as_str().to_string(),
                    });
                }
                if stage.output_kind == StageOutputKind::Json
                    && is_blank(stage.success_condition.as_deref())
                {
                    errors.push(ForgeError::MissingSuccessCondition { stage: stage_id });
                }
            }
            errors.extend(workflow.validate_groups().err());
            errors.extend(failure_target_errors(workflow));
            errors.extend(role_identity_errors(workflow));
        }
        if !self.open_questions.is_empty() {
            errors.push(ForgeError::UnresolvedQuestions {
                items: self
                    .open_questions
                    .iter()
                    .map(|question| question.question.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            });
        }
        errors
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
    let group_schema = serde_json::json!({ "type": "array", "maxItems": MAX_STAGES_PER_WORKFLOW / 2,
                            "items": { "type": "object", "additionalProperties": false,
                                "required": ["id", "branches", "maxParallel", "onFailure"],
                                "properties": {
                                    "id": { "type": "string", "minLength": 1, "maxLength": MAX_STAGE_ID_LEN },
                                    "maxParallel": { "type": "integer", "minimum": 1, "maximum": crate::dsl::MAX_STATIC_PARALLEL },
                                    "branches": { "type": "array", "minItems": 2, "maxItems": MAX_STAGES_PER_WORKFLOW,
                                        "items": { "type": "object", "additionalProperties": false, "required": ["id", "stages"], "properties": {
                                            "id": { "type": "string", "minLength": 1 },
                                            "stages": { "type": "array", "minItems": 1, "maxItems": MAX_STAGES_PER_WORKFLOW, "uniqueItems": true, "items": { "type": "string" } }
                                        } }
                                    },
                                    "onFailure": { "$ref": "#/$defs/stage/properties/onFailure" }
                                }
                            }
                        });
    let keys = GoalElement::ALL.map(|e| e.as_str());
    let fields = keys.iter().map(|k| (k.to_string(), serde_json::json!({ "type": "string", "minLength": 1 }))).collect::<serde_json::Map<_,_>>();
    let goal_schema = serde_json::json!({ "oneOf": [
        { "type": "object", "additionalProperties": false, "required": keys, "properties": fields },
        { "type": "object", "additionalProperties": false, "readOnly": true, "required": ["original"],
          "properties": { "original": { "type": ["string", "null"] } } }
    ] });
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "ForgeSpec",
        "type": "object",
        "required": ["specVersion", "intent", "workflows"],
        "properties": {
            "specVersion": { "const": SPEC_VERSION },
            "revision": { "type": "integer", "minimum": 0, "readOnly": true },
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
                        "id": { "type": "string", "readOnly": true },
                        "parallelGroups": group_schema,
                        "stages": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_STAGES_PER_WORKFLOW,
                            "items": { "$ref": "#/$defs/stage" }
                        }
                    }
                }
            },
            "openQuestions": {
                "type": "array",
                "items": {
                    "oneOf": [
                        { "type": "string" },
                        {
                            "type": "object",
                            "required": ["question", "recommendation"],
                            "properties": {
                                "question": { "type": "string" },
                                "recommendation": { "type": "string" }
                            }
                        }
                    ]
                }
            }
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
                            { "const": "unassigned", "readOnly": true },
                            {
                                "type": "object",
                                "required": ["custom"],
                                "properties": {
                                    "custom": { "type": "object", "required": ["profileName"], "properties": { "profileName": { "type": "string" } } }
                                }
                            }
                        ]
                    },
                    "goal": goal_schema,
                    "roleDefinition": { "type": "string", "description": "custom 角色的完整职责正文(职责/方法/边界),与 goal 分离;缺省回退 goal" },
                    "profileId": { "type": "string", "readOnly": true },
                    "artifact": { "type": "string" },
                    "outputKind": { "enum": ["json", "markdown", "none"] },
                    "schemaHint": {},
                    "successCondition": { "type": "string" },
                    "onFailure": {
                        "oneOf": [
                            { "enum": ["retry", "manual-check", "end", "new-round"] },
                            { "type": "object", "required": ["new-round"], "properties": { "new-round": { "type": "string", "description": "失败后从哪个 stage 重开一轮(stage id)" } } },
                            { "type": "object", "required": ["goto"], "properties": { "goto": { "type": "string", "description": "失败后跳到哪个 stage(stage id)" }, "continueSession": { "type": "boolean", "description": "是否沿用本阶段的会话(带上失败现场)" } } }
                        ]
                    },
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
            goal: goal.into(),
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
            id: String::new(),
            parallel_groups: vec![],
            name: "流程".to_string(),
            stages,
        }
    }

    fn spec_with(workflows: Vec<ForgeWorkflow>) -> ForgeSpec {
        ForgeSpec {
            revision: 0,
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
            id: String::new(),
            parallel_groups: vec![],
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
    fn structured_goal_validation_uses_values_not_translated_headings() {
        let mut spec = serde_json::to_value(valid_spec()).unwrap();
        spec["workflows"][0]["stages"][0]["goal"] = serde_json::json!({
            "input": "要求", "output": "報告", "acceptance": "条件を満たす", "boundary": " "
        });
        let err = ForgeSpec::parse_and_validate(&spec.to_string()).unwrap_err();
        assert!(matches!(err, ForgeError::MissingGoalElement { element, .. } if element == "boundary"));
        spec["workflows"][0]["stages"][0]["goal"]["boundary"] = "デプロイしない".into();
        assert!(ForgeSpec::parse_and_validate(&spec.to_string()).is_ok());
        assert!(serde_json::from_value::<ForgeGoal>(serde_json::json!({"other": true})).is_err());
        assert!(serde_json::from_value::<ForgeGoal>(serde_json::json!({"original": null})).is_ok());
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
        spec.open_questions = vec![ForgeQuestion::ask("产物 schema 未定")];
        let error = spec.validate().unwrap_err();
        assert_eq!(error.code(), "forge/unresolved-questions");
        assert!(!spec.is_ready_to_compile());
        spec.open_questions.clear();
        assert!(spec.is_ready_to_compile());
    }

    #[test]
    fn validate_all_reports_every_problem() {
        // 两个 stage 各缺一个要素，且第二个缺成功条件：一次全部报出。
        let mut first = stage(StageRole::Dev, "<输入>a\n<产出>b\n<验收>c".to_string());
        first.id = Some("s1".to_string());
        let mut second = stage(
            StageRole::Test,
            "<输入>a\n<边界>b\n<验收>c".to_string(),
        );
        second.id = Some("s2".to_string());
        second.success_condition = None;
        let mut spec = spec_with(vec![workflow_with(vec![first, second])]);
        spec.open_questions = vec![ForgeQuestion::ask("未定")];

        let errors = spec.validate_all();

        let codes_contexts: Vec<(String, String)> = errors
            .iter()
            .map(|error| {
                let serialized = serde_json::to_value(error).unwrap();
                (
                    error.code().to_string(),
                    serialized["context"].to_string(),
                )
            })
            .collect();
        assert_eq!(codes_contexts.len(), 4, "{codes_contexts:#?}");
        assert!(codes_contexts[0].1.contains("s1") && codes_contexts[0].1.contains("boundary"));
        assert!(codes_contexts[1].1.contains("s2") && codes_contexts[1].1.contains("output"));
        assert_eq!(codes_contexts[2].0, "forge/missing-success-condition");
        assert_eq!(codes_contexts[3].0, "forge/unresolved-questions");
        // validate 仍是首个问题的快捷方式。
        assert_eq!(spec.validate().unwrap_err().code(), "forge/missing-goal-element");
        assert!(valid_spec().validate_all().is_empty());
    }

    #[test]
    fn stage_ids_must_be_usable_as_node_ids() {
        assert!(stage_id_is_valid("s1"));
        assert!(stage_id_is_valid("review-step_2"));
        for invalid in ["", "  ", "阶段一", "a/b", "a\\b", "..", "$end", "a b", "a.b"] {
            assert!(!stage_id_is_valid(invalid), "{invalid:?} must be invalid");
        }
        let long = "a".repeat(MAX_STAGE_ID_LEN + 1);
        assert!(!stage_id_is_valid(&long));
        assert!(stage_id_is_valid(&"a".repeat(MAX_STAGE_ID_LEN)));
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

    fn custom(id: &str, name: &str, profile_id: Option<&str>, definition: Option<&str>) -> ForgeStage {
        let mut s = stage(
            StageRole::Custom {
                profile_name: name.to_string(),
            },
            base_goal(),
        );
        s.id = Some(id.to_string());
        s.profile_id = profile_id.map(str::to_string);
        s.role_definition = definition.map(str::to_string);
        s
    }

    #[test]
    fn on_failure_wire_forms_round_trip() {
        // 旧形态（字符串）原样可读，带参数的两种写成对象。
        let parse = |json: &str| serde_json::from_str::<OnFailure>(json).unwrap();
        assert_eq!(parse("\"retry\""), OnFailure::Retry);
        assert_eq!(parse("\"manual-check\""), OnFailure::ManualCheck);
        assert_eq!(parse("\"end\""), OnFailure::End);
        assert_eq!(parse("\"new-round\""), OnFailure::NewRound);
        assert_eq!(
            parse("{\"new-round\":\"dev\"}"),
            OnFailure::NewRoundFrom("dev".to_string())
        );
        // `$entry` 就是从入口重开，与不带参数同义。
        assert_eq!(parse("{\"new-round\":\"$entry\"}"), OnFailure::NewRound);
        assert_eq!(
            parse("{\"goto\":\"dev\",\"continueSession\":true}"),
            OnFailure::Jump {
                stage: "dev".to_string(),
                continue_session: true
            }
        );
        assert_eq!(
            parse("{\"goto\":\"dev\"}"),
            OnFailure::Jump {
                stage: "dev".to_string(),
                continue_session: false
            }
        );

        let json = |value: OnFailure| serde_json::to_value(value).unwrap();
        assert_eq!(json(OnFailure::Retry), serde_json::json!("retry"));
        assert_eq!(json(OnFailure::NewRound), serde_json::json!("new-round"));
        assert_eq!(
            json(OnFailure::NewRoundFrom("dev".to_string())),
            serde_json::json!({ "new-round": "dev" })
        );
        assert_eq!(
            json(OnFailure::Jump {
                stage: "dev".to_string(),
                continue_session: true
            }),
            serde_json::json!({ "goto": "dev", "continueSession": true })
        );
        // 未知取值不静默回落。
        assert!(serde_json::from_str::<OnFailure>("\"explode\"").is_err());
        assert!(serde_json::from_str::<OnFailure>("{\"goto\":42}").is_err());
    }

    #[test]
    fn question_accepts_plain_text_and_detail() {
        let text: ForgeQuestion = serde_json::from_str("\"发布对象是什么？\"").unwrap();
        assert_eq!(text.question, "发布对象是什么？");
        assert_eq!(text.recommendation, None);
        let detail: ForgeQuestion = serde_json::from_str(
            "{\"question\":\"发布对象是什么？\",\"recommendation\":\"待发布的工作流及其模板\"}",
        )
        .unwrap();
        assert_eq!(
            detail.recommendation.as_deref(),
            Some("待发布的工作流及其模板")
        );
        // 序列化统一成对象形态。
        assert_eq!(
            serde_json::to_value(ForgeQuestion::ask("还有吗")).unwrap(),
            serde_json::json!({ "question": "还有吗" })
        );
    }

    #[test]
    fn failure_targets_must_be_stages_of_the_same_workflow() {
        let mut jump = custom("audit", "审计员", None, None);
        jump.on_failure = OnFailure::Jump {
            stage: "dev".to_string(),
            continue_session: true,
        };
        let mut restart = stage(StageRole::Accept, base_goal());
        restart.id = Some("accept".to_string());
        restart.on_failure = OnFailure::NewRoundFrom("dev".to_string());
        let spec = spec_with(vec![workflow_with(vec![jump.clone(), restart.clone()])]);
        let codes: Vec<&str> = spec.validate_all().iter().map(ForgeError::code).collect();
        assert_eq!(
            codes,
            [
                "forge/unknown-failure-target",
                "forge/unknown-failure-target"
            ]
        );

        let dev = {
            let mut dev = stage(StageRole::Dev, base_goal());
            dev.id = Some("dev".to_string());
            dev
        };
        spec_with(vec![workflow_with(vec![dev, jump, restart])])
            .validate()
            .unwrap();
    }

    #[test]
    fn a_role_name_maps_to_one_profile_id_and_one_definition() {
        // 同名 stage 共用一个角色：id 与职责正文一致才成立。
        let profile_id = "pf-role";
        let definition = "逐项复核，不修改任何文件。";
        let spec = spec_with(vec![workflow_with(vec![
            custom("audit-1", "审计员", Some(profile_id), Some(definition)),
            custom("audit-2", "审计员", Some(profile_id), Some(definition)),
            // 职责正文缺省的 stage 跟随同名 stage 的那一份，不算冲突。
            custom("audit-3", "审计员", Some(profile_id), None),
            // 名字相同但还没回填 id 的新 stage 同样合法（编译时归入同一个角色）。
            custom("audit-4", "审计员", None, None),
        ])]);
        spec.validate().unwrap();

        let mut conflict = spec.clone();
        conflict.workflows[0].stages[1].profile_id = Some("pf-other".to_string());
        let error = conflict.validate().unwrap_err();
        assert!(matches!(error, ForgeError::RoleIdConflict { ref role, .. } if role == "审计员"));

        let mut conflict = spec.clone();
        conflict.workflows[0].stages[1].role_definition = Some("另一套职责".to_string());
        let error = conflict.validate().unwrap_err();
        assert!(matches!(error, ForgeError::RoleDefinitionConflict { .. }));

        // 不同名字、不同角色互不影响。
        let independent = spec_with(vec![workflow_with(vec![
            custom("audit", "审计员", Some(profile_id), None),
            custom("review", "复核员", Some("pf-other"), None),
        ])]);
        independent.validate().unwrap();
    }
}
