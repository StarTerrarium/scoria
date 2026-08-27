//! Storage.
//!
//! Access goes through the traits in [`repository`]; [`sqlite`] is the only
//! module in the crate that contains SQL.

pub mod repository;
pub mod schema;
pub mod sqlite;

pub use repository::{AlbumRepository, NewAlbum, NewTrack, TrackRepository};
pub use sqlite::SqliteDatabase;
