//! One local source conversation, two bound templates, one real parallel trial. Model replies
//! are deterministic ACP timeline inputs; the provider probe observes actual worker admission.
#[path = "forge_mixed_material_case.rs"]
mod mixed_material;

use super::{BranchProbe, app_with_provider, fixture_git};
use camino::Utf8PathBuf;
use gold_band::{
    app::{
        AcpTurnOutcome, App, CreateConversationTaskCommand, WorkflowTemplate,
        forge_service::{ForgeCompileOutcome, ForgeCompileSaved, ForgeSourceSession},
    },
    config::{ConversationDirectConfig, ConversationRunMode},
    domain::RunOutcome,
    dsl::{END_NODE, EdgeOutcome, NodeDsl, WorkflowDsl},
    forge::ForgeSpec,
    provider::PromptHiddenSection,
    storage::{GoldBandPaths, read_json},
    workflow_model_binding::{TaskAuthoringWorkflow, WorkerModelBinding},
};
use serde_json::{Value, json};
use std::fs;

const NAME: &str = "交付前并行检查";
const COPY: &str = "交付前并行检查（无障碍版）";
const REQUEST: &str = "交付前要同时检查接口和页面，然后汇总。";
const PLAN: &str = "checks 分两支：api-check 核对接口，ui-check 检查页面；最多同时执行两支，两支完成后由 report 汇总。";
const APPROVAL: &str = "确认这个方案。请保留两个独立分支，不要改成串行。";
const UI_EDIT: &str = "只改 ui-check：验收时还要检查键盘焦点。api-check、并发上限和汇总都不动。";
const COPY_EDIT: &str = "只在副本的 ui-check 验收中再加读屏检查，原版不动。";

fn direct(prompt: &str) -> CreateConversationTaskCommand {
    let mut command =
        CreateConversationTaskCommand::new("test", prompt.into(), ConversationRunMode::Direct);
    command.direct_config = Some(ConversationDirectConfig {
        agent_type: "claude-acp".into(),
        model_id: None,
        permission_mode: None,
        auto_accept: false,
        config_options: Default::default(),
        model_bound_overrides: Default::default(),
    });
    command
}
fn rows(app: &App, task: &str, attempt: &str, messages: &[(&str, &str, &str)]) {
    let dir = app
        .paths
        .attempt_dir(task, "run-1", "round-1", "node-1", attempt);
    fs::create_dir_all(&dir).unwrap();
    let mut timeline = String::new();
    for (i, &(kind, text, prompt)) in messages.iter().enumerate() {
        let raw = if kind == "userTextDelta" {
            json!({"source":"goldBandPrompt","promptId":prompt})
        } else {
            Value::Null
        };
        timeline.push_str(&json!({"item":{"id":format!("msg-{i}"),"seq":i+1,"kind":kind,"timestamp":"2026-01-01T00:00:00Z","content":text,"raw":raw}}).to_string());
        timeline.push('\n');
    }
    fs::write(dir.join("acp.timeline.jsonl"), timeline).unwrap();
}
fn settle(app: &App, task: &str, attempt: &str, prompt: &str, delta: Value) -> ForgeSpec {
    let reply = format!("```json\n{delta}\n```");
    rows(
        app,
        task,
        attempt,
        &[
            ("userTextDelta", prompt, attempt),
            ("textDelta", &reply, ""),
        ],
    );
    let result = app
        .forge_settle_turn(
            task,
            "run-1",
            "round-1",
            "node-1",
            attempt,
            Some(attempt),
            AcpTurnOutcome::Completed,
        )
        .unwrap()
        .unwrap();
    assert!(result.spec_updated && result.error.is_none(), "{result:?}");
    app.forge_spec(task).unwrap().unwrap()
}
fn saved(result: anyhow::Result<ForgeCompileOutcome>) -> ForgeCompileSaved {
    match result.unwrap() {
        ForgeCompileOutcome::Saved(saved) => saved,
        other => panic!("expected save: {other:?}"),
    }
}
fn template(app: &App, id: &str) -> WorkflowTemplate {
    app.workflow_templates()
        .unwrap()
        .templates
        .into_iter()
        .find(|t| t.id == id)
        .unwrap()
}
fn node<'a>(workflow: &'a WorkflowDsl, id: &str) -> &'a NodeDsl {
    workflow.nodes.iter().find(|n| n.id() == id).unwrap()
}
fn binding<'a>(template: &'a WorkflowTemplate, id: &str) -> &'a WorkerModelBinding {
    let NodeDsl::Worker(worker) = node(&template.workflow, id) else {
        panic!("worker required")
    };
    template
        .model_bindings
        .bindings
        .iter()
        .find(|b| Some(&b.execution_slot_id) == worker.execution_slot_id.as_ref())
        .unwrap()
}
fn group(cap: usize) -> Value {
    json!({"id":"checks","branches":[{"id":"api","stages":["api-check"]},{"id":"ui","stages":["ui-check"]}],"maxParallel":cap,"onFailure":"end"})
}
fn goal(acceptance: &str) -> Value {
    json!({"input":"合成交付样例","output":"核查结果","boundary":"只读","acceptance":acceptance})
}
fn stage(id: &str, role: &str, acceptance: &str) -> Value {
    json!({"id":id,"role":role,"goal":goal(acceptance),"outputKind":"json","onFailure":"end",
        "artifact":"implementation-result", "schemaHint":{"pass":"boolean"}, "successCondition":"$.pass == true"})
}
fn edit_ui(app: &App, task: &str, prompt: &str, acceptance: &str, attempt: &str) -> ForgeSpec {
    let base = app.forge_spec(task).unwrap().unwrap();
    let w = &base.workflows[0];
    let mut changed = json!(
        w.stages
            .iter()
            .find(|s| s.id.as_deref() == Some("ui-check"))
            .unwrap()
    );
    changed["goal"] = goal(acceptance);
    changed.as_object_mut().unwrap().remove("profileId");
    settle(
        app,
        task,
        attempt,
        prompt,
        json!({"workflowId":w.id,"stages":[changed]}),
    )
}
fn reopen(app: &App, id: &str, prompt: &str) -> String {
    let mut command = direct(prompt);
    command.forge_source_template_id = Some(id.into());
    let task = app.prepare_conversation_task(&command).unwrap().accept().0;
    assert_eq!(app.forge_template_targets(&task).unwrap(), [id]);
    task
}

#[test]
fn session_create_edit_save_as_reopen_and_run_parallel_trial() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Utf8PathBuf::from_path_buf(temp.path().to_owned()).unwrap();
    let paths = GoldBandPaths::new(repo.clone());
    // with_provider uses the active storage path config; refuse a real home *before* App startup.
    let relative_home = paths
        .user_gold_band_root
        .strip_prefix(&repo)
        .expect("test requires disposable storage only");
    fixture_git(&repo, &["init", "-q"]);
    fixture_git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.hooksPath=NUL",
            "commit",
            "--allow-empty",
            "-qm",
            "synthetic baseline",
        ],
    );
    gold_band::git::GitRepositoryService::default()
        .ensure_local_exclude(
            &repo,
            &format!("/{}/", relative_home.as_str().replace('\\', "/")),
        )
        .unwrap();
    let probe = BranchProbe::default();
    let mut app = app_with_provider(repo.clone(), Box::new(probe.clone()));
    app.config
        .agents
        .retain(|id, _| id.as_str() == "claude-acp");

    let source = app
        .prepare_conversation_task(&direct(REQUEST))
        .unwrap()
        .accept()
        .0;
    rows(
        &app,
        &source,
        "attempt-1",
        &[
            ("userTextDelta", REQUEST, "request"),
            ("textDelta", PLAN, ""),
            ("userTextDelta", APPROVAL, "approval"),
        ],
    );
    let session = ForgeSourceSession {
        task_uuid: app.task_show(&source).unwrap().uuid,
        run_id: "run-1".into(),
        round_id: "round-1".into(),
        node_id: "node-1".into(),
        attempt_id: "attempt-1".into(),
        branch_id: "root".into(),
        outer_node_id: None,
        outer_attempt_id: None,
        after: None,
    };
    let mut create = direct("按确认的方案铸造「交付前并行检查」。");
    create.forge_source_task_id = Some(source.clone());
    create.forge_source_session = Some(session);
    let forge = app.prepare_conversation_task(&create).unwrap().accept().0;
    let hidden: Vec<PromptHiddenSection> =
        read_json(&app.paths.first_prompt_hidden_sections_file(&forge)).unwrap();
    let material = &hidden
        .iter()
        .find(|s| s.content.contains(PLAN))
        .expect("source plan reached first prompt")
        .content;
    assert!(
        material.find(REQUEST).unwrap() < material.find(PLAN).unwrap()
            && material.find(PLAN).unwrap() < material.find(APPROVAL).unwrap()
    );
    let draft = settle(
        &app,
        &forge,
        "anchor",
        &create.content,
        json!({"name":NAME,"intent":REQUEST,"openQuestions":[],"stages":[stage("api-check","test","接口状态正确"),stage("ui-check","test","页面可用"),stage("report","accept","汇总两份结果")],"parallelGroups":[group(2)]}),
    );
    let created = saved(app.forge_compile(&draft, Some(&forge), 1));
    assert_eq!((created.created_count, created.updated_count), (1, 0));
    let original_id = &created.template_ids[0];
    let mut original = template(&app, original_id);
    assert_eq!(original.name, NAME);
    assert_eq!(original.workflow.entry, "checks");
    let g = json!(node(&original.workflow, "checks"));
    assert_eq!(g["type"], "parallel-group");
    assert_eq!(g["maxParallel"], 2);
    assert_eq!(g["branches"], group(2)["branches"]);
    assert!(
        original
            .workflow
            .edges
            .iter()
            .any(|e| e.from == "checks" && e.to == "report" && e.on == EdgeOutcome::Success)
    );
    // Non-default controls and a non-default model make resetting execution settings observable.
    original.workflow.control.max_attempts = Some(4);
    let slot = binding(&original, "api-check").execution_slot_id.clone();
    original
        .model_bindings
        .bindings
        .iter_mut()
        .find(|b| b.execution_slot_id == slot)
        .unwrap()
        .model_id = Some("gpt-5.4".into());
    app.update_workflow_template_with_bindings(
        original_id,
        original.workflow,
        original.model_bindings,
    )
    .unwrap();
    let before = template(&app, original_id);
    let modify = reopen(&app, original_id, UI_EDIT);
    let edited = edit_ui(
        &app,
        &modify,
        UI_EDIT,
        "页面可用，键盘焦点可见",
        "edit-original",
    );
    let update = saved(app.forge_compile(&edited, Some(&modify), 1));
    assert_eq!((update.created_count, update.updated_count), (0, 1));
    assert_eq!(&update.template_ids, &created.template_ids);
    let original = template(&app, original_id);
    let mut expected = json!(before.workflow);
    let actual = json!(original.workflow);
    let new_goal = node(&original.workflow, "ui-check").goal().unwrap();
    assert!(new_goal.contains("键盘焦点可见"));
    let changed = expected["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["id"] == "ui-check")
        .unwrap();
    changed["goal"] = json!(new_goal);
    assert_eq!(
        actual, expected,
        "the original save may change only the UI worker goal"
    );
    assert_eq!(
        original.model_bindings.bindings,
        before.model_bindings.bindings
    );
    assert_eq!(
        original.model_bindings.binding_revision,
        before.model_bindings.binding_revision
    );

    let draft = app.forge_spec(&modify).unwrap().unwrap();
    let key = &draft.workflows[0].id;
    let library = json!(app.workflow_templates().unwrap());
    match app
        .forge_compile_as_copy(&draft, &modify, key, NAME, 1)
        .unwrap()
    {
        ForgeCompileOutcome::Repairable { errors, .. } => assert!(
            errors
                .iter()
                .any(|e| e.code == "forge/duplicate-workflow-name")
        ),
        got => panic!("duplicate accepted: {got:?}"),
    }
    assert_eq!(json!(app.workflow_templates().unwrap()), library);
    assert_eq!(
        json!(app.forge_spec(&modify).unwrap().unwrap()),
        json!(draft)
    );
    assert_eq!(
        app.forge_template_targets(&modify).unwrap(),
        [original_id.clone()]
    );
    let copied = saved(app.forge_compile_as_copy(&draft, &modify, key, COPY, 1));
    assert_eq!((copied.created_count, copied.updated_count), (1, 0));
    let copy_id = &copied.template_ids[0];
    let copy = template(&app, copy_id);
    assert_ne!(copy_id, original_id);
    assert_ne!(copy.workflow.id, original.workflow.id);
    assert_eq!(copy.name, COPY);
    assert_eq!(
        app.forge_template_targets(&modify).unwrap(),
        [copy_id.clone()]
    );
    assert_eq!(json!(node(&copy.workflow, "checks")), g);
    assert_eq!(node(&copy.workflow, "ui-check").goal(), Some(new_goal));
    assert_eq!(json!(template(&app, original_id)), json!(original));

    let edit_copy = reopen(&app, copy_id, COPY_EDIT);
    let copy_draft = edit_ui(
        &app,
        &edit_copy,
        COPY_EDIT,
        "页面可用，键盘焦点可见，读屏顺序正确",
        "edit-copy",
    );
    let saved_copy = saved(app.forge_compile(&copy_draft, Some(&edit_copy), 1));
    assert_eq!(saved_copy.template_ids, [copy_id.clone()]);
    let original_task = reopen(&app, original_id, "重开原版核对内容。");
    for (task, expected_reader) in [(&original_task, false), (&edit_copy, true)] {
        let projected = app.forge_spec(task).unwrap().unwrap();
        let ui = projected.workflows[0]
            .stages
            .iter()
            .find(|s| s.id.as_deref() == Some("ui-check"))
            .unwrap();
        assert_eq!(
            ui.goal.worker_text().unwrap().contains("读屏顺序正确"),
            expected_reader
        );
    }
    assert_eq!(json!(template(&app, original_id)), json!(original));
    let info = saved_copy.trial.unwrap();
    let limits = info.group.unwrap();
    assert_eq!((limits.branches, limits.cap), (2, 2));
    let trial = app
        .forge_trial(copy_id, &info.revision, "测试一份合成交付样例。")
        .unwrap();
    let frozen: TaskAuthoringWorkflow =
        read_json(&app.paths.workflow_file(&trial.task.id)).unwrap();
    assert_eq!(frozen.workflow.entry, "checks");
    assert_eq!(frozen.workflow.nodes.len(), 3);
    assert_eq!(json!(node(&frozen.workflow, "checks")), g);
    assert!(
        frozen
            .workflow
            .edges
            .iter()
            .any(|e| e.from == "checks" && e.to == END_NODE && e.on == EdgeOutcome::Success)
    );
    assert!(
        node(&frozen.workflow, "ui-check")
            .goal()
            .unwrap()
            .contains("读屏顺序正确")
    );
    let run = app.run_start(&trial.task.id, None).unwrap();
    assert_eq!(run.outcome, Some(RunOutcome::Success), "{run:?}");
    let state = probe.state.0.lock().unwrap();
    let mut calls: Vec<_> = state.2.iter().map(|(id, _)| id.as_str()).collect();
    calls.sort_unstable();
    assert_eq!(calls, ["api-check", "ui-check"]);
    assert_eq!(
        state.1, 2,
        "both workers must be inside the provider at once"
    );
    assert_ne!(state.2[0].1, state.2[1].1);
    assert!(state.2.iter().all(|(_, dir)| dir != &repo));
    println!(
        "original={original_id} copy={copy_id} trial={} run={} workers={calls:?} peak={} outcome={:?}",
        trial.task.id, run.id, state.1, run.outcome
    );
    drop(state);
    // Trial intentionally skips report. Reopen the saved copy, not the trial snapshot, for both
    // full runs. Only the fake UI check result changes; workflow and running code are identical.
    for fail_node in [None, Some("ui-check")] {
        let probe = BranchProbe {
            fail_node,
            ..Default::default()
        };
        let execution = app_with_provider(repo.clone(), Box::new(probe.clone()));
        let current = template(&execution, copy_id);
        let task = execution
            .create_task_from_requirement_with_bindings(
                super::CreateTaskInput {
                    title: Some(COPY.into()),
                    description: None,
                    requirement_file_name: None,
                    requirement_content: REQUEST.into(),
                    workflow: current.workflow.clone(),
                    workflow_template_id: Some(copy_id.clone()),
                    origin: None,
                },
                current.workflow,
                current.model_bindings,
            )
            .unwrap();
        let run = execution.run_start(&task.task.id, None).unwrap();
        let expected = if fail_node.is_some() {
            RunOutcome::Failure
        } else {
            RunOutcome::Success
        };
        assert_eq!(run.outcome, Some(expected), "{run:?}");
        let state = probe.state.0.lock().unwrap();
        let ids: Vec<_> = state.2.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(state.1, 2);
        assert!(matches!(
            &ids[..2],
            ["api-check", "ui-check"] | ["ui-check", "api-check"]
        ));
        assert_eq!(
            &ids[2..],
            if fail_node.is_none() {
                &["report"][..]
            } else {
                &[]
            }
        );
        println!(
            "full task={} run={} fail_node={fail_node:?} calls={ids:?} peak={} outcome={:?}",
            task.task.id, run.id, state.1, run.outcome
        );
    }
    assert_eq!(
        binding(&copy, "api-check").model_id,
        binding(&original, "api-check").model_id,
        "Save As dropped the unedited API branch's saved model binding"
    );
    for id in ["api-check", "ui-check", "report"] {
        assert_eq!(
            binding(&copy, id),
            binding(&original, id),
            "Save As changed {id}'s execution slot or configuration"
        );
    }
    assert_eq!(
        json!(copy.workflow.control),
        json!(original.workflow.control)
    );
}
