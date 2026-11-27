//! Drive the registered Tauri handlers, including serde argument decoding. The child owns
//! a fresh application home, search-index singleton and trash directory. Never use OS trash.
use super::*;
use crate::{commands, commands_forge};
use serde_json::{Value, json};
use std::sync::OnceLock;
use tauri::test::{MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};

static TRASH: OnceLock<(std::path::PathBuf, std::path::PathBuf)> = OnceLock::new();

fn fixture_trash(path: &Path) -> anyhow::Result<()> {
    let (tasks, destination) = TRASH.get().expect("fixture destination required");
    anyhow::ensure!(path.canonicalize()?.parent() == Some(tasks.canonicalize()?.as_path()));
    Ok(fs::rename(
        path,
        destination.join(path.file_name().unwrap()),
    )?)
}

fn window(ctx: DesktopContext) -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .manage(DesktopState::new(ctx))
        .manage(ConversationTaskTrash(fixture_trash))
        .invoke_handler(tauri::generate_handler![
            commands::create_task,
            commands_forge::forge_create_session,
            commands_forge::forge_start_trial,
            commands_forge::forge_get_spec,
            commands_forge::forge_save_spec,
            commands_forge::forge_compile,
            update_task_metadata,
            get_conversation_task_summary,
            delete_conversation_task,
            pin_conversation,
            get_conversation_task_page,
            get_generation_history_page,
            get_conversation_pinned_task_page,
            get_conversation_recent_task_page,
            search_conversation_tasks,
        ])
        .build(mock_context(noop_assets()))
        .unwrap();
    let view = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    (app, view)
}

fn ipc(
    view: &tauri::WebviewWindow<MockRuntime>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    get_ipc_response(
        view,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    )
    .map(|body| body.deserialize().unwrap())
}

#[test]
fn public_task_management_lifecycle() {
    // Desktop and SQLite resolve process-wide paths. Isolate those singletons and HOME from
    // all other tests instead of flipping process environment in a parallel suite.
    if std::env::var_os("GOLD_BAND_MANAGEMENT_TEST_CHILD").is_none() {
        let sandbox = tempfile::tempdir().unwrap();
        let home = sandbox.path().join("home");
        fs::create_dir(&home).unwrap();
        let output = gold_band::process::background_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands_conversation::management_tests::public_task_management_lifecycle",
                "--nocapture",
            ])
            .env("GOLD_BAND_MANAGEMENT_TEST_CHILD", "1")
            .env("GOLD_BAND_HOME", &home)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", &home)
            .env("LOCALAPPDATA", &home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let sandbox = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::from_path_buf(sandbox.path().into()).unwrap();
    let ctx = |name: &str| DesktopContext {
        repo_root: root.join(name),
        config: Default::default(),
        recent_workspaces: vec![],
        needs_workspace: false,
    };
    let owner = ctx("owner");
    let peer = ctx("peer");
    let app = owner.app();
    let home = Utf8PathBuf::from(std::env::var("GOLD_BAND_HOME").unwrap());
    assert!(app.paths.user_gold_band_root.starts_with(&home));
    let peer_app = peer.app();
    for a in [&app, &peer_app] {
        fs::create_dir_all(&a.paths.repo_root).unwrap();
        a.paths.provision_project_manifest().unwrap();
    }
    let trash = root.join("trash");
    fs::create_dir(&trash).unwrap();
    TRASH
        .set((
            app.paths.tasks_dir().into_std_path_buf(),
            trash.as_std_path().into(),
        ))
        .unwrap();
    gold_band::storage::sqlite::init_search_index(&root.join("search.db"), &root.join("empty"))
        .unwrap();
    app.with_state(|s| {
        s.conversation_workspaces = [&app, &peer_app]
            .into_iter()
            .map(|a| ConversationWorkspaceEntry {
                project_id: a.paths.project_id.clone(),
                workspace_path: a.paths.repo_root.to_string(),
                name: a.paths.project_id.clone(),
                added_at: "2026-01-01T00:00:00Z".into(),
            })
            .collect();
        (true, ())
    })
    .unwrap();
    let pid = &app.paths.project_id;
    let (_host, view) = window(owner.clone());
    let (_other_host, other_view) = window(peer);
    let templates = app.workflow_templates().unwrap();
    let workflow = &templates.templates[0].workflow;
    let ordinary_input = json!({"input":{"title":"Ordinary", "requirementContent":"Synthetic requirement", "workflow":workflow}});
    // The actual Direct application wrapper persists conversation.json and the task as
    // one prepared creation. Accept the task without ever launching an external Agent.
    let input = serde_json::from_value(
        json!({"projectId":pid,"content":"Ordinary", "runMode":"direct",
        "directConfig":{"agentType":"claude-acp","autoAccept":false}}),
    )
    .unwrap();
    let (ordinary, _, _) =
        crate::view_models_conversation::create_conversation_task_vm(&app, &input).unwrap();
    let other =
        ipc(&other_view, "create_task", ordinary_input.clone()).unwrap()["task"]["id"].clone();
    assert_eq!(other, ordinary);
    let uuid = app.task_show(&ordinary).unwrap().uuid.unwrap();
    let locator = |id: &str, uuid: &str| json!({"projectId":pid,"taskId":id,"taskUuid":uuid});
    // A reused numeric task ID never substitutes for the confirmed instance.
    let stale = ipc(
        &view,
        "delete_conversation_task",
        locator(&ordinary, "old-generation"),
    );
    assert_eq!(stale.unwrap_err()["code"], "conversation.task-not-found");
    let bad_run = app.paths.run_file(&ordinary, "run-001");
    fs::create_dir_all(bad_run.parent().unwrap()).unwrap();
    for contents in [None, Some("invalid-json")] {
        if let Some(contents) = contents {
            fs::write(&bad_run, contents).unwrap();
        }
        assert!(
            ipc(&view, "delete_conversation_task", locator(&ordinary, &uuid)).is_err(),
            "incomplete or unreadable run must block deletion"
        );
        assert!(app.paths.task_file(&ordinary).exists());
    }
    fs::remove_dir_all(bad_run.parent().unwrap()).unwrap();
    assert_eq!(fs::read_dir(&trash).unwrap().count(), 0);
    let forge = ipc(&view,"forge_create_session", json!({"input":{"projectId":pid,"agentType":"claude-acp","contextContent":"Synthetic draft"}})).unwrap()["taskId"].as_str().unwrap().to_owned();
    let draft = json!({"specVersion":"0.1","name":"Fixture workflow","intent":"Synthetic intent","sourceSkills":[],"openQuestions":[],"workflows":[{"name":"Fixture workflow","stages":[{"id":"check","role":"dev","goal":"<输入>样例\n<产出>文本\n<边界>本地\n<验收>输出样例","artifact":"result","outputKind":"json","schemaHint":{"ok":"boolean"},"successCondition":"$.ok == true","onFailure":"retry","manualCheck":false}]}]});
    let forge_uuid = app.task_show(&forge).unwrap().uuid.unwrap();
    let peer_forge = ipc(&other_view, "forge_create_session", json!({"input":{"projectId":peer_app.paths.project_id,"agentType":"claude-acp","contextContent":"Peer synthetic draft"}})).unwrap();
    assert_eq!(peer_forge["taskId"], forge);
    for a in [&app, &peer_app] {
        gold_band::storage::sqlite::index_task_activity_with_retry(
            &a.paths.task_dir(&forge),
            &forge,
            "2099-01-01T00:00:00Z",
        );
    }
    let first = ipc(&view, "get_generation_history_page", json!({"limit":1})).unwrap();
    assert_eq!(first["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(
        ipc(
            &view,
            "get_generation_history_page",
            json!({"cursor":"bogus"})
        )
        .unwrap_err()["code"],
        "conversation.invalid-cursor"
    );
    assert_eq!(
        ipc(
            &view,
            "get_generation_history_page",
            json!({"cursor":first["nextCursor"], "projectId":pid})
        )
        .unwrap_err()["code"],
        "conversation.invalid-cursor"
    );
    let second = ipc(
        &view,
        "get_generation_history_page",
        json!({"limit":1, "cursor":first["nextCursor"].as_str().unwrap()}),
    )
    .unwrap();
    assert!(second["nextCursor"].is_null());
    let mut projects = vec![pid.as_str(), peer_app.paths.project_id.as_str()];
    projects.sort_unstable_by(|a, b| b.cmp(a));
    for (page, project) in [first, second].iter().zip(projects) {
        assert_eq!(page["tasks"].as_array().unwrap().len(), 1);
        let row = &page["tasks"][0];
        assert_eq!(row["projectId"], project);
        assert_eq!(row["taskId"], forge);
        assert_eq!(row["origin"], "forge");
        assert!(row["taskUuid"].is_string());
        assert!(row["latestRun"].is_null());
    }
    let peer_id = &peer_app.paths.project_id;
    let peer_uuid = peer_app.task_show(&forge).unwrap().uuid.unwrap();
    ipc(
        &view,
        "update_task_metadata",
        json!({"projectId":peer_id,"taskId":forge,"taskUuid":peer_uuid,"title":"Legacy fixture"}),
    )
    .unwrap();
    let db = rusqlite::Connection::open(root.join("search.db")).unwrap();
    assert_eq!(
        db.execute(
            "UPDATE tasks SET task_uuid=NULL WHERE task_path=?1",
            [peer_app.paths.task_dir(&forge).as_str()]
        )
        .unwrap(),
        1
    );
    let legacy = ipc(
        &view,
        "get_generation_history_page",
        json!({"projectId":peer_id}),
    )
    .unwrap();
    assert_eq!(legacy["tasks"][0]["taskUuid"], peer_uuid);
    let search = ipc(
        &view,
        "get_generation_history_page",
        json!({"projectId":peer_id,"query":"Legacy fixture"}),
    )
    .unwrap();
    assert!(search["tasks"].as_array().unwrap().is_empty());
    drop(db);
    let saved = ipc(
        &view,
        "forge_save_spec",
        json!({"projectId":pid,"taskId":forge,"taskUuid":forge_uuid,"spec":draft}),
    )
    .unwrap();
    let compiled = ipc(
        &view,
        "forge_compile",
        json!({"projectId":pid,"spec":saved,"taskId":forge,"repairAttempt":1}),
    )
    .unwrap();
    assert_eq!(compiled["status"], "saved", "{compiled}");
    let review = &compiled["trial"];
    let trial = ipc(&view,"forge_start_trial",json!({"projectId":pid,"templateId":review["templateId"],"revision":review["revision"],"intent":"Synthetic trial"})).unwrap()["taskId"].as_str().unwrap().to_owned();
    let trial_uuid = app.task_show(&trial).unwrap().uuid.unwrap();
    // Preparing creates a durable running run but never starts a provider. Dropping the
    // unaccepted run rolls it back, leaving a manageable no-run trial conversation.
    {
        assert!(
            app.prepare_run(&trial, None).is_err(),
            "no provider is available in this isolated home"
        );
        assert!(app.run_list(&trial).unwrap().is_empty());
        let ids: Vec<_> = app
            .config
            .agents
            .keys()
            .map(|id| id.as_str().to_owned())
            .collect();
        let prepared_app = owner
            .app()
            .with_provider_diagnostics_source(std::sync::Arc::new(move || {
                Ok(ids
                    .iter()
                    .cloned()
                    .map(|id| {
                        (
                            id,
                            gold_band::config::ProviderDiagnosticSnapshot {
                                config_fingerprint: None,
                                available: true,
                                error: None,
                                checked_at: "2026-01-01T00:00:00Z".into(),
                                capabilities: None,
                            },
                        )
                    })
                    .collect())
            }));
        let _prepared = prepared_app.prepare_run(&trial, None).unwrap();
        let err = ipc(
            &view,
            "delete_conversation_task",
            locator(&trial, &trial_uuid),
        )
        .unwrap_err();
        assert_eq!(err["code"], "conversation.task-running");
    }
    let unknown = ipc(
        &view,
        "get_generation_history_page",
        json!({"query":"Matrix", "projectId":"not-registered"}),
    )
    .unwrap_err();
    assert_eq!(unknown["code"], "workspace.not-found");
    let before_templates =
        serde_json::to_value(app.workflow_templates().unwrap().templates).unwrap();
    let (_reopened_host, reopened) = window(owner);
    for (id, origin, title) in [
        (&ordinary, Value::Null, "MatrixOrdinary"),
        (&forge, json!("forge"), "MatrixForge"),
        (&trial, json!("forge-trial"), "MatrixTrial"),
    ] {
        let uuid = app.task_show(id).unwrap().uuid.unwrap();
        let mut address = locator(id, &uuid);
        address["title"] = json!(title);
        ipc(&view, "update_task_metadata", address.clone()).unwrap();
        if id == &forge {
            let mut draft = ipc(&reopened, "forge_get_spec", address.clone()).unwrap();
            draft["name"] = json!("Changed draft not manual title");
            ipc(
                &reopened,
                "forge_save_spec",
                json!({"projectId":pid,"taskId":id,"taskUuid":uuid,"spec":draft}),
            )
            .unwrap();
        }
        let row = ipc(&reopened, "get_conversation_task_summary", address.clone()).unwrap();
        assert_eq!(row["title"], title);
        assert_eq!(row["origin"], origin);
        ipc(&reopened, "pin_conversation", address.clone()).unwrap();
        let check_lists = |present: bool| {
            for command in [
                "get_conversation_task_page",
                "get_conversation_recent_task_page",
                "get_conversation_pinned_task_page",
                "search_conversation_tasks",
            ] {
                let response = ipc(
                    &reopened,
                    command,
                    json!({"projectId":pid,"query":title,"limit":20}),
                )
                .unwrap();
                let rows = response
                    .as_array()
                    .or_else(|| response["tasks"].as_array())
                    .unwrap();
                assert_eq!(
                    rows.iter().any(|r| r["projectId"] == *pid
                        && r["taskId"] == *id
                        && r["taskUuid"] == uuid),
                    present,
                    "{command} {id}: {response}"
                );
            }
        };
        check_lists(true);
        if id == &forge {
            // Simulate a replaced task body while SQLite still describes its predecessor.
            let path = app.paths.task_file(id);
            let original = fs::read(&path).unwrap();
            for change in ["uuid", "title", "description", "id"] {
                let mut stale: Value = serde_json::from_slice(&original).unwrap();
                stale[change] = json!("changed-since-indexing");
                gold_band::storage::write_json(&path, &stale).unwrap();
                for scope in [Value::Null, json!(pid)] {
                    let page = ipc(
                        &reopened,
                        "get_generation_history_page",
                        json!({"query":title,"projectId":scope}),
                    )
                    .unwrap();
                    assert!(
                        page["tasks"].as_array().unwrap().is_empty(),
                        "stale {change} was actionable"
                    );
                    assert_eq!(page["errors"][0]["params"]["taskId"], *id);
                }
            }
            fs::write(path, original).unwrap();
        }
        let filtered = ipc(
            &reopened,
            "get_generation_history_page",
            json!({"projectId":pid,"limit":20}),
        )
        .unwrap();
        assert_eq!(
            filtered["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["taskUuid"] == uuid),
            !origin.is_null()
        );
        let scoped = ipc(
            &reopened,
            "get_generation_history_page",
            json!({"query":title,"projectId":pid,"origin":origin}),
        )
        .unwrap();
        let match_row = scoped["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["taskUuid"] == uuid);
        if origin.is_null() {
            assert!(match_row.is_none());
        } else {
            assert_eq!(match_row.unwrap()["origin"], origin);
        }
        ipc(&reopened, "delete_conversation_task", address.clone()).unwrap();
        check_lists(false);
        assert!(ipc(&reopened, "delete_conversation_task", address).is_err());
        assert!(trash.join(id).join("task.json").is_file());
        assert!(peer_app.task_show(&ordinary).is_ok());
        assert_eq!(
            serde_json::to_value(app.workflow_templates().unwrap().templates).unwrap(),
            before_templates
        );
    }
    let replacement = ipc(&reopened, "create_task", ordinary_input).unwrap();
    assert_eq!(replacement["task"]["id"], ordinary);
    let replacement_uuid = app.task_show(&ordinary).unwrap().uuid.unwrap();
    assert_ne!(replacement_uuid, uuid);
    let stale = ipc(
        &reopened,
        "delete_conversation_task",
        locator(&ordinary, &uuid),
    )
    .unwrap_err();
    assert_eq!(stale["code"], "conversation.task-not-found");
    assert_eq!(
        app.task_show(&ordinary).unwrap().uuid,
        Some(replacement_uuid)
    );
    let delayed = json!({"projectId":pid,"taskId":forge,"taskUuid":forge_uuid,"spec":draft});
    assert!(ipc(&reopened, "forge_save_spec", delayed.clone()).is_err());
    assert!(!app.paths.task_dir(&forge).exists());
    let next_forge = ipc(&reopened,"forge_create_session",json!({"input":{"projectId":pid,"agentType":"claude-acp","contextContent":"Replacement generation"}})).unwrap();
    assert_eq!(next_forge["taskId"], forge);
    assert!(ipc(&reopened, "forge_save_spec", delayed).is_err());
    assert!(
        ipc(
            &reopened,
            "forge_get_spec",
            json!({"projectId":pid,"taskId":forge})
        )
        .unwrap()
        .is_null()
    );
}
