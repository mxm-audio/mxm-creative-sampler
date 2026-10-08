# AGENTS.md — plugins/mxm-creative-sampler/

Parent: [`../AGENTS.md`](../AGENTS.md)

# Purpose

The `mxm-creative-sampler` CLAP instrument: identity, host parameters/events/state, audio-file preparation,
lock-free immutable-asset publication, telemetry and MXM editor around
`crates/mxm-creative-sampler-dsp`. Product plan:
`plans/plan-mxm-creative-sampler.md` (in the private archive). UI brief:
[`../../docs/briefs/mxm-creative-sampler.md`](../../docs/briefs/mxm-creative-sampler.md).

This is an original small-sample sound-design instrument, not a hardware copy, workstation,
realistic multisampler or library player.
Workflow evidence is `research:interfaces/creative-sampler-workflows.md`.

The reasoning, history and measurements behind every rule below are in [NOTES.md](NOTES.md);
read the linked section before changing what a rule governs.

# Ownership

This folder owns the plugin crate: `src/`, `presets/`, `examples/`, `host-tests/` (the slow tier,
through MXM Player) and `README.md`; the licence is the repository's `LICENSE`. What each source
file holds: [NOTES.md § File map](NOTES.md#file-map). Placement rules:

- `src/init.rs` is the one declarative, compiled-in Init patch (sources, defaults, route state);
  `src/params.rs` holds ids, ranges and display laws and reads its defaults from it.
- `src/routes.rs` holds the routing pairs' parameters, ids derived from one
  `#[nested(id_prefix = …)]` per target.
- `presets/` — the four recipe files. Generated, never hand-edited: change the design in
  `src/preset.rs` and rerun the ignored `rewrite_factory_files`.
- `src/asset.rs` owns import, the canonical payload, three-slot publication, off-audio reclamation
  and every fallible reservation. `editor::panel` takes its host and asset dependencies as
  arguments, so the keyboard-cursor check can paint it headlessly.

# Local Contracts

## Identity and provisional controls

Product name: `mxm-creative-sampler`; CLAP id: `dk.mxm.mxm-creative-sampler`. `plugin_name!` is the
one crate-local literal and `bundler.toml` is the one external duplicate, pinned by a test.

**All parameter ids are provisional until the S3 owner listening gate.** The plugin is undistributed
and this playable surface exists so that gate can happen. Fix the ids and control-map roles in the
same pass that records approval; do not claim backwards compatibility before then.

## Assets ([NOTES.md § Assets](NOTES.md#assets-acquisition-publication-and-refusal))

- **New audio is never heard through the patch it replaces.** `AssetBank::stage` works off audio,
  `commit` is one atomic store, and the barrier is the process boundary. `commit_revision(expected)`
  publishes only the revision decided about; a load finished while the editor was closed is
  finalised once; the generated-source flag is written before `revision.store`.
- Three A/B snapshot slots. The callback borrows one under a reader count: no `Arc` traffic, lock,
  allocation, decoding or destruction. `unsafe` in `asset.rs` expresses only that slot protocol.
- State is one schema-1 `samples` field; `Plugin::filter_state` parses and bounds it first, and
  malformed state becomes a no-op. Paths never enter state; the chunk never enters the payload.
- Import decodes by content, quantises once to canonical stereo i16, and on any failure keeps the
  current sound and names the error. `MAX_FRAMES_PER_LAYER` is provisional. Open: Browse and the
  hints still say WAV.
- Every buffer a restore, preset or import sizes is reserved fallibly (`asset::reserve`,
  `reserve_text`); a refusal changes nothing. Payloads are validated in place and decoded once.

## Parameters, Init and generated sources ([NOTES.md § Parameters](NOTES.md#parameters-init-and-generated-sources))

- The ten source-owned ids (`Instrument::is_source_owned`) describe a recording; `Preset::resolve`
  skips them for a `state = null` preset. The loop crossfade is not source-owned; Init's is zero.
- Init and recipes preserve the loaded A/B payload; user saves, bank exports and host state carry
  it. `src/init.rs::PATCH` is Init's only declaration, and Init has no file
  (`init_uses_the_current_live_sound_declared_in_one_patch`).
- A fresh instance is playable: `asset::default_source` publishes the static Init saw to A. Its
  frame and cycle counts are fixed by whole cycles, a whole MIDI note and the band limit
  (`the_generated_source_loops_seamlessly_and_is_in_tune`).
- Generated sources are product audio, never shipped files: one additive engine, every partial whole
  cycles with no two sharing a count, sweeps out and back, no carrier past the band limit, phase and
  sign part of the waveform, names ending in `C3` (`every_recipe_closes_its_loop` and the others).
- The picker is the shared caret `selector`, derived from the loaded source's name; it stages off
  the UI thread and is disabled while a load is in flight.

Adding a parameter must update the `#[id]` field/builder in `params.rs`, the declaration table and
count assertion in `preset.rs`, the `Bound` list/card row in `sections.rs`, and `layer_only` in
`editor.rs`. `every_parameter_is_bound_exactly_once` guards the hand-written lists.

## Arrival and Root ([NOTES.md § Root, arrival and loops](NOTES.md#root-arrival-and-loops))

| Arrival | Start, End | Loop points | Loop mode | Crossfade | Root |
|---|---|---|---|---|---|
| Generated | defaults | whole buffer | Forward | 0 | the name's key, else C4 |
| A WAV with a usable loop | defaults | its frames | Forward or Alternate | 0 | the loop's pitch, else the name's key, else C4 |
| A WAV without one | defaults | whole file | the layer's | the layer's | the name's key, else C4 |

- `editor::resolve_arrival` decides from the staged sample (`AssetBank::with_staged`);
  `write_arrival` writes each value as one complete gesture before commit; failure writes nothing.
  `asset::Arrival` belongs to its revision. `settle_arrivals`/`params::settle_source` make a new
  source final on the first callback that hears it; a publication replacing both layers records both.
- `wav_loop::read_loop` reads a WAV's `smpl` loop on the task thread, bounded, never failing an
  import; type 2 is unusable. Never take Root from `dwMIDIUnityNote` alone.
- A loop at 20 Hz or faster is a pitch: cycles are counted against the set unity note (trusted, D7),
  the detector only where it is unset, within `WHOLE_CYCLE_TOLERANCE`, on the canonical 16-bit audio.
- The name parser reads only the last token, uppercase A–G; the name gives the class, the audio the
  octave. Nothing is guessed from audio alone; an unpitched sample is never analysed.
- Root holds cents (D6); a drag moves whole notes (`Bound::stepped`). `Oct ±` keep the cents and
  refuse at the ends (`octave_target`); they and Auto are not host parameters.
- Auto (`auto_root`) reads from the region start until the first pitch change (`PITCH_SEARCH_S`,
  `root_follows_the_region_start`), shows its outcome including none, and is keyed by revision.

## Events and activity ([NOTES.md § Events](NOTES.md#events-voices-and-activity))

- Events split rendering at their offset; spans cap at 64 frames. Inert output is exact zero.
  `Activity::Live` returns `ProcessStatus::KeepAlive`, `Inert` returns `Normal`; releases report a
  recomputed tail. `GrainScheduler::prime` puts the first grain on the first sample
  (`a_sample_shorter_than_one_grain_still_sounds`). Debug builds keep `assert_process_allocs`.
- CC 120/123 end notes and keep controllers (`Engine::performance` around `Engine::panic`;
  `a_cc_panic_keeps_every_controller_where_the_player_left_it`); `reset` and `activate` neutralise.
- Bend range is smoothed and applied per sample (`follow_bend_range`); a bend event scales by the
  range already applied. The four tempo syncs resolve once per callback (`MxmCreativeSampler::synced`).

## Modulation ([NOTES.md § Modulation](NOTES.md#the-modulation-conversion-and-what-it-cost))

- `routes.rs` id prefixes carry no trailing underscore. A route's depth is read per sample for live
  routes (`Routes::advance`), topology once per block (`Engine::set_topology`); the two are paired.
- A re-added route snaps its smoother (`Routes::routing_from`) and its source is cleared
  (`a_re_added_route_arrives_at_its_stored_depth_rather_than_ramping_from_a_stale_one`).
- Performance inputs are per channel. Both audio sources publish at the end of a sample. A mono
  fall-back restores the press (`Ledger` of `Press`); a covering press carries expression forward.
- Each route row's keyboard scope is its parameter id. A route reads in its target's unit through
  `amount_param_at`, never a negative zero (`a_route_reads_what_its_pair_delivers_and_reads_back`,
  `every_route_parameter_says_what_the_dsp_does`). `a_control_map_role_never_points_at_a_dead_route`.
- LFOs free-run; only publication is skipped; Sample-and-hold draws at reset. Do not "fix"
  `a_signed_amount_does_not_always_survive_normalised_storage` in a shared crate. Open: the
  wrapper's own event split has no test; the conversion's CPU cost is the owner's call.

## Converter ([NOTES.md § The converter](NOTES.md#the-converter-character))

- Every stage is visible on its card, reading in the machines' numbers through the DSP's public
  `character_rate_divisor`, `character_bits` and `character_drive_db`; Sample rate is quoted
  against the source rate (`params.source_rate`), never the host's.
- Jitter is in the reader, a stateless hash of the cell index, under one cell
  (`a_jittering_clock_never_runs_backwards`). Companding saturates; drive stays after the
  converter; no dither (the owner removed it). `Linear` with zero Jitter is bit-exact
  (`a_linear_converter_with_no_dither_is_the_converter_this_instrument_shipped`).
- `Bit smoothing` (id `reconstruction`) and `Drive` are the owner's painted names; host names stay.

## Grain and editor ([§ Grain](NOTES.md#grain-shape-and-the-grain-display), [§ Editor](NOTES.md#editor))

- Grain shape is grain-fx's `window::window`, reproduced; `window_moments` integrates its moments;
  Stretch keeps plain Hann. The grain display is live state on the Sample waveform
  (`Engine::grain_views`, one layer via `Telemetry::show_layer`).
- Every card body is a `mxm_ui::tree`; floors are computed, never typed; the opening size is derived
  (`the_opening_size_is_the_budget_hugged`); the Playback card reserves its tallest reader.
- Load's dialog never runs inside a frame (`mxm_ui::offthread`). Master sits in the app bar
  (`paged_with_bar`). Coverage is proved once per selected layer with `Exactly`. Developer channel
  (`MXM_DEV_CC`): CC 119 0–5 and 127, CC 118 unanswered, CC 117, CC 116.
- The Sample card's row order is the owner's. Chips are at least `CHIP_WIDTH` and elide from the
  middle at `CHIP_NAME_BUDGET`; **never extend a user-provided name** (`elision_tests`). Each chip
  has its own `remove_mark` cross; never draw a glyph the font lacks.
- The waveform is the region editor; a drag takes the circle it is pressed on and writes from its
  first frame (`a_waveform_drag_writes_from_its_first_frame_and_always_closes`). BACK during the
  drag puts the handle back where it was taken from (`mxm_ui::drag`; the owner, 2026-10-08).
- **No explanatory prose under the knobs**; facts go to tooltips. A route stack sits under the
  control it moves. Host names keep A/B; painted labels drop them (`binding::layer_label`). Open: a
  fully routed card scrolls at `REFERENCE`, so paging is tested at Init.

## Validation and exclusions ([NOTES.md § Validation](NOTES.md#validation-and-deliberate-exclusions))

- Parameter text is idempotent through the host: every `value_to_string` has a `string_to_value`,
  branches partition the travel and decide on the finer reading's rounding, one rounding feeds one
  reading, no negative zero (`every_parameter_round_trips_its_own_text`).
- Stretch's search bounds stay ordered at `MIN_REGION_FRAMES`; `param-fuzz-basic` stays in the gate.
- No Blend, macros, Nudge, undo or editor locks: every panel control is a host parameter.

# Work Guidance

- Preserve the stage/commit barrier across every acquisition and state-loading path.
- Treat the parameter IDs as provisional until the owner listening gate fixes them.
- Treat generated sources as product audio: changes require the same tuning, loop and spectral
  checks as imported-source processing.
- Keep file preparation, asset destruction and editor-only acquisition state off the realtime
  callback, and out of the host parameters.

# Verification

```bash
cargo test -p mxm-creative-sampler
cargo clippy -p mxm-creative-sampler --all-targets
cargo xtask bundle mxm-creative-sampler --release
clap-validator validate target/bundled/mxm-creative-sampler.clap
```

`every_card_passes_the_tree_checks_in_every_state` runs the tree checks over the editor's
structural states ([NOTES.md § Editor tree checks](NOTES.md#editor-tree-checks-and-review-pictures)).
Review pictures: `MXM_PICTURES=after cargo test -p mxm-creative-sampler --lib tree_pictures -- --ignored`.

Player restore and malformed-state coverage runs through MXM Player in `host-tests/tests/behaviour.rs`
(`cargo test -p mxm-creative-sampler-host-tests`; until 2026-10-05 it was the monorepo's
`apps/mxm-player/tests/mxm_creative_sampler_behaviour.rs`); native file drop and real-DAW operation
remain manual gates.

# Child DOX Index

No child `AGENTS.md` files.
