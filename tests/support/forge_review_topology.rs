use super::{expect_saved, fixture, source_task, template_of};
use gold_band::dsl::{EdgeOutcome, NodeDsl, WorkflowDsl, END_NODE};
use gold_band::workflow_model_binding::TaskAuthoringWorkflow;
use serde_json::{json, to_value};

fn group_definition() -> WorkflowDsl {
    let mut nodes = vec![json!({"type":"parallel-group","id":"fork","maxParallel":1,
        "branches":[{"id":"backend","stages":["api","test"]},{"id":"front","stages":["ui"]}]})];
    // Deliberately not branch order: only the structural references determine execution.
    nodes.extend(["report", "ui", "test", "api"].map(|id| json!({"type":"worker","id":id,
        "executionSlotId":format!("slot-{id}"),"profile":"pf-builtin-dev"})));
    serde_json::from_value(json!({"id":"fixture","version":"0.1","entry":"fork",
        "control":{"max_rounds":4,"max_attempts":7},"nodes":nodes,"edges":[
            {"from":"fork","on":"success","to":"report"}, {"from":"fork","on":"failure","to":"$end"},
            {"from":"test","on":"failure","to":"api"}, {"from":"report","on":"success","to":"$end"}]})).unwrap()
}

#[test]
fn unrelated_edit_keeps_the_original_implicit_terminal_success() {
    let (app, _temp) = fixture();
    let mut graph = group_definition();
    graph.edges.retain(|e| !(e.from == "report" && e.on == EdgeOutcome::Success));
    let store = app.save_workflow_template("Implicit success".into(), graph).unwrap();
    let base = store.templates.iter().find(|t| t.name == "Implicit success").unwrap();
    let (task_id, draft) = source_task(&app, &base.id);
    expect_saved(app.forge_compile(&draft, Some(&task_id), 1));
    assert!(!template_of(&app, &base.id).workflow.edges.iter().any(|e| e.from == "report" && e.on == EdgeOutcome::Success));
}

#[test]
fn trial_rejects_changed_definition_and_binding_before_creating_any_task() {
    use gold_band::app::forge_service::StaleForgeTrial;
    let (app, _temp) = fixture();
    let saved = expect_saved(app.forge_compile(&super::spec(), None, 1));
    let first = saved.trial.unwrap();
    assert!(first.group.is_none());
    let id = first.template_id;
    let count = app.task_list().unwrap().len();
    let mut group = group_definition();
    group.id = template_of(&app, &id).workflow.id;
    app.update_workflow_template(&id, group).unwrap();
    let retry = |revision: &str| {
        let error = app.forge_trial(&id, revision, "local sample").unwrap_err();
        let current = error.downcast::<StaleForgeTrial>().unwrap().current;
        assert_eq!(current.template_id, id);
        assert_ne!(current.revision, revision);
        assert_eq!(app.task_list().unwrap().len(), count);
        current
    };
    let second = retry(&first.revision);
    let group = second.group.unwrap();
    assert_eq!((group.branches, group.cap), (2, 1));
    let mut template = template_of(&app, &id);
    template.model_bindings.bindings.push(serde_json::from_value(json!({
        "executionSlotId":"slot-api", "agentId":"claude-acp", "modelId":"fixture-model"
    })).unwrap());
    app.update_workflow_template_with_bindings(&id, template.workflow, template.model_bindings).unwrap();
    let third = retry(&second.revision);
    let trial = app.forge_trial(&id, &third.revision, "local sample").unwrap();
    let frozen: TaskAuthoringWorkflow = gold_band::storage::read_json(&app.paths.workflow_file(&trial.task.id)).unwrap();
    assert_eq!(app.task_list().unwrap().len(), count + 1);
    assert_eq!(frozen.workflow.entry, "fork");
    assert!(frozen.model_bindings.bindings.iter().any(|b| b.execution_slot_id == "slot-api" && b.model_id.as_deref() == Some("fixture-model")));
}

#[test]
fn forge_parallel_group_modify_and_trial_keep_the_frozen_first_group() {
    let (app, _temp) = fixture();
    let store = app.save_workflow_template("Group fixture".into(), group_definition()).unwrap();
    let before = store.templates.iter().find(|t| t.name == "Group fixture").unwrap();
    let (task_id, mut spec) = source_task(&app, &before.id);
    let w = &mut spec.workflows[0];
    assert_eq!(w.stages.iter().map(|s| s.id.as_deref().unwrap()).collect::<Vec<_>>(), ["api", "test", "ui", "report"]);
    assert_eq!(w.parallel_groups[0].max_parallel, 1);
    w.stages[3].goal = serde_json::from_value(json!({"input":"Reports","output":"Summary","boundary":"Read only","acceptance":"All checked"})).unwrap();
    let saved = expect_saved(app.forge_compile(&spec, Some(&task_id), 1));
    assert_eq!(saved.template_ids, [before.id.clone()]);
    let after = template_of(&app, &before.id);
    assert_eq!(after.workflow.entry, "fork");
    assert_eq!(to_value(&after.workflow.control).unwrap(), to_value(&before.workflow.control).unwrap());
    let shape = |workflow: &WorkflowDsl| {
        let mut edges = workflow.edges.iter().map(|e| serde_json::to_string(e).unwrap()).collect::<Vec<_>>();
        edges.sort(); edges
    };
    assert_eq!(shape(&after.workflow), shape(&before.workflow));
    let structure = |graph: &WorkflowDsl| to_value(graph.nodes.iter().find(|n| n.id() == "fork").unwrap()).unwrap();
    assert_eq!(structure(&after.workflow), structure(&before.workflow));
    for n in &after.workflow.nodes {
        if let NodeDsl::Worker(w) = n {
            assert_eq!(w.execution_slot_id, Some(format!("slot-{}", w.id)));
            if w.id == "report" { assert!(w.goal.as_deref().unwrap().contains("Summary")); }
        }
    }
    let trial = app.forge_trial(&before.id, &saved.trial.unwrap().revision, "isolated group trial").unwrap();
    let frozen = gold_band::storage::read_json::<TaskAuthoringWorkflow>(&app.paths.workflow_file(&trial.task.id)).unwrap().workflow;
    assert_eq!(frozen.entry, "fork");
    assert_eq!(frozen.nodes.len(), 4);
    assert_eq!(frozen.control.max_rounds, Some(1));
    assert_eq!(frozen.control.max_attempts, Some(7));
    assert!(!frozen.nodes.iter().any(|n| n.id() == "report"));
    assert_eq!(structure(&frozen), structure(&before.workflow));
    assert!(frozen.edges.iter().any(|e| e.from == "test" && e.to == "api" && e.on == EdgeOutcome::Failure));
    assert!(frozen.edges.iter().any(|e| e.from == "fork" && e.to == END_NODE && e.on == EdgeOutcome::Success));
    assert_eq!(template_of(&app, &before.id).workflow.nodes.len(), 5);
}
