use crate::pty::PtySession;
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
        let inner = Arc::new(Mutex::new(TerminalInner::new(pty)));
        let terminal = Terminal { terminal: inner };
        terminal.start_feeding();
        terminal
    }

    pub fn as_text(&self) -> String {
        self.terminal.lock().expect("Failed to lock terminal").as_text()
    }

    pub fn write(&self, data: &[u8]) {
        self.terminal.lock().expect("Failed to lock terminal").write(data);
    }

    fn start_feeding(&self) {
        let terminal = Arc::clone(&self.terminal);
        let reader = terminal.lock().expect("Failed to lock terminal").pty.get_reader();
        std::thread::spawn(move || {
            for output in reader.iter() {
                println!("PTY RAW: {:?}", output);
                if output.is_empty() {
                    continue; // Skip empty outputs
                }
                terminal
                    .lock()
                    .expect("Failed to lock terminal")
                    .feed_bytes(output.as_bytes());
                println!(
                    "TERMINAL TEXT (from receiver): {:?}",
                    terminal.lock().expect("failed to lock").as_text()
                );
            }
        });
    }
}

struct TerminalInner {
    pub lines: VecDeque<String>,
    pub cursor_x: usize,
    pub cursor_y: usize,
    pty: PtySession,
}

impl TerminalInner {
    const MAX_LINES: usize = 1000;

    pub fn new(pty: PtySession) -> Self {
        Self {
            lines: VecDeque::with_capacity(Self::MAX_LINES),
            cursor_x: 0,
            cursor_y: 0,
            pty,
        }
    }

    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        let mut parser = Parser::new();
        parser.advance(self, bytes);
    }

    pub fn as_text(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
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
        self.pty.get_writer().send(command.to_vec()).expect("Failed to write to PTY");
    }

}

impl Perform for TerminalInner {
    fn print(&mut self, c: char) {
        while self.lines.len() <= self.cursor_y {
            self.lines.push_back(String::new());
        }

        let line = self.lines.get_mut(self.cursor_y).unwrap();

        if self.cursor_x > line.chars().count() {
            line.extend(std::iter::repeat(' ').take(self.cursor_x - line.chars().count()));
        }

        if self.cursor_x == line.chars().count() {
            line.push(c);
        } else {
            let start = line
                .char_indices()
                .nth(self.cursor_x)
                .map(|(i, _)| i)
                .unwrap();
            let end = line
                .char_indices()
                .nth(self.cursor_x + 1)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            line.replace_range(start..end, &c.to_string());
        }

        self.cursor_x += 1;
    }

    fn execute(&mut self, byte: u8) {
        eprintln!("Execute byte: {:?}", byte);
        match byte {
            b'\n' => {
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= Self::MAX_LINES {
                    self.lines.pop_front();
                    self.cursor_y = Self::MAX_LINES - 1;
                }
                if self.cursor_y >= self.lines.len() {
                    self.lines.push_back(String::new());
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
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
        eprintln!(
            "CSI Dispatch: params={:?}, intermediates={:?}, ignore={}, c='{}'",
            params, intermediates, ignore, c
        );
        let params: Vec<&[u16]> = params.iter().collect();
        // Handle some common CSI sequences
        match c {
            'H' | 'f' => {
                // Cursor Position
                let row = params.get(0).and_then(|p| p.first()).copied().unwrap_or(1) as usize;
                let col = params.get(1).and_then(|p| p.first()).copied().unwrap_or(1) as usize;

                self.cursor_y = row.saturating_sub(1);
                self.cursor_x = col.saturating_sub(1);

                // Ensure lines vec is long enough
                while self.lines.len() <= self.cursor_y {
                    self.lines.push_back(String::new());
                }
            }
            'J' => {
                // Erase in Display
                // For simplicity: clear all screen if param 2 or 3, else do nothing
                let param = params.get(0).and_then(|p| p.first()).copied().unwrap_or(0);
                if param == 2 || param == 3 {
                    self.lines.clear();
                    self.cursor_x = 0;
                    self.cursor_y = 0;
                }
            }
            // Cursor request
            'n' => {
                println!("Cursor position request received");
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
            _ => {
                // Ignore other CSI sequences for now
            }
        }
    }
}
