use gold_band::{
    app::forge_service::{ForgeMaterialUnavailable, ForgeSourceItem},
    skill::claude_plugin::scan_claude_plugins,
};
use serde_json::json;

fn selection(catalog: &str, source: &str, id: &str) -> ForgeSourceItem {
    serde_json::from_value(json!({"catalog":catalog,"source":source,"assetId":id})).unwrap()
}

fn fixture_skill(app: &gold_band::app::App, body: &str) -> ForgeSourceItem {
    let dir = app.paths.project_skills_dir().join("fixture");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: same\ndescription: skill summary\n---\n{body}"),
    )
    .unwrap();
    selection(
        "skill",
        "project",
        &gold_band::skill::forge_skill_asset_id(&dir),
    )
}

#[test]
fn mixed_material_authority_and_budget_apply_to_existing_skill_inputs() {
    let (app, _temp) = super::fixture();
    let item = fixture_skill(&app, &"字".repeat(20_000));
    let mut command = super::direct_command();
    command.role = Some(gold_band::provider::UserPromptRole {
        profile_id: "pf-builtin-workflow-forge".into(),
        name: "claim".into(),
        content: "untrusted author".into(),
    });
    command.forge_source_items = Some(vec![item.clone()]);
    let id = app.prepare_conversation_task(&command).unwrap().accept().0;
    let input: gold_band::provider::TaskPromptInput =
        gold_band::storage::read_json(&app.paths.task_prompt_input_file(&id)).unwrap();
    assert_ne!(input.role.unwrap().content, "untrusted author");
}

#[test]
fn mixed_material_hidden_section_counts_all_headers_flags_and_separators() {
    let (app, _temp) = super::fixture();
    let item = fixture_skill(&app, &"字".repeat(20_000));
    let material = app.forge_items_material(&[item]).unwrap();
    assert!(material.content.contains("截断"));
    assert!(
        material.content.chars().count() <= 16_000,
        "{}",
        material.content.chars().count()
    );
}

#[test]
fn mixed_material_role_only_and_mixed_resolve_current_ids_and_scopes() {
    let (app, temp) = super::fixture();
    let root = app.paths.user_context_profiles_dir();
    assert!(root.as_std_path().starts_with(temp.path()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("unrelated-filename.md");
    let write_role = |name: &str, body: &str| {
        std::fs::write(&path, format!("---\nid: pf-stable\nname: {name}\nsummary: role summary\ndynamicTemplate: true\n---\n{body}")).unwrap()
    };
    write_role("same", "first body");
    let role = selection("profile", "global", "pf-stable");
    let builtin = selection("profile", "built-in", "pf-builtin-dev");
    let skill = fixture_skill(&app, "skill contents");
    let items = [builtin.clone(), skill.clone(), role.clone()];
    let summaries = serde_json::to_value(app.forge_items_digest(&items).unwrap()).unwrap();
    assert_eq!(summaries[0]["kind"], "role");
    assert_eq!(summaries[0]["source"], "built-in");
    assert_eq!(summaries[1]["kind"], "skill");
    assert_eq!(summaries[2]["source"], "global");
    assert_eq!(summaries[2]["description"], "role summary");
    assert!(!summaries.to_string().contains("first body"));
    let literal = "ROLE SOURCE {{ never_render_this }}";
    write_role("renamed", literal);
    let hidden = app.forge_items_material(&items).unwrap().content;
    assert!(hidden.find("skill contents").unwrap() < hidden.find(literal).unwrap());
    let mut command = super::direct_command();
    command.forge_source_items = Some(vec![role.clone()]);
    let id = app.prepare_conversation_task(&command).unwrap().accept().0;
    let sections: Vec<gold_band::provider::PromptHiddenSection> =
        gold_band::storage::read_json(&app.paths.first_prompt_hidden_sections_file(&id)).unwrap();
    assert!(sections[0].content.contains(literal));
    assert!(sections[0].content.contains("renamed"));
    let input: gold_band::provider::TaskPromptInput =
        gold_band::storage::read_json(&app.paths.task_prompt_input_file(&id)).unwrap();
    assert_eq!(input.role.unwrap().profile_id, "pf-builtin-workflow-forge");
    let invalid = |refs: &[ForgeSourceItem]| {
        assert!(
            app.forge_items_digest(refs)
                .unwrap_err()
                .is::<ForgeMaterialUnavailable>()
        );
        assert!(
            app.forge_items_material(refs)
                .unwrap_err()
                .is::<ForgeMaterialUnavailable>()
        );
    };
    for item in [
        selection("profile", "project", "pf-stable"),
        selection("profile", "built-in", "pf-stable"),
        selection("profile", "global", "pf-builtin-dev"),
        selection("profile", "global", "renamed"),
        selection("profile", "global", path.as_str()),
        selection("skill", "built-in", &skill.asset_id),
        selection("plugin", "built-in", "unused"),
    ] {
        invalid(&[item]);
    }
    invalid(&[role.clone(), role.clone()]);
    std::fs::remove_file(path).unwrap();
    invalid(&items);
    assert!(app.prepare_conversation_task(&command).is_err());
}

#[test]
fn mixed_material_profile_reads_are_bounded_and_ambiguous_ids_fail_closed() {
    let (app, _temp) = super::fixture();
    let root = app.paths.user_context_profiles_dir();
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("first.md");
    let header = "\u{feff}---\r\nid: stored-id\r\nname: bounded\r\nsummary: >\r\n  current\r\n  summary\r\n---\r\n";
    let refs = [selection("profile", "global", "stored-id")];
    let write = |bytes: &[u8]| std::fs::write(&path, [header.as_bytes(), bytes].concat()).unwrap();
    write(&[0xff]); // Digest does not decode or require even the selected body.
    assert_eq!(
        app.forge_items_digest(&refs).unwrap()[0].description,
        "current summary"
    );
    let mut bytes = "𐀀".repeat(17_000).into_bytes();
    bytes.push(0xff); // Full-file reads would fail; only a usable bounded prefix is needed.
    write(&bytes);
    let text = app.forge_items_material(&refs).unwrap().content;
    assert!(text.contains('𐀀') && text.contains("截断") && text.chars().count() <= 16_000);
    std::fs::write(root.join("second.md"), format!("{header}different body")).unwrap();
    assert!(
        app.forge_items_digest(&refs)
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::write(
        root.join("role-legacy.md"),
        "---\nname: no stored id\n---\nnot resolvable",
    )
    .unwrap();
    assert!(
        app.forge_items_digest(&[selection("profile", "global", "legacy")])
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
    std::fs::write(
        path,
        format!(
            "---\nid: oversized\nsummary: {}\n---\nnever read",
            "x".repeat(20_000)
        ),
    )
    .unwrap();
    assert!(
        app.forge_items_material(&[selection("profile", "global", "oversized")])
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
}

#[test]
fn mixed_material_directory_scan_is_bounded_even_for_non_profiles() {
    let (app, _temp) = super::fixture();
    let root = app.paths.user_context_profiles_dir();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("selected.md"),
        "---\nid: selected\nname: selected\n---\nbody",
    )
    .unwrap();
    let items = [selection("profile", "global", "selected")];
    // Count directory entries before filtering, not just successfully parsed profiles.
    for i in 1..=1_024 {
        std::fs::File::create(root.join(i.to_string())).unwrap();
    }
    assert!(
        app.forge_items_digest(&items)
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
    std::fs::remove_file(root.join("1024")).unwrap();
    assert_eq!(app.forge_items_digest(&items).unwrap().len(), 1);
}

#[test]
fn mixed_material_combined_selection_has_an_inclusive_maximum() {
    let (app, _temp) = super::fixture();
    let root = app.paths.user_context_profiles_dir();
    std::fs::create_dir_all(&root).unwrap();
    let mut items = vec![fixture_skill(&app, &"字".repeat(20_000))];
    for i in 1..=32 {
        let id = format!("pf-{i}");
        std::fs::write(
            root.join(format!("{i}.md")),
            format!("---\nid: {id}\nname: role {i}\nsummary: s\n---\nbody {i}"),
        )
        .unwrap();
        items.push(selection("profile", "global", &id));
    }
    assert_eq!(app.forge_items_digest(&items[..32]).unwrap().len(), 32);
    let text = app.forge_items_material(&items[..32]).unwrap().content;
    assert!(text.contains("截断") && text.contains("body 31"));
    assert!(text.chars().count() <= 16_000);
    assert!(app.forge_items_digest(&items).is_err());
    assert!(app.forge_items_material(&items).is_err());
}

#[test]
fn material_ids_resolve_only_current_entries_in_the_selected_project() {
    let (app, _temp) = super::fixture();
    let root = &app.paths.repo_root;
    let manifest = root.join(".claude/skills/dummy/.claude-plugin/plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    let write_manifest = |body: &str| {
        std::fs::write(
            &manifest,
            json!({"name":"dummy", "commands":{"fixture":{"content":body}}}).to_string(),
        )
        .unwrap();
    };
    write_manifest("fixture version one");
    // Explicit empty user/plugin roots: discovery never consults the actual machine inventory.
    let inventory = scan_claude_plugins(
        &root.join("empty-cache"),
        &root.join("empty-home"),
        Some(root),
    );
    let id = &inventory[0].assets[0].asset_id;
    let selection = json!({"catalog":"plugin", "source":"project", "assetId":id});
    let item: ForgeSourceItem = serde_json::from_value(selection.clone()).unwrap();
    let items = std::slice::from_ref(&item);
    assert_eq!(app.forge_items_digest(items).unwrap()[0].name, "fixture");
    let mut extra_fields = selection;
    extra_fields["content"] = "dummy".into();
    assert!(serde_json::from_value::<ForgeSourceItem>(extra_fields).is_err());

    write_manifest("fixture version two");
    let material = app.forge_items_material(items).unwrap();
    assert!(material.content.contains("fixture version two"));
    assert!(!material.content.contains("fixture version one"));

    let (other, _other_temp) = super::fixture();
    assert!(
        other
            .forge_items_material(items)
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
    std::fs::remove_file(manifest).unwrap();
    assert!(
        app.forge_items_digest(items)
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
    assert!(
        app.forge_items_material(items)
            .unwrap_err()
            .is::<ForgeMaterialUnavailable>()
    );
}
