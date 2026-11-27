//! Structural graph projection. Branch admission status belongs to the branch,
//! not to all its worker stages (the latter execute sequentially in a child run).
use super::*;

fn branch_id(group: &str, branch: &str) -> String {
    format!("$branch:{group}:{branch}")
}

fn join_id(group: &str) -> String {
    format!("$join:{group}")
}

fn marker(id: String, kind: &str, label: &str) -> GraphNodeVm {
    GraphNodeVm {
        id,
        round_id: None,
        node_id: None,
        sequence: None,
        label: label.into(),
        node_type: kind.into(),
        status: None,
        outcome: None,
        runtime_display: runtime_display_vm(None, None, false, None, false),
        attempt_id: None,
        outer_node_id: None,
        outer_attempt_id: None,
        attempt_count: 0,
        attempts: vec![],
        artifact_count: 0,
        attachment_count: 0,
        current: false,
        retryable: false,
        icon_key: None,
        session_mode: None,
        continue_from_node_id: None,
        dynamic_summary: None,
        dynamic_group_id: None,
    }
}

fn connect(graph: &mut GraphVm, from: &str, to: &str) {
    graph.edges.push(GraphEdgeVm {
        from: from.into(),
        to: to.into(),
        label: "success".into(),
        traversal_count: 0,
        last_outcome: None,
        blocked_reason: None,
    });
}

pub(super) fn add_structure(graph: &mut GraphVm, workflow: &WorkflowDsl) {
    if !workflow
        .nodes
        .iter()
        .any(|n| matches!(n, NodeDsl::ParallelGroup(_)))
    {
        return;
    }
    graph.nodes.push(marker("$start".into(), "start", "$start"));
    graph.nodes.push(marker("$end".into(), "end", "$end"));
    connect(graph, "$start", &workflow.entry);
    for node in &workflow.nodes {
        let NodeDsl::ParallelGroup(group) = node else {
            continue;
        };
        let join = join_id(&group.id);
        graph
            .nodes
            .push(marker(join.clone(), "parallel-join", &group.id));
        for edge in &mut graph.edges {
            if edge.from == group.id {
                edge.from.clone_from(&join);
            }
        }
        for branch in &group.branches {
            let header = branch_id(&group.id, &branch.id);
            graph
                .nodes
                .push(marker(header.clone(), "parallel-branch", &branch.id));
            connect(graph, &group.id, &header);
            let mut previous = header.as_str();
            for stage in &branch.stages {
                connect(graph, previous, stage);
                previous = stage;
            }
            connect(graph, previous, &join);
        }
    }
}

pub(super) fn round_projection(
    app: &App,
    task_id: &str,
    run: &RunState,
    round: &RoundState,
    workflow: &WorkflowDsl,
    observed: GraphVm,
) -> GraphVm {
    let mut graph = workflow_graph_vm(app, workflow);
    // Keep traversal data for declared outer transitions only. The outer trace orders
    // group attempts, not their children; it must never imply an edge between siblings.
    let exits: HashMap<_, _> = workflow
        .nodes
        .iter()
        .filter_map(|node| match node {
            NodeDsl::ParallelGroup(group) => Some((group.id.as_str(), join_id(&group.id))),
            _ => None,
        })
        .collect();
    // AI-DYNAMIC may appear before or after the fixed group. Preserve its own
    // observed dependency graph; those nodes are not authored branch workers.
    let dynamic_ids: HashSet<_> = workflow
        .nodes
        .iter()
        .filter_map(|node| match node {
            NodeDsl::AiDynamic(dynamic) => Some(dynamic.id.as_str()),
            _ => None,
        })
        .collect();
    let (expanded, ordinary): (Vec<_>, Vec<_>) = observed.nodes.into_iter().partition(|n| {
        n.outer_node_id
            .as_deref()
            .is_some_and(|id| dynamic_ids.contains(id))
    });
    let expanded_ids: HashSet<_> = expanded.iter().map(|n| n.id.as_str()).collect();
    let mut traversals = HashMap::new();
    let mut dynamic_edges = Vec::new();
    for mut edge in observed.edges {
        if let Some(join) = exits.get(edge.from.as_str()) {
            edge.from.clone_from(join);
        }
        if expanded_ids.contains(edge.from.as_str()) || expanded_ids.contains(edge.to.as_str()) {
            dynamic_edges.push(edge);
        } else {
            traversals.insert(
                (edge.from.clone(), edge.to.clone(), edge.label.clone()),
                edge,
            );
        }
    }
    graph.groups.extend(observed.groups);
    graph.nodes.extend(expanded);
    for edge in &mut graph.edges {
        if let Some(observed) =
            traversals.remove(&(edge.from.clone(), edge.to.clone(), edge.label.clone()))
        {
            *edge = observed;
        }
    }
    graph.edges.extend(dynamic_edges);
    let mut observations: HashMap<_, _> = ordinary.into_iter().map(|n| (n.id.clone(), n)).collect();
    let indexes: HashMap<_, _> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    for node in &workflow.nodes {
        let Some(vm) = observations.remove(node.id()) else {
            continue;
        };
        if let (NodeDsl::ParallelGroup(group), Some(attempt)) = (node, vm.attempt_id.as_deref()) {
            if let Some(state) =
                dynamic_graph_state_optional(app, task_id, &run.id, &round.id, &group.id, attempt)
            {
                for leaf in &state.nodes {
                    let Some(id) = leaf.workflow_id.as_deref() else {
                        continue;
                    };
                    let Some(&i) = indexes.get(&branch_id(&group.id, id)) else {
                        continue;
                    };
                    let branch = &mut graph.nodes[i];
                    let status = enum_label(&leaf.status);
                    let outcome = leaf.outcome.map(|o| enum_label(&o));
                    branch.current = vm.current && state.run.current_node_ids.contains(&leaf.id);
                    branch.runtime_display = runtime_display_vm(
                        Some(&status),
                        outcome.as_deref(),
                        branch.current,
                        leaf.pause_reason.as_ref().map(enum_label).as_deref(),
                        false,
                    );
                    branch.status = Some(status);
                    branch.outcome = outcome;
                }
                if let Some(&i) = indexes.get(&join_id(&group.id)) {
                    if vm.outcome.is_some() {
                        graph.nodes[i].runtime_display = vm.runtime_display.clone();
                        graph.nodes[i].status.clone_from(&vm.status);
                        graph.nodes[i].outcome.clone_from(&vm.outcome);
                    }
                }
            }
        }
        if let Some(&i) = indexes.get(node.id()) {
            graph.nodes[i] = vm;
        }
    }
    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_definition_draws_declared_parallel_paths_not_completion_order() {
        let temp = tempfile::tempdir().unwrap();
        let app = App::new(camino::Utf8PathBuf::from_path_buf(temp.path().into()).unwrap());
        let workflow: WorkflowDsl = serde_json::from_value(serde_json::json!({
            "version": "0.1", "id": "regression", "entry": "checks",
            "nodes": [
                {"id":"checks", "type":"parallel-group", "branches":[
                    {"id":"api", "stages":["unit","contract"]}, {"id":"web", "stages":["ui"]}
                ]},
                {"type":"worker", "id":"unit"}, {"type":"worker", "id":"contract"},
                {"type":"worker", "id":"ui"}, {"type":"worker", "id":"report"}
            ], "edges":[{"from":"checks","on":"success","to":"report"}, {"from":"report","on":"success","to":"$end"}]
        })).unwrap();
        let graph = workflow_graph_vm(&app, &workflow);
        let edges: HashSet<_> = graph
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert!(edges.contains(&("$start", "checks")));
        assert!(edges.contains(&("unit", "contract")));
        assert!(
            edges.contains(&("contract", "$join:checks"))
                && edges.contains(&("ui", "$join:checks"))
        );
        assert!(edges.contains(&("$join:checks", "report")));
        assert!(!edges.contains(&("contract", "ui")) && !edges.contains(&("checks", "report")));
        for node in &graph.nodes {
            if node.id.starts_with('$') {
                assert!(node.node_id.is_none() && node.attempt_id.is_none());
            }
        }
    }
}
