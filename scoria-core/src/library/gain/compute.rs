//! From measurements to gains, by `rsgain easy`'s rules.
//!
//! Every constant here is one of rsgain's defaults, read from its installed
//! preset (`default.ini`) rather than remembered: ReplayGain 2.0's -18 LUFS
//! reference, clipping protection for positive gains only, a 0 dB ceiling, and
//! sample peak rather than true peak. Changing any of them would make Scoria's
//! tags disagree with rsgain's for the same file.

use super::measure::Measurement;
use ebur128::EbuR128;

/// ReplayGain 2.0's reference loudness, in LUFS.
pub const TARGET_LUFS: f64 = -18.0;

/// The loudest a positive gain may push the peak, in dBFS (rsgain's
/// `MaxPeakLevel`).
const MAX_PEAK_DB: f64 = 0.0;

/// A gain and the peak it was judged against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gain {
    pub gain_db: f64,
    /// Linear amplitude.
    pub peak: f64,
}

impl Gain {
    /// As it is written into a tag: `-6.52 dB`.
    #[must_use]
    pub fn gain_tag(&self) -> String {
        format!("{:.2} dB", self.gain_db)
    }

    /// As it is written into a tag: `0.988553`.
    #[must_use]
    pub fn peak_tag(&self) -> String {
        format!("{:.6}", self.peak)
    }
}

/// An album's gains: one per track, in the order measured, and the album's.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumGains {
    /// `None` for a track with no measurable loudness -- silence.
    pub tracks: Vec<Option<Gain>>,
    pub album: Option<Gain>,
}

/// The gain for something measured at `loudness` LUFS peaking at `peak`.
///
/// `None` for silence: §6.7 says never estimate, and the alternative is an
/// infinite number in a tag.
#[must_use]
pub fn gain(loudness: f64, peak: f64) -> Option<Gain> {
    if !loudness.is_finite() || !peak.is_finite() {
        return None;
    }
    let mut gain_db = TARGET_LUFS - loudness;

    // ClipMode=p: only a gain that raises the level can make it clip. Lowered
    // to exactly what reaches the ceiling, which can take it below zero for
    // material that already peaks above full scale.
    if gain_db > 0.0 && peak > 0.0 {
        let ceiling = 10_f64.powf(MAX_PEAK_DB / 20.0);
        if peak * 10_f64.powf(gain_db / 20.0) > ceiling {
            gain_db = 20.0 * (ceiling / peak).log10();
        }
    }
    gain_db.is_finite().then_some(Gain { gain_db, peak })
}

/// Gains for a whole album.
///
/// Album loudness is gated over every block of every track together, not
/// averaged -- that is what makes a quiet interlude on a loud record stay
/// quiet. The album peak is the loudest track's.
#[must_use]
pub fn album(measurements: &[Measurement]) -> AlbumGains {
    let tracks = measurements
        .iter()
        .map(|m| gain(m.loudness(), m.peak))
        .collect();
    let peak = measurements.iter().map(|m| m.peak).fold(0.0_f64, f64::max);
    let album = EbuR128::loudness_global_multiple(measurements.iter().map(Measurement::meter))
        .ok()
        .and_then(|loudness| gain(loudness, peak));
    AlbumGains { tracks, album }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use crate::library::gain::measure::measure_samples;

    fn sine(dbfs: f64, seconds: f64) -> Vec<f32> {
        let amplitude = 10_f64.powf(dbfs / 20.0);
        (0..(seconds * 48_000.0) as usize)
            .flat_map(|n| {
                let t = n as f64 / 48_000.0;
                let v = (amplitude * (2.0 * std::f64::consts::PI * 1000.0 * t).sin()) as f32;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn gain_brings_the_measurement_to_the_reference() {
        let g = gain(-10.0, 0.5).unwrap();
        assert!((g.gain_db + 8.0).abs() < 1e-9);
    }

    #[test]
    fn a_negative_gain_is_never_limited() {
        // It lowers the level, so it cannot clip, whatever the peak says.
        let g = gain(-10.0, 1.5).unwrap();
        assert!((g.gain_db + 8.0).abs() < 1e-9);
    }

    #[test]
    fn a_positive_gain_is_lowered_to_what_reaches_full_scale() {
        // +8 dB on a peak of 0.5 (-6 dBFS) would clip; it stops at +6.02.
        let g = gain(-26.0, 0.5).unwrap();
        assert!(
            (g.gain_db - 20.0 * 2_f64.log10()).abs() < 1e-9,
            "{}",
            g.gain_db
        );
        assert!(g.peak * 10_f64.powf(g.gain_db / 20.0) <= 1.0 + 1e-12);
    }

    #[test]
    fn a_positive_gain_that_fits_is_left_alone() {
        let g = gain(-20.0, 0.1).unwrap();
        assert!((g.gain_db - 2.0).abs() < 1e-9);
    }

    #[test]
    fn silence_gets_no_gain_rather_than_an_infinite_one() {
        assert_eq!(gain(f64::NEG_INFINITY, 0.0), None);
        assert_eq!(gain(f64::NAN, 0.5), None);
    }

    #[test]
    fn tags_are_written_the_way_rsgain_writes_them() {
        let g = Gain {
            gain_db: -6.519_7,
            peak: 0.988_553_2,
        };
        assert_eq!(g.gain_tag(), "-6.52 dB");
        assert_eq!(g.peak_tag(), "0.988553");
        assert_eq!(
            Gain {
                gain_db: 3.0,
                peak: 1.0
            }
            .gain_tag(),
            "3.00 dB"
        );
    }

    #[test]
    fn album_loudness_is_gated_over_everything_not_averaged() {
        // A loud track and a much quieter one. Averaging their loudnesses
        // would put the album halfway between; gating over every block
        // together weights it by energy, so it lands near the loud one.
        let loud = measure_samples(&sine(-10.0, 10.0), 2, 48_000).unwrap();
        let quiet = measure_samples(&sine(-30.0, 10.0), 2, 48_000).unwrap();
        let (l, q) = (loud.loudness(), quiet.loudness());
        let gains = album(&[loud, quiet]);

        let album_gain = gains.album.unwrap().gain_db;
        let album_loudness = TARGET_LUFS - album_gain;
        assert!(album_loudness > (l + q) / 2.0 + 5.0, "{album_loudness}");
        assert!(album_loudness < l + 0.1);
        assert_eq!(gains.tracks.len(), 2);
    }

    #[test]
    fn the_album_peak_is_the_loudest_tracks() {
        let a = measure_samples(&sine(-12.0, 3.0), 2, 48_000).unwrap();
        let b = measure_samples(&sine(-3.0, 3.0), 2, 48_000).unwrap();
        let peak_b = b.peak;
        let gains = album(&[a, b]);
        assert!((gains.album.unwrap().peak - peak_b).abs() < 1e-12);
    }

    #[test]
    fn a_silent_track_on_an_album_gets_nothing_and_takes_nothing() {
        let tone = measure_samples(&sine(-20.0, 5.0), 2, 48_000).unwrap();
        let silence = measure_samples(&vec![0.0; 480_000], 2, 48_000).unwrap();
        let tone_alone = gain(tone.loudness(), tone.peak).unwrap();
        let gains = album(&[tone, silence]);

        assert_eq!(gains.tracks[1], None);
        // Gating discards silent blocks, so the album is just the tone.
        assert!((gains.album.unwrap().gain_db - tone_alone.gain_db).abs() < 0.01);
    }
}
