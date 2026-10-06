# NOTES.md — crates/mxm-creative-sampler-dsp/

The detail behind this folder's AGENTS.md: history, measurements, rationale and worked
examples. AGENTS.md is the contract; this file is the reference it links to.

## Contents

- [Modulation is routed, and this crate declares what the instrument can route](#modulation-is-routed-and-this-crate-declares-what-the-instrument-can-route)
- [The amplitude target is the collection's factor; every other target sums](#the-amplitude-target-is-the-collections-factor-every-other-target-sums)
- [The performance sources are the collection's standard](#the-performance-sources-are-the-collections-standard)
- [A modulation source that is audio is projected, not summed](#a-modulation-source-that-is-audio-is-projected-not-summed)
- [Three readers are three intentions](#three-readers-are-three-intentions)
- [The splice is aligned, and that is the whole reader](#the-splice-is-aligned-and-that-is-the-whole-reader)
- [The period narrows the search, and the window stayed fixed](#the-period-narrows-the-search-and-the-window-stayed-fixed)
- [Alignment and traversal invariants](#alignment-and-traversal-invariants)
- [The onset map is part of constructing a Sample](#the-onset-map-is-part-of-constructing-a-sample)
- [Two kernels, and why only one is tabled](#two-kernels-and-why-only-one-is-tabled)
- [Character is source-domain, and its converter is one per voice](#character-is-source-domain-and-its-converter-is-one-per-voice)
- [Naming the note a sample is playing](#naming-the-note-a-sample-is-playing)
- [Bounds, levels and idleness](#bounds-levels-and-idleness)
- [A loop is the half-open span from Loop start to one frame past Loop end](#a-loop-is-the-half-open-span-from-loop-start-to-one-frame-past-loop-end)
- [A forward loop's seam is a property of the frames read](#a-forward-loops-seam-is-a-property-of-the-frames-read)
- [Two normalisation laws, because coherent and incoherent overlap do not sum the same way](#two-normalisation-laws-because-coherent-and-incoherent-overlap-do-not-sum-the-same-way)
- [Voices: the budget, note ownership and stealing](#voices-the-budget-note-ownership-and-stealing)
- [The keyboard: one ledger, three modes, and what a release means](#the-keyboard-one-ledger-three-modes-and-what-a-release-means)
- [Routing and expression ownership](#routing-and-expression-ownership)
- [Recorded pre-listening measurements](#recorded-pre-listening-measurements)
  - [Stretch, after the splice was aligned (2026-09-12, same machine, three runs)](#stretch-after-the-splice-was-aligned-2026-09-12-same-machine-three-runs)
  - [What the owner's first listening round found, and what it cost to see](#what-the-owners-first-listening-round-found-and-what-it-cost-to-see)
  - [S2, and the metric that was needed to grade it (2026-09-12)](#s2-and-the-metric-that-was-needed-to-grade-it-2026-09-12)
- [The grain is not expensive; building the kernel twice a microsecond is](#the-grain-is-not-expensive-building-the-kernel-twice-a-microsecond-is)

## Modulation is routed, and this crate declares what the instrument can route

`src/routing.rs` is the instrument's own half of
`plans/plan-modulation-routing.md` (in the private archive): a **source list**, a
**target list**, each target's **full scale per route**, and the routes the init patch wires.
The shared machinery is [`mxm-modulation`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-modulation/AGENTS.md), which this crate takes as
its **one dependency** — the routing half only, zero-dependency and MSRV 1.87, so this crate does not
inherit the 1.95 GUI floor. `cargo tree -p mxm-creative-sampler-dsp` is the check.

**Eleven sources, five targets.** Order is load-bearing: a route whose source comes earlier reads
this sample's value, later reads last sample's — the unit delay that makes a player-made cycle
finite. **The two layers' audio is declared last**, so every route from it is a backward route, one
sample late, and combs at audio rate.

The engine evaluates both global LFOs once per sample and publishes each value into every active
voice frame, as §4.1 permits for global sources read by voice-scoped targets. Per-channel wheel and
bend are projected into the owning voice’s frame. **No third frame scope exists**, and shared
combination laws remain unchanged for compatibility with the pilot’s pinned digests.

Three rules the per-sample path depends on:

- **A source nothing reads is not published.** `Routing::needs` gates every write, and the saving is
  per voice, so it is paid eight times over. **The gate owes a clear**: `Graph::set_topology` zeroes,
  in each voice's frame, a source that has just become needed, or a re-added backward route reads
  the tap left from before the gap (mxm-kit's [`crates/mxm-modulation/AGENTS.md`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-modulation/AGENTS.md), *A gated publication owes a
  `clear`*). `a_re_added_backward_route_reads_zero_rather_than_the_tap_from_before_the_gap` holds
  it.
- **Topology is compacted once on the engine**, never once per voice, and a **started voice is armed
  from it** — `Voice::start` resets the graph, so a note allocated after the last topology change
  would otherwise render with nothing routed at all.
- **A voice frame ceases with its voice**, so a reused slot cannot read a dead voice's latched key or
  velocity.

## The amplitude target is the collection's factor; every other target sums

**Not a stylistic choice.** §4.1 requires the target that ends a voice's life to take its routes as a
factor on the envelope: a latched source *added* to the gain keeps a voice audible after its envelope
ends, so ending the voice on the envelope cuts a non-zero signal and ending it on audibility makes
the voice immortal.

**It is `mxm_modulation::standard::amplitude_factor`**, `1 + clamp(Σ, ±1)` — the collection's one
amplitude law (`plans/plan-modulation-standard.md`, 2026-09-26) — where it was a product of
`1 + amount × (source − 1)` factors. **For Velocity the two are the same number**: published as the
standard's `v − 1`, one route's factor is `1 + sense × (v − 1)`, which is the velocity-sensitivity
law `1 − sense × (1 − v)` term for term, so the route that replaced that knob still renders the
identical sample at the same number (`the_velocity_route_is_the_velocity_knob_it_replaces`). Every
factory design's amplitude routes are that one and LFO 2 at zero, so none moved. What changed is any
other source into Amplitude: a bipolar LFO is now a tremolo either side of the level, silence to
double, where the product made it a dip only.

## The performance sources are the collection's standard

Key (the glided key), Velocity, Wheel, Pressure, Bend and Random are published through
`mxm_modulation::standard`, each zero at its rest. **Velocity is the press that last triggered the
envelope**, `Voice::velocity`: a Legato press that only re-pitches keeps the phrase's, where the
voice used to publish the new note's. Routes this sampler wired before routing (`routing::MACHINE`)
keep their reach; every other route takes the standard's — Pitch ← Key is twelve semitones per octave
of keyboard, where the pitch column's twelve per unit made it 2.4. `conformance.rs`'s `Declared`
runs the standard's checks, each falsified once, and the plugin's tests reuse it through the
`conformance` feature.

## A modulation source that is audio is projected, not summed

The frame carries one bounded scalar and a layer produces stereo, so an audio source needs a **tap**
and a **projection**, and both are decisions rather than details:

- **Tapped at the reader's output**, before that layer's level and pan and before the voice's
  converter — a route whose depth changed when the player rebalanced the mix would be a defect, and
  the converter is a deliberate nonlinearity that does not belong in a control signal.
- **Projected through `Sample::mono_mix`**, never `(L + R) / 2`. That sum is **silence on an
  anti-phase source**, which this crate has already met once: the alignment search and both analyses
  would have declined on perfectly valid audio, which is why `Sample` picks its mix at load.

## Three readers are three intentions

- **Repitch** advances source position with key pitch and reads through a sixteen-tap windowed-sinc
  kernel in Clean mode. It changes duration with pitch. The kernel comes from a table at or below
  the recorded rate and is computed above it — see *Two kernels, and why only one is tabled*.
- **Stretch** is deterministic dual-head overlap-add, **aligned to the waveform rather than to a
  clock** — see *The splice is aligned, and that is the whole reader*. Travel advances independently
  of note pitch; within-window read rate follows pitch. Zero travel holds a region and negative
  travel reverses it.
- **Grain** schedules into a share of the instrument-wide [`GRAIN_BUDGET`]. Onset, position, pitch,
  direction and pan are latched from the voice's seeded stream. Density is an onset rate, and what
  was once one control is now four, each named for one behaviour: **pitch variation** detunes,
  **position variation** draws the read position away from the playhead, **onset timing** hands the
  schedule over from periodic to stochastic, and **stereo width** and **reverse chance** are the
  thrown-about character. Position variation and onset timing were one knob, which is why the
  normalisation law had to guess which of the two a setting meant. Welding any of them
  together meant a cloud could not be thickened into a section of players without also being thrown
  about. `spread_and_scatter_are_independent` and `scatter_decorrelates_without_panning_or_reversing`
  hold them apart. **Travel advances per sample, on Stretch's law and in the same units**, and a
  spawn only latches it. Accumulating it per spawn instead reduced to the dimensionless overlap
  ratio rather than an interval, and sat after the pool-full early return, so the playhead crawled
  (1/2700 rate at default size and density) and stopped entirely once the pool saturated — the
  dense setting. Both halves are held by
  `grain_travel_traverses_the_region_and_does_not_stall_on_a_saturated_pool`, which now asserts its
  own saturation: a budget that grows out from under that case would leave the regression untested.

All start audibly on note-on. None performs analysis, allocation or preparation while rendering.

## The splice is aligned, and that is the whole reader

**The owner's report was that Stretch sounded poor; the cause was that a head re-anchored on a
clock.** `plans/plan-stretch-quality.md` S1. Whenever pitch and scan differ — every transposed note
— the outgoing and incoming heads met at an arbitrary relative phase, so every window boundary was
a partial cancellation, and the boundary was periodic, so it was a comb that rebuilt itself at the
window rate. Measured on a twelve-harmonic tone as the sideband energy that splice puts either side
of each partial: at +7 st the sidebands were **9.4 dB above the harmonics themselves**.

Four things answer it, and they are separable:

- **A re-anchoring head searches for its splice**, taking the offset whose normalised correlation
  best continues what the other head is still producing — **WSOLA**, Verhelst and Roelands, ICASSP
  1993. Coarse-to-fine, because a full-resolution sweep measured the same answer for an order more
  work.
- **Ties go to zero offset, by a margin rather than by a strict `>`.** A tie is never exactly a tie
  in floating point: on a periodic source, zero and one period are the same alignment and rounding
  decides between them. The first version of the search picked *one period away from a unity read*
  for that reason alone. `ALIGN_TIE_MARGIN` sits far above float noise and far below a real
  difference in alignment.
- **A transient re-lays both heads onto the playhead**, one at the start of its envelope and one at
  the peak of its own, so the pair stays complementary and the attack is read from its first frame
  by a head that has just started. `Sample` carries the onset map (below).
- **A head carries its read position directly** instead of reconstructing it from `anchor + age ×
  rate`. The two are algebraically the same and only one of them was true at note-on: the second
  head starts half a window into its envelope, so the reconstruction had it reading **half a window
  ahead of the playhead** for the whole first window. The reader was not an identity at unity even
  before a note was transposed.

**Unity is an invariant, not a bypass.** At root pitch, unity scan and forward direction the engine
still runs and its output *is* the plain resampled read;
`stretch_is_the_plain_read_at_root_pitch_and_unity_scan` holds it against Repitch through the same
envelope, filter and master. A skip-the-work bypass was designed and rejected: **equal output is not
equal state**, so leaving unity would start the engine cold, and pitch glides and the Travel LFO
cross unity constantly. The residue is one sample in 24 000 at one ULP, which Character's deliberate
16-bit Clean floor turns into exactly one converter step — 1/32768 through an 0.82 envelope and an
0.8 master is 2.002e-5, the difference the test measures to every digit.

**The degenerate cases are decided, not inherited** (plan §3a). The time-scale factor is a positive
magnitude and the traversal sign lives once, on the playhead and the read step. **Layer Reverse and
negative Scan are different controls**: Reverse turns the playhead *and* the read around, negative
Scan turns only the playhead around, so windows read forwards while marching backwards. **Hold
freezes the playhead and keeps searching**, so it is an aligned loop of the neighbourhood rather
than one buffer repeating; transient anchoring is inert there as a consequence rather than as a
special case, because a frozen playhead crosses no markers. **The search is clamped to the region**,
which is what keeps the bounds contract true for a reader that looks before it commits.

**Two defects were repaired with it.** `stretch` hard-coded `LoopMode::Forward` for its reads and
now uses the layer's own; and head positions wrap in the reader's float domain rather than relying
on `frame()`'s integer wrap, because the two disagree by up to a frame — which is inaudible alone
and is exactly the difference between the identity at unity being true and being nearly true. The
looping case of that test measured 9.6e-4 against a 1e-5 tolerance before the wrap moved. The
disagreement itself is gone since: every wrap and every tap now uses the loop's half-open span (*A
loop is the half-open span from Loop start to one frame past Loop end*, below).

## The period narrows the search, and the window stayed fixed

**S2 was specified as period-informed *windows* and shipped as a period-informed *search*, because
that is what the measurement said.** `plans/plan-stretch-quality.md` S2.

`Sample` carries a period track beside the onset map: YIN again — the same difference function and
cumulative-mean normalisation `detect_root` uses — on a four-times-decimated copy, every
[`PERIOD_TRACK_HOP_S`], answering in source frames. It **declines** exactly as `detect_root` does,
and a zero means the reader keeps S1's blind range, which is what leaves noise, drums and chords
untouched.

**What the period is for.** Alignment repeats every period, so exactly one good offset lives in any
half-period either side of the natural position, and a wider search can only find the same answer a
cycle away — displaced in time, heard as the material jumping. Narrowing is therefore a quality
change first and a saving second, and it measured as both.

**Two things it is deliberately not used for.**

- **Not the window length.** Setting the window to a fixed number of cycles is the textbook move and
  is what the plan specified; it was built, measured and rejected. Measured as error against a
  synthesised ideal (below): a period-derived window is **worse than a fixed one at 55 Hz** and
  dearer everywhere, because a shorter window re-anchors more often and every re-anchor is a splice.
  A sweep of 1.5 / 2 / 3 / 4 cycles moved the 55 Hz figures by under a decibel, which is the reading
  that says the window was never the limiting variable there.
- **Not near an attack.** A period is a statement about periodic material, and across an onset the
  source is not periodic; narrowing onto the sustain's period there leaves the aligner unable to
  reach the offset the attack needs. Held blind within one window of a marker — without that, attack
  sharpness fell from 5.81 to **4.77**, below even the clocked splice S1 replaced.

**A head carries a phase rather than an age over a duration.** The two are equivalent while the
window is fixed and only the phase form survives a window that changes under a sounding note, since
both heads advance at the same rate and keep the half-window offset that makes the Hann pair sum to
one. The variable window was rejected; the representation stays because it is the simpler of the two
and it is what let the experiment be run.

## Alignment and traversal invariants

Reader quality probes do not cover the callback and traversal edge cases below; each remains part of
the implementation contract.

- Voice/layer start phases are deterministic but staggered while each head pair remains half a
  window apart. Shared onsets must not collapse a chord back into lockstep. Attack sharpness is
  therefore reported as a distribution.
- In `LoopMode::Alternate`, the playhead reflects and each head keeps its own read direction.
- Refined onsets are sorted before `seek_marker` binary-searches them.
- `traversal_gap` folds distances across a loop seam. Replay/near-attack comparisons use one named
  domain; source-frame spans are never compared directly with host-sample windows.
- `Sample` chooses its analysis mono mix once at load; simple `L + R` is forbidden because it
  silences an anti-phase source.
- [`ALIGN_IDENTITY_EPS`] is distinct from [`ALIGN_TIE_MARGIN`]; identity tolerance must not suppress
  sub-sample refinement.
- `walk_traversal` follows the actual path, splitting at loop seams and Alternate turns. It re-seeks
  at each monotonic segment, coalesces multiple passages within one host sample into one splice, and
  re-seeks without reporting a crossing after a jump or exhausted [`TRAVERSAL_SEGMENTS`] budget.
- Reverse traversal beginning on a seam must advance. Region or loop-mode changes remap the playhead,
  re-seek the cursor and discard pending splices before the next read.
- Alignment work is bounded independently of source rate. Refinement narrows in fixed-probe levels
  using [`ALIGN_COARSE_PROBES`]; the accepted 768 kHz-source/8 kHz-host extreme remains covered.
- Instrumentation counters used by parallel tests are thread-local.

The direct guards are `no_single_sample_runs_more_than_a_few_alignment_searches`,
`a_marker_splices_once_per_passage`, `alternate_leaves_the_head_read_direction_alone`, the sorted
onset property, anti-phase analysis, bass detection, swept-region traversal and extreme-rate burst
regressions. Tests construct regions and cursors through production constructors; hand-built state
must not make an oracle pass on an unreachable case.

## The onset map is part of constructing a `Sample`

**Analysis completes before publication and there is no fallback**, because a published `Sample` is
immutable while any reader can see it: analysis arriving late would need a second publication, a
revision counter and a policy for voices already sounding from the first. Doing it in `Sample::new`
means a note can only reach a sample whose analysis is complete, and the plugin's existing
stage/commit barrier already keeps new audio from being heard through the old patch.

Energy-based onset detection with an adaptive threshold — Bello, Daudet, Abdallah, Duxbury, Davies
and Sandler, *A Tutorial on Onset Detection in Music Signals*, IEEE TSALP 13(5), 2005. The detection
function is the log-energy rise of a first-differenced mono sum: the first difference is a one-tap
high-pass, there because an attack is broadband while the sustain it interrupts usually is not, and
the logarithm is what makes the function a *ratio* so one threshold constant works at any mastering
level. **It declines rather than guesses** — silence, a steady tone and a source too short to have a
neighbourhood all return nothing, the same standard `detect_root` is held to, because a wrong marker
re-lays the overlap on material with no attack in it. It is **derived on load and never serialised**:
every user patch embeds its audio, and a cache in the state blob would grow the patch.

**Every buffer the analysis sizes from the source is reserved fallibly** (audit D14): the onset
function, the onset list (counted first, so it is held at its length), the decimated copy and the
period track come from `reserved`, and a refusal returns `SampleError::Allocation` instead of
aborting — the plugin builds a `Sample` from restored state, presets and imports, where an abort is
the host's. The lag buffers are sized by the rate and stay infallible. `tests/allocation_refusal.rs`
refuses each of the four in turn through a refusing global allocator; an infallible one aborts that
binary. All of it happens off the audio thread. Markers are walked with a per-layer cursor rather than
searched, because the playhead moves by well under a frame most samples and a binary search per
sample per voice is real money for an answer that is almost always *no*.

## Two kernels, and why only one is tabled

`sinc_read` has two paths, chosen by the band limit its caller derived as `1 / max(rate, 1)`:

- **At or below the recorded rate** the cutoff is exactly 1.0 and the kernel is one fixed shape.
  `UNITY_KERNEL` holds it at 512 fractional phases, normalised per row at build time, and a read
  interpolates two adjacent rows. Both rows sum to one and summing is linear, so the interpolated
  row sums to one too — **the normalising division goes away with the coefficients**. Rounding to
  the nearest row instead would be a ±1/1024-sample fractional-delay jitter, which is noise well
  above this reader's measured alias floor; the interpolation is what makes the table honest.
- **Above the recorded rate** the cutoff narrows with pitch, so no single table describes it. That
  path keeps the computed kernel exactly as it was, and keeps its −45.0 dB measurement with it.

**`SINC_TAPS` is sixteen, and eight was measured and rejected** — the alias rejection falls to
−22.1 dB, which is not worth three points of CPU. Do not take that number from a general granular
reference: it is true of a *fixed-cutoff* interpolator and false of this one.

Both tables, and the shared window, are built by `LazyLock` and **forced in `Engine::new`** so a
callback never runs an initialiser. They live in static arrays, so nothing allocates.

## Character is source-domain, and its converter is one per voice

**The contract moved on 2026-09-10, and the sound moved with it** (owner: *"it changes the sound —
that is ok. We are still exploring the architecture."*). Character is two things and they now sit in
two places:

- **The address generator stays in the reader**, per read: virtual-rate hold and reconstruction are
  about *how the source is read*, they need the read position, and they transpose with the note
  because the grid is measured in source frames.
- **The converter is one per voice**, applied to the summed layers, between the digital section and
  the filter — where the machines this borrows from put their DAC. Quantisation and companding were
  inside every grain read, so sixteen live grains were sixteen independent converters inside one
  voice.

**Quantise-then-sum is not sum-then-quantise**, and that is the whole audible content of the change:
sixteen grains each rounded to their own grid, then windowed, panned and summed, gave sixteen
uncorrelated error signals that averaged toward smoothness. One converter on the sum gives the
stepped edge the hardware had. `a_voice_playing_many_grains_lands_on_one_converters_grid` is the
test that tells the two designs apart — with the converter back inside the read it fails by half a
cell, the maximum possible.

**It changes nothing where only one read is summed.** Repitch with a single layer converts the same
signal either way, which is why the recorded Repitch Character figures below did not move. It also
costs less: 35.8% → 32.7% of a core on the dense probe.

`character = 0` is Clean, and the 16-bit floor there is deliberate. The performed axis opens hold,
converter and reconstruction together, while Rate, Converter, Reconstruction and Input retain direct
control. Intentional hold images and low-resolution aliasing are labelled character; non-finite
output, out-of-bounds reads and unstable recursion are defects.

**One part of the owner's reading was not taken.**
`plans/handover-2026-09-10-creative-sampler-grain.md` §4.2 says the emulated clock should
transpose and does not. Reading the code, it already does: `grid` is measured in **source** frames
and `position` advances at the playback rate, so a hold of `grid` source frames lasts `grid / rate`
output samples and its images sit at multiples of `rate / grid` — they move up with pitch, exactly
as a sampler's do when transposing raises its clock. Making the grid fixed in *output* time would
freeze the artefacts against pitch, which is a modern bitcrusher after the resampler rather than the
hardware. Left alone deliberately; reopen it with a measurement rather than a reading.

## Naming the note a sample is playing

**`detect_root` exists because Root otherwise asks the player to guess.** It defaults to a number,
the sample is whatever it is, and the two agree by luck; finding the truth means auditioning the
keyboard against the source by ear. The Auto button asks the question the control is really asking.

**YIN**, not plain autocorrelation. Autocorrelation peaks at lag zero and prefers longer lags, so it
octave-errors *downward* on exactly the harmonically rich material a sampler is pointed at.
Difference function, cumulative mean normalisation — the step that removes that bias — an absolute
threshold at 0.20, and parabolic interpolation on the winning lag, because a whole-sample lag near
2 kHz is already a third of a semitone at 48 kHz.

It reads from the **region start**, not frame zero, so pointing Start into the middle of a sample
tunes to what is there. It skips leading silence and then 20 ms more: a struck or plucked onset is
inharmonic for a few milliseconds and will report a fifth if it is allowed to.

**It declines rather than guesses.** Silence, noise, a drum and a chord return `None`, because a
wrong Root retunes the whole keyboard on a decision the player never made. The search runs 30 Hz to
5 kHz — below a five-string bass, above a piccolo — and widening either end buys octave errors.

Held by four tests over C2 to C6, a bare sine against a 24-partial tone, a two-note sample read from
each half, and silence/noise/too-short. A fifth lives in the plugin: the generated init saw declares
C3 from its frame and cycle counts, and `the_generated_source_is_heard_as_the_root_it_declares`
derives the same answer from the audio. Two derivations meeting is worth more than either alone.

## Bounds, levels and idleness

Every read clamps or wraps inside the projected source region; one-frame and microscopic regions are
valid. A missing layer is silent and costs the other nothing; the layers sum, and each Level is an
actual level reaching 200%. The final envelope reaches exact zero, then its voice is reset and
performs no reader/filter work. Reset and panic remove every head and grain. `Engine::panic` also
returns the per-channel wheel, bender and channel pressure to neutral, which is what a reset and an
activation want; a caller whose panic should end notes rather than controllers (the plugin's CC 120
and CC 123) brackets it with `Engine::performance` and `Engine::restore_performance`.

**"Valid" means a one-frame region must render, and `f32::clamp` is how that stops being true.**
`clamp` asserts `min <= max`, so a bound pair that can cross is a panic rather than a degenerate
number — and a panic cannot unwind across the CLAP boundary, so it takes the host down. Stretch's
search bound was `clamp(1.0, region.len() * 0.5)`: `Region::len` floors at one frame, so closing
Start onto End left half a frame against a one-frame floor and aborted the host. **Where either
bound is computed, write `min` then `max` and put the floor last**, which also answers NaN by
returning the other operand instead of asserting. The bounds that are provably ordered —
`Region`'s own `start`/`end` and `loop_start`/`loop_end`, which `Region::new` sorts — may stay as
`clamp`. Held by `a_region_with_no_room_in_it_still_renders` and by
`no_random_parameter_combination_panics_the_engine`.

## A loop is the half-open span from Loop start to one frame past Loop end

**Loop end names the loop's last frame, as a WAV loop does** (owner, D1, 2026-09-14;
`plans/plan-sampler-wav-loop-import.md` §1.1, in the private archive). The loop
occupies `Loop start, Loop end + 1)`, and its period is `P = Loop end + 1 − Loop start`, fractional
when the points are. `Region::loop_span`, `loop_limit` and `wrap_loop` compute it in one place, and
every wrap reads them:

- Repitch's position (`advance_position`), Stretch's travel fold and marker walk (`traversal_gap`,
  the splice span) and Stretch's heads (`wrap_read`) wrap a position past the span to `x − kP`;
- the kernel's and the hold path's taps wrap through the seam (`Seam::value`) by the same `P`,
  interpolated where that lands between frames;
- a crossfade's fade-out ends at `loop_limit`; its return point is Loop start plus the fade, its
  period `P − fade`, and its ceiling half of `P`;
- the latch's room is `P − fade`, and `reaches_loop` and `approach` measure against `loop_limit`.

**Alternate reflects where it did.** Its position and travel still turn at Loop start and Loop end.
Only its forward-style wraps take the span: Stretch's heads, and taps past either end. On whole-frame
points that leaves a tap's wrap unchanged and moves a head's by a frame. **Grain is untouched**: it
reads `Region::new` and never the loop.

**Why.** The float wraps used `loop_end − loop_start` while `frame`'s taps wrapped over the inclusive
integer span, one frame more. That was recorded as inaudible, and was, on every buffer the tests used.
On a single-cycle wave — the owner's JD-800 set loops 169 frames over 0..168 — the positions played
168 frames a pass: ten cents sharp, a frame skipped each cycle, and the taps disagreeing with the
position about where the loop was.

**Loop points land on frames, and only on the frames they came from.** A file's loop travels through
a normalised `f32`, which cannot place every frame of a long file. `settle_on_frame`, in `Region::new`,
lands a point on frame `k` only when its normalised value is bit for bit `k / last` — what writing
that frame produces — so a file's loop plays its own frames rather than interpolating every wrapped
tap, and a deliberate fraction stays where it lies. The first version settled anything within
`last × ε` of a frame, which at the importer's cap is 0.69 of a frame and rounded every point there
(code review round 1).

**Equal points are a one-frame loop.** `Region::new` once widened equal points by a frame, from when
the span was their difference; under the half-open span they loop the one frame they name, through
the whole-frame fetch, which wraps by that same frame.

**Tests:**

- `a_whole_frame_loop_repeats_every_frame_it_names`: Repitch and Stretch on a 169-frame cycle and on
  a loop ending where loud material begins. At root the output repeats every 169 samples; a semitone
  down, no read takes the loud material.
- `a_loop_between_frames_wraps_every_reader_by_one_span`: fractional points over a ruler, whose frame
  `i` holds `i`. Every wrapped tap reads `index − kP`, and with whole-frame points the taps are the old
  integer wrap, in Forward and Alternate.
- `loop_frames_survive_the_normalised_parameter`: loop frames read back as those frames at every
  length the importer accepts, and a deliberate fraction low, middle and high in each file is not
  moved.
- `equal_loop_points_loop_the_one_frame_they_name`: equal points make a one-frame span, and a render
  at root over frames alternating ±0.5 reads one value.

**Falsified by mutation** (2026-09-14): positions wrapping by the old period fail both loop tests — a
head at 302.125 wraps to 101.75, not 100.75, and one cycle looped whole no longer repeats every 169
samples; taps taking the integer wrap at no fade fail the fractional one (tap −40 reads frame 162,
not the loop signal at 161.375); without `settle_on_frame` a loop over frames 6..111 of 169 reads
back from 6.0000005; settling by nearness moves a start placed at 6.25 of 5 760 000 frames to 6.0;
and widening equal points again makes the one-frame loop (1 000, 1 001).

**What it cost, accepted by the owner:** every forward loop is a frame longer. Of the factory renders
(`examples/measure_baseline.rs`, release, 2026-09-14) only `Long bass` moves, `16402832d6d650e1` →
`243518671b9d4735`; `Held pad`, `Struck hit`, `Drifting texture` and Init render as before.

## A forward loop's seam is a property of the frames read

**The loop crossfade is built under the kernel, not beside the readers**
(`plans/plan-sampler-loop-crossfade.md`, in the private archive). In a forward
loop with a fade, every fetch — the sixteen taps, Character's hold path, Stretch's alignment search —
reads a **virtual loop signal**: the recording over the effective loop, each frame inside the fade
blended with the frame exactly one loop period away, and the whole extended periodically across the
seam. A periodic signal has no seam of its own, so Repitch either way round, Stretch's heads and
negative Scan all cross it alike, with no per-reader logic, no state and no allocation. `Seam` holds
the geometry and `Region::looped` fits it; `Region::new` stays seamless.

**The faded-in material comes from inside the loop, and nowhere else** (owner, 2026-09-14;
`Seam::fit`). The fade that ends one frame past Loop end blends in the loop's own opening, and after the seam the
loop returns to **Loop start plus the fade**: each pass is shorter by the fade, and the frames it
skips are heard as the fade-in. What precedes Loop start is a note's attack — the owner's case is a
guitar string looped for sustain — and the first version, which took room outside the loop first and
borrowed from inside only for the shortfall, faded that attack into every pass. A fade and the loop
it shortens cannot overlap, so the ceiling is half the loop. Every quantity is continuous in every
input.

**A note plays from Start, and a loop begins only where it begins.** Until 2026-09-14 every read in a
looping mode wrapped into the loop: `frame` and `Walk` folded any index below Loop start into it,
Stretch folded its heads and playhead with `wrap_read`, and Grain read through the player's loop
points. Everything before Loop start was replaced by loop material from a note's first sample, with
or without a fade, and it went unseen because every shipped patch loops its whole region. Now:

A reversed note plays from End in the same way, and its loop begins at Loop end.

**Each reader latches onto the loop: `LayerVoice::looping` for the playhead, `OlaHead::looping` for
each Stretch head.** Until its latch is set a reader reads the recording exactly as a one-shot note
would (`loop_read` with `LoopMode::Off`); once set, it reads and wraps through the loop as it always
has (`wrap_read`).

- **`reaches_loop` sets it once the reader is inside the loop by more than a read reaches** — half the
  kernel plus Character's hold cell and clock wander (`loop_reach`) — and, reversed, past the fade too.
  There a looped read and a plain one fetch the same frames. A wrap or turn also sets it, for a loop
  shorter than that reach.
- **Then the reader crosses into the loop over `LOOP_ENTRY_S` (2 ms, `Crossing`).** It reads both the
  recording, carried on unwrapped as the one-shot note would have played it, and the loop, and
  fades between them. The margin is capped to fit the loop, so some loops have no place where the two
  reads agree: a loop shorter than twice a read's reach, or a fade at half the loop, which leaves no
  room. There latching once switched every read in one sample, a step a code review found. **Where
  the two reads agree, the fade is exact to the bit**, so a loop long enough renders as if nothing
  crossed. A head anchored with its window closed needs no crossing.
- **A loop with nothing before it on the approach side is reached on the first sample** (`reset_layer`),
  so a whole-region loop **with no fade** takes none of this. All four factory recipes rendered the
  digests recorded on 2026-09-13 (`examples/measure_baseline.rs`, release), and three still do; the
  half-open loop model (above) moved `Long bass`'s. **A whole-region loop
  with a fade starts unlatched, by design.** The fade moves the return point to Start plus the fade,
  so a note plays the fade-in material from Start as recorded. Reversed, it plays the fade-out zone
  from End as recorded, since the latch waits until the reader is past the fade. That is the owner's
  ruling applied, not an exception to it.
- **A re-anchored Stretch head inherits the playhead's latch, and the alignment search scores each
  series in its own head's mode.** A head the search places just before Loop start after a wrap reads
  the loop's end, not the attack; the playhead latches with a search width to spare, so no head
  anchored from it reaches back out of the loop.
- **Stretch's playhead walks like a one-shot's until it latches.** A zero Scan holds it at Start (or
  End), and markers are walked. Alternate's out-and-back distance is taken on its rising half. A step
  that crosses a whole short loop walks the recording to the far end before folding the remainder,
  so an attack passed on the way still splices. A playhead heading away from a loop still ahead —
  negative Scan from Start — folds in, as it always did.
- **Grain reads loop-blind**, wrapping at the region's edges, which is all its `LoopMode::Forward`
  ever meant.

**Rejected: deciding by position alone** (the first repair, `within_loop`). A position cannot tell a
first pass approaching Loop start from a head anchored a few milliseconds before it after a wrap, and
the second must read the loop; the search and the render also disagreed about a head straddling Loop
start. It had also missed `Held pad`'s heads anchored outside a whole-region loop until the digests
caught it.

**Tests:**
- `a_looping_note_plays_what_precedes_the_loop_exactly_as_a_one_shot_does`: Repitch and Stretch,
  forward and reversed, in Forward with and without a fade and in Alternate, match the one-shot note
  bit for bit. Stretch is compared until its kernel reaches the loop. Repitch is compared through the
  entry, until its kernel reaches the loop's far end or the fade there, which is the no-click claim.
  It renders a semitone down: at unity a read on a whole frame weights every tap but one zero, and a
  latch with no margin passed. A held Stretch playhead in either direction holds across a moved loop
  point. Grain plays the same whatever its loop points. The test failed at sample 0 on the code
  before.
- `a_head_anchored_before_loop_start_after_a_wrap_reads_the_loop`: requires every Stretch head to be
  inside the loop on every sample once the first pass is over. It uses a 1 000-frame loop of a 100 Hz
  tone, transposed both ways. At unity the heads sit on the playhead and never leave the loop. A level
  check alone missed the defect, because a head is anchored with its window closed.
- `an_attack_crossed_on_the_way_into_a_short_loop_is_still_spliced`: covers the whole-loop step.
- `a_loop_too_short_for_a_read_is_entered_without_a_step`:
  - **Setup:** a 14-frame loop of +0.5 between −0.5, read through a 16-frame Character cell with a
    steady and a wandering clock. The source's detected onsets must lie at least 1 000 frames from
    the loop, where a Stretch splice re-lays heads that all read one level.
  - **Cases:** Repitch and Stretch, forward and reversed, with no fade and at the half-loop fade.
  - **Bound:** entering may add no step beyond what any read of that recording can make — a
    full-scale change across the narrowest cell interval, the jump a wandering instant makes where
    the cell changes — plus a 32-sample transition.
- The first-pass test also checks the whole-region fade case: latched at note-on without a fade, not
  latched with one, and matching the one-shot through the entry.
- `a_reversed_playhead_entering_an_alternate_loop_keeps_descending`: covers the out-and-back distance
  taken at entry. Revision 9 took it on the falling half, where a reversed playhead's step raises it.

**The placement answers two hazards.**

- A partner fetched *through* the wrap lands on the frame it should differ from, and the fade renders
  nothing — so a blended frame is two fetches of the raw, region-clamped recording, and nothing a
  blend reads is itself blended. `a_crossfade_removes_the_step_at_a_seam_that_does_not_close` fails
  with the partner collapsed.
- A blend placed *above* the interpolator must stop the taps wrapping once the fade is non-zero, which
  switches regime as the fade leaves zero. `a_crossfade_converges_to_the_hard_wrap_as_it_shortens`
  holds the convergence with Character's hold path off and engaged, and **requires the distance to
  scale with the fade below one frame and not to jump across it** — a test that only asked for
  smallness passed an implementation that skipped the seam below one frame, which moves the regime
  switch to one frame rather than removing it. That bypass fails the test (distances `[0, 0, 0]`). A
  position-level blend itself is a rewrite rather than a mutation, and was not run.

**The period is exact, and a hard wrap is the seam with no fade.** Loop points are fractional
and automatable, and a period rounded to whole frames would move every blended frame onto its
neighbour at each crossing. So the wrap is arithmetic on the index, and a fetch that lands between
frames interpolates the two raw frames around it. `a_blended_frame_moves_smoothly_as_the_loop_end_slides`
fails with the period rounded — **0.0122 against a 1e-3 bound** — and the output-level test first
written for that claim did not: a one-frame partner shift is an ordinary-sized step at the output.
What the output can prove under automation is
`sweeping_the_loop_and_the_fade_under_a_note_neither_clicks_nor_strays`: Loop start, Loop end and the
fade swept alone and together through the point where borrowing begins, and the fade up from zero,
for Repitch and Stretch, with every read position checked against the region each sample — because
the fetch clamps, and a stray read renders finite. **That check only bites when the region ends
within a Stretch window of the loop**: with room to spare, removing the heads' wrap altogether left
every head inside, and the region was tightened until that mutation failed. **Every looping region
carries a seam, a zero-length one at no fade** (`plans/plan-sampler-wav-loop-import.md` §1.1, which
decided the crossfade plan's §6.6), so the taps and the float positions wrap by one span at every
fade, zero included. With whole-frame loop points every wrapped tap lands on a frame, and the fetch is
the integer wrap this crate always made. **The easing across the first frame of fade is gone** with
the separate whole-frame hard wrap it joined to the seam: a fade shortening to zero converges on the
hard wrap by construction. `a_crossfade_converges_to_the_hard_wrap_as_it_shortens` now renders a
2 kHz tone at rungs of 0.01, 0.1 and 1 frame, because a hundredth of a frame on the 220 Hz tone it
first used fell under Character's 16-bit floor and read as zero, and it requires no jump between 0.99
and 1.01 frames. The whole-frame fetch, `frame_plain`, remains for one-shots, Grain and a loop under
two frames long: a one-frame loop, which it wraps by that frame, or points less than a frame apart.

**Gain law: amplitude-complementary `sin²`/`cos²`** — the shared Hann read at half the ramp, Stretch's
own pair — erring coherent, because there is no limiter here. Measured by `examples/measure_readers.rs`
(release, 2026-09-14, the fade inside the loop): a loop whose shortened period is whole cycles reads
**−0.00 dB** through the fade, the control, since its partner is the same material; noise reads
**−2.73 dB** over the fade's central quarter and **−1.21 dB** over the whole fade. The level figures
are deterministic, and match those taken with the first, outside-room geometry to within 0.1 dB. That dip is the price of never putting +3 dB on correlated material,
and the owner's ear decides it (plan §6.3).

**Grain never hears it.** Its read keeps `Region::new`, so a Grain layer renders the same bits at any
fade, as Alternate and a one-shot do; `grain_alternate_and_one_shot_do_not_hear_the_crossfade` fails
if Grain is handed the looped region.

**The walk.** A crossfaded walk fetches per tap through `Seam::frame`, and its contiguous fast path is
taken only for a run clear of both fade halves and the seam, fully eased;
`the_kernel_reads_the_seam_frames_whether_walked_or_contiguous` holds both paths to the same frames.

**Cost, measured** (8 voices × 2 layers, release, one run each): with the whole loop fading, Repitch
**3.3% → 24.7%** of a core and Stretch **7.7% → 50.1%** on a quiet machine with the first geometry
(2026-09-13), and **3.3% → 21.1%** and **8.1% → 47.3%** with the fade inside the loop (2026-09-14) —
**that second run had an `mxm-player` process open**, so its figures are unconfirmed. Every tap in the
fade pays up to four raw frames, two blends, a table read and a remainder. A fade over part of a loop
pays in proportion; no fade costs a branch. Stretch's own probe read 7.8% in the same run against the
7.0–7.1% recorded on 2026-09-12, and no before-figure was taken in this session, so that difference is
unattributed rather than charged to anything. **Recorded, not gated** (plan §6.5). An optimisation
would hoist the per-read geometry and blend two contiguous kernels inside the fade, and must be held
against this form by a test.

`loop_crossfade` is public and returns the geometry the editor draws, from the same arithmetic.
[`MAX_LOOP_CROSSFADE_S`] — two seconds, provisional — bounds the input here rather than trusting the
plugin.

## Two normalisation laws, because coherent and incoherent overlap do not sum the same way

`size × density` is the overlap the parameters request, clamped to [`MAX_OVERLAP`]. `overlap()` is
the one place that arithmetic lives so the editor's readout and the gain law cannot disagree — and
that was a claim before it was true: the editor re-derived it from raw parameter values without
these clamps, so the two *did* disagree below one grain of overlap and above the ceiling. The Reader
card calls `overlap()` now.

**Dividing by the live grain count was the first defect.** It cancelled exactly the build-up that
makes a cloud thicken — eight grains came out as loud as one — and modulated the level every time a
grain started or ended.

**Dividing by `sqrt(overlap)` unconditionally was the second.** Amplitude adds when grains read the
same material; power adds only when they read different material. One law for both is wrong by up
to the square root of the count, and it is wrong in the direction that matters: with the position
draw shut, every grain reads the same age at the same rate, so a cloud that should have fused into
a wall instead got *louder* while staying a comb. Two laws now:

- **coherent** `1 / (overlap × mean(w))`, and
- **incoherent** `INCOHERENT_HEADROOM / sqrt(overlap × mean(w²))`,

crossfaded by how far the position draw is open. Neither ever boosts below one grain of overlap, and
erring coherent is the safe direction — the failure is a texture too quiet rather than one that
grows. This is `mxm-grain-fx`'s law, ported rather than shared, and it now says the same thing that
crate's does: **the position draw alone decides the incoherent share**, because onsets scattered
across identical material sum coherently however they are placed. It could not say that while one
control drove both the draw and the schedule — the law had to gate on the pair and compromised in
both directions, measuring 0.1082 at mid-travel where it belonged near 0.18. Position variation and
Onset timing are separate controls now, and the compromise is gone.

**The incoherent branch needs a crest allowance and the coherent branch does not.** RMS compensation
bounds power, not peaks: many independent grains sum toward a Gaussian whose peaks run several times
its RMS, while a coherent sum keeps the crest of the source it copies. Measured on the probe's
chirp-and-transient source, the uncompensated incoherent branch peaked at **1.400** against 0.900
worst on the coherent branch. `INCOHERENT_HEADROOM = 0.70` puts the worst case at **0.980**, just
under full scale — which is where it has to sit, because there is no limiter in this instrument,
only a clip indicator. `the_overlap_ceiling_stays_under_full_scale_everywhere` holds it, over a
crest-heavy source: the same sweep against a sine peaks at 0.887 and proves nothing.

**The ceiling is musical, not a resource bound.** [`MAX_OVERLAP`] is what the knobs may ask for and
what the gain law divides by, and it is deliberately independent of the share a voice's current
budget can supply. Tying it to the share would make a patch's loudness depend on how many notes are
held, so a chord would change the level of the notes already sounding. Above it the grain-size
control has no further effect; the Reader card prints what was asked for against what is played and
colours it when it saturates. A control with inert travel is a tuning question, but one with no
indication of it is an interface defect.

## Voices: the budget, note ownership and stealing

Eight voices are the provisional bounded budget. A note ID owns release and per-note tuning when
supplied; ID-less duplicates address newest-first. Channel-bend deltas update every sounding or
releasing voice on that channel without erasing its per-note tuning; how smoothly they arrive is the
plugin's, and `Engine::sounding_pitch` reports the newest voice's pitch so the plugin can prove a
range edit ramps. Stealing takes the quietest
released voice, then the oldest active voice. These choices are tested and may change only before
the listening gate.

## The keyboard: one ledger, three modes, and what a release means

A bounded sixteen-press ledger records what is held, newest last, in **every** mode — so switching
to Mono under held keys does not start from an empty one. A seventeenth press drops the oldest
rather than growing.

- **Poly** is the voice pool above. **Mono** and **Legato** share one voice and differ only in
  whether an overlapping press restarts the envelope and the readers; Legato re-pitches what is
  already sounding.
- **Releasing the top of a trill falls back to the newest key still held, and never retriggers.**
  Lifting a finger is not a new articulation, and a re-struck envelope there is the classic
  complaint about mono synths that do retrigger. The glide is taken from the pitch *actually
  sounding*, so a fall-back during a glide continues from where it had reached.
- **Glide is seconds per octave**, not per note: an interval measured per note makes a wide leap
  arrive with a semitone, which is not what a player hears as portamento. Zero is off, and it is
  off in Poly.
- `note_off` carries no parameters, so the engine remembers the mode it was last told in a press or
  a rendered sample. A release must not answer a different mode from the press it answers.

## Routing and expression ownership

This framework-free crate declares the available routing grid and each pair's reach in `FULL_SCALE`;
it does not own a product patch. `Engine::new` therefore starts with `Routing::new()`, and the plugin
supplies its compiled Init topology before rendering exactly as it supplies every later parameter
edit. The one declarative Init lives in `plugins/mxm-creative-sampler/src/init.rs`, beside the host
parameters and generated-source policy that also read it. Cutoff routes sum in octaves and clamp
once, which keeps a closed filter from being pushed below audibility by one route and dragged back
by another.

Reader, filter and normalization tests construct the routes their oracle needs rather than inheriting
product policy. The plugin's default-parameter and fresh-instance integration tests cover Init.

Volume and pan expression are not routes: they are the voice's gain and its position after layer
pans. They address releasing voices as well as held ones, and an ID wins over the newest matching
ID-less press, as for release ownership.

The envelope runs **before** readers in a voice sample, so a newly closed voice does no reader or
filter work and the filter follows this sample’s envelope rather than the previous one.

## Recorded pre-listening measurements

`cargo run -p mxm-creative-sampler-dsp --example measure_readers --release` on the Windows
development machine (2026-09-10) reports:

- Repitch at +12 st retains the 2→4 kHz wanted tone at 0.295893 and leaves the deliberately injected
  18→12 kHz alias at 0.001674, **−45.0 dB** relative.
- On the same transient/chirp source at +12 st, correlations are Repitch/Stretch 0.0010,
  Repitch/Grain 0.0002 and Stretch/Grain −0.0039. This establishes distinct output, not quality.
  The two Grain figures moved from 0.0007 and −0.0002 when Grain's travel began advancing; a reader
  whose playhead was frozen was being compared against two that traverse.
- Clean versus all Character stages at maximum correlates 0.0122; RMS moves 0.21177→0.55135. The
  large level change is a listening-gate question, not approval.
- The first measured 16-voice × 2-layer × 8-grain stress case consumed 354.9% of one core realtime;
  a coefficient recurrence reduced that to 213.6%, still unfit. The evidence therefore changed the
  provisional budget to 8 voices × 2 layers × 4 grains, at **54.7%** of one core.
- **A correction that was itself wrong has been withdrawn.** A later session replaced that 54.7%
  with 85%, calling it unreproducible. On a quiet machine the same command reproduces it exactly:
  30.4% after the optimisation below and **55.1/55.2/54.9%** before it, three runs. The 85% figure
  and the 165.8/324.1% pool table beside it are uniformly ~1.55× high, which is what measuring
  under a concurrent build does; that session's own handover §9 records background processes
  committing and pushing while it measured. **Never take a timing from a machine that is also
  building.**
- **A grain read costs a third of what it did**, and every figure above is unchanged to the digit
  the probe prints. Six equivalences, in the order they were taken, each held by its own test:
  the loop wrap resolved once per read and walked rather than divided per tap; the converter's
  level count as a shift rather than `powf`; a grain's equal-power pan latched at spawn; the
  **unity-rate kernel tabled** (below); one **shared Hann table** replacing a cosine per window per
  sample; and Character resolved **once per rendered sample** instead of once per grain, with the
  hold grid's division moved inside the branch that uses it and the converter's division replaced
  by an exactly-invertible reciprocal.
- **Only the unity-rate kernel can be tabled, and that is the point.** Every call site derives the
  band limit as `1 / max(rate, 1)`, so a read at or below the recorded rate has a cutoff of exactly
  1.0 — one kernel, tabled at 512 phases and interpolated pairwise so the normalising division
  disappears with it, because both rows sum to one. A read *above* the recorded rate has a cutoff
  that narrows with pitch and keeps the computed kernel, unchanged. Tabling one kernel for all of
  them would throw away the transposition band-limiting this crate is measured on.
- **Sixteen taps stay, on a measurement that contradicted the guess.**
  `plans/plan-granular-grain-budget.md` §2 expected eight tabled taps to beat sixteen computed ones
  on quality as well as cost. On this crate's own +12 st probe they do not, and not marginally: the
  alias rejection falls from **−45.0 dB to −22.1 dB**, for 18.8% → 15.9% of a core. Twenty-three
  decibels is not three points of CPU. The reason is that this kernel is not a fixed-cutoff
  interpolator — at a 0.5 cutoff, eight taps span only about four lobes of the narrowed sinc.
- **The unity path's own quality figure**, because alias rejection does not measure it: at −1 st
  the sixteen-tap table keeps 2 kHz at 0.259482 and 18 kHz at 0.056450, **−13.25 dB** of
  top-octave droop; eight taps give −14.49 dB.
- **Pool size, measured per-layer before the budget was shared** (two runs each, and slightly
  *sub*-linear because the contiguous-run path vectorises): 8 grains **18.8%**, 16 grains
  **35.4/35.3%**, 24 grains **45.6/46.0%**. That ladder set `GRAINS_PER_LAYER = 16`, and it is
  kept here because it is what priced [`GRAIN_BUDGET`].
- **The budget is shared now, and the worst case got cheaper rather than dearer.** 256 concurrent
  grains was the *old* ceiling — eight voices × two layers × sixteen — reached only by a full chord
  at full density, while a single held note was charged that provisioning and given sixteen. As one
  shared budget the same 256 measures **31.9% of one core** with 229 live on the dense probe,
  against 35.4% before: compaction retires a grain by swapping the last live one over it, so the
  walk is O(live) over a contiguous run instead of a scan over a fixed array. The ceiling did not
  move and the distribution did — one note reaches 128 per layer, four notes 32, eight notes the
  sixteen they always had.
- **The probe counts grains now instead of inferring them.** `MAX_VOICES × 2 × GRAINS_PER_LAYER`
  stopped being the live total the moment the pool stopped being per-layer, so `Engine::active_grains`
  reports it and the printed line carries the observed number.
- **Raising `MAX_GRAIN_DENSITY_HZ` from 80 to 200 cost nothing measurable** (35.2/35.8%) when the
  per-layer pool bound before the onset rate did. **That argument no longer holds**: under a shared
  budget a dense layer can spend grains another voice is not using, so 200 Hz is free by measurement
  (31.9% on the dense probe, which runs at 100 Hz and 0.25 s and saturates) rather than by that
  reasoning.
- **The overlap law, measured**: as requested overlap goes from ~1 to 3.8, a coherent cloud's RMS
  rises 0.15339→0.25068 (**1.63×**) with its peak at 0.490→0.622, while a scattered cloud holds
  level at 0.18400→0.15707 (0.85×) with its peak *falling* 0.887→0.552. Under the previous
  live-count divisor the same coherent test went 0.16970→**0.14408** — four times the density came
  out *quieter*, which is why a dense setting changed texture and never weight.

### Stretch, after the splice was aligned (2026-09-12, same machine, three runs)

Measured against **a clocked-splice reference implemented in the probe itself**, so the comparison
is one implementation with one difference; mxm-kit's [`docs/oscillators/AGENTS.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/oscillators/AGENTS.md) rule that an unshipped
candidate lives in the harness is the right one here. Both sides interpolate linearly, deliberately:
what is being compared is the splice, and matching the shipped sixteen-tap kernel on one side only
would confuse the two effects.

- **Splice sidebands on a twelve-harmonic tone**, the artefact turned into a number — energy at
  ±1 and ±2 window rates around each of eight harmonics, relative to the harmonics:

  | interval | clocked splice | aligned splice | better by |
  |---|---|---|---|
  | +5 st | −0.4 dB | −45.0 dB | **44.6 dB** |
  | +7 st | **+9.4 dB** | −38.6 dB | **48.0 dB** |
  | +12 st | −35.7 dB | −44.0 dB | 8.3 dB |

  **At +7 st the clocked splice put the artefact above the music.** The octave is the easy case and
  improves least, which is what the complaint said: it is the odd intervals that sounded wrong.
  On a **55 Hz** source the same comparison is 63–71 dB: clocked +19.4 / +27.3 / +19.6 dB at
  +1 / +3 / +7 st against aligned −51.2 / −42.0 / −43.8.

  **The shipped reader's own figures need their control, and the first version of this section
  printed them without one.** Repitch splices nothing, so what it reports is the metric's floor:
  **−52 dB at 220 Hz** and −66 to −71 dB at 55 Hz. Against that floor the shipped reader measures
  −49 to −65 dB at 220 Hz — **at the floor, not below it**, so the honest statement is that the
  splice artefact is no longer what this metric can see on that source, not that it is 50 dB down.
  At 55 Hz it measures −53 to −67 dB. Unity reads −100 dB, the identity again.
- **Attack preservation** on a four-strike source at +7 st, as sharpest 2 ms rise over the signal's
  own RMS so the figures are scale-free: source 4.48, clocked splice 4.74, aligned splice 5.69.
  The shipped reader is **mean 5.26 over eight voices, worst 4.44, best 5.94**, and reads 4.48 at
  unity — exactly the source, the identity again. **It is reported as a distribution because the
  start phase is staggered per voice**, so which window phase an attack lands on varies by design;
  a single note is one draw, and reporting one as *the* figure is how a 5.70 and a 4.44 both looked
  like facts on consecutive runs. Transposing up compresses an attack in time, which is why the
  figures exceed the source's.
- **Onsets land on the frame**: 12 000 / 30 000 / 48 000 / 66 000, exactly the strikes.
- **Cost: 7.0 / 7.1 / 7.1% of one core** for eight voices × two layers of aligned Stretch, against
  Grain's 31.9% on the same run. The search is bursty by construction — once per head per window —
  and this is the worst case with every voice sounding, which is the number that matters.
- **Load-time analysis is effectively free: 0.1 / 0.4 / 2.4 ms** for 1 / 5 / 30 s stereo sources,
  about 0.08 ms per second of audio. Against the 0.113/0.170 s preparation the plugin already
  records, the onset map is under 2% of it — which is what makes *analysis completes before
  publication* cost nothing worth trading a fallback path for.
- **Nothing else moved.** Repitch's +12 st alias rejection is −45.0 dB and the whole grain ladder is
  unchanged to the digit.

### What the owner's first listening round found, and what it cost to see

**The verdict was "still some granular-like artifacts going a few notes up and down, but not
terrible" (2026-09-12), and chasing it found two real defects and one wrong conclusion of ours.**

- **The onset detector fired on bass.** Its analysis window was two hops — 10 ms, *shorter than one
  period of 55 Hz* — so a low fundamental's own cycle read as a string of attacks: **55 onsets on a
  steady two-second 55 Hz tone**, each hard-splicing both heads. The artefact measured **+18 dB
  above the music**, worse than the clocked splice S1 replaced. The window is eight hops now and the
  hop still sets the time resolution; the same source measures −53 to −67 dB.
  `the_transient_map_finds_attacks_and_declines_what_has_none` asserts silence on 40, 55, 82 and
  110 Hz tones and fails on the old constant with the diagnosis in its message.
- **The refinement searched the wrong way.** The flux reports the window in which a rise *completed*
  and the search looked backwards from its start, leaving markers **32 ms early** — which puts an
  attack at a tenth of the spliced head's window gain and quietly makes transient anchoring do
  nothing. Searching forward across the window lands them on the frame.
- **A splice must not replay an attack the heads have already passed.** A head reads at the note's
  pitch while the playhead travels at the scan rate, so transposed up it runs *ahead* — 40 ms at a
  fifth. The playhead crosses the marker long after the heads passed the attack, and jumping them
  back plays it twice. Guarded, attack sharpness went 4.23 → **5.81**.
- **Sub-sample alignment**, because half a frame is 0.8° of phase at a 220 Hz fundamental and **ten
  degrees at its twelfth harmonic**: a parabola through the three correlation scores around the
  winner. An exact match returns zero offset explicitly, since the symmetry that would give zero
  holds in expectation and not on a finite window — without that, unity fell from −100 to −95 dB.

**Every source in the first round was 220 Hz or above, which is why none of this was visible.** The
crate's own measurement debt said so — *"broader multisines, pitches, source classes"* — and the
listening round found what the probe had been told to look for and had not been given.

### S2, and the metric that was needed to grade it (2026-09-12)

**The sideband metric ran out of resolution, and that was the first finding.** With the detector
fixed, S1 and every S2 variant sit *at* the Repitch control on steady tones — −51 against a −52
floor at 220 Hz. A correctly aligned overlap-add is transparent on a steady tone by construction, so
nothing about window length can show there. Grading S2 needed a source whose timing moves and a
ground truth to compare against: **harmonics under a deep 3 Hz tremolo, against the same tremolo at
the transposed pitch** — the exact answer for transposing while holding the timeline, because the
source is ours to synthesise. Compared as a **time-frequency envelope**, six harmonics over
overlapping frames, so a phase offset — inaudible, and scored as total error by a waveform
comparison — does not count. It reads **−107 dB at unity**, which is the identity confirming the
metric rather than the reader.

Error against that ideal, mean over ±1 to ±5 st, and the runtime cost with it:

| | S1 (fixed window, blind search) | period-derived window | **shipped: fixed window, narrowed search** |
|---|---|---|---|
| 55 Hz | −30.0 dB | −27.9 | **−30.0** |
| 110 Hz | −51.8 | −51.9 | **−54.8** |
| 220 Hz | −52.3 | −55.4 | **−57.5** |
| 440 Hz | −49.8 | −53.3 | **−52.8** |
| CPU, 8 voices × 2 layers | 7.3% | 8.4% | **7.0%** |

**Better on every fundamental and cheaper than S1**, and the middle column is the one the plan
specified. A fixed-window sweep says the same thing from the other side: 20 / 30 / 50 / 80 ms
measure −44 / −45 / −49 / −52 dB at 220 Hz for 12.0 / 9.9 / 8.2 / 7.3% of a core, so **longer and
cheaper is better** — which reverses the sweep taken before the detector was fixed, and that earlier
sweep should be treated as void rather than as evidence.

**Load analysis is 26 ms for a 30 s stereo source**, against 2.4 ms for the onset map alone. The
track hop is chosen for how fast material changes rather than by the metric, which is flat across
50 / 100 / 200 ms because a steady tone has nothing for a finer hop to catch.

**Bass at downward transposition is where this reader is still weakest** — −17 to −20 dB against
−50 to −60 elsewhere — and neither S1, S2 nor the window sweep moved it. It is unexplained, not
tuned away, and it is the first thing to point S3 at.

**Three versions of this measurement were wrong before this one, and each flattered the result.**
They are recorded because the metric was harder to get right than the code. Measuring the shipped
reader through the default amplitude envelope reported 0.0885 at both +7 and +12 st — a number that
did not move with the interval, because it was the envelope's decay ramp and not the splice.
Measuring *amplitude modulation depth* read near zero for a clearly audible defect: a deterministic
splice on a periodic source repeats its phase relationship exactly every window, so the artefact is
a comb that is constant in time and carries almost no AM. And measuring the spectrum without a
window function put the carrier's own leakage, about 40 dB down eighteen bins out, on top of the
sidebands being looked for. **A single sine is also a degenerate probe** — two heads reading one
sine sum to a sine — so the source is twelve harmonics.

The probe uses project-authored generated signals, skips attack, and names the frequencies it
measures. Broader multisines, pitches, source classes and a listening decision remain required.
**The owner's ear has not yet judged any of the above**, which is the other half of S1's gate.

## The grain is not expensive; building the kernel twice a microsecond is

**Measured, against the technique itself rather than against this crate's own history.**
[`docs/oscillators/10-granular.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/oscillators/10-granular.md) §10.6.1 prices a
sample-reading grain on this development machine: **8.87 ns** for a stereo four-point cubic read,
**10.09 ns** for an eight-tap sinc taken from a 512-phase polyphase table — against **9.42 ns** for
a windowed *sine* measured in the same run. **A grain that reads a sample costs what a grain that
synthesises one costs.** Reading the sample was never the expense, and the pool ceiling this crate
has argued with twice is not a property of granular synthesis.

At the shipped [`GRAIN_BUDGET`] of 256, this crate's grain costs about **28.8 ns** — 31.9% of one
core over 229 concurrent grains on the dense probe, on an idle machine. It cost **93 ns** when first
measured against that floor, ten times it; the kernel was most of the ten, and it has been taken.

- **The kernel was rebuilt on every grain on every sample.** It now comes from a table wherever the
  band limit allows one, which is every read at or below the recorded rate — see *Two kernels, and
  why only one is tabled*. With the shared window table and Character resolved once per sample, the
  grain fell to 28.8 ns and the pool doubled twice: 128 grains at 57%, then 256 at 35%. Sharing the
  budget and compacting the live run then took the same 256 to 31.9%.
- **The remaining 2.9× is sixteen taps against the floor's eight, and it is deliberate.** It was
  measured rather than assumed: eight taps take the +12 st alias rejection from **−45.0 dB to
  −22.1 dB** for three points of a core. `plans/plan-granular-grain-budget.md` §2 expected
  the opposite, on the reasonable inference that eight *tabled* taps land within 14% of a
  four-point cubic on twice the taps. **That inference holds for a fixed-cutoff interpolator and
  fails here**, because this band limit narrows with pitch — at a 0.5 cutoff, eight taps span about
  four lobes of the narrowed sinc. A cheaper kernel adopted on inference is exactly the move this
  crate's DOX forbids elsewhere, and the probe that exists for it earned its keep.
- **The voice around the kernel gave up its converter**, which the owner authorised on 2026-09-10
  knowing it changes the sound. One converter per voice instead of one inside every grain read is
  35.8% → **32.7%** and closer to the machine at the same time; see *Character is source-domain,
  and its converter is one per voice*.
- **Reaching the 10 ns floor itself needs SIMD**, which
  [`15-granular-in-the-wild.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/oscillators/15-granular-in-the-wild.md) §15.2 prices
  and the owner declined on 2026-09-10. Its §15.2 also sets the honest in-DAW target — about 500
  grains on a quarter core for a cheaper 8-tap grain — against which 256 at 31.9% is this reader
  paying for its taps.
