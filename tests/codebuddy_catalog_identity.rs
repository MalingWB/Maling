use gold_band::config::SettingsConfig;

#[test]
fn loading_previous_defaults_names_the_cli_without_rewriting_custom_labels() {
    for (version, original, expected) in [
        (12, "CodeBuddy", "CodeBuddy Code"),
        (13, "WorkBuddy", "CodeBuddy Code"),
        (13, "My local agent", "My local agent"),
    ] {
        let raw = format!(r#"{{"settingsSchemaVersion":{version},"agents":{{"codebuddy-code":{{"adapter":{{"displayName":"{original}","command":"codebuddy"}}}}}}}}"#);
        let (settings, _) = SettingsConfig::from_json_value_with_migration(raw.parse().unwrap()).unwrap();
        let agents = settings.agents.unwrap();
        let agent = agents.get(&"codebuddy-code".parse().unwrap()).unwrap();
        assert_eq!(agent.adapter.display_name, expected);
        // Catalog launch uses npx; the local executable is the separate localLaunch command.
        let cli = gold_band::config::catalog_agent_default_config("codebuddy-code").unwrap();
        assert_eq!(agent.adapter.command, cli.adapter.command);
        assert_eq!(agent.adapter.args, cli.adapter.args);
        assert!(agent.adapter.args.iter().any(|arg| arg.starts_with("@tencent-ai/codebuddy-code@")));
        assert!(agent.adapter.args.iter().any(|arg| arg == "--acp"));
    }
}
