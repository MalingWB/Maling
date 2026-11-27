//! Cross-domain review regressions against the production App, no model or desktop shell.
use camino::Utf8PathBuf;
use gold_band::app::{
    App, CreateConversationTaskCommand, WorkflowTemplate,
    forge_service::{CreateForgeTaskInput, ForgeCompileOutcome, ForgeCompileSaved},
};
use gold_band::config::{ConversationDirectConfig, ConversationRunMode, RuntimeConfig};
use gold_band::forge::{ForgeError, ForgeSpec, OnFailure, StageRole};
use gold_band::storage::{GoldBandPaths, StoragePathConfig};
use serde_json::json;
use tempfile::TempDir;

#[path = "support/forge_review_material.rs"]
mod material;
#[path = "support/forge_source_exchange.rs"]
mod source_exchange;
#[path = "support/forge_review_roles.rs"]
mod roles;
#[path = "support/forge_review_topology.rs"]
mod topology;

fn fixture() -> (App, TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let path = Utf8PathBuf::from_path_buf(temp.path().to_owned()).unwrap();
    // An unset, test-specific override makes GoldBandPaths use the repository-local temp home.
    // Verify this before App construction; never mutate process environment or a real library.
    let cfg = StoragePathConfig {
        app_key: "forge-review",
        config_dir_name: ".gold-band",
        home_env_var: "GOLD_BAND_FORGE_REVIEW_FIXTURE_HOME",
    };
    let paths = GoldBandPaths::new_with_path_config(path.clone(), cfg);
    assert!(paths.user_gold_band_root.starts_with(&path));
    (
        App::with_config_and_path_config(path, RuntimeConfig::default(), cfg),
        temp,
    )
}

fn task(app: &App) -> String {
    app.create_forge_task(CreateForgeTaskInput {
        title: Some("Fixture".into()),
        requirement_content: "Fixture requirement".into(),
        agent_type: "claude-acp".into(),
        workflow: None,
        initial_spec: None,
    })
    .unwrap()
    .task
    .id
}

fn spec() -> ForgeSpec {
    serde_json::from_value(json!({ "specVersion":"0.1", "name":"First", "intent":"Check locally", "workflows":[{"name":"First","stages":[{"id":"s1","role":"dev","goal":{"input":"Fixture","output":"Report","boundary":"Local only","acceptance":"Checks pass"},"outputKind":"none","onFailure":"retry","manualCheck":false}]}] })).unwrap()
}

fn expect_saved(result: anyhow::Result<ForgeCompileOutcome>) -> ForgeCompileSaved {
    match result.unwrap() {
        ForgeCompileOutcome::Saved(saved) => saved,
        other => panic!("expected save, got {other:?}"),
    }
}
fn template_of(app: &App, id: &str) -> WorkflowTemplate {
    app.workflow_templates()
        .unwrap()
        .templates
        .into_iter()
        .find(|t| t.id == id)
        .unwrap()
}
fn direct_command() -> CreateConversationTaskCommand {
    let mut command = CreateConversationTaskCommand::new(
        "test",
        "Review fixture".into(),
        ConversationRunMode::Direct,
    );
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
fn source_task(app: &App, template_id: &str) -> (String, ForgeSpec) {
    let mut command = direct_command();
    command.forge_source_template_id = Some(template_id.into());
    let id = app.prepare_conversation_task(&command).unwrap().accept().0;
    let draft = app.forge_spec(&id).unwrap().unwrap();
    (id, draft)
}
fn write_forge_turn(app: &App, id: &str, attempt: &str, reply: &str) {
    let dir = app
        .paths
        .attempt_dir(id, "run-1", "round-1", "node-1", attempt);
    std::fs::create_dir_all(&dir).unwrap();
    let rows = [
        json!({"item":{"id":"u1","seq":1,"kind":"userTextDelta","timestamp":"1","content":"Continue","raw":{"source":"goldBandPrompt","promptId":"forge-prompt"}}}),
        json!({"item":{"id":"a2","seq":2,"kind":"textDelta","timestamp":"2","content":reply}}),
    ];
    std::fs::write(
        dir.join("acp.timeline.jsonl"),
        rows.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
}

#[test]
fn forge_settlement_never_applies_an_unconfirmed_or_oversized_turn() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let dir = app
        .paths
        .attempt_dir(&task, "run-1", "round-1", "node-1", "attempt-1");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("acp.timeline.jsonl");
    let prompt = |seq, id| json!({"item":{"id":format!("u{seq}"),"seq":seq,"kind":"userTextDelta","timestamp":"1","content":"Continue","raw":{"source":"goldBandPrompt","promptId":id}}});
    let reply = |seq, text: &str| json!({"item":{"id":format!("a{seq}"),"seq":seq,"kind":"textDelta","timestamp":"2","content":text}});
    let delta = "```json\n{\"intent\":\"current\"}\n```";
    let write = |rows: Vec<serde_json::Value>| {
        std::fs::write(
            &path,
            rows.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap()
    };
    let settle = |prompt_id: Option<&str>| {
        app.forge_settle_turn(
            &task,
            "run-1",
            "round-1",
            "node-1",
            "attempt-1",
            prompt_id,
            gold_band::app::AcpTurnOutcome::Completed,
        )
        .unwrap()
        .unwrap()
    };
    write(vec![
        prompt(1, "old"),
        reply(2, "previous"),
        prompt(3, "current"),
        reply(4, delta),
    ]);
    for id in [Some("old"), None] {
        let terminal = settle(id);
        assert!(
            !terminal.spec_updated && terminal.error.is_some(),
            "unconfirmed {id:?} applied a different turn: {terminal:?}"
        );
        assert!(app.forge_spec(&task).unwrap().is_none());
    }
    // A usable fence in the in-budget suffix is not proof we have a complete turn.
    write(vec![
        prompt(1, "current"),
        reply(2, &"x".repeat(1024 * 1024)),
        reply(3, delta),
    ]);
    gold_band::acp::timeline::read_indexed_timeline_page(&path, None, None, None, 1).unwrap();
    let terminal = settle(Some("current"));
    assert!(!terminal.spec_updated);
    assert_eq!(terminal.error.unwrap().code(), "forge/turn-unconfirmed");
    assert!(app.forge_spec(&task).unwrap().is_none());
    // Previously oversized history is harmless when the indexed *current* turn fits.
    write(vec![
        prompt(1, "old"),
        reply(2, &"x".repeat(1024 * 1024)),
        prompt(3, "current"),
        reply(4, delta),
    ]);
    gold_band::acp::timeline::read_indexed_timeline_page(&path, None, None, None, 1).unwrap();
    for outcome in [
        gold_band::app::AcpTurnOutcome::Failed,
        gold_band::app::AcpTurnOutcome::Cancelled,
    ] {
        let terminal = app
            .forge_settle_turn(
                &task,
                "run-1",
                "round-1",
                "node-1",
                "attempt-1",
                Some("current"),
                outcome,
            )
            .unwrap()
            .unwrap();
        assert!(!terminal.spec_updated);
        assert!(app.forge_spec(&task).unwrap().is_none());
    }
    assert!(settle(Some("current")).spec_updated);
    assert_eq!(app.forge_spec(&task).unwrap().unwrap().intent, "current");
}

#[test]
fn split_resave_reorder_and_retry_keep_both_bound_instances() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let mut spec = spec();
    spec.name = None;
    let mut second = spec.workflows[0].clone();
    second.name = "second".into();
    second.stages[0].id = Some("s2".into());
    spec.workflows.push(second);
    let initial = expect_saved(app.forge_compile(&spec, Some(&task), 1));
    let mut draft = initial.spec.unwrap();
    assert_ne!(draft.workflows[0].id, draft.workflows[1].id);
    let unchanged = expect_saved(app.forge_compile(&draft, Some(&task), 1));
    assert_eq!(unchanged.template_ids, initial.template_ids);
    draft = unchanged.spec.unwrap();
    draft.workflows.reverse();
    draft.workflows[0].name = "renamed".into();
    draft.workflows[1].stages[0].role = StageRole::Custom {
        profile_name: " ".into(),
    };
    let error = app.forge_compile(&draft, Some(&task), 1).unwrap_err();
    assert!(error.to_string().contains("profile"), "{error}");
    let expected = vec![
        initial.template_ids[1].clone(),
        initial.template_ids[0].clone(),
    ];
    assert_eq!(app.forge_template_targets(&task).unwrap(), expected);
    assert_eq!(template_of(&app, &expected[0]).name, "renamed");
    let mut fixed = app.forge_spec(&task).unwrap().unwrap();
    fixed.workflows[1].stages[0].role = StageRole::Dev;
    let retry = expect_saved(app.forge_compile(&fixed, Some(&task), 1));
    assert_eq!(retry.template_ids, expected);
    assert_eq!((retry.updated_count, retry.created_count), (2, 0));
}

#[test]
fn concurrent_cards_and_turn_merge_latest_and_stale_full_writes_cannot_commit() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let mut spec = spec();
    let mut second = spec.workflows[0].stages[0].clone();
    second.id = Some("s2".into());
    spec.workflows[0].stages.push(second);
    let saved = expect_saved(app.forge_compile(&spec, Some(&task), 1));
    let stale = saved.spec.unwrap();
    write_forge_turn(
        &app,
        &task,
        "attempt-1",
        "```json
{\"intent\":\"merged\"}
```",
    );
    let barrier = std::sync::Barrier::new(3);
    std::thread::scope(|scope| {
        let one = scope.spawn(|| {
            barrier.wait();
            app.forge_update_stage_failure(&task, "s1", OnFailure::NewRound)
                .unwrap()
        });
        let two = scope.spawn(|| {
            barrier.wait();
            app.forge_update_stage_failure(&task, "s2", OnFailure::NewRound)
                .unwrap()
        });
        barrier.wait();
        app.forge_settle_turn(&task, "run-1", "round-1", "node-1", "attempt-1", Some("forge-prompt"), gold_band::app::AcpTurnOutcome::Completed)
            .unwrap();
        assert_ne!(one.join().unwrap().revision, two.join().unwrap().revision);
    });
    let current = app.forge_spec(&task).unwrap().unwrap();
    assert_eq!(current.intent, "merged");
    assert_eq!(current.revision, stale.revision + 3);
    assert!(
        current.workflows[0]
            .stages
            .iter()
            .all(|s| s.on_failure == OnFailure::NewRound)
    );
    let mut late = stale;
    late.name = Some("late overwrite".into());
    for err in [
        app.save_forge_spec(&task, &late).unwrap_err(),
        app.forge_compile(&late, Some(&task), 1).unwrap_err(),
    ] {
        assert_eq!(
            err.downcast_ref::<ForgeError>().unwrap().code(),
            "forge/revision-conflict"
        );
    }
    assert_eq!(
        app.forge_spec(&task).unwrap().unwrap().revision,
        current.revision
    );
    assert_ne!(
        template_of(&app, &saved.template_ids[0]).name,
        "late overwrite"
    );
}

#[test]
fn historical_raw_draft_can_be_read_and_explicitly_recovered_without_rebinding() {
    let (app, _temp) = fixture();
    let source = expect_saved(app.forge_compile(&spec(), None, 1));
    let task = task(&app);
    let dir = app.paths.task_dir(&task);
    let original = template_of(&app, &source.template_ids[0]);
    let mut old = serde_json::to_value(spec()).unwrap();
    old.as_object_mut().unwrap().remove("revision");
    old["workflows"][0].as_object_mut().unwrap().remove("id");
    old["workflows"][0]["stages"][0]["profileId"] = serde_json::json!("pf-untrusted-legacy");
    let path = gold_band::forge::archive::spec_archive_path(&dir);
    gold_band::storage::write_json(&path, &old).unwrap();
    gold_band::storage::write_json(
        &dir.join("forge/templates.json"),
        &serde_json::json!({"templateIds":source.template_ids}),
    )
    .unwrap();
    let archived = app.forge_spec(&task).unwrap().unwrap();
    assert!(archived.workflows[0].stages[0].profile_id.is_none());
    assert!(app.forge_template_targets(&task).unwrap().is_empty());
    assert_eq!(
        app.forge_compile(&archived, Some(&task), 1)
            .unwrap_err()
            .downcast_ref::<ForgeError>()
            .unwrap()
            .code(),
        "forge/target-unavailable"
    );
    assert_eq!(
        gold_band::storage::read_json::<serde_json::Value>(&path).unwrap(),
        old,
        "read and rejection must not migrate on disk"
    );
    let saved = expect_saved(app.forge_compile_as_copy(
        &archived,
        &task,
        &archived.workflows[0].id,
        "Recovered old",
        1,
    ));
    assert_eq!(saved.created_count, 1);
    assert_ne!(saved.template_ids[0], original.id);
    assert_eq!(
        app.forge_template_targets(&task).unwrap(),
        saved.template_ids
    );
    assert_eq!(
        serde_json::to_value(template_of(&app, &original.id)).unwrap(),
        serde_json::to_value(original).unwrap()
    );
}

#[test]
fn stale_second_checkpoint_is_rejected_before_any_sibling_is_written() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let mut batch = spec();
    batch.name = None;
    let mut second = batch.workflows[0].clone();
    second.name = "Second".into();
    batch.workflows.push(second);
    let saved = expect_saved(app.forge_compile(&batch, Some(&task), 1));
    let first_before = serde_json::to_value(template_of(&app, &saved.template_ids[0])).unwrap();
    let second = template_of(&app, &saved.template_ids[1]);
    app.update_workflow_template_with_name(
        &second.id,
        Some("Editor rename"),
        second.workflow,
        second.model_bindings,
    )
    .unwrap();
    let mut draft = saved.spec.unwrap();
    draft.workflows[0].name = "Do not commit".into();
    let err = app.forge_compile(&draft, Some(&task), 1).unwrap_err();
    assert!(
        matches!(err.downcast_ref::<ForgeError>(), Some(ForgeError::TargetUnavailable { workflow }) if workflow == &draft.workflows[1].id)
    );
    assert_eq!(
        serde_json::to_value(template_of(&app, &saved.template_ids[0])).unwrap(),
        first_before
    );
    assert_eq!(template_of(&app, &second.id).name, "Editor rename");
    assert_eq!(
        app.forge_spec(&task).unwrap().unwrap().revision,
        draft.revision
    );
}

#[test]
fn historical_instance_only_binding_requires_explicit_copy() {
    let (app, _temp) = fixture();
    let id = expect_saved(app.forge_compile(&spec(), None, 1))
        .template_ids
        .remove(0);
    let (task, draft) = source_task(&app, &id);
    let key = &draft.workflows[0].id;
    let source = template_of(&app, &id);
    let path = gold_band::forge::archive::spec_archive_path(&app.paths.task_dir(&task));
    let mut old = serde_json::to_value(&draft).unwrap();
    old["targets"][key] = json!({"kind":"bound", "templateId":id, "instanceId":source.workflow.id});
    gold_band::storage::write_json(&path, &old).unwrap();
    let draft = app.forge_spec(&task).unwrap().unwrap();
    let err = app.forge_compile(&draft, Some(&task), 1).unwrap_err();
    assert!(
        matches!(err.downcast_ref::<ForgeError>(), Some(ForgeError::TargetUnavailable { workflow }) if workflow == key)
    );
    assert_eq!(
        gold_band::storage::read_json::<serde_json::Value>(&path).unwrap(),
        old
    );
    let copy = expect_saved(app.forge_compile_as_copy(&draft, &task, key, "Explicit recovery", 1));
    assert_ne!(copy.template_ids[0], id);
    assert_eq!(
        serde_json::to_value(template_of(&app, &id)).unwrap(),
        serde_json::to_value(source).unwrap()
    );
}

#[test]
fn bound_forge_keeps_intervening_execution_settings_on_a_retained_worker() {
    use gold_band::dsl::NodeDsl;
    let (app, _temp) = fixture();
    let id = expect_saved(app.forge_compile(&spec(), None, 1))
        .template_ids
        .remove(0);
    let (task, mut draft) = source_task(&app, &id);
    let source = template_of(&app, &id);
    let mut workflow = source.workflow;
    let NodeDsl::Worker(worker) = &mut workflow.nodes[0] else {
        panic!("worker")
    };
    worker.execution_slot_id = Some("editor-slot".into());
    workflow.control.max_rounds = Some(8);
    let mut bindings = source.model_bindings;
    bindings.bindings[0].execution_slot_id = "editor-slot".into();
    bindings.bindings[0].model_id = Some("editor-model".into());
    app.update_workflow_template_with_bindings(&id, workflow, bindings)
        .unwrap();
    draft.workflows[0].name = "Forge rename".into();
    draft.name = Some("Forge rename".into());
    assert_eq!(
        expect_saved(app.forge_compile(&draft, Some(&task), 1)).template_ids,
        [id.clone()]
    );
    let saved = template_of(&app, &id);
    let NodeDsl::Worker(worker) = &saved.workflow.nodes[0] else {
        panic!("worker")
    };
    assert_eq!(saved.name, "Forge rename");
    assert_eq!(worker.execution_slot_id.as_deref(), Some("editor-slot"));
    assert_eq!(saved.workflow.control.max_rounds, Some(8));
    assert_eq!(
        saved.model_bindings.bindings[0].execution_slot_id,
        "editor-slot"
    );
    assert_eq!(
        saved.model_bindings.bindings[0].model_id.as_deref(),
        Some("editor-model")
    );
}

#[test]
fn stale_bound_forge_cannot_overwrite_a_worker_added_in_the_template_editor() {
    use gold_band::dsl::{END_NODE, EdgeDsl, EdgeOutcome, NodeDsl};
    let (app, _temp) = fixture();
    let id = expect_saved(app.forge_compile(&spec(), None, 1))
        .template_ids
        .remove(0);
    let (task, mut draft) = source_task(&app, &id);

    let source = template_of(&app, &id);
    let mut edited = source.workflow;
    let NodeDsl::Worker(mut added) = edited.nodes[0].clone() else {
        panic!("worker")
    };
    added.id = "editor-review".into();
    added.execution_slot_id = None;
    edited.nodes.push(NodeDsl::Worker(added));
    let end = edited
        .edges
        .iter_mut()
        .find(|e| e.on == EdgeOutcome::Success)
        .unwrap();
    end.to = "editor-review".into();
    edited.edges.push(EdgeDsl {
        from: "editor-review".into(),
        to: END_NODE.into(),
        on: EdgeOutcome::Success,
        session: None,
        new_round_entry: None,
    });
    let mut editor_bindings = source.model_bindings;
    editor_bindings.bindings[0].model_id = Some("editor-only-model".into());
    app.update_workflow_template_with_bindings(&id, edited, editor_bindings)
        .unwrap();
    assert_eq!(
        app.forge_from_workflow(&id).unwrap().workflows[0]
            .stages
            .len(),
        2,
        "intervening edit is valid and Forge-representable"
    );
    let before = serde_json::to_value(template_of(&app, &id)).unwrap();

    draft.workflows[0].stages[0].goal = serde_json::from_value(json!({"input":"stale conversation","output":"Report","boundary":"Local only","acceptance":"Checks pass"})).unwrap();
    let draft = app.save_forge_spec(&task, &draft).unwrap();
    let result = app.forge_compile(&draft, Some(&task), 1);
    assert!(
        template_of(&app, &id)
            .workflow
            .nodes
            .iter()
            .any(|n| n.id() == "editor-review"),
        "stale Forge save deleted the editor's new worker"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err.downcast_ref::<ForgeError>(), Some(ForgeError::TargetUnavailable { workflow }) if workflow == &draft.workflows[0].id)
    );
    assert_eq!(
        app.forge_spec(&task).unwrap().unwrap().revision,
        draft.revision
    );
    let copy = expect_saved(app.forge_compile_as_copy(
        &draft,
        &task,
        &draft.workflows[0].id,
        "Recovered draft",
        1,
    ));
    assert_ne!(copy.template_ids[0], id);
    let copied = template_of(&app, &copy.template_ids[0]);
    assert_eq!(copied.workflow.nodes.len(), 1);
    assert_eq!(copied.model_bindings.bindings[0].model_id, None,
        "a stale template cannot supply runtime settings for an archive-only recovery");
    assert!(
        copied.workflow.nodes[0]
            .goal()
            .unwrap()
            .contains("stale conversation")
    );
    assert_eq!(
        serde_json::to_value(template_of(&app, &id)).unwrap(),
        before
    );
    assert_eq!(
        expect_saved(app.forge_compile(copy.spec.as_ref().unwrap(), Some(&task), 1)).template_ids,
        copy.template_ids
    );
}

#[test]
fn reinstalled_external_key_is_not_the_bound_source_instance() {
    let (app, _temp) = fixture();
    let source = expect_saved(app.forge_compile(&spec(), None, 1));
    let exchange = gold_band::app::MulticaWorkflowExchange {
        id: "fixture-external".into(),
        name: "Imported".into(),
        is_built_in: false,
        workflow: template_of(&app, &source.template_ids[0]).workflow,
    };
    let (first, _) = app
        .install_workflow_template_from_multica(&exchange)
        .unwrap();
    let (task, mut archived) = source_task(&app, &first.id);
    app.delete_workflow_template(&first.id).unwrap();
    let (replacement, _) = app
        .install_workflow_template_from_multica(&exchange)
        .unwrap();
    assert_eq!(replacement.id, first.id);
    assert_ne!(replacement.workflow.id, first.workflow.id);
    archived.name = Some("Must not overwrite".into());
    assert_eq!(
        app.forge_compile(&archived, Some(&task), 1)
            .unwrap_err()
            .downcast_ref::<ForgeError>()
            .unwrap()
            .code(),
        "forge/target-unavailable"
    );
    let copy = expect_saved(app.forge_compile_as_copy(
        &archived,
        &task,
        &archived.workflows[0].id,
        "Explicit recovery",
        1,
    ));
    assert_ne!(copy.template_ids[0], replacement.id);
    assert_eq!(
        serde_json::to_value(template_of(&app, &replacement.id)).unwrap(),
        serde_json::to_value(replacement).unwrap()
    );
}

#[test]
fn ordinary_and_out_of_scope_task_locators_cannot_read_or_replace_drafts() {
    let (app, _temp) = fixture();
    let forged = task(&app);
    let archived = app.save_forge_spec(&forged, &spec()).unwrap();
    let ordinary = app
        .prepare_conversation_task(&direct_command())
        .unwrap()
        .accept()
        .0;
    for locator in [&ordinary, &format!("../tasks/{forged}")] {
        assert!(app.forge_spec(locator).is_err());
        assert!(app.save_forge_spec(locator, &spec()).is_err());
        assert!(app.forge_compile(&spec(), Some(locator), 1).is_err());
    }
    assert!(!gold_band::forge::archive::spec_archive_path(&app.paths.task_dir(&ordinary)).exists());
    assert_eq!(
        app.forge_spec(&forged).unwrap().unwrap().revision,
        archived.revision
    );
}

#[test]
fn deleting_and_recreating_a_local_name_never_grants_the_old_session_write_authority() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let saved = expect_saved(app.forge_compile(&spec(), Some(&task), 1));
    let original = template_of(&app, &saved.template_ids[0]);
    let mut draft = saved.spec.unwrap();
    app.delete_workflow_template(&original.id).unwrap();
    let store = app
        .save_workflow_template(original.name.clone(), original.workflow)
        .unwrap();
    let replacement = store
        .templates
        .into_iter()
        .find(|t| t.name == original.name)
        .unwrap();
    assert_ne!(original.id, replacement.id);
    draft.name = Some("Must not overwrite".into());
    assert_eq!(
        app.forge_compile(&draft, Some(&task), 1)
            .unwrap_err()
            .downcast_ref::<ForgeError>()
            .unwrap()
            .code(),
        "forge/target-unavailable"
    );
    assert_eq!(
        serde_json::to_value(template_of(&app, &replacement.id)).unwrap(),
        serde_json::to_value(&replacement).unwrap()
    );
}

#[test]
fn multica_reuses_same_key_but_rejects_other_key_with_same_name() {
    let (app, _temp) = fixture();
    let first = expect_saved(app.forge_compile(&spec(), None, 1));
    let source = template_of(&app, &first.template_ids[0]);
    let mut exchange = gold_band::app::MulticaWorkflowExchange {
        id: "fixture-a".into(),
        name: "Imported".into(),
        is_built_in: false,
        workflow: source.workflow,
    };
    let imported = app
        .install_workflow_template_from_multica(&exchange)
        .unwrap()
        .0;
    let reused = app
        .install_workflow_template_from_multica(&exchange)
        .unwrap()
        .0;
    assert_eq!(reused.workflow.id, imported.workflow.id);
    exchange.id = "fixture-b".into();
    let err = app
        .install_workflow_template_from_multica(&exchange)
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<gold_band::app::WorkflowTemplateCommandError>(),
            Some(gold_band::app::WorkflowTemplateCommandError::DuplicateName { .. })
        ),
        "{err}"
    );
    assert_eq!(
        app.workflow_templates()
            .unwrap()
            .templates
            .iter()
            .filter(|t| t.name == exchange.name)
            .count(),
        1
    );
}

#[test]
fn terminal_success_cycle_cannot_be_silently_replaced_with_end() {
    use gold_band::dsl::{END_NODE, EdgeDsl, EdgeOutcome};
    let (app, _temp) = fixture();
    let task = task(&app);
    let mut spec = spec();
    let mut next = spec.workflows[0].stages[0].clone();
    next.id = Some("s2".into());
    spec.workflows[0].stages.push(next);
    let saved = expect_saved(app.forge_compile(&spec, Some(&task), 1));
    let id = &saved.template_ids[0];
    let mut wf = template_of(&app, id).workflow;
    wf.edges.retain(|e| e.from != "s2");
    for (on, to) in [
        (EdgeOutcome::Success, wf.entry.clone()),
        (EdgeOutcome::Failure, END_NODE.into()),
    ] {
        wf.edges.push(EdgeDsl {
            from: "s2".into(),
            on,
            to,
            session: None,
            new_round_entry: None,
        });
    }
    app.update_workflow_template(id, wf).unwrap();
    let original = serde_json::to_value(template_of(&app, id)).unwrap();
    assert!(app.forge_from_workflow(id).is_err());
    assert!(
        app.forge_compile(saved.spec.as_ref().unwrap(), Some(&task), 1)
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(template_of(&app, id)).unwrap(),
        original
    );
}
