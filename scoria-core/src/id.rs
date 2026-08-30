//! Identifiers.
//!
//! All three are opaque newtypes rather than bare integers so that a track id
//! cannot be passed where an album id belongs. They are `Copy` and cheap; a
//! snapshot of 100,000 of them is 800 KB (`DESIGN.md` §6.2).

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);

        impl $name {
            /// The underlying integer, for storage and display only.
            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type! {
    /// Identifies one track. Also the terminal tiebreaker of every sort chain,
    /// which is what makes equal-key rows stable across runs (`DESIGN.md` §4.4).
    TrackId
}

id_type! {
    /// Identifies one album, resolved by the ladder in `DESIGN.md` §4.2.
    AlbumId
}
