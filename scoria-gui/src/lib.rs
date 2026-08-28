//! iced frontend.
//!
//! Sends `scoria_core::Command` and renders `scoria_core::Event`. A view
//! function that reaches into a library, database or playback object has broken
//! the seam -- see `CLAUDE.md` invariant 2.
//!
//! iced 0.14 specifically. The 0.12 and 0.13 APIs are not compatible and are
//! well represented in training data; check docs.rs/iced/0.14 rather than
//! writing from memory.

pub mod app;
pub mod keymap;
pub mod layouts;
pub mod overlays;
pub mod theme;
pub mod widgets;

use anyhow::Result;

/// Runs the graphical frontend.
///
/// # Errors
///
/// If the library cannot be opened, or the window cannot be created.
pub fn run() -> Result<()> {
    // The library is opened before the window, so a failure to read it is a
    // clear error on the terminal rather than an empty window.
    app::App::new()?;

    iced::application(boot, app::App::update, app::App::view)
        .title(app::App::title)
        .theme(app::App::theme)
        .subscription(app::App::subscription)
        .window_size((1200.0, 760.0))
        .run()?;
    Ok(())
}

/// iced calls this to build the initial state.
///
/// It re-opens rather than moving one in, because `application` takes a `Fn`
/// that may be called more than once. Opening is cheap -- the expensive part,
/// scanning, is a command.
fn boot() -> app::App {
    app::App::new().unwrap_or_else(|_| {
        // `run` already opened the library successfully, so reaching here means
        // it became unreadable in between. Better an empty window that says so
        // than a panic.
        app::App::disconnected("the library could not be opened".to_owned())
    })
}
