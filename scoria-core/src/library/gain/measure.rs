//! Decoding a file and measuring its loudness.

use crate::error::CoreError;
use ebur128::{EbuR128, Mode};
use std::path::Path;

/// How many frames are handed to the meter at a time. Large enough that the
/// per-call overhead disappears, small enough to stay in cache.
const CHUNK_FRAMES: usize = 4096;

/// One track's loudness measurement.
///
/// Holds the meter itself rather than just its answer, because album loudness
/// is not an average of track loudnesses: it is gated over every block of
/// every track together, and that needs each track's block history.
pub struct Measurement {
    meter: EbuR128,
    /// The highest absolute sample on any channel, as a linear amplitude.
    pub peak: f64,
}

impl Measurement {
    /// Integrated loudness in LUFS. Negative infinity for silence.
    #[must_use]
    pub fn loudness(&self) -> f64 {
        self.meter.loudness_global().unwrap_or(f64::NEG_INFINITY)
    }

    pub(crate) fn meter(&self) -> &EbuR128 {
        &self.meter
    }
}

/// Decodes `path` and measures it.
///
/// # Errors
///
/// [`CoreError::Audio`] if the file cannot be decoded, or if its channel count
/// or sample rate changes partway through -- the meter's filters are built for
/// one of each, and silently carrying on would measure nonsense.
pub fn measure(path: &Path) -> Result<Measurement, CoreError> {
    use rodio::Source;

    let mut decoder = crate::decode::open(path)?;
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    let changed = || {
        CoreError::Audio(format!(
            "{}: the channel layout or sample rate changes partway through",
            path.display()
        ))
    };

    let mut meter = new_meter(u32::from(channels.get()), rate.get(), path)?;
    let width = usize::from(channels.get());
    let mut chunk: Vec<f32> = Vec::with_capacity(CHUNK_FRAMES.saturating_mul(width));
    loop {
        if decoder.channels() != channels || decoder.sample_rate() != rate {
            return Err(changed());
        }
        chunk.clear();
        chunk.extend(decoder.by_ref().take(CHUNK_FRAMES.saturating_mul(width)));
        if chunk.is_empty() {
            break;
        }
        // A trailing partial frame is a damaged file's last few bytes, not
        // audio; the meter would reject the whole chunk for it.
        let whole = chunk.len() - chunk.len() % width;
        chunk.truncate(whole);
        meter
            .add_frames_f32(&chunk)
            .map_err(|e| CoreError::Audio(format!("{}: {e}", path.display())))?;
    }
    Ok(finish(meter))
}

/// Measures interleaved samples already in memory.
///
/// What [`measure`] does after decoding, exposed so the arithmetic can be
/// tested against known signals without a file.
///
/// # Errors
///
/// [`CoreError::Audio`] if the layout is one the meter cannot represent.
pub fn measure_samples(
    samples: &[f32],
    channels: u32,
    rate: u32,
) -> Result<Measurement, CoreError> {
    let mut meter = new_meter(channels, rate, Path::new("samples"))?;
    let width = usize::try_from(channels).unwrap_or(1).max(1);
    let whole = samples.len() - samples.len() % width;
    meter
        .add_frames_f32(samples.get(..whole).unwrap_or_default())
        .map_err(|e| CoreError::Audio(format!("cannot measure: {e}")))?;
    Ok(finish(meter))
}

fn new_meter(channels: u32, rate: u32, path: &Path) -> Result<EbuR128, CoreError> {
    // Integrated loudness and sample peak: rsgain easy's mode. Not the
    // histogram mode, which quantises and would drift from rsgain's numbers.
    EbuR128::new(channels, rate, Mode::I | Mode::SAMPLE_PEAK).map_err(|e| {
        CoreError::Audio(format!(
            "{}: cannot measure {channels} channels at {rate} Hz: {e}",
            path.display()
        ))
    })
}

fn finish(meter: EbuR128) -> Measurement {
    let peak = (0..meter.channels())
        .filter_map(|ch| meter.sample_peak(ch).ok())
        .fold(0.0_f64, f64::max);
    Measurement { meter, peak }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A stereo sine, identical on both channels.
    pub(crate) fn sine(frequency: f64, dbfs: f64, seconds: f64, rate: u32) -> Vec<f32> {
        let amplitude = 10_f64.powf(dbfs / 20.0);
        let frames = (seconds * f64::from(rate)) as usize;
        (0..frames)
            .flat_map(|n| {
                let t = n as f64 / f64::from(rate);
                let v = (amplitude * (2.0 * std::f64::consts::PI * frequency * t).sin()) as f32;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn a_reference_tone_measures_what_the_standard_says() {
        // EBU Tech 3341, case 1: a 1 kHz stereo sine at -23 dBFS reads
        // -23.0 LUFS, within 0.1.
        let m = measure_samples(&sine(1000.0, -23.0, 20.0, 48_000), 2, 48_000).unwrap();
        assert!((m.loudness() + 23.0).abs() < 0.1, "{}", m.loudness());
    }

    #[test]
    fn the_rate_does_not_change_the_answer() {
        let m = measure_samples(&sine(1000.0, -23.0, 20.0, 44_100), 2, 44_100).unwrap();
        assert!((m.loudness() + 23.0).abs() < 0.1, "{}", m.loudness());
    }

    #[test]
    fn the_peak_is_the_loudest_sample() {
        let m = measure_samples(&sine(1000.0, -6.0, 2.0, 48_000), 2, 48_000).unwrap();
        let expected = 10_f64.powf(-6.0 / 20.0);
        assert!((m.peak - expected).abs() < 1e-3, "{}", m.peak);
    }

    #[test]
    fn silence_has_no_loudness() {
        let m = measure_samples(&vec![0.0; 96_000], 2, 48_000).unwrap();
        assert!(m.loudness().is_infinite() && m.loudness() < 0.0);
        assert!(m.peak.abs() < f64::EPSILON);
    }
}
