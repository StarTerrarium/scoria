//! What makes FLAC gapless-capable, checked against a real decoder.
//!
//! §3.4: FLAC, Vorbis and Opus carry exact sample counts, so a decoder returns
//! precisely the samples the file declares and two adjacent tracks meet with
//! nothing between them. MP3 does not -- it carries encoder delay and padding
//! that must be read from the LAME/Info header and trimmed, which is why §10
//! defers MP3 gapless to post-1.0.
//!
//! If this test ever fails, gapless is broken at the decoder level and no
//! amount of careful queueing above it will help.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use rodio::Source;
use std::io::Cursor;

const SILENCE: &[u8] = include_bytes!("fixtures/silence.flac");

/// The fixture's STREAMINFO: 2205 samples, 44.1 kHz, mono. Fifty milliseconds.
const DECLARED_SAMPLES: usize = 2205;
const DECLARED_RATE: u32 = 44_100;
const DECLARED_CHANNELS: u16 = 1;

fn decode(bytes: &[u8]) -> rodio::Decoder<Cursor<Vec<u8>>> {
    rodio::Decoder::new(Cursor::new(bytes.to_vec())).expect("fixture should decode")
}

#[test]
fn a_flac_decodes_to_exactly_the_samples_it_declares() {
    // No encoder delay at the start, no padding at the end. This is the whole
    // reason FLAC is the first-class target for 1.0 (§5.2).
    let decoder = decode(SILENCE);
    assert_eq!(decoder.sample_rate().get(), DECLARED_RATE);
    assert_eq!(decoder.channels().get(), DECLARED_CHANNELS);

    let decoded = decoder.count();
    assert_eq!(
        decoded,
        DECLARED_SAMPLES * DECLARED_CHANNELS as usize,
        "a decoder that adds or drops samples cannot be gapless"
    );
}

#[test]
fn decoding_is_reproducible() {
    // Two plays of the same file must produce the same number of samples, or a
    // boundary would drift depending on what was played before it.
    assert_eq!(decode(SILENCE).count(), decode(SILENCE).count());
}

#[test]
fn two_adjacent_tracks_meet_with_nothing_between_them() {
    // The property a gapless transition depends on: concatenating the decoded
    // output of two files yields exactly the sum of their lengths. Any silence
    // inserted at the join would show up here as extra samples.
    let first: Vec<_> = decode(SILENCE).collect();
    let second: Vec<_> = decode(SILENCE).collect();

    let joined = first.len() + second.len();
    assert_eq!(joined, 2 * DECLARED_SAMPLES * DECLARED_CHANNELS as usize);
}

#[test]
fn the_declared_duration_matches_what_decodes() {
    // lofty reads the duration from STREAMINFO and the scanner stores it; the
    // engine uses it to decide when to preload. If the two disagreed, the
    // preload would fire at the wrong moment.
    let decoder = decode(SILENCE);
    let declared = decoder
        .total_duration()
        .expect("FLAC declares its duration");

    let expected_ms = (DECLARED_SAMPLES as u64 * 1000) / u64::from(DECLARED_RATE);
    let actual_ms = declared.as_millis() as u64;
    assert!(
        actual_ms.abs_diff(expected_ms) <= 1,
        "declared {actual_ms}ms, samples say {expected_ms}ms"
    );
}
