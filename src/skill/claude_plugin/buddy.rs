//! CodeBuddy is a format/engine, not an installation owner. CLI records and native
//! WorkBuddy directory selections have separate authority, identities and lookup paths.
use super::*;
use crate::skill::{
    SKILL_BUNDLE_MAX_BUNDLE_BYTES, SKILL_BUNDLE_MAX_FILE_BYTES, SKILL_BUNDLE_MAX_FILES,
};
use crate::storage::{GoldBandPaths, normalize_workspace_path};
use cap_std::{ambient_authority, fs::Dir};
use serde::Deserialize;
use std::io::Read;

const MANUAL: &str = "workbuddy";
const CLI: &str = "codebuddy-code";
const CLI_PREFIX: &str = "buddy-cli-";
const MANUAL_PREFIX: &str = "buddy-manual-";
pub(super) const BUNDLED_PREFIX: &str = "buddy-bundled-";
pub(super) const REGISTERED_PREFIX: &str = "buddy-registered-";

#[derive(Clone, Copy)]
pub(super) enum Origin<'a> {
    Manual,
    Cli,
    Registered { name: &'a str, version: &'a str },
    Bundled { version: &'a str },
}
impl Origin<'_> {
    fn identity(self) -> (&'static str, &'static str, PluginProvenance) {
        match self {
            Self::Manual => (MANUAL, MANUAL_PREFIX, PluginProvenance::ManualDirectory),
            Self::Cli => (CLI, CLI_PREFIX, PluginProvenance::HostInventory),
            Self::Registered { .. } => {
                (MANUAL, REGISTERED_PREFIX, PluginProvenance::DesktopRegistry)
            }
            Self::Bundled { .. } => (MANUAL, BUNDLED_PREFIX, PluginProvenance::DesktopBundled),
        }
    }
}
const BUDDY_MANIFESTS: [&str; 2] = [
    ".codebuddy-plugin/plugin.json",
    ".workbuddy-plugin/plugin.json",
];
const CLAUDE_MANIFEST: &str = ".claude-plugin/plugin.json";

/// A private, single active native selection per owner. Do not store authority in a
/// checked-out project's files or in importable user configuration.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    root: Utf8PathBuf,
    plugin_id: String,
    name: String,
}

fn read_selection(file: &Utf8Path) -> Option<Selection> {
    let store = Dir::open_ambient_dir(file.parent()?, ambient_authority()).ok()?;
    serde_json::from_str(&bounded_file(
        &store,
        Utf8Path::new(file.file_name()?),
        64 * 1024,
    )?)
    .ok()
}

pub fn forget_workbuddy_directory(
    paths: &GoldBandPaths,
    scope: SkillSource,
    plugin_id: &str,
) -> anyhow::Result<bool> {
    let Some(file) = owner_file(paths, scope) else {
        return Ok(false);
    };
    crate::storage::with_file_lock(&file, || {
        if !read_selection(&file).is_some_and(|v| v.plugin_id == plugin_id) {
            return Ok(false);
        }
        std::fs::remove_file(&file)?;
        Ok(true)
    })
}

fn owner_file(paths: &GoldBandPaths, scope: SkillSource) -> Option<Utf8PathBuf> {
    let owner = match scope {
        SkillSource::Global => "global".to_owned(),
        SkillSource::Project => format!(
            "project-{}",
            blake3::hash(paths.project_id.as_bytes()).to_hex()
        ),
        SkillSource::BuiltIn => return None,
    };
    let state = paths.user_state_file();
    state.is_absolute().then(|| {
        state
            .with_file_name("workbuddy-sources")
            .join(format!("{owner}.json"))
    })
}

/// Reopen a canonical absolute directory without following a replaced ancestor.
/// Pin each directory handle before moving to its child (junctions are reparse points).
pub(super) fn open_canonical_dir(path: &Utf8Path) -> Option<Dir> {
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    if !path.is_absolute() {
        return None;
    }
    let root = path.ancestors().last()?;
    let mut dir = Dir::open_ambient_dir(root, ambient_authority()).ok()?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    #[cfg(windows)]
    options.custom_flags(0x0200_0000 | 0x0020_0000); // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
    for component in path.strip_prefix(root).ok()?.components() {
        let file = dir.open_with(component.as_str(), &options).ok()?;
        let meta = file.metadata().ok()?;
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return None;
            } // FILE_ATTRIBUTE_REPARSE_POINT
        }
        if !meta.is_dir() {
            return None;
        }
        dir = Dir::from_std_file(file.into_std());
    }
    Some(dir)
}

pub(super) fn bounded_file(dir: &Dir, rel: &Utf8Path, limit: u64) -> Option<String> {
    let meta = dir.symlink_metadata(rel).ok()?;
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // OPEN_REPARSE_POINT: validate the opened entry itself.
    }
    let file = dir.open_with(rel, &options).ok()?;
    let meta = file.metadata().ok()?;
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return None;
        }
    }
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

pub fn select_workbuddy_directory(
    paths: &GoldBandPaths,
    scope: SkillSource,
    native_path: &Utf8Path,
) -> anyhow::Result<InstalledPlugin> {
    let unavailable = || anyhow::anyhow!("plugin.asset-unavailable");
    let file = owner_file(paths, scope).ok_or_else(unavailable)?;
    let root = native_path.canonicalize_utf8()?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(root.as_std_path().components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::VerbatimDisk(_) | Prefix::Disk(_)))
        {
            return Err(unavailable());
        }
    }
    let owner = (scope == SkillSource::Project).then_some(paths.project_id.as_str());
    let mut plugin = if super::workbuddy_desktop::is_install_candidate(&root) {
        super::workbuddy_desktop::bundled(&root, scope, owner, None)
    } else {
        read_source(&root, Origin::Manual, scope, owner, None)
    }
    .ok_or_else(unavailable)?;
    plugin.can_forget = true;
    crate::storage::with_file_lock(&file, || {
        crate::storage::write_json(
            &file,
            &Selection {
                root,
                plugin_id: plugin.plugin_id.clone(),
                name: plugin.display_name.clone(),
            },
        )
    })?;
    Ok(plugin)
}

fn selected_manual(
    paths: &GoldBandPaths,
    scope: SkillSource,
    filter: InstallationFilter<'_>,
) -> Option<InstalledPlugin> {
    let file = owner_file(paths, scope)?;
    let grant = read_selection(&file)?;
    if filter.is_some_and(|f| !f.contains_key(grant.plugin_id.as_str())) {
        return None;
    }
    if grant.root.is_absolute()
        && grant
            .root
            .canonicalize_utf8()
            .is_ok_and(|p| p == grant.root)
    {
        let owner = (scope == SkillSource::Project).then_some(paths.project_id.as_str());
        let current = if grant.plugin_id.starts_with(BUNDLED_PREFIX) {
            super::workbuddy_desktop::bundled(&grant.root, scope, owner, filter)
        } else {
            read_source(&grant.root, Origin::Manual, scope, owner, filter)
        };
        if let Some(mut p) = current.filter(|p| p.plugin_id == grant.plugin_id) {
            p.can_forget = true;
            return Some(p);
        }
    }
    // Unresolved references remain visible only for revocation, never for dispatch.
    filter.is_none().then(|| InstalledPlugin {
        provenance: if grant.plugin_id.starts_with(BUNDLED_PREFIX) {
            PluginProvenance::DesktopBundled
        } else {
            PluginProvenance::ManualDirectory
        },
        can_forget: true,
        unavailable: true,
        provider_id: MANUAL,
        display_name: grant.name.clone(),
        name: grant.name,
        plugin_id: grant.plugin_id,
        install_path: String::new(),
        description: String::new(),
        version: String::new(),
        source: scope,
        assets: vec![],
        has_hooks: false,
        has_mcp_servers: false,
    })
}

pub fn workbuddy_source(paths: &GoldBandPaths, scope: SkillSource) -> Option<InstalledPlugin> {
    selected_manual(paths, scope, None)
}

pub fn list_plugin_inventory(paths: &GoldBandPaths, include_project: bool) -> Vec<InstalledPlugin> {
    let mut plugins = scan_installed_plugins(include_project.then_some(&paths.repo_root));
    plugins.extend(super::workbuddy_desktop::inventory(paths, include_project));
    plugins
}

/// Route opaque ids only to their owning source, including at send/import time.
/// Resolving a manual selection does not enumerate a single host directory.
pub fn resolve_plugin_assets<'a>(
    paths: &GoldBandPaths,
    scope: SkillSource,
    ids: impl IntoIterator<Item = &'a str>,
) -> Vec<InstalledPlugin> {
    if scope == SkillSource::BuiltIn {
        return vec![];
    }
    let project = (scope == SkillSource::Project).then_some(paths.repo_root.as_path());
    let mut manual = BTreeMap::<_, BTreeSet<_>>::new();
    let mut cli = BTreeMap::<_, BTreeSet<_>>::new();
    let mut registered = BTreeMap::<_, BTreeSet<_>>::new();
    let mut bundled = BTreeMap::<_, BTreeSet<_>>::new();
    let mut others = Vec::new();
    for id in ids {
        let Some((installation, _)) = id.split_once(':') else {
            continue;
        };
        if installation.starts_with(MANUAL_PREFIX) {
            manual.entry(installation).or_default().insert(id);
        } else if installation.starts_with(REGISTERED_PREFIX) {
            registered.entry(installation).or_default().insert(id);
        } else if installation.starts_with(BUNDLED_PREFIX) {
            bundled.entry(installation).or_default().insert(id);
        } else if installation.starts_with(CLI_PREFIX) {
            cli.entry(installation).or_default().insert(id);
        } else {
            others.push(id);
        }
    }
    let mut plugins = scan_selected_plugins(project, others);
    if !manual.is_empty() {
        plugins.extend(selected_manual(paths, scope, Some(&manual)));
    }
    if scope == SkillSource::Global && !registered.is_empty() {
        if let Some(home) = super::workbuddy_desktop::desktop_home() {
            plugins.extend(super::workbuddy_desktop::registered(
                &home,
                Some(&registered),
            ));
        }
    }
    if !bundled.is_empty() {
        if let Some(pin) = selected_manual(paths, scope, Some(&bundled)) {
            bundled.remove(pin.plugin_id.as_str());
            plugins.push(pin);
        }
        if scope == SkillSource::Global && !bundled.is_empty() {
            plugins.extend(super::workbuddy_desktop::installed_bundles(Some(&bundled)));
        }
    }
    if !cli.is_empty() {
        let config = std::env::var("CODEBUDDY_CONFIG_DIR")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .map(Utf8PathBuf::from)
            .or_else(|| {
                Utf8PathBuf::from_path_buf(dirs::home_dir()?)
                    .ok()
                    .map(|p| p.join(".codebuddy"))
            });
        if let Some(config) = config {
            plugins.extend(scan_cli_cache(
                Some(&cli),
                &config.join("plugins"),
                project,
                scope == SkillSource::Global,
            ));
        }
    }
    plugins.retain(|p| p.source == scope);
    plugins
}

pub(super) fn scan_cli_cache(
    filter: InstallationFilter<'_>,
    store: &Utf8Path,
    project: Option<&Utf8Path>,
    include_global: bool,
) -> Vec<InstalledPlugin> {
    let load = || -> Option<Vec<_>> {
        let dir = Dir::open_ambient_dir(store, ambient_authority()).ok()?;
        let registry: Value = serde_json::from_str(&bounded_file(
            &dir,
            Utf8Path::new(INSTALLED_PLUGINS_FILE),
            SKILL_BUNDLE_MAX_FILE_BYTES,
        )?)
        .ok()?;
        if registry
            .get("version")
            .is_some_and(|v| v.as_u64() != Some(2))
        {
            return None;
        }
        let cache = store.join("cache").canonicalize_utf8().ok()?;
        if !cache.starts_with(store.canonicalize_utf8().ok()?) {
            return None;
        }
        let current = project.and_then(|p| p.canonicalize_utf8().ok());
        let owner = current
            .as_deref()
            .map(crate::storage::project_id_for_workspace);
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for (idx, rec) in registry_records(&registry).enumerate() {
            if idx >= SKILL_BUNDLE_MAX_FILES {
                return None;
            }
            let (scope, project_id) = match rec["scope"].as_str() {
                Some("user") if include_global && rec["projectPath"].is_null() => {
                    (SkillSource::Global, None)
                }
                Some("project" | "local")
                    if current.is_some()
                        && rec["projectPath"]
                            .as_str()
                            .map(Utf8Path::new)
                            .filter(|p| p.is_absolute())
                            .and_then(|p| p.canonicalize_utf8().ok())
                            == current =>
                {
                    (SkillSource::Project, owner.as_deref())
                }
                _ => continue,
            };
            let Some(root) = rec["installPath"]
                .as_str()
                .map(Utf8Path::new)
                .filter(|p| p.is_absolute())
                .and_then(|p| p.canonicalize_utf8().ok())
            else {
                continue;
            };
            if root == cache
                || !root.starts_with(&cache)
                || !seen.insert((scope == SkillSource::Project, root.clone()))
            {
                continue;
            }
            if let Some(plugin) = read_source(&root, Origin::Cli, scope, project_id, filter) {
                result.push(plugin);
            }
        }
        Some(result)
    };
    load().unwrap_or_default()
}

pub(super) fn relative_path(value: &str) -> Option<Utf8PathBuf> {
    let value = value
        .strip_prefix("./")
        .unwrap_or(value)
        .trim_end_matches('/');
    if value.is_empty()
        || value.len() > 1024
        || value.split('/').count() > 16
        || value.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.ends_with([' ', '.'])
                || p.chars()
                    .any(|c| c.is_control() || "\\<>:\"|?*".contains(c))
        })
    {
        return None;
    }
    Some(value.into())
}

/// Strict string/array overrides replace (never augment) defaults. Traverse only
/// immediate documented component directories; budget includes unrelated entries.
fn targets(manifest: &Value, key: &str) -> Option<Vec<Utf8PathBuf>> {
    match manifest.get(key) {
        None => Some(vec![key.into()]),
        Some(Value::String(p)) => Some(relative_path(p).into_iter().collect()),
        Some(Value::Array(v)) if v.len() <= SKILL_BUNDLE_MAX_FILES => Some(
            v.iter()
                .filter_map(|p| relative_path(p.as_str()?))
                .collect(),
        ),
        Some(Value::Array(_)) => None, // declaration budget exceeded
        _ => Some(vec![]),             // malformed override cannot grant default components
    }
}

pub(super) fn read_source(
    root: &Utf8Path,
    origin: Origin<'_>,
    scope: SkillSource,
    owner: Option<&str>,
    filter: InstallationFilter<'_>,
) -> Option<InstalledPlugin> {
    read_source_with_budget(
        root,
        origin,
        scope,
        owner,
        filter,
        &mut (SKILL_BUNDLE_MAX_BUNDLE_BYTES + SKILL_BUNDLE_MAX_FILE_BYTES),
    )
}

pub(super) fn read_source_with_budget(
    root: &Utf8Path,
    origin: Origin<'_>,
    scope: SkillSource,
    owner: Option<&str>,
    filter: InstallationFilter<'_>,
    total_bytes_left: &mut u64,
) -> Option<InstalledPlugin> {
    let (provider, prefix, provenance) = origin.identity();
    let dir = open_canonical_dir(root)?;
    let manual = matches!(origin, Origin::Manual);
    let installation = blake3::hash(
        &serde_json::to_vec(&(provider, scope, owner, normalize_workspace_path(root))).ok()?,
    );
    let plugin_id = format!("{prefix}{}", installation.to_hex());
    let wanted = match filter {
        Some(f) => Some(f.get(plugin_id.as_str())?),
        None => None,
    };
    let mut manifest = None;
    let bundle = matches!(origin, Origin::Bundled { .. });
    for name in BUDDY_MANIFESTS
        .into_iter()
        .chain([CLAUDE_MANIFEST])
        .filter(|_| !bundle)
    {
        // A directory advertising a package must not fall back to a loose SKILL.md
        // when its manifest is invalid, unsupported, or inaccessible.
        match dir.symlink_metadata(Utf8Path::new(name).parent()?) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(meta) if meta.is_dir() => (),
            _ => return None,
        }
        if manifest.is_some() || (!matches!(origin, Origin::Cli) && name == CLAUDE_MANIFEST) {
            return None;
        }
        let raw = bounded_file(
            &dir,
            Utf8Path::new(name),
            SKILL_BUNDLE_MAX_FILE_BYTES.min(*total_bytes_left),
        )?;
        *total_bytes_left -= raw.len() as u64;
        let value: Value = serde_json::from_str(&raw).ok()?;
        let name = value["name"].as_str()?.trim();
        if name.is_empty()
            || name.len() > 256
            || [
                ("displayName", 256),
                ("description", 4096),
                ("version", 128),
            ]
            .iter()
            .any(|(key, max)| {
                value
                    .get(key)
                    .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= *max))
            })
        {
            return None;
        }
        manifest = Some((value, blake3::hash(raw.as_bytes())));
    }
    if let Origin::Registered { name, version } = origin {
        let value = &manifest.as_ref()?.0;
        if value["name"].as_str() != Some(name) || value["version"].as_str() != Some(version) {
            return None;
        }
    }
    // The verified desktop distribution has a Skill collection, not a plugin manifest.
    // Never infer commands, agents, hooks, MCP, or active status from this directory.
    let standalone = manifest.is_none() && !bundle;
    if standalone
        && (!manual
            || [
                "plugin.json",
                ".codex-plugin",
                ".cursor-plugin",
                "gemini-extension.json",
                "qwen-extension.json",
            ]
            .iter()
            .any(|p| dir.symlink_metadata(p).is_ok()))
    {
        return None;
    }
    let (manifest, manifest_rev) = manifest.unwrap_or_else(|| {
        let identity = match origin {
            Origin::Bundled { version } => version.as_bytes(),
            _ => b"standalone-skill",
        };
        (Value::Null, blake3::hash(identity))
    });
    let mut docs = BTreeSet::new();
    let mut entries_left = SKILL_BUNDLE_MAX_FILES;
    if standalone {
        docs.insert((ClaudePluginAssetKind::Skill, Utf8PathBuf::from(SKILL_FILE)));
    } else {
        for (key, kind) in [
            ("skills", ClaudePluginAssetKind::Skill),
            ("agents", ClaudePluginAssetKind::Agent),
            ("commands", ClaudePluginAssetKind::Command),
        ] {
            if bundle && kind != ClaudePluginAssetKind::Skill {
                continue;
            }
            for rel in targets(&manifest, key)? {
                let Ok(meta) = dir.symlink_metadata(&rel) else {
                    continue;
                };
                if meta.is_file()
                    && kind != ClaudePluginAssetKind::Skill
                    && has_markdown_extension(rel.as_std_path())
                {
                    docs.insert((kind, rel));
                    continue;
                }
                if !meta.is_dir() {
                    continue;
                }
                if !bundle
                    && kind == ClaudePluginAssetKind::Skill
                    && dir.symlink_metadata(rel.join(SKILL_FILE)).is_ok()
                {
                    docs.insert((kind, rel.join(SKILL_FILE)));
                    continue;
                }
                for entry in dir.read_dir(&rel).ok()? {
                    if entries_left == 0 && bundle {
                        break;
                    }
                    entries_left = entries_left.checked_sub(1)?;
                    let entry = entry.ok()?;
                    let Some(child) = entry.file_name().to_str().and_then(relative_path) else {
                        continue;
                    };
                    let child = rel.join(child);
                    let ty = entry.file_type().ok()?;
                    if kind == ClaudePluginAssetKind::Skill
                        && ty.is_dir()
                        && dir.symlink_metadata(child.join(SKILL_FILE)).is_ok()
                    {
                        docs.insert((kind, child.join(SKILL_FILE)));
                    } else if kind != ClaudePluginAssetKind::Skill
                        && ty.is_file()
                        && has_markdown_extension(child.as_std_path())
                    {
                        docs.insert((kind, child));
                    }
                }
            }
        }
    }
    if docs.len() > SKILL_BUNDLE_MAX_FILES {
        return None;
    }
    let mut bytes_left = SKILL_BUNDLE_MAX_BUNDLE_BYTES.min(*total_bytes_left);
    let mut support_entries_left = SKILL_BUNDLE_MAX_FILES * 4;
    let mut assets = Vec::new();
    for (kind, rel) in docs {
        let locator =
            blake3::hash(&serde_json::to_vec(&(kind, rel.as_str().replace('\\', "/"))).ok()?);
        let prefix = format!("{plugin_id}:{}:", locator.to_hex());
        if wanted.is_some_and(|w| {
            !w.range(prefix.as_str()..)
                .next()
                .is_some_and(|id| id.starts_with(&prefix))
        }) {
            continue;
        }
        let Some(raw) = bounded_file(&dir, &rel, SKILL_BUNDLE_MAX_FILE_BYTES.min(bytes_left))
        else {
            continue;
        };
        bytes_left -= raw.len() as u64;
        *total_bytes_left -= raw.len() as u64;
        let Ok(doc) = (if kind == ClaudePluginAssetKind::Skill {
            crate::frontmatter::parse_frontmatter_document(&raw)
        } else {
            crate::frontmatter::parse_optional_frontmatter_document(&raw)
        }) else {
            continue;
        };
        let skill_dir = rel.parent().unwrap_or(Utf8Path::new(""));
        let fallback = if kind == ClaudePluginAssetKind::Skill {
            skill_dir.file_name().or(root.file_name())?
        } else {
            rel.file_stem()?
        };
        let name = doc
            .fields
            .get("name")
            .map(String::as_str)
            .unwrap_or(fallback)
            .trim();
        let description = doc
            .fields
            .get("description")
            .map(String::as_str)
            .unwrap_or_default()
            .trim();
        if name.is_empty()
            || name.len() > 256
            || description.len() > 4096
            || doc.body.trim().is_empty()
            || (kind == ClaudePluginAssetKind::Skill
                && (!doc.fields.contains_key("name") || description.is_empty()))
        {
            continue;
        }
        let (support_revision, import_blocker) = if kind == ClaudePluginAssetKind::Skill {
            use super::buddy_support::{InspectionError, inspect};
            match inspect(&dir, skill_dir, raw.len(), &mut support_entries_left, false) {
                Ok(support) => (Some(support.revision), None),
                Err(InspectionError::Blocked(reason)) => (None, Some(reason)),
                Err(InspectionError::SharedBudgetExhausted) => (None, None),
            }
        } else {
            (None, None)
        };
        // A large bundled collection may exhaust the aggregate sidecar budget. Keep
        // already verified definitions; do not issue an identity for a partial inspection.
        if bundle && support_entries_left == 0 {
            break;
        }
        let mut revision = blake3::Hasher::new();
        revision.update(manifest_rev.as_bytes());
        revision.update(raw.as_bytes());
        if let Some(hash) = support_revision {
            revision.update(hash.as_bytes());
        }
        let asset_id = format!("{prefix}{}", revision.finalize().to_hex());
        if wanted.is_some_and(|w| !w.contains(asset_id.as_str())) {
            continue;
        }
        assets.push(ClaudePluginAsset {
            asset_id,
            kind,
            support_revision,
            import_blocker,
            importable: kind == ClaudePluginAssetKind::Agent || support_revision.is_some(),
            name: name.to_owned(),
            description: description.to_owned(),
            path: Some(
                root.join(if kind == ClaudePluginAssetKind::Skill {
                    skill_dir
                } else {
                    &rel
                })
                .to_string(),
            ),
            content: wanted.map(|_| raw),
        });
    }
    if (!bundle && support_entries_left == 0)
        || ((!matches!(origin, Origin::Cli) || wanted.is_some()) && assets.is_empty())
    {
        return None;
    }
    let name = if bundle {
        "workbuddy-builtin"
    } else if standalone {
        assets.first()?.name.as_str()
    } else {
        manifest["name"].as_str()?
    }
    .to_owned();
    Some(InstalledPlugin {
        unavailable: false,
        provenance,
        can_forget: manual,
        provider_id: provider,
        display_name: manifest["displayName"].as_str().unwrap_or(&name).into(),
        description: manifest["description"]
            .as_str()
            .or_else(|| {
                assets
                    .first()
                    .filter(|_| standalone)
                    .map(|a| a.description.as_str())
            })
            .unwrap_or("")
            .into(),
        version: match origin {
            Origin::Bundled { version } => version,
            _ => manifest["version"].as_str().unwrap_or(""),
        }
        .into(),
        name,
        plugin_id,
        install_path: root.to_string(),
        source: scope,
        assets,
        has_hooks: manifest.get("hooks").is_some(),
        has_mcp_servers: manifest.get("mcpServers").is_some(),
    })
}

pub(super) fn definition_only(plugin: &InstalledPlugin) -> bool {
    matches!(plugin.provider_id, CLI | MANUAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workbuddy_definition_and_support_are_independently_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(temp.path().canonicalize().unwrap()).unwrap();
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            ".codebuddy-plugin/plugin.json",
            r#"{"name":"expert","skills":["./bundles","../outside"],"agents":["./invalid-agent.md"]}"#,
        );
        write(
            "bundles/a/SKILL.md",
            "---\nname: alpha\ndescription: alpha\n---\nRead references/sample.md",
        );
        write("bundles/a/references/sample.md", "sample");
        write("bundles/invalid/SKILL.md", "broken");
        write(
            "bundles/b/SKILL.md",
            "---\nname: beta\ndescription: beta\n---\nBeta",
        );
        write("bundles/b/scripts/do.cmd", "fixture only");
        let list = || {
            read_source(
                &root,
                Origin::Manual,
                SkillSource::Project,
                Some("project-a"),
                None,
            )
            .unwrap()
        };
        let row = list();
        assert_eq!(row.assets.len(), 2);
        assert!(row.assets[0].importable && !row.assets[1].importable);
        let id = row.assets[0].asset_id.as_str();
        let filter = BTreeMap::from([(row.plugin_id.as_str(), BTreeSet::from([id]))]);
        let selected = || {
            read_source(
                &root,
                Origin::Manual,
                SkillSource::Project,
                Some("project-a"),
                Some(&filter),
            )
        };
        let item = selected().unwrap();
        assert_eq!(item.assets.len(), 1);
        assert_eq!(
            item.declarative_skill_files(&item.assets[0]).unwrap(),
            [("references/sample.md".into(), "sample".into())]
        );
        write("bundles/a/references/sample.md", "new content");
        assert!(selected().is_none());
        assert!(item.declarative_skill_files(&item.assets[0]).is_none());
    }
}
