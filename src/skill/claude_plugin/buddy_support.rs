//! Definition-only import of a dedicated Skill. Metadata classification never reads
//! sidecar bytes. Import reopens the bounded capability and revalidates the revision.
use super::{ImportBlocker, ImportBlockerCode, ImportBlockerCode::*};
use crate::skill::{
    SKILL_BUNDLE_MAX_BUNDLE_BYTES, SKILL_BUNDLE_MAX_DEPTH, SKILL_BUNDLE_MAX_FILE_BYTES,
    SKILL_BUNDLE_MAX_FILES,
};
use camino::{Utf8Path, Utf8PathBuf};
use cap_std::fs::Dir;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

pub(super) struct Support {
    pub revision: blake3::Hash,
    pub files: Vec<(String, String)>,
}

pub(super) enum InspectionError {
    SharedBudgetExhausted,
    Blocked(ImportBlocker),
}

fn blocked(code: ImportBlockerCode, path: Option<&Utf8Path>) -> InspectionError {
    // Only expose validated Skill-relative names, using portable separators. Omit
    // unrepresentable paths rather than truncating them into a different filename.
    let path = path.and_then(|p| {
        let portable = p.as_str().replace(std::path::MAIN_SEPARATOR, "/");
        super::buddy::relative_path(&portable)?;
        (!portable.chars().any(|c| {
            matches!(
                c.general_category(),
                GeneralCategory::Format
                    | GeneralCategory::LineSeparator
                    | GeneralCategory::ParagraphSeparator
            )
        }))
        .then_some(portable)
    });
    let limit = match code {
        DepthLimit => Some(SKILL_BUNDLE_MAX_DEPTH as u64),
        FileSizeLimit => Some(SKILL_BUNDLE_MAX_FILE_BYTES),
        BundleSizeLimit => Some(SKILL_BUNDLE_MAX_BUNDLE_BYTES),
        FileCountLimit => Some(SKILL_BUNDLE_MAX_FILES as u64),
        _ => None,
    };
    InspectionError::Blocked(ImportBlocker { code, path, limit })
}

pub(super) fn inspect(
    root: &Dir,
    rel: &Utf8Path,
    main_bytes: usize,
    entries_left: &mut usize,
    read: bool,
) -> Result<Support, InspectionError> {
    let incomplete = || blocked(InspectionIncomplete, None);
    let dir = if rel.as_str().is_empty() {
        root.try_clone()
    } else {
        root.open_dir(rel)
    }
    .map_err(|_| incomplete())?;
    let mut pending = vec![(Utf8PathBuf::new(), 0usize)];
    let mut files = Vec::new();
    let mut budget = SKILL_BUNDLE_MAX_BUNDLE_BYTES
        .checked_sub(main_bytes as u64)
        .ok_or_else(|| blocked(BundleSizeLimit, None))?;
    while let Some((prefix, depth)) = pending.pop() {
        let iter = if prefix.as_str().is_empty() {
            dir.entries()
        } else {
            dir.read_dir(&prefix)
        }
        .map_err(|_| blocked(InspectionIncomplete, Some(&prefix)))?;
        for e in iter {
            *entries_left = entries_left
                .checked_sub(1)
                .ok_or(InspectionError::SharedBudgetExhausted)?;
            let e = e.map_err(|_| incomplete())?;
            let name = e.file_name().into_string().map_err(|_| incomplete())?;
            let path = prefix.join(super::buddy::relative_path(&name).ok_or_else(incomplete)?);
            let at_path = |code| blocked(code, Some(&path));
            let ty = e.file_type().map_err(|_| at_path(InspectionIncomplete))?;
            if depth == 0 && name == super::SKILL_FILE && ty.is_file() {
                continue;
            }
            if name.starts_with('.') {
                return Err(at_path(HiddenEntry));
            }
            if ty.is_dir() {
                if depth + 1 >= SKILL_BUNDLE_MAX_DEPTH {
                    return Err(at_path(DepthLimit));
                }
                if depth == 0 && !matches!(name.as_str(), "references" | "assets") {
                    return Err(at_path(UnsupportedDirectory));
                }
                pending.push((path, depth + 1));
            } else if ty.is_file() {
                // The library's bundle format is UTF-8. Only inert documents/data; do
                // not partially import a Skill that also depends on scripts or binaries.
                let ext = path.extension().unwrap_or_default().to_ascii_lowercase();
                if !matches!(ext.as_str(), "md" | "txt" | "csv" | "json" | "yaml" | "yml") {
                    return Err(at_path(UnsupportedFile));
                }
                let meta = dir
                    .symlink_metadata(&path)
                    .map_err(|_| at_path(InspectionIncomplete))?;
                if !meta.is_file() {
                    return Err(at_path(SpecialEntry));
                }
                if meta.len() > SKILL_BUNDLE_MAX_FILE_BYTES {
                    return Err(at_path(FileSizeLimit));
                }
                if files.len() + 1 >= SKILL_BUNDLE_MAX_FILES {
                    return Err(blocked(FileCountLimit, None));
                }
                budget = budget
                    .checked_sub(meta.len())
                    .ok_or_else(|| blocked(BundleSizeLimit, None))?;
                let stamp = meta
                    .modified()
                    .map_err(|_| at_path(InspectionIncomplete))?
                    .into_std()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| at_path(InspectionIncomplete))?
                    .as_nanos();
                files.push((path, meta.len(), stamp));
            } else {
                return Err(at_path(SpecialEntry));
            }
        }
    }
    files.sort();
    let revision = blake3::hash(&serde_json::to_vec(&files).map_err(|_| incomplete())?);
    let files = if read {
        files
            .into_iter()
            .map(|(path, len, _)| {
                let incomplete = || blocked(InspectionIncomplete, Some(&path));
                let text = super::buddy::bounded_file(&dir, &path, len).ok_or_else(incomplete)?;
                if text.len() as u64 != len {
                    return Err(incomplete());
                }
                Ok((path.as_str().replace('\\', "/"), text))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![]
    };
    Ok(Support { revision, files })
}
