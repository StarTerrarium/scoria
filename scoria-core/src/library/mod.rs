//! Scanning, tag reading and album identity.
//!
//! The scanner and tag reader arrive at M3. Album identity is here now because
//! storage keys albums on it.

pub mod identity;

pub use identity::AlbumIdentity;
