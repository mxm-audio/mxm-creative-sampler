//! Framework-free DSP for `mxm-creative-sampler`.
//!
//! Clean playback uses a sixteen-tap Blackman-windowed sinc interpolator. Stretch is deterministic
//! dual-head windowed overlap-add; Grain is a bounded stochastic scheduler. Character deliberately
//! moves virtual input-rate conversion, quantisation and reconstruction into each source reader.

#[cfg(any(test, feature = "conformance"))]
pub mod conformance;
pub mod routing;

use core::f32::consts::{PI, TAU};
use mxm_modulation::standard;

pub const MAX_VOICES: usize = 8;

/// How many grains may sound at once **across the whole instrument**.
///
/// A shared budget, not a per-voice allowance. It was `GRAINS_PER_LAYER = 16` held in a fixed array
/// inside every layer of every voice, which provisioned for the eight-note worst case and then
/// charged that provisioning to every single note: one held note could reach sixteen grains per
/// layer while the engine was sized for two hundred and fifty-six.
///
/// 256 is exactly the old worst case — eight voices × two layers × sixteen — so the measured CPU
/// ceiling does not move. What changed is the distribution: a note gets `GRAIN_BUDGET / active
/// layers`, so one note reaches 128 per layer, four notes 32, and eight notes the 16 they had.
/// Fewer notes, more grains each; the eight-voice case degrades to precisely its old behaviour.
pub const GRAIN_BUDGET: usize = 256;

/// The largest share one layer can ever hold, and so the size of its array.
///
/// A single voice with both layers loaded: `GRAIN_BUDGET / 2`. Arrays stay per-layer and **kept
/// compacted** rather than becoming one shared free list, because the render walk's contiguous run
/// is what vectorises — the sub-linear scaling that made a bigger pool affordable in the first
/// place. A linked list would spend that to save 57 KB.
pub const MAX_GRAINS_PER_LAYER: usize = GRAIN_BUDGET / 2;

/// The most overlap the controls may ask for, and the count the gain law divides by.
///
/// **Musical, not a resource bound** — see [`overlap`]. Held at the largest share a layer can
/// actually be granted so a single note can reach what it asks for.
pub const MAX_OVERLAP: f32 = MAX_GRAINS_PER_LAYER as f32;

/// The mean and mean-square of the Hann grain window, which [`normalisation`] needs to tell
/// coherent overlap from incoherent. Exact for `0.5 - 0.5cos`, and constant because the window
/// shape is not a control here.
/// How many points [`window_moments`] integrates the window's mean and mean-square over.
///
/// Chosen. The window is smooth and these two numbers only set a gain, so a coarse sum is honest;
/// 128 puts the error on both well below a thousandth. `mxm-grain-fx`'s `window::moments` uses the
/// same count for the same reason.
const MOMENT_POINTS: usize = 128;

/// Crest allowance for the incoherent law, measured.
///
/// **RMS compensation bounds power, not peaks.** A stochastic cloud's crest factor grows with the
/// count — many independent grains sum toward a Gaussian, whose peaks run several times its RMS —
/// while a coherent sum keeps the crest of the source it is copying. So the incoherent branch, and
/// only the incoherent branch, needs an explicit allowance, or a dense scattered cloud clips while
/// its RMS sits exactly where the law intends.
///
/// Measured on the probe's harmonic source (`examples/measure_readers`, the peak sweep): the
/// uncompensated incoherent branch peaked at **1.400** at Scatter 1.0, against 0.900 worst on the
/// coherent branch. 0.70 brings the worst case just under full scale, which is where the owner
/// asked the ceiling to sit — there is no limiter here, only a clip indicator.
const INCOHERENT_HEADROOM: f32 = 0.70;

/// The fastest onset rate a layer will schedule.
pub const MAX_GRAIN_DENSITY_HZ: f32 = 200.0;

/// The widest per-grain detune Spread reaches, either way. An octave is already far past a
/// section of players and into a chord; the control is skewed so the first third holds the cents.
pub const MAX_GRAIN_SPREAD_ST: f32 = 12.0;

/// The longest loop crossfade, in seconds of source. **Provisional**, chosen against the material
/// this instrument holds: a generated source is about a second long and a dropped loop a few, so two
/// seconds already fades most of a loop into itself, and the loop's own length caps it further.
pub const MAX_LOOP_CROSSFADE_S: f32 = 2.0;
/// Taps in the interpolation kernel. Public so the measurement harness can name the number it is
/// reporting a quality figure for.
pub const SINC_TAPS: i32 = 16;
const MIN_REGION_FRAMES: usize = 1;

/// The overlap-add window, in seconds. Public so the measurement harness can name the rate it is
/// reporting flutter at rather than repeating the number and drifting from it.
pub const STRETCH_WINDOW_S: f32 = 0.080;

/// Hop between onset-detector frames, in seconds. Short enough that the refinement below has
/// little left to do, long enough that the log-energy difference is a ratio between two
/// meaningfully different windows rather than between two views of the same few samples.
const TRANSIENT_HOP_S: f32 = 0.005;
/// The shortest gap between two accepted onsets. Below this an attack's own decay re-triggers the
/// detector and one note is reported as several.
const TRANSIENT_MIN_SPACING_S: f32 = 0.030;
/// Hops per analysis window. Long enough to average over several periods of a bass note, so a low
/// fundamental's own cycle is not read as a string of attacks; the hop, not this, sets how finely
/// an onset is located.
const TRANSIENT_WINDOW_HOPS: usize = 8;
/// How many hops either side form the local mean the adaptive threshold is taken from.
const TRANSIENT_LOCAL_HOPS: usize = 12;
/// How far above the local mean a flux peak must stand to be an onset.
const TRANSIENT_THRESHOLD: f32 = 2.2;
/// The absolute floor under the adaptive threshold, so a near-silent passage's own noise cannot
/// clear a local mean that is itself near zero.
const TRANSIENT_FLOOR: f32 = 0.35;

/// Immutable stereo audio, with the load-time analysis the readers need.
///
/// **The analysis is part of constructing a `Sample`, not something attached afterwards.** A
/// published sample is immutable while any reader can see it, so analysis arriving late would need
/// a second publication, a revision counter and a policy for voices already sounding from the
/// first. Doing it here means a note can only ever reach a sample whose analysis is already
/// complete. See `plans/plan-stretch-quality.md` §5.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    frames: Vec<[f32; 2]>,
    sample_rate: f32,
    /// Onset positions in source frames, ascending and de-duplicated.
    transients: Vec<u32>,
    /// How the two channels are summed for analysis and alignment: `[0.5, 0.5]` normally, and
    /// `[0.5, -0.5]` where the source is anti-phase. **`L + R` is zero on an anti-phase sample**,
    /// so a plain sum would hand the alignment search, the onset detector and the period track
    /// silence on perfectly valid audio, and all three would decline — degrading the reader back to
    /// the clocked splice with nothing to say why. Decided once at load from which combination
    /// carries the energy.
    mono_mix: [f32; 2],
    /// The source's own period, in source frames, sampled every [`PERIOD_TRACK_HOP_S`] seconds.
    /// **Zero means the detector declined** — silence, noise, a drum, a chord — and a reader that
    /// meets a zero falls back to its fixed window and blind search rather than to a guessed
    /// period.
    periods: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleError {
    Empty,
    InvalidRate,
    NonFinite,
    /// A buffer sized by the source could not be reserved. Nothing is built, and the caller keeps
    /// whatever it had.
    Allocation,
}

impl Sample {
    /// Build a sample, **including its load-time analysis**.
    ///
    /// **This allocates and walks the whole source twice — never call it from an audio thread.**
    /// Every buffer it sizes from the source is reserved fallibly, and one it cannot reserve returns
    /// [`SampleError::Allocation`] rather than aborting the process: a plugin builds a `Sample` from
    /// restored state, a preset and an import, where a failed allocation would take the host down.
    /// Constructing a `Sample` runs onset detection and a period track, because the readers need
    /// both and a published sample is immutable, so analysis cannot be attached afterwards without
    /// a second publication. It costs about 26 ms for a 30 s stereo source on the development
    /// machine and scales with length. The plugin calls this off the audio thread, behind the
    /// stage/commit barrier that keeps new audio from being heard through the old patch.
    pub fn new(frames: Vec<[f32; 2]>, sample_rate: f32) -> Result<Self, SampleError> {
        if frames.is_empty() {
            return Err(SampleError::Empty);
        }
        if !sample_rate.is_finite() || !(1_000.0..=768_000.0).contains(&sample_rate) {
            return Err(SampleError::InvalidRate);
        }
        if frames.iter().flatten().any(|value| !value.is_finite()) {
            return Err(SampleError::NonFinite);
        }
        let mono_mix = choose_mono_mix(&frames);
        let transients = detect_transients(&frames, mono_mix, sample_rate)?;
        let periods = track_periods(&frames, mono_mix, sample_rate)?;
        Ok(Self {
            frames,
            sample_rate,
            transients,
            mono_mix,
            periods,
        })
    }

    pub fn frames(&self) -> &[[f32; 2]] {
        &self.frames
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Onset positions in source frames, ascending. Empty where the material has no attacks the
    /// detector will commit to — which is a finding, not a failure, exactly as `detect_root`
    /// declining is.
    pub fn transients(&self) -> &[u32] {
        &self.transients
    }

    /// The channel weights analysis and alignment sum with. See [`Sample::mono_mix`].
    #[must_use]
    pub fn mono_mix(&self) -> [f32; 2] {
        self.mono_mix
    }

    /// The source's period in source frames at a position, or `None` where the track declined.
    ///
    /// **One track for the stereo pair, never one per channel.** Two channels analysed separately
    /// disagree, and a reader that took a different window for each would hear the disagreement as
    /// the image moving — the same rule the alignment decision follows.
    #[must_use]
    pub fn period_at(&self, position: f32) -> Option<f32> {
        if self.periods.is_empty() || !position.is_finite() {
            return None;
        }
        let hop = (PERIOD_TRACK_HOP_S * self.sample_rate).max(1.0);
        let slot = (position.max(0.0) / hop) as usize;
        let period = *self.periods.get(slot.min(self.periods.len() - 1))?;
        (period > 0.0).then_some(period)
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reader {
    #[default]
    Repitch,
    Stretch,
    Grain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    #[default]
    Off,
    Forward,
    Alternate,
}

#[derive(Debug, Clone, Copy)]
pub struct LayerParams {
    pub reader: Reader,
    pub root_note: f32,
    pub tune_semitones: f32,
    pub start: f32,
    pub end: f32,
    pub loop_start: f32,
    pub loop_end: f32,
    pub loop_mode: LoopMode,
    /// **How long a forward loop's seam fades, in seconds of source.** Zero is the hard wrap this
    /// crate shipped with, bit for bit. Inert in Alternate, in Off and in Grain. See [`Seam`] and
    /// [`loop_crossfade`].
    pub loop_crossfade_s: f32,
    pub reverse: bool,
    pub level: f32,
    pub pan: f32,
    pub stretch_travel: f32,
    pub grain_size_s: f32,
    pub grain_density_hz: f32,
    /// **How far each grain's read position is drawn away from the travelling playhead.**
    ///
    /// This is the decorrelator: grains that read *different material* stop summing coherently, and
    /// that is the only thing that turns overlapping copies into a cloud rather than a comb. It is
    /// also what moves the normalisation law — see [`normalisation`].
    pub grain_position_variation: f32,
    /// **How far each onset is drawn away from the clock**, from a periodic schedule at zero,
    /// through a jittered one, to a per-sample stochastic draw at the top.
    ///
    /// Separate from the position draw because they are unrelated behaviours: a periodic scheduler
    /// puts an amplitude modulator at the grain rate under everything, which is a *timing* sound,
    /// while reading the same material twice is a *material* one. One control did both.
    pub grain_onset_timing: f32,
    /// Per-grain random detune, in semitones either way. Independent of everything else: a cloud can
    /// be detuned without also being scattered in position, timing, direction or pan.
    pub grain_spread_semitones: f32,
    /// How far grains are thrown across the stereo field, `0..=1`.
    pub grain_stereo_width: f32,
    /// The probability that a grain reads backwards, `0..=1`.
    pub grain_reverse_chance: f32,
    /// The grain envelope, 0 a near-boxcar, 0.5 a triangle, 1 a Hann. See [`grain_window`].
    pub grain_shape: f32,
}

impl Default for LayerParams {
    fn default() -> Self {
        Self {
            reader: Reader::Repitch,
            root_note: 60.0,
            tune_semitones: 0.0,
            start: 0.0,
            end: 1.0,
            loop_start: 0.0,
            loop_end: 1.0,
            loop_mode: LoopMode::Off,
            loop_crossfade_s: 0.0,
            reverse: false,
            level: 1.0,
            pan: 0.0,
            stretch_travel: 1.0,
            grain_size_s: 0.090,
            grain_density_hz: 14.0,
            grain_position_variation: 0.25,
            grain_onset_timing: 0.25,
            grain_spread_semitones: 0.0,
            grain_stereo_width: 0.0,
            grain_reverse_chance: 0.0,
            // A Hann, which is what this instrument's grains were before the shape was a control.
            grain_shape: 1.0,
        }
    }
}

/// Which converter the voice's DAC is, which is a **distortion character and not a bit depth**.
///
/// mxm-kit's `docs/oscillators/14-samplers.md` §14.1: the vintage sampler DAC names — Emu II,
/// AM6070 — are companding parts rather than resolutions, and §14.2 measures why that is the whole
/// difference.
/// A linear converter degrades one-for-one with level, so at −36 dB an 8-bit linear converter sits
/// at −15.2 dB of junk and by −54 dB **the signal is gone entirely**: every sample is smaller than
/// one step and rounds to zero. A companding one holds about −36 dB across a 30 dB span of input
/// level. §14.8's recommendation is this enum's second variant, in its own words: *"if a
/// low-resolution mode is on the table, µ-law is the one to implement."*
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConverterType {
    /// Uniform steps. What this instrument shipped with, and what Clean must stay.
    #[default]
    Linear,
    /// µ-law: logarithmic steps, coarse at full scale and fine near silence.
    Companding,
}

#[derive(Debug, Clone, Copy)]
pub struct Character {
    /// Virtual grid spacing, 0 is native and 1 is thirty-two source frames.
    pub rate: f32,
    /// Linear PCM resolution reduction, 0 is 16-bit and 1 is 5-bit.
    pub converter: f32,
    /// 0 is sinc/linear reconstruction and 1 is a zero-order hold image.
    pub reconstruction: f32,
    /// Post-quantiser saturation drive.
    ///
    /// **Not companding, despite what this field was called until the compander arrived.** It is
    /// applied to the quantiser's *output*, so it cannot change the quantiser's relationship to the
    /// signal at all — which is what companding is. [`ConverterType::Companding`] is that; this is a
    /// saturator, it stays exactly where it sits in the chain, and the two are independent.
    pub input: f32,
    /// Which converter, [`ConverterType`].
    pub converter_type: ConverterType,
    /// Clock instability, `0..=1`, where 1 is [`JITTER_CELLS`] of a converter cell.
    ///
    /// **Jitter is *when* the converter samples, not *what* it adds.** A real sampler's DAC clock is
    /// not perfectly even, and the unsteadiness smears its images — which is a large part of why the
    /// machines this section emulates sound alive rather than merely coarse. It is the control TAL's
    /// interface calls Jitter, and it is not dither: dither adds noise before the quantiser and
    /// leaves the clock alone.
    ///
    /// **Zero is exact**: the cell boundary is untouched and the render is bit-identical to one with
    /// no jitter at all, which is what keeps Clean clean.
    pub jitter: f32,
}

/// How far the converter's clock wanders at full Jitter, as a fraction of one cell peak-to-peak.
///
/// **Just under one cell, and the bound is structural rather than a taste.** A clock's sampling
/// instants are *ordered*: instant `n` happens before instant `n + 1`. Cell `n` is displaced by
/// `±JITTER_CELLS/2`, so consecutive instants stay in order exactly while `JITTER_CELLS < 1` — at one
/// cell two instants can coincide, and past it they cross, at which point the reader is scrambling
/// addresses rather than sampling on a wobbly clock. An earlier version set this to `4.0` for
/// loudness and was exactly that: frames could be read out of order, which is a different effect
/// wearing this one's name.
///
/// `0.9` leaves the interval between two instants in `(0.1, 1.9)` cells — a clock that stretches and
/// squeezes by nearly double without ever running backwards.
pub const JITTER_CELLS: f32 = 0.9;

/// A cell's own wobble, `0..=1`, from its index alone.
///
/// **Stateless, and that is the whole design.** A stream would have to live somewhere, and the only
/// streams a voice has are the ones sequencing its grains — which is the trap the dither work found
/// and had to route around. A hash of the cell index needs no state at all, so it cannot perturb
/// anything else, it is identical however the block is split, and it is reproducible across runs.
#[inline]
fn cell_wobble(cell: f32) -> f32 {
    let mut z = (cell as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u32 << 24) as f32
}

impl Default for Character {
    fn default() -> Self {
        Self {
            rate: 0.0,
            converter: 0.0,
            reconstruction: 0.0,
            input: 0.0,
            converter_type: ConverterType::Linear,
            jitter: 0.0,
        }
    }
}

/// µ-law's µ, the standard 255. See [`ConverterType::Companding`].
const MU: f32 = 255.0;

/// Compresses `value` onto the companded domain, where the quantiser's steps are uniform.
///
/// **Bounded input, deliberately.** The law is defined on `-1..=1` and a voice routinely sums two
/// layers past that — each `level` reaches 200% before the converter sees it. Saturating at the
/// bound rather than letting the logarithm run is the conservative answer of the three available:
/// wrapping would fold a loud note to a quiet one, and rescaling would make the converter's
/// character depend on how hard it is driven. It also matches what the hardware did, since a DAC
/// has a full-scale code and nothing above it.
#[inline]
fn compand(value: f32) -> f32 {
    let magnitude = value.abs().min(1.0);
    let compressed = (1.0 + MU * magnitude).ln() / (1.0 + MU).ln();
    compressed.copysign(value)
}

/// The inverse of [`compand`].
#[inline]
fn expand(value: f32) -> f32 {
    let magnitude = value.abs().min(1.0);
    let expanded = ((1.0 + MU).powf(magnitude) - 1.0) / MU;
    expanded.copysign(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterMode {
    LowPass,
    BandPass,
    HighPass,
    #[default]
    Off,
}

/// How the keyboard hands presses to voices.
///
/// `Mono` and `Legato` share one voice and one last-note-priority press ledger; they differ only
/// in whether an overlapping press restarts the envelope and the readers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VoiceMode {
    #[default]
    Poly,
    Mono,
    Legato,
}

#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub layers: [LayerParams; 2],
    pub character: Character,
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
    pub filter_mode: FilterMode,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub filter_drive: f32,
    /// **Retired into the (Cutoff <- Envelope) route.** The reach it had is that route's scale, so
    /// the same number means the same octaves; see [`routing::FULL_SCALE`].
    /// **Retired into a route.** Kept as a field only so the equality gate can drive the old path
    /// beside the new one; nothing in the plugin writes it any more.
    #[cfg(test)]
    pub velocity_to_level: f32,
    /// LFO 1's rate, in hertz.
    pub lfo_rate_hz: f32,
    /// LFO 1's shape.
    pub lfo_shape: LfoShape,
    /// LFO 2's rate, in hertz.
    pub lfo2_rate_hz: f32,
    /// LFO 2's shape.
    pub lfo2_shape: LfoShape,
    /// **Retired into two routes**, on each layer's scan speed. See [`routing::TRAVEL_A_REACH`].
    #[cfg(test)]
    pub lfo_amount: f32,

    pub master: f32,
    pub voice_mode: VoiceMode,
    /// Seconds for a glide to cover one octave. Zero is off.
    pub glide_s: f32,
}

/// How far a full Shape-envelope excursion moves the filter at `filter_env = ±1`.
///
/// Four octaves is chosen, not measured: it is enough for a closed low pass to open into the
/// source's own band on a percussive shape, and small enough that the residual at the sustain
/// level is still a usable tone rather than a fully open filter. Provisional with everything else
/// here until the listening gate.
pub const FILTER_ENV_OCTAVES: f32 = 4.0;

/// How far full note pressure opens the filter. A fixed common route, not an authored one: the
/// bounded route list is post-listening work, and an expressive controller that moved nothing
/// would read as a broken instrument.
pub const PRESSURE_OCTAVES: f32 = 2.0;

impl Default for Params {
    fn default() -> Self {
        Self {
            layers: [LayerParams::default(), LayerParams::default()],
            character: Character::default(),
            attack_s: 0.005,
            decay_s: 0.180,
            sustain: 0.82,
            release_s: 0.350,
            filter_mode: FilterMode::Off,
            cutoff_hz: 18_000.0,
            resonance: 0.1,
            filter_drive: 0.0,
            #[cfg(test)]
            velocity_to_level: 0.35,
            lfo_rate_hz: 0.25,
            lfo_shape: LfoShape::Sine,
            lfo2_rate_hz: 0.13,
            lfo2_shape: LfoShape::Triangle,
            #[cfg(test)]
            lfo_amount: 0.0,

            master: 0.8,
            voice_mode: VoiceMode::Poly,
            glide_s: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub id: Option<u32>,
    pub channel: u8,
    pub key: u8,
    pub velocity: f32,
    pub tuning: f32,
}

impl Note {
    pub fn midi(key: u8, velocity: f32) -> Self {
        Self {
            id: None,
            channel: 0,
            key,
            velocity,
            tuning: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Inert,
    Tailing,
    Live,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stereo {
    pub left: f32,
    pub right: f32,
}

impl Stereo {
    fn add(self, other: Self) -> Self {
        Self {
            left: self.left + other.left,
            right: self.right + other.right,
        }
    }

    fn gain(self, gain: f32) -> Self {
        Self {
            left: self.left * gain,
            right: self.right * gain,
        }
    }
}

/// Where jitter ends and the hand-over to the stochastic process begins.
const HANDOVER_AT: f32 = 0.5;

/// How far into a slot a fully jittered onset may fall. Less than one so a jittered onset cannot
/// land exactly on the next slot's boundary and read as a double-fire.
const MAX_JITTER: f32 = 0.98;

/// How Scatter divides into a jitter depth and a hand-over fraction.
#[derive(Debug, Clone, Copy)]
struct Scatter {
    /// How much of a slot a jittered onset may wander over, `0..=MAX_JITTER`.
    jitter: f32,
    /// How much of the density has been handed to the per-sample draw, `0..=1`.
    handover: f32,
}

impl Scatter {
    fn from_control(scatter: f32) -> Self {
        let s = sanitise(scatter, 0.0).clamp(0.0, 1.0);
        if s <= HANDOVER_AT {
            Self {
                jitter: (s / HANDOVER_AT) * MAX_JITTER,
                handover: 0.0,
            }
        } else {
            Self {
                jitter: MAX_JITTER,
                handover: (s - HANDOVER_AT) / (1.0 - HANDOVER_AT),
            }
        }
    }
}

/// When the next grain starts.
///
/// **Periodic onsets are why a low-Scatter cloud rings.** A synchronous scheduler puts an amplitude
/// modulator at the grain rate under everything, and the grain window is its waveform — so the
/// cloud acquires a pitch that is not in the source, and the grains are heard individually. The
/// cure is not a wider jitter: however far an onset wanders inside its slot, there is still exactly
/// one onset per slot, and that is what a jitter can never escape.
///
/// **One control reaches all three families, in two segments.** Below [`HANDOVER_AT`] the onset is
/// drawn inside a widening fraction of its slot — semi-synchronous. Above it the periodic process
/// is *handed over* to a stochastic one: each slot's onset is suppressed with probability `q`,
/// while a per-sample draw fires at `q / slot`. The two rates sum to `1 / slot` at every setting,
/// so mean density does not move across the hand-over — only the statistics do — and at the top the
/// slot phasor never fires at all. Slots may then hold two onsets or none, which is what "cloud"
/// means and what a jitter cannot produce.
///
/// `handover` is also what moves the normalisation law in [`Voice::grain`], so regime and law
/// change together and there is no undefined region between them.
#[derive(Debug, Clone, Copy, Default)]
struct GrainScheduler {
    /// How far into the current slot we are, in samples.
    slot_position: f32,
    /// The slot this cycle was started with. Held so a moving density control cannot shorten a slot
    /// out from under an onset already scheduled inside it.
    slot_length: f32,
    /// Where in this slot the periodic onset fires.
    offset: f32,
    /// This slot's onset was handed to the stochastic process.
    suppressed: bool,
    /// The periodic onset for this slot has not fired yet.
    armed: bool,
}

impl GrainScheduler {
    /// Advance one sample and say how many grains start on it.
    ///
    /// Both processes can fire on the same sample while the hand-over is part way across; that is
    /// the Poisson statistics the asynchronous branch exists to provide, not a defect to guard.
    /// Arms the very first onset to fire on the voice's first sample.
    ///
    /// **A note has to start when it is played.** Left to its ordinary cycle the scheduler opens a
    /// voice by rolling a jittered offset into the first slot and, above the hand-over, by deciding
    /// that slot belongs to the stochastic process instead — so the first grain of a fresh note
    /// landed anywhere inside the first slot, or nowhere. At the init patch's forty grains a second
    /// that is up to 25 ms of silence before a key does anything, on top of the envelope's attack,
    /// and a different number every time. Two `mxm-player` tests measured it as a note that never
    /// sounded; a player hears it as a keyboard that does not answer.
    ///
    /// `slot_length` of one sample is what makes this exactly one onset: the first tick does not
    /// re-arm, fires at `slot_position` zero, and the tick after it rolls an ordinary slot. The
    /// cloud's statistics from the second onset onwards are untouched.
    fn prime(&mut self) {
        self.slot_position = 0.0;
        self.slot_length = 1.0;
        self.offset = 0.0;
        self.suppressed = false;
        self.armed = true;
    }

    #[inline]
    fn tick(&mut self, slot: f32, scatter: Scatter, rng: &mut u64) -> u32 {
        let slot = slot.max(1.0);
        if self.slot_position >= self.slot_length {
            // Carry the overshoot so onsets do not drift against the sample clock, and adopt the
            // current control value here rather than mid-slot.
            self.slot_position -= self.slot_length;
            self.slot_length = slot;
            self.offset = split_mix(rng) * scatter.jitter * slot;
            self.suppressed = split_mix(rng) < scatter.handover;
            self.armed = true;
        }
        let mut starts = 0;
        if self.armed && !self.suppressed && self.slot_position >= self.offset {
            self.armed = false;
            starts += 1;
        }
        if scatter.handover > 0.0 && split_mix(rng) < scatter.handover / slot {
            starts += 1;
        }
        self.slot_position += 1.0;
        starts
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Grain {
    age: u32,
    duration: u32,
    position: f32,
    step: f32,
    /// The equal-power gains for this grain's pan, latched at spawn rather than kept as an angle:
    /// a grain's pan is fixed for its life, and the position itself is never read again.
    pan_left: f32,
    pan_right: f32,
}

/// One overlap-add head: where it is reading, and how far through its window it is.
///
/// **The read position is carried directly and advanced per sample**, rather than being
/// reconstructed as `anchor + age × rate`. The two are algebraically the same and the difference
/// is that only one of them is true at note-on: the second head starts half a window into its
/// envelope, so reconstructing its position from its age placed it half a window *ahead of the
/// playhead* for the whole first window — the reader was not an identity at unity even before a
/// note was transposed. Carrying the position means a head's envelope phase and its read position
/// are independent, which is exactly what a transient splice needs to re-lay one without moving
/// the other.
#[derive(Debug, Clone, Copy, Default)]
struct OlaHead {
    /// How far through its envelope, in [0, 1). **A phase, not an age over a duration.** The two
    /// are equivalent while the window is fixed, and only the phase form survives a window that
    /// changes under a sounding note: both heads advance at the same rate, so the half-window
    /// offset that makes the Hann pair sum to one is preserved however often the length moves,
    /// where changing a duration would move a mid-flight head's envelope position and break the
    /// sum. The variable window was measured and rejected; the representation stays because it is
    /// the simpler of the two and it is what let the experiment be run at all.
    phase: f32,
    position: f32,
    /// **Whether this head reads through the loop yet.** A head reads the recording as a one-shot until
    /// it is inside the loop by more than its reads reach, and through the loop from then on. One
    /// anchored by a playhead that has already reached the loop inherits it, so a head the alignment
    /// search places just before Loop start after a wrap reads the loop, not what precedes it. See
    /// [`reaches_loop`].
    looping: bool,
    /// How far across from the recording to the loop this head's reads are. See [`Crossing`].
    crossing: Crossing,
}

#[derive(Debug, Clone)]
struct LayerVoice {
    position: f32,
    travel: f32,
    /// **Whether the playhead — Repitch's position, Stretch's travel — has reached the loop.** Until
    /// it has, the layer plays as a one-shot: a note plays from Start, attack and all. See
    /// [`reaches_loop`].
    looping: bool,
    /// How far across from the recording to the loop Repitch's reads are. See [`Crossing`].
    crossing: Crossing,
    direction: f32,
    ended: bool,
    reader: Reader,
    ola: [OlaHead; 2],
    /// The live grains, **compacted into the prefix `[..live]`**. Nothing past `live` is ever read,
    /// so a dead grain is retired by swapping the last live one over it rather than by clearing a
    /// flag and walking past it forever. That keeps the render walk O(live) over a contiguous run
    /// at any share, which is the property that vectorises.
    grains: [Grain; MAX_GRAINS_PER_LAYER],
    live: usize,
    scheduler: GrainScheduler,
    /// Index of the next onset marker at or after the playhead, in ascending source order. A
    /// cursor rather than a search, because the playhead moves by well under a frame most samples
    /// and a binary search per sample per voice is real money for an answer that is almost always
    /// "no".
    /// The overlap window both heads are currently using, in output samples. **Fixed at
    /// [`STRETCH_WINDOW_S`]**: deriving it from the source's period was built, measured and
    /// rejected — see the crate DOX. It is carried per layer rather than recomputed because the
    /// phase advance and the near-attack test both need it, and because the rejected experiment is
    /// one line away should S3 want it back.
    window: f32,
    /// This layer's share of the start-phase stagger. **Kept, not just applied once**: a transient
    /// splice re-lays both heads, and resetting them to a constant 0.0/0.5 would resynchronise
    /// every voice that crossed the same marker — which, since the playhead travels at the scan
    /// rate and not the note's pitch, is *every voice in a chord, at every attack*. The stagger has
    /// to survive the splice or it only ever holds until the first one.
    stagger: f32,
    /// The region this layer's cursor and pending splice were last synchronised against. **A
    /// region is automatable**, so start, end and both loop points can move under a sounding note:
    /// the playhead is then remapped somewhere the cursor does not describe, and a small apparent
    /// movement would let the walk stride an arbitrary stretch of the onset list and report
    /// crossings that never happened. A change re-seeks and drops any pending splice, which would
    /// otherwise re-lay both heads onto an attack belonging to the old region.
    region_key: [f32; 5],
    /// Distance travelled along an Alternate loop's out-and-back path, in [0, 2 × span). Only
    /// Alternate uses it; folding this is what makes an overshoot of any size reflect correctly.
    travel_raw: f32,
    marker: usize,
    /// How many transient splices this layer has taken. Test-visible so a regression can assert
    /// *how many* times a marker re-laid the heads, which is the claim, rather than that the output
    /// stayed finite, which is not.
    splices: u32,
    /// Set when the playhead crossed an onset during the previous sample's advance, acted on at
    /// the top of the next one. **A flag, so several passages in one rendered sample coalesce into
    /// one splice** — which only happens on a loop shorter than a sample's travel, where the heads
    /// are re-laid onto the newest crossing and splicing once per passage would mean re-laying them
    /// several times inside one sample for no audible gain. One sample of latency, and it is what lets the splice land before
    /// the attack is rendered rather than one window into it.
    pending_splice: bool,
}

impl Default for LayerVoice {
    fn default() -> Self {
        Self {
            position: 0.0,
            travel: 0.0,
            looping: false,
            crossing: Crossing::DONE,
            direction: 1.0,
            ended: false,
            reader: Reader::Repitch,
            ola: [OlaHead::default(); 2],
            grains: [Grain::default(); MAX_GRAINS_PER_LAYER],
            live: 0,
            scheduler: GrainScheduler::default(),
            window: 1.0,
            stagger: 0.0,
            travel_raw: 0.0,
            region_key: [0.0; 5],
            splices: 0,
            marker: 0,
            pending_splice: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Debug, Clone, Copy)]
struct Envelope {
    stage: EnvelopeStage,
    level: f32,
}

impl Default for Envelope {
    fn default() -> Self {
        Self {
            stage: EnvelopeStage::Idle,
            level: 0.0,
        }
    }
}

impl Envelope {
    fn trigger(&mut self) {
        self.stage = EnvelopeStage::Attack;
    }

    fn release(&mut self) {
        if self.stage != EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Release;
        }
    }

    fn next(&mut self, params: &Params, sample_rate: f32) -> f32 {
        let attack = sanitise(params.attack_s, 0.005).clamp(0.000_05, 30.0);
        let decay = sanitise(params.decay_s, 0.18).clamp(0.000_05, 30.0);
        let release = sanitise(params.release_s, 0.35).clamp(0.000_05, 60.0);
        let sustain = sanitise(params.sustain, 0.82).clamp(0.0, 1.0);
        match self.stage {
            EnvelopeStage::Idle => return 0.0,
            EnvelopeStage::Attack => {
                self.level += 1.0 / (attack * sample_rate);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvelopeStage::Decay;
                }
            }
            EnvelopeStage::Decay => {
                self.level -= (1.0 - sustain) / (decay * sample_rate);
                if self.level <= sustain {
                    self.level = sustain;
                    self.stage = EnvelopeStage::Sustain;
                }
            }
            EnvelopeStage::Sustain => self.level = sustain,
            EnvelopeStage::Release => {
                self.level -= 1.0 / (release * sample_rate);
                if self.level <= 0.0 || !self.level.is_finite() {
                    self.level = 0.0;
                    self.stage = EnvelopeStage::Idle;
                }
            }
        }
        self.level
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// `cutoff_hz` is the voice's **effective** cutoff — the patch value after the Shape envelope
    /// and pressure have moved it — not `params.cutoff_hz`, which is only the base.
    fn process(&mut self, input: f32, params: &Params, cutoff_hz: f32, sample_rate: f32) -> f32 {
        if params.filter_mode == FilterMode::Off {
            return input;
        }
        let drive = 1.0 + sanitise(params.filter_drive, 0.0).clamp(0.0, 1.0) * 12.0;
        let driven = soft_clip(input * drive) / soft_clip(drive);
        let cutoff = sanitise(cutoff_hz, 18_000.0).clamp(10.0, sample_rate * 0.45);
        let g = (PI * cutoff / sample_rate).tan().min(12.0);
        let resonance = sanitise(params.resonance, 0.1).clamp(0.0, 0.98);
        let k = 2.0 - resonance * 1.9;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let v1 = a1 * (self.ic1 + g * (driven - self.ic2));
        let v2 = self.ic2 + g * v1;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        let value = match params.filter_mode {
            FilterMode::LowPass => v2,
            FilterMode::BandPass => v1,
            FilterMode::HighPass => driven - k * v1 - v2,
            FilterMode::Off => driven,
        };
        sanitise(value, 0.0)
    }
}

#[derive(Debug, Clone)]
struct Voice {
    active: bool,
    released: bool,
    note: Note,
    born: u64,
    layers: [LayerVoice; 2],
    envelope: Envelope,
    filters: [Svf; 2],
    rng: u64,
    /// This voice's own source frame and per-target route lists.
    ///
    /// **Per voice, which is `plan-modulation-routing.md` §4.1's rule**, and what keeps a latched
    /// key from holding a patch open: the frame ceases with the voice, so a decayed voice is inert
    /// whatever it last published.
    graph: routing::Graph,
    /// One draw, taken when the note starts and held for its life. `routing::source::RANDOM`.
    ///
    /// **A per-note constant rather than a signal**, which is what makes it useful for spreading a
    /// bank of notes. Lifetime is not cadence, so: a stolen voice ends and its replacement draws
    /// afresh, because the replacement is a different note.
    random: f32,
    /// The velocity of **the press that last triggered the envelope**, `0..=1`: the Velocity
    /// source, by the modulation standard. A Legato press that only re-pitches keeps the phrase's.
    velocity: f32,
    /// Note pressure, `0..=1`. A fixed common route to the filter.
    pressure: f32,
    /// Volume expression as a gain ratio, `1.0` being unity.
    expression_gain: f32,
    /// Pan expression, `-1..=1`, applied after the layers own pans.
    expression_pan: f32,
    /// Remaining glide, in semitones, added to this voice's pitch and decaying to zero.
    glide: f32,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            released: false,
            note: Note::midi(60, 0.0),
            born: 0,
            layers: std::array::from_fn(|_| LayerVoice::default()),
            envelope: Envelope::default(),
            filters: [Svf::default(); 2],
            rng: 0x6a09_e667_f3bc_c909,
            graph: routing::Graph::new(),
            random: 0.0,
            velocity: 1.0,
            pressure: 0.0,
            expression_gain: 1.0,
            expression_pan: 0.0,
            glide: 0.0,
        }
    }
}

impl Voice {
    fn start(&mut self, note: Note, born: u64) {
        *self = Self::default();
        self.active = true;
        self.note = note;
        self.born = born;
        self.rng ^= born.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ ((note.key as u64) << 24)
            ^ note.id.unwrap_or(0) as u64;
        self.random = Self::random_for(born, note.key);
        self.velocity = note.velocity;
        self.envelope.trigger();
    }

    /// One press's random draw, from a stream of its own.
    ///
    /// **Not from `self.rng`**: that stream sequences grain onsets, position and pitch draws, and
    /// taking a value from it before any grain read it moved every onset — caught by
    /// `a_cloud_gains_weight_into_the_wall_and_then_holds_it` going red, which is the same defect
    /// the dither streams exist to avoid.
    ///
    /// **A free function of the press**, so the ledger and the voice cannot disagree about what a
    /// press was given: a mono fallback restores a held press's draw, and it has to be the same
    /// number the voice had.
    fn random_for(born: u64, key: u8) -> f32 {
        let mut seed =
            born.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ ((key as u64) << 17) ^ 0xa5a5_5a5a_c3c3_3c3c;
        standard::random(split_mix(&mut seed))
    }

    /// The pitch this voice is sounding right now, in MIDI semitones, glide included.
    fn pitch_semitones(&self) -> f32 {
        self.note.key as f32 + sanitise(self.note.tuning, 0.0) + sanitise(self.glide, 0.0)
    }

    fn reset_layer(&mut self, index: usize, sample: &Sample, params: &LayerParams, host_rate: f32) {
        // The looping region, so the traversal key a note starts with is the one its first sample
        // reads — a seamless key here would read as a moved region on that sample and re-seek.
        let region = Region::looped(sample, params);
        let direction = if params.reverse { -1.0 } else { 1.0 };
        let start = if direction > 0.0 {
            region.start
        } else {
            region.end
        };
        // **A loop with nothing before it is reached on the first sample**, so a note on a
        // whole-region loop reads it from its first read, heads included, exactly as it always did.
        let looping =
            params.loop_mode != LoopMode::Off && reaches_loop(start, direction, region, 0.0);
        let stretch_frames = (STRETCH_WINDOW_S * host_rate).clamp(32.0, 32_768.0);
        let stagger = (((self.rng >> 11) as u32 % 1_000) as f32 / 1_000.0 + index as f32 * 0.37)
            .fract()
            .clamp(0.0, 0.999);
        self.layers[index] = LayerVoice {
            position: start,
            travel: start,
            looping,
            crossing: Crossing::DONE,
            direction,
            ended: false,
            reader: params.reader,
            // Both heads start on the playhead and differ only in envelope phase. The second used
            // to start at `start` with its age already at half a window, which made it read half a
            // window ahead of the playhead until its first re-anchor.
            // **The pair is staggered against other voices, not just against each other.** Every
            // layer used to start at phase 0.0/0.5, so eight notes pressed in one block re-anchored
            // on the same sample for the life of the chord — up to sixteen correlation searches
            // landing together, which is a callback deadline rather than an average. The offset is
            // derived from the voice's own seeded stream and the layer index, so it is spread and
            // still exactly reproducible. Adding the same offset to both heads keeps them half a
            // window apart, which is what the Hann pair summing to one depends on.
            ola: [
                OlaHead {
                    phase: stagger,
                    position: start,
                    looping,
                    crossing: Crossing::DONE,
                },
                OlaHead {
                    phase: (stagger + 0.5).fract(),
                    position: start,
                    looping,
                    crossing: Crossing::DONE,
                },
            ],
            window: stretch_frames,
            stagger,
            travel_raw: (start - region.loop_start).max(0.0),
            region_key: [
                region.start,
                region.end,
                region.loop_start,
                region.loop_end,
                loop_mode_key(params.loop_mode),
            ],
            splices: 0,
            grains: [Grain::default(); MAX_GRAINS_PER_LAYER],
            live: 0,
            // Primed, so the first grain of this layer lands on its first sample rather than
            // somewhere inside the opening slot. See [`GrainScheduler::prime`].
            scheduler: {
                let mut scheduler = GrainScheduler::default();
                scheduler.prime();
                scheduler
            },
            marker: seek_marker(sample.transients(), start),
            pending_splice: false,
        };
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        samples: [Option<&Sample>; 2],
        params: &Params,
        character: &ResolvedCharacter,
        sample_rate: f32,
        share: usize,
        routing: &routing::Routing,
        globals: &[f32; 2],
        performance: &Performance,
    ) -> Stereo {
        if !self.active {
            return Stereo::default();
        }
        // The envelope runs first so a voice that has just closed does no reader or filter work at
        // all, and so the filter can be modulated by this sample's envelope rather than the last.
        let envelope = self.envelope.next(params, sample_rate);
        if self.envelope.stage == EnvelopeStage::Idle {
            self.active = false;
            self.released = false;
            self.layers = std::array::from_fn(|_| LayerVoice::default());
            self.filters = [Svf::default(); 2];
            self.glide = 0.0;
            // **The voice frame ceases with the voice** — §4.1's rule, and what stops a voice
            // starting on this slot from reading the dead voice's latched key or velocity.
            self.graph.reset();
            return Stereo::default();
        }
        self.advance_glide(params, sample_rate);
        let velocity = sanitise(self.velocity, 1.0);

        // ---- Publication, in declared source order ----
        //
        // **Everything this voice publishes goes in before anything reads.** The two audio sources
        // are the exception and are published further down as the layers render, which needs no rule
        // of its own: the frame returns *this* sample's value if this sample produced it and last
        // sample's otherwise, so a target reading a layer earlier in the same sample gets the delayed
        // value by construction. That is the backward route, declared in `routing`.
        //
        // **A source nothing reads is not published.** `Routing::needs` is what makes eleven columns
        // cost only what is wired, and the saving is per voice, so it is eight times this test.
        let routed = routing.any();
        if routed {
            self.graph.begin_sample();
            if routing.needs(routing::source::LFO1) {
                self.graph.write(routing::source::LFO1, globals[0]);
            }
            if routing.needs(routing::source::LFO2) {
                self.graph.write(routing::source::LFO2, globals[1]);
            }
            if routing.needs(routing::source::ENVELOPE) {
                self.graph.write(routing::source::ENVELOPE, envelope);
            }
            // The performance sources through the collection's standard, so each is zero at its
            // rest: Velocity at the hardest note, Key at middle C, a gesture let go.
            if routing.needs(routing::source::VELOCITY) {
                self.graph
                    .write(routing::source::VELOCITY, standard::velocity(velocity));
            }
            if routing.needs(routing::source::KEY) {
                // The glided key, over five octaves either side of middle C, so a cutoff route
                // scaled by `KEY_OCTAVES` is unity keyboard tracking.
                self.graph.write(
                    routing::source::KEY,
                    standard::key(
                        self.note.key as f32 + sanitise(self.glide, 0.0),
                        routing::KEY_UNIT_SEMITONES,
                    ),
                );
            }
            // **Per channel, selected by this voice's own channel.** That is the whole point of
            // projecting rather than passing one scalar: two voices sounding on different channels
            // read different wheels, which a single value silently made impossible.
            let channel = (self.note.channel as usize).min(MIDI_CHANNELS - 1);
            if routing.needs(routing::source::WHEEL) {
                self.graph.write(
                    routing::source::WHEEL,
                    standard::wheel(sanitise(performance.wheel[channel], 0.0)),
                );
            }
            if routing.needs(routing::source::PRESSURE) {
                self.graph.write(
                    routing::source::PRESSURE,
                    standard::pressure(sanitise(self.pressure, 0.0)),
                );
            }
            if routing.needs(routing::source::BEND) {
                self.graph.write(
                    routing::source::BEND,
                    standard::bend(sanitise(performance.bend[channel], 0.0)),
                );
            }
            if routing.needs(routing::source::RANDOM) {
                self.graph.write(routing::source::RANDOM, self.random);
            }
        }

        // **Amplitude is the collection's one law**, `standard::amplitude_factor`: a factor on the
        // envelope, never added to it, so a latched source cannot keep a voice audible after its
        // envelope ends (`plan-modulation-routing.md` §4.1), and bounded silence to double however
        // many routes are summed.
        //
        // With Velocity published as `v − 1`, one route's factor is `1 + amount × (v − 1)`, which
        // is `1 − amount × (1 − v)` term for term — **exactly the velocity-sensitivity law this
        // replaces**. So the route at the old knob's number renders the identical sample, as it did
        // under the product law this factor replaced.
        let velocity_gain = if routed {
            standard::amplitude_factor(self.graph.sum(
                routing::target::AMPLITUDE,
                routing,
                routing::AMPLITUDE_BOUND,
            ))
        } else {
            1.0
        };
        // **Pitch is one target over both layers; scan speed is one per layer.** The layers are two
        // and everything else about a layer already is — and the Travel LFO this replaces pushed
        // them in *opposite* directions with different reaches, which one target cannot express.
        let pitch_offset = if routed {
            self.graph.sum(
                routing::target::PITCH,
                routing,
                routing::PITCH_SEMITONES * 4.0,
            )
        } else {
            0.0
        };
        let scan_targets = [routing::target::SCAN_A, routing::target::SCAN_B];
        let mut rendered = [Stereo::default(); 2];
        // Each layer's mono tap, held until every target for this sample has been read.
        let mut tapped = [0.0f32; 2];
        for index in 0..2 {
            let Some(sample) = samples[index] else {
                continue;
            };
            let mut layer = params.layers[index];
            if routed {
                layer.tune_semitones += pitch_offset;
                // The same `±2` bound the hard-wired Travel LFO was clamped to, so a route cannot
                // reach a speed the reader never had to cope with.
                layer.stretch_travel = (layer.stretch_travel
                    + self.graph.sum(scan_targets[index], routing, 2.0))
                .clamp(-2.0, 2.0);
            }
            let lp = &layer;
            if self.layers[index].reader != lp.reader {
                self.reset_layer(index, sample, lp, sample_rate);
            }
            rendered[index] = self.render_layer(index, sample, lp, character, sample_rate, share);
            // **The audio source is tapped here — before this layer's level and pan — but it is
            // not published here.** A modulation route whose depth changed because the player
            // rebalanced the mix would be a defect, and the converter below is a deliberate
            // nonlinearity whose character does not belong in a control signal. Projected through
            // the sample's own mono mix rather than `(L + R) / 2`, because that sum is **silence on
            // an anti-phase source** — a failure this crate already met once in the alignment
            // search, which is why `Sample` picks its mix at load.
            if routed {
                let mix = sample.mono_mix();
                tapped[index] = rendered[index].left * mix[0] + rendered[index].right * mix[1];
            }
            rendered[index] = pan(rendered[index].gain(lp.level.clamp(0.0, 2.0)), lp.pan);
        }
        // **Two layers sum; each one's Level is what says how loud it is.**
        //
        // There used to be an equal-power Blend across the pair as well, and it was one control too
        // many: at Blend 0 a layer's Level did nothing at all, so the two disagreed about how loud
        // B was. A mixer answers that once. `Level` reaches 200%, so the headroom a crossfade used
        // to take back at centre is the player's to give.
        let mix = rendered[0].add(rendered[1]);
        // The voice's one converter, between the digital section and the filter — where the
        // hardware put its DAC.
        let mix = character.convert(mix);
        let cutoff = self.effective_cutoff(params, routing, routed);
        let filtered = Stereo {
            left: self.filters[0].process(mix.left, params, cutoff, sample_rate),
            right: self.filters[1].process(mix.right, params, cutoff, sample_rate),
        };
        // **Both audio sources are published here, after every target for this sample has been
        // read.** Publishing layer A inside the loop made it *current* for any target read later in
        // the same sample — layer B's scan speed and the cutoff — while `routing`'s declared order
        // says every audio route is backward. One of those had to be wrong, and it was the code:
        // a route is one sample late only if nothing reads the value in the sample that produced it.
        // Doing it once at the end makes the delay uniform, which is what the declared order and the
        // cycle-boundedness argument both rest on.
        if routed {
            for (index, tap) in tapped.iter().enumerate() {
                let audio_source = routing::source::LAYER_A + index;
                if routing.needs(audio_source) {
                    self.graph.write(audio_source, *tap);
                }
            }
        }
        let gain = envelope
            * velocity_gain
            * sanitise(self.expression_gain, 1.0).clamp(0.0, 4.0)
            * params.master.clamp(0.0, 2.0);
        pan(filtered.gain(gain), sanitise(self.expression_pan, 0.0))
    }

    /// The base cutoff moved by the Shape envelope and by pressure, both in octaves.
    ///
    /// Summing in octaves and clamping once at the end is what keeps a closed filter from being
    /// pushed below audibility by one route and dragged back by another: the destination is
    /// resolved in its own physical domain before it is bounded.
    /// The base cutoff moved by everything routed to it, summed in octaves.
    ///
    /// **Summing in octaves and clamping once at the end** is what keeps a closed filter from being
    /// pushed below audibility by one route and dragged back by another: the destination is resolved
    /// in its own physical domain before it is bounded. That was true when this summed two fixed
    /// terms and it is why the target's domain is octaves now.
    ///
    /// The envelope and pressure terms are **routes** rather than fixed paths, and each keeps the
    /// reach it had — `FILTER_ENV_OCTAVES` and `PRESSURE_OCTAVES` are the LFO-and-envelope columns of
    /// `routing::FULL_SCALE`, which is §5's conservative form and why no stored value changed
    /// meaning. Pressure remains available at that reach but is absent from the owner-selected Init.
    fn effective_cutoff(&self, params: &Params, routing: &routing::Routing, routed: bool) -> f32 {
        let base = sanitise(params.cutoff_hz, 18_000.0).clamp(20.0, 22_000.0);
        let octaves = if routed {
            self.graph.sum(routing::target::CUTOFF, routing, 10.0)
        } else {
            0.0
        };
        (base * 2.0f32.powf(octaves.clamp(-10.0, 10.0))).clamp(20.0, 22_000.0)
    }

    /// One glide step. Glide time is per octave, so a wide interval takes proportionally longer
    /// and a semitone is quick — the behaviour a player expects from a portamento control.
    fn advance_glide(&mut self, params: &Params, sample_rate: f32) {
        if self.glide == 0.0 {
            return;
        }
        let seconds = sanitise(params.glide_s, 0.0).clamp(0.0, 5.0);
        if seconds <= 0.0 {
            self.glide = 0.0;
            return;
        }
        let step = 12.0 / (seconds * sample_rate.max(1.0));
        if self.glide.abs() <= step {
            self.glide = 0.0;
        } else {
            self.glide -= step * self.glide.signum();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_layer(
        &mut self,
        index: usize,
        sample: &Sample,
        params: &LayerParams,
        character: &ResolvedCharacter,
        host_rate: f32,
        share: usize,
    ) -> Stereo {
        match params.reader {
            Reader::Repitch => self.repitch(index, sample, params, character, host_rate),
            Reader::Stretch => self.stretch(index, sample, params, character, host_rate),
            Reader::Grain => self.grain(index, sample, params, character, host_rate, share),
        }
    }

    fn pitch_ratio(&self, params: &LayerParams) -> f32 {
        let semitones =
            self.note.key as f32 + sanitise(self.note.tuning, 0.0) + sanitise(self.glide, 0.0)
                - sanitise(params.root_note, 60.0)
                + sanitise(params.tune_semitones, 0.0);
        2.0f32.powf((semitones / 12.0).clamp(-8.0, 8.0))
    }

    fn repitch(
        &mut self,
        index: usize,
        sample: &Sample,
        params: &LayerParams,
        character: &ResolvedCharacter,
        host_rate: f32,
    ) -> Stereo {
        let ratio = self.pitch_ratio(params) * sample.sample_rate() / host_rate;
        let state = &mut self.layers[index];
        if state.ended {
            return Stereo::default();
        }
        let region = Region::looped(sample, params);
        let loops = params.loop_mode != LoopMode::Off;
        if !state.looping
            && loops
            && reaches_loop(
                state.position,
                state.direction,
                region,
                loop_reach(character),
            )
        {
            state.looping = true;
            state.crossing = Crossing::from(state.position);
        }
        let value = loop_read(
            sample,
            state.position,
            region,
            params.loop_mode,
            state.looping,
            state.crossing,
            character,
            (1.0 / ratio.abs().max(1.0)).clamp(0.02, 1.0),
        );
        let (before, heading) = (state.position, state.direction);
        let step = ratio * state.direction;
        state.position += step;
        advance_position(state, region, params.loop_mode);
        if state.looping {
            state.crossing.advance(step, crossing_rate(host_rate));
        } else if loops && (state.direction != heading || (state.position - before) * heading < 0.0)
        {
            // A wrap or a turn is the loop, however short a loop it crossed to get there; the
            // recording carries on from where the step would have taken it.
            state.looping = true;
            state.crossing = Crossing::from(before + step);
        }
        value
    }

    /// Deterministic waveform-aligned overlap-add.
    ///
    /// The order inside one sample is: act on a splice the previous sample asked for, re-anchor any
    /// head whose window has run out, render and advance both heads, then advance the playhead and
    /// look for the next onset. **The playhead advances last on purpose** — a head re-anchors onto
    /// the position the heads are currently reading, and advancing the playhead first would offset
    /// every anchor by one step and break the identity at unity by exactly that much.
    fn stretch(
        &mut self,
        index: usize,
        sample: &Sample,
        params: &LayerParams,
        character: &ResolvedCharacter,
        host_rate: f32,
    ) -> Stereo {
        let region = Region::looped(sample, params);
        let rate = sample.sample_rate() / host_rate;
        let pitch_ratio = self.pitch_ratio(params);
        let loop_mode = params.loop_mode;
        let scan = sanitise(params.stretch_travel, 1.0).clamp(-2.0, 2.0);
        let search = (ALIGN_SEARCH_S * sample.sample_rate())
            .round()
            .clamp(0.0, region.len() * 0.5) as i32;
        let transients = sample.transients();

        let layer = &mut self.layers[index];
        // **Which way the playhead heads**: Scan's sign on the note's direction, and the note's own
        // direction while Scan holds it, so a held reversed note is still approaching from End.
        let heading = if scan < 0.0 {
            -layer.direction
        } else {
            layer.direction
        };
        // **A region that moved invalidates the cursor and any pending splice.** Both describe
        // positions inside the region they were taken in, and start, end and the loop points are
        // all automatable — so a sweep under a sounding note leaves the cursor pointing at markers
        // the playhead is nowhere near, and a pending splice about to re-lay both heads onto an
        // attack from a region that no longer exists.
        // **The loop mode belongs in the key.** Forward and Off do not maintain `travel_raw`, so
        // switching to Alternate under a sounding note would fold a stale one and throw the
        // playhead back near the loop start — a mode change is a traversal discontinuity exactly as
        // a moved region is.
        let key = [
            region.start,
            region.end,
            region.loop_start,
            region.loop_end,
            loop_mode_key(loop_mode),
        ];
        if key != layer.region_key {
            layer.region_key = key;
            // **The playhead is remapped before anything is re-seeked to it.** It was placed under
            // the old region and may now sit outside the new one entirely; seeking the cursor to
            // where it used to be, and then folding it for the walk, puts both in a place the
            // playhead never was — which is how the cursor came to sit two markers from the truth.
            if layer.looping || loop_mode == LoopMode::Off {
                layer.travel = wrap_read(layer.travel, region, loop_mode);
            } else {
                // Not in the loop yet: held in the region, unless the move left it past the loop's
                // far end.
                let (travel, entered) = approach(layer.travel, heading, region, loop_mode);
                layer.travel = travel;
                layer.looping = entered;
            }
            layer.travel_raw = (layer.travel - region.loop_start)
                .rem_euclid(((region.loop_end - region.loop_start).max(1.0e-6)) * 2.0);
            layer.marker = seek_marker(sample.transients(), layer.travel);
            layer.pending_splice = false;
        }
        // **The playhead latches with a search's width to spare**, so a head anchored from a playhead
        // that has reached the loop is inside it by more than its own reach as well.
        let loop_reach_frames = loop_reach(character);
        if !layer.looping
            && loop_mode != LoopMode::Off
            && reaches_loop(
                layer.travel,
                heading,
                region,
                loop_reach_frames + search as f32,
            )
        {
            layer.looping = true;
        }
        // Magnitude and direction are separated, and the direction is carried once — on the
        // playhead and on the read step, never inside a window or a search. §3a.
        let read_step = pitch_ratio * rate * layer.direction;
        let travel_step = rate * scan * layer.direction;
        let bandlimit = (1.0 / read_step.abs().max(1.0)).clamp(0.02, 1.0);
        // **The period narrows the search, and that is all it is for.**
        //
        // Alignment repeats every period, so exactly one good offset lives in any half-period
        // either side of the natural position and a wider search can only find the same answer a
        // cycle away — displaced in time, which is heard as the material jumping. Narrowing is
        // therefore a quality change first and a saving second, and it is both: it is better on
        // every fundamental measured *and* cheaper than the blind range it replaces.
        //
        // **Deriving the window length from the period as well was built, measured and rejected.**
        // S2 set the window to a fixed number of cycles, which is the textbook move and what this
        // plan specified. It measured *worse than a fixed window* at 55 Hz and dearer everywhere —
        // shorter windows re-anchor more often, and each re-anchor is a splice. The window stays
        // fixed; see the crate DOX for the table.
        let period = sample
            .period_at(layer.travel)
            .filter(|frames| frames.is_finite() && *frames > 0.0);
        layer.window = (STRETCH_WINDOW_S * host_rate).clamp(32.0, 32_768.0);
        // Where the track declined there is no period to narrow with, so the blind range stands —
        // which is what keeps noise, drums and chords exactly as S1 left them.
        //
        // **And it stands near an attack too, whatever the track says.** A period is a statement
        // about *periodic* material; across an onset the source is not periodic, and narrowing the
        // search onto the sustain's period there leaves the aligner unable to reach the offset the
        // attack actually needs. Measured on a struck source at +7 st: narrowing everywhere took
        // attack sharpness from 5.81 back to 4.77, below even the clocked splice.
        // **In source frames, because that is what a marker and the playhead are.** `window` is
        // host samples; the span of *source* a head covers in one window is `window × read_step`,
        // and comparing the two directly is only accidentally right when the source and host rates
        // match — which every test here did until one was written that did not.
        let reach = layer.window.max(1.0) * read_step.abs().max(1.0e-6);
        let near_attack = transients
            .get(layer.marker)
            .map(|next| (*next as f32 - layer.travel).abs() < reach)
            .unwrap_or(false)
            || layer
                .marker
                .checked_sub(1)
                .and_then(|previous| transients.get(previous))
                .map(|previous| (layer.travel - *previous as f32).abs() < reach)
                .unwrap_or(false);
        let search = match period {
            // **`min` then `max`, never `clamp`, because these two bounds can cross.**
            // `Region::len` floors at one frame and `MIN_REGION_FRAMES` is 1, so a region whose
            // Start and End meet leaves half a frame to search in — under the one-frame floor.
            // `f32::clamp` asserts `min <= max`, so that combination did not degrade, it panicked,
            // and a panic across the CLAP boundary cannot unwind: the host aborted. Start and End
            // are ordinary automatable controls, so this needed nothing exotic — a sweep that
            // closes the region on a Stretch layer reaches it. `clap-validator`'s
            // `param-fuzz-basic` found it and the other three fuzz tests could not, because they
            // snap every parameter to one end of its range and so never select the middle variant
            // of an enum — `Reader::Stretch` is only reachable from the interior.
            //
            // Ordering them this way also disposes of the NaN case for free: `f32::min`/`max`
            // return the other operand rather than asserting.
            Some(frames) if !near_attack => {
                (frames * 0.5 + 1.0).min(region.len() * 0.5).max(0.0) as i32
            }
            _ => search,
        };
        let span = (ALIGN_SPAN_S * host_rate).min(layer.window * 0.5);

        // 1. A transient crossed during the previous sample re-lays both heads onto the playhead,
        //    one at the start of its envelope and one at the peak of its own. The pair stays
        //    complementary, so the level does not move; what moves is that the attack is now read
        //    from its first frame by a head that has just started, instead of being crossfaded
        //    through by two heads reading elsewhere.
        if layer.pending_splice {
            layer.pending_splice = false;
            // **Only splice onto an attack the heads have not already played.** A head reads at the
            // note's pitch while the playhead travels at the scan rate, so transposed up it runs
            // *ahead* — by up to 40 ms at a fifth. The playhead then crosses the marker long after
            // the heads have passed the attack, and jumping them back to it plays the attack a
            // second time: a flam, which is the artefact this splice exists to prevent. Measured as
            // attack sharpness falling to 4.23 against the source's 4.48 once markers became
            // sample-exact and the double hit lined up.
            // **The guard is only asked the question it can answer.**
            //
            // It exists because a head transposed up runs ahead of the playhead, so the playhead
            // can cross a marker after the heads have already read the attack, and splicing back
            // would play it twice. Deciding that needs "is the head ahead of the marker", and on a
            // loop that is only well defined while the head and the playhead are closer than half
            // the loop — which a microscopic loop, or a large pitch-against-scan ratio, breaks.
            //
            // So it is two rules rather than one guess. Where the loop is comfortably longer than a
            // window the shortest circular distance is the answer and is used. Where it is not, the
            // question is meaningless — a head circles such a loop many times per window and every
            // attack in it is being re-read constantly — and the splice is taken, which is the same
            // thing S1 did before the guard existed.
            // A playhead that has not reached the loop is still a one-shot's.
            let span = match (loop_mode, layer.looping) {
                (LoopMode::Off, _) | (_, false) => region.end - region.start,
                _ => region.loop_span(),
            };
            // **The bound, with its units right.** `span` is source frames and `window` is host
            // samples, so comparing them directly — which the first version did — is not a weaker
            // test, it is a different quantity. A head and the playhead diverge at
            // `|read_step - travel_step|` source frames per sample and are re-anchored together
            // every window, so their separation cannot exceed this; when a whole loop is more than
            // twice that, the shortest way round is provably the right way and `traversal_gap` can
            // be believed.
            let divergence = layer.window.max(1.0) * (read_step - travel_step).abs()
                + layer.window.max(1.0) * read_step.abs();
            // **Alternate is excluded outright.** Its playhead reflects while its heads wrap, so
            // the two travel different paths and "has the head passed this attack" has no clean
            // answer at all — no bound rescues it. Taking the splice there is what S1 did before
            // this guard existed.
            let decidable =
                !(loop_mode == LoopMode::Alternate && layer.looping) && span > 2.0 * divergence;
            let ahead = traversal_gap(
                layer.ola[0].position,
                layer.travel,
                region,
                loop_mode,
                layer.looping && layer.ola[0].looping,
            ) * read_step.signum();
            if !decidable || ahead <= 0.0 {
                // The splice's job is done by both heads landing on the same position — the pair
                // sums to one at any phase, so the attack is read at unit gain whatever the phase
                // is, and keeping the stagger costs the splice nothing.
                layer.splices = layer.splices.wrapping_add(1);
                layer.ola[0].phase = layer.stagger;
                layer.ola[1].phase = (layer.stagger + 0.5).fract();
                layer.ola[0].position = layer.travel;
                layer.ola[1].position = layer.travel;
                layer.ola[0].looping = layer.looping;
                layer.ola[1].looping = layer.looping;
                layer.ola[0].crossing = Crossing::DONE;
                layer.ola[1].crossing = Crossing::DONE;
            }
        }

        // 2. Re-anchor whichever head has run out, aligned to what the other one is still
        //    producing. Suppressed once a non-looping region has ended, so the live heads finish
        //    and nothing new launches past the end.
        for head_index in 0..2 {
            if layer.ola[head_index].phase < 1.0 {
                continue;
            }
            if layer.ended {
                continue;
            }
            let reference = layer.ola[1 - head_index];
            // **Each series in the mode its head reads in**: the other head's own, and the new
            // head's, which is the playhead's — so a splice is scored against exactly what it plays.
            let mode_of = |looping: bool| {
                if looping { loop_mode } else { LoopMode::Off }
            };
            let offset = align_offset(
                sample,
                region,
                mode_of(reference.looping),
                mode_of(layer.looping),
                reference.position,
                layer.travel,
                read_step,
                span,
                search,
            );
            // Subtract rather than zero, so the half-window offset between the pair survives a
            // wrap exactly and the Hann pair keeps summing to one.
            layer.ola[head_index].phase -= 1.0;
            layer.ola[head_index].position = layer.travel + offset;
            layer.ola[head_index].looping = layer.looping;
            // Anchored with its window closed, which is a crossing of its own.
            layer.ola[head_index].crossing = Crossing::DONE;
        }

        // 3. Render and advance the two heads.
        let mut output = Stereo::default();
        let mut weight_sum = 0.0;
        let advance = 1.0 / layer.window.max(1.0);
        let crossing_step = crossing_rate(host_rate);
        for head_index in 0..2 {
            let head = &mut layer.ola[head_index];
            let window = hann(head.phase);
            let (position, looping, crossing) = (head.position, head.looping, head.crossing);
            if looping || loop_mode == LoopMode::Off {
                head.position = wrap_read(head.position + read_step, region, loop_mode);
                head.crossing.advance(read_step, crossing_step);
            } else {
                let unwrapped = head.position + read_step;
                let (moved, entered) = approach(unwrapped, read_step, region, loop_mode);
                head.position = moved;
                if entered || reaches_loop(moved, read_step, region, loop_reach_frames) {
                    head.looping = true;
                    head.crossing = Crossing::from(unwrapped);
                }
            }
            head.phase += advance;
            if window <= 0.0 {
                continue;
            }
            output = output.add(
                loop_read(
                    sample, position, region, loop_mode, looping, crossing, character, bandlimit,
                )
                .gain(window),
            );
            weight_sum += window;
        }

        // 4. Advance the playhead, and settle what the region boundary does. A complementary Hann
        //    pair sums to one, so the normalisation is a no-op in normal operation; it earns its
        //    keep only in the tail of a region that has ended, where both windows are closing.
        let previous = layer.travel;
        let crossed;
        let (low, high) = (region.loop_start, region.loop_end);
        // The far edge the playhead reaches going up: a forward loop's half-open span ends one frame
        // past Loop end, while Alternate turns on Loop end itself.
        let far_high = if loop_mode == LoopMode::Forward {
            region.loop_limit()
        } else {
            high
        };
        let limit = travel_step.abs() * MARKER_JUMP_SLACK + MARKER_JUMP_SLACK;
        // **Still approaching a loop it has not reached** — before it on a first pass, past it on a
        // reversed one, or holding. A playhead heading away from a loop still ahead of it has nothing
        // to walk through, and folds into the loop as it always did.
        let approaching = !layer.looping
            && loop_mode != LoopMode::Off
            && if heading >= 0.0 {
                previous <= far_high
            } else {
                previous >= low
            };
        let beyond = if heading >= 0.0 {
            previous + travel_step > far_high
        } else {
            previous + travel_step < low
        };
        let mut entry_crossed = false;
        if !layer.ended && approaching && !beyond {
            // **Before the loop the playhead walks as a one-shot's does**, from Start into the loop,
            // and folds only once it has reached it. Folding it straight away skipped the frames
            // before Loop start, attack and all; a zero Scan now holds it at Start.
            layer.travel = (previous + travel_step).clamp(region.start, region.end);
            crossed = advance_marker(transients, layer, previous, layer.travel, limit);
            if reaches_loop(
                layer.travel,
                heading,
                region,
                loop_reach_frames + search as f32,
            ) {
                layer.looping = true;
                // The out-and-back distance at entry, on its rising half, so a step either way moves
                // the playhead the way the step goes.
                layer.travel_raw = (layer.travel - low).clamp(0.0, (high - low).max(1.0e-6));
            }
            layer.pending_splice = crossed;
        } else if !layer.ended {
            // **A playhead that crosses a whole short loop on its way in** — a loop shorter than one
            // sample's travel — walks the recording to the far end first, so an attack it passed on
            // the way is still reported, and folds only the rest of the step from the seam there.
            let (previous, travel_step) = if approaching {
                let far = if heading >= 0.0 { far_high } else { low };
                entry_crossed = advance_marker(transients, layer, previous, far, limit);
                if loop_mode == LoopMode::Alternate {
                    // The out-and-back path turns at the far end: Loop end is half its length.
                    layer.travel_raw = if heading >= 0.0 {
                        (high - low).max(1.0e-6)
                    } else {
                        0.0
                    };
                }
                let seam = match (loop_mode, heading >= 0.0) {
                    (LoopMode::Forward, true) => low,
                    (LoopMode::Forward, false) => far_high,
                    _ => far,
                };
                layer.marker = seek_marker(transients, seam);
                (far, previous + travel_step - far)
            } else {
                (previous, travel_step)
            };
            if loop_mode != LoopMode::Off {
                layer.looping = true;
            }
            layer.travel = previous + travel_step;
            match loop_mode {
                LoopMode::Off => {
                    if layer.travel > region.end || layer.travel < region.start {
                        layer.travel = layer.travel.clamp(region.start, region.end);
                        layer.ended = true;
                    }
                    crossed = advance_marker(
                        transients,
                        layer,
                        previous,
                        layer.travel,
                        travel_step.abs() * MARKER_JUMP_SLACK + MARKER_JUMP_SLACK,
                    );
                }
                // **Alternate turns the playhead around; it does not turn a head around.** The
                // reader has always read its windows in one direction — `stretch` passed
                // `LoopMode::Forward` to every read before S1 — and reversing an individual head
                // mid-window would need a per-head direction with its own wrap. What alternates is
                // the traversal, which is what the control names. Without this, Alternate was
                // silently Forward for Stretch alone, while Repitch ping-ponged.
                LoopMode::Alternate => {
                    // **The traversal ping-pongs; the read direction does not move.** Flipping
                    // `layer.direction` here would have reversed `read_step` too, turning every
                    // live head around mid-window — the opposite of the contract, and a different
                    // sound from the one the control names. The sweep is carried by an unwrapped
                    // distance folded into a triangle, which reflects correctly however far a
                    // single step overshoots: a reflect-once-then-clamp is wrong the moment a step
                    // spans more than one loop length, which a microscopic loop makes ordinary.
                    let span = (high - low).max(1.0e-6);
                    let before_raw = layer.travel_raw.rem_euclid(span * 2.0);
                    let fold = |raw: f32| {
                        if raw <= span {
                            low + raw
                        } else {
                            low + (span * 2.0 - raw)
                        }
                    };
                    crossed = walk_traversal(
                        transients,
                        layer,
                        before_raw,
                        travel_step,
                        span,
                        span * 2.0,
                        fold,
                    );
                    layer.travel_raw = (before_raw + travel_step).rem_euclid(span * 2.0);
                    layer.travel = fold(layer.travel_raw).clamp(low, high);
                }
                LoopMode::Forward => {
                    let span = region.loop_span();
                    let before_raw = (previous - low).rem_euclid(span);
                    crossed = walk_traversal(
                        transients,
                        layer,
                        before_raw,
                        travel_step,
                        span,
                        span,
                        |raw: f32| low + raw,
                    );
                    layer.travel = low + (before_raw + travel_step).rem_euclid(span);
                }
            }
            layer.pending_splice = crossed || entry_crossed;
        }

        if weight_sum > 1.0e-3 {
            output.gain((1.0 / weight_sum).min(2.0))
        } else {
            Stereo::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn grain(
        &mut self,
        index: usize,
        sample: &Sample,
        params: &LayerParams,
        character: &ResolvedCharacter,
        host_rate: f32,
        share: usize,
    ) -> Stereo {
        let region = Region::new(sample.len(), params);
        // **Grain reads loop-blind**: its reads wrap at the region's edges, which is all its
        // `LoopMode::Forward` ever meant. Handing it the player's loop points wrapped a grain before
        // Loop start into the loop, so a cloud near Start read loop material instead of the attack.
        let reads = Region {
            loop_start: region.start,
            loop_end: region.end,
            ..region
        };
        let density = sanitise(params.grain_density_hz, 14.0).clamp(0.5, MAX_GRAIN_DENSITY_HZ);
        // Travel is a continuous playhead that onsets sample, advanced per sample exactly as
        // Stretch advances it. Advancing it per spawn instead stalls the read position whenever the
        // pool is saturated and every spawn is dropped -- which is precisely the dense setting.
        let travel_step = sample.sample_rate() / host_rate
            * sanitise(params.stretch_travel, 1.0).clamp(-2.0, 2.0)
            * self.layers[index].direction;
        let scatter = Scatter::from_control(params.grain_onset_timing);
        let slot = host_rate / density;
        let starts = self.layers[index]
            .scheduler
            .tick(slot, scatter, &mut self.rng);
        for _ in 0..starts {
            self.spawn_grain(index, sample, params, host_rate, region, share);
        }
        self.layers[index].travel += travel_step;
        wrap_travel(&mut self.layers[index].travel, region);
        let mut output = Stereo::default();
        let layer = &mut self.layers[index];
        let mut slot = 0usize;
        while slot < layer.live {
            let grain = &mut layer.grains[slot];
            let phase = grain.age as f32 / grain.duration.max(1) as f32;
            if phase >= 1.0 {
                // Retire by swapping the last live grain into this slot, so the live run stays
                // contiguous and the walk never pays for a dead grain again. Do not advance `slot`:
                // the grain just swapped in has not been rendered yet.
                layer.live -= 1;
                layer.grains[slot] = layer.grains[layer.live];
                continue;
            }
            let window = grain_window(params.grain_shape, phase);
            let value = read(
                sample,
                grain.position + grain.age as f32 * grain.step,
                reads,
                LoopMode::Forward,
                character,
                (1.0 / grain.step.abs().max(1.0)).clamp(0.02, 1.0),
            );
            let windowed = value.gain(window);
            output = output.add(Stereo {
                left: windowed.left * grain.pan_left,
                right: windowed.right * grain.pan_right,
            });
            grain.age += 1;
            slot += 1;
        }
        if layer.live == 0 {
            return output;
        }
        output.gain(normalisation(params, density))
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_grain(
        &mut self,
        index: usize,
        sample: &Sample,
        params: &LayerParams,
        host_rate: f32,
        region: Region,
        share: usize,
    ) {
        // The share is this layer's slice of the instrument-wide budget. A layer only ever spawns
        // while it is under its share, and the share only rises when a voice deactivates — which
        // already clears that voice's grains — so the sum across layers never exceeds
        // [`GRAIN_BUDGET`], transients included.
        if self.layers[index].live >= share {
            return;
        }
        let slot = self.layers[index].live;
        // **The position draw is what decorrelates a cloud.** Grains that read the same material at
        // the same rate sum coherently however their onsets are placed — overlapping identical
        // copies at regular intervals is a comb filter, not a cloud — so this is the one control
        // that makes many grains sound like many sources.
        let position = sanitise(params.grain_position_variation, 0.0).clamp(0.0, 1.0);
        let position_jitter = self.random_bipolar() * position * region.len() * 0.35;
        // **Detune is Spread's, not Scatter's.** Welding them meant a cloud could not be thickened
        // into a section of players without also being thrown about in position, direction and
        // pan — and thickening is what a granular sampler is reached for. Spread is in semitones
        // because that is what a detune is heard as.
        let spread = sanitise(params.grain_spread_semitones, 0.0).clamp(0.0, MAX_GRAIN_SPREAD_ST);
        let pitch_jitter = self.random_bipolar() * spread / 12.0;
        // **Width and reversal are their own controls**, because throwing a cloud across the field
        // and running grains backwards are two sounds you choose, not the toll for decorrelating
        // one. Reverse chance is a plain probability rather than a law that only opens past the
        // halfway point of a shared knob.
        let width = sanitise(params.grain_stereo_width, 0.0).clamp(0.0, 1.0);
        let pan = self.random_bipolar() * width;
        let gains = pan_gains(pan);
        let reverse =
            self.random_unit() < sanitise(params.grain_reverse_chance, 0.0).clamp(0.0, 1.0);
        let duration = (sanitise(params.grain_size_s, 0.09).clamp(0.008, 1.0) * host_rate)
            .round()
            .max(8.0) as u32;
        let direction = if params.reverse ^ reverse { -1.0 } else { 1.0 };
        let pitch =
            self.pitch_ratio(params) * 2.0f32.powf(pitch_jitter) * sample.sample_rate() / host_rate;
        let centre = self.layers[index].travel + position_jitter;
        let position = region.wrap(centre);
        self.layers[index].grains[slot] = Grain {
            age: 0,
            duration,
            position,
            step: direction * pitch,
            pan_left: gains.0,
            pan_right: gains.1,
        };
        self.layers[index].live += 1;
    }

    fn random_unit(&mut self) -> f32 {
        split_mix(&mut self.rng)
    }

    fn random_bipolar(&mut self) -> f32 {
        self.random_unit() * 2.0 - 1.0
    }
}

/// One draw from a voice's seeded stream.
///
/// Free rather than a method so the grain scheduler can draw from the same stream while the layer
/// it schedules for is already mutably borrowed. SplitMix64: fixed arithmetic and chronology make
/// callback partition irrelevant.
#[inline]
fn split_mix(state: &mut u64) -> f32 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u32 << 24) as f32
}

#[derive(Debug, Clone, Copy)]
struct Region {
    start: f32,
    end: f32,
    /// Where a looping read returns to. **With a crossfade that had to borrow room from inside the
    /// loop, this is the moved return point rather than the player's Loop start** — every wrap in the
    /// crate reads this one field, so they all agree on one loop. See [`Seam::fit`].
    loop_start: f32,
    /// **The loop's last frame.** A loop is the half-open span from Loop start to one frame past
    /// this — see [`Region::loop_span`].
    loop_end: f32,
    /// How a looping reader's taps wrap: the loop's exact span, and a forward loop's crossfade.
    /// `None` wherever nothing reads the loop — a one-shot, Grain's loop-blind reads, or a loop
    /// shorter than [`MIN_SEAM_LOOP_FRAMES`] — and the fetch then takes the plain whole-frame path.
    seam: Option<Seam>,
}

impl Region {
    fn new(len: usize, params: &LayerParams) -> Self {
        let last = len.saturating_sub(1) as f32;
        let start_n = sanitise(params.start, 0.0).clamp(0.0, 1.0);
        let end_n = sanitise(params.end, 1.0).clamp(0.0, 1.0);
        let mut start = start_n.min(end_n) * last;
        let mut end = start_n.max(end_n) * last;
        if end - start < MIN_REGION_FRAMES as f32 && last > 0.0 {
            end = (start + MIN_REGION_FRAMES as f32).min(last);
            start = (end - MIN_REGION_FRAMES as f32).max(0.0);
        }
        let ls_n = sanitise(params.loop_start, start_n).clamp(0.0, 1.0);
        let le_n = sanitise(params.loop_end, end_n).clamp(0.0, 1.0);
        let loop_start = settle_on_frame(ls_n.min(le_n), last).clamp(start, end);
        // **Equal points are a one-frame loop.** The span is half-open, so a Loop end naming Loop
        // start's own frame loops that frame. When the span was the points' difference, equal points
        // were widened by a frame to give the loop any length at all, which now played two.
        let loop_end = settle_on_frame(ls_n.max(le_n), last).clamp(start, end);
        Self {
            start,
            end,
            loop_start,
            loop_end,
            seam: None,
        }
    }

    /// The region a **looping reader** reads: [`Region::new`], plus the forward loop's crossfade.
    ///
    /// Grain does not come here. It schedules inside the region and never reads the loop, so it
    /// takes the seamless region and renders the same sample at every crossfade.
    fn looped(sample: &Sample, params: &LayerParams) -> Self {
        let region = Self::new(sample.len(), params);
        match params.loop_mode {
            LoopMode::Off => region,
            LoopMode::Forward => {
                Seam::fit(region, crossfade_seconds(params) * sample.sample_rate())
            }
            // No crossfade, but its taps past either end wrap by the same exact span.
            LoopMode::Alternate => Seam::fit(region, 0.0),
        }
    }

    /// **One loop, in source frames: from Loop start to one frame past Loop end**, fractional where
    /// the points are. Loop end names the loop's last frame, as a WAV loop does, so a 169-frame cycle
    /// looped over frames 0..168 repeats every 169 frames.
    ///
    /// The float wraps used a period of `loop_end - loop_start` while the taps wrapped over the
    /// inclusive span, one frame more: inaudible on the buffers every test used, and on a single
    /// cycle 10 cents sharp with a frame skipped every pass. `plans/plan-sampler-wav-loop-import.md`
    /// §1.1 moved every wrap onto this span.
    fn loop_span(self) -> f32 {
        (self.loop_end + 1.0 - self.loop_start).max(1.0)
    }

    /// Where the loop's half-open span ends: one frame past Loop end.
    fn loop_limit(self) -> f32 {
        self.loop_start + self.loop_span()
    }

    /// `position` folded into the loop's span.
    fn wrap_loop(self, position: f32) -> f32 {
        let span = self.loop_span();
        let wrapped = self.loop_start + (position - self.loop_start).rem_euclid(span);
        // `rem_euclid` can round up onto the span itself.
        if wrapped >= self.loop_start + span {
            wrapped - span
        } else {
            wrapped
        }
    }

    fn len(self) -> f32 {
        (self.end - self.start).max(1.0)
    }

    fn wrap(self, position: f32) -> f32 {
        let len = self.len();
        self.start + (position - self.start).rem_euclid(len)
    }
}

/// A normalised loop point **that is exactly what writing a whole frame produces** is that frame.
///
/// A loop written from a file's frames travels through a normalised `f32`, which cannot place every
/// frame of a long file: `k / last * last` lands thousandths of a frame off `k` on a 90 000-frame file
/// and a third of a frame off on the longest the importer accepts. Off by any amount, the point is
/// fractional and every wrapped tap interpolates, so a file's loop would not play its own frames.
/// `plans/plan-sampler-wav-loop-import.md` §3.
///
/// **The test is identity, not nearness.** The first version settled anything within `last × ε` of a
/// frame, which at the importer's cap is 0.69 of a frame: every point there was rounded, however
/// deliberately fractional (code review round 1). A value bit for bit `k / last` came from frame `k`,
/// or from a point the parameter cannot tell from it, and anything else stays where it lies. The
/// nearest frame is the only candidate, because the two roundings move a point by under half a frame
/// at every length the importer accepts.
fn settle_on_frame(normalised: f32, last: f32) -> f32 {
    let position = normalised * last;
    let frame = position.round();
    if last > 0.0 && frame / last == normalised {
        frame
    } else {
        position
    }
}

/// A forward loop's crossfade, in source frames.
///
/// **The seam is a property of the frames read, not of the reader reading them.** Every frame inside
/// the fade is a blend of itself and the frame exactly one loop period away, and the loop is extended
/// periodically across the seam by that exact period. Every reader that meets the seam — Repitch
/// either way round, Stretch's heads and its alignment search, the kernel's taps and Character's hold
/// path — meets it through the frames it fetches, so a periodic signal with no seam of its own is
/// what they all read, whichever way they cross it. `plans/plan-sampler-loop-crossfade.md` §2.1.
///
/// **A launched tail reader was the alternative, and it is per-reader logic in disguise**: a second
/// head per layer, a policy for a second wrap before the first tail finishes, a separate mechanism
/// for reverse travel, and a third for Stretch heads that wrap on their own schedule.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Seam {
    /// Frames faded before the seam, ending one frame past Loop end, with the loop's own opening
    /// fading in over them. **Zero is the hard wrap**: the loop still wraps by its exact span.
    length: f32,
    /// One loop, exactly: the half-open span from the return point to one frame past Loop end,
    /// fractional.
    period: f32,
    /// The player's own Loop start, before the fade moved the return point in from it.
    plain_loop_start: f32,
}

/// The shortest loop a wrap geometry is built for, in source frames. Below two frames there is no
/// seam to speak of, and the fetch takes the plain whole-frame path — which, for a one-frame loop on
/// a whole frame, wraps by the same single frame the positions do.
const MIN_SEAM_LOOP_FRAMES: f32 = 2.0;

impl Seam {
    /// Fits a fade of `requested` source frames into `region`'s loop, and returns the region the
    /// readers wrap on.
    ///
    /// **The fade takes its material from inside the loop, and nowhere else** (owner, 2026-09-14).
    /// What precedes Loop start is the note's attack — a guitar string looped for sustain is exactly
    /// the case — and a fade that read it would replay the attack into every pass. So the fade that
    /// ends one frame past Loop end blends in the loop's own opening, and after the seam the loop returns to where
    /// that opening ends: **Loop start plus the fade**. Each pass is shorter by the fade, and the
    /// frames it skips are heard as the fade-in rather than lost.
    ///
    /// The first version took room from outside the loop first and borrowed from inside only for the
    /// shortfall. That read the attack into the fade, and on a whole-sample loop it moved the return
    /// point past the very frames a note starts on.
    ///
    /// The fade and the loop it shortens cannot overlap, so a fade reaches half the loop at most.
    /// Every quantity here is continuous in every input, so sweeping a loop point or the time moves
    /// the fade smoothly.
    ///
    /// **Every looping region gets one, fade or not**: with no fade it is the loop's exact span and
    /// nothing else, so a loop whose points fall between frames wraps its taps by the same span its
    /// positions wrap by. The hard wrap is this seam at zero, and a fade shortening to zero converges
    /// on it — which is why the easing from a separate whole-frame hard wrap, which revision 8 of the
    /// crossfade plan needed to keep zero bit-identical, is gone.
    fn fit(mut region: Region, requested: f32) -> Region {
        let span = region.loop_span();
        if span < MIN_SEAM_LOOP_FRAMES {
            return region;
        }
        let length = if requested > 0.0 {
            requested.min(span * 0.5)
        } else {
            0.0
        };
        let plain_loop_start = region.loop_start;
        region.loop_start = plain_loop_start + length;
        region.seam = Some(Seam {
            length,
            period: span - length,
            plain_loop_start,
        });
        region
    }

    /// The virtual loop signal at whole frame `index`.
    ///
    /// **Two raw fetches, never two wrapped ones.** A frame one period away fetched through the loop
    /// wrap would land on the very frame it is meant to differ from, and the fade would blend a signal
    /// with itself and render nothing. So the wrap here is arithmetic on the index and fetches
    /// nothing, and the blend reads the recording clamped to the region exactly as a non-looping read
    /// does. Nothing a blend reads is itself blended.
    ///
    /// **Under the kernel, not above it.** Blending two already-interpolated reads would need the
    /// kernel's taps to stop wrapping whenever the fade is non-zero, which changes what the taps
    /// across the seam read the instant the fade leaves zero, and turns a fade shorter than the kernel
    /// into a hard cut between two band-limited reads. Interpolating over blended frames keeps the
    /// kernel reading one continuous signal and gives Character's hold path one grid to sample.
    ///
    /// **The period is exact, so it slides.** Loop points are fractional and automatable; a period
    /// rounded to whole frames would move every blended frame onto its neighbour at once each time the
    /// loop length crossed a frame, which on a slow sweep is a zipper across the whole fade.
    ///
    /// **With whole-frame loop points every wrapped index lands on a frame**, and the fetch is exactly
    /// the integer wrap this crate always made; only points between frames interpolate.
    fn frame(&self, sample: &Sample, index: i64, region: Region) -> Stereo {
        self.value(sample, index as f32, region)
    }

    fn value(&self, sample: &Sample, position: f32, region: Region) -> Stereo {
        // The loop's half-open span, from the return point to one frame past Loop end.
        let low = region.loop_start;
        let high = low + self.period;
        let mut at = position;
        if !(at >= low && at < high) {
            at = low + (at - low).rem_euclid(self.period);
            // `rem_euclid` can round up onto the period itself.
            if at >= high {
                at -= self.period;
            }
        }
        let fade_out = high - self.length;
        if self.length > 0.0 && at >= fade_out {
            // Before the seam: this frame leaves, and the one a period earlier — the loop's own
            // opening, between Loop start and the return point — arrives.
            let rise = seam_rise((at - fade_out) / self.length);
            mix(
                raw_read(sample, at, region),
                raw_read(sample, at - self.period, region),
                rise,
            )
        } else {
            raw_read(sample, at, region)
        }
    }

    /// Whether whole frames `first..=last` read the recording untouched — inside the loop and clear
    /// of the fade — so the kernel may take them as one contiguous run.
    fn clear(&self, first: i64, last: i64, region: Region) -> bool {
        first as f32 >= region.loop_start
            && (last as f32) < region.loop_start + self.period - self.length
    }
}

/// The crossfade time a layer asks for, bounded here rather than trusted from the caller.
fn crossfade_seconds(params: &LayerParams) -> f32 {
    sanitise(params.loop_crossfade_s, 0.0).clamp(0.0, MAX_LOOP_CROSSFADE_S)
}

/// The crossfade's incoming gain at `ramp` through the fade: `sin²(ramp·π/2)`.
///
/// **Amplitude-complementary**: its outgoing partner is `cos²`, and the pair sums to one everywhere.
/// That is Stretch's own head pair, and it is the shared Hann table read at half the ramp, so it costs
/// no table. It errs coherent — material worth looping on is usually similar at both ends, where an
/// equal-power law would put +3 dB in the middle of every fade on an instrument with no limiter.
#[inline]
fn seam_rise(ramp: f32) -> f32 {
    hann(sanitise(ramp, 0.0).clamp(0.0, 1.0) * 0.5)
}

/// The recording at a fractional source position, clamped to the region, between the two frames
/// either side. **Loop-blind by design**; see [`Seam::frame`].
fn raw_read(sample: &Sample, position: f32, region: Region) -> Stereo {
    let base = position.floor();
    let below = frame_plain(sample, base as i64, region, LoopMode::Off);
    let fraction = position - base;
    if fraction <= 0.0 {
        return below;
    }
    let above = frame_plain(sample, base as i64 + 1, region, LoopMode::Off);
    mix(below, above, fraction)
}

/// `from` crossfaded toward `to` by `amount`.
#[inline]
fn mix(from: Stereo, to: Stereo, amount: f32) -> Stereo {
    Stereo {
        left: from.left + (to.left - from.left) * amount,
        right: from.right + (to.right - from.right) * amount,
    }
}

/// Where a layer's forward-loop crossfade lies, for a display that must not drift from the audio.
///
/// Positions and lengths are normalised over the sample, exactly as the loop parameters are.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LoopCrossfade {
    /// The time asked for, in seconds of source.
    pub requested_s: f32,
    /// The time the loop can hold, in seconds of source — at most half the loop. **Zero where the
    /// fade is inert**: a loop that is not Forward, or a Grain layer, which never reads the loop.
    pub effective_s: f32,
    /// Loop start, where the fade-in's material begins.
    pub loop_start: f32,
    /// Where the loop returns to after the seam: Loop start plus the fade, whose frames are heard as
    /// the fade-in. Equal to `loop_start` where there is no fade.
    pub return_point: f32,
    /// Loop end, where the fade ends. The fade out is the same length as the fade-in, before it.
    pub loop_end: f32,
}

/// The crossfade the engine plays for `params` on `sample`, from **the same arithmetic the readers
/// use** rather than a copy of it — `character_rate_divisor`'s arrangement — so the waveform cannot
/// draw a fade the audio does not make.
pub fn loop_crossfade(sample: &Sample, params: &LayerParams) -> LoopCrossfade {
    let region = if params.reader == Reader::Grain {
        Region::new(sample.len(), params)
    } else {
        Region::looped(sample, params)
    };
    let last = sample.len().saturating_sub(1).max(1) as f32;
    let rate = sample.sample_rate();
    LoopCrossfade {
        requested_s: crossfade_seconds(params),
        effective_s: region.seam.map_or(0.0, |seam| seam.length / rate),
        loop_start: region
            .seam
            .map_or(region.loop_start, |seam| seam.plain_loop_start)
            / last,
        return_point: region.loop_start / last,
        loop_end: region.loop_end / last,
    }
}

/// A read by a reader that loops — Repitch or Stretch — at `position`.
///
/// **A reader reads the recording until it has reached the loop, and through the loop from then
/// on** — `looping`, latched per reader by [`reaches_loop`]. Until 2026-09-14 every read wrapped into
/// the loop, so whatever preceded Loop start was replaced by loop material from the note's first
/// sample; the owner heard it as a guitar string with no attack.
/// `a_looping_note_plays_what_precedes_the_loop_exactly_as_a_one_shot_does` holds it.
///
/// **A latch, not a position test**, which is what the first repair used and a code review took
/// apart: from its position alone a read cannot tell a first pass approaching Loop start from a
/// Stretch head the search anchored a few milliseconds before it just after a wrap — and that head
/// must read the loop, not the attack.
#[allow(clippy::too_many_arguments)]
fn loop_read(
    sample: &Sample,
    position: f32,
    region: Region,
    loop_mode: LoopMode,
    looping: bool,
    crossing: Crossing,
    character: &ResolvedCharacter,
    bandlimit: f32,
) -> Stereo {
    if !looping {
        return read(
            sample,
            position,
            region,
            LoopMode::Off,
            character,
            bandlimit,
        );
    }
    let looped = read(sample, position, region, loop_mode, character, bandlimit);
    if crossing.progress >= 1.0 {
        return looped;
    }
    let plain = read(
        sample,
        crossing.continuation,
        region,
        LoopMode::Off,
        character,
        bandlimit,
    );
    // Exact where the two agree, signed zero included, so a crossing on a loop long enough to hold
    // a read changes no bit of what the one-shot note played.
    let across = |from: f32, to: f32| {
        if to == from {
            from
        } else {
            from + (to - from) * crossing.progress
        }
    };
    Stereo {
        left: across(plain.left, looped.left),
        right: across(plain.right, looped.right),
    }
}

/// **A reader crossing from the recording into the loop, once it has latched.**
///
/// Where the reader is inside the loop by more than a read reaches, a looped read and a plain one
/// fetch the same frames and latching changes nothing. **Some loops have no such place**: a loop
/// shorter than twice a read's reach — tens of frames at a coarse Character grid — or a fade that
/// leaves no room between its return point and its fade-out. There latching switched every read in
/// one sample, a step, which a code review found against the claim that the entry could not click.
/// So for [`LOOP_ENTRY_S`] after it latches, a reader reads both — the recording carried on unwrapped
/// from where it latched, exactly as the one-shot note would have played it, and the loop — and
/// fades from one to the other. On any loop long enough the two agree and the crossing is exact to
/// the bit.
#[derive(Debug, Clone, Copy)]
struct Crossing {
    /// 0 where the reader latched, 1 once it reads the loop alone.
    progress: f32,
    /// Where the recording the reader was playing has got to, unwrapped, in source frames.
    continuation: f32,
}

impl Crossing {
    /// Reading the loop alone: a reader latched on its first sample, or anchored with its window
    /// closed, which is a crossing of its own.
    const DONE: Self = Self {
        progress: 1.0,
        continuation: 0.0,
    };

    /// A crossing that starts with the recording at `continuation`.
    fn from(continuation: f32) -> Self {
        Self {
            progress: 0.0,
            continuation,
        }
    }

    /// One sample on: the recording moves by the reader's own step, and the fade by `rate`.
    fn advance(&mut self, step: f32, rate: f32) {
        if self.progress < 1.0 {
            self.continuation += step;
            self.progress = (self.progress + rate).min(1.0);
        }
    }
}

impl Default for Crossing {
    fn default() -> Self {
        Self::DONE
    }
}

/// How long a reader takes to cross from the recording into the loop once it has latched. Long
/// enough that a switch between two different reads is a fade, not a step; short enough to be over
/// long before the reader reaches the far side of any loop it is needed on.
const LOOP_ENTRY_S: f32 = 0.002;

/// A crossing's progress per host sample.
fn crossing_rate(host_rate: f32) -> f32 {
    1.0 / (LOOP_ENTRY_S * host_rate).max(1.0)
}

/// How far a read reaches either side of its position, in source frames: half the kernel, and the
/// Character hold path's cell and clock wander.
fn loop_reach(character: &ResolvedCharacter) -> f32 {
    SINC_TAPS as f32 / 2.0 + 2.0 + character.grid + character.jitter
}

/// Whether a reader at `position`, heading the way `heading`'s sign says, has reached the loop.
///
/// **Reached means inside it by more than `margin`**, the reach of every read. At that moment a read
/// through the loop and a read of the recording fetch the same frames, so the latch switches nothing
/// audible — no click as a note enters a loop whose ends do not meet. A reversed reader must also be
/// past the fade, which lies at the loop's far end from its side.
///
/// **A loop with nothing before it on the approach side is reached from the first sample.** A note
/// starting where its loop starts reads the loop at once, exactly as it always has, so a patch whose
/// loop is its whole region takes none of this and renders as it did.
fn reaches_loop(position: f32, heading: f32, region: Region, margin: f32) -> bool {
    let fade = region.seam.map_or(0.0, |seam| seam.length);
    let room = (region.loop_span() - fade).max(0.0);
    let margin = margin.min(room * 0.5);
    if heading >= 0.0 {
        region.loop_start <= region.start
            || (position >= region.loop_start + margin && position < region.loop_limit())
    } else {
        (region.loop_end >= region.end && fade <= 0.0)
            || (position <= region.loop_limit() - fade - margin && position >= region.loop_start)
    }
}

/// Where a reader that has not reached the loop lands after moving to `position`, and whether the move
/// took it into the loop.
///
/// **Held in the region while the loop is still ahead**, so it walks in on its own. Past the loop's
/// far end in the direction it travels — a loop shorter than one step crossed whole, or a reader
/// heading away from a loop still ahead of it — it wraps into the loop exactly as [`wrap_read`]
/// always did, and has entered it.
fn approach(position: f32, heading: f32, region: Region, loop_mode: LoopMode) -> (f32, bool) {
    let past = if heading >= 0.0 {
        position >= region.loop_limit()
    } else {
        position < region.loop_start
    };
    if past {
        (wrap_read(position, region, loop_mode), true)
    } else {
        (position.clamp(region.start, region.end), false)
    }
}

fn advance_position(state: &mut LayerVoice, region: Region, loop_mode: LoopMode) {
    let (start, end) = if loop_mode == LoopMode::Off {
        (region.start, region.end)
    } else {
        (region.loop_start, region.loop_end)
    };
    // A forward loop's span is half-open, one frame past Loop end; Alternate turns on Loop end.
    let past_end = if loop_mode == LoopMode::Forward {
        state.position >= region.loop_limit()
    } else {
        state.position > end
    };
    if state.direction > 0.0 && past_end {
        match loop_mode {
            LoopMode::Off => {
                state.position = end;
                state.ended = true;
            }
            LoopMode::Forward => state.position = region.wrap_loop(state.position),
            LoopMode::Alternate => {
                state.position = end - (state.position - end);
                state.direction = -1.0;
            }
        }
    } else if state.direction < 0.0 && state.position < start {
        match loop_mode {
            LoopMode::Off => {
                state.position = start;
                state.ended = true;
            }
            LoopMode::Forward => state.position = region.wrap_loop(state.position),
            LoopMode::Alternate => {
                state.position = start + (start - state.position);
                state.direction = 1.0;
            }
        }
    }
}

fn wrap_travel(position: &mut f32, region: Region) {
    *position = region.wrap(*position);
}

/// Where a read lands once it leaves the region, in the reader's own float domain.
///
/// **The fetch already wraps an out-of-range index, and that is not enough.** A head's float
/// position has to wrap by the same span its taps do, or *"Stretch is the plain read at unity"*
/// is nearly true rather than true — the looping case of that test measured 9.6e-4 against a 1e-5
/// tolerance before the heads wrapped here instead of inside the interpolator. The span is the
/// loop's half-open one ([`Region::loop_span`]), which the taps, Repitch's position and Stretch's
/// travel all share.
///
/// Alternate is wrapped as Forward. A head is not the playhead: turning an individual head around
/// mid-window would need a per-head direction, and the reader has always read alternating regions
/// in one direction — `stretch` used to pass `LoopMode::Forward` unconditionally.
fn wrap_read(position: f32, region: Region, loop_mode: LoopMode) -> f32 {
    if loop_mode == LoopMode::Off {
        return position.clamp(region.start, region.end);
    }
    if position >= region.loop_start && position < region.loop_limit() {
        return position;
    }
    region.wrap_loop(position)
}

/// Slack above one sample's travel before a move counts as a jump rather than a step.
///
/// **Derived from the step, not fixed.** A constant was wrong at the rate extremes this crate
/// accepts: a 768 kHz source under an 8 kHz host travels 96 source frames per sample before Scan
/// doubles it, so a 64-frame constant classified ordinary playback as a discontinuity and turned
/// transient splices off exactly where the source was most detailed.
const MARKER_JUMP_SLACK: f32 = 4.0;

/// Walk the onset cursor along the **path actually travelled**, not between its endpoints.
///
/// **Folded endpoints do not bracket what a step passed over.** A Forward step that wraps, or an
/// Alternate step that crosses a turn, ends up somewhere the straight interval from its start does
/// not contain — and a step long enough to cover a whole loop can land exactly where it began,
/// bracketing nothing while having passed every marker in the region. So the step is cut at each
/// boundary and each piece walked in its own direction, because a folded piece *is* monotonic and
/// its endpoints do bracket what it crossed.
///
/// **Bounded by construction.** At most [`TRAVERSAL_SEGMENTS`] pieces are walked; a longer step is
/// a loop shorter than one sample's travel, where every attack is being re-read continuously and
/// one more splice means nothing. There the cursor is re-seeked and nothing is reported crossed —
/// the same answer the replay guard gives on those spans.
fn walk_traversal(
    transients: &[u32],
    layer: &mut LayerVoice,
    from_raw: f32,
    step: f32,
    turn: f32,
    period: f32,
    fold: impl Fn(f32) -> f32,
) -> bool {
    // Each piece is at most one step long, so this bounds every walk below without ever
    // misclassifying legitimate travel.
    let limit = step.abs() * MARKER_JUMP_SLACK + MARKER_JUMP_SLACK;
    if transients.is_empty() || step == 0.0 || !step.is_finite() || period <= 0.0 {
        return false;
    }
    let destination = (from_raw + step).rem_euclid(period);
    if step.abs() >= period * TRAVERSAL_SEGMENTS as f32 {
        layer.marker = seek_marker(transients, fold(destination));
        return false;
    }
    let mut crossed = false;
    let mut raw = from_raw;
    let mut remaining = step;
    let mut exhausted = true;
    for piece_index in 0..TRAVERSAL_SEGMENTS {
        if remaining == 0.0 {
            exhausted = false;
            break;
        }
        // **The next breakpoint, not the next period seam.** `fold` is monotonic only between
        // breakpoints, and Alternate has one *inside* its period — the turn at `span`. Segmenting
        // on the seam alone let a step across the turn have identical folded endpoints, which
        // brackets nothing: a move from 0.9 to 1.1 spans passes the turn and reports no crossing.
        // **Step off the seam before measuring.** Going backwards from exactly 0 there is no edge
        // behind it inside [0, period], so `to_edge` was -0.0, every piece was empty, and the walk
        // spent its whole budget standing still before re-seeking — which silently disabled
        // transient splices for any reverse traversal that began on the seam.
        let mut wrapped_seam = false;
        if remaining > 0.0 && raw >= period {
            raw = 0.0;
            wrapped_seam = true;
        } else if remaining < 0.0 && raw <= 0.0 {
            raw = period;
            wrapped_seam = true;
        }
        let to_edge = if remaining > 0.0 {
            if raw < turn { turn - raw } else { period - raw }
        } else if raw > turn {
            -(raw - turn)
        } else {
            -raw
        };
        let piece = if remaining.abs() <= to_edge.abs() {
            remaining
        } else {
            to_edge
        };
        // Every piece after the first begins on the far side of a breakpoint, where the cursor
        // still describes where the playhead was; walking from there finds nothing.
        // **Any piece that begins on the far side of a breakpoint needs the cursor moved to it** —
        // including the *first*, when that piece began by stepping over the seam. Re-seeking only
        // on later pieces left a reverse walk starting at the loop start looking from a cursor that
        // described the other end, so it saw nothing; the test that should have caught it had been
        // handed a cursor value by hand instead of one `seek_marker` would produce.
        if piece_index > 0 || wrapped_seam {
            layer.marker = seek_marker(transients, fold(raw));
        }
        let landed = (raw + piece).clamp(0.0, period);
        if advance_marker(transients, layer, fold(raw), fold(landed), limit) {
            crossed = true;
        }
        remaining -= piece;
        raw = landed.rem_euclid(period);
        if piece == 0.0 {
            // Sitting exactly on a breakpoint: step off it so the next piece makes progress.
            raw = if remaining > 0.0 {
                (raw + f32::EPSILON).min(period)
            } else {
                (raw - f32::EPSILON).max(0.0)
            };
        }
    }
    if exhausted && remaining != 0.0 {
        // **The budget ran out mid-path.** Leaving the cursor where the walk stopped would leave it
        // describing somewhere the playhead is not, so it is put where the playhead actually
        // landed. Crossings already found still count; the unwalked tail is not reported.
        layer.marker = seek_marker(transients, fold(destination));
    }
    crossed
}

/// How many loop boundaries one rendered sample's travel may cross before the walk re-seeks
/// instead. Ordinary travel crosses none; a region a few frames long crosses several.
const TRAVERSAL_SEGMENTS: usize = 4;

/// A loop mode as a number, so it can sit in the traversal key beside the region's bounds.
fn loop_mode_key(mode: LoopMode) -> f32 {
    match mode {
        LoopMode::Off => 0.0,
        LoopMode::Forward => 1.0,
        LoopMode::Alternate => 2.0,
    }
}

/// Signed distance from `travel` to `position` in traversal order, across a loop seam.
///
/// A looping region has no total order, so a plain subtraction is wrong by a whole span exactly
/// when the two arguments straddle the seam. Folding the difference into half a span either way is
/// the shortest path between them, which is what "ahead" and "behind" mean on a loop.
///
/// **The shortest path is the right answer only while the two are closer than half a span**, and
/// the sole caller checks that before believing it. Alternate is folded as circular here, which is
/// wrong in principle — an out-and-back path is not a circle — and harmless in practice because
/// that caller has already rejected the spans where the distinction could change its answer.
fn traversal_gap(
    position: f32,
    travel: f32,
    region: Region,
    loop_mode: LoopMode,
    looping: bool,
) -> f32 {
    let raw = position - travel;
    // Before the loop is reached — a first pass — the order is the recording's own.
    if loop_mode == LoopMode::Off || !looping {
        return raw;
    }
    let span = region.loop_span();
    let folded = (raw + span * 0.5).rem_euclid(span) - span * 0.5;
    if folded.is_finite() { folded } else { raw }
}

/// Index of the first onset at or after `position`. Used when the playhead arrives somewhere
/// rather than walks there — a note starting, or a region wrap.
fn seek_marker(transients: &[u32], position: f32) -> usize {
    transients.partition_point(|&onset| (onset as f32) < position)
}

/// Walk the onset cursor to wherever the playhead has reached, reporting whether it passed one.
///
/// The cursor moves by at most a few entries per sample because the playhead moves by well under a
/// frame at ordinary scan speeds, so this is O(1) amortised in both directions. **A hold crosses
/// nothing**, which is why transient anchoring being inert at zero scan needs no special case:
/// the playhead does not move, so the cursor does not either.
fn advance_marker(
    transients: &[u32],
    layer: &mut LayerVoice,
    from: f32,
    to: f32,
    limit: f32,
) -> bool {
    if transients.is_empty() || from == to {
        return false;
    }
    // **A cursor walk is O(1) only while the playhead walks.** A region edited under a sounding
    // note, a loop point moved by automation or a clamped jump moves it arbitrarily far, and the
    // walk below would then visit every marker in between — work proportional to the sample, inside
    // the callback. Past a stride no ordinary advance can reach, re-seek instead: the same answer
    // in a bounded number of comparisons.
    if (to - from).abs() > limit {
        // **Re-seek, and report nothing crossed.** The cursor has to follow the playhead wherever
        // it was put, but a jump is not a passage: scheduling a splice here would re-lay both heads
        // at wherever the jump landed, which is not an attack and need not be near one.
        layer.marker = seek_marker(transients, to);
        return false;
    }
    let mut crossed = false;
    if to > from {
        while layer.marker < transients.len() && (transients[layer.marker] as f32) <= to {
            layer.marker += 1;
            crossed = true;
        }
    } else {
        while layer.marker > 0 && (transients[layer.marker - 1] as f32) >= to {
            layer.marker -= 1;
            crossed = true;
        }
    }
    crossed
}

/// Character's five controls, resolved into the numbers a read actually uses.
///
/// **Resolved once per rendered sample, not once per grain.** Every live grain in every voice was
/// re-deriving the identical five clamps, the hold grid and the converter depth from the identical
/// parameters — up to 128 times per sample to reach the same answer. The values depend only on
/// `Character`, so hoisting them is arithmetically identical and simply stops repeating the work.
#[derive(Debug, Clone, Copy)]
struct ResolvedCharacter {
    grid: f32,
    reconstruction: f32,
    levels: f32,
    /// Exactly `1 / levels`, because a power of two inverts exactly.
    inverse_levels: f32,
    input: f32,
    /// The hold and reconstruction stages have nothing to do at Clean.
    transparent: bool,
    converter_type: ConverterType,
    /// How far the clock may wander, in **source frames**: `JITTER_CELLS` of a cell at full.
    ///
    /// Resolved here because the cell width is `grid`, which this already knows, and because zero
    /// has to stay exactly zero.
    jitter: f32,
}

impl ResolvedCharacter {
    fn new(character: Character) -> Self {
        // **Four stages, no macro over them.** A single Character knob added a different weight to
        // each of these and named none of them, which is the opposite of what this section is for:
        // it is the instrument's reason for existing, an emulation of what a Fairlight or an
        // Emulator II did to a sound on the way in. Those machines are remembered by numbers —
        // eight bits, twenty-odd kilohertz — and a knob reading 43% is not that number.
        let rate = sanitise(character.rate, 0.0).clamp(0.0, 1.0);
        let converter = sanitise(character.converter, 0.0).clamp(0.0, 1.0);
        let reconstruction = sanitise(character.reconstruction, 0.0).clamp(0.0, 1.0);
        let input = sanitise(character.input, 0.0).clamp(0.0, 1.0);
        let jitter = sanitise(character.jitter, 0.0).clamp(0.0, 1.0);
        let grid = character_rate_divisor(rate);
        let bits = character_bits(converter);
        Self {
            grid,
            reconstruction,
            // `bits` is an integer in 5..=16, so the level count is an exact power of two. `powf`
            // returns the same number and costs a transcendental call to do it.
            levels: quantizer_levels(bits),
            inverse_levels: 1.0 / quantizer_levels(bits),
            input,
            // **Jitter is part of what makes the stage do something.** `transparent` skips the
            // whole hold path, and a jittering clock is a reason not to skip it even at the native
            // rate — otherwise the control would be dead at the setting most of a session is spent
            // at, which is exactly the "I cannot hear what it does" failing that removed the last
            // occupant of this slot.
            transparent: reconstruction <= 0.0 && grid <= 1.000_1 && jitter <= 0.0,
            converter_type: character.converter_type,
            // In source frames: a fraction of one cell, so the wander scales with the rate the way a
            // real clock's does.
            jitter: jitter * JITTER_CELLS * grid,
        }
    }

    /// The resolved clock wander in source frames, for `a_jittering_clock_never_runs_backwards`.
    #[cfg(test)]
    fn jitter_for_test(&self) -> f32 {
        self.jitter
    }

    /// **One converter, at the end of the voice.** Quantisation and companding, applied once to
    /// what the whole voice is sounding rather than separately inside every grain read.
    ///
    /// The machines this borrows from put one DAC after the digital section and before the analogue
    /// filter, so everything a voice was playing arrived at the converter summed. Quantising is
    /// nonlinear, so quantise-then-sum is genuinely not sum-then-quantise: sixteen grains each
    /// rounded to their own grid gave sixteen uncorrelated error signals that averaged toward
    /// smoothness, and one converter on the sum gives the stepped edge the hardware had.
    fn convert(&self, value: Stereo) -> Stereo {
        // **Companding happens around the quantiser, not instead of it.** The steps stay uniform;
        // what changes is the domain they are uniform in, which is the whole mechanism — see
        // `ConverterType` and mxm-kit's `docs/oscillators/14-samplers.md` §14.2.
        let companding = self.converter_type == ConverterType::Companding;
        let (left, right) = if companding {
            (compand(value.left), compand(value.right))
        } else {
            (value.left, value.right)
        };
        // `levels` is an exact power of two, so its reciprocal is exact and the multiply is
        // bit-identical to the division it replaces.
        let quantized = Stereo {
            left: (left * self.levels).round() * self.inverse_levels,
            right: (right * self.levels).round() * self.inverse_levels,
        };
        let quantized = if companding {
            Stereo {
                left: expand(quantized.left),
                right: expand(quantized.right),
            }
        } else {
            quantized
        };
        if self.input <= 0.0 {
            return quantized;
        }
        let drive = 1.0 + self.input * 15.0;
        let norm = soft_clip(drive);
        Stereo {
            left: soft_clip(quantized.left * drive) / norm,
            right: soft_clip(quantized.right * drive) / norm,
        }
    }
}

fn read(
    sample: &Sample,
    position: f32,
    region: Region,
    loop_mode: LoopMode,
    character: &ResolvedCharacter,
    bandlimit: f32,
) -> Stereo {
    let ResolvedCharacter {
        grid,
        reconstruction,
        transparent,
        jitter,
        ..
    } = *character;
    let clean = sinc_read(sample, position, region, loop_mode, bandlimit);
    let reconstructed = if transparent {
        clean
    } else {
        // Only the hold path needs the virtual grid, and it costs a division to find. At Clean —
        // the setting most of a session is spent at — it was being paid on every grain read for a
        // value the next line threw away.
        // **The clock wanders, and this is where it wanders.** A cell boundary is *when* the
        // converter samples, so moving it is jitter — as against dither, which leaves the clock
        // alone and adds noise to what it read. The wobble is a hash of the cell's own index, so it
        // needs no state, is identical however the block is split, and cannot disturb the streams
        // that schedule grains.
        //
        // **Both ends of the interval are displaced, and that is what makes this a clock.** The
        // first version displaced only the cell the position fell in and then stepped a *nominal*
        // grid to find its neighbour, interpolating across a nominal interval — so the two instants
        // were inconsistent with each other and, at a wide enough wobble, out of order. Asking the
        // same function for instant `n` and instant `n + 1` keeps them ordered by construction
        // (`JITTER_CELLS < 1`) and makes the reconstruction a crossfade between the samples the
        // converter actually took.
        let cell = (position / grid).floor();
        let instant = |index: f32| {
            if jitter > 0.0 {
                index * grid + jitter * (cell_wobble(index) - 0.5)
            } else {
                index * grid
            }
        };
        let this = instant(cell);
        let following = instant(cell + 1.0);
        let held = frame(sample, this.floor() as i64, region, loop_mode);
        let next = frame(sample, following.floor() as i64, region, loop_mode);
        // The interval between the two instants the converter actually took, not a nominal cell.
        // It cannot be negative while `JITTER_CELLS < 1`; the floor guards a division only.
        let span = (following - this).max(1.0e-6);
        let fraction = ((position - this) / span).clamp(0.0, 1.0);
        let linear = Stereo {
            left: held.left + (next.left - held.left) * fraction,
            right: held.right + (next.right - held.right) * fraction,
        };
        linear
            .gain(1.0 - reconstruction)
            .add(held.gain(reconstruction))
    };

    // **The converter is not here.** It is one per voice, downstream of the layer sum — see
    // `ResolvedCharacter::convert` and the crate DOX's *One converter per voice*.
    reconstructed
}

fn sinc_read(
    sample: &Sample,
    position: f32,
    region: Region,
    loop_mode: LoopMode,
    bandlimit: f32,
) -> Stereo {
    let centre = position.floor() as i64;
    let cutoff_now = sanitise(bandlimit, 1.0).clamp(0.02, 1.0);
    if cutoff_now >= 1.0 {
        // **The unity-rate path, which is most of them**, reading a kernel that was built and
        // normalised once instead of sixteen coefficients — each with its own division — on every
        // grain on every sample. `osc_spike` §9f priced that difference at 6.8× on the kernel.
        let table: &[f32] = &UNITY_KERNEL[..];
        let frac = (position - centre as f32).clamp(0.0, 1.0);
        let scaled = frac * SINC_PHASES as f32;
        let phase = (scaled as usize).min(SINC_PHASES - 1);
        let blend = scaled - phase as f32;
        let row = &table[phase * SINC_TAPS_N..phase * SINC_TAPS_N + SINC_TAPS_N];
        let next = &table[(phase + 1) * SINC_TAPS_N..(phase + 1) * SINC_TAPS_N + SINC_TAPS_N];
        let mut output = Stereo::default();
        let mut walk = Walk::new(
            sample,
            centre + i64::from(-(SINC_TAPS / 2 - 1)),
            region,
            loop_mode,
        );
        if let Some(start) = walk.contiguous(SINC_TAPS_N) {
            for (tap, value) in sample.frames[start..start + SINC_TAPS_N].iter().enumerate() {
                let low = row[tap];
                let weight = low + (next[tap] - low) * blend;
                output.left += value[0] * weight;
                output.right += value[1] * weight;
            }
            return output;
        }
        for tap in 0..SINC_TAPS_N {
            let low = row[tap];
            let weight = low + (next[tap] - low) * blend;
            let value = walk.fetch(sample);
            output.left += value.left * weight;
            output.right += value.right * weight;
            walk.advance();
        }
        return output;
    }
    let mut output = Stereo::default();
    let mut sum = 0.0;
    // Adjacent taps differ by constant angles. Walking those angles with a recurrence preserves
    // this sixteen-tap kernel while replacing three transcendentals per tap with two `sin_cos`
    // evaluations per complete read.
    let cutoff = sanitise(bandlimit, 1.0).clamp(0.02, 1.0);
    let first_tap = -(SINC_TAPS / 2 - 1);
    let mut distance = position - (centre + i64::from(first_tap)) as f32;
    let (mut sinc_sin, mut sinc_cos) = (PI * cutoff * distance).sin_cos();
    let (sinc_step_sin, sinc_step_cos) = (-PI * cutoff).sin_cos();
    let half_width = SINC_TAPS as f32 / 2.0;
    let (mut window_sin, mut window_cos) = (PI * distance / half_width).sin_cos();
    let (window_step_sin, window_step_cos) = (-PI / half_width).sin_cos();
    // **The taps are consecutive, so the loop wrap is resolved once and then walked.** Calling
    // `frame` per tap costs an integer division (`rem_euclid`) sixteen times per read, and that
    // division — not the kernel arithmetic — was the single largest cost in a dense grain cloud.
    // Because `wrap(i + 1)` is `wrap(i) + 1` except at the loop end, a conditional subtract is
    // exactly equivalent to redividing. `sinc_read_walks_the_same_frames_as_a_naive_reader` holds
    // the two forms together.
    let mut walk = Walk::new(sample, centre + i64::from(first_tap), region, loop_mode);
    for _ in first_tap..=SINC_TAPS / 2 {
        let sinc = if distance.abs() < 1.0e-6 {
            cutoff
        } else {
            sinc_sin / (PI * distance)
        };
        let window = 0.42 + 0.5 * window_cos + 0.08 * (2.0 * window_cos * window_cos - 1.0);
        let weight = sinc * window;
        let value = walk.fetch(sample);
        output.left += value.left * weight;
        output.right += value.right * weight;
        walk.advance();
        sum += weight;

        (sinc_sin, sinc_cos) = (
            sinc_sin * sinc_step_cos + sinc_cos * sinc_step_sin,
            sinc_cos * sinc_step_cos - sinc_sin * sinc_step_sin,
        );
        (window_sin, window_cos) = (
            window_sin * window_step_cos + window_cos * window_step_sin,
            window_cos * window_step_cos - window_sin * window_step_sin,
        );
        distance -= 1.0;
    }
    if sum.abs() > 1.0e-6 {
        output.gain(1.0 / sum)
    } else {
        frame(sample, centre, region, loop_mode)
    }
}

/// Points in the shared grain and overlap-add window.
const WINDOW_POINTS: usize = 1024;

/// One Hann window, shared by every grain and every overlap-add head in every voice.
///
/// Both readers took a cosine per window per sample — one transcendental for each of up to 128
/// live grains, 48 000 times a second, to evaluate the same curve. Interpolating 1024 points is
/// accurate to well under the converter's own step and is held there by a test.
static HANN: std::sync::LazyLock<[f32; WINDOW_POINTS + 1]> = std::sync::LazyLock::new(|| {
    let mut table = [0.0f32; WINDOW_POINTS + 1];
    for (point, slot) in table.iter_mut().enumerate() {
        let phase = point as f64 / WINDOW_POINTS as f64;
        *slot = (0.5 - 0.5 * (std::f64::consts::TAU * phase).cos()) as f32;
    }
    table
});

/// The shared Hann window at `phase`, which is clamped into `0..=1`.
/// The grain envelope at `phase`, for a given `shape`: near-boxcar, triangle, Hann.
///
/// **The window is a tone control, not an engineering choice.** Under a periodic scheduler the
/// envelope *is* the waveform of an amplitude modulator whose period is the grain, so its shape
/// decides how many sidebands there are and how far they reach - which is why `mxm-grain-fx` makes
/// it a performed control, and the owner asked for the same here.
///
/// The law is `mxm-grain-fx`'s `window::window`, reproduced rather than shared: both DSP crates
/// depend on nothing but core, and a dependency between two plugins' engines is the wrong shape for
/// one function. Below the midpoint a triangular ramp is multiplied by a slope and clamped, running
/// a trapezoid from near-boxcar to triangle; above it the ramp is crossfaded toward
/// `sin^2(ramp*pi/2)`, which **is** a Hann rather than an approximation to one.
///
/// **The smooth end costs no new table.** `sin^2(ramp*pi/2)` is this crate's own Hann read at half
/// the ramp - `hann(r / 2) = sin^2(pi * r / 2)` - so the 1024-point table already here is the whole
/// upper branch.
///
/// The midpoint is an exact triangle from either side. The published implementation the research
/// read has a silent point there, where both branches set slope and smoothing to zero and every
/// grain renders at zero gain; that is a latent bug rather than a wart, and defects are not
/// reproduced.
#[must_use]
pub fn grain_window(shape: f32, phase: f32) -> f32 {
    let phase = sanitise(phase, 0.0).clamp(0.0, 1.0);
    let shape = sanitise(shape, 1.0).clamp(0.0, 1.0);
    // **Smooth is the default and costs exactly what it always did.** At the top of the control the
    // crossfade lands entirely on `sin²(ramp·π/2)`, and that is this crate's own Hann read at half
    // the ramp — which by the window's symmetry is `hann(phase)`. Taking it directly is one table
    // lookup rather than a ramp, a lookup and a lerp, and the dense probe measured the difference:
    // 36.8% of a core against the 31.9% this crate was tuned to.
    if shape >= 0.999 {
        return hann(phase);
    }
    let ramp = if phase < 0.5 {
        phase * 2.0
    } else {
        2.0 - phase * 2.0
    };
    if shape < 0.5 {
        // At shape 0 the slope is 50 - an attack of a hundredth of the grain, audibly a boxcar and
        // deliberately clicky. At 0.5 it is exactly 1, the triangle the upper branch starts from,
        // so the two halves meet with no step.
        let slope = 0.5 / (shape * 0.98 + 0.01);
        (ramp * slope).min(1.0)
    } else {
        let blend = (shape - 0.5) * 2.0;
        let smooth = hann(ramp * 0.5);
        ramp + blend * (smooth - ramp)
    }
}

/// How many shapes the moment table holds between 0 and 1.
///
/// The two moments move smoothly and slowly with the shape, so a coarse table interpolated linearly
/// is far finer than the gain it sets needs. 128 puts the interpolation error below a thousandth of
/// a decibel.
const MOMENT_TABLE: usize = 128;

/// `(mean, mean_square)` for every shape, built once off the audio thread.
///
/// **Because `normalisation` runs per rendered sample.** Integrating the window there was shipped
/// for one round and it cost about 128 window evaluations per sample per layer: the owner reported
/// a held note stuttering, cutting out, and resuming on release, which is a callback overrunning
/// its budget while grains are alive and recovering when they are not. The moments belong at
/// control rate; a table makes them a lookup instead, the same trade `HANN` and `UNITY_KERNEL`
/// already take in this crate.
static WINDOW_MOMENTS: std::sync::LazyLock<[(f32, f32); MOMENT_TABLE + 1]> =
    std::sync::LazyLock::new(|| {
        std::array::from_fn(|point| integrate_moments(point as f32 / MOMENT_TABLE as f32))
    });

/// The window's mean and mean-square at `shape`, which are the two normalisation laws' content.
///
/// A tabled lookup — see [`WINDOW_MOMENTS`] for why it is not integrated on the spot.
#[must_use]
pub fn window_moments(shape: f32) -> (f32, f32) {
    let scaled = sanitise(shape, 1.0).clamp(0.0, 1.0) * MOMENT_TABLE as f32;
    let point = (scaled as usize).min(MOMENT_TABLE - 1);
    let blend = scaled - point as f32;
    let (low_mean, low_square) = WINDOW_MOMENTS[point];
    let (high_mean, high_square) = WINDOW_MOMENTS[point + 1];
    (
        low_mean + (high_mean - low_mean) * blend,
        low_square + (high_square - low_square) * blend,
    )
}

/// The integration the table is built from. Midpoint rule, so the window's interior is sampled and
/// not its two zero endpoints.
fn integrate_moments(shape: f32) -> (f32, f32) {
    let mut sum = 0.0f32;
    let mut sum_sq = 0.0f32;
    for point in 0..MOMENT_POINTS {
        let phase = (point as f32 + 0.5) / MOMENT_POINTS as f32;
        let w = grain_window(shape, phase);
        sum += w;
        sum_sq += w * w;
    }
    let n = MOMENT_POINTS as f32;
    (sum / n, sum_sq / n)
}

fn hann(phase: f32) -> f32 {
    let scaled = sanitise(phase, 0.0).clamp(0.0, 1.0) * WINDOW_POINTS as f32;
    let point = (scaled as usize).min(WINDOW_POINTS - 1);
    let blend = scaled - point as f32;
    let low = HANN[point];
    low + (HANN[point + 1] - low) * blend
}

/// Fractional positions in the unity-rate kernel table.
///
/// 512 rows interpolated pairwise, not 512 rows rounded to: **the interpolation is what makes the
/// table honest.** Rounding to the nearest row is a fractional-delay quantisation of ±1/1024 of a
/// sample, which is audible noise well above this reader's measured alias floor. Interpolating two
/// rows costs one multiply-add per tap and, because both rows sum to one and that sum is linear,
/// the interpolated row sums to one as well — so the normalising division disappears with it.
const SINC_PHASES: usize = 512;

const SINC_TAPS_N: usize = SINC_TAPS as usize;

/// The sixteen-tap Blackman-windowed sinc at **unity rate**, normalised per row at build time,
/// with a duplicate final row so interpolating `phase`→`phase + 1` never needs a bound.
///
/// **Only unity rate can be tabled**, and that is not a compromise: every call site derives the
/// band limit as `1 / max(rate, 1)`, so a read at or below the recorded rate has a cutoff of
/// exactly 1.0 and a read above it has a cutoff that narrows with pitch. Tabling one kernel for
/// all of them would silently throw away the transposition band-limiting this reader is measured
/// on. Reads above unity keep the computed kernel, unchanged, and keep their measurement with it.
///
/// Built once, off the audio thread: [`Engine::new`] touches it so a callback never runs the
/// initialiser. The array lives in static memory rather than a `Vec`, so no allocation happens at
/// all.
static UNITY_KERNEL: std::sync::LazyLock<[f32; (SINC_PHASES + 1) * SINC_TAPS_N]> =
    std::sync::LazyLock::new(|| {
        let mut table = [0.0f32; (SINC_PHASES + 1) * SINC_TAPS_N];
        let first_tap = -f64::from(SINC_TAPS / 2 - 1);
        let half_width = f64::from(SINC_TAPS) / 2.0;
        for phase in 0..=SINC_PHASES {
            let frac = phase as f64 / SINC_PHASES as f64;
            let mut sum = 0.0f64;
            for tap in 0..SINC_TAPS_N {
                // The same distance the computed kernel walks: it starts at `frac - first_tap`
                // and decrements by one per tap.
                let distance = frac - first_tap - tap as f64;
                let sinc = if distance.abs() < 1.0e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * distance).sin() / (std::f64::consts::PI * distance)
                };
                let cosine = (std::f64::consts::PI * distance / half_width).cos();
                let window = 0.42 + 0.5 * cosine + 0.08 * (2.0 * cosine * cosine - 1.0);
                let weight = sinc * window;
                table[phase * SINC_TAPS_N + tap] = weight as f32;
                sum += weight;
            }
            if sum.abs() > 1.0e-12 {
                for tap in 0..SINC_TAPS_N {
                    let weight = f64::from(table[phase * SINC_TAPS_N + tap]) / sum;
                    table[phase * SINC_TAPS_N + tap] = weight as f32;
                }
            }
        }
        table
    });

/// What the virtual converter's controls mean, as the numbers the machines were sold on.
///
/// Public because **the panel has to show these, not a percentage.** A Fairlight is remembered as
/// eight bits at around twenty kilohertz; "Converter 43%" is not a fact about a sound. Exposing the
/// laws here rather than restating them in the editor is what stops the two drifting apart.
/// **Geometric, because sample rate is heard in octaves.** The law was `1 + rate²·31`, which put
/// the first fifth of the knob between 48 and 21 kHz — two settings that differ only above the top
/// of hearing — and squeezed every rate a listener can actually name into the upper half. A
/// doubling per fifth of travel spreads the machines evenly instead: 24 kHz at 20%, 12 at 40%,
/// 6 at 60%, 3 at 80%. Both ends are unchanged, so Clean is still exactly transparent.
#[must_use]
pub fn character_rate_divisor(rate: f32) -> f32 {
    (5.0 * sanitise(rate, 0.0).clamp(0.0, 1.0)).exp2()
}

/// The converter's depth in bits, 16 down to 5.
///
/// **Square-rooted, because the machines this borrows from live in the last third of a linear
/// law.** `16 − 11·c` held the knob above eleven bits for its whole first half, and eleven-bit
/// quantisation noise sits near −66 dBFS: inaudible under the sound that made it. The owner's
/// report that the converter did nothing was that half of the travel. The root reaches 12 bits —
/// an S900, an Emulator III — by 15%, and 8 bits — a Fairlight, an Emulator II — at halfway, which
/// leaves the whole upper half for the range the section exists to offer. 16 bits at 0 is
/// deliberate and unchanged: Clean has to be clean.
#[must_use]
pub fn character_bits(converter: f32) -> f32 {
    (16.0 - sanitise(converter, 0.0).clamp(0.0, 1.0).sqrt() * 11.0)
        .round()
        .clamp(5.0, 16.0)
}

/// How hard the compander drives the converter, in decibels.
#[must_use]
pub fn character_drive_db(input: f32) -> f32 {
    20.0 * (1.0 + sanitise(input, 0.0).clamp(0.0, 1.0) * 15.0).log10()
}

/// The lowest and highest pitch [`detect_root`] will look for, in hertz.
///
/// Chosen for what a sampler is actually given. 30 Hz is below the bottom of a five-string bass and
/// of a 32-foot organ stop; 5 kHz is above the top of a piccolo. Widening either end costs search
/// range on every call and buys octave errors, because a wider window admits more sub-harmonics.
const PITCH_FLOOR_HZ: f32 = 30.0;
const PITCH_CEILING_HZ: f32 = 5_000.0;

/// Below this the cumulative-mean difference is a confident period rather than a coincidence.
///
/// YIN's own figure. Above it the function is measuring noise, and a sampler would rather be told
/// nothing than be tuned to a cymbal.
const PITCH_CONFIDENCE: f32 = 0.20;

/// **What was tried for the octave error, and why it is not here.**
///
/// A bass patch with a sub oscillator genuinely repeats at the sub's period, so YIN is right about
/// the signal and wrong about the note — measured against `mxm-mono-01`'s oscillator, every such
/// case came back an octave flat or declined outright. The obvious remedy is to prefer a half,
/// third or quarter of the winning lag whenever that shorter lag scores nearly as well. It was
/// built, and `dsp-lab`'s `root_spike` says it buys nothing: the margin changed no verdict at 0.25,
/// 0.45 or 0.70 on either the synthetic set or a real bass pack, and made two more wrong at 1.00.
/// It is not here because it was not earned.
///
/// The reason it cannot work is structural. A note at E1 is 41 Hz, so its sub is 20.6 Hz and the
/// signal's true period is **below [`PITCH_FLOOR_HZ`]** — there is no lag in the search range that
/// describes it, and no amount of dividing the winner reaches a period the search never held. A fix
/// has to decide the octave on something other than the difference function: harmonic summation
/// over a spectrum, or comparing the energy at the candidate against the energy an octave below.
/// That is a different detector, not a tuning constant.
/// Above this the best lag in a window is not a period at all, and the window says nothing.
const PITCH_REJECT: f32 = 0.35;

/// How far apart successive windows may be and still count as the same note, in cents.
///
/// A quarter-tone. Wider admits a glide as a held note; narrower breaks a real note up over
/// vibrato and the ordinary drift of a plucked string.
const PITCH_AGREE_CENTS: f32 = 50.0;

/// How many agreeing windows in a row make a note.
///
/// Three at a 25 ms hop is about a tenth of a second of held pitch. Fewer lets an 808's glide stop
/// briefly and be believed; more misses a short first note in a busy phrase.
const PITCH_RUN: usize = 3;

/// The gap between window starts.
const PITCH_HOP_S: f32 = 0.025;

/// How far past the region start the search will look for a note that holds.
///
/// **The answer is meant to be the sound nearest the start, so the search does not wander.** It ran
/// six seconds forward, which on a twelve-second loop could report a note from the middle of a
/// progression and call it the root. A second and a half covers a slow attack and an 808's pitch
/// envelope settling — the two things that legitimately delay a steady pitch — and stops well short
/// of the next musical event. Move the region start and the answer moves with it, which is the
/// control the owner asked for: put Start on the sound you want the pitch of.
const PITCH_SEARCH_S: f32 = 1.5;

/// How much of one note the answer is averaged over, once a note has been found.
///
/// The rule is *the pitch until the first pitch change after the start*, so what ends the average
/// is normally the change itself. This only bounds a note that never changes — a held pad, or a
/// one-shot that rings for ten seconds — where continuing to measure buys nothing and costs a
/// button that stalls. Three seconds is more of one note than any answer needs.
const PITCH_NOTE_S: f32 = 3.0;

/// What the sample sounds like it is playing where it starts, as a MIDI note.
///
/// **A player should not have to guess a sample's pitch by ear**, which is what an untouched Root
/// asks of them: it defaults to a number, the sample is whatever it is, and the two agree only by
/// luck. This answers the question the control is really asking — *what note is this?*
///
/// `from` is the normalised region start, so this reads where playback will actually begin rather
/// than at frame zero. Leading silence is skipped, and so is the first slice of the attack: a
/// struck or plucked onset is inharmonic for a few milliseconds and will happily report a fifth.
///
/// **YIN** (de Cheveigné and Kawahara 2002), the difference function with its cumulative mean
/// normalisation, an absolute threshold, and parabolic interpolation on the winning lag. Plain
/// autocorrelation was not enough here: it peaks at lag zero and prefers longer lags, so it octave-
/// errors downward on exactly the harmonically rich material a sampler is pointed at. The
/// normalisation is the step that removes that bias.
///
/// Returns `None` rather than a guess when nothing is confidently periodic — silence, noise, a
/// drum, a chord. Off the audio thread: this is a button, not a per-sample cost.
#[must_use]
pub fn detect_root(sample: &Sample, from: f32) -> Option<f32> {
    let frames = sample.frames();
    let rate = sample.sample_rate();
    let last = frames.len().saturating_sub(1);
    let begin = (sanitise(from, 0.0).clamp(0.0, 1.0) * last as f32) as usize;

    let tau_max = (rate / PITCH_FLOOR_HZ).ceil() as usize;
    let tau_min = (rate / PITCH_CEILING_HZ).floor().max(2.0) as usize;
    let window = tau_max * 2;
    let need = window + tau_max;
    if frames.len().saturating_sub(begin) < need {
        return None;
    }

    // Mono, and skip what is not yet the sound: silence first, then a little of the attack.
    let mono: Vec<f32> = frames[begin..]
        .iter()
        .map(|frame| (frame[0] + frame[1]) * 0.5)
        .collect();
    let peak = mono.iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
    if peak < 1.0e-4 {
        return None;
    }
    let onset = mono.iter().position(|v| v.abs() > peak * 0.02)?;
    let skip = onset + (rate * 0.02) as usize;
    let usable = mono.get(skip..)?;
    if usable.len() < need {
        return None;
    }

    // Scratch for the difference function and its normalisation, allocated once for every window.
    let mut d = vec![0.0f32; tau_max + 1];
    let mut normalised = vec![1.0f32; tau_max + 1];

    let hop = ((rate * PITCH_HOP_S) as usize).max(1);
    let mut run: Vec<f32> = Vec::new();
    let mut every: Vec<f32> = Vec::new();
    let mut offset = 0;
    let reach = ((rate * PITCH_SEARCH_S) as usize).max(hop);
    let span = ((rate * PITCH_NOTE_S) as usize).max(hop);
    // **Established, then held to the end of the note.** The run stops being a candidate and starts
    // being the answer at `PITCH_RUN` windows; from there it keeps taking windows for as long as
    // they agree, so the note is measured over its whole length rather than over the first tenth of
    // a second of it. What ends it is the first pitch change after the start, which is the owner's
    // rule exactly.
    let mut established = false;
    while offset + need <= usable.len() {
        if !established && offset > reach {
            break;
        }
        if established && offset > reach + span {
            break;
        }
        match yin(
            &usable[offset..offset + need],
            rate,
            tau_min,
            tau_max,
            window,
            &mut d,
            &mut normalised,
        ) {
            Some(note) => {
                every.push(note);
                // A run is broken by disagreement with its own first window, not with the previous
                // one: a glide moves a little at a time, and comparing neighbours would let it walk
                // any distance while every step looked like the same note.
                let changed = run
                    .first()
                    .is_some_and(|first| (note - first).abs() * 100.0 > PITCH_AGREE_CENTS);
                if changed {
                    if established {
                        return Some(median(&mut run));
                    }
                    run.clear();
                }
                run.push(note);
                established |= run.len() >= PITCH_RUN;
            }
            None => {
                if established {
                    return Some(median(&mut run));
                }
                run.clear();
            }
        }
        offset += hop;
    }

    if established {
        return Some(median(&mut run));
    }
    // Nothing held still for long enough. The median of what was seen is a better answer than
    // silence for a short or restless sample, and still refuses when no window was periodic.
    (!every.is_empty()).then(|| median(&mut every))
}

/// The middle value, which is what a set of pitch estimates should be reduced by.
///
/// Not the mean: one octave error would drag a mean half an octave, and the median ignores it.
fn median(values: &mut [f32]) -> f32 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

/// YIN over one window, as a MIDI note.
///
/// The buffers are borrowed rather than allocated here because [`detect_root`] runs this over every
/// window of a sample and the two of them are the whole working set.
#[allow(clippy::too_many_arguments)]
fn yin(
    x: &[f32],
    rate: f32,
    tau_min: usize,
    tau_max: usize,
    window: usize,
    d: &mut [f32],
    normalised: &mut [f32],
) -> Option<f32> {
    // YIN step 1: the squared difference at each lag.
    d[..=tau_max].fill(0.0);
    for (tau, slot) in d.iter_mut().enumerate().take(tau_max + 1).skip(tau_min) {
        let mut sum = 0.0f32;
        for i in 0..window {
            let delta = x[i] - x[i + tau];
            sum += delta * delta;
        }
        *slot = sum;
    }

    // YIN step 2: the cumulative mean normalisation, which is what kills the lag-zero bias.
    normalised[..=tau_max].fill(1.0);
    let mut running = 0.0f32;
    for tau in tau_min..=tau_max {
        running += d[tau];
        normalised[tau] = if running > 0.0 {
            d[tau] * (tau - tau_min + 1) as f32 / running
        } else {
            1.0
        };
    }

    // YIN step 3, and the one change this crate makes to it. **The best lag, then the shortest lag
    // that is nearly as good.** Plain YIN takes the first lag under an absolute threshold, which
    // handles a harmonic series; it does not handle a sub oscillator, because a patch with one
    // genuinely repeats at the sub's period and every lag shorter than that scores badly. The note
    // is the shorter period whenever the shorter period is close — see [`PITCH_OCTAVE_MARGIN`].
    // **The first lag under the threshold, not the smallest.** The smallest is usually an octave
    // down, and this step is the whole reason YIN beats plain autocorrelation on harmonically rich
    // material. Replacing it with the global minimum was tried and put every note above C4 two
    // octaves flat, which `root_spike` caught immediately.
    let mut chosen = None;
    for at in tau_min..=tau_max {
        if normalised[at] < PITCH_CONFIDENCE {
            let mut descend = at;
            while descend < tau_max && normalised[descend + 1] < normalised[descend] {
                descend += 1;
            }
            chosen = Some(descend);
            break;
        }
    }
    let tau = match chosen {
        Some(tau) => tau,
        // Nothing was confident. The global minimum is still worth taking if it is close.
        None => {
            let mut best = tau_min;
            for at in tau_min..=tau_max {
                if normalised[at] < normalised[best] {
                    best = at;
                }
            }
            if normalised[best] >= PITCH_REJECT {
                return None;
            }
            best
        }
    };

    // YIN step 4: parabolic interpolation, so the answer is not quantised to whole samples. At
    // 48 kHz a whole-sample lag near 2 kHz is already a third of a semitone.
    let refined = if tau > tau_min && tau < tau_max {
        let (a, b, c) = (normalised[tau - 1], normalised[tau], normalised[tau + 1]);
        let divisor = 2.0 * (2.0 * b - a - c);
        if divisor.abs() > f32::EPSILON {
            tau as f32 + (c - a) / divisor
        } else {
            tau as f32
        }
    } else {
        tau as f32
    };

    let hz = rate / refined;
    if !(PITCH_FLOOR_HZ..=PITCH_CEILING_HZ).contains(&hz) {
        return None;
    }
    let note = 69.0 + 12.0 * (hz / 440.0).log2();
    (0.0..=127.0).contains(&note).then_some(note)
}

/// How many grains the parameters ask to have sounding at once.
///
/// **A musical ceiling, not a resource one.** This is what the knobs may ask for and what the gain
/// law divides by, and it is deliberately independent of how many grains a voice's current share of
/// the budget can actually supply. Tying it to the share would make a patch's loudness depend on
/// how many notes are held — a chord would change the level of the notes already sounding.
///
/// The editor calls this rather than re-deriving it, so the readout and the gain law cannot
/// disagree. It re-derived it once, without these clamps, and they did.
pub fn overlap(params: &LayerParams, density_hz: f32) -> f32 {
    let size = sanitise(params.grain_size_s, 0.09).clamp(0.008, 1.0);
    let density = sanitise(density_hz, 14.0).clamp(0.5, MAX_GRAIN_DENSITY_HZ);
    (size * density).clamp(1.0, MAX_OVERLAP)
}

/// The gain a cloud of this shape needs, before it is summed.
///
/// **Coherent and incoherent overlap do not sum the same way, and one law for both is wrong by up
/// to the square root of the grain count.** Amplitude adds when grains read the same material, so
/// the compensation is the reciprocal of `overlap × mean(w)`; power adds when they read different
/// material, so it goes as one over the square root of `overlap × mean(w²)`.
///
/// **A stochastic scheduler is not on its own enough to reach the incoherent law.** With the
/// position draw shut, every grain reads the same material at the same rate and they sum
/// coherently however their onsets are scattered; applying the incoherent law there under-
/// compensates by the square root of the count, and the cloud gets *louder* as it gets denser
/// instead of fusing. That is the defect this replaces: `1/√overlap` was applied unconditionally.
///
/// So the incoherent share is the scheduler's hand-over **times** how far the position draw is
/// open, and with either shut the coherent law holds. Erring coherent is the safe direction — it
/// never under-compensates, so the failure is a texture that is too quiet rather than one that
/// grows without bound.
///
/// The count is the **requested** overlap, not the live occupancy: a measured count would depend on
/// pool state and would therefore be neither invariant under block partitioning nor repeatable
/// after a reset.
fn normalisation(params: &LayerParams, density_hz: f32) -> f32 {
    let overlap = overlap(params, density_hz);
    // **The two moments follow the shape control.** They were constants - 0.5 and 0.375, exact for
    // a Hann - and they could be, because the window was not a control. It is now, and a trapezoid
    // has a mean near 1 against a Hann's 0.5: leaving them fixed would make a cloud jump six
    // decibels as the shape is turned, which is the level change a normalisation exists to prevent.
    let (mean, mean_square) = window_moments(params.grain_shape);
    let coherent = 1.0 / (overlap * mean).max(1.0);
    let incoherent = INCOHERENT_HEADROOM / (overlap * mean_square).max(1.0).sqrt();
    // **The position draw alone decides how much of the incoherent law applies**, which is what
    // `mxm-grain-fx` measured: onsets scattered across material that is still identical sum
    // coherently however they are placed. This crate could not say that until the position draw and
    // the onset schedule became separate controls — one knob drove both, so the law had to gate on
    // the pair and compromised in both directions.
    let decorrelated = sanitise(params.grain_position_variation, 0.0).clamp(0.0, 1.0);
    coherent + decorrelated * (incoherent - coherent)
}

/// A cursor over consecutive source frames that resolves the region's wrap once.
///
/// [`frame`] answers one arbitrary index and must therefore divide. A kernel reads a *run* of
/// consecutive indices, where the next answer is the previous one plus one — except at the loop
/// end, where it is the loop start. Holding that state turns sixteen divisions into sixteen
/// increments and one comparison.
#[derive(Debug, Clone, Copy)]
struct Walk {
    /// The current source index before the final bounds clamp.
    at: i64,
    /// `None` when the region does not wrap, and the index is clamped to it instead.
    wrap: Option<(i64, i64)>,
    region: (i64, i64),
    last: i64,
    /// A forward loop's crossfade, fetched frame by frame through [`Seam::frame`] rather than walked:
    /// its wrap is fractional, so there is no whole-frame state for a walk to hold.
    seam: Option<(Seam, Region)>,
}

impl Walk {
    fn new(sample: &Sample, first: i64, region: Region, loop_mode: LoopMode) -> Self {
        let start = region.start.floor() as i64;
        let end = region.end.ceil() as i64;
        let last = sample.len().saturating_sub(1) as i64;
        if loop_mode != LoopMode::Off
            && let Some(seam) = region.seam
        {
            return Self {
                at: first,
                wrap: None,
                region: (start, end),
                last,
                seam: Some((seam, region)),
            };
        }
        if loop_mode == LoopMode::Off || end <= start {
            return Self {
                at: first,
                wrap: None,
                region: (start, end),
                last,
                seam: None,
            };
        }
        let loop_start = region.loop_start.floor() as i64;
        let len = (region.loop_end.ceil() as i64 - loop_start + 1).max(1);
        Self {
            at: loop_start + (first - loop_start).rem_euclid(len),
            wrap: Some((loop_start, len)),
            region: (start, end),
            last,
            seam: None,
        }
    }

    /// The frame this walk is on.
    fn fetch(&self, sample: &Sample) -> Stereo {
        if let Some((seam, region)) = self.seam {
            return seam.frame(sample, self.at, region);
        }
        let value = sample.frames[self.index()];
        Stereo {
            left: value[0],
            right: value[1],
        }
    }

    fn index(&self) -> usize {
        let index = match self.wrap {
            Some(_) => self.at,
            None => self.at.clamp(self.region.0, self.region.1),
        };
        index.clamp(0, self.last) as usize
    }

    /// The start of a run of `taps` frames that needs no wrapping and no clamping at all, when
    /// this walk is about to make one.
    ///
    /// **The usual case, and worth separating.** A kernel reads sixteen consecutive frames; only
    /// the reads that straddle a loop end or a region edge need the walk's state machine, and the
    /// rest are a contiguous slice the compiler can take at full speed. Returning `None` simply
    /// means falling back to stepping one frame at a time.
    fn contiguous(&self, taps: usize) -> Option<usize> {
        let taps = taps as i64;
        let last_index = self.at.checked_add(taps - 1)?;
        if self.at < 0 || last_index > self.last {
            return None;
        }
        if let Some((seam, region)) = self.seam {
            return seam
                .clear(self.at, last_index, region)
                .then_some(self.at as usize);
        }
        match self.wrap {
            Some((loop_start, len)) => (last_index < loop_start + len).then_some(self.at as usize),
            None => (self.at >= self.region.0 && last_index <= self.region.1)
                .then_some(self.at as usize),
        }
    }

    fn advance(&mut self) {
        self.at += 1;
        if let Some((loop_start, len)) = self.wrap
            && self.at >= loop_start + len
        {
            self.at -= len;
        }
    }
}

/// How far either side of the playhead a re-anchoring head may look for a better splice.
///
/// It has to reach at least one period of the lowest note anyone will play through this, or the
/// search cannot find the alignment that exists; reaching much further buys nothing and costs
/// linearly.
const ALIGN_SEARCH_S: f32 = 0.010;
/// Points compared per candidate offset. The correlation is a similarity measure over a span, not
/// audio, so it is sampled rather than walked.
const ALIGN_POINTS: usize = 64;
/// Output-sample span the comparison covers, clamped to half a window by the caller.
const ALIGN_SPAN_S: f32 = 0.020;
/// The coarse pass steps by this many source frames; the fine pass then sweeps one coarse step
/// either side of the winner. Searching every offset at full resolution costs this factor more for
/// an answer that measured the same.
const ALIGN_COARSE_STRIDE: i32 = 16;
/// How many offsets either side the coarse pass probes, whatever the source rate. The stride widens
/// to hold this, so the search costs the same on a 768 kHz source as on a 48 kHz one.
const ALIGN_COARSE_PROBES: i32 = 30;
/// How much better a shifted candidate must correlate before it displaces a smaller shift.
///
/// **A strict `>` is not a tie-break, because a tie is never exactly a tie in floating point.** On
/// an exactly periodic source the offsets zero and one period are the same alignment, and which of
/// them scores higher is decided by rounding in the accumulation — the first version of this
/// search picked one period away from a unity read for that reason alone. The margin has to sit
/// far above that noise, around 1e-7, and far below a real difference in alignment.
const ALIGN_TIE_MARGIN: f32 = 1.0e-3;
/// How close to a perfect correlation counts as *the same segment*, for the early return that
/// protects the identity at unity.
///
/// **Much tighter than [`ALIGN_TIE_MARGIN`], and the two answer different questions.** The margin
/// asks whether one offset is meaningfully better than another; this asks whether there is anything
/// left to refine at all. Reusing the margin here — which the first version did — suppressed
/// sub-sample refinement for every highly correlated but non-identical segment, which is most
/// sustained material. An exact match scores 1.0 to within a few parts in a hundred million.
const ALIGN_IDENTITY_EPS: f32 = 1.0e-5;

#[cfg(test)]
thread_local! {
    /// How many alignment searches have run. **Test builds only**, and it exists because "the searches
    /// are bursty but the peak is fine" is an argument, not a measurement: a five-second average cannot
    /// see sixteen searches landing on one rendered sample.
    ///
    /// **Thread-local, not a global.** The harness runs tests in parallel in one process, so a shared
    /// counter is incremented by every other test rendering a Stretch voice — which made this pass
    /// alone and fail in a full run, the most misleading way for a measurement to be wrong.
#[cfg(test)]
    pub(crate) static ALIGN_SEARCHES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The best splice offset for a head about to re-anchor: **WSOLA**, after Verhelst and Roelands,
/// *An overlap-add technique based on waveform similarity (WSOLA) for high quality time-scale
/// modification of speech*, ICASSP 1993.
///
/// The head that is about to start reads from `candidate`; the head already sounding reads from
/// `reference`. Both advance at `step`, so comparing the two forward from where they are is
/// comparing what the crossfade is actually about to sum. The returned offset is in source frames
/// and is added to `candidate`.
///
/// **Ties go to zero offset, and that is load-bearing rather than tidy.** At root pitch and unity
/// scan the reference and the candidate are the same point, so a zero offset correlates at exactly
/// 1.0 and nothing can beat it — which is what makes the reader an identity there. A perfectly
/// periodic waveform correlates just as well one period away, and a search that took that offset
/// would be right in spectrum and wrong in phase. Evaluating zero first and improving only on a
/// strict `>` is the whole mechanism.
///
/// The search is clamped so no candidate can read outside the projected region, which is what
/// keeps the bounds contract true for a reader that looks before it commits.
#[allow(clippy::too_many_arguments)]
fn align_offset(
    sample: &Sample,
    region: Region,
    reference_mode: LoopMode,
    candidate_mode: LoopMode,
    reference: f32,
    candidate: f32,
    step: f32,
    span: f32,
    search: i32,
) -> f32 {
    #[cfg(test)]
    ALIGN_SEARCHES.with(|count| count.set(count.get() + 1));
    if search <= 0 {
        return 0.0;
    }
    let dt = (span / ALIGN_POINTS as f32).max(1.0e-3);
    let mix = sample.mono_mix();
    let mut best = 0i32;
    let mut best_score = f32::NEG_INFINITY;
    // **Each series reads in its own head's mode**, which the caller passes and the head renders in,
    // so a splice is scored against exactly what it will play.
    let evaluate = |offset: i32, best: &mut i32, best_score: &mut f32| {
        let shift = offset as f32;
        let mut ab = 0.0f32;
        let mut aa = 0.0f32;
        let mut bb = 0.0f32;
        for point in 0..ALIGN_POINTS {
            let walk = point as f32 * dt * step;
            let a = mono(
                frame(
                    sample,
                    (reference + walk).round() as i64,
                    region,
                    reference_mode,
                ),
                mix,
            );
            let b = mono(
                frame(
                    sample,
                    (candidate + shift + walk).round() as i64,
                    region,
                    candidate_mode,
                ),
                mix,
            );
            ab += a * b;
            aa += a * a;
            bb += b * b;
        }
        let denominator = (aa * bb).sqrt();
        let score = if denominator > 1.0e-12 {
            ab / denominator
        } else {
            0.0
        };
        if score > *best_score + ALIGN_TIE_MARGIN {
            *best_score = score;
            *best = offset;
        }
    };

    // Coarse: zero first, then outward in step. Fine: one coarse step either side of the winner,
    // again from the middle out, so a tie anywhere still resolves toward the smaller shift.
    evaluate(0, &mut best, &mut best_score);
    // **The stride scales with the range, so the probe count does not.** A stride fixed in frames
    // means a 768 kHz source probes fifteen times as many offsets as a 48 kHz one for the same ten
    // milliseconds of search — the cost of a realtime search should not depend on what rate someone
    // recorded at.
    let stride = (search / ALIGN_COARSE_PROBES).max(ALIGN_COARSE_STRIDE);
    let mut offset = stride;
    while offset <= search {
        evaluate(offset, &mut best, &mut best_score);
        evaluate(-offset, &mut best, &mut best_score);
        offset += stride;
    }
    // **Refine in levels, so the fine pass is bounded too.** Sweeping every offset up to the stride
    // put the rate dependence back where the coarse pass had just lost it: at 768 kHz the stride is
    // sixteen times wider, so a unit-resolution sweep of it is sixteen times the work. Each level
    // narrows by the same factor the coarse pass uses and probes the same bounded number of
    // candidates, so the whole search is a couple of hundred evaluations at any rate.
    let mut level = stride;
    while level > 1 {
        let step = (level / ALIGN_COARSE_PROBES).max(1);
        let centre = best;
        let mut offset = step;
        while offset <= level {
            for candidate_offset in [centre + offset, centre - offset] {
                if candidate_offset.abs() <= search {
                    evaluate(candidate_offset, &mut best, &mut best_score);
                }
            }
            offset += step;
        }
        level = step;
    }

    // **Sub-sample alignment, because half a frame is not a small error at the top of the
    // spectrum.** An integer search leaves up to half a frame of residual, which is 0.8 degrees of
    // phase at a 220 Hz fundamental and **ten degrees at its twelfth harmonic** — so the
    // fundamental aligns, the top of the spectrum combs, and what is left is heard as roughness.
    // The correlation is smooth near its peak, so a parabola through the three scores around the
    // winner places it between frames; `read` already interpolates a fractional position.
    //
    // The unity invariant survives by symmetry: identical material makes the two neighbours score
    // equally, the numerator is zero, and the offset stays exactly zero.
    let mut scores = [0.0f32; 3];
    for (slot, neighbour) in [best - 1, best, best + 1].into_iter().enumerate() {
        let mut throwaway_best = 0i32;
        let mut score = f32::NEG_INFINITY;
        evaluate(neighbour, &mut throwaway_best, &mut score);
        scores[slot] = score;
    }
    // **An exact match has nothing to refine, and saying so is what protects the identity.** The
    // symmetry argument above holds in expectation, not on a finite window: at unity the segments
    // either side of a perfect match do not score *exactly* alike, so the parabola nudges the
    // offset off zero and the reader stops being the plain read — measured as unity falling from
    // −100 dB to −95 dB the first time this was built. A correlation of one means the candidate
    // already is the reference.
    if best == 0 && scores[1] >= 1.0 - ALIGN_IDENTITY_EPS {
        return 0.0;
    }
    let curvature = scores[0] - 2.0 * scores[1] + scores[2];
    let fraction = if curvature.abs() > 1.0e-9 {
        (0.5 * (scores[0] - scores[2]) / curvature).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    best as f32 + fraction
}

fn mono(value: Stereo, mix: [f32; 2]) -> f32 {
    value.left * mix[0] + value.right * mix[1]
}

/// Whether this source's channels reinforce or cancel when summed.
///
/// One pass, at load. An anti-phase stereo sample is ordinary audio and it sums to silence, so
/// every analysis that reduces to mono has to know which combination to take before it runs.
fn choose_mono_mix(frames: &[[f32; 2]]) -> [f32; 2] {
    let mut sum = 0.0f64;
    let mut difference = 0.0f64;
    for frame in frames {
        let s = f64::from(frame[0] + frame[1]);
        let d = f64::from(frame[0] - frame[1]);
        sum += s * s;
        difference += d * d;
    }
    if difference > sum {
        [0.5, -0.5]
    } else {
        [0.5, 0.5]
    }
}

/// Hop between period-track frames.
///
/// **Chosen for how fast material changes, not by the quality metric**, which is flat across
/// 50/100/200 ms because a steady tone has nothing for a finer hop to catch — the honest reading of
/// that is that the metric cannot see this constant, not that coarser is free. Ten readings a
/// second follows a melody; the cost halves with every doubling (44 → 26 → 17 ms on a 30 s source).
const PERIOD_TRACK_HOP_S: f32 = 0.100;
/// The track runs on a decimated copy of the source. Pitch detection needs no top octave, and the
/// cost of YIN is quadratic in the lag range, so this is a sixteen-fold saving before anything else.
const PERIOD_DECIMATION: usize = 4;
/// The band the track will commit to a period in. Narrower than `detect_root`'s deliberately: this
/// narrows an *alignment search*, and above the ceiling a period is already shorter than the
/// smallest offset worth resolving, so searching there buys cost and octave errors rather than
/// answers.
const PERIOD_TRACK_FLOOR_HZ: f32 = 40.0;
const PERIOD_TRACK_CEILING_HZ: f32 = 1_000.0;

/// The source's period, frame by frame — what narrows the alignment search to a single cycle.
///
/// **YIN again, on a decimated copy.** The same difference function and cumulative-mean
/// normalisation `detect_root` uses, and for the same reason: autocorrelation prefers longer lags
/// and octave-errors downward on exactly the harmonically rich material a sampler is pointed at.
/// What differs is the question. `detect_root` asks *what note is this sample* once, over the
/// note's length, and answers in MIDI; this asks *what is the period here* every
/// [`PERIOD_TRACK_HOP_S`], all the way through, and answers in source frames.
///
/// **It declines, and the declining is what the fallback is built on.** A hop whose lowest
/// normalised difference never reaches [`PITCH_CONFIDENCE`] stores zero, and a reader that reads a
/// zero uses its fixed window and blind search. Silence, noise, a drum and a chord therefore cost
/// nothing and change nothing, which is the same standard `detect_root` is held to.
fn track_periods(
    frames: &[[f32; 2]],
    mix: [f32; 2],
    sample_rate: f32,
) -> Result<Vec<f32>, SampleError> {
    let decimated_rate = sample_rate / PERIOD_DECIMATION as f32;
    let tau_max = (decimated_rate / PERIOD_TRACK_FLOOR_HZ).ceil() as usize;
    let tau_min = (decimated_rate / PERIOD_TRACK_CEILING_HZ).floor().max(2.0) as usize;
    let window = tau_max * 2;
    let need = window + tau_max;
    let hop = (PERIOD_TRACK_HOP_S * sample_rate).max(1.0) as usize;
    let hops = frames.len() / hop + 1;
    let mut track = reserved(hops)?;
    if frames.len() < need * PERIOD_DECIMATION {
        track.resize(hops, 0.0);
        return Ok(track);
    }

    // A box average before dropping samples. It is a crude anti-alias filter and a good one would
    // be better, but YIN is looking for a repeat rather than a spectrum, and the alternative is
    // aliasing the top octave straight onto the band the period lives in.
    let chunks = frames.chunks(PERIOD_DECIMATION);
    let mut decimated = reserved(chunks.len())?;
    decimated.extend(chunks.map(|chunk| {
        chunk
            .iter()
            .map(|frame| frame[0] * mix[0] + frame[1] * mix[1])
            .sum::<f32>()
            / chunk.len() as f32
    }));

    // Sized by the rate's lag range, not by the source, so the rate bound already bounds them.
    let mut d = vec![0.0f32; tau_max + 1];
    let mut normalised = vec![1.0f32; tau_max + 1];
    for slot in 0..hops {
        let begin = slot * hop / PERIOD_DECIMATION;
        let period = if begin + need <= decimated.len() {
            yin(
                &decimated[begin..begin + need],
                decimated_rate,
                tau_min,
                tau_max,
                window,
                &mut d,
                &mut normalised,
            )
            .map(|note| {
                // `yin` answers in MIDI; the search range wants frames, at the original rate.
                let hz = 440.0 * 2.0f32.powf((note - 69.0) / 12.0);
                sample_rate / hz
            })
            .unwrap_or(0.0)
        } else {
            0.0
        };
        track.push(period);
    }
    Ok(track)
}

/// An empty buffer with room for exactly `capacity` elements, or [`SampleError::Allocation`].
///
/// **Every buffer `Sample::new` sizes from the source comes from here** (audit D14), so a source the
/// frame cap admits cannot abort the process by asking for memory that is not there. Filling one to
/// its capacity never reallocates.
fn reserved<T>(capacity: usize) -> Result<Vec<T>, SampleError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| SampleError::Allocation)?;
    Ok(buffer)
}

/// Where the attacks are, in source frames.
///
/// **Energy-based onset detection with an adaptive threshold**, the standard construction
/// surveyed by Bello, Daudet, Abdallah, Duxbury, Davies and Sandler, *A Tutorial on Onset
/// Detection in Music Signals*, IEEE TSALP 13(5), 2005: a detection function, half-wave
/// rectification, a threshold taken from the signal's own local statistics, and peak picking with
/// a refractory gap.
///
/// Three choices worth naming, because each is the cheap option and each was taken deliberately:
///
/// - **The detection function is the log-energy rise of a first-differenced mono sum.** The first
///   difference is a one-tap high-pass, and it is there because an attack is broadband while the
///   sustain it interrupts usually is not — a bright onset over a loud low note moves this
///   function where plain broadband energy barely notices. It is not a filterbank and does not
///   pretend to be; multiband spectral flux is what S2/S3 would bring if this proves too blunt.
/// - **Log energy, so the function measures a ratio.** A rise from 0.01 to 0.02 is the same onset
///   as 0.1 to 0.2, and only the logarithm says so. This is also why the threshold can be one
///   constant rather than a function of how loud the sample was mastered.
/// - **The threshold is local**, so a sample that gets louder does not stop reporting onsets, and
///   a floor sits under it so the local mean of a near-silent passage cannot be cleared by its own
///   noise.
///
/// Runs once, off the audio thread, inside `Sample::new`. Allocates a detection function the
/// length of the source in hops, plus the onset list it returns.
fn detect_transients(
    frames: &[[f32; 2]],
    mix: [f32; 2],
    sample_rate: f32,
) -> Result<Vec<u32>, SampleError> {
    let hop = (TRANSIENT_HOP_S * sample_rate).round().max(1.0) as usize;
    // **The window is long and the hop is short, and they are independent for a reason that cost
    // a listening round.** A window has to span several periods of the lowest note anyone will
    // sample, or the short-time energy simply swings with the waveform and every cycle reads as an
    // attack: at two hops — 10 ms, less than one period of 55 Hz — a steady bass tone reported
    // **55 onsets**, each one hard-splicing both heads, which is heard as granular chatter on
    // exactly the material a sampler is most often pointed at. At `TRANSIENT_WINDOW_HOPS` it
    // reports none, and the hop still sets the time resolution the refinement starts from.
    let window = hop * TRANSIENT_WINDOW_HOPS;
    if frames.len() < window * 3 {
        // Too short for the detector to have a local neighbourhood to compare against. Declining
        // is correct: a wrong marker re-lays the overlap on material that has no attack in it.
        return Ok(Vec::new());
    }
    let hops = (frames.len() - window) / hop + 1;
    let mut flux = reserved(hops)?;
    flux.resize(hops, 0.0f32);
    let mut previous_log = f32::NAN;
    for (k, value) in flux.iter_mut().enumerate() {
        let base = k * hop;
        let mut energy = 0.0f64;
        // The first difference of the mono sum: one-tap high-pass emphasis, computed inline so
        // nothing the size of the sample is allocated for it.
        for n in base..base + window {
            let here = frames[n][0] * mix[0] + frames[n][1] * mix[1];
            let before = if n == 0 {
                0.0
            } else {
                frames[n - 1][0] * mix[0] + frames[n - 1][1] * mix[1]
            };
            let emphasised = (here - before) as f64;
            energy += emphasised * emphasised;
        }
        let rms = (energy / window as f64).sqrt() as f32;
        let log_energy = (rms + 1.0e-9).ln();
        *value = if previous_log.is_nan() {
            0.0
        } else {
            (log_energy - previous_log).max(0.0)
        };
        previous_log = log_energy;
    }

    let spacing_hops = ((TRANSIENT_MIN_SPACING_S * sample_rate) / hop as f32)
        .ceil()
        .max(1.0) as usize;
    // Counted before the list is reserved, so it is reserved at its length: a bound from the spacing
    // alone would hold hundreds of kilobytes for a source with a handful of attacks, for as long as
    // the sample lives.
    let mut onsets: Vec<u32> = reserved(accepted_hops(&flux, spacing_hops).count())?;
    onsets.extend(
        accepted_hops(&flux, spacing_hops)
            .map(|k| refine_onset(frames, mix, k * hop, hop, window) as u32),
    );
    // **Sort before de-duplicating, because `refine_onset` can reorder.** Refinement moves a
    // marker forward by up to a window, while the acceptance gap between two coarse detections is
    // shorter than that — so a later detection can end up ahead of an earlier one. `seek_marker`
    // binary-searches this list, and `partition_point` on unsorted input does not return a wrong
    // answer loudly, it returns a plausible one.
    onsets.sort_unstable();
    onsets.dedup();
    Ok(onsets)
}

/// The hops the onset detector commits to: a local peak of the detection function that clears the
/// local mean by [`TRANSIENT_THRESHOLD`] and the [`TRANSIENT_FLOOR`], at least `spacing_hops` after
/// the last one accepted.
fn accepted_hops(flux: &[f32], spacing_hops: usize) -> impl Iterator<Item = usize> + '_ {
    let hops = flux.len();
    let mut last_accepted: Option<usize> = None;
    (1..hops.saturating_sub(1)).filter(move |&k| {
        let value = flux[k];
        if value <= 0.0 || value < flux[k - 1] || value < flux[k + 1] {
            return false;
        }
        let low = k.saturating_sub(TRANSIENT_LOCAL_HOPS);
        let high = (k + TRANSIENT_LOCAL_HOPS + 1).min(hops);
        let neighbourhood = &flux[low..high];
        let mean = neighbourhood.iter().sum::<f32>() / neighbourhood.len().max(1) as f32;
        if value < mean * TRANSIENT_THRESHOLD || value < TRANSIENT_FLOOR {
            return false;
        }
        if let Some(previous) = last_accepted
            && k - previous < spacing_hops
        {
            return false;
        }
        last_accepted = Some(k);
        true
    })
}

/// Move a hop-resolution onset onto the frame where the energy actually starts rising.
///
/// A hop is several milliseconds and an attack anchored that late is an attack with its first
/// milliseconds crossfaded away, which is the artefact the marker exists to prevent. The search
/// runs over the hop *before* the reported one as well, because the detection function reports the
/// window in which the rise completed rather than the frame at which it began.
fn refine_onset(
    frames: &[[f32; 2]],
    mix: [f32; 2],
    reported: usize,
    hop: usize,
    window: usize,
) -> usize {
    // **Search forward across the window, because the reported hop is where the window *starts*.**
    // The flux at hop k compares the window beginning at `k × hop` against the one before it, so an
    // attack that has just entered lies near the far end of that window, not behind its start.
    // Searching backwards by a hop — which is what this did first — left markers **32 ms early**
    // with an eight-hop window, putting the attack at a tenth of the spliced head's gain and
    // quietly making transient anchoring do nothing at all.
    let begin = reported;
    let end = (reported + window + hop).min(frames.len().saturating_sub(1));
    if end <= begin {
        return reported.min(frames.len().saturating_sub(1));
    }
    // A short rectified envelope, differenced: the steepest rise inside the neighbourhood.
    let span = (hop / 4).max(1);
    let mut best = reported.min(end);
    let mut best_rise = f32::NEG_INFINITY;
    let mut n = begin;
    while n + span * 2 <= end {
        let before: f32 = frames[n..n + span]
            .iter()
            .map(|f| (f[0] * mix[0] + f[1] * mix[1]).abs())
            .sum::<f32>()
            / span as f32;
        let after: f32 = frames[n + span..n + span * 2]
            .iter()
            .map(|f| (f[0] * mix[0] + f[1] * mix[1]).abs())
            .sum::<f32>()
            / span as f32;
        let rise = after - before;
        if rise > best_rise {
            best_rise = rise;
            best = n + span;
        }
        n += span.max(1);
    }
    best
}

fn frame(sample: &Sample, index: i64, region: Region, loop_mode: LoopMode) -> Stereo {
    if loop_mode != LoopMode::Off
        && let Some(seam) = region.seam
    {
        return seam.frame(sample, index, region);
    }
    frame_plain(sample, index, region, loop_mode)
}

/// A whole frame through the integer wrap — the only fetch this crate had before [`Seam`]. Every
/// looping region now fetches through its seam; this remains for one-shots, Grain's loop-blind reads
/// and a loop under [`MIN_SEAM_LOOP_FRAMES`], which only points less than a frame apart can make.
fn frame_plain(sample: &Sample, index: i64, region: Region, loop_mode: LoopMode) -> Stereo {
    let start = region.start.floor() as i64;
    let end = region.end.ceil() as i64;
    let index = if loop_mode == LoopMode::Off || end <= start {
        index.clamp(start, end)
    } else {
        let loop_start = region.loop_start.floor() as i64;
        let loop_end = region.loop_end.ceil() as i64;
        let len = (loop_end - loop_start + 1).max(1);
        loop_start + (index - loop_start).rem_euclid(len)
    }
    .clamp(0, sample.len().saturating_sub(1) as i64) as usize;
    let value = sample.frames[index];
    Stereo {
        left: value[0],
        right: value[1],
    }
}

fn pan(value: Stereo, pan: f32) -> Stereo {
    let (left, right) = pan_gains(pan);
    Stereo {
        left: value.left * left,
        right: value.right * right,
    }
}

/// The equal-power pair for one pan position.
///
/// Split out so a grain can latch it at spawn: a grain's pan does not change while it lives, and
/// paying two transcendentals per sample to recompute a constant was measurable.
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = (sanitise(pan, 0.0).clamp(-1.0, 1.0) + 1.0) * PI * 0.25;
    (
        angle.cos() * std::f32::consts::SQRT_2,
        angle.sin() * std::f32::consts::SQRT_2,
    )
}

/// The converter's level count for a bit depth.
///
/// **The 16-bit floor at `character = 0` is behaviour, not waste**: the machine this emulates put a
/// real converter in the path, and Clean means that converter rather than none.
fn quantizer_levels(bits: f32) -> f32 {
    (1u32 << (bits.clamp(5.0, 16.0) as u32 - 1)) as f32
}

fn soft_clip(value: f32) -> f32 {
    value / (1.0 + value.abs())
}

fn sanitise(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

fn flush(value: f32) -> f32 {
    if value.is_finite() && value.abs() >= 1.0e-30 {
        value
    } else {
        0.0
    }
}

/// The performance inputs that are **per channel**, for the sample being rendered.
///
/// **Per channel and not per voice**, because that is what they are: a wheel belongs to a MIDI
/// channel and every voice on it reads the same one. Each voice projects its own channel's value
/// into its frame, which is `plan-modulation-routing.md` §4.1's *the same value in every voice*
/// narrowed to *every voice on that channel* — so no third frame scope is needed to express it.
///
/// **An array rather than one scalar**, because a scalar silently made every voice read channel 0:
/// two voices sounding on different channels shared one wheel, which is wrong on any MPE controller
/// and on any multi-timbral part.
#[derive(Debug, Clone, Copy)]
pub struct Performance {
    /// Each channel's mod wheel, `0..=1`.
    pub wheel: [f32; MIDI_CHANNELS],
    /// Each channel's pitch bender, `-1..=1`.
    pub bend: [f32; MIDI_CHANNELS],
    /// Each channel's *channel* pressure, `0..=1`, **retained**.
    ///
    /// A per-note message addresses one voice and needs no retention; a channel message describes
    /// the channel, so a note started afterwards inherits it. Without that, pressing a key while
    /// already leaning on the keyboard gave a voice no pressure at all until the next message.
    pub pressure: [f32; MIDI_CHANNELS],
}

/// How many MIDI channels the engine keeps performance state for.
pub const MIDI_CHANNELS: usize = 16;

impl Default for Performance {
    fn default() -> Self {
        Self {
            wheel: [0.0; MIDI_CHANNELS],
            bend: [0.0; MIDI_CHANNELS],
            pressure: [0.0; MIDI_CHANNELS],
        }
    }
}

/// What an LFO draws.
///
/// **One sine was enough for a fixed drift and is not enough for two general-purpose sources.**
/// The periodic four plus sample-and-hold, which is the one that is not a waveform and the reason
/// the set is worth naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    RampUp,
    RampDown,
    Square,
    /// A new random value each cycle, held between draws.
    SampleHold,
}

/// One free-running low-frequency oscillator.
///
/// **Global and free-running**, which is what the one this replaces was, and what makes two layers
/// drift against each other rather than restart together on every note.
#[derive(Debug, Clone, Copy, Default)]
struct Lfo {
    phase: f32,
    /// The value `SampleHold` is currently holding, redrawn when the phase wraps.
    held: f32,
    rng: u64,
    /// The stream's starting point, so a reset is a **reset** rather than one more draw.
    ///
    /// Without it `reset` advanced whatever state playing had left behind, so what the instrument
    /// produced after a reset depended on what it had produced before one — which is exactly the
    /// determinism a reset exists to restore.
    seed: u64,
}

impl Lfo {
    fn new(seed: u64) -> Self {
        let mut lfo = Self {
            phase: 0.0,
            held: 0.0,
            rng: seed,
            seed,
        };
        lfo.reset();
        lfo
    }

    /// This sample's value, bipolar, then advances the phase.
    fn tick(&mut self, rate_hz: f32, shape: LfoShape, sample_rate: f32) -> f32 {
        let value = match shape {
            LfoShape::Sine => (TAU * self.phase).sin(),
            LfoShape::Triangle => 4.0 * (self.phase - (self.phase + 0.5).floor()).abs() - 1.0,
            LfoShape::RampUp => 2.0 * self.phase - 1.0,
            LfoShape::RampDown => 1.0 - 2.0 * self.phase,
            LfoShape::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            LfoShape::SampleHold => self.held,
        };
        let rate = sanitise(rate_hz, 0.25).clamp(0.01, 40.0);
        let advanced = self.phase + rate / sample_rate.max(1.0);
        if advanced >= 1.0 {
            // One draw per cycle, taken on the wrap, so the held value changes at the rate the
            // control says rather than at some multiple of it.
            self.held = split_mix(&mut self.rng) * 2.0 - 1.0;
        }
        self.phase = advanced.fract();
        value
    }

    /// **The first held value is drawn here, not left at zero.**
    ///
    /// Sample-and-hold only draws on a phase wrap, so a held value initialised to zero means the
    /// shape produces *nothing* until the first cycle completes — a hundred seconds at the slowest
    /// rate this control offers. Drawing one at reset makes the first cycle a real value like every
    /// cycle after it, and keeps the stream deterministic because the seed is fixed.
    fn reset(&mut self) {
        self.phase = 0.0;
        // **From the seed, not from wherever the stream had got to.** A reset must produce the same
        // instrument every time, and drawing from the live state made the first held value depend on
        // how long the last session played.
        self.rng = self.seed;
        self.held = split_mix(&mut self.rng) * 2.0 - 1.0;
    }
}

/// How many presses the ledger remembers for last-note priority. Sixteen is a keyboard's worth of
/// fingers and then some; a seventeenth press drops the oldest entry rather than growing.
pub const MAX_HELD: usize = 16;

/// The presses a player is currently holding, newest last.
///
/// Mono and Legato read it to decide where the single voice goes when a key is released under
/// other held keys. It is maintained in every mode so a mode change under held notes does not
/// start from an empty ledger.
/// One held press: the note, and the per-note state that belongs to *that press*.
///
/// **A mono fallback restores a note, so it has to restore the note's state with it.** Releasing the
/// top of a trill hands the voice back to a key that was pressed earlier — a different note, with its
/// own velocity, its own pressure at the moment it was pressed, and its own random draw. Carrying the
/// released key's values across made the pressure wrong on a same-channel trill and made the random
/// per *phrase* rather than per note, which is not what its declared lifetime says.
#[derive(Debug, Clone, Copy)]
struct Press {
    note: Note,
    /// The channel pressure this press started with.
    pressure: f32,
    /// The draw this press was given. Restored on fallback, so a note sounds as it did.
    random: f32,
    /// Volume expression as a gain ratio, as this press last had it.
    expression_gain: f32,
    /// Pan expression, as this press last had it.
    expression_pan: f32,
}

#[derive(Debug, Clone)]
struct Ledger {
    held: [Press; MAX_HELD],
    len: usize,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            held: [Press {
                note: Note::midi(60, 0.0),
                pressure: 0.0,
                random: 0.0,
                expression_gain: 1.0,
                expression_pan: 0.0,
            }; MAX_HELD],
            len: 0,
        }
    }
}

impl Ledger {
    fn press(&mut self, press: Press) {
        self.remove(press.note.id, press.note.channel, press.note.key);
        if self.len == MAX_HELD {
            self.held.rotate_left(1);
            self.len -= 1;
        }
        self.held[self.len] = press;
        self.len += 1;
    }

    /// Removes the newest press this event addresses. An id wins; without one the newest press on
    /// the same channel and key is released, which is the same ownership rule the voices use.
    fn remove(&mut self, id: Option<u32>, channel: u8, key: u8) -> Option<Press> {
        let found = (0..self.len).rev().find(|&index| {
            let held = self.held[index].note;
            match (id, held.id) {
                (Some(id), Some(held_id)) => id == held_id,
                (None, None) => held.channel == channel && held.key == key,
                _ => false,
            }
        })?;
        let press = self.held[found];
        for index in found..self.len - 1 {
            self.held[index] = self.held[index + 1];
        }
        self.len -= 1;
        Some(press)
    }

    fn newest(&self) -> Option<Press> {
        (self.len > 0).then(|| self.held[self.len - 1])
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// One sounding grain, as a display needs it: where it began, where it has got to, how loud it is.
///
/// **Live, not historical.** An onset log answers *when did grains start*; this answers *where are
/// they now*, which is what a picture drawn over the waveform has to show. The pair of positions is
/// the whole point — a grain is a read travelling through the source, so the segment between them
/// is the part of the sample that grain is sounding, and watching it move is watching Scan speed
/// and the grain's own pitch do their work.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GrainView {
    /// The frame it started reading at.
    pub start: f32,
    /// The frame it is reading now. Behind `start` when the grain runs backwards.
    pub current: f32,
    /// Its window weight, `0..=1` — so a grain can be drawn fading in and out rather than blinking.
    pub weight: f32,
}

#[derive(Debug, Clone)]
pub struct Engine {
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    chronology: u64,
    lfos: [Lfo; 2],
    /// The topology every voice shares, compacted **once** rather than once per voice.
    ///
    /// Topology is the same for all eight voices because it comes from parameters, so rebuilding it
    /// per voice would be eight times the work for one answer. Rebuilt only when `present` changes.
    topology: routing::Routing,
    ledger: Ledger,
    /// The per-channel performance inputs, set by events like every other piece of live state.
    performance: Performance,
    /// The voice mode last seen in a press or a rendered sample.
    ///
    /// `note_off` has no parameters of its own and a release must not consult a different mode
    /// from the press it answers, so the engine remembers the one it was last told.
    mode: VoiceMode,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(48_000.0)
    }
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            // Force the kernel table off the audio thread: a callback must never run its
            // initialiser.
            sample_rate: {
                std::sync::LazyLock::force(&UNITY_KERNEL);
                std::sync::LazyLock::force(&HANN);
                std::sync::LazyLock::force(&WINDOW_MOMENTS);
                valid_sample_rate(sample_rate)
            },
            voices: std::array::from_fn(|_| Voice::default()),
            chronology: 0,
            // Different seeds, so two sample-and-hold LFOs at the same rate are not one LFO.
            lfos: [
                Lfo::new(0x243f_6a88_85a3_08d3),
                Lfo::new(0x1319_8a2e_0370_7344),
            ],
            // A framework-free engine starts with no musical patch policy. The plugin supplies its
            // compiled Init topology before rendering, just as it supplies every later edit.
            topology: routing::Routing::new(),
            performance: Performance::default(),
            ledger: Ledger::default(),
            mode: VoiceMode::Poly,
        }
    }

    /// Fills `out` with every grain sounding on `layer`, and returns how many were written.
    ///
    /// **Read straight off the voices, once a block.** There is no log and nothing to drain: a
    /// grain's whole state is its start, its age, its step and its length, so where it is now is
    /// arithmetic rather than history. Overflowing `out` stops early — a display that has run out
    /// of room has already shown more marks than can be told apart.
    pub fn grain_views(&self, layer: usize, shape: f32, out: &mut [GrainView]) -> usize {
        let mut found = 0;
        for voice in &self.voices {
            if !voice.active {
                continue;
            }
            let state = &voice.layers[layer.min(1)];
            for grain in &state.grains[..state.live] {
                if found >= out.len() {
                    return found;
                }
                let age = grain.age as f32;
                let phase = if grain.duration == 0 {
                    0.0
                } else {
                    age / grain.duration as f32
                };
                out[found] = GrainView {
                    start: grain.position,
                    current: grain.position + age * grain.step,
                    weight: grain_window(shape, phase),
                };
                found += 1;
            }
        }
        found
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = valid_sample_rate(sample_rate);
    }

    pub fn note_on(&mut self, note: Note, samples: [Option<&Sample>; 2], params: &Params) {
        self.chronology = self.chronology.wrapping_add(1);
        self.mode = params.voice_mode;
        // **The ledger remembers what this press *is*, not only which key it was.** A mono fallback
        // restores a held press, and a press's pressure and random draw are its own.
        self.ledger.press(Press {
            note,
            pressure: self.performance.pressure[(note.channel as usize).min(MIDI_CHANNELS - 1)],
            random: Voice::random_for(self.chronology, note.key),
            // A new press starts at unity and centre; per-note expression that arrives later is
            // recorded against this press by `with_expression_target`.
            expression_gain: 1.0,
            expression_pan: 0.0,
        });
        if params.voice_mode != VoiceMode::Poly {
            self.mono_note_on(note, samples, params);
            return;
        }
        let slot = self
            .voices
            .iter()
            .position(|voice| !voice.active)
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, voice)| voice.released)
                    .min_by(|(_, a), (_, b)| a.envelope.level.total_cmp(&b.envelope.level))
                    .map(|(index, _)| index)
            })
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, voice)| voice.born)
                    .map(|(index, _)| index)
                    .unwrap_or(0)
            });
        self.start_voice(slot, note);
        self.reset_layers(slot, samples, params);
    }

    /// Starts a voice in a slot and gives it everything a *new note* is owed.
    ///
    /// **One place, because there are three start paths and they drifted.** Poly allocation, Mono's
    /// first press and Legato's first press all begin a note, and only the first of them inherited
    /// its channel's retained pressure — so a mono patch played while leaning on the keyboard got no
    /// pressure at all until the next message.
    fn start_voice(&mut self, slot: usize, note: Note) {
        self.voices[slot].start(note, self.chronology);
        self.voices[slot].pressure =
            self.performance.pressure[(note.channel as usize).min(MIDI_CHANNELS - 1)];
        self.voices[slot].random = Voice::random_for(self.chronology, note.key);
        self.arm_routing(slot);
    }

    /// Hands a freshly started voice the topology every other voice already has.
    ///
    /// **`Voice::start` resets the voice, and that includes its graph**, so a voice allocated after
    /// the last topology change would render with no routes at all. Called from every path that
    /// starts one.
    fn arm_routing(&mut self, slot: usize) {
        let topology = self.topology;
        self.voices[slot].graph.set_topology(&topology);
    }

    fn reset_layers(&mut self, slot: usize, samples: [Option<&Sample>; 2], params: &Params) {
        for (layer_index, sample) in samples.into_iter().enumerate() {
            if let Some(sample) = sample {
                self.voices[slot].reset_layer(
                    layer_index,
                    sample,
                    &params.layers[layer_index],
                    self.sample_rate,
                );
            }
        }
    }

    /// The single-voice paths. Mono restarts the envelope and the readers on every press; Legato
    /// only re-pitches while a key is already down, so an overlapping press continues the sound
    /// it arrived in the middle of.
    fn mono_note_on(&mut self, note: Note, samples: [Option<&Sample>; 2], params: &Params) {
        let sounding = self
            .voices
            .iter()
            .position(|voice| voice.active && !voice.released);
        let Some(slot) = sounding else {
            // Nothing is held: the first press of a phrase starts from silence in both modes, and
            // there is no previous pitch to glide from.
            let slot = self.quietest_slot();
            self.silence_other_voices(slot);
            self.start_voice(slot, note);
            self.reset_layers(slot, samples, params);
            return;
        };
        self.silence_other_voices(slot);
        let previous = self.voices[slot].pitch_semitones();
        let channel_before = self.voices[slot].note.channel;
        let retained_pressure =
            self.performance.pressure[(note.channel as usize).min(MIDI_CHANNELS - 1)];
        let voice = &mut self.voices[slot];
        let expression = (voice.pressure, voice.expression_gain, voice.expression_pan);
        let overlapping = params.voice_mode == VoiceMode::Legato;
        voice.note = note;
        voice.born = self.chronology;
        voice.released = false;
        // Mono fallback carries the sounding press's expression onto the note that replaces it;
        // dropping it would make a held controller position vanish mid-phrase. **A note arriving on
        // a different channel takes that channel's retained pressure instead**, because the held
        // position it would otherwise inherit belongs to a channel it is not on.
        voice.pressure = if note.channel == channel_before {
            expression.0
        } else {
            retained_pressure
        };
        voice.expression_gain = expression.1;
        voice.expression_pan = expression.2;
        // **The random is per note and this is a new note**, whatever the envelope does. Its
        // declared lifetime is the note's, and a mono phrase is a sequence of notes sharing one
        // voice — so carrying the old draw would make it per *phrase* instead.
        voice.random = {
            let mut seed = self.chronology.wrapping_mul(0x9e37_79b9_7f4a_7c15)
                ^ ((note.key as u64) << 17)
                ^ 0xa5a5_5a5a_c3c3_3c3c;
            standard::random(split_mix(&mut seed))
        };
        voice.glide = if sanitise(params.glide_s, 0.0) > 0.0 {
            (previous - note.key as f32 - sanitise(note.tuning, 0.0)).clamp(-128.0, 128.0)
        } else {
            0.0
        };
        if !overlapping {
            voice.envelope.trigger();
            voice.envelope.level = 0.0;
            // The press that triggers the envelope carries the Velocity source; a Legato press
            // that only re-pitches keeps the phrase's.
            voice.velocity = note.velocity;
            self.reset_layers(slot, samples, params);
        }
    }

    fn quietest_slot(&self) -> usize {
        self.voices
            .iter()
            .position(|voice| !voice.active)
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.envelope.level.total_cmp(&b.envelope.level))
                    .map(|(index, _)| index)
            })
            .unwrap_or(0)
    }

    /// Releases every voice but `keep`, so a mode change from Poly under held notes leaves one
    /// voice playing rather than a chord that no further press can reach.
    fn silence_other_voices(&mut self, keep: usize) {
        for (index, voice) in self.voices.iter_mut().enumerate() {
            if index != keep && voice.active && !voice.released {
                voice.released = true;
                voice.envelope.release();
            }
        }
    }

    pub fn note_off(&mut self, id: Option<u32>, channel: u8, key: u8) {
        let released = self.ledger.remove(id, channel, key);
        if released.is_some()
            && let Some(slot) = self.mono_slot_for(id, channel, key)
            && let Some(previous) = self.ledger.newest()
        {
            self.mono_fall_back(slot, previous);
            return;
        }
        let match_index = if let Some(id) = id {
            self.voices
                .iter()
                .position(|voice| voice.active && !voice.released && voice.note.id == Some(id))
        } else {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, voice)| {
                    voice.active
                        && !voice.released
                        && voice.note.id.is_none()
                        && voice.note.channel == channel
                        && voice.note.key == key
                })
                .max_by_key(|(_, voice)| voice.born)
                .map(|(index, _)| index)
        };
        if let Some(index) = match_index {
            self.voices[index].released = true;
            self.voices[index].envelope.release();
        }
    }

    /// The single sounding voice this release addresses, when there is one and the mode has one.
    fn mono_slot_for(&self, id: Option<u32>, channel: u8, key: u8) -> Option<usize> {
        if self.mode == VoiceMode::Poly {
            return None;
        }
        self.voices.iter().position(|voice| {
            voice.active
                && !voice.released
                && match (id, voice.note.id) {
                    (Some(id), Some(voice_id)) => id == voice_id,
                    (None, None) => voice.note.channel == channel && voice.note.key == key,
                    _ => false,
                }
        })
    }

    /// Returns the mono voice to a key that is still held.
    ///
    /// **No retrigger.** Lifting a finger from the top of a trill is not a new articulation, and a
    /// re-struck envelope there is the classic complaint about mono synths that do retrigger. The
    /// glide is set from the pitch actually sounding, so a fall-back during a glide continues from
    /// where it had reached rather than from where it started.
    fn mono_fall_back(&mut self, slot: usize, previous: Press) {
        let sounding = self.voices[slot].pitch_semitones();
        // **Falling back *restores* a press rather than re-pitching the released one.** Only the
        // pitch used to change here, which left the released key's pressure and random draw on a
        // voice now sounding a different note — wrong on the same channel as well as across
        // channels, and it made the random per *phrase* against its declared per-note lifetime. The
        // ledger carries each press's own state so this can hand back exactly what was pressed.
        let voice = &mut self.voices[slot];
        voice.note = previous.note;
        voice.pressure = previous.pressure;
        voice.random = previous.random;
        // **Per-note expression belongs to a note identity and must not leak across one.** The
        // *press* path carries it deliberately — a held controller position should not vanish
        // mid-phrase when a new key is added — but this is the opposite direction: the note that was
        // carrying it has been released, and the note underneath has expression of its own.
        voice.expression_gain = previous.expression_gain;
        voice.expression_pan = previous.expression_pan;
        voice.glide = (sounding - previous.note.key as f32 - sanitise(previous.note.tuning, 0.0))
            .clamp(-128.0, 128.0);
    }

    /// Note pressure for one press: an id wins, and an id-less event addresses the newest matching
    /// press. Retained through release so a tail keeps the expression it was played with.
    pub fn set_pressure(&mut self, id: Option<u32>, channel: u8, key: u8, pressure: f32) -> bool {
        let value = sanitise(pressure, 0.0).clamp(0.0, 1.0);
        self.record_expression(id, channel, key, |press| press.pressure = value);
        self.with_expression_target(id, channel, key, |voice| voice.pressure = value)
    }

    /// Records a per-note expression value against the held press as well as the voice.
    ///
    /// **Without this a fallback restores what the press started with rather than what it has.** A
    /// held key whose volume was ridden down, then covered by another key and uncovered again,
    /// would jump back to unity.
    fn record_expression(
        &mut self,
        id: Option<u32>,
        channel: u8,
        key: u8,
        write: impl Fn(&mut Press),
    ) {
        let found = (0..self.ledger.len).rev().find(|&index| {
            let held = self.ledger.held[index].note;
            match (id, held.id) {
                (Some(id), Some(held_id)) => id == held_id,
                (None, None) => held.channel == channel && held.key == key,
                _ => false,
            }
        });
        if let Some(index) = found {
            write(&mut self.ledger.held[index]);
        }
    }

    /// Volume expression as a gain ratio, `1.0` being unity.
    pub fn set_expression_gain(
        &mut self,
        id: Option<u32>,
        channel: u8,
        key: u8,
        gain: f32,
    ) -> bool {
        let value = sanitise(gain, 1.0).clamp(0.0, 4.0);
        self.record_expression(id, channel, key, |press| press.expression_gain = value);
        self.with_expression_target(id, channel, key, |voice| voice.expression_gain = value)
    }

    /// Pan expression, `-1..=1`, applied after layer pans and Blend.
    pub fn set_expression_pan(&mut self, id: Option<u32>, channel: u8, key: u8, pan: f32) -> bool {
        let value = sanitise(pan, 0.0).clamp(-1.0, 1.0);
        self.record_expression(id, channel, key, |press| press.expression_pan = value);
        self.with_expression_target(id, channel, key, |voice| voice.expression_pan = value)
    }

    /// Expression addresses a **releasing** voice too: a controller that keeps moving after the
    /// key is up still belongs to the note it was played on until that note ends.
    fn with_expression_target(
        &mut self,
        id: Option<u32>,
        channel: u8,
        key: u8,
        apply: impl FnOnce(&mut Voice),
    ) -> bool {
        let index = if let Some(id) = id {
            self.voices
                .iter()
                .position(|voice| voice.active && voice.note.id == Some(id))
        } else {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, voice)| {
                    voice.active
                        && voice.note.id.is_none()
                        && voice.note.channel == channel
                        && voice.note.key == key
                })
                .max_by_key(|(_, voice)| voice.born)
                .map(|(index, _)| index)
        };
        let Some(index) = index else {
            return false;
        };
        apply(&mut self.voices[index]);
        true
    }

    /// Add a channel pitch-bend delta to every sounding or releasing voice on that channel.
    /// Deltas keep per-note tuning intact when the bend wheel moves after note-on.
    pub fn add_channel_tuning(&mut self, channel: u8, delta_semitones: f32) {
        let delta = sanitise(delta_semitones, 0.0).clamp(-128.0, 128.0);
        for voice in &mut self.voices {
            if voice.active && voice.note.channel == channel {
                voice.note.tuning = (voice.note.tuning + delta).clamp(-128.0, 128.0);
            }
        }
    }

    /// Replace one held note's tuning. IDs win; an ID-less event addresses the newest matching
    /// press, mirroring ID-less release ownership.
    pub fn set_note_tuning(
        &mut self,
        id: Option<u32>,
        channel: u8,
        key: u8,
        semitones: f32,
    ) -> bool {
        let index = if let Some(id) = id {
            self.voices
                .iter()
                .position(|voice| voice.active && !voice.released && voice.note.id == Some(id))
        } else {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, voice)| {
                    voice.active
                        && !voice.released
                        && voice.note.id.is_none()
                        && voice.note.channel == channel
                        && voice.note.key == key
                })
                .max_by_key(|(_, voice)| voice.born)
                .map(|(index, _)| index)
        };
        let Some(index) = index else {
            return false;
        };
        self.voices[index].note.tuning = sanitise(semitones, 0.0).clamp(-128.0, 128.0);
        true
    }

    pub fn choke(&mut self, id: Option<u32>, channel: u8, key: u8) {
        self.ledger.remove(id, channel, key);
        for voice in &mut self.voices {
            let matches = if let Some(id) = id {
                voice.note.id == Some(id)
            } else {
                voice.note.channel == channel && voice.note.key == key
            };
            if voice.active && matches {
                *voice = Voice::default();
            }
        }
    }

    /// The mod wheel for one channel. Every voice on it reads this one.
    pub fn set_wheel(&mut self, channel: u8, value: f32) {
        self.performance.wheel[(channel as usize).min(MIDI_CHANNELS - 1)] = value;
    }

    /// The bender's *position* for one channel, `-1..=1`.
    ///
    /// **Not the tuning.** `add_channel_tuning` is how bend reaches pitch and it is unchanged; this
    /// is the same gesture published as a modulation source, which is §5.2's rule — an existing
    /// gesture path stays where it is while the raw gesture also becomes a source.
    pub fn set_bend_position(&mut self, channel: u8, value: f32) {
        self.performance.bend[(channel as usize).min(MIDI_CHANNELS - 1)] = value;
    }

    /// Takes the **topology** — which pairs are present. **Once per processing interval.**
    ///
    /// Rebuilds the compacted lists and every voice's graph, but only when a presence actually
    /// moved: topology is discrete and changes on a parameter event, never between two samples of
    /// one block.
    ///
    /// **Separate from [`Engine::set_amounts`] because the comparison is not free.** Both were one
    /// call once, made per sample, which compared fifty-five presences on every one of them — a
    /// measurable cost for a question whose answer cannot change inside a block.
    pub fn set_topology(&mut self, routing: &routing::Routing) {
        if self.topology.present == routing.present {
            return;
        }
        self.topology = *routing;
        self.topology.compact();
        for voice in &mut self.voices {
            voice.graph.set_topology(&self.topology);
        }
    }

    /// Takes the **depths**. **Once per sample**, and O(live) rather than O(55).
    ///
    /// **It is paired with [`Engine::set_topology`] and neither replaces the other.** Topology
    /// returns early when no presence moved, so a caller that sets only it never delivers a changed
    /// depth; a caller that sets only this never wires a new route.
    ///
    /// Depths have to arrive this often because they are smoothed and a host can modulate them:
    /// reading one once per block leaves route automation a block stale and CLAP parameter
    /// modulation inaudible.
    #[inline]
    pub fn set_amounts(&mut self, routing: &routing::Routing) {
        let live_len = self.topology.live().len();
        for i in 0..live_len {
            let (target, source) = self.topology.live()[i];
            self.topology.amounts[target as usize][source as usize] =
                routing.amounts[target as usize][source as usize];
        }
    }

    /// Channel pressure: every sounding voice on that channel takes it.
    ///
    /// **This instrument's own pressure is per note** — it reads `PolyPressure`, which is the right
    /// scope for a polyphonic instrument. A controller without MPE sends channel pressure instead,
    /// and the two write the same voice-scoped value: a per-note message addresses one voice, a
    /// channel message addresses every voice on the channel, and **the later message wins**. That
    /// needs no priority table and is what a player expects from an MPE controller layered over a
    /// channel one. A message arriving with no voice sounding writes nothing, because there is no
    /// voice to hold it.
    pub fn set_channel_pressure(&mut self, channel: u8, pressure: f32) -> bool {
        self.performance.pressure[(channel as usize).min(MIDI_CHANNELS - 1)] = pressure;
        let mut touched = false;
        for voice in &mut self.voices {
            if voice.active && voice.note.channel == channel {
                voice.pressure = pressure;
                touched = true;
            }
        }
        touched
    }

    /// Every channel's controller positions: wheel, bender and retained channel pressure.
    pub fn performance(&self) -> Performance {
        self.performance
    }

    /// Puts back controller positions [`Engine::performance`] returned.
    ///
    /// **For a panic that ends notes rather than a player's controllers.** [`Engine::panic`] returns
    /// them to neutral, which is right for a reset or an activation; a host's CC 120 or 123 moves no
    /// wheel, bender or key pressure, and none of them is sent again until it moves.
    pub fn restore_performance(&mut self, performance: Performance) {
        self.performance = performance;
    }

    pub fn panic(&mut self) {
        self.voices = std::array::from_fn(|_| Voice::default());
        for lfo in &mut self.lfos {
            lfo.reset();
        }
        // **The performance inputs go back to neutral too.** They are latched state, so leaving them
        // meant a wheel pushed up before a reset stayed up afterwards — and `panic` is what the
        // plugin's `reset` and `activate` call, so that survived a transport stop and a re-activate.
        self.performance = Performance::default();
        self.ledger.clear();
    }

    /// One sample.
    ///
    /// **The routing and the performance inputs are the engine's own state**, set by
    /// [`Engine::set_routing`] and the event setters rather than carried in [`Params`]. They were
    /// fields on it once, which meant a `Routing` — fifty-five presences, fifty-five amounts and a
    /// compacted list — was built into a fresh `Params` every sample by the caller and copied again
    /// here. Both copies are gone, and the signature is the one every caller already had.
    pub fn render(&mut self, samples: [Option<&Sample>; 2], params: &Params) -> Stereo {
        self.mode = params.voice_mode;
        // **Topology once, not once per voice, and only when it has actually changed.** It comes
        // from parameters, so all eight voices share one answer; rebuilding per voice would be
        // eight times the work for the same list. `Routing::compact` is what the plugin owes after
        // any presence write, and this is the guard that makes a missed one cheap rather than wrong.
        // **The two globals, evaluated once for the whole instrument.** `plan-modulation-routing.md`
        // §4.1's global frame: a shared LFO is the same value in every voice, so each voice
        // publishes this number into its own frame rather than the engine keeping a second frame
        // the summing laws would have to learn to read.
        //
        // **Both tick unconditionally, and only the publication is skipped.** An LFO nobody is
        // listening to must not park: turning a route on would otherwise resume it from wherever it
        // stopped, so a free-running control would depend on when it was wired. The saving that
        // `Routing::needs` buys is in the voice, where the write happens eight times.
        let globals = [
            self.lfos[0].tick(params.lfo_rate_hz, params.lfo_shape, self.sample_rate),
            self.lfos[1].tick(params.lfo2_rate_hz, params.lfo2_shape, self.sample_rate),
        ];
        let modulated = *params;
        // Character resolves once per rendered sample rather than once per grain.
        let character = ResolvedCharacter::new(modulated.character);
        // **The budget is shared, so the share is recomputed every sample.** Dividing it among the
        // layers that are actually sounding is the whole point: a single held note draws on the
        // entire instrument-wide budget, and only a full chord is reduced to what every note used
        // to get unconditionally. A voice that is releasing still owns its grains, so it still
        // counts — `active_voices` includes tailing voices, which is what this needs.
        let layers_loaded = samples.iter().filter(|s| s.is_some()).count().max(1);
        let share = GRAIN_BUDGET / (self.active_voices() * layers_loaded).max(1);
        let share = share.min(MAX_GRAINS_PER_LAYER);
        let mut output = Stereo::default();
        for voice in &mut self.voices {
            output = output.add(voice.render(
                samples,
                &modulated,
                &character,
                self.sample_rate,
                share,
                &self.topology,
                &globals,
                &self.performance,
            ));
        }
        Stereo {
            left: flush(output.left),
            right: flush(output.right),
        }
    }

    pub fn activity(&self) -> Activity {
        if self
            .voices
            .iter()
            .any(|voice| voice.active && !voice.released)
        {
            Activity::Live
        } else if self.voices.iter().any(|voice| voice.active) {
            Activity::Tailing
        } else {
            Activity::Inert
        }
    }

    /// Whether a slot is sounding, for the voice-mode tests.
    #[cfg(test)]
    fn voice_active_for_test(&self, slot: usize) -> bool {
        self.voices[slot].active
    }

    /// A voice's chronology stamp, so a test can find the newest.
    #[cfg(test)]
    fn voice_born_for_test(&self, slot: usize) -> u64 {
        self.voices[slot].born
    }

    /// A voice's volume expression, for the fallback test.
    #[cfg(test)]
    fn voice_expression_gain_for_test(&self, slot: usize) -> f32 {
        self.voices[slot].expression_gain
    }

    /// A voice's pan expression, for the fallback test.
    #[cfg(test)]
    fn voice_expression_pan_for_test(&self, slot: usize) -> f32 {
        self.voices[slot].expression_pan
    }

    /// A voice's key, for the fallback test.
    #[cfg(test)]
    fn voice_key_for_test(&self, slot: usize) -> u8 {
        self.voices[slot].note.key
    }

    /// A voice's per-note random draw.
    #[cfg(test)]
    fn voice_random_for_test(&self, slot: usize) -> f32 {
        self.voices[slot].random
    }

    /// The performance inputs, for the reset test.
    #[cfg(test)]
    fn performance_for_test(&self) -> &Performance {
        &self.performance
    }

    /// One voice's pressure, for the channel-inheritance test.
    #[cfg(test)]
    fn voice_pressure_for_test(&self, slot: usize) -> f32 {
        self.voices[slot].pressure
    }

    /// What one voice's frame currently holds for one source.
    ///
    /// **Reads the slot rather than its effect**, which is what
    /// `a_reused_voice_slot_does_not_read_the_dead_voices_values` needs: the audible consequence of
    /// a leak is one voice inside a sum of eight, and is not separable.
    #[cfg(test)]
    fn voice_frame_for_test(&self, slot: usize, source: usize) -> f32 {
        self.voices[slot].graph.read_for_test(source)
    }

    /// The newest voice's grain stream, for `turning_dither_does_not_disturb_any_other_draw`.
    ///
    /// Reading the stream itself rather than its effects is what makes that test complete: every
    /// draw a grain took is folded into this one number, so nothing can move without it moving.
    #[cfg(test)]
    fn voice_rng_for_test(&self) -> u64 {
        self.voices
            .iter()
            .filter(|voice| voice.active)
            .map(|voice| voice.rng)
            .fold(0, |acc, rng| acc ^ rng)
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|voice| voice.active).count()
    }

    /// The pitch the newest sounding voice is playing, in MIDI semitones: its key, its tuning (the
    /// channel bend and any per-note expression) and its glide.
    ///
    /// **What a pitch control actually delivered**, beside [`Engine::playhead`]. A voice's tuning is
    /// an accumulated sum of bend deltas, so a plugin that scales the bender by a range can only show
    /// that the sum ramps rather than steps by reading it here; a read position resolves a pitch too
    /// coarsely to see one sample's move.
    pub fn sounding_pitch(&self) -> Option<f32> {
        self.voices
            .iter()
            .filter(|voice| voice.active)
            .max_by_key(|voice| voice.born)
            .map(Voice::pitch_semitones)
    }

    /// Where the newest sounding voice is reading, in frames, for one layer.
    ///
    /// **The editor cannot draw a playhead it is not told about.** `LayerVoice`'s read position is
    /// private and nothing published it, so the brief's *"source extent, region, loop and current
    /// voice playhead"* had no data behind its last clause.
    ///
    /// The newest voice rather than all eight: one marker is what a waveform overview can honestly
    /// show, and the note a player just pressed is the one they are looking for. Which field is the
    /// playhead depends on the reader — Repitch advances `position` with key pitch, while Stretch
    /// and Grain both travel independently of it.
    pub fn playhead(&self, layer: usize) -> Option<f32> {
        if layer >= 2 {
            return None;
        }
        self.voices
            .iter()
            .filter(|voice| voice.active && !voice.layers[layer].ended)
            .max_by_key(|voice| voice.born)
            .map(|voice| {
                let read = &voice.layers[layer];
                match read.reader {
                    Reader::Repitch => read.position,
                    Reader::Stretch | Reader::Grain => read.travel,
                }
            })
            .filter(|position| position.is_finite())
    }

    /// How many grains are sounding right now, across every voice and layer.
    ///
    /// The budget is shared, so the only honest way to report occupancy is to count it. The CPU
    /// probe needs this: it used to infer the live total from `MAX_VOICES × 2 × GRAINS_PER_LAYER`,
    /// which stops being true the moment the pool stops being per-layer.
    pub fn active_grains(&self) -> usize {
        self.voices
            .iter()
            .filter(|voice| voice.active)
            .flat_map(|voice| voice.layers.iter())
            .map(|layer| layer.live)
            .sum()
    }
}

fn valid_sample_rate(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() {
        sample_rate.clamp(8_000.0, 384_000.0)
    } else {
        48_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ================= The conversion's equality gates =================
    //
    // `plan-sampler-modulation-and-converter.md` §5. These are the strongest thing in the plan:
    // each states that a route **is** the fixed path it replaces, as an equality on the bits rather
    // than as an argument, which is what `plan-modulation-routing.md`'s decision 1.13 asks a
    // conversion to carry.

    /// Bit equality, **with positive and negative zero counted as the same number**.
    ///
    /// The one place a route is not literally bit-identical to the fixed path it replaces:
    /// `sum_scaled` accumulates from `0.0`, so a single negative term lands as `+0.0` where the
    /// hard-wired expression produced `-0.0`. They compare equal under `==`, differ under
    /// `to_bits`, and nothing downstream can tell them apart — no reciprocal, no `signum`, no
    /// `copysign` reads these sums. Stated here rather than hidden behind a tolerance, because a
    /// tolerance would also hide a real difference.
    fn same_bits(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits() || (a == 0.0 && b == 0.0)
    }

    /// Builds a routing with exactly one pair present, so a gate measures one route and not Init.
    fn only(target: usize, source: usize, amount: f32) -> routing::Routing {
        let mut r = routing::Routing::new();
        r.present[target][source] = true;
        r.amounts[target][source] = amount;
        r.compact();
        r
    }

    /// Algorithm tests opt out of the product's current Init routing. A patch retune must not turn
    /// a reader, filter or normalization oracle into a test of whichever modulation rows are live.
    fn engine_without_routes() -> Engine {
        let mut engine = Engine::default();
        engine.set_topology(&routing::Routing::new());
        engine
    }

    /// **The velocity route is the velocity-sensitivity knob, bit for bit.**
    ///
    /// The knob computed `1 − sense × (1 − velocity)`. With Velocity published as the standard's
    /// `v − 1`, the standard amplitude factor is `1 + sense × (v − 1)` — the same expression term for
    /// term, as `mxm_modulation::product`'s factor was before it. A Velocity published raw into an
    /// additive factor could not reach the knob at any amount: it gives unity at rest rising *above*
    /// unity as you play harder, where the knob gives unity at full velocity falling to silence.
    ///
    /// Falsified by publishing the raw velocity: every velocity below one then disagrees on the
    /// first comparison.
    #[test]
    fn the_velocity_route_is_the_velocity_knob_it_replaces() {
        let mut graph = routing::Graph::new();
        for sense_step in 0..=20 {
            let sense = sense_step as f32 / 20.0;
            let r = only(routing::target::AMPLITUDE, routing::source::VELOCITY, sense);
            graph.set_topology(&r);
            for velocity_step in 0..=20 {
                let velocity = velocity_step as f32 / 20.0;
                graph.begin_sample();
                graph.write(routing::source::VELOCITY, standard::velocity(velocity));
                let routed = standard::amplitude_factor(graph.sum(
                    routing::target::AMPLITUDE,
                    &r,
                    routing::AMPLITUDE_BOUND,
                ));
                // The law this instrument shipped with, written out.
                let knob = 1.0 - sense.clamp(0.0, 1.0) * (1.0 - velocity);
                assert!(
                    same_bits(routed, knob),
                    "sense {sense} velocity {velocity}: route {routed} knob {knob}"
                );
            }
        }
    }

    /// **LFO 1 into the two scan targets is the Travel LFO, bit for bit, opposed direction and all.**
    ///
    /// The hard-wired path computed `motion = sin(phase) × depth`, then added `motion × 0.6` to
    /// layer A's travel and subtracted `motion × 0.47` from layer B's. A route computes
    /// `(amount × source) × scale`, and `routing::FULL_SCALE` carries `+0.6` and `−0.47` in LFO 1's
    /// column — so the multiplies are the same two in the same order, which is what
    /// `plan-modulation-routing.md` §6.2 warns is *not* true if the scale is folded into the amount.
    ///
    /// **The opposed sign is the half worth testing.** Both layers moving together would still pass
    /// a test that only checked that scan speed moved, and it would be the wrong sound: the point of
    /// that LFO is that the layers drift apart.
    #[test]
    fn the_travel_lfo_survives_as_two_routes_that_still_pull_apart() {
        let mut graph = routing::Graph::new();
        for depth_step in 0..=20 {
            let depth = depth_step as f32 / 20.0;
            let mut r = routing::Routing::new();
            r.present[routing::target::SCAN_A][routing::source::LFO1] = true;
            r.amounts[routing::target::SCAN_A][routing::source::LFO1] = depth;
            r.present[routing::target::SCAN_B][routing::source::LFO1] = true;
            r.amounts[routing::target::SCAN_B][routing::source::LFO1] = depth;
            r.compact();
            graph.set_topology(&r);
            for phase_step in 0..64 {
                let lfo = (TAU * phase_step as f32 / 64.0).sin();
                graph.begin_sample();
                graph.write(routing::source::LFO1, lfo);
                let a = graph.sum(routing::target::SCAN_A, &r, 2.0);
                let b = graph.sum(routing::target::SCAN_B, &r, 2.0);
                // The hard-wired arithmetic, written out.
                let motion = lfo * depth;
                assert!(same_bits(a, motion * 0.6), "layer A at {depth}: {a}");
                assert!(same_bits(b, -(motion * 0.47)), "layer B at {depth}: {b}");
                // And they pull apart rather than together, which is the sound.
                if motion.abs() > 1.0e-6 && depth > 0.0 {
                    assert!(
                        a.signum() != b.signum(),
                        "the layers moved the same way: {a} and {b}"
                    );
                }
            }
        }
    }

    /// **The cutoff routes keep the reaches the fixed paths had.**
    ///
    /// The envelope reached `FILTER_ENV_OCTAVES` octaves and pressure `PRESSURE_OCTAVES`, from the
    /// same cutoff — which is exactly why `plan-modulation-routing.md` §5 makes the scale **per
    /// route** rather than per target. One scale for the target would rescale both and move every
    /// stored value's meaning.
    #[test]
    fn the_cutoff_routes_keep_the_reach_each_fixed_path_had() {
        let mut graph = routing::Graph::new();
        for step in 0..=20 {
            let level = step as f32 / 20.0;
            for amount in [-1.0f32, -0.4, 0.0, 0.4, 1.0] {
                let r = only(routing::target::CUTOFF, routing::source::ENVELOPE, amount);
                graph.set_topology(&r);
                graph.begin_sample();
                graph.write(routing::source::ENVELOPE, level);
                let octaves = graph.sum(routing::target::CUTOFF, &r, 10.0);
                assert!(
                    same_bits(octaves, (amount * level) * FILTER_ENV_OCTAVES),
                    "envelope at {level}, amount {amount}: {octaves}"
                );

                let r = only(routing::target::CUTOFF, routing::source::PRESSURE, amount);
                graph.set_topology(&r);
                graph.begin_sample();
                graph.write(routing::source::PRESSURE, level);
                let octaves = graph.sum(routing::target::CUTOFF, &r, 10.0);
                assert!(
                    same_bits(octaves, (amount * level) * PRESSURE_OCTAVES),
                    "pressure at {level}, amount {amount}: {octaves}"
                );
            }
        }
    }

    /// **A reused voice slot reads no dead voice's values** — §4.1's *the voice frame ceases with
    /// its voice*, which is what stops a latched key from holding a patch open.
    ///
    /// **Two earlier versions of this proved nothing, and both failures are worth keeping.** The
    /// first watched velocity, which every voice publishes before anything reads it, so the frame's
    /// contents could not have mattered. The second watched a layer's audio — read one sample late
    /// by construction, which is the right kind of probe — but let the previous voice *decay*, and a
    /// voice that ends naturally ends silent, so there was nothing left in the slot to leak. Both
    /// passed with the reset removed.
    ///
    /// So this asserts the property itself on the slot that was reused, rather than trying to hear
    /// one voice inside a sum: after a voice has published a frame full of values and a new note
    /// takes its slot, **every source reads neutral**.
    ///
    /// **Two independent resets guarantee that, and the test goes red only when both are removed.**
    /// `Voice::start` replaces the whole voice, and the idle path clears the frame when the envelope
    /// closes. Either alone is sufficient, so neither can be falsified on its own — which is worth
    /// saying plainly rather than claiming a falsification this cannot perform. What is pinned here
    /// is the property; the redundancy is cheap and deliberate.
    #[test]
    fn a_reused_voice_slot_does_not_read_the_dead_voices_values() {
        let sample = tone(8192);
        let params = Params {
            attack_s: 0.000_1,
            sustain: 1.0,
            release_s: 0.000_1,
            master: 1.0,
            ..Params::default()
        };
        // Every source live, so the first voice fills its whole frame rather than six slots of it.
        let mut all = routing::Routing::new();
        for source in 0..routing::SOURCES {
            all.present[routing::target::CUTOFF][source] = true;
            all.amounts[routing::target::CUTOFF][source] = 0.5;
        }
        all.compact();

        let mut engine = Engine::new(48_000.0);
        engine.set_topology(&all);
        engine.set_amounts(&all);
        engine.set_wheel(0, 1.0);
        engine.set_bend_position(0, 1.0);
        engine.note_on(Note::midi(84, 1.0), [Some(&sample), None], &params);
        assert!(engine.set_pressure(None, 0, 84, 1.0));
        for _ in 0..512 {
            engine.render([Some(&sample), None], &params);
        }
        // That voice has now published a high key, full velocity, pressure, wheel, bend and audio.
        assert!(
            engine.voice_frame_for_test(0, routing::source::KEY).abs() > 0.1,
            "the first voice never filled its frame, so this proves nothing"
        );

        // Close it, then start a new note on the same slot.
        engine.note_off(None, 0, 84);
        for _ in 0..4_096 {
            engine.render([Some(&sample), None], &params);
        }
        assert_eq!(engine.active_voices(), 0, "the first note never closed");
        engine.set_wheel(0, 0.0);
        engine.set_bend_position(0, 0.0);
        engine.note_on(Note::midi(60, 0.5), [Some(&sample), None], &params);

        // Before it renders a sample, nothing of the dead voice may remain in the slot.
        for source in 0..routing::SOURCES {
            let held = engine.voice_frame_for_test(0, source);
            assert_eq!(
                held, 0.0,
                "source {source} still holds {held} from the voice that used this slot"
            );
        }
    }

    /// **A backward source that starts being read after a long gap reads zero, not a value from
    /// before the gap** — mxm-kit's `crates/mxm-modulation/AGENTS.md`, *A gated publication owes a
    /// `clear`*.
    ///
    /// A layer's audio publishes only while some route reads it, and publishes after every target
    /// has read, so a route from it is one sample late by construction. Remove the route from a
    /// sounding voice and the slot keeps the last tap it held; re-add it and, without the clear, the
    /// first sample reads that tap — however long ago it was, which is the host's buffer sizes
    /// deciding a sound. Read off the voice's own slot before the next sample renders, because that
    /// is the value the first read takes and one voice's tap inside a filtered sum is not separable.
    ///
    /// **Falsified before trusted**: with the clear removed from `Graph::set_topology`, the final
    /// read is the tap from before the gap.
    #[test]
    fn a_re_added_backward_route_reads_zero_rather_than_the_tap_from_before_the_gap() {
        let sample = tone(8192);
        let params = Params {
            attack_s: 0.000_1,
            sustain: 1.0,
            release_s: 0.000_1,
            master: 1.0,
            ..Params::default()
        };
        // Velocity stays routed throughout, so the voice keeps publishing and the frame keeps
        // counting samples across the gap — the realistic case, a patch with other routes live.
        let topology = |with_layer: bool| {
            let mut routing = routing::Routing::new();
            routing.present[routing::target::CUTOFF][routing::source::VELOCITY] = true;
            routing.present[routing::target::CUTOFF][routing::source::LAYER_A] = with_layer;
            routing.amounts[routing::target::CUTOFF][routing::source::LAYER_A] = 0.5;
            routing.compact();
            routing
        };

        let mut engine = Engine::new(48_000.0);
        engine.set_topology(&topology(true));
        engine.set_amounts(&topology(true));
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        for _ in 0..512 {
            engine.render([Some(&sample), None], &params);
        }
        let tap = engine.voice_frame_for_test(0, routing::source::LAYER_A);
        assert!(
            tap.abs() > 1.0e-3,
            "layer A never published a tap, so this proves nothing: {tap}"
        );

        // The route goes; the note keeps sounding and nothing publishes layer A for a long while.
        engine.set_topology(&topology(false));
        for _ in 0..4_096 {
            engine.render([Some(&sample), None], &params);
        }
        assert_eq!(
            engine.voice_frame_for_test(0, routing::source::LAYER_A),
            tap,
            "the premise: an unread source is not published, so its slot keeps the old tap"
        );

        // The route comes back. The first sample's read must be zero.
        engine.set_topology(&topology(true));
        let first = engine.voice_frame_for_test(0, routing::source::LAYER_A);
        assert_eq!(
            first, 0.0,
            "a re-added backward route read {first}, the tap from before the gap"
        );
    }

    /// **Companding is transparent where it has resolution to spare, through the real converter.**
    ///
    /// `the_companding_law_inverts_itself` checks the pure law and deliberately bypasses the
    /// quantiser, so it cannot say what the *stage* does. This drives `ResolvedCharacter::convert`
    /// end to end at 16 bits and asks for the thing that matters: that choosing Companding on a
    /// converter with depth to spare does not audibly change the signal.
    #[test]
    fn companding_at_full_depth_is_transparent_through_the_whole_stage() {
        let render = |converter_type: ConverterType| {
            let resolved = ResolvedCharacter::new(Character {
                rate: 0.0,
                converter: 0.0, // 16 bits
                reconstruction: 0.0,
                input: 0.0,
                converter_type,
                jitter: 0.0,
            });
            (0..4_096)
                .map(|n| {
                    let value = 0.9 * (TAU * 440.0 * n as f32 / 48_000.0).sin();
                    let out = resolved
                        .convert(Stereo {
                            left: value,
                            right: value,
                        })
                        .left;
                    (out - value).abs()
                })
                .fold(0.0f32, f32::max)
        };
        // At 16 bits a step is 1/32768; companding may cost a few of them near full scale, where
        // its steps are coarsest, and that is the trade it exists to make.
        let step = 1.0 / quantizer_levels(16.0);
        let linear = render(ConverterType::Linear);
        let companded = render(ConverterType::Companding);
        assert!(
            linear <= step,
            "a 16-bit linear converter should be within one step: {linear} vs {step}"
        );
        assert!(
            companded < step * 40.0,
            "companding at 16 bits should still be transparent: {companded} vs {step}"
        );
    }

    /// **A reset returns the performance inputs to neutral, and the LFOs to the same stream.**
    ///
    /// `panic` is what the plugin's `reset` and `activate` call, so anything it leaves behind
    /// survives a transport stop and a re-activate. The wheel and the bender are latched state and
    /// were left latched; an LFO's sample-and-hold stream drew from wherever playing had left it, so
    /// what the instrument produced after a reset depended on how long it had played before one.
    #[test]
    fn a_reset_returns_the_performance_inputs_and_the_random_streams_to_their_start() {
        let sample = tone(8192);
        let params = Params {
            lfo_shape: LfoShape::SampleHold,
            lfo_rate_hz: 40.0,
            sustain: 1.0,
            master: 1.0,
            ..Params::default()
        };
        // A route from LFO 1 to the cutoff, so the held value is observable in the output.
        let mut routing = routing::Routing::new();
        routing.present[routing::target::CUTOFF][routing::source::LFO1] = true;
        routing.amounts[routing::target::CUTOFF][routing::source::LFO1] = 1.0;
        routing.compact();

        let run = |engine: &mut Engine| {
            engine.set_topology(&routing);
            engine.set_amounts(&routing);
            engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
            (0..2_048)
                .map(|_| {
                    let f = engine.render([Some(&sample), None], &params);
                    f.left.to_bits()
                })
                .collect::<Vec<u32>>()
        };

        let mut engine = Engine::new(48_000.0);
        let first = run(&mut engine);
        // Play on, so the random stream and the wheel are both somewhere else entirely.
        engine.set_wheel(0, 1.0);
        engine.set_bend_position(0, -1.0);
        for _ in 0..5_000 {
            engine.render([Some(&sample), None], &params);
        }
        engine.panic();
        let second = run(&mut engine);
        assert_eq!(
            first, second,
            "a reset did not return the instrument to the same starting state"
        );

        // And the latched gestures are neutral again, which is what `panic` is for.
        let mut fresh = Engine::new(48_000.0);
        fresh.set_topology(&routing);
        fresh.set_amounts(&routing);
        fresh.set_wheel(3, 1.0);
        fresh.panic();
        fresh.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        fresh.render([Some(&sample), None], &params);
        assert_eq!(
            fresh.performance_for_test().wheel[3],
            0.0,
            "the wheel stayed latched across a reset"
        );
    }

    /// **Two voices on different channels read their own channel's wheel**, and a voice started
    /// after a channel-pressure message inherits it.
    ///
    /// The first is what a single scalar made impossible: every voice read channel 0, so an MPE
    /// controller — or any multi-timbral part — had one wheel between them. The second is why
    /// channel pressure is *retained* rather than only applied to whoever is sounding: pressing a
    /// key while already leaning on the keyboard would otherwise give that note no pressure at all.
    #[test]
    fn each_voice_reads_its_own_channels_performance_inputs() {
        let sample = tone(8192);
        let params = Params {
            sustain: 1.0,
            master: 1.0,
            ..Params::default()
        };
        let mut routing = routing::Routing::new();
        routing.present[routing::target::CUTOFF][routing::source::WHEEL] = true;
        routing.amounts[routing::target::CUTOFF][routing::source::WHEEL] = 1.0;
        routing.compact();

        let mut engine = Engine::new(48_000.0);
        engine.set_topology(&routing);
        engine.set_amounts(&routing);
        engine.set_wheel(0, 0.0);
        engine.set_wheel(1, 1.0);
        for channel in 0..2u8 {
            engine.note_on(
                Note {
                    id: None,
                    channel,
                    key: 60,
                    velocity: 1.0,
                    tuning: 0.0,
                },
                [Some(&sample), None],
                &params,
            );
        }
        engine.render([Some(&sample), None], &params);
        assert_eq!(
            engine.voice_frame_for_test(0, routing::source::WHEEL),
            0.0,
            "the voice on channel 0 read another channel's wheel"
        );
        assert_eq!(
            engine.voice_frame_for_test(1, routing::source::WHEEL),
            1.0,
            "the voice on channel 1 did not read its own wheel"
        );

        // Channel pressure arrives with nothing sounding, then a note starts on that channel.
        let mut engine = Engine::new(48_000.0);
        assert!(
            !engine.set_channel_pressure(2, 0.75),
            "nothing was sounding, so no voice should have been touched"
        );
        engine.note_on(
            Note {
                id: None,
                channel: 2,
                key: 60,
                velocity: 1.0,
                tuning: 0.0,
            },
            [Some(&sample), None],
            &params,
        );
        assert!(
            (engine.voice_pressure_for_test(0) - 0.75).abs() < 1.0e-6,
            "a voice started after the message did not inherit its channel's pressure"
        );
    }

    /// **A note start inherits its channel's pressure in every voice mode, and the per-note random
    /// is per note in every voice mode.**
    ///
    /// There are three start paths — Poly allocation, Mono's first press and Legato's first press —
    /// and only the first inherited. A mono patch played while already leaning on the keyboard got no
    /// pressure at all until the next message. Mono's *fallback* path is different again: it reuses a
    /// sounding voice deliberately, carrying the held expression onto the note that replaces it, so
    /// what it must not carry is the pressure of a **different channel**, or the random draw, whose
    /// declared lifetime is one note rather than one phrase.
    #[test]
    fn every_voice_mode_starts_a_note_with_its_own_channels_pressure_and_its_own_random() {
        let sample = tone(8192);
        for mode in [VoiceMode::Poly, VoiceMode::Mono, VoiceMode::Legato] {
            let params = Params {
                voice_mode: mode,
                sustain: 1.0,
                master: 1.0,
                ..Params::default()
            };
            let mut engine = Engine::new(48_000.0);
            // Pressure arrives on channel 1 with nothing sounding.
            engine.set_channel_pressure(1, 0.6);
            engine.note_on(
                Note {
                    id: None,
                    channel: 1,
                    key: 60,
                    velocity: 1.0,
                    tuning: 0.0,
                },
                [Some(&sample), None],
                &params,
            );
            let slot = (0..MAX_VOICES)
                .find(|s| engine.voice_active_for_test(*s))
                .expect("no voice started");
            assert!(
                (engine.voice_pressure_for_test(slot) - 0.6).abs() < 1.0e-6,
                "{mode:?}: the first note did not inherit its channel's pressure"
            );
            let first_random = engine.voice_random_for_test(slot);

            // A second note, on a different channel with a different retained pressure. In Mono and
            // Legato this reuses the sounding voice; in Poly it takes a new slot.
            engine.set_channel_pressure(2, 0.2);
            engine.note_on(
                Note {
                    id: None,
                    channel: 2,
                    key: 64,
                    velocity: 1.0,
                    tuning: 0.0,
                },
                [Some(&sample), None],
                &params,
            );
            let second = (0..MAX_VOICES)
                .filter(|s| engine.voice_active_for_test(*s))
                .max_by_key(|s| engine.voice_born_for_test(*s))
                .expect("no voice for the second note");
            assert!(
                (engine.voice_pressure_for_test(second) - 0.2).abs() < 1.0e-6,
                "{mode:?}: a note on another channel kept the first channel's pressure"
            );
            assert_ne!(
                engine.voice_random_for_test(second).to_bits(),
                first_random.to_bits(),
                "{mode:?}: the second note reused the first note's random draw"
            );

            // **Releasing the top of the trill falls back to the held key**, which is a third note
            // start in Mono and Legato and was carrying the released key's state with it.
            if mode != VoiceMode::Poly {
                // **Exact restoration, not merely "different".** The fallback hands the voice back
                // to a press that was made earlier, so what it must restore is *that press's* own
                // pressure and draw — the values it had when it was pressed, which the ledger keeps.
                // Asserting only that the draw changed would pass on a fresh draw, which is a
                // different note again.
                let first_pressure = 0.6;
                engine.note_off(None, 2, 64);
                let back = (0..MAX_VOICES)
                    .find(|s| engine.voice_active_for_test(*s))
                    .expect("the fallback left nothing sounding");
                assert!(
                    (engine.voice_pressure_for_test(back) - first_pressure).abs() < 1.0e-6,
                    "{mode:?}: the fallback did not restore the held press's pressure"
                );
                assert_eq!(
                    engine.voice_random_for_test(back).to_bits(),
                    first_random.to_bits(),
                    "{mode:?}: the fallback did not restore the held press's own random draw"
                );
                assert_eq!(
                    engine.voice_key_for_test(back),
                    60,
                    "{mode:?}: the fallback did not return to the held key"
                );
            }
        }
    }

    /// **Per-note expression does not leak across a mono fallback.**
    ///
    /// A held key whose volume was ridden down, then covered by a second key and uncovered again,
    /// must come back at the value *it* had — not at the covering note's, and not reset to unity.
    /// The press path deliberately carries expression forward, because a held controller position
    /// should not vanish when a new key is added; this is the opposite direction, where the note
    /// carrying it has been released and the note underneath has expression of its own.
    #[test]
    fn a_mono_fallback_restores_the_held_presss_own_expression() {
        let sample = tone(8192);
        let params = Params {
            voice_mode: VoiceMode::Mono,
            sustain: 1.0,
            master: 1.0,
            ..Params::default()
        };
        let mut engine = Engine::new(48_000.0);
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        assert!(engine.set_expression_gain(None, 0, 60, 0.25));
        assert!(engine.set_expression_pan(None, 0, 60, -0.5));

        // A second key covers it and rides its own expression somewhere else entirely.
        engine.note_on(Note::midi(64, 1.0), [Some(&sample), None], &params);
        assert!(engine.set_expression_gain(None, 0, 64, 4.0));
        assert!(engine.set_expression_pan(None, 0, 64, 1.0));

        // Uncovering returns to the first key, which must arrive as it was left.
        engine.note_off(None, 0, 64);
        let slot = (0..MAX_VOICES)
            .find(|s| engine.voice_active_for_test(*s))
            .expect("the fallback left nothing sounding");
        assert_eq!(engine.voice_key_for_test(slot), 60);
        assert!(
            (engine.voice_expression_gain_for_test(slot) - 0.25).abs() < 1.0e-6,
            "the fallback kept the covering note's volume: {}",
            engine.voice_expression_gain_for_test(slot)
        );
        assert!(
            (engine.voice_expression_pan_for_test(slot) + 0.5).abs() < 1.0e-6,
            "the fallback kept the covering note's pan: {}",
            engine.voice_expression_pan_for_test(slot)
        );
    }

    // ================= The converter: companding and dither =================

    /// Every converter setting, so a preservation check covers the grid rather than a corner.
    fn converter_grid() -> Vec<Character> {
        let mut out = Vec::new();
        for rate in [0.0f32, 0.2, 0.5, 1.0] {
            for converter in [0.0f32, 0.25, 0.6, 1.0] {
                for reconstruction in [0.0f32, 0.45, 1.0] {
                    for input in [0.0f32, 0.4] {
                        out.push(Character {
                            rate,
                            converter,
                            reconstruction,
                            input,
                            converter_type: ConverterType::Linear,
                            jitter: 0.0,
                        });
                    }
                }
            }
        }
        out
    }

    /// **Clean has to be clean, and so does every other linear setting.**
    ///
    /// The converter DOX's *both endpoints are unchanged, so Clean is still exactly transparent* is
    /// a bit-identity claim, and adding two stages to `convert` is exactly the change that could
    /// break it without moving any number far enough to notice by ear. So this is `==` on the bits,
    /// over the whole rate x depth x reconstruction x drive grid, against a hand-written reference
    /// that is the arithmetic the function had **before** the compander and the dither arrived.
    ///
    /// Falsified by making the dither gate `>=` rather than `>`: the zero-amount draw then perturbs
    /// every sample and this goes red on the first frame.
    #[test]
    fn a_linear_converter_with_no_dither_is_the_converter_this_instrument_shipped() {
        for character in converter_grid() {
            let resolved = ResolvedCharacter::new(character);
            for step in 0..2_048 {
                let value = step as f32 / 1_024.0 - 1.0;
                let frame = Stereo {
                    left: value,
                    right: -value * 0.5,
                };
                // The old arithmetic, written out: quantise on a uniform grid, then drive.
                let quantized = Stereo {
                    left: (frame.left * resolved.levels).round() * resolved.inverse_levels,
                    right: (frame.right * resolved.levels).round() * resolved.inverse_levels,
                };
                let want = if resolved.input <= 0.0 {
                    quantized
                } else {
                    let drive = 1.0 + resolved.input * 15.0;
                    let norm = soft_clip(drive);
                    Stereo {
                        left: soft_clip(quantized.left * drive) / norm,
                        right: soft_clip(quantized.right * drive) / norm,
                    }
                };
                let got = resolved.convert(frame);
                assert_eq!(
                    got.left.to_bits(),
                    want.left.to_bits(),
                    "{character:?} at {value}"
                );
                assert_eq!(got.right.to_bits(), want.right.to_bits(), "{character:?}");
            }
        }
    }

    /// **Companding holds its signal-to-junk ratio where linear collapses**, which is the entire
    /// reason mxm-kit's `docs/oscillators/14-samplers.md` §14.8 names µ-law as the one to
    /// implement.
    ///
    /// §14.2 measured a 440 Hz sine at 8 bits: linear sits near −49 dB at full scale and falls to
    /// −15 dB by −36 dBFS, while µ-law holds near −36 dB across that span. This reproduces the
    /// **shape of that result through this plugin** rather than trusting the chapter: at a quiet
    /// level the companded converter must beat the linear one by a wide margin, and at full scale
    /// the linear one must win — the trade is what makes it the right choice, and a test that only
    /// checked "companding is better" would pass on a converter that was simply louder.
    #[test]
    fn companding_beats_linear_where_music_lives_and_loses_at_full_scale() {
        // Junk is everything that is not the wanted bin, measured on the quantiser alone so the
        // readers and the filter cannot contribute.
        let junk = |converter_type: ConverterType, level: f32| {
            // `character_bits` is `16 - sqrt(c)*11`, so this is **8 bits** — the depth §14.2's
            // table is taken at, rather than the 5-bit floor.
            let character = Character {
                rate: 0.0,
                converter: (8.0f32 / 11.0).powi(2),
                reconstruction: 0.0,
                input: 0.0,
                converter_type,
                jitter: 0.0,
            };
            let resolved = ResolvedCharacter::new(character);
            let frames: Vec<f32> = (0..4_096)
                .map(|n| {
                    let value = level * (TAU * 440.0 * n as f32 / 48_000.0).sin();
                    resolved
                        .convert(Stereo {
                            left: value,
                            right: value,
                        })
                        .left
                })
                .collect();
            let bin = 440.0 * 4_096.0 / 48_000.0;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (n, v) in frames.iter().enumerate() {
                let angle = -std::f64::consts::TAU * bin * n as f64 / 4_096.0;
                re += *v as f64 * angle.cos();
                im += *v as f64 * angle.sin();
            }
            // A real sine of amplitude A has mean-square power A^2/2, and an exact-bin DFT of it
            // has magnitude A*N/2 — so the power is 2|X|^2/N^2. Getting this wrong by the factor of
            // two made `total - signal` negative at full scale and every level read -300 dB.
            let signal = 2.0 * (re * re + im * im) / (4_096.0 * 4_096.0);
            let total: f64 = frames
                .iter()
                .map(|v| (*v as f64) * (*v as f64))
                .sum::<f64>()
                / frames.len() as f64;
            10.0 * ((total - signal).max(1.0e-30) / signal).log10()
        };

        let quiet_linear = junk(ConverterType::Linear, 0.015_8);
        let quiet_companded = junk(ConverterType::Companding, 0.015_8);
        assert!(
            quiet_companded < quiet_linear - 10.0,
            "at -36 dBFS companding should be far cleaner: linear {quiet_linear:.1} dB, \
             companded {quiet_companded:.1} dB"
        );
        let loud_linear = junk(ConverterType::Linear, 1.0);
        let loud_companded = junk(ConverterType::Companding, 1.0);
        assert!(
            loud_companded > loud_linear,
            "at full scale linear should win, which is the trade: linear {loud_linear:.1} dB, \
             companded {loud_companded:.1} dB"
        );
    }

    /// **A jittering clock still runs forwards**, which is what separates it from scrambling.
    ///
    /// Sampling instants are ordered: instant `n` happens before instant `n + 1`. Cell `n` is
    /// displaced by `±JITTER_CELLS/2`, so the interval between two instants is
    /// `grid × (1 ± JITTER_CELLS)` and stays positive exactly while `JITTER_CELLS < 1`. An earlier
    /// version used `4.0` because it was louder, and at that width the instants crossed — frames
    /// came back out of order, which is address scrambling under this control's name.
    ///
    /// This walks the whole displacement range rather than trusting the arithmetic, because the
    /// bound is the one property that makes the effect what it claims to be.
    #[test]
    fn a_jittering_clock_never_runs_backwards() {
        // A `const` comparison, so clippy is right that it cannot vary at runtime — but it is the
        // whole point: the constant is what the rest of this test depends on, and a future edit
        // raising it should fail here with the reason rather than in a puzzling ordering assert.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(
                JITTER_CELLS < 1.0,
                "at or past one cell the sampling instants can cross, and this stops being a clock"
            );
        }
        for rate in [0.0f32, 0.2, 0.5, 1.0] {
            let resolved = ResolvedCharacter::new(Character {
                rate,
                converter: 0.0,
                reconstruction: 0.5,
                input: 0.0,
                converter_type: ConverterType::Linear,
                jitter: 1.0,
            });
            let grid = character_rate_divisor(rate);
            let displacement = |index: f32| resolved.jitter_for_test() * (cell_wobble(index) - 0.5);
            // Negative indices too: a reversed layer and a negative scan speed both read backwards
            // through the source, so the hash is asked for cells on both sides of zero.
            let mut previous = f32::NEG_INFINITY;
            for index in -5_000..5_000 {
                let cell = index as f32;
                let instant = cell * grid + displacement(cell);
                assert!(
                    instant > previous,
                    "rate {rate}: instant for cell {cell} landed at or before its predecessor"
                );
                assert!(
                    displacement(cell).abs() <= grid * JITTER_CELLS / 2.0 + 1.0e-4,
                    "rate {rate}: cell {cell} wandered further than the bound allows"
                );
                previous = instant;
            }
        }
    }

    /// **Turning Jitter changes the clock and nothing else.**
    ///
    /// The control this replaced had to be given its own random streams, because drawing from the
    /// voice's would have advanced the stream that sequences grain onsets, position and pitch draws
    /// — a converter knob silently moving every grain. Jitter needs no stream at all: its wobble is
    /// a hash of the cell's own index, so the guarantee is **by construction** rather than by
    /// discipline.
    ///
    /// Pinned anyway, because "by construction" is a property of today's implementation and this is
    /// the exact defect that was found the hard way once. The whole stream is compared, not onset
    /// times, because a position draw could move while onsets stayed put.
    #[test]
    fn turning_jitter_does_not_disturb_any_other_draw() {
        let source = crested(48_000);
        let stream_after = |jitter: f32| {
            let mut params = Params {
                character: Character {
                    rate: 0.6,
                    converter: 0.0,
                    reconstruction: 0.0,
                    input: 0.0,
                    converter_type: ConverterType::Linear,
                    jitter,
                },
                sustain: 1.0,
                master: 1.0,
                ..Params::default()
            };
            params.layers[0].reader = Reader::Grain;
            params.layers[0].loop_mode = LoopMode::Forward;
            params.layers[0].level = 1.0;
            params.layers[0].grain_position_variation = 0.7;
            params.layers[0].grain_spread_semitones = 0.4;
            params.layers[0].grain_reverse_chance = 0.3;
            params.layers[0].grain_stereo_width = 0.6;
            let mut engine = Engine::new(48_000.0);
            engine.note_on(Note::midi(60, 1.0), [Some(&source), None], &params);
            for _ in 0..24_000 {
                engine.render([Some(&source), None], &params);
            }
            engine.voice_rng_for_test()
        };
        assert_eq!(
            stream_after(0.0),
            stream_after(1.0),
            "the jitter drew from the voice's own stream, so it moved the grains"
        );
    }

    /// **Jitter moves the converter's clock, and it is audible where the other stages are not.**
    ///
    /// The control it replaces was removed by the owner for the opposite failing — *"I cannot hear
    /// what it does so it adds no creative character to the sound"* — so the claim worth pinning is
    /// that this one **changes the sound**, and by an amount that scales with the knob.
    ///
    /// Measured as the difference from the un-jittered render of the same signal, which is the
    /// honest quantity: a clock sampling at the wrong instant reads a different value, and the size
    /// of that error is what a player hears as grit.
    #[test]
    fn jitter_changes_what_the_converter_reads_and_more_of_it_changes_more() {
        let source = tone(8192);
        let render = |jitter: f32| {
            let mut params = Params {
                character: Character {
                    // A reduced rate, so the cells are wide enough to hear the clock move.
                    rate: 1.0,
                    converter: 0.0,
                    reconstruction: 0.0,
                    input: 0.0,
                    converter_type: ConverterType::Linear,
                    jitter,
                },
                sustain: 1.0,
                master: 1.0,
                attack_s: 0.000_1,
                ..Params::default()
            };
            params.layers[0].level = 1.0;
            params.layers[0].loop_mode = LoopMode::Forward;
            let mut engine = Engine::new(48_000.0);
            engine.note_on(Note::midi(60, 1.0), [Some(&source), None], &params);
            (0..8_192)
                .map(|_| engine.render([Some(&source), None], &params).left)
                .collect::<Vec<f32>>()
        };
        let clean = render(0.0);
        let difference = |jitter: f32| {
            let jittered = render(jitter);
            let sum: f64 = clean
                .iter()
                .zip(&jittered)
                .map(|(a, b)| ((a - b) as f64) * ((a - b) as f64))
                .sum();
            (sum / clean.len() as f64).sqrt()
        };
        let reference: f64 = {
            let sum: f64 = clean.iter().map(|v| (*v as f64) * (*v as f64)).sum();
            (sum / clean.len() as f64).sqrt()
        };

        // **The whole travel earns its keep, and none of it saturates.** Measured here at a fully
        // reduced rate: 2.5% of the signal at a twentieth of the knob, climbing smoothly to 12.4% at
        // the top.
        //
        // **12% rather than the 72% an earlier version reached, and that is the price of being a
        // clock.** That version displaced one end of the interval and stepped a nominal grid to the
        // other, so at a wide wobble the sampling instants crossed and frames came back out of
        // order — louder, but address scrambling rather than jitter. Ordered instants bound the
        // effect, and the bound is the honest one.
        let steps = [0.05f32, 0.25, 0.5, 0.75, 1.0];
        let mut last = 0.0;
        for step in steps {
            let moved = difference(step);
            assert!(
                moved > last * 1.2,
                "jitter {step} did not read further off the clock than the setting below it:                  {moved:e} against {last:e}"
            );
            last = moved;
        }
        assert!(
            last > reference * 0.08,
            "full jitter should be plainly audible, not a shimmer: {last:e} against {reference:e}"
        );
        // And zero is exact, which is what keeps Clean clean.
        assert_eq!(difference(0.0), 0.0, "zero jitter was not exact");
    }

    /// **Companding is invertible, so a converter with resolution to spare is still transparent.**
    ///
    /// At 16 bits the round trip through the log and its inverse must return what went in, within
    /// the step it is allowed to lose. Without this a sign or a bound error in `expand` would only
    /// show up as a vague loss of level somewhere in a render.
    #[test]
    fn the_companding_law_inverts_itself() {
        for step in 0..4_001 {
            let value = step as f32 / 2_000.0 - 1.0;
            let round_trip = expand(compand(value));
            assert!(
                (round_trip - value).abs() < 1.0e-5,
                "{value} came back as {round_trip}"
            );
            assert_eq!(
                compand(value).signum(),
                value.signum(),
                "companding flipped the sign at {value}"
            );
        }
        // Out of range saturates rather than folding: a voice sums two layers whose levels reach
        // 200%, so this is the ordinary case and not an edge.
        assert_eq!(compand(4.0), compand(1.0));
        assert_eq!(compand(-4.0), compand(-1.0));
    }

    /// A chirp with periodic transients — the same shape the measurement probe uses.
    ///
    /// **A sine is the wrong source for a peak assertion.** Its crest factor is fixed, so a cloud
    /// built from it hides exactly the overshoot the incoherent law has to allow for: this source
    /// peaked at 1.400 where a sine peaked at 0.887, and only one of those numbers clips.
    fn crested(frames: usize) -> Sample {
        Sample::new(
            (0..frames)
                .map(|n| {
                    let t = n as f32 / 48_000.0;
                    let chirp = (TAU * (180.0 * t + 5_200.0 * t * t)).sin();
                    let transient = if n % 4_000 < 80 {
                        1.0 - (n % 4_000) as f32 / 80.0
                    } else {
                        0.0
                    };
                    let value = chirp * 0.45 + transient * 0.35;
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .unwrap()
    }

    fn tone(frames: usize) -> Sample {
        Sample::new(
            (0..frames)
                .map(|n| {
                    let value = (TAU * 220.0 * n as f32 / 48_000.0).sin() * 0.5;
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .unwrap()
    }

    /// A step source: silent first half, tone second half.
    fn stepped(frames: usize) -> Sample {
        Sample::new(
            (0..frames)
                .map(|n| {
                    let value = if n < frames / 2 {
                        0.0
                    } else {
                        (TAU * 220.0 * n as f32 / 48_000.0).sin() * 0.5
                    };
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .unwrap()
    }

    /// Grain's playhead must traverse the region at the rate Travel commands, in both a sparse and
    /// a pool-saturated cloud. Advancing travel once per spawn failed both: the units reduced to
    /// the dimensionless overlap ratio, and a saturated pool drops every spawn, so the read froze
    /// at the region start and the second half of the source was never reached.
    #[test]
    fn grain_travel_traverses_the_region_and_does_not_stall_on_a_saturated_pool() {
        // Four seconds of source, so a full traverse at Travel = 1 takes four seconds.
        let sample = stepped(4 * 48_000);
        // The saturated case has to actually saturate, and what saturates moved: a layer's share of
        // the shared budget reaches MAX_GRAINS_PER_LAYER, so `size x density` has to clear that
        // rather than the sixteen a per-layer pool used to hold. At 1 s grains and 200/s it asks for
        // 200 against a share of 128 and spends the whole render with the share full, which is the
        // state the travel accumulator used to freeze in.
        for (label, size, density) in [("sparse", 0.09_f32, 14.0_f32), ("saturated", 1.0, 200.0)] {
            let mut params = Params::default();
            params.layers[0] = LayerParams {
                reader: Reader::Grain,
                loop_mode: LoopMode::Forward,
                stretch_travel: 1.0,
                grain_size_s: size,
                grain_density_hz: density,
                // Zero scatter keeps position jitter out of it, so only travel can move the read.
                grain_position_variation: 0.0,
                grain_onset_timing: 0.0,
                ..LayerParams::default()
            };
            let mut engine = Engine::default();
            engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);

            // Render the first second (inside the silent half) and the third (inside the tone).
            let mut energy = [0.0f32; 3];
            let mut occupancy = 0usize;
            for second in &mut energy {
                for _ in 0..48_000 {
                    let value = engine.render([Some(&sample), None], &params);
                    *second += value.left * value.left + value.right * value.right;
                    occupancy = occupancy.max(engine.active_grains());
                }
            }
            // The saturated case must genuinely saturate, or it is not testing what it is named
            // for. A pool that grows out from under this case would leave the regression it guards
            // silently untested, which is how a budget change hides a travel defect.
            if label == "saturated" {
                assert!(
                    occupancy >= MAX_GRAINS_PER_LAYER,
                    "the saturated case never filled its share: {occupancy} grains"
                );
            }
            // The first two seconds sit in the silent half and the third in the tone, so an
            // absolute floor is the real assertion; a ratio against a zero baseline proves nothing.
            assert!(
                energy[0] <= 1.0e-6 && energy[1] <= 1.0e-6,
                "{label}: the read left the silent half early: {energy:?}"
            );
            assert!(
                energy[2] > 100.0,
                "{label}: travel did not reach the tone half: {energy:?}"
            );
        }
    }

    #[test]
    fn a_first_note_is_audible_in_every_reader() {
        let sample = tone(48_000);
        for reader in [Reader::Repitch, Reader::Stretch, Reader::Grain] {
            let mut params = Params::default();
            params.layers[0].reader = reader;
            let mut engine = Engine::default();
            engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
            let energy: f32 = (0..4096)
                .map(|_| {
                    let value = engine.render([Some(&sample), None], &params);
                    value.left * value.left + value.right * value.right
                })
                .sum();
            assert!(energy > 1.0, "{reader:?} was silent: {energy}");
        }
    }

    /// **The layers sum, and each one's Level is the whole of how loud it is.**
    ///
    /// Replaces a test of an equal-power Blend across the pair. That control was removed because it
    /// gave a second, disagreeing answer to how loud B was — at Blend 0 a layer's own Level did
    /// nothing at all. What has to stay true is what that test was really guarding: **a layer that
    /// is not there contributes silence rather than attenuating the one that is.**
    #[test]
    fn layers_sum_and_a_missing_one_costs_the_other_nothing() {
        let sample = tone(8192);
        let params = Params::default();
        let energy = |engine: &mut Engine, samples: [Option<&Sample>; 2], params: &Params| {
            let mut sum = 0.0f32;
            for _ in 0..4096 {
                let value = engine.render(samples, params);
                sum += value.left * value.left + value.right * value.right;
            }
            sum
        };
        let note = Note::midi(60, 1.0);

        let mut alone = Engine::default();
        alone.note_on(note, [Some(&sample), None], &params);
        let a_alone = energy(&mut alone, [Some(&sample), None], &params);
        assert!(a_alone > 1.0, "A was silent on its own: {a_alone}");

        // B present but at zero Level must be exactly as good as B absent.
        let mut silent_b = params;
        silent_b.layers[1].level = 0.0;
        let mut with_silent = Engine::default();
        with_silent.note_on(note, [Some(&sample), Some(&sample)], &silent_b);
        let with_silent_b = energy(&mut with_silent, [Some(&sample), Some(&sample)], &silent_b);
        let ratio = with_silent_b / a_alone;
        assert!(
            (0.99..1.01).contains(&ratio),
            "a zero-level B changed A: scaled by {ratio}"
        );

        // And a B at full Level genuinely adds.
        let mut both = Engine::default();
        both.note_on(note, [Some(&sample), Some(&sample)], &params);
        let together = energy(&mut both, [Some(&sample), Some(&sample)], &params);
        assert!(
            together > a_alone * 1.5,
            "summing B added nothing: {a_alone} -> {together}"
        );
    }

    #[test]
    fn id_less_duplicate_releases_are_newest_first() {
        let sample = tone(2048);
        let params = Params::default();
        let mut engine = Engine::default();
        let note = Note::midi(60, 1.0);
        engine.note_on(note, [Some(&sample), None], &params);
        engine.note_on(note, [Some(&sample), None], &params);
        engine.note_off(None, 0, 60);
        assert_eq!(
            engine.voices.iter().filter(|voice| voice.released).count(),
            1
        );
        let released_born = engine
            .voices
            .iter()
            .find(|voice| voice.released)
            .unwrap()
            .born;
        assert_eq!(released_born, 2);
    }

    #[test]
    fn tiny_regions_and_extreme_character_stay_finite() {
        let sample = Sample::new(vec![[0.25, -0.25]], 48_000.0).unwrap();
        let mut params = Params::default();
        params.layers[0].start = 1.0;
        params.layers[0].end = 0.0;
        params.layers[0].loop_mode = LoopMode::Alternate;
        params.character = Character {
            rate: 1.0,
            converter: 1.0,
            reconstruction: 1.0,
            input: 1.0,
            ..Character::default()
        };
        let mut engine = Engine::default();
        engine.note_on(Note::midi(127, 1.0), [Some(&sample), None], &params);
        for _ in 0..4096 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
        }
    }

    #[test]
    fn release_reaches_exact_idle_and_panic_clears_immediately() {
        let sample = tone(4096);
        let params = Params {
            release_s: 0.001,
            ..Params::default()
        };
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        engine.note_off(None, 0, 60);
        for _ in 0..128 {
            engine.render([Some(&sample), None], &params);
        }
        assert_eq!(engine.activity(), Activity::Inert);
        assert_eq!(
            engine.render([Some(&sample), None], &params),
            Stereo::default()
        );
        engine.note_on(Note::midi(64, 1.0), [Some(&sample), None], &params);
        engine.panic();
        assert_eq!(engine.activity(), Activity::Inert);
    }

    #[test]
    fn held_channel_bend_and_per_note_tuning_update_the_owned_voices() {
        let sample = tone(8_192);
        let params = Params::default();
        let mut engine = Engine::default();
        engine.note_on(
            Note {
                id: Some(7),
                channel: 2,
                key: 60,
                velocity: 1.0,
                tuning: 0.0,
            },
            [Some(&sample), None],
            &params,
        );
        engine.note_on(
            Note {
                id: Some(8),
                channel: 3,
                key: 64,
                velocity: 1.0,
                tuning: 0.25,
            },
            [Some(&sample), None],
            &params,
        );
        engine.add_channel_tuning(2, 1.5);
        assert_eq!(engine.voices[0].note.tuning, 1.5);
        assert_eq!(engine.voices[1].note.tuning, 0.25);
        assert!(engine.set_note_tuning(Some(7), 2, 60, -0.75));
        assert_eq!(engine.voices[0].note.tuning, -0.75);
        assert!(!engine.set_note_tuning(Some(99), 2, 60, 1.0));
    }

    /// The pitch a rendered note is actually reading at, measured from how far the Repitch head
    /// travels over a block. Reading the parameter back would prove nothing about the audio.
    fn measured_ratio(engine: &mut Engine, sample: &Sample, params: &Params, frames: usize) -> f32 {
        let head = |engine: &Engine| {
            engine
                .voices
                .iter()
                .find(|voice| voice.active)
                .expect("a sounding voice")
                .layers[0]
                .position
        };
        let before = head(engine);
        for _ in 0..frames {
            engine.render([Some(sample), None], params);
        }
        (head(engine) - before) / frames as f32
    }

    fn envelope_level(engine: &Engine) -> f32 {
        engine
            .voices
            .iter()
            .find(|voice| voice.active)
            .expect("a sounding voice")
            .envelope
            .level
    }

    #[test]
    fn mono_glides_from_the_sounding_pitch_and_legato_does_not_restart_the_envelope() {
        let sample = tone(48_000);
        for mode in [VoiceMode::Mono, VoiceMode::Legato] {
            let params = Params {
                voice_mode: mode,
                glide_s: 0.25,
                attack_s: 1.0,
                ..Params::default()
            };
            let mut engine = Engine::default();
            engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
            for _ in 0..4_800 {
                engine.render([Some(&sample), None], &params);
            }
            let before = envelope_level(&engine);
            engine.note_on(Note::midi(72, 1.0), [Some(&sample), None], &params);
            assert_eq!(
                engine.active_voices(),
                1,
                "{mode:?} used more than one voice"
            );
            let after = envelope_level(&engine);
            if mode == VoiceMode::Legato {
                assert_eq!(
                    after, before,
                    "Legato restarted the envelope instead of continuing it"
                );
            } else {
                assert_eq!(after, 0.0, "Mono did not restart the envelope");
            }

            // The first block after the press is still nearer the old pitch than the new one.
            let early = measured_ratio(&mut engine, &sample, &params, 64);
            assert!(
                early < 1.5,
                "{mode:?} jumped to the new pitch instead of gliding: {early}"
            );
            for _ in 0..24_000 {
                engine.render([Some(&sample), None], &params);
            }
            let arrived = measured_ratio(&mut engine, &sample, &params, 64);
            assert!(
                (arrived - 2.0).abs() < 0.02,
                "{mode:?} never arrived at the octave: {arrived}"
            );
        }
    }

    #[test]
    fn releasing_the_top_of_a_trill_falls_back_to_the_held_key_without_retriggering() {
        let sample = tone(48_000);
        let params = Params {
            voice_mode: VoiceMode::Mono,
            glide_s: 0.0,
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        engine.note_on(Note::midi(67, 1.0), [Some(&sample), None], &params);
        for _ in 0..2_048 {
            engine.render([Some(&sample), None], &params);
        }
        let held = measured_ratio(&mut engine, &sample, &params, 64);
        assert!((held - 2.0f32.powf(7.0 / 12.0)).abs() < 0.01, "{held}");

        engine.note_off(None, 0, 67);
        assert_eq!(
            engine.activity(),
            Activity::Live,
            "the held key was dropped"
        );
        let fallen = measured_ratio(&mut engine, &sample, &params, 64);
        assert!(
            (fallen - 1.0).abs() < 0.01,
            "did not fall back to C: {fallen}"
        );
        assert!(
            envelope_level(&engine) > 0.99,
            "the fall-back retriggered the envelope"
        );

        engine.note_off(None, 0, 60);
        assert_eq!(engine.activity(), Activity::Tailing);
    }

    #[test]
    fn a_seventeenth_press_drops_the_oldest_rather_than_growing_the_ledger() {
        let mut ledger = Ledger::default();
        for key in 0..(MAX_HELD as u8 + 1) {
            ledger.press(Press {
                note: Note::midi(key, 1.0),
                pressure: 0.0,
                random: 0.0,
                expression_gain: 1.0,
                expression_pan: 0.0,
            });
        }
        assert_eq!(ledger.len, MAX_HELD);
        assert_eq!(ledger.newest().expect("a press").note.key, MAX_HELD as u8);
        assert!(
            ledger.remove(None, 0, 0).is_none(),
            "the oldest press survived"
        );
        assert_eq!(ledger.len, MAX_HELD, "a release nobody held shortened it");
        assert!(ledger.remove(None, 0, 1).is_some());
        assert_eq!(ledger.len, MAX_HELD - 1);
    }

    #[test]
    fn pressure_and_the_shape_envelope_move_the_filter_and_expression_moves_the_output() {
        let sample = tone(8192);
        let base = Params {
            filter_mode: FilterMode::LowPass,
            cutoff_hz: 200.0,
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        let energy = |params: &Params, apply: &dyn Fn(&mut Engine)| {
            let mut engine = engine_without_routes();
            engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], params);
            apply(&mut engine);
            (0..4096)
                .map(|_| {
                    let frame = engine.render([Some(&sample), None], params);
                    frame.left * frame.left + frame.right * frame.right
                })
                .sum::<f32>()
        };
        // **The filter-envelope amount is a route now.** It was `filter_env`, a field; setting the
        // one isolated pair is the same gesture as turning that knob and does not depend on today's
        // Init topology.
        let with_env =
            |amount: f32| only(routing::target::CUTOFF, routing::source::ENVELOPE, amount);
        let closed = energy(&base, &|_| {});
        let opened = energy(&base, &|engine| {
            let r = with_env(1.0);
            engine.set_topology(&r);
            engine.set_amounts(&r);
        });
        assert!(
            opened > closed * 2.0,
            "the envelope did not open the filter: {closed} -> {opened}"
        );
        let inverted = energy(&base, &|engine| {
            let r = with_env(-1.0);
            engine.set_topology(&r);
            engine.set_amounts(&r);
        });
        assert!(
            inverted < closed,
            "a negative amount did not close the filter"
        );
        let pressed = energy(&base, &|engine| {
            let r = only(routing::target::CUTOFF, routing::source::PRESSURE, 1.0);
            engine.set_topology(&r);
            engine.set_amounts(&r);
            assert!(engine.set_pressure(None, 0, 60, 1.0));
        });
        assert!(
            pressed > closed * 2.0,
            "pressure did not open the filter: {closed} -> {pressed}"
        );

        // Expression addresses one press, and an unheld key silently changes nothing.
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &base);
        assert!(!engine.set_expression_gain(None, 0, 61, 0.0));
        assert!(engine.set_expression_gain(None, 0, 60, 0.0));
        for _ in 0..64 {
            assert_eq!(
                engine.render([Some(&sample), None], &base),
                Stereo::default()
            );
        }
        assert!(engine.set_expression_gain(None, 0, 60, 1.0));
        assert!(engine.set_expression_pan(None, 0, 60, 1.0));
        let sums = (0..256)
            .map(|_| engine.render([Some(&sample), None], &base))
            .fold(Stereo::default(), |sum, frame| Stereo {
                left: sum.left + frame.left.abs(),
                right: sum.right + frame.right.abs(),
            });
        assert!(
            sums.right > sums.left * 4.0,
            "pan expression did nothing: {sums:?}"
        );
    }

    #[test]
    fn expression_survives_release_and_panic_clears_the_ledger() {
        let sample = tone(8192);
        let params = Params {
            release_s: 1.0,
            ..Params::default()
        };
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        engine.note_off(None, 0, 60);
        assert!(
            engine.set_pressure(None, 0, 60, 0.5),
            "a releasing tail stopped answering its own expression"
        );
        engine.panic();
        assert!(!engine.set_pressure(None, 0, 60, 0.5));

        let mono = Params {
            voice_mode: VoiceMode::Mono,
            ..params
        };
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &mono);
        engine.note_on(Note::midi(64, 1.0), [Some(&sample), None], &mono);
        engine.panic();
        engine.note_off(None, 0, 64);
        assert_eq!(
            engine.activity(),
            Activity::Inert,
            "a release after panic revived a note from a stale ledger"
        );
    }

    #[test]
    fn a_mode_change_under_held_notes_leaves_one_sounding_voice() {
        let sample = tone(8192);
        let poly = Params {
            sustain: 1.0,
            attack_s: 0.001,
            ..Params::default()
        };
        let mono = Params {
            voice_mode: VoiceMode::Mono,
            ..poly
        };
        let mut engine = Engine::default();
        for key in [60, 64, 67] {
            engine.note_on(Note::midi(key, 1.0), [Some(&sample), None], &poly);
        }
        assert_eq!(engine.active_voices(), 3);
        engine.note_on(Note::midi(72, 1.0), [Some(&sample), None], &mono);
        let sounding = engine
            .voices
            .iter()
            .filter(|voice| voice.active && !voice.released)
            .count();
        assert_eq!(sounding, 1, "the poly chord kept sounding under Mono");
    }

    /// The RMS of a grain cloud held for `frames`, past the attack.
    /// How far a cloud departs from mono, as mean |L-R|.
    fn cloud_stereo_difference(sample: &Sample, position: f32, width: f32) -> f32 {
        let mut params = Params {
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_size_s: 0.06,
            grain_density_hz: 64.0,
            grain_position_variation: position,
            grain_stereo_width: width,
            ..LayerParams::default()
        };
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(sample), None], &params);
        for _ in 0..4_800 {
            engine.render([Some(sample), None], &params);
        }
        let frames = 48_000;
        let sum: f64 = (0..frames)
            .map(|_| {
                let f = engine.render([Some(sample), None], &params);
                f64::from((f.left - f.right).abs())
            })
            .sum();
        (sum / frames as f64) as f32
    }

    fn cloud_peak(sample: &Sample, size_s: f32, density: f32, scatter: f32, spread: f32) -> f32 {
        let mut params = Params {
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_size_s: size_s,
            grain_density_hz: density,
            grain_position_variation: scatter,
            grain_onset_timing: scatter,
            grain_spread_semitones: spread,
            ..LayerParams::default()
        };
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(sample), None], &params);
        let mut peak = 0.0f32;
        for _ in 0..52_800 {
            let f = engine.render([Some(sample), None], &params);
            peak = peak.max(f.left.abs()).max(f.right.abs());
        }
        peak
    }

    fn cloud_rms(sample: &Sample, size_s: f32, density: f32, scatter: f32, spread: f32) -> f32 {
        let mut params = Params {
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_size_s: size_s,
            grain_density_hz: density,
            grain_position_variation: scatter,
            grain_onset_timing: scatter,
            grain_spread_semitones: spread,
            ..LayerParams::default()
        };
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(sample), None], &params);
        for _ in 0..4_800 {
            engine.render([Some(sample), None], &params);
        }
        let frames = 48_000;
        let sum: f64 = (0..frames)
            .map(|_| {
                let frame = engine.render([Some(sample), None], &params);
                f64::from(frame.left) * f64::from(frame.left)
            })
            .sum();
        (sum / frames as f64).sqrt() as f32
    }

    /// **The cloud has to thicken, or density only changes texture and never weight.**
    ///
    /// Dividing by the live grain count made eight grains as loud as one, which is why a dropped
    /// violin note stayed a violin note however dense the setting. The overlap law leaves a
    /// coherent cloud growing as √overlap and a decorrelated one level; both are bounded by the
    /// pool. The numbers this produces are recorded in the crate's DOX.
    /// **Weight into the wall, then fusion — not a level that keeps climbing.**
    ///
    /// Replaces an assertion that measured the old single law against its pool-shaped ceiling.
    /// Density buys loudness while a cloud is still assembling itself out of separate events, and
    /// past that it buys *thickness* at a level that holds. A cloud whose level kept climbing with
    /// density would be the comb getting louder, which is the sound this work set out to remove.
    /// A harmonically rich tone at a known pitch, with an onset and leading silence in front of it
    /// — which is what a dropped sample actually looks like.
    fn pitched(hz: f32, partials: usize, silence_s: f32) -> Sample {
        let rate = 48_000.0f32;
        let lead = (rate * silence_s) as usize;
        let frames = lead + 48_000;
        Sample::new(
            (0..frames)
                .map(|n| {
                    if n < lead {
                        return [0.0, 0.0];
                    }
                    let t = (n - lead) as f32 / rate;
                    // A short attack, so the detector has to skip past an inharmonic onset.
                    let attack = (t / 0.01).min(1.0);
                    let value: f32 = (1..=partials)
                        .map(|k| (TAU * hz * k as f32 * t).sin() / k as f32)
                        .sum();
                    let value = value * 0.3 * attack;
                    [value, value]
                })
                .collect(),
            rate,
        )
        .unwrap()
    }

    /// **A player should not have to guess a sample's pitch by ear.**
    ///
    /// Every note here is checked to within a fifth of a semitone, which is tighter than the whole
    /// semitone Root snaps to — so a detector that is merely close still lands on the right key.
    #[test]
    fn the_detector_names_the_note_a_sample_is_playing() {
        for (hz, note, partials) in [
            (130.812_78_f32, 48.0_f32, 12), // C3, the generated init source
            (261.625_56, 60.0, 10),         // C4
            (440.0, 69.0, 8),               // A4
            (65.406_39, 36.0, 16),          // C2, where octave errors live
            (1_046.502_3, 84.0, 4),         // C6
        ] {
            let sample = pitched(hz, partials, 0.05);
            let found = detect_root(&sample, 0.0)
                .unwrap_or_else(|| panic!("{hz} Hz was not detected at all"));
            assert!(
                (found - note).abs() < 0.2,
                "{hz} Hz should be MIDI {note}, detected {found}"
            );
        }
    }

    /// **A pure sine is the case plain autocorrelation gets right and the rich tones it does not.**
    /// Both must work, because a sampler is given both.
    #[test]
    fn the_detector_handles_a_bare_sine_and_a_rich_tone_alike() {
        for partials in [1usize, 24] {
            let sample = pitched(220.0, partials, 0.0);
            let found = detect_root(&sample, 0.0).expect("220 Hz was not detected");
            assert!(
                (found - 57.0).abs() < 0.2,
                "220 Hz with {partials} partials detected as {found}"
            );
        }
    }

    /// **It reads from where playback starts**, not from frame zero — otherwise pointing Start at
    /// the second half of a two-note sample would still tune to the first.
    #[test]
    fn the_detector_reads_from_the_region_start() {
        let rate = 48_000.0f32;
        let half = 48_000usize;
        let two_notes = Sample::new(
            (0..half * 2)
                .map(|n| {
                    let hz = if n < half { 220.0f32 } else { 440.0 };
                    let t = n as f32 / rate;
                    let value = (TAU * hz * t).sin() * 0.4;
                    [value, value]
                })
                .collect(),
            rate,
        )
        .unwrap();
        let first = detect_root(&two_notes, 0.0).expect("the first note");
        let second = detect_root(&two_notes, 0.55).expect("the second note");
        assert!((first - 57.0).abs() < 0.2, "first half detected as {first}");
        assert!(
            (second - 69.0).abs() < 0.2,
            "second half detected as {second}"
        );
    }

    /// **Silence and noise are told apart from a note**, because a wrong Root is worse than none:
    /// it retunes the whole keyboard on a guess the player did not make.
    #[test]
    fn the_detector_declines_what_has_no_pitch() {
        let silent = Sample::new(vec![[0.0, 0.0]; 48_000], 48_000.0).unwrap();
        assert!(detect_root(&silent, 0.0).is_none(), "silence got a pitch");

        let mut rng = 0x1234_5678_9abc_def0u64;
        let noise = Sample::new(
            (0..48_000)
                .map(|_| {
                    let value = split_mix(&mut rng) * 2.0 - 1.0;
                    [value * 0.4, value * 0.4]
                })
                .collect(),
            48_000.0,
        )
        .unwrap();
        assert!(detect_root(&noise, 0.0).is_none(), "noise got a pitch");

        let tiny = Sample::new(vec![[0.5, 0.5]; 64], 48_000.0).unwrap();
        assert!(
            detect_root(&tiny, 0.0).is_none(),
            "a 64-frame sample got a pitch"
        );
    }

    /// **A sample shorter than one grain still has to sound.**
    ///
    /// The exact case two `mxm-player` tests fail on: the restored patch is the parameter defaults
    /// — Grain, root C3, size 0.120 s, 40 grains a second — and the sample is an 8,192-frame sine.
    /// Played at C4 that is read at twice speed, so one grain wants 11,520 source frames out of a
    /// region that holds 8,192.
    #[test]
    fn a_sample_shorter_than_one_grain_still_sounds() {
        let rate = 48_000.0f32;
        let frames: Vec<[f32; 2]> = (0..8_192)
            .map(|n| {
                let v = (TAU * 220.0 * n as f32 / rate).sin() * 0.73;
                [v, v]
            })
            .collect();
        let sample = Sample::new(frames, rate).expect("the test source");

        // The exact patch the plugin hands the engine on that restore, printed from inside it.
        let mut params = Params {
            attack_s: 0.22,
            decay_s: 0.6,
            sustain: 0.88,
            release_s: 0.8,
            master: 0.8,
            filter_mode: FilterMode::LowPass,
            cutoff_hz: 18_000.0,
            resonance: 0.1,
            velocity_to_level: 0.35,
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            root_note: 48.0,
            tune_semitones: 0.0,
            start: 0.0,
            end: 1.0,
            loop_start: 0.0,
            loop_end: 1.0,
            loop_mode: LoopMode::Forward,
            loop_crossfade_s: 0.0,
            reverse: false,
            level: 1.0,
            pan: 0.0,
            stretch_travel: 1.0,
            grain_size_s: 0.12,
            grain_density_hz: 40.0,
            grain_position_variation: 0.65,
            grain_onset_timing: 0.65,
            grain_spread_semitones: 0.25,
            grain_stereo_width: 0.2,
            grain_reverse_chance: 0.0,
            grain_shape: 1.0,
        };

        let mut engine = Engine::new(rate);
        // Idle first, as a host does between a state load and the first key.
        for _ in 0..2_048 {
            engine.render([Some(&sample), None], &params);
        }
        engine.note_on(Note::midi(60, 0.8), [Some(&sample), None], &params);

        // **A grain on the very first sample.** Not "eventually": the scheduler is primed at the
        // start of a voice, so a key that is pressed is answered by a read immediately and the
        // note's onset latency is the envelope's, not a random draw.
        let mut views = [GrainView::default(); 8];
        engine.render([Some(&sample), None], &params);
        assert!(
            engine.grain_views(0, params.layers[0].grain_shape, &mut views) > 0,
            "no grain started on the voice's first sample"
        );

        // And audible well inside the eight blocks `mxm-player`'s tests listen for.
        let peak = (0..8 * 512)
            .map(|_| engine.render([Some(&sample), None], &params).left.abs())
            .fold(0.0f32, f32::max);
        assert!(
            peak > 0.01,
            "a sample shorter than one grain rendered {peak} in eight blocks"
        );
    }

    /// **Root follows the region start, which is the control the owner asked for.**
    ///
    /// *"The sound closest to the start position — so I can move it and get a new pitch if I don't
    /// want the start of the sample to be the pitch."* A two-note phrase is the whole test: reading
    /// from the top hears the first note, and reading from past the change hears the second. A
    /// detector that surveyed the whole file and took its most common note would pass neither half.
    #[test]
    fn root_follows_the_region_start() {
        let rate = 48_000.0f32;
        let hold = 48_000; // A second each.
        let tone = |note: f32, n: usize| {
            let hz = 440.0 * ((note - 69.0) / 12.0).exp2();
            // Three harmonics, so this is a pitch rather than a textbook sine.
            let phase = TAU * hz * n as f32 / rate;
            (phase.sin() + 0.5 * (2.0 * phase).sin() + 0.25 * (3.0 * phase).sin()) * 0.3
        };
        let frames: Vec<[f32; 2]> = (0..hold * 2)
            .map(|n| {
                let value = if n < hold {
                    tone(52.0, n)
                } else {
                    tone(59.0, n - hold)
                };
                [value, value]
            })
            .collect();
        let sample = Sample::new(frames, rate).expect("a two-note phrase");

        let from_top = detect_root(&sample, 0.0).expect("the first note");
        assert!(
            (from_top - 52.0).abs() < 0.5,
            "reading from the top heard {from_top:.2}, not the first note"
        );

        // Past the change, and clear of the window that would still straddle it.
        let from_later = detect_root(&sample, 0.6).expect("the second note");
        assert!(
            (from_later - 59.0).abs() < 0.5,
            "reading from 60% heard {from_later:.2}, not the note that starts there"
        );
    }

    /// The search stays near the start rather than wandering into the next musical event.
    #[test]
    fn a_silent_start_does_not_borrow_a_note_from_far_away() {
        let rate = 48_000.0f32;
        // Four seconds of near-silence, then a note — further off than the search reaches.
        let frames: Vec<[f32; 2]> = (0..(rate as usize * 6))
            .map(|n| {
                if n < rate as usize * 4 {
                    [0.0, 0.0]
                } else {
                    let phase = TAU * 220.0 * (n - rate as usize * 4) as f32 / rate;
                    let value = phase.sin() * 0.4;
                    [value, value]
                }
            })
            .collect();
        let sample = Sample::new(frames, rate).expect("a late note");
        // The onset skip finds the note itself, so this is still answered — what must not happen is
        // a *pitched* start reporting something seconds later, which `PITCH_SEARCH_S` bounds.
        let found = detect_root(&sample, 0.0).expect("the note it eventually reaches");
        assert!(
            (found - 57.0).abs() < 0.5,
            "silence then A3 heard {found:.2}"
        );
    }

    /// The editor's grain overlay is only as true as this: if the engine will not report its own
    /// grains, the picture is empty however well it draws.
    #[test]
    fn the_engine_reports_the_grains_it_is_sounding() {
        let sample = tone(48_000);
        let mut params = Params::default();
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_density_hz: 40.0,
            grain_size_s: 0.12,
            grain_position_variation: 0.65,
            ..params.layers[0]
        };
        let mut engine = Engine::new(48_000.0);
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        for _ in 0..8_000 {
            engine.render([Some(&sample), None], &params);
        }

        let mut views = [GrainView::default(); 64];
        let found = engine.grain_views(0, params.layers[0].grain_shape, &mut views);
        assert!(found > 0, "a sounding cloud reported no grains");
        let length = (sample.len() - 1) as f32;
        for view in &views[..found] {
            assert!(
                (0.0..=length).contains(&view.start),
                "a grain started outside the sample: {}",
                view.start
            );
            assert!(
                view.weight >= 0.0 && view.weight <= 1.0,
                "weight out of range"
            );
            // The read has to have moved, or the overlay draws a point and the owner sees nothing
            // travelling.
            assert!(view.current.is_finite());
        }
        assert!(
            views[..found].iter().any(|v| v.weight > 0.05),
            "every reported grain was silent, so the overlay would draw nothing"
        );
        assert!(
            views[..found]
                .iter()
                .any(|v| (v.current - v.start).abs() > 1.0),
            "no grain had advanced, so the overlay would draw only points"
        );
    }

    #[test]
    fn a_cloud_gains_weight_into_the_wall_and_then_holds_it() {
        let sample = tone(48_000);
        for scatter in [0.0f32, 0.5, 1.0] {
            let sparse = cloud_rms(&sample, 0.06, 16.0, scatter, 0.0);
            let wall = cloud_rms(&sample, 0.06, 64.0, scatter, 0.0);
            assert!(
                wall > sparse * 1.25,
                "density bought no weight at scatter {scatter}: {sparse} -> {wall}"
            );

            // Past the wall the law holds the level while the texture keeps thickening.
            let thicker = cloud_rms(&sample, 0.25, 200.0, scatter, 0.0);
            let ratio = thicker / wall.max(1.0e-9);
            assert!(
                (0.6..1.4).contains(&ratio),
                "level ran away past the wall at scatter {scatter}: scaled by {ratio}"
            );
        }
    }

    /// The musical ceiling is safe at full travel, by measurement rather than by argument.
    ///
    /// [`MAX_OVERLAP`] is 128 because a voice's largest share is, and nothing normalises peaks
    /// afterwards — there is no limiter in this instrument, only a clip indicator. So the laws
    /// themselves have to land under full scale at every setting the controls can reach.
    #[test]
    fn the_overlap_ceiling_stays_under_full_scale_everywhere() {
        let sample = crested(48_000);
        for scatter in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            for (size, density) in [
                (0.008f32, 16.0f32),
                (0.06, 16.0),
                (0.06, 64.0),
                (0.12, 128.0),
                (0.25, 200.0),
                (1.0, 200.0),
            ] {
                let peak = cloud_peak(&sample, size, density, scatter, 0.0);
                assert!(
                    peak.is_finite() && peak < 1.0,
                    "scatter {scatter}, size {size}, density {density} peaked at {peak}"
                );
            }
        }
    }

    /// **A jitter cannot make a cloud; a hand-over can.**
    ///
    /// However wide an onset wanders inside its slot there is still exactly one onset per slot, so
    /// the grain rate stays audible as a pitch. The asynchronous branch fires from a per-sample
    /// draw, so slots hold two onsets or none — the statistic the old `base * (1 + jitter)` could
    /// not produce at any setting, and the reason a scattered cloud still rang.
    #[test]
    fn full_scatter_reaches_a_stochastic_scheduler_a_jitter_cannot() {
        let slot = 480.0f32;
        let slots = 400usize;

        let run = |scatter: f32| {
            let mut scheduler = GrainScheduler::default();
            let mut rng = 0x1234_5678_9abc_def0u64;
            let mut counts = Vec::new();
            for _ in 0..slots {
                let mut in_slot = 0;
                for _ in 0..slot as usize {
                    in_slot += scheduler.tick(slot, Scatter::from_control(scatter), &mut rng);
                }
                counts.push(in_slot);
            }
            counts
        };

        // Periodic at zero: one onset per slot, every slot. That is the synchronous family, and it
        // is a sound rather than a defect — but it must be reachable only where it is asked for.
        let counts = run(0.0);
        assert!(
            counts.iter().all(|&c| c == 1),
            "a synchronous scheduler must fire exactly once per slot, got {counts:?}"
        );

        // Stochastic at one: some slots empty, some doubled.
        let counts = run(1.0);
        let empty = counts.iter().filter(|&&c| c == 0).count();
        let doubled = counts.iter().filter(|&&c| c >= 2).count();
        assert!(
            empty > 0 && doubled > 0,
            "a stochastic scheduler must leave slots empty and double others: \
             {empty} empty, {doubled} doubled"
        );

        // The mean rate does not move across the hand-over: only the statistics do.
        let total: u32 = counts.iter().sum();
        let mean = f64::from(total) / slots as f64;
        assert!(
            (0.75..1.25).contains(&mean),
            "the hand-over moved the density: {mean} onsets per slot"
        );
    }

    /// **Decorrelating a cloud must not also throw it about.**
    ///
    /// The position draw is what makes many grains sound like many sources; stereo width is a sound
    /// you choose. With width shut, a fully decorrelated cloud stays centred — which is the whole
    /// point of them being two controls.
    #[test]
    fn the_position_draw_decorrelates_without_moving_the_cloud_off_centre() {
        let sample = tone(48_000);
        let centred = cloud_stereo_difference(&sample, 1.0, 0.0);
        let thrown = cloud_stereo_difference(&sample, 1.0, 1.0);
        assert!(
            centred < 1.0e-6,
            "the position draw alone moved the cloud off centre by {centred}"
        );
        assert!(
            thrown > 1.0e-4,
            "stereo width did not spread the cloud: {centred} -> {thrown}"
        );
    }

    /// **The budget is shared, so fewer notes get more grains each** — and the total never exceeds
    /// it. The old pool provisioned every note for the eight-note worst case and then charged that
    /// provisioning to every single note.
    #[test]
    fn the_grain_budget_is_shared_and_never_exceeded() {
        let sample = tone(48_000);
        let mut params = Params {
            attack_s: 0.001,
            sustain: 1.0,
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_size_s: 1.0,
            grain_density_hz: 200.0,
            grain_position_variation: 0.5,
            grain_onset_timing: 0.5,
            ..LayerParams::default()
        };

        let occupancy = |notes: u8| {
            let mut engine = Engine::default();
            for key in 60..60 + notes {
                engine.note_on(Note::midi(key, 1.0), [Some(&sample), None], &params);
            }
            let mut peak = 0usize;
            for _ in 0..96_000 {
                engine.render([Some(&sample), None], &params);
                peak = peak.max(engine.active_grains());
            }
            peak
        };

        let one = occupancy(1);
        let eight = occupancy(8);
        assert!(
            one > 64,
            "a single note should draw deep on the shared budget, got {one} grains"
        );
        assert!(
            one <= GRAIN_BUDGET && eight <= GRAIN_BUDGET,
            "the shared budget was exceeded: {one} on one note, {eight} on eight"
        );
        // Eight notes split the same budget, so each holds far fewer than one note alone does.
        assert!(
            eight / 8 < one,
            "sharing did not favour the sparser texture: {one} on one note, {eight} across eight"
        );
    }

    #[test]
    fn spread_and_scatter_are_independent() {
        let sample = tone(48_000);
        let plain = cloud_rms(&sample, 0.06, 32.0, 0.0, 0.0);
        let detuned = cloud_rms(&sample, 0.06, 32.0, 0.0, 3.0);
        assert!(
            (detuned - plain).abs() > plain * 0.02,
            "Spread changed nothing at zero Scatter: {plain} -> {detuned}"
        );

        // And Scatter still works with Spread at zero, so neither depends on the other.
        let scattered = cloud_rms(&sample, 0.06, 32.0, 1.0, 0.0);
        assert!(
            (scattered - plain).abs() > plain * 0.02,
            "Scatter changed nothing at zero Spread: {plain} -> {scattered}"
        );
        assert!(detuned.is_finite() && scattered.is_finite());
    }

    /// A source whose every frame names its own index, so a reader that visits the wrong frame is
    /// caught by value rather than by a statistic.
    fn ruler(frames: usize) -> Sample {
        Sample::new(
            (0..frames).map(|n| [n as f32, -(n as f32)]).collect(),
            48_000.0,
        )
        .expect("a valid sample")
    }

    /// The frames a naive reader would visit: one `frame` call per tap, the way `sinc_read` did
    /// before the walk replaced it.
    fn naive_walk(sample: &Sample, position: f32, region: Region, loop_mode: LoopMode) -> Vec<f32> {
        let centre = position.floor() as i64;
        let first = -(SINC_TAPS / 2 - 1);
        (first..=SINC_TAPS / 2)
            .map(|tap| frame(sample, centre + i64::from(tap), region, loop_mode).left)
            .collect()
    }

    /// **The optimisation's own regression.** Resolving the loop wrap once and walking it is only
    /// sound because consecutive taps are consecutive frames; a bug there reads the wrong audio at
    /// a loop boundary, which is exactly where nobody looks. This holds the walked form against the
    /// naive one over regions, loop modes and positions that straddle every boundary.
    #[test]
    fn sinc_read_walks_the_same_frames_as_a_naive_reader() {
        let sample = ruler(512);
        let layers = [
            LayerParams::default(),
            LayerParams {
                start: 0.2,
                end: 0.6,
                loop_start: 0.25,
                loop_end: 0.35,
                ..LayerParams::default()
            },
            // A loop of a single frame, and a region of nearly nothing.
            LayerParams {
                start: 0.5,
                end: 0.5,
                loop_start: 0.5,
                loop_end: 0.5,
                ..LayerParams::default()
            },
            LayerParams {
                start: 0.0,
                end: 1.0,
                loop_start: 0.99,
                loop_end: 1.0,
                ..LayerParams::default()
            },
        ];
        for params in layers {
            let region = Region::new(sample.len(), &params);
            for loop_mode in [LoopMode::Off, LoopMode::Forward, LoopMode::Alternate] {
                // Positions before, inside, at both edges of and past the region and its loop.
                for step in -4..=520 {
                    let position = step as f32 + 0.37;
                    let expected = naive_walk(&sample, position, region, loop_mode);
                    let mut walk = Walk::new(
                        &sample,
                        position.floor() as i64 - i64::from(SINC_TAPS / 2 - 1),
                        region,
                        loop_mode,
                    );
                    for (tap, wanted) in expected.iter().enumerate() {
                        let got = sample.frames[walk.index()][0];
                        assert_eq!(
                            got, *wanted,
                            "tap {tap} at {position} in {loop_mode:?} read frame {got} not {wanted}"
                        );
                        walk.advance();
                    }
                }
            }
        }
    }

    /// The sixteen-tap kernel as it was computed before the table: coefficients built per read,
    /// each sinc tap costing its own division, and the whole row normalised at the end.
    fn computed_kernel(
        sample: &Sample,
        position: f32,
        region: Region,
        loop_mode: LoopMode,
    ) -> Stereo {
        let centre = position.floor() as i64;
        let cutoff = 1.0f32;
        let first_tap = -(SINC_TAPS / 2 - 1);
        let mut distance = position - (centre + i64::from(first_tap)) as f32;
        let half_width = SINC_TAPS as f32 / 2.0;
        let mut output = Stereo::default();
        let mut sum = 0.0f32;
        let mut walk = Walk::new(sample, centre + i64::from(first_tap), region, loop_mode);
        for _ in 0..SINC_TAPS_N {
            let sinc = if distance.abs() < 1.0e-6 {
                cutoff
            } else {
                (PI * cutoff * distance).sin() / (PI * distance)
            };
            let cosine = (PI * distance / half_width).cos();
            let window = 0.42 + 0.5 * cosine + 0.08 * (2.0 * cosine * cosine - 1.0);
            let weight = sinc * window;
            let value = sample.frames[walk.index()];
            output.left += value[0] * weight;
            output.right += value[1] * weight;
            sum += weight;
            walk.advance();
            distance -= 1.0;
        }
        output.gain(1.0 / sum)
    }

    /// **One converter per voice, proved on the sum rather than asserted.**
    ///
    /// With a coarse converter and nothing after it but unity gain, everything the voice sounds
    /// must land on the converter's own grid. Under the previous design — a converter inside every
    /// grain read — sixteen grains were each rounded to the grid and then multiplied by their own
    /// window and pan and summed, so the voice's output lay nowhere near it. This is the test that
    /// tells the two designs apart, and it is the whole audible content of the change.
    #[test]
    fn a_voice_playing_many_grains_lands_on_one_converters_grid() {
        let sample = tone(48_000);
        let mut params = Params {
            attack_s: 0.001,
            decay_s: 0.001,
            sustain: 1.0,
            // Nothing between the converter and the output may move a sample off the grid.
            filter_mode: FilterMode::Off,
            velocity_to_level: 0.0,
            master: 1.0,
            character: Character {
                rate: 0.0,
                converter: 1.0,
                reconstruction: 0.0,
                input: 0.0,
                ..Character::default()
            },
            ..Params::default()
        };
        params.layers[0] = LayerParams {
            reader: Reader::Grain,
            loop_mode: LoopMode::Forward,
            grain_size_s: 0.25,
            grain_density_hz: 80.0,
            grain_position_variation: 1.0,
            grain_onset_timing: 1.0,
            ..LayerParams::default()
        };
        let step = 1.0 / f32::from(1u16 << 4); // five bits, so sixteen levels either side of zero
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        // Past the attack, and long enough for the pool to fill.
        for _ in 0..24_000 {
            engine.render([Some(&sample), None], &params);
        }
        let mut checked = 0usize;
        let mut worst = 0.0f32;
        for _ in 0..4_000 {
            let frame = engine.render([Some(&sample), None], &params);
            for value in [frame.left, frame.right] {
                let cells = value / step;
                worst = worst.max((cells - cells.round()).abs());
                checked += 1;
            }
        }
        assert!(checked > 0);
        assert!(
            worst < 1.0e-3,
            "a voice's output sat {worst} of a cell off the converter's grid, so it was not \
             converted once at the end"
        );
    }

    /// The shared window table against the cosine every grain used to evaluate for itself, bounded
    /// by the same converter step the kernel is held to.
    #[test]
    fn the_tabled_window_matches_the_cosine_it_replaced() {
        let step = 1.0 / 32_768.0;
        let mut worst = 0.0f32;
        for index in 0..100_001 {
            let phase = index as f32 / 100_000.0;
            let exact = 0.5 - 0.5 * (TAU * phase).cos();
            worst = worst.max((hann(phase) - exact).abs());
        }
        assert!(
            worst < step,
            "the window table deviates by {worst}, past the {step} converter step"
        );
        // The ends are exact, and nothing outside the range escapes.
        assert_eq!(hann(0.0), 0.0);
        assert_eq!(hann(1.0), 0.0);
        assert!(hann(-1.0).is_finite() && hann(2.0).is_finite());
        assert!(hann(f32::NAN).is_finite());
    }

    /// **The table has to be the kernel, not a kernel.** A polyphase table is not bit-identical to
    /// coefficients computed per read, so the difference is bounded here against something real:
    /// the 16-bit step the converter applies to every sample anyway, `1/32768` ≈ 3.05e-5. Anything
    /// under that cannot survive the DAC that follows it.
    #[test]
    fn the_tabled_unity_kernel_matches_the_kernel_it_replaced() {
        let sample = tone(4_096);
        let params = LayerParams {
            loop_mode: LoopMode::Forward,
            ..LayerParams::default()
        };
        let region = Region::new(sample.len(), &params);
        let step = 1.0 / 32_768.0;
        let mut worst = 0.0f32;
        for index in 0..20_000 {
            // Positions spread over the region at an irrational stride, so the fractional part
            // lands between table rows as often as on them.
            let position = 40.0 + index as f32 * 0.937_954_1;
            let tabled = sinc_read(&sample, position, region, LoopMode::Forward, 1.0);
            let computed = computed_kernel(&sample, position, region, LoopMode::Forward);
            worst = worst
                .max((tabled.left - computed.left).abs())
                .max((tabled.right - computed.right).abs());
        }
        assert!(
            worst < step,
            "the tabled kernel deviates by {worst}, past the {step} converter step"
        );
    }

    /// Above unity rate the kernel narrows with pitch, and **that cannot be tabled by one row set**.
    /// This holds the boundary: the fast path is entered only where the cutoff is open.
    #[test]
    fn a_read_above_unity_rate_keeps_the_computed_kernel() {
        // Content at Nyquist, which is the only thing a band limit is *for*. On a 220 Hz tone the
        // two kernels agree to a millionth and the assertion would prove nothing.
        let sample = Sample::new(
            (0..4_096)
                .map(|n| {
                    let value = if n % 2 == 0 { 0.5 } else { -0.5 };
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .expect("a valid sample");
        let params = LayerParams {
            loop_mode: LoopMode::Forward,
            ..LayerParams::default()
        };
        let region = Region::new(sample.len(), &params);
        let narrow = sinc_read(&sample, 100.37, region, LoopMode::Forward, 0.5);
        let open = sinc_read(&sample, 100.37, region, LoopMode::Forward, 1.0);
        assert!(
            (narrow.left - open.left).abs() > 0.05,
            "a narrowed band limit read the unity kernel: {narrow:?} vs {open:?}"
        );
        assert!(
            narrow.left.abs() < open.left.abs(),
            "narrowing the kernel must attenuate content at Nyquist"
        );
    }

    /// The quantizer's level count is an exact power of two at every depth it can take, so the
    /// shift and the `powf` it replaced are the same number and not merely a close one.
    #[test]
    fn the_quantizer_shift_is_exactly_the_power_it_replaced() {
        for bits in 5..=16 {
            let bits = bits as f32;
            assert_eq!(
                quantizer_levels(bits),
                2.0f32.powf(bits - 1.0),
                "{bits} bits"
            );
        }
        // Character sweeps the depth continuously; every position it can reach is covered.
        for step in 0..=1000 {
            let converter = step as f32 / 1000.0;
            let bits = (16.0 - converter * 11.0).round().clamp(5.0, 16.0);
            assert_eq!(
                quantizer_levels(bits),
                2.0f32.powf(bits - 1.0),
                "{converter}"
            );
        }
    }

    /// The latched grain pan and the general helper are the same law.
    #[test]
    fn latched_pan_gains_match_the_general_pan() {
        for step in -20..=20 {
            let position = step as f32 / 20.0;
            let (left, right) = pan_gains(position);
            let applied = pan(
                Stereo {
                    left: 1.0,
                    right: 1.0,
                },
                position,
            );
            assert_eq!((left, right), (applied.left, applied.right), "{position}");
        }
    }

    #[test]
    fn malformed_samples_are_rejected() {
        assert_eq!(Sample::new(Vec::new(), 48_000.0), Err(SampleError::Empty));
        assert_eq!(
            Sample::new(vec![[f32::NAN, 0.0]], 48_000.0),
            Err(SampleError::NonFinite)
        );
    }

    // ---------------------------------------------------------------------------------------
    // S1: the waveform-aligned splice. plans/plan-stretch-quality.md
    // ---------------------------------------------------------------------------------------

    /// A source with unmistakable attacks: a click train over a quiet steady tone, so the detector
    /// has to find the attacks *and* not report the tone.
    fn struck(rate: f32, onsets: &[usize], len: usize) -> Sample {
        struck_over(rate, onsets, len, 0.05)
    }

    /// [`struck`] over a tone of the given level.
    fn struck_over(rate: f32, onsets: &[usize], len: usize, level: f32) -> Sample {
        let mut frames = vec![[0.0f32; 2]; len];
        for (n, frame) in frames.iter_mut().enumerate() {
            let tone = (TAU * 220.0 * n as f32 / rate).sin() * level;
            frame[0] = tone;
            frame[1] = tone;
        }
        for &onset in onsets {
            for n in 0..(rate * 0.05) as usize {
                let index = onset + n;
                if index >= len {
                    break;
                }
                let decay = (-(n as f32) / (rate * 0.008)).exp();
                let strike = (TAU * 1_400.0 * n as f32 / rate).sin() * decay * 0.8;
                frames[index][0] += strike;
                frames[index][1] += strike;
            }
        }
        Sample::new(frames, rate).expect("a struck source")
    }

    fn stretch_params(reader: Reader) -> Params {
        let mut params = Params::default();
        params.layers[0].reader = reader;
        params.layers[0].root_note = 60.0;
        params.layers[0].tune_semitones = 0.0;
        params.layers[0].stretch_travel = 1.0;
        params.layers[0].loop_mode = LoopMode::Off;
        params
    }

    fn render_layer_a(sample: &Sample, params: &Params, key: u8, frames: usize) -> Vec<Stereo> {
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(key, 1.0), [Some(sample), None], params);
        (0..frames)
            .map(|_| engine.render([Some(sample), None], params))
            .collect()
    }

    /// **The invariant the whole design turns on.** At root pitch, unity scan and forward
    /// direction Stretch *is* the plain resampled read, so it must produce exactly what Repitch
    /// produces through the same envelope, filter and master.
    ///
    /// This failed before S1 for a reason that had nothing to do with transposition: the second
    /// head reconstructed its read position from an age that started at half a window, so it read
    /// half a window ahead of the playhead until its first re-anchor. Carrying the position
    /// directly fixed it, and the alignment search's tie-break toward zero is what keeps it true
    /// once the search exists.
    ///
    /// **Why the tolerance is a converter step rather than zero, measured rather than assumed.**
    /// The two readers agree bit-for-bit at all but one sample in 24 000, and there they differ by
    /// **one ULP** — the floor of a two-term weighted sum, `(w₀·r + w₁·r)/(w₀+w₁)` against `r`.
    /// Character's Clean setting is a deliberate 16-bit floor, so a one-ULP difference that
    /// straddles a quantisation boundary comes out as exactly one 16-bit step: 1/32768 through an
    /// 0.82 envelope and an 0.8 master is 2.002e-5, which is the difference this test measured to
    /// every digit it prints. The tolerance is set just above that and is still four orders of
    /// magnitude under the defect it exists to catch.
    #[test]
    fn stretch_is_the_plain_read_at_root_pitch_and_unity_scan() {
        let rate = 48_000.0;
        let sample = struck(rate, &[6_000, 20_000], 48_000);
        let repitch = render_layer_a(&sample, &stretch_params(Reader::Repitch), 60, 24_000);
        let stretched = render_layer_a(&sample, &stretch_params(Reader::Stretch), 60, 24_000);
        let mut worst = 0.0f32;
        let mut differing = 0usize;
        for (a, b) in repitch.iter().zip(stretched.iter()) {
            let difference = (a.left - b.left).abs().max((a.right - b.right).abs());
            worst = worst.max(difference);
            if difference > 0.0 {
                differing += 1;
            }
        }
        assert!(
            worst < 1.0e-4,
            "Stretch is not an identity at unity: worst sample differs by {worst}"
        );
        assert!(
            differing < repitch.len() / 100,
            "{differing} of {} samples differ, which is a systematic offset rather than rounding",
            repitch.len()
        );
        assert!(
            repitch.iter().any(|v| v.left.abs() > 0.05),
            "the comparison rendered silence and would have passed on nothing"
        );
    }

    /// The same invariant is not an accident of one loop mode.
    #[test]
    fn the_identity_at_unity_survives_a_looping_region() {
        let rate = 48_000.0;
        let sample = struck(rate, &[4_000], 24_000);
        let mut repitch_params = stretch_params(Reader::Repitch);
        repitch_params.layers[0].loop_mode = LoopMode::Forward;
        let mut stretched_params = stretch_params(Reader::Stretch);
        stretched_params.layers[0].loop_mode = LoopMode::Forward;
        let repitch = render_layer_a(&sample, &repitch_params, 60, 48_000);
        let stretched = render_layer_a(&sample, &stretched_params, 60, 48_000);
        let worst = repitch
            .iter()
            .zip(stretched.iter())
            .map(|(a, b)| (a.left - b.left).abs().max((a.right - b.right).abs()))
            .fold(0.0f32, f32::max);
        assert!(worst < 1.0e-4, "looping broke the identity: {worst}");
    }

    // ================= The loop crossfade =================
    //
    // `plans/plan-sampler-loop-crossfade.md` §5. The seam is a property of the frames read, so these
    // read it through every reader and direction that can meet it, and through the geometry the
    // editor draws.

    /// A half-scale tone `frames` long, looped whole by the tests below. **Its length decides whether
    /// the loop closes.** A whole number of cycles closes; a quarter of a cycle more leaves the frames
    /// either side of the seam a quarter-cycle apart, a step of the tone's full amplitude — where half
    /// a cycle would meet at a zero crossing and step by almost nothing.
    fn looped_tone(frames: usize, hz: f32) -> Sample {
        Sample::new(
            (0..frames)
                .map(|n| {
                    let value = (TAU * hz * n as f32 / 48_000.0).sin() * 0.5;
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .unwrap()
    }

    /// A forward loop on layer A with a fast, flat envelope, so a step in the output is a step in
    /// what was read.
    fn looping(reader: Reader, crossfade_s: f32) -> Params {
        let mut params = stretch_params(reader);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[0].loop_crossfade_s = crossfade_s;
        params.attack_s = 0.001;
        params.sustain = 1.0;
        params
    }

    /// The largest sample-to-sample step on the left channel.
    fn largest_step(render: &[Stereo]) -> f32 {
        render
            .windows(2)
            .map(|pair| (pair[1].left - pair[0].left).abs())
            .fold(0.0, f32::max)
    }

    /// **The whole claim**: a seam that does not close steps, and the crossfade takes the step away —
    /// on a whole-region loop, which is what an import leaves and which has no room outside the loop
    /// at all, so this is also the test that fails if the paired frame is fetched through the wrap.
    /// Every reader and direction that meets the seam, and a bass note, because every figure in this
    /// crate was once taken at 220 Hz and above.
    ///
    /// **The metric carries its own control: the same reader on a loop that closes.** Its largest
    /// step is the tone's own, plus whatever the reader does at a seam that needs no fade; the hard
    /// render must step well past it, or the source was not a bad seam and the assertion on the faded
    /// one would prove nothing. An early stretch of the hard render is not a control — with negative
    /// Scan the playhead wraps on its first sample and Stretch's heads cross the seam from the start.
    ///
    /// Lengths: 12 273 frames is 56.25 cycles of 220 Hz and 12 436 is 14.25 of 55 Hz; 12 000 and
    /// 9 600 are whole numbers of each.
    #[test]
    fn a_crossfade_removes_the_step_at_a_seam_that_does_not_close() {
        for (frames, closing, hz, fade_s) in [
            (12_273usize, 12_000usize, 220.0f32, 0.05f32),
            (12_436, 9_600, 55.0, 0.10),
        ] {
            let sample = looped_tone(frames, hz);
            let closed = looped_tone(closing, hz);
            for (name, reader, reverse, scan) in [
                ("Repitch", Reader::Repitch, false, 1.0f32),
                ("reversed Repitch", Reader::Repitch, true, 1.0),
                ("Stretch", Reader::Stretch, false, 1.0),
                ("Stretch with negative Scan", Reader::Stretch, false, -1.0),
            ] {
                let params = |fade: f32| {
                    let mut params = looping(reader, fade);
                    params.layers[0].reverse = reverse;
                    params.layers[0].stretch_travel = scan;
                    params
                };
                let hard = render_layer_a(&sample, &params(0.0), 60, 60_000);
                let faded = render_layer_a(&sample, &params(fade_s), 60, 60_000);
                let interior =
                    largest_step(&render_layer_a(&closed, &params(0.0), 60, 60_000)[5_000..]);
                let hard_step = largest_step(&hard[5_000..]);
                let faded_step = largest_step(&faded[5_000..]);
                assert!(
                    hard_step > 3.0 * interior,
                    "{name} at {hz} Hz: the hard seam does not step ({hard_step} against {interior}), \
                     so this measures nothing"
                );
                assert!(
                    faded_step < 1.3 * interior,
                    "{name} at {hz} Hz: the crossfaded seam still steps by {faded_step} against the \
                     tone's own {interior}"
                );
            }
        }
    }

    /// **No regime switch at zero.** As the fade shortens the render must approach the hard wrap's,
    /// with no jump as it leaves zero — through the kernel, and through Character's hold path, which
    /// fetches the same frames on a grid of its own. This is the test that fails if the blend moves
    /// above the interpolator, where a positive fade would change what every tap across the seam reads.
    #[test]
    fn a_crossfade_converges_to_the_hard_wrap_as_it_shortens() {
        // **A steep tone, so a hundredth of a frame shows.** The hard wrap is the seam at zero, so a
        // fade a hundredth of a frame long moves each wrapped tap by a hundredth of a frame and no more.
        // On the 220 Hz tone this test first used, that change fell under Character's 16-bit floor and
        // read as exactly zero, which the rungs below would take for a skipped seam.
        let sample = looped_tone(12_273, 2_000.0);
        let engaged = Character {
            rate: 0.35,
            reconstruction: 0.6,
            jitter: 0.7,
            ..Character::default()
        };
        for character in [Character::default(), engaged] {
            let mut hard = looping(Reader::Repitch, 0.0);
            hard.character = character;
            // **Compared up to the first wrap, and no further.** The fade moves the return point in by
            // its own length, so after a wrap the position lands a thousandth of a frame later; sooner
            // or later a wrap then falls one sample earlier in one render than the other, and a maximum
            // difference reads that sample as a whole seam's step. That is how any hard wrap samples,
            // not a regime switch, and the return point's own continuity is
            // `the_fade_takes_its_material_from_inside_the_loop`'s to hold. The first pass still reads
            // the whole fade before Loop end and every tap across it.
            //
            // **A semitone down, not at the root.** At unity every read on the first pass lands on a
            // whole frame, where the kernel is the centre frame and nothing else: no tap across Loop end
            // and no fade shorter than a frame can reach the output, and every distance read zero. A
            // semitone down keeps the same tabled kernel and puts every read between frames.
            hard.layers[0].loop_start = 0.1;
            hard.layers[0].loop_end = 0.85;
            let ratio = 2.0f32.powf(-1.0 / 12.0);
            let first_wrap = (0.85 * 12_272.0 / ratio) as usize;
            let reference = render_layer_a(&sample, &hard, 59, first_wrap);
            let distance = |fade_frames: f32| {
                let mut params = hard;
                params.layers[0].loop_crossfade_s = fade_frames / 48_000.0;
                render_layer_a(&sample, &params, 59, first_wrap)
                    .iter()
                    .zip(&reference)
                    .map(|(a, b)| (a.left - b.left).abs().max((a.right - b.right).abs()))
                    .fold(0.0f32, f32::max)
            };
            // **Below a frame the distance must scale with the fade**, not merely stay small: an
            // implementation that skipped the seam below one frame and switched it on at one would
            // read zero on the rungs under a frame and pass a test that only asked for smallness — the
            // regime switch moved from zero to one frame rather than removed. Each rung is ten times
            // the last, so a continuous seam grows about tenfold between them.
            let near: Vec<f32> = [0.01f32, 0.1, 1.0]
                .iter()
                .map(|&frames| distance(frames))
                .collect();
            assert!(
                near[0] < 2.0e-3,
                "a thousandth of a frame of fade moved the render by {} ({character:?})",
                near[0]
            );
            for pair in near.windows(2) {
                assert!(
                    pair[1] > 5.0 * pair[0],
                    "a tenfold longer fade did not move the render proportionally further: {near:?} \
                     ({character:?})"
                );
            }
            // **And across one frame nothing jumps.** An easing from a separate whole-frame hard wrap
            // once ended there; the hard wrap is now this seam at zero, so there is nothing to end.
            let (under, over) = (distance(0.99), distance(1.01));
            assert!(
                (over - under).abs() < 0.05 * under.max(over),
                "the render jumps across one frame of fade: {under} at 0.99 frames, {over} at 1.01 \
                 ({character:?})"
            );
            let long = distance(64.0);
            assert!(
                long > over,
                "sixty-four frames of fade are no further from the hard wrap than one frame: \
                 {long} against {over}"
            );
        }
    }

    /// **Moving the loop and the fade under a sounding note neither clicks nor sends a read outside
    /// the region**, in Repitch and in Stretch — which re-seeks its cursor on every one of these
    /// samples, because every one of them moves the region's loop.
    ///
    /// Each control is swept alone and then all three together, through the range where the fade runs
    /// out of room outside the loop and the return point starts to move; the fade is also swept up from
    /// zero. A sweep may step no harder than 1.3× the larger of its two unswept endpoints, so passing
    /// through zero may step as the hard wrap does and no harder. **The read positions are checked
    /// every sample**, Stretch's heads and Repitch's playhead both, because the fetch clamps to the
    /// region and a read that strayed would render finite and plausible.
    ///
    /// **This is not the test that catches a rounded period, though its first version was written
    /// believing it was.** A period rounded to whole frames moves each blended frame by at most one
    /// frame, a smaller step than the tone's own; the output cannot show it, and falsifying the period
    /// that way left it green. `a_blended_frame_moves_smoothly_as_the_loop_end_slides` measures it
    /// where it lives.
    #[test]
    fn sweeping_the_loop_and_the_fade_under_a_note_neither_clicks_nor_strays() {
        const FRAMES: usize = 36_000;
        let sample = tone(48_000);
        // (loop start, loop end, fade seconds) at each end of a sweep, inside a region of 0.01..0.63.
        // **The region ends close past the loop on purpose.** A Stretch head runs up to a window —
        // 3 840 frames here — past where it anchored before it re-anchors onto the playhead, so a
        // head that stopped wrapping could only leave a region that ends within a window of Loop end.
        // With 14 400 frames of room after the loop, the first version of this test watched every
        // head stay inside while the wrap was removed altogether.
        type Patch = (f32, f32, f32);
        let sweeps: [(&str, Patch, Patch); 5] = [
            // Loop start closing onto Start: the pass before the loop shrinks to nothing while the
            // return point, a fade's length inside it, follows.
            ("Loop start", (0.05, 0.60, 0.03), (0.01, 0.60, 0.03)),
            ("Loop end", (0.05, 0.60, 0.03), (0.05, 0.62, 0.03)),
            // A fade growing from 480 to 2 400 frames, moving the return point in with it.
            ("the fade", (0.02, 0.60, 0.01), (0.02, 0.60, 0.05)),
            ("all three together", (0.05, 0.60, 0.01), (0.01, 0.62, 0.05)),
            (
                "the fade up from zero",
                (0.02, 0.60, 0.0),
                (0.02, 0.60, 0.02),
            ),
        ];
        let patch = |reader: Reader, (loop_start, loop_end, fade): Patch| {
            let mut params = looping(reader, fade);
            params.layers[0].start = 0.01;
            params.layers[0].end = 0.63;
            params.layers[0].loop_start = loop_start;
            params.layers[0].loop_end = loop_end;
            params
        };
        for reader in [Reader::Repitch, Reader::Stretch] {
            for (name, from, to) in sweeps {
                let still = |end: Patch| {
                    largest_step(&render_layer_a(&sample, &patch(reader, end), 60, FRAMES)[2_000..])
                };
                let bound = still(from).max(still(to));

                let mut params = patch(reader, from);
                let mut engine = engine_without_routes();
                engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
                let mut swept = Vec::with_capacity(FRAMES);
                for n in 0..FRAMES {
                    let t = n as f32 / FRAMES as f32;
                    let lerp = |a: f32, b: f32| a + (b - a) * t;
                    params.layers[0].loop_start = lerp(from.0, to.0);
                    params.layers[0].loop_end = lerp(from.1, to.1);
                    params.layers[0].loop_crossfade_s = lerp(from.2, to.2);
                    swept.push(engine.render([Some(&sample), None], &params));

                    let region = Region::looped(&sample, &params.layers[0]);
                    let inside = |position: f32| {
                        position >= region.start - 1.0e-3 && position <= region.end + 1.0e-3
                    };
                    for voice in engine.voices.iter().filter(|voice| voice.active) {
                        let layer = &voice.layers[0];
                        let reads: &[f32] = match reader {
                            Reader::Stretch => &[layer.ola[0].position, layer.ola[1].position],
                            _ => &[layer.position],
                        };
                        for &position in reads {
                            assert!(
                                inside(position),
                                "{reader:?}, sweeping {name}: a read at {position} left the region \
                                 {}..{} at sample {n}",
                                region.start,
                                region.end
                            );
                        }
                    }
                }
                let stepped = largest_step(&swept[2_000..]);
                assert!(
                    stepped <= 1.3 * bound,
                    "{reader:?}, sweeping {name}: stepped by {stepped} against its endpoints' {bound}"
                );
            }
        }
    }

    /// **The period slides, and a blended frame slides with it.** Loop points are fractional and
    /// automatable, so under automation a loop's length passes through fractions of a frame. A period
    /// rounded to whole frames would move every blended frame onto its neighbour at once each time the
    /// length crossed one — a zipper across the whole fade on a slow sweep. So the frames a kernel
    /// reads deep inside the fade are sampled while Loop end slides by thousandths of a frame, and none
    /// may move by more than that slide can explain.
    ///
    /// Falsified by rounding the partner's period: a frame deep in the fade then jumps by up to the
    /// tone's whole per-frame step at each crossing, which is many times this bound.
    #[test]
    fn a_blended_frame_moves_smoothly_as_the_loop_end_slides() {
        let sample = tone(48_000);
        let last = 47_999.0f32;
        let mut previous: Option<Vec<Stereo>> = None;
        let mut worst = 0.0f32;
        let mut moved = 0usize;
        for step in 0..1_000 {
            // Two thousandths of a frame a step, two frames in all, so a rounded period would change
            // several times on the way. The normalised parameter resolves about three thousandths of
            // a frame here, so some steps land on the same position and none skips more than one.
            let loop_end = (28_799.4 + step as f32 * 0.002) / last;
            let region = Region::looped(
                &sample,
                &LayerParams {
                    loop_start: 0.2,
                    loop_end,
                    loop_mode: LoopMode::Forward,
                    loop_crossfade_s: 0.03,
                    ..LayerParams::default()
                },
            );
            assert!(region.seam.is_some(), "no seam at step {step}");
            // Fixed indices well inside the 1 440-frame fade that ends at the seam.
            let frames: Vec<Stereo> = (27_600..28_700)
                .step_by(50)
                .map(|index| frame(&sample, index, region, LoopMode::Forward))
                .collect();
            if let Some(was) = &previous {
                for (before, after) in was.iter().zip(&frames) {
                    let change = (after.left - before.left).abs();
                    worst = worst.max(change);
                    moved += usize::from(change > 0.0);
                }
            }
            previous = Some(frames);
        }
        assert!(
            moved > 0,
            "sliding Loop end never moved a blended frame, so this measures nothing"
        );
        assert!(
            worst < 1.0e-3,
            "a blended frame jumped by {worst} while Loop end slid by thousandths of a frame"
        );
    }

    /// **Nothing that does not read a forward loop hears the fade**, on the bits. Grain schedules
    /// inside the region and never reads the loop — its read passes `LoopMode::Forward` only so taps
    /// wrap at region edges, and that path must not pick up the seam. Alternate turns at a
    /// value-continuous reflection, and a one-shot has no seam at all.
    #[test]
    fn grain_alternate_and_one_shot_do_not_hear_the_crossfade() {
        let sample = looped_tone(12_273, 220.0);
        for (reader, mode) in [
            (Reader::Grain, LoopMode::Forward),
            (Reader::Repitch, LoopMode::Alternate),
            (Reader::Stretch, LoopMode::Alternate),
            (Reader::Repitch, LoopMode::Off),
        ] {
            let mut hard = looping(reader, 0.0);
            hard.layers[0].loop_mode = mode;
            let mut faded = hard;
            faded.layers[0].loop_crossfade_s = 0.2;
            let one = render_layer_a(&sample, &hard, 60, 30_000);
            let other = render_layer_a(&sample, &faded, 60, 30_000);
            assert!(
                one.iter()
                    .zip(&other)
                    .all(|(a, b)| a.left.to_bits() == b.left.to_bits()
                        && a.right.to_bits() == b.right.to_bits()),
                "{reader:?} in {mode:?} heard the crossfade"
            );
        }
    }

    /// Stretch's identity at unity survives a crossfade: both readers read the same frames.
    #[test]
    fn the_identity_at_unity_survives_a_crossfaded_loop() {
        let rate = 48_000.0;
        let sample = struck(rate, &[4_000], 24_000);
        let repitch = render_layer_a(&sample, &looping(Reader::Repitch, 0.05), 60, 48_000);
        let stretched = render_layer_a(&sample, &looping(Reader::Stretch, 0.05), 60, 48_000);
        let worst = repitch
            .iter()
            .zip(stretched.iter())
            .map(|(a, b)| (a.left - b.left).abs().max((a.right - b.right).abs()))
            .fold(0.0f32, f32::max);
        assert!(worst < 1.0e-4, "a crossfade broke the identity: {worst}");
    }

    /// **The fade takes its material from inside the loop and nowhere else** (owner, 2026-09-14): the
    /// return point is Loop start plus the fade, the fade-in reads the frames between them, the fade
    /// reaches half the loop at most — and every quantity is continuous as a loop point sweeps. The
    /// editor draws from this same arithmetic.
    #[test]
    fn the_fade_takes_its_material_from_inside_the_loop() {
        // 10 001 frames, so a normalised position times ten thousand is a frame.
        let sample = tone(10_001);
        let fit = |loop_start: f32, loop_end: f32, fade_frames: f32| {
            Region::looped(
                &sample,
                &LayerParams {
                    loop_start,
                    loop_end,
                    loop_mode: LoopMode::Forward,
                    loop_crossfade_s: fade_frames / 48_000.0,
                    ..LayerParams::default()
                },
            )
        };
        let close = |got: f32, wanted: f32, what: &str| {
            assert!(
                (got - wanted).abs() < 0.05,
                "{what}: {got} against {wanted}"
            );
        };

        // Three thousand frames of room before the loop, and none of it is taken: the return point is
        // Loop start plus the fade, and the fade-in reads the loop's own opening between them. The
        // loop is the half-open span from frame 3 000 to one past frame 6 000, 3 001 frames.
        let region = fit(0.3, 0.6, 1_000.0);
        let seam = region.seam.expect("a seam");
        close(seam.plain_loop_start, 3_000.0, "Loop start");
        close(region.loop_start, 4_000.0, "return point");
        close(seam.length, 1_000.0, "fade");
        close(seam.period, 2_001.0, "period");
        let fade_out = region.loop_start + seam.period - seam.length;
        close(fade_out, 5_001.0, "fade-out start");

        // Where the fade-in's material comes from: one period before the fade-out, which is the
        // stretch from Loop start to the return point — inside the loop, never before it.
        close(
            fade_out - seam.period,
            seam.plain_loop_start,
            "first frame faded in",
        );
        close(
            region.loop_end + 1.0 - seam.period,
            region.loop_start,
            "last frame faded in",
        );

        // The same on a whole-region loop, which has nothing outside it to take.
        let region = fit(0.0, 1.0, 1_000.0);
        let seam = region.seam.expect("a seam");
        close(region.loop_start, 1_000.0, "return point");
        close(seam.period, 9_001.0, "period");

        // The fade and the loop it shortens cannot overlap, so a fade reaches half the loop.
        let seam = fit(0.3, 0.6, 2_000.0).seam.expect("a seam");
        close(seam.length, 1_500.5, "ceiling");
        close(seam.period, 1_500.5, "period at the ceiling");
        let seam = fit(0.0, 1.0, 8_000.0).seam.expect("a seam");
        close(seam.length, 5_000.5, "ceiling on a whole-region loop");

        // Zero is the hard wrap, by the loop's exact span; so is Alternate, whatever the fade asks
        // for. Grain never reads the loop at all.
        let zero = fit(0.3, 0.6, 0.0)
            .seam
            .expect("a hard wrap still has its span");
        assert_eq!(zero.length, 0.0, "zero fade built a fade");
        close(zero.period, 3_001.0, "hard wrap's period");
        let mut params = LayerParams {
            loop_start: 0.3,
            loop_end: 0.6,
            loop_mode: LoopMode::Alternate,
            loop_crossfade_s: 0.1,
            ..LayerParams::default()
        };
        assert_eq!(
            Region::looped(&sample, &params)
                .seam
                .map(|seam| seam.length),
            Some(0.0),
            "Alternate took a fade"
        );
        params.loop_mode = LoopMode::Forward;
        assert!(loop_crossfade(&sample, &params).effective_s > 0.0);
        params.reader = Reader::Grain;
        assert_eq!(
            loop_crossfade(&sample, &params).effective_s,
            0.0,
            "Grain never reads the loop, so no fade may be drawn for it"
        );

        // Continuous: Loop start sweeping one frame at a time, and Loop end closing in until the
        // ceiling takes over, move nothing by more than a frame.
        let mut previous = fit(0.0, 0.6, 1_000.0);
        for step in 1..=2_000 {
            let now = fit(step as f32 * 1.0e-4, 0.6, 1_000.0);
            let (was, is) = (previous.seam.unwrap(), now.seam.unwrap());
            for (what, from, to) in [
                ("return point", previous.loop_start, now.loop_start),
                ("fade", was.length, is.length),
                ("period", was.period, is.period),
            ] {
                assert!(
                    (from - to).abs() <= 1.05,
                    "{what} jumped from {from} to {to} at step {step}"
                );
            }
            previous = now;
        }
        let mut previous = fit(0.3, 0.6, 1_000.0);
        for step in 1..=1_500 {
            let now = fit(0.3, 0.6 - step as f32 * 1.0e-4, 1_000.0);
            let (was, is) = (previous.seam.unwrap(), now.seam.unwrap());
            for (what, from, to) in [
                ("return point", previous.loop_start, now.loop_start),
                ("fade", was.length, is.length),
                ("period", was.period, is.period),
            ] {
                assert!(
                    (from - to).abs() <= 1.05,
                    "{what} jumped from {from} to {to} at step {step}"
                );
            }
            previous = now;
        }
    }

    /// **The kernel reads the seam's frames, walked or contiguous.** A walk over a crossfaded loop
    /// fetches through the seam tap by tap, and its contiguous fast path may only be taken where the
    /// frames are the untouched recording — a run that strayed into a fade or across the seam would
    /// read the wrong audio exactly where nobody looks.
    #[test]
    fn the_kernel_reads_the_seam_frames_whether_walked_or_contiguous() {
        let sample = ruler(512);
        let cases = [
            // A loop inside the region, with material before and after it that no read may take.
            LayerParams {
                start: 0.1,
                end: 0.9,
                loop_start: 0.3,
                loop_end: 0.6,
                loop_crossfade_s: 20.0 / 48_000.0,
                ..LayerParams::default()
            },
            // A whole-region loop.
            LayerParams {
                loop_crossfade_s: 40.0 / 48_000.0,
                ..LayerParams::default()
            },
            // Under a frame of fade.
            LayerParams {
                start: 0.1,
                end: 0.9,
                loop_start: 0.3,
                loop_end: 0.6,
                loop_crossfade_s: 0.3 / 48_000.0,
                ..LayerParams::default()
            },
        ];
        for mut params in cases {
            params.loop_mode = LoopMode::Forward;
            let region = Region::looped(&sample, &params);
            assert!(region.seam.is_some(), "{params:?} built no seam");
            let mut fast_runs = 0usize;
            for step in -4..=520i64 {
                let first = step - i64::from(SINC_TAPS / 2 - 1);
                let mut walk = Walk::new(&sample, first, region, LoopMode::Forward);
                let contiguous = walk.contiguous(SINC_TAPS_N);
                fast_runs += usize::from(contiguous.is_some());
                for tap in 0..SINC_TAPS_N {
                    let wanted = frame(&sample, first + tap as i64, region, LoopMode::Forward);
                    assert_eq!(
                        walk.fetch(&sample),
                        wanted,
                        "tap {tap} of the run at {step}"
                    );
                    if let Some(start) = contiguous {
                        assert_eq!(
                            sample.frames[start + tap][0],
                            wanted.left,
                            "the contiguous run at {step} read past the untouched frames"
                        );
                    }
                    walk.advance();
                }
            }
            assert!(
                fast_runs > 0,
                "the fast path was never taken for {params:?}"
            );
        }
    }

    /// **A loop written from a file's frames reads back as those frames**, at every length the
    /// importer accepts. The loop points are normalised `f32`s, which cannot place every frame of a long
    /// file, so without `settle_on_frame` a loop imported from a 90 000-frame file started a few
    /// thousandths of a frame off its frame and every wrapped tap interpolated.
    #[test]
    fn loop_frames_survive_the_normalised_parameter() {
        let mut state = 0x243f_6a88_85a3_08d3u64;
        for frames in [169usize, 1_025, 87_375, 1_048_577, 5_760_000] {
            let last = (frames - 1) as f32;
            let top = frames as u32 - 1;
            let mut pairs = vec![(0u32, top), (0, 1), (top - 1, top)];
            for _ in 0..4_000 {
                let a = ((split_mix(&mut state) * last) as u32).min(top - 1);
                let b = ((split_mix(&mut state) * last) as u32).min(top);
                pairs.push((a.min(b), a.max(b).max(a.min(b) + 1)));
            }
            for (from, to) in pairs {
                let region = Region::new(
                    frames,
                    &LayerParams {
                        loop_start: from as f32 / last,
                        loop_end: to as f32 / last,
                        ..LayerParams::default()
                    },
                );
                assert_eq!(
                    (region.loop_start, region.loop_end),
                    (from as f32, to as f32),
                    "a loop over frames {from}..{to} of {frames} read back off its frames"
                );
            }
            // **A deliberate fraction stays where it lies**, low, middle and high in the file (code
            // review round 1): only a value bit for bit a frame's own image is settled. At the
            // importer's cap a nearness test rounded every one of these.
            for position in [6.25f32, (last * 0.35).floor() + 0.5, last - 2.5] {
                let normalised = position / last;
                assert_ne!(
                    (normalised * last).round() / last,
                    normalised,
                    "{position} of {frames} is a frame's own image, so this case proves nothing"
                );
                let region = Region::new(
                    frames,
                    &LayerParams {
                        loop_start: normalised,
                        loop_end: 1.0,
                        ..LayerParams::default()
                    },
                );
                assert_eq!(
                    region.loop_start,
                    normalised * last,
                    "a loop start placed at {position} of {frames} was moved"
                );
            }
        }
    }

    /// **Equal loop points are a one-frame loop, and play that frame** (code review round 1). The span
    /// is half-open, so a WAV loop whose start and end name one frame loops that frame; `Region::new`
    /// once widened equal points by a frame, from when the span was their difference, and played two.
    /// Rendered at root over frames alternating +0.5 and −0.5, a one-frame loop reads one value.
    #[test]
    fn equal_loop_points_loop_the_one_frame_they_name() {
        let frames = 4_096usize;
        let sample = Sample::new(
            (0..frames)
                .map(|n| {
                    let value = if n % 2 == 0 { 0.5 } else { -0.5 };
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .unwrap();
        let last = (frames - 1) as f32;
        let mut params = looping(Reader::Repitch, 0.0);
        params.layers[0].start = 0.0;
        params.layers[0].end = 1.0;
        params.layers[0].loop_start = 1_000.0 / last;
        params.layers[0].loop_end = 1_000.0 / last;
        let region = Region::looped(&sample, &params.layers[0]);
        assert_eq!(
            (region.loop_start, region.loop_end, region.loop_span()),
            (1_000.0, 1_000.0, 1.0),
            "equal loop points did not make a one-frame loop"
        );
        assert_eq!(region.wrap_loop(1_001.0), 1_000.0);

        let render = render_layer_a(&sample, &params, 60, 8_000);
        let settled = largest_step(&render[4_000..]);
        assert!(
            settled < 1.0e-3,
            "a one-frame loop still steps by {settled} sample to sample, so it reads more than one frame"
        );
    }

    /// **A loop plays every frame from Loop start through Loop end, and repeats at exactly that
    /// length** (`plans/plan-sampler-wav-loop-import.md` §1.1). The float wraps used a period one frame
    /// short of the span the taps wrapped over. A single cycle — the shape the owner's JD-800 waves
    /// are, 169 frames looped over 0..168 — then played 168 frames a pass: 10 cents sharp, with a frame
    /// skipped every cycle.
    ///
    /// Repitch and Stretch, on a buffer that is exactly one cycle and on a loop over one cycle that
    /// ends where loud material begins. **At root the output repeats every 169 samples**, and not one
    /// sample sooner, which is what one frame short does. **A semitone down, where every read falls
    /// between frames and the kernel's taps reach across Loop end**, no read takes the loud material:
    /// the loop's peak is the peak of the same cycle looped whole.
    #[test]
    fn a_whole_frame_loop_repeats_every_frame_it_names() {
        const CYCLE: usize = 169;
        let wave = |n: usize| {
            let phase = TAU * (n % CYCLE) as f32 / CYCLE as f32;
            phase.sin() * 0.5 + (2.0 * phase).sin() * 0.2
        };
        // One cycle, looped whole: its last index is 168, so Loop end 1.0 is frame 168.
        let single = Sample::new((0..CYCLE).map(|n| [wave(n), wave(n)]).collect(), 48_000.0)
            .expect("a single cycle");
        // Two cycles, then loud material, in 1 025 frames so a normalised point times 1 024 is a
        // whole frame: the loop is the second cycle, frames 169..337, and frame 338 onward is DC.
        let early = Sample::new(
            (0..1_025)
                .map(|n| {
                    if n < 2 * CYCLE {
                        [wave(n), wave(n)]
                    } else {
                        [0.9, 0.9]
                    }
                })
                .collect(),
            48_000.0,
        )
        .expect("a loop before loud material");
        let peak = |render: &[Stereo]| render.iter().fold(0.0f32, |p, v| p.max(v.left.abs()));
        for reader in [Reader::Repitch, Reader::Stretch] {
            let mut whole = looping(reader, 0.0);
            whole.layers[0].loop_start = 0.0;
            whole.layers[0].loop_end = 1.0;
            let mut before_loud = whole;
            before_loud.layers[0].loop_start = CYCLE as f32 / 1_024.0;
            before_loud.layers[0].loop_end = (2 * CYCLE - 1) as f32 / 1_024.0;
            for (label, sample, params) in [
                ("one cycle looped whole", &single, whole),
                ("one cycle looped before loud material", &early, before_loud),
            ] {
                let rendered = render_layer_a(sample, &params, 60, 12_000);
                let apart = |lag: usize| {
                    (6_000..11_000)
                        .map(|n| (rendered[n + lag].left - rendered[n].left).abs())
                        .fold(0.0f32, f32::max)
                };
                let (exact, short) = (apart(CYCLE), apart(CYCLE - 1));
                assert!(
                    exact < 1.0e-4,
                    "{reader:?}, {label}: the output does not repeat every {CYCLE} samples \
                     (off by {exact})"
                );
                assert!(
                    short > 1.0e-2,
                    "{reader:?}, {label}: the output also repeats a sample sooner ({short}), so \
                     this measures nothing"
                );
            }
            let whole_peak = peak(&render_layer_a(&single, &whole, 59, 12_000)[6_000..]);
            let loop_peak = peak(&render_layer_a(&early, &before_loud, 59, 12_000)[6_000..]);
            assert!(
                loop_peak <= whole_peak * 1.05,
                "{reader:?}: a loop before loud material peaked at {loop_peak} against the cycle's \
                 own {whole_peak} — a read took a frame past Loop end"
            );
        }
    }

    /// **A loop whose points fall between frames is one loop, not two.** Every tap past the loop's
    /// half-open span reads the loop signal at `index − kP`, interpolated between the frames either
    /// side, and Repitch's position and Stretch's heads wrap by the same `P`. Revision 1 of the plan
    /// lengthened the float period by a frame over today's whole-frame taps, which agrees only when
    /// both points are whole frames. The oracle reads a ruler, whose frame `i` holds `i`, so the
    /// expected tap is simply the position it should land on.
    ///
    /// **With whole-frame points the model's taps are exactly the integer wrap this crate always
    /// made**, in Forward and Alternate, which is what keeps Alternate's renders on whole-frame loops
    /// as they were.
    #[test]
    fn a_loop_between_frames_wraps_every_reader_by_one_span() {
        let sample = ruler(1_025);
        let last = 1_024.0f32;
        // Loop start 100.25 and Loop end 300.625, both exact in the normalised parameter: the loop is
        // the half-open span from 100.25 to 301.625, 201.375 frames.
        let (loop_start, loop_end) = (100.25f64, 300.625f64);
        let span = loop_end + 1.0 - loop_start;
        let fractional = LayerParams {
            loop_start: loop_start as f32 / last,
            loop_end: loop_end as f32 / last,
            loop_mode: LoopMode::Forward,
            ..LayerParams::default()
        };

        // The hard wrap: every tap, before, inside and past the loop, is the loop signal.
        let region = Region::looped(&sample, &fractional);
        assert!(
            (region.loop_span() as f64 - span).abs() < 1.0e-3,
            "the span is {} frames, not {span}",
            region.loop_span()
        );
        for index in -40..1_100i64 {
            let wanted = loop_start + (index as f64 - loop_start).rem_euclid(span);
            let got = frame(&sample, index, region, LoopMode::Forward).left as f64;
            assert!(
                (got - wanted).abs() < 1.0e-3,
                "tap {index} read {got}, not the loop signal at {wanted}"
            );
        }

        // With a fade: past the fade-out, the same wrap by the period from the return point; inside
        // it, the documented blend of the frame and the one a period earlier.
        let mut faded = fractional;
        faded.loop_crossfade_s = 20.5 / 48_000.0;
        let region = Region::looped(&sample, &faded);
        let seam = region.seam.expect("a seam");
        let (low, period, length) = (
            region.loop_start as f64,
            seam.period as f64,
            seam.length as f64,
        );
        assert!(
            (low - (loop_start + 20.5)).abs() < 1.0e-3,
            "return point {low}"
        );
        for index in -40..1_100i64 {
            let at = low + (index as f64 - low).rem_euclid(period);
            let fade_out = low + period - length;
            let wanted = if at >= fade_out {
                let rise = seam_rise(((at - fade_out) / length) as f32) as f64;
                at + (at - period - at) * rise
            } else {
                at
            };
            let got = frame(&sample, index, region, LoopMode::Forward).left as f64;
            assert!(
                (got - wanted).abs() < 2.0e-3,
                "with a fade, tap {index} read {got}, not {wanted}"
            );
        }

        // Positions: a Stretch head past the span, and Repitch's playhead at its first wrap.
        let region = Region::looped(&sample, &fractional);
        for (from, to) in [
            (region.loop_limit() + 0.5, region.loop_start + 0.5),
            (region.loop_start - 0.25, region.loop_limit() - 0.25),
        ] {
            let wrapped = wrap_read(from, region, LoopMode::Forward);
            assert!(
                (wrapped - to).abs() < 1.0e-3,
                "a head at {from} wrapped to {wrapped}, not {to}"
            );
        }
        let mut params = stretch_params(Reader::Repitch);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[0].loop_start = fractional.loop_start;
        params.layers[0].loop_end = fractional.loop_end;
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        let mut previous = 0.0f32;
        let mut wrapped = None;
        for _ in 0..600 {
            engine.render([Some(&sample), None], &params);
            let position = engine
                .voices
                .iter()
                .find(|v| v.active)
                .expect("sounding")
                .layers[0]
                .position;
            if position < previous {
                wrapped = Some((previous, position));
                break;
            }
            previous = position;
        }
        let (before, after) = wrapped.expect("the playhead never wrapped");
        assert!(
            ((before + 1.0 - after) as f64 - span).abs() < 1.0e-3,
            "Repitch wrapped from {before} to {after}, a period of {} rather than {span}",
            before + 1.0 - after
        );

        // Whole-frame points: exactly the integer wrap, bit for bit, in both looping modes.
        for loop_mode in [LoopMode::Forward, LoopMode::Alternate] {
            let whole = LayerParams {
                loop_start: 100.0 / last,
                loop_end: 300.0 / last,
                loop_mode,
                ..LayerParams::default()
            };
            let model = Region::looped(&sample, &whole);
            let plain = Region::new(sample.len(), &whole);
            for index in -40..1_100i64 {
                assert_eq!(
                    frame(&sample, index, model, loop_mode),
                    frame_plain(&sample, index, plain, loop_mode),
                    "{loop_mode:?}: whole-frame tap {index} moved"
                );
            }
        }
    }

    /// Transposed, the reader must still traverse the source at the scan rate rather than at the
    /// pitch rate — that is the whole difference between Stretch and Repitch — and it must stay
    /// finite while doing it.
    #[test]
    fn stretch_holds_the_timeline_while_the_key_moves_the_pitch() {
        let rate = 48_000.0;
        let sample = struck(rate, &[8_000], 96_000);
        for key in [48, 55, 60, 67, 72] {
            let params = stretch_params(Reader::Stretch);
            let mut engine = engine_without_routes();
            engine.note_on(Note::midi(key, 1.0), [Some(&sample), None], &params);
            for _ in 0..48_000 {
                let value = engine.render([Some(&sample), None], &params);
                assert!(value.left.is_finite() && value.right.is_finite());
            }
            let playhead = engine.playhead(0).expect("a sounding voice");
            // One second of scan at unity is one second of source, whatever the key.
            assert!(
                (playhead - 48_000.0).abs() < 2_000.0,
                "key {key} travelled to {playhead}, which is the pitch rate rather than the scan rate"
            );
        }
    }

    /// Zero scan holds the playhead and the alignment search keeps running, so a hold is a
    /// sustained aligned loop of the neighbourhood rather than silence or a frozen buffer.
    #[test]
    fn a_hold_freezes_the_playhead_and_keeps_sounding() {
        let rate = 48_000.0;
        let sample = struck(rate, &[2_000], 48_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].stretch_travel = 0.0;
        params.layers[0].start = 0.25;
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        let start = engine.playhead(0).expect("a sounding voice");
        let mut energy = 0.0f32;
        for _ in 0..48_000 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
            energy += value.left * value.left;
        }
        let end = engine.playhead(0).expect("still sounding");
        assert!(
            (end - start).abs() < 1.0,
            "a hold moved the playhead from {start} to {end}"
        );
        assert!(energy > 1.0e-4, "a hold went silent");
    }

    /// The two reversals are different controls and both must survive. Layer Reverse turns the
    /// playhead *and* the read around; negative Scan turns only the playhead around, so the
    /// windows still read the source forwards.
    ///
    /// **Direction is counted step by step rather than measured end to end.** A looping playhead
    /// that starts at a region edge wraps on its first backwards step, so comparing where it
    /// finished against where it began reports the wrap and not the direction — which is how the
    /// first version of this test failed while the reader was behaving correctly.
    fn backwards_share(params: &Params, sample: &Sample, steps: usize) -> f32 {
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(sample), None], params);
        let mut previous = engine.playhead(0).expect("sounding");
        let mut backwards = 0usize;
        let mut counted = 0usize;
        for _ in 0..steps {
            let value = engine.render([Some(sample), None], params);
            assert!(value.left.is_finite() && value.right.is_finite());
            let now = engine.playhead(0).expect("sounding");
            // Skip the wrap itself; it is a jump, not a step.
            if (now - previous).abs() < 8.0 {
                counted += 1;
                if now < previous {
                    backwards += 1;
                }
            }
            previous = now;
        }
        backwards as f32 / counted.max(1) as f32
    }

    #[test]
    fn the_two_reversals_stay_different() {
        let rate = 48_000.0;
        let sample = struck(rate, &[10_000], 48_000);

        let mut reversed = stretch_params(Reader::Stretch);
        reversed.layers[0].reverse = true;
        reversed.layers[0].loop_mode = LoopMode::Forward;
        assert!(
            backwards_share(&reversed, &sample, 12_000) > 0.99,
            "layer Reverse did not walk the playhead backwards"
        );

        let mut backwards_scan = stretch_params(Reader::Stretch);
        backwards_scan.layers[0].stretch_travel = -1.0;
        backwards_scan.layers[0].loop_mode = LoopMode::Forward;
        assert!(
            backwards_share(&backwards_scan, &sample, 12_000) > 0.99,
            "negative Scan did not walk the playhead backwards"
        );

        // And they are not the same sound: Reverse reads the waveform backwards, negative Scan
        // reads it forwards inside windows that march backwards.
        let one = render_layer_a(&sample, &reversed, 60, 24_000);
        let other = render_layer_a(&sample, &backwards_scan, 60, 24_000);
        let difference = one
            .iter()
            .zip(other.iter())
            .map(|(a, b)| (a.left - b.left).abs())
            .fold(0.0f32, f32::max);
        assert!(
            difference > 1.0e-3,
            "the two reversals produced the same output"
        );
    }

    /// A region that does not loop ends: the playhead stops at the boundary, no new head launches
    /// past it, and the live ones finish into silence rather than wrapping.
    #[test]
    fn a_region_that_does_not_loop_finishes_and_stops() {
        let rate = 48_000.0;
        let sample = struck(rate, &[2_000], 24_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Off;
        params.layers[0].end = 0.25;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        let mut tail = 0.0f32;
        for n in 0..48_000 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
            if n > 24_000 {
                tail = tail.max(value.left.abs());
            }
        }
        assert!(
            tail < 1.0e-4,
            "a non-looping region kept sounding past its end at {tail}"
        );
    }

    /// A wrap under a live head must stay bounded and finite — the head is mid-window when the
    /// playhead jumps the region, which is the case the old code spliced hard.
    #[test]
    fn a_wrap_under_a_live_head_stays_bounded() {
        let rate = 48_000.0;
        let sample = struck(rate, &[500], 12_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[0].start = 0.1;
        params.layers[0].end = 0.35;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(72, 1.0), [Some(&sample), None], &params);
        let mut peak = 0.0f32;
        for _ in 0..96_000 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
            peak = peak.max(value.left.abs()).max(value.right.abs());
        }
        assert!(peak < 4.0, "a wrap under a head reached {peak}");
    }

    /// Stretch is contractually deterministic. A correlation search is; nothing here may quietly
    /// introduce a draw.
    #[test]
    fn stretch_is_deterministic() {
        let rate = 48_000.0;
        let sample = struck(rate, &[3_000, 9_000], 48_000);
        let params = stretch_params(Reader::Stretch);
        let first = render_layer_a(&sample, &params, 64, 24_000);
        let second = render_layer_a(&sample, &params, 64, 24_000);
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.left.to_bits(), b.left.to_bits());
            assert_eq!(a.right.to_bits(), b.right.to_bits());
        }
    }

    /// The detector finds the attacks it was pointed at, and refuses the ones that are not there.
    #[test]
    fn the_transient_map_finds_attacks_and_declines_what_has_none() {
        let rate = 48_000.0;
        let onsets = [6_000usize, 18_000, 30_000, 42_000];
        let sample = struck(rate, &onsets, 48_000);
        let found = sample.transients();
        assert!(
            found.len() >= onsets.len(),
            "found {} onsets, expected at least {}",
            found.len(),
            onsets.len()
        );
        for &expected in &onsets {
            let nearest = found
                .iter()
                .map(|&f| (f as i64 - expected as i64).abs())
                .min()
                .unwrap_or(i64::MAX);
            assert!(
                nearest < (rate * 0.012) as i64,
                "no marker within 12 ms of the attack at {expected}; nearest was {nearest} frames"
            );
        }

        let silence = Sample::new(vec![[0.0, 0.0]; 48_000], rate).unwrap();
        assert!(silence.transients().is_empty(), "silence reported attacks");

        let steady: Vec<[f32; 2]> = (0..48_000)
            .map(|n| {
                let v = (TAU * 220.0 * n as f32 / rate).sin() * 0.5;
                [v, v]
            })
            .collect();
        let steady = Sample::new(steady, rate).unwrap();
        assert!(
            steady.transients().len() <= 1,
            "a steady tone reported {} attacks",
            steady.transients().len()
        );

        // **A steady bass tone is the case that was wrong, and it is the reason this assertion
        // exists.** A detection window shorter than one period of the fundamental sees the
        // waveform's own swing as a string of attacks: at 55 Hz, with a 10 ms window, this
        // reported **55 onsets in two seconds** and every one of them hard-spliced both overlap
        // heads. The splice artefact measured **+18 dB above the music** on that material — worse
        // than the clocked splice S1 replaced — and the first round of measurements missed it
        // entirely because every source in the probe was 220 Hz or above.
        for hz in [40.0f32, 55.0, 82.0, 110.0] {
            let frames: Vec<[f32; 2]> = (0..96_000)
                .map(|n| {
                    let t = n as f32 / rate;
                    let mut value = 0.0;
                    for harmonic in 1..=12 {
                        value += (TAU * hz * harmonic as f32 * t).sin() / harmonic as f32;
                    }
                    let value = value * 0.25;
                    [value, value]
                })
                .collect();
            let low = Sample::new(frames, rate).unwrap();
            assert!(
                low.transients().is_empty(),
                "a steady {hz} Hz tone reported {} onsets; a window shorter than its period reads                  every cycle as an attack",
                low.transients().len()
            );
        }

        // A one-frame source has no neighbourhood to compare against, and declining is correct.
        assert!(
            Sample::new(vec![[0.5, 0.5]], rate)
                .unwrap()
                .transients()
                .is_empty()
        );
    }

    /// The period track is what narrows the alignment search, so it has to be right where it
    /// answers and silent where it cannot — a guessed period narrows the search onto the wrong
    /// offset, which is worse than not narrowing at all.
    #[test]
    fn the_period_track_measures_what_it_can_and_declines_the_rest() {
        let rate = 48_000.0;
        for hz in [55.0f32, 110.0, 220.0, 440.0] {
            let frames: Vec<[f32; 2]> = (0..96_000)
                .map(|n| {
                    let t = n as f32 / rate;
                    let mut value = 0.0;
                    for harmonic in 1..=12 {
                        value += (TAU * hz * harmonic as f32 * t).sin() / harmonic as f32;
                    }
                    let value = value * 0.25;
                    [value, value]
                })
                .collect();
            let tone = Sample::new(frames, rate).unwrap();
            let measured = tone
                .period_at(48_000.0)
                .expect("a period for a steady tone");
            let expected = rate / hz;
            assert!(
                (measured - expected).abs() < expected * 0.02,
                "{hz} Hz: tracked {measured} frames against {expected}"
            );
        }

        // **Declining is the half that keeps the fallback honest.** Noise and silence have no
        // period, and a track that invented one would aim the alignment search at an offset the
        // material does not support — worse than the blind range it replaced.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let noise: Vec<[f32; 2]> = (0..96_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let value = (state >> 40) as f32 / 8_388_608.0 - 1.0;
                [value * 0.4, value * 0.4]
            })
            .collect();
        let noise = Sample::new(noise, rate).unwrap();
        let declined = (0..40)
            .filter(|slot| noise.period_at(*slot as f32 * 2_400.0).is_none())
            .count();
        assert!(
            declined > 30,
            "the track committed to a period on noise in {} of 40 places",
            40 - declined
        );

        let silence = Sample::new(vec![[0.0, 0.0]; 96_000], rate).unwrap();
        assert!(
            silence.period_at(48_000.0).is_none(),
            "the track found a period in silence"
        );
    }

    /// **The gate's own measurement.** Transposed, the old reader re-anchored on a clock, so every
    /// window boundary was an unaligned splice and the attacks were crossfaded through. Aligning
    /// the splice and re-laying the heads at each marker has to leave a recognisable attack in the
    /// transposed output.
    ///
    /// Measured as the sharpest energy rise the output reaches: a smeared attack is a slower rise,
    /// whatever its level.
    #[test]
    fn transient_anchoring_keeps_the_attack_sharper_than_a_clocked_splice() {
        let rate = 48_000.0;
        let sample = struck(rate, &[24_000], 96_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Off;
        params.attack_s = 0.001;
        // Up a fifth, so the window rate and the material are unrelated — the case the old
        // scheduler could only get right by luck.
        let rendered = render_layer_a(&sample, &params, 67, 48_000);

        let envelope: Vec<f32> = rendered.iter().map(|v| (v.left + v.right).abs()).collect();
        let span = (rate * 0.002) as usize;
        let mut sharpest = 0.0f32;
        let mut n = span;
        while n + span < envelope.len() {
            let before: f32 = envelope[n - span..n].iter().sum::<f32>() / span as f32;
            let after: f32 = envelope[n..n + span].iter().sum::<f32>() / span as f32;
            sharpest = sharpest.max(after - before);
            n += span;
        }
        assert!(
            sharpest > 0.02,
            "the transposed output has no attack left in it: sharpest rise {sharpest}"
        );
    }

    /// **Alternate turns the playhead around.** It was silently Forward for Stretch alone while
    /// Repitch ping-ponged, because `wrap_read` folds Alternate onto Forward and nothing flipped
    /// the layer's direction.
    #[test]
    fn alternate_looping_turns_the_playhead_around() {
        let rate = 48_000.0;
        let sample = struck(rate, &[4_000], 48_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Alternate;
        params.layers[0].start = 0.2;
        params.layers[0].end = 0.4;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        // **Counting steps in each direction is not enough**, and the first version of this test
        // did exactly that: a playhead that merely reflects at the boundary without turning around
        // oscillates there, which registers plenty of steps both ways while traversing nothing. The
        // claim is that Alternate *sweeps the region in both directions*, so the assertion is the
        // span actually covered plus a reversal count in the low tens rather than the thousands.
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        let mut reversals = 0usize;
        let mut previous = engine.playhead(0).expect("sounding");
        let mut rising: Option<bool> = None;
        for _ in 0..96_000 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
            let now = engine.playhead(0).expect("sounding");
            low = low.min(now);
            high = high.max(now);
            if (now - previous).abs() > 1.0e-6 {
                let up = now > previous;
                if rising == Some(!up) {
                    reversals += 1;
                }
                rising = Some(up);
            }
            previous = now;
        }
        let region = (0.4 - 0.2) * 47_999.0;
        assert!(
            high - low > region * 0.8,
            "Alternate covered {:.0} frames of a {region:.0}-frame region",
            high - low
        );
        assert!(
            (4..64).contains(&reversals),
            "Alternate reversed {reversals} times; a sweep should turn around a handful of times"
        );
    }

    /// A source whose channels cancel is ordinary audio, and every analysis that reduces to mono
    /// has to survive it. `L + R` is silence here; the reader must still align, still find the
    /// attacks and still track the period.
    #[test]
    fn an_anti_phase_source_is_analysed_rather_than_silenced() {
        let rate = 48_000.0;
        let normal = struck(rate, &[6_000, 24_000], 48_000);
        let flipped: Vec<[f32; 2]> = normal.frames().iter().map(|f| [f[0], -f[0]]).collect();
        let flipped = Sample::new(flipped, rate).expect("an anti-phase source");
        assert_eq!(flipped.mono_mix(), [0.5, -0.5]);
        assert!(
            flipped.transients().len() >= 2,
            "anti-phase source reported {} onsets",
            flipped.transients().len()
        );
        let params = stretch_params(Reader::Stretch);
        let rendered = render_layer_a(&flipped, &params, 67, 24_000);
        assert!(rendered.iter().all(|v| v.left.is_finite()));
        assert!(
            rendered.iter().any(|v| v.left.abs() > 0.01),
            "an anti-phase source rendered silence"
        );
    }

    /// The marker cursor must survive the playhead being moved rather than walked — a region edited
    /// under a sounding note is the ordinary way that happens — and must do bounded work doing it.
    #[test]
    fn the_marker_cursor_survives_a_region_moved_under_a_sounding_note() {
        let rate = 48_000.0;
        let sample = struck(rate, &[2_000, 10_000, 20_000, 30_000, 40_000], 48_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(62, 1.0), [Some(&sample), None], &params);
        for step in 0..48_000 {
            // Sweep the region hard while the note sounds, which moves the playhead in jumps.
            params.layers[0].start = 0.05 + 0.4 * ((step as f32 / 4_000.0).sin() * 0.5 + 0.5);
            params.layers[0].end = params.layers[0].start + 0.3;
            let value = engine.render([Some(&sample), None], &params);
            assert!(
                value.left.is_finite() && value.right.is_finite(),
                "a moving region produced a non-finite sample at {step}"
            );
        }
    }

    /// Eight voices pressed together must not re-anchor in lockstep for the life of the chord: the
    /// searches would land on one sample, which is a callback deadline rather than an average.
    #[test]
    fn a_chord_does_not_re_anchor_in_lockstep() {
        let rate = 48_000.0;
        // Several attacks, and played at the root so the heads track the playhead exactly and the
        // replay guard lets every splice through — at a transposed pitch the heads run ahead, the
        // guard correctly skips, and the test would prove nothing about the splice at all.
        let sample = struck(rate, &[3_000, 9_000, 15_000, 21_000], 48_000);
        let params = stretch_params(Reader::Stretch);
        // **Eight voices at the same key**, so nothing but the stagger distinguishes them: same
        // pitch, same read step, same playhead, so they cross every marker together and any
        // resynchronisation shows as an exact collapse. Different keys hid it — the replay guard
        // decides differently per pitch, so some voices spliced and some did not, and the spread
        // survived for a reason that had nothing to do with the fix.
        let mut engine = Engine::new(rate);
        for id in 0..8u32 {
            let mut note = Note::midi(60, 1.0);
            note.id = Some(id);
            engine.note_on(note, [Some(&sample), None], &params);
        }
        let mut phases: Vec<f32> = Vec::new();
        for voice in &engine.voices {
            if voice.active {
                phases.push(voice.layers[0].ola[0].phase);
            }
        }
        assert_eq!(phases.len(), 8, "expected eight sounding voices");
        let spread = |phases: &[f32]| {
            let mut distinct = phases.to_vec();
            distinct.sort_by(|a, b| a.total_cmp(b));
            distinct.dedup_by(|a, b| (*a - *b).abs() < 0.01);
            distinct.len()
        };
        assert!(
            spread(&phases) >= 6,
            "eight voices share {} distinct start phases; they will re-anchor together",
            spread(&phases)
        );

        // **And it has to survive the attacks, which is where it was being undone.** The playhead
        // travels at the scan rate rather than the note's pitch, so every voice in a chord crosses
        // every marker at the same instant; a splice that re-laid them to a constant phase put the
        // whole chord back in lockstep at the first attack and kept it there.
        for _ in 0..24_000 {
            engine.render([Some(&sample), None], &params);
        }
        let after: Vec<f32> = engine
            .voices
            .iter()
            .filter(|voice| voice.active)
            .map(|voice| voice.layers[0].ola[0].phase)
            .collect();
        assert_eq!(after.len(), 8, "expected eight voices still sounding");
        assert!(
            spread(&after) >= 6,
            "after crossing shared attacks the chord collapsed to {} distinct phases",
            spread(&after)
        );
    }

    /// The onset list is binary-searched, so it has to be sorted — and refinement can reorder it,
    /// because it moves a marker forward by up to a window while two coarse detections may be
    /// accepted a shorter distance apart than that.
    #[test]
    fn the_onset_list_is_sorted_even_when_refinement_reorders_it() {
        let rate = 48_000.0;
        // Attacks spaced near the acceptance gap, which is what lets refinement overtake.
        for spacing in [1_500usize, 1_800, 2_100, 2_400, 3_000] {
            let onsets: Vec<usize> = (0..12).map(|n| 4_000 + n * spacing).collect();
            let sample = struck(rate, &onsets, 48_000);
            let found = sample.transients();
            assert!(
                found.windows(2).all(|pair| pair[0] <= pair[1]),
                "spacing {spacing}: onset list is not ascending: {found:?}"
            );
        }
    }

    /// **The peak, not the average.** The searches are bursty by construction — one per head per
    /// window — so the claim that they fit is a claim about the worst rendered sample, and a
    /// five-second mean cannot see it. This counts searches per sample across a full chord.
    #[test]
    fn no_single_sample_runs_more_than_a_few_alignment_searches() {
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[1] = params.layers[0];
        // **Across both rate extremes, source as well as host.** The search range is ten
        // milliseconds of *source*, so a high-rate source once probed proportionally more offsets;
        // the coarse stride widens with the range now, and this is what holds that. The accepted
        // extreme is a 768 kHz source under an 8 kHz host.
        for (source_rate, host_rate) in [
            (48_000.0f32, 44_100.0f32),
            (48_000.0, 48_000.0),
            (48_000.0, 96_000.0),
            (768_000.0, 8_000.0),
            (8_000.0, 96_000.0),
        ] {
            let sample = struck(
                source_rate,
                &[source_rate as usize / 8],
                source_rate as usize / 2,
            );
            let _ = &sample;
            let mut engine = Engine::new(host_rate);
            for id in 0..MAX_VOICES as u32 {
                let mut note = Note::midi(60, 1.0);
                note.id = Some(id);
                engine.note_on(note, [Some(&sample), Some(&sample)], &params);
            }
            let mut worst = 0u32;
            let mut total = 0u64;
            let blocks = (host_rate as usize).min(48_000);
            for _ in 0..blocks {
                ALIGN_SEARCHES.with(|count| count.set(0));
                engine.render([Some(&sample), Some(&sample)], &params);
                let now = ALIGN_SEARCHES.with(|count| count.get());
                worst = worst.max(now);
                total += u64::from(now);
            }
            // Sixteen is every layer of every voice searching on one sample, which is what the
            // stagger exists to prevent; four is comfortably under it and still slack enough that
            // an unlucky alignment of two voices does not fail the run.
            assert!(
                worst <= 4,
                "source {source_rate} into host {host_rate}: one sample ran {worst} searches"
            );
            // And the mean is what says the peak is not being bought with a flood of small ones.
            let mean = total as f64 / blocks as f64;
            assert!(
                mean < 0.5,
                "source {source_rate} into host {host_rate}: mean {mean:.3} searches per sample"
            );
        }
    }

    /// Alternate turns the playhead around and leaves the heads reading forward. The playhead half
    /// is checked above; this is the other half, which the sweep test cannot see.
    #[test]
    fn alternate_leaves_the_head_read_direction_alone() {
        let rate = 48_000.0;
        let sample = struck(rate, &[4_000], 48_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Alternate;
        params.layers[0].start = 0.2;
        params.layers[0].end = 0.4;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        for _ in 0..96_000 {
            engine.render([Some(&sample), None], &params);
            let direction = engine.voices.iter().find(|v| v.active).unwrap().layers[0].direction;
            assert_eq!(
                direction, 1.0,
                "Alternate reversed the head read direction, which is layer Reverse's job"
            );
        }
    }

    /// A marker causes **one** splice per passage, not none and not one per sample. Counting is the
    /// only way to see that; finite, bounded, non-silent output is true of every wrong answer too.
    /// **Random parameter values, from every reader, must not panic the engine.**
    ///
    /// This is `clap-validator`'s `param-fuzz-basic` brought in-crate. That test crashed the
    /// sampler and no other plugin in the collection, and it was the only one of the four fuzz
    /// tests that did: the other three snap each parameter to one end of its range, so the middle
    /// variant of an enum — `Reader::Stretch`, `LoopMode::Forward` — is the one thing they never
    /// select. Out of process all the crash says is `0xc0000409` and two failed allocations of a
    /// few dozen bytes, which is a panic message being formatted under nice-plug's audio-thread
    /// allocation guard rather than anything about memory. In here a panic says what it is and
    /// where.
    /// **A region with nothing in it must render, not abort the host.**
    ///
    /// Start and End meeting leaves `Region::len` at its one-frame floor, and the aligner's search
    /// bound was `clamp(1.0, half a frame)` — bounds that cross, which `f32::clamp` answers with a
    /// panic. Across the CLAP boundary a panic cannot unwind, so the host went down with it. The
    /// period track only narrows the search where it found a period and the playhead is away from
    /// an attack, so the source here is steady and pitched: a sine reports a period everywhere.
    #[test]
    fn a_region_with_no_room_in_it_still_renders() {
        let sample = Sample::new(
            (0..48_000)
                .map(|i| {
                    let v = (i as f32 / 48_000.0 * 220.0 * std::f32::consts::TAU).sin() * 0.5;
                    [v, v]
                })
                .collect(),
            48_000.0,
        )
        .unwrap();
        for (start, end) in [(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (0.5, 0.500_01)] {
            let mut params = Params::default();
            for layer in &mut params.layers {
                layer.reader = Reader::Stretch;
                layer.start = start;
                layer.end = end;
                layer.loop_start = start;
                layer.loop_end = end;
            }
            let mut engine = Engine::new(48_000.0);
            let samples = [Some(&sample), None];
            engine.note_on(
                Note {
                    id: Some(1),
                    channel: 0,
                    key: 60,
                    velocity: 0.9,
                    tuning: 0.0,
                },
                samples,
                &params,
            );
            for _ in 0..4_096 {
                let frame = engine.render(samples, &params);
                assert!(
                    frame.left.is_finite() && frame.right.is_finite(),
                    "a region of ({start}, {end}) rendered {frame:?}"
                );
            }
        }
    }

    #[test]
    fn no_random_parameter_combination_panics_the_engine() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / 16_777_216.0
        };
        // **The shape the instrument actually starts on.** A sine mix has no onsets at all, so
        // `Sample::new` hands back an empty transient list and the whole splice path — the part
        // `Reader::Stretch` gained in S2 — is skipped before it begins. A band-limited saw has a
        // sharp edge every cycle, which is what puts markers under the walk.
        let sample = Sample::new(
            (0..52_105)
                .map(|i| {
                    let phase = 142.0 * i as f32 / 52_105.0;
                    let v: f32 = (1..=91)
                        .map(|k| (phase * k as f32 * std::f32::consts::TAU).sin() / k as f32)
                        .sum();
                    [v * 0.5, v * -0.5]
                })
                .collect(),
            48_000.0,
        )
        .unwrap();
        let readers = [Reader::Repitch, Reader::Stretch, Reader::Grain];
        let loops = [LoopMode::Off, LoopMode::Forward, LoopMode::Alternate];
        let modes = [VoiceMode::Poly, VoiceMode::Mono, VoiceMode::Legato];
        let filters = [
            FilterMode::LowPass,
            FilterMode::BandPass,
            FilterMode::HighPass,
            FilterMode::Off,
        ];
        let mut engine = Engine::new(48_000.0);
        for permutation in 0..600 {
            let pick = |n: usize, r: f32| ((r * n as f32) as usize).min(n - 1);
            // **Every reader in turn rather than only by chance.** Reader is an enum, and the
            // defect this test was written for lived in the middle variant — the one a uniform
            // draw reaches a third of the time and a bounds-snapping fuzz never reaches at all.
            let reader = readers[permutation as usize % readers.len()];
            let layer = |r: &mut dyn FnMut() -> f32| {
                // **A quarter of the regions are closed on purpose.** Start and End are ordinary
                // automatable controls that can meet, and the region they leave is one frame long
                // — a degenerate case the arithmetic downstream has to survive, not a value only a
                // fuzzer would invent. Two independent uniform draws land within one frame of each
                // other about once in twenty-six thousand, which is to say never inside one run.
                let start = r();
                let end = if r() < 0.25 { start } else { r() };
                let loop_start = r();
                let loop_end = if r() < 0.25 { loop_start } else { r() };
                LayerParams {
                    reader,
                    root_note: r() * 127.0,
                    tune_semitones: (r() - 0.5) * 48.0,
                    start,
                    end,
                    loop_start,
                    loop_end,
                    loop_mode: loops[pick(3, r())],
                    // A third at zero, some under one frame, and some longer than any loop here.
                    loop_crossfade_s: match pick(3, r()) {
                        0 => 0.0,
                        1 => r() * 2.0e-5,
                        _ => r() * r() * 3.0,
                    },
                    reverse: r() < 0.5,
                    level: r(),
                    pan: r() * 2.0 - 1.0,
                    stretch_travel: r() * 2.0 - 1.0,
                    grain_size_s: 0.000_05 + r() * 0.5,
                    grain_density_hz: 0.1 + r() * 200.0,
                    grain_position_variation: r(),
                    grain_onset_timing: r(),
                    grain_spread_semitones: r() * 12.0,
                    grain_stereo_width: r(),
                    grain_reverse_chance: r(),
                    grain_shape: r(),
                }
            };
            let shapes = [
                LfoShape::Sine,
                LfoShape::Triangle,
                LfoShape::RampUp,
                LfoShape::RampDown,
                LfoShape::Square,
                LfoShape::SampleHold,
            ];
            let params = Params {
                layers: [layer(&mut next), layer(&mut next)],
                character: Character {
                    rate: next(),
                    converter: next(),
                    reconstruction: next(),
                    input: next(),
                    ..Character::default()
                },
                attack_s: next() * 10.0,
                decay_s: next() * 10.0,
                sustain: next(),
                release_s: next() * 10.0,
                filter_mode: filters[((next() * 4.0) as usize).min(3)],
                cutoff_hz: 20.0 + next() * 21_980.0,
                resonance: next(),
                filter_drive: next(),
                velocity_to_level: next(),
                lfo_rate_hz: 0.01 + next() * 40.0,
                lfo_shape: shapes[((next() * 6.0) as usize).min(5)],
                lfo2_rate_hz: 0.01 + next() * 40.0,
                lfo2_shape: shapes[((next() * 6.0) as usize).min(5)],
                lfo_amount: next(),
                master: next(),
                voice_mode: modes[((next() * 3.0) as usize).min(2)],
                glide_s: next() * 2.0,
            };
            // **The routing grid is drawn too, presences and amounts both.** The last
            // host-aborting crash in this plugin was a bound pair that could cross, and only
            // `param-fuzz-basic` could reach the enum variant that produced it — a fuzz that stops
            // at the fixed parameters would miss the same class again now that fifty-five more pairs
            // can each push a target somewhere it has never been.
            let mut fuzzed = routing::Routing::new();
            for target in 0..routing::TARGETS {
                for source in 0..routing::SOURCES {
                    fuzzed.present[target][source] = next() < 0.35;
                    fuzzed.amounts[target][source] = next() * 2.0 - 1.0;
                }
            }
            fuzzed.compact();
            engine.set_topology(&fuzzed);
            engine.set_amounts(&fuzzed);
            for channel in 0..16u8 {
                engine.set_wheel(channel, next());
                engine.set_bend_position(channel, next() * 2.0 - 1.0);
            }
            let samples = [Some(&sample), Some(&sample)];
            if permutation % 3 == 0 {
                engine.note_on(
                    Note {
                        id: Some(permutation),
                        channel: (permutation % 16) as u8,
                        key: (24 + permutation % 72) as u8,
                        velocity: 0.8,
                        tuning: 0.0,
                    },
                    samples,
                    &params,
                );
            }
            for _ in 0..512 {
                let frame = engine.render(samples, &params);
                assert!(
                    frame.left.is_finite() && frame.right.is_finite(),
                    "permutation {permutation} rendered {frame:?}"
                );
            }
            if permutation % 7 == 0 {
                engine.note_off(Some(permutation.saturating_sub(6)), 0, 48);
            }
        }
    }

    #[test]
    fn a_marker_splices_once_per_passage() {
        let rate = 48_000.0;
        // One marker, and a loop short enough to pass it a known number of times.
        let sample = struck(rate, &[6_000], 48_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[0].start = 0.0;
        params.layers[0].end = 0.25;
        params.layers[0].loop_start = 0.0;
        params.layers[0].loop_end = 0.25;
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        let span = 0.25 * 47_999.0;
        let passages = 4usize;
        for _ in 0..(span as usize * passages) {
            engine.render([Some(&sample), None], &params);
        }
        let splices = engine.voices.iter().find(|v| v.active).unwrap().layers[0].splices;
        assert!(
            (splices as i64 - passages as i64).abs() <= 1,
            "{passages} passages over one marker produced {splices} splices"
        );

        // **Alternate passes the same marker twice per round trip**, once each way, and its turns
        // are the case folded endpoints cannot see.
        let mut alternating = params;
        alternating.layers[0].loop_mode = LoopMode::Alternate;
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &alternating);
        let round_trips = 3usize;
        for _ in 0..(span as usize * 2 * round_trips) {
            engine.render([Some(&sample), None], &alternating);
        }
        let splices = engine.voices.iter().find(|v| v.active).unwrap().layers[0].splices;
        let expected = round_trips as i64 * 2;
        assert!(
            (splices as i64 - expected).abs() <= 2,
            "{round_trips} Alternate round trips over one marker produced {splices} splices, \
             expected about {expected}"
        );
    }

    /// **The traversal walk, tested directly rather than through the reader.** Going through a
    /// rendered voice could only ever show that the output stayed finite; these are the crossings
    /// themselves, including the two shapes that folded endpoints cannot see.
    #[test]
    fn the_traversal_walk_sees_every_passage_it_crosses() {
        fn layer_at(raw: f32, marker: usize) -> LayerVoice {
            LayerVoice {
                travel_raw: raw,
                marker,
                ..LayerVoice::default()
            }
        }
        // A loop of 100 frames starting at 0, with one marker in the middle.
        let span = 100.0f32;
        let transients = [50u32];
        let forward = |raw: f32| raw;
        let triangle = |raw: f32| if raw <= span { raw } else { span * 2.0 - raw };

        // Forward, no wrap, marker ahead: crossed.
        let mut layer = layer_at(0.0, 0);
        assert!(walk_traversal(
            &transients,
            &mut layer,
            10.0,
            50.0,
            span,
            span,
            forward
        ));
        // Forward, no wrap, marker behind: not crossed.
        let mut layer = layer_at(0.0, 1);
        assert!(!walk_traversal(
            &transients,
            &mut layer,
            60.0,
            30.0,
            span,
            span,
            forward
        ));
        // **Forward, wrapping past the seam onto the marker.** Endpoints 90 -> 60 do not bracket
        // 50 in a straight line — 60 is above it and 90 is above it — but the path 90 -> 100,
        // 0 -> 60 passes straight over it.
        let mut layer = layer_at(0.0, seek_marker(&transients, 90.0));
        assert!(
            walk_traversal(&transients, &mut layer, 90.0, 70.0, span, span, forward),
            "a wrapping step missed the marker beyond the seam"
        );
        // And a wrap that lands short of it does not claim a crossing.
        let mut layer = layer_at(0.0, seek_marker(&transients, 90.0));
        assert!(
            !walk_traversal(&transients, &mut layer, 90.0, 30.0, span, span, forward),
            "a wrapping step that stops before the marker reported crossing it"
        );

        // **Alternate across the turn, with identical folded endpoints.** 0.9 span -> 1.1 span both
        // fold to 90, and the path goes 90 -> 100 -> 90, passing nothing; move the marker to 95 and
        // it must be seen twice over.
        let near_turn = [95u32];
        let mut layer = layer_at(0.0, 0);
        assert!(
            walk_traversal(
                &near_turn,
                &mut layer,
                90.0,
                20.0,
                span,
                span * 2.0,
                triangle
            ),
            "a step across the Alternate turn reported no crossing while passing a marker"
        );

        // The segment budget: a step far longer than the loop re-seeks and reports nothing, and
        // leaves the cursor describing where the playhead actually landed.
        let mut layer = layer_at(0.0, 0);
        let crossed = walk_traversal(
            &transients,
            &mut layer,
            10.0,
            span * TRAVERSAL_SEGMENTS as f32 + 40.0,
            span,
            span,
            forward,
        );
        assert!(
            !crossed,
            "an over-budget step should report nothing crossed"
        );
        let destination = (10.0 + span * TRAVERSAL_SEGMENTS as f32 + 40.0).rem_euclid(span);
        assert_eq!(
            layer.marker,
            seek_marker(&transients, destination),
            "an over-budget step left the cursor away from the playhead"
        );

        // **Backwards, and backwards from the seam**, which is where the walk once stood still: at
        // raw 0 there is no edge behind it inside the period, so every piece came out empty and the
        // budget was spent making no progress — reverse traversal silently stopped seeing attacks.
        // **The cursor has to be the one a real start would produce**, not one chosen to suit the
        // test: negative Scan begins at the loop start, where `seek_marker` returns 0, and a
        // hand-picked 1 hid the fact that the reverse walk never re-seeked across the seam.
        let mut layer = layer_at(0.0, seek_marker(&transients, 0.0));
        assert!(
            walk_traversal(&transients, &mut layer, 0.0, -60.0, span, span, forward),
            "a reverse step from the seam missed the marker behind it"
        );
        let mut layer = layer_at(0.0, seek_marker(&transients, 70.0));
        assert!(
            walk_traversal(&transients, &mut layer, 70.0, -40.0, span, span, forward),
            "an ordinary reverse step missed the marker behind it"
        );
        let mut layer = layer_at(0.0, seek_marker(&transients, 70.0));
        assert!(
            !walk_traversal(&transients, &mut layer, 70.0, -10.0, span, span, forward),
            "a reverse step stopping short of the marker reported crossing it"
        );
        // Reverse across the Alternate turn, the mirror of the positive case above.
        let mut layer = layer_at(0.0, 1);
        assert!(
            walk_traversal(
                &near_turn,
                &mut layer,
                110.0,
                -20.0,
                span,
                span * 2.0,
                triangle
            ),
            "a reverse step across the Alternate turn missed the marker"
        );

        // **Just under the budget**, so the loop exhausts its segments rather than taking the
        // early over-budget path — a different branch, and the one that has to leave the cursor
        // where the playhead actually is.
        let mut layer = layer_at(0.0, 0);
        let step = span * TRAVERSAL_SEGMENTS as f32 - span * 0.5;
        walk_traversal(&transients, &mut layer, 10.0, step, span, span, forward);
        let landed = (10.0 + step).rem_euclid(span);
        assert_eq!(
            layer.marker,
            seek_marker(&transients, landed),
            "a step that exhausted the segment budget left the cursor behind"
        );

        // And an ordinary step leaves the cursor exactly where a fresh seek would put it.
        let mut layer = layer_at(0.0, 0);
        walk_traversal(&transients, &mut layer, 10.0, 70.0, span, span, forward);
        assert_eq!(
            layer.marker,
            seek_marker(&transients, 80.0),
            "an ordinary step left the cursor away from the playhead"
        );
    }

    /// Switching loop mode under a sounding note is a traversal discontinuity: Forward and Off do
    /// not maintain `travel_raw`, so Alternate would otherwise fold a stale one and throw the
    /// playhead back near the loop start.
    #[test]
    fn changing_loop_mode_under_a_note_does_not_jump_the_playhead() {
        let rate = 48_000.0;
        let sample = struck(rate, &[4_000, 12_000], 24_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[0].start = 0.1;
        params.layers[0].end = 0.6;
        params.layers[0].loop_start = 0.1;
        params.layers[0].loop_end = 0.6;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        for _ in 0..9_000 {
            engine.render([Some(&sample), None], &params);
        }
        let before = engine.playhead(0).expect("sounding");
        params.layers[0].loop_mode = LoopMode::Alternate;
        engine.render([Some(&sample), None], &params);
        let after = engine.playhead(0).expect("sounding");
        assert!(
            (after - before).abs() < 8.0,
            "a loop-mode change moved the playhead from {before} to {after}"
        );
        let voice = engine.voices.iter().find(|v| v.active).expect("sounding");
        let fresh = seek_marker(sample.transients(), after) as i64;
        assert!(
            (voice.layers[0].marker as i64 - fresh).abs() <= 1,
            "a loop-mode change left the cursor at {} against a fresh seek of {fresh}",
            voice.layers[0].marker
        );
        for _ in 0..48_000 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
        }
    }

    /// A region swept under a sounding note must not let the cursor describe the old one: the
    /// playhead is remapped, and a stale cursor walks a stretch of the onset list that the
    /// playhead never crossed.
    #[test]
    fn moving_the_region_resynchronises_the_cursor_and_drops_stale_splices() {
        let rate = 48_000.0;
        let sample = struck(rate, &[2_000, 10_000, 20_000, 30_000, 40_000], 48_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        for step in 0..48_000 {
            params.layers[0].start = 0.05 + 0.4 * ((step as f32 / 700.0).sin() * 0.5 + 0.5);
            params.layers[0].end = params.layers[0].start + 0.3;
            engine.render([Some(&sample), None], &params);
            let voice = engine.voices.iter().find(|v| v.active).expect("sounding");
            let layer = &voice.layers[0];
            // **Within one, not exactly equal.** `advance_marker` steps past a marker the playhead
            // has reached while `seek_marker` returns its index, so the two differ by one at exact
            // equality — which is a boundary convention, not drift. A stale cursor after a region
            // move is off by many.
            let fresh = seek_marker(sample.transients(), layer.travel) as i64;
            assert!(
                (layer.marker as i64 - fresh).abs() <= 1,
                "cursor at {} against a fresh seek of {fresh} at step {step}",
                layer.marker
            );
        }
    }

    /// **A loop shorter than one sample of travel** crosses several boundaries per rendered sample,
    /// which is what the segmented walk is bounded for. It must stay finite, bounded and defined.
    #[test]
    fn a_loop_shorter_than_one_step_stays_bounded() {
        let rate = 48_000.0;
        let sample = struck(rate, &[600], 12_000);
        for mode in [LoopMode::Forward, LoopMode::Alternate] {
            let mut params = stretch_params(Reader::Stretch);
            params.layers[0].loop_mode = mode;
            params.layers[0].start = 0.10;
            params.layers[0].end = 0.1002;
            params.layers[0].loop_start = 0.10;
            params.layers[0].loop_end = 0.1002;
            params.layers[0].stretch_travel = 2.0;
            let rendered = render_layer_a(&sample, &params, 96, 48_000);
            let peak = rendered
                .iter()
                .fold(0.0f32, |peak, v| peak.max(v.left.abs()).max(v.right.abs()));
            assert!(
                peak.is_finite() && peak < 4.0,
                "{mode:?} on a sub-step loop reached {peak}"
            );
        }
    }

    /// **Source and host rates differ in the field and matched in every early test**, which is how
    /// two separate places came to compare source frames against host samples and look right.
    #[test]
    fn a_source_rate_that_differs_from_the_host_is_handled_in_the_right_units() {
        for source_rate in [22_050.0f32, 44_100.0, 96_000.0] {
            let sample = struck(
                source_rate,
                &[(source_rate * 0.2) as usize],
                (source_rate) as usize,
            );
            for host_rate in [44_100.0f32, 48_000.0, 96_000.0] {
                let mut params = stretch_params(Reader::Stretch);
                params.layers[0].loop_mode = LoopMode::Forward;
                let mut engine = Engine::new(host_rate);
                engine.note_on(Note::midi(67, 1.0), [Some(&sample), None], &params);
                let mut peak = 0.0f32;
                for _ in 0..(host_rate as usize / 2) {
                    let value = engine.render([Some(&sample), None], &params);
                    assert!(
                        value.left.is_finite() && value.right.is_finite(),
                        "source {source_rate} into host {host_rate} produced a non-finite sample"
                    );
                    peak = peak.max(value.left.abs());
                }
                assert!(
                    peak > 0.001,
                    "source {source_rate} into host {host_rate} rendered silence"
                );
            }
        }
    }

    /// A marker inside a loop, crossed on the wrap, is the case the replay guard reasons about.
    /// It must stay finite and bounded whichever side of the seam the heads are on.
    #[test]
    fn a_marker_inside_a_loop_is_crossed_safely_on_every_wrap() {
        let rate = 48_000.0;
        let sample = struck(rate, &[6_000, 9_000], 24_000);
        for mode in [LoopMode::Forward, LoopMode::Alternate] {
            for key in [55u8, 60, 67, 72] {
                let mut params = stretch_params(Reader::Stretch);
                params.layers[0].loop_mode = mode;
                params.layers[0].start = 0.2;
                params.layers[0].end = 0.5;
                params.layers[0].loop_start = 0.2;
                params.layers[0].loop_end = 0.5;
                let rendered = render_layer_a(&sample, &params, key, 96_000);
                let peak = rendered
                    .iter()
                    .fold(0.0f32, |peak, v| peak.max(v.left.abs()).max(v.right.abs()));
                assert!(
                    peak.is_finite() && peak < 4.0,
                    "{mode:?} at key {key} reached {peak}"
                );
                assert!(
                    rendered.iter().any(|v| v.left.abs() > 0.001),
                    "{mode:?} at key {key} rendered silence"
                );
            }
        }
    }

    /// **A note plays from Start, and a loop begins only where it begins.** The owner's report
    /// (2026-09-14): a guitar string looped for sustain lost its attack. Every read was wrapped into the
    /// loop — including the first pass, before the playhead had reached it — and a crossfade that
    /// borrowed room moved that loop further in. The oracle is the same note played one-shot: until the
    /// playhead reaches the loop, a looping note must render exactly what a one-shot note renders.
    #[test]
    fn a_looping_note_plays_what_precedes_the_loop_exactly_as_a_one_shot_does() {
        let rate = 48_000.0;
        // **A loud tone, so the converter cannot hide a difference.** Clean keeps a 16-bit floor, and
        // over the usual 0.05 tone a latch with no margin — whose crossing blends reads that differ
        // only in a kernel's tail — changed nothing above that floor and passed.
        let sample = struck_over(rate, &[1_000, 6_000, 14_000], 36_000, 0.5);
        let last = 35_999.0f32;
        let (loop_start, loop_end) = (0.3f32, 0.8f32);
        // **A semitone down, so every read falls between frames.** At unity each read lands on a whole
        // frame and the kernel passes that frame through with every other tap weighted zero, so a tap
        // reading the loop's far end instead of the recording changed nothing — an entry that switched
        // every tap at Loop start passed this test until it was moved off the frame grid.
        let key = 59u8;
        let ratio = 2.0f32.powf(-1.0 / 12.0);
        let bits = |one: &[Stereo], other: &[Stereo]| {
            one.iter().zip(other).position(|(a, b)| {
                a.left.to_bits() != b.left.to_bits() || a.right.to_bits() != b.right.to_bits()
            })
        };
        for reader in [Reader::Repitch, Reader::Stretch] {
            // Reversed too: a reversed note plays from End, and its loop begins at Loop end.
            for reverse in [false, true] {
                let mut one_shot = stretch_params(reader);
                one_shot.layers[0].loop_start = loop_start;
                one_shot.layers[0].loop_end = loop_end;
                one_shot.layers[0].reverse = reverse;
                let reference = render_layer_a(&sample, &one_shot, key, 32_000);
                for (mode, fade_s) in [
                    (LoopMode::Forward, 0.0f32),
                    (LoopMode::Forward, 0.05),
                    (LoopMode::Alternate, 0.0),
                ] {
                    let fade = fade_s * rate;
                    // **Repitch is compared through the entry**, up to where its kernel first reaches
                    // the loop's far end or the fade there: inside the loop by more than a read
                    // reaches, a looped read and a plain one fetch the same frames, so entering a loop
                    // whose ends do not meet makes no click — a code review's finding against the
                    // first repair, which switched every tap on entry. Stretch is compared up to the
                    // loop, because its splice guard judges a looping playhead by the loop's length.
                    // Frames from where the note starts; Repitch covers them at the pitch ratio, and
                    // Stretch's playhead at a frame a sample whatever the pitch.
                    let (far, speed) = match (reader, reverse) {
                        (Reader::Repitch, false) => (loop_end * last - fade, ratio),
                        (Reader::Repitch, true) => (last - (loop_start * last + fade), ratio),
                        (_, false) => (loop_start * last, 1.0),
                        (_, true) => (last - loop_end * last, 1.0),
                    };
                    let untouched = ((far - SINC_TAPS as f32) / speed) as usize;
                    let mut looped = one_shot;
                    looped.layers[0].loop_mode = mode;
                    looped.layers[0].loop_crossfade_s = fade_s;
                    let rendered = render_layer_a(&sample, &looped, key, 32_000);
                    let departed = bits(&rendered[..untouched], &reference[..untouched]);
                    assert!(
                        departed.is_none(),
                        "{reader:?} {} in {mode:?} with a {fade_s} s fade departed from the \
                         one-shot note at sample {} of {untouched}",
                        if reverse { "reversed" } else { "forward" },
                        departed.unwrap_or(0)
                    );
                }
            }
        }

        // **A held Stretch playhead before the loop holds at Start** — or at End, reversed — a code
        // review's finding: a zero Scan fell through to the fold and jumped into the loop. In both
        // modes, and across a loop point moved under the note, which goes through the region remap; it
        // never reaches the loop, so it renders exactly what the held one-shot note does.
        let held = |mode: LoopMode, reverse: bool| {
            let mut params = stretch_params(Reader::Stretch);
            params.layers[0].loop_mode = mode;
            params.layers[0].reverse = reverse;
            params.layers[0].stretch_travel = 0.0;
            params.layers[0].loop_start = loop_start;
            params.layers[0].loop_end = loop_end;
            let mut engine = engine_without_routes();
            engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
            (0..12_000)
                .map(|n| {
                    if n == 6_000 {
                        params.layers[0].loop_end = 0.75;
                    }
                    engine.render([Some(&sample), None], &params)
                })
                .collect::<Vec<_>>()
        };
        for reverse in [false, true] {
            let reference = held(LoopMode::Off, reverse);
            for mode in [LoopMode::Forward, LoopMode::Alternate] {
                let departed = bits(&held(mode, reverse), &reference);
                assert!(
                    departed.is_none(),
                    "a held {} Stretch playhead in {mode:?} left where it started at sample {}",
                    if reverse { "reversed" } else { "forward" },
                    departed.unwrap_or(0)
                );
            }
        }

        // **A whole-region loop with a fade starts unlatched, by design** — a code review asked that it
        // be stated and held. The fade moves the return point to Start plus the fade, so a note plays
        // its fade-in material from Start as recorded, and, reversed, its fade-out zone from End. Only a
        // whole-region loop with no fade is reached on its first sample, which is what keeps every
        // factory recipe rendering as it did.
        for reader in [Reader::Repitch, Reader::Stretch] {
            for reverse in [false, true] {
                let mut one_shot = stretch_params(reader);
                one_shot.layers[0].reverse = reverse;
                one_shot.layers[0].loop_start = 0.0;
                one_shot.layers[0].loop_end = 1.0;
                let with_fade = |fade_s: f32| {
                    let mut params = one_shot;
                    params.layers[0].loop_mode = LoopMode::Forward;
                    params.layers[0].loop_crossfade_s = fade_s;
                    params
                };
                let latched_at_note_on = |params: &Params| {
                    let mut engine = engine_without_routes();
                    engine.note_on(Note::midi(key, 1.0), [Some(&sample), None], params);
                    engine
                        .voices
                        .iter()
                        .find(|v| v.active)
                        .expect("sounding")
                        .layers[0]
                        .looping
                };
                let label = format!(
                    "{reader:?} {}",
                    if reverse { "reversed" } else { "forward" }
                );
                assert!(
                    latched_at_note_on(&with_fade(0.0)),
                    "{label}: a whole-region loop with no fade was not reached on its first sample"
                );
                assert!(
                    !latched_at_note_on(&with_fade(0.05)),
                    "{label}: a whole-region loop with a fade was reached before the note had played \
                     its fade"
                );
                // Repitch through the entry, up to the fade at the loop's far side; Stretch up to where
                // its kernel reaches the loop, at the fade's end.
                let fade = 0.05 * rate;
                let (far, speed) = match reader {
                    Reader::Repitch => (last - fade, ratio),
                    _ => (fade, 1.0),
                };
                let untouched = ((far - SINC_TAPS as f32) / speed) as usize;
                let reference = render_layer_a(&sample, &one_shot, key, untouched);
                let rendered = render_layer_a(&sample, &with_fade(0.05), key, untouched);
                let departed = bits(&rendered, &reference);
                assert!(
                    departed.is_none(),
                    "{label}: a whole-region loop with a fade departed from the one-shot note at \
                     sample {} of {untouched}",
                    departed.unwrap_or(0)
                );
            }
        }

        // **Grain never reads the loop**, so where the loop points sit changes nothing it plays.
        let mut whole = stretch_params(Reader::Grain);
        whole.layers[0].loop_mode = LoopMode::Forward;
        let mut inside = whole;
        inside.layers[0].loop_start = loop_start;
        inside.layers[0].loop_end = loop_end;
        let one = render_layer_a(&sample, &whole, 60, 24_000);
        let other = render_layer_a(&sample, &inside, 60, 24_000);
        assert!(
            one.iter()
                .zip(&other)
                .all(|(a, b)| a.left.to_bits() == b.left.to_bits()
                    && a.right.to_bits() == b.right.to_bits()),
            "Grain played differently because of where the loop points sit"
        );
    }

    /// **A head anchored before Loop start after a wrap reads the loop, not what precedes it.** A code
    /// review's finding (2026-09-14): deciding "before the loop" from a position cannot tell a first
    /// pass from a Stretch head the alignment search placed a few milliseconds early after the playhead
    /// wrapped, and such a head read the recording before Loop start.
    ///
    /// **The oracle is where the heads are, every sample, once the first pass is over.** A head that
    /// has latched is folded into the loop as it advances, so from then on none is ever left outside
    /// it; a head outside the loop after the first pass is one reading the recording beyond it. The
    /// level check alone was blind: a head is anchored with its window closed, and it walks back into
    /// the loop before that window opens far enough to be heard over a threshold — a version measuring
    /// only the level passed with the latch not inherited.
    ///
    /// **Built to anchor heads outside the loop often.** The loop is 1 000 frames, so the playhead
    /// wraps more often than a head is re-anchored. It holds a 100 Hz tone, whose 480-frame period
    /// gives the search phase matches spread either side of the playhead. The notes are transposed
    /// in both directions: at unity the heads sit exactly on the playhead and never leave the loop.
    #[test]
    fn a_head_anchored_before_loop_start_after_a_wrap_reads_the_loop() {
        let rate = 48_000.0f32;
        let len = 36_000usize;
        let (from, to) = (10_800usize, 11_800usize);
        let frames: Vec<[f32; 2]> = (0..len)
            .map(|n| {
                let time = n as f32 / rate;
                let value = if (from..=to).contains(&n) {
                    (TAU * 100.0 * time).sin() * 0.05
                } else {
                    (TAU * 220.0 * time).sin() * 0.9
                };
                [value, value]
            })
            .collect();
        let sample = Sample::new(frames, rate).unwrap();
        let last = (len - 1) as f32;
        let window = (STRETCH_WINDOW_S * 48_000.0) as usize;
        for reverse in [false, true] {
            for key in [48u8, 55, 67, 72] {
                let mut params = looping(Reader::Stretch, 0.0);
                params.layers[0].reverse = reverse;
                params.layers[0].loop_start = from as f32 / last;
                params.layers[0].loop_end = to as f32 / last;
                let region = Region::looped(&sample, &params.layers[0]);
                let direction = if reverse { "reversed" } else { "forward" };
                // The playhead travels a frame a sample from where the note starts: the first pass
                // is over once it has crossed the loop, and every head anchored before that has had
                // a window to finish, with a search width to spare.
                let first_pass = if reverse { len - from } else { to } + window + 480;
                let mut engine = engine_without_routes();
                engine.note_on(Note::midi(key, 1.0), [Some(&sample), None], &params);
                let (mut opening, mut peak, mut outside) = (0.0f32, 0.0f32, 0usize);
                for n in 0..60_000 {
                    let out = engine.render([Some(&sample), None], &params);
                    if n < 5_000 {
                        opening = opening.max(out.left.abs());
                    }
                    if n < first_pass {
                        continue;
                    }
                    peak = peak.max(out.left.abs());
                    let voice = engine.voices.iter().find(|v| v.active).expect("sounding");
                    outside += voice.layers[0]
                        .ola
                        .iter()
                        .filter(|head| {
                            head.position < region.loop_start
                                || head.position >= region.loop_limit()
                        })
                        .count();
                }
                assert!(
                    opening > 0.3,
                    "{direction} at key {key}: the first pass did not play the loud opening \
                     ({opening})"
                );
                assert_eq!(
                    outside, 0,
                    "{direction} at key {key}: a head sat outside the loop after the first pass \
                     ({outside} head-samples), reading the recording beyond it"
                );
                assert!(
                    peak < 0.1,
                    "{direction} at key {key}: after the first pass the layer reached {peak}, past \
                     the loop's own level"
                );
            }
        }
    }

    /// **A loop too short to hold a read is entered without a step.** A code review's finding against
    /// the latch (2026-09-14): its margin is capped to fit the loop, so on a loop shorter than twice a
    /// read's reach — or with a fade leaving no room — no place inside lets a looped read and a plain
    /// one agree, and latching switched every read in one sample. The reader now crosses over
    /// [`LOOP_ENTRY_S`] from the recording, carried on, to the loop.
    ///
    /// **Built so a switch has nothing to hide behind.** A 14-frame loop of +0.5, frames 8 004 to
    /// 8 017, sits between −0.5 on both sides. It is read through Character's hold path at a 16-frame
    /// cell, so every read reaches past both loop ends, and the recording changes by at most one cell's
    /// slope a sample. The cell instant at 8 016 falls inside the loop and the one at 8 000 outside it,
    /// which does two things:
    /// - at the latch the one-shot is far from the loop's level, so a switch in one sample is a step;
    /// - a plain read taken at the reader's wrapped position jumps at every wrap, so a crossing must
    ///   carry the recording on rather than read it where the loop has put the reader.
    ///
    /// **The bound is the recording's, not the one-shot's.** Once the looping note's searches are
    /// scored in the loop, a Stretch note's heads follow different paths from its one-shot's, so the
    /// one-shot is no oracle for Stretch. Every legitimate read of this recording is bound by the
    /// recording itself:
    /// - a full-scale change across the narrowest interval the clock allows, at the reader's speed;
    /// - the jump a wandering instant makes where the cell changes;
    /// - a transition spread over 32 samples.
    ///
    /// On the steady clock that is exactly what the one-shot measured, and a switch in one sample is
    /// several times it. Repitch and Stretch, forward and reversed, with no fade and with the fade at
    /// its half-loop ceiling, a semitone down. The clock is steady, which places those instants exactly,
    /// and wandering, which must pass as well. A code review found the first, Repitch-only version
    /// could not see a Stretch head that skipped its crossing.
    ///
    /// **Onsets are kept away from the loop, not absent.** The detector finds one in this source, about
    /// 1 700 frames before the loop. A splice re-lays both heads onto the playhead, and there every head —
    /// within a search width and a window's drift of the playhead, some 700 frames — reads the same
    /// level, so the splice cannot step. A reversed playhead wraps before reaching it at all.
    #[test]
    fn a_loop_too_short_for_a_read_is_entered_without_a_step() {
        let rate = 48_000.0f32;
        let len = 24_000usize;
        let (from, to) = (8_004usize, 8_017usize);
        let frames: Vec<[f32; 2]> = (0..len)
            .map(|n| {
                let value = if (from..=to).contains(&n) { 0.5 } else { -0.5 };
                [value, value]
            })
            .collect();
        let sample = Sample::new(frames, rate).unwrap();
        assert!(
            sample
                .transients()
                .iter()
                .all(|&onset| (onset as usize).abs_diff(from) > 1_000
                    && (onset as usize).abs_diff(to) > 1_000),
            "an onset lies within 1 000 frames of the loop, where a splice could land heads on its \
             edge: {:?}",
            sample.transients()
        );
        let last = (len - 1) as f32;
        let grid = character_rate_divisor(0.8);
        let speed = 2.0f32.powf(-1.0 / 12.0);
        for (reader, reverse, fade_s, jitter) in [Reader::Repitch, Reader::Stretch]
            .into_iter()
            .flat_map(|reader| [false, true].map(|reverse| (reader, reverse)))
            .flat_map(|(reader, reverse)| {
                [0.0f32, 6.0 / rate].map(|fade_s| (reader, reverse, fade_s))
            })
            .flat_map(|(reader, reverse, fade_s)| {
                [0.0f32, 0.5].map(|jitter| (reader, reverse, fade_s, jitter))
            })
        {
            let mut looped = stretch_params(reader);
            looped.character = Character {
                rate: 0.8,
                converter: 0.0,
                reconstruction: 0.0,
                input: 0.0,
                converter_type: ConverterType::Linear,
                jitter,
            };
            looped.attack_s = 0.001;
            looped.sustain = 1.0;
            looped.layers[0].reverse = reverse;
            looped.layers[0].loop_start = from as f32 / last;
            looped.layers[0].loop_end = to as f32 / last;
            looped.layers[0].loop_mode = LoopMode::Forward;
            looped.layers[0].loop_crossfade_s = fade_s;
            let case = format!(
                "{reader:?} {}, jitter {jitter}, a {fade_s} s fade",
                if reverse { "reversed" } else { "forward" }
            );
            // Past the attack. Either reader reaches the loop near sample 8 000 forward and 16 000
            // reversed.
            let entered = render_layer_a(&sample, &looped, 59, 20_000);
            let entered = &entered[500..];
            let (low, high) = entered.iter().fold((f32::MAX, f32::MIN), |(low, high), v| {
                (low.min(v.left), high.max(v.left))
            });
            let wander = jitter * JITTER_CELLS * grid;
            let interval = grid - wander;
            let allowed = (high - low) * ((speed + wander * 0.5) / interval + 1.0 / 32.0);
            let step = largest_step(entered);
            assert!(
                step <= allowed,
                "{case}: entering the loop stepped by {step}, past what any read of the recording \
                 can make and a 32-sample transition ({allowed})"
            );
            // And it did enter: it ends at the loop's level, not the recording's.
            let end = entered.last().expect("rendered").left;
            assert!(end > 0.0, "{case} never reached the loop: it ends at {end}");
        }
    }

    /// **A reversed playhead entering an Alternate loop from above keeps descending.** Revision 9 took
    /// the out-and-back distance at entry on the falling half for a reversed note, where a step down
    /// raises the playhead, so it turned back up the moment it entered — the first code review asked
    /// about exactly that entry. Taken on the rising half, a step moves the playhead the way the step
    /// goes, until the turn at Loop start.
    #[test]
    fn a_reversed_playhead_entering_an_alternate_loop_keeps_descending() {
        let sample = struck(48_000.0, &[], 36_000);
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Alternate;
        params.layers[0].reverse = true;
        params.layers[0].loop_start = 0.3;
        params.layers[0].loop_end = 0.8;
        let mut engine = engine_without_routes();
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        let (mut previous, mut entered_at) = (f32::INFINITY, None);
        // A frame a sample from End reaches Loop end at sample 7 200 and the turn at Loop start at
        // 25 199; this stops short of the turn.
        for n in 0..24_000 {
            engine.render([Some(&sample), None], &params);
            let layer = &engine
                .voices
                .iter()
                .find(|v| v.active)
                .expect("sounding")
                .layers[0];
            if layer.looping && entered_at.is_none() {
                entered_at = Some(n);
            }
            assert!(
                layer.travel <= previous,
                "the reversed playhead rose from {previous} to {} at sample {n} (it reached the \
                 loop at sample {entered_at:?})",
                layer.travel
            );
            previous = layer.travel;
        }
        assert!(entered_at.is_some(), "the playhead never reached the loop");
    }

    /// **An attack passed on the way into a loop shorter than one step is still spliced.** A code
    /// review's finding (2026-09-14): a playhead crossing a marker and a whole loop in one step
    /// entered at the loop's near end and walked only from there, so the attack before Loop start was
    /// never reported. A 768 kHz source under an 8 kHz host travels 192 frames a sample at double Scan,
    /// enough to cross the marker and a 60-frame loop together.
    #[test]
    fn an_attack_crossed_on_the_way_into_a_short_loop_is_still_spliced() {
        let source_rate = 768_000.0f32;
        let len = 768_000usize;
        let sample = struck(source_rate, &[400_000], len);
        let marker = *sample
            .transients()
            .iter()
            .find(|&&onset| onset > 390_000)
            .expect("the strike is found") as f32;
        let last = (len - 1) as f32;
        let step = 192.0f32;
        // Fifty steps land the playhead 20 frames short of the marker, so the fifty-first crosses the
        // marker, Loop start and Loop end together.
        let start = marker - 20.0 - step * 50.0;
        let mut params = stretch_params(Reader::Stretch);
        params.layers[0].loop_mode = LoopMode::Forward;
        params.layers[0].stretch_travel = 2.0;
        params.layers[0].start = start / last;
        params.layers[0].loop_start = (marker + 40.0) / last;
        params.layers[0].loop_end = (marker + 100.0) / last;
        let mut engine = Engine::new(8_000.0);
        engine.note_on(Note::midi(60, 1.0), [Some(&sample), None], &params);
        for _ in 0..60 {
            engine.render([Some(&sample), None], &params);
        }
        let voice = engine.voices.iter().find(|v| v.active).expect("sounding");
        assert!(
            voice.layers[0].splices >= 1,
            "the attack crossed on the way into the loop was never spliced"
        );
    }

    /// A one-frame region is valid at every reader, and Stretch has its own path through it.
    #[test]
    fn a_one_frame_region_is_valid_in_stretch() {
        let sample = Sample::new(vec![[0.25, -0.25]; 8], 48_000.0).unwrap();
        let mut params = stretch_params(Reader::Stretch);
        // Equal endpoints, because `Region::new` normalises a reversed pair back to the full
        // region — the first version of this test asked for 1.0/0.0 and quietly exercised
        // everything except the case it names.
        params.layers[0].start = 0.5;
        params.layers[0].end = 0.5;
        params.layers[0].loop_start = 0.5;
        params.layers[0].loop_end = 0.5;
        params.layers[0].loop_mode = LoopMode::Alternate;
        let mut engine = Engine::default();
        engine.note_on(Note::midi(127, 1.0), [Some(&sample), None], &params);
        for _ in 0..8_192 {
            let value = engine.render([Some(&sample), None], &params);
            assert!(value.left.is_finite() && value.right.is_finite());
        }
    }

    /// The alignment search prefers the smaller shift when two offsets fit equally well, which is
    /// what makes the identity at unity exact rather than merely spectrally right.
    #[test]
    fn the_alignment_search_breaks_ties_toward_zero() {
        let rate = 48_000.0;
        // Exactly periodic, so offsets of zero and one period correlate identically.
        let period = 240.0f32;
        let frames: Vec<[f32; 2]> = (0..48_000)
            .map(|n| {
                let v = (TAU * n as f32 / period).sin() * 0.5;
                [v, v]
            })
            .collect();
        let sample = Sample::new(frames, rate).unwrap();
        let params = stretch_params(Reader::Stretch);
        let region = Region::new(sample.len(), &params.layers[0]);
        let offset = align_offset(
            &sample,
            region,
            LoopMode::Forward,
            LoopMode::Forward,
            10_000.0,
            10_000.0,
            1.0,
            (ALIGN_SPAN_S * rate).min(2_000.0),
            (ALIGN_SEARCH_S * rate) as i32,
        );
        assert_eq!(
            offset, 0.0,
            "a tie on a periodic source resolved to {offset} instead of zero"
        );
    }
}
