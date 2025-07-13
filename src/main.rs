use crate::application::Application;
use crate::pty::PtySession;
use crate::terminal::Terminal;
use winit::event_loop::{ControlFlow, EventLoop};

mod application;
mod pty;
mod terminal;
mod window;

fn main() -> anyhow::Result<()> {
    let session = PtySession::spawn()?;
    let terminal = Terminal::new(session);

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut Application::new(terminal))?;

    Ok(())
}
