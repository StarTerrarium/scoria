//! A source that can be dropped after it has been queued.
//!
//! rodio's `Player` has no way to un-append: `clear` skips the *current* track
//! as well, which is exactly what a preload must not do. But a queued source
//! that yields no samples is skipped instantly, so cancelling is a matter of
//! making it end early rather than removing it.
//!
//! Without this, changing shuffle in the last few seconds of a track would play
//! whatever had already been queued. That is a small window, but a wrong track
//! is a wrong track.

use rodio::source::Source;
use rodio::{ChannelCount, Sample, SampleRate};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Set to cancel the source it was handed to.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Wraps a source so it stops producing samples once cancelled.
pub struct Cancellable<S> {
    inner: S,
    cancel: Cancel,
}

impl<S> Cancellable<S> {
    pub const fn new(inner: S, cancel: Cancel) -> Self {
        Self { inner, cancel }
    }
}

impl<S: Source> Iterator for Cancellable<S> {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        // Checked per sample rather than once, so cancelling takes effect even
        // if the source has already started.
        if self.cancel.is_cancelled() {
            return None;
        }
        self.inner.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S: Source> Source for Cancellable<S> {
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    #[inline]
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    #[inline]
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    /// Forwards to the wrapped source.
    ///
    /// `Source::try_seek` defaults to reporting that seeking is unsupported, so
    /// a wrapper that does not forward it silently breaks seeking for
    /// everything underneath. That is exactly what happened the first time this
    /// wrapper was added: the transport's seek stopped working, and the failure
    /// surfaced as an error naming this type rather than as anything obviously
    /// to do with gapless.
    #[inline]
    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(position)
    }
}
