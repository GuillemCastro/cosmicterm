use crate::application::Application;
use crate::pty::PtySession;
use crate::terminal::Terminal;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use winit::event_loop::ControlFlow;
use winit::event_loop::EventLoop;

mod application;
mod pty;
mod terminal;
mod window;

pub fn configure_logger() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        // filter only the `cosmicterm` target
        .with(
            tracing_subscriber::filter::Targets::new()
                .with_target("cosmicterm", tracing::Level::DEBUG),
        )
        // initialize the subscriber
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
