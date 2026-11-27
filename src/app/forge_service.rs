//! Forge 铸造的 App 层服务（方案 §6 C3/D 线）。
//!
//! - `create_forge_task`：入口 A 创建 origin=Forge 的 Task 并落初始 Spec 存档；
//! - `save_forge_spec` / `forge_spec`：每轮 Spec 变更原子写 / 会话恢复读取；
//! - `forge_trial`：以模板创建 origin=ForgeTrial 的试运行任务
//!   （control 覆写 max_rounds=1，首节点完成即停由现有 pause 机制承接）；
//! - 生成会话在保存时投影任务标题（工作流名）；生成会话与会话列表里的
//!   其他记录是同一条任务，删除走会话列表的同一个入口。
//!
//! 存储语义：Task/模板均为现有结构，Forge 只增加 `origin`/`source`
//! 标记与 task 级 Spec 存档（`<task_dir>/forge/spec.json`），不新增
//! canonical 数据结构（方案自评审结论）。

mod session_source;
pub use session_source::{ForgeSourceMessage, ForgeSourceSession};

use crate::app::{App, CreateTaskInput, TaskSummary, WorkflowModelBindings};
use crate::forge::archive::{ForgeDraft, TemplateBinding, checkpoint, load_draft};
use crate::forge::spec::ForgeError;
use crate::forge::spec::ForgeSpec;
use crate::runtime::TaskOrigin;
use anyhow::{Context, Result, bail};

#[derive(Debug, thiserror::Error)]
pub enum ForgeTaskAccessError {
    #[error("forge/invalid-task-locator")]
    InvalidLocator,
    #[error("forge/not-generation-task")]
    NotGenerationTask,
    #[error("forge/source-session-unavailable")]
    SourceSessionUnavailable,
    #[error("forge/stale-message")]
    StaleMessage,
}

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
        let mut summary = self.create_task_from_requirement(CreateTaskInput {
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
            summary.task = self.task_show(&summary.task.id)?;
        }
        Ok(summary)
    }

    /// Compare and swap the complete draft. Only the archive authorizes existing bindings.
    pub fn save_forge_spec(&self, task_id: &str, spec: &ForgeSpec) -> Result<ForgeSpec> {
        let (_, task) = self.checked_task_locator(task_id)?;
        let uuid = task.uuid.as_deref().ok_or(ForgeTaskAccessError::InvalidLocator)?;
        self.save_forge_spec_for_instance(task_id, uuid, spec)
    }

    pub fn save_forge_spec_for_instance(&self, task_id: &str, expected_uuid: &str, spec: &ForgeSpec) -> Result<ForgeSpec> {
        let _lock = self.forge_write_lock(task_id)?;
        let (dir, task) = self.checked_forge_task(task_id)?;
        anyhow::ensure!(!expected_uuid.is_empty() && task.uuid.as_deref() == Some(expected_uuid), ForgeTaskAccessError::InvalidLocator);
        let mut draft = prepare_full_draft(spec, load_draft(&dir)?)?;
        self.checkpoint_forge_draft(&dir, &mut draft)?;
        Ok(draft.spec)
    }

    /// Called under the same authoring lock as rename/delete. Project only the
    /// name supplied by the saved draft; lists and search never read its body.
    fn checkpoint_forge_draft(&self, dir: &camino::Utf8Path, draft: &mut ForgeDraft) -> Result<()> {
        checkpoint(dir, draft)?;
        let task_path = dir.join("task.json");
        let mut task: crate::runtime::TaskState = crate::storage::read_json(&task_path)?;
        use crate::runtime::ForgeNaming;
        let naming = match &task.forge_naming {
            Some(ForgeNaming::Manual) => return Ok(()),
            Some(naming) => naming.clone(),
            None => {
                let path = dir.join("authoring/conversation.json");
                let automatic = path
                    .try_exists()?
                    .then(|| crate::storage::read_json::<crate::app::ConversationMetadata>(&path))
                    .transpose()?
                    .map(|m| m.title_auto_generated);
                ForgeNaming::from_legacy_title(task.title.clone(), automatic)
            }
        };
        let title = naming.effective_title(task.title.as_deref(), draft.spec.name.as_deref());
        if task.title != title || task.forge_naming.is_none() {
            task.title = title;
            task.forge_naming = Some(naming);
            crate::storage::write_json(&task_path, &task)?;
            (self.task_search_indexer)(dir, &task.id);
        }
        Ok(())
    }

    fn forge_write_lock(&self, task_id: &str) -> Result<impl Drop + use<>> {
        let key = crate::execution_plan::authoring_lock_key(&self.paths.project_id, task_id);
        Ok(crate::execution_plan::acquire_plan_write_wait(&key)?)
    }

    pub fn forge_spec(&self, task_id: &str) -> Result<Option<ForgeSpec>> {
        Ok(crate::forge::archive::load_spec_archive(
            &self.forge_task_dir(task_id)?,
        )?)
    }

    /// 试运行（方案 D1）：以模板创建真实 Task（origin=ForgeTrial）。
    ///
    /// 复用 task→run 全链路，不造 dry-run 专用代码路径。“首节点即停”以
    /// 数据级截断实现：只保留 entry 节点、success 边直连 `$end`，模板
    /// 本体不动——首节点产物/契约照常验证，跑完即终止。`control` 同步
    /// 覆写 `max_rounds=1`。样例需求 = 锚定轮确认的 intent。
    pub fn forge_trial(&self, template_id: &str, revision: &str, intent: &str) -> Result<TaskSummary> {
        if intent.trim().is_empty() {
            bail!("forge trial intent cannot be empty");
        }
        let store = self.read_workflow_templates()?;
        let template = store
            .templates
            .iter()
            .find(|template| template.id == template_id)
            .with_context(|| format!("workflow template `{template_id}` not found"))?;
        let (current, workflow, bindings) = self.prepare_forge_trial(template)?;
        if current.revision != revision {
            return Err(StaleForgeTrial { current }.into());
        }
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

    /// 反投影入口（方案 §4 Tauri 命令 `forge_from_workflow` 的底层）：
    /// 现有模板 → Forge Spec，进入 Forge 会话做对话式修改。
    pub fn forge_from_workflow(&self, template_id: &str) -> Result<ForgeSpec> {
        self.forge_project_source(template_id).map(|(spec, _)| spec)
    }

    pub(crate) fn forge_project_source(
        &self,
        template_id: &str,
    ) -> Result<(ForgeSpec, TemplateBinding)> {
        let store = self.workflow_templates()?;
        let source = store
            .templates
            .iter()
            .find(|t| t.id == template_id)
            .with_context(|| format!("workflow template `{template_id}` not found"))?;
        let mut spec = crate::forge::project::from_dsl(
            &self.paths,
            self.config.desktop_language,
            &source.workflow,
            &source.name,
            "",
        )?;
        spec.workflows[0].id = crate::app::ids::next_workflow_id();
        Ok((spec, source.into()))
    }

    /// Both the spec and its binding came from the same server-side template snapshot.
    pub(crate) fn seed_forge_source(
        &self,
        task_id: &str,
        spec: ForgeSpec,
        source: TemplateBinding,
    ) -> Result<()> {
        let dir = self.forge_task_dir(task_id)?;
        let _lock = self.forge_write_lock(task_id)?;
        check_forge_revision(0, load_draft(&dir)?.as_ref())?;
        let targets = [(spec.workflows[0].id.clone(), source)].into();
        self.checkpoint_forge_draft(&dir, &mut ForgeDraft { spec, targets })
    }

    pub fn forge_compile(
        &self,
        spec: &ForgeSpec,
        task_id: Option<&str>,
        repair_attempt: u32,
    ) -> Result<ForgeCompileOutcome> {
        self.forge_compile_inner(spec, task_id, repair_attempt, None)
    }

    /// Explicitly copy one bound workflow under a new unique name; siblings are untouched.
    pub fn forge_compile_as_copy(
        &self,
        spec: &ForgeSpec,
        task_id: &str,
        workflow_id: &str,
        new_name: &str,
        repair_attempt: u32,
    ) -> Result<ForgeCompileOutcome> {
        self.forge_compile_inner(
            spec,
            Some(task_id),
            repair_attempt,
            Some((workflow_id, new_name)),
        )
    }

    fn forge_compile_inner(
        &self,
        spec: &ForgeSpec,
        task_id: Option<&str>,
        repair_attempt: u32,
        copy: Option<(&str, &str)>,
    ) -> Result<ForgeCompileOutcome> {
        let dir = task_id.map(|id| self.forge_task_dir(id)).transpose()?;
        let base = dir
            .as_ref()
            .map(|dir| load_draft(dir))
            .transpose()?
            .flatten();
        let mut draft = if let Some((workflow_id, name)) = copy {
            check_forge_revision(spec.revision, base.as_ref())?;
            let mut draft = base.context("save as requires a persisted draft")?;
            let single = draft.spec.workflows.len() == 1;
            if draft
                .spec
                .workflows
                .iter()
                .any(|w| w.id != workflow_id && w.name.trim() == name.trim())
            {
                return self.forge_repair_outcome(
                    &[ForgeError::DuplicateWorkflowName {
                        name: name.trim().into(),
                    }],
                    repair_attempt,
                );
            }
            let target = draft
                .spec
                .workflows
                .iter_mut()
                .find(|w| w.id == workflow_id && draft.targets.contains_key(&w.id))
                .context("save as requires a bound workflow")?;
            anyhow::ensure!(!name.trim().is_empty(), ForgeError::MissingName);
            target.name = name.trim().into();
            if single {
                draft.spec.name = Some(target.name.clone());
            }
            self.forge_detach_copy_roles(target)?;
            draft
        } else if task_id.is_none() {
            let mut detached = spec.clone();
            detached.revision = 0;
            prepare_full_draft(&detached, None)?
        } else {
            match prepare_full_draft(spec, base) {
                Ok(draft) => draft,
                Err(error) => match error.downcast_ref::<ForgeError>() {
                    Some(reason @ ForgeError::ReadonlyRole { .. }) => {
                        return self
                            .forge_repair_outcome(std::slice::from_ref(reason), repair_attempt);
                    }
                    _ => return Err(error),
                },
            }
        };
        let mut to_compile = draft.spec.clone();
        if let Some((id, _)) = copy {
            to_compile.workflows.retain(|w| w.id == id);
            to_compile.name = Some(to_compile.workflows[0].name.clone());
        }
        let mut store = self.workflow_templates()?;
        let mut errors = Vec::new();
        let mut target_ids = Vec::new();
        for w in &to_compile.workflows {
            // Save As always creates: even a healthy bound source must not be exempt
            // from name uniqueness or become an update destination.
            let target_id = if copy.is_some() {
                None
            } else {
                self.resolve_forge_binding(&store, draft.targets.get(&w.id), &w.id)?
                    .filter(|t| !t.is_built_in)
                    .map(|t| t.id.clone())
            };
            target_ids.push(target_id);
        }
        let compiled_list = match crate::forge::compiler::compile_reporting_all(&to_compile) {
            Ok(list) => list,
            Err(validation) => {
                errors.extend(validation);
                Vec::new()
            }
        };
        errors.extend(self.forge_preflight_errors(&to_compile, &target_ids, &store)?);
        if !errors.is_empty() {
            return self.forge_repair_outcome(&errors, repair_attempt);
        }
        let mut saved = ForgeCompileSaved::default();
        for ((w, mut compiled), target_id) in to_compile.workflows.iter().zip(compiled_list).zip(target_ids) {
            // No lock during parsing, DSL compilation, or model execution. Each local template
            // commit has its own checkpoint: subsequent members must pass the task CAS again.
            let _lock = task_id.map(|id| self.forge_write_lock(id)).transpose()?;
            if let Some(dir) = &dir {
                check_forge_revision(draft.spec.revision, load_draft(dir)?.as_ref())?;
            }
            // Verify the library snapshot before touching profiles, under the writer lock
            // shared with template installation, deletion and ordinary template edits.
            let (committed, dropped, template_id) =
                crate::storage::configuration::with_configuration(
                    &self.paths.user_gold_band_dir(),
                    || {
                        let mut store = store;
                        crate::storage::configuration::verify_revision(
                            &self.paths.workflow_templates_file(),
                            &store.storage_revision,
                        )?;
                        let source = match self.resolve_forge_binding(&store, draft.targets.get(&w.id), &w.id) {
                            // Recovery can use the archived draft, never the settings of a
                            // changed or replaced source. Ordinary updates still fail closed.
                            Err(ForgeError::TargetUnavailable { .. }) if copy.is_some() => None,
                            result => result?,
                        };
                        if let Some(t) = source {
                            compiled.workflow =
                                crate::forge::project::write_back(&t.workflow, &compiled)?;
                        }
                        let ids = backfill_pending_profiles(
                            &self.paths,
                            self.config.desktop_language,
                            &mut compiled,
                            &draft.spec.intent,
                            source.map(|s| s.name.as_str()),
                        )?;
                        let definition = compiled.workflow;
                        let bindings = source.map(|t| t.model_bindings.clone()).unwrap_or_default();
                        let (definition, bindings) =
                            self.forge_authoring_for_save(definition, bindings)?;
                        let (template_id, dropped) = if let Some(id) = target_id {
                            let (updated, dropped) = self.update_workflow_template_in_store(
                                store,
                                &id,
                                Some(&compiled.name),
                                definition,
                                bindings,
                            )?;
                            store = updated;
                            saved.updated_count += 1;
                            (id, dropped)
                        } else {
                            let (new_store, id) = self.save_workflow_template_with_source(
                                compiled.name,
                                definition,
                                bindings,
                                crate::app::TemplateSource::Forged,
                            )?;
                            store = new_store;
                            saved.created_count += 1;
                            (id, Vec::new())
                        };
                        if let Some(dir) = &dir {
                            let target = draft
                                .spec
                                .workflows
                                .iter_mut()
                                .find(|item| item.id == w.id)
                                .expect("compiled archived workflow");
                            for (stage, id, charter) in ids {
                                target.stages[stage].profile_id = Some(id);
                                target.stages[stage].role_definition = charter;
                            }
                            let template = store
                                .templates
                                .iter()
                                .find(|t| t.id == template_id)
                                .expect("just saved");
                            draft.targets.insert(w.id.clone(), template.into());
                            self.checkpoint_forge_draft(dir, &mut draft)?;
                        }
                        Ok((store, dropped, template_id))
                    },
                )?;
            store = committed;
            drop(_lock);
            self.recycle_forged_profiles(dropped)?;
            if saved.trial.is_none() {
                // Derive disclosure from the commit result, never a later library read.
                let template = store.templates.iter().find(|t| t.id == template_id).expect("just saved");
                saved.trial = Some(self.prepare_forge_trial(template)?.0);
            }
            saved.template_ids.push(template_id);
        }
        saved.spec = dir.map(|_| draft.spec);
        Ok(ForgeCompileOutcome::Saved(saved))
    }

    /// 落库前的核对（要读角色库与模板库，所以不放在纯函数的 Spec 校验里）：
    /// - 内置角色是只读引用：stage 里的角色名必须就是内置角色的名字——Spec 是对话的
    ///   事实源，改了名字会让预览与实际引用的角色对不上；
    /// - 工作流名唯一：要落库的工作流不能与别的模板重名，原位更新时它自己不算。
    fn forge_preflight_errors(
        &self,
        spec: &ForgeSpec,
        target_ids: &[Option<String>],
        store: &crate::app::WorkflowTemplateStore,
    ) -> Result<Vec<ForgeError>> {
        let language = self.config.desktop_language;
        let mut errors = Vec::new();
        enum Binding { Editable, ReadOnly(String), Missing }
        let mut bindings = std::collections::BTreeMap::new();
        let mut proposed_names = std::collections::BTreeSet::new();
        for (index, workflow) in spec.workflows.iter().enumerate() {
            let name = if spec.workflows.len() == 1 {
                spec.name.as_deref().unwrap_or(workflow.name.as_str())
            } else {
                workflow.name.as_str()
            };
            let target_id = target_ids[index].as_deref();
            let taken = store.templates.iter().any(|template| {
                template.name.trim() == name.trim()
                    && Some(template.id.as_str()) != target_id
            });
            if taken || !proposed_names.insert(name.trim()) {
                errors.push(ForgeError::DuplicateWorkflowName {
                    name: name.trim().to_string(),
                });
            }
            for (position, stage) in workflow.stages.iter().enumerate() {
                let (Some(profile_id), Some(persona)) = (stage.profile_id.as_deref(), stage.custom_role_name())
                else {
                    continue;
                };
                if !bindings.contains_key(profile_id) {
                    let binding = match crate::app::profiles::find_profile_by_id(&self.paths, profile_id, language)? {
                        None => Binding::Missing,
                        Some(entry) if entry.is_built_in || !entry.forged => Binding::ReadOnly(entry.name),
                        Some(_) => Binding::Editable,
                    };
                    bindings.insert(profile_id.to_string(), binding);
                }
                let stage_id = || stage.id.clone().unwrap_or_else(|| format!("#{}", position + 1));
                match &bindings[profile_id] {
                    Binding::Missing => errors.push(ForgeError::UnavailableRole { stage: stage_id(), role: persona.into() }),
                    Binding::ReadOnly(name) if name.trim() != persona || stage.role_definition.is_some() => {
                        errors.push(ForgeError::ReadonlyRole { stage: stage_id(), role: name.clone() });
                    }
                    _ => {}
                }
            }
        }
        Ok(errors)
    }

    /// Explicit copy: recreate only this authorized draft's generated personas. A role with
    /// no archived charter is an external reference and can never be claimed by name.
    pub(crate) fn forge_detach_copy_roles(&self, workflow: &mut crate::forge::ForgeWorkflow) -> Result<()> {
        for stage in &mut workflow.stages {
            // The save-as caller reads this charter from the durable archive, not its request.
            // Bound stages without an editable charter are library references: keep the ID;
            // if it has disappeared, ordinary resolution reports unavailable (never recreate it).
            if stage.role_definition.as_deref().is_some_and(|text| !text.trim().is_empty()) {
                stage.profile_id = None;
            }
        }
        Ok(())
    }

    /// 回收不再被引用的铸造角色：候选里只删“由工作流生成（forged 标记），且没有任何
    /// 模板、任务或可继续的运行再引用”的角色。用户手工建的角色没有标记，永不自动删除。
    ///
    /// 调用方在模板落盘之后调用，并把“这次改动之前被引用、之后不再被引用”的 profile
    /// 作为候选传进来。引用统计要读任务与运行目录，只在真有候选时才发生。
    /// 返回实际删除的 profile id。
    pub(crate) fn recycle_forged_profiles(
        &self,
        candidates: impl IntoIterator<Item = String>,
    ) -> Result<Vec<String>> {
        let language = self.config.desktop_language;
        let mut deleted = Vec::new();
        for profile_id in candidates {
            let Ok(Some(entry)) = crate::app::profiles::find_profile_by_id(
                &self.paths,
                &profile_id,
                language,
            ) else {
                continue;
            };
            if entry.is_built_in || !entry.forged {
                continue;
            }
            let usage = self.profile_usage_counts(&entry.id)?;
            if usage.template_count > 0 || usage.task_count > 0 || usage.run_count > 0 {
                continue;
            }
            crate::app::profiles::delete_profile(&self.paths, &entry.id)
                .with_context(|| format!("failed to recycle forged profile `{}`", entry.id))?;
            deleted.push(entry.id);
        }
        Ok(deleted)
    }

    pub fn forge_template_targets(&self, task_id: &str) -> Result<Vec<String>> {
        let Some(draft) = load_draft(&self.forge_task_dir(task_id)?)? else {
            return Ok(vec![]);
        };
        Ok(draft
            .spec
            .workflows
            .iter()
            .filter_map(|w| match draft.targets.get(&w.id) {
                Some(TemplateBinding::Bound { template_id, .. }) => Some(template_id.clone()),
                _ => None,
            })
            .collect())
    }

    pub fn checked_task_locator(&self, task_id: &str) -> Result<(camino::Utf8PathBuf, crate::runtime::TaskState)> {
        let locator = camino::Utf8Path::new(task_id);
        if locator.file_name() != Some(task_id) || task_id.contains(['/', '\\', ':']) {
            bail!(ForgeTaskAccessError::InvalidLocator);
        }
        let tasks = self.paths.tasks_dir().canonicalize_utf8()?;
        let task_dir = self.paths.task_dir(task_id);
        let canonical = task_dir.canonicalize_utf8()?;
        if tasks != self.paths.runtime_root.canonicalize_utf8()?.join("tasks") || canonical != tasks.join(task_id) || canonical.join("task.json").canonicalize_utf8()? != canonical.join("task.json") {
            bail!(ForgeTaskAccessError::InvalidLocator);
        }
        let task = self.task_show(task_id)?;
        if task.id != task_id {
            bail!(ForgeTaskAccessError::InvalidLocator);
        }
        Ok((task_dir, task))
    }

    fn forge_task_dir(&self, task_id: &str) -> Result<camino::Utf8PathBuf> {
        Ok(self.checked_forge_task(task_id)?.0)
    }

    fn checked_forge_task(&self, task_id: &str) -> Result<(camino::Utf8PathBuf, crate::runtime::TaskState)> {
        let (task_dir, task) = self.checked_task_locator(task_id)?;
        if task.origin != Some(TaskOrigin::Forge) {
            bail!(ForgeTaskAccessError::NotGenerationTask);
        }
        let canonical = task_dir.canonicalize_utf8()?;
        let dir = canonical.join(crate::forge::FORGE_DIR_NAME);
        for path in [&dir, &crate::forge::archive::spec_archive_path(&canonical)] {
            match std::fs::symlink_metadata(path) {
                Ok(_) if path.canonicalize_utf8()? != *path => bail!(ForgeTaskAccessError::InvalidLocator),
                Ok(_) => (),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => (),
                Err(err) => return Err(err.into()),
            }
        }
        Ok((task_dir, task))
    }

    fn resolve_forge_binding<'a>(
        &self,
        store: &'a crate::app::WorkflowTemplateStore,
        binding: Option<&TemplateBinding>,
        workflow: &str,
    ) -> Result<Option<&'a crate::app::WorkflowTemplate>, ForgeError> {
        let Some(binding) = binding else {
            return Ok(None);
        };
        let unavailable = || ForgeError::TargetUnavailable {
            workflow: workflow.to_owned(),
        };
        let TemplateBinding::Bound {
            template_id,
            instance_id,
            authoring_etag: Some(expected),
        } = binding
        else {
            return Err(unavailable());
        };
        let template = store
            .templates
            .iter()
            .find(|t| t.id == *template_id && t.workflow.id == *instance_id)
            .ok_or_else(&unavailable)?;
        crate::forge::project::ordered_worker_ids(&template.workflow).map_err(|_| unavailable())?;
        if crate::forge::archive::authoring_etag(template) != *expected {
            return Err(unavailable());
        }
        Ok(Some(template))
    }

    /// 编译产物 → 可保存的作者态：先给缺执行槽的节点分配 slot 并剔除已删节点的
    /// 绑定，再为没有绑定的 slot 补默认 agent 绑定（顺序不能反：slot 不存在时
    /// 无从补绑定，模板会以“无绑定”落库，启动任务时才报 agent-required）。
    fn forge_authoring_for_save(
        &self,
        mut workflow: crate::dsl::WorkflowDsl,
        mut bindings: WorkflowModelBindings,
    ) -> Result<(crate::dsl::WorkflowDsl, WorkflowModelBindings)> {
        crate::workflow_model_binding::migrate_authoring_workflow(
            &mut workflow,
            &mut bindings,
            None,
        )?;
        let bindings = with_default_agent_bindings(&workflow, bindings, &self.config.agents);
        Ok((workflow, bindings))
    }

    /// 渲染修正提示词（模板在 `src/prompts/{lang}/forge/spec-repair.md`）：
    /// 首行是给用户看的进行态提示，其后是隐藏区段内的全部结构化校验错误。
    fn forge_repair_outcome(&self, errors: &[ForgeError], repair_attempt: u32) -> Result<ForgeCompileOutcome> {
        Ok(ForgeCompileOutcome::Repairable {
            errors: errors.iter().map(ForgeErrorVm::from).collect(),
            repair_prompt: self.render_forge_repair_prompt(errors, repair_attempt)?,
        })
    }

    fn render_forge_repair_prompt(&self, errors: &[ForgeError], repair_attempt: u32) -> Result<String> {
        let errors = errors
            .iter()
            .map(|error| format!("- {}: {}", error.code(), error))
            .collect::<Vec<_>>()
            .join("\n");
        crate::prompts::render(
            crate::prompts::FORGE_SPEC_REPAIR.resolve(self.config.desktop_language),
            serde_json::json!({
                "errors": errors.replace("</hidden>", "<\\/hidden>"),
                "attempt": repair_attempt.clamp(1, FORGE_MAX_REPAIR_ATTEMPTS),
                "max_attempts": FORGE_MAX_REPAIR_ATTEMPTS,
            }),
        )
    }

    /// 铸造会话一轮收口：提取并合并本轮的 Spec 增量。
    ///
    /// 返回 `None` 表示不是铸造会话；否则总是给出这一轮的收口结果（增量已应用 /
    /// 没有增量 / 增量无法应用），调用方据此通知前端——预览 Tab 与自动修正回路读到
    /// 的就是这一轮之后的存档，不依赖各事件之间的到达顺序。轮次一旦结束就必须收口：
    /// 提取失败若只记日志而不通知，自动修正回路会一直停在“正在修正”。
    pub fn forge_settle_turn(
        &self,
        task_id: &str,
        run_id: &str,
        round_id: &str,
        node_id: &str,
        attempt_id: &str,
        prompt_id: Option<&str>,
        outcome: super::AcpTurnOutcome,
    ) -> Result<Option<ForgeTurnSettlement>> {
        if self.task_show(task_id)?.origin != Some(TaskOrigin::Forge) {
            return Ok(None);
        }
        if outcome != super::AcpTurnOutcome::Completed {
            // A cancelled/failed reply may already contain a valid-looking but partial fence.
            return Ok(Some(ForgeTurnSettlement { spec_updated: false, error: None }));
        }
        let settlement = match self.forge_extract_turn_delta(task_id, run_id, round_id, node_id, attempt_id, prompt_id) {
            Ok(Some(_)) => ForgeTurnSettlement { spec_updated: true, error: None },
            Ok(None) => ForgeTurnSettlement { spec_updated: false, error: None },
            Err(error) => {
                // 多半是模型输出的增量 JSON 不合法（ForgeError）；其它失败（读写等）也带明细。
                let error = error
                    .downcast_ref::<ForgeError>()
                    .cloned()
                    .unwrap_or_else(|| ForgeError::InvalidSpec { detail: error.to_string() });
                ForgeTurnSettlement { spec_updated: false, error: Some(error) }
            }
        };
        Ok(Some(settlement))
    }

    /// 铸造对话轮完成后的 Spec 增量提取（回路心脏，方案 §3.1 增量生长）。
    ///
    /// 读取该 attempt 时间线,把最后一个用户 prompt 之后的 `textDelta`
    /// 拼接为最后一条助手消息,提取其中最后一个 ```json 围栏作为
    /// SpecDelta,合并进存档(无存档时以单空工作流骨架起步)。普通
    /// 对话轮(无围栏)返回 Ok(None),不产生副作用;非铸造会话同样返回
    /// Ok(None)。生产调用方走 [`Self::forge_settle_turn`]。
    fn forge_extract_turn_delta(
        &self,
        task_id: &str,
        run_id: &str,
        round_id: &str,
        node_id: &str,
        attempt_id: &str,
        prompt_id: Option<&str>,
    ) -> Result<Option<ForgeSpec>> {
        let task = self.task_show(task_id)?;
        if task.origin != Some(TaskOrigin::Forge) {
            return Ok(None);
        }
        let attempt_dir = self
            .paths
            .attempt_dir(task_id, run_id, round_id, node_id, attempt_id);
        let timeline_path = attempt_dir.join("acp.timeline.jsonl");
        let prompt_id = prompt_id.filter(|id| !id.is_empty()).ok_or(ForgeError::TurnUnconfirmed)?;
        let dir = self.forge_task_dir(task_id)?;
        // Serialize the bounded read and merge so a delayed older callback cannot write
        // after a newer turn has been applied to this draft.
        let _lock = self.forge_write_lock(task_id)?;
        let assistant_text = crate::acp::timeline::finished_prompt_reply(&timeline_path, prompt_id)
            .map_err(|_| ForgeError::TurnUnconfirmed)?
            .ok_or(ForgeError::TurnUnconfirmed)?;
        let Some(delta_json) = extract_last_fenced_json(&assistant_text) else {
            return Ok(None);
        };
        let delta = crate::forge::interview::SpecDelta::parse(&delta_json)
            .map_err(|error| anyhow::anyhow!("{}", error.to_string()))?;
        let mut draft = load_draft(&dir)?.unwrap_or_else(|| ForgeDraft {
            spec: initial_forge_spec_baseline(), targets: Default::default(),
        });
        draft.spec = crate::forge::interview::merge_delta(&draft.spec, &delta)?;
        for workflow in &mut draft.spec.workflows {
            if workflow.id.is_empty() { workflow.id = crate::app::ids::next_workflow_id(); }
        }
        crate::forge::interview::assign_stage_ids(&mut draft.spec);
        self.checkpoint_forge_draft(&dir, &mut draft)?;
        Ok(Some(draft.spec))
    }

    /// 在预览里直接改某个阶段的失败策略。改动是一条只含这个 stage 的增量，经
    /// [`merge_delta`] 合并——与对话里的修改走同一个合并入口，不产生第二套合并规则；
    /// 之后的对话轮次以合并后的存档为基线，模型看到的就是这次改动。
    ///
    /// 失败目标不是同一条工作流的 stage 时不落盘，返回 `forge/unknown-failure-target`。
    /// 只校验这次改动会破坏的约束：草稿里别处尚未收敛的问题由对话和编译去处理。
    pub fn forge_update_stage_failure(
        &self,
        task_id: &str,
        stage_id: &str,
        on_failure: crate::forge::OnFailure,
    ) -> Result<ForgeSpec> {
        let task_dir = self.forge_task_dir(task_id)?;
        let _lock = self.forge_write_lock(task_id)?;
        let mut draft = load_draft(&task_dir)?.with_context(|| format!("no forge draft for `{task_id}`"))?;
        let base = &draft.spec;
        let workflow = base
            .workflows
            .iter()
            .find(|workflow| {
                workflow
                    .stages
                    .iter()
                    .any(|stage| stage.id.as_deref() == Some(stage_id))
            })
            .with_context(|| format!("stage `{stage_id}` not found"))?;
        let mut next = workflow
            .stages
            .iter()
            .find(|stage| stage.id.as_deref() == Some(stage_id))
            .cloned()
            .expect("stage found above");
        next.manual_check = on_failure == crate::forge::OnFailure::ManualCheck;
        next.on_failure = on_failure;
        let workflow_id = workflow.id.clone();
        let merged = crate::forge::interview::merge_delta(
            &base,
            &crate::forge::interview::SpecDelta {
                stages: vec![next],
                workflow_id: Some(workflow_id.clone()),
                ..Default::default()
            },
        )?;
        let workflow = merged
            .workflows
            .iter()
            .find(|workflow| workflow.id == workflow_id)
            .expect("merged workflow exists");
        if let Some(error) = crate::forge::spec::failure_target_errors(workflow).into_iter().next() {
            return Err(error)?;
        }
        draft.spec = merged;
        self.checkpoint_forge_draft(&task_dir, &mut draft)?;
        Ok(draft.spec)
    }

    /// 入口 B/C：素材清单的结构化摘要（composer 预填用；文案由前端按语言拼装）。
    ///
    /// 逐个解析素材元数据；单个缺失即报错（清单来自同域扫描，缺失意味着
    /// 过期勾选，应整单失败而非静默丢弃）。
    pub fn forge_items_digest(&self, items: &[ForgeSourceItem]) -> Result<Vec<ForgeSourceSummary>> {
        Ok(self
            .read_forge_items(items, false)?
            .into_iter()
            .map(|content| ForgeSourceSummary {
                kind: content.kind,
                name: content.name,
                source: content.source,
                origin: content.origin,
                description: content.description,
            })
            .collect())
    }

    /// 入口 B/C：素材正文（逐个注入首条 prompt 的隐藏区段，G5 预算截断）。
    ///
    /// 超预算的素材正文截断并在区段内标注，对话中模型可见"已截断"事实。
    pub fn forge_items_material(
        &self,
        items: &[ForgeSourceItem],
    ) -> Result<crate::provider::PromptHiddenSection> {
        let contents = self.read_forge_items(items, true)?;
        const CLIPPED: &str = "\n（正文超预算已截断）\n\n";
        let lengths: Vec<_> = contents
            .iter()
            .map(|c| c.prompt_chars().take(HIDDEN_TOTAL_CHAR_CAP + 1).count())
            .collect();
        // Keep short entries intact; divide the rest among oversized entries. Allocation
        // never changes input order or lets a large early body hide later selections.
        let mut sorted = lengths.clone();
        sorted.sort_unstable();
        let mut remaining = HIDDEN_TOTAL_CHAR_CAP;
        let mut limit = HIDDEN_TOTAL_CHAR_CAP;
        for (i, &length) in sorted.iter().enumerate() {
            let share = remaining / (sorted.len() - i);
            if length > share {
                limit = share;
                break;
            }
            remaining -= length;
        }
        let mut body = String::new();
        for (content, length) in contents.iter().zip(lengths) {
            if length <= limit {
                body.extend(content.prompt_chars());
            } else {
                body.extend(content.prompt_chars().take(limit - CLIPPED.chars().count()));
                body.push_str(CLIPPED);
            }
        }
        Ok(crate::provider::PromptHiddenSection {
            title: "素材".to_string(),
            content: body,
        })
    }

    fn read_forge_items(
        &self,
        items: &[ForgeSourceItem],
        with_body: bool,
    ) -> Result<Vec<ForgeSourceContent>> {
        use crate::app::profiles::{ProfileScope, read_profile_materials};
        use crate::config::SkillSource::{BuiltIn, Global, Project};
        use crate::skill::claude_plugin::{ClaudePluginAssetKind, resolve_plugin_assets};
        use std::collections::{BTreeMap, BTreeSet};
        anyhow::ensure!(
            items.len() <= FORGE_MATERIAL_SELECTION_MAX,
            ForgeMaterialUnavailable
        );
        anyhow::ensure!(
            items.iter().all(|it| !it.asset_id.is_empty()
                && match it.catalog {
                    ForgeMaterialCatalog::Profile => it.source != Project,
                    ForgeMaterialCatalog::Skill | ForgeMaterialCatalog::Plugin =>
                        it.source != BuiltIn,
                }),
            ForgeMaterialUnavailable
        );
        let manager =
            crate::skill::SkillManager::new(self.paths.clone(), self.config.agents.clone());
        let mut ordered: Vec<Option<ForgeSourceContent>> = (0..items.len()).map(|_| None).collect();
        for scope in [Project, Global, BuiltIn] {
            let mut resolved = BTreeMap::new();
            let ids = |catalog| {
                items
                    .iter()
                    .filter(|it| it.source == scope && it.catalog == catalog)
                    .map(|it| it.asset_id.as_str())
                    .collect::<BTreeSet<_>>()
            };
            let roles = ids(ForgeMaterialCatalog::Profile);
            if !roles.is_empty() {
                let profile_scope = match scope {
                    Global => ProfileScope::User,
                    BuiltIn => ProfileScope::BuiltIn,
                    Project => unreachable!("project profile rejected above"),
                };
                let body_limit = if with_body {
                    HIDDEN_TOTAL_CHAR_CAP + 1
                } else {
                    0
                };
                for (id, role) in read_profile_materials(
                    &self.paths,
                    profile_scope,
                    &roles,
                    self.config.desktop_language,
                    body_limit,
                )
                .map_err(|_| ForgeMaterialUnavailable)?
                {
                    resolved.insert(
                        (ForgeMaterialCatalog::Profile, id),
                        ForgeSourceContent {
                            kind: ForgeSourceKind::Role,
                            name: role.name,
                            description: role.summary,
                            body: role.body,
                            label: forge_material_label(scope, None),
                            source: scope,
                            origin: None,
                        },
                    );
                }
            }
            for (id, skill) in
                manager.read_listed_forge_skills(scope, &ids(ForgeMaterialCatalog::Skill))
            {
                let origin = Some(skill.meta.agent_source);
                resolved.insert(
                    (ForgeMaterialCatalog::Skill, id),
                    ForgeSourceContent {
                        label: forge_material_label(scope, origin.as_deref()),
                        origin,
                        source: scope,
                        kind: ForgeSourceKind::Skill,
                        name: skill.meta.name,
                        description: skill.meta.description,
                        body: if with_body { skill.body } else { String::new() },
                    },
                );
            }
            let wanted = ids(ForgeMaterialCatalog::Plugin);
            let plugins = if wanted.is_empty() {
                Vec::new()
            } else {
                resolve_plugin_assets(&self.paths, scope, wanted.iter().copied())
            };
            for plugin in plugins.iter().filter(|plugin| plugin.source == scope) {
                for asset in plugin
                    .assets
                    .iter()
                    .filter(|a| wanted.contains(a.asset_id.as_str()))
                {
                    let body = if with_body {
                        plugin.asset_body(asset).ok_or(ForgeMaterialUnavailable)?
                    } else {
                        String::new()
                    };
                    let origin = Some(format!("{} · plugin {}", plugin.provider_id, plugin.name));
                    let kind = match asset.kind {
                        ClaudePluginAssetKind::Skill => ForgeSourceKind::Skill,
                        ClaudePluginAssetKind::Command => ForgeSourceKind::Command,
                        ClaudePluginAssetKind::Agent => ForgeSourceKind::Agent,
                    };
                    resolved.insert(
                        (ForgeMaterialCatalog::Plugin, asset.asset_id.clone()),
                        ForgeSourceContent {
                            kind,
                            source: plugin.source,
                            name: asset.name.clone(),
                            description: asset.description.clone(),
                            label: forge_material_label(plugin.source, origin.as_deref()),
                            origin,
                            body,
                        },
                    );
                }
            }
            for (slot, item) in ordered
                .iter_mut()
                .zip(items)
                .filter(|(_, it)| it.source == scope)
            {
                // A selection is a set. The second occurrence is unavailable; never silently drop it.
                *slot = Some(
                    resolved
                        .remove(&(item.catalog, item.asset_id.clone()))
                        .ok_or(ForgeMaterialUnavailable)?,
                );
            }
        }
        Ok(ordered
            .into_iter()
            .map(|slot| slot.expect("all material scopes visited"))
            .collect())
    }

    /// workflow-forge 角色（按界面语言解析内置 profile）。
    ///
    /// 角色正文后拼接渲染后的 Spec 输出协议（规则与 JSON Schema）。角色是
    /// system prompt、会话内不变，所以只承载不随轮次变化的内容；当前基线
    /// Spec 每轮都在变，由 [`forge_baseline_section`] 随每条消息注入——写死
    /// 在这里会让“修改存量工作流”的会话从首轮起就看不到要修改的基线。
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
            serde_json::json!({ "spec_schema": schema }),
        )?;
        Ok(crate::provider::UserPromptRole {
            profile_id: profile.id,
            name: profile.name,
            content: format!("{}\n\n---\n\n{}", profile.content, rendered),
        })
    }

    fn prepare_forge_trial(
        &self,
        template: &crate::app::WorkflowTemplate,
    ) -> Result<(ForgeTrialInfo, crate::dsl::WorkflowDsl, WorkflowModelBindings)> {
        let mut workflow = truncate_to_first_node(template.workflow.clone());
        workflow.control.max_rounds = Some(1);
        let bindings = with_default_agent_bindings(&workflow, template.model_bindings.clone(), &self.config.agents);
        // definitionRevision deliberately ignores slot assignment. Include the exact saved DSL
        // and effective bindings (including their existing revisions and any fallback agent).
        let content = serde_json::to_vec(&(&self.paths.project_id, &template.id, &template.workflow, &bindings))?;
        let group = workflow.nodes.iter().find_map(|node| match node {
            crate::dsl::NodeDsl::ParallelGroup(g) if g.id == workflow.entry => Some(ForgeTrialGroup {
                branches: g.branches.len(), cap: g.branches.len().min(g.max_parallel as usize),
            }),
            _ => None,
        });
        let info = ForgeTrialInfo {
            template_id: template.id.clone(), revision: blake3::hash(&content).to_hex().to_string(), group,
        };
        Ok((info, workflow, bindings))
    }
}

fn check_forge_revision(expected: u64, base: Option<&ForgeDraft>) -> Result<()> {
    let actual = base.map_or(0, |d| d.spec.revision);
    anyhow::ensure!(
        expected == actual,
        ForgeError::RevisionConflict { expected, actual }
    );
    Ok(())
}

fn prepare_full_draft(submitted: &ForgeSpec, baseline: Option<ForgeDraft>) -> Result<ForgeDraft> {
    check_forge_revision(submitted.revision, baseline.as_ref())?;
    let mut spec = submitted.clone();
    let mut seen = std::collections::BTreeSet::new();
    // Losing an existing binding and introducing an anonymous replacement in one full
    // submission is not a rename or a copy command; never silently reset its authority.
    if let Some(base) = &baseline
        && spec.workflows.iter().any(|w| w.id.is_empty())
        && let Some(missing) = base
            .targets
            .keys()
            .find(|id| !spec.workflows.iter().any(|w| &w.id == *id))
    {
        return Err(ForgeError::TargetUnavailable {
            workflow: missing.clone(),
        }
        .into());
    }
    for w in &mut spec.workflows {
        let previous = baseline
            .as_ref()
            .and_then(|base| base.spec.workflows.iter().find(|item| item.id == w.id));
        if w.id.is_empty() || baseline.is_none() {
            w.id = crate::app::ids::next_workflow_id();
        } else if previous.is_none() {
            bail!(ForgeError::InvalidSpec {
                detail: "unknown workflow id".into()
            });
        }
        anyhow::ensure!(
            seen.insert(w.id.clone()),
            ForgeError::InvalidSpec {
                detail: "duplicate workflow id".into()
            }
        );
        w.validate_groups()?;
        crate::forge::interview::reconcile_workflow_authority(previous, w)?;
    }
    crate::forge::interview::assign_stage_ids(&mut spec);
    let mut targets = baseline.map(|b| b.targets).unwrap_or_default();
    targets.retain(|id, _| seen.contains(id));
    Ok(ForgeDraft { spec, targets })
}

/// 基线 Spec 区段的标题（首条 prompt 上显示为独立的隐藏区段标题；与「源会话材料」
/// 「素材」等运行上下文区段标题同一约定）。
pub(crate) const FORGE_BASELINE_SECTION_TITLE: &str = "当前基线 Spec";

/// Forge 铸造会话每条消息随运行上下文注入的基线 Spec 区段（方案 §3.1：Spec 是
/// 对话的唯一事实源）。角色（system prompt）在会话内不变，只承载协议规则；每轮
/// 都在变的基线由这里注入——模型据此按 stage id 对齐增量、原样携带 profileId，
/// 修改存量工作流的会话首轮即拿到反投影基线。非 Forge 任务、尚无存档或读取失败
/// 时返回 `None`（fail-open：不阻断对话，模型按“基线为空”起步）。
pub(crate) fn forge_baseline_section(
    app: &App,
    task_id: &str,
) -> Option<crate::provider::PromptHiddenSection> {
    let spec = app.forge_spec(task_id).ok().flatten()?;
    let spec_json = serde_json::to_string_pretty(&spec).ok()?;
    let content = crate::prompts::render(
        crate::prompts::FORGE_SPEC_BASELINE.resolve(app.config.desktop_language),
        serde_json::json!({ "spec_json": spec_json }),
    )
    .ok()?;
    Some(crate::provider::PromptHiddenSection {
        title: FORGE_BASELINE_SECTION_TITLE.to_string(),
        content,
    })
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
        revision: 0,
        spec_version: crate::forge::SPEC_VERSION.to_string(),
        name: None,
        intent: String::new(),
        source_skills: vec![],
        workflows: vec![crate::forge::spec::ForgeWorkflow {
            id: String::new(),
            parallel_groups: vec![],
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

/// IPC and durable draft selection. Files, inline bodies and labels are inventory-owned.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgeSourceItem {
    pub catalog: ForgeMaterialCatalog,
    pub source: crate::config::SkillSource,
    pub asset_id: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum ForgeMaterialCatalog {
    Skill,
    Plugin,
    Profile,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ForgeSourceKind {
    Skill,
    Command,
    Agent,
    Role,
}

impl ForgeSourceKind {
    /// 材料标题里的种类标签。
    pub fn tag(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Command => "command",
            Self::Agent => "agent",
            Self::Role => "role",
        }
    }
}

/// 素材的结构化摘要（composer 预填；文案由前端按界面语言拼装）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSourceSummary {
    pub kind: ForgeSourceKind,
    pub name: String,
    pub source: crate::config::SkillSource,
    pub origin: Option<String>,
    pub description: String,
}

#[derive(Debug, thiserror::Error)]
#[error("plugin.asset-unavailable")]
pub struct ForgeMaterialUnavailable;

fn forge_material_label(source: crate::config::SkillSource, origin: Option<&str>) -> String {
    let scope = match source {
        crate::config::SkillSource::BuiltIn => "内置",
        crate::config::SkillSource::Global => "全局",
        crate::config::SkillSource::Project => "项目",
    };
    origin
        .filter(|s| !s.is_empty())
        .map(|origin| format!("{scope} · {origin}"))
        .unwrap_or_else(|| scope.to_owned())
}

/// 读到的素材正文及其标注（后端内部投影）。
struct ForgeSourceContent {
    kind: ForgeSourceKind,
    name: String,
    description: String,
    body: String,
    /// 材料标题里的来源标注：内置 / 全局 / 项目，可带来源目录或 plugin 名。
    label: String,
    source: crate::config::SkillSource,
    origin: Option<String>,
}

impl ForgeSourceContent {
    fn prompt_chars(&self) -> impl Iterator<Item = char> + '_ {
        [
            "## ",
            self.kind.tag(),
            ": ",
            &self.name,
            "（",
            &self.label,
            "）\n> ",
            &self.description,
            "\n\n",
            &self.body,
            "\n\n",
        ]
        .into_iter()
        .flat_map(str::chars)
    }
}

/// Spec 校验错误的结构化投影：前端按 code 本地化，context 是错误的字段明细。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeErrorVm {
    pub code: String,
    pub context: serde_json::Value,
}

impl From<&crate::forge::spec::ForgeError> for ForgeErrorVm {
    fn from(error: &crate::forge::spec::ForgeError) -> Self {
        let serialized = serde_json::to_value(error).unwrap_or(serde_json::Value::Null);
        Self {
            code: error.code().to_string(),
            context: serialized
                .get("context")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        }
    }
}

/// 一次铸造对话轮的收口结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeTurnSettlement {
    /// 本轮回复是否带来了可解析、已合并进存档的 Spec 增量。
    pub spec_updated: bool,
    /// 回复里有增量但无法应用（JSON 不合法等）。轮次本身已经正常结束。
    pub error: Option<ForgeError>,
}

/// 修正循环上限（方案 §5.5）：3 次仍失败则带未收敛项回到用户对话。
pub const FORGE_MAX_REPAIR_ATTEMPTS: u32 = 3;

/// 一次编译尝试的结果。
#[derive(Debug)]
pub enum ForgeCompileOutcome {
    Saved(ForgeCompileSaved),
    /// Spec 未通过校验：没有落库任何模板，错误可交回铸造对话修正后重试。
    Repairable {
        errors: Vec<ForgeErrorVm>,
        repair_prompt: String,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeTrialInfo {
    pub template_id: String,
    pub revision: String,
    pub group: Option<ForgeTrialGroup>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ForgeTrialGroup {
    pub branches: usize,
    pub cap: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("forge/stale-trial")]
pub struct StaleForgeTrial {
    pub current: ForgeTrialInfo,
}

/// 编译落库结果。
#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeCompileSaved {
    pub trial: Option<ForgeTrialInfo>,
    pub spec: Option<ForgeSpec>,
    /// 与 Spec.workflows 下标对齐的模板 id。
    pub template_ids: Vec<String>,
    pub created_count: usize,
    pub updated_count: usize,
}


/// 内置 workflow-forge profile id（`DEFAULT_PROFILE_SEEDS`）。
pub const WORKFLOW_FORGE_PROFILE_ID: &str = "pf-builtin-workflow-forge";
/// 隐藏注入预算（G5：单条用户消息字符上限）。
const HIDDEN_PROMPT_CHAR_CAP: usize = 2_000;
/// 隐藏注入总预算（约 50KB 上限,UTF-8 中文约 3 字节/字符）。
const HIDDEN_TOTAL_CHAR_CAP: usize = 16_000;
/// One ordered set across roles, skills and plugin assets, not a per-catalog limit.
const FORGE_MATERIAL_SELECTION_MAX: usize = 32;
/// composer 预填摘要上限。
const DIGEST_CHAR_CAP: usize = 2_000;

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

/// custom 角色的落库与节点引用回填（对话反哺的落点），对一条工作流的编译产物逐个处理。
///
/// 编译器为 custom stage 置占位 profile（`<node_id>-profile`，无法通过保存链路的
/// profile 解析）。角色以名字为身份（Spec 层已校验同名 stage 的 profileId 与职责正文
/// 一致），落库时按名字归组：
/// - 同名的 stage 共用一个角色：Spec 里已回填的 profileId 是这一组的锚点，还没回填的
///   stage 归入同一个角色，首次编译时整组只新建一次；
/// - `profileId` 指向内置角色的，是只读引用：节点直接引用它，不改名、不改正文；
/// - 其余按 id 原地更新（对话里改了名字或职责直接反哺），没有 id 的新建 User scope
///   profile 并打上铸造标记（随工作流回收，见 [`App::recycle_forged_profiles`]）。
///
/// 角色显示名（`forge::project::forge_role_name`）：角色库是用户级的，而每次铸造
/// 都会新建角色，所以新角色叫「工作流名 · 角色名」，工作流一多也分得清归属。
/// 更新既有角色时，名字带本工作流前缀的才是这个工作流铸造出来的角色，保持这个格式
/// （对话里改了角色名、或工作流改名都跟随）；别的工作流铸造出来的角色归那边所有，
/// 名字不动；模板本来就引用的用户角色，名字是用户的资产——只有对话里明确改了角色名
/// 才改名，且不加前缀。
fn backfill_pending_profiles(
    paths: &crate::storage::GoldBandPaths,
    language: crate::config::DesktopLanguage,
    compiled: &mut crate::forge::compiler::CompiledWorkflow,
    intent: &str,
    former_workflow_name: Option<&str>,
) -> Result<Vec<(usize, String, Option<String>)>> {
    use crate::forge::project::{forge_role_name, is_forged_role_name};
    let workflow_name = compiled.name.trim().to_owned();
    let mut groups = std::collections::BTreeMap::<String, Vec<_>>::new();
    for role in compiled.pending_profiles.clone() {
        groups.entry(role.name.trim().to_owned()).or_default().push(role);
    }
    // Resolve every bound role for this member under the same commit lock, before the first
    // profile write. No additional library read: the write pass consumes these results.
    let mut resolved = std::collections::BTreeMap::new();
    for (persona, stages) in &groups {
        if persona.is_empty() { bail!("custom role profile name cannot be empty"); }
        if let Some(role) = stages.iter().find(|s| s.profile_id.is_some()) {
            let id = role.profile_id.as_deref().expect("bound role");
            let entry = crate::app::profiles::find_profile_by_id(paths, id, language)?
                .ok_or_else(|| ForgeError::UnavailableRole { stage: role.node_id.clone(), role: persona.clone() })?;
            if (entry.is_built_in || !entry.forged) && (entry.name.trim() != persona || role.role_definition.is_some()) {
                return Err(ForgeError::ReadonlyRole { stage: role.node_id.clone(), role: entry.name }.into());
            }
            resolved.insert(persona.clone(), entry);
        }
    }
    let mut forged_ids = Vec::new();
    for (persona, stages) in groups {
        let definition = stages.iter().find_map(|s| s.role_definition.clone())
            .unwrap_or_else(|| stages[0].definition.clone());
        let summary: String = definition.lines().find(|s| !s.trim().is_empty()).unwrap_or(intent)
            .trim().chars().take(80).collect();
        let input = |name| crate::app::profiles::ProfileInput { name, summary, content: definition, dynamic_template: false };
        let entry = if let Some(existing) = resolved.remove(&persona) {
            if existing.is_built_in || !existing.forged {
                existing // A template reference grants use, not permission to edit a library role.
            } else {
                let name = if is_forged_role_name(&existing.name, &workflow_name, former_workflow_name) {
                    forge_role_name(&workflow_name, &persona)
                } else { existing.name };
                crate::app::profiles::update_profile(paths, &existing.id, input(name))?
            }
        } else {
            crate::app::profiles::create_forged_profile(paths, input(forge_role_name(&workflow_name, &persona)))?
        };
        for stage in stages {
            point_node_to_profile(compiled, &stage.node_id, &format!("{}-profile", stage.node_id), &entry.id);
            forged_ids.push((stage.stage_index, entry.id.clone(), entry.forged.then(|| entry.content.clone())));
        }
    }
    Ok(forged_ids)
}

/// 把节点上的占位 profile 引用换成真实 profile id。
fn point_node_to_profile(
    compiled: &mut crate::forge::compiler::CompiledWorkflow,
    node_id: &str,
    placeholder: &str,
    profile_id: &str,
) {
    for node in &mut compiled.workflow.nodes {
        if node.id() != node_id {
            continue;
        }
        if let crate::dsl::NodeDsl::Worker(worker) = node
            && worker.profile.as_deref() == Some(placeholder)
        {
            worker.profile = Some(profile_id.to_string());
        }
    }
}

/// 试运行截断：仅保留 entry 节点，success 边直连 `$end`。
///
/// 数据级变换（模板本体不动），复用现有 runtime 全链路：首节点按原
/// 产物契约执行与验证，完成后无后继节点即停——等价于“真实运行的
/// 第一环”验证语义（方案 §9.4）。
fn truncate_to_first_node(mut workflow: crate::dsl::WorkflowDsl) -> crate::dsl::WorkflowDsl {
    use crate::dsl::{EdgeDsl, EdgeOutcome, END_NODE};
    let entry = workflow.entry.clone();
    let members: std::collections::HashSet<String> = workflow.nodes.iter().find_map(|node| match node {
        crate::dsl::NodeDsl::ParallelGroup(group) if group.id == entry => Some(group.branches.iter()
            .flat_map(|branch| branch.stages.iter().cloned()).collect()),
        _ => None,
    }).unwrap_or_default();
    workflow.nodes.retain(|node| node.id() == entry || members.contains(node.id()));
    workflow.edges.retain(|edge| members.contains(&edge.from) && members.contains(&edge.to));
    workflow.edges.push(EdgeDsl {
        from: entry,
        to: END_NODE.to_string(),
        on: EdgeOutcome::Success,
        session: None,
        new_round_entry: None,
    });
    workflow
}


#[cfg(test)]
pub(crate) mod tests {
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

    pub(crate) fn tests_visible_sample_spec() -> ForgeSpec {
        sample_spec()
    }

    fn sample_spec() -> ForgeSpec {
        ForgeSpec {
            revision: 0,
            spec_version: SPEC_VERSION.to_string(),
            name: Some("样本流程".to_string()),
            intent: "从会话到工作流".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                id: String::new(),
                parallel_groups: vec![],
                name: "样本流程".to_string(),
                stages: vec![ForgeStage {
                    id: Some("s1".to_string()),
                    role: StageRole::Dev,

                    role_definition: None,

                    profile_id: None,
                    goal: "<输入>会话上下文\n<产出>结构化产物\n<边界>不做部署\n<验收>产物过schema".into(),
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
    fn forge_task_lifecycle_create_spec_roundtrip() {
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
        let revised = app.save_forge_spec(task_id, &revised).unwrap();
        assert_eq!(app.forge_spec(task_id).unwrap().unwrap().intent, "新意图");
        // 对话里给工作流改名后，会话列表里的标题跟着变。
        assert_eq!(app.task_show(task_id).unwrap().title.as_deref(), Some("样本流程"));
        let mut renamed = revised;
        renamed.name = Some("发布闸门".to_string());
        app.save_forge_spec(task_id, &renamed).unwrap();
        assert_eq!(app.task_show(task_id).unwrap().title.as_deref(), Some("发布闸门"));
    }

    #[test]
    fn forge_session_title_falls_back_until_the_workflow_is_named() {
        let (app, _temp, _guard) = forge_test_app();
        // 还没有草稿：没有标题来源。
        let task_id = forge_task(&app);
        assert_eq!(app.task_show(&task_id).unwrap().title.as_deref(), Some("铸造会话"));
        // 首轮骨架只有占位的工作流，顶层还没有名字。
        app.save_forge_spec(&task_id, &initial_forge_spec_baseline()).unwrap();
        assert_eq!(app.task_show(&task_id).unwrap().title.as_deref(), Some("铸造会话"));
        // 普通任务即使带着 Spec 存档也不走这条标题。
        let ordinary = app
            .create_task_from_requirement(CreateTaskInput {
                title: Some("普通任务".to_string()),
                description: None,
                requirement_file_name: None,
                requirement_content: "普通任务".to_string(),
                workflow: forge_conversation_workflow("claude-acp"),
                workflow_template_id: None,
                origin: None,
            })
            .unwrap();
        assert!(app.save_forge_spec(&ordinary.task.id, &sample_spec()).is_err());
        assert!(app.forge_spec(&ordinary.task.id).is_err());
        assert!(!crate::forge::archive::spec_archive_path(&app.paths.task_dir(&ordinary.task.id)).exists());
        assert_eq!(app.task_show(&ordinary.task.id).unwrap().title.as_deref(), Some("普通任务"));
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

    fn custom_stage(id: &str, profile_name: &str, profile_id: Option<&str>) -> ForgeStage {
        let mut stage = sample_spec().workflows[0].stages[0].clone();
        stage.id = Some(id.to_string());
        stage.role = StageRole::Custom {
            profile_name: profile_name.to_string(),
        };
        stage.role_definition = Some("你是发布审计员：逐项复核，不修改任何文件。".to_string());
        stage.profile_id = profile_id.map(str::to_string);
        stage
    }

    fn profile_name_of(app: &App, profile_id: &str) -> String {
        crate::app::profiles::find_profile_by_id(
            &app.paths,
            profile_id,
            app.config.desktop_language,
        )
        .unwrap()
        .expect("forged profile exists")
        .name
    }

    fn custom_profile_id(app: &App, template_id: &str, node_id: &str) -> String {
        app.workflow_templates()
            .unwrap()
            .templates
            .into_iter()
            .find(|template| template.id == template_id)
            .expect("template exists")
            .workflow
            .nodes
            .into_iter()
            .find(|node| node.id() == node_id)
            .and_then(|node| match node {
                crate::dsl::NodeDsl::Worker(worker) => worker.profile,
                _ => None,
            })
            .expect("node references a profile")
    }

    fn persona_of(spec: &ForgeSpec, stage_id: &str) -> (String, Option<String>) {
        let stage = spec.workflows[0]
            .stages
            .iter()
            .find(|stage| stage.id.as_deref() == Some(stage_id))
            .expect("stage exists");
        let StageRole::Custom { profile_name } = &stage.role else {
            panic!("expected a custom role")
        };
        (profile_name.clone(), stage.profile_id.clone())
    }

    #[test]
    fn custom_roles_are_named_after_their_workflow() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let mut spec = sample_spec();
        spec.workflows[0].stages.push(custom_stage("audit", "发布审计员", None));

        // 角色库是用户级的：每次铸造都会新建一个角色，用「工作流名 · 角色名」区分归属，
        // 避免多个工作流各自铸造的角色同名混淆。
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
        let profile_id = custom_profile_id(&app, &saved.template_ids[0], "audit");
        assert_eq!(profile_name_of(&app, &profile_id), "样本流程 · 发布审计员");

        // 同一会话重新编译：按 profileId 原地更新，名字不叠加前缀。
        let archived = app.forge_spec(&task_id).unwrap().unwrap();
        assert_eq!(persona_of(&archived, "audit"), ("发布审计员".to_string(), Some(profile_id.clone())));
        expect_saved(app.forge_compile(&archived, Some(&task_id), 1));
        assert_eq!(profile_name_of(&app, &profile_id), "样本流程 · 发布审计员");
        assert_eq!(user_profile_count(&app), 1);

        // 对话里调整了角色名：跟随，仍带工作流前缀。
        let mut renamed_role = app.forge_spec(&task_id).unwrap().unwrap();
        renamed_role.workflows[0].stages[1].role = StageRole::Custom {
            profile_name: "发布复核员".to_string(),
        };
        expect_saved(app.forge_compile(&renamed_role, Some(&task_id), 1));
        assert_eq!(profile_name_of(&app, &profile_id), "样本流程 · 发布复核员");

        // 工作流改名：角色名里的归属跟随。
        let mut renamed_workflow = app.forge_spec(&task_id).unwrap().unwrap();
        renamed_workflow.name = Some("发布闸门".to_string());
        renamed_workflow.workflows[0].name = "发布闸门".to_string();
        let saved = expect_saved(app.forge_compile(&renamed_workflow, Some(&task_id), 1));
        assert_eq!(profile_name_of(&app, &profile_id), "发布闸门 · 发布复核员");

        // 新开会话修改这个模板：反投影还原出角色名（去掉前缀）与 profileId，
        // 之后的重新编译据此继续原位更新、不叠加前缀。
        let projected = app.forge_from_workflow(&saved.template_ids[0]).unwrap();
        assert_eq!(persona_of(&projected, "audit"), ("发布复核员".to_string(), Some(profile_id.clone())));
        let modify_task = forge_task(&app);
        let projected = seed_source(&app, &modify_task, &saved.template_ids[0]);
        let recompiled = expect_saved(app.forge_compile(&projected, Some(&modify_task), 1));
        assert_eq!((recompiled.created_count, recompiled.updated_count), (0, 1));
        assert_eq!(profile_name_of(&app, &profile_id), "发布闸门 · 发布复核员");
        assert_eq!(user_profile_count(&app), 1);
    }

    #[test]
    fn projected_roleless_editor_worker_saves_without_minting_a_persona() {
        let (app, _temp, _guard) = forge_test_app();
        let mut workflow = crate::forge::compiler::compile(&sample_spec()).unwrap().remove(0).workflow;
        let crate::dsl::NodeDsl::Worker(worker) = &mut workflow.nodes[0] else { panic!("worker") };
        worker.prompt_envelope = crate::dsl::PromptEnvelopeMode::RawAgent;
    worker.profile = None;
        let prose = "  Editor goal without role or structured headings.  ";
        worker.goal = Some(prose.into());
        let template = save_template(&app, "Roleless worker", workflow);
        let task = forge_task(&app);
        let mut spec = seed_source(&app, &task, &template);
        spec.intent = "change the intent only".into();
        let count = user_profile_count(&app);
        expect_saved(app.forge_compile(&spec, Some(&task), 1));
        let updated = template_of(&app, &template);
        assert_eq!(worker_of(&updated, 0).profile, None);
        assert_eq!(worker_of(&updated, 0).goal.as_deref(), Some(prose));
        assert_eq!(user_profile_count(&app), count);
    }

    #[test]
    fn roles_the_template_already_referenced_keep_their_names() {
        let (app, _temp, _guard) = forge_test_app();
        // 模板引用的是用户自己建的通用角色（不是铸造出来的）。
        let hand_made = crate::app::profiles::create_profile(
            &app.paths,
            crate::app::profiles::ProfileInput {
                name: "严格审查员".to_string(),
                summary: "通用审查角色".to_string(),
                content: "严格审查，不修改文件。".to_string(),
                dynamic_template: false,
            },
        )
        .unwrap();
        let mut workflow = crate::forge::compiler::compile(&sample_spec()).unwrap().remove(0).workflow;
        let crate::dsl::NodeDsl::Worker(worker) = &mut workflow.nodes[0] else { panic!("worker") };
        worker.profile = Some(hand_made.id.clone());
        let id = save_template(&app, "Library reference", workflow);
        let task = forge_task(&app);
        let spec = seed_source(&app, &task, &id);
        expect_saved(app.forge_compile(&spec, Some(&task), 1));
        let mut revised = app.forge_spec(&task).unwrap().unwrap();
        assert_eq!(revised.workflows[0].stages[0].profile_id.as_deref(), Some(hand_made.id.as_str()));
        assert_eq!(revised.workflows[0].stages[0].role_definition, None);
        revised.workflows[0].stages[0].role_definition = Some("new charter".into());
        assert_eq!(repairable_codes(app.forge_compile(&revised, Some(&task), 1)), ["forge/readonly-role"]);
        assert_eq!(app.profile_show(&hand_made.id).unwrap().content, hand_made.content);
        assert_eq!(user_profile_count(&app), 1);
    }

    #[test]
    fn forge_trial_uses_template_with_single_round() {
        let (app, _temp, _guard) = forge_test_app();
        let store = app.workflow_templates().unwrap();
        let template = &store.templates[0];
        let summary = app
            .forge_trial(&template.id, &app.prepare_forge_trial(template).unwrap().0.revision, "试运行样例需求")
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
        assert!(app.forge_trial("no-such-template", "unreviewed", "x").is_err());
        let store = app.workflow_templates().unwrap();
        assert!(app.forge_trial(&store.templates[0].id, "review", "  ").is_err());
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
            acp_ui_event(1, "userTextDelta", Some("继续"), Some(serde_json::json!({ "source": "goldBandPrompt", "promptId": "forge-prompt" }))),
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
            .forge_extract_turn_delta(&task_id, "run-1", "round-1", "node-1", "attempt-1", Some("forge-prompt"))
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
            app.forge_extract_turn_delta(&normal.task.id, "run-1", "round-1", "node-1", "attempt-1", Some("forge-prompt"))
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
                        Some(serde_json::json!({ "source": "goldBandPrompt", "promptId": "forge-prompt" })),
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
            app.forge_extract_turn_delta(&forge.task.id, "run-1", "round-1", "node-1", "attempt-1", Some("forge-prompt"))
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
        let summary = app.forge_trial(&template.id, &app.prepare_forge_trial(template).unwrap().0.revision, "试运行").unwrap();
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

    #[test]
    fn forge_items_material_reads_agent_directory_skills() {
        let (app, _temp, _guard) = forge_test_app();
        // agent 目录(如 .claude/skills)下的 skill 不在 Gold Band 自己的 skills
        // 目录内:按列表给出的目录路径读取,而不是按名称回查 Gold Band 目录。
        let skill_dir = app
            .paths
            .repo_root
            .join(".claude")
            .join("skills")
            .join("forge-agent-dir-probe");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: forge-agent-dir-probe\ndescription: agent 目录内的探测 skill\n---\n审查正文:逐文件核对变更与风险。",
        )
        .unwrap();
        let material = app
            .forge_items_material(&[ForgeSourceItem {
                catalog: ForgeMaterialCatalog::Skill,
                source: crate::config::SkillSource::Project,
                asset_id: crate::skill::forge_skill_asset_id(&skill_dir),
            }])
            .unwrap();
        assert!(material.content.contains("审查正文:逐文件核对变更与风险"));
    }

    /// Spec 不实现相等比较：按序列化结果比较。
    fn spec_json<T: serde::Serialize>(value: &T) -> serde_json::Value {
        serde_json::to_value(value).expect("spec is serializable")
    }

    fn expect_saved(outcome: Result<ForgeCompileOutcome>) -> ForgeCompileSaved {
        match outcome.unwrap() {
            ForgeCompileOutcome::Saved(saved) => saved,
            ForgeCompileOutcome::Repairable { errors, repair_prompt } => {
                panic!("expected saved outcome, got repairable: {errors:?} / {repair_prompt}")
            }
        }
    }

    fn first_worker_goal(app: &App, template_id: &str) -> String {
        let store = app.workflow_templates().unwrap();
        let template = store
            .templates
            .iter()
            .find(|template| template.id == template_id)
            .unwrap();
        let crate::dsl::NodeDsl::Worker(worker) = &template.workflow.nodes[0] else {
            panic!("expected worker node");
        };
        worker.goal.clone().unwrap_or_default()
    }

    fn forged_count(app: &App) -> usize {
        app.workflow_templates()
            .unwrap()
            .templates
            .into_iter()
            .filter(|template| template.source == Some(crate::app::TemplateSource::Forged))
            .count()
    }

    /// 用户级角色数量（不含内置）：铸造角色不应重复建、也不应改动数量之外的东西。
    fn user_profile_count(app: &App) -> usize {
        app.profiles()
            .unwrap()
            .profiles
            .into_iter()
            .filter(|profile| !profile.is_built_in)
            .count()
    }

    const REVISED_GOAL: &str =
        "<输入>会话上下文\n<产出>修订后的产物\n<边界>不做部署\n<验收>产物过schema";

    #[test]
    fn recompile_updates_the_session_templates_in_place() {
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
        let task_id = &summary.task.id;

        let ForgeCompileOutcome::Saved(first) =
            app.forge_compile(&sample_spec(), Some(task_id), 1).unwrap()
        else {
            panic!("expected saved outcome");
        };
        assert_eq!((first.created_count, first.updated_count), (1, 0));

        let mut revised = app.forge_spec(task_id).unwrap().unwrap();
        revised.workflows[0].stages[0].goal = REVISED_GOAL.into();
        let ForgeCompileOutcome::Saved(second) =
            app.forge_compile(&revised, Some(task_id), 1).unwrap()
        else {
            panic!("expected saved outcome");
        };

        assert_eq!((second.created_count, second.updated_count), (0, 1));
        assert_eq!(second.template_ids, first.template_ids);
        assert_eq!(app.forge_template_targets(task_id).unwrap(), first.template_ids);
        assert_eq!(forged_count(&app), 1);
        assert_eq!(first_worker_goal(&app, &first.template_ids[0]), REVISED_GOAL);
    }

    #[test]
    fn recompile_creates_a_new_template_when_the_previous_one_is_gone() {
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
        let task_id = &summary.task.id;
        let ForgeCompileOutcome::Saved(first) =
            app.forge_compile(&sample_spec(), Some(task_id), 1).unwrap()
        else {
            panic!("expected saved outcome");
        };
        app.delete_workflow_template(&first.template_ids[0]).unwrap();

        let archived = app.forge_spec(task_id).unwrap().unwrap();
        assert_eq!(app.forge_compile(&archived, Some(task_id), 1).unwrap_err().downcast_ref::<ForgeError>().unwrap().code(), "forge/target-unavailable");
        let second = expect_saved(app.forge_compile_as_copy(&archived, task_id, &archived.workflows[0].id, "Recovered", 1));
        assert_ne!(first.template_ids, second.template_ids);
        assert_eq!(forged_count(&app), 1);
        assert_eq!(
            app.forge_template_targets(task_id).unwrap(),
            second.template_ids
        );
        assert_eq!(
            first_worker_goal(&app, &second.template_ids[0]),
            sample_spec().workflows[0].stages[0].goal.worker_text().unwrap()
        );
    }

    fn seed_source(app: &App, task_id: &str, template_id: &str) -> ForgeSpec {
        let (spec, binding) = app.forge_project_source(template_id).unwrap();
        app.seed_forge_source(task_id, spec, binding).unwrap();
        app.forge_spec(task_id).unwrap().unwrap()
    }

    fn template_of(app: &App, template_id: &str) -> crate::app::WorkflowTemplate {
        app.workflow_templates()
            .unwrap()
            .templates
            .into_iter()
            .find(|template| template.id == template_id)
            .expect("template exists")
    }

    fn forge_task(app: &App) -> String {
        app.create_forge_task(CreateForgeTaskInput {
            title: Some("铸造会话".to_string()),
            requirement_content: "源会话摘要".to_string(),
            agent_type: "claude-acp".to_string(),
            workflow: None,
            initial_spec: None,
        })
        .unwrap()
        .task
        .id
    }

    fn worker_of(template: &crate::app::WorkflowTemplate, index: usize) -> &crate::dsl::WorkerNode {
        match &template.workflow.nodes[index] {
            crate::dsl::NodeDsl::Worker(worker) => worker,
            other => panic!("expected worker node, got {other:?}"),
        }
    }

    #[test]
    fn recompile_keeps_bindings_slots_and_control_of_matched_nodes() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let first = expect_saved(app.forge_compile(&sample_spec(), Some(&task_id), 1));
        let template_id = first.template_ids[0].clone();

        // 模拟用户在工作流编辑器里的配置：首节点改绑模型、调大轮数上限。
        let before = template_of(&app, &template_id);
        let first_slot = worker_of(&before, 0)
            .execution_slot_id
            .clone()
            .unwrap();
        let mut workflow = before.workflow.clone();
        workflow.control.max_rounds = Some(7);
        let mut bindings = before.model_bindings.clone();
        bindings.bindings[0].model_id = Some("custom-model".to_string());
        app.update_workflow_template_with_bindings(&template_id, workflow, bindings)
            .unwrap();

        // 对话修改：工作流改名、首节点 goal 改写、追加一个阶段。
        let mut revised = app.forge_spec(&task_id).unwrap().unwrap();
        revised.name = Some("接口开发与测试".to_string());
        revised.workflows[0].name = "接口开发与测试".to_string();
        revised.workflows[0].stages[0].goal = REVISED_GOAL.into();
        let mut added = sample_spec().workflows[0].stages[0].clone();
        added.id = Some("review".to_string());
        revised.workflows[0].stages.push(added);
        let second = expect_saved(app.forge_compile(&revised, Some(&task_id), 1));

        assert_eq!((second.created_count, second.updated_count), (0, 1));
        assert_eq!(second.template_ids, [template_id.clone()]);
        let after = template_of(&app, &template_id);
        // 模板名跟随 Spec，且与定义变更同次落盘。
        assert_eq!(after.name, "接口开发与测试");
        // 模板级可调项保留。
        assert_eq!(after.workflow.control.max_rounds, Some(7));
        // 既有节点：slot 与模型绑定原样保留，只覆盖 Spec 表达的 goal。
        let kept = worker_of(&after, 0);
        assert_eq!(kept.execution_slot_id.as_deref(), Some(first_slot.as_str()));
        assert_eq!(kept.goal.as_deref(), Some(REVISED_GOAL));
        let kept_binding = after
            .model_bindings
            .bindings
            .iter()
            .find(|binding| binding.execution_slot_id == first_slot)
            .expect("binding of the kept node survives recompile");
        assert_eq!(kept_binding.model_id.as_deref(), Some("custom-model"));
        // 新节点：按 stage id 取节点 id，有自己的 slot 与默认绑定。
        let added = worker_of(&after, 1);
        assert_eq!(added.id, "review");
        let added_slot = added.execution_slot_id.clone().unwrap();
        assert_ne!(added_slot, first_slot);
        assert!(after
            .model_bindings
            .bindings
            .iter()
            .any(|binding| binding.execution_slot_id == added_slot));
    }

    #[test]
    fn recompile_saves_a_new_template_when_the_target_left_the_forge_model() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let first = expect_saved(app.forge_compile(&sample_spec(), Some(&task_id), 1));
        let template_id = first.template_ids[0].clone();

        // 用户在编辑器里给模板加了一条指向真实节点的失败分支（Forge 的线性
        // 链表达不了）：不再原位覆盖这些手工改动，而是另存新模板。
        let before = template_of(&app, &template_id);
        let mut edited = before.workflow.clone();
        let mut fallback = worker_of(&before, 0).clone();
        fallback.id = "fallback".to_string();
        fallback.execution_slot_id = None;
        let first_node_id = worker_of(&before, 0).id.clone();
        edited.nodes.push(crate::dsl::NodeDsl::Worker(fallback));
        // 每个节点只能有一条失败边：编译产物的重试自环让位给这条手工加的分支。
        edited.edges.retain(|edge| {
            !(edge.from == first_node_id && edge.on == crate::dsl::EdgeOutcome::Failure)
        });
        edited.edges.push(crate::dsl::EdgeDsl {
            from: first_node_id,
            to: "fallback".to_string(),
            on: crate::dsl::EdgeOutcome::Failure,
            session: None,
            new_round_entry: None,
        });
        edited.edges.push(crate::dsl::EdgeDsl {
            from: "fallback".to_string(),
            to: crate::dsl::END_NODE.to_string(),
            on: crate::dsl::EdgeOutcome::Success,
            session: None,
            new_round_entry: None,
        });
        app.update_workflow_template(&template_id, edited).unwrap();

        let archived = app.forge_spec(&task_id).unwrap().unwrap();
        let id = &archived.workflows[0].id;
        assert_eq!(app.forge_compile(&archived, Some(&task_id), 1).unwrap_err().downcast_ref::<ForgeError>().unwrap().code(), "forge/target-unavailable");
        assert_eq!(repairable_codes(app.forge_compile_as_copy(&archived, &task_id, id, &before.name, 1)), ["forge/duplicate-workflow-name"]);
        let second = expect_saved(app.forge_compile_as_copy(&archived, &task_id, id, "Recovered", 1));

        assert_eq!((second.created_count, second.updated_count), (1, 0));
        assert_eq!(forged_count(&app), 2);
        // 手工改动原样留在旧模板里；新模板是 Spec 表达的线性链。
        assert_eq!(template_of(&app, &template_id).workflow.nodes.len(), 2);
        assert_eq!(
            template_of(&app, &second.template_ids[0]).workflow.nodes.len(),
            1
        );
        // 对应关系推进到新模板：之后的编译继续原位更新它。
        assert_eq!(
            app.forge_template_targets(&task_id).unwrap(),
            second.template_ids
        );
    }

    #[test]
    fn template_targets_advance_per_template_when_a_later_save_fails() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        // 两条工作流：第二条的 custom 角色名为空白，铸造角色时失败（第一条已落库）。
        let mut spec = sample_spec();
        spec.name = None;
        spec.workflows[0].name = "流程甲".to_string();
        let mut broken = sample_spec().workflows[0].stages[0].clone();
        broken.id = Some("b1".to_string());
        broken.role = StageRole::Custom {
            profile_name: "   ".to_string(),
        };
        spec.workflows.push(ForgeWorkflow {
            id: String::new(),
            parallel_groups: vec![],
            name: "流程乙".to_string(),
            stages: vec![broken],
        });

        let error = app
            .forge_compile(&spec, Some(&task_id), 1)
            .expect_err("the second template must fail to save");
        assert!(error.to_string().contains("profile"), "{error}");

        // 已落库的第一条被记入对应关系：修正后重试时原位更新它，不再建一份。
        let targets = app.forge_template_targets(&task_id).unwrap();
        assert_eq!(targets.len(), 1);
        let mut fixed = app.forge_spec(&task_id).unwrap().unwrap();
        fixed.workflows[1].stages[0].role = StageRole::Review;
        let retried = expect_saved(app.forge_compile(&fixed, Some(&task_id), 1));
        assert_eq!((retried.created_count, retried.updated_count), (1, 1));
        assert_eq!(retried.template_ids[0], targets[0]);
        assert_eq!(forged_count(&app), 2);
    }

    #[test]
    fn modify_session_recompiles_into_its_source_template() {
        let (app, _temp, _guard) = forge_test_app();
        // 存量模板（在铸造会话之外编译得到）。
        let ForgeCompileOutcome::Saved(first) = app.forge_compile(&sample_spec(), None, 1).unwrap()
        else {
            panic!("expected saved outcome");
        };
        let source_template_id = first.template_ids[0].clone();

        // 新开会话修改存量工作流：会话在创建时即绑定源模板。
        let mut command = crate::app::CreateConversationTaskCommand::new(
            "test",
            "把样本流程改严谨一点".to_string(),
            crate::config::ConversationRunMode::Direct,
        );
        command.direct_config = Some(crate::config::ConversationDirectConfig {
            agent_type: "claude-acp".to_string(),
            model_id: None,
            permission_mode: None,
            auto_accept: false,
            config_options: Default::default(),
            model_bound_overrides: Default::default(),
        });
        command.forge_source_template_id = Some(source_template_id.clone());
        let prepared = app.prepare_conversation_task(&command).unwrap();
        let (task_id, _, _) = prepared.accept();
        assert_eq!(
            app.forge_template_targets(&task_id).unwrap(),
            [source_template_id.clone()]
        );

        // 修改后的 Spec 编译：落回源模板，不新建。
        let mut revised = app.forge_spec(&task_id).unwrap().unwrap();
        revised.workflows[0].stages[0].goal = REVISED_GOAL.into();
        let ForgeCompileOutcome::Saved(saved) =
            app.forge_compile(&revised, Some(&task_id), 1).unwrap()
        else {
            panic!("expected saved outcome");
        };
        assert_eq!((saved.created_count, saved.updated_count), (0, 1));
        assert_eq!(saved.template_ids, [source_template_id.clone()]);
        assert_eq!(forged_count(&app), 1);
        assert_eq!(first_worker_goal(&app, &source_template_id), REVISED_GOAL);
    }

    /// 把一轮对话写进独立的 attempt 时间线：用户 prompt + 助手回复（含围栏增量）。
    fn write_forge_turn(app: &App, task_id: &str, attempt: &str, reply: &str) {
        let attempt_dir =
            app.paths
                .attempt_dir(task_id, "run-1", "round-1", "node-1", attempt);
        std::fs::create_dir_all(attempt_dir.as_std_path()).unwrap();
        let events = vec![
            acp_ui_event(
                1,
                "userTextDelta",
                Some("继续"),
                Some(serde_json::json!({ "source": "goldBandPrompt", "promptId": "forge-prompt" })),
            ),
            acp_ui_event(2, "textDelta", Some(reply), None),
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
    }

    fn stage_ids(spec: &ForgeSpec) -> Vec<String> {
        spec.workflows[0]
            .stages
            .iter()
            .map(|stage| stage.id.clone().unwrap_or_default())
            .collect()
    }

    #[test]
    fn forge_extract_turn_delta_assigns_stage_ids_and_aligns_later_deltas_by_id() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);

        // 首轮：两个没有 id 的 stage → 系统补齐稳定 id 并存档。
        write_forge_turn(
            &app,
            &task_id,
            "attempt-1",
            "```json\n{\"intent\":\"实现登录\",\"stages\":[\
                {\"role\":\"plan\",\"goal\":\"<输入>需求\\n<产出>计划\\n<边界>不写代码\\n<验收>计划可执行\",\"outputKind\":\"none\",\"onFailure\":\"retry\",\"manualCheck\":false},\
                {\"role\":\"dev\",\"goal\":\"<输入>计划\\n<产出>代码\\n<边界>不做部署\\n<验收>测试通过\",\"outputKind\":\"none\",\"onFailure\":\"retry\",\"manualCheck\":false}]}\
                \n```",
        );
        let first = app
            .forge_extract_turn_delta(&task_id, "run-1", "round-1", "node-1", "attempt-1", Some("forge-prompt"))
            .unwrap()
            .expect("first delta merged");
        assert_eq!(stage_ids(&first), ["plan-1", "dev-1"]);

        // 第二轮只说“改一下测试阶段”却没带 id：是新 stage，不能按序号覆盖到第一个阶段上。
        write_forge_turn(
            &app,
            &task_id,
            "attempt-2",
            "```json\n{\"stages\":[{\"role\":\"test\",\"goal\":\"<输入>代码\\n<产出>测试报告\\n<边界>不改代码\\n<验收>用例全过\",\"outputKind\":\"none\",\"onFailure\":\"retry\",\"manualCheck\":false}]}\n```",
        );
        let second = app
            .forge_extract_turn_delta(&task_id, "run-1", "round-1", "node-1", "attempt-2", Some("forge-prompt"))
            .unwrap()
            .expect("second delta merged");
        assert_eq!(stage_ids(&second), ["plan-1", "dev-1", "test-1"]);
        assert_eq!(second.workflows[0].stages[0].goal, first.workflows[0].stages[0].goal);

        // 第三轮带上基线里的 id：命中的 stage 整体覆盖，其余不动。
        write_forge_turn(
            &app,
            &task_id,
            "attempt-3",
            "```json\n{\"stages\":[{\"id\":\"dev-1\",\"role\":\"dev\",\"goal\":\"<输入>计划\\n<产出>代码与说明\\n<边界>不做部署\\n<验收>测试通过\",\"outputKind\":\"none\",\"onFailure\":\"manual-check\",\"manualCheck\":true}]}\n```",
        );
        let third = app
            .forge_extract_turn_delta(&task_id, "run-1", "round-1", "node-1", "attempt-3", Some("forge-prompt"))
            .unwrap()
            .expect("third delta merged");
        assert_eq!(stage_ids(&third), ["plan-1", "dev-1", "test-1"]);
        assert!(third.workflows[0].stages[1].goal.worker_text().unwrap().contains("代码与说明"));
        assert_eq!(third.workflows[0].stages[0].goal, first.workflows[0].stages[0].goal);
        assert_eq!(
            stage_ids(&app.forge_spec(&task_id).unwrap().unwrap()),
            ["plan-1", "dev-1", "test-1"]
        );
    }

    #[test]
    fn forge_settle_turn_reports_whether_the_archive_advanced() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let settle = |attempt: &str| {
            app.forge_settle_turn(&task_id, "run-1", "round-1", "node-1", attempt, Some("forge-prompt"), crate::app::AcpTurnOutcome::Completed)
                .unwrap()
        };

        // 普通对话轮（回复里没有增量）：是铸造会话，但存档没有变化。
        write_forge_turn(&app, &task_id, "attempt-1", "好的，我们先确认第一个阶段。");
        assert_eq!(
            settle("attempt-1"),
            Some(ForgeTurnSettlement { spec_updated: false, error: None })
        );
        assert!(app.forge_spec(&task_id).unwrap().is_none());

        // 带增量的轮：存档在返回之前已经更新（调用方据此再通知前端）。
        let dev_stage = "\"role\":\"dev\",\"goal\":\"<输入>需求\\n<产出>代码\\n<边界>不做部署\\n<验收>测试通过\",\"outputKind\":\"none\",\"onFailure\":\"retry\",\"manualCheck\":false";
        write_forge_turn(
            &app,
            &task_id,
            "attempt-2",
            &format!("```json\n{{\"stages\":[{{{dev_stage}}}]}}\n```"),
        );
        assert_eq!(
            settle("attempt-2"),
            Some(ForgeTurnSettlement { spec_updated: true, error: None })
        );
        assert_eq!(app.forge_spec(&task_id).unwrap().unwrap().workflows[0].stages.len(), 1);

        // 真机回归：模型的围栏正常闭合，JSON 却漏了最外层的 `}`——补全后照常应用。
        write_forge_turn(
            &app,
            &task_id,
            "attempt-3",
            &format!("```json\n{{\"intent\":\"实现登录\",\"stages\":[{{{dev_stage}}}]}}\n```"),
        );
        assert_eq!(
            settle("attempt-3"),
            Some(ForgeTurnSettlement { spec_updated: true, error: None })
        );
        assert_eq!(app.forge_spec(&task_id).unwrap().unwrap().intent, "实现登录");
        // 这个增量里的 stage 没带 id：按协议是新 stage，追加在既有 stage 之后。
        assert_eq!(app.forge_spec(&task_id).unwrap().unwrap().workflows[0].stages.len(), 2);

        // 增量确实无法解析（截断在字符串里）：这一轮同样已经结束，必须带着原因收口。
        // 只记一条警告而不通知前端的话，自动修正回路会一直停在“正在修正”。
        write_forge_turn(&app, &task_id, "attempt-4", "```json\n{\"stages\": [{\"id\": \"s1\", \"goal\": \"没写完\n```");
        let settlement = settle("attempt-4").unwrap();
        assert!(!settlement.spec_updated);
        let error = settlement.error.expect("unparsable delta is reported");
        assert_eq!(error.code(), "forge/invalid-spec");
        // 存档保持上一轮的结果。
        assert_eq!(app.forge_spec(&task_id).unwrap().unwrap().workflows[0].stages.len(), 2);

        // 非铸造会话：不是铸造回路的事，返回 None。
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
            .unwrap()
            .task
            .id;
        assert_eq!(
            app.forge_settle_turn(&normal, "run-1", "round-1", "node-1", "attempt-1", Some("forge-prompt"), crate::app::AcpTurnOutcome::Completed)
                .unwrap(),
            None
        );
    }

    #[test]
    fn forged_templates_get_a_default_binding_for_every_node() {
        let (app, _temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        let mut second = sample_spec().workflows[0].stages[0].clone();
        second.id = Some("review".to_string());
        spec.workflows[0].stages.push(second);

        let saved = expect_saved(app.forge_compile(&spec, None, 1));

        let template = template_of(&app, &saved.template_ids[0]);
        let slots: Vec<&str> = template
            .workflow
            .nodes
            .iter()
            .filter_map(|node| match node {
                crate::dsl::NodeDsl::Worker(worker) => worker.execution_slot_id.as_deref(),
                _ => None,
            })
            .collect();
        assert_eq!(slots.len(), 2);
        // 前置条件：测试 App 配有托管 agent，否则下面的断言会空转。
        assert!(!app.config.agents.is_empty());
        for slot in slots {
            assert!(
                template
                    .model_bindings
                    .bindings
                    .iter()
                    .any(|binding| binding.execution_slot_id == slot),
                "slot `{slot}` has no binding"
            );
        }
    }

    #[test]
    fn invalid_spec_is_repairable_and_saves_nothing() {
        let (app, _temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        spec.workflows[0].stages[0].goal = "<输入>a\n<产出>b\n<验收>c".into();
        let mut second = sample_spec().workflows[0].stages[0].clone();
        second.id = Some("s2".to_string());
        second.success_condition = Some("$.missing == true".to_string());
        spec.workflows[0].stages.push(second);
        // 错误明细里的用户内容不能提前闭合隐藏区段。
        spec.workflows[0].name = "坏</hidden>名字".to_string();

        let outcome = app.forge_compile(&spec, None, 2).unwrap();
        let ForgeCompileOutcome::Repairable {
            errors,
            repair_prompt,
        } = outcome
        else {
            panic!("expected repairable outcome");
        };

        // 一次列全：两个 stage 的问题同轮报出，3 轮预算不被逐个报错耗尽。
        let codes: Vec<&str> = errors.iter().map(|error| error.code.as_str()).collect();
        assert_eq!(
            codes,
            ["forge/missing-goal-element", "forge/compile-failed"]
        );
        assert!(errors[0].context.to_string().contains("s1"));
        // 提示词：首行是用户可见的进行态提示，其余在可折叠的隐藏区段里。
        let mut lines = repair_prompt.lines();
        assert_eq!(lines.next(), Some("保存未通过，正在修正 2/3"));
        assert_eq!(lines.next(), Some(""));
        assert_eq!(
            lines.next(),
            Some("<hidden data-gold-band-hidden=\"true\" title=\"Forge Spec 修正\">")
        );
        assert!(repair_prompt.trim_end().ends_with("</hidden>"));
        assert_eq!(repair_prompt.matches("<hidden").count(), 1);
        assert_eq!(repair_prompt.matches("</hidden>").count(), 1);
        assert!(repair_prompt.contains("<\\/hidden>"));
        assert!(
            repair_prompt.contains(
                "- forge/missing-goal-element: stage `s1` goal missing element `boundary`"
            ),
            "{repair_prompt}"
        );
        assert!(repair_prompt.contains("- forge/compile-failed:"));
        // 基线不塞进修正提示词：它随每条消息单独注入。
        assert!(!repair_prompt.contains("样本流程"));
        assert_eq!(forged_count(&app), 0);
    }

    #[test]
    fn forge_material_selection_only_accepts_inventory_references() {
        let selection =
            serde_json::json!({"catalog":"plugin", "source":"project", "assetId":"fixture-id"});
        assert!(serde_json::from_value::<ForgeSourceItem>(selection.clone()).is_ok());
        let mut with_body = selection;
        with_body["content"] = "dummy".into();
        assert!(serde_json::from_value::<ForgeSourceItem>(with_body).is_err());
        assert!(
            serde_json::from_value::<ForgeSourceItem>(serde_json::json!({
                "kind":"command", "source":"project", "name":"dummy", "content":"dummy"
            }))
            .is_err()
        );
    }

    #[test]
    fn source_material_covers_plugin_commands_and_agents() {
        let (app, _temp, _guard) = forge_test_app();
        let base = app.paths.repo_root.join(".claude/skills/probe");
        for dir in [".claude-plugin", "commands", "agents"] {
            std::fs::create_dir_all(base.join(dir)).unwrap();
        }
        std::fs::write(base.join(".claude-plugin/plugin.json"), r#"{"name":"probe","commands":{"release":{"source":"commands/release.md"},"about":{"content":"内联命令正文"}}}"#).unwrap();
        std::fs::write(
            base.join("commands/release.md"),
            "---
description: 发布流程
---
发布命令正文",
        )
        .unwrap();
        std::fs::write(
            base.join("agents/auditor.md"),
            "---
description: 审计
---
审计员职责正文",
        )
        .unwrap();
        let plugins = crate::skill::claude_plugin::scan_project_plugins(&app.paths.repo_root);
        let items: Vec<_> = ["release", "auditor", "about"]
            .into_iter()
            .map(|name| ForgeSourceItem {
                asset_id: plugins[0]
                    .assets
                    .iter()
                    .find(|a| a.name == name)
                    .unwrap()
                    .asset_id
                    .clone(),
                source: crate::config::SkillSource::Project,
                catalog: ForgeMaterialCatalog::Plugin,
            })
            .collect();
        let stale = ForgeSourceItem {
            asset_id: "fixture-unlisted".into(),
            ..items[0].clone()
        };
        assert!(
            app.forge_items_digest(std::slice::from_ref(&stale))
                .unwrap_err()
                .is::<ForgeMaterialUnavailable>()
        );
        assert!(
            app.forge_items_material(&[stale])
                .unwrap_err()
                .is::<ForgeMaterialUnavailable>()
        );

        let material = app.forge_items_material(&items).unwrap();
        let text = &material.content;
        assert!(text.contains("## command: release（项目 · claude-acp · plugin probe）"));
        assert!(text.contains("## agent: auditor（项目 · claude-acp · plugin probe）"));
        assert!(text.contains("发布命令正文"));
        assert!(text.contains("审计员职责正文"));
        assert!(text.contains("内联命令正文"));
        // frontmatter 不进入正文。
        assert!(!text.contains("description:"));

        let digest = app.forge_items_digest(&items).unwrap();
        assert_eq!(digest[0].name, "release");
        assert_eq!(digest[0].kind, ForgeSourceKind::Command);
        assert_eq!(digest[0].description, "发布流程");
        assert_eq!(digest[1].kind, ForgeSourceKind::Agent);
        assert_eq!(digest[1].description, "审计");
        assert!(digest[2].description.is_empty());

        std::fs::write(
            base.join(".claude-plugin/plugin.json"),
            r#"{"name":"probe","commands":{"about":{"content":"updated fixture"}}}"#,
        )
        .unwrap();
        let fresh = app.forge_items_material(&items[2..]).unwrap();
        assert!(fresh.content.contains("updated fixture"));
        assert!(!fresh.content.contains("内联命令正文"));
        assert!(
            app.forge_items_material(&items)
                .unwrap_err()
                .is::<ForgeMaterialUnavailable>()
        );
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
    fn forge_compile_creates_forged_template_with_builtin_profiles() {
        let (app, _temp, _guard) = forge_test_app();
        let ids = expect_saved(app.forge_compile(&sample_spec(), None, 1)).template_ids;
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
    fn forge_compile_backfills_custom_profile() {
        let (app, _temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        spec.workflows[0].stages[0].role = StageRole::Custom {
            profile_name: "接口联调".to_string(),
        };
        spec.workflows[0].stages[0].role_definition =
            Some("你是接口联调工程师:负责打通服务间调用、核对契约并输出联调结论。".to_string());
        let ids = expect_saved(app.forge_compile(&spec, None, 1)).template_ids;
        let store = app.workflow_templates().unwrap();
        let template = store
            .templates
            .iter()
            .find(|t| t.id == ids[0])
            .expect("forged template saved");
        let crate::dsl::NodeDsl::Worker(worker) = &template.workflow.nodes[0] else {
            panic!("expected worker node");
        };
        // 占位被回填为真实 User scope profile id，且可解析、正文为角色职责。
        let profile = worker.profile.as_deref().expect("profile backfilled");
        assert_ne!(profile, "s1-profile");
        assert!(profile.starts_with("pf-"));
        let entry = crate::app::profiles::find_profile_by_id(
            &app.paths,
            profile,
            crate::config::DesktopLanguage::ZhCn,
        )
        .unwrap()
        .expect("forged profile resolvable");
        // 角色显示名带所属工作流：角色库是用户级的，工作流多了才分得清归属。
        assert_eq!(entry.name, "样本流程 · 接口联调");
        assert!(entry.content.contains("接口联调工程师"));
    }

    #[test]
    fn forge_recompile_updates_role_in_place_via_archive_writeback() {
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

        // 首次编译:创建自定义角色,profileId 回写存档。
        let mut spec = sample_spec();
        spec.workflows[0].stages[0].role = StageRole::Custom {
            profile_name: "联调员".to_string(),
        };
        expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
        let archived = app.forge_spec(&task_id).unwrap().unwrap();
        let forged_id = archived.workflows[0].stages[0]
            .profile_id
            .clone()
            .expect("profile id written back to archive");

        // 对话反哺:用户在会话中改角色名与职责,重编译按 id 原地更新。
        let mut revised = archived;
        revised.workflows[0].stages[0].role = StageRole::Custom {
            profile_name: "高级联调工程师".to_string(),
        };
        revised.workflows[0].stages[0].role_definition =
            Some("你是高级联调工程师:除联调外还负责契约评审与风险上报。".to_string());
        expect_saved(app.forge_compile(&revised, Some(&task_id), 1));

        let entry = crate::app::profiles::find_profile_by_id(
            &app.paths,
            &forged_id,
            crate::config::DesktopLanguage::ZhCn,
        )
        .unwrap()
        .expect("same profile still resolvable");
        assert_eq!(entry.name, "样本流程 · 高级联调工程师");
        assert!(entry.content.contains("契约评审"));
        // 不产生重复:联调角色只有一个。
        let count = crate::app::profiles::list_profiles(
            &app.paths,
            crate::config::DesktopLanguage::ZhCn,
        )
        .unwrap()
        .profiles
        .iter()
        .filter(|p| p.content.contains("联调"))
        .count();
        assert_eq!(count, 1, "recompile must not duplicate the forged role");
    }

    fn repairable_codes(outcome: Result<ForgeCompileOutcome>) -> Vec<String> {
        match outcome.unwrap() {
            ForgeCompileOutcome::Repairable { errors, .. } => {
                errors.into_iter().map(|error| error.code).collect()
            }
            ForgeCompileOutcome::Saved(saved) => {
                panic!("expected repairable outcome, got saved {saved:?}")
            }
        }
    }

    fn profile_ids_of(app: &App, template_id: &str) -> Vec<Option<String>> {
        template_of(app, template_id)
            .workflow
            .nodes
            .iter()
            .map(|node| node.profile().map(str::to_string))
            .collect()
    }

    fn edge_shape(workflow: &crate::dsl::WorkflowDsl) -> Vec<[String; 5]> {
        let mut shape: Vec<_> = workflow
            .edges
            .iter()
            .map(|edge| {
                [
                    edge.from.clone(),
                    edge.to.clone(),
                    format!("{:?}", edge.on),
                    edge.session
                        .map(|session| format!("{session:?}"))
                        .unwrap_or_default(),
                    edge.new_round_entry.clone().unwrap_or_default(),
                ]
            })
            .collect();
        shape.sort();
        shape
    }

    fn stage_with_role(id: &str, profile: &str) -> crate::dsl::NodeDsl {
        crate::dsl::NodeDsl::Worker(crate::dsl::WorkerNode {
            id: id.to_string(),
            execution_slot_id: None,
            provider: None,
            model: None,
            profile: Some(profile.to_string()),
            goal: None,
            output: None,
            success_condition: None,
            permission_mode: None,
            auto_accept: false,
            config_options: Default::default(),
            manual_check: None,
            prompt_envelope: Default::default(),
        })
    }

    fn edge(from: &str, to: &str, on: crate::dsl::EdgeOutcome, session: Option<crate::domain::SessionMode>) -> crate::dsl::EdgeDsl {
        crate::dsl::EdgeDsl {
            from: from.to_string(),
            to: to.to_string(),
            on,
            session,
            new_round_entry: None,
        }
    }

    fn save_template(app: &App, name: &str, workflow: crate::dsl::WorkflowDsl) -> String {
        let store = app
            .save_workflow_template(name.to_string(), workflow)
            .unwrap();
        store
            .templates
            .into_iter()
            .find(|template| template.name == name)
            .expect("saved template is listed")
            .id
    }

    #[test]
    fn stages_with_the_same_role_name_share_one_forged_role() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let mut spec = sample_spec();
        spec.workflows[0].stages.push(custom_stage("audit-1", "审计员", None));
        spec.workflows[0].stages.push(custom_stage("audit-2", "审计员", None));
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));

        // 同名 stage 只建一个角色：两个节点引用同一个 profile，存档里回填同一个 id。
        assert_eq!(user_profile_count(&app), 1);
        let refs = profile_ids_of(&app, &saved.template_ids[0]);
        let role_id = refs[1].clone().expect("custom-1 references a profile");
        assert_eq!(refs[2].as_deref(), Some(role_id.as_str()));
        let archived = app.forge_spec(&task_id).unwrap().unwrap();
        assert_eq!(persona_of(&archived, "audit-1").1.as_deref(), Some(role_id.as_str()));
        assert_eq!(persona_of(&archived, "audit-2").1.as_deref(), Some(role_id.as_str()));

        // 之后再加一个同名角色的 stage：归入同一个角色，不另建。
        let mut grown = archived;
        grown.workflows[0].stages.push(custom_stage("audit-3", "审计员", None));
        expect_saved(app.forge_compile(&grown, Some(&task_id), 1));
        assert_eq!(user_profile_count(&app), 1);
        let refs = profile_ids_of(&app, &saved.template_ids[0]);
        assert_eq!(refs[3].as_deref(), Some(role_id.as_str()));

        // 同名角色只能有一份职责正文：两份不同的正文报错，交回修正。
        let mut conflicting = app.forge_spec(&task_id).unwrap().unwrap();
        conflicting.workflows[0].stages[1].role_definition = Some("第一套职责".to_string());
        conflicting.workflows[0].stages[2].role_definition = Some("第二套职责".to_string());
        assert_eq!(
            repairable_codes(app.forge_compile(&conflicting, Some(&task_id), 1)),
            ["forge/role-definition-conflict"]
        );
    }

    #[test]
    fn each_localized_goal_example_saves_and_keeps_its_values() {
        use crate::config::DesktopLanguage::*;
        let (mut app, _temp, _guard) = forge_test_app();
        for lang in [ZhCn, ZhTw, En, JaJp, KoKr, PtBr, Es] {
            app.config.desktop_language = lang;
            let example = crate::prompts::FORGE_SPEC_PROTOCOL.resolve(lang)
                .split("```json").nth(1).unwrap().split("```").next().unwrap();
            let example: serde_json::Value = serde_json::from_str(example.trim()).unwrap();
            let goal = example["stages"][0]["goal"].clone();
            let mut spec = sample_spec();
            spec.name = Some(format!("Goal-{lang:?}"));
            spec.workflows[0].name = spec.name.clone().unwrap();
            spec.workflows[0].stages[0].goal = serde_json::from_value(goal.clone()).unwrap();
            let task = forge_task(&app);
            let saved = expect_saved(app.forge_compile(&spec, Some(&task), 1));
            let archived = app.forge_spec(&task).unwrap().unwrap();
            assert_eq!(serde_json::to_value(&archived.workflows[0].stages[0].goal).unwrap(), goal);
            let worker = first_worker_goal(&app, &saved.template_ids[0]);
            for key in ["input", "output", "boundary", "acceptance"] {
                assert!(worker.contains(goal[key].as_str().unwrap()), "{lang:?}: {key}");
            }
        }
    }

    #[test]
    fn compile_keeps_authorized_role_and_switches_to_existing_persona() {
        let (app, _temp, _guard) = forge_test_app();
        let task = forge_task(&app);
        let mut spec = sample_spec();
        spec.workflows[0].stages.extend([custom_stage("first", "Analyst", None), custom_stage("second", "Reviewer", None)]);
        spec.workflows[0].stages[2].role_definition = Some("Review the report".into());
        let saved = expect_saved(app.forge_compile(&spec, Some(&task), 1));
        let template = &saved.template_ids[0];
        let initial = profile_ids_of(&app, template);
        let mut revised = app.forge_spec(&task).unwrap().unwrap();
        revised.workflows[0].stages[1].profile_id = Some("unbound-fixture-id".into());
        expect_saved(app.forge_compile(&revised, Some(&task), 1));
        assert_eq!(profile_ids_of(&app, template), initial);
        let baseline = app.forge_spec(&task).unwrap().unwrap();
        let mut stage = baseline.workflows[0].stages[1].clone();
        stage.role = baseline.workflows[0].stages[2].role.clone();
        stage.profile_id = None;
        let delta = crate::forge::interview::SpecDelta { stages: vec![stage], ..Default::default() };
        let revised = crate::forge::interview::merge_delta(&baseline, &delta).unwrap();
        expect_saved(app.forge_compile(&revised, Some(&task), 1));
        let ids = profile_ids_of(&app, template);
        assert_eq!(ids[1], initial[2]);
        assert_eq!(ids[2], initial[2]);
        assert_eq!(app.profile_show(ids[1].as_ref().unwrap()).unwrap().content, "Review the report");
    }

    #[test]
    fn unavailable_bound_role_does_not_change_any_other_role_or_template() {
        let (app, temp, _guard) = forge_test_app();
        let task = forge_task(&app);
        let mut spec = sample_spec();
        spec.workflows[0].stages.extend([custom_stage("a", "Alpha", None), custom_stage("b", "Beta", None)]);
        let saved = expect_saved(app.forge_compile(&spec, Some(&task), 1));
        let template = &saved.template_ids[0];
        let mut edit = app.forge_spec(&task).unwrap().unwrap();
        let alpha = app.profile_show(edit.workflows[0].stages[1].profile_id.as_ref().unwrap()).unwrap();
        let beta = app.profile_show(edit.workflows[0].stages[2].profile_id.as_ref().unwrap()).unwrap();
        // Simulate a missing, previously authorized role only within this disposable store.
        let fixture = std::fs::canonicalize(temp.path()).unwrap();
        let path = std::fs::canonicalize(&beta.path).unwrap();
        assert!(path.starts_with(&fixture));
        std::fs::remove_file(path).unwrap();
        let before = serde_json::to_value(template_of(&app, template)).unwrap();
        edit.workflows[0].stages[1].role_definition = Some("Changed charter".into());
        let outcome = app.forge_compile(&edit, Some(&task), 1);
        assert_eq!(app.profile_show(&alpha.id).unwrap().content, alpha.content);
        assert_eq!(serde_json::to_value(template_of(&app, template)).unwrap(), before);
        assert_eq!(app.forge_spec(&task).unwrap().unwrap().revision, edit.revision);
        assert_eq!(repairable_codes(outcome), ["forge/unavailable-role"]);
    }

    #[test]
    fn explicit_copy_rebuilds_a_recycled_role_from_its_archived_charter() {
        let (app, _temp, _guard) = forge_test_app();
        let task = forge_task(&app);
        let mut spec = sample_spec();
        spec.workflows[0].stages.push(custom_stage("audit", "Auditor", None));
        let first = expect_saved(app.forge_compile(&spec, Some(&task), 1));
        let archived = app.forge_spec(&task).unwrap().unwrap();
        let previous = archived.workflows[0].stages[1].profile_id.clone().unwrap();
        let charter = app.profile_show(&previous).unwrap().content;
        app.delete_workflow_template(&first.template_ids[0]).unwrap();
        assert!(app.profile_show(&previous).is_err());
        let copy = expect_saved(app.forge_compile_as_copy(&archived, &task, &archived.workflows[0].id, "Recovered", 1));
        let rebuilt = custom_profile_id(&app, &copy.template_ids[0], "audit");
        assert_ne!(rebuilt, previous);
        assert_eq!(app.profile_show(&rebuilt).unwrap().content, charter);
        assert_eq!(app.forge_spec(&task).unwrap().unwrap().workflows[0].stages[1].profile_id, Some(rebuilt));
    }

    #[test]
    fn forged_roles_are_recycled_once_nothing_references_them() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let mut spec = sample_spec();
        spec.workflows[0].stages.push(custom_stage("audit", "审计员", None));
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
        let role_id = profile_ids_of(&app, &saved.template_ids[0])[1]
            .clone()
            .unwrap();
        assert!(app.profile_show(&role_id).unwrap().forged);
        assert_eq!(user_profile_count(&app), 1);

        // 阶段在对话里被删掉：模板不再引用这个角色，角色随之回收。
        let mut trimmed = app.forge_spec(&task_id).unwrap().unwrap();
        trimmed
            .workflows[0]
            .stages
            .retain(|stage| stage.id.as_deref() != Some("audit"));
        expect_saved(app.forge_compile(&trimmed, Some(&task_id), 1));
        assert_eq!(user_profile_count(&app), 0);
        assert!(app.profile_show(&role_id).is_err());

        // 模板删除同理；用户手工建的角色即使不再被引用也不删。
        let hand_made = crate::app::profiles::create_profile(
            &app.paths,
            crate::app::profiles::ProfileInput {
                name: "严格审查员".to_string(),
                summary: "通用审查角色".to_string(),
                content: "严格审查，不修改文件。".to_string(),
                dynamic_template: false,
            },
        )
        .unwrap();
        let mut workflow = crate::forge::compiler::compile(&sample_spec()).unwrap().remove(0).workflow;
        let crate::dsl::NodeDsl::Worker(worker) = &mut workflow.nodes[0] else { panic!("worker") };
        worker.profile = Some(hand_made.id.clone());
        let library_template = save_template(&app, "库引用", workflow);
        app.delete_workflow_template(&library_template).unwrap();
        assert!(app.profile_show(&hand_made.id).is_ok());
        assert_eq!(user_profile_count(&app), 1);

        let mut spec = app.forge_spec(&task_id).unwrap().unwrap();
        spec.workflows[0].stages.push(custom_stage("audit", "发布审计员", None));
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
        let template_id = saved.template_ids[0].clone();
        let role_id = profile_ids_of(&app, &template_id)[1].clone().unwrap();
        let copy_id = save_template(&app, "引用同一角色的副本", template_of(&app, &template_id).workflow);
        app.delete_workflow_template(&template_id).unwrap();
        assert!(app.profile_show(&role_id).is_ok());
        app.delete_workflow_template(&copy_id).unwrap();
        assert!(app.profile_show(&role_id).is_err());
    }

    #[test]
    fn deleting_generated_template_keeps_roles_used_by_another_workspace() {
        let (app, temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        spec.workflows[0].stages.push(custom_stage("audit", "审计员", None));
        let saved = expect_saved(app.forge_compile(&spec, None, 1));
        let template = template_of(&app, &saved.template_ids[0]);
        let role_id = profile_ids_of(&app, &template.id)[1].clone().unwrap();
        let other_root = Utf8PathBuf::from_path_buf(temp.path().join("other-workspace")).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let other = App::with_config_and_path_config(other_root, app.config.clone(), app.paths.storage_path_config());
        assert_eq!(app.paths.projects_dir(), other.paths.projects_dir());
        assert_ne!(app.paths.tasks_dir(), other.paths.tasks_dir());
        other.create_task_from_requirement_with_bindings(CreateTaskInput {
            title: None, description: None, requirement_file_name: None,
            requirement_content: "复用模板".into(), workflow: template.workflow.clone(),
            workflow_template_id: Some(template.id.clone()), origin: None,
        }, template.workflow, template.model_bindings).unwrap();
        app.delete_workflow_template(&template.id).unwrap();
        assert!(other.profile_show(&role_id).is_ok(), "cross-workspace task lost its role");
    }

    #[test]
    fn duplicate_names_in_one_generation_save_nothing() {
        let (app, _temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        spec.workflows.push(spec.workflows[0].clone());
        assert_eq!(repairable_codes(app.forge_compile(&spec, None, 1)), ["forge/duplicate-workflow-name"]);
        assert_eq!(forged_count(&app), 0);
    }

    #[test]
    fn workflow_names_are_unique() {
        let (app, _temp, _guard) = forge_test_app();
        let first = expect_saved(app.forge_compile(&sample_spec(), None, 1));
        let template_id = first.template_ids[0].clone();
        let builtin_name = app.workflow_templates().unwrap().templates[0].name.clone();

        // 同名的第二个工作流：手工另存、改名、铸造新工作流都不允许。
        let workflow = template_of(&app, &template_id).workflow;
        let error = app
            .save_workflow_template("样本流程".to_string(), workflow.clone())
            .unwrap_err();
        assert!(error.to_string().contains("workflow-template.duplicate-name"), "{error:#}");
        let error = app
            .update_workflow_template_with_name(&template_id, Some(&builtin_name), workflow, Default::default())
            .unwrap_err();
        assert!(error.to_string().contains("workflow-template.duplicate-name"), "{error:#}");
        assert_eq!(
            repairable_codes(app.forge_compile(&sample_spec(), None, 1)),
            ["forge/duplicate-workflow-name"]
        );

        // 原位更新不与自己重名；改成一个没人用的名字可以。
        let task_id = forge_task(&app);
        let mut renamed = seed_source(&app, &task_id, &template_id);
        renamed.name = Some("样本流程二号".to_string());
        renamed.workflows[0].name = "样本流程二号".to_string();
        let saved = expect_saved(app.forge_compile(&renamed, Some(&task_id), 1));
        assert_eq!((saved.created_count, saved.updated_count), (0, 1));
        assert_eq!(template_of(&app, &template_id).name, "样本流程二号");
    }

    #[test]
    fn modify_session_keeps_failure_jumps_of_the_source_template() {
        let (app, _temp, _guard) = forge_test_app();
        // 手写工作流：测试失败时带着会话回到开发，开发到测试的成功边也沿用会话。
        let workflow = crate::dsl::WorkflowDsl {
            version: "0.1".to_string(),
            id: "repair-loop".to_string(),
            entry: "dev".to_string(),
            control: Default::default(),
            nodes: vec![
                stage_with_role("dev", "pf-builtin-dev"),
                stage_with_role("test", "pf-builtin-test"),
            ],
            edges: vec![
                edge("dev", "test", crate::dsl::EdgeOutcome::Success, Some(crate::domain::SessionMode::Continue)),
                edge("test", crate::dsl::END_NODE, crate::dsl::EdgeOutcome::Success, None),
                edge("test", "dev", crate::dsl::EdgeOutcome::Failure, Some(crate::domain::SessionMode::Continue)),
            ],
        };
        let template_id = save_template(&app, "修复回路", workflow.clone());
        let task_id = forge_task(&app);
        let mut spec = seed_source(&app, &task_id, &template_id);
        assert_eq!(
            spec.workflows[0].stages[1].on_failure,
            OnFailure::Jump {
                stage: "dev".to_string(),
                continue_session: true
            }
        );
        // 对话里只改了测试阶段的目标。
        spec.workflows[0].stages[1].goal =
            "<输入>实现\n<产出>测试结论\n<边界>不改实现\n<验收>逐项有结论".into();
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));

        assert_eq!(saved.updated_count, 1);
        let updated = template_of(&app, &template_id);
        assert_eq!(edge_shape(&updated.workflow), edge_shape(&workflow));
    }

    #[test]
    fn stage_failure_changes_in_the_preview_share_the_conversation_merge() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        let initial = app.save_forge_spec(&task_id, &sample_spec()).unwrap();

        // 目标不是这条工作流里的 stage：拒绝，存档不变。
        let error = app
            .forge_update_stage_failure(
                &task_id,
                "s1",
                OnFailure::Jump {
                    stage: "nope".to_string(),
                    continue_session: false,
                },
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<ForgeError>().map(ForgeError::code),
            Some("forge/unknown-failure-target")
        );
        assert_eq!(
            spec_json(&app.forge_spec(&task_id).unwrap().unwrap()),
            spec_json(&initial)
        );

        // 对话里加了一个阶段。
        let second = ForgeStage {
            id: Some("s2".to_string()),
            role: StageRole::Test,
            artifact: Some("s2-result".to_string()),
            ..sample_spec().workflows[0].stages[0].clone()
        };
        let after_turn = crate::forge::interview::merge_delta(
            &initial,
            &crate::forge::interview::SpecDelta {
                stages: vec![second],
                ..Default::default()
            },
        )
        .unwrap();
        app.save_forge_spec(&task_id, &after_turn).unwrap();

        // 预览里把第一步改成“失败后从第二步开启新一轮”。
        let updated = app
            .forge_update_stage_failure(&task_id, "s1", OnFailure::NewRoundFrom("s2".to_string()))
            .unwrap();
        assert_eq!(
            updated.workflows[0].stages[0].on_failure,
            OnFailure::NewRoundFrom("s2".to_string())
        );
        // 其余内容原样保留，存档就是合并后的结果。
        assert_eq!(
            spec_json(&updated.workflows[0].stages[1]),
            spec_json(&after_turn.workflows[0].stages[1])
        );
        assert_eq!(
            spec_json(&updated.open_questions),
            spec_json(&after_turn.open_questions)
        );
        assert_eq!(
            spec_json(&app.forge_spec(&task_id).unwrap().unwrap()),
            spec_json(&updated)
        );

        // 编译产物带着这条“从 s2 重开”的失败边。
        let saved = expect_saved(app.forge_compile(&updated, None, 1));
        let template = template_of(&app, &saved.template_ids[0]);
        let restart: Vec<_> = template
            .workflow
            .edges
            .iter()
            .filter(|edge| edge.on == crate::dsl::EdgeOutcome::Failure)
            .collect();
        assert_eq!(restart.len(), 2, "s1 restarts the round and s2 keeps its retry loop");
        let restart = restart
            .iter()
            .find(|edge| edge.from == "s1")
            .expect("s1 failure edge");
        assert_eq!(
            (
                restart.to.as_str(),
                restart.new_round_entry.as_deref()
            ),
            (crate::dsl::NEW_ROUND_NODE, Some("s2"))
        );
    }

    #[test]
    fn editing_a_template_with_a_terminal_success_cycle_preserves_the_original() {
        use crate::dsl::{EdgeDsl, EdgeOutcome, END_NODE};
        let (app, _temp, _guard) = forge_test_app();
        let mut spec = sample_spec();
        let mut second = spec.workflows[0].stages[0].clone();
        second.id = Some("s2".into());
        spec.workflows[0].stages.push(second);
        let saved = expect_saved(app.forge_compile(&spec, None, 1));
        let id = &saved.template_ids[0];
        let mut graph = template_of(&app, id).workflow;
        let last = graph.nodes.last().unwrap().id().to_string();
        graph.edges.retain(|edge| edge.from != last);
        for (on, to) in [(EdgeOutcome::Success, graph.entry.clone()), (EdgeOutcome::Failure, END_NODE.to_string())] {
            graph.edges.push(EdgeDsl { from: last.clone(), on, to, session: None, new_round_entry: None });
        }
        app.update_workflow_template(id, graph).unwrap();
        let before = serde_json::to_value(template_of(&app, id).workflow).unwrap();
        assert!(app.forge_from_workflow(id).is_err());
        assert_eq!(serde_json::to_value(template_of(&app, id).workflow).unwrap(), before);
    }

    #[test]
    fn stage_failure_changes_are_limited_to_forge_sessions() {
        let (app, _temp, _guard) = forge_test_app();
        let task_id = forge_task(&app);
        app.save_forge_spec(&task_id, &sample_spec()).unwrap();
        assert!(app
            .forge_update_stage_failure(&task_id, "missing", OnFailure::End)
            .is_err());

        let ordinary = app
            .create_task_from_requirement(CreateTaskInput {
                title: Some("普通任务".to_string()),
                description: None,
                requirement_file_name: None,
                requirement_content: "普通任务".to_string(),
                workflow: forge_conversation_workflow("claude-acp"),
                workflow_template_id: None,
                origin: None,
            })
            .unwrap();
        assert!(app.save_forge_spec(&ordinary.task.id, &sample_spec()).is_err());
        assert!(app.forge_spec(&ordinary.task.id).is_err());
        assert!(!crate::forge::archive::spec_archive_path(&app.paths.task_dir(&ordinary.task.id)).exists());
        assert!(app
            .forge_update_stage_failure(&ordinary.task.id, "s1", OnFailure::End)
            .is_err());
    }

    #[test]
    fn modifying_builtin_template_forks_with_its_edges_bindings_and_limits() {
        let (app, _temp, _guard) = forge_test_app();
        let source = template_of(&app, crate::app::DEFAULT_WORKFLOW_TEMPLATE_ID);
        let task_id = forge_task(&app);
        let mut spec = seed_source(&app, &task_id, &source.id);
        spec.name = Some("团队回归流程".into());
        spec.workflows[0].name = "团队回归流程".into();
        for stage in &mut spec.workflows[0].stages { stage.goal = REVISED_GOAL.into(); }
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
        let copy = template_of(&app, &saved.template_ids[0]);
        assert_ne!(copy.id, source.id);
        assert_eq!(copy.workflow.control.max_attempts, source.workflow.control.max_attempts);
        assert_eq!(copy.workflow.control.max_rounds, source.workflow.control.max_rounds);
        assert_eq!(edge_shape(&copy.workflow), edge_shape(&source.workflow));
        for binding in &source.model_bindings.bindings {
            assert!(copy.model_bindings.bindings.contains(binding));
        }
        for node in &source.workflow.nodes {
            let crate::dsl::NodeDsl::Worker(old) = node else { unreachable!() };
            let retained = copy.workflow.nodes.iter().find(|n| n.id() == old.id).unwrap();
            let crate::dsl::NodeDsl::Worker(retained) = retained else { unreachable!() };
            assert_eq!(retained.execution_slot_id, old.execution_slot_id);
        }
        assert_eq!(spec_json(&template_of(&app, &source.id).workflow), spec_json(&source.workflow));
        let latest = app.forge_spec(&task_id).unwrap().unwrap();
        let saved_again = expect_saved(app.forge_compile(&latest, Some(&task_id), 1));
        assert_eq!(saved_again.template_ids, [copy.id]);
        assert_eq!(saved_again.updated_count, 1);
    }

    #[test]
    fn builtin_roles_beyond_the_five_are_read_only_references() {
        let (app, _temp, _guard) = forge_test_app();
        let cleanup = app.profile_show("pf-builtin-cleanup").unwrap();
        assert!(cleanup.is_built_in);
        let workflow = crate::dsl::WorkflowDsl {
            version: "0.1".to_string(),
            id: "with-cleanup".to_string(),
            entry: "cleanup".to_string(),
            control: Default::default(),
            nodes: vec![stage_with_role("cleanup", "pf-builtin-cleanup")],
            edges: vec![edge(
                "cleanup",
                crate::dsl::END_NODE,
                crate::dsl::EdgeOutcome::Success,
                None,
            )],
        };
        let template_id = save_template(&app, "带清理的流程", workflow);
        let task_id = forge_task(&app);
        let spec = seed_source(&app, &task_id, &template_id);
        let stage = &spec.workflows[0].stages[0];
        // 内置角色作为只读引用进入草稿：带 id 与显示名，不把内置正文当作可编辑的职责。
        assert_eq!(
            stage.role,
            StageRole::Custom {
                profile_name: cleanup.name.clone()
            }
        );
        assert_eq!(stage.profile_id.as_deref(), Some("pf-builtin-cleanup"));
        assert_eq!(stage.role_definition, None);

        // 重新编译：节点仍引用内置角色；内置角色没被改动，也没有新建用户角色。
        let before = user_profile_count(&app);
        let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
        assert_eq!(
            profile_ids_of(&app, &saved.template_ids[0])[0].as_deref(),
            Some("pf-builtin-cleanup")
        );
        assert_eq!(user_profile_count(&app), before);
        assert_eq!(app.profile_show("pf-builtin-cleanup").unwrap().name, cleanup.name);

        // 在草稿里给内置角色改名：报只读，交回修正。
        let mut renamed = app.forge_spec(&task_id).unwrap().unwrap();
        renamed.workflows[0].stages[0].role = StageRole::Custom {
            profile_name: "清理工".to_string(),
        };
        assert_eq!(
            repairable_codes(app.forge_compile(&renamed, Some(&task_id), 1)),
            ["forge/readonly-role"]
        );
    }
}
