//! Library index, scanner, query engine, playback engine and queue.
//!
//! This crate never depends on a UI crate. The frontend talks to it by sending
//! `Command` and consuming `Event`, both serialisable; it never calls in
//! here directly. See `DESIGN.md` §3.2 for why the seam is shaped that way, and
//! `scripts/check-boundaries.sh` for what enforces it.
