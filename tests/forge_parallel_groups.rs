use gold_band::forge::{ForgeSpec, compiler::compile};

// Use the public wire protocol so a parser silently ignoring topology cannot pass.
#[test]
fn archived_group_compiles_as_one_outer_node_not_a_worker_chain() {
    let spec = ForgeSpec::parse_and_validate(r#"{
      "specVersion":"0.1","name":"fixture","intent":"parallel review",
      "workflows":[{"name":"fixture","parallelGroups":[{"id":"gate","maxParallel":1,
      "branches":[{"id":"api","stages":["build","check"]},{"id":"ui","stages":["render"]}],
      "onFailure":{"goto":"report"}}],"stages":[
      {"id":"build","role":"dev","goal":"<输入>a<产出>b<边界>c<验收>d","outputKind":"none","onFailure":"retry"},
      {"id":"check","role":"test","goal":"<输入>a<产出>b<边界>c<验收>d","outputKind":"none","onFailure":{"goto":"build"}},
      {"id":"render","role":"dev","goal":"<输入>a<产出>b<边界>c<验收>d","outputKind":"none","onFailure":"end"},
      {"id":"report","role":"accept","goal":"<输入>a<产出>b<边界>c<验收>d","outputKind":"none","onFailure":"end"}]}]}
    "#).unwrap();
    let dsl = compile(&spec).unwrap().remove(0).workflow;
    assert_eq!(dsl.entry, "gate");
    assert_eq!(dsl.nodes.len(), 5);
    let routes: Vec<_> = dsl.edges.iter().map(|e| (e.from.as_str(), e.to.as_str(), e.on)).collect();
    use gold_band::dsl::EdgeOutcome::{Success, Failure};
    assert!(routes.contains(&("gate", "report", Success)));
    assert!(routes.contains(&("gate", "report", Failure)));
    assert!(routes.contains(&("check", "build", Failure)));
    assert!(!routes.iter().any(|(from,_,on)| ["build","check","render"].contains(from) && *on == Success));
    assert!(!routes.iter().any(|(from,_,_)| from == &"render"));
}
