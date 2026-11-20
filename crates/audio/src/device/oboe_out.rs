//! Android's output through Oboe (Google's low-latency audio library, as FL Studio Mobile's `Native` sound mode uses):
//! AAudio on Android 8.1 and up, OpenSL ES before. Each [`BufferMode`] is one of FL Studio Mobile's sound modes:
//!
//! | Mode                   | Performance | Sharing   | Buffer                       | Grows on underruns |
//! |------------------------|-------------|-----------|------------------------------|--------------------|
//! | `Native` + low latency | LowLatency  | Exclusive | 1 burst                      | yes                |
//! | `Native`               | LowLatency  | Exclusive | 2 bursts                     | yes                |
//! | `Fast`                 | LowLatency  | Shared    | 2 bursts                     | yes                |
//! | `Safe`                 | LowLatency  | Shared    | 4 bursts, at least 1024      | yes                |
//! | `UltraSafe`            | None        | Shared    | the whole capacity           | no                 |
//! | `Fixed(n)`             | LowLatency  | Shared    | n frames, whole bursts       | no                 |
//! | `AtLeast(mode, n)`     | the mode's  | the mode's| the mode's, at least n frames| the mode's         |
//!
//! `AtLeast` is the `Audio buffer size` slider on top of the sound mode: its stream, with a buffer no smaller.
//!
//! A buffer that grows on underruns does as Oboe's own `LatencyTuner`: a burst more after each callback that saw new
//! ones, up to the capacity, so a phone that cannot keep up settles at the smallest buffer it plays without crackles.
//! A device that refuses exclusive mode opens shared (Oboe falls back by itself).
//!
//! The stream runs at the device's own rate for the `Device` sample rate (most phones 48 kHz, the only rate their
//! fast path takes), else at the rate asked for, which Oboe's own resampler converts to the device's so the stream
//! keeps its low-latency path (Android's would take it off it).
//!
//! The stream opens and starts before its renderer exists, whose rate is the stream's: it plays silence until
//! [`Opened::deliver`] hands it the callback state, a moment later.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use oboe::{
    AudioApi, AudioOutputCallback, AudioOutputStreamSafe, AudioStream, AudioStreamAsync, AudioStreamBase,
    AudioStreamBuilder, AudioStreamSafe, ContentType, DataCallbackResult, Error, Output,
    PerformanceMode, SampleRateConversionQuality, SharingMode, Stereo, Usage,
};

use super::{AudioRenderer, BufferMode, CallbackState, SUB_BLOCK, clock};

/// How a [`BufferMode`] opens its stream (module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Profile {
    performance: PerformanceMode,
    sharing: SharingMode,
    usage: Usage,
    /// The buffer in bursts; 0 for the whole capacity.
    bursts: i32,
    /// At least this many frames.
    min_frames: i32,
    /// The buffer grows a burst after each callback that saw new underruns.
    adaptive: bool,
}

fn profile(mode: BufferMode) -> Profile {
    let fast = Profile {
        performance: PerformanceMode::LowLatency,
        sharing: SharingMode::Shared,
        usage: Usage::Media,
        bursts: 2,
        min_frames: 0,
        adaptive: true,
    };
    match mode {
        BufferMode::Lowest => Profile {
            sharing: SharingMode::Exclusive,
            usage: Usage::Game,
            bursts: 1,
            ..fast
        },
        BufferMode::Native => Profile {
            sharing: SharingMode::Exclusive,
            usage: Usage::Game,
            ..fast
        },
        BufferMode::Fast => fast,
        BufferMode::Safe => Profile {
            bursts: 4,
            min_frames: 1024,
            ..fast
        },
        BufferMode::UltraSafe => Profile {
            performance: PerformanceMode::None,
            bursts: 0,
            adaptive: false,
            ..fast
        },
        BufferMode::AtLeast(base, frames) => {
            let p = profile(base.mode());
            Profile {
                min_frames: p.min_frames.max(i32::try_from(frames).unwrap_or(i32::MAX)),
                ..p
            }
        }
        BufferMode::Fixed(frames) => Profile {
            bursts: 1,
            min_frames: i32::try_from(frames).unwrap_or(i32::MAX),
            adaptive: false,
            ..fast
        },
    }
}

/// The frames of a buffer of `p` for a stream of `burst` frames a burst and `capacity` frames of room: whole bursts.
fn buffer_frames(p: Profile, burst: i32, capacity: i32) -> i32 {
    let burst = burst.max(16);
    if p.bursts == 0 {
        return capacity.max(burst);
    }
    let want = (burst * p.bursts).max(p.min_frames);
    let whole = (want + burst - 1) / burst * burst;
    whole.min(capacity.max(burst))
}

/// Where the callback state waits for the callback to take it, once, with the stream's rate it renders at.
type Inbox<R> = Arc<Mutex<Option<(CallbackState<R>, u32)>>>;

pub(super) struct Callback<R: AudioRenderer> {
    state: Option<CallbackState<R>>,
    inbox: Inbox<R>,
    scratch: Vec<f32>,
    sample_rate: u32,
    adaptive: bool,
    /// The stream's underruns so far, as last read.
    xruns: i32,
    /// The latency Oboe last worked out, in ns, and the callbacks until it is asked again.
    latency_ns: i64,
    until_latency: u32,
}

/// Callbacks between two latency readings (each reads the stream's timestamp).
const LATENCY_EVERY: u32 = 8;

impl<R: AudioRenderer> AudioOutputCallback for Callback<R> {
    type FrameType = (f32, Stereo);

    fn on_audio_ready(
        &mut self,
        stream: &mut dyn AudioOutputStreamSafe,
        frames: &mut [(f32, f32)],
    ) -> DataCallbackResult {
        if self.state.is_none() {
            // Never blocks: the opener holds the lock only to put the state in.
            if let Ok(mut inbox) = self.inbox.try_lock()
                && let Some((state, rate)) = inbox.take()
            {
                (self.state, self.sample_rate) = (Some(state), rate);
            }
        }
        let Some(state) = self.state.as_mut() else {
            frames.fill((0.0, 0.0));
            return DataCallbackResult::Continue;
        };
        let n = frames.len();
        let _ = (state.shared.buffer_frames).compare_exchange(
            0,
            u32::try_from(n).unwrap_or(u32::MAX),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if self.until_latency == 0 {
            self.until_latency = LATENCY_EVERY;
            if let Ok(ms) = stream.calculate_latency_millis()
                && ms.is_finite()
                && (0.0..1000.0).contains(&ms)
            {
                self.latency_ns = (ms * 1e6) as i64;
            }
        }
        self.until_latency -= 1;
        // When its first frame is heard: now and the latency Oboe works out from the stream's timestamps.
        let heard = clock().map(|now| now + self.latency_ns);
        let rate = self.sample_rate;
        state.callback(n, 2, rate, heard, &mut self.scratch, |at, rendered| {
            for (out, pair) in frames[at..].iter_mut().zip(rendered.chunks_exact(2)) {
                let clean = |s: f32| {
                    if s.is_finite() {
                        s.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    }
                };
                *out = (clean(pair[0]), clean(pair[1]));
            }
        });
        if let Ok(count) = stream.get_xrun_count()
            && count > self.xruns
        {
            let new = count - self.xruns;
            self.xruns = count;
            (state.shared.underruns).fetch_add(u64::try_from(new).unwrap_or(0), Ordering::Release);
            if self.adaptive {
                let burst = stream.get_frames_per_burst().max(16);
                let (size, room) = (
                    stream.get_buffer_size_in_frames(),
                    stream.get_buffer_capacity_in_frames(),
                );
                if size < room {
                    let _ = stream.set_buffer_size_in_frames((size + burst).min(room));
                }
            }
        }
        DataCallbackResult::Continue
    }

    /// A disconnect (headphones unplugged, a route change) or another error closed the stream: the app reopens it.
    fn on_error_after_close(&mut self, _stream: &mut dyn AudioOutputStreamSafe, _error: Error) {
        if self.state.is_none()
            && let Ok(mut inbox) = self.inbox.lock()
        {
            self.state = inbox.take().map(|(state, _)| state);
        }
        if let Some(state) = &self.state {
            state.shared.failed.store(true, Ordering::Release);
        }
    }
}

/// A started stream whose callback plays silence until it has its state.
pub(super) struct Opened<R: AudioRenderer> {
    pub(super) stream: AudioStreamAsync<Output, Callback<R>>,
    inbox: Inbox<R>,
    pub(super) sample_rate: u32,
}

impl<R: AudioRenderer> Opened<R> {
    /// Hands the callback its state: it plays from its next callback on.
    pub(super) fn deliver(self, state: CallbackState<R>) -> AudioStreamAsync<Output, Callback<R>> {
        if let Ok(mut inbox) = self.inbox.lock() {
            *inbox = Some((state, self.sample_rate));
        }
        self.stream
    }
}

/// Opens and starts a stereo float stream for `mode` at `rate` Hz (0: the device's own rate), through `api` (OpenSL
/// ES, or Oboe's choice: AAudio from Android 8.1).
pub(super) fn open<R: AudioRenderer>(
    mode: BufferMode,
    rate: u32,
    api: AudioApi,
) -> Result<Opened<R>, Error> {
    let p = profile(mode);
    let inbox: Inbox<R> = Arc::new(Mutex::new(None));
    let mut builder = AudioStreamBuilder::default()
        .set_output()
        .set_audio_api(api)
        .set_f32()
        .set_stereo()
        .set_performance_mode(p.performance)
        .set_sharing_mode(p.sharing)
        .set_usage(p.usage)
        .set_content_type(ContentType::Music)
        .set_format_conversion_allowed(true)
        .set_channel_conversion_allowed(true);
    if let Ok(rate) = i32::try_from(rate)
        && rate > 0
    {
        builder = builder
            .set_sample_rate(rate)
            .set_sample_rate_conversion_quality(SampleRateConversionQuality::Medium);
    }
    if let BufferMode::Fixed(frames) | BufferMode::AtLeast(_, frames) = mode {
        let room = i32::try_from(frames.saturating_mul(2)).unwrap_or(i32::MAX);
        builder = builder.set_buffer_capacity_in_frames(room);
    }
    let callback = Callback {
        state: None,
        inbox: inbox.clone(),
        scratch: vec![0.0; SUB_BLOCK * 2],
        sample_rate: 0,
        adaptive: p.adaptive,
        xruns: 0,
        latency_ns: 0,
        until_latency: 0,
    };
    // The callback's rate is the stream's, known once it is open: it comes with the state ([`Opened::deliver`]).
    let mut stream = builder.set_callback(callback).open_stream()?;
    let sample_rate = u32::try_from(stream.get_sample_rate()).unwrap_or(0);
    if sample_rate == 0 {
        return Err(Error::InvalidRate);
    }
    let burst = stream.get_frames_per_burst();
    let capacity = stream.get_buffer_capacity_in_frames();
    let _ = stream.set_buffer_size_in_frames(buffer_frames(p, burst, capacity));
    stream.request_start()?;
    Ok(Opened {
        stream,
        inbox,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffers_are_whole_bursts_within_the_capacity() {
        assert_eq!(buffer_frames(profile(BufferMode::Native), 192, 3072), 384);
        assert_eq!(buffer_frames(profile(BufferMode::Lowest), 192, 3072), 192);
        assert_eq!(buffer_frames(profile(BufferMode::Safe), 192, 3072), 1152);
        assert_eq!(
            buffer_frames(profile(BufferMode::UltraSafe), 192, 3072),
            3072
        );
        assert_eq!(
            buffer_frames(profile(BufferMode::Fixed(500)), 192, 3072),
            576
        );
        assert_eq!(
            buffer_frames(profile(BufferMode::Fixed(9000)), 192, 3072),
            3072
        );
        // The slider on top of a mode: no smaller than either, the mode's stream kept.
        use crate::device::SoundMode;
        let at_least = |base, n| profile(BufferMode::AtLeast(base, n));
        assert_eq!(
            buffer_frames(at_least(SoundMode::Native, 512), 192, 3072),
            576
        );
        assert_eq!(
            buffer_frames(at_least(SoundMode::Safe, 256), 192, 3072),
            1152
        );
        assert_eq!(
            at_least(SoundMode::Lowest, 64).sharing,
            SharingMode::Exclusive
        );
    }
}
