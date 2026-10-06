//! What `mxm-creative-sampler` can modulate, and with what.
//!
//! `plans/plan-sampler-modulation-and-converter.md` §1, under
//! `plans/plan-modulation-routing.md` §7.4. The shared machinery is [`mxm_modulation`]; this module
//! is the instrument's own declaration — its **source list**, its **target list**, and each target's
//! **full scale**. The plugin crate owns the compiled Init patch that chooses routes from this grid.
//!
//! # This is the collection's first polyphonic conversion
//!
//! The pilot is monophonic, so one source frame is the whole instrument and
//! `plan-modulation-routing.md` §4.1's scope rule costs it nothing — *"a monosynth's two frames
//! collapse into one."* Here they do not. §4.1 settles the model: **a frame is per scope, one global
//! and one per voice**, and *"a voice-scoped target may read either frame. A global source is simply
//! the same value in every voice, which is what a shared LFO already is."*
//!
//! **This takes that second option**, and it is what keeps the shared crate untouched: the engine
//! evaluates the two LFOs once per sample and each voice publishes the same value into its own
//! frame. The alternative — teaching the summing laws to read a source from the frame its scope
//! names — is a change to a crate the pilot already ships on, for a saving of at most two stores per
//! voice per sample, and only for sources something actually routes.
//!
//! # The performance sources and the amplitude law are the collection's standard
//!
//! Key, Velocity, Wheel, Pressure, Bend and Random mean what they mean on every instrument, and a
//! route this sampler did not wire before it had routing reaches what it reaches on every instrument
//! ([`mxm_modulation::standard`]; `plans/plan-modulation-standard.md`). Velocity is `v − 1`, so a
//! route from it does nothing at the hardest note — and **Amplitude is the standard factor**,
//! `1 + clamp(Σ, ±1)`, where it was a product of `1 + a·(s − 1)` factors. For Velocity the two are
//! the same number, which is why the velocity-sensitivity knob this route replaced is still exactly
//! reachable.

use mxm_modulation::standard::{self, AMPLITUDE_SUM_BOUND, Law, Offer, Performance, reach};
use mxm_modulation::{Compacted, SourceFrame};

use crate::{FILTER_ENV_OCTAVES, PRESSURE_OCTAVES};

/// Every source this instrument can route, in **declared evaluation order**.
///
/// The order is load-bearing rather than cosmetic: a route whose source comes earlier reads *this*
/// sample's value, and one whose source comes later reads *last* sample's. That is the unit delay,
/// and it is what makes a player-made cycle finite.
///
/// **The two layers' audio is last**, so every route from it is a **backward** route — one sample
/// late, and a comb at audio rate. `plan-modulation-routing.md` §6.4 asks for that to be declared
/// rather than discovered, and this is the declaration.
pub mod source {
    /// LFO 1, free-running. Bipolar. Global.
    pub const LFO1: usize = 0;
    /// LFO 2, free-running. Bipolar. Global.
    pub const LFO2: usize = 1;
    /// The voice's amplitude envelope. Unipolar. Voice-scoped.
    pub const ENVELOPE: usize = 2;
    /// The velocity of the press that last triggered the envelope, `v − 1`
    /// (`standard::velocity`): zero at the hardest note. Voice-scoped — a Legato press that
    /// re-pitches without retriggering keeps the phrase's.
    pub const VELOCITY: usize = 3;
    /// The played note **after glide** as a signed distance from middle C over five octaves
    /// (`standard::key`). Voice-scoped.
    pub const KEY: usize = 4;
    /// The mod wheel, CC 1. Unipolar. Per channel, projected into the voice frame.
    pub const WHEEL: usize = 5;
    /// Note pressure. Unipolar. Voice-scoped — this instrument reads `PolyPressure`.
    pub const PRESSURE: usize = 6;
    /// The pitch bender, signed. Per channel, projected into the voice frame.
    pub const BEND: usize = 7;
    /// One draw per note, `2u − 1` (`standard::random`), held for the life of the voice.
    /// Voice-scoped.
    pub const RANDOM: usize = 8;
    /// Layer A's reader output, mono, **before that layer's level and pan**. Audio rate.
    pub const LAYER_A: usize = 9;
    /// Layer B's reader output, mono, **before that layer's level and pan**. Audio rate.
    pub const LAYER_B: usize = 10;
}

/// How many sources the instrument declares.
pub const SOURCES: usize = 11;

/// Their names, in source order, for the interface and for accessibility.
pub const SOURCE_NAMES: [&str; SOURCES] = [
    "LFO 1", "LFO 2", "Envelope", "Velocity", "Key", "Wheel", "Pressure", "Bend", "Random",
    "Layer A", "Layer B",
];

/// Which sources are **global** — evaluated once per sample by the engine rather than per voice.
///
/// Everything else is voice-scoped or is a per-channel value projected into the voice frame, which
/// §4.1 permits because a voice knows its channel.
pub const GLOBAL: [bool; SOURCES] = {
    let mut global = [false; SOURCES];
    global[source::LFO1] = true;
    global[source::LFO2] = true;
    global
};

/// Every target this instrument declares.
///
/// **Deliberate rather than maximal** — the places worth modulating on a sampler, not every
/// continuous parameter. A target nobody would reach for costs a full column of permanent
/// parameters, so the list is chosen once and recorded rather than swept.
///
/// **Scan speed is two targets, not one**, because the layers are two and everything else about a
/// layer already is — and because the Travel LFO this replaces pushed them in *opposite* directions
/// with different reaches, which one target cannot express.
pub mod target {
    /// The voice's amplifier. **Multiplicative on the envelope**, not additive — see [`FULL_SCALE`].
    pub const AMPLITUDE: usize = 0;
    /// Filter cutoff, summed in octaves.
    pub const CUTOFF: usize = 1;
    /// Playback pitch, summed in semitones, on both layers.
    pub const PITCH: usize = 2;
    /// Layer A's scan speed, in travel units.
    pub const SCAN_A: usize = 3;
    /// Layer B's scan speed, in travel units.
    pub const SCAN_B: usize = 4;
}

/// How many targets the instrument declares.
pub const TARGETS: usize = 5;

/// Their names, in target order.
pub const TARGET_NAMES: [&str; TARGETS] = [
    "Amplitude",
    "Cutoff",
    "Pitch",
    "Scan speed A",
    "Scan speed B",
];

/// The key source's unit, in semitones: a voice publishes `(note − 60) / 60`.
pub const KEY_UNIT_SEMITONES: f32 = 60.0;

/// Which of the standard's performance sources each source is — `None` for the LFOs, the envelope
/// and the layers' audio, which keep their own meaning.
pub const PERFORMANCE: [Option<Performance>; SOURCES] = [
    None,
    None,
    None,
    Some(Performance::Velocity),
    Some(Performance::Key),
    Some(Performance::Wheel),
    Some(Performance::Pressure),
    Some(Performance::Bend),
    Some(Performance::Random),
    None,
    None,
];

/// Each target's law, for the standard's offer: the amplitude factor and four sums.
pub const LAW: [Law; TARGETS] = [Law::Factor, Law::Sum, Law::Sum, Law::Sum, Law::Sum];

/// The paths this sampler **wired before it had routing**, which keep the reach they had: the
/// velocity-sensitivity knob, the filter envelope and pressure on the cutoff, and the Travel LFO on
/// the two scan speeds.
pub const MACHINE: [(usize, usize); 5] = [
    (target::AMPLITUDE, source::VELOCITY),
    (target::CUTOFF, source::ENVELOPE),
    (target::CUTOFF, source::PRESSURE),
    (target::SCAN_A, source::LFO1),
    (target::SCAN_B, source::LFO1),
];

/// Whether this pair is one of [`MACHINE`].
#[must_use]
pub const fn machine(target: usize, source: usize) -> bool {
    let mut i = 0;
    while i < MACHINE.len() {
        if MACHINE[i].0 == target && MACHINE[i].1 == source {
            return true;
        }
        i += 1;
    }
    false
}

/// Whether and how a pair is offered — `standard::offer`. Every target here sums or scales, so
/// every pair is offered on both halves.
#[must_use]
pub const fn offer(target: usize, source: usize) -> Offer {
    standard::offer(LAW[target], PERFORMANCE[source], machine(target, source))
}

/// The amplitude sum's bound — the standard's, silence to double.
pub const AMPLITUDE_BOUND: f32 = AMPLITUDE_SUM_BOUND;

/// What LFO 1 reached on each layer's scan speed before it was a route.
///
/// `Engine::render` added `+0.6` of the Travel LFO to layer A and subtracted `0.47` from layer B, so
/// the two drifted apart rather than sweeping together. **These are those two numbers**, and keeping
/// them as the route's *scale* rather than as its amount is what makes the old sound reachable at
/// the old knob position — `plan-modulation-routing.md` §5's conservative form.
pub const TRAVEL_A_REACH: f32 = 0.6;
/// See [`TRAVEL_A_REACH`]. Negative, because the layers move **apart**.
pub const TRAVEL_B_REACH: f32 = -0.47;

/// How far a Key route reaches on the cutoff: one octave of cutoff per octave of keyboard — the
/// standard's.
pub const KEY_OCTAVES: f32 = standard::key_scale(reach::KEY_OCTAVES_PER_OCTAVE, KEY_UNIT_SEMITONES);

/// How far a pitch route reaches, in semitones, at an amount of one — the standard's octave.
pub const PITCH_SEMITONES: f32 = reach::PITCH_SEMITONES;

/// Each target's full scale, in the target's own domain, at an amount of one — **per route, not per
/// target**.
///
/// A route is `amount × source × full_scale`, summed in that domain and converted once, which is
/// what these voices already did with the constant written into each expression.
///
/// # Why the table has a column per source
///
/// Because the machine's own wiring is not uniform, and `plan-modulation-routing.md` §5 takes the
/// conservative form: **a route the instrument itself wires keeps the reach it always had.** The
/// filter envelope reached four octaves and pressure two, from the same cutoff; LFO 1 reached
/// `0.6` on layer A's scan and `-0.47` on layer B's. Declaring one scale per target would rescale
/// all of them, which moves every stored value's meaning and every render for no musical gain.
///
/// **Every other route takes the collection's standard reach** (`standard::reach`): an octave of
/// pitch and twelve semitones per octave of keyboard from Key — it was 2.4 — four octaves of cutoff,
/// the whole amplitude factor, a scan speed's own unit, and a fifth of a linear reach per octave
/// from Key.
pub const FULL_SCALE: [[f32; SOURCES]; TARGETS] = {
    let key_linear = reach::KEY_LINEAR_FRACTION_PER_OCTAVE;
    // A fraction of the amplifier's own level: one route can silence it or double it — and one is
    // also exactly what the velocity-sensitivity knob was.
    let mut amplitude = [reach::AMPLITUDE; SOURCES];
    amplitude[source::KEY] = standard::key_scale(reach::AMPLITUDE * key_linear, KEY_UNIT_SEMITONES);
    // Octaves. Four is the standard's reach, which is what the filter envelope already had.
    let mut cutoff = [reach::OCTAVES; SOURCES];
    cutoff[source::ENVELOPE] = FILTER_ENV_OCTAVES;
    // Two, because that is what pressure already reached — a fixed route with no depth control.
    cutoff[source::PRESSURE] = PRESSURE_OCTAVES;
    cutoff[source::KEY] = KEY_OCTAVES;
    let mut pitch = [PITCH_SEMITONES; SOURCES];
    pitch[source::KEY] =
        standard::key_scale(reach::KEY_PITCH_SEMITONES_PER_OCTAVE, KEY_UNIT_SEMITONES);
    // Travel units. LFO 1 keeps the two reaches the hard-wired Travel LFO had, including its sign.
    let key_scan = standard::key_scale(reach::CONTROL * key_linear, KEY_UNIT_SEMITONES);
    let mut scan_a = [reach::CONTROL; SOURCES];
    scan_a[source::LFO1] = TRAVEL_A_REACH;
    scan_a[source::KEY] = key_scan;
    let mut scan_b = [reach::CONTROL; SOURCES];
    scan_b[source::LFO1] = TRAVEL_B_REACH;
    scan_b[source::KEY] = key_scan;
    [amplitude, cutoff, pitch, scan_a, scan_b]
};

/// Which sources are live into which targets, and how much of each.
///
/// **Presence is what the DSP reads.** An absent route contributes nothing whatever its amount
/// holds, which is what makes removing a source one parameter write and re-adding it restore the
/// depth the player last set.
#[derive(Debug, Clone, Copy)]
pub struct Routing {
    /// Per target, per source: whether that route exists.
    pub present: [[bool; SOURCES]; TARGETS],
    /// Per target, per source: how much, signed, as a fraction of that route's full scale.
    pub amounts: [[f32; SOURCES]; TARGETS],
    /// The live pairs, `(target, source)`, compacted by [`Routing::compact`].
    ///
    /// **Topology is discrete and changes only on a parameter event**, so this is built once per
    /// processing interval rather than walked per sample. On the pilot, walking the whole grid every
    /// sample cost more than everything else the routing added put together.
    live: [(u8, u8); TARGETS * SOURCES],
    live_len: usize,
    /// Which sources some live route actually reads.
    ///
    /// **A source nothing reads is not published.** Publishing all eleven when the init patch wires
    /// three costs a finite check, a clamp and two stores each for a value no sum will look at.
    needed: [bool; SOURCES],
}

impl Default for Routing {
    fn default() -> Self {
        Self::new()
    }
}

impl Routing {
    /// Nothing routed anywhere.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            present: [[false; SOURCES]; TARGETS],
            amounts: [[0.0; SOURCES]; TARGETS],
            live: [(0, 0); TARGETS * SOURCES],
            live_len: 0,
            needed: [false; SOURCES],
        }
    }

    /// Rebuilds [`Routing::live`] and [`Routing::needed`] from [`Routing::present`].
    ///
    /// **Once per interval, never per sample**, and every caller that changes `present` owes this
    /// before the next render.
    pub fn compact(&mut self) {
        self.live_len = 0;
        self.needed = [false; SOURCES];
        for (t, target) in self.present.iter().enumerate() {
            for (s, &on) in target.iter().enumerate() {
                if on {
                    self.live[self.live_len] = (t as u8, s as u8);
                    self.live_len += 1;
                    self.needed[s] = true;
                }
            }
        }
    }

    /// The live pairs, `(target, source)`, as compacted.
    #[inline]
    #[must_use]
    pub fn live(&self) -> &[(u8, u8)] {
        &self.live[..self.live_len]
    }

    /// Whether any route at all is live. The whole per-sample path is skipped when nothing is.
    #[inline]
    #[must_use]
    pub fn any(&self) -> bool {
        self.live_len > 0
    }

    /// Whether some live route reads this source.
    #[inline]
    #[must_use]
    pub fn needs(&self, source: usize) -> bool {
        self.needed[source]
    }
}

/// One voice's routing state: its own source frame, and one compacted list per target.
///
/// The frame is **per voice**, which is §4.1's rule and what stops a latched key from holding a
/// patch open: a voice frame ceases with its voice, so a decayed voice is inert whatever it last
/// published.
#[derive(Debug, Clone)]
pub struct Graph {
    frame: SourceFrame<SOURCES>,
    live: [Compacted<SOURCES>; TARGETS],
    /// Which sources the last topology had some live route read, so the next one can tell which
    /// have **just** become read. See [`Graph::set_topology`].
    needed: [bool; SOURCES],
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    #[must_use]
    pub fn new() -> Self {
        Self {
            frame: SourceFrame::default(),
            live: std::array::from_fn(|_| Compacted::default()),
            needed: [false; SOURCES],
        }
    }

    /// Rebuilds this voice's per-target lists. **Once per interval**, with [`Routing::compact`].
    ///
    /// **A source that becomes needed starts from silence.** Publication is gated on
    /// [`Routing::needs`], so while nothing read a source nothing published it, and its slot still
    /// holds whatever it held the last time something did — possibly from a different phrase. A
    /// **backward** route added to a sounding voice (either layer's audio, which publishes after
    /// every target has read) would then read that ancient value for exactly one sample, and how
    /// ancient would depend on how the host split its buffers. `mxm_modulation::SourceFrame::clear`
    /// makes the first read a deterministic zero instead; mxm-kit's
    /// `crates/mxm-modulation/AGENTS.md`, *A gated publication owes a `clear`*.
    pub fn set_topology(&mut self, routing: &Routing) {
        for (target, live) in self.live.iter_mut().enumerate() {
            live.build(&routing.present[target]);
        }
        for (source, before) in self.needed.iter_mut().enumerate() {
            let now = routing.needs(source);
            if now && !*before {
                self.frame.clear(source);
            }
            *before = now;
        }
    }

    /// Opens a sample. Every publication for this sample happens after this and before any read.
    #[inline]
    pub fn begin_sample(&mut self) {
        self.frame.begin_sample();
    }

    /// Publishes one source's value, bounded to unit magnitude by the frame itself.
    #[inline]
    pub fn write(&mut self, source: usize, value: f32) {
        self.frame.write(source, value);
    }

    /// This target's sum, in its own domain, with each route's own full scale applied last.
    #[inline]
    #[must_use]
    pub fn sum(&self, target: usize, routing: &Routing, bound: f32) -> f32 {
        mxm_modulation::sum_scaled(
            &self.frame,
            &self.live[target],
            &routing.amounts[target],
            &FULL_SCALE[target],
            bound,
        )
    }

    /// What this frame holds for one source, for the voice-slot test.
    #[cfg(test)]
    pub fn read_for_test(&self, source: usize) -> f32 {
        self.frame.read(source)
    }

    /// Forgets everything, so a reused voice slot cannot read a dead voice's values.
    pub fn reset(&mut self) {
        self.frame.reset();
    }
}
