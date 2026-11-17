//! A conditional command over the existing Runtime lifecycle. Offers are
//! projections of canonical identities/revisions, never a second pending state.
use crate::dynamic_store::load_dynamic_graph;
use serde::{Deserialize, Serialize};

use super::intervention::{
    InterventionError, InterventionErrorCode, InterventionLocator, InterventionRequestIdentity,
};
use super::{App, RuntimeInterventionKind, RuntimeLifecycleEvent};
use crate::{
    domain::RunStatus,
    dynamic::{DynamicGraphState, DynamicNodeStatus, DynamicRunPhase, DynamicRunStatus},
    runtime::{NodeState, RunState, TaskState},
    storage::read_json,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeResumeKind {
    ContinueCurrentAttempt,
    RecoverCompletedAttempt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeResumeBoundary {
    RunExecution { revision: u64 },
    DynamicLeaf { revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResumeSourceIdentity {
    pub task_uuid: String,
    pub run_uuid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResumeOffer {
    pub locator: InterventionLocator,
    pub source_identity: RuntimeResumeSourceIdentity,
    pub kind: RuntimeResumeKind,
    pub boundary: RuntimeResumeBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeResumeBusyReason {
    Starting,
    PromptActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeResumeReadiness {
    Ready,
    Busy { reason: RuntimeResumeBusyReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResumeInspection {
    pub offer: RuntimeResumeOffer,
    pub readiness: RuntimeResumeReadiness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<crate::domain::PauseReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeResumeOutcome {
    Started,
    Recovered,
    Completed,
    LaunchFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResumeResult {
    pub kind: RuntimeResumeKind,
    pub revision: u64,
    pub outcome: RuntimeResumeOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<InterventionError>,
}

/// Scope-local admission evidence, shared only with the existing launch handshake.
/// It is not persistent state and is never reconstructed from a changed revision.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeResumeAdmission {
    pub offer: RuntimeResumeOffer,
    committed: std::sync::Arc<std::sync::Mutex<Option<u64>>>,
}
impl std::ops::Deref for RuntimeResumeAdmission {
    type Target = RuntimeResumeOffer;
    fn deref(&self) -> &Self::Target {
        &self.offer
    }
}
impl RuntimeResumeAdmission {
    pub(crate) fn new(offer: RuntimeResumeOffer) -> Self {
        Self {
            offer,
            committed: Default::default(),
        }
    }
    pub(crate) fn mark_committed(&self, revision: u64) {
        if let Ok(mut committed) = self.committed.lock() {
            *committed = Some(committed.unwrap_or(revision).max(revision));
        }
    }
    pub(crate) fn committed_revision(&self) -> Option<u64> {
        self.committed.lock().ok().and_then(|committed| *committed)
    }
}

#[derive(Default)]
pub struct RuntimeResumeInput {
    pub prompt_id: Option<String>,
    pub input: Option<crate::provider::ConversationPromptInput>,
    pub attachment_paths: Vec<String>,
    pub model_override: Option<String>,
    pub permission_mode_override: Option<String>,
}

fn unavailable(reason: &'static str) -> InterventionError {
    InterventionError::new(InterventionErrorCode::InterventionRuntimeStateMismatch)
        .with_detail("reason", reason)
}

fn read_failure(reason: &'static str, error: anyhow::Error) -> InterventionError {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    {
        return unavailable(reason);
    }
    InterventionError {
        retryable: true,
        ..InterventionError::new(InterventionErrorCode::InterventionStorageUnavailable)
            .with_detail("reason", reason)
    }
}

impl RuntimeResumeOffer {
    pub fn request(&self) -> InterventionRequestIdentity {
        InterventionRequestIdentity::RuntimeResume {
            source_identity: self.source_identity.clone(),
            resume_kind: self.kind,
            boundary: self.boundary,
        }
    }
    pub fn canonical_event_id(&self) -> String {
        // Serialize only stable decision inputs: readiness and display data are excluded.
        format!(
            "runtime-resume:{}",
            blake3::hash(&serde_json::to_vec(self).expect("resume offer serializes")).to_hex()
        )
    }
}

impl App {
    pub fn inspect_runtime_resume(
        &self,
        locator: InterventionLocator,
    ) -> Result<RuntimeResumeInspection, InterventionError> {
        let run: RunState = read_json(&self.paths.run_file(&locator.task_id, &locator.run_id))
            .map_err(|error| read_failure("run-unavailable", error))?;
        let graph = match (&locator.outer_node_id, &locator.outer_attempt_id) {
            (Some(node), Some(attempt)) => Some(
                load_dynamic_graph(
                    &self.paths.dynamic_graph_file(
                        &locator.task_id,
                        &locator.run_id,
                        &locator.round_id,
                        node,
                        attempt,
                    ),
                    &self.paths.repo_root,
                )
                .map_err(|error| read_failure("graph-unavailable", error))?,
            ),
            (None, None) => None,
            _ => {
                return Err(InterventionError::new(
                    InterventionErrorCode::InterventionLocatorInvalid,
                ));
            }
        };
        self.inspect_runtime_resume_with_state(locator, &run, graph.as_ref())
    }

    /// Reuse the canonical state already loaded by a conversation projection.
    pub fn inspect_runtime_resume_with_state(
        &self,
        locator: InterventionLocator,
        run: &RunState,
        graph: Option<&DynamicGraphState>,
    ) -> Result<RuntimeResumeInspection, InterventionError> {
        let (offer, pause_reason) = self.runtime_resume_source_from_state(&locator, run, graph)?;
        let dir = self.runtime_resume_attempt_dir(&locator);
        let starting = if let (Some(outer), Some(attempt)) =
            (&locator.outer_node_id, &locator.outer_attempt_id)
        {
            self.dynamic_resume_target_is_active(
                &locator.task_id,
                &locator.run_id,
                &locator.round_id,
                outer,
                attempt,
                &locator.node_id,
                &locator.attempt_id,
            )
            .map_err(|error| read_failure("lease-unavailable", error))?
        } else {
            super::orchestrator::fixed_run_continue_is_starting(
                self,
                &locator.task_id,
                &locator.run_id,
            )
        };
        let readiness = if starting {
            RuntimeResumeReadiness::Busy {
                reason: RuntimeResumeBusyReason::Starting,
            }
        } else if crate::acp::client::prompt_activity(&dir).is_some() {
            RuntimeResumeReadiness::Busy {
                reason: RuntimeResumeBusyReason::PromptActive,
            }
        } else {
            RuntimeResumeReadiness::Ready
        };
        Ok(RuntimeResumeInspection {
            offer,
            readiness,
            pause_reason,
        })
    }

    /// Called again under the existing attempt/graph commit lock before any
    /// candidate, Runtime write, completion reconciliation or provider dispatch.
    pub(crate) fn validate_runtime_resume_source(
        &self,
        expected: &RuntimeResumeOffer,
        run: &RunState,
        graph: Option<&DynamicGraphState>,
    ) -> Result<(), InterventionError> {
        let current = self.runtime_resume_offer_from_state(&expected.locator, run, graph)?;
        if current.source_identity != expected.source_identity {
            return Err(unavailable("source-identity-changed"));
        }
        if current != *expected {
            return Err(InterventionError::new(
                InterventionErrorCode::InterventionRevisionConflict,
            )
            .with_detail("reason", "stale-boundary"));
        }
        if crate::acp::client::prompt_activity(&self.runtime_resume_attempt_dir(&expected.locator))
            .is_some()
        {
            return Err(InterventionError::busy());
        }
        Ok(())
    }

    pub(crate) fn runtime_resume_offer_from_state(
        &self,
        locator: &InterventionLocator,
        run: &RunState,
        graph: Option<&DynamicGraphState>,
    ) -> Result<RuntimeResumeOffer, InterventionError> {
        self.runtime_resume_source_from_state(locator, run, graph)
            .map(|(offer, _)| offer)
    }

    fn runtime_resume_source_from_state(
        &self,
        locator: &InterventionLocator,
        run: &RunState,
        graph: Option<&DynamicGraphState>,
    ) -> Result<(RuntimeResumeOffer, Option<crate::domain::PauseReason>), InterventionError> {
        if locator.project_id != self.paths.project_id
            || locator.task_id != run.task_id
            || locator.run_id != run.id
            || run.is_sealed()
            || run.outcome.is_some()
        {
            return Err(unavailable("owner-mismatch"));
        }
        let task: TaskState = read_json(&self.paths.task_file(&locator.task_id))
            .map_err(|error| read_failure("task-unavailable", error))?;
        let task_uuid = task
            .uuid
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| unavailable("source-identity-missing"))?;
        let run_uuid = run
            .uuid
            .clone()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| unavailable("source-identity-missing"))?;
        if run.task_uuid.as_ref() != Some(&task_uuid) {
            return Err(unavailable("source-identity-changed"));
        }
        // Read authoring mode only; no executable preflight or history hydration.
        let metadata: serde_json::Value = read_json(
            &self
                .paths
                .task_dir(&locator.task_id)
                .join("authoring/conversation.json"),
        )
        .map_err(|error| read_failure("conversation-unavailable", error))?;
        if !matches!(
            metadata.get("runMode").and_then(serde_json::Value::as_str),
            Some("workflow" | "auto")
        ) {
            return Err(unavailable("not-orchestrated"));
        }
        let owner_node = locator.outer_node_id.as_deref().unwrap_or(&locator.node_id);
        let owner_attempt = locator
            .outer_attempt_id
            .as_deref()
            .unwrap_or(&locator.attempt_id);
        if run.current_round.as_deref() != Some(&locator.round_id)
            || run.current_node.as_deref() != Some(owner_node)
            || run.current_attempt.as_deref() != Some(owner_attempt)
        {
            return Err(unavailable("owner-mismatch"));
        }
        let (kind, boundary, pause_reason) = if let Some(graph) = graph {
            if !(super::is_run_continuable(run) || run.status == RunStatus::Running)
                || graph.run.phase != DynamicRunPhase::Executing
                || graph.run.status == DynamicRunStatus::Completed
                || (graph.run.status == DynamicRunStatus::Paused
                    && !graph
                        .run
                        .pause_reason
                        .is_some_and(crate::domain::PauseReason::allows_explicit_runtime_continue))
            {
                return Err(unavailable("dynamic-not-resumable"));
            }
            let node = graph
                .nodes
                .iter()
                .find(|node| node.id == locator.node_id)
                .ok_or_else(|| unavailable("leaf-missing"))?;
            if super::orchestrator::dynamic_attempt_id(node) != locator.attempt_id
                || node.status != DynamicNodeStatus::Paused
                || node.outcome.is_some()
                || !node
                    .pause_reason
                    .is_some_and(crate::domain::PauseReason::allows_explicit_runtime_continue)
            {
                return Err(unavailable("leaf-not-resumable"));
            }
            (
                RuntimeResumeKind::ContinueCurrentAttempt,
                RuntimeResumeBoundary::DynamicLeaf {
                    revision: node.runtime_lifecycle_revision,
                },
                node.pause_reason,
            )
        } else {
            if !super::is_run_continuable(run) {
                return Err(unavailable("run-not-resumable"));
            }
            let node: NodeState = read_json(&self.paths.node_file(
                &locator.task_id,
                &locator.run_id,
                &locator.round_id,
                &locator.node_id,
                &locator.attempt_id,
            ))
            .map_err(|error| read_failure("attempt-unavailable", error))?;
            if node.node_type == crate::domain::NodeType::AiDynamic {
                return Err(unavailable("dynamic-leaf-required"));
            }
            if node.manual_check_pending {
                return Err(unavailable("manual-check-pending"));
            }
            let kind = if super::is_completed_attempt_recoverable(run, &node) {
                RuntimeResumeKind::RecoverCompletedAttempt
            } else if node.status == RunStatus::Paused && node.outcome.is_none() {
                RuntimeResumeKind::ContinueCurrentAttempt
            } else {
                return Err(unavailable("attempt-not-resumable"));
            };
            (
                kind,
                RuntimeResumeBoundary::RunExecution {
                    revision: run.execution.revision,
                },
                run.pause_reason,
            )
        };
        // Session metadata is bounded independently of timeline/raw history.
        let snapshot_path = self
            .runtime_resume_attempt_dir(locator)
            .join("acp.snapshot.json");
        if snapshot_path.exists() {
            let metadata: serde_json::Value = read_json(&snapshot_path)
                .map_err(|error| read_failure("session-unavailable", error))?;
            if matches!(
                metadata
                    .pointer("/timing/waitReason")
                    .and_then(serde_json::Value::as_str),
                Some("permission" | "elicitation")
            ) {
                return Err(unavailable("interaction-pending"));
            }
            for (field, permission) in [
                ("pendingPermissionIds", true),
                ("pendingElicitationIds", false),
            ] {
                if let Some(ids) = metadata
                    .pointer(&format!("/timing/stateSnapshot/{field}"))
                    .and_then(serde_json::Value::as_array)
                {
                    for id in ids.iter().filter_map(serde_json::Value::as_str) {
                        let dir = self.runtime_resume_attempt_dir(locator);
                        let (pending, response) = if permission {
                            (
                                crate::acp::permission::pending_permission_file(&dir, id),
                                crate::acp::permission::permission_response_file(&dir, id),
                            )
                        } else {
                            (
                                crate::acp::elicitation::pending_elicitation_file(&dir, id),
                                crate::acp::elicitation::elicitation_response_file(&dir, id),
                            )
                        };
                        if pending.try_exists().map_err(|error| {
                            read_failure("interaction-unavailable", error.into())
                        })? {
                            if !response.try_exists().map_err(|error| {
                                read_failure("interaction-unavailable", error.into())
                            })? {
                                return Err(unavailable("interaction-pending"));
                            }
                            if permission {
                                let _: crate::acp::permission::PermissionResponseState =
                                    read_json(&response).map_err(|error| {
                                        read_failure("interaction-unavailable", error)
                                    })?;
                            } else {
                                let _: crate::acp::elicitation::ElicitationResponseState =
                                    read_json(&response).map_err(|error| {
                                        read_failure("interaction-unavailable", error)
                                    })?;
                            }
                        }
                    }
                }
            }
            if metadata
                .get("promptRetry")
                .is_some_and(|retry| !retry.is_null())
                && crate::acp::client::prompt_activity(&self.runtime_resume_attempt_dir(locator))
                    .is_some()
            {
                return Err(unavailable("automatic-retry-active"));
            }
        }
        Ok((
            RuntimeResumeOffer {
                locator: locator.clone(),
                source_identity: RuntimeResumeSourceIdentity {
                    task_uuid,
                    run_uuid,
                },
                kind,
                boundary,
            },
            pause_reason,
        ))
    }

    pub fn execute_runtime_resume(
        &self,
        offer: RuntimeResumeOffer,
        input: RuntimeResumeInput,
    ) -> Result<(RuntimeResumeResult, RunState), InterventionError> {
        let inspected = self.inspect_runtime_resume(offer.locator.clone())?;
        if inspected.offer.source_identity != offer.source_identity {
            return Err(unavailable("source-identity-changed"));
        }
        if inspected.offer != offer {
            return Err(InterventionError::new(
                InterventionErrorCode::InterventionRevisionConflict,
            )
            .with_detail("reason", "stale-boundary"));
        }
        if inspected.readiness != RuntimeResumeReadiness::Ready {
            return Err(InterventionError::busy());
        }
        let admission = RuntimeResumeAdmission::new(offer.clone());
        let l = &offer.locator;
        let result = match (&l.outer_node_id, &l.outer_attempt_id, offer.kind) {
            (Some(outer), Some(attempt), RuntimeResumeKind::ContinueCurrentAttempt) => {
                super::orchestrator::run_continue_dynamic_inner_background_checked(
                    self,
                    &l.task_id,
                    &l.run_id,
                    &l.round_id,
                    outer,
                    attempt,
                    &l.node_id,
                    &l.attempt_id,
                    input.prompt_id,
                    input.input,
                    input.attachment_paths,
                    input.model_override,
                    input.permission_mode_override,
                    Some(admission.clone()),
                )
            }
            (None, None, RuntimeResumeKind::ContinueCurrentAttempt) => {
                super::orchestrator::run_continue_background_checked(
                    self,
                    &l.task_id,
                    &l.run_id,
                    input.prompt_id,
                    input.input,
                    input.attachment_paths,
                    input.model_override,
                    input.permission_mode_override,
                    Some(&admission),
                )
            }
            (None, None, RuntimeResumeKind::RecoverCompletedAttempt) => {
                super::orchestrator::run_recover_completed_background_checked(
                    self,
                    &l.task_id,
                    &l.run_id,
                    &l.round_id,
                    &l.node_id,
                    &l.attempt_id,
                    match offer.boundary {
                        RuntimeResumeBoundary::RunExecution { revision } => revision,
                        _ => return Err(unavailable("boundary-invalid")),
                    },
                    Some(&admission),
                )
            }
            _ => return Err(unavailable("action-invalid")),
        };
        let (run, outcome, error) = match result {
            Ok(run) => {
                let outcome = if run.status == RunStatus::Completed {
                    RuntimeResumeOutcome::Completed
                } else if offer.kind == RuntimeResumeKind::RecoverCompletedAttempt {
                    RuntimeResumeOutcome::Recovered
                } else {
                    RuntimeResumeOutcome::Started
                };
                (run, outcome, None)
            }
            Err(error) => {
                if admission.committed_revision().is_none() {
                    if let Some(error) = error.downcast_ref::<InterventionError>() {
                        return Err(error.clone());
                    }
                }
                let run = self
                    .run_status(&l.task_id, &l.run_id)
                    .map_err(|_| unavailable("run-unavailable"))?;
                if admission.committed_revision().is_none() {
                    let mut failure = unavailable("preflight-failed");
                    failure.retryable = true;
                    failure.details.insert(
                        "causeCode".into(),
                        crate::runtime_error::normalize_runtime_error(&error)
                            .code_str()
                            .to_string(),
                    );
                    return Err(failure);
                }
                (
                    run,
                    RuntimeResumeOutcome::LaunchFailed,
                    Some(unavailable("launch-failed")),
                )
            }
        };
        let revision = admission
            .committed_revision()
            .ok_or_else(|| unavailable("admission-unconfirmed"))?;
        Ok((
            RuntimeResumeResult {
                kind: offer.kind,
                revision,
                outcome,
                error,
            },
            run,
        ))
    }

    pub(crate) fn runtime_resume_attempt_dir(
        &self,
        l: &InterventionLocator,
    ) -> camino::Utf8PathBuf {
        match (&l.outer_node_id, &l.outer_attempt_id) {
            (Some(node), Some(attempt)) => self.paths.dynamic_node_attempt_dir(
                &l.task_id,
                &l.run_id,
                &l.round_id,
                node,
                attempt,
                &l.node_id,
                &l.attempt_id,
            ),
            _ => self.paths.attempt_dir(
                &l.task_id,
                &l.run_id,
                &l.round_id,
                &l.node_id,
                &l.attempt_id,
            ),
        }
    }

    pub(crate) fn runtime_resume_offer_for_run(
        &self,
        run: &RunState,
    ) -> Option<RuntimeResumeOffer> {
        let locator = InterventionLocator {
            project_id: self.paths.project_id.clone(),
            task_id: run.task_id.clone(),
            run_id: run.id.clone(),
            round_id: run.current_round.clone()?,
            node_id: run.current_node.clone()?,
            attempt_id: run.current_attempt.clone()?,
            outer_node_id: None,
            outer_attempt_id: None,
        };
        self.runtime_resume_offer_from_state(&locator, run, None)
            .ok()
    }

    pub(crate) fn publish_dynamic_runtime_resume_requests(
        &self,
        run: &RunState,
        graph: &DynamicGraphState,
        node_ids: &[String],
    ) -> Vec<RuntimeResumeOffer> {
        let mut offers = Vec::new();
        let changed: std::collections::HashSet<&str> =
            node_ids.iter().map(String::as_str).collect();
        for node in graph
            .nodes
            .iter()
            .filter(|node| changed.contains(node.id.as_str()))
        {
            let locator = InterventionLocator {
                project_id: self.paths.project_id.clone(),
                task_id: run.task_id.clone(),
                run_id: run.id.clone(),
                round_id: graph.run.parent_round_id.clone(),
                node_id: node.id.clone(),
                attempt_id: super::orchestrator::dynamic_attempt_id(node),
                outer_node_id: Some(graph.run.parent_node_id.clone()),
                outer_attempt_id: Some(graph.run.parent_attempt_id.clone()),
            };
            if let (Ok(offer), Some(occurred_at)) = (
                self.runtime_resume_offer_from_state(&locator, run, Some(graph)),
                node.runtime_lifecycle_updated_at.clone(),
            ) {
                self.emit_runtime_resume_offer(&offer, occurred_at);
                offers.push(offer);
            }
        }
        offers
    }

    pub fn publish_runtime_resume_offer(&self, offer: &RuntimeResumeOffer) {
        self.publish_runtime_resume_offers(std::slice::from_ref(offer));
    }

    /// Recheck one pause's offers against one current Run/Graph snapshot.
    pub(crate) fn publish_runtime_resume_offers(&self, offers: &[RuntimeResumeOffer]) {
        let Some(offer) = offers.first() else { return };
        let l = &offer.locator;
        let Ok(run) = self.run_status(&l.task_id, &l.run_id) else {
            return;
        };
        let graph = match (&l.outer_node_id, &l.outer_attempt_id) {
            (Some(node), Some(attempt)) => match load_dynamic_graph(
                &self
                    .paths
                    .dynamic_graph_file(&l.task_id, &l.run_id, &l.round_id, node, attempt),
                &self.paths.repo_root,
            ) {
                Ok(graph) => Some(graph),
                Err(_) => return,
            },
            (None, None) => None,
            _ => return,
        };
        for offer in offers {
            let target = &offer.locator;
            if target.project_id != l.project_id
                || target.task_id != l.task_id
                || target.run_id != l.run_id
                || target.round_id != l.round_id
                || target.outer_node_id != l.outer_node_id
                || target.outer_attempt_id != l.outer_attempt_id
                || self
                    .runtime_resume_offer_from_state(target, &run, graph.as_ref())
                    .as_ref()
                    != Ok(offer)
            {
                continue;
            }
            let occurred_at = graph
                .as_ref()
                .and_then(|graph| {
                    graph
                        .nodes
                        .iter()
                        .find(|node| node.id == target.node_id)
                        .and_then(|node| node.runtime_lifecycle_updated_at.clone())
                })
                .or_else(|| graph.is_none().then(|| run.updated_at.clone()));
            if let Some(occurred_at) = occurred_at {
                self.emit_runtime_resume_offer(offer, occurred_at);
            }
        }
    }

    pub(crate) fn emit_runtime_resume_offer(
        &self,
        offer: &RuntimeResumeOffer,
        occurred_at: String,
    ) {
        let l = &offer.locator;
        self.emit_lifecycle_event(RuntimeLifecycleEvent::InterventionRequested {
            event_id: offer.canonical_event_id(),
            occurred_at,
            scheduled_occurrence_id: None,
            project_id: l.project_id.clone(),
            task_id: l.task_id.clone(),
            task_uuid: Some(offer.source_identity.task_uuid.clone()),
            run_id: l.run_id.clone(),
            round_id: l.round_id.clone(),
            node_id: l.node_id.clone(),
            attempt_id: l.attempt_id.clone(),
            outer_node_id: l.outer_node_id.clone(),
            outer_attempt_id: l.outer_attempt_id.clone(),
            request: offer.request(),
            node_label: l.node_id.clone(),
            kind: RuntimeInterventionKind::RuntimeResumeRequired,
            task_title: None,
        });
    }
}
