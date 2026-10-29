//! Loudness analysis: computing ReplayGain and writing it into the files.
//!
//! `DESIGN.md` §6.7. Only when asked, always confirmed, and with the same
//! values `rsgain easy` writes, so tags from either tool sit at the same level.
//!
//! - [`measure`] decodes a file and measures it.
//! - [`compute`] turns measurements into gains, by rsgain's rules.
//! - [`write`] puts them into the files, and is the only code in Scoria that
//!   writes to the library.
//! - [`plan`] chooses which albums need it, and [`job`] runs the analysis.

pub mod compute;
pub mod job;
pub mod measure;
pub mod plan;
pub mod write;

pub use compute::{AlbumGains, Gain};
pub use job::GainReport;
pub use measure::Measurement;
pub use plan::{GainPlan, Skipped};
