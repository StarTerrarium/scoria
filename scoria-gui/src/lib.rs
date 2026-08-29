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
pub mod picker;
pub mod settings;
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
        .window(window_settings())
        .run()?;
    Ok(())
}

/// How the window introduces itself to the desktop.
///
/// The application id is the pairing between a running window and the
/// `scoria.desktop` entry, and therefore the icon: without it a Wayland
/// compositor has nothing to match on and shows its placeholder. The embedded
/// icon covers the case where the desktop entry is not installed at all, which
/// is every `cargo run`.
fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(1200.0, 760.0),
        icon: icon(),
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "scoria".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The window icon, decoded from the same source the packaging installs.
///
/// `None` rather than a failure: a window with the desktop's placeholder icon
/// is a cosmetic problem, and refusing to open over one would not be.
fn icon() -> Option<iced::window::Icon> {
    const ICON: &[u8] = include_bytes!("../../packaging/scoria.png");

    let decoded = image::load_from_memory_with_format(ICON, image::ImageFormat::Png).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    iced::window::icon::from_rgba(rgba.into_raw(), width, height).ok()
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
