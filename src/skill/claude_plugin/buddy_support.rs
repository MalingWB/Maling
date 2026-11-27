//! Definition-only import of a dedicated Skill. Metadata classification never reads
//! sidecar bytes. Import reopens the bounded capability and revalidates the revision.
use crate::skill::{
    SKILL_BUNDLE_MAX_BUNDLE_BYTES, SKILL_BUNDLE_MAX_DEPTH, SKILL_BUNDLE_MAX_FILE_BYTES,
    SKILL_BUNDLE_MAX_FILES,
};
use camino::{Utf8Path, Utf8PathBuf};
use cap_std::fs::Dir;

pub(super) struct Support {
    pub revision: blake3::Hash,
    pub files: Vec<(String, String)>,
}

pub(super) fn inspect(
    root: &Dir,
    rel: &Utf8Path,
    main_bytes: usize,
    entries_left: &mut usize,
    read: bool,
) -> Option<Support> {
    let dir = if rel.as_str().is_empty() {
        root.try_clone().ok()?
    } else {
        root.open_dir(rel).ok()?
    };
    let mut pending = vec![(Utf8PathBuf::new(), 0usize)];
    let mut files = Vec::new();
    let mut budget = SKILL_BUNDLE_MAX_BUNDLE_BYTES.checked_sub(main_bytes as u64)?;
    while let Some((prefix, depth)) = pending.pop() {
        let iter = if prefix.as_str().is_empty() {
            dir.entries()
        } else {
            dir.read_dir(&prefix)
        }
        .ok()?;
        for e in iter {
            *entries_left = entries_left.checked_sub(1)?;
            let e = e.ok()?;
            let name = e.file_name().into_string().ok()?;
            let path = prefix.join(super::buddy::relative_path(&name)?);
            let ty = e.file_type().ok()?;
            if depth == 0 && name == super::SKILL_FILE && ty.is_file() {
                continue;
            }
            if name.starts_with('.') {
                return None;
            }
            if ty.is_dir() {
                if depth + 1 >= SKILL_BUNDLE_MAX_DEPTH
                    || (depth == 0 && !matches!(name.as_str(), "references" | "assets"))
                {
                    return None;
                }
                pending.push((path, depth + 1));
            } else if ty.is_file() {
                // The library's bundle format is UTF-8. Only inert documents/data; do
                // not partially import a Skill that also depends on scripts or binaries.
                let ext = path.extension()?.to_ascii_lowercase();
                if !matches!(ext.as_str(), "md" | "txt" | "csv" | "json" | "yaml" | "yml") {
                    return None;
                }
                let meta = dir.symlink_metadata(&path).ok()?;
                if !meta.is_file()
                    || meta.len() > SKILL_BUNDLE_MAX_FILE_BYTES
                    || files.len() + 1 >= SKILL_BUNDLE_MAX_FILES
                {
                    return None;
                }
                budget = budget.checked_sub(meta.len())?;
                let stamp = meta
                    .modified()
                    .ok()?
                    .into_std()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_nanos();
                files.push((path, meta.len(), stamp));
            } else {
                return None;
            }
        }
    }
    files.sort();
    let revision = blake3::hash(&serde_json::to_vec(&files).ok()?);
    let files = if read {
        files
            .into_iter()
            .map(|(path, len, _)| {
                let text = super::buddy::bounded_file(&dir, &path, len)?;
                if text.len() as u64 != len {
                    return None;
                }
                Some((path.as_str().replace('\\', "/"), text))
            })
            .collect::<Option<Vec<_>>>()?
    } else {
        vec![]
    };
    Some(Support { revision, files })
}
