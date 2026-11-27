use gold_band::dsl::{
    WorkflowDsl, validate_authoring_workflow, validate_workflow, validate_workflow_snapshot,
};

fn parse_workflow(json: &str) -> WorkflowDsl {
    serde_json::from_str(json).expect("workflow should deserialize")
}

#[test]
fn static_parallel_group_owns_worker_branches_without_duplicate_success_edges() {
    let definition = serde_json::json!({
        "version":"0.1", "id":"parallel", "entry":"checks",
        "nodes":[
            {"id":"checks", "type":"parallel-group", "branches":[{"id":"checks-a", "stages":["lint"]}, {"id":"checks-b", "stages":["test"]}], "maxParallel":2},
            {"id":"lint", "type":"worker", "executionSlotId":"lint-slot"},
            {"id":"test", "type":"worker", "executionSlotId":"test-slot"}
        ],
        "edges":[{"from":"checks", "on":"success", "to":"$end"}]
    });
    let workflow: WorkflowDsl = serde_json::from_value(definition).expect("static group node type");
    validate_authoring_workflow(workflow.clone()).expect("members are reachable through ownership");
    assert!(
        validate_workflow(workflow).is_err(),
        "member workers still need executable bindings"
    );
}

#[test]
fn static_parallel_group_rejects_ownership_escapes_and_unbounded_retries() {
    use serde_json::json;
    let base = json!({"version":"0.1","id":"wf","entry":"g","control":{"max_attempts":1},"nodes":[{"type":"parallel-group","id":"g","branches":[{"id":"a","stages":["x","y"]},{"id":"b","stages":["z"]}]},{"type":"worker","id":"x","provider":"claude-acp"},{"type":"worker","id":"y","provider":"claude-acp"},{"type":"worker","id":"z","provider":"claude-acp"}],"edges":[{"from":"g","on":"success","to":"$end"},{"from":"y","on":"failure","to":"x"}]});
    let valid = validate_workflow(serde_json::from_value(base.clone()).unwrap()).unwrap();
    let gold_band::dsl::NodeDsl::ParallelGroup(group) = valid.get_node("g").unwrap() else {
        panic!("expected group")
    };
    assert_eq!(group.max_parallel, 3);
    let branch = group.branches[0].workflow(&valid).unwrap();
    let frozen = validate_workflow(branch).unwrap();
    assert_eq!(frozen.raw.entry, "x");
    assert_eq!(frozen.raw.edges.len(), 3);
    for (pointer, value) in [
        ("/entry", json!("x")),
        ("/nodes/0/branches/1/id", json!("a")),
        ("/nodes/0/branches/1/stages", json!([])),
        ("/nodes/0/branches/1/stages/0", json!("x")),
        ("/nodes/0/branches/0/stages/0", json!("g")),
        ("/edges/1/to", json!("z")),
        ("/edges/1/to", json!("$new-round")),
        ("/edges/1/on", json!("success")),
        ("/edges/0/to", json!("x")),
        ("/control", json!({})),
    ] {
        let mut bad = base.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(
            validate_authoring_workflow(serde_json::from_value(bad).unwrap()).is_err(),
            "{pointer}"
        );
    }
    for budget in [0, 6] {
        let mut bad = base.clone();
        bad["nodes"][0]["maxParallel"] = json!(budget);
        assert!(validate_workflow(serde_json::from_value(bad).unwrap()).is_err());
    }
}

#[test]
fn static_parallel_group_rejects_case_aliases_only_within_the_same_child() {
    use serde_json::json;
    let mut definition = json!({
        "version":"0.1", "id":"fixture", "entry":"g",
        "nodes":[{"type":"parallel-group","id":"g","branches":[
            {"id":"one","stages":["Test","test"]}, {"id":"two","stages":["Review"]}]},
            {"type":"worker","id":"Test","provider":"claude-acp"},
            {"type":"worker","id":"test","provider":"claude-acp"},
            {"type":"worker","id":"Review","provider":"claude-acp"}],
        "edges":[{"from":"g","on":"success","to":"$end"}]
    });
    let ambiguous: WorkflowDsl = serde_json::from_value(definition.clone()).unwrap();
    assert!(validate_workflow(ambiguous).is_err(), "same-child stage directories alias");
    definition["nodes"][0]["branches"][0]["stages"] = json!(["Test", "Review"]);
    definition["nodes"][0]["branches"][1]["stages"] = json!(["test"]);
    validate_workflow(serde_json::from_value(definition).unwrap())
        .expect("distinct child runs do not share node directories");
}

#[test]
fn validates_basic_workflow() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "dev-test-accept",
            "entry": "dev",
            "control": { "max_attempts": 3 },
            "nodes": [
                {
                    "id": "dev",
                    "type": "worker",
                    "provider": "claude-acp",
                    "profile": "developer",
                    "goal": "implement requirement"
                },
                {
                    "id": "test",
                    "type": "worker",
                    "provider": "claude-acp",
                    "profile": "tester",
                    "goal": "Run checks and return JSON with result and reason fields.",
                    "output": { "kind": "json", "artifact": "test-result" },
                    "success_condition": { "path": "result", "equals": true }
                },
                {
                    "id": "accept",
                    "type": "worker",
                    "provider": "claude-acp",
                    "profile": "acceptance",
                    "goal": "Assess acceptance and return JSON with result and reason fields.",
                    "output": { "kind": "json", "artifact": "accept-result" },
                    "success_condition": { "path": "result", "equals": true }
                }
            ],
            "edges": [
                { "from": "dev", "to": "test", "on": "success" },
                { "from": "test", "to": "accept", "on": "success" },
                { "from": "test", "to": "dev", "on": "failure", "session": "continue" },
                { "from": "accept", "to": "$end", "on": "success" },
                { "from": "accept", "to": "$new-round", "on": "failure", "new_round_entry": "$entry" }
            ]
        }"#,
    );

    let validated = validate_workflow(workflow).expect("workflow should validate");
    assert_eq!(validated.raw.entry, "dev");
    assert!(validated.get_node("accept").is_some());
}

#[test]
fn authoring_workflow_accepts_worker_without_execution_provider() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "authoring-worker",
            "entry": "grill",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "grill",
                    "type": "worker",
                    "executionSlotId": "default-lightweight:grill",
                    "profile": "pf-builtin-grill"
                }
            ],
            "edges": [
                { "from": "grill", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    validate_authoring_workflow(workflow.clone())
        .expect("authoring workflow should not own provider configuration");
    let error = validate_workflow(workflow)
        .expect_err("executable workflow must still require an injected provider");
    assert!(error.to_string().contains("provider cannot be blank"));
}

#[test]
fn authoring_workflow_still_rejects_structural_errors() {
    let missing_end = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "missing-end",
            "entry": "grill",
            "control": { "max_attempts": 1 },
            "nodes": [{ "id": "grill", "type": "worker" }],
            "edges": []
        }"#,
    );
    assert!(validate_authoring_workflow(missing_end).is_err());

    let unreachable = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "unreachable",
            "entry": "grill",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "grill", "type": "worker" },
                { "id": "orphan", "type": "worker" }
            ],
            "edges": [{ "from": "grill", "to": "$end", "on": "success" }]
        }"#,
    );
    assert!(validate_authoring_workflow(unreachable).is_err());
}

#[test]
fn accepts_ai_dynamic_real_provider_permission_mode() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "dynamic-real-permission-mode",
            "entry": "ai-dynamic",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "ai-dynamic",
                    "type": "ai-dynamic",
                    "agentStrategy": {
                        "mode": "dynamic",
                        "bootstrapProvider": "claude-acp",
                        "availableAgents": [
                            { "provider": "claude-acp", "model": "claude-sonnet-4-6" }
                        ],
                        "routingPrompt": "Pick a Claude worker."
                    },
                    "permissionMode": "bypassPermissions",
                    "control": {
                        "maxDynamicNodes": 4,
                        "maxFanout": 2,
                        "maxDepth": 2,
                        "maxParallel": 1
                    }
                }
            ],
            "edges": [
                { "from": "ai-dynamic", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    validate_workflow(workflow).expect("real provider permission mode should validate");
}

#[test]
fn rejects_workflow_without_end_node() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "missing-end",
            "entry": "dev",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "dev",
                    "type": "worker",
                    "provider": "claude-acp",
                    "profile": "developer",
                    "goal": "implement requirement"
                }
            ],
            "edges": [
                { "from": "dev", "to": "$new-round", "on": "failure", "new_round_entry": "$entry" }
            ]
        }"#,
    );

    let error = validate_workflow(workflow).expect_err("workflow should require an end node");
    assert!(
        error
            .to_string()
            .contains("must include an edge targeting `$end`")
    );
}

#[test]
fn rejects_unreachable_node() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "unreachable-node",
            "entry": "router",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "router", "type": "worker", "provider": "claude-acp" },
                { "id": "accept", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "router", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    let error = validate_workflow(workflow).expect_err("workflow should reject unreachable node");
    assert!(
        error
            .to_string()
            .contains("node `accept` is unreachable from entry")
    );
}

#[test]
fn rejects_success_edge_to_new_round() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "success-new-round",
            "entry": "accept",
            "control": { "max_rounds": 1 },
            "nodes": [
                { "id": "accept", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "accept", "to": "$new-round", "on": "success" }
            ]
        }"#,
    );

    let error = validate_workflow(workflow).expect_err("success should not open new round");
    assert!(
        error
            .to_string()
            .contains("cannot target `$new-round` on success")
    );
}

#[test]
fn rejects_new_round_edge_without_entry() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "missing-new-round-entry",
            "entry": "accept",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "accept", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "accept", "to": "$new-round", "on": "failure" },
                { "from": "accept", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    let error = validate_workflow(workflow).expect_err("new round edge requires an entry");
    assert!(error.to_string().contains("must declare `new_round_entry`"));
}

#[test]
fn snapshot_validation_defaults_missing_new_round_entry_to_entry() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "legacy-new-round-entry",
            "entry": "accept",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "accept", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "accept", "to": "$new-round", "on": "failure" },
                { "from": "accept", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    let validated =
        validate_workflow_snapshot(workflow).expect("legacy snapshot should default to entry");
    let edge = validated
        .raw
        .edges
        .iter()
        .find(|edge| edge.to == "$new-round")
        .unwrap();
    assert_eq!(edge.new_round_entry.as_deref(), Some("$entry"));
}

#[test]
fn rejects_new_round_edge_with_missing_entry_node() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "invalid-new-round-entry",
            "entry": "accept",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "accept", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "accept", "to": "$new-round", "on": "failure", "new_round_entry": "dev" },
                { "from": "accept", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    let error = validate_workflow(workflow).expect_err("new round entry must exist");
    assert!(error.to_string().contains("invalid `new_round_entry`: dev"));
}

#[test]
fn accepts_new_round_entry_as_reachable_start_node() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "new-round-entry-node",
            "entry": "accept",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "accept", "type": "worker", "provider": "claude-acp" },
                { "id": "dev", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "accept", "to": "$new-round", "on": "failure", "new_round_entry": "dev" },
                { "from": "accept", "to": "$end", "on": "success" },
                { "from": "dev", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    validate_workflow(workflow).expect("new round entry should make the node reachable");
}

#[test]
fn serializes_non_new_round_edge_without_new_round_entry() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "serialize-edge",
            "entry": "dev",
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "dev", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    let json = serde_json::to_string(&workflow.edges[0]).unwrap();
    assert!(!json.contains("new_round_entry"));
}

#[test]
fn rejects_unknown_node_type() {
    let workflow = serde_json::from_str::<WorkflowDsl>(
        r#"{
            "version": "0.1",
            "id": "unknown-node-type",
            "entry": "custom",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "custom", "type": "custom", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "review", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    assert!(workflow.is_err());
}

#[test]
fn rejects_reserved_terminal_node_ids() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "reserved-id",
            "entry": "$end",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "$end", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": []
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn accepts_missing_loop_limits() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "unlimited-loops",
            "entry": "dev",
            "control": {},
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "dev", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    validate_workflow(workflow).expect("missing limits should mean unlimited");
}

#[test]
fn rejects_zero_attempt_limit() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "zero-attempts",
            "entry": "dev",
            "control": { "max_attempts": 0 },
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": []
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn rejects_zero_round_limit() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "zero-rounds",
            "entry": "dev",
            "control": { "max_rounds": 0 },
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": []
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn rejects_invalid_edge_outcome() {
    let err = serde_json::from_str::<WorkflowDsl>(
        r#"{
            "version": "0.1",
            "id": "invalid-edge-outcome",
            "entry": "dev",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" },
                { "id": "test", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "dev", "to": "test", "on": "success" },
                { "from": "test", "to": "$end", "on": "invalid" }
            ]
        }"#,
    )
    .expect_err("invalid edge outcome should not deserialize");

    assert!(err.to_string().contains("unknown variant"));
}

#[test]
fn rejects_duplicate_edge_outcomes_from_same_source() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "duplicate-success-edge",
            "entry": "dev",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" },
                { "id": "test", "type": "worker", "provider": "claude-acp" },
                { "id": "accept", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "dev", "to": "test", "on": "success" },
                { "from": "dev", "to": "accept", "on": "success" }
            ]
        }"#,
    );

    let error =
        validate_workflow(workflow).expect_err("duplicate outcome edges should be rejected");
    assert!(error.to_string().contains("already has a Success edge"));
}

#[test]
fn rejects_continue_edges_to_unsupported_provider() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "unsupported-provider",
            "entry": "dev",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "dev", "type": "worker", "provider": "claude-acp" },
                { "id": "review", "type": "worker", "provider": "other-provider" }
            ],
            "edges": [
                { "from": "dev", "to": "review", "on": "success", "session": "continue" }
            ]
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn accepts_worker_json_output_validation() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": { "kind": "json", "artifact": "review-result" },
                    "success_condition": { "path": "passed", "equals": true }
                },
                {
                    "id": "test",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": { "kind": "json", "artifact": "test-result" },
                    "success_condition": { "path": "passed", "equals": true }
                }
            ],
            "edges": [
                { "from": "review", "to": "test", "on": "success" },
                { "from": "test", "to": "$end", "on": "success" },
                { "from": "test", "to": "$new-round", "on": "failure", "new_round_entry": "$entry" }
            ]
        }"#,
    );

    let validated = validate_workflow(workflow).expect("workflow should validate");
    assert_eq!(validated.raw.nodes.len(), 2);
}

#[test]
fn accepts_simplified_output_schema_with_matching_expression() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": {
                        "kind": "json",
                        "artifact": "review-result",
                        "schema": { "reason": "String", "result": "boolean" }
                    },
                    "success_condition": { "expression": "$.result == true" }
                }
            ],
            "edges": [
                { "from": "review", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    validate_workflow(workflow).expect("workflow should validate");
}

#[test]
fn rejects_success_expression_missing_from_simplified_schema() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": {
                        "kind": "json",
                        "artifact": "review-result",
                        "schema": { "reason": "String" }
                    },
                    "success_condition": { "expression": "$.result == true" }
                }
            ],
            "edges": [
                { "from": "review", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn accepts_nested_simplified_schema_path() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": {
                        "kind": "json",
                        "artifact": "review-result",
                        "schema": { "xx": { "yy": [{ "zz": "boolean" }] } }
                    },
                    "success_condition": { "expression": "$.xx.yy[0].zz == true" }
                }
            ],
            "edges": [
                { "from": "review", "to": "$end", "on": "success" }
            ]
        }"#,
    );

    validate_workflow(workflow).expect("workflow should validate");
}

#[test]
fn rejects_malformed_success_expression_path() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": {
                        "kind": "json",
                        "artifact": "review-result",
                        "schema": { "xx": { "yy": "boolean" } }
                    },
                    "success_condition": { "expression": "$.xx..yy == true" }
                }
            ],
            "edges": []
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn rejects_legacy_json_schema_output_constraint() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": {
                        "kind": "json",
                        "artifact": "review-result",
                        "schema": {
                            "type": "object",
                            "properties": { "result": { "type": "boolean" } },
                            "required": ["result"]
                        }
                    },
                    "success_condition": { "expression": "$.result == true" }
                }
            ],
            "edges": []
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn rejects_success_condition_without_output() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                {
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "success_condition": { "path": "passed", "equals": true }
                }
            ],
            "edges": []
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

#[test]
fn rejects_continue_to_new_round_target() {
    let workflow = parse_workflow(
        r#"{
            "version": "0.1",
            "id": "new-round-session",
            "entry": "review",
            "control": { "max_attempts": 1 },
            "nodes": [
                { "id": "review", "type": "worker", "provider": "claude-acp" }
            ],
            "edges": [
                { "from": "review", "to": "$new-round", "on": "failure", "session": "continue", "new_round_entry": "$entry" }
            ]
        }"#,
    );

    assert!(validate_workflow(workflow).is_err());
}

/// 单 worker 工作流，output.schema 取给定的 JSON 文本。
fn worker_with_output_schema(schema: &str) -> WorkflowDsl {
    let json = format!(
        r#"{{
            "version": "0.1",
            "id": "worker-validation",
            "entry": "review",
            "control": {{ "max_attempts": 1 }},
            "nodes": [
                {{
                    "id": "review",
                    "type": "worker",
                    "provider": "claude-acp",
                    "output": {{ "kind": "json", "artifact": "review-result", "schema": {schema} }}
                }}
            ],
            "edges": [ {{ "from": "review", "to": "$end", "on": "success" }} ]
        }}"#
    );
    parse_workflow(&json)
}

#[test]
fn accepts_simplified_schema_fields_named_like_json_schema_keywords() {
    // 回归：简化字段表是「字段名 → 形状」，字段名可以恰好叫 items / type / required。
    // 旧判定只看键名，把 Forge 对话产出的 `items` 字段当成了 JSON Schema。
    for schema in [
        r#"{ "items": "数组，每项一条核对结果", "complete": "boolean" }"#,
        r#"{ "items": [{ "name": "string", "status": "string" }], "total": "number" }"#,
        r#"{ "type": "string", "level": "string", "message": "string" }"#,
        r#"{ "required": "boolean", "properties": { "name": "string" } }"#,
        r#"{ "additionalProperties": { "key": "string" } }"#,
        r#"{ "type": "string", "items": "string" }"#,
    ] {
        validate_workflow(worker_with_output_schema(schema))
            .unwrap_or_else(|error| panic!("{schema} should validate: {error}"));
    }
}

#[test]
fn still_rejects_legacy_json_schema_output_constraints() {
    for schema in [
        r#"{ "type": "object", "properties": { "result": { "type": "boolean" } }, "required": ["result"] }"#,
        r#"{ "type": "array", "items": { "type": "object", "properties": { "id": { "type": "string" } } } }"#,
        r#"{ "properties": { "result": { "type": "boolean" } } }"#,
        r#"{ "type": "object", "additionalProperties": false }"#,
        r#"{ "type": "object", "required": ["result"] }"#,
        r#"{ "type": ["string", "null"] }"#,
        r#"{ "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object" }"#,
        // 旧版基本类型根节点：整个输出就是一个字符串/布尔值，简化字段表不这样表达。
        r#"{ "type": "boolean" }"#,
    ] {
        assert!(
            validate_workflow(worker_with_output_schema(schema)).is_err(),
            "{schema} should be rejected as JSON Schema"
        );
    }
}
