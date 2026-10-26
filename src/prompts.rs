use anyhow::{Result, anyhow};
use minijinja::{Environment, UndefinedBehavior};
use serde::{Deserialize, Serialize};

use crate::config::DesktopLanguage;

#[derive(Debug, Clone, Copy)]
pub struct LocalizedText {
    pub zh_cn: &'static str,
    pub en: &'static str,
    pub zh_tw: Option<&'static str>,
    pub ja_jp: Option<&'static str>,
    pub ko_kr: Option<&'static str>,
    pub pt_br: Option<&'static str>,
    pub es: Option<&'static str>,
}

impl LocalizedText {
    pub const fn all(
        zh_cn: &'static str,
        zh_tw: &'static str,
        en: &'static str,
        ja_jp: &'static str,
        ko_kr: &'static str,
        pt_br: &'static str,
        es: &'static str,
    ) -> Self {
        Self {
            zh_cn,
            en,
            zh_tw: Some(zh_tw),
            ja_jp: Some(ja_jp),
            ko_kr: Some(ko_kr),
            pt_br: Some(pt_br),
            es: Some(es),
        }
    }

    pub const fn zh_en(zh_cn: &'static str, en: &'static str) -> Self {
        Self {
            zh_cn,
            en,
            zh_tw: None,
            ja_jp: None,
            ko_kr: None,
            pt_br: None,
            es: None,
        }
    }

    pub fn resolve(self, language: DesktopLanguage) -> &'static str {
        match language {
            DesktopLanguage::ZhCn => self.zh_cn,
            DesktopLanguage::En => self.en,
            DesktopLanguage::ZhTw => self.zh_tw.unwrap_or(self.en),
            DesktopLanguage::JaJp => self.ja_jp.unwrap_or(self.en),
            DesktopLanguage::KoKr => self.ko_kr.unwrap_or(self.en),
            DesktopLanguage::PtBr => self.pt_br.unwrap_or(self.en),
            DesktopLanguage::Es => self.es.unwrap_or(self.en),
        }
    }
}

macro_rules! localized_prompt {
    ($path:literal) => {
        LocalizedText::all(
            include_str!(concat!("prompts/zh-CN/", $path)),
            include_str!(concat!("prompts/zh-TW/", $path)),
            include_str!(concat!("prompts/en/", $path)),
            include_str!(concat!("prompts/ja-JP/", $path)),
            include_str!(concat!("prompts/ko-KR/", $path)),
            include_str!(concat!("prompts/pt-BR/", $path)),
            include_str!(concat!("prompts/es/", $path)),
        )
    };
}

macro_rules! localized_prompt_zh_en {
    ($path:literal) => {
        LocalizedText::zh_en(
            include_str!(concat!("prompts/zh-CN/", $path)),
            include_str!(concat!("prompts/en/", $path)),
        )
    };
}

pub const PROFILE_PLAN: LocalizedText = localized_prompt!("profile/plan.md");
pub const PROFILE_DEV: LocalizedText = localized_prompt!("profile/dev.md");
pub const PROFILE_DEV_TEST: LocalizedText = localized_prompt!("profile/dev-test.md");
pub const PROFILE_REVIEW: LocalizedText = localized_prompt!("profile/review.md");
pub const PROFILE_TEST: LocalizedText = localized_prompt!("profile/test.md");
pub const PROFILE_CICD: LocalizedText = localized_prompt_zh_en!("profile/cicd.md");
pub const PROFILE_ACCEPT: LocalizedText = localized_prompt!("profile/accept.md");
pub const PROFILE_CLEAN: LocalizedText = localized_prompt!("profile/clean.md");
pub const PROFILE_INTERVIEW: LocalizedText = localized_prompt!("profile/interview.md");
pub const PROFILE_GRILLME: LocalizedText = localized_prompt!("profile/GrillMe.md");
pub const PROFILE_OVERLAY_REQUIREMENT_IDENTITY: LocalizedText =
    localized_prompt!("profile/overlays/requirement-identity.md");
pub const PROFILE_OVERLAY_DEV_TEST_AUTO_COMMIT: LocalizedText =
    localized_prompt!("profile/overlays/dev-test-auto-commit.md");
pub const RUNTIME_SYSTEM: LocalizedText = localized_prompt!("runtime/system.md");
pub const RUNTIME_HIDDEN_CONTEXT: LocalizedText = localized_prompt!("runtime/hidden_context.md");
pub const RUNTIME_USER: LocalizedText = localized_prompt!("runtime/user.md");
pub const RUNTIME_INVALID_OUTPUT_REPAIR: LocalizedText =
    localized_prompt!("runtime/invalid_output_repair.md");
pub const RUNTIME_SCHEDULED_TASK_CONTEXT: LocalizedText =
    localized_prompt!("runtime/scheduled_task_context.md");
pub const RUNTIME_REMOTE_TASK_CONTEXT: LocalizedText =
    localized_prompt_zh_en!("runtime/remote_task_context.md");
pub const RUNTIME_REMOTE_TASK_PARENT_OUTPUT: LocalizedText =
    localized_prompt_zh_en!("runtime/remote_task_parent_output.md");
pub const RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL: LocalizedText =
    localized_prompt_zh_en!("runtime/remote_task_completion_protocol.md");
pub const RUNTIME_REMOTE_TASK_NODE_PROTOCOL: LocalizedText =
    localized_prompt_zh_en!("runtime/remote_task_node_protocol.md");
pub const RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES: LocalizedText =
    localized_prompt_zh_en!("runtime/remote_task_attachment_failures.md");
pub const RUNTIME_REMOTE_TASK_UPSTREAM_PULL: LocalizedText =
    localized_prompt_zh_en!("runtime/remote_task_upstream_pull.md");
pub const RUNTIME_ARTIFACT_FINALIZE: LocalizedText =
    localized_prompt!("runtime/artifact_finalize.md");
pub const RUNTIME_CONTROL_RESUME: LocalizedText =
    localized_prompt!("runtime/runtime_control_resume.md");
pub const RUNTIME_CONTROL_RESUME_WITH_MESSAGE: LocalizedText =
    localized_prompt!("runtime/runtime_control_resume_with_message.md");
pub const RUNTIME_WORKFLOW_RESUME: LocalizedText = localized_prompt!("runtime/workflow_resume.md");
pub const RUNTIME_USER_ROLE_MESSAGE: LocalizedText =
    localized_prompt!("runtime/user_role_message.md");
pub const RUNTIME_USER_QUOTE: LocalizedText = localized_prompt!("runtime/user_quote.md");
pub const RUNTIME_USER_FILES: LocalizedText = localized_prompt!("runtime/user_files.md");
pub const CICD_GOAL: LocalizedText = localized_prompt_zh_en!("runtime/cicd-goal.md");
pub const AI_DYNAMIC_PROPOSAL_REPAIR: LocalizedText =
    localized_prompt!("runtime/ai-dynamic/proposal_repair.md");
pub const AI_DYNAMIC_FANOUT: LocalizedText = localized_prompt!("runtime/ai-dynamic/fanout.md");
pub const AI_DYNAMIC_MERGE: LocalizedText = localized_prompt!("runtime/ai-dynamic/merge.md");
pub const AI_DYNAMIC_ACCEPTANCE: LocalizedText =
    localized_prompt!("runtime/ai-dynamic/acceptance.md");
pub const AI_DYNAMIC_NODE_TASK: LocalizedText =
    localized_prompt!("runtime/ai-dynamic/node_task.md");
pub const AI_DYNAMIC_HIDDEN_CONTEXT: LocalizedText =
    localized_prompt!("runtime/ai-dynamic/hidden_context.md");
pub const AI_DYNAMIC_WORKFLOW_INVOCATION: LocalizedText =
    localized_prompt!("runtime/ai-dynamic/workflow_invocation.md");
pub const AI_DYNAMIC_SYSTEM: LocalizedText = localized_prompt!("runtime/ai-dynamic/system.md");
pub const AI_DYNAMIC_OUTPUT_PROTOCOL: LocalizedText =
    localized_prompt!("runtime/ai-dynamic/output_protocol.md");
pub const PERSONAL_ANALYTICS_SYSTEM: LocalizedText =
    localized_prompt!("personal-analytics/system.md");
pub const PERSONAL_ANALYTICS_USER: LocalizedText = localized_prompt!("personal-analytics/user.md");
pub const PERSONAL_ANALYTICS_REPAIR_SYSTEM: LocalizedText =
    localized_prompt!("personal-analytics/repair_system.md");
pub const PERSONAL_ANALYTICS_REPAIR_USER: LocalizedText =
    localized_prompt!("personal-analytics/repair_user.md");
pub const MEMORY_RULES: LocalizedText = localized_prompt!("runtime/memory-rules.md");
pub const MEMORY_TOOLS: LocalizedText = localized_prompt!("runtime/memory-tools.json");

pub const PROFILE_PLAN_ZH_CN: &str = PROFILE_PLAN.zh_cn;
pub const PROFILE_PLAN_EN: &str = PROFILE_PLAN.en;
pub const PROFILE_DEV_ZH_CN: &str = PROFILE_DEV.zh_cn;
pub const PROFILE_DEV_EN: &str = PROFILE_DEV.en;
pub const PROFILE_DEV_TEST_ZH_CN: &str = PROFILE_DEV_TEST.zh_cn;
pub const PROFILE_DEV_TEST_EN: &str = PROFILE_DEV_TEST.en;
pub const PROFILE_REVIEW_ZH_CN: &str = PROFILE_REVIEW.zh_cn;
pub const PROFILE_REVIEW_EN: &str = PROFILE_REVIEW.en;
pub const PROFILE_TEST_ZH_CN: &str = PROFILE_TEST.zh_cn;
pub const PROFILE_TEST_EN: &str = PROFILE_TEST.en;
pub const PROFILE_CICD_ZH_CN: &str = PROFILE_CICD.zh_cn;
pub const PROFILE_CICD_EN: &str = PROFILE_CICD.en;
pub const PROFILE_ACCEPT_ZH_CN: &str = PROFILE_ACCEPT.zh_cn;
pub const PROFILE_ACCEPT_EN: &str = PROFILE_ACCEPT.en;
pub const PROFILE_CLEAN_ZH_CN: &str = PROFILE_CLEAN.zh_cn;
pub const PROFILE_CLEAN_EN: &str = PROFILE_CLEAN.en;
pub const PROFILE_INTERVIEW_ZH_CN: &str = PROFILE_INTERVIEW.zh_cn;
pub const PROFILE_INTERVIEW_EN: &str = PROFILE_INTERVIEW.en;
pub const PROFILE_GRILLME_ZH_CN: &str = PROFILE_GRILLME.zh_cn;
pub const PROFILE_GRILLME_EN: &str = PROFILE_GRILLME.en;
pub const PROFILE_OVERLAY_REQUIREMENT_IDENTITY_ZH_CN: &str =
    PROFILE_OVERLAY_REQUIREMENT_IDENTITY.zh_cn;
pub const PROFILE_OVERLAY_REQUIREMENT_IDENTITY_EN: &str = PROFILE_OVERLAY_REQUIREMENT_IDENTITY.en;
pub const PROFILE_OVERLAY_DEV_TEST_AUTO_COMMIT_ZH_CN: &str =
    PROFILE_OVERLAY_DEV_TEST_AUTO_COMMIT.zh_cn;
pub const PROFILE_OVERLAY_DEV_TEST_AUTO_COMMIT_EN: &str = PROFILE_OVERLAY_DEV_TEST_AUTO_COMMIT.en;
pub const RUNTIME_SYSTEM_ZH_CN: &str = RUNTIME_SYSTEM.zh_cn;
pub const RUNTIME_SYSTEM_EN: &str = RUNTIME_SYSTEM.en;
pub const RUNTIME_HIDDEN_CONTEXT_ZH_CN: &str = RUNTIME_HIDDEN_CONTEXT.zh_cn;
pub const RUNTIME_HIDDEN_CONTEXT_EN: &str = RUNTIME_HIDDEN_CONTEXT.en;
pub const RUNTIME_USER_ZH_CN: &str = RUNTIME_USER.zh_cn;
pub const RUNTIME_USER_EN: &str = RUNTIME_USER.en;
pub const RUNTIME_INVALID_OUTPUT_REPAIR_ZH_CN: &str = RUNTIME_INVALID_OUTPUT_REPAIR.zh_cn;
pub const RUNTIME_INVALID_OUTPUT_REPAIR_EN: &str = RUNTIME_INVALID_OUTPUT_REPAIR.en;
pub const RUNTIME_SCHEDULED_TASK_CONTEXT_ZH_CN: &str = RUNTIME_SCHEDULED_TASK_CONTEXT.zh_cn;
pub const RUNTIME_SCHEDULED_TASK_CONTEXT_EN: &str = RUNTIME_SCHEDULED_TASK_CONTEXT.en;
pub const RUNTIME_REMOTE_TASK_CONTEXT_ZH_CN: &str = RUNTIME_REMOTE_TASK_CONTEXT.zh_cn;
pub const RUNTIME_REMOTE_TASK_CONTEXT_EN: &str = RUNTIME_REMOTE_TASK_CONTEXT.en;
pub const RUNTIME_REMOTE_TASK_PARENT_OUTPUT_ZH_CN: &str = RUNTIME_REMOTE_TASK_PARENT_OUTPUT.zh_cn;
pub const RUNTIME_REMOTE_TASK_PARENT_OUTPUT_EN: &str = RUNTIME_REMOTE_TASK_PARENT_OUTPUT.en;
pub const RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL_ZH_CN: &str =
    RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL.zh_cn;
pub const RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL_EN: &str =
    RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL.en;
pub const RUNTIME_REMOTE_TASK_NODE_PROTOCOL_ZH_CN: &str =
    RUNTIME_REMOTE_TASK_NODE_PROTOCOL.zh_cn;
pub const RUNTIME_REMOTE_TASK_NODE_PROTOCOL_EN: &str = RUNTIME_REMOTE_TASK_NODE_PROTOCOL.en;
pub const RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES_ZH_CN: &str =
    RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES.zh_cn;
pub const RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES_EN: &str =
    RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES.en;
pub const RUNTIME_ARTIFACT_FINALIZE_ZH_CN: &str = RUNTIME_ARTIFACT_FINALIZE.zh_cn;
pub const RUNTIME_ARTIFACT_FINALIZE_EN: &str = RUNTIME_ARTIFACT_FINALIZE.en;
pub const RUNTIME_CONTROL_RESUME_ZH_CN: &str = RUNTIME_CONTROL_RESUME.zh_cn;
pub const RUNTIME_CONTROL_RESUME_EN: &str = RUNTIME_CONTROL_RESUME.en;
pub const RUNTIME_CONTROL_RESUME_WITH_MESSAGE_ZH_CN: &str =
    RUNTIME_CONTROL_RESUME_WITH_MESSAGE.zh_cn;
pub const RUNTIME_CONTROL_RESUME_WITH_MESSAGE_EN: &str = RUNTIME_CONTROL_RESUME_WITH_MESSAGE.en;
pub const RUNTIME_WORKFLOW_RESUME_ZH_CN: &str = RUNTIME_WORKFLOW_RESUME.zh_cn;
pub const RUNTIME_WORKFLOW_RESUME_EN: &str = RUNTIME_WORKFLOW_RESUME.en;
pub const RUNTIME_USER_ROLE_MESSAGE_ZH_CN: &str = RUNTIME_USER_ROLE_MESSAGE.zh_cn;
pub const RUNTIME_USER_ROLE_MESSAGE_EN: &str = RUNTIME_USER_ROLE_MESSAGE.en;
pub const AI_DYNAMIC_PROPOSAL_REPAIR_ZH_CN: &str = AI_DYNAMIC_PROPOSAL_REPAIR.zh_cn;
pub const AI_DYNAMIC_PROPOSAL_REPAIR_EN: &str = AI_DYNAMIC_PROPOSAL_REPAIR.en;
pub const AI_DYNAMIC_FANOUT_ZH_CN: &str = AI_DYNAMIC_FANOUT.zh_cn;
pub const AI_DYNAMIC_FANOUT_EN: &str = AI_DYNAMIC_FANOUT.en;
pub const AI_DYNAMIC_MERGE_ZH_CN: &str = AI_DYNAMIC_MERGE.zh_cn;
pub const AI_DYNAMIC_MERGE_EN: &str = AI_DYNAMIC_MERGE.en;
pub const AI_DYNAMIC_ACCEPTANCE_ZH_CN: &str = AI_DYNAMIC_ACCEPTANCE.zh_cn;
pub const AI_DYNAMIC_ACCEPTANCE_EN: &str = AI_DYNAMIC_ACCEPTANCE.en;
pub const AI_DYNAMIC_NODE_TASK_ZH_CN: &str = AI_DYNAMIC_NODE_TASK.zh_cn;
pub const AI_DYNAMIC_NODE_TASK_EN: &str = AI_DYNAMIC_NODE_TASK.en;
pub const AI_DYNAMIC_HIDDEN_CONTEXT_ZH_CN: &str = AI_DYNAMIC_HIDDEN_CONTEXT.zh_cn;
pub const AI_DYNAMIC_HIDDEN_CONTEXT_EN: &str = AI_DYNAMIC_HIDDEN_CONTEXT.en;
pub const AI_DYNAMIC_WORKFLOW_INVOCATION_ZH_CN: &str = AI_DYNAMIC_WORKFLOW_INVOCATION.zh_cn;
pub const AI_DYNAMIC_WORKFLOW_INVOCATION_EN: &str = AI_DYNAMIC_WORKFLOW_INVOCATION.en;
pub const AI_DYNAMIC_SYSTEM_ZH_CN: &str = AI_DYNAMIC_SYSTEM.zh_cn;
pub const AI_DYNAMIC_SYSTEM_EN: &str = AI_DYNAMIC_SYSTEM.en;
pub const AI_DYNAMIC_OUTPUT_PROTOCOL_ZH_CN: &str = AI_DYNAMIC_OUTPUT_PROTOCOL.zh_cn;
pub const AI_DYNAMIC_OUTPUT_PROTOCOL_EN: &str = AI_DYNAMIC_OUTPUT_PROTOCOL.en;
pub const PERSONAL_ANALYTICS_SYSTEM_ZH_CN: &str = PERSONAL_ANALYTICS_SYSTEM.zh_cn;
pub const PERSONAL_ANALYTICS_SYSTEM_EN: &str = PERSONAL_ANALYTICS_SYSTEM.en;
pub const PERSONAL_ANALYTICS_USER_ZH_CN: &str = PERSONAL_ANALYTICS_USER.zh_cn;
pub const PERSONAL_ANALYTICS_USER_EN: &str = PERSONAL_ANALYTICS_USER.en;
pub const PERSONAL_ANALYTICS_REPAIR_SYSTEM_ZH_CN: &str = PERSONAL_ANALYTICS_REPAIR_SYSTEM.zh_cn;
pub const PERSONAL_ANALYTICS_REPAIR_SYSTEM_EN: &str = PERSONAL_ANALYTICS_REPAIR_SYSTEM.en;
pub const PERSONAL_ANALYTICS_REPAIR_USER_ZH_CN: &str = PERSONAL_ANALYTICS_REPAIR_USER.zh_cn;
pub const PERSONAL_ANALYTICS_REPAIR_USER_EN: &str = PERSONAL_ANALYTICS_REPAIR_USER.en;

pub const BUNDLED_PROMPTS: &[(&str, LocalizedText)] = &[
    ("profile/plan.md", PROFILE_PLAN),
    ("profile/dev.md", PROFILE_DEV),
    ("profile/dev-test.md", PROFILE_DEV_TEST),
    ("profile/review.md", PROFILE_REVIEW),
    ("profile/test.md", PROFILE_TEST),
    ("profile/cicd.md", PROFILE_CICD),
    ("profile/accept.md", PROFILE_ACCEPT),
    ("profile/clean.md", PROFILE_CLEAN),
    ("profile/interview.md", PROFILE_INTERVIEW),
    ("profile/GrillMe.md", PROFILE_GRILLME),
    (
        "profile/overlays/requirement-identity.md",
        PROFILE_OVERLAY_REQUIREMENT_IDENTITY,
    ),
    (
        "profile/overlays/dev-test-auto-commit.md",
        PROFILE_OVERLAY_DEV_TEST_AUTO_COMMIT,
    ),
    ("runtime/system.md", RUNTIME_SYSTEM),
    ("runtime/hidden_context.md", RUNTIME_HIDDEN_CONTEXT),
    ("runtime/user.md", RUNTIME_USER),
    (
        "runtime/invalid_output_repair.md",
        RUNTIME_INVALID_OUTPUT_REPAIR,
    ),
    (
        "runtime/scheduled_task_context.md",
        RUNTIME_SCHEDULED_TASK_CONTEXT,
    ),
    (
        "runtime/remote_task_context.md",
        RUNTIME_REMOTE_TASK_CONTEXT,
    ),
    (
        "runtime/remote_task_parent_output.md",
        RUNTIME_REMOTE_TASK_PARENT_OUTPUT,
    ),
    (
        "runtime/remote_task_completion_protocol.md",
        RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL,
    ),
    (
        "runtime/remote_task_node_protocol.md",
        RUNTIME_REMOTE_TASK_NODE_PROTOCOL,
    ),
    (
        "runtime/remote_task_attachment_failures.md",
        RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES,
    ),
    (
        "runtime/remote_task_upstream_pull.md",
        RUNTIME_REMOTE_TASK_UPSTREAM_PULL,
    ),
    ("runtime/artifact_finalize.md", RUNTIME_ARTIFACT_FINALIZE),
    ("runtime/runtime_control_resume.md", RUNTIME_CONTROL_RESUME),
    (
        "runtime/runtime_control_resume_with_message.md",
        RUNTIME_CONTROL_RESUME_WITH_MESSAGE,
    ),
    ("runtime/workflow_resume.md", RUNTIME_WORKFLOW_RESUME),
    ("runtime/user_role_message.md", RUNTIME_USER_ROLE_MESSAGE),
    ("runtime/user_quote.md", RUNTIME_USER_QUOTE),
    ("runtime/user_files.md", RUNTIME_USER_FILES),
    ("runtime/cicd-goal.md", CICD_GOAL),
    (
        "runtime/ai-dynamic/proposal_repair.md",
        AI_DYNAMIC_PROPOSAL_REPAIR,
    ),
    ("runtime/ai-dynamic/fanout.md", AI_DYNAMIC_FANOUT),
    ("runtime/ai-dynamic/merge.md", AI_DYNAMIC_MERGE),
    ("runtime/ai-dynamic/acceptance.md", AI_DYNAMIC_ACCEPTANCE),
    ("runtime/ai-dynamic/node_task.md", AI_DYNAMIC_NODE_TASK),
    (
        "runtime/ai-dynamic/hidden_context.md",
        AI_DYNAMIC_HIDDEN_CONTEXT,
    ),
    (
        "runtime/ai-dynamic/workflow_invocation.md",
        AI_DYNAMIC_WORKFLOW_INVOCATION,
    ),
    ("runtime/ai-dynamic/system.md", AI_DYNAMIC_SYSTEM),
    (
        "runtime/ai-dynamic/output_protocol.md",
        AI_DYNAMIC_OUTPUT_PROTOCOL,
    ),
    ("personal-analytics/system.md", PERSONAL_ANALYTICS_SYSTEM),
    ("personal-analytics/user.md", PERSONAL_ANALYTICS_USER),
    (
        "personal-analytics/repair_system.md",
        PERSONAL_ANALYTICS_REPAIR_SYSTEM,
    ),
    (
        "personal-analytics/repair_user.md",
        PERSONAL_ANALYTICS_REPAIR_USER,
    ),
    ("runtime/memory-rules.md", MEMORY_RULES),
    ("runtime/memory-tools.json", MEMORY_TOOLS),
];

pub fn prompt_by_language(language: DesktopLanguage, text: LocalizedText) -> &'static str {
    text.resolve(language)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileTemplateContext {
    pub execution: ProfileExecutionTemplateContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptExecutionSurface {
    Workflow,
    AiDynamic,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileExecutionTemplateContext {
    pub surface: PromptExecutionSurface,
    pub can_route_next: bool,
    pub has_output_contract: bool,
    pub session_mode: String,
}

pub fn profile_template_context(
    surface: PromptExecutionSurface,
    has_output_contract: bool,
    session_mode: &str,
) -> ProfileTemplateContext {
    ProfileTemplateContext {
        execution: ProfileExecutionTemplateContext {
            surface,
            can_route_next: surface == PromptExecutionSurface::AiDynamic && has_output_contract,
            has_output_contract,
            session_mode: session_mode.to_string(),
        },
    }
}

pub fn profile_template_validation_contexts() -> [ProfileTemplateContext; 4] {
    [
        profile_template_context(PromptExecutionSurface::Workflow, false, "new"),
        profile_template_context(PromptExecutionSurface::Workflow, true, "continue"),
        profile_template_context(PromptExecutionSurface::AiDynamic, true, "new"),
        profile_template_context(PromptExecutionSurface::AiDynamic, true, "continue"),
    ]
}

pub fn render<T: Serialize>(template: &str, context: T) -> Result<String> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    let template = env
        .template_from_str(template)
        .map_err(|error| anyhow!(error.to_string()))?;
    template
        .render(context)
        .map_err(|error| anyhow!(error.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn bundled_prompt_line_endings_are_platform_independent() {
        for (path, prompt) in BUNDLED_PROMPTS {
            for text in [
                Some(prompt.zh_cn),
                Some(prompt.en),
                prompt.zh_tw,
                prompt.ja_jp,
                prompt.ko_kr,
                prompt.pt_br,
                prompt.es,
            ]
            .into_iter()
            .flatten()
            {
                assert!(!text.contains('\r'), "{path} must use LF line endings");
            }
        }
    }

    fn assert_fully_rendered(template: &str, context: serde_json::Value) -> String {
        let rendered =
            render(template, context).expect("prompt should render with strict variables");
        assert!(!rendered.contains("{{"), "unresolved output expression");
        assert!(!rendered.contains("{%"), "unresolved control expression");
        rendered
    }

    #[test]
    fn personal_analytics_templates_render_with_strict_contexts() {
        let report_schema = r#"{"type":"object","required":["schemaVersion"]}"#;

        for template in [
            PERSONAL_ANALYTICS_SYSTEM_ZH_CN,
            PERSONAL_ANALYTICS_SYSTEM_EN,
        ] {
            assert_fully_rendered(template, json!({ "report_schema": report_schema }));
        }

        for template in [PERSONAL_ANALYTICS_USER_ZH_CN, PERSONAL_ANALYTICS_USER_EN] {
            assert_fully_rendered(
                template,
                json!({
                    "operation_id": "operation-001",
                    "report_schema_version": "1.0.0",
                    "source_watermark": "2026-08-17T12:00:00Z",
                    "index_revision": 7,
                    "date_range": "{\"start\":null,\"end\":null}",
                    "projection_path": "C:\\analytics\\projection.json",
                    "content_manifest_path": "C:\\analytics\\content-manifest.json",
                    "semantic_batch_manifest_path": "C:\\analytics\\semantic-batches.json",
                    "coverage_summary": "{\"parsed\": 90, \"skipped\": 2}"
                }),
            );
        }

        for template in [
            PERSONAL_ANALYTICS_REPAIR_SYSTEM_ZH_CN,
            PERSONAL_ANALYTICS_REPAIR_SYSTEM_EN,
        ] {
            assert_fully_rendered(template, json!({}));
        }

        for template in [
            PERSONAL_ANALYTICS_REPAIR_USER_ZH_CN,
            PERSONAL_ANALYTICS_REPAIR_USER_EN,
        ] {
            assert_fully_rendered(
                template,
                json!({
                    "operation_id": "operation-001",
                    "invalid_report_path": "C:\\analytics\\invalid-report.json",
                    "validation_errors": "$.overview: required property is missing",
                    "report_schema": report_schema
                }),
            );
        }
    }

    #[test]
    fn remote_task_context_templates_render_all_and_partial_fields() {
        // 远程任务 DPMS 溯源块（multica 预填 composer / 会话起始输入）：全字段与部分字段
        // （其余 null，对应 wire 缺省键）都需完整渲染，且中文标签锁定（发布计划 ID / 业务需求 ID / 需求链接）。
        let full = json!({
            "release_plan_id": 538181,
            "dev_user": "alice,bob",
            "test_user": "carol",
            "business_story_id": 674290,
            "origin_url": "https://dpms.example.com/story/674290"
        });
        let zh = assert_fully_rendered(RUNTIME_REMOTE_TASK_CONTEXT_ZH_CN, full.clone());
        assert!(zh.contains("- 发布计划 ID: 538181"));
        assert!(zh.contains("- 开发负责人: alice,bob"));
        assert!(zh.contains("- 业务需求 ID: 674290"));
        assert!(zh.contains("- 需求链接: https://dpms.example.com/story/674290"));

        let en = assert_fully_rendered(RUNTIME_REMOTE_TASK_CONTEXT_EN, full);
        assert!(en.contains("- Release plan ID: 538181"));
        assert!(en.contains("- Business story ID: 674290"));
        assert!(en.contains("- Requirement link: https://dpms.example.com/story/674290"));

        // 部分字段（其余 null）：仅渲染存在的行，缺席字段不出行、无空行残留。
        let partial = json!({
            "release_plan_id": 538181,
            "dev_user": null,
            "test_user": null,
            "business_story_id": null,
            "origin_url": null
        });
        let zh_partial = assert_fully_rendered(RUNTIME_REMOTE_TASK_CONTEXT_ZH_CN, partial.clone());
        assert!(zh_partial.contains("- 发布计划 ID: 538181"));
        assert!(!zh_partial.contains("开发负责人"));
        assert!(!zh_partial.contains("\n\n"));
        let en_partial = assert_fully_rendered(RUNTIME_REMOTE_TASK_CONTEXT_EN, partial);
        assert!(!en_partial.contains("Dev owner"));
    }

    #[test]
    fn remote_task_parent_output_templates_render_handoff() {
        // 上游交付说明块（issue 完成输出传递特性；三次调整后随预填进 composer）：parent_output 渲染
        // 进模板正文，中英文都需完整渲染且锁定关键语义（执行上下文 + 交付说明）。
        let ctx =
            json!({"parent_output": "部署地址: https://t.example.com\n测试要点: 回归登录链路"});
        let zh = assert_fully_rendered(RUNTIME_REMOTE_TASK_PARENT_OUTPUT_ZH_CN, ctx.clone());
        assert!(zh.contains("执行上下文"));
        assert!(zh.contains("部署地址: https://t.example.com"));

        let en = assert_fully_rendered(RUNTIME_REMOTE_TASK_PARENT_OUTPUT_EN, ctx);
        assert!(en.contains("execution context"));
        assert!(en.contains("部署地址: https://t.example.com"));
    }

    #[test]
    fn remote_task_attachment_failures_templates_render_list() {
        // 附件软失败清单（M5-bu）：failures 数组逐项渲染 filename+reason；中英文都需
        // 完整渲染且锁定「不臆测文件内容 + 与用户确认替代来源」的关键语义。
        let ctx = json!({"failures": [
            {"filename": "多值测试样本.zip", "reason": "HTTP 404"},
            {"filename": "需求说明.pdf", "reason": "timeout after 120s"}
        ]});
        let zh = assert_fully_rendered(RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES_ZH_CN, ctx.clone());
        assert!(zh.contains("需求附件未能下载到本地"));
        assert!(zh.contains("多值测试样本.zip（HTTP 404）"));
        assert!(zh.contains("需求说明.pdf（timeout after 120s）"));
        assert!(zh.contains("不要臆测文件内容"));

        let en = assert_fully_rendered(RUNTIME_REMOTE_TASK_ATTACHMENT_FAILURES_EN, ctx);
        assert!(en.contains("could not be downloaded locally"));
        assert!(en.contains("多值测试样本.zip (HTTP 404)"));
        assert!(en.contains("需求说明.pdf (timeout after 120s)"));
        assert!(en.contains("instead of guessing their contents"));
    }

    #[test]
    fn remote_task_completion_protocol_templates_lock_fence_contract() {
        // 完成输出协议（写侧）：模板不含变量、无条件渲染；中英文都必须锁定围栏块 info 串
        // `completion-output`（提取函数按该 info 串精确匹配，两语言漂移即提取失效）。
        // 服务端上限 16k 收紧（2026-09-21）后另锁四点篇幅契约：非 run 日志（最终交付说明）、
        // 2k 字符以内、超长走工作区文件 + 块内路径引用（超长 400 会卡整个 done 请求）、
        // 传递给下游子任务（读侧 parent_output 的消费方）。协议块改由隐式隐藏区段注入（M5-bk），
        // 文案同步精简为要点列表，契约锚点不变。
        // M5-bp（开发设计 §12.55）：块由「尽力附带」升级为 direct+issue 任务的**完成门控**
        // （有块才完成、无块扣留保持进行中）——文案必须双向锁定：块在才标完成 + 未完成不得输出。
        for template in [
            RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL_ZH_CN,
            RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL_EN,
        ] {
            let rendered = assert_fully_rendered(template, json!({}));
            assert!(rendered.contains("completion-output"));
        }
        let zh = RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL_ZH_CN;
        let en = RUNTIME_REMOTE_TASK_COMPLETION_PROTOCOL_EN;
        assert!(zh.contains("最终交付说明"));
        assert!(zh.contains("run 日志"));
        assert!(zh.contains("2000 字符"));
        assert!(zh.contains("工作项无法标记完成"));
        assert!(zh.contains("文件路径引用"));
        assert!(zh.contains("下游子任务"));
        assert!(en.contains("final deliverable handoff"));
        assert!(en.contains("run log"));
        assert!(en.contains("2,000 characters"));
        assert!(en.contains("cannot be marked done"));
        assert!(en.contains("file path references"));
        assert!(en.contains("downstream sub-tasks"));
        // M5-bp 门控语义双向锁定：块在才完成 + 未完成不得输出。
        assert!(zh.contains("才会被标记完成"));
        assert!(zh.contains("未完成全部工作时不得输出此块"));
        assert!(en.contains("is marked done only when"));
        assert!(en.contains("Do not output this block until all work is finished"));
        // §12.55 根因修复：内网实测 agent 把 info 串写到裸围栏下一行，提取按 GFM 语义正确
        // 拒绝、任务静默卡在进行中——协议必须给出**精确的单行开栏示例**（```completion-output
        // 同行）+ 同行规则文案，堵住「靠想象排版」；两语言均锁定。
        assert!(zh.contains("```completion-output"));
        assert!(zh.contains("同一行"));
        assert!(en.contains("```completion-output"));
        assert!(en.contains("on the same line"));
        // 整体总结口径（§12.59 直面对应）：块总结整个工作项（会话全部产出与附件），不得只写
        // 最后一轮；不可得时简要总结兜底。两语言均锁定。
        assert!(zh.contains("整个工作项"));
        assert!(zh.contains("不得只写最后一轮的内容"));
        assert!(zh.contains("简要总结"));
        assert!(en.contains("entire work item"));
        assert!(en.contains("do not limit it to the last turn's content"));
        assert!(en.contains("brief summary"));
        // M5-bw（开发设计 §12.65）：覆盖清单由平铺改为分级——构建包名/制品名与部署环境是
        // 最重要输出（含「不涉及构建部署时明确说明」防歧义条款），测试要点/变更范围降为
        // 锦上添花。两语言均锁定。（编排面的「前序 CI/CD 产出携带进块」条款随 M5-cg 移至
        // remote_task_node_protocol——direct 会话无前序节点。）
        assert!(zh.contains("构建出的包名/制品名"));
        assert!(zh.contains("部署的环境"));
        assert!(zh.contains("不涉及构建或部署"));
        assert!(en.contains("the built artifact/package name"));
        assert!(en.contains("the environment it was deployed to"));
        assert!(en.contains("no build or deployment"));
        // M5-by（开发设计 §12.67）：核心信息三项化——改动范围（极其精简）升为第三核心项；
        // 新增完成后修订条款——任务完成后用户在会话中要求调整时重新输出完整新块（整块覆盖
        // 非增量、核心信息保留），只更新远程侧存储内容、不改变任务完成状态。完成门控语义不变
        // （首次块判定完成，后续块走迟到补写通道幂等覆盖）。两语言均锁定。
        assert!(zh.contains("极其精简"));
        assert!(zh.contains("重新输出完整的新块"));
        assert!(zh.contains("不改变任务的完成状态"));
        assert!(en.contains("extremely brief"));
        assert!(en.contains("complete new block"));
        assert!(en.contains("does not change the task's completion status"));
        // M5-cg（开发设计 改动七十六）：direct 协议收窄为纯 direct 语义——编排形态的产出
        // 规则移至 remote_task_node_protocol（按节点身份指定）。direct 模板不得再含编排分支
        // 文案（收尾节点 / 阶段性上送 / 交付上传角色），防止两套产出规则并存。
        assert!(!zh.contains("阶段性"));
        assert!(!zh.contains("交付上传"));
        assert!(!zh.contains("收尾节点"));
        assert!(!en.contains("staged snapshot"));
        assert!(!en.contains("deliverable-upload"));
        assert!(!en.contains("finishing node"));
    }

    #[test]
    fn remote_task_node_protocol_templates_designation_contract() {
        // 编排面完成输出协议（M5-cg 产出侧门禁，开发设计 改动七十六）：产出资格由运行时按节点
        // 身份指定（收尾/验收节点 → 最终交付；标记声明节点 → 阶段性上送；其余 → 禁止产出），
        // 模板按 `designation` 单变量三分支渲染。三态都必须锁定：指定权声明 + 各自身份指令；
        // final/staged 另锁共享交付规则（核心信息分级、前序 CI/CD 携带、整体总结、≤2000、
        // JSON 裁决前置、精确围栏格式）；prohibited 必须短且不含围栏示例与产出规则。
        for language in [
            RUNTIME_REMOTE_TASK_NODE_PROTOCOL_ZH_CN,
            RUNTIME_REMOTE_TASK_NODE_PROTOCOL_EN,
        ] {
            for designation in ["final", "staged", "prohibited"] {
                let rendered = assert_fully_rendered(
                    language,
                    json!({ "designation": designation }),
                );
                assert!(rendered.contains("completion-output"));
            }
        }
        let zh = RUNTIME_REMOTE_TASK_NODE_PROTOCOL_ZH_CN;
        let en = RUNTIME_REMOTE_TASK_NODE_PROTOCOL_EN;

        // final：收尾节点产出最终交付说明。
        let zh_final = assert_fully_rendered(zh, json!({ "designation": "final" }));
        assert!(zh_final.contains("收尾节点"));
        assert!(zh_final.contains("最终交付说明"));
        let en_final = assert_fully_rendered(en, json!({ "designation": "final" }));
        assert!(en_final.contains("finishing node"));
        assert!(en_final.contains("final deliverable handoff"));

        // staged：被指定承担阶段性上送的节点（角色 / 声明标记），须注明阶段性口径。
        let zh_staged = assert_fully_rendered(zh, json!({ "designation": "staged" }));
        assert!(zh_staged.contains("阶段性上送"));
        assert!(zh_staged.contains("「交付上传」角色"));
        assert!(zh_staged.contains("声明标记"));
        assert!(zh_staged.contains("阶段性快照"));
        assert!(zh_staged.contains("尚未最终完成"));
        assert!(zh_staged.contains("在工作流结束前拉取"));
        let en_staged = assert_fully_rendered(en, json!({ "designation": "staged" }));
        assert!(en_staged.contains("staged upload"));
        assert!(en_staged.contains("deliverable-upload role"));
        assert!(en_staged.contains("declaration marker"));
        assert!(en_staged.contains("staged snapshot"));
        assert!(en_staged.contains("with work still pending"));
        assert!(en_staged.contains("pull before the workflow finishes"));

        // final/staged 共享交付规则：核心信息分级（含前序 CI/CD 携带）、整体总结（不得只写本
        // 节点产物）、JSON 裁决前置、篇幅与围栏格式契约。
        for rendered in [zh_final.clone(), zh_staged.clone()] {
            assert!(rendered.contains("构建出的包名/制品名"));
            assert!(rendered.contains("尤其是 CI/CD 节点"));
            assert!(rendered.contains("携带进你的块"));
            assert!(rendered.contains("整个工作项"));
            assert!(rendered.contains("不得只写你自己节点的产物"));
            assert!(rendered.contains("简要总结"));
            assert!(rendered.contains("JSON 裁决"));
            assert!(rendered.contains("2000 字符"));
            assert!(rendered.contains("```completion-output"));
            assert!(rendered.contains("同一行"));
        }
        for rendered in [en_final.clone(), en_staged.clone()] {
            assert!(rendered.contains("especially those of CI/CD nodes"));
            assert!(rendered.contains("carry that information forward into your block"));
            assert!(rendered.contains("entire work item"));
            assert!(rendered.contains("do not limit the block to your own node's output"));
            assert!(rendered.contains("brief summary"));
            assert!(rendered.contains("JSON verdict"));
            assert!(rendered.contains("2,000 characters"));
            assert!(rendered.contains("```completion-output"));
            assert!(rendered.contains("on the same line"));
        }

        // prohibited：明确禁止产出，不含围栏示例与篇幅/核心信息规则（无产出即无格式契约）。
        let zh_prohibited = assert_fully_rendered(zh, json!({ "designation": "prohibited" }));
        assert!(zh_prohibited.contains("不要输出 `completion-output` 块"));
        assert!(!zh_prohibited.contains("```completion-output"));
        assert!(!zh_prohibited.contains("2000 字符"));
        let en_prohibited = assert_fully_rendered(en, json!({ "designation": "prohibited" }));
        assert!(en_prohibited.contains("do not output a `completion-output` block"));
        assert!(!en_prohibited.contains("```completion-output"));
        assert!(!en_prohibited.contains("2,000 characters"));
    }

    #[test]
    fn remote_task_upstream_pull_templates_render_parent_snapshot() {
        // 上游交付拉取（M5-cf 读侧）：`$upstream.completion-output` 令牌的替换文本。模板需完整
        // 渲染 parent_status 与 parent_output 两个变量，并锁定阶段性快照口径——父状态可见 +
        // 未完成时是阶段性快照可能被覆盖 + 按当前值执行并在结论注明版本口径。两语言均锁定。
        let ctx = json!({
            "parent_status": "in_progress",
            "parent_output": "包名：gold-band_0.19.0_x64.msi；部署环境：内网测试环境 http://t.example.com"
        });
        let zh = assert_fully_rendered(RUNTIME_REMOTE_TASK_UPSTREAM_PULL.zh_cn, ctx.clone());
        assert!(zh.contains("in_progress"));
        assert!(zh.contains("gold-band_0.19.0_x64.msi"));
        assert!(zh.contains("阶段性快照"));
        assert!(zh.contains("按当前值执行"));
        assert!(zh.contains("版本口径"));

        let en = assert_fully_rendered(RUNTIME_REMOTE_TASK_UPSTREAM_PULL.en, ctx);
        assert!(en.contains("in_progress"));
        assert!(en.contains("gold-band_0.19.0_x64.msi"));
        assert!(en.contains("staged snapshot"));
        assert!(en.contains("execute against the current value"));
        assert!(en.contains("version basis"));
    }

    #[test]
    fn personal_analytics_system_prompts_lock_metric_and_evidence_contracts() {
        for (template, metric_names) in [
            (
                PERSONAL_ANALYTICS_SYSTEM_ZH_CN,
                [
                    "direct.reply_completion_rate",
                    "workflow.run_terminal_success_rate",
                    "auto.outer_run_terminal_success_rate",
                ],
            ),
            (
                PERSONAL_ANALYTICS_SYSTEM_EN,
                [
                    "direct.reply_completion_rate",
                    "workflow.run_terminal_success_rate",
                    "auto.outer_run_terminal_success_rate",
                ],
            ),
        ] {
            for metric_name in metric_names {
                assert!(
                    template.contains(metric_name),
                    "missing metric name: {metric_name}"
                );
            }
            assert!(template.to_ascii_lowercase().contains("evidence locator"));
            assert!(template.contains("sampleCount"));
            assert!(template.contains("confidence"));
            assert!(template.contains("acp.raw.jsonl"));
            assert!(template.contains("{{ report_schema }}"));
        }
    }

    #[test]
    fn personal_analytics_repair_prompts_forbid_unsupported_facts() {
        assert!(PERSONAL_ANALYTICS_REPAIR_SYSTEM_ZH_CN.contains("不新增洞察"));
        assert!(PERSONAL_ANALYTICS_REPAIR_SYSTEM_ZH_CN.contains("不得猜测"));
        assert!(PERSONAL_ANALYTICS_REPAIR_SYSTEM_EN.contains("do not repeat the analysis"));
        assert!(PERSONAL_ANALYTICS_REPAIR_SYSTEM_EN.contains("Never guess"));
    }

    #[test]
    fn personal_analytics_templates_reject_missing_variables() {
        let error = render(PERSONAL_ANALYTICS_USER_ZH_CN, json!({}));
        assert!(
            error.is_err(),
            "strict rendering must reject a missing operation context"
        );
    }

    fn assert_prompt_contract(prompt: &str, required_fragments: &[&str]) {
        for fragment in required_fragments {
            assert!(
                prompt.contains(fragment),
                "prompt contract is missing `{fragment}`"
            );
        }
    }

    #[test]
    fn ai_dynamic_system_prompts_define_scope_authority_and_the_scope_gate() {
        assert_prompt_contract(
            AI_DYNAMIC_SYSTEM_ZH_CN,
            &[
                "人类最新指令 > 原始需求及其中或用户明确声明的非目标",
                "运行前已纳入范围的项目契约",
                "低层内容只能细化执行，不能扩大或缩小高层范围",
                "不能把需求原文提到的内容划为非目标",
                "runtime 任务可以拆解已授权工作",
                "本轮新增内容只能提供证据或建议",
                "省略后会失败的既定结果",
                "内部手段无需在需求中逐字出现",
                "可归因到本轮变更的范围漂移",
                "恢复最小范围内方案",
            ],
        );
        assert_prompt_contract(
            AI_DYNAMIC_SYSTEM_EN,
            &[
                "latest relevant human instruction > original requirement and non-goals stated in it or explicitly declared by the user",
                "pre-run project contracts already in scope",
                "cannot expand or narrow higher-authority scope",
                "cannot classify content mentioned in the original requirement text as a non-goal",
                "runtime tasks may decompose authorized work",
                "content added during this run provide evidence or suggestions",
                "established outcome that would fail without it",
                "need not appear verbatim in the requirement",
                "scope drift proven by change evidence attributable to this run",
                "Restore the minimum in-scope solution",
            ],
        );
    }

    #[test]
    fn acceptance_prompts_separate_scope_or_regression_backed_blockers_from_follow_ups() {
        for prompt in [AI_DYNAMIC_ACCEPTANCE_ZH_CN, PROFILE_ACCEPT_ZH_CN] {
            assert_prompt_contract(
                prompt,
                &[
                    "BLOCKER",
                    "FOLLOW_UP",
                    "范围内结果失败或无法验证",
                    "可达回归",
                    "可归因到本轮变更",
                    "范围依据",
                    "当前证据",
                    "失败因果",
                    "不影响通过",
                    "不创建修复",
                    "恢复最小范围内方案",
                    "不能删除、缩小、拆散、替换或弱化",
                    "不属于任何已批准验收条款",
                    "`PARTIAL`",
                    "`MISSING`",
                    "`FOLLOW_UP` 只用于三类观察",
                    "仅因环境或人工条件无法执行的验证",
                    "不属于本轮需求所要求范围的问题",
                    "本轮需求所要求的条款一律不得降为 `FOLLOW_UP`",
                ],
            );
        }
        for prompt in [AI_DYNAMIC_ACCEPTANCE_EN, PROFILE_ACCEPT_EN] {
            assert_prompt_contract(
                prompt,
                &[
                    "BLOCKER",
                    "FOLLOW_UP",
                    "in-scope outcome that fails or cannot be verified",
                    "reachable regression",
                    "change evidence attributable to this run",
                    "scope basis",
                    "current evidence",
                    "failure causality",
                    "does not affect acceptance",
                    "create repair",
                    "minimum in-scope solution",
                    "cannot delete, narrow, split, replace, or weaken",
                    "does not belong to any approved acceptance criterion",
                    "`PARTIAL`",
                    "`MISSING`",
                    "`FOLLOW_UP` is limited to three kinds of observation",
                    "a check that cannot be executed solely because of environment or manual conditions",
                    "problems outside the scope this round's requirement asks for",
                    "A criterion this round's requirement asks for is never downgraded to `FOLLOW_UP`",
                ],
            );
        }

        assert_prompt_contract(AI_DYNAMIC_ACCEPTANCE_ZH_CN, &["不得修改业务代码或测试代码"]);
        assert_prompt_contract(
            AI_DYNAMIC_ACCEPTANCE_EN,
            &["Do not modify business code or test code"],
        );
    }

    #[test]
    fn development_role_removes_unnecessary_self_selected_mechanisms() {
        assert_prompt_contract(
            PROFILE_DEV_TEST_ZH_CN,
            &[
                "节点任务和反馈只能细化执行，不能扩大范围",
                "省略后会失败的既定结果",
                "内部手段无需在需求中逐字出现",
                "首次执行完成既定范围",
                "只修复有范围依据、当前证据和失败因果的 `BLOCKER`",
                "恢复最小范围内方案",
            ],
        );
        assert_prompt_contract(
            PROFILE_DEV_TEST_EN,
            &[
                "Node tasks and feedback may refine execution, but cannot expand scope",
                "established outcome that would fail without it",
                "need not appear verbatim in the requirement",
                "On initial execution, complete the established scope",
                "repair only a `BLOCKER` with a scope basis, current evidence, and failure causality",
                "minimum in-scope solution",
            ],
        );
    }

    #[test]
    fn routing_prompts_do_not_promote_suggestions_into_successor_scope() {
        assert_prompt_contract(
            AI_DYNAMIC_OUTPUT_PROTOCOL_ZH_CN,
            &[
                "后继任务",
                "既定范围内结果",
                "BLOCKER",
                "FOLLOW_UP",
                "前序建议",
            ],
        );
        assert_prompt_contract(
            AI_DYNAMIC_OUTPUT_PROTOCOL_EN,
            &[
                "successor task",
                "established in-scope outcome",
                "BLOCKER",
                "FOLLOW_UP",
                "predecessor suggestion",
            ],
        );
        assert_prompt_contract(
            AI_DYNAMIC_PROPOSAL_REPAIR_ZH_CN,
            &[
                "只修复协议校验错误",
                "不重新执行任务",
                "符合范围契约",
                "删除或收窄",
            ],
        );
        assert_prompt_contract(
            AI_DYNAMIC_PROPOSAL_REPAIR_EN,
            &[
                "Repair only protocol validation errors",
                "do not re-execute the task",
                "satisfy the scope contract",
                "remove or narrow",
            ],
        );
    }

    #[test]
    fn routing_prompts_end_only_after_acceptance() {
        assert_prompt_contract(
            AI_DYNAMIC_OUTPUT_PROTOCOL_ZH_CN,
            &[
                "整个需求的开发全部完成后，必须交给验收节点对照需求统一收尾",
                "中间阶段完成时不单独验收",
                "验收不通过继续指派节点修复，修复后再次验收",
                "只有验收通过后才使用 `next.type=\"end\"`",
            ],
        );
        assert_prompt_contract(
            AI_DYNAMIC_OUTPUT_PROTOCOL_EN,
            &[
                "Once development of the whole requirement is complete, hand off to an acceptance node",
                "do not run acceptance for intermediate stages",
                "keep assigning nodes to fix it and run acceptance again",
                "Use `next.type=\"end\"` only after acceptance passes",
            ],
        );
        assert_prompt_contract(
            AI_DYNAMIC_SYSTEM_ZH_CN,
            &["整个需求的交付已通过验收、没有后续工作"],
        );
        assert_prompt_contract(
            AI_DYNAMIC_SYSTEM_EN,
            &["the delivery of the whole requirement has passed acceptance and no work remains"],
        );
    }

    #[test]
    fn workflow_resume_optimizes_for_established_scope_not_reviewer_approval() {
        assert_prompt_contract(
            RUNTIME_WORKFLOW_RESUME_ZH_CN,
            &["继续当前节点任务", "反馈是证据，不是新增授权", "既定范围"],
        );
        assert_prompt_contract(
            RUNTIME_WORKFLOW_RESUME_EN,
            &[
                "Continue the current node task",
                "Feedback is evidence, not new authorization",
                "established scope",
            ],
        );
        assert!(RUNTIME_WORKFLOW_RESUME_ZH_CN.chars().count() <= 60);
        assert!(RUNTIME_WORKFLOW_RESUME_EN.chars().count() <= 180);
    }

    #[test]
    fn bundled_prompts_keep_marker_order_and_cicd_falls_back_to_english() {
        assert_eq!(PROFILE_CICD.resolve(DesktopLanguage::JaJp), PROFILE_CICD.en);
        assert_eq!(CICD_GOAL.resolve(DesktopLanguage::ZhTw), CICD_GOAL.en);
        assert_ne!(PROFILE_PLAN.resolve(DesktopLanguage::JaJp), PROFILE_PLAN.en);
        assert_ne!(
            PROFILE_PLAN.resolve(DesktopLanguage::ZhTw),
            PROFILE_PLAN.zh_cn
        );

        for (path, text) in BUNDLED_PROMPTS {
            let english_markers = template_markers(text.en);
            for (language, value) in [
                ("zh-CN", Some(text.zh_cn)),
                ("zh-TW", text.zh_tw),
                ("ja-JP", text.ja_jp),
                ("ko-KR", text.ko_kr),
                ("pt-BR", text.pt_br),
                ("es", text.es),
            ] {
                let Some(value) = value else {
                    continue;
                };
                assert_eq!(
                    template_markers(value),
                    english_markers,
                    "{path} {language} MiniJinja markers diverged"
                );
                assert!(
                    !value.to_ascii_lowercase().contains("wb"),
                    "{path} {language} contains a channel token"
                );
            }
            if *path == "runtime/memory-tools.json" {
                let english_keys = json_object_keys(text.en);
                for value in [
                    text.zh_cn,
                    text.zh_tw.unwrap(),
                    text.ja_jp.unwrap(),
                    text.ko_kr.unwrap(),
                    text.pt_br.unwrap(),
                    text.es.unwrap(),
                ] {
                    assert_eq!(json_object_keys(value), english_keys);
                }
            }
        }
    }

    fn template_markers(text: &str) -> Vec<String> {
        let mut markers = Vec::new();
        let mut rest = text;
        loop {
            let open_expr = rest.find("{{");
            let open_stmt = rest.find("{%");
            let start = match (open_expr, open_stmt) {
                (Some(expr), Some(stmt)) => expr.min(stmt),
                (Some(expr), None) => expr,
                (None, Some(stmt)) => stmt,
                (None, None) => break,
            };
            let after = &rest[start..];
            let close = if after.starts_with("{{") { "}}" } else { "%}" };
            let end = after.find(close).expect("MiniJinja marker closes");
            markers.push(after[..end + close.len()].to_string());
            rest = &after[end + close.len()..];
        }
        markers
    }

    fn json_object_keys(text: &str) -> Vec<String> {
        let mut keys = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text)
            .expect("memory tool descriptions are json")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }
}
