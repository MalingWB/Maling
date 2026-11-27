//! Claude Code plugin 发现：把已安装 plugin 的 skill / command / agent 作为
//! 工作流铸造的素材来源。
//!
//! 只读扫描，不修改 `~/.claude` 下任何文件。发现来源：
//! - `<plugins 根>/installed_plugins.json` 记录的每个安装（取其 `installPath`）；
//! - `<plugins 根>/synced/` 下从 claude.ai 同步的 plugin（按标准布局识别根目录）；
//! - skills 目录 plugin：`~/.claude/skills/*/` 与项目 `.claude/skills/*/` 下带
//!   `.claude-plugin/plugin.json` 的目录。
//!
//! plugin 根内的组件按官方标准布局解析，并遵循清单声明：`skills` 在默认目录
//! 之外追加，`commands` / `agents` 替换默认目录；声明在 plugin 根之外的路径
//! 一律忽略。hooks 与 MCP server 不是流程素材，只上报是否存在。

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::Value;

use super::SkillSource;
mod buddy;
mod buddy_support;
mod workbuddy_desktop;
pub use buddy::{
    forget_workbuddy_directory, list_plugin_inventory, resolve_plugin_assets,
    select_workbuddy_directory, workbuddy_source,
};
pub use workbuddy_desktop::scan_workbuddy_desktop;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginProvenance {
    HostInventory,
    ManualDirectory,
    DesktopBundled,
    DesktopRegistry,
}

/// Claude Code 官方的 plugins 根目录覆盖变量。
pub const CLAUDE_PLUGIN_CACHE_DIR_ENV: &str = "CLAUDE_CODE_PLUGIN_CACHE_DIR";

const INSTALLED_PLUGINS_FILE: &str = "installed_plugins.json";
const SYNCED_DIR: &str = "synced";
const MANIFEST_DIR: &str = ".claude-plugin";
const MANIFEST_FILE: &str = "plugin.json";
const SKILL_FILE: &str = "SKILL.md";
/// synced/ 的目录布局没有公开约定，按标准布局在有限深度内识别 plugin 根。
const SYNCED_SEARCH_MAX_DEPTH: usize = 4;
const RECURSIVE_SCAN_MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClaudePluginAssetKind {
    Skill,
    Command,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImportBlockerCode {
    UnsupportedDirectory,
    UnsupportedFile,
    HiddenEntry,
    SpecialEntry,
    DepthLimit,
    FileSizeLimit,
    BundleSizeLimit,
    FileCountLimit,
    InspectionIncomplete,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportBlocker {
    pub code: ImportBlockerCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginAsset {
    pub asset_id: String,
    pub kind: ClaudePluginAssetKind,
    pub importable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_blocker: Option<ImportBlocker>,
    #[serde(skip)]
    support_revision: Option<blake3::Hash>,
    pub name: String,
    pub description: String,
    /// 文件型素材：skill 为其所在目录，command / agent 为 Markdown 文件。
    #[serde(skip)]
    pub path: Option<String>,
    /// 清单内联 command（`commands.<name>.content`）：没有文件，正文随清单返回。
    #[serde(skip)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub provenance: PluginProvenance,
    pub can_forget: bool,
    pub unavailable: bool,
    pub provider_id: &'static str,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub plugin_id: String,
    #[serde(skip)]
    pub install_path: String,
    /// 已验证归属的 project/local 安装及项目 `.claude/skills/` 属于项目。
    pub source: SkillSource,
    pub assets: Vec<ClaudePluginAsset>,
    pub has_hooks: bool,
    pub has_mcp_servers: bool,
}

fn installation_id(provider: &str, source: SkillSource, root: &Utf8Path) -> String {
    let root = crate::storage::normalize_workspace_path(root);
    blake3::hash(
        &serde_json::to_vec(&(provider, source, root)).expect("installation locator serializes"),
    )
    .to_hex()
    .to_string()
}

type InstallationFilter<'a> = Option<&'a BTreeMap<&'a str, BTreeSet<&'a str>>>;
fn includes_installation(
    filter: InstallationFilter<'_>,
    provider: &str,
    source: SkillSource,
    root: &Utf8Path,
) -> bool {
    filter.is_none_or(|ids| ids.contains_key(installation_id(provider, source, root).as_str()))
}

/// Establish material identity before loading Markdown; aliases are manifest keys, not display names.
struct AssetReader<'a> {
    root: &'a Utf8Path,
    plugin_id: String,
    filter: InstallationFilter<'a>,
}
impl AssetReader<'_> {
    fn id(
        &self,
        kind: ClaudePluginAssetKind,
        path: Option<&Utf8Path>,
        alias: Option<&str>,
    ) -> Option<String> {
        let relative = match path {
            Some(p) => Some(
                p.strip_prefix(self.root)
                    .ok()?
                    .as_str()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
            ),
            None => None,
        };
        let local = blake3::hash(
            &serde_json::to_vec(&(kind, relative, alias)).expect("asset locator serializes"),
        );
        let id = format!("{}:{}", self.plugin_id, local.to_hex());
        self.filter
            .is_none_or(|filter| {
                filter
                    .get(self.plugin_id.as_str())
                    .is_some_and(|ids| ids.contains(id.as_str()))
            })
            .then_some(id)
    }
    fn file(
        &self,
        kind: ClaudePluginAssetKind,
        path: &Utf8Path,
        default_name: &str,
        alias: Option<&str>,
    ) -> Option<ClaudePluginAsset> {
        let asset_id = self.id(kind, Some(path), alias)?;
        let doc = if kind == ClaudePluginAssetKind::Skill {
            let file = path.join(SKILL_FILE);
            if !is_local(path, &file) {
                return None;
            }
            file
        } else {
            path.to_owned()
        };
        let (name, description, _) = read_markdown(self.root, &doc, default_name)?;
        Some(ClaudePluginAsset {
            asset_id,
            kind,
            importable: kind != ClaudePluginAssetKind::Command,
            import_blocker: None,
            support_revision: None,
            name: alias.map(str::to_owned).unwrap_or(name),
            description,
            path: Some(path.to_string()),
            content: None,
        })
    }
}

impl InstalledPlugin {
    /// Read a selected document only after reconfirming its canonical location under the installation.
    pub fn asset_path(&self, asset: &ClaudePluginAsset) -> Option<Utf8PathBuf> {
        let root = Utf8Path::new(&self.install_path).canonicalize_utf8().ok()?;
        let path = Utf8Path::new(asset.path.as_deref()?)
            .canonicalize_utf8()
            .ok()?;
        if !path.starts_with(&root) {
            return None;
        }
        if asset.kind == ClaudePluginAssetKind::Skill {
            let doc = path.join(SKILL_FILE).canonicalize_utf8().ok()?;
            if doc.parent() != Some(path.as_path()) || !doc.is_file() {
                return None;
            }
        } else if !path.is_file() {
            return None;
        }
        Some(path)
    }

    /// Buddy selections carry just the verified definition bytes, never its surrounding bundle.
    pub fn definition_only(&self) -> bool {
        buddy::definition_only(self)
    }

    /// Revalidate the same dedicated Skill directory before copying its inert support files.
    pub fn declarative_skill_files(
        &self,
        asset: &ClaudePluginAsset,
    ) -> Option<Vec<(String, String)>> {
        if !self.definition_only()
            || !asset.importable
            || asset.kind != ClaudePluginAssetKind::Skill
        {
            return None;
        }
        let root = Utf8Path::new(&self.install_path);
        let rel = Utf8Path::new(asset.path.as_deref()?)
            .strip_prefix(root)
            .ok()?;
        let dir = buddy::open_canonical_dir(root)?;
        let content = asset.content.as_ref()?;
        if buddy::bounded_file(
            &dir,
            &rel.join(SKILL_FILE),
            super::SKILL_BUNDLE_MAX_FILE_BYTES,
        )?
        .as_str()
            != content
        {
            return None;
        }
        let bundle = buddy_support::inspect(
            &dir,
            rel,
            content.len(),
            &mut (super::SKILL_BUNDLE_MAX_FILES * 4),
            true,
        )
        .ok()?;
        (Some(bundle.revision) == asset.support_revision).then_some(bundle.files)
    }

    pub fn asset_body(&self, asset: &ClaudePluginAsset) -> Option<String> {
        if self.definition_only() {
            return crate::frontmatter::parse_optional_frontmatter_document(
                asset.content.as_deref()?,
            )
            .ok()
            .map(|doc| doc.body);
        }
        if let Some(body) = &asset.content {
            return Some(body.clone());
        }
        let path = self.asset_path(asset)?;
        let path = if asset.kind == ClaudePluginAssetKind::Skill {
            path.join(SKILL_FILE)
        } else {
            path
        };
        read_markdown(Utf8Path::new(&self.install_path), &path, &asset.name)
            .map(|(_, _, body)| body)
    }
}

/// Claude Code 的 plugins 根目录：`CLAUDE_CODE_PLUGIN_CACHE_DIR` 优先，
/// 否则 `<home>/.claude/plugins`。
pub fn claude_plugins_root(home: &Utf8Path) -> Utf8PathBuf {
    claude_plugins_root_with_override(
        home,
        std::env::var(CLAUDE_PLUGIN_CACHE_DIR_ENV).ok().as_deref(),
    )
}

pub fn claude_plugins_root_with_override(
    home: &Utf8Path,
    override_dir: Option<&str>,
) -> Utf8PathBuf {
    override_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Utf8PathBuf::from)
        .unwrap_or_else(|| home.join(".claude").join("plugins"))
}

/// 扫描全部可发现的 plugin，按 plugin 名排序。
pub fn scan_claude_plugins(
    plugins_root: &Utf8Path,
    home: &Utf8Path,
    project_root: Option<&Utf8Path>,
) -> Vec<InstalledPlugin> {
    scan_claude_plugins_filtered(plugins_root, home, project_root, None)
}

fn scan_claude_plugins_filtered(
    plugins_root: &Utf8Path,
    home: &Utf8Path,
    project_root: Option<&Utf8Path>,
    filter: InstallationFilter<'_>,
) -> Vec<InstalledPlugin> {
    let mut roots = installed_plugin_roots(plugins_root, project_root, true);

    let synced = plugins_root.join(SYNCED_DIR);
    if synced.is_dir() {
        for child in subdirs(&synced) {
            collect_synced_roots(&child, 0, &mut roots);
        }
    }

    roots.extend(skill_plugin_roots(home, SkillSource::Global));
    if let Some(project) = project_root {
        roots.extend(skill_plugin_roots(project, SkillSource::Project));
    }

    scan_roots(roots, filter)
}

fn scan_roots(roots: Vec<PluginRoot>, filter: InstallationFilter<'_>) -> Vec<InstalledPlugin> {
    let mut seen = BTreeSet::new();
    let mut plugins: Vec<_> = roots
        .into_iter()
        .filter(|p| filter.is_none_or(|ids| ids.contains_key(p.plugin_id.as_str())))
        .filter(|p| seen.insert(p.plugin_id.clone()) && p.root.is_dir())
        .map(|p| scan_root(p, filter))
        .collect();
    plugins.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.install_path.cmp(&right.install_path))
    });
    plugins
}

fn skill_plugin_roots(base: &Utf8Path, source: SkillSource) -> Vec<PluginRoot> {
    let store = base.join(".claude/skills");
    if !is_local(base, &store) {
        return Vec::new();
    }
    subdirs(&store)
        .into_iter()
        .filter(|dir| {
            let manifest = dir.join(MANIFEST_DIR).join(MANIFEST_FILE);
            is_local(&store, dir) && is_local(dir, &manifest) && manifest.is_file()
        })
        .map(|root| PluginRoot::new(root, source))
        .collect()
}

#[cfg(test)]
pub(crate) fn scan_project_plugins(project: &Utf8Path) -> Vec<InstalledPlugin> {
    scan_roots(skill_plugin_roots(project, SkillSource::Project), None)
}

/// Re-discover installation roots in the selected scope. Only matching installations
/// have their manifests/assets loaded; supplied IDs are compared, never converted to paths.
pub fn scan_selected_plugins<'a>(
    project: Option<&Utf8Path>,
    ids: impl IntoIterator<Item = &'a str>,
) -> Vec<InstalledPlugin> {
    let mut installations = BTreeMap::<_, BTreeSet<_>>::new();
    for id in ids {
        if let Some((install, _)) = id.split_once(':') {
            installations.entry(install).or_default().insert(id);
        }
    }
    if installations.is_empty() {
        return Vec::new();
    }
    match project {
        Some(project) => {
            let mut roots = Vec::new();
            if installations
                .keys()
                .any(|id| id.starts_with(PROJECT_REGISTRY_PREFIX))
            {
                // A project-scoped registry selection requires only the Claude registry, not
                // the other user inventories. Resolve the explicit override before looking up home.
                let registry = std::env::var(CLAUDE_PLUGIN_CACHE_DIR_ENV).ok();
                let registry = registry
                    .as_deref()
                    .map(str::trim)
                    .filter(|d| !d.is_empty())
                    .map(Utf8PathBuf::from)
                    .or_else(|| {
                        dirs::home_dir()
                            .and_then(|p| Utf8PathBuf::from_path_buf(p).ok())
                            .map(|h| claude_plugins_root_with_override(&h, None))
                    });
                if let Some(registry) = registry {
                    roots.extend(installed_plugin_roots(&registry, Some(project), false));
                }
            }
            if installations
                .keys()
                .any(|id| !id.starts_with(PROJECT_REGISTRY_PREFIX))
            {
                roots.extend(skill_plugin_roots(project, SkillSource::Project));
            }
            scan_roots(roots, Some(&installations))
        }
        None => scan_installed_plugins_filtered(None, Some(&installations)),
    }
}

fn is_local(root: &Utf8Path, path: &Utf8Path) -> bool {
    let (Ok(root), Ok(path)) = (root.canonicalize_utf8(), path.canonicalize_utf8()) else {
        return false;
    };
    path.starts_with(root)
}

/// 一个待解析的 plugin 根目录。
struct PluginRoot {
    root: Utf8PathBuf,
    source: SkillSource,
    plugin_id: String,
    /// 目录名不能代表 plugin 名时的名称提示（如 synced 下的版本号目录）。
    name_hint: Option<String>,
}

impl PluginRoot {
    fn new(root: Utf8PathBuf, source: SkillSource) -> Self {
        let plugin_id = installation_id("claude-acp", source, &root);
        Self {
            root,
            source,
            plugin_id,
            name_hint: None,
        }
    }
}

/// 解析一个 plugin 根目录。
pub fn scan_plugin_root(root: &Utf8Path, source: SkillSource) -> InstalledPlugin {
    scan_root(PluginRoot::new(root.to_owned(), source), None)
}

fn scan_root(entry: PluginRoot, filter: InstallationFilter<'_>) -> InstalledPlugin {
    let root = entry.root.as_path();
    let source = entry.source;
    let name_hint = entry.name_hint.as_deref();
    let reader = AssetReader {
        root,
        plugin_id: entry.plugin_id,
        filter,
    };
    let manifest = read_manifest(root);
    let manifest_text = |key: &str| {
        manifest
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let dir_name = root.file_name().unwrap_or("plugin").to_string();
    let fallback_name = name_hint
        .filter(|hint| !hint.is_empty())
        .unwrap_or(&dir_name);
    let name = manifest_text("name").unwrap_or_else(|| fallback_name.to_string());
    let mut assets: Vec<ClaudePluginAsset> = Vec::new();

    // skills：默认目录之外追加清单声明的目录。
    let default_skills_dir = root.join("skills");
    if default_skills_dir.is_dir() {
        collect_skills(&reader, &default_skills_dir, &mut assets);
    }
    let declared_skills = manifest.as_ref().and_then(|value| value.get("skills"));
    let mut extra_skill_dirs: Vec<String> = Vec::new();
    match declared_skills {
        Some(Value::String(path)) => extra_skill_dirs.push(path.clone()),
        Some(Value::Array(paths)) => {
            extra_skill_dirs.extend(paths.iter().filter_map(Value::as_str).map(str::to_string))
        }
        _ => {}
    }
    for relative in extra_skill_dirs {
        let Some(dir) = contained_path(root, &relative) else {
            continue;
        };
        if dir.is_dir() {
            collect_skills(&reader, &dir, &mut assets);
        }
    }
    // 根目录自带 SKILL.md、没有 skills/ 也没有清单声明时，整个 plugin 是单个 skill。
    if !default_skills_dir.exists() && declared_skills.is_none() && root.join(SKILL_FILE).is_file()
    {
        assets.extend(reader.file(ClaudePluginAssetKind::Skill, root, &name, None));
    }

    // commands：清单声明替换默认目录；对象形式支持 source 文件与 content 内联。
    let mut command_files: Vec<(Utf8PathBuf, String)> = Vec::new();
    match manifest.as_ref().and_then(|value| value.get("commands")) {
        None => command_files.extend(markdown_targets(root, "commands")),
        Some(Value::String(path)) => command_files.extend(markdown_targets(root, path)),
        Some(Value::Array(paths)) => {
            for path in paths.iter().filter_map(Value::as_str) {
                command_files.extend(markdown_targets(root, path));
            }
        }
        Some(Value::Object(map)) => {
            for (command_name, spec) in map {
                let fields = spec.as_object();
                let inline = fields
                    .and_then(|value| value.get("content"))
                    .and_then(Value::as_str);
                let source_path = fields
                    .and_then(|value| value.get("source"))
                    .and_then(Value::as_str);
                let declared_description = fields
                    .and_then(|value| value.get("description"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match (inline, source_path) {
                    (Some(content), _) => {
                        if let Some(asset_id) =
                            reader.id(ClaudePluginAssetKind::Command, None, Some(command_name))
                        {
                            assets.push(ClaudePluginAsset {
                                asset_id,
                                kind: ClaudePluginAssetKind::Command,
                                importable: false,
                                import_blocker: None,
                                support_revision: None,
                                name: command_name.clone(),
                                description: declared_description.trim().to_string(),
                                path: None,
                                content: Some(content.to_string()),
                            });
                        }
                    }
                    (None, Some(path)) => {
                        if let Some((file, _)) = markdown_targets(root, path).into_iter().next() {
                            assets.extend(reader.file(
                                ClaudePluginAssetKind::Command,
                                &file,
                                command_name,
                                Some(command_name),
                            ));
                        }
                    }
                    (None, None) => {}
                }
            }
        }
        _ => {}
    }
    for (file, default_name) in command_files {
        assets.extend(reader.file(ClaudePluginAssetKind::Command, &file, &default_name, None));
    }

    // agents：清单声明替换默认目录；声明项必须是文件。
    let mut agent_files: Vec<(Utf8PathBuf, String)> = Vec::new();
    match manifest.as_ref().and_then(|value| value.get("agents")) {
        None if is_local(root, &root.join("agents")) => {
            agent_files.extend(recursive_markdown_files(&root.join("agents")))
        }
        Some(Value::String(path)) => {
            agent_files.extend(
                markdown_targets(root, path)
                    .into_iter()
                    .filter(|(file, _)| file.is_file()),
            );
        }
        Some(Value::Array(paths)) => {
            for path in paths.iter().filter_map(Value::as_str) {
                agent_files.extend(
                    markdown_targets(root, path)
                        .into_iter()
                        .filter(|(file, _)| file.is_file()),
                );
            }
        }
        _ => {}
    }
    for (file, default_name) in agent_files {
        assets.extend(reader.file(ClaudePluginAssetKind::Agent, &file, &default_name, None));
    }

    assets.sort_by(|left, right| (left.kind, &left.name).cmp(&(right.kind, &right.name)));
    InstalledPlugin {
        provenance: PluginProvenance::HostInventory,
        can_forget: false,
        unavailable: false,
        provider_id: "claude-acp",
        display_name: manifest_text("displayName").unwrap_or_else(|| name.clone()),
        description: manifest_text("description").unwrap_or_default(),
        version: manifest_text("version").unwrap_or_default(),
        has_hooks: root.join("hooks").join("hooks.json").is_file()
            || manifest_has_key(&manifest, "hooks"),
        has_mcp_servers: root.join(".mcp.json").is_file()
            || manifest_has_key(&manifest, "mcpServers"),
        name,
        install_path: root.to_string(),
        plugin_id: reader.plugin_id,
        source,
        assets,
    }
}

/// 扫描当前用户的 Claude Code plugin（plugins 根与 home 均按官方规则解析）。
pub fn scan_installed_plugins(project_root: Option<&Utf8Path>) -> Vec<InstalledPlugin> {
    scan_installed_plugins_filtered(project_root, None)
}
fn scan_installed_plugins_filtered(
    project_root: Option<&Utf8Path>,
    filter: InstallationFilter<'_>,
) -> Vec<InstalledPlugin> {
    let Some(home) = dirs::home_dir().and_then(|home| Utf8PathBuf::from_path_buf(home).ok()) else {
        return Vec::new();
    };
    let mut plugins =
        scan_claude_plugins_filtered(&claude_plugins_root(&home), &home, project_root, filter);
    let qwen = std::env::var("QWEN_HOME")
        .ok()
        .filter(|dir| !dir.is_empty())
        .map(Utf8PathBuf::from)
        .unwrap_or_else(|| home.join(".qwen"));
    plugins.extend(scan_extensions(
        filter,
        "qwen-code",
        &qwen.join("extensions"),
        &[
            "qwen-extension.json",
            ".claude-plugin/plugin.json",
            "plugin.json",
        ],
    ));
    plugins.extend(scan_extensions(
        filter,
        "gemini",
        &home.join(".gemini/extensions"),
        &["gemini-extension.json"],
    ));
    let config_root = |variable: &str, default: &str| {
        std::env::var(variable)
            .ok()
            .filter(|d| !d.trim().is_empty())
            .map(Utf8PathBuf::from)
            .unwrap_or_else(|| home.join(default))
    };
    let buddy_root = config_root("CODEBUDDY_CONFIG_DIR", ".codebuddy").join("plugins");
    plugins.extend(buddy::scan_cli_cache(
        filter,
        &buddy_root,
        project_root,
        true,
    ));
    plugins.extend(scan_codex_cache(
        filter,
        &config_root("CODEX_HOME", ".codex").join("plugins/cache"),
    ));
    plugins
}

/// Read only the supplied CLI registry/cache; useful without ambient host discovery.
pub fn scan_codebuddy_plugins(
    store: &Utf8Path,
    project: Option<&Utf8Path>,
) -> Vec<InstalledPlugin> {
    buddy::scan_cli_cache(None, store, project, true)
}

#[cfg(test)]
fn scan_codebuddy_cache(filter: InstallationFilter<'_>, store: &Utf8Path) -> Vec<InstalledPlugin> {
    buddy::scan_cli_cache(filter, store, None, true)
}

fn scan_codex_cache(filter: InstallationFilter<'_>, cache: &Utf8Path) -> Vec<InstalledPlugin> {
    // Upstream core-plugin-common/installed.rs: local wins, else compare by semver when both parse.
    subdirs(cache)
        .into_iter()
        .flat_map(|market| subdirs(&market))
        .filter_map(|base| {
            let mut versions = subdirs(&base);
            versions.retain(|p| {
                p.file_name().is_some_and(|s| {
                    s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.+".contains(&c))
                })
            });
            let version = versions
                .iter()
                .find(|p| p.file_name() == Some("local"))
                .or_else(|| {
                    versions.iter().max_by(|a, b| {
                        let (a, b) = (
                            a.file_name().unwrap_or_default(),
                            b.file_name().unwrap_or_default(),
                        );
                        match (semver::Version::parse(a), semver::Version::parse(b)) {
                            (Ok(a), Ok(b)) => a.cmp(&b),
                            _ => a.cmp(b),
                        }
                    })
                })?;
            read_extension(
                filter,
                "codex-acp",
                version,
                &[
                    "plugin.json",
                    ".codex-plugin/plugin.json",
                    ".claude-plugin/plugin.json",
                    ".cursor-plugin/plugin.json",
                ],
            )
        })
        .collect()
}

/// Only directory-backed installations and their independent Skill documents. Do not
/// execute extension modules, traverse marketplaces, or treat installed as enabled.
fn scan_extensions(
    filter: InstallationFilter<'_>,
    provider: &'static str,
    store: &Utf8Path,
    manifests: &[&str],
) -> Vec<InstalledPlugin> {
    subdirs(store)
        .into_iter()
        .filter_map(|root| read_extension(filter, provider, &root, manifests))
        .collect()
}

fn read_extension(
    filter: InstallationFilter<'_>,
    provider: &'static str,
    root: &Utf8Path,
    manifests: &[&str],
) -> Option<InstalledPlugin> {
    if !includes_installation(filter, provider, SkillSource::Global, root) {
        return None;
    }
    let reader = AssetReader {
        root,
        plugin_id: installation_id(provider, SkillSource::Global, root),
        filter,
    };
    let canonical = root.canonicalize_utf8().ok()?;
    let local = |path: &Utf8Path| {
        path.canonicalize_utf8()
            .ok()
            .is_some_and(|p| p.starts_with(&canonical))
    };
    let manifest = manifests.iter().find_map(|relative| {
        let file = root.join(relative);
        if !file.is_file() || !local(&file) || file.metadata().ok()?.len() > 1024 * 1024 {
            return None;
        }
        let value: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        if *relative == "plugin.json"
            && value["$schema"].as_str()
                != Some("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json")
        {
            return None;
        }
        value["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())?;
        Some(value)
    })?;
    let skills = root.join("skills");
    let mut assets = Vec::new();
    if skills.is_dir() && local(&skills) {
        for dir in subdirs(&skills) {
            assets.extend(reader.file(
                ClaudePluginAssetKind::Skill,
                &dir,
                dir.file_name().unwrap_or("skill"),
                None,
            ));
        }
    }
    let name = manifest["name"].as_str()?.to_owned();
    Some(InstalledPlugin {
        provenance: PluginProvenance::HostInventory,
        can_forget: false,
        unavailable: false,
        provider_id: provider,
        display_name: manifest["displayName"].as_str().unwrap_or(&name).to_owned(),
        name,
        description: manifest["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        version: manifest["version"].as_str().unwrap_or_default().to_owned(),
        install_path: root.to_string(),
        plugin_id: reader.plugin_id,
        source: SkillSource::Global,
        assets,
        has_hooks: manifest.get("hooks").is_some(),
        has_mcp_servers: manifest.get("mcpServers").is_some() || root.join("mcp.json").is_file(),
    })
}

fn manifest_has_key(manifest: &Option<Value>, key: &str) -> bool {
    manifest
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|map| map.contains_key(key))
}

fn read_manifest(root: &Utf8Path) -> Option<Value> {
    let file = root
        .join(MANIFEST_DIR)
        .join(MANIFEST_FILE)
        .canonicalize_utf8()
        .ok()?;
    if !is_local(root, &file)
        || !file.is_file()
        || file.metadata().ok()?.len() > super::SKILL_BUNDLE_MAX_FILE_BYTES
    {
        return None;
    }
    let raw = std::fs::read_to_string(file).ok()?;
    serde_json::from_str(&raw).ok()
}

const PROJECT_REGISTRY_PREFIX: &str = "project-registry-";

fn read_registry(store: &Utf8Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(store.join(INSTALLED_PLUGINS_FILE)).ok()?).ok()
}

fn registry_records(registry: &Value) -> impl Iterator<Item = &Value> {
    registry["plugins"]
        .as_object()
        .into_iter()
        .flat_map(|groups| groups.values())
        .filter_map(Value::as_array)
        .flatten()
}

/// An installed_plugins.json v2 record, not the location of its cached files, grants scope.
fn installed_plugin_roots(
    store: &Utf8Path,
    project: Option<&Utf8Path>,
    include_global: bool,
) -> Vec<PluginRoot> {
    let Some(registry) = read_registry(store).filter(|r| r["version"].as_u64() == Some(2)) else {
        return Vec::new();
    };
    let current = project
        .filter(|p| p.is_dir())
        .and_then(|p| p.canonicalize_utf8().ok());
    registry_records(&registry)
        .filter_map(|record| {
            let root = Utf8Path::new(record["installPath"].as_str()?);
            if !root.is_absolute() {
                return None;
            }
            match record["scope"].as_str()? {
                "user" | "managed" if include_global && record["projectPath"].is_null() => {
                    Some(PluginRoot::new(root.to_owned(), SkillSource::Global))
                }
                "project" | "local" => {
                    let current = current.as_ref()?;
                    let owner = Utf8Path::new(record["projectPath"].as_str()?);
                    if !owner.is_absolute()
                        || owner.canonicalize_utf8().ok()?.as_path() != current.as_path()
                    {
                        return None;
                    }
                    // Multiple projects can use the same cache directory. Bind the opaque id to
                    // the verified owner so a selection cannot follow a reassigned installation.
                    let canonical = root.canonicalize_utf8().ok()?;
                    let digest = blake3::hash(
                        &serde_json::to_vec(&(current, canonical))
                            .expect("installation locator serializes"),
                    );
                    Some(PluginRoot {
                        root: root.to_owned(),
                        source: SkillSource::Project,
                        plugin_id: format!("{PROJECT_REGISTRY_PREFIX}{}", digest.to_hex()),
                        name_hint: None,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn collect_synced_roots(dir: &Utf8Path, depth: usize, roots: &mut Vec<PluginRoot>) {
    if looks_like_plugin_root(dir) {
        let mut entry = PluginRoot::new(dir.to_owned(), SkillSource::Global);
        // `synced/<plugin>/<version>/` 形态下目录名是版本号，plugin 名在上一层。
        entry.name_hint = is_version_like(dir).then(|| parent_name(dir)).flatten();
        roots.push(entry);
        return;
    }
    if depth >= SYNCED_SEARCH_MAX_DEPTH {
        return;
    }
    for child in subdirs(dir) {
        collect_synced_roots(&child, depth + 1, roots);
    }
}

fn is_version_like(dir: &Utf8Path) -> bool {
    dir.file_name()
        .and_then(|name| name.chars().next())
        .is_some_and(|first| first.is_ascii_digit())
}

fn parent_name(dir: &Utf8Path) -> Option<String> {
    dir.parent()?.file_name().map(str::to_string)
}

fn looks_like_plugin_root(dir: &Utf8Path) -> bool {
    dir.join(MANIFEST_DIR).join(MANIFEST_FILE).is_file()
        || dir.join(SKILL_FILE).is_file()
        || dir.join(".mcp.json").is_file()
        || dir.join("hooks").join("hooks.json").is_file()
        || ["skills", "commands", "agents"]
            .iter()
            .any(|name| dir.join(name).is_dir())
}

fn subdirs(dir: &Utf8Path) -> Vec<Utf8PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir.as_std_path()) else {
        return Vec::new();
    };
    let mut dirs: Vec<Utf8PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| !is_symlink(path) && path.is_dir())
        .filter_map(|path| Utf8PathBuf::from_path_buf(path).ok())
        .collect();
    dirs.sort();
    dirs
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

/// 解析相对 plugin 根的声明路径；绝对路径与含 `..` 的路径在根之外，忽略。
fn contained_path(root: &Utf8Path, relative: &str) -> Option<Utf8PathBuf> {
    let relative = Path::new(relative.trim());
    // 官方清单路径以 `./` 开头，前导 `.` 不是越界；只拒绝根、盘符前缀与 `..`。
    if relative.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        return None;
    }
    Some(
        relative
            .components()
            .filter(|component| !matches!(component, Component::CurDir))
            .fold(root.to_path_buf(), |path, part| {
                path.join(part.as_os_str().to_string_lossy().to_string())
            }),
    )
}

fn collect_skills(reader: &AssetReader<'_>, dir: &Utf8Path, assets: &mut Vec<ClaudePluginAsset>) {
    if !is_local(reader.root, dir) {
        return;
    }
    let dirs = if dir.join(SKILL_FILE).is_file() {
        vec![dir.to_owned()]
    } else {
        subdirs(dir)
    };
    for dir in dirs {
        assets.extend(reader.file(
            ClaudePluginAssetKind::Skill,
            &dir,
            dir.file_name().unwrap_or("skill"),
            None,
        ));
    }
}

/// 声明路径指向单个 Markdown 文件时就是它本身，指向目录时取目录内的平铺 Markdown。
fn markdown_targets(root: &Utf8Path, relative: &str) -> Vec<(Utf8PathBuf, String)> {
    let Some(path) = contained_path(root, relative) else {
        return Vec::new();
    };
    if path.is_file() {
        if has_markdown_extension(path.as_std_path()) {
            let name = path.file_stem().unwrap_or("command").to_string();
            return vec![(path, name)];
        }
        return Vec::new();
    }
    if !is_local(root, &path) {
        return Vec::new();
    }
    flat_markdown_files(&path)
}

fn flat_markdown_files(dir: &Utf8Path) -> Vec<(Utf8PathBuf, String)> {
    let Ok(entries) = std::fs::read_dir(dir.as_std_path()) else {
        return Vec::new();
    };
    let mut files: Vec<(Utf8PathBuf, String)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && has_markdown_extension(path))
        .filter_map(|path| {
            let name = path.file_stem()?.to_str()?.to_string();
            Some((Utf8PathBuf::from_path_buf(path).ok()?, name))
        })
        .collect();
    files.sort();
    files
}

/// 递归收集 Markdown 文件；子目录路径是名称的一部分（官方 agents 约定）。
fn recursive_markdown_files(dir: &Utf8Path) -> Vec<(Utf8PathBuf, String)> {
    let mut files: Vec<(Utf8PathBuf, String)> = walkdir::WalkDir::new(dir)
        .max_depth(RECURSIVE_SCAN_MAX_DEPTH)
        .into_iter()
        .filter_map(walkdir::Result::ok)
        .filter(|entry| entry.file_type().is_file() && has_markdown_extension(entry.path()))
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(dir.as_std_path()).ok()?;
            let name = relative
                .with_extension("")
                .to_string_lossy()
                .chars()
                .map(|character| if character == '\\' { '/' } else { character })
                .collect();
            Some((
                Utf8PathBuf::from_path_buf(entry.path().to_path_buf()).ok()?,
                name,
            ))
        })
        .collect();
    files.sort();
    files
}

fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// 读取带可选 frontmatter 的 Markdown：`(name, description, 正文)`；
/// name 缺省取 `default_name`，frontmatter 损坏时按无 frontmatter 处理。
fn read_markdown(
    root: &Utf8Path,
    path: &Utf8Path,
    default_name: &str,
) -> Option<(String, String, String)> {
    let path = path.canonicalize_utf8().ok()?;
    if !is_local(root, &path)
        || !path.is_file()
        || path.metadata().ok()?.len() > super::SKILL_BUNDLE_MAX_FILE_BYTES
    {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    let document =
        crate::frontmatter::parse_optional_frontmatter_document(&raw).unwrap_or_else(|_| {
            crate::frontmatter::FrontmatterDocument {
                fields: Default::default(),
                field_sources: Default::default(),
                body: raw.clone(),
            }
        });
    let field = |key: &str| {
        document
            .fields
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Some((
        field("name").unwrap_or_else(|| default_name.to_string()),
        field("description").unwrap_or_default(),
        document.body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Utf8Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("path has a parent")).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn temp_root(temp: &tempfile::TempDir) -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(temp.path().to_path_buf()).unwrap()
    }

    #[test]
    fn extension_inventory_requires_host_manifest_and_preserves_provider() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp_root(&temp);
        write(
            &store.join("one/plugin.json"),
            r#"{"name":"one","$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"}"#,
        );
        write(
            &store.join("one/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: Test Skill\n---\nExample",
        );
        write(
            &store.join("two/plugin.json"),
            r#"{"name":"not-an-agent-plugin"}"#,
        );
        write(
            &store.join("three/gemini-extension.json"),
            r#"{"name":"three"}"#,
        );
        let qwen = scan_extensions(None, "qwen-code", &store, &["plugin.json"]);
        assert_eq!(qwen.len(), 1);
        assert_eq!(
            (qwen[0].provider_id, qwen[0].assets[0].name.as_str()),
            ("qwen-code", "demo")
        );
        let gemini = scan_extensions(None, "gemini", &store, &["gemini-extension.json"]);
        assert_eq!(gemini.len(), 1);
        assert_eq!(
            (gemini[0].provider_id, gemini[0].name.as_str()),
            ("gemini", "three")
        );
    }

    #[test]
    fn installed_cache_scan_uses_only_registered_or_active_versions() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp_root(&temp);
        for version in ["1.0.0", "2.0.0", "10.0.0", "local"] {
            write(
                &store.join(format!(
                    "cache/market/example/{version}/.codex-plugin/plugin.json"
                )),
                &format!(r#"{{"name":"example","version":"{version}"}}"#),
            );
        }
        assert_eq!(
            scan_codex_cache(None, &store.join("cache"))[0].version,
            "local"
        );
        std::fs::remove_dir_all(store.join("cache/market/example/local")).unwrap();
        assert_eq!(
            scan_codex_cache(None, &store.join("cache"))[0].version,
            "10.0.0"
        );
        let installed = store.join("cache/market/buddy/1");
        write(
            &installed.join(".codebuddy-plugin/plugin.json"),
            r#"{"name":"buddy"}"#,
        );
        assert!(scan_codebuddy_cache(None, &store).is_empty());
        write(
            &store.join(INSTALLED_PLUGINS_FILE),
            &serde_json::json!({"plugins":{"buddy@market":[{"scope":"user", "installPath":installed}]}})
                .to_string(),
        );
        assert_eq!(
            scan_codebuddy_cache(None, &store)[0].provider_id,
            "codebuddy-code"
        );
    }

    #[test]
    fn codebuddy_global_inventory_never_adopts_other_project() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp_root(&temp);
        let root = store.join("cache/fixture");
        write(
            &root.join(".codebuddy-plugin/plugin.json"),
            r#"{"name":"fixture"}"#,
        );
        write(
            &store.join(INSTALLED_PLUGINS_FILE),
            &serde_json::json!({"plugins":{"fixture":[
                {"installPath":root, "scope":"project", "projectPath":store}
            ]}})
            .to_string(),
        );
        assert!(
            scan_codebuddy_cache(None, &store).is_empty(),
            "project registration leaked into global inventory"
        );
    }

    #[test]
    fn codebuddy_custom_assets_replace_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp_root(&temp);
        let root = store.join("cache/fixture");
        write(
            &root.join(".codebuddy-plugin/plugin.json"),
            r#"{"name":"fixture","skills":"./custom/","agents":["./roles/review.md"]}"#,
        );
        write(&root.join("skills/obsolete/SKILL.md"), "obsolete");
        write(
            &root.join("custom/probe/SKILL.md"),
            "---\nname: probe\ndescription: Test\n---\nProbe",
        );
        write(
            &root.join("roles/review.md"),
            "---\nname: review\ndescription: Review\n---\nReview",
        );
        write(
            &store.join(INSTALLED_PLUGINS_FILE),
            &serde_json::json!({"plugins":{"fixture":[
                {"installPath":root, "scope":"user"}
            ]}})
            .to_string(),
        );
        let rows = scan_codebuddy_cache(None, &store);
        assert_eq!(
            names(&rows[0].assets, ClaudePluginAssetKind::Skill),
            ["probe"]
        );
        assert_eq!(
            names(&rows[0].assets, ClaudePluginAssetKind::Agent),
            ["review"]
        );
    }

    /// 官方标准布局的样本 plugin。
    fn fixture_plugin(root: &Utf8Path) {
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"deploy-tools","displayName":"Deploy Tools","version":"1.2.0","description":"Deployment tooling"}"#,
        );
        write(
            &root.join("skills/deploy/SKILL.md"),
            "---\nname: deploy\ndescription: Run a deployment\n---\nDeploy steps",
        );
        write(
            &root.join("skills/review/SKILL.md"),
            "---\ndescription: Review a deployment\n---\nReview steps",
        );
        write(
            &root.join("commands/status.md"),
            "---\ndescription: Show deployment status\n---\nStatus body",
        );
        write(&root.join("commands/about.md"), "About body");
        write(
            &root.join("agents/reviewer.md"),
            "---\nname: reviewer\ndescription: Reviews changes\ntools: Read\n---\nYou review changes.",
        );
        write(
            &root.join("agents/CI/smoke.md"),
            "---\ndescription: Smoke tests\n---\nSmoke agent",
        );
        write(&root.join("hooks/hooks.json"), "{}");
        write(&root.join(".mcp.json"), "{}");
    }

    fn names(assets: &[ClaudePluginAsset], kind: ClaudePluginAssetKind) -> Vec<String> {
        assets
            .iter()
            .filter(|asset| asset.kind == kind)
            .map(|asset| asset.name.clone())
            .collect()
    }

    fn plugin<'a>(plugins: &'a [InstalledPlugin], name: &str) -> &'a InstalledPlugin {
        plugins
            .iter()
            .find(|plugin| plugin.name == name)
            .unwrap_or_else(|| panic!("plugin `{name}` not found"))
    }

    #[test]
    fn standard_layout_lists_skills_commands_and_agents() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("deploy-tools");
        fixture_plugin(&root);

        let plugin = scan_plugin_root(&root, SkillSource::Global);

        assert_eq!(plugin.name, "deploy-tools");
        assert_eq!(plugin.display_name, "Deploy Tools");
        assert_eq!(plugin.version, "1.2.0");
        assert_eq!(plugin.description, "Deployment tooling");
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Skill),
            ["deploy", "review"]
        );
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Command),
            ["about", "status"]
        );
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Agent),
            ["CI/smoke", "reviewer"]
        );
        let reviewer = plugin
            .assets
            .iter()
            .find(|asset| asset.name == "reviewer")
            .unwrap();
        assert_eq!(reviewer.description, "Reviews changes");
        assert_eq!(
            reviewer.path.as_deref(),
            Some(root.join("agents").join("reviewer.md").to_string()).as_deref()
        );
        assert!(plugin.has_hooks);
        assert!(plugin.has_mcp_servers);
    }

    #[test]
    fn selected_project_inventory_contains_only_the_requested_assets() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp_root(&temp);
        for name in ["first", "second"] {
            fixture_plugin(&project.join(".claude/skills").join(name));
        }
        let listed = scan_project_plugins(&project);
        let id = &listed[0].assets[0].asset_id;
        assert_ne!(id, &listed[1].assets[0].asset_id);
        let matched = scan_selected_plugins(Some(&project), [id.as_str()]);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].assets.len(), 1);
        assert_eq!(matched[0].assets[0].asset_id, *id);
        let json = serde_json::to_value(&matched[0]).unwrap();
        assert!(json.get("installPath").is_none());
        let asset = &json["assets"][0];
        assert_eq!(asset["assetId"], id.as_str());
        assert!(asset.get("path").is_none() && asset.get("content").is_none());
    }

    #[test]
    fn manifest_commands_replace_the_default_directory_and_accept_inline_content() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("deploy-tools");
        fixture_plugin(&root);
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"deploy-tools","commands":{"status":{"source":"./commands/status.md"},"about":{"content":"Inline about","description":"Inline description"}}}"#,
        );
        write(&root.join("commands/legacy.md"), "Legacy body");

        let plugin = scan_plugin_root(&root, SkillSource::Global);
        let commands: Vec<_> = plugin
            .assets
            .iter()
            .filter(|asset| asset.kind == ClaudePluginAssetKind::Command)
            .collect();

        // 清单声明替换默认目录：legacy.md 不再是 command。
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Command),
            ["about", "status"]
        );
        let status = commands
            .iter()
            .find(|asset| asset.name == "status")
            .unwrap();
        assert_eq!(status.description, "Show deployment status");
        assert!(status.content.is_none());
        let about = commands.iter().find(|asset| asset.name == "about").unwrap();
        assert_eq!(about.content.as_deref(), Some("Inline about"));
        assert!(about.path.is_none());
        assert_eq!(about.description, "Inline description");
        // skills / agents 仍按默认目录解析。
        assert_eq!(names(&plugin.assets, ClaudePluginAssetKind::Agent).len(), 2);
    }

    #[test]
    fn manifest_skills_add_directories_and_agents_replace_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("deploy-tools");
        fixture_plugin(&root);
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"deploy-tools","skills":["./extra-skills/"],"agents":"./custom-agents/auditor.md"}"#,
        );
        write(
            &root.join("extra-skills/SKILL.md"),
            "---\ndescription: One extra skill\n---\nExtra body",
        );
        write(
            &root.join("custom-agents/auditor.md"),
            "---\ndescription: Audits releases\n---\nAuditor body",
        );

        let plugin = scan_plugin_root(&root, SkillSource::Global);

        // skills 是追加：默认目录保留，额外目录本身是一个 skill。
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Skill),
            ["deploy", "extra-skills", "review"]
        );
        // agents 是替换：默认 agents/ 不再解析。
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Agent),
            ["auditor"]
        );
    }

    #[test]
    fn paths_outside_the_plugin_root_are_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp_root(&temp);
        let root = base.join("plugin");
        write(&root.join("commands/inside.md"), "Inside body");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"plugin","skills":["../outside-skills","/abs-skills"],"agents":["./agents/../../outside-agent.md"]}"#,
        );
        write(&base.join("outside-agent.md"), "Outside agent");

        let plugin = scan_plugin_root(&root, SkillSource::Global);

        assert!(names(&plugin.assets, ClaudePluginAssetKind::Skill).is_empty());
        assert!(names(&plugin.assets, ClaudePluginAssetKind::Agent).is_empty());
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Command),
            ["inside"]
        );
    }

    #[test]
    fn root_skill_file_without_skills_directory_is_a_single_skill() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("solo");
        write(
            &root.join("SKILL.md"),
            "---\ndescription: Does one thing\n---\nSolo body",
        );
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"solo"}"#,
        );

        let plugin = scan_plugin_root(&root, SkillSource::Global);

        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Skill),
            ["solo"]
        );
        assert!(!plugin.has_hooks);
        assert!(!plugin.has_mcp_servers);
    }

    #[test]
    fn missing_or_broken_manifest_falls_back_to_the_default_layout() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp_root(&temp).join("plain");
        write(&root.join(".claude-plugin/plugin.json"), "{ broken");
        write(
            &root.join("skills/only/SKILL.md"),
            "---\ndescription: Only skill\n---\nBody",
        );

        let plugin = scan_plugin_root(&root, SkillSource::Global);

        assert_eq!(plugin.name, "plain");
        assert_eq!(plugin.display_name, "plain");
        assert_eq!(plugin.version, "");
        assert_eq!(
            names(&plugin.assets, ClaudePluginAssetKind::Skill),
            ["only"]
        );
    }

    #[test]
    fn plugins_root_honors_the_official_override_variable() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp_root(&temp);

        assert_eq!(
            claude_plugins_root_with_override(&home, Some(" /custom/plugins ")),
            Utf8PathBuf::from("/custom/plugins")
        );
        assert_eq!(
            claude_plugins_root_with_override(&home, Some("   ")),
            home.join(".claude").join("plugins")
        );
        assert_eq!(
            claude_plugins_root_with_override(&home, None),
            home.join(".claude").join("plugins")
        );
    }

    #[test]
    fn discovers_installed_synced_and_skills_directory_plugins() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp_root(&temp);
        let plugins_root = base.join("plugins");
        let home = base.join("home");
        let project = base.join("project");

        // installed_plugins.json v2：同一注册安装出现多次只解析一次。
        let installed = base.join("cache/acme/deploy-tools/1.2.0");
        fixture_plugin(&installed);
        write(
            &plugins_root.join("installed_plugins.json"),
            &serde_json::json!({
                "version": 2,
                "plugins": {
                    "deploy-tools@acme": [
                        {"scope": "user", "installPath": installed.as_str(), "version": "1.2.0"},
                        {"scope": "user", "installPath": installed.as_str(), "version": "1.2.0"}
                    ]
                }
            })
            .to_string(),
        );
        // synced/<plugin>/<version>/：版本号目录不是 plugin 名。
        write(
            &plugins_root.join("synced/synced-plugin/3.0.0/skills/one/SKILL.md"),
            "---\ndescription: Synced skill\n---\nBody",
        );
        // skills 目录 plugin：用户级与项目级各一个。
        write(
            &home.join(".claude/skills/sd-plugin/.claude-plugin/plugin.json"),
            r#"{"name":"sd-plugin"}"#,
        );
        write(
            &home.join(".claude/skills/sd-plugin/skills/a/SKILL.md"),
            "---\ndescription: A\n---\nA body",
        );
        write(
            &project.join(".claude/skills/proj-plugin/.claude-plugin/plugin.json"),
            r#"{"name":"proj-plugin"}"#,
        );
        write(
            &project.join(".claude/skills/proj-plugin/agents/p.md"),
            "---\ndescription: P\n---\nP body",
        );

        let plugins = scan_claude_plugins(&plugins_root, &home, Some(&project));
        let listed: Vec<&str> = plugins.iter().map(|plugin| plugin.name.as_str()).collect();

        assert_eq!(
            listed,
            ["deploy-tools", "proj-plugin", "sd-plugin", "synced-plugin"]
        );
        assert_eq!(
            plugin(&plugins, "deploy-tools").install_path,
            installed.to_string()
        );
        assert_eq!(
            plugin(&plugins, "synced-plugin").source,
            SkillSource::Global
        );
        assert_eq!(
            names(
                &plugin(&plugins, "synced-plugin").assets,
                ClaudePluginAssetKind::Skill
            ),
            ["one"]
        );
        assert_eq!(plugin(&plugins, "sd-plugin").source, SkillSource::Global);
        assert_eq!(plugin(&plugins, "proj-plugin").source, SkillSource::Project);
        assert_eq!(
            names(
                &plugin(&plugins, "proj-plugin").assets,
                ClaudePluginAssetKind::Agent
            ),
            ["p"]
        );
    }

    #[test]
    fn unrecognized_registry_layout_does_not_establish_scope() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp_root(&temp);
        let plugins_root = base.join("plugins");
        let legacy = base.join("cache/legacy/1.0.0");
        write(&legacy.join("commands/run.md"), "Run body");
        write(
            &plugins_root.join("installed_plugins.json"),
            &serde_json::json!([
                {"projectPath": "/p", "plugins": [{"installPath": legacy.as_str()}]}
            ])
            .to_string(),
        );

        let plugins = scan_claude_plugins(&plugins_root, &base, None);

        assert!(plugins.is_empty());
    }
}
