//! Pinned composer for interactive coding-harness sessions.
//!
//! Codex (and similar TUIs) keep a dedicated input box at the bottom of the
//! terminal so keystrokes never land in the agent's output stream. Permagent's
//! rustyline prompt only existed *between* turns, and during a turn the PTY
//! was back in cooked+echo — so typing mixed into the stream.
//!
//! This module owns that bottom strip for the whole session: a filled field
//! that stays available while the agent works, queues follow-ups, and
//! interrupts on escape. Rendering and key handling are pure so they can be
//! regression-tested without a TTY.

use super::output::Theme;
use console::measure_text_width;
use std::time::Instant;

pub const MAX_INPUT_ROWS: usize = 6;
pub const PLACEHOLDER: &str = "Ask Permagent to do anything";

const CYAN: (u8, u8, u8) = (0x00, 0xD5, 0xFF);
const GOLD: (u8, u8, u8) = (0xC4, 0x88, 0x3A);
const DIM: (u8, u8, u8) = (0x5A, 0x6D, 0x84);
const FG_DARK: (u8, u8, u8) = (0xE8, 0xE4, 0xDF);
const BG_DARK: (u8, u8, u8) = (0x1C, 0x24, 0x32);
const FG_LIGHT: (u8, u8, u8) = (0x1C, 0x24, 0x32);
const BG_LIGHT: (u8, u8, u8) = (0xE6, 0xE2, 0xDC);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    Char(char),
    Enter,
    AltEnter,
    Newline,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Escape,
    CtrlC,
    CtrlD,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerAction {
    Submit(String),
    Queue(String),
    Interrupt,
    Exit,
    Redraw,
}

#[derive(Debug, Clone, Default)]
pub struct ComposerState {
    pub buffer: String,
    pub cursor: usize,
    pub busy: bool,
    pub busy_since: Option<Instant>,
    pub queued: Vec<String>,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub stash: Option<String>,
    pub model: String,
    pub cwd: String,
    pub cost: String,
    pub tokens: String,
    pub maybe_exit: bool,
    pub light: bool,
}

impl ComposerState {
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        if busy {
            self.busy_since = Some(Instant::now());
        } else {
            self.busy_since = None;
        }
        self.maybe_exit = false;
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.light = matches!(theme, Theme::Light);
    }

    pub fn pop_queue(&mut self) -> Option<String> {
        if self.queued.is_empty() {
            None
        } else {
            Some(self.queued.remove(0))
        }
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.busy_since.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }

    pub fn apply(&mut self, key: KeyEvent) -> ComposerAction {
        self.maybe_exit = matches!(key, KeyEvent::CtrlC) && self.maybe_exit_after(key);
        match key {
            KeyEvent::Char(c) if !c.is_control() => {
                self.maybe_exit = false;
                self.history_idx = None;
                self.stash = None;
                self.insert_char(c);
                ComposerAction::Redraw
            }
            KeyEvent::Newline => {
                self.maybe_exit = false;
                self.insert_char('\n');
                ComposerAction::Redraw
            }
            KeyEvent::Backspace => {
                self.maybe_exit = false;
                self.backspace();
                ComposerAction::Redraw
            }
            KeyEvent::Delete => {
                self.maybe_exit = false;
                self.delete();
                ComposerAction::Redraw
            }
            KeyEvent::Left => {
                self.move_left();
                ComposerAction::Redraw
            }
            KeyEvent::Right => {
                self.move_right();
                ComposerAction::Redraw
            }
            KeyEvent::Home => {
                self.cursor = 0;
                ComposerAction::Redraw
            }
            KeyEvent::End => {
                self.cursor = self.buffer.len();
                ComposerAction::Redraw
            }
            KeyEvent::Up => {
                self.history_prev();
                ComposerAction::Redraw
            }
            KeyEvent::Down => {
                self.history_next();
                ComposerAction::Redraw
            }
            KeyEvent::Enter => self.submit(false),
            KeyEvent::AltEnter => self.submit(true),
            KeyEvent::Escape => self.on_escape(),
            KeyEvent::CtrlC => self.on_ctrl_c(),
            KeyEvent::CtrlD => {
                if self.buffer.is_empty() {
                    ComposerAction::Exit
                } else {
                    ComposerAction::Redraw
                }
            }
            KeyEvent::Tab | KeyEvent::Char(_) => ComposerAction::Redraw,
        }
    }

    fn maybe_exit_after(&self, key: KeyEvent) -> bool {
        matches!(key, KeyEvent::CtrlC)
            && self.buffer.is_empty()
            && !self.busy
            && self.queued.is_empty()
            && self.maybe_exit
    }

    fn submit(&mut self, force_queue: bool) -> ComposerAction {
        let text = self.buffer.trim().to_string();
        if text.is_empty() {
            return ComposerAction::Redraw;
        }
        self.push_history(text.clone());
        self.buffer.clear();
        self.cursor = 0;
        self.history_idx = None;
        self.stash = None;
        self.maybe_exit = false;
        if self.busy || force_queue {
            self.queued.push(text.clone());
            ComposerAction::Queue(text)
        } else {
            ComposerAction::Submit(text)
        }
    }

    fn on_escape(&mut self) -> ComposerAction {
        if !self.buffer.is_empty() {
            self.buffer.clear();
            self.cursor = 0;
            return ComposerAction::Redraw;
        }
        if !self.queued.is_empty() {
            self.queued.clear();
            return ComposerAction::Redraw;
        }
        if self.busy {
            ComposerAction::Interrupt
        } else {
            ComposerAction::Redraw
        }
    }

    fn on_ctrl_c(&mut self) -> ComposerAction {
        if !self.buffer.is_empty() {
            self.buffer.clear();
            self.cursor = 0;
            self.maybe_exit = false;
            return ComposerAction::Redraw;
        }
        if self.busy {
            return ComposerAction::Interrupt;
        }
        if self.maybe_exit {
            ComposerAction::Exit
        } else {
            self.maybe_exit = true;
            ComposerAction::Redraw
        }
    }

    fn insert_char(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = before_cursor(&self.buffer, self.cursor)
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        let start = self.cursor - prev;
        self.buffer.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    fn delete(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = after_cursor(&self.buffer, self.cursor)
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.buffer
            .replace_range(self.cursor..self.cursor + next, "");
    }

    fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = before_cursor(&self.buffer, self.cursor)
            .chars()
            .next_back()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor -= prev;
    }

    fn move_right(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = after_cursor(&self.buffer, self.cursor)
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(0);
        self.cursor += next;
    }

    fn push_history(&mut self, text: String) {
        if self.history.last() != Some(&text) {
            self.history.push(text);
        }
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_idx {
            None => {
                self.stash = Some(self.buffer.clone());
                let idx = self.history.len() - 1;
                self.history_idx = Some(idx);
                self.buffer = self.history[idx].clone();
                self.cursor = self.buffer.len();
            }
            Some(0) => {}
            Some(i) => {
                let idx = i - 1;
                self.history_idx = Some(idx);
                self.buffer = self.history[idx].clone();
                self.cursor = self.buffer.len();
            }
        }
    }

    fn history_next(&mut self) {
        let Some(i) = self.history_idx else {
            return;
        };
        if i + 1 >= self.history.len() {
            self.history_idx = None;
            self.buffer = self.stash.take().unwrap_or_default();
            self.cursor = self.buffer.len();
        } else {
            let idx = i + 1;
            self.history_idx = Some(idx);
            self.buffer = self.history[idx].clone();
            self.cursor = self.buffer.len();
        }
    }
}

pub fn prompt_glyph() -> &'static str {
    if is_vte_with_broken_emoji_width() {
        "> "
    } else {
        "❯ "
    }
}

fn is_vte_with_broken_emoji_width() -> bool {
    let Ok(vte_version) = std::env::var("VTE_VERSION") else {
        return false;
    };
    let Ok(version) = vte_version.parse::<u32>() else {
        return true;
    };
    version < 7000
}

/// Rows the composer occupies, including status + box + footer.
pub fn composer_rows(buffer: &str) -> usize {
    let input_rows = buffer.split('\n').count().clamp(1, MAX_INPUT_ROWS);
    4 + input_rows
}

pub fn status_line(state: &ComposerState) -> String {
    if state.maybe_exit {
        return "• Press Ctrl+C again to exit".to_string();
    }
    if !state.queued.is_empty() {
        let n = state.queued.len();
        let preview = truncate(state.queued.last().map(String::as_str).unwrap_or(""), 40);
        return format!("• Queued {n} · {preview}  —  will send when this turn ends");
    }
    if state.busy {
        return format!(
            "• Working ({}s • esc to interrupt · enter queues a follow-up)",
            state.elapsed_secs()
        );
    }
    "• Ready  ·  enter send · ctrl+j newline · /help".to_string()
}

fn truncate(s: &str, max: usize) -> String {
    let w = measure_text_width(s);
    if w <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = measure_text_width(&c.to_string());
        if used + cw + 1 > max {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

fn pad_to(s: &str, width: usize) -> String {
    let w = measure_text_width(s);
    if w >= width {
        truncate(s, width)
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

/// Plain (no ANSI) lines — used by tests to pin the layout contract:
/// a boxed field is always present, busy or idle.
#[cfg(test)]
pub fn render_plain(state: &ComposerState, width: usize) -> Vec<String> {
    let width = width.max(24);
    let inner = width.saturating_sub(2);
    let mut lines = Vec::new();
    lines.push(truncate(&status_line(state), width));

    let top = format!("╭{}╮", "─".repeat(inner));
    let bot = format!("╰{}╯", "─".repeat(inner));
    lines.push(top);

    let prompt = prompt_glyph();
    let prompt_w = measure_text_width(prompt);
    let content_w = inner.saturating_sub(2);
    let text_w = content_w.saturating_sub(prompt_w);

    let raw_lines: Vec<&str> = if state.buffer.is_empty() {
        vec![""]
    } else {
        state.buffer.split('\n').collect()
    };
    let visible = raw_lines.len().clamp(1, MAX_INPUT_ROWS);
    for i in 0..visible {
        let body = if state.buffer.is_empty() {
            PLACEHOLDER.to_string()
        } else {
            raw_lines.get(i).copied().unwrap_or("").to_string()
        };
        let prefix = if i == 0 { prompt } else { "  " };
        let row = format!("│ {}{} │", prefix, pad_to(&body, text_w));
        lines.push(truncate(&row, width));
    }
    lines.push(bot);

    let left = [state.model.as_str(), state.tokens.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let right = [state.cost.as_str(), state.cwd.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let footer = if right.is_empty() {
        left
    } else if left.is_empty() {
        right
    } else {
        let gap = width
            .saturating_sub(measure_text_width(&left) + measure_text_width(&right))
            .max(2);
        format!("{left}{}{right}", " ".repeat(gap))
    };
    lines.push(truncate(&footer, width));
    lines
}

fn sgr_fg(rgb: (u8, u8, u8)) -> String {
    format!("\x1b[38;2;{};{};{}m", rgb.0, rgb.1, rgb.2)
}
fn sgr_bg(rgb: (u8, u8, u8)) -> String {
    format!("\x1b[48;2;{};{};{}m", rgb.0, rgb.1, rgb.2)
}

/// ANSI frame for the TTY painter. Same layout as [`render_plain`], colored
/// and with a filled input field so it reads as a real text box, not a prompt
/// mixed into the stream.
pub fn render_ansi(state: &ComposerState, width: usize) -> Vec<String> {
    let width = width.max(24);
    let inner = width.saturating_sub(2);
    let (fg, bg, accent, dim) = if state.light {
        (FG_LIGHT, BG_LIGHT, CYAN, DIM)
    } else {
        (FG_DARK, BG_DARK, if state.busy { GOLD } else { CYAN }, DIM)
    };
    let reset = "\x1b[0m";
    let mut lines = Vec::new();

    let status_color = if state.busy || !state.queued.is_empty() {
        GOLD
    } else {
        accent
    };
    lines.push(format!(
        "{}{}{reset}",
        sgr_fg(status_color),
        pad_to(&status_line(state), width)
    ));

    let border = sgr_fg(accent);
    lines.push(format!("{border}╭{}╮{reset}", "─".repeat(inner)));

    let prompt = prompt_glyph();
    let prompt_w = measure_text_width(prompt);
    let content_w = inner.saturating_sub(2);
    let text_w = content_w.saturating_sub(prompt_w);
    let raw_lines: Vec<&str> = if state.buffer.is_empty() {
        vec![""]
    } else {
        state.buffer.split('\n').collect()
    };
    let visible = raw_lines.len().clamp(1, MAX_INPUT_ROWS);
    let fill = format!("{}{}", sgr_bg(bg), sgr_fg(fg));
    for i in 0..visible {
        let (body, is_placeholder) = if state.buffer.is_empty() {
            (PLACEHOLDER.to_string(), true)
        } else {
            (raw_lines.get(i).copied().unwrap_or("").to_string(), false)
        };
        let prefix = if i == 0 { prompt } else { "  " };
        let body_s = pad_to(&body, text_w);
        let body_color = if is_placeholder {
            sgr_fg(dim)
        } else {
            sgr_fg(fg)
        };
        lines.push(format!(
            "{border}│{reset}{fill} {}{prefix}{reset}{fill}{body_color}{body_s}{reset}{fill} {reset}{border}│{reset}",
            sgr_fg(accent),
        ));
    }
    lines.push(format!("{border}╰{}╯{reset}", "─".repeat(inner)));

    let left = [state.model.as_str(), state.tokens.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let right = [state.cost.as_str(), state.cwd.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let footer = if right.is_empty() {
        left
    } else if left.is_empty() {
        right
    } else {
        let gap = width
            .saturating_sub(measure_text_width(&left) + measure_text_width(&right))
            .max(2);
        format!("{left}{}{right}", " ".repeat(gap))
    };
    lines.push(format!("{}{}{reset}", sgr_fg(dim), pad_to(&footer, width)));
    lines
}

/// Byte-level key parser. Enter (`\\r`) submits; Ctrl+J (`\\n`) is a newline
/// so a follow-up can be composed without leaving the field.
pub fn decode_keys(input: &[u8], pending: &mut Vec<u8>) -> Vec<KeyEvent> {
    pending.extend_from_slice(input);
    let mut events = Vec::new();
    let mut i = 0;
    let bytes = pending.as_slice();
    while i < bytes.len() {
        match bytes[i] {
            0x0d => {
                // ESC-CR is Alt+Enter on many terminals.
                if i > 0 && bytes[i - 1] == 0x1b {
                    // already consumed as part of alt seq below
                }
                events.push(KeyEvent::Enter);
                i += 1;
            }
            0x0a => {
                events.push(KeyEvent::Newline);
                i += 1;
            }
            0x03 => {
                events.push(KeyEvent::CtrlC);
                i += 1;
            }
            0x04 => {
                events.push(KeyEvent::CtrlD);
                i += 1;
            }
            0x08 | 0x7f => {
                events.push(KeyEvent::Backspace);
                i += 1;
            }
            0x09 => {
                events.push(KeyEvent::Tab);
                i += 1;
            }
            0x1b => {
                if i + 1 >= bytes.len() {
                    break;
                }
                if bytes[i + 1] == 0x0d {
                    events.push(KeyEvent::AltEnter);
                    i += 2;
                    continue;
                }
                if bytes[i + 1] != b'[' {
                    events.push(KeyEvent::Escape);
                    i += 1;
                    continue;
                }
                if i + 2 >= bytes.len() {
                    break;
                }
                match bytes[i + 2] {
                    b'A' => events.push(KeyEvent::Up),
                    b'B' => events.push(KeyEvent::Down),
                    b'C' => events.push(KeyEvent::Right),
                    b'D' => events.push(KeyEvent::Left),
                    b'H' => events.push(KeyEvent::Home),
                    b'F' => events.push(KeyEvent::End),
                    b'3' => {
                        if i + 3 < bytes.len() && bytes[i + 3] == b'~' {
                            events.push(KeyEvent::Delete);
                            i += 4;
                            continue;
                        }
                        if i + 3 >= bytes.len() {
                            break;
                        }
                        events.push(KeyEvent::Escape);
                        i += 1;
                        continue;
                    }
                    _ => {
                        events.push(KeyEvent::Escape);
                        i += 1;
                        continue;
                    }
                }
                i += 3;
            }
            b => {
                // UTF-8
                let width = utf8_width(b);
                if i + width > bytes.len() {
                    break;
                }
                if let Ok(s) = std::str::from_utf8(&bytes[i..i + width]) {
                    if let Some(c) = s.chars().next() {
                        if !c.is_control() {
                            events.push(KeyEvent::Char(c));
                        }
                    }
                }
                i += width;
            }
        }
    }
    pending.drain(..i);
    events
}

fn utf8_width(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b & 0xe0 == 0xc0 {
        2
    } else if b & 0xf0 == 0xe0 {
        3
    } else if b & 0xf8 == 0xf0 {
        4
    } else {
        1
    }
}

pub fn abbreviate_home(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if let Some(rest) = path.strip_prefix(&home) {
                return format!("~{rest}");
            }
        }
    }
    path.to_string()
}

/// `cursor` is always a UTF-8 boundary: insert/delete/move step by `len_utf8`.
fn before_cursor(s: &str, cursor: usize) -> &str {
    s.get(..cursor).unwrap_or("")
}

fn after_cursor(s: &str, cursor: usize) -> &str {
    s.get(cursor..).unwrap_or("")
}

pub fn format_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M tok", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k tok", n as f64 / 1_000.0)
    } else if n > 0 {
        format!("{n} tok")
    } else {
        String::new()
    }
}

pub fn format_cost(session_total_usd: Option<f64>, total_tokens: i32) -> String {
    match session_total_usd {
        Some(v) if v == 0.0 && total_tokens > 0 => "$0.00".to_string(),
        Some(v) => format!("${v:.2}"),
        None => String::new(),
    }
}

// ── TTY overlay (unix) ──────────────────────────────────────────────────────

#[cfg(unix)]
mod tty {
    use super::*;
    use std::io::{self, Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    pub struct RawMode {
        fd: i32,
        original: libc::termios,
        applied: bool,
    }

    impl RawMode {
        pub fn new() -> io::Result<Self> {
            let fd = libc::STDIN_FILENO;
            let mut original = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut raw = original;
            unsafe { libc::cfmakeraw(&mut raw) };
            // Keep output post-processing so println! still returns the
            // cursor; otherwise agent stream staircases across the field.
            raw.c_oflag |= libc::OPOST;
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                fd,
                original,
                applied: true,
            })
        }

        pub fn restore(&mut self) {
            if self.applied {
                unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
                self.applied = false;
            }
        }

        pub fn apply(&mut self) {
            if self.applied {
                return;
            }
            let mut raw = self.original;
            unsafe { libc::cfmakeraw(&mut raw) };
            raw.c_oflag |= libc::OPOST;
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &raw) };
            self.applied = true;
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            self.restore();
        }
    }

    fn poll_stdin(timeout_ms: i32) -> bool {
        let mut pfd = libc::pollfd {
            fd: libc::STDIN_FILENO,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut pfd, 1, timeout_ms) > 0 }
    }

    pub struct Composer {
        pub state: ComposerState,
        raw: Option<RawMode>,
        key_rx: mpsc::UnboundedReceiver<Vec<u8>>,
        parse_buf: Vec<u8>,
        paused: Arc<AtomicBool>,
        cols: usize,
        rows: usize,
        last_height: usize,
        installed: bool,
    }

    impl Composer {
        pub fn try_install() -> Option<Self> {
            use std::io::IsTerminal;
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return None;
            }
            let raw = RawMode::new().ok()?;
            let (tx, key_rx) = mpsc::unbounded_channel();
            let paused = Arc::new(AtomicBool::new(false));
            let paused_t = paused.clone();
            std::thread::Builder::new()
                .name("permagent-composer".into())
                .spawn(move || {
                    let mut stdin = io::stdin();
                    let mut buf = [0u8; 64];
                    loop {
                        if paused_t.load(Ordering::Relaxed) {
                            std::thread::sleep(std::time::Duration::from_millis(30));
                            continue;
                        }
                        if !poll_stdin(50) {
                            continue;
                        }
                        match stdin.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                if tx.send(buf[..n].to_vec()).is_err() {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                })
                .ok()?;

            let (rows, cols) = console::Term::stdout().size();
            let mut c = Self {
                state: ComposerState::default(),
                raw: Some(raw),
                key_rx,
                parse_buf: Vec::new(),
                paused,
                cols: cols as usize,
                rows: rows as usize,
                last_height: 0,
                installed: false,
            };
            c.install_region();
            c.paint();
            Some(c)
        }

        fn refresh_size(&mut self) {
            let (rows, cols) = console::Term::stdout().size();
            self.rows = rows.max(8) as usize;
            self.cols = cols.max(24) as usize;
        }

        fn region_bottom(&self) -> usize {
            self.rows.saturating_sub(self.last_height).max(1)
        }

        fn install_region(&mut self) {
            self.refresh_size();
            let height = composer_rows(&self.state.buffer);
            if !self.installed {
                // Push the cursor up so the reserved strip is empty.
                for _ in 0..height {
                    print!("\r\n");
                }
                self.installed = true;
            }
            self.last_height = height;
            let bottom = self.region_bottom();
            print!("\x1b[1;{bottom}r");
            print!("\x1b[{bottom};1H");
            let _ = io::stdout().flush();
        }

        fn reset_region(&self) {
            print!("\x1b[r");
            print!("\x1b[?25h");
            let _ = io::stdout().flush();
        }

        pub fn paint(&mut self) {
            self.refresh_size();
            let height = composer_rows(&self.state.buffer);
            if height != self.last_height {
                self.last_height = height;
                let bottom = self.region_bottom();
                print!("\x1b[1;{bottom}r");
            }
            let start = self.rows.saturating_sub(height) + 1;
            let frame = render_ansi(&self.state, self.cols);
            print!("\x1b[?25l");
            for (i, line) in frame.iter().enumerate() {
                print!("\x1b[{};1H\x1b[K{}", start + i, line);
            }
            // Park the cursor inside the field so typing never lands in the stream.
            let prompt_w = measure_text_width(prompt_glyph());
            let prefix = before_cursor(&self.state.buffer, self.state.cursor);
            let cursor_line = prefix.bytes().filter(|&b| b == b'\n').count();
            let line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
            let col_text = measure_text_width(
                self.state
                    .buffer
                    .get(line_start..self.state.cursor)
                    .unwrap_or(""),
            );
            let row =
                start + 1 /* status */ + 1 /* top border */ + cursor_line.min(MAX_INPUT_ROWS - 1);
            let col = 3 + prompt_w + col_text;
            print!("\x1b[{};{}H\x1b[?25h", row, col.min(self.cols));
            let _ = io::stdout().flush();
        }

        pub fn set_busy(&mut self, busy: bool) {
            self.state.set_busy(busy);
            self.paint();
        }

        pub fn suspend(&mut self) {
            self.paused.store(true, Ordering::Relaxed);
            self.reset_region();
            if let Some(raw) = self.raw.as_mut() {
                raw.restore();
            }
            println!();
            let _ = io::stdout().flush();
        }

        pub fn resume(&mut self) {
            if let Some(raw) = self.raw.as_mut() {
                raw.apply();
            }
            self.paused.store(false, Ordering::Relaxed);
            self.installed = false;
            self.install_region();
            self.paint();
        }

        /// Drain pending key bytes and apply them. Returns the first
        /// non-redraw action, if any.
        pub fn drain_keys(&mut self) -> Option<ComposerAction> {
            let mut first = None;
            while let Ok(bytes) = self.key_rx.try_recv() {
                let events = decode_keys(&bytes, &mut self.parse_buf);
                for ev in events {
                    let action = self.state.apply(ev);
                    if !matches!(action, ComposerAction::Redraw) && first.is_none() {
                        first = Some(action);
                    }
                }
            }
            self.paint();
            first
        }

        pub async fn next_action(&mut self) -> Option<ComposerAction> {
            loop {
                tokio::select! {
                    msg = self.key_rx.recv() => {
                        let bytes = msg?;
                        let events = decode_keys(&bytes, &mut self.parse_buf);
                        let mut out = None;
                        for ev in events {
                            let action = self.state.apply(ev);
                            if !matches!(action, ComposerAction::Redraw) {
                                out = Some(action);
                            }
                        }
                        self.paint();
                        if let Some(action) = out {
                            return Some(action);
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                        if self.state.busy {
                            self.paint();
                        }
                    }
                }
            }
        }
    }

    impl Drop for Composer {
        fn drop(&mut self) {
            self.paused.store(true, Ordering::Relaxed);
            self.reset_region();
            print!("\r\n");
            let _ = io::stdout().flush();
        }
    }
}

#[cfg(unix)]
pub use tty::Composer;

#[cfg(not(unix))]
pub struct Composer {
    pub state: ComposerState,
}

#[cfg(not(unix))]
impl Composer {
    pub fn try_install() -> Option<Self> {
        None
    }
    pub fn paint(&mut self) {}
    pub fn set_busy(&mut self, busy: bool) {
        self.state.set_busy(busy);
    }
    pub fn suspend(&mut self) {}
    pub fn resume(&mut self) {}
    pub fn drain_keys(&mut self) -> Option<ComposerAction> {
        None
    }
    pub async fn next_action(&mut self) -> Option<ComposerAction> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle() -> ComposerState {
        ComposerState::default()
    }

    #[test]
    fn boxed_field_is_always_present_idle_and_busy() {
        let mut state = idle();
        let idle_frame = render_plain(&state, 60);
        assert!(
            idle_frame.iter().any(|l| l.contains('╭')),
            "idle frame must draw a top border: {idle_frame:?}"
        );
        assert!(
            idle_frame.iter().any(|l| l.contains(PLACEHOLDER)),
            "idle frame must show the placeholder: {idle_frame:?}"
        );
        assert!(
            idle_frame.iter().any(|l| l.contains("Ready")),
            "idle status: {idle_frame:?}"
        );

        state.set_busy(true);
        let busy_frame = render_plain(&state, 60);
        assert!(
            busy_frame.iter().any(|l| l.contains('╭')),
            "busy frame must keep the box: {busy_frame:?}"
        );
        assert!(
            busy_frame.iter().any(|l| l.contains("Working")),
            "busy status: {busy_frame:?}"
        );
        assert_eq!(idle_frame.len(), busy_frame.len());
    }

    #[test]
    fn typing_stays_in_the_field_not_the_status() {
        let mut state = idle();
        state.set_busy(true);
        state.apply(KeyEvent::Char('h'));
        state.apply(KeyEvent::Char('i'));
        let frame = render_plain(&state, 60);
        let box_line = frame
            .iter()
            .find(|l| l.contains('│') && l.contains("hi"))
            .cloned()
            .expect("typed text belongs inside the box");
        assert!(!box_line.contains("Working"));
        let status = &frame[0];
        assert!(status.contains("Working"));
        assert!(!status.contains("hi"));
    }

    #[test]
    fn enter_while_busy_queues_instead_of_submitting() {
        let mut state = idle();
        state.set_busy(true);
        state.apply(KeyEvent::Char('s'));
        state.apply(KeyEvent::Char('t'));
        state.apply(KeyEvent::Char('e'));
        state.apply(KeyEvent::Char('e'));
        state.apply(KeyEvent::Char('r'));
        let action = state.apply(KeyEvent::Enter);
        assert_eq!(action, ComposerAction::Queue("steer".into()));
        assert!(state.buffer.is_empty());
        assert_eq!(state.queued, vec!["steer".to_string()]);
        let frame = render_plain(&state, 80);
        assert!(frame[0].contains("Queued"));
    }

    #[test]
    fn enter_while_idle_submits() {
        let mut state = idle();
        for c in "fix the bug".chars() {
            state.apply(KeyEvent::Char(c));
        }
        let action = state.apply(KeyEvent::Enter);
        assert_eq!(action, ComposerAction::Submit("fix the bug".into()));
        assert!(state.buffer.is_empty());
    }

    #[test]
    fn escape_interrupts_only_when_busy_and_empty() {
        let mut state = idle();
        state.apply(KeyEvent::Char('x'));
        assert_eq!(state.apply(KeyEvent::Escape), ComposerAction::Redraw);
        assert!(state.buffer.is_empty());

        state.set_busy(true);
        assert_eq!(state.apply(KeyEvent::Escape), ComposerAction::Interrupt);
    }

    #[test]
    fn ctrl_c_twice_on_empty_idle_exits() {
        let mut state = idle();
        assert_eq!(state.apply(KeyEvent::CtrlC), ComposerAction::Redraw);
        assert!(state.maybe_exit);
        assert_eq!(state.apply(KeyEvent::CtrlC), ComposerAction::Exit);
    }

    #[test]
    fn decode_enter_vs_newline() {
        let mut pending = Vec::new();
        assert_eq!(decode_keys(b"\r", &mut pending), vec![KeyEvent::Enter]);
        assert!(pending.is_empty());
        assert_eq!(decode_keys(b"\n", &mut pending), vec![KeyEvent::Newline]);
        assert_eq!(decode_keys(b"\x1b[A", &mut pending), vec![KeyEvent::Up]);
        assert_eq!(
            decode_keys(b"\x7f", &mut pending),
            vec![KeyEvent::Backspace]
        );
        assert_eq!(
            decode_keys(&[0x1b, 0x0d], &mut pending),
            vec![KeyEvent::AltEnter]
        );
    }

    #[test]
    fn decode_holds_incomplete_escape() {
        let mut pending = Vec::new();
        assert!(decode_keys(b"\x1b", &mut pending).is_empty());
        assert_eq!(pending, vec![0x1b]);
        assert_eq!(decode_keys(b"[C", &mut pending), vec![KeyEvent::Right]);
        assert!(pending.is_empty());
    }

    #[test]
    fn history_walks_then_restores_stash() {
        let mut state = idle();
        for c in "one".chars() {
            state.apply(KeyEvent::Char(c));
        }
        state.apply(KeyEvent::Enter);
        for c in "two".chars() {
            state.apply(KeyEvent::Char(c));
        }
        state.apply(KeyEvent::Enter);
        for c in "draft".chars() {
            state.apply(KeyEvent::Char(c));
        }
        state.apply(KeyEvent::Up);
        assert_eq!(state.buffer, "two");
        state.apply(KeyEvent::Up);
        assert_eq!(state.buffer, "one");
        state.apply(KeyEvent::Down);
        state.apply(KeyEvent::Down);
        assert_eq!(state.buffer, "draft");
    }

    #[test]
    fn composer_rows_grow_with_newlines_and_cap() {
        assert_eq!(composer_rows(""), 5);
        assert_eq!(composer_rows("a\nb"), 6);
        let many = (0..20).map(|_| "x").collect::<Vec<_>>().join("\n");
        assert_eq!(composer_rows(&many), 4 + MAX_INPUT_ROWS);
    }

    #[test]
    fn footer_carries_model_and_cwd() {
        let mut state = idle();
        state.model = "claude-opus-4-6".into();
        state.cwd = "~/dev/app".into();
        state.cost = "$0.12".into();
        let frame = render_plain(&state, 80);
        let footer = frame.last().unwrap();
        assert!(footer.contains("claude-opus-4-6"), "{footer}");
        assert!(footer.contains("~/dev/app"), "{footer}");
        assert!(footer.contains("$0.12"), "{footer}");
    }
}
