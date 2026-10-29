//! Loudness analysis against real files (§6.7).
//!
//! The writer is the only code in Scoria that changes the library, so what has
//! to hold is that it changes exactly four comments and nothing else -- on a
//! file tagged the way Picard tags one, with the multi-valued fields and the
//! cover art that a careless rewrite would lose.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

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
