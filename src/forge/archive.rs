//! task 级 Spec 存档：`<task_dir>/forge/spec.json`（方案 §6 D2）。
//!
//! Spec 是 Forge 会话的唯一对话事实源（§3.1），每轮 Spec 变更后原子写
//! 入 task 目录；会话重开时从此处恢复中断前的图状态。存档是 task 的
//! authoring 辅助事实，task 删除时随目录一起清理，无独立生命周期。

use crate::forge::spec::{ForgeError, ForgeSpec};
use crate::forge::{FORGE_DIR_NAME, SPEC_ARCHIVE_FILE};
use camino::Utf8Path;

/// Spec 存档文件路径。
pub fn spec_archive_path(task_dir: &Utf8Path) -> camino::Utf8PathBuf {
    task_dir.join(FORGE_DIR_NAME).join(SPEC_ARCHIVE_FILE)
}

/// 原子写入 Spec 存档（每轮 Spec 变更后调用）。
pub fn save_spec_archive(task_dir: &Utf8Path, spec: &ForgeSpec) -> Result<(), ForgeError> {
    let dir = task_dir.join(FORGE_DIR_NAME);
    std::fs::create_dir_all(dir.as_std_path()).map_err(|error| ForgeError::InvalidSpec {
        detail: format!("failed to create forge archive directory: {error}"),
    })?;
    crate::storage::write_json(&spec_archive_path(task_dir), spec).map_err(|error| {
        ForgeError::InvalidSpec {
            detail: format!("failed to write spec archive: {error}"),
        }
    })
}

/// 读取 Spec 存档；不存在视为空（新建会话尚未产出 Spec）。
pub fn load_spec_archive(task_dir: &Utf8Path) -> Result<Option<ForgeSpec>, ForgeError> {
    let path = spec_archive_path(task_dir);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(path.as_std_path()).map_err(|error| ForgeError::InvalidSpec {
            detail: format!("failed to read spec archive: {error}"),
        })?;
    let spec: ForgeSpec = serde_json::from_str(&content)?;
    Ok(Some(spec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::spec::{ForgeStage, ForgeWorkflow, StageOutputKind, StageRole, OnFailure};

    fn sample_spec() -> ForgeSpec {
        ForgeSpec {
            spec_version: crate::forge::SPEC_VERSION.to_string(),
            name: Some("样本".to_string()),
            intent: "意图".to_string(),
            source_skills: vec![],
            workflows: vec![ForgeWorkflow {
                name: "样本".to_string(),
                stages: vec![ForgeStage {
                    id: Some("s1".to_string()),
                    role: StageRole::Dev,

                    role_definition: None,

                    profile_id: None,
                    goal: "<输入>a\n<产出>b\n<边界>c\n<验收>d".to_string(),
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
        std::fs::write(
            spec_archive_path(task_dir).as_std_path(),
            "{ broken",
        )
        .unwrap();
        let error = load_spec_archive(task_dir).unwrap_err();
        assert_eq!(error.code(), "forge/invalid-spec");
    }
}
