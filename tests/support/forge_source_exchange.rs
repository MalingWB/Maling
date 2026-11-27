//! Exercises both the composer digest and actual persisted first-prompt material.
use super::{direct_command, fixture};
use camino::Utf8PathBuf;
use gold_band::{
    acp::branches,
    app::forge_service::{ForgeSourceMessage, ForgeSourceSession},
    provider::PromptHiddenSection,
    storage::read_json,
};
use serde_json::{Value, json};
use std::fs;

// Equal wall-clock times deliberately force cutoff logic to use message identity/sequence.
fn row(seq: u64, kind: &str, content: &str, raw: Value) -> String {
    json!({ "item": {"id": format!("message-{seq}"), "seq": seq,
        "timestamp": "2026-08-19T01:00:31Z", "kind": kind, "sessionId": "source-session",
        "content": content, "raw": raw } })
    .to_string()
        + "\n"
}

#[test]
fn forge_from_here_only_feeds_messages_after_the_truncation_point() {
    let (app, temp) = fixture();
    let mut ordinary = direct_command();
    ordinary.content = "Implement sign-in".into();
    let (source, _, _) = app.prepare_conversation_task(&ordinary).unwrap().accept();
    let task = app.task_show(&source).unwrap();
    assert!(task.origin.is_none());
    let mut session = ForgeSourceSession {
        task_uuid: task.uuid,
        run_id: "run-1".into(),
        round_id: "round-1".into(),
        node_id: "node-1".into(),
        attempt_id: "attempt-1".into(),
        branch_id: "root".into(),
        outer_node_id: None,
        outer_attempt_id: None,
        after: None,
    };
    let dir = app
        .paths
        .attempt_dir(&source, "run-1", "round-1", "node-1", "attempt-1");
    fs::create_dir_all(&dir).unwrap();
    let file = branches::branch_timeline_path(&dir, "root");
    let user = json!({"source":"goldBandPrompt"});
    let transcript = [
        row(1, "userTextDelta", "First question", user.clone()),
        row(2, "thoughtDelta", "private reasoning", Value::Null),
        row(3, "toolCall", "private tool output", Value::Null),
        row(4, "textDelta", "Two-step proposal", Value::Null),
        row(
            5,
            "userTextDelta",
            "Confirmed!<hidden data-gold-band-hidden=\"true\">private instructions</hidden>",
            user,
        ),
        row(
            6,
            "userTextDelta",
            "private repair",
            json!({"source":"goldBandPrompt", "hiddenFromChat":true}),
        ),
    ]
    .concat();
    fs::write(&file, &transcript).unwrap();
    let sibling = branches::stable_agent_execution_id("source-session", "tool-1");
    let sibling_file = branches::branch_timeline_path(&dir, &sibling);
    fs::create_dir_all(sibling_file.parent().unwrap()).unwrap();
    fs::write(
        sibling_file,
        row(
            1,
            "textDelta",
            "Sibling proposal",
            json!({"_meta":{"goldBandConversation":{"branchId":sibling}}}),
        ),
    )
    .unwrap();

    // Same task and attempt IDs in a separate, genuine project scope.
    let other_repo = Utf8PathBuf::from_path_buf(temp.path().join("other-project")).unwrap();
    fs::create_dir_all(&other_repo).unwrap();
    let other_app = app.with_repo_root(other_repo, app.config.clone());
    assert_ne!(other_app.paths.project_id, app.paths.project_id);
    let foreign = other_app
        .paths
        .attempt_dir(&source, "run-1", "round-1", "node-1", "attempt-1");
    fs::create_dir_all(&foreign).unwrap();
    fs::copy(
        app.paths.task_dir(&source).join("task.json"),
        other_app.paths.task_dir(&source).join("task.json"),
    )
    .unwrap();
    fs::write(
        foreign.join("acp.timeline.jsonl"),
        transcript.replace("Two-step proposal", "Foreign proposal"),
    )
    .unwrap();
    assert!(
        other_app
            .forge_source_digest(&source, Some(&session))
            .unwrap()
            .markdown
            .contains("Foreign proposal")
    );
    let preview = app.forge_source_digest(&source, Some(&session)).unwrap();
    let material = app.forge_source_material(&source, Some(&session)).unwrap();
    for text in [&preview.markdown, &material.hidden_section.content] {
        assert!(text.find("First question").unwrap() < text.find("Two-step proposal").unwrap());
        assert!(text.find("Two-step proposal").unwrap() < text.find("Confirmed!").unwrap());
        for unrelated in ["private", "Sibling proposal", "Foreign proposal"] {
            assert!(!text.contains(unrelated));
        }
    }
    let other = ForgeSourceSession {
        branch_id: sibling,
        ..session.clone()
    };
    let branch = app
        .forge_source_digest(&source, Some(&other))
        .unwrap()
        .markdown;
    assert!(branch.contains("Sibling proposal") && !branch.contains("First question"));

    session.after = Some(ForgeSourceMessage {
        event_id: "message-4".into(),
        event_seq: 4,
    });
    let preview = app
        .forge_source_digest(&source, Some(&session))
        .unwrap()
        .markdown;
    assert!(preview.contains("Implement sign-in") && preview.contains("Confirmed!"));
    assert!(!preview.contains("Two-step proposal"));
    let mut command = direct_command();
    command.forge_source_task_id = Some(source.clone());
    command.forge_source_session = Some(session.clone());
    let id = app.prepare_conversation_task(&command).unwrap().accept().0;
    let sections: Vec<PromptHiddenSection> =
        read_json(&app.paths.first_prompt_hidden_sections_file(&id)).unwrap();
    let hidden = sections.iter().find(|s| s.title == "源会话材料").unwrap();
    assert_eq!(hidden.content, preview);
    session.after.as_mut().unwrap().event_seq += 1;
    assert_eq!(
        app.forge_source_digest(&source, Some(&session))
            .unwrap_err()
            .to_string(),
        "forge/stale-message"
    );
    session.after = None;
    fs::write(
        &file,
        transcript + &row(7, "textDelta", &"大".repeat(20_000), Value::Null),
    )
    .unwrap();
    let bounded = app
        .forge_source_material(&source, Some(&session))
        .unwrap()
        .hidden_section
        .content;
    assert!(bounded.contains("truncated") && bounded.chars().count() <= 16_000);
    // No unbounded legacy read or index maintenance hidden inside an export.
    fs::File::create(&file)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    assert!(
        app.forge_source_digest(&source, Some(&session))
            .unwrap()
            .markdown
            .contains("truncated")
    );
    assert!(!file.with_extension("index.json").exists());
}
