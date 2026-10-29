//! Argument parsing.
//!
//! Hand-rolled rather than `clap`. The surface is a handful of subcommands with
//! at most one flag each, and `clap` would be the largest dependency in the
//! workspace for a tool that exists to debug the seam. If this grows options
//! worth a parser, that is the moment to add one -- not before.

use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Cmd {
    /// Launch the graphical frontend. What a bare `scoria` does.
    Gui,
    Help,
    Version,
    Info,
    Config,
    Scan {
        root: Option<PathBuf>,
    },
    Watch,
    /// Remove tracks whose files are gone (§7.4). Asks first unless `yes`.
    ForgetMissing {
        yes: bool,
    },
    /// Measure loudness and write ReplayGain tags (§6.7).
    AnalyseGain {
        /// Re-analyse albums that are already tagged.
        all: bool,
        /// Measure and print; write nothing.
        dry_run: bool,
        yes: bool,
    },
    Tracks {
        limit: usize,
    },
    Artists,
    Albums {
        artist: Option<String>,
    },
    Search {
        query: String,
    },
    Play {
        query: Option<String>,
        shuffle: bool,
    },
}

#[derive(Debug)]
pub struct Parsed {
    pub command: Cmd,
    /// Overrides the configured library root for this run only.
    pub root: Option<PathBuf>,
}

const DEFAULT_LIMIT: usize = 50;

/// Parses argv, or returns a message explaining what was wrong with it.
///
/// # Errors
///
/// A human-readable complaint, already phrased for printing.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut root = None;
    let mut rest: Vec<&str> = Vec::new();

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => {
                let value = it.next().ok_or("--root needs a path")?;
                root = Some(PathBuf::from(value));
            }
            other => rest.push(other),
        }
    }

    // A bare `scoria` opens the window. §3.1: this binary is thin and selects a
    // frontend; the subcommands are the debugging harness underneath it.
    let Some((name, tail)) = rest.split_first() else {
        return Ok(Parsed {
            command: Cmd::Gui,
            root,
        });
    };

    let command = match *name {
        "gui" => Cmd::Gui,
        "help" | "-h" | "--help" => Cmd::Help,
        "version" | "-V" | "--version" => Cmd::Version,
        "info" => Cmd::Info,
        "config" => Cmd::Config,
        "watch" => Cmd::Watch,
        "forget-missing" => Cmd::ForgetMissing {
            yes: tail.iter().any(|a| matches!(*a, "--yes" | "-y")),
        },
        "analyse-gain" | "analyze-gain" => Cmd::AnalyseGain {
            all: tail.contains(&"--all"),
            dry_run: tail.contains(&"--dry-run"),
            yes: tail.iter().any(|a| matches!(*a, "--yes" | "-y")),
        },
        "scan" => Cmd::Scan {
            root: tail.first().map(PathBuf::from),
        },
        "tracks" => {
            let limit = match tail.first() {
                None => DEFAULT_LIMIT,
                Some(v) => v.parse().map_err(|_| format!("not a number: {v}"))?,
            };
            Cmd::Tracks { limit }
        }
        "artists" => Cmd::Artists,
        "albums" => Cmd::Albums { artist: join(tail) },
        "search" => {
            let query = join(tail).ok_or("search needs something to search for")?;
            Cmd::Search { query }
        }
        "play" => {
            let shuffle = tail.contains(&"--shuffle");
            let terms: Vec<&str> = tail.iter().copied().filter(|a| *a != "--shuffle").collect();
            Cmd::Play {
                query: join(&terms),
                shuffle,
            }
        }
        other => return Err(format!("unknown command: {other}\n\nTry `scoria help`.")),
    };

    Ok(Parsed { command, root })
}

/// Joins the remaining words, so `scoria play electric callboy` works without
/// quoting.
fn join(parts: &[&str]) -> Option<String> {
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(" "))
}

pub const HELP: &str = "\
scoria -- a library-centric music player for Linux

USAGE
    scoria                     Open the player.
    scoria [--root PATH] <command>

LIBRARY
    scan [PATH]        Index a library. With PATH, makes it the library root.
    watch              Watch the root and rescan when it changes.
    forget-missing     Remove tracks whose files are gone, and their play counts.
        --yes          Do not ask first.
    analyse-gain       Measure loudness and write ReplayGain tags, as `rsgain easy`
                       would, into albums that lack them.
        --dry-run      Measure and print; write nothing.
        --all          Re-analyse albums that are already tagged.
        --yes          Do not ask first.
    info               Library and playback status.
    config             Settings, and where they live.

BROWSING
    tracks [N]         List tracks in the default sort order. Default 50.
    artists            Album artists with track counts, as the filter pane shows them.
    albums [ARTIST]    Albums, optionally for one artist.
    search QUERY       Full-text search across title, artist and album.

PLAYBACK
    play [QUERY]       Play matching tracks, or the whole library.
        --shuffle      Shuffle the snapshot.

    While playing, type a key and press Enter:
        p  pause or resume       n  next            b  previous
        s  toggle shuffle        r  cycle repeat    q  quit
        +  louder                -  quieter         ?  what is playing

OPTIONS
    --root PATH        Use this library root for one run, without saving it.

This is a debugging harness for the core. It drives the same Command/Event
seam the graphical frontend will, which is what keeps that seam honest.
";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn parse_args(args: &[&str]) -> Parsed {
        parse(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn no_arguments_opens_the_window() {
        assert_eq!(parse_args(&[]).command, Cmd::Gui);
    }

    #[test]
    fn multi_word_queries_need_no_quoting() {
        // `scoria play electric callboy` is what someone will actually type.
        let parsed = parse_args(&["play", "electric", "callboy"]);
        assert_eq!(
            parsed.command,
            Cmd::Play {
                query: Some("electric callboy".to_owned()),
                shuffle: false
            }
        );
    }

    #[test]
    fn the_shuffle_flag_is_not_mistaken_for_a_search_term() {
        let parsed = parse_args(&["play", "ghost", "--shuffle"]);
        assert_eq!(
            parsed.command,
            Cmd::Play {
                query: Some("ghost".to_owned()),
                shuffle: true
            }
        );
    }

    #[test]
    fn root_can_appear_anywhere() {
        for args in [
            vec!["--root", "/music", "artists"],
            vec!["artists", "--root", "/music"],
        ] {
            let parsed = parse_args(&args);
            assert_eq!(parsed.command, Cmd::Artists);
            assert_eq!(parsed.root.as_deref(), Some(std::path::Path::new("/music")));
        }
    }

    #[test]
    fn a_missing_root_value_is_explained_rather_than_ignored() {
        let args = vec!["--root".to_owned()];
        assert!(parse(&args).is_err());
    }

    #[test]
    fn an_unknown_command_says_so() {
        let args = vec!["frobnicate".to_owned()];
        let complaint = parse(&args).unwrap_err();
        assert!(complaint.contains("frobnicate"), "{complaint}");
    }

    #[test]
    fn track_limits_are_parsed_and_defaulted() {
        assert_eq!(
            parse_args(&["tracks"]).command,
            Cmd::Tracks {
                limit: DEFAULT_LIMIT
            }
        );
        assert_eq!(
            parse_args(&["tracks", "5"]).command,
            Cmd::Tracks { limit: 5 }
        );
        assert!(parse(&["tracks".to_owned(), "many".to_owned()]).is_err());
    }

    #[test]
    fn scan_takes_an_optional_path() {
        assert_eq!(parse_args(&["scan"]).command, Cmd::Scan { root: None });
    }

    #[test]
    fn analysing_takes_its_flags_in_any_order() {
        assert_eq!(
            parse_args(&["analyse-gain"]).command,
            Cmd::AnalyseGain {
                all: false,
                dry_run: false,
                yes: false
            }
        );
        assert_eq!(
            parse_args(&["analyse-gain", "-y", "--dry-run", "--all"]).command,
            Cmd::AnalyseGain {
                all: true,
                dry_run: true,
                yes: true
            }
        );
        // The other spelling, because someone will type it.
        assert_eq!(
            parse_args(&["analyze-gain"]).command,
            parse_args(&["analyse-gain"]).command
        );
    }

    #[test]
    fn forgetting_asks_unless_told_not_to() {
        assert_eq!(
            parse_args(&["forget-missing"]).command,
            Cmd::ForgetMissing { yes: false }
        );
        for flag in ["--yes", "-y"] {
            assert_eq!(
                parse_args(&["forget-missing", flag]).command,
                Cmd::ForgetMissing { yes: true }
            );
        }
        assert_eq!(
            parse_args(&["scan", "/music"]).command,
            Cmd::Scan {
                root: Some(PathBuf::from("/music"))
            }
        );
    }

    #[test]
    fn help_covers_every_command_it_accepts() {
        // A command that parses but is undocumented is one nobody finds.
        for name in [
            "scan", "watch", "info", "config", "tracks", "artists", "albums", "search", "play",
        ] {
            assert!(HELP.contains(name), "{name} is missing from the help text");
        }
    }
}
