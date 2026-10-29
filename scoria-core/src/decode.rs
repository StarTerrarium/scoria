//! Opening a file for decoding.
//!
//! Shared by playback and by loudness analysis (§6.7), so that what is measured
//! is exactly what is heard: the same gapless trimming, the same format
//! detection, the same seekability. Two copies of this setup would drift, and
//! the first sign would be gain tags that are subtly wrong for one format.

use crate::error::CoreError;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// A decoder over a file on disk.
pub type FileDecoder = rodio::Decoder<BufReader<File>>;

/// Opens `path` for decoding.
///
/// # Errors
///
/// [`CoreError::Audio`] if the file cannot be opened or its format is not one
/// the decoder understands.
pub fn open(path: &Path) -> Result<FileDecoder, CoreError> {
    let file = File::open(path)
        .map_err(|e| CoreError::Audio(format!("cannot open {}: {e}", path.display())))?;

    // The byte length is what makes the stream seekable. `Decoder::new`
    // declares neither, so Symphonia treats the input as a stream it cannot
    // rewind: seeking forward still works, because that is reading ahead, but
    // every backward seek fails. The symptom is a seek bar that only moves one
    // way.
    let byte_len = file.metadata().ok().map(|m| m.len());

    let mut builder = rodio::Decoder::builder()
        .with_data(BufReader::new(file))
        .with_seekable(true)
        // Trims encoder delay and padding where the format records them. Free
        // for FLAC, which has neither, and the beginning of what §10 defers for
        // MP3.
        .with_gapless(true);
    if let Some(len) = byte_len {
        builder = builder.with_byte_len(len);
    }
    if let Some(hint) = path.extension().and_then(|e| e.to_str()) {
        builder = builder.with_hint(hint);
    }

    builder
        .build()
        .map_err(|e| CoreError::Audio(format!("cannot decode {}: {e}", path.display())))
}
