use camino::{Utf8Path, Utf8PathBuf};
use gold_band::{
    config::SkillSource,
    skill::claude_plugin::{
        CLAUDE_PLUGIN_CACHE_DIR_ENV, scan_claude_plugins, scan_selected_plugins,
    },
};
use serde_json::json;

fn install(cache: &Utf8Path, name: &str) -> Utf8PathBuf {
    let root = cache.join(name);
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin/plugin.json"),
        json!({"name": name}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("agents")).unwrap();
    std::fs::write(root.join("agents/notes.md"), "Notes fixture").unwrap();
    root
}

#[test]
fn registry_scope_follows_the_registered_project() {
    // Child-only override: exercise the same resolver used by import and send without
    // changing the test process environment or resolving a Windows known-folder home.
    const CHILD: &str = "GOLD_BAND_REGISTRY_FIXTURE_CHILD";
    const PREFIX: &str = "gold-band-plugin-scope-";
    if std::env::var_os(CHILD).is_none() {
        let temp = tempfile::Builder::new().prefix(PREFIX).tempdir().unwrap();
        let status = gold_band::process::background_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "registry_scope_follows_the_registered_project",
                "--nocapture",
            ])
            .current_dir(temp.path())
            .env(CHILD, "1")
            .env(
                CLAUDE_PLUGIN_CACHE_DIR_ENV,
                temp.path().join("home/.claude/plugins"),
            )
            .status()
            .unwrap();
        assert!(
            status.success(),
            "temporary registry fixture failed: {status}"
        );
        return;
    }
    let base = Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap();
    assert!(base.file_name().unwrap().starts_with(PREFIX));
    assert!(std::fs::read_dir(&base).unwrap().next().is_none());
    let home = base.join("home");
    let registry = home.join(".claude/plugins");
    let cache = registry.join("cache");
    let first = base.join("first");
    let second = base.join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let shared = install(&cache, "shared");
    let notes = install(&cache, "notes");
    let review = install(&cache, "review");
    let pending = install(&cache, "pending");
    let registry_file = registry.join("installed_plugins.json");
    let mut data = json!({"version": 2, "plugins": {
        "shared@fixture": [{"scope":"user", "installPath":shared}],
        "notes@fixture": [{"scope":"project", "projectPath":first.join("."), "installPath":notes}],
        "review@fixture": [{"scope":"local", "projectPath":first, "installPath":review}],
        "pending@fixture": [{"scope":"project", "installPath":pending}]
    }});
    std::fs::write(&registry_file, data.to_string()).unwrap();
    let list = |project: Option<&Utf8Path>| scan_claude_plugins(&registry, &home, project);
    let listed = list(Some(&first));
    let rows: Vec<_> = listed.iter().map(|p| (p.name.as_str(), p.source)).collect();
    assert_eq!(
        rows,
        [
            ("notes", SkillSource::Project),
            ("review", SkillSource::Project),
            ("shared", SkillSource::Global)
        ]
    );
    for inventory in [list(Some(&second)), list(None)] {
        assert_eq!(inventory.len(), 1);
        assert_eq!(
            inventory[0].assets[0].asset_id,
            listed[2].assets[0].asset_id
        );
    }
    assert_eq!(
        Utf8Path::new(&std::env::var(CLAUDE_PLUGIN_CACHE_DIR_ENV).unwrap())
            .canonicalize_utf8()
            .unwrap(),
        registry.canonicalize_utf8().unwrap()
    );
    let id = &listed[0].assets[0].asset_id;
    let resolve = |project: &Utf8Path| scan_selected_plugins(Some(project), [id.as_str()]);
    let selected = resolve(&first);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].assets[0].asset_id, *id);
    assert!(resolve(&second).is_empty());
    // A cache directory can be installed for two distinct projects. Its asset identity
    // must stay bound to the installation's owner, not just the shared cache bytes.
    data["plugins"]["notes@fixture"][0]["projectPath"] = json!(second);
    std::fs::write(&registry_file, data.to_string()).unwrap();
    assert_ne!(list(Some(&second))[0].assets[0].asset_id, *id);
    assert!(resolve(&first).is_empty());
    assert!(resolve(&second).is_empty());
}
