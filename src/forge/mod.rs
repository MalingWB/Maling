//! 工作流铸造（WorkFlow Forge）：自然语言/skill → Forge Spec → WorkflowDsl。
//!
//! 模块划分（见实施方案 §6 第一阶段拆解）：
//! - [`spec`]：Spec 数据模型、收敛校验、结构化错误码；
//! - [`compiler`]：Spec → WorkflowDsl 确定性编译（无 LLM 参与）；
//! - [`project`]：WorkflowDsl → Spec 反投影（对话式修改路径）；
//! - [`interview`]：openQuestions 收敛判定与 Spec 增量修订合并；
//! - [`archive`]：task 级 Spec 存档（`<task_dir>/forge/spec.json`）读写。

pub mod archive;
pub mod compiler;
pub mod interview;
pub mod project;
pub mod spec;

pub use spec::{
    ForgeError, ForgeSpec, ForgeStage, ForgeWorkflow, GoalElement, OnFailure, StageOutputKind,
    StageRole, MAX_STAGES_PER_WORKFLOW, SPEC_VERSION,
};

/// Spec 存档在 task 目录下的相对路径（方案 §6 D2）。
pub const FORGE_DIR_NAME: &str = "forge";
pub const SPEC_ARCHIVE_FILE: &str = "spec.json";

#[cfg(test)]
mod tests;
