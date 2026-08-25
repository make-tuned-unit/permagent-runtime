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
use std::time::{Duration, Instant};

pub const MAX_INPUT_ROWS: usize = 6;
pub const PLACEHOLDER: &str = "Ask Permagent to do anything";

/// DEC private mode 2004. Also the readiness signal the desktop app's
/// terminal pane waits for before delivering a follow-up prompt
/// (`ui/command-center/src/components/terminal/followUpDelivery.ts`).
pub const ENABLE_BRACKETED_PASTE: &str = "\x1b[?2004h";
pub const DISABLE_BRACKETED_PASTE: &str = "\x1b[?2004l";

const CYAN: (u8, u8, u8) = (0x00, 0xD5, 0xFF);
const MAGENTA: (u8, u8, u8) = (0xA8, 0x55, 0xCC);
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
    /// `ESC [ 200 ~` — everything until [`KeyEvent::PasteEnd`] is literal text,
    /// so a pasted directive stays ONE message instead of one per line.
    PasteStart,
    /// `ESC [ 201 ~`
    PasteEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerAction {
    Submit(String),
    Queue(String),
    Interrupt,
    Exit,
    Redraw,
}

/// What the turn is doing right now.
///
/// Added 2026-08-25. A GLM-5.3 session spent four minutes and fourteen
/// thousand reasoning tokens in one stream with no text and no tool call, and
/// the status line said only "Working (Ns)" the whole time. The user read it as
/// a hang, which was reasonable — nothing on screen distinguished thinking from
/// stuck from waiting on a rate limit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TurnPhase {
    /// Nothing has reported a phase. Renders as the generic "Working" the
    /// composer has always shown — we say what we know, and this is the case
    /// where we know nothing more than "a turn is in flight".
    #[default]
    Unreported,
    /// The model is producing reasoning or text.
    Thinking,
    /// A tool is running. `target` is the thing it is acting on — a path, a
    /// pattern, the head of a command — and is what makes the line worth
    /// reading, so a collapsed tool line still says WHAT it is doing.
    RunningTool {
        name: String,
        target: Option<String>,
    },
}

/// After this long with nothing printed, say so and name the way out. Chosen to
/// sit well past a slow tool call and well inside a user's patience.
pub const STUCK_HINT_AFTER: Duration = Duration::from_secs(60);

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
    /// Inside `ESC[200~ … ESC[201~`: the terminal is replaying pasted bytes,
    /// not reporting keystrokes.
    pub pasting: bool,
    /// Last pasted byte was CR — used to fold a CRLF pair into one line break.
    pub paste_saw_cr: bool,
    /// What this turn is doing, as last reported by the session loop.
    pub phase: TurnPhase,
    /// Tokens this turn, if the provider has reported any yet.
    pub turn_tokens: Option<u64>,
    /// When anything was last printed. `None` means nothing yet this turn.
    pub last_output_at: Option<Instant>,
    /// A provider wait in progress, already rendered by
    /// `permagent::providers::wait_status`. Pre-rendered rather than modelled
    /// here so this module stays pure and testable.
    pub provider_wait: Option<String>,
}

impl ComposerState {
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        if busy {
            self.busy_since = Some(Instant::now());
        } else {
            self.busy_since = None;
        }
        // Either way the phase starts unknown: a previous turn's tool name
        // lingering into the next turn would be a lie, and claiming "Thinking"
        // before the model has said anything is a guess.
        self.phase = TurnPhase::Unreported;
        self.turn_tokens = None;
        self.last_output_at = None;
        self.provider_wait = None;
        self.maybe_exit = false;
    }

    /// What the turn is doing now. Called by the session loop as tool calls
    /// start and finish.
    pub fn set_phase(&mut self, phase: TurnPhase) {
        self.phase = phase;
    }

    /// Note that something was printed, so the silence timer restarts.
    pub fn mark_output(&mut self) {
        self.last_output_at = Some(Instant::now());
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
        if self.pasting {
            return self.apply_pasted(key);
        }
        self.maybe_exit = matches!(key, KeyEvent::CtrlC) && self.maybe_exit_after(key);
        match key {
            KeyEvent::PasteStart => {
                self.maybe_exit = false;
                self.history_idx = None;
                self.stash = None;
                self.pasting = true;
                self.paste_saw_cr = false;
                ComposerAction::Redraw
            }
            KeyEvent::PasteEnd => ComposerAction::Redraw,
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

    /// Keys arriving between the paste markers are *content*. A terminal
    /// replays a pasted newline as CR, so treating CR as Enter here is what
    /// split one 32-line directive into 32 queued messages; inside a paste it
    /// is a literal line break, and only the CR that follows `ESC[201~`
    /// submits.
    fn apply_pasted(&mut self, key: KeyEvent) -> ComposerAction {
        match key {
            KeyEvent::PasteEnd => {
                self.pasting = false;
            }
            KeyEvent::Enter | KeyEvent::AltEnter => {
                self.insert_char('\n');
                self.paste_saw_cr = true;
                return ComposerAction::Redraw;
            }
            KeyEvent::Newline => {
                // A CRLF pair is one line break, not two.
                if !self.paste_saw_cr {
                    self.insert_char('\n');
                }
            }
            KeyEvent::Tab => self.insert_char('\t'),
            KeyEvent::Char(c) if !c.is_control() => self.insert_char(c),
            // Escape/Ctrl+C bytes inside a paste must not interrupt the turn
            // or exit the session; drop everything else.
            _ => {}
        }
        self.paste_saw_cr = false;
        ComposerAction::Redraw
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct InputLayout {
    rows: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
}

/// Return a display-width-limited slice which keeps `cursor` visible.
fn line_viewport(line: &str, cursor: usize, width: usize) -> (String, usize) {
    if width == 0 {
        return (String::new(), 0);
    }
    let cursor = cursor.min(line.len());
    let chars: Vec<char> = line.chars().collect();
    // `cursor` is a byte offset from the edit buffer; `get` degrades a stray
    // mid-codepoint offset to "count the whole line" instead of panicking.
    let cursor_idx = line.get(..cursor).unwrap_or(line).chars().count();
    let mut start = cursor_idx;
    let mut before_width = 0;
    while start > 0 {
        let char_width = measure_text_width(&chars[start - 1].to_string());
        if before_width + char_width > width {
            break;
        }
        before_width += char_width;
        start -= 1;
    }

    let mut body = String::new();
    let mut body_width = 0;
    for ch in chars.into_iter().skip(start) {
        let char_width = measure_text_width(&ch.to_string());
        if body_width + char_width > width {
            break;
        }
        body.push(ch);
        body_width += char_width;
    }
    (body, before_width.min(width))
}

/// A single width-aware layout plan drives height, rendered rows, and cursor.
fn input_layout(state: &ComposerState, width: usize) -> InputLayout {
    let width = width.max(24);
    let prompt_w = measure_text_width(prompt_glyph());
    let text_w = width
        .saturating_sub(2) // borders
        .saturating_sub(2) // box padding
        .saturating_sub(prompt_w);
    let display = sanitize_keep_newlines(&state.buffer);
    let logical: Vec<&str> = if display.is_empty() {
        vec![""]
    } else {
        display.split('\n').collect()
    };
    let prefix = before_cursor(&state.buffer, state.cursor);
    let cursor_line = prefix.bytes().filter(|&byte| byte == b'\n').count();
    let cursor_line_start = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let cursor_in_line = state.cursor.saturating_sub(cursor_line_start);
    let visible_count = logical.len().clamp(1, MAX_INPUT_ROWS);
    let first = if logical.len() <= MAX_INPUT_ROWS {
        0
    } else {
        cursor_line
            .saturating_sub(MAX_INPUT_ROWS - 1)
            .min(logical.len() - MAX_INPUT_ROWS)
    };

    let mut rows = Vec::with_capacity(visible_count);
    let mut cursor_col = 0;
    for (logical_idx, line) in logical.iter().enumerate().skip(first).take(visible_count) {
        let line = *line;
        if display.is_empty() {
            rows.push(truncate(PLACEHOLDER, text_w));
        } else if logical_idx == cursor_line {
            let (visible, col) = line_viewport(line, cursor_in_line, text_w);
            rows.push(visible);
            cursor_col = col;
        } else {
            rows.push(truncate(line, text_w));
        }
    }

    InputLayout {
        rows,
        cursor_row: cursor_line.saturating_sub(first).min(visible_count - 1),
        cursor_col,
    }
}

/// Rows the composer occupies, including status + box + footer.
pub fn composer_rows(state: &ComposerState, width: usize) -> usize {
    4 + input_layout(state, width).rows.len()
}

pub fn status_line(state: &ComposerState) -> String {
    if state.maybe_exit {
        return "• Press Ctrl+C again to exit".to_string();
    }
    if !state.queued.is_empty() {
        let n = state.queued.len();
        let preview = queue_preview(state.queued.last().map(String::as_str).unwrap_or(""));
        return format!("• Queued {n} · {preview}  —  will send when this turn ends");
    }
    if state.busy {
        return busy_status_line(state);
    }
    "• Ready  ·  enter send · ctrl+j newline · /help".to_string()
}

/// The busy line, most-specific fact first.
///
/// Everything lives on ONE row on purpose: the pinned strip's height is
/// `4 + input rows`, and a status line that wrapped would push the composer's
/// own box off its reserved rows and smear it into the scrollback. The caller
/// truncates to width; what goes leftmost is what survives a narrow terminal.
fn busy_status_line(state: &ComposerState) -> String {
    // A provider wait outranks everything: it is the only case where the agent
    // is doing nothing at all, and the only one with a number that counts down.
    if let Some(wait) = &state.provider_wait {
        return format!("• {wait} · esc to interrupt");
    }

    let mut line = format!("• {}", phase_text(&state.phase));
    line.push_str(&format!(" ({}s", state.elapsed_secs()));
    if let Some(tokens) = state.turn_tokens {
        line.push_str(&format!(" · {}", format_turn_tokens(tokens)));
    }
    line.push(')');

    // Silence is the thing the user cannot interpret. Name it and name the way
    // out; do not wait for them to guess whether it is stuck.
    if let Some(quiet) = quiet_for(state) {
        if quiet >= STUCK_HINT_AFTER {
            line.push_str(&format!(
                " · no output for {}s — Ctrl+C to interrupt",
                quiet.as_secs()
            ));
            return line;
        }
    }

    line.push_str(" · esc to interrupt · enter queues a follow-up");
    line
}

/// Read the phase off an agent message, if it says anything about one.
///
/// `None` means "this message does not change what we can honestly claim the
/// turn is doing" — the status keeps whatever it had rather than flickering.
pub fn phase_of(message: &permagent::conversation::message::Message) -> Option<TurnPhase> {
    use permagent::conversation::message::MessageContent;
    // A tool request is the most specific thing a message can carry, so it wins
    // over reasoning that arrived in the same message.
    for content in &message.content {
        if let MessageContent::ToolRequest(request) = content {
            if let Ok(call) = &request.tool_call {
                return Some(TurnPhase::RunningTool {
                    name: call.name.to_string(),
                    target: tool_target(call.arguments.as_ref()),
                });
            }
        }
    }
    for content in &message.content {
        match content {
            MessageContent::Thinking(_) => return Some(TurnPhase::Thinking),
            MessageContent::Text(_) => return Some(TurnPhase::Thinking),
            _ => {}
        }
    }
    None
}

/// The thing a tool is acting on, for the status row.
///
/// Argument names differ per tool, so this reads the handful that actually name
/// a target and gives up rather than printing a JSON blob at the user. Order is
/// most-specific first.
fn tool_target(arguments: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<String> {
    let args = arguments?;
    for key in [
        "path",
        "file_path",
        "pattern",
        "command",
        "query",
        "instructions",
    ] {
        if let Some(value) = args.get(key).and_then(|v| v.as_str()) {
            let value = value.trim();
            if !value.is_empty() {
                // A shell command is often a whole pipeline; its head is the
                // part worth a status row.
                let head = value.lines().next().unwrap_or(value);
                return Some(head.to_string());
            }
        }
    }
    None
}

/// How long since anything was printed this turn. `None` when no turn is in
/// flight, or when nothing has been printed yet and there is no turn start to
/// measure from.
fn quiet_for(state: &ComposerState) -> Option<Duration> {
    // Falling back to the turn start matters: a turn that prints NOTHING from
    // its first instant — the exact GLM case — has no last-output mark, and
    // that is precisely when the hint is most needed.
    let since = state.last_output_at.or(state.busy_since)?;
    Some(since.elapsed())
}

fn phase_text(phase: &TurnPhase) -> String {
    match phase {
        TurnPhase::Unreported => "Working".to_string(),
        TurnPhase::Thinking => "Thinking".to_string(),
        TurnPhase::RunningTool { name, target } => match target {
            Some(target) if !target.is_empty() => {
                format!("{name} {}", truncate(&sanitize_row(target), 44))
            }
            _ => name.clone(),
        },
    }
}

/// `840 tok` / `12.4k tok` — short enough to sit in a status row without
/// pushing the interrupt hint off a narrow terminal. Distinct from
/// `format_tokens` below, which formats a session total for the footer.
fn format_turn_tokens(tokens: u64) -> String {
    if tokens < 10_000 {
        format!("{tokens} tok")
    } else {
        format!("{:.1}k tok", tokens as f64 / 1000.0)
    }
}

/// One queued message is one row: its first line, plus how many more it
/// carries. A pasted directive is a single queue entry, not one per line.
fn queue_preview(text: &str) -> String {
    let mut lines = text.lines();
    let first = sanitize_row(lines.next().unwrap_or(""));
    let extra = lines.count();
    if extra == 0 {
        truncate(&first, 40)
    } else {
        format!("{} (+{extra} lines)", truncate(&first, 26))
    }
}

/// Replace control bytes with spaces so pasted text can never emit a raw
/// newline (or escape) from inside the pinned frame — one stray `\n` scrolls
/// the terminal and smears a copy of the composer into the scrollback.
/// Byte lengths are preserved, so cursor offsets stay valid.
fn sanitize_keep_newlines(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || !c.is_control() { c } else { ' ' })
        .collect()
}

fn sanitize_row(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
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

    let layout = input_layout(state, width);
    for (i, body) in layout.rows.iter().enumerate() {
        let prefix = if i == 0 { prompt } else { "  " };
        let row = format!("│ {}{} │", prefix, pad_to(body, text_w));
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

fn prepare_output_ansi(bottom: usize, restore_saved_cursor: bool) -> String {
    if restore_saved_cursor {
        "\x1b[?25l\x1b[u".to_string()
    } else {
        format!("\x1b[?25l\x1b[{bottom};1H")
    }
}

/// Erase the bottom `height` rows — the pinned strip — without printing a
/// single newline.
fn clear_frame_ansi(rows: usize, height: usize) -> String {
    let clear_start = rows.saturating_sub(height) + 1;
    let mut ansi = String::from("\x1b[?25l");
    for row in clear_start..=rows {
        ansi.push_str(&format!("\x1b[{row};1H\x1b[K"));
    }
    ansi
}

/// DECSTBM for a strip of `height` rows pinned at the bottom of `rows`.
fn set_region_ansi(rows: usize, height: usize) -> String {
    format!("\x1b[1;{}r", region_bottom_row(rows, height))
}

fn region_bottom_row(rows: usize, height: usize) -> usize {
    rows.saturating_sub(height).max(1)
}

/// The exact bytes one composer frame writes: every row is addressed
/// absolutely (`CSI row;1H`) and erased in place, so a frame never contains a
/// newline and can never scroll a copy of itself into the scrollback.
pub fn paint_frame_ansi(state: &ComposerState, cols: usize, rows: usize) -> String {
    let cols = cols.max(24);
    let height = composer_rows(state, cols);
    let start = rows.saturating_sub(height) + 1;
    let mut out = String::from("\x1b[?25l");
    for (i, line) in render_ansi(state, cols).iter().enumerate() {
        out.push_str(&format!("\x1b[{};1H\x1b[K{}", start + i, line));
    }
    // Park the cursor inside the field so typing never lands in the stream.
    let layout = input_layout(state, cols);
    let prompt_w = measure_text_width(prompt_glyph());
    let row = start + 2 /* status + top border */ + layout.cursor_row;
    let col = 3 + prompt_w + layout.cursor_col;
    out.push_str(&format!("\x1b[{};{}H\x1b[?25h", row, col.min(cols)));
    out
}

fn busy_second_changed(busy: bool, last: Option<u64>, current: u64) -> bool {
    busy && last != Some(current)
}

/// ANSI frame for the TTY painter. Same layout as [`render_plain`], colored
/// and with a filled input field so it reads as a real text box, not a prompt
/// mixed into the stream.
pub fn render_ansi(state: &ComposerState, width: usize) -> Vec<String> {
    let width = width.max(24);
    let inner = width.saturating_sub(2);
    let (fg, bg, dim) = if state.light {
        (FG_LIGHT, BG_LIGHT, DIM)
    } else {
        (FG_DARK, BG_DARK, DIM)
    };
    let accent = if state.busy || !state.queued.is_empty() {
        MAGENTA
    } else {
        CYAN
    };
    let reset = "\x1b[0m";
    let mut lines = Vec::new();

    let status_color = if state.busy || !state.queued.is_empty() {
        MAGENTA
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
    let layout = input_layout(state, width);
    let fill = format!("{}{}", sgr_bg(bg), sgr_fg(fg));
    for (i, body) in layout.rows.iter().enumerate() {
        let is_placeholder = state.buffer.is_empty();
        let prefix = if i == 0 { prompt } else { "  " };
        let body_s = pad_to(body, text_w);
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
                match paste_marker(&bytes[i..]) {
                    Some(Ok((ev, len))) => {
                        events.push(ev);
                        i += len;
                        continue;
                    }
                    // A proper prefix of a marker: wait for the rest instead of
                    // decoding `ESC [ 2 0 0 ~` as Escape plus five characters.
                    Some(Err(())) => break,
                    None => {}
                }
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

pub const PASTE_START: &[u8] = b"\x1b[200~";
pub const PASTE_END: &[u8] = b"\x1b[201~";

/// `Some(Ok((event, consumed)))` for a complete bracketed-paste marker,
/// `Some(Err(()))` when `bytes` is only a prefix of one (a 64-byte read can
/// split `ESC[200~` anywhere), `None` when it is not a marker at all.
#[allow(clippy::result_unit_err)]
fn paste_marker(bytes: &[u8]) -> Option<Result<(KeyEvent, usize), ()>> {
    for (seq, ev) in [
        (PASTE_START, KeyEvent::PasteStart),
        (PASTE_END, KeyEvent::PasteEnd),
    ] {
        if bytes.starts_with(seq) {
            return Some(Ok((ev, seq.len())));
        }
        if bytes.len() < seq.len() && seq.starts_with(bytes) {
            return Some(Err(()));
        }
    }
    None
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
        last_rows: usize,
        last_cols: usize,
        last_busy_second: Option<u64>,
        output_prepared: bool,
        has_output_cursor: bool,
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
                last_rows: 0,
                last_cols: 0,
                last_busy_second: None,
                output_prepared: false,
                has_output_cursor: false,
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
            region_bottom_row(self.rows, self.last_height)
        }

        fn install_region(&mut self) {
            self.refresh_size();
            let height = composer_rows(&self.state, self.cols);
            if !self.installed {
                // Push the cursor up so the reserved strip is empty.
                for _ in 0..height {
                    print!("\r\n");
                }
                self.installed = true;
            }
            self.last_height = height;
            self.last_rows = self.rows;
            self.last_cols = self.cols;
            let bottom = self.region_bottom();
            // DECSTBM homes the cursor, so re-park it explicitly afterwards.
            print!("{}", set_region_ansi(self.rows, height));
            print!("\x1b[{bottom};1H");
            // Tell the terminal to bracket pastes. Without this a pasted
            // directive arrives as bare CR-separated lines — one submit per
            // line — and the desktop app's follow-up delivery, which waits for
            // mode 2004 before writing, never fires at all.
            print!("{ENABLE_BRACKETED_PASTE}");
            let _ = io::stdout().flush();
        }

        fn release_region(&mut self) {
            // Order matters: reset DECSTBM first (it homes the cursor), then
            // erase the pinned strip and park under the last output line. If
            // the strip were still drawn when the full-screen region takes
            // over, the next scroll would push a copy of it into the
            // scrollback — one leaked composer per suspend.
            let start = self.rows.saturating_sub(self.last_height) + 1;
            print!("{DISABLE_BRACKETED_PASTE}\x1b[r");
            print!("\x1b[{start};1H\x1b[J\x1b[?25h");
            self.output_prepared = false;
            self.has_output_cursor = false;
            self.installed = false;
            let _ = io::stdout().flush();
        }

        /// Re-assert the scroll region whenever the geometry it was derived
        /// from moved. Watching only `height` (the first cut) leaves a stale
        /// DECSTBM after a terminal-pane resize: the pinned strip then sits
        /// *inside* the scrolling region and every streamed chunk scrolls
        /// another copy of it into the scrollback.
        fn resync_region(&mut self) -> String {
            self.refresh_size();
            let height = composer_rows(&self.state, self.cols);
            if height == self.last_height
                && self.rows == self.last_rows
                && self.cols == self.last_cols
            {
                return String::new();
            }
            // Erase from wherever the old frame started down to the bottom:
            // after a resize the old and new strips are different rows.
            let new_start = self.rows.saturating_sub(height) + 1;
            let old_start = if self.last_rows == 0 {
                new_start
            } else {
                self.last_rows.saturating_sub(self.last_height) + 1
            };
            let clear_from = old_start.min(new_start).max(1);
            let mut ansi = clear_frame_ansi(self.rows, self.rows.saturating_sub(clear_from) + 1);
            ansi.push_str(&set_region_ansi(self.rows, height));
            self.has_output_cursor = false;
            self.last_height = height;
            self.last_rows = self.rows;
            self.last_cols = self.cols;
            ansi
        }

        pub fn paint(&mut self) {
            let mut out = self.resync_region();
            if self.output_prepared {
                // Preserve partial-line streaming position while the edit cursor
                // temporarily owns the terminal between output events.
                out.push_str("\x1b[s");
                self.has_output_cursor = true;
                self.output_prepared = false;
            }
            out.push_str(&paint_frame_ansi(&self.state, self.cols, self.rows));
            print!("{out}");
            let _ = io::stdout().flush();
            self.last_busy_second = self.state.busy.then(|| self.state.elapsed_secs());
        }

        /// Transfer terminal ownership to normal output. Call [`paint`] after
        /// the write to restore the pinned composer and its edit cursor.
        pub fn prepare_output(&mut self) {
            let mut out = self.resync_region();
            let bottom = self.region_bottom();
            out.push_str(&prepare_output_ansi(bottom, self.has_output_cursor));
            print!("{out}");
            self.output_prepared = true;
            let _ = io::stdout().flush();
        }

        pub fn set_busy(&mut self, busy: bool) {
            self.state.set_busy(busy);
            self.paint();
        }

        pub fn set_phase(&mut self, phase: TurnPhase) {
            if self.state.phase == phase {
                return;
            }
            self.state.set_phase(phase);
            self.paint();
        }

        pub fn set_turn_tokens(&mut self, tokens: Option<u64>) {
            self.state.turn_tokens = tokens;
        }

        /// Something was printed — restart the silence timer. Deliberately does
        /// NOT repaint: the caller is mid-output and owns the terminal.
        pub fn mark_output(&mut self) {
            self.state.mark_output();
        }

        /// Pull the current provider wait into the state. Called from the
        /// once-a-second tick rather than pushed from the retry layer, because
        /// the retry layer lives in another crate and has no business knowing a
        /// TUI exists.
        fn refresh_provider_wait(&mut self) -> bool {
            let line = permagent::providers::wait_status::current().map(|w| w.status_line());
            let changed = line != self.state.provider_wait;
            self.state.provider_wait = line;
            changed
        }

        pub fn suspend(&mut self) {
            self.paused.store(true, Ordering::Relaxed);
            self.release_region();
            if let Some(raw) = self.raw.as_mut() {
                raw.restore();
            }
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
            let mut changed = false;
            while let Ok(bytes) = self.key_rx.try_recv() {
                let events = decode_keys(&bytes, &mut self.parse_buf);
                for ev in events {
                    changed = true;
                    let action = self.state.apply(ev);
                    if !matches!(action, ComposerAction::Redraw) && first.is_none() {
                        first = Some(action);
                    }
                }
            }
            // Repaint on an edit, and also once a second while busy so the
            // elapsed counter ticks even when nothing was typed.
            let wait_changed = self.refresh_provider_wait();
            if changed
                || wait_changed
                || busy_second_changed(
                    self.state.busy,
                    self.last_busy_second,
                    self.state.elapsed_secs(),
                )
            {
                self.paint();
            }
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
                        let wait_changed = self.refresh_provider_wait();
                        if wait_changed
                            || busy_second_changed(
                                self.state.busy,
                                self.last_busy_second,
                                self.state.elapsed_secs(),
                            )
                        {
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
            self.release_region();
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
    pub fn prepare_output(&mut self) {}
    pub fn set_busy(&mut self, busy: bool) {
        self.state.set_busy(busy);
    }
    pub fn set_phase(&mut self, phase: TurnPhase) {
        self.state.set_phase(phase);
    }
    pub fn set_turn_tokens(&mut self, tokens: Option<u64>) {
        self.state.turn_tokens = tokens;
    }
    pub fn mark_output(&mut self) {
        self.state.mark_output();
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
        let mut state = idle();
        assert_eq!(composer_rows(&state, 24), 5);
        state.buffer = "a\nb".into();
        state.cursor = state.buffer.len();
        assert_eq!(composer_rows(&state, 24), 6);
        let many = (0..20).map(|_| "x").collect::<Vec<_>>().join("\n");
        state.buffer = many;
        state.cursor = state.buffer.len();
        assert_eq!(composer_rows(&state, 24), 4 + MAX_INPUT_ROWS);
    }

    #[test]
    fn narrow_layout_keeps_long_unicode_tail_and_cursor_visible() {
        for width in [24, 40, 80, 120] {
            let mut state = idle();
            state.buffer = "beginning-🙂-東京-abcdefghijklmnopqrstuvwxyz-END".into();
            state.cursor = state.buffer.len();
            let layout = input_layout(&state, width);
            assert!(layout.rows[0].ends_with("END"), "width {width}: {layout:?}");
            assert!(layout.cursor_col <= width, "width {width}: {layout:?}");
            let frame = render_plain(&state, width);
            assert_eq!(frame.len(), composer_rows(&state, width));
            assert!(
                frame.iter().all(|line| measure_text_width(line) <= width),
                "width {width}: {frame:?}"
            );
        }
    }

    #[test]
    fn multiline_viewport_follows_cursor_beyond_six_lines() {
        let mut state = idle();
        state.buffer = (0..9)
            .map(|line| format!("line-{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        state.cursor = state.buffer.find("line-7").unwrap() + "line-7".len();
        let layout = input_layout(&state, 32);
        assert_eq!(layout.rows.len(), MAX_INPUT_ROWS);
        assert!(layout.rows.iter().any(|row| row == "line-7"), "{layout:?}");
        assert!(layout.cursor_row < MAX_INPUT_ROWS);
    }

    #[test]
    fn ansi_palette_and_output_handoff_match_brand_contract() {
        let state = idle();
        let idle_ansi = render_ansi(&state, 24).join("");
        assert!(idle_ansi.contains("\x1b[38;2;0;213;255m"));

        let mut busy = state;
        busy.set_busy(true);
        let busy_ansi = render_ansi(&busy, 24).join("");
        assert!(busy_ansi.contains("\x1b[38;2;168;85;204m"));
        assert!(!busy_ansi.contains("\x1b[38;2;196;136;58m"));
        assert_eq!(prepare_output_ansi(17, false), "\x1b[?25l\x1b[17;1H");
        assert_eq!(prepare_output_ansi(17, true), "\x1b[?25l\x1b[u");

        let shrink = clear_frame_ansi(24, 10);
        assert!(shrink.contains("\x1b[15;1H\x1b[K"));
        assert!(shrink.contains("\x1b[24;1H\x1b[K"));
    }

    #[test]
    fn busy_timer_repaints_only_when_the_displayed_second_changes() {
        assert!(!busy_second_changed(true, Some(4), 4));
        assert!(busy_second_changed(true, Some(4), 5));
        assert!(!busy_second_changed(false, Some(4), 5));
    }

    // ── bracketed paste ────────────────────────────────────────────────────

    fn paste_bytes(body: &str, trailing_cr: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PASTE_START);
        bytes.extend_from_slice(body.as_bytes());
        bytes.extend_from_slice(PASTE_END);
        if trailing_cr {
            bytes.push(b'\r');
        }
        bytes
    }

    fn feed(state: &mut ComposerState, pending: &mut Vec<u8>, bytes: &[u8]) -> Vec<ComposerAction> {
        decode_keys(bytes, pending)
            .into_iter()
            .map(|ev| state.apply(ev))
            .collect()
    }

    fn non_redraw(actions: &[ComposerAction]) -> Vec<ComposerAction> {
        actions
            .iter()
            .filter(|a| !matches!(a, ComposerAction::Redraw))
            .cloned()
            .collect()
    }

    fn thirty_lines(sep: &str) -> String {
        (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join(sep)
    }

    #[test]
    fn bracketed_paste_of_thirty_lines_is_one_message() {
        // A terminal replays pasted newlines as CR; that is exactly what used
        // to submit once per line.
        for sep in ["\r", "\n", "\r\n"] {
            let mut state = idle();
            let mut pending = Vec::new();
            let actions = feed(
                &mut state,
                &mut pending,
                &paste_bytes(&thirty_lines(sep), true),
            );
            let submitted = non_redraw(&actions);
            assert_eq!(submitted.len(), 1, "sep {sep:?}: {submitted:?}");
            let ComposerAction::Submit(text) = &submitted[0] else {
                panic!("sep {sep:?}: expected a single Submit, got {submitted:?}");
            };
            assert_eq!(text.lines().count(), 30, "sep {sep:?}: {text:?}");
            assert!(text.starts_with("line 0"), "{text:?}");
            assert!(text.ends_with("line 29"), "{text:?}");
            assert!(state.buffer.is_empty());
            assert!(!state.pasting);
        }
    }

    #[test]
    fn paste_split_across_reads_still_lands_as_one_message() {
        // A 64-byte read can cut `ESC[200~` in half.
        let bytes = paste_bytes(&thirty_lines("\r"), true);
        let mut state = idle();
        let mut pending = Vec::new();
        let mut actions = Vec::new();
        for chunk in bytes.chunks(7) {
            actions.extend(feed(&mut state, &mut pending, chunk));
        }
        let submitted = non_redraw(&actions);
        assert_eq!(submitted.len(), 1, "{submitted:?}");
        let ComposerAction::Submit(text) = &submitted[0] else {
            panic!("{submitted:?}");
        };
        assert_eq!(text.lines().count(), 30);
        assert!(
            pending.is_empty(),
            "no partial marker left over: {pending:?}"
        );
    }

    #[test]
    fn bare_enter_still_submits_and_paste_then_enter_submits_once() {
        let mut state = idle();
        let mut pending = Vec::new();
        let typed = non_redraw(&feed(&mut state, &mut pending, b"hi\r"));
        assert_eq!(typed, vec![ComposerAction::Submit("hi".into())]);

        // Paste with no trailing CR: it only fills the field.
        let pasted = non_redraw(&feed(
            &mut state,
            &mut pending,
            &paste_bytes("one\rtwo", false),
        ));
        assert!(
            pasted.is_empty(),
            "a paste alone must not submit: {pasted:?}"
        );
        assert_eq!(state.buffer, "one\ntwo");
        let enter = non_redraw(&feed(&mut state, &mut pending, b"\r"));
        assert_eq!(enter, vec![ComposerAction::Submit("one\ntwo".into())]);
    }

    #[test]
    fn escape_inside_a_paste_does_not_interrupt_the_turn() {
        let mut state = idle();
        state.set_busy(true);
        let mut pending = Vec::new();
        // ESC bytes inside pasted text must be swallowed, not treated as
        // "interrupt the agent".
        let actions = non_redraw(&feed(
            &mut state,
            &mut pending,
            &paste_bytes("a\x1bb\x03c", false),
        ));
        assert!(actions.is_empty(), "{actions:?}");
        assert_eq!(state.buffer, "abc");
    }

    #[test]
    fn decode_recognises_paste_markers_and_waits_on_a_partial_one() {
        let mut pending = Vec::new();
        assert_eq!(
            decode_keys(PASTE_START, &mut pending),
            vec![KeyEvent::PasteStart]
        );
        assert_eq!(
            decode_keys(PASTE_END, &mut pending),
            vec![KeyEvent::PasteEnd]
        );
        assert!(decode_keys(b"\x1b[20", &mut pending).is_empty());
        assert_eq!(pending, b"\x1b[20".to_vec());
        assert_eq!(decode_keys(b"0~", &mut pending), vec![KeyEvent::PasteStart]);
        assert!(pending.is_empty());
        // Not a paste marker: the old unknown-CSI path is untouched.
        assert_eq!(decode_keys(b"\x1b[2~", &mut pending)[0], KeyEvent::Escape);
    }

    #[test]
    fn a_pasted_directive_queues_as_one_item_with_a_one_line_row() {
        let mut state = idle();
        state.set_busy(true);
        let mut pending = Vec::new();
        let queued = non_redraw(&feed(
            &mut state,
            &mut pending,
            &paste_bytes(&thirty_lines("\r"), true),
        ));
        assert_eq!(queued.len(), 1, "{queued:?}");
        assert!(matches!(queued[0], ComposerAction::Queue(_)));
        assert_eq!(state.queued.len(), 1);
        let row = status_line(&state);
        assert!(row.contains("Queued 1"), "{row}");
        assert!(row.contains("line 0"), "{row}");
        assert!(row.contains("(+29 lines)"), "{row}");
        assert!(
            !row.contains('\n'),
            "the queue row must stay one line: {row:?}"
        );
    }

    // ── turn phase / stuck hint (2026-08-25) ───────────────────────────────

    fn busy_state() -> ComposerState {
        let mut state = idle();
        state.set_busy(true);
        state
    }

    /// The GLM case: four minutes of reasoning with nothing printed. The line
    /// must say what it is doing, not just that it is doing something.
    #[test]
    fn a_thinking_turn_says_thinking() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        let line = status_line(&state);
        assert!(line.contains("Thinking"), "{line}");
        assert!(line.contains("esc to interrupt"), "{line}");
    }

    /// A collapsed tool line still has to name the tool AND its target.
    #[test]
    fn a_running_tool_names_its_target() {
        let mut state = busy_state();
        state.phase = TurnPhase::RunningTool {
            name: "read".to_string(),
            target: Some("crates/goose/src/providers/zai.rs".to_string()),
        };
        let line = status_line(&state);
        assert!(line.contains("read"), "{line}");
        assert!(line.contains("zai.rs"), "{line}");
    }

    #[test]
    fn a_tool_with_no_target_still_renders() {
        let mut state = busy_state();
        state.phase = TurnPhase::RunningTool {
            name: "verify".to_string(),
            target: None,
        };
        assert!(status_line(&state).contains("verify"));
    }

    /// A target carrying control bytes must not escape the pinned frame.
    #[test]
    fn a_tool_target_cannot_break_the_frame() {
        let mut state = busy_state();
        state.phase = TurnPhase::RunningTool {
            name: "shell".to_string(),
            target: Some("a\nb\x1b[2Jc".to_string()),
        };
        let line = status_line(&state);
        assert!(!line.contains('\n'), "{line:?}");
        assert!(!line.contains('\x1b'), "{line:?}");
    }

    #[test]
    fn tokens_this_turn_are_shown_when_known() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.turn_tokens = Some(840);
        assert!(status_line(&state).contains("840 tok"));
        state.turn_tokens = Some(12_400);
        assert!(status_line(&state).contains("12.4k tok"));
    }

    #[test]
    fn tokens_are_omitted_rather_than_guessed_at_zero() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.turn_tokens = None;
        assert!(
            !status_line(&state).contains("tok"),
            "{}",
            status_line(&state)
        );
    }

    /// Silence past the threshold is named, with the way out.
    #[test]
    fn a_long_silence_is_named_and_offers_the_way_out() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.last_output_at = Some(Instant::now() - Duration::from_secs(62));
        let line = status_line(&state);
        assert!(line.contains("no output for 62s"), "{line}");
        assert!(line.contains("Ctrl+C to interrupt"), "{line}");
    }

    /// A turn that has printed NOTHING from its first instant is exactly the
    /// case that needs the hint, so it must fall back to the turn start.
    #[test]
    fn a_turn_that_never_printed_still_gets_the_hint() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.busy_since = Some(Instant::now() - Duration::from_secs(90));
        state.last_output_at = None;
        assert!(status_line(&state).contains("no output for 90s"));
    }

    #[test]
    fn a_short_silence_says_nothing_about_being_stuck() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.last_output_at = Some(Instant::now() - Duration::from_secs(5));
        assert!(!status_line(&state).contains("no output"));
    }

    /// A provider wait outranks the phase: it is the one case where the agent
    /// is doing nothing at all, and it comes with a number that counts down.
    #[test]
    fn a_provider_wait_replaces_the_phase() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.last_output_at = Some(Instant::now() - Duration::from_secs(120));
        state.provider_wait =
            Some("Z.AI rate limit — retrying in 12 s (attempt 2 of 4)".to_string());
        let line = status_line(&state);
        assert!(line.contains("Z.AI rate limit"), "{line}");
        assert!(line.contains("retrying in 12 s"), "{line}");
        assert!(!line.contains("Thinking"), "{line}");
        // even a long silence does not override it — we KNOW why it is quiet
        assert!(!line.contains("no output"), "{line}");
    }

    /// Queued input still outranks everything, unchanged.
    #[test]
    fn a_queued_message_still_wins_over_the_phase() {
        let mut state = busy_state();
        state.phase = TurnPhase::Thinking;
        state.queued.push("next thing".to_string());
        assert!(status_line(&state).contains("Queued 1"));
    }

    /// The whole point of one line: the pinned strip's height must not move.
    #[test]
    fn the_richer_status_does_not_add_a_row() {
        let plain = busy_state();
        let mut rich = busy_state();
        rich.phase = TurnPhase::RunningTool {
            name: "shell".to_string(),
            target: Some("a".repeat(300)),
        };
        rich.turn_tokens = Some(999_999);
        rich.last_output_at = Some(Instant::now() - Duration::from_secs(300));
        for width in [40usize, 80, 200] {
            assert_eq!(
                composer_rows(&plain, width),
                composer_rows(&rich, width),
                "width {width}"
            );
            let frame = render_plain(&rich, width);
            assert_eq!(
                frame[0].lines().count().max(1),
                1,
                "status must stay one row at width {width}"
            );
        }
    }

    // ── pinned region ──────────────────────────────────────────────────────

    #[test]
    fn a_frame_never_contains_a_newline() {
        let mut state = idle();
        state.buffer = "pasted\nlines\there".into();
        state.cursor = state.buffer.len();
        let frame = paint_frame_ansi(&state, 80, 30);
        assert!(!frame.contains('\n'), "frame must address rows absolutely");
        assert!(!frame.contains('\r'));
    }

    #[test]
    fn pinned_block_is_drawn_once_per_frame_and_never_in_the_scrollback() {
        let (cols, rows) = (80usize, 30usize);
        let mut state = idle();
        state.model = "MiniMax-M2.7".into();
        state.cwd = "~/Documents/dev/atlasatlantic".into();
        state.set_busy(true);
        let chunks = [
            "search.7\n",
            "Found 653 matches across 41 files\n",
            "…still working\n",
            "done\n",
        ];
        let bottom = region_bottom_row(rows, composer_rows(&state, cols));

        let mut log = String::new();
        let mut scrollback = String::new();
        for chunk in chunks {
            log.push_str(&prepare_output_ansi(bottom, false));
            // Everything the agent writes between the handoff and the repaint
            // is what ends up in the scrollback.
            scrollback.push_str(chunk);
            log.push_str(chunk);
            let frame = paint_frame_ansi(&state, cols, rows);
            assert_eq!(frame.matches('╭').count(), 1, "one input box per frame");
            assert_eq!(frame.matches('╰').count(), 1);
            assert_eq!(
                frame.matches("• Working").count(),
                1,
                "one status line per frame"
            );
            log.push_str(&frame);
        }

        assert_eq!(log.matches('╭').count(), chunks.len());
        assert_eq!(log.matches(&state.cwd).count(), chunks.len());
        for marker in ['╭', '╰', '│'] {
            assert!(
                !scrollback.contains(marker),
                "pinned bytes leaked into the scrollback segment: {scrollback:?}"
            );
        }
        assert!(!scrollback.contains("• Working"));
        assert!(!scrollback.contains(&state.cwd));
    }

    #[test]
    fn resize_reasserts_the_scroll_region_not_just_a_height_change() {
        // The strip must always sit BELOW the scrolling region; a pane that
        // changed height without changing the composer's height used to keep
        // the stale DECSTBM and scroll the composer into the scrollback.
        for (rows, height) in [(30usize, 5usize), (24, 5), (12, 5), (6, 5)] {
            let bottom = region_bottom_row(rows, height);
            let start = rows.saturating_sub(height) + 1;
            assert_eq!(set_region_ansi(rows, height), format!("\x1b[1;{bottom}r"));
            assert!(
                bottom < start || rows <= height,
                "rows {rows}: {bottom}/{start}"
            );
        }
        assert!(clear_frame_ansi(30, 5).contains("\x1b[26;1H\x1b[K"));
        assert!(clear_frame_ansi(30, 5).contains("\x1b[30;1H\x1b[K"));
        assert!(!clear_frame_ansi(30, 5).contains('\n'));
    }

    #[test]
    fn pasted_control_bytes_cannot_break_the_pinned_frame() {
        let mut state = idle();
        state.buffer = "col\tone\u{7}two".into();
        state.cursor = state.buffer.len();
        let frame = render_plain(&state, 60);
        assert_eq!(frame.len(), composer_rows(&state, 60));
        assert!(frame
            .iter()
            .all(|l| !l.contains('\t') && !l.contains('\u{7}')));
        assert!(frame.iter().all(|l| measure_text_width(l) <= 60));
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

/// Real-PTY verification for the two defects this module fixes. Ignored by
/// default because it needs `--nocapture` (libtest otherwise diverts `print!`
/// away from the pty) and a serial run:
///
/// ```text
/// cargo test -p permagent-cli pty_ -- --ignored --nocapture --test-threads=1
/// ```
#[cfg(all(test, unix))]
mod pty_tests {
    use super::*;
    use std::os::unix::io::RawFd;

    struct Pty {
        master: RawFd,
        slave: RawFd,
        saved_in: RawFd,
        saved_out: RawFd,
    }

    impl Pty {
        fn open(rows: u16, cols: u16) -> Self {
            let mut master = 0;
            let mut slave = 0;
            let mut ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            // `winp` is `*mut winsize` on macOS and `*const winsize` on Linux;
            // a typed raw pointer satisfies both without passing `&mut` where
            // one platform only wants a const pointer.
            let ws_ptr: *mut libc::winsize = &mut ws;
            assert_eq!(
                unsafe {
                    libc::openpty(
                        &mut master,
                        &mut slave,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        ws_ptr,
                    )
                },
                0,
                "openpty"
            );
            unsafe {
                let flags = libc::fcntl(master, libc::F_GETFL);
                libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            let saved_in = unsafe { libc::dup(0) };
            let saved_out = unsafe { libc::dup(1) };
            unsafe {
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
            }
            Self {
                master,
                slave,
                saved_in,
                saved_out,
            }
        }

        fn resize(&self, rows: u16, cols: u16) {
            let ws = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let ws_ptr: *const libc::winsize = &ws;
            unsafe { libc::ioctl(self.master, libc::TIOCSWINSZ, ws_ptr) };
        }

        fn write(&self, bytes: &[u8]) {
            let mut sent = 0;
            while sent < bytes.len() {
                let n = unsafe {
                    libc::write(
                        self.master,
                        bytes[sent..].as_ptr() as *const libc::c_void,
                        bytes.len() - sent,
                    )
                };
                if n > 0 {
                    sent += n as usize;
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }

        fn drain(&self) -> String {
            let mut out = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = unsafe {
                    libc::read(
                        self.master,
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if n <= 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n as usize]);
            }
            String::from_utf8_lossy(&out).into_owned()
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            unsafe {
                libc::dup2(self.saved_in, 0);
                libc::dup2(self.saved_out, 1);
                libc::close(self.saved_in);
                libc::close(self.saved_out);
                libc::close(self.slave);
                libc::close(self.master);
            }
        }
    }

    fn settle() {
        std::thread::sleep(std::time::Duration::from_millis(120));
    }

    #[test]
    #[ignore = "needs --nocapture and a real pty; run serially"]
    fn pty_paste_is_one_message_and_the_frame_never_scrolls() {
        let pty = Pty::open(24, 80);
        let mut composer = Composer::try_install().expect("composer on a pty");
        settle();

        let startup = pty.drain();
        assert!(
            startup.contains(ENABLE_BRACKETED_PASTE),
            "startup must turn on bracketed paste (the app's readiness gate): {startup:?}"
        );
        assert!(startup.contains("\x1b[1;19r"), "scroll region: {startup:?}");

        // 20-line bracketed paste, exactly as the terminal delivers one.
        let body = (1..=20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\r");
        let mut bytes = Vec::from(PASTE_START);
        bytes.extend_from_slice(body.as_bytes());
        bytes.extend_from_slice(PASTE_END);
        bytes.push(b'\r');
        pty.write(&bytes);
        settle();

        let mut action = None;
        for _ in 0..40 {
            if let Some(a) = composer.drain_keys() {
                action = Some(a);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let typed = pty.drain();
        match action {
            Some(ComposerAction::Submit(text)) => {
                assert_eq!(text.lines().count(), 20, "one message, 20 lines: {text:?}");
            }
            other => panic!("expected a single Submit, got {other:?}"),
        }
        assert!(
            !typed.contains('\n'),
            "the composer must not emit a newline while echoing a paste"
        );

        // Stream four chunks past the pinned strip.
        composer.set_busy(true);
        let _ = pty.drain();
        let chunks = [
            "search.7",
            "Found 653 matches across 41 files",
            "…still working",
            "done",
        ];
        for chunk in chunks {
            composer.prepare_output();
            println!("{chunk}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            composer.paint();
        }
        let streamed = pty.drain();
        assert_eq!(
            streamed.matches('╭').count(),
            chunks.len(),
            "one input box per frame, not per line of output"
        );
        assert_eq!(streamed.matches("• Working").count(), chunks.len());

        // A pane resize must re-assert the region, or the strip ends up inside
        // the scrolling area and every chunk scrolls a copy into the scrollback.
        pty.resize(16, 80);
        composer.paint();
        let resized = pty.drain();
        assert!(
            resized.contains("\x1b[1;11r"),
            "resize must re-assert DECSTBM: {resized:?}"
        );

        drop(composer);
        let teardown = pty.drain();
        assert!(
            teardown.contains(DISABLE_BRACKETED_PASTE) && teardown.contains("\x1b[r"),
            "teardown must release paste mode and the region: {teardown:?}"
        );
    }
}
