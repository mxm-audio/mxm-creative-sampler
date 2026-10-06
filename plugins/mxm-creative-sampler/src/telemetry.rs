//! Bounded callback-to-editor telemetry. No sample asset crosses this channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// How many sounding grains the editor can see at once.
///
/// The instrument-wide grain budget is 256 and a display cannot separate that many marks over one
/// waveform anyway, so this is a drawing limit rather than an engine one. Running out shows fewer
/// grains, never wrong ones.
pub const GRAIN_SLOTS: usize = 128;

#[derive(Debug)]
pub struct Telemetry {
    peak: AtomicU32,
    clipped: AtomicBool,
    voices: AtomicUsize,
    /// Each layer's newest read position, normalised over the whole sample, or negative for none.
    /// A scalar per layer — still no sample asset on this channel.
    playhead: [AtomicU32; 2],
    /// How many of `grains` are currently sounding.
    grain_count: AtomicUsize,
    /// Which layer the editor is showing, so `process` publishes that one's grains.
    ///
    /// **The editor asks and the callback answers.** Publishing both layers would double the
    /// traffic for a picture that can only be drawn over one waveform at a time, and the selection
    /// is editor state the audio thread has no other reason to know.
    shown_layer: AtomicUsize,
    /// Every sounding grain of the layer the editor last asked for, each packed into one word:
    /// the start position in the top sixteen bits, the current position in the next sixteen, and
    /// the window weight in the sixteen below that, all as fractions of the whole sample.
    ///
    /// **Packed so a slot is one store** and can never be read half-updated — a grain drawn with
    /// one frame's start and the next frame's current position would be a segment that never
    /// existed. Sixteen bits is a four-thousandth of the sample, far finer than a pixel on any
    /// waveform this is drawn over.
    grains: [AtomicU64; GRAIN_SLOTS],
    /// Developer requests, each taken once by the editor: a CC 119 view address, a CC 117 browser
    /// open or close, and a CC 116 theme index. [`NO_REQUEST`] means none is pending.
    ///
    /// **No CC 118 slot**, because this editor has no expander to open; the shape in mxm-kit's
    /// `docs/plugin-conventions.md` (*A developer channel in every editor*) for a plugin without
    /// one is to answer it with nothing.
    dev_view: AtomicU8,
    dev_browser: AtomicU8,
    dev_theme: AtomicU8,
    /// The host tempo in force, so a synced rate reads its division.
    pub tempo: mxm_tempo::TempoCell,
}

/// An empty developer request slot.
const NO_REQUEST: u8 = u8::MAX;

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            peak: AtomicU32::new(0),
            clipped: AtomicBool::new(false),
            voices: AtomicUsize::new(0),
            playhead: [
                AtomicU32::new((-1.0f32).to_bits()),
                AtomicU32::new((-1.0f32).to_bits()),
            ],
            grain_count: AtomicUsize::new(0),
            shown_layer: AtomicUsize::new(0),
            grains: std::array::from_fn(|_| AtomicU64::new(0)),
            dev_view: AtomicU8::new(NO_REQUEST),
            dev_browser: AtomicU8::new(NO_REQUEST),
            dev_theme: AtomicU8::new(NO_REQUEST),
            tempo: mxm_tempo::TempoCell::new(),
        }
    }
}

impl Telemetry {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn publish(&self, peak: f32, voices: usize, playhead: [Option<f32>; 2]) {
        for (slot, position) in self.playhead.iter().zip(playhead) {
            let value = position.filter(|p| p.is_finite()).unwrap_or(-1.0);
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        let peak = if peak.is_finite() { peak.abs() } else { 0.0 };
        let mut current = self.peak.load(Ordering::Relaxed);
        loop {
            let combined = f32::from_bits(current).max(peak);
            match self.peak.compare_exchange_weak(
                current,
                combined.to_bits(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(seen) => current = seen,
            }
        }
        if peak >= 1.0 {
            self.clipped.store(true, Ordering::Relaxed);
        }
        self.voices.store(voices, Ordering::Relaxed);
    }

    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }

    /// Where the selected layer is reading, `0..=1`, or `None` when nothing is sounding.
    pub fn playhead(&self, layer: usize) -> Option<f32> {
        let raw = f32::from_bits(self.playhead.get(layer)?.load(Ordering::Relaxed));
        (raw >= 0.0 && raw.is_finite()).then_some(raw)
    }

    pub fn clipped(&self) -> bool {
        self.clipped.load(Ordering::Relaxed)
    }

    pub fn clear_clip(&self) {
        self.clipped.store(false, Ordering::Relaxed);
    }

    pub fn voices(&self) -> usize {
        self.voices.load(Ordering::Relaxed)
    }

    /// Which layer `process` should publish grains for.
    pub fn shown_layer(&self) -> usize {
        self.shown_layer.load(Ordering::Relaxed).min(1)
    }

    /// Tells the callback which layer the editor is showing. Called from the editor each frame.
    pub fn show_layer(&self, layer: usize) {
        self.shown_layer.store(layer.min(1), Ordering::Relaxed);
    }

    /// Publishes the grains sounding on one layer, as fractions of the whole sample.
    ///
    /// **Realtime-safe**: relaxed stores only, no allocation and no lock. Lossy by design — a
    /// reader that misses a frame simply sees the next one, which for a live picture is the right
    /// loss. Design system §13 licenses it.
    pub fn publish_grains(&self, grains: &[(f32, f32, f32)]) {
        let count = grains.len().min(GRAIN_SLOTS);
        for (slot, (start, current, weight)) in self.grains.iter().zip(&grains[..count]) {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 65_535.0) as u64;
            slot.store(
                (q(*start) << 48) | (q(*current) << 32) | (q(*weight) << 16),
                Ordering::Relaxed,
            );
        }
        // Published last, so a reader that sees the count has already seen the slots.
        self.grain_count.store(count, Ordering::Release);
    }

    /// Fills `out` with the sounding grains and returns how many were written.
    ///
    /// Each is `(start, current, weight)`, all `0..=1` over the whole sample. Borrowed buffer
    /// rather than a returned `Vec`: the editor calls this every frame and must not allocate for a
    /// picture.
    pub fn grains(&self, out: &mut [(f32, f32, f32); GRAIN_SLOTS]) -> usize {
        let count = self.grain_count.load(Ordering::Acquire).min(GRAIN_SLOTS);
        for (slot, out) in out.iter_mut().enumerate().take(count) {
            let packed = self.grains[slot].load(Ordering::Relaxed);
            let unq = |shift: u32| ((packed >> shift) & 0xffff) as f32 / 65_535.0;
            *out = (unq(48), unq(32), unq(16));
        }
        count
    }

    /// A developer view address: a category, 0–5, or the Parameters surface, 127.
    pub fn request_view(&self, view: u8) {
        self.dev_view
            .store(view.min(NO_REQUEST - 1), Ordering::Relaxed);
    }

    pub fn take_view_request(&self) -> Option<usize> {
        match self.dev_view.swap(NO_REQUEST, Ordering::Relaxed) {
            NO_REQUEST => None,
            view => Some(view as usize),
        }
    }

    pub fn request_browser(&self, open: bool) {
        self.dev_browser.store(u8::from(open), Ordering::Relaxed);
    }

    pub fn take_browser_request(&self) -> Option<bool> {
        match self.dev_browser.swap(NO_REQUEST, Ordering::Relaxed) {
            NO_REQUEST => None,
            open => Some(open != 0),
        }
    }

    /// A theme by index — 0 light, 1 dark, 2 system, as `mxm_ui::theme::from_index` reads it.
    pub fn request_theme(&self, theme: u8) {
        self.dev_theme
            .store(theme.min(NO_REQUEST - 1), Ordering::Relaxed);
    }

    pub fn take_theme_request(&self) -> Option<u8> {
        match self.dev_theme.swap(NO_REQUEST, Ordering::Relaxed) {
            NO_REQUEST => None,
            theme => Some(theme),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlay is only as true as this round trip: the callback packs three fractions into one
    /// word and the editor unpacks them, and a mistake either way draws nothing, or draws it in the
    /// wrong place.
    #[test]
    fn sounding_grains_survive_the_packing() {
        let telemetry = Telemetry::default();
        let sent = [
            (0.0f32, 0.25f32, 1.0f32),
            (0.5, 0.5, 0.5),
            (0.75, 0.9, 0.125),
            (1.0, 1.0, 0.0),
        ];
        telemetry.publish_grains(&sent);

        let mut got = [(0.0f32, 0.0f32, 0.0f32); GRAIN_SLOTS];
        let found = telemetry.grains(&mut got);
        assert_eq!(found, sent.len(), "the count did not survive");
        for (sent, got) in sent.iter().zip(&got[..found]) {
            // The packing is sixteen bits per fraction, so this is far inside its resolution.
            for (a, b) in [(sent.0, got.0), (sent.1, got.1), (sent.2, got.2)] {
                assert!((a - b).abs() < 1.0e-4, "{a} came back as {b}");
            }
        }
    }

    /// Publishing fewer grains than last time must not leave the older ones on screen.
    #[test]
    fn a_thinning_cloud_does_not_keep_its_old_marks() {
        let telemetry = Telemetry::default();
        telemetry.publish_grains(&[(0.1, 0.2, 1.0); 8]);
        telemetry.publish_grains(&[(0.3, 0.4, 1.0); 2]);
        let mut got = [(0.0f32, 0.0f32, 0.0f32); GRAIN_SLOTS];
        assert_eq!(telemetry.grains(&mut got), 2);
    }
}
