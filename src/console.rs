//! A build's console output: incremental buffer, text clean-up, scrolling.
//! No IO.

use std::{cell::Cell, collections::VecDeque, time::Instant};

/// Lines kept in memory; older ones are dropped (and counted).
pub const MAX_LINES: usize = 100_000;

/// Lines Jenkins returns per `progressiveText` request while the build runs
/// (stapler's `LargeText`, `MAX_LINES_READ`); a full batch means more is
/// already there.
pub const JENKINS_BATCH_LINES: usize = 10_000;

/// Columns moved by one ←/→ step.
pub const HSCROLL_STEP: usize = 8;

/// One chunk of `logText/progressiveText`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleChunk {
    pub bytes: Vec<u8>,
    /// Byte offset to ask for next (`X-Text-Size`).
    pub next: u64,
    /// The build is still writing (`X-More-Data: true`).
    pub more: bool,
}

impl ConsoleChunk {
    /// Jenkins stopped at its batch size: more output is waiting already.
    pub fn is_full_batch(&self) -> bool {
        self.more && self.bytes.iter().filter(|&&b| b == b'\n').count() >= JENKINS_BATCH_LINES
    }
}

/// Request path for the console log of `number` of a job, from `start`.
pub fn console_path(full_name: &str, number: u64, start: u64) -> String {
    format!(
        "{}{number}/logText/progressiveText?start={start}",
        crate::builds::job_path(full_name)
    )
}

/// Clean up one line for display: drop ANSI escape sequences and other
/// control characters, keep only what's after the last `\r` (progress bars
/// redraw with it), expand tabs.
pub fn clean_line(raw: &str) -> String {
    let raw = raw.strip_suffix('\r').unwrap_or(raw);
    let raw = raw.rsplit('\r').next().unwrap_or(raw);
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek() {
                // CSI: ESC [ params final-byte(@..~)
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.peek() == Some(&'\\')) {
                            if c == '\x1b' {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                // Two-character escapes (ESC c, ESC =, ...).
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\t' => {
                let spaces = 8 - out.chars().count() % 8;
                out.extend(std::iter::repeat_n(' ', spaces));
            }
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Loading state of the console view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConsoleLoad {
    #[default]
    Loading,
    Loaded,
    Failed(String),
}

/// The console view of one build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleView {
    pub job: String,
    pub number: u64,
    pub load: ConsoleLoad,
    /// Complete lines (cleaned).
    lines: VecDeque<String>,
    /// The current, not yet terminated last line (cleaned when shown).
    partial: String,
    /// Trailing bytes of an incomplete UTF-8 character, completed by the
    /// next chunk (chunks end at arbitrary byte offsets).
    carry: Vec<u8>,
    /// Lines dropped from the front because of [`MAX_LINES`].
    pub dropped: usize,
    /// Byte offset of the next chunk.
    pub offset: u64,
    /// Jenkins says more output may come (the build is running).
    pub more: bool,
    /// A fetch is running.
    pub in_flight: bool,
    /// The last poll failed (output so far stays visible).
    pub last_error: Option<String>,
    pub fetched_at: Option<Instant>,
    pub attempted_at: Option<Instant>,
    /// Pinned to the bottom: new output scrolls into view.
    pub following: bool,
    /// First visible line (when not following).
    pub top: usize,
    /// Columns scrolled to the right.
    pub left: usize,
    /// Visible text height, recorded by the renderer for paging/clamping.
    pub viewport: Cell<u16>,
}

impl ConsoleView {
    pub fn new(job: String, number: u64) -> Self {
        Self {
            job,
            number,
            load: ConsoleLoad::Loading,
            lines: VecDeque::new(),
            partial: String::new(),
            carry: Vec::new(),
            dropped: 0,
            offset: 0,
            more: true,
            in_flight: true,
            last_error: None,
            fetched_at: None,
            attempted_at: None,
            following: true,
            top: 0,
            left: 0,
            viewport: Cell::new(0),
        }
    }

    /// Append a chunk of raw log bytes.
    pub fn push(&mut self, chunk: ConsoleChunk) {
        let mut bytes = std::mem::take(&mut self.carry);
        bytes.extend_from_slice(&chunk.bytes);
        // Keep an incomplete UTF-8 sequence at the end for next time.
        let valid = match std::str::from_utf8(&bytes) {
            Ok(_) => bytes.len(),
            Err(err) if err.error_len().is_none() => err.valid_up_to(),
            Err(_) => bytes.len(), // truly invalid bytes: decoded lossily below
        };
        self.carry = bytes.split_off(valid);
        let text = String::from_utf8_lossy(&bytes);

        let mut rest: &str = &text;
        while let Some(newline) = rest.find('\n') {
            self.partial.push_str(&rest[..newline]);
            let line = clean_line(&std::mem::take(&mut self.partial));
            self.lines.push_back(line);
            rest = &rest[newline + 1..];
        }
        self.partial.push_str(rest);
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
            self.dropped += 1;
            self.top = self.top.saturating_sub(1);
        }
        self.offset = chunk.next;
        self.more = chunk.more;
    }

    /// Lines to display: complete ones plus the current partial line.
    pub fn line_count(&self) -> usize {
        self.lines.len() + usize::from(!self.partial.is_empty())
    }

    /// Display line `i` (0-based, excluding dropped ones).
    pub fn line(&self, i: usize) -> Option<String> {
        match self.lines.get(i) {
            Some(line) => Some(line.clone()),
            None if i == self.lines.len() && !self.partial.is_empty() => {
                Some(clean_line(&self.partial))
            }
            None => None,
        }
    }

    fn page(&self) -> usize {
        usize::from(self.viewport.get()).max(1)
    }

    /// Largest useful `top`.
    pub fn max_top(&self) -> usize {
        self.line_count().saturating_sub(self.page())
    }

    /// First visible line, following or not.
    pub fn visible_top(&self) -> usize {
        if self.following {
            self.max_top()
        } else {
            self.top.min(self.max_top())
        }
    }

    /// Scroll by `delta` lines (negative = up). Leaving the bottom pauses
    /// following; reaching it resumes.
    pub fn scroll_by(&mut self, delta: isize) {
        let current = self.visible_top() as isize;
        let target = (current + delta).clamp(0, self.max_top() as isize) as usize;
        self.top = target;
        self.following = target >= self.max_top();
    }

    pub fn page_by(&mut self, pages: isize) {
        self.scroll_by(pages * self.page() as isize);
    }

    pub fn scroll_home(&mut self) {
        self.top = 0;
        self.following = self.max_top() == 0;
    }

    pub fn scroll_end(&mut self) {
        self.following = true;
    }

    /// Whether a poll is due: still running, idle, and `interval` since the
    /// last attempt.
    pub fn poll_due(&self, now: Instant, interval: std::time::Duration) -> bool {
        self.more
            && !self.in_flight
            && self
                .attempted_at
                .is_none_or(|last| now.saturating_duration_since(last) >= interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(text: &[u8], next: u64, more: bool) -> ConsoleChunk {
        ConsoleChunk {
            bytes: text.to_vec(),
            next,
            more,
        }
    }

    fn lines(view: &ConsoleView) -> Vec<String> {
        (0..view.line_count())
            .filter_map(|i| view.line(i))
            .collect()
    }

    #[test]
    fn path() {
        assert_eq!(
            console_path("team/my svc", 42, 1024),
            "job/team/job/my%20svc/42/logText/progressiveText?start=1024"
        );
    }

    #[test]
    fn cleaning() {
        assert_eq!(clean_line("\x1b[1;31mERROR\x1b[0m done"), "ERROR done");
        assert_eq!(clean_line("10%\r50%\r100%"), "100%");
        assert_eq!(clean_line("windows line\r"), "windows line");
        assert_eq!(clean_line("a\tb"), "a       b");
        assert_eq!(clean_line("\x1b]8;;http://x\x07link\x1b]8;;\x07"), "link");
        assert_eq!(clean_line("bell\x07 and \x1bcreset"), "bell and reset");
    }

    #[test]
    fn chunks_join_lines_and_utf8_across_boundaries() {
        let mut view = ConsoleView::new("job".into(), 1);
        let text = "first\nsecond é line\nthird".as_bytes();
        let split = text.iter().position(|&b| b == 0xc3).unwrap() + 1; // inside é
        view.push(chunk(&text[..split], split as u64, true));
        assert_eq!(lines(&view), ["first", "second "], "partial line shown");
        view.push(chunk(&text[split..], text.len() as u64, false));
        assert_eq!(lines(&view), ["first", "second é line", "third"]);
        assert_eq!(view.offset, text.len() as u64);
        assert!(!view.more);
    }

    #[test]
    fn line_cap_drops_oldest() {
        let mut view = ConsoleView::new("job".into(), 1);
        let text: String = (0..MAX_LINES + 5).map(|i| format!("line {i}\n")).collect();
        view.push(chunk(text.as_bytes(), text.len() as u64, false));
        assert_eq!(view.line_count(), MAX_LINES);
        assert_eq!(view.dropped, 5);
        assert_eq!(view.line(0).as_deref(), Some("line 5"));
    }

    fn view_with_lines(n: usize, viewport: u16) -> ConsoleView {
        let mut view = ConsoleView::new("job".into(), 1);
        let text: String = (0..n).map(|i| format!("{i}\n")).collect();
        view.push(chunk(text.as_bytes(), text.len() as u64, true));
        view.viewport.set(viewport);
        view
    }

    #[test]
    fn follow_pause_resume() {
        let mut view = view_with_lines(100, 10);
        assert!(view.following);
        assert_eq!(view.visible_top(), 90);

        view.scroll_by(-1);
        assert!(!view.following, "scrolling up pauses");
        assert_eq!(view.visible_top(), 89);
        // New output doesn't move a paused view.
        view.push(chunk(b"100\n101\n", 0, true));
        assert_eq!(view.visible_top(), 89);

        view.page_by(1);
        assert!(view.following, "back at the bottom resumes");
        assert_eq!(view.visible_top(), 92);

        view.scroll_home();
        assert_eq!((view.visible_top(), view.following), (0, false));
        view.page_by(-1);
        assert_eq!(view.visible_top(), 0, "clamped at the top");
        view.scroll_end();
        assert_eq!((view.visible_top(), view.following), (92, true));
    }

    #[test]
    fn short_logs_always_follow() {
        let mut view = view_with_lines(3, 10);
        view.scroll_home();
        assert!(view.following);
        assert_eq!(view.visible_top(), 0);
    }

    #[test]
    fn polling() {
        let now = Instant::now();
        let mut view = view_with_lines(1, 10);
        view.in_flight = false;
        view.attempted_at = Some(now);
        let second = std::time::Duration::from_secs(1);
        assert!(!view.poll_due(now, second));
        assert!(view.poll_due(now + second, second));
        view.more = false;
        assert!(
            !view.poll_due(now + second * 10, second),
            "finished: no polling"
        );
    }
}
