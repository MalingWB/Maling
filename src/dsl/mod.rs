use crate::domain::{NodeType, SessionMode};
use crate::provider::supports_continue_session;
use anyhow::{Context, Result, anyhow, bail, ensure};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};

/// 工作流预设（provider → 单节点 WorkflowDsl 复用构造，开发设计 2.5）。
pub mod presets;

pub const END_NODE: &str = "$end";
pub const ENTRY_NODE: &str = "$entry";
pub const NEW_ROUND_NODE: &str = "$new-round";
const RESERVED_NODE_IDS: &[&str] = &[END_NODE, ENTRY_NODE, NEW_ROUND_NODE];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WorkflowValidationError {
    #[error("workflow must include an edge targeting `$end`")]
    MissingEndNode,
    #[error("node `{node_id}` is unreachable from entry")]
    UnreachableNode { node_id: String },
    #[error("edge `{from}` cannot target `$new-round` on success")]
    SuccessNewRoundTarget { from: String },
    #[error("edge `{from}` targeting `$new-round` must declare `new_round_entry`")]
    MissingNewRoundEntry { from: String },
    #[error("edge `{from}` has invalid `new_round_entry`: {entry}")]
    InvalidNewRoundEntry { from: String, entry: String },
    #[error("workflow `{workflow_name}` id `{workflow_id}` is duplicated with {conflicts}")]
    DuplicateWorkflowId {
        workflow_name: String,
        workflow_id: String,
        conflicts: String,
    },
    #[error("ai-dynamic node `{node_id}` references invalid workflow `{workflow_name}`: {reason}")]
    AiDynamicInvalidWorkflow {
        node_id: String,
        workflow_name: String,
        reason: String,
    },
    #[error("model cannot be blank")]
    WorkerModelBlank { node_id: String, provider: String },
    #[error("ai-dynamic node `{node_id}` fixed model cannot be blank")]
    DynamicFixedModelBlank { node_id: String },
    #[error("ai-dynamic node `{node_id}` availableAgents cannot be empty")]
    DynamicAgentsEmpty { node_id: String },
    #[error("ai-dynamic node `{node_id}` available agent `{provider}` is duplicated")]
    DynamicAgentDuplicate { node_id: String, provider: String },
    #[error("ai-dynamic node `{node_id}` available agent `{provider}` model cannot be blank")]
    DynamicAgentModelBlank { node_id: String, provider: String },
    #[error("agent `{provider}` model cannot be blank")]
    AgentModelBlank { provider: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonPathSegment {
    Key(String),
    Index(usize),
}

pub fn parse_json_path(path: &str) -> Result<Vec<JsonPathSegment>> {
    let mut value = path.trim();
    if let Some(rest) = value.strip_prefix("$.") {
        value = rest;
    } else if value == "$" {
        bail!("json path cannot be root only");
    } else if let Some(rest) = value.strip_prefix('$') {
        value = rest;
    }
    ensure!(!value.is_empty(), "json path cannot be empty");

    let chars = value.chars().collect::<Vec<_>>();
    let mut segments = Vec::new();
    let mut key = String::new();
    let mut index = 0;

    while index < chars.len() {
        match chars[index] {
            '.' => {
                if key.is_empty() {
                    ensure!(
                        matches!(segments.last(), Some(JsonPathSegment::Index(_))),
                        "json path contains an empty segment: {path}"
                    );
                } else {
                    segments.push(JsonPathSegment::Key(std::mem::take(&mut key)));
                }
                index += 1;
            }
            '[' => {
                if !key.is_empty() {
                    segments.push(JsonPathSegment::Key(std::mem::take(&mut key)));
                }
                index += 1;
                let start = index;
                while index < chars.len() && chars[index] != ']' {
                    ensure!(
                        chars[index].is_ascii_digit(),
                        "json path array index must be numeric: {path}"
                    );
                    index += 1;
                }
                ensure!(
                    index < chars.len() && chars[index] == ']',
                    "json path array index is not closed: {path}"
                );
                ensure!(
                    index > start,
                    "json path array index cannot be empty: {path}"
                );
                let array_index = chars[start..index]
                    .iter()
                    .collect::<String>()
                    .parse::<usize>()?;
                segments.push(JsonPathSegment::Index(array_index));
                index += 1;
                if index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    bail!("json path segment must be separated by `.` or array index: {path}");
                }
            }
            character => {
                key.push(character);
                index += 1;
            }
        }
    }

    ensure!(
        !key.is_empty() || matches!(segments.last(), Some(JsonPathSegment::Index(_))),
        "json path cannot end with an empty segment: {path}"
    );
    if !key.is_empty() {
        segments.push(JsonPathSegment::Key(key));
    }
    ensure!(!segments.is_empty(), "json path cannot be empty");
    Ok(segments)
}

pub fn parse_success_expression_path(expression: &str) -> Result<Vec<JsonPathSegment>> {
    const OPERATORS: [&str; 6] = [">=", "<=", "!=", "==", ">", "<"];
    let trimmed = expression.trim();
    let (_, left, _) = OPERATORS
        .iter()
        .find_map(|operator| {
            trimmed
                .split_once(operator)
                .map(|(left, right)| (*operator, left.trim(), right.trim()))
        })
        .ok_or_else(|| anyhow!("unsupported success expression: {expression}"))?;
    ensure!(
        left.starts_with('$'),
        "success expression left side must start with `$`: {expression}"
    );
    parse_json_path(left)
}

pub fn simple_schema_contains_path(schema: &serde_json::Value, path: &[JsonPathSegment]) -> bool {
    let mut cursor = schema;
    for segment in path {
        match segment {
            JsonPathSegment::Key(key) => {
                let Some(object) = cursor.as_object() else {
                    return false;
                };
                let Some(next) = object.get(key) else {
                    return false;
                };
                cursor = next;
            }
            JsonPathSegment::Index(index) => {
                let Some(array) = cursor.as_array() else {
                    return false;
                };
                let Some(next) = array.get(*index).or_else(|| array.first()) else {
                    return false;
                };
                cursor = next;
            }
        }
    }
    true
}

/// `schema` 是 JSON Schema 而非简化字段表。
///
/// 简化字段表是「字段名 → 形状」（字符串是类型名，对象是嵌套字段，数组是 `[元素形状]`），
/// 字段名可以恰好叫 `items` / `type` / `required` / `properties`。所以不能只看键名：只有当
/// 关键字位置上的值具有 JSON Schema 才有的形态时才算 JSON Schema。前端工作流编辑器有
/// 一份等价判定（`web/src/lib/output-schema.ts`），两边的用例表保持一致。
pub fn looks_like_json_schema(schema: &serde_json::Value) -> bool {
    use serde_json::Value;
    let Some(object) = schema.as_object() else {
        return false;
    };
    let is_type_name = |value: &Value| {
        value.as_str().is_some_and(|name| {
            matches!(
                name,
                "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
            )
        })
    };
    let has_structure_keyword = |keys: &[&str]| keys.iter().any(|key| object.contains_key(*key));
    // 简化字段表里不会出现的声明式关键字。
    if has_structure_keyword(&["$schema", "$ref", "anyOf", "oneOf", "allOf"]) {
        return true;
    }
    // additionalProperties 只可能是布尔或子模式；简化形状里不存在布尔值。
    if matches!(object.get("additionalProperties"), Some(Value::Bool(_))) {
        return true;
    }
    if let Some(kind) = object.get("type") {
        // 联合类型：简化形状的数组只取第一个元素当元素形状，多个类型名的数组只会是 JSON Schema。
        if matches!(kind, Value::Array(types) if types.len() > 1 && types.iter().all(Value::is_string))
        {
            return true;
        }
        // 旧版基本类型根节点：只有 type 一个键，表示整个输出就是一个基本类型值。
        if is_type_name(kind) && object.len() == 1 {
            return true;
        }
        // 容器类型与其它结构关键字同时出现。
        if matches!(kind.as_str(), Some("object" | "array"))
            && has_structure_keyword(&["properties", "required", "additionalProperties", "items"])
        {
            return true;
        }
    }
    // 属性节点：`properties` 的某个值是带 `type` 的子模式（简化形状的嵌套字段不以 type 描述自己）。
    object
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| {
            properties.values().any(|value| {
                matches!(value, Value::Object(inner) if inner.get("type").is_some_and(|t| t.is_string() || t.is_array()))
            })
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDsl {
    pub version: String,
    pub id: String,
    pub entry: String,
    #[serde(default)]
    pub control: WorkflowControl,
    pub nodes: Vec<NodeDsl>,
    pub edges: Vec<EdgeDsl>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum NodeDsl {
    Worker(WorkerNode),
    AiDynamic(AiDynamicNode),
    ParallelGroup(ParallelGroupNode),
}

pub const PARALLEL_GROUP_RESULT: &str = "parallel-group-result";
pub const MAX_STATIC_PARALLEL: u32 = 5;
pub const MAX_PARALLEL_GROUP_STAGES: usize = 64;
pub const DEFAULT_STATIC_PARALLEL: u32 = 3;

fn default_static_parallel() -> u32 {
    DEFAULT_STATIC_PARALLEL
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParallelGroupNode {
    pub id: String,
    pub branches: Vec<ParallelBranchDsl>,
    #[serde(default = "default_static_parallel")]
    pub max_parallel: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParallelBranchDsl {
    pub id: String,
    pub stages: Vec<String>,
}

impl NodeDsl {
    pub fn id(&self) -> &str {
        match self {
            Self::Worker(node) => &node.id,
            Self::AiDynamic(node) => &node.id,
            Self::ParallelGroup(node) => &node.id,
        }
    }

    pub fn node_type(&self) -> NodeType {
        match self {
            Self::Worker(_) => NodeType::Worker,
            Self::AiDynamic(_) => NodeType::AiDynamic,
            Self::ParallelGroup(_) => NodeType::ParallelGroup,
        }
    }

    pub fn provider(&self) -> Option<&str> {
        match self {
            Self::Worker(node) => node.provider.as_deref(),
            Self::AiDynamic(node) => node.bootstrap_provider(),
            Self::ParallelGroup(_) => None,
        }
    }

    pub fn profile(&self) -> Option<&str> {
        match self {
            Self::Worker(node) => node.profile.as_deref(),
            Self::AiDynamic(_) | Self::ParallelGroup(_) => None,
        }
    }

    pub fn goal(&self) -> Option<&str> {
        match self {
            Self::Worker(node) => node.goal.as_deref(),
            Self::AiDynamic(_) | Self::ParallelGroup(_) => None,
        }
    }

    pub fn manual_check_enabled(&self) -> bool {
        match self {
            Self::Worker(node) => node.manual_check.unwrap_or(false),
            Self::AiDynamic(_) | Self::ParallelGroup(_) => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerNode {
    pub id: String,
    #[serde(
        default,
        rename = "executionSlotId",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_slot_id: Option<String>,
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub profile: Option<String>,
    pub goal: Option<String>,
    pub output: Option<OutputContractDsl>,
    pub success_condition: Option<JsonConditionDsl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_accept: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config_options: BTreeMap<String, String>,
    #[serde(default)]
    pub manual_check: Option<bool>,
    #[serde(default)]
    pub prompt_envelope: PromptEnvelopeMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptEnvelopeMode {
    #[default]
    RuntimeManaged,
    RawAgent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDynamicNode {
    pub id: String,
    pub agent_strategy: AiDynamicAgentStrategy,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config_options: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_bound_overrides: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_goal: Option<String>,
    #[serde(default)]
    pub control: DynamicControlDsl,
    #[serde(default)]
    pub allowed_workflows: Vec<AllowedWorkflowRefDsl>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AiDynamicNodeCompat {
    pub id: String,
    #[serde(default)]
    pub agent_strategy: Option<AiDynamicAgentStrategy>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub config_options: BTreeMap<String, String>,
    #[serde(default)]
    pub model_bound_overrides: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub allowed_profiles: Vec<String>,
    #[serde(default)]
    pub global_goal: Option<String>,
    #[serde(default)]
    pub control: DynamicControlDsl,
    #[serde(default)]
    pub allowed_workflows: Vec<AllowedWorkflowRefDsl>,
}

impl<'de> Deserialize<'de> for AiDynamicNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = AiDynamicNodeCompat::deserialize(deserializer)?;
        let agent_strategy = match raw.agent_strategy {
            Some(strategy) => strategy,
            None => {
                let provider = raw
                    .provider
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| serde::de::Error::missing_field("agentStrategy"))?;
                AiDynamicAgentStrategy::Fixed {
                    provider,
                    model: None,
                    permission_mode: None,
                    auto_accept: false,
                }
            }
        };
        Ok(Self {
            id: raw.id,
            agent_strategy,
            config_options: raw.config_options,
            model_bound_overrides: raw.model_bound_overrides,
            allowed_profiles: raw
                .allowed_profiles
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect(),
            global_goal: raw
                .global_goal
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            control: raw.control,
            allowed_workflows: raw.allowed_workflows,
        })
    }
}

impl AiDynamicNode {
    pub fn bootstrap_provider(&self) -> Option<&str> {
        match &self.agent_strategy {
            AiDynamicAgentStrategy::Fixed { provider, .. } => Some(provider.as_str()),
            AiDynamicAgentStrategy::Dynamic {
                bootstrap_provider, ..
            } => Some(bootstrap_provider.as_str()),
        }
    }

    pub fn bootstrap_model(&self) -> Option<&str> {
        match &self.agent_strategy {
            AiDynamicAgentStrategy::Fixed { model, .. } => model.as_deref(),
            AiDynamicAgentStrategy::Dynamic {
                bootstrap_model, ..
            } => bootstrap_model.as_deref(),
        }
    }

    pub fn global_goal(&self) -> Option<&str> {
        self.global_goal.as_deref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum AiDynamicAgentStrategy {
    #[serde(rename_all = "camelCase")]
    Fixed {
        provider: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        auto_accept: bool,
    },
    #[serde(rename_all = "camelCase")]
    Dynamic {
        bootstrap_provider: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bootstrap_model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        auto_accept: bool,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        bootstrap_config_options: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        bootstrap_model_bound_overrides: BTreeMap<String, BTreeMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        acceptance_model: Option<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        acceptance_config_options: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        acceptance_model_bound_overrides: BTreeMap<String, BTreeMap<String, String>>,
        routing_prompt: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        available_agents: Vec<DynamicAgentRef>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicAgentRef {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_accept: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config_options: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_bound_overrides: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DynamicControlDsl {
    #[serde(default = "default_max_dynamic_nodes")]
    pub max_dynamic_nodes: u32,
    #[serde(default = "default_max_fanout")]
    pub max_fanout: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: u32,
    #[serde(default = "default_max_group_depth")]
    pub max_group_depth: u32,
    #[serde(default = "default_max_workflow_invocations")]
    pub max_workflow_invocations: u32,
    #[serde(default)]
    pub allow_nested_dynamic: bool,
}

impl Default for DynamicControlDsl {
    fn default() -> Self {
        Self {
            max_dynamic_nodes: default_max_dynamic_nodes(),
            max_fanout: default_max_fanout(),
            max_depth: default_max_depth(),
            max_parallel: default_max_parallel(),
            max_group_depth: default_max_group_depth(),
            max_workflow_invocations: default_max_workflow_invocations(),
            allow_nested_dynamic: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedWorkflowRefDsl {
    pub workflow_id: String,
}

fn default_max_dynamic_nodes() -> u32 {
    20
}
fn default_max_fanout() -> u32 {
    5
}
fn default_max_depth() -> u32 {
    6
}
fn default_max_parallel() -> u32 {
    3
}
fn default_max_group_depth() -> u32 {
    1
}
fn default_max_workflow_invocations() -> u32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputContractDsl {
    pub kind: OutputKind,
    pub artifact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputKind {
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonConditionDsl {
    Expression {
        expression: String,
    },
    PathEquals {
        path: String,
        equals: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDsl {
    pub from: String,
    pub to: String,
    pub on: EdgeOutcome,
    pub session: Option<SessionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_round_entry: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeOutcome {
    Success,
    Failure,
}

#[derive(Debug, Clone)]
pub struct ValidatedWorkflow {
    pub raw: WorkflowDsl,
    pub nodes_by_id: IndexMap<String, NodeDsl>,
}

pub fn workflow_contains_ai_dynamic(workflow: &WorkflowDsl) -> bool {
    workflow
        .nodes
        .iter()
        .any(|node| matches!(node, NodeDsl::AiDynamic(_)))
}

pub fn workflow_requires_branch_workspaces(workflow: &WorkflowDsl) -> bool {
    workflow
        .nodes
        .iter()
        .any(|node| node.node_type().owns_branch_graph())
}

impl ValidatedWorkflow {
    pub fn get_node(&self, id: &str) -> Option<&NodeDsl> {
        self.nodes_by_id.get(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowValidationMode {
    Authoring,
    Executable,
}

fn validate_worker_node(worker: &WorkerNode, id: &str, mode: WorkflowValidationMode) -> Result<()> {
    if mode == WorkflowValidationMode::Executable {
        let provider = worker
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("worker node `{id}` provider cannot be blank"))?;
        if let Some(model) = &worker.model {
            if model.trim().is_empty() {
                bail!(WorkflowValidationError::WorkerModelBlank {
                    node_id: id.to_string(),
                    provider: provider.to_string(),
                });
            }
        }
        if let Some(permission_mode) = &worker.permission_mode {
            ensure!(
                !permission_mode.trim().is_empty(),
                "worker node `{id}` permission_mode cannot be blank"
            );
        }
    }
    if let Some(profile) = &worker.profile {
        ensure!(
            !profile.trim().is_empty(),
            "worker node `{id}` profile cannot be blank"
        );
    }
    ensure!(
        !worker.manual_check.unwrap_or(false)
            || (worker.output.is_none() && worker.success_condition.is_none()),
        "worker node `{id}` cannot enable manual_check together with output validation"
    );
    if let Some(output) = &worker.output {
        ensure!(
            !output.artifact.trim().is_empty(),
            "worker node `{id}` output artifact cannot be blank"
        );
        if let Some(schema) = &output.schema {
            ensure!(
                !looks_like_json_schema(schema),
                "worker node `{id}` output schema must use simplified output shape instead of JSON Schema"
            );
        }
    }
    if let Some(condition) = &worker.success_condition {
        ensure!(
            worker
                .output
                .as_ref()
                .is_some_and(|output| output.kind == OutputKind::Json),
            "worker node `{id}` success_condition requires json output"
        );
        let path = match condition {
            JsonConditionDsl::Expression { expression } => {
                ensure!(
                    !expression.trim().is_empty(),
                    "worker node `{id}` success_condition expression cannot be blank"
                );
                parse_success_expression_path(expression)?
            }
            JsonConditionDsl::PathEquals { path, .. } => {
                ensure!(
                    !path.trim().is_empty(),
                    "worker node `{id}` success_condition path cannot be blank"
                );
                parse_json_path(path)?
            }
        };
        if let Some(schema) = worker
            .output
            .as_ref()
            .and_then(|output| output.schema.as_ref())
        {
            ensure!(
                simple_schema_contains_path(schema, &path),
                "worker node `{id}` success_condition path is not declared in output schema"
            );
        }
    }
    Ok(())
}

fn validate_ai_dynamic_node(node: &AiDynamicNode, id: &str) -> Result<()> {
    match &node.agent_strategy {
        AiDynamicAgentStrategy::Fixed {
            provider,
            model,
            permission_mode,
            ..
        } => {
            ensure!(
                !provider.trim().is_empty(),
                "ai-dynamic node `{id}` fixed provider cannot be blank"
            );
            if let Some(m) = model {
                if m.trim().is_empty() {
                    bail!(WorkflowValidationError::DynamicFixedModelBlank {
                        node_id: id.to_string(),
                    });
                }
            }
            if let Some(mode) = permission_mode {
                ensure!(
                    !mode.trim().is_empty(),
                    "ai-dynamic node `{id}` fixed permissionMode cannot be blank"
                );
            }
        }
        AiDynamicAgentStrategy::Dynamic {
            bootstrap_provider,
            bootstrap_model,
            permission_mode,
            bootstrap_config_options: _,
            acceptance_model,
            acceptance_config_options: _,
            routing_prompt: _,
            available_agents,
            ..
        } => {
            ensure!(
                !bootstrap_provider.trim().is_empty(),
                "ai-dynamic node `{id}` bootstrapProvider cannot be blank"
            );
            if let Some(model) = bootstrap_model {
                if model.trim().is_empty() {
                    bail!(WorkflowValidationError::DynamicFixedModelBlank {
                        node_id: id.to_string(),
                    });
                }
            }
            if let Some(mode) = permission_mode {
                ensure!(
                    !mode.trim().is_empty(),
                    "ai-dynamic node `{id}` permissionMode cannot be blank"
                );
            }
            if let Some(model) = acceptance_model {
                if model.trim().is_empty() {
                    bail!(WorkflowValidationError::DynamicFixedModelBlank {
                        node_id: id.to_string(),
                    });
                }
            }
            if available_agents.is_empty() {
                bail!(WorkflowValidationError::DynamicAgentsEmpty {
                    node_id: id.to_string(),
                });
            }
            let mut seen_providers = HashSet::new();
            for agent_ref in available_agents {
                let provider = agent_ref.provider.trim();
                ensure!(
                    !provider.is_empty(),
                    "ai-dynamic node `{id}` available agent provider cannot be blank"
                );
                if !seen_providers.insert(provider.to_string()) {
                    bail!(WorkflowValidationError::DynamicAgentDuplicate {
                        node_id: id.to_string(),
                        provider: provider.to_string(),
                    });
                }
                if let Some(m) = &agent_ref.model {
                    if m.trim().is_empty() {
                        bail!(WorkflowValidationError::DynamicAgentModelBlank {
                            node_id: id.to_string(),
                            provider: provider.to_string(),
                        });
                    }
                }
                if let Some(mode) = &agent_ref.permission_mode {
                    ensure!(
                        !mode.trim().is_empty(),
                        "ai-dynamic node `{id}` available agent `{provider}` permissionMode cannot be blank"
                    );
                }
            }
        }
    }
    ensure!(
        node.control.max_dynamic_nodes > 0,
        "ai-dynamic node `{id}` maxDynamicNodes must be positive"
    );
    ensure!(
        node.control.max_fanout > 0,
        "ai-dynamic node `{id}` maxFanout must be positive"
    );
    ensure!(
        node.control.max_depth > 0,
        "ai-dynamic node `{id}` maxDepth must be positive"
    );
    ensure!(
        node.control.max_parallel > 0,
        "ai-dynamic node `{id}` maxParallel must be positive"
    );
    ensure!(
        node.control.max_group_depth > 0,
        "ai-dynamic node `{id}` maxGroupDepth must be positive"
    );
    ensure!(
        node.control.max_workflow_invocations > 0,
        "ai-dynamic node `{id}` maxWorkflowInvocations must be positive"
    );
    let mut profiles = HashSet::new();
    for profile in &node.allowed_profiles {
        let profile_id = profile.trim();
        ensure!(
            !profile_id.is_empty(),
            "ai-dynamic node `{id}` allowed profile cannot be blank"
        );
        ensure!(
            profiles.insert(profile_id.to_string()),
            "ai-dynamic node `{id}` allowed profile `{profile_id}` is duplicated"
        );
    }
    if let Some(global_goal) = &node.global_goal {
        ensure!(
            !global_goal.trim().is_empty(),
            "ai-dynamic node `{id}` globalGoal cannot be blank"
        );
    }
    let mut workflows = HashSet::new();
    for allowed in &node.allowed_workflows {
        let workflow_id = allowed.workflow_id.trim();
        ensure!(
            !workflow_id.is_empty(),
            "ai-dynamic node `{id}` allowed workflow id cannot be blank"
        );
        ensure!(
            workflows.insert(workflow_id.to_string()),
            "ai-dynamic node `{id}` allowed workflow `{workflow_id}` is duplicated"
        );
    }
    Ok(())
}

pub(crate) fn is_portable_component(id: &str) -> bool {
    let reserved_name = id
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    !id.is_empty()
        && id.len() <= 128
        && id.trim() == id
        && !id.ends_with('.')
        && !id
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && !matches!(
            reserved_name.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !["COM", "LPT"].iter().any(|p| {
            reserved_name
                .strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
        })
}

// A Windows path component compares case-insensitively, without expanding one
// character into multiple characters (e.g. `ß` is not the path `SS`). Keep the
// worker ID itself unchanged; only use this key inside one child run's namespace.
fn portable_component_case_key(id: &str) -> String {
    id.chars().map(|c| {
        let mut upper = c.to_uppercase();
        let first = upper.next().unwrap_or(c);
        if upper.next().is_none() { first } else { c }
    }).collect()
}

fn validate_parallel_groups(
    workflow: &WorkflowDsl,
    nodes: &IndexMap<String, NodeDsl>,
) -> Result<()> {
    let mut members = BTreeMap::new();
    for node in &workflow.nodes {
        let NodeDsl::ParallelGroup(group) = node else {
            continue;
        };
        ensure!(
            is_portable_component(&group.id),
            "invalid parallel-group path component"
        );
        ensure!(
            (1..=MAX_STATIC_PARALLEL).contains(&group.max_parallel),
            "parallel-group `{}` maxParallel must be 1..={MAX_STATIC_PARALLEL}",
            group.id
        );
        ensure!(
            (2..=MAX_PARALLEL_GROUP_STAGES).contains(&group.branches.len()),
            "parallel-group `{}` needs 2..={MAX_PARALLEL_GROUP_STAGES} branches",
            group.id
        );
        let mut branch_ids = HashSet::new();
        let mut count = 0;
        for branch in &group.branches {
            ensure!(
                !branch.id.trim().is_empty() && branch_ids.insert(&branch.id),
                "parallel-group `{}` requires unique nonempty branch ids",
                group.id
            );
            ensure!(
                !branch.stages.is_empty(),
                "branch `{}` has no stages",
                branch.id
            );
            count += branch.stages.len();
            ensure!(
                count <= MAX_PARALLEL_GROUP_STAGES,
                "parallel-group `{}` exceeds its stage limit",
                group.id
            );
            let mut stage_directories = HashSet::new();
            for (index, id) in branch.stages.iter().enumerate() {
                ensure!(
                    is_portable_component(id),
                    "invalid parallel stage path component"
                );
                ensure!(
                    stage_directories.insert(portable_component_case_key(id)),
                    "branch `{}` has colliding stage directory `{id}`",
                    branch.id
                );
                ensure!(
                    matches!(nodes.get(id), Some(NodeDsl::Worker(w)) if !w.manual_check.unwrap_or(false)),
                    "branch stage `{id}` must be an ordinary worker without manual_check"
                );
                ensure!(
                    members
                        .insert(id.as_str(), (&group.id, &branch.id, index))
                        .is_none(),
                    "branch stage `{id}` has multiple owners"
                );
            }
        }
    }
    if members.is_empty() {
        return Ok(());
    }
    ensure!(
        !members.contains_key(workflow.entry.as_str()),
        "workflow entry cannot target a branch member"
    );
    for edge in &workflow.edges {
        let source = members.get(edge.from.as_str());
        let target = members.get(edge.to.as_str());
        if let Some((group, branch, index)) = source {
            ensure!(
                edge.on == EdgeOutcome::Failure
                    && edge.new_round_entry.is_none()
                    && workflow.control.max_attempts.is_some()
                    && target.is_some_and(|(g, b, i)| g == group && b == branch && i <= index),
                "branch edge `{} -> {}` must be a bounded local failure retry",
                edge.from,
                edge.to
            );
        } else {
            ensure!(
                target.is_none()
                    && !edge
                        .new_round_entry
                        .as_deref()
                        .is_some_and(|entry| members.contains_key(entry.trim())),
                "outer edge cannot enter a branch member"
            );
        }
        if matches!(nodes.get(&edge.to), Some(NodeDsl::ParallelGroup(_)))
            || matches!(nodes.get(&edge.from), Some(NodeDsl::ParallelGroup(_)))
        {
            ensure!(
                edge.session != Some(SessionMode::Continue),
                "parallel-group has no session to continue"
            );
        }
    }
    Ok(())
}

/// Compile an owned branch from the frozen executable definition, preserving each worker's
/// identity, execution configuration and explicit local retry edge.
impl ParallelBranchDsl {
    pub fn workflow(&self, parent: &ValidatedWorkflow) -> Result<WorkflowDsl> {
        let nodes = self
            .stages
            .iter()
            .map(|id| {
                parent
                    .get_node(id)
                    .cloned()
                    .ok_or_else(|| anyhow!("branch stage `{id}` missing"))
            })
            .collect::<Result<Vec<_>>>()?;
        let ids = self.stages.iter().collect::<HashSet<_>>();
        let mut edges = parent
            .raw
            .edges
            .iter()
            .filter(|edge| ids.contains(&edge.from))
            .cloned()
            .collect::<Vec<_>>();
        for (i, id) in self.stages.iter().enumerate() {
            edges.push(EdgeDsl {
                from: id.clone(),
                to: self
                    .stages
                    .get(i + 1)
                    .cloned()
                    .unwrap_or_else(|| END_NODE.into()),
                on: EdgeOutcome::Success,
                session: None,
                new_round_entry: None,
            });
        }
        Ok(WorkflowDsl {
            version: parent.raw.version.clone(),
            id: self.id.clone(),
            entry: self.stages.first().context("empty branch")?.clone(),
            control: parent.raw.control.clone(),
            nodes,
            edges,
        })
    }
}

fn validate_workflow_with_mode(
    workflow: WorkflowDsl,
    mode: WorkflowValidationMode,
) -> Result<ValidatedWorkflow> {
    ensure!(
        workflow.version == "0.1",
        "unsupported workflow version: {}",
        workflow.version
    );
    ensure!(
        !workflow.id.trim().is_empty(),
        "workflow id cannot be empty"
    );
    ensure!(
        !workflow.entry.trim().is_empty(),
        "workflow entry cannot be empty"
    );
    ensure!(
        !workflow.nodes.is_empty(),
        "workflow must contain at least one node"
    );
    if let Some(max_attempts) = workflow.control.max_attempts {
        ensure!(max_attempts > 0, "max_attempts must be a positive integer");
    }
    if let Some(max_rounds) = workflow.control.max_rounds {
        ensure!(max_rounds > 0, "max_rounds must be a positive integer");
    }

    let mut nodes_by_id = IndexMap::new();
    let mut seen_ids = HashSet::new();

    for node in &workflow.nodes {
        let id = node.id();
        ensure!(!id.trim().is_empty(), "node id cannot be empty");
        ensure!(seen_ids.insert(id.to_string()), "duplicate node id: {id}");
        ensure!(
            !RESERVED_NODE_IDS.contains(&id),
            "node id `{id}` is reserved and cannot be used"
        );

        match node {
            NodeDsl::Worker(worker) => validate_worker_node(worker, id, mode)?,
            NodeDsl::AiDynamic(dynamic) => validate_ai_dynamic_node(dynamic, id)?,
            NodeDsl::ParallelGroup(_) => {}
        }

        nodes_by_id.insert(id.to_string(), node.clone());
    }

    ensure!(
        nodes_by_id.contains_key(&workflow.entry),
        "entry node not found: {}",
        workflow.entry
    );
    validate_parallel_groups(&workflow, &nodes_by_id)?;
    let mut edge_outcomes_by_source = HashSet::new();
    let mut has_end_target = false;
    for edge in &workflow.edges {
        ensure!(
            nodes_by_id.contains_key(&edge.from),
            "edge source not found: {}",
            edge.from
        );
        ensure!(
            edge.to == END_NODE || edge.to == NEW_ROUND_NODE || nodes_by_id.contains_key(&edge.to),
            "edge target not found: {}",
            edge.to
        );
        if edge.to == END_NODE {
            has_end_target = true;
        }
        if edge.to == NEW_ROUND_NODE && edge.on == EdgeOutcome::Success {
            return Err(WorkflowValidationError::SuccessNewRoundTarget {
                from: edge.from.clone(),
            }
            .into());
        }
        if edge.to == NEW_ROUND_NODE {
            let Some(new_round_entry) = edge
                .new_round_entry
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                return Err(WorkflowValidationError::MissingNewRoundEntry {
                    from: edge.from.clone(),
                }
                .into());
            };
            if new_round_entry != ENTRY_NODE && !nodes_by_id.contains_key(new_round_entry) {
                return Err(WorkflowValidationError::InvalidNewRoundEntry {
                    from: edge.from.clone(),
                    entry: new_round_entry.to_string(),
                }
                .into());
            }
        }
        ensure!(
            edge.from != END_NODE && edge.from != NEW_ROUND_NODE,
            "edge source cannot be a terminal target: {}",
            edge.from
        );
        ensure!(
            edge_outcomes_by_source.insert((edge.from.clone(), edge.on)),
            "node `{}` already has a {:?} edge",
            edge.from,
            edge.on
        );

        if matches!(edge.session, Some(SessionMode::Continue)) {
            ensure!(
                edge.to != END_NODE && edge.to != NEW_ROUND_NODE,
                "session=continue requires a real node target"
            );
            if mode == WorkflowValidationMode::Executable {
                let target = nodes_by_id
                    .get(&edge.to)
                    .ok_or_else(|| anyhow!("edge target not found: {}", edge.to))?;
                let provider = target
                    .provider()
                    .ok_or_else(|| anyhow!("target node `{}` provider cannot be blank", edge.to))?;
                ensure!(
                    supports_continue_session(provider)?,
                    "session=continue currently only supports agents with continue-session capability"
                );
            }
        }
    }

    if !has_end_target {
        return Err(WorkflowValidationError::MissingEndNode.into());
    }

    let mut reachable = HashSet::new();
    let mut pending = vec![workflow.entry.clone()];
    while let Some(node_id) = pending.pop() {
        if !reachable.insert(node_id.clone()) {
            continue;
        }
        if let Some(NodeDsl::ParallelGroup(group)) = nodes_by_id.get(&node_id) {
            pending.extend(
                group
                    .branches
                    .iter()
                    .flat_map(|branch| branch.stages.iter().cloned()),
            );
        }
        workflow
            .edges
            .iter()
            .filter(|edge| edge.from == node_id)
            .filter(|edge| edge.to != END_NODE)
            .for_each(|edge| {
                if edge.to == NEW_ROUND_NODE {
                    let new_round_entry = edge
                        .new_round_entry
                        .as_deref()
                        .map(str::trim)
                        .unwrap_or_default();
                    pending.push(if new_round_entry == ENTRY_NODE {
                        workflow.entry.clone()
                    } else {
                        new_round_entry.to_string()
                    });
                } else {
                    pending.push(edge.to.clone());
                }
            });
    }
    if let Some(node_id) = nodes_by_id
        .keys()
        .find(|node_id| !reachable.contains(*node_id))
    {
        return Err(WorkflowValidationError::UnreachableNode {
            node_id: node_id.clone(),
        }
        .into());
    }

    Ok(ValidatedWorkflow {
        raw: workflow,
        nodes_by_id,
    })
}

pub fn validate_workflow(workflow: WorkflowDsl) -> Result<ValidatedWorkflow> {
    validate_workflow_with_mode(workflow, WorkflowValidationMode::Executable)
}

pub fn validate_authoring_workflow(workflow: WorkflowDsl) -> Result<ValidatedWorkflow> {
    validate_workflow_with_mode(workflow, WorkflowValidationMode::Authoring)
}

pub fn normalize_legacy_workflow_snapshot(mut workflow: WorkflowDsl) -> WorkflowDsl {
    for edge in &mut workflow.edges {
        if edge.to == NEW_ROUND_NODE
            && edge
                .new_round_entry
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
        {
            edge.new_round_entry = Some(ENTRY_NODE.to_string());
        }
    }
    workflow
}

pub fn validate_workflow_snapshot(workflow: WorkflowDsl) -> Result<ValidatedWorkflow> {
    validate_workflow_with_mode(
        normalize_legacy_workflow_snapshot(workflow),
        WorkflowValidationMode::Executable,
    )
}
