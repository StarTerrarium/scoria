//! Playback, and the little bit of interaction that makes it testable.
//!
//! Line-based rather than raw-mode: typing a key and pressing Enter needs no
//! terminal library and no dependency, and this is a debugging harness rather
//! than the interface anyone is meant to live in. The real keyboard model is a
//! data-driven keymap in the UI layer (§10), which is a different thing
//! entirely and is deliberately not prototyped here.

use crate::format;
use crate::out::Out;
use anyhow::{Context, Result};
use scoria_core::command::Command;
use scoria_core::event::{Event, PlaybackState};
use scoria_core::id::TrackId;
use scoria_core::playback::RepeatMode;
use scoria_core::query::Selection;
use scoria_core::session::Session;
use std::io::{BufRead, Write};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

/// Plays what the query selects, or the whole library.
pub fn run(session: &mut Session, out: &mut Out, query: Option<&str>, shuffle: bool) -> Result<()> {
    let selection = Selection {
        search: query.map(ToOwned::to_owned),
        ..Selection::all()
    };
    let outcome = session.query(&selection)?;

    if outcome.is_empty() {
        match query {
            Some(q) => writeln!(out, "nothing matches {q:?}")?,
            None => writeln!(
                out,
                "the library is empty -- run `scoria scan <path>` first"
            )?,
        }
        return Ok(());
    }

    let Some(first) = outcome.tracks.first().copied() else {
        return Ok(());
    };

    // The §6.3 setting decides what gets snapshotted; the session is the one
    // place that decision is made.
    let label = selection.label().or_else(|| query.map(ToOwned::to_owned));
    let spec = session.source_spec(outcome.tracks.clone(), label);

    writeln!(out, "playing {} track(s)", outcome.len())?;
    writeln!(
        out,
        "type a key and press Enter -- p pause, n next, b back, s shuffle, r repeat, q quit\n"
    )?;

    // Media keys work here too. Playing from a terminal is still playing, and
    // the desktop has no reason to care which frontend asked.
    if let Err(e) = session.start_media_controls() {
        writeln!(out, "  media keys unavailable: {e}")?;
    }

    session
        .handle(Command::PlayFrom {
            source: spec,
            at: first,
        })
        .context("cannot start playback")?;

    if shuffle {
        session.handle(Command::SetShuffle(true))?;
    }

    let keys = spawn_stdin_reader();
    let mut ui = Transport::new(shuffle);

    loop {
        while let Some(event) = session.next_event(Duration::from_millis(80)) {
            if ui.apply(session, out, &event)? {
                return Ok(());
            }
        }

        match keys.try_recv() {
            Ok(key) => {
                if ui.key(session, out, &key)? {
                    writeln!(out)?;
                    return Ok(());
                }
            }
            Err(TryRecvError::Empty) => {}
            // stdin closed: keep playing, since output may be piped somewhere.
            Err(TryRecvError::Disconnected) => {}
        }
    }
}

/// Returns to the start of a clean line.
///
/// Only meaningful on a terminal: piped into a file or another program, the
/// carriage return cannot rewrite anything and would just be noise.
fn clear_line(out: &mut Out) -> Result<()> {
    if out.interactive() {
        write!(out, "\r{}\r", " ".repeat(78))?;
    }
    Ok(())
}

/// Reads keys on their own thread so the event loop never blocks on input.
fn spawn_stdin_reader() -> Receiver<String> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if tx.send(line.trim().to_owned()).is_err() {
                return;
            }
        }
    });
    rx
}

/// What the transport bar would show, as one rewritten line.
struct Transport {
    track: Option<TrackId>,
    title: String,
    subtitle: String,
    position: Duration,
    duration: Duration,
    state: PlaybackState,
    shuffled: bool,
    repeat: RepeatMode,
    volume: f32,
}

impl Transport {
    fn new(shuffled: bool) -> Self {
        Self {
            track: None,
            title: String::new(),
            subtitle: String::new(),
            position: Duration::ZERO,
            duration: Duration::ZERO,
            state: PlaybackState::Stopped,
            shuffled,
            repeat: RepeatMode::Off,
            volume: 1.0,
        }
    }

    /// Applies an event. Returns true when playback has finished for good.
    fn apply(&mut self, session: &Session, out: &mut Out, event: &Event) -> Result<bool> {
        match event {
            Event::TrackChanged { track, .. } => {
                self.track = Some(*track);
                self.position = Duration::ZERO;
                if let Some(t) = session.index().track(*track) {
                    // §8: title, then `artist — album`.
                    self.title = t.title.clone();
                    self.subtitle = format!("{} — {}", t.artist, t.album);
                    self.duration = t.duration;
                }
                clear_line(out)?;
                writeln!(out, "  ▸ {}", self.title)?;
                writeln!(out, "    {}", self.subtitle)?;
            }
            Event::PositionChanged(p) => self.position = *p,
            Event::PlaybackStateChanged(state) => {
                self.state = *state;
                if *state == PlaybackState::Stopped && self.track.is_some() {
                    clear_line(out)?;
                    writeln!(out, "  end of queue")?;
                    return Ok(true);
                }
            }
            Event::QueueChanged(source) => {
                self.shuffled = source.is_shuffled();
                self.repeat = source.repeat();
            }
            Event::Error(e) => {
                clear_line(out)?;
                writeln!(out, "  error: {e}")?;
            }
            _ => {}
        }
        self.draw(out)?;
        Ok(false)
    }

    fn draw(&self, out: &mut Out) -> Result<()> {
        // A transport bar is a line that rewrites itself. Without a cursor to
        // move there is nothing to rewrite, so it would just repeat.
        if !out.interactive() {
            return Ok(());
        }

        let flags = format!(
            "{}{}",
            if self.shuffled { " shuffle" } else { "" },
            match self.repeat {
                RepeatMode::Off => "",
                RepeatMode::All => " repeat-all",
                RepeatMode::One => " repeat-one",
            }
        );
        write!(
            out,
            "\r  {} {} / {}  vol {:.0}%{}   ",
            match self.state {
                PlaybackState::Playing => "▶",
                PlaybackState::Paused => "⏸",
                PlaybackState::Stopped => "■",
            },
            format::duration(self.position),
            format::duration(self.duration),
            self.volume * 100.0,
            flags,
        )?;
        let _ = out.flush();
        Ok(())
    }

    /// Handles a key. Returns true when the user asked to quit.
    fn key(&mut self, session: &mut Session, out: &mut Out, key: &str) -> Result<bool> {
        match key {
            "q" | "quit" | "exit" => {
                session.handle(Command::Stop).ok();
                return Ok(true);
            }
            "p" | "" => {
                let command = if self.state == PlaybackState::Playing {
                    Command::Pause
                } else {
                    Command::Resume
                };
                session.handle(command)?;
            }
            "n" | "next" => session.handle(Command::Next)?,
            "b" | "prev" => session.handle(Command::Previous)?,
            "s" => {
                self.shuffled = !self.shuffled;
                session.handle(Command::SetShuffle(self.shuffled))?;
            }
            "r" => {
                self.repeat = match self.repeat {
                    RepeatMode::Off => RepeatMode::All,
                    RepeatMode::All => RepeatMode::One,
                    RepeatMode::One => RepeatMode::Off,
                };
                session.handle(Command::SetRepeat(self.repeat))?;
            }
            "+" => {
                self.volume = (self.volume + 0.1).min(1.0);
                session.handle(Command::SetVolume(self.volume))?;
            }
            "-" => {
                self.volume = (self.volume - 0.1).max(0.0);
                session.handle(Command::SetVolume(self.volume))?;
            }
            "?" => {
                clear_line(out)?;
                writeln!(out, "  {} — {}", self.title, self.subtitle)?;
            }
            other => {
                clear_line(out)?;
                writeln!(out, "  unknown key {other:?}")?;
            }
        }
        self.draw(out)?;
        Ok(false)
    }
}
