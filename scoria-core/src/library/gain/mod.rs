//! Loudness analysis: computing ReplayGain and writing it into the files.
//!
//! `DESIGN.md` §6.7. Only when asked, always confirmed, and with the same
//! values `rsgain easy` writes, so tags from either tool sit at the same level.
//!
//! - [`measure`] decodes a file and measures it.
//! - [`compute`] turns measurements into gains, by rsgain's rules.
//!
//! Nothing in here writes yet.

pub mod compute;
pub mod measure;

pub use compute::{AlbumGains, Gain};
pub use measure::Measurement;
