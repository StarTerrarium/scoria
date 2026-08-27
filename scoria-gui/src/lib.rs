//! iced frontend.
//!
//! Sends `scoria_core::Command` and renders `scoria_core::Event`. A view
//! function that reaches into a library, database or playback object has broken
//! the seam -- see `CLAUDE.md` invariant 2.
//!
//! iced 0.14 specifically. The 0.12 and 0.13 APIs are not compatible and are
//! well represented in training data; check docs.rs/iced/0.14 rather than
//! writing from memory.
