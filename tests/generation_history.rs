//! Cross-domain review regressions against the production App, no model or desktop shell.
use camino::Utf8PathBuf;
use gold_band::app::{
    App, CreateConversationTaskCommand, WorkflowTemplate,
    forge_service::{ForgeCompileOutcome, ForgeCompileSaved},
};
use gold_band::config::{ConversationDirectConfig, ConversationRunMode, RuntimeConfig};
use gold_band::forge::ForgeSpec;
use gold_band::storage::{GoldBandPaths, StoragePathConfig};
use serde_json::json;
use tempfile::TempDir;

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

#[test]
fn registered_generation_page_filters_before_global_keyset_and_caps_candidates() {
    use gold_band::{runtime::TaskOrigin, storage::sqlite::SearchIndex};
    use std::collections::BTreeMap;
    let temp = tempfile::tempdir().unwrap();
    let base = Utf8PathBuf::from_path_buf(temp.path().into()).unwrap();
    let file = base.join("fixture.sqlite");
    let index = SearchIndex::open(&file).unwrap();
    let roots = BTreeMap::from([
        ("a".to_owned(), base.join("registered_a").to_string()),
        ("b".to_owned(), base.join("registered_b").to_string()),
    ]);
    let db = rusqlite::Connection::open(&file).unwrap();
    let add = |root: &str, id: &str, origin: &str, title: &str| {
        db.execute("INSERT INTO tasks(task_path,task_id,task_uuid,title,description,requirement_text,origin,updated_at) VALUES(?1,?2,?3,?4,'索引','body only',?5,'2026-01-01T00:00:00Z')", rusqlite::params![Utf8PathBuf::from(root).join(id).as_str(),id,format!("{root}-{id}"),title,origin]).unwrap();
    };
    for n in 1..=110 {
        add(&roots["a"], &format!("task-{n:03}"), "forge", "old");
    }
    for root in roots.values() {
        add(root, "task-120", "forge-trial", "Review");
    }
    for root in [
        base.join("outside"),
        Utf8PathBuf::from(&roots["a"]).join("nested"),
    ] {
        add(root.as_str(), "task-999", "forge-trial", "Review");
    }
    add(&roots["a"], "task-998", "", "Review");
    let filters = Some(TaskOrigin::ForgeTrial);
    for q in ["Review", "Re 索"] {
        let (rows, next) = index
            .registered_generation_page(&roots, filters, q, None, 1)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key.project_id, "b");
        assert_eq!(rows[0].key.task_id, "task-120");
        let (rows, end) = index
            .registered_generation_page(&roots, filters, q, Some(&next.unwrap()), 1)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key.project_id, "a");
        assert_eq!(rows[0].key.task_id, "task-120");
        assert!(end.is_none());
    }
    assert!(
        index
            .registered_generation_page(&roots, None, "body only", None, 100)
            .unwrap()
            .0
            .is_empty()
    );
    assert_eq!(
        index
            .registered_generation_page(&roots, None, "", None, usize::MAX)
            .unwrap()
            .0
            .len(),
        100
    );
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
#[test]
fn generation_history_explicit_rename_retires_auto_title() {
    let (app, _temp) = fixture();
    let id = app
        .prepare_conversation_task(&direct_command())
        .unwrap()
        .accept()
        .0;
    app.update_task_metadata(
        &id,
        app.task_show(&id).unwrap().uuid.as_deref().unwrap(),
        "Hand chosen",
        None,
    )
    .unwrap();
    let metadata: gold_band::app::ConversationMetadata =
        gold_band::storage::read_json(&app.paths.task_dir(&id).join("authoring/conversation.json"))
            .unwrap();
    assert!(
        !metadata.title_auto_generated,
        "explicit rename kept automatic title ownership"
    );
}

#[test]
fn generation_history_titles_survive_reopen_and_match_public_search() {
    // The production search handle lives for the process. Its owner removes the
    // disposable database only after the child exits and Windows releases it.
    const DB_ROOT: &str = "GOLD_BAND_GENERATION_HISTORY_TEST_ROOT";
    let Ok(root) = std::env::var(DB_ROOT) else {
        let sandbox = tempfile::tempdir().unwrap();
        let child = gold_band::process::background_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "generation_history_titles_survive_reopen_and_match_public_search",
                "--nocapture",
            ])
            .env(DB_ROOT, sandbox.path())
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
        return;
    };
    let db_path = Utf8PathBuf::from(root).join("history.sqlite");
    assert!(!db_path.exists());
    let (app, _temp) = fixture();
    let index =
        gold_band::storage::sqlite::init_search_index(&db_path, &app.paths.repo_root.join("empty"))
            .unwrap();
    let saved = expect_saved(app.forge_compile(&spec(), None, 1));
    let trial = saved.trial.unwrap();
    let template = saved.template_ids[0].clone();
    let (id, mut draft) = source_task(&app, &template);
    let other = fixture();
    let other_id = other
        .0
        .prepare_conversation_task(&direct_command())
        .unwrap()
        .accept()
        .0;
    assert_eq!(other_id, id);
    let search = |app: &App, query: &str| {
        index
            .search_tasks_in_task_roots(query, &[app.paths.tasks_dir().to_string()], 1)
            .unwrap()
    };
    draft.name = Some("Autumn rehearsal".into());
    let mut draft = app.save_forge_spec(&id, &draft).unwrap();
    assert_eq!(
        search(&app, "Autumn rehearsal")
            .first()
            .map(|r| r.title.as_str()),
        Some("Autumn rehearsal")
    );
    app.update_task_metadata(
        &id,
        app.task_show(&id).unwrap().uuid.as_deref().unwrap(),
        "Pinned review title",
        None,
    )
    .unwrap();
    draft.name = Some("New draft name".into());
    app.save_forge_spec(&id, &draft).unwrap();
    let restored = App::with_config_and_path_config(
        app.paths.repo_root.clone(),
        RuntimeConfig::default(),
        StoragePathConfig {
            app_key: "forge-review",
            config_dir_name: ".gold-band",
            home_env_var: "GOLD_BAND_FORGE_REVIEW_FIXTURE_HOME",
        },
    );
    assert_eq!(
        search(&restored, "Pinned review title")
            .first()
            .map(|r| r.title.as_str()),
        Some("Pinned review title")
    );
    assert!(search(&restored, "New draft name").is_empty());
    assert!(search(&other.0, "Pinned review title").is_empty());
    assert_eq!(template_of(&restored, &template).name, "First");
    let task_uuid = restored.task_show(&id).unwrap().uuid.unwrap();
    assert!(
        restored
            .update_task_metadata(&id, "replaced-instance", "Not this task", None)
            .is_err()
    );
    assert_eq!(
        restored.task_show(&id).unwrap().uuid.as_deref(),
        Some(task_uuid.as_str())
    );
    assert_eq!(
        restored.task_show(&id).unwrap().title.as_deref(),
        Some("Pinned review title")
    );
    let trial = restored
        .forge_trial(&trial.template_id, &trial.revision, "Pinned review fixture")
        .unwrap();
    for _ in 0..9 {
        restored
            .prepare_conversation_task(&direct_command())
            .unwrap()
            .accept();
    }
    let roots = std::collections::BTreeMap::from([(
        restored.paths.project_id.clone(),
        restored.paths.tasks_dir().to_string(),
    )]);
    let (first, next) = index
        .registered_generation_page(&roots, None, "", None, 1)
        .unwrap();
    let (second, done) = index
        .registered_generation_page(&roots, None, "", Some(&next.unwrap()), 1)
        .unwrap();
    assert!(done.is_none());
    let mut ids = vec![first[0].key.task_id.clone(), second[0].key.task_id.clone()];
    ids.sort();
    assert_eq!(ids, vec![id.clone(), trial.task.id.clone()]);
    let reopened_index = gold_band::storage::sqlite::SearchIndex::open(&db_path).unwrap();
    let hits = reopened_index
        .search_generation_tasks_in_task_root("Pinned review title", &restored.paths.tasks_dir(), 1)
        .unwrap();
    assert_eq!(hits[0].task_id, id);
    assert_eq!(hits[0].origin, "forge");
    assert_eq!(hits[0].task_uuid.as_deref(), Some(task_uuid.as_str()));
    // A legacy unverified hit would otherwise outrank the trial at limit=1.
    // Browsing can resolve the canonical task, but text search must not reuse legacy matches.
    let db = rusqlite::Connection::open(db_path.as_std_path()).unwrap();
    db.execute(
        "UPDATE tasks SET task_uuid=NULL WHERE task_path=?1",
        [restored.paths.task_dir(&id).as_str()],
    )
    .unwrap();
    let verified = reopened_index
        .search_conversation_task_instances(
            "Pinned review",
            &[restored.paths.tasks_dir().to_string()],
            true,
            1,
        )
        .unwrap();
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0].task_id, trial.task.id);
    assert!(
        reopened_index
            .registered_generation_page(&roots, None, "", None, 7)
            .unwrap()
            .0
            .iter()
            .any(|row| row.key.task_id == id)
    );
}
