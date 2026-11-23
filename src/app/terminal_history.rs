//! T13/T14: the backend-held terminal output history with a stable absolute
//! byte cursor.
//!
//! The buffer is bounded (`MAX_TERMINAL_HISTORY_BYTES`) and trimmed from the
//! front, so buffer indices shift over time; only the absolute stream cursor
//! `(base, end)` is stable. Consumers join on that cursor:
//!
//! - the MCP `read_output` tool maps `fromOffset` onto the absolute cursor and
//!   learns when its request predates the retained window (trimmed) instead of
//!   silently replaying a wrong slice;
//! - the frontend terminal view joins its history snapshot and live event
//!   stream on `end` with no gap or duplication.

/// Bounds the per-session history buffer so a long-lived shell can't grow it
/// without bound.
pub const MAX_TERMINAL_HISTORY_BYTES: usize = 512 * 1024;

/// A per-session history buffer paired with the ABSOLUTE byte offset of its
/// first retained byte (`base`).
#[derive(Debug, Default)]
pub struct BoundedTerminalHistory {
    pub base: u64,
    pub buffer: Vec<u8>,
}

impl BoundedTerminalHistory {
    pub fn end(&self) -> u64 {
        self.base + self.buffer.len() as u64
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_TERMINAL_HISTORY_BYTES {
            let excess = self.buffer.len() - MAX_TERMINAL_HISTORY_BYTES;
            self.buffer.drain(..excess);
            self.base += excess as u64;
        }
    }

    /// Returns the retained bytes from absolute `from` (absolute stream
    /// position), or `None` when `from` is ahead of the retained data (the
    /// consumer's live stream is ahead of this snapshot). `from <= base`
    /// yields the whole retained window (fresh consumer or trimmed read —
    /// the caller distinguishes by comparing `from` with `base`).
    pub fn slice_from(&self, from: u64) -> Option<&[u8]> {
        if from <= self.base {
            return Some(&self.buffer);
        }
        let rel = (from - self.base) as usize;
        if rel > self.buffer.len() {
            return None;
        }
        Some(&self.buffer[rel..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_tracks_the_absolute_cursor() {
        let mut history = BoundedTerminalHistory::default();
        history.append(b"hello ");
        history.append(b"world\n");
        assert_eq!(history.base, 0);
        assert_eq!(history.end(), 12);
        assert_eq!(history.buffer, b"hello world\n");
    }

    #[test]
    fn append_trims_from_the_front_and_advances_base() {
        let mut history = BoundedTerminalHistory::default();
        let big = vec![b'x'; MAX_TERMINAL_HISTORY_BYTES + 1000];
        history.append(&big);
        assert!(history.buffer.len() <= MAX_TERMINAL_HISTORY_BYTES);
        assert_eq!(history.base, 1000, "the excess prefix is accounted");
        assert_eq!(
            history.end(),
            (MAX_TERMINAL_HISTORY_BYTES + 1000) as u64,
            "the absolute cursor keeps counting trimmed bytes"
        );
    }

    #[test]
    fn slice_from_maps_absolute_offsets_onto_the_retained_window() {
        let mut history = BoundedTerminalHistory::default();
        history.append(b"hello world");
        // Fresh consumer (no cursor): the whole window.
        assert_eq!(history.slice_from(0), Some(&b"hello world"[..]));
        // Mid-stream cursor: only the suffix.
        assert_eq!(history.slice_from(6), Some(&b"world"[..]));
        // Cursor at the end: empty, not an error.
        assert_eq!(history.slice_from(11), Some(&b""[..]));
        // Consumer ahead of the buffer: None (its live stream is the source).
        assert_eq!(history.slice_from(999), None);
    }

    #[test]
    fn slice_from_after_trimming_answers_the_retained_suffix() {
        let mut history = BoundedTerminalHistory::default();
        history.append(b"abcdef");
        history.append(&vec![b'y'; MAX_TERMINAL_HISTORY_BYTES]);
        // "abcdef" was trimmed; base advanced past it.
        assert!(history.base >= 6);
        // Reading from before the trim still answers — with the retained
        // window only (the caller detects the gap by comparing with `base`).
        assert_eq!(history.slice_from(0), Some(&history.buffer[..]));
        // Reading from inside the retained window gives the exact suffix.
        let mid = history.base + 3;
        assert_eq!(history.slice_from(mid), Some(&history.buffer[3..]));
    }
}
