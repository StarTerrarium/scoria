//! Loudness analysis against real files (§6.7).
//!
//! The writer is the only code in Scoria that changes the library, so what has
//! to hold is that it changes exactly four comments and nothing else -- on a
//! file tagged the way Picard tags one, with the multi-valued fields and the
//! cover art that a careless rewrite would lose.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::flac::FlacFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OggPictureStorage, VorbisFile};
use lofty::picture::{MimeType, Picture, PictureInformation, PictureType};
use scoria_core::library::gain::Gain;
use scoria_core::library::gain::measure::{measure, measure_samples};
use scoria_core::library::gain::write::{GainTags, write};
use std::fs::File;
use std::path::{Path, PathBuf};

mod common;
use common::Scratch;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// A 1x1 PNG: real enough to be a picture, small enough to inline.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

fn tags() -> GainTags {
    GainTags {
        track: Gain {
            gain_db: -10.123,
            peak: 0.971_802_4,
        },
        album: Gain {
            gain_db: -9.756,
            peak: 0.991_425_1,
        },
    }
}

fn copy_fixture(scratch: &Scratch, name: &str) -> PathBuf {
    let to = scratch.join(name);
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::copy(Path::new(FIXTURES).join(name), &to).unwrap();
    to
}

/// The FLAC fixture, tagged the way Picard leaves a file.
fn picard_flac(scratch: &Scratch) -> PathBuf {
    let path = copy_fixture(scratch, "silence.flac");
    let mut flac =
        FlacFile::read_from(&mut File::open(&path).unwrap(), ParseOptions::new()).unwrap();

    let mut comments = VorbisComments::default();
    comments.set_vendor("reference libFLAC 1.4.3 20230623".to_owned());
    for (key, value) in [
        ("TITLE", "花冷え。"),
        ("ARTIST", "Electric Callboy feat. Conquer Divide"),
        ("ALBUMARTIST", "Electric Callboy"),
        ("ALBUM", "TEKKNO"),
        ("GENRE", "Metalcore"),
        ("GENRE", "Electronicore"),
        (
            "MUSICBRAINZ_ALBUMID",
            "9a7d2c43-6e1b-4a1f-8a51-2c3d1e9f0b77",
        ),
        (
            "MUSICBRAINZ_TRACKID",
            "0d1f4e7a-3b2c-4d5e-9f8a-7b6c5d4e3f21",
        ),
        ("ARTISTS", "Electric Callboy"),
        ("ARTISTS", "Conquer Divide"),
        // Something another tool wrote, in a case nobody should normalise.
        ("replaygain_reference_loudness", "89.0 dB"),
    ] {
        comments.push(key.to_owned(), value.to_owned());
    }
    flac.set_vorbis_comments(comments);
    let cover = Picture::unchecked(PNG.to_vec())
        .pic_type(PictureType::CoverFront)
        .mime_type(MimeType::Png)
        .description("front")
        .build();
    flac.insert_picture(cover, Some(PictureInformation::default()))
        .unwrap();
    flac.save_to_path(&path, WriteOptions::default()).unwrap();
    path
}

fn read_flac(path: &Path) -> FlacFile {
    FlacFile::read_from(&mut File::open(path).unwrap(), ParseOptions::new()).unwrap()
}

fn comments_except_gain(flac: &FlacFile) -> Vec<(String, String)> {
    flac.vorbis_comments()
        .unwrap()
        .items()
        .filter(|(k, _)| !k.starts_with("REPLAYGAIN_"))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn samples(path: &Path) -> Vec<f32> {
    scoria_core::decode::open(path).unwrap().collect()
}

// ------------------------------------------------------------------ writing --

#[test]
fn writing_gain_changes_four_comments_and_nothing_else() {
    let scratch = Scratch::new("gain-picard");
    let path = picard_flac(&scratch);
    let before = read_flac(&path);
    let audio_before = samples(&path);

    write(&path, &tags()).unwrap();
    let after = read_flac(&path);

    assert_eq!(
        comments_except_gain(&before),
        comments_except_gain(&after),
        "every other comment survives, in its order, multi-valued fields included"
    );
    assert_eq!(
        before.vorbis_comments().unwrap().vendor(),
        after.vorbis_comments().unwrap().vendor()
    );

    assert_eq!(after.pictures().len(), 1, "the cover art survives");
    let (picture, _) = &after.pictures()[0];
    assert_eq!(picture.data(), PNG);
    assert_eq!(picture.pic_type(), PictureType::CoverFront);
    assert_eq!(picture.description(), Some("front"));

    let comments = after.vorbis_comments().unwrap();
    assert_eq!(comments.get("REPLAYGAIN_TRACK_GAIN"), Some("-10.12 dB"));
    assert_eq!(comments.get("REPLAYGAIN_TRACK_PEAK"), Some("0.971802"));
    assert_eq!(comments.get("REPLAYGAIN_ALBUM_GAIN"), Some("-9.76 dB"));
    assert_eq!(comments.get("REPLAYGAIN_ALBUM_PEAK"), Some("0.991425"));

    assert_eq!(samples(&path), audio_before, "the audio is untouched");
}

#[test]
fn writing_again_replaces_rather_than_repeats() {
    let scratch = Scratch::new("gain-twice");
    let path = picard_flac(&scratch);
    write(&path, &tags()).unwrap();

    let mut louder = tags();
    louder.track.gain_db = -3.0;
    write(&path, &louder).unwrap();

    let after = read_flac(&path);
    let comments = after.vorbis_comments().unwrap();
    let gains: Vec<&str> = comments.get_all("REPLAYGAIN_TRACK_GAIN").collect();
    assert_eq!(gains, ["-3.00 dB"]);
}

#[test]
fn an_ogg_vorbis_file_is_written_too() {
    let scratch = Scratch::new("gain-ogg");
    let path = copy_fixture(&scratch, "silence.ogg");
    write(&path, &tags()).unwrap();

    let vorbis =
        VorbisFile::read_from(&mut File::open(&path).unwrap(), ParseOptions::new()).unwrap();
    assert_eq!(
        vorbis.vorbis_comments().get("REPLAYGAIN_ALBUM_GAIN"),
        Some("-9.76 dB")
    );
}

#[test]
fn a_format_without_a_writer_is_refused_and_left_untouched() {
    let scratch = Scratch::new("gain-mp3");
    let path = copy_fixture(&scratch, "silence.mp3");
    let before = std::fs::read(&path).unwrap();

    assert!(write(&path, &tags()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn a_file_whose_extension_lies_is_judged_by_what_it_is() {
    // An MP3 named .flac must be refused, not written as if it were FLAC.
    let scratch = Scratch::new("gain-liar");
    let liar = scratch.join("liar.flac");
    std::fs::copy(Path::new(FIXTURES).join("silence.mp3"), &liar).unwrap();
    let before = std::fs::read(&liar).unwrap();

    assert!(write(&liar, &tags()).is_err());
    assert_eq!(std::fs::read(&liar).unwrap(), before);
}

// ---------------------------------------------------------------- measuring --

/// A 16-bit stereo WAV, written by hand so no fixture binary is needed.
fn write_wav(path: &Path, samples: &[f32], rate: u32) {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
        .collect();
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&2_u16.to_le_bytes()); // stereo
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4).to_le_bytes());
    wav.extend_from_slice(&4_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    std::fs::write(path, wav).unwrap();
}

#[test]
fn a_decoded_file_measures_the_same_as_its_samples() {
    let scratch = Scratch::new("gain-wav");
    let rate = 44_100;
    let tone: Vec<f32> = (0..rate * 5)
        .flat_map(|n| {
            let v = (0.3
                * (2.0 * std::f64::consts::PI * 440.0 * f64::from(n) / f64::from(rate)).sin())
                as f32;
            [v, v * 0.5]
        })
        .collect();
    let path = scratch.join("tone.wav");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_wav(&path, &tone, rate);

    let decoded = measure(&path).unwrap();
    let direct = measure_samples(&tone, 2, rate).unwrap();
    assert!(
        (decoded.loudness() - direct.loudness()).abs() < 0.01,
        "{} vs {}",
        decoded.loudness(),
        direct.loudness()
    );
    assert!((decoded.peak - direct.peak).abs() < 1e-4);
}

// ---------------------------------------------------------------------- job --

use lofty::prelude::ItemKey;
use scoria_core::config::Paths;
use scoria_core::event::Event;
use scoria_core::library::gain::GainReport;
use scoria_core::library::gain::job::{self, Shared};
use scoria_core::library::gain::plan::AlbumJob;
use scoria_core::session::Session;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Home {
    scratch: Scratch,
}

impl Home {
    fn new(name: &str) -> Self {
        Self {
            scratch: Scratch::new(name),
        }
    }

    fn root(&self) -> PathBuf {
        self.scratch.root()
    }

    fn session(&self) -> Session {
        let mut session = Session::open(Paths::rooted_at(self.scratch.path())).unwrap();
        session.config_mut().root = Some(self.root());
        session.rescan(&mut |_| {}).unwrap();
        session
    }

    fn write(&self, relative: &str, album: &str) -> PathBuf {
        self.scratch.write_track(
            &self.root(),
            relative,
            &[
                (ItemKey::TrackTitle, relative),
                (ItemKey::AlbumArtist, "Someone"),
                (ItemKey::AlbumTitle, album),
            ],
        )
    }
}

fn track_gain(path: &Path) -> Option<String> {
    read_flac(path)
        .vorbis_comments()?
        .get("REPLAYGAIN_TRACK_GAIN")
        .map(str::to_owned)
}

/// Waits for an analysis to finish, as a frontend does.
fn wait_for_report(session: &mut Session) -> GainReport {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if let Some(Event::GainFinished(report)) = session.next_event(Duration::from_millis(100)) {
            return report;
        }
    }
    panic!("the analysis never said it had finished");
}

#[test]
fn an_analysis_tags_every_gap_and_leaves_none_behind() {
    let home = Home::new("gain-job");
    let a = home.write("a/1.flac", "A");
    let b = home.write("a/2.flac", "A");
    let c = home.write("b/1.flac", "B");
    let mut session = home.session();

    let plan = session.gain_plan(false).unwrap();
    assert_eq!(
        (plan.albums.len(), plan.files(), plan.tracks_levelled),
        (2, 3, 0)
    );

    assert_eq!(session.start_gain_analysis(false).unwrap(), 2);
    assert!(session.gain_running());
    let report = wait_for_report(&mut session);
    assert!(!session.gain_running());
    assert_eq!((report.albums_done, report.albums_total), (2, 2));
    assert!(!report.stopped);
    assert!(report.failed.is_empty(), "{:?}", report.failed);

    // The fixture is silence, which rsgain leaves at unity too.
    for path in [&a, &b, &c] {
        assert_eq!(track_gain(path).as_deref(), Some("0.00 dB"));
    }

    // Once the tags are read back, there is nothing left to do.
    session.rescan(&mut |_| {}).unwrap();
    let after = session.gain_plan(false).unwrap();
    assert!(after.albums.is_empty());
    assert_eq!(after.tracks_levelled, 3);
}

#[test]
fn an_album_with_a_missing_file_is_left_untouched() {
    let home = Home::new("gain-missing");
    let kept = home.write("a/1.flac", "A");
    let gone = home.write("a/2.flac", "A");
    let mut session = home.session();
    std::fs::remove_file(&gone).unwrap();
    session.rescan(&mut |_| {}).unwrap();

    session.start_gain_analysis(false).unwrap();
    let report = wait_for_report(&mut session);
    assert_eq!(report.skipped.missing_files, 1);
    assert_eq!(report.albums_total, 0);
    assert_eq!(track_gain(&kept), None);
}

#[test]
fn a_second_analysis_is_refused_while_one_runs() {
    let home = Home::new("gain-busy");
    for n in 0..20 {
        home.write(&format!("a{n}/1.flac"), &format!("Album {n}"));
    }
    let mut session = home.session();
    session.start_gain_analysis(false).unwrap();
    // Either it is still running and refuses, or it already finished --
    // never two at once.
    if session.gain_running() {
        assert!(session.start_gain_analysis(false).is_err());
    }
    wait_for_report(&mut session);
}

#[test]
fn an_unreachable_library_is_refused() {
    let home = Home::new("gain-unplugged");
    home.write("a/1.flac", "A");
    let mut session = home.session();
    std::fs::remove_dir_all(home.root()).unwrap();
    assert!(matches!(
        session.start_gain_analysis(false),
        Err(scoria_core::CoreError::LibraryUnreachable { .. })
    ));
    assert!(!session.gain_running());
}

fn shared() -> Shared {
    Shared {
        cancel: Arc::new(AtomicBool::new(false)),
        protected: Arc::new(Mutex::new(HashSet::new())),
        held: Arc::new(Mutex::new(Vec::new())),
        scan_running: Arc::new(AtomicBool::new(false)),
    }
}

fn album(tracks: &[&Path]) -> AlbumJob {
    AlbumJob {
        album_artist: "Someone".to_owned(),
        title: "A".to_owned(),
        tracks: tracks.iter().map(|p| p.to_path_buf()).collect(),
    }
}

fn report_of(events: &std::sync::mpsc::Receiver<Event>) -> GainReport {
    events
        .try_iter()
        .find_map(|e| match e {
            Event::GainFinished(report) => Some(report),
            _ => None,
        })
        .unwrap()
}

#[test]
fn stopping_writes_nothing_more_and_says_it_stopped() {
    let home = Home::new("gain-stop");
    let path = home.write("a/1.flac", "A");
    let shared = shared();
    shared
        .cancel
        .store(true, std::sync::atomic::Ordering::Release);
    let (tx, rx) = std::sync::mpsc::channel();

    job::run(&[album(&[&path])], Default::default(), &shared, &tx);
    let report = report_of(&rx);
    assert!(report.stopped);
    assert_eq!(report.albums_done, 0);
    assert_eq!(track_gain(&path), None);
}

#[test]
fn the_playing_file_is_held_and_written_once_playback_moves_on() {
    let home = Home::new("gain-held");
    let playing = home.write("a/1.flac", "A");
    let other = home.write("a/2.flac", "A");
    let shared = shared();
    shared.protected.lock().unwrap().insert(playing.clone());
    let (tx, rx) = std::sync::mpsc::channel();

    job::run(
        &[album(&[&playing, &other])],
        Default::default(),
        &shared,
        &tx,
    );
    assert!(report_of(&rx).failed.is_empty());
    assert_eq!(track_gain(&playing), None, "not while it is playing");
    assert_eq!(track_gain(&other).as_deref(), Some("0.00 dB"));
    assert_eq!(shared.held.lock().unwrap().len(), 1);

    // Still playing: still held.
    assert!(job::flush_held(&shared.protected, &shared.held).is_empty());
    assert_eq!(track_gain(&playing), None);

    shared.protected.lock().unwrap().clear();
    assert!(job::flush_held(&shared.protected, &shared.held).is_empty());
    assert_eq!(track_gain(&playing).as_deref(), Some("0.00 dB"));
    assert!(shared.held.lock().unwrap().is_empty());
}

#[test]
fn a_file_that_will_not_decode_leaves_its_whole_album_unwritten() {
    let home = Home::new("gain-broken");
    let good = home.write("a/1.flac", "A");
    let broken = home.root().join("a/2.flac");
    std::fs::write(&broken, b"not audio at all").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();

    job::run(
        &[album(&[&good, &broken])],
        Default::default(),
        &shared(),
        &tx,
    );
    let report = report_of(&rx);
    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].0.ends_with("2.flac"));
    assert_eq!(
        track_gain(&good),
        None,
        "album gain without track 2 would be wrong"
    );
}

#[test]
fn with_watching_on_the_new_tags_arrive_by_themselves_afterwards() {
    // Scans are held while the analysis writes, then one runs and brings the
    // tags into the index -- the frontend never has to ask.
    let home = Home::new("gain-watched");
    home.write("a/1.flac", "A");
    home.write("a/2.flac", "A");
    let mut session = home.session();
    session.start_watching().unwrap();
    // The catch-up scan watching always starts with.
    let settle = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < settle {
        if let Some(Event::ScanFinished) = session.next_event(Duration::from_millis(100)) {
            break;
        }
    }

    session.start_gain_analysis(false).unwrap();
    // A scan already under way when the analysis started is allowed to
    // finish, and says everything it has to before the first album is
    // written. Between the first album and the end, nothing may scan.
    let mut writing = false;
    let mut finished = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        match session.next_event(Duration::from_millis(100)) {
            Some(Event::GainProgress { .. }) => writing = true,
            Some(Event::GainFinished(_)) => finished = true,
            Some(Event::ScanProgress { .. }) => {
                assert!(
                    !writing || finished,
                    "a scan ran while the analysis was writing"
                );
            }
            Some(Event::LibraryChanged(_)) if finished => {
                session.reload_index().unwrap();
                assert!(
                    session
                        .index()
                        .tracks()
                        .iter()
                        .all(|t| t.replay_gain.track_gain_db.is_some())
                );
                return;
            }
            _ => {}
        }
    }
    panic!("the new tags never reached the index");
}
