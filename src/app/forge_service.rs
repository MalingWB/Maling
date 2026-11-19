//! Forge 铸造的 App 层服务（方案 §6 C3/D 线）。
//!
//! - `create_forge_task`：入口 A 创建 origin=Forge 的 Task 并落初始 Spec 存档；
//! - `save_forge_spec` / `forge_spec`：每轮 Spec 变更原子写 / 会话恢复读取；
//! - `forge_trial`：以模板创建 origin=ForgeTrial 的试运行任务
//!   （control 覆写 max_rounds=1，首节点完成即停由现有 pause 机制承接）；
//! - `forge_sessions` / `cleanup_forge_sessions`：铸造历史列表与一键清理（G2）。
//!
//! 存储语义：Task/模板均为现有结构，Forge 只增加 `origin`/`source`
//! 标记与 task 级 Spec 存档（`<task_dir>/forge/spec.json`），不新增
//! canonical 数据结构（方案自评审结论）。

use crate::app::{App, CreateTaskInput, TaskSummary, WorkflowModelBindings};
use crate::forge::spec::ForgeSpec;
use crate::runtime::TaskOrigin;
use anyhow::{Context, Result, bail};

impl App {
    /// 入口 A：创建 Forge 铸造会话 Task（origin=Forge）。
    ///
    /// `context` 为源会话摘要/意图等上下文（写进 requirement，作为
    /// Forge 会话的首轮素材）。初始 Spec（可为未收敛态）落存档供
    /// 会话恢复。创建后由前端自动打开右侧“铸造预览”Tab。
    pub fn create_forge_task(&self, context: CreateForgeTaskInput) -> Result<TaskSummary> {
        if context.requirement_content.trim().is_empty() {
            bail!("forge context cannot be empty");
        }
        let summary = self.create_task_from_requirement(CreateTaskInput {
            title: context.title.filter(|value| !value.trim().is_empty()),
            description: None,
            requirement_file_name: None,
            requirement_content: context.requirement_content,
            // 铸造会话是对话型会话（RawAgent 单节点，同 Direct preset），
            // 不跑流水线图；工作流产物由编译链路另行创建。
            workflow: context
                .workflow
                .unwrap_or_else(|| forge_conversation_workflow(&context.agent_type)),
            workflow_template_id: None,
            origin: Some(TaskOrigin::Forge),
        })?;
        if let Some(spec) = &context.initial_spec {
            self.save_forge_spec(&summary.task.id, spec)?;
        }
        Ok(summary)
    }

    /// 每轮 Spec 变更后原子写存档（方案 D2）。
    pub fn save_forge_spec(&self, task_id: &str, spec: &ForgeSpec) -> Result<()> {
        let task_dir = self.paths.task_dir(task_id);
        if !task_dir.exists() {
            bail!("task `{task_id}` not found");
        }
        crate::forge::archive::save_spec_archive(&task_dir, spec)?;
        Ok(())
    }

    /// 读取 Spec 存档（会话恢复/预览 Tab 重开）。
    pub fn forge_spec(&self, task_id: &str) -> Result<Option<ForgeSpec>> {
        let task_dir = self.paths.task_dir(task_id);
        if !task_dir.exists() {
            bail!("task `{task_id}` not found");
        }
        Ok(crate::forge::archive::load_spec_archive(&task_dir)?)
    }

    /// 试运行（方案 D1）：以模板创建真实 Task（origin=ForgeTrial）。
    ///
    /// 复用 task→run 全链路，不造 dry-run 专用代码路径。“首节点即停”以
    /// 数据级截断实现：只保留 entry 节点、success 边直连 `$end`，模板
    /// 本体不动——首节点产物/契约照常验证，跑完即终止。`control` 同步
    /// 覆写 `max_rounds=1`。样例需求 = 锚定轮确认的 intent。
    pub fn forge_trial(&self, template_id: &str, intent: &str) -> Result<TaskSummary> {
        if intent.trim().is_empty() {
            bail!("forge trial intent cannot be empty");
        }
        let store = self.load_workflow_template_store_forge()?;
        let template = store
            .templates
            .iter()
            .find(|template| template.id == template_id)
            .with_context(|| format!("workflow template `{template_id}` not found"))?;
        let mut workflow = truncate_to_first_node(template.workflow.clone());
        // 试运行降级：单轮、首节点验证即停。
        workflow.control.max_rounds = Some(1);
        let bindings = with_default_agent_bindings(
            &workflow,
            template.model_bindings.clone(),
            &self.config.agents,
        );
        self.create_task_from_requirement_with_bindings(
            CreateTaskInput {
                title: Some(format!("试运行：{}", template.name)),
                description: None,
                requirement_file_name: None,
                requirement_content: intent.to_string(),
                workflow: workflow.clone(),
                workflow_template_id: Some(template_id.to_string()),
                origin: Some(TaskOrigin::ForgeTrial),
            },
            workflow,
            bindings,
        )
    }

    /// 铸造历史（方案 D3）：列出 origin=Forge/ForgeTrial 的 Task 与
    /// Spec 摘要（会话恢复入口）、产出模板组关联。
    pub fn forge_sessions(&self) -> Result<Vec<ForgeSessionEntry>> {
        let mut sessions = Vec::new();
        for task in self.task_list()? {
            let Some(origin) = task.origin else {
                continue;
            };
            let task_dir = self.paths.task_dir(&task.id);
            let spec = crate::forge::archive::load_spec_archive(&task_dir).ok().flatten();
            let spec_summary = spec.as_ref().map(|spec| ForgeSpecSummary {
                name: spec.name.clone().unwrap_or_default(),
                intent: spec.intent.clone(),
                stage_count: spec
                    .workflows
                    .iter()
                    .map(|workflow| workflow.stages.len())
                    .sum::<usize>(),
                workflow_count: spec.workflows.len(),
                open_question_count: spec.open_questions.len(),
            });
            sessions.push(ForgeSessionEntry {
                task_id: task.id.clone(),
                title: task.title.clone(),
                origin,
                spec_summary,
                updated_at: spec_archive_mtime(&task_dir),
            });
        }
        sessions.sort_by(|left, right| right.task_id.cmp(&left.task_id));
        Ok(sessions)
    }

    /// 一键清理未完成铸造与试运行 Task（G2）。
    ///
    /// 删除 task 目录（进回收站由 Tauri 层 trash 语义承接；本层为
    /// 文件系统删除 + SQLite 索引清理，与现有 delete 链路一致）。
    /// 返回实际删除的 task id 列表（不存在或非 Forge 来源的跳过）。
    pub fn cleanup_forge_sessions(&self, task_ids: &[String]) -> Result<Vec<String>> {
        let mut deleted = Vec::new();
        for task_id in task_ids {
            let Ok(task) = self.task_show(task_id) else {
                continue;
            };
            if task.origin.is_none() {
                continue;
            }
            let task_dir = self.paths.task_dir(task_id);
            if !task_dir.exists() {
                continue;
            }
            std::fs::remove_dir_all(task_dir.as_std_path())?;
            crate::storage::sqlite::delete_task(&task_dir);
            deleted.push(task_id.clone());
        }
        Ok(deleted)
    }

    /// 反投影入口（方案 §4 Tauri 命令 `forge_from_workflow` 的底层）：
    /// 现有模板 → Forge Spec，进入 Forge 会话做对话式修改。
    pub fn forge_from_workflow(&self, template_id: &str) -> Result<ForgeSpec> {
        let store = self.load_workflow_template_store_forge()?;
        let template = store
            .templates
            .iter()
            .find(|template| template.id == template_id)
            .with_context(|| format!("workflow template `{template_id}` not found"))?;
        let spec = crate::forge::project::from_dsl(
            &template.workflow,
            &template.name,
            "",
        )?;
        Ok(spec)
    }

    /// 编译并保存（方案 §4 `forge_compile` 的落库形态）。
    ///
    /// Spec → WorkflowDsl 若干条；custom role 的 pending profile 注册为
    /// User scope 后回填节点引用；产物以 `source=Forged` 存为模板,并为
    /// 缺绑定的执行槽补默认 agent 绑定（§3.2"默认模型绑定"——作者态
    /// provider 置空,migrate 不生成绑定,无默认绑定的模板无法启动）。
    /// 返回新建模板 id 列表（多工作流拆分时多个）。
    pub fn forge_compile_and_save(&self, spec: &ForgeSpec) -> Result<Vec<String>> {
        let compiled_list = crate::forge::compiler::compile(spec)
            .map_err(|error| anyhow::anyhow!("{}", error.to_string()))?;
        let mut created_template_ids = Vec::new();
        for mut compiled in compiled_list {
            backfill_pending_profiles(&self.paths, &mut compiled, &spec.intent)?;
            let bindings = with_default_agent_bindings(&compiled.workflow, WorkflowModelBindings::default(), &self.config.agents);
            let (_store, created_id) = self.save_workflow_template_with_source(
                compiled.name.clone(),
                compiled.workflow.clone(),
                bindings,
                crate::app::TemplateSource::Forged,
            )?;
            created_template_ids.push(created_id);
        }
        Ok(created_template_ids)
    }

    /// 铸造对话轮完成后的 Spec 增量提取（回路心脏，方案 §3.1 增量生长）。
    ///
    /// 读取该 attempt 时间线,把最后一个用户 prompt 之后的 `textDelta`
    /// 拼接为最后一条助手消息,提取其中最后一个 ```json 围栏作为
    /// SpecDelta,合并进存档(无存档时以单空工作流骨架起步)。普通
    /// 对话轮(无围栏)返回 Ok(None),不产生副作用。
    pub fn forge_extract_turn_delta(
        &self,
        task_id: &str,
        run_id: &str,
        round_id: &str,
        node_id: &str,
        attempt_id: &str,
    ) -> Result<Option<ForgeSpec>> {
        let task = self.task_show(task_id)?;
        if task.origin != Some(TaskOrigin::Forge) {
            return Ok(None);
        }
        let attempt_dir = self
            .paths
            .attempt_dir(task_id, run_id, round_id, node_id, attempt_id);
        let timeline_path = attempt_dir.join("acp.timeline.jsonl");
        if !timeline_path.exists() {
            return Ok(None);
        }
        let items = crate::acp::events::load_timeline_items(&timeline_path)?;
        // 最后一个用户 prompt 之后的所有 textDelta = 最后一条助手消息。
        let last_prompt_seq = items
            .iter()
            .filter(|item| {
                item.kind == "userTextDelta"
                    && item
                        .raw
                        .as_ref()
                        .and_then(|raw| raw.get("source"))
                        .and_then(|value| value.as_str())
                        == Some("goldBandPrompt")
            })
            .map(|item| item.seq)
            .max();
        let Some(last_prompt_seq) = last_prompt_seq else {
            return Ok(None);
        };
        let mut assistant_text = String::new();
        for item in items.iter().filter(|item| item.seq > last_prompt_seq) {
            if item.kind == "textDelta" {
                if let Some(chunk) = item.content.as_deref() {
                    assistant_text.push_str(chunk);
                }
            }
        }
        let Some(delta_json) = extract_last_fenced_json(&assistant_text) else {
            return Ok(None);
        };
        let delta = crate::forge::interview::SpecDelta::parse(&delta_json)
            .map_err(|error| anyhow::anyhow!("{}", error.to_string()))?;
        let base = crate::forge::archive::load_spec_archive(&self.paths.task_dir(task_id))?
            .unwrap_or_else(initial_forge_spec_baseline);
        let merged = crate::forge::interview::merge_delta(&base, &delta)
            .map_err(|error| anyhow::anyhow!("{}", error.to_string()))?;
        crate::forge::archive::save_spec_archive(&self.paths.task_dir(task_id), &merged)
            .map_err(|error| anyhow::anyhow!("{}", error.to_string()))?;
        Ok(Some(merged))
    }

    /// 源会话材料（入口 A）：标题 + 需求 + 用户消息，按 G5 预算截断。
    ///
    /// 可见摘要供 composer 预填（用户可编辑），完整材料作为首条
    /// prompt 的隐藏区段注入（`prepare_conversation_task` 消费）。
    pub fn forge_source_material(&self, source_task_id: &str) -> Result<ForgeSourceMaterial> {
        let task_dir = self.paths.task_dir(source_task_id);
        let task: crate::runtime::TaskState = crate::storage::read_json(&task_dir.join("task.json"))
            .with_context(|| format!("forge source task `{source_task_id}` not found"))?;
        let title = task.title.unwrap_or_else(|| source_task_id.to_string());
        let requirement = std::fs::read_to_string(
            task_dir
                .join("authoring")
                .join("requirement.md")
                .as_std_path(),
        )
        .unwrap_or_default();
        let prompts = crate::storage::sqlite::search_index()
            .map(|index| index.prompts_for_task(source_task_id, HIDDEN_PROMPT_CHAR_CAP))
            .transpose()
            .map_err(|error| anyhow::anyhow!("failed to load source prompts: {error}"))?
            .unwrap_or_default();
        let mut content = String::new();
        content.push_str(&format!("# {title}\n\n"));
        if !requirement.trim().is_empty() {
            let cap = HIDDEN_TOTAL_CHAR_CAP / 2;
            let requirement = char_truncate(&requirement, cap);
            content.push_str("## 需求\n\n");
            content.push_str(&requirement);
            content.push_str("\n\n");
        }
        if !prompts.is_empty() {
            content.push_str("## 用户消息（时间序）\n\n");
            for prompt in prompts {
                let prompt = prompt.trim();
                if prompt.is_empty() {
                    continue;
                }
                content.push_str("- ");
                content.push_str(prompt);
                content.push('\n');
            }
        }
        let content = char_truncate(&content, HIDDEN_TOTAL_CHAR_CAP);
        Ok(ForgeSourceMaterial {
            title,
            hidden_section: crate::provider::PromptHiddenSection {
                title: "源会话材料".to_string(),
                content,
            },
        })
    }

    /// composer 预填用的可见摘要（比隐藏材料更短：需求 + 少量近期消息）。
    pub fn forge_source_digest(&self, source_task_id: &str) -> Result<ForgeSourceDigest> {
        let material = self.forge_source_material(source_task_id)?;
        // 可见摘要只保留标题与需求段（用户消息全量走隐藏注入）。
        let requirement = material
            .hidden_section
            .content
            .split("## 用户消息")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        Ok(ForgeSourceDigest {
            title: material.title,
            markdown: char_truncate(&requirement, DIGEST_CHAR_CAP),
        })
    }

    /// workflow-forge 角色（按界面语言解析内置 profile）。
    ///
    /// 角色正文后拼接渲染后的 Spec 输出协议（含 JSON Schema 与空基线
    /// 状态）——profile 正文只描述策略,增量格式契约由协议模板承载,
    /// 模型自首轮起即可输出围栏 JSON 增量（B2 的注入点）。
    pub(crate) fn workflow_forge_role(&self) -> Result<crate::provider::UserPromptRole> {
        let profile = crate::app::profiles::find_profile_by_id(
            &self.paths,
            WORKFLOW_FORGE_PROFILE_ID,
            self.config.desktop_language,
        )?
        .ok_or_else(|| {
            anyhow::anyhow!("built-in profile `{WORKFLOW_FORGE_PROFILE_ID}` is unavailable")
        })?;
        let protocol_template =
            crate::prompts::FORGE_SPEC_PROTOCOL.resolve(self.config.desktop_language);
        let schema = serde_json::to_string_pretty(&crate::forge::spec::spec_json_schema())
            .unwrap_or_default();
        let rendered = crate::prompts::render(
            protocol_template,
            serde_json::json!({
                "spec_schema": schema,
                "current_spec": "（新会话：基线为空,首条增量将创建初始阶段）",
            }),
        )?;
        Ok(crate::provider::UserPromptRole {
            profile_id: profile.id,
            name: profile.name,
            content: format!("{}\n\n---\n\n{}", profile.content, rendered),
        })
    }

    fn load_workflow_template_store_forge(
        &self,
    ) -> Result<crate::app::WorkflowTemplateStore> {
        // 复用现有模板加载入口（含内置模板物化与迁移）。
        self.workflow_templates()
    }
}

/// 提取文本中最后一个 ```json 围栏的内容(首行 ```json 起,围栏闭合止)。
fn extract_last_fenced_json(text: &str) -> Option<String> {
    let mut fenced: Option<String> = None;
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !in_fence {
            if trimmed.starts_with("```json") {
                in_fence = true;
                fenced = Some(String::new());
            }
        } else if trimmed.starts_with("```") {
            in_fence = false;
        } else if let Some(buffer) = fenced.as_mut() {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    // 未闭合的围栏(流中断)丢弃,不做半截解析。
    if in_fence {
        return None;
    }
    fenced
        .filter(|buffer| !buffer.trim().is_empty())
}

/// 增量合并的初始基线：单空工作流承载第一轮增量(merge 要求 workflows
/// 非空;骨架字段由后续增量逐轮覆盖)。
fn initial_forge_spec_baseline() -> ForgeSpec {
    ForgeSpec {
        spec_version: crate::forge::SPEC_VERSION.to_string(),
        name: None,
        intent: String::new(),
        source_skills: vec![],
        workflows: vec![crate::forge::spec::ForgeWorkflow {
            name: "未命名工作流".to_string(),
            stages: vec![],
        }],
        open_questions: vec![],
    }
}

/// 源会话材料（入口 A,G5 预算内截断）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSourceMaterial {
    pub title: String,
    /// 随首条 prompt 注入的隐藏区段。
    pub hidden_section: crate::provider::PromptHiddenSection,
}

/// composer 预填用的可见摘要（比隐藏材料更短）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSourceDigest {
    pub title: String,
    pub markdown: String,
}

/// 内置 workflow-forge profile id（`DEFAULT_PROFILE_SEEDS`）。
pub const WORKFLOW_FORGE_PROFILE_ID: &str = "pf-builtin-workflow-forge";
/// 隐藏注入预算（G5：单条用户消息字符上限）。
const HIDDEN_PROMPT_CHAR_CAP: usize = 2_000;
/// 隐藏注入总预算（约 50KB 上限,UTF-8 中文约 3 字节/字符）。
const HIDDEN_TOTAL_CHAR_CAP: usize = 16_000;
/// composer 预填摘要上限。
const DIGEST_CHAR_CAP: usize = 2_000;

/// 按字符数截断（不切断 UTF-8 边界）。
fn char_truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    text.chars().take(cap).collect()
}

/// `create_forge_task` 输入。
#[derive(Debug, Clone)]
pub struct CreateForgeTaskInput {
    pub title: Option<String>,
    /// 源会话摘要/意图上下文（写入 requirement 作为首轮素材）。
    pub requirement_content: String,
    /// 承载 Forge 对话的 agent 类型（同 Direct 会话的 agent 绑定）。
    pub agent_type: String,
    /// 铸造会话初始工作流（可选；缺省用 Direct 同构的对话型单节点）。
    pub workflow: Option<crate::dsl::WorkflowDsl>,
    /// 初始 Spec（锚定轮产出；可为未收敛态）。
    pub initial_spec: Option<ForgeSpec>,
}

/// 铸造历史条目（方案 D3）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSessionEntry {
    pub task_id: String,
    pub title: Option<String>,
    pub origin: TaskOrigin,
    /// Spec 存档摘要（无存档时 None：会话尚未产出 Spec）。
    pub spec_summary: Option<ForgeSpecSummary>,
    /// Spec 存档最后修改时间（epoch 秒文本；无存档为空）。
    pub updated_at: String,
}

/// Spec 存档摘要（列表投影，不加载全量 Spec）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSpecSummary {
    pub name: String,
    pub intent: String,
    pub stage_count: usize,
    pub workflow_count: usize,
    pub open_question_count: usize,
}

/// Forge 铸造会话的工作流：与 Direct 会话同构的对话型单节点
/// （RawAgent、无 role 绑定），会话本体不跑流水线图，仅承载对话。
fn forge_conversation_workflow(agent_type: &str) -> crate::dsl::WorkflowDsl {
    crate::dsl::presets::direct_workflow(
        agent_type.to_string(),
        None,
        None,
        false,
        Default::default(),
    )
}

/// 为缺绑定的执行槽补默认 agent 绑定（§3.2 默认模型绑定）。
///
/// 铸造节点作者态 provider 置空，`migrate_authoring_workflow` 只分配 slot
/// 不生成绑定；无绑定的模板/任务启动时报
/// `workflow-model-binding.agent-required`。默认 agent 取配置中排序首个
/// managed agent（与前端 composer 的 agents[0] 兜底一致）；无任何 agent
/// 时保持原样（G1 提醒路径）。用户可在工作流编辑器改绑。
fn with_default_agent_bindings(
    workflow: &crate::dsl::WorkflowDsl,
    mut bindings: WorkflowModelBindings,
    agents: &std::collections::BTreeMap<crate::config::ManagedAgentId, crate::config::ManagedAgentConfig>,
) -> WorkflowModelBindings {
    let Some(default_agent) = agents.keys().next().map(|id| id.as_str().to_string()) else {
        return bindings;
    };
    for node in &workflow.nodes {
        let crate::dsl::NodeDsl::Worker(worker) = node else {
            continue;
        };
        let Some(slot) = worker
            .execution_slot_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let occupied = bindings
            .bindings
            .iter()
            .any(|binding| binding.execution_slot_id == slot);
        if !occupied {
            bindings.bindings.push(crate::workflow_model_binding::WorkerModelBinding {
                execution_slot_id: slot.to_string(),
                agent_id: default_agent.clone(),
                model_id: None,
                permission_mode_id: None,
                auto_accept: false,
                config_options: Default::default(),
                model_bound_overrides: Default::default(),
            });
        }
    }
    bindings
}

/// custom role 的 pending profile 注册为 User scope 并回填节点引用。
///
/// 编译器为 custom stage 置占位 profile（`<node_id>-profile`，无法通过
/// 保存链路的 profile 解析）；此处创建真实 profile 后整体替换。
fn backfill_pending_profiles(
    paths: &crate::storage::GoldBandPaths,
    compiled: &mut crate::forge::compiler::CompiledWorkflow,
    intent: &str,
) -> Result<()> {
    for (node_id, profile_name, goal_body) in compiled.pending_profiles.clone() {
        let placeholder = format!("{node_id}-profile");
        let summary: String = goal_body
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or(intent)
            .trim()
            .chars()
            .take(80)
            .collect();
        let entry = crate::app::profiles::create_profile(
            paths,
            crate::app::profiles::ProfileInput {
                name: profile_name.clone(),
                summary,
                content: goal_body.clone(),
                dynamic_template: false,
            },
        )
        .with_context(|| format!("failed to create forged profile `{profile_name}`"))?;
        for node in &mut compiled.workflow.nodes {
            if node.id() == node_id {
                if let crate::dsl::NodeDsl::Worker(worker) = node {
                    if worker.profile.as_deref() == Some(placeholder.as_str()) {
                        worker.profile = Some(entry.id.clone());
                    }
                }
            }
        }
    }
    Ok(())
}

/// 试运行截断：仅保留 entry 节点，success 边直连 `$end`。
///
/// 数据级变换（模板本体不动），复用现有 runtime 全链路：首节点按原
/// 产物契约执行与验证，完成后无后继节点即停——等价于“真实运行的
/// 第一环”验证语义（方案 §9.4）。
fn truncate_to_first_node(mut workflow: crate::dsl::WorkflowDsl) -> crate::dsl::WorkflowDsl {
    use crate::dsl::{EdgeDsl, EdgeOutcome, END_NODE};
    let entry = workflow.entry.clone();
    workflow.nodes.retain(|node| node.id() == entry);
    workflow.edges = vec![EdgeDsl {
        from: entry,
        to: END_NODE.to_string(),
        on: EdgeOutcome::Success,
        session: None,
        new_round_entry: None,
    }];
    workflow
}

/// Spec 存档最后修改时间（epoch 秒文本；无存档为空字符串）。
fn spec_archive_mtime(task_dir: &camino::Utf8Path) -> String {
    let modified = crate::forge::archive::spec_archive_path(task_dir)
        .metadata()
        .and_then(|meta| meta.modified())
        .ok();
    modified
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{env_guard, test_app};
    use crate::forge::spec::{
        ForgeStage, ForgeWorkflow, StageOutputKind, StageRole, OnFailure, SPEC_VERSION,
    };
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    /// 与兄弟测试模块同惯例:guard + tempdir 由测试全程持有。
    fn forge_test_app() -> (App, tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let guard = env_guard();
        let temp = tempdir().unwrap();
        let repo_root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap();
        (test_app(repo_root), temp, guard)
    }

    fn sample_spec() -> ForgeSpec {
        ForgeSpec {
            spec_version: SPEC_VERSION.to_string(),
            name: Some("样本流程".to_string()),
            intent: "从会话到工作流".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                name: "样本流程".to_string(),
                stages: vec![ForgeStage {
                    id: Some("s1".to_string()),
                    role: StageRole::Dev,
                    goal: "<输入>会话上下文\n<产出>结构化产物\n<边界>不做部署\n<验收>产物过schema".to_string(),
                    artifact: Some("result".to_string()),
                    output_kind: StageOutputKind::Json,
                    schema_hint: Some(serde_json::json!({ "ok": "boolean" })),
                    success_condition: Some("$.ok == true".to_string()),
                    on_failure: OnFailure::Retry,
                    manual_check: false,
                }],
            }],
            open_questions: vec![],
        }
    }

    #[test]
    fn forge_task_lifecycle_create_spec_roundtrip_cleanup() {
        let (app, _temp, _guard) = forge_test_app();
        // 创建
        let summary = app
            .create_forge_task(CreateForgeTaskInput {
                title: Some("铸造会话".to_string()),
                requirement_content: "源会话摘要".to_string(),
                agent_type: "claude-acp".to_string(),
                workflow: None,
                initial_spec: Some(sample_spec()),
            })
            .unwrap();
        let task_id = &summary.task.id;
        assert_eq!(summary.task.origin, Some(TaskOrigin::Forge));
        // Spec 存档 round-trip
        let loaded = app.forge_spec(task_id).unwrap().expect("spec archived");
        assert_eq!(loaded.intent, "从会话到工作流");
        // 每轮变更覆写
        let mut revised = loaded;
        revised.intent = "新意图".to_string();
        app.save_forge_spec(task_id, &revised).unwrap();
        assert_eq!(app.forge_spec(task_id).unwrap().unwrap().intent, "新意图");
        // 铸造历史列表包含该会话
        let sessions = app.forge_sessions().unwrap();
        let entry = sessions
            .iter()
            .find(|session| session.task_id == *task_id)
            .expect("forge session listed");
        assert_eq!(entry.spec_summary.as_ref().unwrap().stage_count, 1);
        // 清理
        let deleted = app.cleanup_forge_sessions(&[task_id.clone()]).unwrap();
        assert_eq!(deleted, vec![task_id.clone()]);
        assert!(app.task_show(task_id).is_err());
    }

    #[test]
    fn forge_task_rejects_empty_context() {
        let (app, _temp, _guard) = forge_test_app();
        let error = app
            .create_forge_task(CreateForgeTaskInput {
                title: None,
                requirement_content: "   ".to_string(),
                agent_type: "claude-acp".to_string(),
                workflow: None,
                initial_spec: None,
            })
            .unwrap_err();
        assert!(error.to_string().contains("cannot be empty"));
    }

    #[test]
    fn cleanup_skips_normal_tasks() {
        let (app, _temp, _guard) = forge_test_app();
        let normal = app
            .create_task_from_requirement(CreateTaskInput {
                title: Some("普通任务".to_string()),
                description: None,
                requirement_file_name: None,
                requirement_content: "需求".to_string(),
                workflow: forge_conversation_workflow("claude-acp"),
                workflow_template_id: None,
                origin: None,
            })
            .unwrap();
        let deleted = app
            .cleanup_forge_sessions(&[normal.task.id.clone()])
            .unwrap();
        assert!(deleted.is_empty());
        assert!(app.task_show(&normal.task.id).is_ok());
    }

    #[test]
    fn forge_sessions_exclude_normal_tasks() {
        let (app, _temp, _guard) = forge_test_app();
        let _normal = app
            .create_task_from_requirement(CreateTaskInput {
                title: Some("普通任务2".to_string()),
                description: None,
                requirement_file_name: None,
                requirement_content: "需求".to_string(),
                workflow: forge_conversation_workflow("claude-acp"),
                workflow_template_id: None,
                origin: None,
            })
            .unwrap();
        let sessions = app.forge_sessions().unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn forge_trial_uses_template_with_single_round() {
        let (app, _temp, _guard) = forge_test_app();
        let store = app.workflow_templates().unwrap();
        let template = &store.templates[0];
        let summary = app
            .forge_trial(&template.id, "试运行样例需求")
            .unwrap();
        assert_eq!(summary.task.origin, Some(TaskOrigin::ForgeTrial));
        let authoring: crate::workflow_model_binding::TaskAuthoringWorkflow = crate::storage::read_json(
            &app.paths.workflow_file(&summary.task.id),
        )
        .unwrap();
        // 试运行 control 降级为 max_rounds=1。
        assert_eq!(authoring.workflow.control.max_rounds, Some(1));
        // 首节点即停:仅保留 entry 节点,success 边直连 $end。
        assert_eq!(authoring.workflow.nodes.len(), 1);
        assert_eq!(authoring.workflow.nodes[0].id(), template.workflow.entry);
        assert_eq!(authoring.workflow.edges.len(), 1);
        assert_eq!(authoring.workflow.edges[0].to, crate::dsl::END_NODE);
        // 模板本体未被截断改动。
        let store_after = app.workflow_templates().unwrap();
        let template_after = store_after
            .templates
            .iter()
            .find(|t| t.id == template.id)
            .unwrap();
        assert_eq!(template_after.workflow.nodes.len(), template.workflow.nodes.len());
    }

    #[test]
    fn forge_trial_rejects_unknown_template_and_empty_intent() {
        let (app, _temp, _guard) = forge_test_app();
        assert!(app.forge_trial("no-such-template", "x").is_err());
        let store = app.workflow_templates().unwrap();
        assert!(app.forge_trial(&store.templates[0].id, "  ").is_err());
    }

    #[test]
    fn forge_extract_turn_delta_merges_fenced_spec_into_archive() {
        let (app, _temp, _guard) = forge_test_app();
        let summary = app
            .create_forge_task(CreateForgeTaskInput {
                title: Some("铸造会话".to_string()),
                requirement_content: "源会话摘要".to_string(),
                agent_type: "claude-acp".to_string(),
                workflow: None,
                initial_spec: None,
            })
            .unwrap();
        let task_id = summary.task.id.clone();

        // 写入伪造 attempt 时间线:用户 prompt + 助手增量(含围栏 JSON 增量)。
        let attempt_dir = app
            .paths
            .attempt_dir(&task_id, "run-1", "round-1", "node-1", "attempt-1");
        std::fs::create_dir_all(attempt_dir.as_std_path()).unwrap();
        let events = vec![
            acp_ui_event(1, "userTextDelta", Some("继续"), Some(serde_json::json!({ "source": "goldBandPrompt" }))),
            acp_ui_event(2, "textDelta", Some("本轮结论:\n```json\n"), None),
            acp_ui_event(3, "textDelta", Some("{\"intent\":\"实现登录\",\"stages\":[{\"role\":\"dev\",\"goal\":\"<输入>需求\\n<产出>代码\\n<边界>不做部署\\n<验收>测试通过\",\"outputKind\":\"none\",\"onFailure\":\"retry\",\"manualCheck\":false}]}"), None),
            acp_ui_event(4, "textDelta", Some("\n```\n以上是增量。"), None),
        ];
        let mut jsonl = String::new();
        for event in events {
            jsonl.push_str(
                &serde_json::to_string(&crate::acp::events::AcpTimelineItem { item: event })
                    .unwrap(),
            );
            jsonl.push('\n');
        }
        std::fs::write(attempt_dir.join("acp.timeline.jsonl").as_std_path(), jsonl).unwrap();

        let merged = app
            .forge_extract_turn_delta(&task_id, "run-1", "round-1", "node-1", "attempt-1")
            .unwrap()
            .expect("delta merged");
        assert_eq!(merged.intent, "实现登录");
        assert_eq!(merged.workflows[0].stages.len(), 1);
        // 存档可恢复,内容一致。
        let archived = app.forge_spec(&task_id).unwrap().unwrap();
        assert_eq!(archived.intent, merged.intent);
        assert_eq!(archived.workflows[0].stages.len(), 1);
    }

    #[test]
    fn forge_extract_turn_delta_ignores_plain_turns_and_normal_tasks() {
        let (app, _temp, _guard) = forge_test_app();
        // 普通任务:即便有带围栏的时间线也不处理。
        let normal = app
            .create_task_from_requirement(CreateTaskInput {
                title: Some("普通".to_string()),
                description: None,
                requirement_file_name: None,
                requirement_content: "需求".to_string(),
                workflow: forge_conversation_workflow("claude-acp"),
                workflow_template_id: None,
                origin: None,
            })
            .unwrap();
        let attempt_dir = app
            .paths
            .attempt_dir(&normal.task.id, "run-1", "round-1", "node-1", "attempt-1");
        std::fs::create_dir_all(attempt_dir.as_std_path()).unwrap();
        std::fs::write(
            attempt_dir.join("acp.timeline.jsonl").as_std_path(),
            format!(
                "{}\n",
                serde_json::to_string(&crate::acp::events::AcpTimelineItem {
                    item: acp_ui_event(1, "textDelta", Some("```json\n{}\\n```"), None),
                })
                .unwrap()
            ),
        )
        .unwrap();
        assert!(
            app.forge_extract_turn_delta(&normal.task.id, "run-1", "round-1", "node-1", "attempt-1")
                .unwrap()
                .is_none()
        );

        // Forge 会话的普通轮(无围栏):零副作用。
        let forge = app
            .create_forge_task(CreateForgeTaskInput {
                title: None,
                requirement_content: "摘要".to_string(),
                agent_type: "claude-acp".to_string(),
                workflow: None,
                initial_spec: None,
            })
            .unwrap();
        let attempt_dir = app
            .paths
            .attempt_dir(&forge.task.id, "run-1", "round-1", "node-1", "attempt-1");
        std::fs::create_dir_all(attempt_dir.as_std_path()).unwrap();
        std::fs::write(
            attempt_dir.join("acp.timeline.jsonl").as_std_path(),
            format!(
                "{}\n{}\n",
                serde_json::to_string(&crate::acp::events::AcpTimelineItem {
                    item: acp_ui_event(
                        1,
                        "userTextDelta",
                        Some("问"),
                        Some(serde_json::json!({ "source": "goldBandPrompt" })),
                    ),
                })
                .unwrap(),
                serde_json::to_string(&crate::acp::events::AcpTimelineItem {
                    item: acp_ui_event(2, "textDelta", Some("普通回答,无增量"), None),
                })
                .unwrap()
            ),
        )
        .unwrap();
        assert!(
            app.forge_extract_turn_delta(&forge.task.id, "run-1", "round-1", "node-1", "attempt-1")
                .unwrap()
                .is_none()
        );
        assert!(app.forge_spec(&forge.task.id).unwrap().is_none());
    }

    #[test]
    fn forge_trial_fills_default_agent_binding_for_unbound_slots() {
        let (app, _temp, _guard) = forge_test_app();
        // 内置模板节点带 slot 但绑定可能为空;试运行必须能启动(§3.2 默认绑定)。
        let store = app.workflow_templates().unwrap();
        let template = &store.templates[0];
        let summary = app.forge_trial(&template.id, "试运行").unwrap();
        let authoring: crate::workflow_model_binding::TaskAuthoringWorkflow = crate::storage::read_json(
            &app.paths.workflow_file(&summary.task.id),
        )
        .unwrap();
        let slots: Vec<String> = authoring
            .workflow
            .nodes
            .iter()
            .filter_map(|node| match node {
                crate::dsl::NodeDsl::Worker(worker) => worker.execution_slot_id.clone(),
                _ => None,
            })
            .collect();
        assert!(!slots.is_empty());
        for slot in &slots {
            assert!(
                authoring
                    .model_bindings
                    .bindings
                    .iter()
                    .any(|binding| &binding.execution_slot_id == slot),
                "slot {slot} must have a default binding"
            );
        }
    }

    fn acp_ui_event(
        seq: u64,
        kind: &str,
        content: Option<&str>,
        raw: Option<serde_json::Value>,
    ) -> crate::acp::events::AcpUiEvent {
        crate::acp::events::AcpUiEvent {
            id: format!("{kind}-{seq}"),
            seq,
            timestamp: format!("2026-11-12T00:00:{seq:02}Z"),
            kind: kind.to_string(),
            session_id: Some("session-1".to_string()),
            content: content.map(str::to_string),
            title: None,
            tool_call_id: None,
            status: None,
            started_seq: None,
            ended_seq: None,
            started_at: None,
            ended_at: None,
            timing: None,
            raw,
        }
    }

    #[test]
    fn forge_compile_and_save_creates_forged_template_with_builtin_profiles() {
        let (app, _temp, _guard) = forge_test_app();
        let ids = app.forge_compile_and_save(&sample_spec()).unwrap();
        assert_eq!(ids.len(), 1);
        let store = app.workflow_templates().unwrap();
        let template = store
            .templates
            .iter()
            .find(|t| t.id == ids[0])
            .expect("forged template saved");
        assert_eq!(template.source, Some(crate::app::TemplateSource::Forged));
        // 节点引用内置 profile 完整 id（可通过保存链路的 profile 解析）。
        let worker_profile = match (&template.workflow.nodes[0], ()) {
            (crate::dsl::NodeDsl::Worker(worker), ()) => worker.profile.clone(),
            _ => None,
        };
        assert_eq!(worker_profile.as_deref(), Some("pf-builtin-dev"));
    }

    #[test]
    fn forge_compile_and_save_backfills_custom_profile() {
        let (app, _temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        spec.workflows[0].stages[0].role = StageRole::Custom {
            profile_name: "接口联调".to_string(),
        };
        let ids = app.forge_compile_and_save(&spec).unwrap();
        let store = app.workflow_templates().unwrap();
        let template = store
            .templates
            .iter()
            .find(|t| t.id == ids[0])
            .expect("forged template saved");
        let crate::dsl::NodeDsl::Worker(worker) = &template.workflow.nodes[0] else {
            panic!("expected worker node");
        };
        // 占位被回填为真实 User scope profile id，且可解析。
        let profile = worker.profile.as_deref().expect("profile backfilled");
        assert_ne!(profile, "s1-profile");
        assert!(profile.starts_with("pf-"));
        assert!(
            crate::app::profiles::find_profile_by_id(
                &app.paths,
                profile,
                crate::config::DesktopLanguage::ZhCn,
            )
            .unwrap()
            .is_some()
        );
    }
}
