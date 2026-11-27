use super::{expect_saved, fixture, source_task, spec, task, template_of as template};
use gold_band::{
    app::forge_service::ForgeCompileOutcome,
    dsl::NodeDsl,
    forge::{
        ForgeSpec, StageRole,
        interview::{SpecDelta, merge_delta},
    },
};

fn saved(outcome: anyhow::Result<ForgeCompileOutcome>) -> String {
    expect_saved(outcome)
        .template_ids
        .into_iter()
        .next()
        .unwrap()
}

fn paired_roles() -> ForgeSpec {
    let mut draft = spec();
    let original = draft.workflows[0].stages[0].clone();
    draft.workflows[0].stages = [
        ("alpha", "Alpha", "Alpha charter"),
        ("beta", "Beta", "Beta charter"),
    ]
    .into_iter()
    .map(|(id, name, charter)| {
        let mut stage = original.clone();
        stage.id = Some(id.into());
        stage.role = StageRole::Custom {
            profile_name: name.into(),
        };
        stage.role_definition = Some(charter.into());
        stage.profile_id = None;
        stage
    })
    .collect();
    draft
}

#[test]
fn model_and_full_save_use_authorized_ids_and_can_switch_to_an_existing_persona() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let id = saved(app.forge_compile(&paired_roles(), Some(&task), 1));
    let mut draft = app.forge_spec(&task).unwrap().unwrap();
    let alpha = draft.workflows[0].stages[0].profile_id.clone().unwrap();
    let beta = draft.workflows[0].stages[1].profile_id.clone().unwrap();
    draft.workflows[0].stages[0].profile_id = Some("fixture-untrusted".into());
    let delta = SpecDelta {
        stages: vec![draft.workflows[0].stages[0].clone()],
        ..Default::default()
    };
    let baseline = app.forge_spec(&task).unwrap().unwrap();
    assert_eq!(
        merge_delta(&baseline, &delta).unwrap().workflows[0].stages[0]
            .profile_id
            .as_deref(),
        Some(alpha.as_str())
    );
    saved(app.forge_compile(&draft, Some(&task), 1));
    assert_eq!(
        template(&app, &id).workflow.nodes[0].profile(),
        Some(alpha.as_str())
    );
    let baseline = app.forge_spec(&task).unwrap().unwrap();
    let mut change = baseline.workflows[0].stages[0].clone();
    change.role = baseline.workflows[0].stages[1].role.clone();
    change.profile_id = None;
    let revised = merge_delta(
        &baseline,
        &SpecDelta {
            stages: vec![change],
            ..Default::default()
        },
    )
    .unwrap();
    saved(app.forge_compile(&revised, Some(&task), 1));
    let saved_template = template(&app, &id);
    assert!(
        saved_template
            .workflow
            .nodes
            .iter()
            .all(|n| n.profile() == Some(beta.as_str()))
    );
    assert_eq!(app.profile_show(&beta).unwrap().content, "Beta charter");
}

#[test]
fn healthy_copy_detaches_generated_roles_only_after_the_name_is_accepted() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let id = saved(app.forge_compile(&paired_roles(), Some(&task), 1));
    let source = template(&app, &id);
    let draft = app.forge_spec(&task).unwrap().unwrap();
    let key = &draft.workflows[0].id;
    let forged_roles = || app.profiles().unwrap().profiles.into_iter()
        .filter(|role| role.forged).map(|role| (role.id.clone(), serde_json::json!(role)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let originals = forged_roles();
    let duplicate = app.forge_compile_as_copy(&draft, &task, key, &source.name, 1).unwrap();
    assert!(matches!(duplicate, ForgeCompileOutcome::Repairable { errors, .. }
        if errors.iter().any(|error| error.code == "forge/duplicate-workflow-name")));
    assert_eq!(forged_roles(), originals);

    let copy_id = saved(app.forge_compile_as_copy(&draft, &task, key, "Independent copy", 1));
    let copy = template(&app, &copy_id);
    for node in &source.workflow.nodes {
        let original_id = node.profile().unwrap();
        let copy_id = copy.workflow.nodes.iter().find(|other| other.id() == node.id())
            .unwrap().profile().unwrap();
        assert!(!originals.contains_key(copy_id));
        assert_eq!(serde_json::json!(app.profile_show(original_id).unwrap()), originals[original_id]);
        assert_eq!(app.profile_show(copy_id).unwrap().content, originals[original_id]["content"].as_str().unwrap());
    }
}

#[test]
fn explicit_copy_recovers_recycled_personas_without_claiming_their_old_ids() {
    let (app, _temp) = fixture();
    let task = task(&app);
    let id = saved(app.forge_compile(&paired_roles(), Some(&task), 1));
    let before = app.forge_spec(&task).unwrap().unwrap();
    app.delete_workflow_template(&id).unwrap();
    let old = before.workflows[0].stages[0].profile_id.as_ref().unwrap();
    assert!(app.profile_show(old).is_err());
    let copy = saved(app.forge_compile_as_copy(
        &before,
        &task,
        &before.workflows[0].id,
        "Recovered fixture",
        1,
    ));
    assert_ne!(copy, id);
    let after = app.forge_spec(&task).unwrap().unwrap();
    for (initial, restored) in before.workflows[0]
        .stages
        .iter()
        .zip(&after.workflows[0].stages)
    {
        assert_ne!(initial.profile_id, restored.profile_id);
        assert_eq!(
            Some(
                app.profile_show(restored.profile_id.as_ref().unwrap())
                    .unwrap()
                    .content
            ),
            initial.role_definition
        );
    }
}

#[test]
fn missing_second_role_rejects_before_changing_first_role_template_or_draft() {
    let (app, temp) = fixture();
    let task = task(&app);
    let id = saved(app.forge_compile(&paired_roles(), Some(&task), 1));
    let mut edit = app.forge_spec(&task).unwrap().unwrap();
    let alpha = app
        .profile_show(edit.workflows[0].stages[0].profile_id.as_ref().unwrap())
        .unwrap();
    let beta = app
        .profile_show(edit.workflows[0].stages[1].profile_id.as_ref().unwrap())
        .unwrap();
    let path = std::fs::canonicalize(&beta.path).unwrap();
    assert!(path.starts_with(std::fs::canonicalize(temp.path()).unwrap()));
    std::fs::remove_file(path).unwrap();
    let original = serde_json::to_value(template(&app, &id)).unwrap();
    edit.workflows[0].stages[0].role_definition = Some("Revised alpha charter".into());
    let outcome = app.forge_compile(&edit, Some(&task), 1);
    assert_eq!(app.profile_show(&alpha.id).unwrap().content, alpha.content);
    assert_eq!(serde_json::to_value(template(&app, &id)).unwrap(), original);
    assert_eq!(
        app.forge_spec(&task).unwrap().unwrap().revision,
        edit.revision
    );
    let ForgeCompileOutcome::Repairable { errors, .. } = outcome.unwrap() else {
        panic!("must not save")
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "forge/unavailable-role");
}

#[test]
fn all_seven_localized_protocol_goals_reach_saved_workers() {
    use gold_band::config::DesktopLanguage::*;
    let (mut app, _temp) = fixture();
    for lang in [ZhCn, ZhTw, En, JaJp, KoKr, PtBr, Es] {
        app.config.desktop_language = lang;
        let mut draft = spec();
        let name = format!("Goal-{lang:?}");
        draft.name = Some(name.clone());
        draft.workflows[0].name = name;
        let example = gold_band::prompts::FORGE_SPEC_PROTOCOL
            .resolve(lang)
            .split("```json")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        let example: serde_json::Value = serde_json::from_str(example.trim()).unwrap();
        let goal = &example["stages"][0]["goal"];
        draft.workflows[0].stages[0].goal = serde_json::from_value(goal.clone()).unwrap();
        let task = task(&app);
        let id = saved(app.forge_compile(&draft, Some(&task), 1));
        let archived = app.forge_spec(&task).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&archived.workflows[0].stages[0].goal).unwrap(),
            *goal
        );
        let saved = template(&app, &id);
        let NodeDsl::Worker(worker) = &saved.workflow.nodes[0] else {
            panic!("expected worker")
        };
        for key in ["input", "output", "boundary", "acceptance"] {
            assert!(
                worker
                    .goal
                    .as_ref()
                    .unwrap()
                    .contains(goal[key].as_str().unwrap())
            );
        }
    }
}

#[test]
fn editor_worker_without_a_role_roundtrips_its_unmodified_free_text_and_identity() {
    let (app, _temp) = fixture();
    let mut workflow = gold_band::forge::compiler::compile(&spec())
        .unwrap()
        .remove(0)
        .workflow;
    let NodeDsl::Worker(worker) = &mut workflow.nodes[0] else {
        panic!("worker")
    };
    worker.prompt_envelope = gold_band::dsl::PromptEnvelopeMode::RawAgent;
    worker.profile = None;
    let text = "  No prescribed headings; preserve this editor text.  ";
    worker.goal = Some(text.into());
    let name = "Editor worker without role";
    let store = app.save_workflow_template(name.into(), workflow).unwrap();
    let initial = store.templates.iter().find(|t| t.name == name).unwrap();
    let (task, mut projected) = source_task(&app, &initial.id);
    projected.intent = "Change the workflow intent only".into();
    let count = app.profiles().unwrap().profiles.len();
    assert_eq!(
        saved(app.forge_compile(&projected, Some(&task), 1)),
        initial.id
    );
    let revised = template(&app, &initial.id);
    let NodeDsl::Worker(worker) = &revised.workflow.nodes[0] else {
        panic!("worker")
    };
    assert_eq!(worker.profile, None);
    assert_eq!(worker.goal.as_deref(), Some(text));
    assert_eq!(app.profiles().unwrap().profiles.len(), count);
}
