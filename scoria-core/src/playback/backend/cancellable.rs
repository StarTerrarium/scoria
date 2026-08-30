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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use rodio::buffer::SamplesBuffer;

    /// A second of mono audio at a rate that divides evenly, so a seek lands
    /// somewhere arithmetic rather than somewhere approximate.
    fn source() -> SamplesBuffer {
        let channels = ChannelCount::new(1).unwrap_or(ChannelCount::MIN);
        let rate = SampleRate::new(1000).unwrap_or(SampleRate::MIN);
        SamplesBuffer::new(channels, rate, vec![0.5_f32; 1000])
    }

    #[test]
    fn an_uncancelled_source_yields_everything_the_inner_one_does() {
        let expected = source().count();
        let wrapped = Cancellable::new(source(), Cancel::new());
        assert_eq!(wrapped.count(), expected);
    }

    #[test]
    fn a_cancelled_source_ends_rather_than_playing() {
        // This is the whole mechanism: rodio's Player cannot un-append, and
        // `clear` would skip the current track too, so retracting a preload
        // means making the queued source end early.
        let cancel = Cancel::new();
        cancel.cancel();
        let wrapped = Cancellable::new(source(), cancel);
        assert_eq!(wrapped.count(), 0);
    }

    #[test]
    fn cancelling_part_way_through_stops_it_there() {
        // Checked per sample rather than once at the start, so cancelling
        // takes effect on a source that has already begun.
        let cancel = Cancel::new();
        let mut wrapped = Cancellable::new(source(), cancel.clone());

        assert!(wrapped.next().is_some());
        assert!(wrapped.next().is_some());
        cancel.cancel();
        assert!(wrapped.next().is_none(), "it kept going after cancelling");
    }

    #[test]
    fn seeking_reaches_the_source_underneath() {
        // The regression this type caused once already. `Source::try_seek`
        // defaults to reporting that seeking is unsupported, so a wrapper that
        // does not forward it silently breaks seeking for everything below --
        // and it surfaced as an error naming this type rather than as anything
        // to do with gapless.
        let mut wrapped = Cancellable::new(source(), Cancel::new());
        assert!(
            wrapped.try_seek(Duration::from_millis(500)).is_ok(),
            "seeking stopped at the wrapper"
        );
        // Seeking half way through leaves half the samples.
        assert_eq!(wrapped.count(), 500);
    }

    #[test]
    fn the_stream_describes_itself_the_way_the_source_does() {
        // rodio mixes by these. A wrapper reporting its own answer would
        // resample or remix whatever it wrapped.
        let inner = source();
        let (channels, rate, duration) = (
            inner.channels(),
            inner.sample_rate(),
            inner.total_duration(),
        );

        let wrapped = Cancellable::new(source(), Cancel::new());
        assert_eq!(wrapped.channels(), channels);
        assert_eq!(wrapped.sample_rate(), rate);
        assert_eq!(wrapped.total_duration(), duration);
    }
}
