//! 工作流铸造（Forge）Tauri 命令层（实施方案 §4）。
//!
//! 项目作用域命令（创建会话、试运行、铸造历史、清理、Spec 存档）经
//! `workspace_entry_for_project` 解析项目 app；用户级命令（编译保存、
//! 反投影）走主 app——模板库是用户级全局（方案 §5.2 挂靠模型）。

use gold_band::app::forge_service::{
    CreateForgeTaskInput, ForgeSessionEntry,
};
use gold_band::forge::spec::ForgeSpec;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::commands::{CommandErrorVm, CommandResult, command_error, spawn_blocking_command};
use crate::conversation_workspace::{app_for_workspace, workspace_entry_for_project};
use crate::state::DesktopState;

/// `forge_create_session` 输入。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeCreateSessionInputVm {
    pub project_id: String,
    /// 承载 Forge 对话的 agent 类型（同 Direct 会话的 agent 绑定）。
    pub agent_type: String,
    pub title: Option<String>,
    /// 源会话摘要/skill 清单等引导上下文（写进 requirement 作为首轮素材；
    /// composer 预填由前端持有，用户手动发送触发铸造）。
    pub context_content: String,
}

/// `forge_create_session` 输出。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSessionCreatedVm {
    pub project_id: String,
    pub task_id: String,
    pub title: Option<String>,
}

/// `forge_compile` 输出。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeCompileResultVm {
    pub created_template_ids: Vec<String>,
    /// 保存后的模板库全量（前端直接刷新模板列表）。
    pub templates: gold_band::app::WorkflowTemplateStore,
}

/// 解析项目作用域 app：不存在的项目报结构化错误码。
fn project_scoped_app(
    context: &crate::state::DesktopContext,
    project_id: &str,
) -> CommandResult<(gold_band::app::App, String)> {
    let state = context.app().load_state().map_err(command_error)?;
    let (workspace_path, resolved_project_id) =
        workspace_entry_for_project(&state, project_id).ok_or_else(|| {
            CommandErrorVm::new(
                "workspace.not-found",
                serde_json::json!({ "projectId": project_id }),
            )
        })?;
    let app = app_for_workspace(context, &workspace_path).map_err(command_error)?;
    Ok((app, resolved_project_id))
}

/// 入口 A 的源会话摘要：composer 预填用（用户可编辑后手动发送；
/// 完整材料由后端在发送创建时注入隐藏区段）。
#[tauri::command]
pub async fn forge_source_digest(
    state: State<'_, DesktopState>,
    project_id: String,
    task_id: String,
) -> CommandResult<gold_band::app::forge_service::ForgeSourceDigest> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.forge_source_digest(&task_id).map_err(command_error)
    })
    .await
}

/// 入口 A：创建 Forge 铸造会话（origin=Forge，对话型单节点工作流）。
#[tauri::command]
pub async fn forge_create_session(
    state: State<'_, DesktopState>,
    input: ForgeCreateSessionInputVm,
) -> CommandResult<ForgeSessionCreatedVm> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, project_id) = project_scoped_app(&context, &input.project_id)?;
        let summary = app
            .create_forge_task(CreateForgeTaskInput {
                title: input.title,
                requirement_content: input.context_content,
                agent_type: input.agent_type,
                workflow: None,
                initial_spec: None,
            })
            .map_err(command_error)?;
        Ok(ForgeSessionCreatedVm {
            project_id,
            task_id: summary.task.id,
            title: summary.task.title,
        })
    })
    .await
}

/// 读取 Spec 存档（会话恢复/预览 Tab 重开）。
#[tauri::command]
pub async fn forge_get_spec(
    state: State<'_, DesktopState>,
    project_id: String,
    task_id: String,
) -> CommandResult<Option<ForgeSpec>> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.forge_spec(&task_id).map_err(command_error)
    })
    .await
}

/// 每轮 Spec 变更后原子写存档。
#[tauri::command]
pub async fn forge_save_spec(
    state: State<'_, DesktopState>,
    project_id: String,
    task_id: String,
    spec: ForgeSpec,
) -> CommandResult<()> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.save_forge_spec(&task_id, &spec).map_err(command_error)
    })
    .await
}

/// Spec → WorkflowDsl 编译并落模板（source=Forged）；custom role 的
/// pending profile 一并注册。用户级（模板库全局）。
#[tauri::command]
pub async fn forge_compile(
    state: State<'_, DesktopState>,
    spec: ForgeSpec,
) -> CommandResult<ForgeCompileResultVm> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let app = context.app();
        let created_template_ids = app
            .forge_compile_and_save(&spec)
            .map_err(command_error)?;
        let templates = app.workflow_templates().map_err(command_error)?;
        Ok(ForgeCompileResultVm {
            created_template_ids,
            templates,
        })
    })
    .await
}

/// 现有模板 → Forge Spec（对话式修改的反投影入口）。用户级。
#[tauri::command]
pub async fn forge_from_workflow(
    state: State<'_, DesktopState>,
    template_id: String,
) -> CommandResult<ForgeSpec> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let app = context.app();
        app.forge_from_workflow(&template_id).map_err(command_error)
    })
    .await
}

/// 试运行：以模板创建 origin=ForgeTrial 的真实任务（首节点即停），
/// 样例需求 = 锚定轮确认的 intent。
#[tauri::command]
pub async fn forge_start_trial(
    state: State<'_, DesktopState>,
    project_id: String,
    template_id: String,
    intent: String,
) -> CommandResult<ForgeSessionCreatedVm> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, resolved_project_id) = project_scoped_app(&context, &project_id)?;
        let summary = app
            .forge_trial(&template_id, &intent)
            .map_err(command_error)?;
        Ok(ForgeSessionCreatedVm {
            project_id: resolved_project_id,
            task_id: summary.task.id,
            title: summary.task.title,
        })
    })
    .await
}

/// 铸造历史：项目下全部 origin=Forge/ForgeTrial 会话与 Spec 摘要（G2）。
#[tauri::command]
pub async fn forge_list_sessions(
    state: State<'_, DesktopState>,
    project_id: String,
) -> CommandResult<Vec<ForgeSessionEntry>> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.forge_sessions().map_err(command_error)
    })
    .await
}

/// 一键清理未完成铸造与试运行任务（G2；仅 Forge 来源可删）。
#[tauri::command]
pub async fn forge_cleanup_sessions(
    state: State<'_, DesktopState>,
    project_id: String,
    task_ids: Vec<String>,
) -> CommandResult<Vec<String>> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.cleanup_forge_sessions(&task_ids).map_err(command_error)
    })
    .await
}
