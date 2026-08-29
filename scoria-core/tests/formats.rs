//! Every format the scanner indexes can be decoded.
//!
//! §5.2 makes FLAC the first-class target and broad format support a
//! requirement before release. The trap in "broad" is a library that lists
//! tracks it cannot play: the scanner decides what to index from a file's
//! extension, and nothing else checks that a decoder exists behind it.
//!
//! So this test walks `AUDIO_EXTENSIONS` itself. A new extension with no
//! fixture fails, and a fixture that stops decoding fails -- which is what
//! turns the feature list in `Cargo.toml` into something the build enforces
//! rather than something someone remembered.
//!
//! The fixtures are 50ms of silence, a few hundred bytes each, because a public
//! repository should not carry a pile of audio.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rodio::Source;
use scoria_core::library::tags::AUDIO_EXTENSIONS;
use std::io::Cursor;

/// One fixture per extension the scanner accepts.
const FIXTURES: &[(&str, &[u8])] = &[
    ("flac", include_bytes!("fixtures/silence.flac")),
    ("mp3", include_bytes!("fixtures/silence.mp3")),
    ("ogg", include_bytes!("fixtures/silence.ogg")),
    ("oga", include_bytes!("fixtures/silence.ogg")),
    ("m4a", include_bytes!("fixtures/silence.m4a")),
    ("m4b", include_bytes!("fixtures/silence.m4a")),
    ("aac", include_bytes!("fixtures/silence.aac")),
    ("wav", include_bytes!("fixtures/silence.wav")),
    ("wave", include_bytes!("fixtures/silence.wav")),
    ("aiff", include_bytes!("fixtures/silence.aiff")),
    ("aif", include_bytes!("fixtures/silence.aiff")),
];

/// Apple Lossless, which shares the `.m4a` extension with AAC.
///
/// Separate because the extension cannot tell them apart: the same name is two
/// codecs, and only one of them was decodable before `symphonia-alac`.
const ALAC: &[u8] = include_bytes!("fixtures/silence-alac.m4a");

fn decode(bytes: &[u8], hint: &str) -> Result<rodio::Decoder<Cursor<Vec<u8>>>, String> {
    let len = bytes.len() as u64;
    rodio::Decoder::builder()
        .with_data(Cursor::new(bytes.to_vec()))
        // What the backend does, and what makes seeking work in both
        // directions -- see `playback::backend::rodio`.
        .with_seekable(true)
        .with_byte_len(len)
        .with_hint(hint)
        .build()
        .map_err(|e| format!("{e}"))
}

#[test]
fn every_indexed_extension_has_a_decoder_behind_it() {
    for extension in AUDIO_EXTENSIONS {
        let (_, bytes) = FIXTURES
            .iter()
            .find(|(name, _)| name == extension)
            .unwrap_or_else(|| {
                panic!(
                    "the scanner indexes `.{extension}` but there is no fixture proving it \
                     decodes. Either add one, or stop indexing it."
                )
            });

        let decoder = decode(bytes, extension)
            .unwrap_or_else(|e| panic!("the scanner indexes `.{extension}` but it fails: {e}"));
        let rate = decoder.sample_rate().get();
        let samples = decoder.count();
        assert!(
            samples > 0,
            "`.{extension}` decoded to nothing, which plays as silence rather than as music"
        );
        assert!(rate > 0, "`.{extension}` reported no sample rate");
    }
}

#[test]
fn no_fixture_is_kept_for_a_format_that_is_no_longer_indexed() {
    // The other direction: a fixture with nothing pointing at it is dead
    // weight in a repository that deliberately carries almost no audio.
    for (extension, _) in FIXTURES {
        assert!(
            AUDIO_EXTENSIONS.contains(extension),
            "`.{extension}` has a fixture but is not indexed"
        );
    }
}

#[test]
fn apple_lossless_decodes_as_well_as_aac() {
    // `.m4a` is two codecs wearing one extension. Indexing it means both have
    // to work, and ALAC needed a decoder that rodio does not enable by default.
    let decoder = decode(ALAC, "m4a").expect("apple lossless should decode");
    assert!(decoder.count() > 0);
}

#[test]
fn a_format_with_no_decoder_is_not_indexed() {
    // Symphonia 0.5 has no Opus decoder, and none for WavPack, Monkey's Audio,
    // Musepack or WMA. Listing them would fill a library with tracks that stop
    // the moment they are asked to play.
    for extension in ["opus", "wv", "ape", "mpc", "wma"] {
        assert!(
            !AUDIO_EXTENSIONS.contains(&extension),
            "`.{extension}` is indexed but nothing can decode it"
        );
    }
}

#[test]
fn a_library_of_mixed_formats_is_indexed_whole() {
    // Decoding is only half of it: the scanner reads tags with lofty, and a
    // format lofty cannot open is one the library never hears about. This is
    // the end-to-end version -- a folder with one file of every kind in it,
    // which is what a real library that grew over twenty years looks like.
    use scoria_core::db::{SqliteDatabase, TrackRepository};
    use scoria_core::library::Scanner;

    let root = std::env::temp_dir().join(format!("scoria-formats-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    for (extension, bytes) in FIXTURES {
        std::fs::write(root.join(format!("track.{extension}")), bytes).unwrap();
    }

    let mut db = SqliteDatabase::open_in_memory().unwrap();
    let outcome = Scanner::new(&root).scan(&mut db, &mut |_| {}).unwrap();
    let tracks = db.all_tracks().unwrap();

    assert_eq!(
        tracks.len(),
        FIXTURES.len(),
        "{} of {} files were indexed; the scan reported {outcome:?}",
        tracks.len(),
        FIXTURES.len()
    );
    for track in &tracks {
        assert!(
            !track.title.is_empty(),
            "{:?} was indexed with no title at all",
            track.path
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_file_whose_extension_lies_is_still_read_and_still_plays() {
    // Converters and file managers rename things. lofty picks its parser from
    // the extension, so before the content fallback a FLAC called `.wav` was
    // simply lost -- and `.oga` holding Vorbis, which is legal and common, was
    // lost the same way.
    //
    // Playback has to survive it too, or indexing these would produce exactly
    // the tracks-that-cannot-play this file exists to prevent.
    let flac = include_bytes!("fixtures/silence.flac");

    let root = std::env::temp_dir().join(format!("scoria-mislabelled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("track.wav");
    std::fs::write(&path, flac).unwrap();

    let tags = scoria_core::library::tags::read(&path).expect("a mislabelled file should be read");
    assert!(tags.duration > std::time::Duration::ZERO);

    let decoder = decode(flac, "wav").expect("a mislabelled file should decode");
    assert!(decoder.count() > 0);

    let _ = std::fs::remove_dir_all(&root);
}
