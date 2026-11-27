//! WorkBuddy Desktop distribution and registry evidence. Neither is an enabled-plugin API.
//! Never discover packages by walking a cache, looking inside ASAR, or running a host command.
use super::{
    buddy::{self, Origin},
    *,
};
use crate::{skill::SKILL_BUNDLE_MAX_FILE_BYTES, storage::GoldBandPaths};

const DISTRIBUTION: &str = "resources/app.asar.unpacked/resources/plugins/workbuddy-builtin";
const DESKTOP_EXE: &str = "WorkBuddy.exe";
const MAX_INSTALLATIONS: usize = 8;
const MAX_REGISTRATIONS: usize = 64;

fn local_absolute(path: &Utf8Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::Prefix;
        if !matches!(path.as_std_path().components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return false;
        }
    }
    path.is_absolute()
        && !path
            .as_std_path()
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
}

pub(super) fn desktop_home() -> Option<Utf8PathBuf> {
    // Both are vendor-documented locations; permit isolated child processes to supply their own home.
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(key)
        .ok()
        .map(Utf8PathBuf::from)
        .filter(|p| local_absolute(p))
}

fn component(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-+".contains(&c))
        && !s.ends_with('.')
}

// Compare before any I/O at a registry-supplied path. In particular never touch a UNC,
// unrelated absolute path or symlink to a package outside the exact versioned cache.
fn same_spelling(a: &Utf8Path, b: &Utf8Path) -> bool {
    #[cfg(windows)]
    {
        let spelling = |p: &Utf8Path| {
            p.as_str()
                .replace('\\', "/")
                .trim_start_matches("//?/")
                .to_owned()
        };
        spelling(a).eq_ignore_ascii_case(&spelling(b))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

pub(super) fn registered(home: &Utf8Path, filter: InstallationFilter<'_>) -> Vec<InstalledPlugin> {
    let load = || -> Option<Vec<_>> {
        if !local_absolute(home) {
            return None;
        }
        let store = home.join(".workbuddy/plugins");
        let dir = buddy::open_canonical_dir(&store)?;
        let data: Value = serde_json::from_str(&buddy::bounded_file(
            &dir,
            INSTALLED_PLUGINS_FILE.as_ref(),
            SKILL_BUNDLE_MAX_FILE_BYTES,
        )?)
        .ok()?;
        if data["version"].as_u64() != Some(2) {
            return None;
        }
        let entries = data["plugins"].as_object()?;
        // BTreeMap iteration is deterministic. A large inventory must not erase the
        // already verified prefix. Bound keys (even empty/broken ones), records, and bytes.
        let mut remaining = MAX_REGISTRATIONS;
        let mut bytes_left = crate::skill::SKILL_BUNDLE_MAX_BUNDLE_BYTES;
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for (key, records) in entries.iter().take(crate::skill::SKILL_BUNDLE_MAX_FILES) {
            if remaining == 0 || bytes_left == 0 {
                break;
            }
            let Some(records) = records.as_array() else {
                continue;
            };
            let count = records.len().min(remaining);
            let records = &records[..count];
            remaining -= count;
            let Some((name, market)) = key
                .split_once('@')
                .filter(|(n, m)| component(n) && component(m))
            else {
                continue;
            };
            for record in records {
                if record["scope"].as_str() != Some("user") || !record["projectPath"].is_null() {
                    continue;
                }
                let Some(version) = record["version"].as_str().filter(|v| component(v)) else {
                    continue;
                };
                let Some(actual) = record["installPath"]
                    .as_str()
                    .map(Utf8Path::new)
                    .filter(|p| local_absolute(p))
                else {
                    continue;
                };
                let root = store.join("cache").join(market).join(name).join(version);
                if !same_spelling(&root, actual) || !seen.insert(root.clone()) {
                    continue;
                }
                // read_source opens this exact lexical path component by component. Never
                // substitute the target of a junction canonicalized between check and use.
                if let Some(p) = buddy::read_source_with_budget(
                    &root,
                    Origin::Registered { name, version },
                    SkillSource::Global,
                    None,
                    filter,
                    &mut bytes_left,
                ) {
                    rows.push(p);
                }
            }
        }
        Some(rows)
    };
    load().unwrap_or_default()
}

pub(super) fn is_install_candidate(root: &Utf8Path) -> bool {
    local_absolute(root) && root.join(DESKTOP_EXE).symlink_metadata().is_ok()
}

pub(super) fn bundled(
    root: &Utf8Path,
    scope: SkillSource,
    owner: Option<&str>,
    filter: InstallationFilter<'_>,
) -> Option<InstalledPlugin> {
    if !local_absolute(root) {
        return None;
    }
    let package = root.join(DISTRIBUTION);
    // A known suffix alone is not evidence. Reject replacements/junctions on both paths.
    buddy::open_canonical_dir(&package)?;
    let version = product_version(root)?;
    buddy::read_source(
        &package,
        Origin::Bundled { version: &version },
        scope,
        owner,
        filter,
    )
}

#[cfg(not(windows))]
fn product_version(_: &Utf8Path) -> Option<String> {
    None
}

#[cfg(windows)]
fn product_version(root: &Utf8Path) -> Option<String> {
    use cap_std::fs::{MetadataExt, OpenOptions, OpenOptionsExt};
    use std::{
        io::Read,
        process::Stdio,
        time::{Duration, Instant},
    };
    use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};
    // Read-only handle denies replacement while the OS queries this exact version resource.
    let dir = buddy::open_canonical_dir(root)?;
    let pin = dir
        .open_with(
            DESKTOP_EXE,
            OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x0020_0000),
        )
        .ok()?;
    let meta = pin.metadata().ok()?;
    if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
        return None;
    }
    // Windows PowerShell is used only as a bridge to the OS version-resource API; the
    // fixed expression never evaluates a registry string, package path or executable.
    let system: String = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()?
        .get_value("SystemRoot")
        .ok()?;
    let system = Utf8Path::new(&system);
    if !local_absolute(system) {
        return None;
    }
    let shell = system.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let exe = dunce::simplified(root.join(DESKTOP_EXE).as_std_path()).to_path_buf();
    const QUERY: &str = "$v=[Diagnostics.FileVersionInfo]::GetVersionInfo($env:GB_DESKTOP_VERSION_FILE); if ($v.ProductName -ceq 'WorkBuddy' -and $v.CompanyName -ceq 'Tencent Technology (Shenzhen) Company Limited' -and $v.ProductVersion -match '^[0-9]+(\\.[0-9]+){1,3}$' -and $v.ProductVersion.Length -le 64) { [Console]::Write($v.ProductVersion) }";
    let mut child = crate::process::background_command(&shell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            QUERY,
        ])
        .current_dir(system)
        .env("GB_DESKTOP_VERSION_FILE", exe)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(6) => {
                std::thread::sleep(Duration::from_millis(15))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut version = String::new();
    child
        .stdout
        .take()?
        .take(65)
        .read_to_string(&mut version)
        .ok()?;
    (status.success() && !version.is_empty() && version.len() <= 64).then_some(version)
}

#[cfg(windows)]
fn install_roots() -> Vec<Utf8PathBuf> {
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    };
    let mut roots = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let read = |key: &str| {
            RegKey::predef(hive)
                .open_subkey(key)
                .ok()?
                .get_value::<String, _>("")
                .ok()
        };
        let app = read(r"Software\Microsoft\Windows\CurrentVersion\App Paths\WorkBuddy.exe");
        // This literal association form is present on WorkBuddy 5.11.3. Never execute the command.
        let association = read(r"Software\Classes\workbuddy\shell\open\command").and_then(|s| {
            let path = s.strip_prefix('"')?.strip_suffix("\" \"%1\"")?;
            (!path.contains('"')).then(|| path.to_owned())
        });
        for exe in [app, association].into_iter().flatten() {
            let exe = Utf8Path::new(&exe);
            if local_absolute(exe)
                && exe
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case(DESKTOP_EXE))
            {
                if let Some(parent) = exe.parent() {
                    roots.push(parent.to_owned());
                }
            }
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(Utf8PathBuf::from(local).join("Programs/WorkBuddy"));
    }
    roots
}
#[cfg(not(windows))]
fn install_roots() -> Vec<Utf8PathBuf> {
    vec![]
}

fn bundles(roots: &[Utf8PathBuf], filter: InstallationFilter<'_>) -> Vec<InstalledPlugin> {
    let mut seen = BTreeSet::new();
    roots
        .iter()
        .take(MAX_INSTALLATIONS)
        .filter(|root| {
            local_absolute(root) && seen.insert(crate::storage::normalize_workspace_path(root))
        })
        .filter_map(|root| bundled(root, SkillSource::Global, None, filter))
        .collect()
}
pub(super) fn installed_bundles(filter: InstallationFilter<'_>) -> Vec<InstalledPlugin> {
    bundles(&install_roots(), filter)
}

fn combine(
    paths: &GoldBandPaths,
    include_project: bool,
    automatic: impl IntoIterator<Item = InstalledPlugin>,
) -> Vec<InstalledPlugin> {
    // An explicit grant remains revocable. Deduplicate physical roots only within the
    // same visibility scope; neither a similar name nor another project/copy is an alias.
    let mut rows: Vec<_> = [SkillSource::Global, SkillSource::Project]
        .into_iter()
        .filter(|s| include_project || *s == SkillSource::Global)
        .filter_map(|s| buddy::workbuddy_source(paths, s))
        .collect();
    let mut seen: BTreeSet<_> = rows
        .iter()
        .filter(|p| !p.unavailable)
        .map(|p| {
            (
                p.source == SkillSource::Project,
                crate::storage::normalize_workspace_path(Utf8Path::new(&p.install_path)),
            )
        })
        .collect();
    rows.extend(automatic.into_iter().filter(|p| {
        seen.insert((
            p.source == SkillSource::Project,
            crate::storage::normalize_workspace_path(Utf8Path::new(&p.install_path)),
        ))
    }));
    rows
}

/// Discover only WorkBuddy using explicit desktop home and installation candidates, no other host inventories.
pub fn scan_workbuddy_desktop(
    paths: &GoldBandPaths,
    include_project: bool,
    home: &Utf8Path,
    installation_roots: &[Utf8PathBuf],
) -> Vec<InstalledPlugin> {
    combine(
        paths,
        include_project,
        registered(home, None)
            .into_iter()
            .chain(bundles(installation_roots, None)),
    )
}
pub(super) fn inventory(paths: &GoldBandPaths, include_project: bool) -> Vec<InstalledPlugin> {
    let registry = desktop_home()
        .map(|home| registered(&home, None))
        .unwrap_or_default();
    combine(
        paths,
        include_project,
        registry.into_iter().chain(installed_bundles(None)),
    )
}
