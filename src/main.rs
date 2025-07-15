use crate::application::Application;
use crate::pty::PtySession;
use crate::terminal::Terminal;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, layer::SubscriberExt};
use tracing_subscriber::filter::EnvFilter;
use winit::event_loop::{ControlFlow, EventLoop};

mod application;
mod pty;
mod terminal;
mod window;

pub fn configure_logger() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
}


fn main() -> anyhow::Result<()> {
    configure_logger();

    let session = PtySession::spawn()?;
    let terminal = Terminal::new(session);

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut Application::new(terminal))?;

    Ok(())
}
