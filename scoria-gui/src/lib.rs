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
pub mod selection;
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
    // clear error on the terminal rather than an empty window -- and the one
    // that is opened is the one that gets used.
    //
    // Building it twice was not merely wasteful. `App::new` starts watching,
    // and watching begins with a catch-up scan; dropping the spare joined that
    // scan thread, which only checks whether it should stop between scans. So
    // opening the window waited for a full pass over the library -- every
    // file's tags, since the scanner has no mtime shortcut -- and then a second
    // pass started behind the window that finally appeared.
    let first = std::cell::RefCell::new(Some(app::App::new()?));

    iced::application(move || boot(&first), app::App::update, app::App::view)
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
/// `io.github.starterrarium.Scoria.desktop` entry, and therefore the icon:
/// without it a Wayland compositor has nothing to match on and shows its
/// placeholder. It is reverse-DNS because every store format requires that
/// (`DESIGN.md` §15); the binary and the MPRIS name are both still `scoria`.
/// The embedded icon covers the case where the desktop entry is not installed
/// at all, which is every `cargo run`.
fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(1200.0, 760.0),
        icon: icon(),
        #[cfg(target_os = "linux")]
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: "io.github.starterrarium.Scoria".to_owned(),
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
    const ICON: &[u8] = include_bytes!("../../packaging/io.github.starterrarium.Scoria.png");

    let decoded = image::load_from_memory_with_format(ICON, image::ImageFormat::Png).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    iced::window::icon::from_rgba(rgba.into_raw(), width, height).ok()
}

/// iced calls this to build the initial state.
///
/// Hands over the application `run` already built. `application` takes a `Fn`
/// that may be called more than once, so this still copes with a second call
/// -- it just builds a second application then, rather than throwing the first
/// one away every time.
fn boot(first: &std::cell::RefCell<Option<app::App>>) -> app::App {
    first
        .borrow_mut()
        .take()
        .or_else(|| app::App::new().ok())
        .unwrap_or_else(|| {
            // `run` opened the library successfully, so reaching here means it
            // became unreadable in between. Better an empty window that says
            // so than a panic.
            app::App::disconnected("the library could not be opened".to_owned())
        })
}
