//! Pop-out "detached" windows for terminal sessions and the connections page.
//!
//! A detached window is a real OS window (Tauri v2 `WebviewWindow`) that loads the
//! same frontend entry with a query param telling `main.tsx` to render only that
//! resource (no full shell). Backend session state is a global singleton and
//! terminal events broadcast to all windows, so a detached window drives the SAME
//! session/manager as the main window for free — this module only creates and
//! tears down the window, and lets it route to the right isolated UI.
//!
//! Semantic model (agreed): the detached window's native close button ends the
//! session; a separate "return to main window" action closes the window (the
//! session keeps running and reappears in the main dock). Detached windows are
//! lifecycle children of the main window: they are enumerated + closed on app
//! exit, and they never participate in the main window exit/close handshake.

use std::collections::HashSet;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tracing::debug;

use crate::commands::{CommandErrorVm, CommandResult};
use crate::state::TerminalSessionState;

/// Prefix for every detached window label so we can enumerate them on exit and
/// distinguish them from the main window.
pub const DETACHED_LABEL_PREFIX: &str = "detached-";

/// Frontend event emitted when a detached window is closed by its OS chrome. The
/// main window listens so it can refresh its session list (tab reappears / is
/// removed consistently).
pub const DETACHED_WINDOW_CLOSED_EVENT: &str = "gold-band://detached-window-closed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetachedWindowKind {
    /// A single terminal session, keyed by `session_id`.
    Terminal,
    /// The connections management page (a whole page, not a session).
    Connections,
}

impl DetachedWindowKind {
    fn label(&self, session_id: Option<&str>) -> String {
        match self {
            Self::Connections => format!("{DETACHED_LABEL_PREFIX}connections"),
            Self::Terminal => format!(
                "{DETACHED_LABEL_PREFIX}terminal-{}",
                session_id.unwrap_or("unknown")
            ),
        }
    }
}

fn detached_url(kind: DetachedWindowKind, session_id: Option<&str>) -> String {
    match kind {
        DetachedWindowKind::Terminal => format!(
            "index.html?window=terminal&session={}",
            session_id.unwrap_or("")
        ),
        DetachedWindowKind::Connections => "index.html?window=connections".to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachedWindowVm {
    pub label: String,
}

#[derive(Debug, Default)]
pub struct DetachedWindowRegistry {
    shared: std::sync::Arc<Mutex<HashSet<String>>>,
}

impl DetachedWindowRegistry {
    fn insert(&self, label: &str) {
        if let Ok(mut set) = self.shared.lock() {
            set.insert(label.to_string());
        }
    }

    /// Removes `label`; returns whether it was still registered. The return
    /// value is the "return to main window" discriminator for the Destroyed
    /// hook: that path removes first, a native close does not.
    fn remove(&self, label: &str) -> bool {
        self.shared
            .lock()
            .map(|mut set| set.remove(label))
            .unwrap_or(false)
    }

    /// A handle sharing the same underlying label set, for the window-event
    /// closures that outlive the command's `State` borrow.
    fn clone_shared(&self) -> SharedDetachedLabels {
        SharedDetachedLabels {
            labels: std::sync::Arc::clone(&self.shared),
        }
    }

    pub fn open_labels(&self) -> Vec<String> {
        self.shared
            .lock()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// `Arc`-shared view of the registry's label set, so event closures can
/// unregister their window after the command's `State` borrow has ended.
#[derive(Clone)]
struct SharedDetachedLabels {
    labels: std::sync::Arc<Mutex<HashSet<String>>>,
}

impl SharedDetachedLabels {
    fn remove(&self, label: &str) -> bool {
        self.labels
            .lock()
            .map(|mut set| set.remove(label))
            .unwrap_or(false)
    }
}

fn command_error(code: &str, message: impl std::fmt::Display) -> CommandErrorVm {
    CommandErrorVm::new(code, serde_json::json!({ "message": message.to_string() }))
}

/// Opens a detached window for a terminal session or the connections page.
/// `session_id` is used by the terminal kind; ignored for connections.
///
/// T07: this must stay an `async` command. Tauri 2.x runs sync commands on the
/// main thread on Windows, and creating a `WebviewWindow` from a sync command
/// there deadlocks the main window (locked tauri 2.11.2 contract). The async
/// form runs on the async runtime instead, so the main window keeps
/// responding while the pop-out is built.
#[tauri::command]
pub async fn open_detached_window(
    app: AppHandle,
    registry: tauri::State<'_, DetachedWindowRegistry>,
    terminal_state: tauri::State<'_, TerminalSessionState>,
    kind: DetachedWindowKind,
    session_id: Option<String>,
) -> CommandResult<DetachedWindowVm> {
    open_detached_window_inner(
        app,
        registry.inner(),
        terminal_state.inner(),
        kind,
        session_id,
    )
}

fn open_detached_window_inner(
    app: AppHandle,
    registry: &DetachedWindowRegistry,
    terminal_state: &TerminalSessionState,
    kind: DetachedWindowKind,
    session_id: Option<String>,
) -> CommandResult<DetachedWindowVm> {
    let label = kind.label(session_id.as_deref());

    // Already open: bring it to front instead of stacking a duplicate.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(DetachedWindowVm { label });
    }

    let url = detached_url(kind, session_id.as_deref());
    debug!(label = %label, %url, "opening detached window");

    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title("Gold Band")
        .inner_size(900.0, 640.0)
        .min_inner_size(480.0, 320.0)
        .build()
        .map_err(|error| command_error("window.create-failed", error))?;

    registry.insert(&label);

    let close_app = app.clone();
    let close_label = label.clone();
    // Shared handles for the Destroyed hook below. The registry `remove`
    // doubles as the "return to main window" marker: `close_detached_window`
    // removes the label BEFORE closing the window, so a Destroyed for an
    // already-unregistered label is that keep-alive path and must not end the
    // session. A native close never unregisters first, so it is registered
    // here → end the session per the agreed semantic.
    let close_registry = registry.clone_shared();
    let close_terminal_state = std::sync::Arc::clone(&terminal_state.0);
    window.on_window_event(move |event| {
        match event {
            tauri::WindowEvent::CloseRequested { .. } => {
                debug!(label = %close_label, "detached window close requested");
                let _ = close_app.emit_to(
                    "main",
                    DETACHED_WINDOW_CLOSED_EVENT,
                    DetachedWindowVm {
                        label: close_label.clone(),
                    },
                );
            }
            // T11: the OS chrome destroyed the window (native close button,
            // crash, ...). Without this the registry keeps reporting the label
            // as popped out and the main window hides a tab that has no window
            // while the session leaks. Unregister here; end the session only
            // when the Destroyed is NOT the "return to main window" path
            // (which unregisters first, see `close_detached_window`).
            tauri::WindowEvent::Destroyed => {
                debug!(label = %close_label, "detached window destroyed");
                let was_registered = close_registry.remove(&close_label);
                if was_registered {
                    if let Some(session_id) = terminal_session_id_from_label(&close_label) {
                        debug!(label = %close_label, %session_id, "ending session for natively closed detached terminal");
                        // Fire-and-forget: closing emits the terminal `Exited`
                        // event, which removes the tab on the frontend and
                        // releases the session slot (T09 keeps that consistent).
                        // Clone the Arc into the inner closure: the outer hook
                        // is an `Fn` and may run for many events.
                        let close_terminal_state = std::sync::Arc::clone(&close_terminal_state);
                        std::thread::spawn(move || {
                            if let Ok(mut guard) = close_terminal_state.lock() {
                                let _ = guard.close(&session_id);
                            }
                        });
                    }
                }
            }
            _ => {}
        }
    });

    Ok(DetachedWindowVm { label })
}

/// Extracts the terminal session id from a detached terminal label, or `None`
/// for the connections window / malformed labels.
fn terminal_session_id_from_label(label: &str) -> Option<String> {
    const TERMINAL_PREFIX: &str = "detached-terminal-";
    label
        .strip_prefix(TERMINAL_PREFIX)
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
}

/// Closes a detached window without ending its session (the "return to main
/// window" action). The session keeps running; the main window refreshes and the
/// tab reappears.
#[tauri::command]
pub fn close_detached_window(
    app: AppHandle,
    registry: tauri::State<'_, DetachedWindowRegistry>,
    label: String,
) -> CommandResult<()> {
    registry.remove(&label);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.close();
    }
    Ok(())
}

/// Lists currently-open detached windows.
#[tauri::command]
pub fn list_detached_windows(
    registry: tauri::State<'_, DetachedWindowRegistry>,
) -> CommandResult<Vec<DetachedWindowVm>> {
    Ok(registry
        .open_labels()
        .into_iter()
        .map(|label| DetachedWindowVm { label })
        .collect())
}

/// Closes every detached window. Called during app cleanup so no detached OS
/// window outlives the main one. Enumerates by label prefix rather than trusting
/// the registry, so it is robust even if a registry entry was missed.
pub fn close_all_detached(app: &AppHandle) {
    for label in app
        .webview_windows()
        .keys()
        .filter(|label| label.starts_with(DETACHED_LABEL_PREFIX))
        .cloned()
        .collect::<Vec<_>>()
    {
        debug!(label = %label, "closing detached window on app cleanup");
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_label_embeds_session_id_and_prefix() {
        assert_eq!(
            DetachedWindowKind::Terminal.label(Some("sess-abc")),
            "detached-terminal-sess-abc"
        );
        assert_eq!(
            DetachedWindowKind::Connections.label(None),
            "detached-connections"
        );
    }

    #[test]
    fn detached_url_carries_query_for_window_routing() {
        assert_eq!(
            detached_url(DetachedWindowKind::Terminal, Some("sess-abc")),
            "index.html?window=terminal&session=sess-abc"
        );
        assert_eq!(
            detached_url(DetachedWindowKind::Connections, None),
            "index.html?window=connections"
        );
    }

    #[test]
    fn registry_tracks_and_removes_labels() {
        let registry = DetachedWindowRegistry::default();
        registry.insert("detached-terminal-a");
        registry.insert("detached-connections");
        assert_eq!(registry.open_labels().len(), 2);
        registry.remove("detached-terminal-a");
        assert_eq!(
            registry.open_labels(),
            vec!["detached-connections".to_string()]
        );
    }
}
