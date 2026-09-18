//! OSC 133 marker parsing and per-command output accumulation.
//!
//! Shell integration emits `OSC 133;C` before a command runs and `OSC 133;D`
//! once it finishes. The proxy forwards every byte to the real terminal
//! untouched and uses those two markers to slice one command's output out of
//! the stream.

/// Cap on one command's captured output. A command that prints without bound
/// (`yes`) would otherwise grow the proxy heap until the OOM killer steps in.
/// ponytail: keep the first 8 MiB; spill to a file per command if that bites.
const MAX_BLOCK_BYTES: usize = 8 * 1024 * 1024;

/// Cap on a single OSC payload. Only `133` parameters and window titles travel
/// this way, so anything larger is a child spewing an unterminated sequence —
/// it must not be allowed to grow the parser's heap either.
const MAX_OSC_PAYLOAD: usize = 4096;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

/// What the proxy reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// `OSC 133 ; C` — the shell is about to run a command.
    CommandStart,
    /// `OSC 133 ; D [ ; <exit> ]` — the command finished.
    CommandEnd(Option<i32>),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    /// Saw a bare `ESC`; only `]` makes it an OSC we care about.
    Esc,
    Osc,
    /// Saw `ESC` inside an OSC payload — either the start of `ST` or payload.
    OscEsc,
}

/// Incremental OSC 133 parser plus the current command's output buffer.
#[derive(Debug, Default)]
pub struct Stream {
    buf: Vec<u8>,
    state: State,
    payload: Vec<u8>,
    /// Offset where the marker currently being parsed started.
    seq_start: usize,
    /// Whether a `C` marker has opened a block. Bytes outside a block are the
    /// prompt and the echoed input line, which must never be recorded — a shell
    /// that never emits `C` must produce empty output, not prompt noise.
    in_command: bool,
    capped: bool,
}

impl Stream {
    /// Feed output bytes; returns the events completed by this chunk.
    ///
    /// A marker split across two reads is fine — the state machine carries over.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        for &byte in bytes {
            let pos = self.buf.len();
            if self.in_command {
                if self.buf.len() < MAX_BLOCK_BYTES {
                    self.buf.push(byte);
                } else {
                    self.capped = true;
                }
            }

            match self.state {
                State::Ground => {
                    if byte == ESC {
                        self.state = State::Esc;
                        self.seq_start = pos;
                    }
                }
                State::Esc => {
                    if byte == b']' {
                        self.state = State::Osc;
                        self.payload.clear();
                    } else {
                        self.state = State::Ground;
                    }
                }
                State::Osc => {
                    if byte == BEL {
                        self.finish(&mut events);
                    } else if byte == ESC {
                        self.state = State::OscEsc;
                    } else if self.payload.len() < MAX_OSC_PAYLOAD {
                        self.payload.push(byte);
                    } else {
                        // Unterminated sequence: give up on it and let the rest
                        // stream through as ordinary output.
                        self.payload.clear();
                        self.state = State::Ground;
                    }
                }
                State::OscEsc => {
                    if byte == b'\\' {
                        self.finish(&mut events);
                    } else {
                        // Not a terminator after all; the ESC belonged to the
                        // payload. Drop it — only 133 params are ever read.
                        self.state = State::Osc;
                    }
                }
            }
        }
        events
    }

    /// Output printed by the command that just ended, with the markers removed.
    ///
    /// Call after [`Event::CommandEnd`] and before the next `push`.
    pub fn take_output(&mut self) -> Vec<u8> {
        self.in_command = false;
        self.capped = false;
        let mut buf = std::mem::take(&mut self.buf);
        buf.truncate(MAX_BLOCK_BYTES);
        buf
    }

    /// A marker sequence just ended; classify it and act on the block state.
    fn finish(&mut self, events: &mut Vec<Event>) {
        self.state = State::Ground;
        let payload = std::mem::take(&mut self.payload);
        match classify(&payload) {
            Some(Marker::Start) => {
                // Everything before `C` (prompt, echoed input) is not output.
                self.buf.clear();
                self.capped = false;
                self.in_command = true;
                events.push(Event::CommandStart);
            }
            Some(Marker::End(exit_code)) => {
                // The marker sits at the end of the buffer; drop it.
                if self.in_command {
                    self.buf.truncate(self.seq_start);
                }
                events.push(Event::CommandEnd(exit_code));
            }
            None => {}
        }
    }
}

enum Marker {
    Start,
    End(Option<i32>),
}

/// Recognize `133;C` / `133;D[;<exit>]`, ignoring any other OSC.
fn classify(payload: &[u8]) -> Option<Marker> {
    let mut fields = payload.split(|byte| *byte == b';');
    if fields.next() != Some(b"133".as_slice()) {
        return None;
    }
    match fields.next()? {
        b"C" => Some(Marker::Start),
        b"D" => {
            Some(Marker::End(fields.next().and_then(|code| {
                std::str::from_utf8(code).ok()?.parse().ok()
            })))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Stream, MAX_OSC_PAYLOAD};

    fn run(chunks: &[&[u8]]) -> (Vec<Event>, Vec<u8>) {
        let mut stream = Stream::default();
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(stream.push(chunk));
        }
        let output = stream.take_output();
        (events, output)
    }

    #[test]
    fn slices_output_between_c_and_d() {
        let (events, output) = run(&[b"\x1b]133;C\x1b\\ok\x1b]133;D;0\x1b\\"]);
        assert_eq!(
            events,
            vec![Event::CommandStart, Event::CommandEnd(Some(0))]
        );
        assert_eq!(output, b"ok");
    }

    #[test]
    fn drops_prompt_and_echo_before_c() {
        // Prompt text and the echoed command must not leak into the output.
        let (_, output) = run(&[b"repo on main\n\xe2\x9d\xaf ls\x1b]133;C\x1b\\a\x1b]133;D\x1b\\"]);
        assert_eq!(output, b"a");
    }

    #[test]
    fn accepts_bel_terminator_and_missing_exit_code() {
        let (events, output) = run(&[b"\x1b]133;C\x07out\x1b]133;D\x07"]);
        assert_eq!(events, vec![Event::CommandStart, Event::CommandEnd(None)]);
        assert_eq!(output, b"out");
    }

    #[test]
    fn survives_markers_split_across_reads() {
        let (events, output) = run(&[
            b"\x1b]13",
            b"3;C\x1b",
            b"\\partial",
            b"\x1b]133;D;13",
            b"0\x1b\\",
        ]);
        assert_eq!(
            events,
            vec![Event::CommandStart, Event::CommandEnd(Some(130))]
        );
        assert_eq!(output, b"partial");
    }

    #[test]
    fn keeps_an_osc_title_inside_the_output() {
        // Only 133 is a boundary; other OSC sequences are ordinary output.
        let (_, output) = run(&[b"\x1b]133;C\x1b\\\x1b]0;title\x07body\x1b]133;D\x1b\\"]);
        assert_eq!(output, b"\x1b]0;title\x07body");
    }

    #[test]
    fn keeps_csi_colour_sequences() {
        let (_, output) = run(&[b"\x1b]133;C\x1b\\\x1b[32mok\x1b[0m\x1b]133;D\x1b\\"]);
        assert_eq!(output, b"\x1b[32mok\x1b[0m");
    }

    #[test]
    fn keeps_unicode_output_intact() {
        let (_, output) = run(&[
            b"\x1b]133;C\x1b\\",
            "中文 🎨\n".as_bytes(),
            b"\x1b]133;D\x1b\\",
        ]);
        assert_eq!(output, "中文 🎨\n".as_bytes());
    }

    #[test]
    fn ignores_prompt_noise_when_no_command_started() {
        // A shell that never emits `C` must record nothing rather than the
        // prompt and echoed input.
        let (events, output) = run(&[b"repo on main\n\xe2\x9d\xaf ls\x1b]133;D\x1b\\"]);
        assert_eq!(events, vec![Event::CommandEnd(None)]);
        assert!(output.is_empty());
    }

    #[test]
    fn repeated_commands_reset_the_buffer() {
        let mut stream = Stream::default();
        stream.push(b"\x1b]133;C\x1b\\first\x1b]133;D\x1b\\");
        assert_eq!(stream.take_output(), b"first");
        stream.push(b"\x1b]133;C\x1b\\second\x1b]133;D\x1b\\");
        assert_eq!(stream.take_output(), b"second");
    }

    #[test]
    fn end_without_start_yields_empty_output() {
        let mut stream = Stream::default();
        let events = stream.push(b"\x1b]133;D;1\x1b\\");
        assert_eq!(events, vec![Event::CommandEnd(Some(1))]);
        assert!(stream.take_output().is_empty());
    }

    #[test]
    fn ignores_other_osc_codes() {
        let (events, _) = run(&[b"\x1b]1338;C\x1b\\\x1b]133;C\x1b\\\x1b]133;D\x1b\\"]);
        assert_eq!(events, vec![Event::CommandStart, Event::CommandEnd(None)]);
    }

    #[test]
    fn abandoned_osc_payload_does_not_grow_without_bound() {
        let mut stream = Stream::default();
        // An unterminated OSC that keeps feeding bytes must not buffer forever.
        stream.push(b"\x1b]0;");
        for _ in 0..64 {
            stream.push(&[b'x'; 1024]);
        }
        assert!(
            stream.payload.len() <= MAX_OSC_PAYLOAD,
            "payload grew to {}",
            stream.payload.len()
        );

        // The parser recovers, so a later real marker still lands.
        let events = stream.push(b"\x1b]133;C\x1b\\\x1b]133;D;0\x1b\\");
        assert_eq!(
            events,
            vec![Event::CommandStart, Event::CommandEnd(Some(0))]
        );
    }
}
