//! Material selection through final provider prompt, Forge settlement, library and Git workers.
//! Only model I/O is synthetic. The author never receives a fixture path or expected criteria.
use super::{binding, direct, edit_ui, fixture_git, group, node, reopen, saved, stage, template};
use crate::{BranchProbe, RecordingProvider, app_with_provider, success_result};
use camino::Utf8Path;
use gold_band::{
    app::{
        AcpTurnOutcome, ConversationRunLaunch, CreateConversationTaskCommand, ProfileInput,
        forge_service::{ForgeSourceItem, WORKFLOW_FORGE_PROFILE_ID},
    },
    config::{ConversationRunMode, SkillSource},
    domain::RunOutcome,
    dsl::{END_NODE, EdgeOutcome},
    provider::{
        DoctorResult, ProviderAdapter, ProviderInfo, ProviderRunResult, SessionRef, UserPromptRole,
        WorkerInvocation, render_prompt_bundle,
    },
    skill::{
        SkillManager,
        claude_plugin::{
            ClaudePluginAssetKind, PluginProvenance, select_workbuddy_directory, workbuddy_source,
        },
        forge_skill_asset_id,
    },
    storage::{GoldBandPaths, read_json},
    workflow_model_binding::TaskAuthoringWorkflow,
};
use serde_json::json;
use std::{
    fs,
    sync::{Arc, Mutex},
};

const NAME: &str = "Synthetic mixed-source release checks";
const COPY: &str = "Synthetic mixed-source copy";
const REQUEST: &str =
    "Use the three selected sources to author two concurrent checks and a final report.";
const ROLE: &str = "HTTP 412 must include a precondition receipt";
const SKILL: &str = "Keyboard navigation must restore the previous focus target";
const PLUGIN: &str = "The report must cross-reference both signed receipts";
const OLD: &str = "Obsolete body must not survive digest";
const DECOY: &str = "Unselected command must never run";
const SPOOF: &str = "Forged caller-supplied author instructions";
const EDIT: &str = "Also record the focus target before navigation";
const WORKERS: [(&str, &str); 3] = [
    ("api-check", "pf-builtin-review"),
    ("ui-check", "pf-builtin-test"),
    ("report", "pf-builtin-accept"),
];

fn write(path: &Utf8Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}
fn document(label: &str, target: &str, criterion: &str) -> String {
    format!(
        "---\nname: release-evidence\ndescription: {label}\n---\nCriterion for {target}: {criterion}\n"
    )
}
fn profile(body: &str) -> ProfileInput {
    ProfileInput {
        name: "Source-only release reviewer".into(),
        summary: "Role handbook".into(),
        content: format!("Criterion for api-check: {body}"),
        dynamic_template: false,
    }
}
fn item(catalog: &str, source: SkillSource, asset_id: &str) -> ForgeSourceItem {
    serde_json::from_value(json!({"catalog":catalog,"source":source,"assetId":asset_id})).unwrap()
}

#[derive(Clone, Default)]
struct Author(Arc<Mutex<Vec<(WorkerInvocation, String)>>>);
impl ProviderAdapter for Author {
    fn describe_provider(&self) -> ProviderInfo {
        RecordingProvider::default().describe_provider()
    }
    fn doctor(&self) -> DoctorResult {
        RecordingProvider::default().doctor()
    }
    fn open_session(&self, _: &SessionRef) -> anyhow::Result<()> {
        Ok(())
    }
    fn build_continue_command(&self, _: &SessionRef) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    fn run_worker(&self, req: WorkerInvocation) -> anyhow::Result<ProviderRunResult> {
        let prompt = render_prompt_bundle(&req)?;
        anyhow::ensure!(
            prompt.role.as_ref().map(|r| r.profile_id.as_str()) == Some(WORKFLOW_FORGE_PROFILE_ID),
            "source must not become author"
        );
        let mut stages = Vec::new();
        for line in prompt
            .user_prompt
            .lines()
            .filter_map(|line| line.trim().strip_prefix("Criterion for "))
        {
            let (id, acceptance) = line
                .split_once(": ")
                .ok_or_else(|| anyhow::anyhow!("bad criterion: {line}"))?;
            let role = match id {
                "api-check" => "review",
                "ui-check" => "test",
                "report" => "accept",
                _ => anyhow::bail!("unexpected stage {id}"),
            };
            stages.push(stage(id, role, acceptance));
        }
        anyhow::ensure!(
            stages.len() == 3,
            "expected three actual body criteria, found {}",
            stages.len()
        );
        let delta = json!({"name":NAME,"intent":REQUEST,"openQuestions":[],"stages":stages,"parallelGroups":[group(2)]});
        let pid = req.resume_prompt_id.as_deref().unwrap_or("test-prompt");
        let reply = format!("```json\n{delta}\n```");
        let rows = [
            json!({"id": "request", "seq": 1, "kind": "userTextDelta", "content": prompt.user_prompt,
                "raw": {"source": "goldBandPrompt", "promptId": pid}}),
            json!({"id": "reply", "seq": 2, "kind": "textDelta", "content": reply}),
        ].map(|mut row| {
            row["timestamp"] = json!("2026-01-01T00:00:00Z");
            json!({"item": row}).to_string()
        });
        write(
            &req.attempt_dir.join("acp.timeline.jsonl"),
            &(rows.join("\n") + "\n"),
        );
        self.0.lock().unwrap().push((req, prompt.user_prompt));
        Ok(success_result())
    }
}

/// BranchProbe proves overlapping admission; record returns as well, so a report started
/// while either provider was still executing cannot pass merely because it started third.
#[derive(Clone, Default)]
struct JoinedWorkers {
    probe: BranchProbe,
    finished: Arc<Mutex<Vec<String>>>,
}
impl ProviderAdapter for JoinedWorkers {
    fn describe_provider(&self) -> ProviderInfo {
        self.probe.describe_provider()
    }
    fn doctor(&self) -> DoctorResult {
        self.probe.doctor()
    }
    fn open_session(&self, _: &SessionRef) -> anyhow::Result<()> {
        Ok(())
    }
    fn build_continue_command(&self, _: &SessionRef) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    fn run_worker(&self, req: WorkerInvocation) -> anyhow::Result<ProviderRunResult> {
        let id = req.runtime_context.node_id.clone();
        let expected_role = WORKERS.iter().find(|(key, _)| *key == id).unwrap().1;
        assert_eq!(req.profile.as_deref(), Some(expected_role));
        let required = match id.as_str() {
            "api-check" => &[ROLE][..],
            "ui-check" => &[SKILL, EDIT][..],
            "report" => &[PLUGIN][..],
            _ => unreachable!(),
        };
        let actual_goal = req.task_instruction.as_deref().unwrap();
        assert!(
            required
                .iter()
                .all(|criterion| actual_goal.contains(criterion)),
            "{id} lost source criteria: {actual_goal}"
        );
        if id == "report" {
            let mut prior = self.finished.lock().unwrap().clone();
            prior.sort();
            assert_eq!(
                prior,
                ["api-check", "ui-check"],
                "report before both provider returns"
            );
        } else {
            assert!(
                req.workspace_dir.join(".git").is_file(),
                "expected a linked Git worktree"
            );
            assert_eq!(
                fixture_git(&req.workspace_dir, &["rev-parse", "--is-inside-work-tree"]).trim(),
                "true"
            );
            if id == "api-check" {
                assert_eq!(req.model.as_deref(), Some("gpt-5.4"));
            }
        }
        let result = self.probe.run_worker(req)?;
        self.finished.lock().unwrap().push(id);
        Ok(result)
    }
}

#[test]
fn mixed_material_public_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let repo = root.join("project");
    fs::create_dir(&repo).unwrap();
    let paths = GoldBandPaths::new(repo.clone());
    let home = paths
        .user_gold_band_root
        .strip_prefix(&repo)
        .expect("refusing non-fixture user storage");
    fixture_git(&repo, &["init", "-q"]);
    let hooks = root.join("empty-hooks");
    fs::create_dir(&hooks).unwrap();
    fixture_git(
        &repo,
        &["config", "--local", "core.hooksPath", hooks.as_str()],
    );
    gold_band::git::GitRepositoryService::default()
        .ensure_local_exclude(&repo, &format!("/{}/", home.as_str().replace('\\', "/")))
        .unwrap();
    let author = Author::default();
    let mut app = app_with_provider(repo.clone(), Box::new(author.clone()));
    app.config
        .agents
        .retain(|id, _| id.as_str() == "claude-acp");
    let user_role = app.create_profile(profile(OLD)).unwrap();
    let skill_doc = repo.join(".claude/skills/release-evidence/SKILL.md");
    write(
        &skill_doc,
        &document("Navigation standard", "ui-check", OLD),
    );
    let inventory = SkillManager::new(app.paths.clone(), app.config.agents.clone())
        .list_by_workspace(repo.as_str())
        .unwrap();
    let skill = inventory
        .iter()
        .find(|s| s.name == "release-evidence" && s.source == SkillSource::Project)
        .unwrap();
    let package = root.join("manual-workbuddy");
    write(
        &package.join(".codebuddy-plugin/plugin.json"),
        r#"{"name":"release-checks","version":"1.0.0"}"#,
    );
    write(
        &package.join("commands/report.md"),
        &document("Selected report standard", "report", PLUGIN),
    );
    write(
        &package.join("commands/decoy.md"),
        &document("Unselected standard", "report", DECOY),
    );
    select_workbuddy_directory(&app.paths, SkillSource::Project, &package).unwrap();
    let plugin = workbuddy_source(&app.paths, SkillSource::Project).unwrap();
    assert_eq!(plugin.provider_id, "workbuddy");
    assert_eq!(plugin.provenance, PluginProvenance::ManualDirectory);
    let asset = plugin
        .assets
        .iter()
        .find(|a| a.description == "Selected report standard")
        .unwrap();
    let decoy = plugin
        .assets
        .iter()
        .find(|a| a.description == "Unselected standard")
        .unwrap();
    assert_eq!(asset.kind, ClaudePluginAssetKind::Command);
    assert_eq!(asset.name, decoy.name);
    assert_ne!(asset.asset_id, decoy.asset_id);
    let items = vec![
        item("profile", SkillSource::Global, &user_role.id),
        item(
            "skill",
            skill.source,
            &forge_skill_asset_id(Utf8Path::new(&skill.directory_path)),
        ),
        item("plugin", plugin.source, &asset.asset_id),
    ];
    let digest = app.forge_items_digest(&items).unwrap();
    let kinds: Vec<_> = digest.iter().map(|s| s.kind.tag()).collect();
    assert_eq!(kinds, ["role", "skill", "command"]);
    assert_eq!(digest[2].name, asset.name);
    for forbidden in [OLD, ROLE, SKILL, PLUGIN, DECOY] {
        assert!(!json!(digest).to_string().contains(forbidden));
    }
    let selected_role = app.update_profile(&user_role.id, profile(ROLE)).unwrap();
    write(
        &skill_doc,
        &document("Navigation standard", "ui-check", SKILL),
    );
    fixture_git(
        &repo,
        &[
            "add",
            "-f",
            "--",
            ".claude/skills/release-evidence/SKILL.md",
        ],
    );
    fixture_git(
        &repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "-qm",
            "Synthetic project skill",
        ],
    );

    let mut command = direct(REQUEST);
    command.forge_source_items = Some(items);
    command.role = Some(UserPromptRole {
        profile_id: user_role.id.clone(),
        name: user_role.name.clone(),
        content: SPOOF.into(),
    });
    let created = app
        .create_conversation_run(&command, ConversationRunLaunch::Foreground)
        .unwrap();
    let log = author.0.lock().unwrap();
    assert_eq!(log.len(), 1, "author run: {:?}", created.run);
    let (req, prompt) = &log[0];
    for expected in [ROLE, SKILL, PLUGIN] {
        assert_eq!(prompt.matches(expected).count(), 1, "{expected}");
    }
    for forbidden in [OLD, SPOOF, DECOY] {
        assert!(!prompt.contains(forbidden), "{forbidden}");
    }
    let ctx = &req.runtime_context;
    let settlement = app
        .forge_settle_turn(
            &created.task_id,
            &ctx.run_id,
            &ctx.round_id,
            &ctx.node_id,
            &ctx.attempt_id,
            Some(req.resume_prompt_id.as_deref().unwrap_or("test-prompt")),
            AcpTurnOutcome::Completed,
        )
        .unwrap()
        .unwrap();
    assert!(
        settlement.spec_updated && settlement.error.is_none(),
        "{settlement:?}"
    );
    println!(
        "author task={} run={} digest={kinds:?} provider_role={} prompt_bytes={} input_criteria={:?}",
        created.task_id,
        ctx.run_id,
        WORKFLOW_FORGE_PROFILE_ID,
        prompt.len(),
        [ROLE, SKILL, PLUGIN]
    );
    drop(log);
    let draft = app.forge_spec(&created.task_id).unwrap().unwrap();
    for (stage, criterion) in draft.workflows[0].stages.iter().zip([ROLE, SKILL, PLUGIN]) {
        assert!(stage.goal.worker_text().unwrap().contains(criterion));
        assert!(stage.profile_id.is_none());
    }
    let saved_original = saved(app.forge_compile(&draft, Some(&created.task_id), 1));
    assert_eq!(
        (saved_original.created_count, saved_original.updated_count),
        (1, 0)
    );
    let id = &saved_original.template_ids[0];
    let mut original = template(&app, id);
    original.workflow.control.max_attempts = Some(4);
    let slot = binding(&original, "api-check").execution_slot_id.clone();
    original
        .model_bindings
        .bindings
        .iter_mut()
        .find(|b| b.execution_slot_id == slot)
        .unwrap()
        .model_id = Some("gpt-5.4".into());
    app.update_workflow_template_with_bindings(id, original.workflow, original.model_bindings)
        .unwrap();
    let before = template(&app, id);
    let modify = reopen(&app, id, EDIT);
    let prior = node(&before.workflow, "ui-check")
        .goal()
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("acceptance: "))
        .unwrap();
    let new_acceptance = format!("{prior}; {EDIT}");
    let edited = edit_ui(&app, &modify, EDIT, &new_acceptance, "edit-navigation");
    let updated = saved(app.forge_compile(&edited, Some(&modify), 1));
    assert_eq!((updated.created_count, updated.updated_count), (0, 1));
    assert_eq!(updated.template_ids, [id.clone()]);
    let original = template(&app, id);
    assert_eq!(
        json!(before.workflow.control),
        json!(original.workflow.control)
    );
    let persisted = app.forge_spec(&modify).unwrap().unwrap();
    let copied =
        saved(app.forge_compile_as_copy(&persisted, &modify, &persisted.workflows[0].id, COPY, 1));
    let cid = &copied.template_ids[0];
    assert_ne!(cid, id);
    let copy = template(&app, cid);
    for (worker, role) in WORKERS {
        for t in [&before, &original, &copy] {
            assert_eq!(node(&t.workflow, worker).profile(), Some(role));
            assert_eq!(binding(t, worker), binding(&before, worker));
        }
    }
    assert_eq!(json!(copy.workflow.control), json!(before.workflow.control));
    let reopened = reopen(&app, cid, "Reopen the copy");
    let baseline = app.forge_spec(&reopened).unwrap().unwrap();
    for ((worker, role), criterion) in WORKERS.into_iter().zip([ROLE, &new_acceptance, PLUGIN]) {
        let s = baseline.workflows[0]
            .stages
            .iter()
            .find(|s| s.id.as_deref() == Some(worker))
            .unwrap();
        assert!(s.goal.worker_text().unwrap().contains(criterion));
        assert!(
            s.profile_id.is_none(),
            "builtins use the role enum, not a borrowed profile ID"
        );
        assert_eq!(
            format!("pf-builtin-{}", json!(s.role).as_str().unwrap()),
            role
        );
    }
    assert_eq!(json!(template(&app, id)), json!(original));
    let source_after = app.profile_show(&user_role.id).unwrap();
    assert!(!source_after.forged);
    assert_eq!(json!(source_after), json!(selected_role));
    assert!(!app.profiles().unwrap().profiles.iter().any(|p| p.forged));
    let info = copied.trial.unwrap();
    let limits = info.group.unwrap();
    assert_eq!((limits.branches, limits.cap), (2, 2));
    for (trial, fail) in [(true, false), (false, false), (false, true)] {
        let workers = JoinedWorkers {
            probe: BranchProbe {
                fail_node: fail.then_some("ui-check"),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut exec = app_with_provider(repo.clone(), Box::new(workers.clone()));
        exec.config
            .agents
            .retain(|id, _| id.as_str() == "claude-acp");
        let (task, run) =
            if trial {
                let task = exec
                    .forge_trial(cid, &info.revision, "Synthetic group trial")
                    .unwrap()
                    .task
                    .id;
                let frozen: TaskAuthoringWorkflow =
                    read_json(&exec.paths.workflow_file(&task)).unwrap();
                assert_eq!(frozen.workflow.entry, "checks");
                assert_eq!(frozen.workflow.nodes.len(), 3);
                assert!(frozen.workflow.edges.iter().any(|e| e.from == "checks"
                    && e.to == END_NODE
                    && e.on == EdgeOutcome::Success));
                let run = exec.run_start(&task, None).unwrap();
                (task, run)
            } else {
                let mut cmd = CreateConversationTaskCommand::new(
                    "test",
                    REQUEST.into(),
                    ConversationRunMode::Workflow,
                );
                cmd.workflow_template_id = Some(cid.clone());
                let full = exec
                    .create_conversation_run(&cmd, ConversationRunLaunch::Foreground)
                    .unwrap();
                (full.task_id, full.run)
            };
        let expected = if fail {
            RunOutcome::Failure
        } else {
            RunOutcome::Success
        };
        assert_eq!(run.outcome, Some(expected), "{run:?}");
        let state = workers.probe.state.0.lock().unwrap();
        assert_eq!(state.1, 2);
        let calls: Vec<_> = state.2.iter().map(|(id, _)| id.as_str()).collect();
        assert!(matches!(
            &calls[..2],
            ["ui-check", "api-check"] | ["api-check", "ui-check"]
        ));
        assert_eq!(
            &calls[2..],
            if trial || fail {
                &[][..]
            } else {
                &["report"][..]
            }
        );
        assert_ne!(state.2[0].1, state.2[1].1);
        println!(
            "original={id} copy={cid} trial={trial} fail={fail} task={task} run={} peak={} calls={calls:?} finished={:?} worktrees={:?} outcome={:?}",
            run.id,
            state.1,
            workers.finished.lock().unwrap(),
            [&state.2[0].1, &state.2[1].1],
            run.outcome
        );
    }
}
