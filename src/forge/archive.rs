//! task 级 Spec 存档：`<task_dir>/forge/spec.json`（方案 §6 D2）。
//!
//! Spec 是 Forge 会话的唯一对话事实源（§3.1），每轮 Spec 变更后原子写
//! 入 task 目录；会话重开时从此处恢复中断前的图状态。存档是 task 的
//! authoring 辅助事实，task 删除时随目录一起清理，无独立生命周期。

use crate::forge::spec::{ForgeError, ForgeSpec};
use crate::forge::{FORGE_DIR_NAME, SPEC_ARCHIVE_FILE};
use camino::Utf8Path;
use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum TemplateBinding {
    Bound {
        template_id: String,
        instance_id: String,
        /// Missing in older instance-only checkpoints: readable, but copy-only.
        #[serde(default)]
        authoring_etag: Option<String>,
    },
    /// A pre-checkpoint archive has no proof of the original installed instance.
    Unverified,
}

impl From<&crate::app::WorkflowTemplate> for TemplateBinding {
    fn from(template: &crate::app::WorkflowTemplate) -> Self {
        Self::Bound {
            template_id: template.id.clone(),
            instance_id: template.workflow.id.clone(),
            authoring_etag: Some(authoring_etag(template)),
        }
    }
}

/// Template fields that `project::write_back` can replace. Bind to the actual source or
/// committed template, never to a submitted spec. Retained workers inherit live execution
/// settings; deliberately deleting a stage also deletes its slot. Registry metadata and
/// workflow-wide execution limits are not Forge authoring fields.
pub(crate) fn authoring_etag(template: &crate::app::WorkflowTemplate) -> String {
    use crate::domain::SessionMode;
    use crate::dsl::{EdgeOutcome, NodeDsl};
    use serde_json::json;

    let workflow = &template.workflow;
    let nodes: BTreeMap<_, _> = workflow
        .nodes
        .iter()
        .map(|node| {
            let owned = match node {
                NodeDsl::Worker(w) => json!([
                    "worker",
                    w.profile,
                    w.goal,
                    w.output,
                    w.success_condition,
                    w.manual_check.unwrap_or(false)
                ]),
                // Group membership, branch order and maxParallel are all authoring-owned.
                other => json!(other),
            };
            (node.id(), owned)
        })
        .collect();
    let mut edges: Vec<_> = workflow
        .edges
        .iter()
        .map(|edge| {
            let failure = edge.on == EdgeOutcome::Failure;
            // A surviving success edge inherits the editor's session policy; the Spec controls
            // failure jumps. An implicit new session and an explicit new session are equivalent.
            (
                &edge.from,
                failure,
                &edge.to,
                failure && edge.session == Some(SessionMode::Continue),
                &edge.new_round_entry,
            )
        })
        .collect();
    edges.sort_unstable();
    let mut owned = json!([template.name, workflow.entry, nodes, edges]);
    owned.sort_all_objects(); // Including arbitrary JSON schemas inside output contracts.
    let bytes = serde_json::to_vec(&owned).expect("JSON value is serializable");
    blake3::hash(&bytes).to_hex().to_string()
}

/// A checkpoint is one atomic replacement: spec, revision, role backfill and targets agree.
/// Targets are private authoring authority, never accepted from an IPC spec.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ForgeDraft {
    #[serde(flatten)]
    pub spec: ForgeSpec,
    pub targets: BTreeMap<String, TemplateBinding>,
}

pub fn spec_archive_path(task_dir: &Utf8Path) -> camino::Utf8PathBuf {
    task_dir.join(FORGE_DIR_NAME).join(SPEC_ARCHIVE_FILE)
}

pub(crate) fn load_draft(task_dir: &Utf8Path) -> Result<Option<ForgeDraft>, ForgeError> {
    let path = spec_archive_path(task_dir);
    match std::fs::read(path.as_std_path()) {
        Ok(data) => {
            #[derive(serde::Deserialize)]
            struct StoredDraft {
                #[serde(flatten)]
                spec: ForgeSpec,
                targets: Option<BTreeMap<String, TemplateBinding>>,
            }
            let StoredDraft { mut spec, targets } = serde_json::from_slice(&data)?;
            let targets = match targets {
                Some(targets) => targets,
                None => {
                    // A legacy workflow has no canonical ID. Its full-record fingerprint is
                    // used only to select that exact recovery source, never a library object.
                    // Refuse ambiguous duplicate records rather than assigning by position.
                    let mut targets = BTreeMap::new();
                    for workflow in &mut spec.workflows {
                        let digest = blake3::hash(&serde_json::to_vec(workflow)?);
                        let key = format!("recovered-{digest}");
                        if targets.contains_key(&key) {
                            return Err(ForgeError::InvalidSpec {
                                detail: "ambiguous legacy workflow records".into(),
                            });
                        }
                        workflow.id = key.clone();
                        for stage in &mut workflow.stages {
                            stage.profile_id = None;
                        }
                        targets.insert(key, TemplateBinding::Unverified);
                    }
                    targets
                }
            };
            Ok(Some(ForgeDraft { spec, targets }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ForgeError::InvalidSpec {
            detail: e.to_string(),
        }),
    }
}

pub(crate) fn checkpoint(task_dir: &Utf8Path, draft: &mut ForgeDraft) -> Result<(), ForgeError> {
    for workflow in &draft.spec.workflows {
        workflow.validate_groups()?;
    }
    draft.spec.revision =
        draft
            .spec
            .revision
            .checked_add(1)
            .ok_or_else(|| ForgeError::InvalidSpec {
                detail: "draft revision exhausted".into(),
            })?;
    crate::storage::write_json(&spec_archive_path(task_dir), draft).map_err(|e| {
        ForgeError::InvalidSpec {
            detail: e.to_string(),
        }
    })
}

pub fn load_spec_archive(task_dir: &Utf8Path) -> Result<Option<ForgeSpec>, ForgeError> {
    Ok(load_draft(task_dir)?.map(|d| d.spec))
}

#[cfg(test)]
fn save_spec_archive(task_dir: &Utf8Path, spec: &ForgeSpec) -> Result<(), ForgeError> {
    checkpoint(
        task_dir,
        &mut ForgeDraft {
            spec: spec.clone(),
            targets: Default::default(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::spec::{ForgeStage, ForgeWorkflow, OnFailure, StageOutputKind, StageRole};

    fn sample_spec() -> ForgeSpec {
        ForgeSpec {
            revision: 0,
            spec_version: crate::forge::SPEC_VERSION.to_string(),
            name: Some("样本".to_string()),
            intent: "意图".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                id: String::new(),
                parallel_groups: vec![],
                name: "样本".to_string(),
                stages: vec![ForgeStage {
                    id: Some("s1".to_string()),
                    role: StageRole::Dev,

                    role_definition: None,

                    profile_id: None,
                    goal: "<输入>a\n<产出>b\n<边界>c\n<验收>d".into(),
                    artifact: None,
                    output_kind: StageOutputKind::None,
                    schema_hint: None,
                    success_condition: None,
                    on_failure: OnFailure::Retry,
                    manual_check: false,
                }],
            }],
            open_questions: vec![],
        }
    }

    #[test]
    fn archive_round_trip_in_task_dir() {
        let temp = tempfile::tempdir().unwrap();
        let task_dir = camino::Utf8Path::from_path(temp.path()).unwrap();
        // 未写入时读为 None。
        assert!(load_spec_archive(task_dir).unwrap().is_none());
        let spec = sample_spec();
        save_spec_archive(task_dir, &spec).unwrap();
        let loaded = load_spec_archive(task_dir).unwrap().expect("archived");
        assert_eq!(loaded.intent, spec.intent);
        assert_eq!(loaded.workflows[0].stages[0].id.as_deref(), Some("s1"));
        // 覆写后读新值（每轮变更原子替换）。
        let mut revised = spec.clone();
        revised.intent = "新意图".to_string();
        save_spec_archive(task_dir, &revised).unwrap();
        let loaded = load_spec_archive(task_dir).unwrap().unwrap();
        assert_eq!(loaded.intent, "新意图");
    }

    #[test]
    fn corrupt_archive_reports_invalid_spec() {
        let temp = tempfile::tempdir().unwrap();
        let task_dir = camino::Utf8Path::from_path(temp.path()).unwrap();
        save_spec_archive(task_dir, &sample_spec()).unwrap();
        std::fs::write(spec_archive_path(task_dir).as_std_path(), "{ broken").unwrap();
        let error = load_spec_archive(task_dir).unwrap_err();
        assert_eq!(error.code(), "forge/invalid-spec");
    }
}
