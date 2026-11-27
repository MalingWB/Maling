//! Isolated public App/asset resolution. Never discover the workstation's inventory:
//! both selected ids route directly to one native reference / one synthetic CLI registry.
use camino::{Utf8Path, Utf8PathBuf};
use gold_band::{
    app::{
        App,
        forge_service::{ForgeMaterialCatalog, ForgeSourceItem},
    },
    config::SkillSource::{Global, Project},
    skill::claude_plugin::{PluginProvenance, resolve_plugin_assets, select_workbuddy_directory},
};
use serde_json::json;

fn write(root: &Utf8Path, file: &str, content: &str) {
    let file = root.join(file);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, content).unwrap();
}
const SKILL: &str = "---\nname: probe\ndescription: Fixture\n---\nRead the probe definition.";

#[test]
fn buddy_import_blockers_are_package_relative_and_do_not_authorize_import() {
    use gold_band::skill::{
        SKILL_BUNDLE_MAX_BUNDLE_BYTES, SKILL_BUNDLE_MAX_DEPTH, SKILL_BUNDLE_MAX_FILE_BYTES,
        SKILL_BUNDLE_MAX_FILES, claude_plugin::scan_codebuddy_plugins,
    };
    let tmp = tempfile::tempdir().unwrap();
    let store = Utf8PathBuf::from_path_buf(tmp.path().canonicalize().unwrap()).unwrap();
    let pkg = store.join("cache/probe");
    write(&pkg, ".codebuddy-plugin/plugin.json", r#"{"name":"probe"}"#);
    write(
        &store,
        "installed_plugins.json",
        &json!({"plugins":{"probe":[{"scope":"user","installPath":pkg}]}}).to_string(),
    );
    write(&pkg, "commands/cmd.md", "Command");
    write(&pkg, "agents/agent.md", "Agent");
    let skill = pkg.join("skills/probe");
    write(&skill, "SKILL.md", SKILL);
    let list = || scan_codebuddy_plugins(&store, None).pop().unwrap();
    let clean = list();
    for asset in &clean.assets {
        assert!(
            serde_json::to_value(asset)
                .unwrap()
                .get("importBlocker")
                .is_none()
        );
    }
    let assert_blocked = |expected: serde_json::Value| {
        let row = list();
        assert_eq!(row.assets.len(), 3);
        let asset = row.assets.iter().find(|a| a.name == "probe").unwrap();
        assert!(!asset.importable && row.declarative_skill_files(asset).is_none());
        assert_eq!(
            serde_json::to_value(asset).unwrap()["importBlocker"],
            expected
        );
    };
    for (path, blocker) in [
        (
            "scripts/run.txt",
            json!({"code":"unsupported-directory", "path":"scripts"}),
        ),
        (
            "references/sample.bin",
            json!({"code":"unsupported-file", "path":"references/sample.bin"}),
        ),
        (
            "references/no-extension",
            json!({"code":"unsupported-file", "path":"references/no-extension"}),
        ),
        (
            "references/misleading\u{202e}.bin",
            json!({"code":"unsupported-file"}),
        ),
        (
            "assets/.private",
            json!({"code":"hidden-entry", "path":"assets/.private"}),
        ),
        (
            "references/a/b/c/note.md",
            json!({"code":"depth-limit", "path":"references/a/b/c", "limit":SKILL_BUNDLE_MAX_DEPTH}),
        ),
    ] {
        write(&skill, path, "inert");
        assert_blocked(blocker);
        std::fs::remove_dir_all(skill.join(path.split('/').next().unwrap())).unwrap();
    }
    let data = skill.join("sample.txt");
    std::fs::File::create(&data)
        .unwrap()
        .set_len(SKILL_BUNDLE_MAX_FILE_BYTES + 1)
        .unwrap();
    assert_blocked(
        json!({"code":"file-size-limit", "path":"sample.txt", "limit":SKILL_BUNDLE_MAX_FILE_BYTES}),
    );
    // A timestamp the existing revision format cannot represent is not a package limit.
    std::fs::File::create(&data)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH - std::time::Duration::from_secs(60))
        .unwrap();
    assert_blocked(json!({"code":"inspection-incomplete", "path":"sample.txt"}));
    std::fs::remove_file(data).unwrap();
    let refs = skill.join("references");
    std::fs::create_dir(&refs).unwrap();
    for i in 0..SKILL_BUNDLE_MAX_BUNDLE_BYTES.div_ceil(SKILL_BUNDLE_MAX_FILE_BYTES) {
        std::fs::File::create(refs.join(format!("{i}.txt")))
            .unwrap()
            .set_len(SKILL_BUNDLE_MAX_FILE_BYTES)
            .unwrap();
    }
    // Aggregate limits describe the bundle, not whichever entry the filesystem yielded last.
    assert_blocked(json!({"code":"bundle-size-limit","limit":SKILL_BUNDLE_MAX_BUNDLE_BYTES}));
    std::fs::remove_dir_all(&refs).unwrap();
    for i in 0..SKILL_BUNDLE_MAX_FILES {
        write(&refs, &format!("{i}.txt"), "");
    }
    assert_blocked(json!({"code":"file-count-limit","limit":SKILL_BUNDLE_MAX_FILES}));
    #[cfg(unix)]
    {
        std::fs::remove_dir_all(&refs).unwrap();
        std::os::unix::fs::symlink("not-a-target", skill.join("link.txt")).unwrap();
        assert_blocked(json!({"code":"special-entry", "path":"link.txt"}));
        std::fs::remove_file(skill.join("link.txt")).unwrap();
        write(&skill, "unsafe\nname.txt", "inert");
        assert_blocked(json!({"code":"inspection-incomplete"}));
    }
}

fn isolated_desktop_child(name: &str) -> Option<Utf8PathBuf> {
    const CHILD: &str = "GOLD_BAND_DESKTOP_DISCOVERY_TEST";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        let root =
            Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap().canonicalize().unwrap())
                .unwrap();
        assert!(root.file_name().unwrap().starts_with("gb-desktop-test-"));
        return Some(root);
    }
    let tmp = tempfile::Builder::new()
        .prefix("gb-desktop-test-")
        .tempdir()
        .unwrap();
    let home = tmp.path().canonicalize().unwrap().join("home");
    let status = gold_band::process::background_command(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name)
        .env("GOLD_BAND_HOME", tmp.path().join("app-home"))
        .env("USERPROFILE", &home)
        .env("HOME", &home)
        .current_dir(tmp.path())
        .status()
        .unwrap();
    assert!(status.success());
    None
}

// Create an inert version-resource fixture. It is never launched. Use the OS metadata
// reader just as the native picker does, without inspecting any actual installation.
#[cfg(windows)]
#[test]
fn recognized_desktop_installation_picker_rechecks_product_and_skill_identity() {
    let Some(root) = isolated_desktop_child(
        "recognized_desktop_installation_picker_rechecks_product_and_skill_identity",
    ) else {
        return;
    };
    let install = root.join("installation");
    let bundle = install.join("resources/app.asar.unpacked/resources/plugins/workbuddy-builtin");
    write(&bundle, "skills/expert-manager/SKILL.md", SKILL);
    write(&bundle, "skills/expert-manager/tools/note.txt", "inert");
    write(&bundle, "commands/ignored.md", "Not a bundled Skill");
    let shell = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let status = gold_band::process::background_command(shell)
        .env("DESKTOP_FIXTURE", dunce::simplified(install.as_std_path()))
        .args(["-NoProfile", "-NonInteractive", "-Command",
          "$ErrorActionPreference='Stop'; $n = [Reflection.AssemblyName]::new('Fixture'); $a = [AppDomain]::CurrentDomain.DefineDynamicAssembly($n, 'Save', $env:DESKTOP_FIXTURE); $null = $a.DefineDynamicModule('Fixture','WorkBuddy.exe'); $a.DefineVersionInfoResource('WorkBuddy','1.2.3.4','Tencent Technology (Shenzhen) Company Limited','',''); $a.Save('WorkBuddy.exe')"])
        .status().unwrap();
    assert!(status.success());
    let app = App::new(root.join("project"));
    let automatic = gold_band::skill::claude_plugin::scan_workbuddy_desktop(
        &app.paths,
        false,
        &root.join("home"),
        &[install.clone(), install.clone()],
    );
    assert_eq!(automatic.len(), 1);
    assert_eq!(automatic[0].provenance, PluginProvenance::DesktopBundled);
    assert_eq!(
        (automatic[0].source, automatic[0].can_forget),
        (Global, false)
    );
    assert_eq!(automatic[0].version, "1.2.3.4");
    let selected = select_workbuddy_directory(&app.paths, Project, &install)
        .expect("verified WorkBuddy installation root exposes its bundled Skills");
    assert!(selected.can_forget && selected.plugin_id != automatic[0].plugin_id);
    assert_eq!(
        serde_json::to_value(selected.provenance).unwrap(),
        "desktop-bundled"
    );
    assert_eq!(selected.assets.len(), 1);
    for row in [&automatic[0], &selected] {
        assert!(!row.assets[0].importable);
        assert_eq!(
            serde_json::to_value(&row.assets[0]).unwrap()["importBlocker"],
            json!({"code":"unsupported-directory", "path":"tools"})
        );
    }
    let id = selected.assets[0].asset_id.as_str();
    assert_eq!(
        resolve_plugin_assets(&app.paths, Project, [id])[0].assets[0]
            .content
            .as_deref(),
        Some(SKILL)
    );
    std::fs::write(install.join("WorkBuddy.exe"), "not a WorkBuddy product").unwrap();
    assert!(resolve_plugin_assets(&app.paths, Project, [id]).is_empty());
}

#[test]
fn desktop_registry_revalidates_registered_version_scope_and_declared_material() {
    use gold_band::skill::claude_plugin::{forget_workbuddy_directory, scan_workbuddy_desktop};
    let Some(base) = isolated_desktop_child(
        "desktop_registry_revalidates_registered_version_scope_and_declared_material",
    ) else {
        return;
    };
    let app = App::new(base.join("project"));
    let home = base.join("home");
    let store = home.join(".workbuddy/plugins");
    let pkg = store.join("cache/my-experts/geo-content-team/1.1.23");
    let manifest = r#"{"name":"geo-content-team","version":"1.1.23","skills":["./declared"],"agents":[],"commands":[]}"#;
    let fixture = |root: &Utf8Path| {
        write(root, ".codebuddy-plugin/plugin.json", manifest);
        write(root, "declared/expert/SKILL.md", SKILL);
        write(
            root,
            "declared/expert/references/example.md",
            "inert registry fixture",
        );
        write(
            root,
            "skills/wrong/SKILL.md",
            &SKILL.replace("probe", "not selected"),
        );
    };
    fixture(&pkg);
    fixture(&store.join("cache/my-experts/geo-content-team/1.1.22"));
    let record = json!({"scope":"user","version":"1.1.23","installPath":pkg});
    let register = |version, records: serde_json::Value| {
        write(
            &store,
            "installed_plugins.json",
            &json!({"version":version,"plugins":{"geo-content-team@my-experts":records}})
                .to_string(),
        )
    };
    let list = || scan_workbuddy_desktop(&app.paths, true, &home, &[]);
    register(2, json!(vec![record.clone(); 300]));
    assert_eq!(
        list().len(),
        1,
        "surplus records cannot erase the valid prefix"
    );
    let row = list().pop().unwrap();
    assert_eq!(row.provenance, PluginProvenance::DesktopRegistry);
    assert_eq!(row.provider_id, "workbuddy");
    assert_eq!(row.source, Global);
    assert_eq!(row.assets.len(), 1);
    assert!(row.assets[0].content.is_none() && !row.can_forget);
    let item = ForgeSourceItem {
        catalog: ForgeMaterialCatalog::Plugin,
        source: Global,
        asset_id: row.assets[0].asset_id.clone(),
    };
    let items = std::slice::from_ref(&item);
    assert_eq!(app.forge_items_digest(items).unwrap()[0].name, "probe");
    assert!(
        app.forge_items_material(items)
            .unwrap()
            .content
            .contains("Read the probe definition.")
    );
    assert!(resolve_plugin_assets(&app.paths, Project, [item.asset_id.as_str()]).is_empty());
    let current = resolve_plugin_assets(&app.paths, Global, [item.asset_id.as_str()]);
    assert_eq!(
        current[0]
            .declarative_skill_files(&current[0].assets[0])
            .unwrap(),
        [(
            "references/example.md".into(),
            "inert registry fixture".into()
        )]
    );
    // Identical root + scope deduplicates, selections in another scope or at another root do not.
    let pin = select_workbuddy_directory(&app.paths, Global, &pkg).unwrap();
    assert_ne!(pin.plugin_id, row.plugin_id);
    assert_eq!(list().len(), 1);
    let project_pin = select_workbuddy_directory(&app.paths, Project, &pkg).unwrap();
    assert_eq!(list().len(), 2);
    let copy = base.join("copy");
    fixture(&copy);
    let copy_pin = select_workbuddy_directory(&app.paths, Global, &copy).unwrap();
    assert_eq!(list().len(), 3);
    assert!(forget_workbuddy_directory(&app.paths, Global, &copy_pin.plugin_id).unwrap());
    assert!(forget_workbuddy_directory(&app.paths, Project, &project_pin.plugin_id).unwrap());
    // Forgetting a pin never uninstalls the actual registered package.
    assert_eq!(list().len(), 1);
    write(
        &pkg,
        ".codebuddy-plugin/plugin.json",
        &manifest.replace("1.1.23", "1.2.0"),
    );
    assert!(list().is_empty() && app.forge_items_digest(items).is_err());
    write(&pkg, ".codebuddy-plugin/plugin.json", manifest);
    write(
        &pkg,
        "declared/expert/SKILL.md",
        &SKILL.replace("Read the probe definition.", "Different revision"),
    );
    assert!(app.forge_items_material(items).is_err());
    write(&pkg, "declared/expert/SKILL.md", SKILL);
    for records in [
        json!([]),
        json!([{"scope":"local","projectPath":app.paths.repo_root,"version":"1.1.23","installPath":pkg}]),
        json!([{"scope":"user","version":"1.1.23","installPath":copy}]),
        json!([{"scope":"user","projectPath":app.paths.repo_root,"version":"1.1.23","installPath":pkg}]),
        json!([{"scope":"user","version":"1.1.22","installPath":pkg}]),
    ] {
        register(2, records);
        assert!(list().is_empty());
        assert!(resolve_plugin_assets(&app.paths, Global, [item.asset_id.as_str()]).is_empty());
        assert!(app.forge_items_material(items).is_err());
    }
    register(1, json!([record]));
    assert!(list().is_empty());
}

#[test]
fn workbuddy_native_selection_resolves_only_current_owner_and_definition() {
    const CHILD: &str = "GOLD_BAND_WORKBUDDY_SYNTHETIC_CHILD";
    const PREFIX: &str = "gold-band-workbuddy-source-";
    if std::env::var_os(CHILD).is_none() {
        let tmp = tempfile::Builder::new().prefix(PREFIX).tempdir().unwrap();
        let result = gold_band::process::background_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workbuddy_native_selection_resolves_only_current_owner_and_definition",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("GOLD_BAND_HOME", tmp.path().join("app-home"))
            .env("CODEBUDDY_CONFIG_DIR", tmp.path().join("cli"))
            .current_dir(tmp.path())
            .status()
            .unwrap();
        assert!(result.success());
        return;
    }
    let base = Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap();
    assert!(base.file_name().unwrap().starts_with(PREFIX));
    let a_root = base.join("project-a");
    let b_root = base.join("project-b");
    std::fs::create_dir_all(&a_root).unwrap();
    std::fs::create_dir_all(&b_root).unwrap();
    let a = App::new(a_root.clone());
    let b = App::new(b_root);
    let expert = base.join("manual-expert");
    write(
        &expert,
        ".codebuddy-plugin/plugin.json",
        r#"{"name":"expert","skills":["./defs"],"agents":"./roles/auditor.md","hooks":{"ignored":true}}"#,
    );
    write(&expert, "skills/default/SKILL.md", SKILL);
    write(&expert, "defs/probe/SKILL.md", SKILL);
    write(&expert, "agents/default.md", "wrong default");
    write(
        &expert,
        "roles/auditor.md",
        "---\nname: auditor\n---\nOnly the selected Agent definition.",
    );
    write(
        &expert,
        "defs/probe/scripts/executable.ps1",
        "not a material",
    );
    let listed = select_workbuddy_directory(&a.paths, Project, &expert).unwrap();
    assert_eq!(
        (listed.provider_id, listed.provenance),
        ("workbuddy", PluginProvenance::ManualDirectory)
    );
    assert_eq!(
        listed
            .assets
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["probe", "auditor"]
    );
    assert!(!listed.assets[0].importable && listed.assets[1].importable);
    assert_eq!(
        serde_json::to_value(&listed.assets[0]).unwrap()["importBlocker"],
        json!({"code":"unsupported-directory", "path":"scripts"})
    );
    assert!(
        serde_json::to_value(&listed.assets[1])
            .unwrap()
            .get("importBlocker")
            .is_none()
    );
    let id = listed.assets[0].asset_id.clone();
    let item = ForgeSourceItem {
        source: Project,
        catalog: ForgeMaterialCatalog::Plugin,
        asset_id: id.clone(),
    };
    let items = std::slice::from_ref(&item);
    assert_eq!(a.forge_items_digest(items).unwrap()[0].name, "probe");
    let material = App::new(a_root)
        .forge_items_material(items)
        .unwrap()
        .content;
    assert!(
        material.contains("Read the probe definition.") && !material.contains("executable.ps1")
    );
    assert!(b.forge_items_digest(items).is_err());
    assert!(resolve_plugin_assets(&a.paths, Global, [id.as_str()]).is_empty());
    let global = select_workbuddy_directory(&a.paths, Global, &expert).unwrap();
    assert_ne!(global.plugin_id, listed.plugin_id);
    assert_eq!(
        resolve_plugin_assets(&b.paths, Global, [global.assets[0].asset_id.as_str()]).len(),
        1
    );
    // Foreign, escaped, broken and oversized inputs never replace the last good selection.
    let invalid = base.join("invalid");
    write(&invalid, "SKILL.md", SKILL);
    write(
        &invalid,
        ".claude-plugin/plugin.json",
        r#"{"name":"foreign"}"#,
    );
    assert!(select_workbuddy_directory(&a.paths, Project, &invalid).is_err());
    std::fs::remove_dir_all(invalid.join(".claude-plugin")).unwrap();
    write(
        &invalid,
        ".workbuddy-plugin/plugin.json",
        r#"{"name":"bad","skills":"../manual-expert/defs"}"#,
    );
    assert!(select_workbuddy_directory(&a.paths, Project, &invalid).is_err());
    write(&invalid, ".workbuddy-plugin/plugin.json", "broken");
    assert!(select_workbuddy_directory(&a.paths, Project, &invalid).is_err());
    std::fs::remove_dir_all(invalid.join(".workbuddy-plugin")).unwrap();
    write(
        &invalid,
        "SKILL.md",
        &"x".repeat(1 + gold_band::skill::SKILL_BUNDLE_MAX_FILE_BYTES as usize),
    );
    assert!(select_workbuddy_directory(&a.paths, Project, &invalid).is_err());
    assert!(a.forge_items_material(items).is_ok());
    write(
        &expert,
        "defs/probe/SKILL.md",
        &SKILL.replace("probe definition", "changed definition"),
    );
    assert!(a.forge_items_digest(items).is_err());
    let fresh = select_workbuddy_directory(&a.paths, Project, &expert).unwrap();
    assert_eq!(fresh.plugin_id, listed.plugin_id);
    assert_ne!(fresh.assets[0].asset_id, id);
    let loose = base.join("independent");
    write(&loose, "SKILL.md", SKILL);
    write(&loose, "references/example.md", "reference fixture");
    write(&loose, "assets/sample.csv", "name,value");
    let loose = select_workbuddy_directory(&a.paths, Project, &loose).unwrap();
    assert_ne!(loose.plugin_id, listed.plugin_id);
    assert!(
        resolve_plugin_assets(&a.paths, Project, [fresh.assets[0].asset_id.as_str()]).is_empty()
    );
    let selected = resolve_plugin_assets(&a.paths, Project, [loose.assets[0].asset_id.as_str()]);
    assert_eq!(selected[0].assets[0].content.as_deref(), Some(SKILL));
    assert_eq!(
        selected[0]
            .declarative_skill_files(&selected[0].assets[0])
            .unwrap(),
        [
            ("assets/sample.csv".to_owned(), "name,value".to_owned()),
            (
                "references/example.md".to_owned(),
                "reference fixture".to_owned()
            )
        ]
    );
    assert!(
        serde_json::to_value(&selected[0]).unwrap()["assets"][0]
            .get("content")
            .is_none()
    );
    std::fs::remove_file(base.join("independent/SKILL.md")).unwrap();
    assert!(
        resolve_plugin_assets(&a.paths, Project, [loose.assets[0].asset_id.as_str()]).is_empty()
    );
    use gold_band::skill::claude_plugin::{forget_workbuddy_directory, workbuddy_source};
    let stale = workbuddy_source(&a.paths, Project).unwrap();
    assert!(stale.unavailable && stale.assets.is_empty());
    assert!(!forget_workbuddy_directory(&a.paths, Project, &listed.plugin_id).unwrap());
    assert!(forget_workbuddy_directory(&a.paths, Project, &stale.plugin_id).unwrap());
    assert!(!forget_workbuddy_directory(&a.paths, Project, &stale.plugin_id).unwrap());
    assert!(workbuddy_source(&a.paths, Project).is_none());
    assert!(base.join("independent/references/example.md").is_file());
    assert!(workbuddy_source(&a.paths, Global).is_some());
    // A same-name CLI installation is not evidence of a WorkBuddy installation.
    let registry = base.join("cli/plugins");
    let installed = registry.join("cache/expert");
    write(
        &installed,
        ".codebuddy-plugin/plugin.json",
        r#"{"name":"expert"}"#,
    );
    write(&installed, "skills/probe/SKILL.md", SKILL);
    let register = |owner: &Utf8Path| {
        write(&registry,"installed_plugins.json",&json!({"plugins":{"expert":[{"scope":"local","projectPath":owner,"installPath":installed},{"scope":"user","installPath":installed},{"installPath":installed}]}}).to_string())
    };
    register(&a.paths.repo_root);
    use gold_band::skill::claude_plugin::scan_codebuddy_plugins;
    let cli = scan_codebuddy_plugins(&registry, Some(&a.paths.repo_root));
    assert_eq!(cli.len(), 2);
    let cli_project = cli.iter().find(|v| v.source == Project).unwrap();
    assert_eq!(
        (cli_project.provider_id, cli_project.provenance),
        ("codebuddy-code", PluginProvenance::HostInventory)
    );
    assert_eq!(cli_project.name, listed.name);
    assert_ne!(cli_project.plugin_id, listed.plugin_id);
    assert_ne!(
        cli_project.plugin_id,
        cli.iter().find(|v| v.source == Global).unwrap().plugin_id
    );
    let cli_item = ForgeSourceItem {
        asset_id: cli_project.assets[0].asset_id.clone(),
        ..item.clone()
    };
    let cli_items = std::slice::from_ref(&cli_item);
    assert!(
        a.forge_items_material(cli_items)
            .unwrap()
            .content
            .contains("Read the probe definition.")
    );
    assert!(b.forge_items_digest(cli_items).is_err());
    assert!(resolve_plugin_assets(&a.paths, Global, [cli_item.asset_id.as_str()]).is_empty());
    register(&b.paths.repo_root);
    assert!(b.forge_items_digest(cli_items).is_err());
    assert!(a.forge_items_digest(cli_items).is_err());
    assert!(resolve_plugin_assets(&a.paths, Project, [id.as_str()]).is_empty());
}
