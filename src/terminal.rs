use crate::pty::PtySession;
use anyhow::Result;
use crossbeam_channel::Receiver;
use std::cmp::max;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use vte::Params;
use vte::Parser;
use vte::Perform;

#[derive(Clone)]
pub struct Terminal {
    terminal: Arc<Mutex<TerminalInner>>,
}

impl Terminal {
    pub fn new(pty: PtySession) -> Self {
        let reader = pty.get_reader();
        let inner = Arc::new(Mutex::new(TerminalInner::new(pty)));
        let terminal = Terminal { terminal: inner };
        terminal.start_feeding(reader);
        terminal
    }

    pub fn as_text(&self) -> String {
        self.terminal
            .lock()
            .expect("Failed to lock terminal")
            .as_text()
    }

    pub fn write(&self, data: &[u8]) {
        self.terminal
            .lock()
            .expect("Failed to lock terminal")
            .write(data);
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let mut terminal = self.terminal.lock().expect("Failed to lock terminal");
        tracing::info!("Resizing terminal to {} cols and {} rows", cols, rows);
        terminal.size = Some(Size { cols, rows });
        terminal.pty.resize(cols, rows)
    }

    pub fn cursor(&self) -> (usize, usize) {
        let terminal = self.terminal.lock().expect("Failed to lock terminal");
        (terminal.cursor_x, terminal.cursor_y)
    }

    pub fn is_dirty(&self) -> bool {
        self.terminal.lock().expect("Failed to lock terminal").is_dirty()
    }

    pub fn clear_dirty(&self) {
        self.terminal.lock().expect("Failed to lock terminal").clear_dirty();
    }

    fn start_feeding(&self, reader: Receiver<String>) {
        let terminal = self.terminal.clone();
        std::thread::spawn(move || {
            for output in reader.iter() {
                tracing::info!("PTY RAW: {:?}", output);
                if output.is_empty() {
                    continue; // Skip empty outputs
                }
                terminal
                    .lock()
                    .expect("Failed to lock terminal")
                    .feed_bytes(output.as_bytes());
            }
        });
    }
}

pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

struct TerminalInner {
    pub lines: VecDeque<String>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pty: PtySession,
    parser: Parser,
    size: Option<Size>,
    dirty: bool,
}

impl TerminalInner {
    const MAX_LINES: usize = 1000;

    pub fn new(pty: PtySession) -> Self {
        Self {
            lines: VecDeque::with_capacity(Self::MAX_LINES),
            cursor_x: 0,
            cursor_y: 0,
            pty,
            parser: Parser::new(),
            size: None,
            dirty: false,
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        parser.advance(self, bytes);
        self.parser = parser;
    }

    pub fn as_text(&self) -> String {
        return self
            .lines
            .iter()
            .skip(
                self.lines
                    .len()
                    .saturating_sub(self.size.as_ref().map_or(0, |s| s.rows as usize)),
            )
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    fn write(&mut self, data: &[u8]) {
        if data.is_empty() {
            return; // Skip empty writes
        }
        let command = match data {
            b"\x08" // Replace backspace key with DEL
             => b"\x7f", // DEL
            _ => data
        };
        self.pty
            .get_writer()
            .send(command.to_vec())
            .expect("Failed to write to PTY");
    }

    fn move_cursor(&mut self, x: usize, y: usize) {
        tracing::debug!(
            "Moving cursor from ({}, {}) to ({}, {})",
            self.cursor_x,
            self.cursor_y,
            x,
            y
        );
        self.cursor_x = x;
        self.cursor_y = y;

        // Ensure cursor position is within bounds
        if self.cursor_y >= self.lines.len() {
            self.cursor_y = self.lines.len().saturating_sub(1);
        }
        if self.cursor_x > self.lines[self.cursor_y].chars().count() {
            self.cursor_x = self.lines[self.cursor_y].chars().count();
        }
        tracing::debug!("Cursor moved to ({}, {})", self.cursor_x, self.cursor_y);
    }
}

impl Perform for TerminalInner {
    fn print(&mut self, c: char) {
        // If the cursor position exceeds the current line, extend the lines vector
        if self.cursor_y >= self.lines.len() {
            self.lines.resize(self.cursor_y + 1, String::new());
        }

        let line = &mut self.lines[self.cursor_y];

        // Ensure the line is long enough to accommodate the cursor position
        let char_count = line.chars().count();
        if self.cursor_x > char_count {
            line.extend(std::iter::repeat(' ').take(self.cursor_x - char_count));
        }

        let updated_char_count = line.chars().count();
        if self.cursor_x == updated_char_count {
            line.push(c);
        } else {
            if let Some((start, _)) = line.char_indices().nth(self.cursor_x) {
                let end = line
                    .char_indices()
                    .nth(self.cursor_x + 1)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());

                line.replace_range(start..end, &c.to_string());
            } else {
                // if cursor_x is out of bounds, just append
                line.push(c);
            }
        }

        self.cursor_x += 1;
        self.dirty = true;
    }

    fn execute(&mut self, byte: u8) {
        // eprintln!("Execute byte: {:?}", byte);
        match byte {
            b'\n' => {
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= Self::MAX_LINES {
                    self.lines.pop_front();
                    self.cursor_y = Self::MAX_LINES - 1;
                }
            }
            b'\r' => {
                self.cursor_x = 0;
            }
            b'\x08' => {
                // Backspace
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        tracing::debug!(
            "OSC Dispatch: params={:?}, bell_terminated={}",
            _params,
            _bell_terminated
        );
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
        // eprintln!(
        //     "CSI Dispatch: params={:?}, intermediates={:?}, ignore={}, c='{}'",
        //     params, intermediates, ignore, c
        // );
        let params: Vec<&[u16]> = params.iter().collect();
        // Handle some common CSI sequences
        match c {
            'H' | 'f' => {
                // Cursor Position
                let row = params.get(0).and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let col = params.get(1).and_then(|p| p.first()).copied().unwrap_or(1) as usize;

                self.cursor_y = row.saturating_sub(1);
                self.cursor_x = col.saturating_sub(1);
            }
            'J' => {
                // Erase in Display
                let param = params.get(0).and_then(|p| p.first()).copied().unwrap_or(0);
                // Erase from cursor to end of screen
                if param == 0 {
                    tracing::debug!("Erasing from cursor to end of screen");
                    for line in self.lines.iter_mut().skip(self.cursor_y) {
                        line.clear();
                    }
                    self.lines.truncate(self.cursor_y + 1);
                    if let Some(line) = self.lines.get_mut(self.cursor_y) {
                        *line = line.chars().take(self.cursor_x).collect();
                    }
                }
                // Erase from start of screen to cursor
                else if param == 1 {
                    tracing::debug!("Erasing from start of screen to cursor");
                    for line in self.lines.iter_mut().take(self.cursor_y) {
                        line.clear();
                    }
                    if let Some(line) = self.lines.get_mut(self.cursor_y) {
                        *line = line.chars().skip(self.cursor_x).collect();
                    }
                }
                // Erase entire screen
                else if param == 2 || param == 3 {
                    tracing::debug!("Erasing entire screen");
                    self.lines.clear();
                    self.cursor_x = 0;
                    self.cursor_y = 0;
                }
            }
            // Erase in Line
            'K' => {
                let param = params.get(0).and_then(|p| p.first()).copied().unwrap_or(0);
                if param == 0 {
                    // Erase from cursor to end of line
                    tracing::debug!(
                        "Erasing from cursor to end of line. Cursor at ({}, {})",
                        self.cursor_x,
                        self.cursor_y
                    );
                    tracing::debug!(
                        "Current line before erase: {:?}",
                        self.lines.get(self.cursor_y)
                    );
                    tracing::debug!(
                        "Current line length: {}",
                        self.lines.get(self.cursor_y).map_or(0, |l| l.len())
                    );
                    if let Some(line) = self.lines.get_mut(self.cursor_y) {
                        // take only the first self.cursor_x characters
                        *line = line.chars().take(self.cursor_x).collect();
                    }
                } else if param == 1 {
                    // Erase from start of line to cursor
                    tracing::debug!("Erasing from start of line to cursor");
                    if let Some(line) = self.lines.get_mut(self.cursor_y) {
                        *line = line.chars().skip(self.cursor_x).collect();
                    }
                } else if param == 2 {
                    // Erase entire line
                    tracing::debug!("Erasing entire line");
                    if let Some(line) = self.lines.get_mut(self.cursor_y) {
                        line.clear();
                    }
                }
            }
            // Cursor request
            'n' => {
                tracing::debug!("Cursor position request received");
                // Respond with cursor position
                if params.is_empty() || params[0].is_empty() {
                    // Respond with current cursor position
                    let response = format!("\x1b[{};{}R", self.cursor_y + 1, self.cursor_x + 1);
                    self.write(response.as_bytes());
                } else {
                    // Respond with specific position
                    let row = params[0].get(0).copied().unwrap_or(1) as usize;
                    let col = params[0].get(1).copied().unwrap_or(1) as usize;
                    let response = format!("\x1b[{};{}R", row, col);
                    self.write(response.as_bytes());
                }
            }
            // Cursor Up
            'A' => {
                let count = max(
                    1,
                    params.get(0).and_then(|p| p.first()).copied().unwrap_or(1) as usize,
                );
                tracing::debug!("Cursor Up by {}, other params: {:?}", count, params);
                self.move_cursor(self.cursor_x, self.cursor_y.saturating_sub(count));
            }
            // Cursor Down
            'B' => {
                let count = max(
                    1,
                    params.get(0).and_then(|p| p.first()).copied().unwrap_or(1) as usize,
                );
                tracing::debug!("Cursor Down by {}, other params: {:?}", count, params);
                self.move_cursor(self.cursor_x, self.cursor_y.saturating_add(count));
            }
            // Cursor Right
            'C' => {
                let count = max(
                    1,
                    params.get(0).and_then(|p| p.first()).copied().unwrap_or(1) as usize,
                );
                tracing::debug!("Cursor Right by {}, other params: {:?}", count, params);
                self.move_cursor(self.cursor_x.saturating_add(count), self.cursor_y);
            }
            // Cursor Left
            'D' => {
                let count = max(
                    1,
                    params.get(0).and_then(|p| p.first()).copied().unwrap_or(1) as usize,
                );
                tracing::debug!("Cursor Left by {}, other params: {:?}", count, params);
                self.move_cursor(self.cursor_x.saturating_sub(count), self.cursor_y);
            }
            _ => {
                tracing::debug!("Unhandled CSI sequence: {} with params: {:?}", c, params);
                // Ignore other CSI sequences for now
            }
        }
        self.dirty = true;
    }
}
