# AGENTS.md — crates/mxm-creative-sampler-dsp/

Parent: [`../../AGENTS.md`](../../AGENTS.md)

# Purpose

Framework-free DSP for `mxm-creative-sampler`: immutable stereo sample assets,
Repitch, deterministic Stretch and stochastic Grain readers, source-domain Character, a bounded
press ledger with Poly/Mono/Legato and glide, note expression, filter, envelopes and exact idle.
Product plan: `plans/plan-mxm-creative-sampler.md` (in the private archive). Workflow
evidence: `research:interfaces/creative-sampler-workflows.md`.

This is an original design, not a machine copy. Every constant is chosen for this implementation
and remains provisional until the playable listening gate.

**One dependency, deliberately**: [`mxm-modulation`](https://github.com/mxm-audio/mxm-kit/blob/main/crates/mxm-modulation/AGENTS.md), the routing
half only — zero-dependency itself and MSRV **1.87**, so this crate does not inherit the 1.95
GUI floor that `mxm-modulation-params` carries. `cargo tree -p mxm-creative-sampler-dsp` is the
check, and it is the only thing that catches that floor leaking in.

The reasoning, history and measurements behind every rule here are in [NOTES.md](NOTES.md); read
the linked section before changing what a rule governs.

# Ownership

- `src/lib.rs` — immutable `Sample`, reader laws, the forward loop's crossfade seam, voice pool, the
  press ledger and note ownership, Character, filter, envelope, note expression, the fixed common
  routes, deterministic random stream and activity.
- `examples/creative_sampler_render_demo.rs` — project-authored source-to-output smoke render.
- `tests/allocation_refusal.rs` — its own test binary, because it installs a global allocator that
  refuses large allocations: every buffer `Sample::new` sizes from the source must be refusable.
- `examples/measure_readers.rs` and `examples/measure_converter.rs` — deterministic alias rejection,
  reader-correlation, Character and loop-crossfade level and cost measurements; evidence for
  provisional tuning, not a listening verdict.

No plugin framework, parameter, preset, file decoder, drag event, path, editor or host type belongs
here. The plugin prepares decoded files and owns publication/reclamation.

# Local Contracts

## Modulation ([NOTES.md § Modulation is routed](NOTES.md#modulation-is-routed-and-this-crate-declares-what-the-instrument-can-route))

- `src/routing.rs` declares sources, targets and each pair's reach (`FULL_SCALE`), and owns no patch:
  `Engine::new` starts with `Routing::new()`, and the plugin supplies its compiled Init
  (`plugins/mxm-creative-sampler/src/init.rs`) before rendering.
- **Source order is load-bearing** (earlier reads this sample, later last sample's); the layers'
  audio is declared last, so every route from it is backward. LFOs are evaluated once per sample and
  published into every active voice frame; no third frame scope exists.
- **A source nothing reads is not published** (`Routing::needs`); `Graph::set_topology` zeroes a newly
  needed source in each voice frame
  (`a_re_added_backward_route_reads_zero_rather_than_the_tap_from_before_the_gap`). Topology is
  compacted once on the engine, a started voice is armed from it, and a voice frame ceases with it.
- **Amplitude is `mxm_modulation::standard::amplitude_factor`**, a factor on the envelope; every other
  target sums, and Cutoff routes sum in octaves and clamp once
  ([§ amplitude](NOTES.md#the-amplitude-target-is-the-collections-factor-every-other-target-sums)).
- Performance sources go through `mxm_modulation::standard`, zero at rest; Velocity is the press that
  last triggered the envelope; `routing::MACHINE` routes keep their reach. `conformance.rs`'s
  `Declared` runs the standard's checks, reused by the plugin via the `conformance` feature.
- An audio source is tapped at the reader's output and projected through `Sample::mono_mix`, never
  `(L + R) / 2` ([§ audio](NOTES.md#a-modulation-source-that-is-audio-is-projected-not-summed)).
- Volume and pan expression are not routes; they address releasing voices too. Reader, filter and
  normalisation tests build the routes their oracle needs; the plugin's tests cover Init.

## Readers ([§ Three readers](NOTES.md#three-readers-are-three-intentions), [§ Two kernels](NOTES.md#two-kernels-and-why-only-one-is-tabled))

- **Repitch**: sixteen-tap windowed sinc (`SINC_TAPS`; eight was measured and rejected).
  `UNITY_KERNEL` is tabled at or below the recorded rate, computed above it. Tables are `LazyLock`,
  forced in `Engine::new`.
- **Stretch** is WSOLA, aligned to the waveform, never to a clock
  ([§ The splice is aligned](NOTES.md#the-splice-is-aligned-and-that-is-the-whole-reader)). Ties go
  to zero offset by `ALIGN_TIE_MARGIN`; a transient re-lays both heads, but never replays an attack
  they have passed; a head carries its read position as a phase. **Unity is an invariant, not a
  bypass** (`stretch_is_the_plain_read_at_root_pitch_and_unity_scan`). Reverse turns playhead and
  read, negative Scan only the playhead; Hold keeps searching; the search is clamped to the region.
  Reads use the layer's own `LoopMode`; head positions wrap in the reader's float domain.
- **The period track narrows the search only**: not the window length, and not within one window
  of an onset ([§ period](NOTES.md#the-period-narrows-the-search-and-the-window-stayed-fixed)).
- **Grain** draws on the shared `GRAIN_BUDGET`; its five variation controls stay independent
  (`spread_and_scatter_are_independent`, `scatter_decorrelates_without_panning_or_reversing`).
  Travel advances per sample on Stretch's law
  (`grain_travel_traverses_the_region_and_does_not_stall_on_a_saturated_pool`).
- All readers start audibly on note-on; none analyses, allocates or prepares while rendering.

## Alignment and traversal ([NOTES.md](NOTES.md#alignment-and-traversal-invariants))

- Start phases are deterministic but staggered; a chord must not fall into lockstep. Attack sharpness
  is reported as a distribution.
- Alternate reflects the playhead; each head keeps its read direction. Onsets are sorted before
  `seek_marker`. `traversal_gap` folds across a loop seam; source-frame spans are never compared
  with host-sample windows. `ALIGN_IDENTITY_EPS` is not `ALIGN_TIE_MARGIN`.
- `walk_traversal` follows the actual path (seams, turns), coalesces passages within one host sample
  into one splice, and re-seeks without a crossing after a jump or exhausted `TRAVERSAL_SEGMENTS`.
- Reverse traversal from a seam must advance; region or loop-mode changes remap, re-seek and drop
  pending splices. Alignment work is bounded independently of source rate (`ALIGN_COARSE_PROBES`).
- Counters used by parallel tests are thread-local; tests use production constructors, never
  hand-built state.

## Sample analysis ([§ onset map](NOTES.md#the-onset-map-is-part-of-constructing-a-sample), [§ root](NOTES.md#naming-the-note-a-sample-is-playing))

- Onset map and period track are built in `Sample::new`, before publication, with no fallback, and
  never serialised. The analysis mono mix is chosen once at load.
- Every buffer sized from the source is reserved fallibly (`SampleError::Allocation`,
  `tests/allocation_refusal.rs`). Markers are walked with a per-layer cursor, not searched.
- Analyses **decline rather than guess**. `detect_root` is YIN from the region start, after leading
  silence plus 20 ms, over 30 Hz–5 kHz, and returns `None` rather than a wrong Root.

## Character ([NOTES.md](NOTES.md#character-is-source-domain-and-its-converter-is-one-per-voice))

- Hold and reconstruction stay in the reader, on a grid in source frames, so they transpose with
  the note; moving the grid to output time needs a measurement, not a reading. **The converter is
  one per voice**, on the summed layers before the filter
  (`a_voice_playing_many_grains_lands_on_one_converters_grid`).
- `character = 0` is Clean, with a deliberate 16-bit floor. Hold images and low-resolution aliasing
  are character; non-finite output, out-of-bounds reads and unstable recursion are defects.

## Bounds, levels and idleness ([NOTES.md](NOTES.md#bounds-levels-and-idleness))

- Every read clamps or wraps inside the region; one-frame regions render. Layers sum; each Level
  reaches 200%. The envelope runs before the readers and reaches exact zero, then the voice resets.
- Reset and panic remove every head and grain. `Engine::panic` also neutralises controllers; to end
  notes only, bracket it with `Engine::performance` / `Engine::restore_performance`.
- **Never `f32::clamp` bounds that can cross** — the panic takes the host down. Write `min` then
  `max`, floor last; `Region`'s sorted bounds may stay `clamp`
  (`a_region_with_no_room_in_it_still_renders`, `no_random_parameter_combination_panics_the_engine`).

## Loops ([§ half-open span](NOTES.md#a-loop-is-the-half-open-span-from-loop-start-to-one-frame-past-loop-end), [§ seam](NOTES.md#a-forward-loops-seam-is-a-property-of-the-frames-read))

- **A loop is `[Loop start, Loop end + 1)`**, computed only in `Region::loop_span`, `loop_limit` and
  `wrap_loop`. Equal points loop one frame. `settle_on_frame` lands a point on frame `k` only when
  its value is bit for bit `k / last`.
- The crossfade is a virtual loop signal under the kernel (`Seam`, `Region::looped`): faded-in
  material comes from inside the loop, the fade is at most half of it, a blended frame is two raw
  fetches, the period is never rounded, and every looping region carries a (possibly zero) seam.
- **A note plays from Start (reversed: End) as a one-shot until its reader latches**
  (`LayerVoice::looping`, `OlaHead::looping`), then crosses over `LOOP_ENTRY_S`, bit-exact where
  both reads agree. A whole-region loop latches at once without a fade, and not with one.
- Gain law `sin²`/`cos²`, erring coherent. **Grain reads loop-blind.** `MAX_LOOP_CROSSFADE_S` bounds
  the input here. The cost is recorded, not gated; an optimisation must be held by a test. The loop
  tests named in NOTES.md were falsified by mutation: keep them.

## Grain level ([NOTES.md](NOTES.md#two-normalisation-laws-because-coherent-and-incoherent-overlap-do-not-sum-the-same-way))

- `overlap()` is the only place overlap is computed; the Reader card calls it. Coherent and
  incoherent laws are crossfaded by the position draw alone; never divide by the live grain count
  (`the_overlap_ceiling_stays_under_full_scale_everywhere`). `MAX_OVERLAP` is musical, independent of
  the budget share.

## Voices and the keyboard ([§ Voices](NOTES.md#voices-the-budget-note-ownership-and-stealing), [§ keyboard](NOTES.md#the-keyboard-one-ledger-three-modes-and-what-a-release-means))

- Eight voices, provisional. A note ID owns release and per-note tuning; ID-less duplicates go
  newest-first; channel bend never erases per-note tuning. Steal the quietest released voice, then
  the oldest active.
- A sixteen-press ledger in every mode. Mono and Legato share one voice; Legato re-pitches. A
  release falls back to the newest held key and never retriggers, gliding from the pitch actually
  sounding. Glide is seconds per octave, off in Poly. `note_off` answers its press's mode.

# Work Guidance

- `Sample` is immutable while any reader can see it.
- No allocation, locks, file I/O, logging or destruction in rendering methods.
- **Take a timing on a quiet machine.** Every figure here is one process against one core; a
  concurrent build inflates it by about half, and a wrong number gets a feature declined.
- An optimisation of a reader must be held against the form it replaced by a test, not by a
  measurement that happens not to move — a probe reports statistics, and statistics survive a wrong
  frame at a loop boundary.
- Keep clean interpolation and intentional Character degradation separate so one cannot excuse the
  other's aliasing.
- Measure reader distinction and level laws in addition to compiling.
- **Measure a bass source.** Every reader figure in this crate was taken at 220 Hz or above until
  2026-09-12, and a defect that made low material *worse than the code it replaced* sat under those
  numbers for a whole delivery. A tone at 55 Hz costs one more line in the probe.
- **A quality metric needs a control.** Repitch splices nothing, so running the same measurement
  through it says what the metric's own floor is — without which a reader sitting *at* the floor
  reads as a reader that is 50 dB clean.
- Recorded figures and open questions (bass transposed down is the weakest case; no listening
  verdict yet): [NOTES.md § Recorded pre-listening measurements](NOTES.md#recorded-pre-listening-measurements).

# Verification

```bash
cargo test -p mxm-creative-sampler-dsp
cargo clippy -p mxm-creative-sampler-dsp --all-targets
cargo run -p mxm-creative-sampler-dsp --example creative_sampler_render_demo --release
cargo run -p mxm-creative-sampler-dsp --example measure_readers --release
```

Linux and macOS cannot be verified on the Windows development machine. *Since the split
(2026-10-06):* Linux and macOS are checked later, together, and CI builds and tests Windows, macOS and
Linux when started by hand (root `AGENTS.md`, *Verification*).

# Child DOX Index

None.
