//! The pickers' sample preview (the Sample strip): one sample played once from a frame, at gain 0.7, centred, at its
//! own pitch (rate 1), no loop. It goes out past the master and its meter, as the click does, so no channel, pad or
//! effect applies (master effects neither).
//!
//! A big file's preview starts on the part decoded so far and goes on in each longer part as it comes, from where it
//! is: a part is the whole's start, frame for frame, so the change is seamless. While more comes its end waits,
//! silent, for the next part rather than ending (issues #131, #153).

use std::sync::Arc;

use honeydew_audio::sample::SampleData;

/// The preview's gain.
pub const GAIN: f32 = 0.7;

pub(super) struct Preview {
    data: Arc<SampleData>,
    /// Where it is in the sample's frames, and how far it moves an output frame.
    pos: f64,
    step: f64,
    /// A longer part of the sample comes ([`Self::grow`]): the end waits for it.
    more: bool,
}

impl Preview {
    /// `data` from frame `from`, at output rate `rate`.
    pub fn new(data: Arc<SampleData>, from: usize, rate: f32) -> Self {
        let step = f64::from(data.sample_rate) / f64::from(rate);
        Self {
            data,
            pos: from as f64,
            step,
            more: false,
        }
    }

    /// It goes on in `data`, the same sample decoded further, from where it is; `more`: a longer part still comes.
    /// The Arc it replaces is never the last, as with [`AudioRenderer::preview`].
    pub fn grow(&mut self, data: Arc<SampleData>, more: bool) {
        (self.data, self.more) = (data, more);
    }

    /// Adds its next frames into `left` and `right` (linear between frames); false once it is past the end.
    pub fn add(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let (l, r) = (&*self.data.left, self.data.right.as_deref());
        for (out_l, out_r) in left.iter_mut().zip(right.iter_mut()) {
            let i = self.pos as usize;
            if !self.step.is_finite() || self.step <= 0.0 {
                return false;
            }
            if i >= l.len() {
                return self.more;
            }
            let t = (self.pos - i as f64) as f32;
            let at = |s: &[f32]| s[i] + (s.get(i + 1).copied().unwrap_or(0.0) - s[i]) * t;
            let a = at(l);
            let b = r.map_or(a, at);
            *out_l += GAIN * a;
            *out_r += GAIN * b;
            self.pos += self.step;
        }
        self.more || (self.pos as usize) < l.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
