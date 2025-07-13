use anyhow::Result;
use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use portable_pty::Child;
use portable_pty::CommandBuilder;
use portable_pty::MasterPty;
use portable_pty::NativePtySystem;
use portable_pty::PtySize;
use portable_pty::PtySystem;
use std::io::BufReader;
use std::io::Read;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

#[derive(Clone)]
pub struct PtySession {
    _session: Arc<Mutex<Session>>,
    reader: Receiver<String>,
    writer: Sender<Vec<u8>>,
}

impl PtySession {
    pub fn spawn() -> Result<Self> {
        let inner = Session::spawn()?;
        let reader = inner.receiver.clone();
        let writer = inner.sender.clone();
        Ok(Self { 
            _session: Arc::new(Mutex::new(inner)),
            reader,
            writer,
        })
    }

    pub fn get_reader(&self) -> Receiver<String> {
        self.reader.clone()
    }

    pub fn get_writer(&self) -> Sender<Vec<u8>> {
        self.writer.clone()
    }
}

fn get_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "sh".into())
    }
}

#[allow(dead_code)]
struct Session {
    pub child: Box<dyn Child + Send>,
    pub master: Box<dyn MasterPty + Send>,
    pub receiver: Receiver<String>,
    pub sender: Sender<Vec<u8>>,
}

impl Session {
    const DEFAULT_ENV: &[(&str, &str)] = &[
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("LANG", "en_US.UTF-8"),
    ];

    /// Spawns the shell inside a PTY and returns a receiver for its output
    fn spawn() -> Result<Self> {
        let shell = get_shell();
        eprintln!("Spawning shell: {}", shell);

        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize::default())?;

        let mut command = CommandBuilder::new(shell);
        for (key, value) in Self::DEFAULT_ENV {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command)?;

        let (reader_tx, reader_rx): (Sender<String>, Receiver<String>) =
            crossbeam_channel::unbounded();
        let (writer_tx, writer_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) =
            crossbeam_channel::unbounded();

        // Spawn the reader thread
        Self::start_reader(pair.master.try_clone_reader()?, reader_tx);

        // Spawn the writer thread
        Self::start_writer(pair.master.take_writer()?, writer_rx);

        Ok(Self {
            child,
            master: pair.master,
            receiver: reader_rx,
            sender: writer_tx,
        })
    }

    fn start_reader(reader: Box<dyn std::io::Read + Send>, sender: Sender<String>) {
        let mut reader = BufReader::new(reader);
        thread::spawn(move || {
            let mut leftover = Vec::new();
            let mut buf = [0u8; 4096];

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        leftover.extend_from_slice(&buf[..n]);

                        // Use from_utf8 to check whole buffer validity
                        match std::str::from_utf8(&leftover) {
                            Ok(valid_str) => {
                                // whole buffer is valid UTF-8
                                eprintln!("PTY TEXT: {:?}", valid_str);
                                if sender.send(valid_str.to_string()).is_err() {
                                    break;
                                }
                                leftover.clear();
                            }
                            Err(e) => {
                                let valid_up_to = e.valid_up_to();
                                if valid_up_to > 0 {
                                    // slice only up to the valid UTF-8 boundary
                                    let valid_str =
                                        String::from_utf8_lossy(&leftover[..valid_up_to])
                                            .to_string();
                                    eprintln!("PTY TEXT: {:?}", valid_str);
                                    if sender.send(valid_str).is_err() {
                                        break;
                                    }
                                    // keep the remaining bytes for next iteration
                                    leftover = leftover[valid_up_to..].to_vec();
                                } else {
                                    // no valid UTF-8 data yet, wait for more bytes
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("PTY read error: {e}");
                        break;
                    }
                }
            }
        });
    }

    fn start_writer(mut writer: Box<dyn std::io::Write + Send>, receiver: Receiver<Vec<u8>>) {
        thread::spawn(move || {
            while let Ok(command) = receiver.recv() {
                eprintln!("PTY WRITE: {:?}", command);
                if writer.write_all(&command).is_err() {
                    eprintln!("Failed to write to PTY");
                    break;
                }
                if writer.flush().is_err() {
                    eprintln!("Failed to flush PTY writer");
                    break;
                }
            }
        });
    }
}
