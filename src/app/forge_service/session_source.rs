//! Bounded export of the same persisted branch selected by the conversation view.
use super::*;
use camino::{Utf8Path, Utf8PathBuf};
use std::io::Read;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSourceSession {
    pub task_uuid: Option<String>,
    pub run_id: String,
    pub round_id: String,
    pub node_id: String,
    pub attempt_id: String,
    pub outer_node_id: Option<String>,
    pub outer_attempt_id: Option<String>,
    pub branch_id: String,
    pub after: Option<ForgeSourceMessage>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForgeSourceMessage {
    pub event_id: String,
    pub event_seq: u64,
}

struct Exchange {
    title: String,
    requirement: String,
    messages: Vec<(&'static str, String)>,
    truncated: bool,
}

impl App {
    fn forge_source_exchange(
        &self,
        task_id: &str,
        session: Option<&ForgeSourceSession>,
    ) -> Result<Exchange> {
        let (dir, task) = self.checked_task_locator(task_id)?;
        let canonical = dir.canonicalize_utf8()?;
        let title = task.title.unwrap_or_else(|| task_id.into());
        let mut bytes = Vec::new();
        const REQUIREMENT_BYTES: usize = HIDDEN_TOTAL_CHAR_CAP * 4;
        let requirement_path = dir.join("authoring/requirement.md");
        if checked_file(&dir, &canonical, &requirement_path)? {
            std::fs::File::open(requirement_path)?
                .take((REQUIREMENT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
        }
        let mut truncated = bytes.len() > REQUIREMENT_BYTES;
        bytes.truncate(REQUIREMENT_BYTES);
        let requirement = visible_text(&String::from_utf8_lossy(&bytes));
        let mut messages = Vec::new();
        if let Some(s) = session {
            if s.task_uuid.is_some() && s.task_uuid != task.uuid {
                bail!(ForgeTaskAccessError::InvalidLocator);
            }
            let timeline = s.timeline(self, task_id)?;
            if !checked_file(&dir, &canonical, &timeline)? {
                bail!(ForgeTaskAccessError::SourceSessionUnavailable);
            }
            checked_file(&dir, &canonical, &timeline.with_extension("index.json"))?;
            use crate::acp::timeline::VisibleExchange;
            let (events, clipped) = match crate::acp::timeline::visible_exchange_window(
                &timeline,
                &s.branch_id,
                s.after.as_ref().map(|a| (a.event_id.as_str(), a.event_seq)),
            )? {
                VisibleExchange::Window { events, truncated } => (events, truncated),
                VisibleExchange::StaleMessage => bail!(ForgeTaskAccessError::StaleMessage),
                VisibleExchange::Unavailable => {
                    bail!(ForgeTaskAccessError::SourceSessionUnavailable)
                }
            };
            truncated |= clipped;
            for event in events {
                let Some(content) = event.content.as_deref() else {
                    continue;
                };
                // The control artifact is a separate UI card, never assistant prose.
                let mut text = content.to_owned();
                if let Some(span) = event
                    .raw
                    .as_ref()
                    .and_then(|v| v.get("runtimeControlOutputDisplay"))
                {
                    let (Some(start), Some(end)) = (span["start"].as_u64(), span["end"].as_u64())
                    else {
                        truncated = true;
                        continue;
                    };
                    let units: Vec<_> = text.encode_utf16().collect();
                    if end < start || end > units.len() as u64 {
                        truncated = true;
                        continue;
                    }
                    text = String::from_utf16_lossy(
                        &[&units[..start as usize], &units[end as usize..]].concat(),
                    );
                }
                let text = visible_text(&text);
                if !text.trim().is_empty() {
                    messages.push((
                        if event.kind == "userTextDelta" {
                            "User"
                        } else {
                            "Assistant"
                        },
                        text,
                    ));
                }
            }
        }
        Ok(Exchange {
            title,
            requirement,
            messages,
            truncated,
        })
    }

    pub fn forge_source_material(
        &self,
        task_id: &str,
        session: Option<&ForgeSourceSession>,
    ) -> Result<ForgeSourceMaterial> {
        let source = self.forge_source_exchange(task_id, session)?;
        let content = source.render(HIDDEN_TOTAL_CHAR_CAP);
        Ok(ForgeSourceMaterial {
            title: source.title,
            hidden_section: crate::provider::PromptHiddenSection {
                title: "源会话材料".into(),
                content,
            },
        })
    }

    pub fn forge_source_digest(
        &self,
        task_id: &str,
        session: Option<&ForgeSourceSession>,
    ) -> Result<ForgeSourceDigest> {
        let source = self.forge_source_exchange(task_id, session)?;
        let markdown = source.render(DIGEST_CHAR_CAP);
        Ok(ForgeSourceDigest {
            title: source.title,
            markdown,
        })
    }
}

fn checked_file(root: &Utf8Path, canonical: &Utf8Path, file: &Utf8Path) -> Result<bool> {
    match std::fs::symlink_metadata(file) {
        Ok(_) if file.canonicalize_utf8()? == canonical.join(file.strip_prefix(root)?) => Ok(true),
        Ok(_) => Err(ForgeTaskAccessError::InvalidLocator.into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

impl ForgeSourceSession {
    fn timeline(&self, app: &App, task: &str) -> Result<Utf8PathBuf> {
        for id in [
            &self.run_id,
            &self.round_id,
            &self.node_id,
            &self.attempt_id,
        ]
        .into_iter()
        .chain(self.outer_node_id.iter())
        .chain(self.outer_attempt_id.iter())
        {
            if Utf8Path::new(id).file_name() != Some(id.as_str()) || id.contains(['/', '\\', ':']) {
                bail!(ForgeTaskAccessError::InvalidLocator);
            }
        }
        crate::acp::branches::validate_conversation_branch_id(&self.branch_id)?;
        let attempt = match (&self.outer_node_id, &self.outer_attempt_id) {
            (None, None) => app.paths.attempt_dir(
                task,
                &self.run_id,
                &self.round_id,
                &self.node_id,
                &self.attempt_id,
            ),
            (Some(node), Some(attempt)) => app.paths.dynamic_node_attempt_dir(
                task,
                &self.run_id,
                &self.round_id,
                node,
                attempt,
                &self.node_id,
                &self.attempt_id,
            ),
            _ => bail!(ForgeTaskAccessError::InvalidLocator),
        };
        Ok(crate::acp::branches::branch_timeline_path(
            &attempt,
            &self.branch_id,
        ))
    }
}

/// Provider sections are escaped before embedding. Fail closed on an incomplete section.
fn visible_text(mut content: &str) -> String {
    let mut out = String::new();
    while let Some((before, hidden)) = content.split_once("<hidden") {
        out.push_str(before);
        let Some((attributes, body)) = hidden.split_once('>') else {
            // A byte-limited requirement can end part-way through an opening tag.
            return out;
        };
        if !attributes
            .to_ascii_lowercase()
            .contains("data-gold-band-hidden")
        {
            out.push_str("<hidden");
            out.push_str(attributes);
            out.push('>');
            content = body;
            continue;
        }
        let Some((_, after)) = body.split_once("</hidden>") else {
            return out;
        };
        content = after;
    }
    out.push_str(content);
    out
}

impl Exchange {
    fn render(&self, limit: usize) -> String {
        const NOTE: &str = "\n\n[… truncated …]";
        let mut clipped = self.truncated;
        let mut clip = |text: &str, n: usize| {
            if text.chars().nth(n).is_some() {
                clipped = true;
            }
            text.chars().take(n).collect::<String>()
        };
        let mut out = format!(
            "# {}\n\n{}",
            clip(&self.title, limit / 8),
            clip(&self.requirement, limit / 3)
        );
        let mut remaining = limit.saturating_sub(out.chars().count() + NOTE.chars().count());
        let mut rows = Vec::new();
        for (role, text) in self.messages.iter().rev() {
            let prefix = format!("\n\n{role}: ");
            let cap = remaining
                .saturating_sub(prefix.len())
                .min(HIDDEN_PROMPT_CHAR_CAP)
                .min(limit / 3);
            if cap == 0 {
                clipped = true;
                break;
            }
            let row = prefix + &clip(text.trim(), cap);
            remaining -= row.chars().count();
            rows.push(row);
        }
        for row in rows.into_iter().rev() {
            out.push_str(&row);
        }
        if clipped {
            out.push_str(NOTE);
        }
        out
    }
}
