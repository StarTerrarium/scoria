//! Writing to stdout without panicking.
//!
//! `println!` panics if the pipe it is writing to has closed, which is what
//! happens the moment anyone runs `scoria tracks | head`. Rust sets `SIGPIPE`
//! to be ignored at startup, so the write returns `EPIPE` and the macro turns
//! that into a panic.
//!
//! `CLAUDE.md` requires no known way to panic at runtime, and this is a known
//! way, found by piping the output into `head` while testing. Everything here
//! returns an error instead, and the entry point treats a closed pipe as an
//! ordinary end rather than a failure -- because that is what it is: the reader
//! got what it wanted and left.

use std::io::{self, IsTerminal, StdoutLock, Write};

/// Buffered stdout.
///
/// Buffered because a track listing is thousands of small writes, and an
/// unbuffered one syscalls for each.
pub struct Out {
    inner: io::BufWriter<StdoutLock<'static>>,
    interactive: bool,
}

impl Out {
    #[must_use]
    pub fn new() -> Self {
        let stdout = io::stdout();
        let interactive = stdout.is_terminal();
        Self {
            inner: io::BufWriter::new(stdout.lock()),
            interactive,
        }
    }

    /// Whether output is going to a terminal, so progress rewriting and other
    /// cursor tricks are worth doing.
    #[must_use]
    pub const fn interactive(&self) -> bool {
        self.interactive
    }
}

impl Write for Out {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        // A closed pipe here is the normal end of `... | head`, not a failure.
        let _ = self.inner.flush();
    }
}

/// Whether an error is a reader that hung up.
#[must_use]
pub fn is_closed_pipe(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(|e| e.kind() == io::ErrorKind::BrokenPipe)
}
