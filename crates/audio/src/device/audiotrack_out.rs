//! Android's output through `AudioTrack`, Android's Java audio: its oldest path, the slowest and the most compatible,
//! which FL Studio Mobile's `Ultrasafe` sound mode uses (SETTINGS' `Audio driver:`). A thread of its own renders a
//! chunk at a time and writes it to the track, blocking while the track is full: the track's buffer is the latency.
//!
//! Each [`BufferMode`] sizes the track's buffer from Android's smallest one for the rate (`getMinBufferSize`):
//!
//! | Mode                         | Buffer                                |
//! |------------------------------|---------------------------------------|
//! | `Native`, `Lowest`, `Fast`   | the smallest                          |
//! | `Safe`                       | twice the smallest, at least 1024     |
//! | `UltraSafe`                  | four times the smallest, at least 4096|
//! | `Fixed(n)`                   | the smallest, at least n              |
//! | `AtLeast(mode, n)`           | the mode's, at least n                |

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use jni::objects::{GlobalRef, JValue};
use jni::{JNIEnv, JavaVM};

use super::{AudioRenderer, BufferMode, CallbackState, SUB_BLOCK, clock};

/// `AudioManager.STREAM_MUSIC`, `AudioFormat.CHANNEL_OUT_STEREO`, `AudioFormat.ENCODING_PCM_FLOAT`,
/// `AudioTrack.MODE_STREAM`, `AudioTrack.STATE_INITIALIZED` and `AudioTrack.WRITE_BLOCKING`.
const STREAM_MUSIC: i32 = 3;
const CHANNEL_OUT_STEREO: i32 = 12;
const ENCODING_PCM_FLOAT: i32 = 4;
const MODE_STREAM: i32 = 1;
const STATE_INITIALIZED: i32 = 1;
const WRITE_BLOCKING: i32 = 0;
/// A stereo float frame.
const FRAME_BYTES: i32 = 8;
const TRACK: &str = "android/media/AudioTrack";

/// The playing track's thread, stopped (and the track released) on drop.
pub(super) struct Track {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Track {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A track open and playing silence, waiting for its renderer ([`Opened::play`]).
pub(super) struct Opened {
    vm: JavaVM,
    track: GlobalRef,
    frames: usize,
    pub(super) sample_rate: u32,
}

fn vm() -> Result<JavaVM, String> {
    let context = ndk_context::android_context();
    // SAFETY: the VM android-activity registered for the app, alive while the app runs.
    unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|e| e.to_string())
}

/// The track's buffer in frames for `mode`, from Android's smallest, `min` frames.
fn buffer_frames(mode: BufferMode, min: i32) -> i32 {
    let frames = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
    let (times, at_least) = match mode {
        BufferMode::Native | BufferMode::Lowest | BufferMode::Fast => (1, 0),
        BufferMode::Safe => (2, 1024),
        BufferMode::UltraSafe => (4, 4096),
        BufferMode::Fixed(n) => (1, frames(n)),
        BufferMode::AtLeast(base, n) => return buffer_frames(base.mode(), min).max(frames(n)),
    };
    min.saturating_mul(times).max(at_least)
}

/// Opens a stereo float track for `mode` at `rate` Hz (0: the device's own) and starts it.
pub(super) fn open(mode: BufferMode, rate: u32) -> Result<Opened, String> {
    let vm = vm()?;
    let (track, frames, sample_rate) = {
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let made = make(&mut env, mode, rate);
        if made.is_err() {
            let _ = env.exception_clear();
        }
        made?
    };
    Ok(Opened {
        vm,
        track,
        frames,
        sample_rate,
    })
}

/// [`open`]'s track, its buffer in frames and its rate.
fn make(env: &mut JNIEnv<'_>, mode: BufferMode, rate: u32) -> Result<(GlobalRef, usize, u32), String> {
    let j = |e: jni::errors::Error| e.to_string();
    let rate = match i32::try_from(rate) {
        Ok(rate) if rate > 0 => rate,
        _ => env
            .call_static_method(TRACK, "getNativeOutputSampleRate", "(I)I", &[STREAM_MUSIC.into()])
            .and_then(|v| v.i())
            .map_err(j)?,
    };
    if rate <= 0 {
        return Err(format!("no sample rate ({rate})"));
    }
    let args: [JValue; 3] = [rate.into(), CHANNEL_OUT_STEREO.into(), ENCODING_PCM_FLOAT.into()];
    let min = env
        .call_static_method(TRACK, "getMinBufferSize", "(III)I", &args)
        .and_then(|v| v.i())
        .map_err(j)?;
    if min <= 0 {
        return Err(format!("no buffer size at {rate} Hz ({min})"));
    }
    let frames = buffer_frames(mode, min.div_euclid(FRAME_BYTES).max(64));
    let args: [JValue; 6] = [
        STREAM_MUSIC.into(),
        rate.into(),
        CHANNEL_OUT_STEREO.into(),
        ENCODING_PCM_FLOAT.into(),
        frames.saturating_mul(FRAME_BYTES).into(),
        MODE_STREAM.into(),
    ];
    let track = env.new_object(TRACK, "(IIIIII)V", &args).map_err(j)?;
    let state = env.call_method(&track, "getState", "()I", &[]).and_then(|v| v.i());
    if state.map_err(j)? != STATE_INITIALIZED {
        let _ = env.call_method(&track, "release", "()V", &[]);
        return Err("the track did not initialise".into());
    }
    env.call_method(&track, "play", "()V", &[]).map_err(j)?;
    let track = env.new_global_ref(track).map_err(j)?;
    Ok((track, frames as usize, rate as u32))
}

impl Opened {
    /// Starts the thread that renders `state` into the track.
    pub(super) fn play<R: AudioRenderer>(self, state: CallbackState<R>) -> Result<Track, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("honeydew-audiotrack".into())
            .spawn(move || self.run(state, &flag))
            .map_err(|e| e.to_string())?;
        Ok(Track {
            stop,
            thread: Some(thread),
        })
    }

    /// The thread: a chunk rendered and written at a time, a quarter of the buffer, until `stop` or a failed write.
    fn run<R: AudioRenderer>(self, mut state: CallbackState<R>, stop: &AtomicBool) {
        let fail = |state: &CallbackState<R>| state.shared.failed.store(true, Ordering::Release);
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return fail(&state);
        };
        let chunk = (self.frames / 4).max(64);
        let samples = i32::try_from(chunk * 2).unwrap_or(i32::MAX);
        let Ok(array) = env.new_float_array(samples) else {
            let _ = env.exception_clear();
            return fail(&state);
        };
        let _ = (state.shared.buffer_frames).compare_exchange(
            0,
            u32::try_from(chunk).unwrap_or(u32::MAX),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        let track = self.track.as_obj();
        let rate = self.sample_rate;
        let latency_ns = self.frames as i64 * 1_000_000_000 / i64::from(rate.max(1));
        let (mut out, mut scratch) = (vec![0.0_f32; chunk * 2], vec![0.0_f32; SUB_BLOCK * 2]);
        let mut underruns = 0;
        while !stop.load(Ordering::Acquire) {
            // When this chunk is heard: once the buffer ahead of it has played.
            let heard = clock().map(|now| now + latency_ns);
            state.callback(chunk, 2, rate, heard, &mut scratch, |at, rendered| {
                for (o, &s) in out[at * 2..].iter_mut().zip(rendered) {
                    *o = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
                }
            });
            let wrote = env.set_float_array_region(&array, 0, &out).and_then(|()| {
                env.call_method(track, "write", "([FIII)I", &[
                    JValue::Object(&array),
                    0.into(),
                    samples.into(),
                    WRITE_BLOCKING.into(),
                ])?
                .i()
            });
            if !matches!(wrote, Ok(n) if n >= 0) {
                let _ = env.exception_clear();
                fail(&state);
                break;
            }
            let count = env.call_method(track, "getUnderrunCount", "()I", &[]).and_then(|v| v.i());
            match count {
                Ok(count) if count > underruns => {
                    let new = u64::try_from(count - underruns).unwrap_or(0);
                    state.shared.underruns.fetch_add(new, Ordering::Release);
                    underruns = count;
                }
                Ok(_) => {}
                Err(_) => {
                    let _ = env.exception_clear();
                }
            }
        }
        let _ = env.call_method(track, "stop", "()V", &[]);
        let _ = env.call_method(track, "release", "()V", &[]);
        let _ = env.exception_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::SoundMode;

    #[test]
    fn buffers_grow_with_the_mode_and_the_slider() {
        assert_eq!(buffer_frames(BufferMode::Native, 960), 960);
        assert_eq!(buffer_frames(BufferMode::Safe, 300), 1024);
        assert_eq!(buffer_frames(BufferMode::UltraSafe, 1920), 7680);
        assert_eq!(buffer_frames(BufferMode::AtLeast(SoundMode::Fast, 2048), 960), 2048);
        assert_eq!(buffer_frames(BufferMode::AtLeast(SoundMode::UltraSafe, 512), 960), 4096);
    }
}
