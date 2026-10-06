//! The generated source bank: seventeen waveforms built from partials, not loaded from files.
//!
//! **One additive engine, seventeen spectral recipes.** Every source here is the same loop — sum
//! partials whose amplitudes move as the sample plays — differing only in what a recipe says
//! partial `n`'s amplitude is at position `t`.
//! mxm-kit's [`docs/oscillators/11-additive-resynthesis.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/oscillators/11-additive-resynthesis.md)
//! §11.1 is the argument for that shape: *"There is no 'sawtooth' in the code — a sawtooth is what
//! you get when the amplitudes happen to be `1/k`. Every spectral shape costs the same."*
//!
//! Two things follow, and both are why this is the right construction rather than merely a tidy
//! one:
//!
//! - **Band limiting is structural rather than filtered.** No *carrier* above [`BANDLIMIT_HZ`] is
//!   ever synthesised — the rule is *never emit that partial*, not *filter it afterwards and trust
//!   the filter*. That is not the whole claim, though, and the difference was worth catching: a
//!   moving amplitude is modulation, and modulation puts sidebands either side of what it moves, so
//!   a morph can in principle place energy above a limit its carriers respect. The morph is slow —
//!   one traverse takes half a second — so those sidebands sit within a few hertz of their
//!   carriers, and `nothing_lands_above_the_band_limit` measures the guard band instead of assuming
//!   it.
//! - **It is true spectral morphing.**
//!   mxm-kit's [`docs/oscillators/09-wavetable.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/oscillators/09-wavetable.md) is
//!   blunt that the obvious alternative is not: *"Amplitude crossfading is not spectral morphing.
//!   Mixing table A and table B at 50/50 gives you the sum of two spectra, not a spectrum halfway
//!   between them"*, and *"the null is worst where the tables are most similar."* Moving the
//!   partials' own amplitudes has no null to avoid and needs no phase-locking pass.
//!
//! **The morph is continuous, not stepped into wavetable frames** (owner, 2026-09-12), so the scan
//! position *is* a timbre control: Repitch plays through the sweep as the note evolves, Stretch
//! parks on one part of it or travels slowly, and Grain builds a cloud out of one colour and moves
//! it. That is the whole reason the bank exists rather than a rack of static single-cycle shapes.
//!
//! # Everything here loops, and it takes two rules
//!
//! **Every partial completes a whole number of cycles over the whole sample.** The saw this bank
//! grew out of loops because 142 whole cycles fit 52 105 frames; generalised, a harmonic partial
//! takes `n × 142` and an *inharmonic* one at ratio `r` takes `round(142 r)`. The ratio error that
//! rounding leaves is under 0.35% at the very bottom and far less higher up — inaudible as
//! detuning — and it buys a seamless loop for the metallic and noisy recipes, which would otherwise
//! click once a second like the un-rounded saw would have.
//!
//! Where a recipe needs a partial's *frequency* to move, it cannot: the cycle count is fixed for
//! the render. Those recipes instead carry two partial sets at once and crossfade their amplitudes
//! ([`Recipe::StretchedHarmonics`], [`Recipe::BellToTone`], [`Recipe::SidebandSweep`]). That is not
//! the table crossfade §09 warns about — two partial banks at *different frequencies* beat against
//! each other rather than cancelling, and the beating is the point.
//!
//! # The sweep goes out and back, and that is what closes the loop
//!
//! **Whole cycles make the carrier periodic; they say nothing about the envelope.** A spectrum that
//! runs 0 → 1 across the buffer leaves the last frame sounding nothing like the first, so a source
//! built that way clicks once a second on exactly the loop the frame and cycle counts were chosen
//! to protect — the morph would have undone the one guarantee it inherited. [`sweep`] therefore
//! takes the position out to the far end of the morph by the middle of the sample and back by the
//! end, so the two ends meet in spectrum as well as in phase.
//!
//! It costs nothing musically and arguably gains: a reader travelling through hears the sweep and
//! its return rather than falling off the end of it, and a reader parked anywhere still finds one
//! settled colour.

use std::f32::consts::TAU;

/// Frames in every generated source, and how many whole cycles of the fundamental they hold.
///
/// **Whole cycles, because the init patch loops it** — a loop that closes part way up a ramp clicks
/// once a second. **And a whole MIDI note, because its name tunes it:** Root holds cents, but the
/// `C3` a generated name ends in says a whole note, so a source between notes would arrive detuned
/// by what the name cannot say. 142 cycles in
/// 52 105 frames is 130.81278 Hz — C3, [`ROOT`], to 0.0000 cents. Both constraints hold at once,
/// which is why these are the numbers and not round ones.
pub const FRAMES: usize = 52_105;
pub const CYCLES: usize = 142;
pub const RATE: u32 = 48_000;

/// The MIDI note that plays any generated source at its recorded rate. C3.
///
/// **Every recipe shares it**, which is what lets the editor set Root exactly when it generates
/// one — a generated source knows its own pitch, where a dropped file does not.
pub const ROOT: f32 = 48.0;

/// Where a generated source's partials stop, in hertz.
///
/// Chosen rather than taken to Nyquist. A source built out to 24 kHz folds as soon as it is played
/// above its root, and this instrument is played up a keyboard. Stopping at 12 kHz keeps an octave
/// of headroom before the content itself aliases.
pub const BANDLIMIT_HZ: f32 = 12_000.0;

/// Peak of the normalised result. Leaves headroom for the layer's own level and the converter.
const PEAK: f32 = 0.85;

/// The highest cycle count a partial may have and stay under [`BANDLIMIT_HZ`].
fn cycle_ceiling() -> u32 {
    (BANDLIMIT_HZ * FRAMES as f32 / RATE as f32).floor() as u32
}

/// The highest harmonic of the fundamental that fits under the band limit. 91 at these numbers.
fn harmonic_ceiling() -> u32 {
    cycle_ceiling() / CYCLES as u32
}

/// One partial: a fixed whole number of cycles over the sample, and a starting phase.
#[derive(Debug, Clone, Copy)]
struct Partial {
    cycles: u32,
    phase: f32,
    /// Which harmonic of the fundamental this partial stands for, for recipes whose amplitude law
    /// is written in terms of harmonic number. Inharmonic recipes use it as an index.
    harmonic: f32,
    /// Which of a recipe's partial sets this belongs to, for the recipes that carry two.
    set: u8,
}

impl Partial {
    /// A partial at `ratio` times the fundamental, rounded onto the cycle grid so it loops.
    ///
    /// Returns `None` past the band limit, which is how a recipe stays band-limited without
    /// knowing the limit: it asks for what it wants and the ones that do not fit are simply absent.
    fn at(ratio: f32, harmonic: f32, set: u8, phase: f32) -> Option<Self> {
        if !(ratio.is_finite() && ratio > 0.0) {
            return None;
        }
        let cycles = (ratio * CYCLES as f32).round();
        if cycles < 1.0 || cycles > cycle_ceiling() as f32 {
            return None;
        }
        Some(Self {
            cycles: cycles as u32,
            phase,
            harmonic,
            set,
        })
    }

    /// A plain harmonic.
    fn harmonic(n: u32) -> Option<Self> {
        Partial::at(n as f32, n as f32, 0, 0.0)
    }
}

/// The seventeen generated sources.
///
/// Ordered as they read in the picker. The first two are the two ends of the same idea: the plain
/// saw the instrument has always started on, which does not move and is the one to learn the
/// synth on, and [`Recipe::Supersaw`], which is seven of it and is there to be made enormous with.
/// Then the fifteen that sweep, of which the owner named three — a pulse sweeping its width, a saw
/// sweeping the same way, and a triangle arriving at a sine from an overtone-rich start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    InitSaw,
    Supersaw,
    PulseSweep,
    SawFold,
    OvertoneTriangle,
    HollowToFull,
    FormantGlide,
    TwoVowels,
    PhaseDrift,
    StretchedHarmonics,
    BellToTone,
    Drawbars,
    ToneToNoise,
    SidebandSweep,
    SyncSweep,
    UnisonSpread,
    Glass,
}

impl Recipe {
    /// Every recipe, in picker order.
    pub const ALL: [Recipe; 17] = [
        Recipe::InitSaw,
        Recipe::Supersaw,
        Recipe::PulseSweep,
        Recipe::SawFold,
        Recipe::OvertoneTriangle,
        Recipe::HollowToFull,
        Recipe::FormantGlide,
        Recipe::TwoVowels,
        Recipe::PhaseDrift,
        Recipe::StretchedHarmonics,
        Recipe::BellToTone,
        Recipe::Drawbars,
        Recipe::ToneToNoise,
        Recipe::SidebandSweep,
        Recipe::SyncSweep,
        Recipe::UnisonSpread,
        Recipe::Glass,
    ];

    /// What the picker shows: the shape alone, without the root the payload name carries.
    ///
    /// **No `.wav` on the end.** These are not files and never were; the one that used to be
    /// called `Init saw.wav` carried an extension for a file that does not exist.
    pub const fn label(self) -> &'static str {
        match self {
            Recipe::InitSaw => "Init saw",
            Recipe::Supersaw => "Supersaw",
            Recipe::PulseSweep => "Pulse sweep",
            Recipe::SawFold => "Saw taper",
            Recipe::OvertoneTriangle => "Overtone triangle",
            Recipe::HollowToFull => "Hollow to full",
            Recipe::FormantGlide => "Formant glide",
            Recipe::TwoVowels => "Two vowels",
            Recipe::PhaseDrift => "Phase drift",
            Recipe::StretchedHarmonics => "Stretched harmonics",
            Recipe::BellToTone => "Bell to tone",
            Recipe::Drawbars => "Drawbars",
            Recipe::ToneToNoise => "Tone to noise",
            Recipe::SidebandSweep => "Sideband sweep",
            Recipe::SyncSweep => "Sync sweep",
            Recipe::UnisonSpread => "Unison spread",
            Recipe::Glass => "Glass",
        }
    }

    /// What the loaded source is called, in the layer chip and in saved state.
    ///
    /// **It ends in the note it plays at, and that is load-bearing rather than decorative.** Every
    /// generated source is C3 by construction, but an arrival without a key sets Root to
    /// `NEUTRAL_ROOT` — C4 — because a *dropped file's* pitch is unknown, which would leave the
    /// whole bank an octave out. `root_in_name` already runs on every load and already reads
    /// a trailing key from the name, so naming them this way tunes them through a path that exists
    /// and is already tested, instead of a second mechanism racing the first to write Root.
    pub fn name(self) -> String {
        format!("{} C3", self.label())
    }

    /// One line for the selector's hover text, saying what the sweep does.
    /// One line about the sound, shown under the picker.
    ///
    /// **Every one of these says where the sweep *ends up*, not where it finishes.** [`sweep`] goes
    /// out and back, so the far state is reached halfway along the sample and the near state is at
    /// both ends — which is what makes them loop. Written as "starts as X and becomes Y", these
    /// described a source that does not exist, and they are read by someone about to scan the
    /// sample with three readers whose whole point is *where* in it they sit.
    pub const fn description(self) -> &'static str {
        match self {
            Recipe::InitSaw => {
                "A plain band-limited saw. The sound the instrument has always started on, and the only one here that does not move."
            }
            Recipe::Supersaw => {
                "Seven saws, detuned up to a third of a semitone and beating against each other. \
                 One saw at both ends, all seven through the middle — scan position is how big it \
                 gets."
            }
            Recipe::PulseSweep => {
                "A pulse width closing and reopening: a full square at both ends, a thin reed \
                 through the middle."
            }
            Recipe::SawFold => {
                "A bright saw at both ends, its overtones tilting away to little but the \
                 fundamental through the middle. A spectral taper, not a wavefolder."
            }
            Recipe::OvertoneTriangle => {
                "Richer than a triangle at both ends, its overtones some 30 dB down in the middle \
                 — and exactly a triangle at the quarter and three-quarter points, twice over."
            }
            Recipe::HollowToFull => {
                "Odd harmonics alone at both ends, the even ones fading in and out again: \
                 clarinet at the edges, saw through the middle."
            }
            Recipe::FormantGlide => {
                "A resonant peak sliding up through the harmonics and back down — a vowel opening \
                 and closing."
            }
            Recipe::TwoVowels => {
                "Two resonant peaks crossing in opposite directions and crossing back — one vowel \
                 becoming another and returning."
            }
            Recipe::PhaseDrift => {
                // **Its phase offsets are whole turns at full morph, so the midpoint is the
                // start.** `phase` ramps by `TAU * (harmonic % 4) * t`, and at `t = 1` every one
                // of those is an exact number of turns — which is what keeps the ramp from
                // clicking, and also means the waveform this recipe is *furthest* from its
                // starting shape at is a quarter and three quarters of the way along, not
                // halfway. It says so.
                "The spectrum never changes; only the partials' phases drift away and back, twice \
                 over. The same harmonics throughout — the waveform is furthest from its starting \
                 shape at the quarter and three-quarter points, and back to it at the ends and the \
                 middle."
            }
            Recipe::StretchedHarmonics => {
                "Harmonics pulling sharp of where they should be and settling back: a plain tone \
                 at both ends, through piano, to bell in the middle."
            }
            Recipe::BellToTone => {
                "Struck inharmonic partials at both ends, resolving onto a plain harmonic series \
                 through the middle and back out again. The hum note is held throughout."
            }
            Recipe::Drawbars => {
                "Six drawbars crossfading from a hollow registration to a bright one and back."
            }
            Recipe::ToneToNoise => {
                "Every harmonic spreading into a cluster and gathering again: a clear tone at both \
                 ends, a band of noise in the middle."
            }
            Recipe::SidebandSweep => {
                "Sidebands opening around each harmonic and closing again, as a modulation index \
                 rising and falling would."
            }
            Recipe::SyncSweep => {
                "A peak locked above the fundamental, sweeping up and back down — the \
                 oscillator-sync gesture, out and returned."
            }
            Recipe::UnisonSpread => {
                "Each harmonic splitting into a detuned trio and closing up again: tight unison at \
                 both ends, a wide beating stack in the middle."
            }
            Recipe::Glass => {
                "Only the high partials at both ends, filling downward until the series is whole \
                 in the middle, then emptying again."
            }
        }
    }

    /// The partials this recipe uses, with their fixed cycle counts.
    fn partials(self) -> Vec<Partial> {
        let top = harmonic_ceiling();
        match self {
            Recipe::InitSaw
            | Recipe::PulseSweep
            | Recipe::SawFold
            | Recipe::HollowToFull
            | Recipe::FormantGlide
            | Recipe::TwoVowels
            | Recipe::SyncSweep
            | Recipe::Glass => (1..=top).filter_map(Partial::harmonic).collect(),
            Recipe::OvertoneTriangle => (1..=top)
                .filter(|n| n % 2 == 1)
                .filter_map(Partial::harmonic)
                .collect(),
            // A flat-ish bank is what makes a pure phase change audible; a 1/n spectrum would hide
            // it under the fundamental.
            Recipe::PhaseDrift => (1..=top.min(48))
                .filter_map(|n| Partial::at(n as f32, n as f32, 0, 0.0))
                .collect(),
            // Two banks: the harmonic series, and the same series pulled sharp. Crossfading their
            // amplitudes beats the near-coincident pairs against each other, which is the whole
            // sound — a frequency that had to move instead becomes two that do not.
            Recipe::StretchedHarmonics => {
                let mut partials: Vec<Partial> = (1..=top).filter_map(Partial::harmonic).collect();
                // **From the second partial up.** `1^1.04` is 1, so a stretched fundamental lands
                // on exactly the cycle count the harmonic one already occupies: the two would sum
                // instead of beating, and crossfading between them would take the fundamental out
                // altogether at the far end. A real stretched string keeps its fundamental and
                // spreads what is above it, which is also what this needs to stay playable.
                partials.extend(
                    (2..=top).filter_map(|n| Partial::at((n as f32).powf(1.04), n as f32, 1, 0.0)),
                );
                partials
            }
            // A struck bar's first partials, against the harmonic series they resolve onto.
            Recipe::BellToTone => {
                // **No 1.0 or 2.00 in the bell set.** Those are exactly the harmonic set's own
                // first two, so the pair would land on one cycle count and sum rather than beat —
                // and crossfading two partials at the same frequency is a level change wearing an
                // inharmonicity's name. The fundamental is shared and held instead, which is what
                // a struck bar does: the hum note stays while the partials above it settle.
                const BELL: [f32; 6] = [2.76, 5.40, 8.93, 13.34, 18.64, 24.67];
                let mut partials: Vec<Partial> = Vec::new();
                partials.extend(Partial::at(1.0, 1.0, 2, 0.0));
                partials.extend(
                    BELL.iter().enumerate().filter_map(|(index, ratio)| {
                        Partial::at(*ratio, index as f32 + 2.0, 0, 0.0)
                    }),
                );
                partials.extend((2..=8).filter_map(|n| Partial::at(n as f32, n as f32, 1, 0.0)));
                partials
            }
            // **Seven saws, detuned by whole cycles rather than by cents.**
            //
            // The integer-cycle rule is usually something a recipe has to be rounded onto; here it
            // *is* the tuning. Voice `m` puts its harmonic `k` at `k * (CYCLES + m)` cycles, which
            // is an integer by construction, so every partial closes the loop exactly and the
            // detune needs no rounding at all. It is also the right kind of detune: the offset
            // scales with `k`, so the voices beat `k` times faster at the `k`th harmonic — the
            // shimmer that makes a supersaw, rather than the flat, single-rate beating a constant
            // cycle offset would give.
            //
            // `m` of ±1, ±2, ±3 is 12.1, 24.2 and 36.3 cents at the fundamental, which is a JP-8000
            // at a wide setting, and beats of 0.92, 1.84 and 2.76 Hz — slow enough to hear as
            // movement rather than as roughness.
            //
            // **The detuned voices stop at the 27th harmonic, and the number is forced.** On an
            // integer-cycle grid two voices share a partial as soon as `k1·(142+m1)` equals
            // `k2·(142+m2)`, which first happens where the two counts share a factor: 140 and 145
            // share 5, so the +3 voice's 28th harmonic lands exactly on the −2 voice's 29th. Two
            // partials on one cycle count sum instead of beating — the failure
            // `no_recipe_collides_two_partials_on_one_cycle_count` exists for, and here it would
            // quietly remove the very thing the recipe is made of. 27 is the last harmonic below
            // that, and the next collisions (144 against 140 at 35, 141 against 144 at 47) sit
            // well above it.
            //
            // The cost is that the detuned copies reach 3.5 kHz rather than 12. That is the right
            // place to spend it: the centre saw carries the top on its own, and the beating that
            // makes a supersaw is a low and middle phenomenon — by the tenth harmonic the outer
            // pair is already beating at 28 Hz, which is heard as roughness rather than movement.
            // It also keeps the whole thing to 253 partials, so a pick stays brief.
            Recipe::Supersaw => {
                const DETUNE: [i32; 6] = [1, -1, 2, -2, 3, -3];
                let mut partials: Vec<Partial> = (1..=top).filter_map(Partial::harmonic).collect();
                for offset in DETUNE {
                    let voice = (CYCLES as i32 + offset) as f32;
                    partials.extend((1..=top.min(27)).filter_map(|k| {
                        Partial::at(k as f32 * voice / CYCLES as f32, k as f32, 1, 0.0)
                    }));
                }
                partials
            }
            Recipe::Drawbars => [1u32, 2, 3, 4, 6, 8]
                .into_iter()
                .filter_map(Partial::harmonic)
                .collect(),
            // Five partials per harmonic. The outer pairs are a few cycles off, so they beat
            // rather than reinforce, and enough of them together read as a band rather than a chord.
            Recipe::ToneToNoise => (1..=top.min(40))
                .flat_map(|n| {
                    [-7i32, -3, 0, 3, 7]
                        .into_iter()
                        .enumerate()
                        .map(move |(slot, offset)| {
                            let cycles = (n * CYCLES as u32) as i32 + offset;
                            (cycles, n, slot as u8)
                        })
                })
                .filter_map(|(cycles, n, slot)| {
                    Partial::at(cycles as f32 / CYCLES as f32, n as f32, slot, 0.0)
                })
                .collect(),
            // Carrier harmonics plus two sideband pairs each, opened in turn so the apparent
            // modulation index climbs without any frequency having to move.
            Recipe::SidebandSweep => (1..=8u32)
                .flat_map(|n| {
                    [(0.0f32, 0u8), (0.73, 1), (-0.73, 1), (1.46, 2), (-1.46, 2)]
                        .into_iter()
                        .map(move |(offset, set)| (n as f32 + offset, n, set))
                })
                .filter_map(|(ratio, n, set)| Partial::at(ratio, n as f32, set, 0.0))
                .collect(),
            // A trio per harmonic, one cycle either side: about nine tenths of a hertz of detune,
            // which beats slowly enough to thicken rather than to warble.
            Recipe::UnisonSpread => (1..=top.min(48))
                .flat_map(|n| {
                    [(0i32, 0u8), (-1, 1), (1, 1)]
                        .into_iter()
                        .map(move |(offset, set)| (n, offset, set))
                })
                .filter_map(|(n, offset, set)| {
                    let cycles = (n * CYCLES as u32) as i32 + offset;
                    Partial::at(cycles as f32 / CYCLES as f32, n as f32, set, 0.0)
                })
                .collect(),
        }
    }

    /// Partial `partial`'s amplitude at position `t` in `0..1`.
    fn amplitude(self, partial: &Partial, t: f32) -> f32 {
        let n = partial.harmonic;
        match self {
            // **A corner sliding up the series, which is a tooth narrowing.** `1/(n + c)` is
            // flat below harmonic `c` and falls as `1/n` above it, so raising `c` flattens more
            // and more of the low end while the top keeps its saw slope — and a spectrum that is
            // flat to harmonic `c` is a spike about `1/c` of a cycle wide. The ramp collapses
            // toward the cycle boundary and the rest of the cycle goes quiet: the tooth gets
            // shorter and sharper, its tip where it always was.
            //
            // `c = 0` is `1/n` exactly, so the sample still opens on the saw it always opened on
            // — see `the_saw_still_starts_as_the_saw_it_always_was`. No phase law, so it stays a
            // sine series throughout and remains recognisably a saw rather than becoming a
            // different waveform with a saw's magnitudes.
            Recipe::InitSaw => 1.0 / n,
            Recipe::PulseSweep => {
                // A pulse of duty `d` has harmonic amplitudes `sin(pi n d) / n`. Sweeping `d`
                // sweeps the comb of nulls down through the series, which is the sound.
                let duty = 0.5 - 0.49 * t;
                (std::f32::consts::PI * n * duty).sin() / n
            }
            // The centre saw holds its level and the six fade in around it. **That is a detune
            // knob, not an approximation of one**: seven saws in exact unison sound like one saw
            // that is seven times louder, and the level is what the peak normalisation and the
            // layer's own Level answer. So bringing the detuned voices up from silence is the same
            // gesture as opening the detune from zero, and it is the one this engine can make —
            // a partial's frequency is fixed when it is built.
            Recipe::Supersaw => {
                let share = if partial.set == 0 { 1.0 } else { 0.75 * t };
                share / n
            }
            Recipe::SawFold => 1.0 / n.powf(1.0 + 5.0 * t),
            Recipe::OvertoneTriangle => {
                // **Past a triangle at the start, exactly one in the middle, almost a sine at the
                // end.** The tilt alone carries it: at 2.0 the odd harmonics fall as `1/n²`, which
                // with the alternating signs in `phase` *is* a triangle. An extra roll-off used to
                // sit here and meant the midpoint was near one rather than one — a claim the name
                // makes and the code has to keep.
                let tilt = 1.2 + 1.6 * t;
                1.0 / n.powf(tilt)
            }
            Recipe::HollowToFull => {
                let even = if (n as u32).is_multiple_of(2) { t } else { 1.0 };
                even / n
            }
            Recipe::FormantGlide => (1.0 / n) * (0.15 + bump(n, 2.0 + 38.0 * t, 0.55)),
            Recipe::TwoVowels => {
                let low = bump(n, 3.0 + 22.0 * t, 0.4);
                let high = bump(n, 30.0 - 24.0 * t, 0.4);
                (1.0 / n) * (0.1 + low + high)
            }
            Recipe::PhaseDrift => 1.0 / n.sqrt(),
            Recipe::StretchedHarmonics => {
                // The fundamental stays put; only what is above it hands over.
                let share = if n <= 1.5 {
                    1.0
                } else if partial.set == 0 {
                    1.0 - t
                } else {
                    t
                };
                share / n
            }
            Recipe::BellToTone => {
                // Set 2 is the shared fundamental and takes no part in the hand-over.
                let share = match partial.set {
                    2 => 1.0,
                    0 => 1.0 - t,
                    _ => t,
                };
                share / n
            }
            Recipe::Drawbars => {
                // Two registrations: a hollow 16'/5⅓' pair against a bright full-compass draw.
                let hollow = match n as u32 {
                    1 => 1.0,
                    2 => 0.15,
                    3 => 0.8,
                    _ => 0.05,
                };
                let bright = match n as u32 {
                    1 => 0.7,
                    2 => 0.8,
                    3 => 0.5,
                    4 => 0.9,
                    6 => 0.6,
                    _ => 0.7,
                };
                hollow * (1.0 - t) + bright * t
            }
            Recipe::ToneToNoise => {
                // The centre gives way to its neighbours, so the line widens into a band.
                let spread = match partial.set {
                    2 => 1.0 - 0.85 * t,
                    1 | 3 => t * 0.8,
                    _ => t * 0.5,
                };
                spread / n
            }
            Recipe::SidebandSweep => {
                let depth = match partial.set {
                    0 => 1.0 - 0.5 * t,
                    1 => (t * 2.0).min(1.0) * 0.8,
                    _ => (t * 2.0 - 1.0).max(0.0) * 0.7,
                };
                depth / n
            }
            Recipe::SyncSweep => {
                let peak = 1.0 + 23.0 * t;
                (1.0 / n.sqrt()) * (0.08 + bump(n, peak, 0.45))
            }
            Recipe::UnisonSpread => {
                let share = if partial.set == 0 {
                    1.0 - 0.45 * t
                } else {
                    t * 0.7
                };
                share / n
            }
            Recipe::Glass => {
                // A high-pass corner falling through the series: the top is there from the start,
                // the body arrives late.
                //
                // **The corner has to get out of the way entirely, and `roll_off` cannot do it.**
                // It clamps its reach at one, so the lowest corner it can make still leaves the
                // fundamental at half — a series with its root missing, which is not what "the
                // series is whole again" says. The last part of the sweep therefore fades the
                // high-pass out rather than sliding it further down.
                let corner = 40.0 * (1.0 - t).powi(2) + 1.0;
                let opened = ((t - 0.75) / 0.25).clamp(0.0, 1.0);
                let shelf = (1.0 - roll_off(n, corner)) * (1.0 - opened) + opened;
                (1.0 / n) * shelf
            }
        }
    }

    /// Partial `partial`'s starting phase at position `t`.
    ///
    /// Constant for every recipe but one. [`Recipe::PhaseDrift`] ramps it, and the ramp is a whole
    /// number of turns across the sample so the end still meets the beginning — a partial that
    /// finished part way round would click at the loop exactly as a part-cycle would.
    fn phase(self, partial: &Partial, t: f32) -> f32 {
        match self {
            Recipe::PhaseDrift => partial.phase + TAU * ((partial.harmonic as u32 % 4) as f32) * t,
            // **A pulse is a cosine series, and getting that wrong is not a detail.** The
            // magnitudes `sin(pi n d) / n` are a pulse's magnitudes whatever the phase, so a
            // sine-phase version measures identically on any spectrum-only test while being a
            // different waveform — not flat-topped, and with the wrong crest. It matters here more
            // than it would in a synth: Grain windows this shape directly and Character's converter
            // quantises it, so the waveform is the thing being sampled rather than an intermediate.
            Recipe::PulseSweep => partial.phase + std::f32::consts::FRAC_PI_2,
            // A triangle's odd harmonics alternate in sign. Without it the amplitudes are a
            // triangle's and the shape is not one, which is the same error in the other family.
            Recipe::OvertoneTriangle => {
                let index = ((partial.harmonic as u32).saturating_sub(1)) / 2;
                if index.is_multiple_of(2) {
                    partial.phase
                } else {
                    partial.phase + std::f32::consts::PI
                }
            }
            _ => partial.phase,
        }
    }
}

/// A soft top to a harmonic series: 1 below `reach`, falling away above it.
fn roll_off(n: f32, reach: f32) -> f32 {
    let over = n / reach.max(1.0);
    1.0 / (1.0 + over.powi(6))
}

/// A resonant peak at `centre`, `width` octaves wide, measured in the log domain so it keeps its
/// shape wherever it sits in the series.
fn bump(n: f32, centre: f32, width: f32) -> f32 {
    let octaves = (n.max(0.5) / centre.max(0.5)).log2();
    (-(octaves * octaves) / (2.0 * width * width)).exp()
}

/// How far through its morph a recipe is at `position` through the sample: **out and back**.
///
/// Zero at both ends and one in the middle. See the module note — a one-way sweep leaves the ends
/// sounding different and breaks the seamless loop the frame and cycle counts exist to give.
fn sweep(position: f32) -> f32 {
    if position <= 0.5 {
        position * 2.0
    } else {
        (1.0 - position) * 2.0
    }
}

/// Render a recipe into mono samples, normalised to [`PEAK`].
pub fn render(recipe: Recipe) -> Vec<f32> {
    let partials = recipe.partials();
    // One phase step per partial, hoisted: the frequency never moves, only the amplitude does.
    // Written as the saw's own expression was so `Recipe::InitSaw` still produces what
    // `default_source` produced, rather than something equal to within rounding.
    let steps: Vec<f32> = partials
        .iter()
        .map(|partial| TAU * partial.cycles as f32 / FRAMES as f32)
        .collect();
    let mut frames = vec![0.0f32; FRAMES];
    let span = FRAMES as f32;
    for (index, frame) in frames.iter_mut().enumerate() {
        let position = index as f32 / span;
        let morph = sweep(position);
        let mut sum = 0.0;
        for (partial, step) in partials.iter().zip(&steps) {
            let amplitude = recipe.amplitude(partial, morph);
            if amplitude.abs() < 1.0e-5 {
                continue;
            }
            let angle = step * index as f32 + recipe.phase(partial, morph);
            sum += amplitude * angle.sin();
        }
        *frame = sum;
    }
    let peak = frames
        .iter()
        .fold(0.0f32, |peak, value| peak.max(value.abs()));
    if peak > 0.0 {
        let normalise = PEAK / peak;
        for frame in &mut frames {
            *frame *= normalise;
        }
    }
    frames
}

/// A radix-2 FFT, in place, for the two guard-band measurements.
///
/// **Every bin, because the interesting energy is not on the harmonic grid.** The first version
/// of that measurement stepped bins at the harmonic spacing, which is exactly where the
/// *carriers* are and nowhere near where a modulation sideband or an inharmonic partial lands —
/// so it measured the thing that was true by construction and missed the thing in question.
#[cfg(test)]
fn fft(real: &mut [f64], imaginary: &mut [f64]) {
    let n = real.len();
    let mut target = 0usize;
    for source in 1..n {
        let mut bit = n >> 1;
        while target & bit != 0 {
            target ^= bit;
            bit >>= 1;
        }
        target |= bit;
        if source < target {
            real.swap(source, target);
            imaginary.swap(source, target);
        }
    }
    let mut length = 2;
    while length <= n {
        let angle = -std::f64::consts::TAU / length as f64;
        for block in (0..n).step_by(length) {
            for offset in 0..length / 2 {
                let (sin, cos) = (angle * offset as f64).sin_cos();
                let a = block + offset;
                let b = a + length / 2;
                let tr = real[b] * cos - imaginary[b] * sin;
                let ti = real[b] * sin + imaginary[b] * cos;
                real[b] = real[a] - tr;
                imaginary[b] = imaginary[a] - ti;
                real[a] += tr;
                imaginary[a] += ti;
            }
        }
        length <<= 1;
    }
}

/// Out-of-band energy in a window centred on the loop seam, in dB relative to that window's total.
///
/// **This is the measurement on the audio a player actually hears**, and it exists because
/// `every_recipe_closes_its_loop` does not. That test evaluates an idealised `f64` expression, and
/// widening a test oracle can hide a seam that the shipped `f32` render or the i16 payload
/// introduces. So this one takes a rendered buffer, lays it end to end with itself, and windows
/// the join.
///
/// `pub(crate)` and test-only because two modules need it on two forms of the same audio:
/// `generate`'s tests run it on `render`'s output, `asset`'s on the frames that come back out of
/// the encoded payload.
///
/// **What the numbers mean.** Across the bank the seam measures between −63 and −91 dB.
/// Replacing [`sweep`] with the identity — a genuine step at the join — moves ten of them to
/// between −36 and −52 dB, because a discontinuity is broadband and puts energy everywhere. The
/// bound the callers use is absolute rather than a comparison against an interior window: the
/// timbre changes along the sweep by design, so an interior window is not a control.
#[cfg(test)]
pub(crate) fn seam_out_of_band_db(frames: &[f32]) -> f64 {
    const WINDOW: usize = 8_192;
    assert!(
        frames.len() >= WINDOW,
        "a seam needs a window either side of it"
    );
    let guard_bin = (BANDLIMIT_HZ * WINDOW as f32 / RATE as f32).ceil() as usize;
    let at = |index: usize| f64::from(frames[index % frames.len()]);
    let start = frames.len() + frames.len() - WINDOW / 2;
    let mut real: Vec<f64> = (0..WINDOW)
        .map(|offset| {
            let hann = 0.5 - 0.5 * (std::f64::consts::TAU * offset as f64 / WINDOW as f64).cos();
            at(start + offset) * hann
        })
        .collect();
    let mut imaginary = vec![0.0f64; WINDOW];
    fft(&mut real, &mut imaginary);
    let (mut over, mut total) = (0.0f64, 0.0f64);
    for bin in 1..WINDOW / 2 {
        let power = real[bin] * real[bin] + imaginary[bin] * imaginary[bin];
        total += power;
        if bin >= guard_bin {
            over += power;
        }
    }
    10.0 * (over / total.max(f64::MIN_POSITIVE)).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every partial of every recipe sits on the cycle grid and under the band limit — the two
    /// properties the whole bank's seamless looping and alias freedom rest on.
    #[test]
    fn every_partial_is_a_whole_cycle_under_the_band_limit() {
        for recipe in Recipe::ALL {
            let partials = recipe.partials();
            assert!(
                !partials.is_empty(),
                "{} has no partials at all",
                recipe.name()
            );
            for partial in &partials {
                assert!(
                    partial.cycles >= 1 && partial.cycles <= cycle_ceiling(),
                    "{}: a partial at {} cycles is outside 1..={}",
                    recipe.name(),
                    partial.cycles,
                    cycle_ceiling()
                );
            }
        }
    }

    /// The fundamental the frame and cycle counts imply is the root every recipe declares.
    #[test]
    fn the_bank_is_in_tune_with_the_root_it_declares() {
        let fundamental = RATE as f32 * CYCLES as f32 / FRAMES as f32;
        let midi = 69.0 + 12.0 * (fundamental / 440.0).log2();
        assert!(
            (midi - ROOT).abs() < 0.01,
            "the generated bank is not on its declared root: {midi} against {ROOT}"
        );
    }

    /// Names are what the picker shows and what a loaded source is called, so they have to be
    /// distinct and free of the fake file extension the first generated source carried.
    #[test]
    fn every_recipe_has_a_distinct_name_and_description() {
        let mut labels: Vec<&str> = Recipe::ALL.iter().map(|r| r.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(count, labels.len(), "two recipes share a label");
        for recipe in Recipe::ALL {
            assert!(!recipe.label().ends_with(".wav"), "{}", recipe.label());
            assert!(
                recipe.description().len() > 20,
                "{} has no description worth showing",
                recipe.label()
            );
        }
    }

    /// A window of a rendered recipe, and where it starts.
    ///
    /// **The start index is returned because a cycle is not a whole number of frames.** 52 105 over
    /// 142 is 366.94, so stepping in 366-frame "cycles" drifts almost a frame each time and a
    /// window taken half way through is some seventy frames out of phase — which is enough to
    /// destroy a correlation against a reference built from index zero. Every oracle below is
    /// therefore evaluated at these same absolute indices.
    fn window_at(recipe: Recipe, position: f32) -> (usize, Vec<f32>) {
        let frames = render(recipe);
        let period = FRAMES / CYCLES;
        let start = (frames.len() as f32 * position) as usize;
        let start = start.min(frames.len() - period);
        (start, frames[start..start + period].to_vec())
    }

    /// A reference cycle at the same absolute indices, from harmonics given as
    /// `(harmonic, amplitude)` and a phase offset in radians.
    fn reference_at(start: usize, terms: &[(u32, f32)], offset: f32) -> Vec<f32> {
        let period = FRAMES / CYCLES;
        (0..period)
            .map(|index| {
                let absolute = (start + index) as f32;
                terms
                    .iter()
                    .map(|(n, amplitude)| {
                        let step = TAU * (*n * CYCLES as u32) as f32 / FRAMES as f32;
                        amplitude * (step * absolute + offset).sin()
                    })
                    .sum()
            })
            .collect()
    }

    /// How alike two cycles are, ignoring level.
    fn correlation(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let norm =
            (a.iter().map(|x| x * x).sum::<f32>() * b.iter().map(|y| y * y).sum::<f32>()).sqrt();
        if norm > 0.0 { dot / norm } else { 0.0 }
    }

    /// **`PulseSweep` is a pulse, not a spectrum that measures like one.**
    ///
    /// Its magnitudes are a pulse's at any phase, so an amplitude-only check passes on a waveform
    /// that is not flat-topped and has the wrong crest — which is what this shipped with until a
    /// review caught the claim. The oracle is an independently built band-limited pulse.
    #[test]
    fn the_pulse_sweep_really_is_a_pulse() {
        // At the start the duty is one half: a square, and a square has no even harmonics.
        for n in [2u32, 4, 6, 8] {
            let partial = Partial::harmonic(n).expect("an even harmonic");
            let amplitude = Recipe::PulseSweep.amplitude(&partial, 0.0);
            assert!(
                amplitude.abs() < 1.0e-4,
                "a square should have no harmonic {n}, found {amplitude}"
            );
        }
        // And the waveform itself is the pulse, not merely its magnitudes. `sweep` takes the morph
        // to its far end at the middle of the sample, so that is where the narrow pulse lives.
        for (position, duty) in [(0.01f32, 0.5 - 0.49 * 0.02), (0.5, 0.01)] {
            let terms: Vec<(u32, f32)> = (1..=harmonic_ceiling())
                .map(|n| (n, (std::f32::consts::PI * n as f32 * duty).sin() / n as f32))
                .collect();
            let (start, window) = window_at(Recipe::PulseSweep, position);
            let ideal = reference_at(start, &terms, std::f32::consts::FRAC_PI_2);
            let score = correlation(&window, &ideal);
            // **A 1% pulse is nearly all flat**, so most of the window carries no information and
            // the few samples that do dominate: 0.9 there is a close match where 0.95 on the square
            // is an easy one. What this has to separate is a pulse from a *sine-phase* spectrum of
            // one, and that scores far below either.
            let floor = if duty > 0.1 { 0.95 } else { 0.85 };
            assert!(
                score > floor,
                "Pulse sweep at {position} correlates {score} with a real {:.1}% pulse",
                duty * 100.0
            );
        }
    }

    /// **`OvertoneTriangle` passes through an actual triangle**, which needs the alternating signs
    /// its odd harmonics carry. Amplitudes alone give a triangle's spectrum and a different shape.
    #[test]
    fn the_overtone_triangle_passes_through_a_triangle() {
        let triangle: Vec<(u32, f32)> = (1..=harmonic_ceiling())
            .filter(|n| n % 2 == 1)
            .map(|n| {
                let index = (n - 1) / 2;
                let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
                (n, sign / (n * n) as f32)
            })
            .collect();
        // `sweep` is out and back, so the middle of the *morph* is a quarter of the way through
        // the sample. That is where the tilt reaches 2.0 and the shape is a triangle.
        let (start, window) = window_at(Recipe::OvertoneTriangle, 0.25);
        let score = correlation(&window, &reference_at(start, &triangle, 0.0));
        assert!(
            score > 0.99,
            "halfway through its sweep, Overtone triangle correlates {score} with a real triangle"
        );
        // **And it starts richer, which is a claim about the spectrum rather than the shape.** A
        // waveform correlation is dominated by the fundamental and reads 0.98 against a triangle
        // even when the overtones are several times too loud, so it cannot see this; the third
        // harmonic's weight against the first can.
        let third = Partial::harmonic(3).expect("a third harmonic");
        let first = Partial::harmonic(1).expect("a fundamental");
        let ratio_at = |t: f32| {
            Recipe::OvertoneTriangle.amplitude(&third, t)
                / Recipe::OvertoneTriangle.amplitude(&first, t)
        };
        assert!(
            ratio_at(0.0) > ratio_at(0.5) * 1.8,
            "Overtone triangle does not start richer than its triangle: {} against {}",
            ratio_at(0.0),
            ratio_at(0.5)
        );
        assert!(
            ratio_at(1.0) < ratio_at(0.5) * 0.6,
            "Overtone triangle does not end plainer than its triangle: {} against {}",
            ratio_at(1.0),
            ratio_at(0.5)
        );
    }

    /// Two partials landing on the same cycle count would sum instead of beating, quietly undoing
    /// the detune or inharmonicity the recipe is named for.
    #[test]
    fn no_recipe_collides_two_partials_on_one_cycle_count() {
        for recipe in Recipe::ALL {
            let mut seen: Vec<u32> = recipe
                .partials()
                .iter()
                .map(|partial| partial.cycles)
                .collect();
            let total = seen.len();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(
                total,
                seen.len(),
                "{} puts two partials on one cycle count",
                recipe.label()
            );
        }
    }

    /// Nothing carries DC, which a sampler would hear as an offset through every reader and the
    /// converter would quantise around.
    #[test]
    fn no_recipe_carries_dc() {
        for recipe in Recipe::ALL {
            let frames = render(recipe);
            let mean = frames.iter().sum::<f32>() / frames.len() as f32;
            assert!(
                mean.abs() < 1.0e-3,
                "{} carries {mean} of DC",
                recipe.label()
            );
        }
    }

    /// **Peak normalisation is global, so a recipe whose loudest moment is at one end can be
    /// disproportionately quiet everywhere else** — and a source that is 20 dB down over most of
    /// its length is not a usable scan even though every other test here passes.
    #[test]
    fn no_recipe_is_lopsided_across_its_sweep() {
        for recipe in Recipe::ALL {
            let frames = render(recipe);
            let chunk = frames.len() / 8;
            let levels: Vec<f32> = frames
                .chunks(chunk)
                .take(8)
                .map(|part| (part.iter().map(|v| v * v).sum::<f32>() / part.len() as f32).sqrt())
                .collect();
            let loudest = levels.iter().cloned().fold(0.0f32, f32::max);
            let quietest = levels.iter().cloned().fold(f32::INFINITY, f32::min);
            assert!(
                quietest > loudest * 0.1,
                "{} ranges {quietest:.4} to {loudest:.4} across its sweep, over 20 dB",
                recipe.label()
            );
        }
    }

    /// **Nothing meaningful lands above the band limit — measured over every bin.**
    ///
    /// The carriers are bounded by construction, but a moving amplitude is modulation and
    /// modulation makes sidebands; `sweep` also turns a corner at its apex, which is a kink in the
    /// envelope and broadband in principle. Neither is on the harmonic grid, so this transforms the
    /// rendered audio and integrates the whole guard band from [`BANDLIMIT_HZ`] to Nyquist.
    ///
    /// Three windows, because the morph is somewhere different at each: the start and end of the
    /// sample sit at one extreme of the sweep and its middle at the other. Hann-windowed, so the
    /// window's own leakage sits far below what is being looked for.
    #[test]
    fn nothing_lands_above_the_band_limit() {
        const WINDOW: usize = 16_384;
        let guard_bin = (BANDLIMIT_HZ * WINDOW as f32 / RATE as f32).ceil() as usize;
        for recipe in Recipe::ALL {
            let frames = render(recipe);
            let mut worst = f64::NEG_INFINITY;
            for offset in [0, FRAMES / 4 - WINDOW / 2, FRAMES / 2 - WINDOW / 2] {
                let mut real: Vec<f64> = (0..WINDOW)
                    .map(|index| {
                        let hann = 0.5
                            - 0.5 * (std::f64::consts::TAU * index as f64 / WINDOW as f64).cos();
                        f64::from(frames[offset + index]) * hann
                    })
                    .collect();
                let mut imaginary = vec![0.0f64; WINDOW];
                fft(&mut real, &mut imaginary);
                let mut over = 0.0f64;
                let mut total = 0.0f64;
                for bin in 1..WINDOW / 2 {
                    let power = real[bin] * real[bin] + imaginary[bin] * imaginary[bin];
                    total += power;
                    if bin >= guard_bin {
                        over += power;
                    }
                }
                worst = worst.max(10.0 * (over / total.max(f64::MIN_POSITIVE)).log10());
            }
            assert!(
                worst < -60.0,
                "{} puts {worst:.1} dB of its energy above {BANDLIMIT_HZ} Hz",
                recipe.label()
            );
        }
    }

    /// **Every source closes its own loop**, which is the property a morph is most likely to break
    /// and the one the whole bank inherits from the saw.
    ///
    /// Whole cycles make the *carrier* periodic and say nothing about the envelope: a spectrum
    /// swept one way leaves the last frame sounding nothing like the first, and the loop clicks
    /// once a second. [`sweep`] takes the morph out and back so the ends agree; this is what says
    /// so. Measured the way `asset.rs` measures the saw — the wrap step against the largest step
    /// **The buffer continues into its own head — checked frame by frame, not by loudness.**
    ///
    /// This measured RMS and mean absolute slope over the first and last twentieth and asked the
    /// two ratios to sit near one. That is an aggregate oracle, and it is satisfied by any pair of
    /// ends that merely *sound* about as loud and as bright — two different spectra routinely do.
    /// It said nothing at all about phase, which is the one thing `PhaseDrift` moves. The seam
    /// step was then compared against the largest step anywhere in the buffer, which for a saw is
    /// the saw's own once-per-cycle edge: a bound so loose it would admit almost any jump.
    ///
    /// What makes these buffers loop is two facts, and both are checked here directly. Every
    /// partial completes a whole number of cycles over [`FRAMES`], so `sin(step·FRAMES + phi)` is
    /// `sin(phi)` — the sinusoid arrives back where it started with its slope intact. And the
    /// envelope returns, because [`sweep`] turns around at its apex. Together they say the
    /// generator's own expression, evaluated one whole buffer on, reproduces the head; so that is
    /// what is evaluated. Three consecutive frames, because agreeing at one point is continuity of
    /// **The buffer continues into its own head, in value and in slope — measured, not argued.**
    ///
    /// This used to compare RMS and mean absolute slope over the first and last twentieth and ask
    /// the two ratios to sit near one. That is an aggregate: it is satisfied by any pair of ends
    /// that merely *sound* about as loud and as bright, and it says nothing whatever about phase,
    /// which is the one thing `PhaseDrift` moves. The seam step was then compared against the
    /// largest step anywhere in the buffer — for a saw, its own once-per-cycle edge, a bound loose
    /// enough to admit almost any jump.
    ///
    /// **Everything here is evaluated in `f64`, and that is not fussiness.** The same measurement
    /// in `f32` reported a seam corner that *grew* as the step shrank — 0.0003 at one sample and
    /// 0.044 at a 256th — which is the signature of cancellation, not of a corner: the difference
    /// being divided is smaller than the rounding on the values. The angle is the reason. It
    /// reaches `TAU × cycle_ceiling()` at the seam — some ninety times `TAU × 142`, because the
    /// fundamental's 142 cycles is the *smallest* count in the series, not the largest — and `f32`
    /// holds an angle that size to well over 1e-3 of a radian.
    ///
    /// What the `f64` measurement then shows, at every step size from one sample to a 4096th:
    ///
    /// | Recipe | corner, fraction of peak per sample |
    /// |---|---|
    /// | `PhaseDrift` | 0.00096, the same at every step size |
    /// | every other recipe | below 1e-5, falling with the step |
    ///
    /// **`PhaseDrift`'s is real and the rest are not**, and the structure says why. The corner is
    /// the term carrying `sweep`'s own derivative, which reverses at the seam — the morph arrives
    /// falling at 2 per buffer and leaves rising at 2. In the amplitude term that derivative is
    /// multiplied by `sin(phi)`, and every recipe but `PhaseDrift` has `phi = 0`, so it vanishes
    /// exactly. `PhaseDrift` moves `phi` itself, so its term survives as `cos(phi)`. One tenth of
    /// one percent of peak per sample, against a carrier whose own slope is of order peak per
    /// sample, is not a click; the bound below is what stops it growing into one.
    /// **`PhaseDrift` returns to its starting shape at the midpoint, not only at the ends.**
    ///
    /// Its ramp is `TAU * (harmonic % 4) * t`, and at full morph — reached halfway along the
    /// sample — every one of those is a whole number of turns. That is what stops the ramp
    /// clicking, and it also means the midpoint is the shape the sample started from. The
    /// description used to say "a wholly different waveform in the middle", which was the opposite
    /// of true; the waveform is furthest from its start at the quarter points, where the offsets
    /// are half-turns.
    ///
    /// **Asserted on the phase offsets rather than on rendered frames**, because `FRAMES` is odd:
    /// the buffer has no sample at its exact midpoint, and comparing frame `FRAMES / 2` against
    /// frame 0 measures a half-sample carrier offset far larger than the thing in question. The
    /// carrier itself is not at issue — every partial advances `TAU * n * 71` over half the
    /// buffer, a whole number of turns for all of them.
    #[test]
    fn the_phase_drift_returns_at_the_middle_and_departs_at_the_quarters() {
        let partials = Recipe::PhaseDrift.partials();
        let turns = |t: f32, partial: &Partial| {
            let offset = Recipe::PhaseDrift.phase(partial, t) - partial.phase;
            (offset / TAU).rem_euclid(1.0)
        };
        let mut departed = 0;
        for partial in &partials {
            let middle = turns(sweep(0.5), partial);
            assert!(
                middle < 1.0e-5 || middle > 1.0 - 1.0e-5,
                "harmonic {} is {middle} turns from where it started at the midpoint",
                partial.harmonic
            );
            // At the quarter point the ramp stands at half of `harmonic % 4` turns, so a partial
            // is either exactly where it began or exactly inverted — the furthest this ramp
            // reaches. Half the series is inverted, which is the departure the name describes.
            let quarter = turns(sweep(0.25), partial);
            if (quarter - 0.5).abs() < 1.0e-5 {
                departed += 1;
            } else {
                assert!(
                    quarter < 1.0e-5 || quarter > 1.0 - 1.0e-5,
                    "harmonic {} sits at {quarter} turns, neither home nor inverted",
                    partial.harmonic
                );
            }
        }
        assert_eq!(
            departed * 2,
            partials.len(),
            "half the series should be inverted at the quarter point"
        );
    }

    /// **The seam, measured on the buffer that ships rather than on an expression.**
    ///
    /// `every_recipe_closes_its_loop` evaluates an idealised `f64` form of `render`'s arithmetic,
    /// which is the only way to resolve the derivative at the join — and is exactly the kind of
    /// oracle that can be true while the shipped `f32` buffer has a step in it. This lays the
    /// rendered buffer end to end with itself and asks what the join puts above the band limit.
    /// `asset` runs the same measurement on the frames that come back out of the encoded payload.
    #[test]
    fn no_recipe_has_an_audible_seam_in_the_buffer_that_ships() {
        for recipe in Recipe::ALL {
            let seam = seam_out_of_band_db(&render(recipe));
            assert!(
                seam < -55.0,
                "{} puts {seam:.1} dB of the seam window's energy above {BANDLIMIT_HZ} Hz",
                recipe.label()
            );
        }
    }

    /// **The supersaw is one saw at its ends and seven beating through its middle.**
    ///
    /// That is the whole claim of the recipe and of its description, and it is not something the
    /// spectral tests can see: they measure what partials are present, and seven saws in unison
    /// have exactly the same partials as seven saws detuned would if the detune were rounded away.
    /// What separates them is *movement* — partials a cycle or two apart beat, and beating is an
    /// envelope that will not sit still. So this measures the envelope.
    ///
    /// Peaks are taken over 40 ms blocks: long enough to hold several cycles of a 130 Hz
    /// fundamental, short enough that beats of 0.92 to 2.76 Hz move between one block and the next.
    /// Measured, the ends swing 3% — flat, and the same 3% a plain saw's own blocks swing, which is
    /// the control — while the middle swings 45%.
    #[test]
    fn the_supersaw_is_one_saw_at_its_ends_and_seven_through_its_middle() {
        const BLOCK: usize = 1_920;
        let spread = |frames: &[f32], from: usize| {
            let peaks: Vec<f32> = (0..6)
                .map(|block| {
                    frames[from + block * BLOCK..from + (block + 1) * BLOCK]
                        .iter()
                        .fold(0.0f32, |peak, value| peak.max(value.abs()))
                })
                .collect();
            let high = peaks.iter().cloned().fold(0.0f32, f32::max);
            let low = peaks.iter().cloned().fold(f32::INFINITY, f32::min);
            (high - low) / high
        };
        let frames = render(Recipe::Supersaw);
        let middle = FRAMES / 2 - 3 * BLOCK;
        assert!(
            spread(&frames, 0) < 0.10,
            "the ends should be one steady saw, not {:.3} of envelope movement",
            spread(&frames, 0)
        );
        assert!(
            spread(&frames, middle) > 0.25,
            "the middle should be seven saws beating, not {:.3} of envelope movement",
            spread(&frames, middle)
        );
        // The control: a single saw does not do this anywhere, so the movement above is the
        // detuning and not something every recipe's envelope does.
        let plain = render(Recipe::InitSaw);
        assert!(
            spread(&plain, middle) < 0.10,
            "a plain saw should not beat at all: {:.3}",
            spread(&plain, middle)
        );
    }

    #[test]
    fn every_recipe_closes_its_loop() {
        assert_eq!(
            sweep(1.0),
            sweep(0.0),
            "a morph that does not return cannot close a loop, whatever the partials do"
        );
        for recipe in Recipe::ALL {
            let partials = recipe.partials();
            // `render`'s own arithmetic, widened, and taking a frame index as a real number —
            // including `FRAMES` itself, the frame one past the last, which the buffer never
            // stores and which is exactly the value the first frame has to match.
            //
            // **The position is not wrapped.** An earlier version took `(index / FRAMES).fract()`,
            // which sends that index back to position 0.0 — the same morph the head is rendered at
            // — and so compared the head against itself. It passed with `sweep` replaced by the
            // identity, which is precisely the defect this exists to catch.
            let value_at = |index: f64| -> f64 {
                let position = index / FRAMES as f64;
                let morph = if position <= 0.5 {
                    position * 2.0
                } else {
                    (1.0 - position) * 2.0
                } as f32;
                partials
                    .iter()
                    .map(|partial| {
                        let amplitude = recipe.amplitude(partial, morph);
                        if amplitude.abs() < 1.0e-5 {
                            return 0.0;
                        }
                        let step =
                            std::f64::consts::TAU * f64::from(partial.cycles) / FRAMES as f64;
                        f64::from(amplitude)
                            * (step * index + f64::from(recipe.phase(partial, morph))).sin()
                    })
                    .sum()
            };
            // Scaled against the waveform's own size, so a tolerance means the same thing for a
            // recipe with three partials and one with ninety.
            let peak = (0..FRAMES)
                .map(|index| value_at(index as f64).abs())
                .fold(0.0f64, f64::max);
            assert!(peak > 0.0, "{} renders silence", recipe.label());

            // The value continues: the frame after the last is the first.
            let first = value_at(0.0);
            let past_the_end = value_at(FRAMES as f64);
            let step = (first - past_the_end).abs() / peak;
            assert!(
                step < 1.0e-4,
                "{}: the frame after the last reads {past_the_end} where the first reads {first} \
                 — a step of {:.4}% of peak across the seam",
                recipe.label(),
                step * 100.0
            );

            // And the slope continues. **One-sided differences with the same small step**, so the
            // two are compared on their own sides of the seam: a backward difference minus a
            // forward difference differs by the second derivative on any smooth waveform at all,
            // and that artefact is what an earlier attempt reported as a 0.64% corner on
            // `Pulse sweep`, whose true corner is below 1e-5.
            const H: f64 = 1.0 / 256.0;
            let arriving = (past_the_end - value_at(FRAMES as f64 - H)) / H;
            let leaving = (value_at(H) - first) / H;
            let corner = (arriving - leaving).abs() / peak;
            assert!(
                corner < 3.0e-3,
                "{}: the waveform arrives at the seam sloping {arriving} and leaves sloping \
                 {leaving} — a corner of {:.5} of peak per sample",
                recipe.label(),
                corner
            );
        }
    }

    /// Nothing clips, and nothing is silent — a recipe whose amplitudes cancelled would render as
    /// a flat line and every other test here would still pass.
    #[test]
    fn every_recipe_is_audible_and_within_headroom() {
        for recipe in Recipe::ALL {
            let frames = render(recipe);
            let peak = frames.iter().fold(0.0f32, |peak, v| peak.max(v.abs()));
            assert!(
                (peak - PEAK).abs() < 1.0e-3,
                "{} normalised to {peak} rather than {PEAK}",
                recipe.label()
            );
            let rms = (frames.iter().map(|v| v * v).sum::<f32>() / frames.len() as f32).sqrt();
            assert!(rms > 0.02, "{} is nearly silent: RMS {rms}", recipe.label());
            assert!(
                frames.iter().all(|v| v.is_finite()),
                "{} produced a non-finite sample",
                recipe.label()
            );
        }
    }

    /// The morph actually moves. A recipe whose amplitude law ignored `t` would pass everything
    /// above and be a static waveform wearing a sweep's name.
    #[test]
    fn every_moving_recipe_actually_changes_across_the_sample() {
        for recipe in Recipe::ALL {
            if recipe == Recipe::InitSaw {
                continue;
            }
            let partials = recipe.partials();
            let moved = partials.iter().any(|partial| {
                let start = recipe.amplitude(partial, 0.0);
                let middle = recipe.amplitude(partial, 1.0);
                (start - middle).abs() > 1.0e-3
                    || (recipe.phase(partial, 0.0) - recipe.phase(partial, 1.0)).abs() > 1.0e-3
            });
            assert!(moved, "{} does not move at all", recipe.label());
        }
    }

    /// The name carries the root, and the naming code the editor already runs reads it back as
    /// exactly the note every generated source plays at.
    #[test]
    fn every_name_parses_back_to_the_declared_root() {
        for recipe in Recipe::ALL {
            let name = recipe.name();
            let key = crate::naming::key_in_name(&name)
                .unwrap_or_else(|| panic!("{name} carries no key for the editor to read"));
            let root = crate::naming::root_from_key(key, None);
            assert!(
                (root - ROOT).abs() < 1.0e-3,
                "{name} reads back as {root} rather than {ROOT}"
            );
        }
    }
}
