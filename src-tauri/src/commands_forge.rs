//! 工作流生成（Forge）Tauri 命令层（实施方案 §4）。
//!
//! 项目作用域命令（创建会话、试运行、Spec 存档与预览里的修改）经
//! `workspace_entry_for_project` 解析项目 app；用户级命令（编译保存、
//! 反投影）走主 app——模板库是用户级全局（方案 §5.2 挂靠模型）。

use gold_band::app::forge_service::CreateForgeTaskInput;
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

/// 铸造会话一轮收口事件名（前端订阅名见 `web/src/lib/app-events.ts`）。
pub const FORGE_TURN_SETTLED_EVENT: &str = "gold-band://forge-turn-settled";

/// 铸造会话一轮收口通知：这一轮的终局，以及回复是否带来了 Spec 增量。
///
/// 事件在增量提取并落档**之后**发出——通用的会话更新事件在提取前后各发一次，
/// 前端无法区分；预览 Tab 与自动修正回路以本事件为驱动信号，读到的存档一定是
/// 这一轮之后的。铸造会话的每一轮结束时都会发出（增量无法应用时带 `error`）：
/// 漏发会让自动修正回路一直等下去。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeTurnSettledEventVm {
    pub project_id: String,
    pub prompt_id: Option<String>,
    pub task_id: String,
    pub run_id: String,
    pub round_id: String,
    pub node_id: String,
    pub attempt_id: String,
    /// `completed` / `cancelled` / `failed`。
    pub outcome: &'static str,
    /// 本轮回复是否带来了可解析的 Spec 增量（为 `true` 时存档已更新）。
    pub spec_updated: bool,
    /// 回复里有增量但无法应用（如 JSON 不合法）。普通对话轮没有增量，不带错误。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ForgeTurnSettledErrorVm>,
}

/// 增量无法应用的原因：错误码 + 供对话继续时参考的明细。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeTurnSettledErrorVm {
    pub code: &'static str,
    pub detail: String,
}

/// 解析项目作用域 app：不存在的项目报结构化错误码。
fn project_scoped_app(
    context: &crate::state::DesktopContext,
    project_id: &str,
) -> CommandResult<(gold_band::app::App, String)> {
    let state = context.app().load_state().map_err(command_error)?;
    let (workspace_path, resolved_project_id) = workspace_entry_for_project(&state, project_id)
        .ok_or_else(|| {
            CommandErrorVm::new(
                "workspace.not-found",
                serde_json::json!({ "projectId": project_id }),
            )
        })?;
    let app = app_for_workspace(context, &workspace_path).map_err(command_error)?;
    Ok((app, resolved_project_id))
}

/// 入口 B/C：素材清单的结构化摘要（composer 预填用；文案由前端按语言拼装）。
#[tauri::command]
pub async fn forge_items_digest(
    state: State<'_, DesktopState>,
    project_id: String,
    items: Vec<gold_band::app::forge_service::ForgeSourceItem>,
) -> CommandResult<Vec<gold_band::app::forge_service::ForgeSourceSummary>> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.forge_items_digest(&items).map_err(command_error)
    })
    .await
}

/// 入口 A 的源会话摘要：composer 预填用（用户可编辑后手动发送；
/// 完整材料由后端在发送创建时注入隐藏区段）。`after` 置位时截断为
/// 该分支选中消息之后的可见对话（§5.1）。
#[tauri::command]
pub async fn forge_source_digest(
    state: State<'_, DesktopState>,
    project_id: String,
    task_id: String,
    session: Option<gold_band::app::forge_service::ForgeSourceSession>,
) -> CommandResult<gold_band::app::forge_service::ForgeSourceDigest> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.forge_source_digest(&task_id, session.as_ref())
            .map_err(command_error)
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
    task_uuid: String,
    spec: ForgeSpec,
) -> CommandResult<ForgeSpec> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _project_id) = project_scoped_app(&context, &project_id)?;
        app.save_forge_spec_for_instance(&task_id, &task_uuid, &spec).map_err(command_error)
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeCopyTarget {
    workflow_id: String,
    name: String,
}

/// Spec → WorkflowDsl 编译并落模板。会话已有对应模板时原位更新（对话式修改
/// 落回源模板）；custom 角色按 profileId 更新或新建。Spec 校验失败不落库
/// 任何模板，返回全部可修正错误与修正提示词；`repair_attempt` 是将要发出的
/// 修正轮次。`task_id` 指向的会话属于某个项目，必须经项目作用域 app 解析
/// （模板库本身是用户级全局）。
#[tauri::command]
pub async fn forge_compile(
    state: State<'_, DesktopState>,
    project_id: String,
    spec: ForgeSpec,
    task_id: Option<String>,
    repair_attempt: u32,
    copy: Option<ForgeCopyTarget>,
) -> CommandResult<ForgeCompileOutcomeVm> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _resolved_project_id) = project_scoped_app(&context, &project_id)?;
        let result = match (copy, task_id.as_deref()) {
            (Some(copy), Some(task_id)) => app.forge_compile_as_copy(
                &spec,
                task_id,
                &copy.workflow_id,
                &copy.name,
                repair_attempt,
            ),
            (Some(_), None) => Err(anyhow::anyhow!("save as requires a task")),
            (None, task_id) => app.forge_compile(&spec, task_id, repair_attempt),
        };
        match result.map_err(command_error)? {
            gold_band::app::forge_service::ForgeCompileOutcome::Saved(saved) => {
                let templates = app.workflow_templates().map_err(command_error)?;
                Ok(ForgeCompileOutcomeVm::Saved {
                    trial: saved.trial,
                    spec: saved.spec,
                    template_ids: saved.template_ids,
                    created_count: saved.created_count,
                    updated_count: saved.updated_count,
                    templates,
                })
            }
            gold_band::app::forge_service::ForgeCompileOutcome::Repairable {
                errors,
                repair_prompt,
            } => Ok(ForgeCompileOutcomeVm::Repairable {
                errors,
                repair_prompt,
            }),
        }
    })
    .await
}

/// 入口 C：已安装的 Claude Code plugin 及其可作为生成素材的内容（skill / command / agent）。
/// plugins 根遵循官方 `CLAUDE_CODE_PLUGIN_CACHE_DIR`，否则 `~/.claude/plugins`。
#[tauri::command]
pub async fn list_installed_plugins(
    state: State<'_, DesktopState>,
    project_id: Option<String>,
) -> CommandResult<Vec<gold_band::skill::claude_plugin::InstalledPlugin>> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let project = project_id
            .as_deref()
            .map(|id| project_scoped_app(&context, id))
            .transpose()?;
        let include_project = project.is_some();
        let app = project.map(|(app, _)| app).unwrap_or_else(|| context.app());
        Ok(gold_band::skill::claude_plugin::list_plugin_inventory(
            &app.paths,
            include_project,
        ))
    })
    .await
}

/// No WebView-provided path is a grant. Cancel and invalid selections leave the
/// previous project/global reference intact. The dialog and parsing run off the IPC thread.
#[tauri::command]
pub async fn select_workbuddy_source(
    state: State<'_, DesktopState>,
    window: tauri::WebviewWindow,
    project_id: Option<String>,
) -> CommandResult<Option<gold_band::skill::claude_plugin::InstalledPlugin>> {
    use gold_band::config::SkillSource;
    use tauri_plugin_dialog::DialogExt;
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, scope) = match project_id {
            Some(id) => (project_scoped_app(&context, &id)?.0, SkillSource::Project),
            None => (context.app().clone_for_background(), SkillSource::Global),
        };
        let Some(selected) = window
            .dialog()
            .file()
            .set_parent(&window)
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let invalid = || CommandErrorVm::new("plugin.asset-unavailable", serde_json::json!({}));
        let path = camino::Utf8PathBuf::from_path_buf(selected.into_path().map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
        gold_band::skill::claude_plugin::select_workbuddy_directory(&app.paths, scope, &path)
            .map(Some)
            .map_err(|_| invalid())
    })
    .await
}

#[tauri::command]
pub async fn forget_workbuddy_source(
    state: State<'_, DesktopState>,
    project_id: Option<String>,
    plugin_id: String,
) -> CommandResult<bool> {
    use gold_band::config::SkillSource;
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, scope) = match project_id {
            Some(id) => (project_scoped_app(&context, &id)?.0, SkillSource::Project),
            None => (context.app(), SkillSource::Global),
        };
        gold_band::skill::claude_plugin::forget_workbuddy_directory(&app.paths, scope, &plugin_id)
            .map_err(command_error)
    })
    .await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportPluginAssetInput {
    project_id: Option<String>,
    asset_id: String,
}

/// Resolve the selection against the current inventory. Neither a path from the WebView nor
/// a name supplied by a package author is used as an unchecked destination path.
#[tauri::command]
pub async fn import_plugin_asset(
    state: State<'_, DesktopState>,
    input: ImportPluginAssetInput,
) -> CommandResult<()> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let app = match &input.project_id {
            Some(project_id) => project_scoped_app(&context, project_id)?.0,
            None => context.app().clone_for_background(),
        };
        let scope = if input.project_id.is_some() {
            gold_band::config::SkillSource::Project
        } else {
            gold_band::config::SkillSource::Global
        };
        let plugins = gold_band::skill::claude_plugin::resolve_plugin_assets(
            &app.paths,
            scope,
            [input.asset_id.as_str()],
        );
        import_listed_plugin_asset(&app, &input, &plugins)
    })
    .await
}

fn import_listed_plugin_asset(
    app: &gold_band::app::App,
    input: &ImportPluginAssetInput,
    plugins: &[gold_band::skill::claude_plugin::InstalledPlugin],
) -> CommandResult<()> {
    use gold_band::skill::claude_plugin::ClaudePluginAssetKind::{Agent, Skill};
    let invalid = || CommandErrorVm::new("plugin.asset-unavailable", serde_json::json!({}));
    let scope = if input.project_id.is_some() {
        gold_band::config::SkillSource::Project
    } else {
        gold_band::config::SkillSource::Global
    };
    let (plugin, asset) = plugins
        .iter()
        .filter(|p| p.source == scope)
        .find_map(|p| {
            p.assets
                .iter()
                .find(|a| a.asset_id == input.asset_id && matches!(a.kind, Agent | Skill))
                .map(|a| (p, a))
        })
        .ok_or_else(invalid)?;
    if asset.kind == Skill {
        // Definition-only sources never recursively copy scripts, hooks or sidecars.
        let (key, content, files) = if plugin.definition_only() {
            let key = asset
                .path
                .as_deref()
                .and_then(|p| camino::Utf8Path::new(p).file_name())
                .ok_or_else(invalid)?;
            let files = plugin.declarative_skill_files(asset).ok_or_else(|| {
                CommandErrorVm::new("plugin.bundle-unreadable", serde_json::json!({}))
            })?;
            (
                key.to_owned(),
                asset.content.clone().ok_or_else(invalid)?,
                files,
            )
        } else {
            let path = plugin.asset_path(asset).ok_or_else(invalid)?;
            let key = path.file_name().ok_or_else(invalid)?;
            let bundle = crate::multica::local_skills::read_local_skill_bundle(
                &path,
                key,
                plugin.provider_id,
            )
            .map_err(|_| CommandErrorVm::new("plugin.bundle-unreadable", serde_json::json!({})))?;
            (
                key.to_owned(),
                bundle.content,
                bundle
                    .files
                    .into_iter()
                    .map(|f| (f.path, f.content))
                    .collect(),
            )
        };
        let manager =
            gold_band::skill::SkillManager::new(app.paths.clone(), app.config.agents.clone());
        manager
            .write_bundle_instance(
                &key,
                plugin.source,
                &content,
                &files,
                input
                    .project_id
                    .as_ref()
                    .map(|_| app.paths.repo_root.as_str()),
                None,
                Some(&[]),
            )
            .map_err(command_error)?;
    } else {
        let body = plugin.asset_body(asset).ok_or_else(invalid)?;
        let name = format!("{} · {}", plugin.name, asset.name);
        if app
            .profiles()
            .map_err(command_error)?
            .profiles
            .iter()
            .any(|profile| profile.name == name)
        {
            return Err(CommandErrorVm::new(
                "plugin.role-exists",
                serde_json::json!({"name":name}),
            ));
        }
        app.create_profile(gold_band::app::ProfileInput {
            name,
            summary: asset.description.clone(),
            content: body,
            dynamic_template: false,
        })
        .map_err(command_error)?;
    }
    Ok(())
}

/// `forge_compile` 的结果：落库成功，或 Spec 可修正（未落库任何模板）。
#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ForgeCompileOutcomeVm {
    #[serde(rename_all = "camelCase")]
    Saved {
        trial: Option<gold_band::app::forge_service::ForgeTrialInfo>,
        spec: Option<ForgeSpec>,
        template_ids: Vec<String>,
        created_count: usize,
        updated_count: usize,
        templates: gold_band::app::WorkflowTemplateStore,
    },
    #[serde(rename_all = "camelCase")]
    Repairable {
        errors: Vec<gold_band::app::forge_service::ForgeErrorVm>,
        repair_prompt: String,
    },
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
    revision: String,
    intent: String,
) -> CommandResult<ForgeSessionCreatedVm> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, resolved_project_id) = project_scoped_app(&context, &project_id)?;
        let summary =
            app.forge_trial(&template_id, &revision, &intent)
                .map_err(|error| {
                    match error.downcast_ref::<gold_band::app::forge_service::StaleForgeTrial>() {
                        Some(stale) => CommandErrorVm::new(
                            "forge/stale-trial",
                            serde_json::json!({ "current": stale.current }),
                        ),
                        None => command_error(error),
                    }
                })?;
        Ok(ForgeSessionCreatedVm {
            project_id: resolved_project_id,
            task_id: summary.task.id,
            title: summary.task.title,
        })
    })
    .await
}

/// 在工作流预览里直接改某个阶段的失败策略，返回合并后的 Spec。失败目标不是同一条
/// 工作流的 stage 时拒绝（`forge/unknown-failure-target`），存档不变。
#[tauri::command]
pub async fn forge_set_stage_failure(
    state: State<'_, DesktopState>,
    project_id: String,
    task_id: String,
    stage_id: String,
    on_failure: gold_band::forge::OnFailure,
) -> CommandResult<ForgeSpec> {
    let context = state.context().map_err(command_error)?;
    spawn_blocking_command(move || {
        let (app, _resolved_project_id) = project_scoped_app(&context, &project_id)?;
        app.forge_update_stage_failure(&task_id, &stage_id, on_failure)
            .map_err(command_error)
    })
    .await
}

#[cfg(test)]
mod plugin_import_tests {
    use super::*;

    #[test]
    fn workbuddy_import_preserves_declarative_support_and_refuses_script_dependencies() {
        use gold_band::{
            app::App,
            config::SkillSource::Project,
            skill::claude_plugin::{resolve_plugin_assets, select_workbuddy_directory},
        };
        const CHILD: &str = "GOLD_BAND_WORKBUDDY_IMPORT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let tmp = tempfile::Builder::new()
                .prefix("gold-band-workbuddy-import-")
                .tempdir()
                .unwrap();
            let status=gold_band::process::background_command(std::env::current_exe().unwrap()).args(["workbuddy_import_preserves_declarative_support_and_refuses_script_dependencies", "--nocapture"]).current_dir(tmp.path()).env(CHILD,"1").env("GOLD_BAND_HOME", tmp.path().join("home")).status().unwrap();
            assert!(status.success());
            return;
        }
        let base = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap();
        assert!(
            base.file_name()
                .unwrap()
                .starts_with("gold-band-workbuddy-import-")
        );
        let root = base.join("project");
        std::fs::create_dir_all(&root).unwrap();
        let app = App::new(root);
        let source = base.join("expert");
        let skill = source.join("skills/probe");
        let blocked = source.join("skills/blocked");
        for d in [
            source.join(".codebuddy-plugin"),
            skill.join("references"),
            blocked.join("scripts"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            source.join(".codebuddy-plugin/plugin.json"),
            r#"{"name":"fixture"}"#,
        )
        .unwrap();
        for d in [&skill, &blocked] {
            std::fs::write(
                d.join("SKILL.md"),
                format!(
                    "---\nname: {}\ndescription: fixture\n---\nReference",
                    d.file_name().unwrap()
                ),
            )
            .unwrap();
        }
        std::fs::write(skill.join("references/example.md"), "safe fixture").unwrap();
        std::fs::write(blocked.join("scripts/test.sh"), "not executed").unwrap();
        let listed = select_workbuddy_directory(&app.paths, Project, &source).unwrap();
        for asset in &listed.assets {
            let input = ImportPluginAssetInput {
                project_id: Some(app.paths.project_id.clone()),
                asset_id: asset.asset_id.clone(),
            };
            let import = || {
                let resolved =
                    resolve_plugin_assets(&app.paths, Project, [input.asset_id.as_str()]);
                import_listed_plugin_asset(&app, &input, &resolved)
            };
            if asset.name == "probe" {
                assert!(asset.importable);
                import().unwrap();
                assert!(import().is_err());
            } else {
                assert_eq!(asset.name, "blocked");
                assert!(!asset.importable);
                assert!(import().is_err());
            }
        }
        assert_eq!(
            std::fs::read_to_string(
                app.paths
                    .project_skills_dir()
                    .join("probe/references/example.md")
            )
            .unwrap(),
            "safe fixture"
        );
        assert!(!app.paths.project_skills_dir().join("blocked").exists());
    }

    #[test]
    fn plugin_import_preserves_skill_bundle_and_rejects_unlisted_assets() {
        let temp = tempfile::tempdir().unwrap();
        let repo = camino::Utf8PathBuf::from_path_buf(temp.path().to_owned()).unwrap();
        let plugin_root = repo.join(".claude/skills/example");
        let skill = plugin_root.join("skills/probe");
        std::fs::create_dir_all(skill.join("references")).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---
name: probe
description: Example
---
Read references/example.md",
        )
        .unwrap();
        std::fs::write(skill.join("references/example.md"), "only a fixture").unwrap();
        let plugins = vec![gold_band::skill::claude_plugin::scan_plugin_root(
            &plugin_root,
            gold_band::config::SkillSource::Project,
        )];
        let app = gold_band::app::App::new(repo.clone());
        let mut input = ImportPluginAssetInput {
            project_id: Some(app.paths.project_id.clone()),
            asset_id: "fixture-unlisted".into(),
        };
        assert!(import_listed_plugin_asset(&app, &input, &plugins).is_err());
        input.asset_id = plugins[0]
            .assets
            .iter()
            .find(|a| a.name == "probe")
            .unwrap()
            .asset_id
            .clone();
        import_listed_plugin_asset(&app, &input, &plugins).unwrap();
        let copy = app.paths.project_skills_dir().join("probe");
        assert_eq!(
            std::fs::read_to_string(copy.join("references/example.md")).unwrap(),
            "only a fixture"
        );
        std::fs::write(copy.join("references/example.md"), "local edit").unwrap();
        assert!(import_listed_plugin_asset(&app, &input, &plugins).is_err());
        assert_eq!(
            std::fs::read_to_string(copy.join("references/example.md")).unwrap(),
            "local edit"
        );
    }
}
