//! Scanning, tag reading, artwork and album identity.

pub mod artwork;
pub mod gain;
pub mod identity;
pub mod scanner;
pub mod tags;
pub mod watcher;

pub use artwork::{Artwork, ArtworkCache};
pub use identity::AlbumIdentity;
pub use scanner::{ScanOutcome, ScanProgress, Scanner};
pub use tags::TrackTags;
pub use watcher::LibraryWatcher;
