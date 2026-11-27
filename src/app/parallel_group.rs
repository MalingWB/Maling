//! Static graph policy. Branches are ordinary, frozen child workflows; the outer
//! graph owns admission, workspace identity and the single all-results barrier.
use super::*;
use crate::dsl::{DynamicControlDsl, ParallelGroupNode};

pub(super) fn create_graph(
    ctx: &DynamicExecutionContext<'_>,
    group: &ParallelGroupNode,
    workflow: &ValidatedWorkflow,
    mut main: WorkspaceState,
) -> Result<DynamicGraphState> {
    ensure_repo_gold_band_root_locally_excluded(ctx.app)?;
    let repository = GitRepositoryService::default();
    if !repository.is_clean(&main.path)? {
        return Err(runtime_error(manual_runtime_error_info(
            RuntimeErrorDomain::Workspace,
            "parallel.source-dirty",
            "static branches require a committed source version",
            serde_json::json!({"workspace": main.path, "groupId":group.id}),
        )));
    }
    let checkout = repository.checkout_head(&main.path)?;
    main.fork_commit = checkout.commit;
    main.branch = Some(checkout.reference);
    let now = main.created_at.clone();
    let mut snapshots = Vec::with_capacity(group.branches.len());
    let mut nodes = Vec::with_capacity(group.branches.len());
    for branch in &group.branches {
        let workflow = branch.workflow(workflow)?;
        // Path components are runtime-owned; branch IDs may be Unicode or compare
        // equal on a case-insensitive filesystem. Preserve the business ID in workflow_id.
        let id = format!("branch-{}", blake3::hash(branch.id.as_bytes()).to_hex());
        snapshots.push(AllowedWorkflowSnapshot {
            workflow_id: branch.id.clone(),
            snapshot_id: id.clone(),
            name: branch.id.clone(),
            contains_ai_dynamic: false,
            workflow,
        });
        nodes.push(DynamicNodeState {
            version: VERSION.into(),
            acp_storage_schema_version: crate::runtime::CURRENT_ACP_STORAGE_SCHEMA_VERSION,
            id: id.clone(),
            dynamic_run_id: main.dynamic_run_id.clone(),
            kind: DynamicNodeKind::WorkflowInvocation,
            title: branch.id.clone(),
            task: branch.id.clone(),
            status: DynamicNodeStatus::Pending,
            outcome: None,
            pause_reason: None,
            runtime_error: None,
            runtime_execution_id: None,
            runtime_execution_phase: None,
            runtime_lifecycle_revision: 0,
            runtime_lifecycle_updated_at: None,
            group_id: None,
            chain_id: id.clone(),
            depth: 0,
            depends_on: Vec::new(),
            workspace_id: main.id.clone(),
            provider: None,
            profile: None,
            permission_mode: None,
            auto_accept: false,
            model: None,
            session_mode: SessionMode::New,
            continue_from_node_id: None,
            workflow_id: Some(branch.id.clone()),
            workflow_snapshot_id: Some(id),
            child_run_id: None,
            started_at: None,
            finished_at: None,
            uuid: Some(generate_uuid()),
        });
    }
    Ok(DynamicGraphState {
        version: CURRENT_DYNAMIC_GRAPH_VERSION.into(),
        run: DynamicRunState {
            version: VERSION.into(),
            id: main.dynamic_run_id.clone(),
            parent_run_id: ctx.run_id.into(),
            parent_round_id: ctx.round_id.into(),
            parent_node_id: ctx.outer_node_id.into(),
            parent_attempt_id: ctx.outer_attempt_id.into(),
            status: DynamicRunStatus::Running,
            phase: DynamicRunPhase::Executing,
            outcome: None,
            pause_reason: None,
            started_at: now.clone(),
            updated_at: now,
            control: DynamicControlDsl {
                max_parallel: group.max_parallel,
                ..Default::default()
            },
            allowed_workflow_snapshots: snapshots,
            current_node_ids: Vec::new(),
        },
        nodes,
        groups: Vec::new(),
        workspaces: vec![main],
        proposals: Vec::new(),
    })
}

pub(super) fn validate_members(group: &ParallelGroupNode, graph: &DynamicGraphState) -> Result<()> {
    ensure!(
        graph.nodes.len() == group.branches.len()
            && graph.run.allowed_workflow_snapshots.len() == group.branches.len()
            && graph.groups.is_empty(),
        "static-group membership changed"
    );
    for ((branch, node), snapshot) in group
        .branches
        .iter()
        .zip(&graph.nodes)
        .zip(&graph.run.allowed_workflow_snapshots)
    {
        ensure!(
            node.kind == DynamicNodeKind::WorkflowInvocation
                && node.workflow_id.as_deref() == Some(&branch.id)
                && snapshot.workflow_id == branch.id
                && node.workflow_snapshot_id.as_deref() == Some(&snapshot.snapshot_id),
            "static-group branch identity changed"
        );
    }
    Ok(())
}

pub(super) fn ensure_workspace(
    ctx: &DynamicExecutionContext<'_>,
    graph: &mut DynamicGraphState,
    index: usize,
) -> Result<()> {
    let node = &graph.nodes[index];
    let parent = dynamic_workspace(graph, &node.workspace_id)?;
    if parent.kind == WorkspaceKind::Worktree {
        return Ok(());
    }
    let (id, parent_id, fork) = (
        node.id.clone(),
        parent.id.clone(),
        parent.fork_commit.clone(),
    );
    with_dynamic_workspace_transition(ctx, graph, std::slice::from_ref(&id), |graph| {
        let ws = fork_dynamic_workspace_from_commit(
            ctx,
            graph,
            &parent_id,
            ctx.outer_node_id,
            &id,
            &fork,
        )?;
        graph.nodes[index].workspace_id = ws;
        Ok(())
    })
}

pub(super) fn join(ctx: &DynamicExecutionContext<'_>, graph: &mut DynamicGraphState) -> Result<()> {
    let success = graph
        .nodes
        .iter()
        .all(|n| n.outcome == Some(NodeOutcome::Success));
    if success {
        let owners = graph.nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        with_dynamic_workspace_transition(ctx, graph, &owners, |graph| integrate(ctx, graph))?;
    }
    let branches = graph.nodes.iter().map(|node| -> Result<_> {
        let child_id = node.child_run_id.as_deref().context("terminal branch missing child run")?;
        let child = ctx.app.run_status(ctx.task_id, child_id)?;
        let artifact = child.current_node.as_deref().zip(child.current_round.as_deref()).zip(child.current_attempt.as_deref()).and_then(|((stage, round), attempt)| {
            let snapshot = graph.run.allowed_workflow_snapshots.iter().find(|w| Some(&w.workflow_id) == node.workflow_id.as_ref())?;
            let NodeDsl::Worker(worker) = snapshot.workflow.nodes.iter().find(|n| n.id() == stage)? else { return None };
            let output = worker.output.as_ref()?;
            let path=ctx.app.paths.artifact_file(ctx.task_id, child_id, round, stage, attempt, &output.artifact);
            path.is_file().then_some(path)
        });
        Ok(serde_json::json!({"branchId":node.workflow_id,"childRunId":child_id,"outcome":node.outcome,"terminalStage":child.current_node,"outputArtifact":artifact}))
    }).collect::<Result<Vec<_>>>()?;
    write_json(
        &ctx.app.paths.artifact_file(
            ctx.task_id,
            ctx.run_id,
            ctx.round_id,
            ctx.outer_node_id,
            ctx.outer_attempt_id,
            crate::dsl::PARALLEL_GROUP_RESULT,
        ),
        &serde_json::json!({"groupId":ctx.outer_node_id,"branches":branches}),
    )?;
    graph.run.status = DynamicRunStatus::Completed;
    graph.run.outcome = Some(if success {
        RunOutcome::Success
    } else {
        RunOutcome::Failure
    });
    graph.run.updated_at = now_rfc3339_like();
    refresh_dynamic_current_leaf_ids(graph);
    // The barrier is durable before any cleanup. Re-entry reuses the terminal outcome.
    persist_dynamic_graph(ctx, graph)?;
    if success {
        release(ctx, graph)?;
    }
    emit_dynamic_run_terminal_session_update_best_effort(ctx, graph);
    Ok(())
}

pub(super) fn release(
    ctx: &DynamicExecutionContext<'_>,
    graph: &mut DynamicGraphState,
) -> Result<()> {
    let ids = graph
        .workspaces
        .iter()
        .filter(|w| {
            w.ownership == WorkspaceOwnership::Runtime && w.status != WorkspaceStatus::Released
        })
        .map(|w| w.id.clone())
        .collect::<Vec<_>>();
    for id in ids {
        if release_dynamic_workspace_best_effort(ctx, graph, &id) {
            persist_dynamic_graph(ctx, graph)?;
        }
    }
    Ok(())
}

pub(super) fn owned_workspace(
    app: &App,
    task: &str,
    child: &str,
    record: &RunWorktreeState,
    owner: &crate::runtime::BranchWorkspaceOwner,
) -> Result<Utf8PathBuf> {
    for id in [
        &owner.run_id,
        &owner.round_id,
        &owner.group_id,
        &owner.group_attempt_id,
        &owner.branch_node_id,
    ] {
        ensure!(
            crate::dsl::is_portable_component(id),
            "invalid branch workspace locator"
        );
    }
    let mut project = app.clone_for_background();
    project.paths.repo_root = project
        .paths
        .repo_gold_band_root
        .parent()
        .context("missing project workspace root")?
        .to_owned();
    let ctx = DynamicExecutionContext {
        app: &project,
        task_id: task,
        run_id: &owner.run_id,
        round_id: &owner.round_id,
        outer_node_id: &owner.group_id,
        outer_attempt_id: &owner.group_attempt_id,
        outer_runtime_execution_id: None,
        outer_new_round_trigger: None,
        ai: None,
        task_uuid: None,
        run_uuid: None,
        round_uuid: None,
        outer_node_uuid: None,
        parent_continue_input: None,
        parent_continue_prompt_id: None,
        resume_override: None,
    };
    let graph = load_dynamic_graph(
        &project.paths.dynamic_graph_file(
            task,
            &owner.run_id,
            &owner.round_id,
            &owner.group_id,
            &owner.group_attempt_id,
        ),
        &project.paths.repo_root,
    )?;
    let node = graph
        .nodes
        .iter()
        .find(|n| n.id == owner.branch_node_id && n.child_run_id.as_deref() == Some(child))
        .context("child run does not own this branch")?;
    let ws = dynamic_workspace(&graph, &node.workspace_id)?;
    ensure!(
        ws.kind == WorkspaceKind::Worktree
            && ws.branch.as_deref() == Some(&record.branch)
            && ws.fork_commit == record.fork_commit
            && git_filesystem_paths_equal(&record.path, &ws.path)?
            && git_filesystem_paths_equal(&ws.repo_root, &project.paths.repo_root)?,
        "branch workspace identity changed"
    );
    preflight_dynamic_worktree_namespace(&ctx, &ws.id, &record.path)?;
    ensure_dynamic_workspace(&graph, node)
}

fn integrate(ctx: &DynamicExecutionContext<'_>, graph: &mut DynamicGraphState) -> Result<()> {
    let main = graph
        .workspaces
        .iter()
        .find(|w| w.kind == WorkspaceKind::Main)
        .context("static group main workspace missing")?
        .clone();
    let target = if let Some(target) = main.checkpoint_commit {
        target
    } else {
        let branches = graph
            .nodes
            .iter()
            .map(|n| n.workspace_id.clone())
            .collect::<Vec<_>>();
        let mut commits = Vec::with_capacity(branches.len());
        for id in branches {
            commits.push(checkpoint_dynamic_workspace(
                graph,
                &id,
                Some(ctx.outer_node_id),
            )?);
        }
        let target = GitWorkspaceManager::default().compose_branch_checkpoints(
            &main.path,
            &main.fork_commit,
            &commits,
        )?;
        // Record the intended ref update before touching the target checkout.
        dynamic_workspace_mut(graph, &main.id)?.checkpoint_commit = Some(target.clone());
        persist_dynamic_graph(ctx, graph)?;
        target
    };
    GitWorkspaceManager::default().integrate_checkpoint(
        &main.path,
        main.branch.as_deref(),
        &main.fork_commit,
        &target,
    )
}
