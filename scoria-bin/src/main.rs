//! The `scoria` binary.
//!
//! **This is the only crate in the workspace that builds an executable**, and
//! what it is for is choosing a frontend: `scoria` with no arguments opens the
//! window, and everything else runs here. `scoria-gui` is a library so that
//! nothing about launching a window shares a crate with the widgets, and so
//! that `scoria-tui` can arrive beside it as another library this binary picks
//! between. Frontend selection has to live somewhere that depends on every
//! frontend; this is that somewhere.
//!
//! The command line itself drives `scoria-core` through the same
//! `Command`/`Event` seam the graphical frontend uses, and nothing else. That
//! is deliberate: the seam stays honest because something already uses it, and
//! because this existed before any UI did, invariant 2 could not be violated by
//! code that had nowhere to violate it from.
//!
//! It is also permanently useful. Scanning a library, checking what the sort
//! order actually produces, and confirming playback works are all things worth
//! being able to do without a window.

mod args;
mod format;
mod out;
mod play;

use anyhow::{Context, Result};
use args::Cmd;
use out::Out;
use scoria_core::config::Paths;
use scoria_core::query::Selection;
use scoria_core::session::Session;
use std::io::Write;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        // `scoria tracks | head` closes the pipe once head has what it wants.
        // That is the reader finishing, not this program failing.
        Err(e) if out::is_closed_pipe(&e) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("scoria: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut out = Out::new();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&argv) {
        Ok(parsed) => parsed,
        Err(complaint) => {
            eprintln!("scoria: {complaint}");
            return Ok(());
        }
    };

    if parsed.command == Cmd::Gui {
        return scoria_gui::run();
    }
    if parsed.command == Cmd::Help {
        write!(out, "{}", args::HELP)?;
        return Ok(());
    }
    if parsed.command == Cmd::Version {
        writeln!(out, "scoria {}", env!("CARGO_PKG_VERSION"))?;
        return Ok(());
    }

    let paths = Paths::from_env().context("cannot work out where to keep settings")?;
    let mut session = Session::open(paths).context("cannot open the library")?;

    // --root applies to one run without being saved, so a library can be
    // inspected without disturbing the configured one.
    if let Some(root) = parsed.root {
        session.config_mut().root = Some(root);
    }

    match parsed.command {
        Cmd::Gui | Cmd::Help | Cmd::Version => Ok(()),
        Cmd::Info => info(&session, &mut out),
        Cmd::Config => config(&session, &mut out),
        Cmd::Scan { root } => scan(&mut session, &mut out, root),
        Cmd::Watch => watch(&mut session, &mut out),
        Cmd::Tracks { limit } => tracks(&session, &mut out, limit),
        Cmd::Artists => artists(&session, &mut out),
        Cmd::Albums { artist } => albums(&session, &mut out, artist),
        Cmd::Search { query } => search(&session, &mut out, &query),
        Cmd::Play { query, shuffle } => {
            play::run(&mut session, &mut out, query.as_deref(), shuffle)
        }
    }
}

// --------------------------------------------------------------- inspection --

fn info(session: &Session, out: &mut Out) -> Result<()> {
    let outcome = session.query(&Selection::all())?;

    writeln!(out, "root      {}", root_display(session))?;
    writeln!(out, "tracks    {}", outcome.total)?;
    writeln!(out, "albums    {}", outcome.albums.rows.len())?;
    writeln!(out, "artists   {}", outcome.album_artists.rows.len())?;

    let unavailable = session
        .index()
        .tracks()
        .iter()
        .filter(|t| !t.available)
        .count();
    if unavailable > 0 {
        writeln!(
            out,
            "missing   {unavailable} (shown struck through, never dropped)"
        )?;
    }

    let tagged = session
        .index()
        .tracks()
        .iter()
        .filter(|t| t.replay_gain.is_tagged())
        .count();
    writeln!(out, "replaygain {tagged}/{} tracks tagged", outcome.total)?;
    if tagged < outcome.total && outcome.total > 0 {
        writeln!(
            out,
            "          Scoria reads gain but never computes it; `rsgain easy <root>` fills the gaps."
        )?;
    }
    Ok(())
}

fn config(session: &Session, out: &mut Out) -> Result<()> {
    let paths = session.paths();
    writeln!(out, "root            {}", root_display(session))?;
    writeln!(out, "play behaviour  {:?}", session.config().play_behaviour)?;
    writeln!(out, "sort            {}", session.config().sort.label())?;
    writeln!(out)?;
    writeln!(out, "settings        {}", paths.config_file().display())?;
    writeln!(out, "database        {}", paths.database().display())?;
    writeln!(out, "artwork cache   {}", paths.artwork_cache().display())?;
    Ok(())
}

fn root_display(session: &Session) -> String {
    session.config().root.as_ref().map_or_else(
        || "(not set -- run `scoria scan <path>`)".to_owned(),
        |r| r.display().to_string(),
    )
}

// ------------------------------------------------------------------ library --

fn scan(session: &mut Session, out: &mut Out, root: Option<std::path::PathBuf>) -> Result<()> {
    if let Some(root) = root {
        let root = root
            .canonicalize()
            .with_context(|| format!("no such directory: {}", root.display()))?;
        session.config_mut().root = Some(root);
        session
            .save_config()
            .context("cannot save the library root")?;
    }

    let started = std::time::Instant::now();
    let mut last = 0;
    // Rewriting one line beats scrolling, but only where there is a cursor to
    // move. Piped into a file it would just be noise, so it is not emitted.
    let interactive = out.interactive();

    let outcome = session
        .rescan(&mut |progress| {
            if interactive && (progress.seen >= last + 25 || Some(progress.seen) == progress.total)
            {
                last = progress.seen;
                let total = progress
                    .total
                    .map_or_else(|| "?".to_owned(), |t| t.to_string());
                let _ = write!(out, "\r  scanning {}/{total}", progress.seen);
                let _ = out.flush();
            }
        })
        .context("scan failed")?;

    if interactive {
        write!(out, "\r{}\r", " ".repeat(30))?;
    }
    writeln!(out, "  scanned in {:.1?}", started.elapsed())?;

    writeln!(out)?;
    writeln!(out, "  added        {}", outcome.added.len())?;
    writeln!(out, "  updated      {}", outcome.updated.len())?;
    if !outcome.now_unavailable.is_empty() {
        writeln!(out, "  now missing  {}", outcome.now_unavailable.len())?;
    }
    if !outcome.now_available.is_empty() {
        writeln!(out, "  restored     {}", outcome.now_available.len())?;
    }
    if outcome.albums_removed > 0 {
        writeln!(out, "  albums gone  {}", outcome.albums_removed)?;
    }
    if !outcome.failed.is_empty() {
        writeln!(out, "  unreadable   {}", outcome.failed.len())?;
        for (path, why) in outcome.failed.iter().take(10) {
            writeln!(out, "      {} -- {why}", path.display())?;
        }
        if outcome.failed.len() > 10 {
            writeln!(out, "      ... and {} more", outcome.failed.len() - 10)?;
        }
    }
    Ok(())
}

fn watch(session: &mut Session, out: &mut Out) -> Result<()> {
    let watcher = session.watch().context("cannot watch the library root")?;
    writeln!(out, "watching {} -- Ctrl-C to stop", root_display(session))?;

    loop {
        if let Some(changed) = watcher.next_change(std::time::Duration::from_secs(3600)) {
            writeln!(out, "\nchanged:")?;
            for directory in changed.directories.iter().take(10) {
                writeln!(out, "  {}", directory.display())?;
            }
            let outcome = session.rescan(&mut |_| {}).context("rescan failed")?;
            writeln!(
                out,
                "  rescanned: {} added, {} updated, {} missing",
                outcome.added.len(),
                outcome.updated.len(),
                outcome.now_unavailable.len()
            )?;
        }
    }
}

// ----------------------------------------------------------------- browsing --

fn tracks(session: &Session, out: &mut Out, limit: usize) -> Result<()> {
    let outcome = session.query(&Selection::all())?;
    let rows = session.index().rows(&outcome.tracks);

    writeln!(out, "{}", format::track_header())?;
    let mut group = outcome.groups.iter().peekable();
    for (index, track) in rows.iter().take(limit).enumerate() {
        // §7.3: a header row wherever the primary sort key's value changes.
        if let Some(g) = group.peek()
            && g.start == index
        {
            writeln!(out, "\n  {}", g.label)?;
            group.next();
        }
        writeln!(out, "{}", format::track_row(track))?;
    }

    if rows.len() > limit {
        writeln!(
            out,
            "\n  ... {} more of {} ({} shown)",
            rows.len() - limit,
            rows.len(),
            limit
        )?;
    }
    Ok(())
}

fn artists(session: &Session, out: &mut Out) -> Result<()> {
    let outcome = session.query(&Selection::all())?;
    // §7.2: All (n) is pinned at the top of every pane.
    writeln!(
        out,
        "  {}  {}",
        format::pad("All", 32),
        outcome.album_artists.total
    )?;
    for row in &outcome.album_artists.rows {
        writeln!(out, "  {}  {}", format::pad(&row.label, 32), row.count)?;
    }
    Ok(())
}

fn albums(session: &Session, out: &mut Out, artist: Option<String>) -> Result<()> {
    let selection = Selection {
        album_artists: artist.clone().into_iter().collect(),
        ..Selection::all()
    };
    let outcome = session.query(&selection)?;

    if let Some(artist) = &artist
        && outcome.albums.rows.is_empty()
    {
        writeln!(out, "no albums for {artist}")?;
        return Ok(());
    }

    writeln!(
        out,
        "  {}  {}",
        format::pad("All", 40),
        outcome.albums.total
    )?;
    for row in &outcome.albums.rows {
        writeln!(out, "  {}  {}", format::pad(&row.label, 40), row.count)?;
    }
    Ok(())
}

fn search(session: &Session, out: &mut Out, query: &str) -> Result<()> {
    let selection = Selection {
        search: Some(query.to_owned()),
        ..Selection::all()
    };
    let outcome = session.query(&selection)?;

    if outcome.is_empty() {
        writeln!(out, "nothing matches {query:?}")?;
        if query.chars().count() < 3 {
            writeln!(
                out,
                "(short queries fall back to a scan; try more characters)"
            )?;
        }
        return Ok(());
    }

    writeln!(out, "{}", format::track_header())?;
    for track in session.index().rows(&outcome.tracks) {
        writeln!(out, "{}", format::track_row(track))?;
    }
    writeln!(out, "\n  {} of {} tracks", outcome.len(), outcome.total)?;
    Ok(())
}
