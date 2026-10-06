# NOTES.md — plugins/mxm-creative-sampler/

The detail behind this folder's AGENTS.md: history, measurements, rationale and worked
examples. AGENTS.md is the contract; this file is the reference it links to.

## Contents

- [File map](#file-map)
- [Assets: acquisition, publication and refusal](#assets-acquisition-publication-and-refusal)
  - [A drop is a transaction, and so is a preset — the commit barrier](#a-drop-is-a-transaction-and-so-is-a-preset--the-commit-barrier)
  - [Closing the editor does not bypass the commit barrier](#closing-the-editor-does-not-bypass-the-commit-barrier)
  - [Publication, retirement and callback ownership](#publication-retirement-and-callback-ownership)
  - [The publication flag rides inside the revision barrier](#the-publication-flag-rides-inside-the-revision-barrier)
  - [A payload the frame cap admits can still be refused, and is refused whole](#a-payload-the-frame-cap-admits-can-still-be-refused-and-is-refused-whole)
  - [Recorded asset budget](#recorded-asset-budget)
- [Parameters, Init and generated sources](#parameters-init-and-generated-sources)
  - [Parameters, Init and recipes](#parameters-init-and-recipes)
  - [The instrument starts on a source, and the init patch is a sound](#the-instrument-starts-on-a-source-and-the-init-patch-is-a-sound)
  - [The generated bank, and the two rules that make it loop](#the-generated-bank-and-the-two-rules-that-make-it-loop)
  - [The Init patch and the factory recipes](#the-init-patch-and-the-factory-recipes)
- [Events, voices and activity](#events-voices-and-activity)
  - [Event, activity and tail](#event-activity-and-tail)
  - [A held note says "do not sleep me", and a voice's first grain is immediate](#a-held-note-says-do-not-sleep-me-and-a-voices-first-grain-is-immediate)
- [Root, arrival and loops](#root-arrival-and-loops)
  - [A drop arrives in tune when the file name says how](#a-drop-arrives-in-tune-when-the-file-name-says-how)
  - [Auto hears the first note that holds still, and two harnesses say how well](#auto-hears-the-first-note-that-holds-still-and-two-harnesses-say-how-well)
  - [Auto finds the root, and says when it cannot](#auto-finds-the-root-and-says-when-it-cannot)
  - [An import resets Root to C4, and what Auto heard retires with the sample](#an-import-resets-root-to-c4-and-what-auto-heard-retires-with-the-sample)
  - [Root carries an octave shift, because the octave is the error that actually happens](#root-carries-an-octave-shift-because-the-octave-is-the-error-that-actually-happens)
  - [A generated source arrives looping, and a route stack belongs under the control it moves](#a-generated-source-arrives-looping-and-a-route-stack-belongs-under-the-control-it-moves)
  - [A WAV that carries its loop arrives looping, at its own frames](#a-wav-that-carries-its-loop-arrives-looping-at-its-own-frames)
  - [Root from a loop: the loop gives the cents, the unity note the octave](#root-from-a-loop-the-loop-gives-the-cents-the-unity-note-the-octave)
- [The converter (Character)](#the-converter-character)
  - [The converter is the instrument, so it reads in the machines' own numbers](#the-converter-is-the-instrument-so-it-reads-in-the-machines-own-numbers)
  - [The converter is a character, not a resolution — so it compands, and its clock wanders](#the-converter-is-a-character-not-a-resolution--so-it-compands-and-its-clock-wanders)
- [Grain shape and the grain display](#grain-shape-and-the-grain-display)
  - [Grain shape is a control now, and the normalisation follows it](#grain-shape-is-a-control-now-and-the-normalisation-follows-it)
  - [The grains are drawn on the waveform, not on a plot of their own](#the-grains-are-drawn-on-the-waveform-not-on-a-plot-of-their-own)
- [Editor](#editor)
  - [Editor and native drop](#editor-and-native-drop)
  - [A fully-routed card does not fit a page, and that is open](#a-fully-routed-card-does-not-fit-a-page-and-that-is-open)
  - [A developer channel, off unless the environment asks for it](#a-developer-channel-off-unless-the-environment-asks-for-it)
  - [Everything that belongs to a layer lives under the layer bar](#everything-that-belongs-to-a-layer-lives-under-the-layer-bar)
  - [The layer bar names files, not letters](#the-layer-bar-names-files-not-letters)
  - [A source name is the one string this instrument does not choose](#a-source-name-is-the-one-string-this-instrument-does-not-choose)
  - [The editor follows the acquisition-to-performance workflow](#the-editor-follows-the-acquisition-to-performance-workflow)
  - [No explanatory prose under the knobs](#no-explanatory-prose-under-the-knobs)
  - [Card descriptions name their current destinations](#card-descriptions-name-their-current-destinations)
  - [A route stack belongs under the control it moves](#a-route-stack-belongs-under-the-control-it-moves)
  - [The loop has a crossfade, and the waveform draws it](#the-loop-has-a-crossfade-and-the-waveform-draws-it)
  - [Editor tree checks and review pictures](#editor-tree-checks-and-review-pictures)
- [Modulation](#modulation)
  - [The modulation conversion, and what it cost](#the-modulation-conversion-and-what-it-cost)
- [Validation and deliberate exclusions](#validation-and-deliberate-exclusions)
  - [Validator quirks this plugin met](#validator-quirks-this-plugin-met)
  - [Search bounds remain ordered at the smallest region](#search-bounds-remain-ordered-at-the-smallest-region)
  - [Deliberate exclusions](#deliberate-exclusions)

## File map

- `src/lib.rs` — CLAP identity/export, event timing, parameter-to-DSP translation, the bend-range
  resync, note expression, activity, malformed-state preflight and the developer channel's gate.
- `src/init.rs` — the one declarative, compiled-in Init patch: generated sources, all parameter
  defaults and route state. It is read-only because no preset file exists to overwrite.
- `src/params.rs` — provisional host parameter ids, ranges and display laws, source/processing
  classification and persisted preset identity; defaults are read from `src/init.rs`.
- `src/naming.rs` — what a file name says about its pitch, and the survey of a real library behind
  both the parser and the decision not to take Root from the `smpl` chunk's unity note.
- `src/wav_loop.rs` — the loop a WAV's `smpl` chunk carries, read beside the decoder, and the pitch that
  loop plays at.
- `src/preset.rs` — `mxm-preset::Instrument`: user asset capture, recipe preservation, the
  source-owned id set Init and recipes both consult, and the four pre-listening recipes with the
  designs they are generated from.
- `presets/` — those four recipe files. Generated, never hand-edited: change the design in
  `src/preset.rs` and rerun the ignored `rewrite_factory_files`.
- `src/asset.rs` — import through `mxm-audio-file-decode`, canonical embedded stereo i16 payload, three-slot publication,
  reader lifetime and off-audio retirement/reclamation, the fallible reservation of every buffer a
  payload, preset or import sizes, and the **generated starting source** a fresh instance loads on A.
- `src/editor.rs`, `src/sections.rs`, `src/binding.rs` — native drop/Browse acquisition, waveform
  and the brief-owned cards. `editor::panel` is the whole surface with its host and asset
  dependencies passed in, so the keyboard-cursor check can paint it headlessly.
- `src/generate.rs` — the seventeen generated sources: one additive engine, seventeen spectral recipes,
  and the two rules (whole cycles, an out-and-back sweep) that make every one of them loop.
- `src/visuals.rs` — the grains, drawn over the Sample card's waveform from live voice state.
- `src/telemetry.rs` — lock-free peak, clip, voice-count and playhead telemetry, plus the sounding
  grains the waveform overlay is drawn from, the layer handshake that selects them, and the
  developer channel's request slots.
- `examples/measure_assets.rs` — worst-cap preparation, serialized payload size and off-audio
  reclamation timing on generated project-owned audio.
- `examples/measure_loop_pitch.rs` — how an import hears every looped WAV in a folder given to it:
  the evidence behind `wav_loop::WHOLE_CYCLE_TOLERANCE`. The files it was run on stay outside the
  repository.
- `README.md` — product documentation; the licence is the repository's `LICENSE` (GPL-3.0-or-later).

## Assets: acquisition, publication and refusal

### A drop is a transaction, and so is a preset — the commit barrier

**New audio is never heard through the patch it is replacing.** `AssetBank` writes a complete
revision into a reserved slot with `stage`, which does the allocation, the copying and the retired
`Box`'s destruction off the audio thread, and `commit` then makes it audible with **one atomic
store**. Because `process` takes a single read guard for the whole callback, a commit that lands
mid-block is not seen until the next one and a commit between blocks is seen whole: the barrier is
the process boundary, which is what `plans/plan-mxm-creative-sampler.md` §4.2 left open. Both are
taken under the writer mutex, so a stage cannot reserve the slot a commit is about to activate, and
a second stage before a commit replaces the revision nobody has heard rather than failing.

Who commits, and after what:

| Path | Staged by | Committed after |
|---|---|---|
| Preset or bank load | `Instrument::apply_preset_state` | `Instrument::commit_preset_state`, called by the shared seam once the parameter gestures are written |
| Editor import (drop or Browse) | `AssetField::import_wav`, on the task thread | the editor's root/region/loop reset gestures, in the frame that sees the new revision; `editor_closed` is the backstop |
| Host state restore | `PersistentField::set` | nothing — nice-plug writes every parameter before `deserialize_fields`, so a restore already arrives in the barrier's order and publishes in one step |

Native drag-and-drop onto the Source canvas or directly onto A/B is primary; Browse is secondary.
Import reads **WAV, AIFF, FLAC, ALAC, MP3, AAC in M4A and Ogg Vorbis** through `mxm-audio-file-decode`,
judged by content, not extension: a drop of any of them imports. **Browse still filters to WAV**, and
the Load and drop hints still say WAV — both in `sections.rs`, not yet changed now that the editor work
they waited on has landed (`plans/plan-mxm-audio-file.md`). The import keeps `import_wav`'s name. An
unsupported codec (Opus), detectable damage (a malformed or skipped packet, a truncated file), empty
content, a non-finite sample in any channel, more than **5,760,000 frames per layer** or memory its
buffers cannot have leaves the complete current asset snapshot sounding and names the error in the
Source card. This cap is provisional S1 evidence: it bounds each canonical stereo i16 layer at
21.97 MiB and two embedded layers at 43.95 MiB before JSON/base64 overhead. A successful editor import writes that layer's **arrival** as complete host
gestures, one per parameter carrying its final value: the region, the loop points (the whole file,
or the loop the file carries), Root, and the loop mode and crossfade where the source asks for them
(*A WAV that carries its loop arrives looping*, below). This both prevents a previous crop from
making a new source appear broken and gives the host an ordinary dirty notification. Failure emits no
gesture.

Import quantises once to canonical interleaved stereo i16. The DSP reads the canonical reconstruction,
so host save/reload reproduces the active source even after its path disappears. Paths never enter
state. Files with more than two channels use their first two channels — up to the decoder's 32-channel
ceiling (`mxm_audio_file_decode::DEFAULT_MAX_SOURCE_CHANNELS`), above which they are refused; mono duplicates to stereo.

### Closing the editor does not bypass the commit barrier

**Closing the editor does not bypass the commit barrier.** `AssetField` tracks the revision whose
defaults were **handled**, separately from the current revision. The close path writes defaults from
its still-live context, including generated-source Root; reopening starts from `handled`, so a load
that completed while closed is finalized exactly once.

### Publication, retirement and callback ownership

`AssetBank` has three complete A/B snapshot slots. The callback acquires one reader count and borrows
one immutable snapshot for the whole process call: no `Arc` increment/decrement, lock, allocation,
decoding or destruction. A single off-audio writer reserves an inactive zero-reader slot, replaces
and destroys its retired `Box` there, unlocks it, then publishes one atomic index. Three slots cover
the active callback plus one held retired reader while another complete revision commits. An
unexpected third external hold rejects publication and preserves the current sound.

The `unsafe` in `asset.rs` exists only to express this slot protocol. Every dereference is protected
by a reader count; every replacement owns WRITER after a zero-to-WRITER compare-exchange; the writer
mutex is never touched by audio. Concurrency tests hold old generations and race publication.

Host state persists one schema-1 `samples` field containing both layers. `Plugin::filter_state`
fully parses and bounds that field before nice-plug writes parameters; because nice-plug 0.3 cannot
reject state from this hook, malformed input is transformed into a complete no-op by clearing both
incoming parameter and field maps. Valid state publishes both samples as one revision while the
wrapper serializes process/activation. This avoids a generic nice-plug state patch.

### The publication flag rides inside the revision barrier

**The publication flag rides inside the revision barrier.** Whether a staged revision came from the
generated bank is written *before* `revision.store`, with the status, because the revision is the
promise that everything beside it is already true. Setting it after `stage_layer` returned left a
window where the editor could settle a freshly staged source against the previous publication's
answer — a generated source left one-shot, or a dropped file forced to loop. Found in review, not by
a test.

### A payload the frame cap admits can still be refused, and is refused whole

**The cap bounds a layer; it does not make its allocation succeed** (audit D14,
`docs/code-review-notes.md` §1). At 5,760,000 frames a layer is 43.95 MiB of frames and 29.30 MiB of
base64, and an infallible allocation that fails aborts the host. So every buffer a restored state, a
preset or an import sizes is reserved through `asset::reserve`/`reserve_text` (`try_reserve_exact`),
and `Sample::new` reserves its analysis the same way (`SampleError::Allocation`). A refusal returns
before anything is staged or published: a restore reports it in the Source card, a preset or import
returns it, and the sound, payload, fingerprint, revision ledger and staged slot are as they were.

**A payload is read in place and decoded once.** `validate_serialized_payload` (for `filter_state`)
and `validate_preset_state` borrow the JSON they check (`PayloadView`) and decode in 4,096-character
chunks into a stack buffer, so a check copies no audio and reserves nothing. Checking used to decode
each layer into a vector it threw away and preparation decoded it again — four decodes and two whole
JSON clones for one preset load. `prepare` now decodes each layer once, straight into its frames. A
chunked decode accepts exactly what one whole-string decode of the right length does: a chunk that
held padding mid-stream decodes off the four-byte frame grid and is refused.

`a_refused_reservation_rejects_a_restored_state_whole_and_keeps_the_previous_one`,
`a_refused_reservation_rejects_a_preset_whole_and_stages_nothing` and
`a_refused_reservation_rejects_an_import_whole_and_keeps_the_previous_sound` refuse each reservation
in turn through a test-only seam and hold both the previous state and the inventory of reservations,
so a buffer that bypasses the seam fails them.
`the_chunked_decode_accepts_exactly_what_a_whole_decode_accepts` holds the decoder against the whole
decode, including padding closing a chunk and paid for at the end, which only the frame grid catches.
The DSP's `tests/allocation_refusal.rs` refuses real allocations.

**Still infallible, and not this plugin's to change:** nice-plug parsing the host's state and
deserializing the field it hands to `PersistentField::set`, `mxm-preset` reading a preset file into
JSON, and `mxm-audio-file-decode`'s decoded buffer. Saving (`preset_state`, the host's field
serialization) is not a recall path and still allocates infallibly. The generated sources are fixed
in size.

### Recorded asset budget

**Re-measured 2026-09-15, when import moved from hound to `mxm-audio-file-decode` — no change.**
`main` and the migrated build were run back to back, alternating three times each on the same
machine: preparation of A was 0.419–0.430 s against 0.417–0.420 s and of B 0.747–0.749 s against
0.739–0.748 s, with serialize and reclaim equal. **Every figure was about four times the 2026-09-11
ones below, the decoder-free steps included**, so that day's absolute numbers are not comparable
with these; only a same-session A/B is. In that session a 119.9 s mono 48 kHz import took **0.497 s
as WAV, 0.543 s as FLAC and 0.496 s as MP3** (`measure_assets -- <files>`). The release bundle grew
from **9,494,528 to 11,096,576 bytes (+16.9 %)**, symphonia's decoders statically linked — the plan's
named risk, left for the owner to judge; narrowing decoder features is the remedy.

At `MAX_FRAMES_PER_LAYER = 5_760_000` (120 s at 48 kHz), the release
`examples/measure_assets.rs` run on the Windows development machine (2026-09-11) measured generated
mono WAV → canonical stereo preparation at **0.113 s for A** and **0.170 s for B while A was
resident**; the full two-layer preset payload is **61,440,225 bytes / 58.59 MiB** and serializes in
**0.045 s**; clearing and reclaiming both layers off audio took **0.073 s**. Source generation is
excluded from those timings. These establish current bounds, not broad hardware budgets or DAW state
limits; listening and non-MXM-host gates may reduce the cap.

`Sample::new` computes the transient and period maps during preparation, so a note can only reach a sample
whose analysis is complete — a published `Sample` is immutable, so late analysis would need a second
publication and a policy for voices already sounding. It costs **26 ms for a 30 s stereo source** — the
onset map is 2.4 ms of that and the period track S2 added is the rest — which is under a quarter of
the preparation above; the measurement is in
`crates/mxm-creative-sampler-dsp/NOTES.md`. Nothing is serialized: the map is derived on load, never
written into the state blob, because every user patch already embeds its audio.

## Parameters, Init and generated sources

### Parameters, Init and recipes

Root note, region start/end and loop endpoints are ordinary automatable source-owned parameters.
Reader, loop mode, **loop crossfade**, direction, pitch, layer level and pan, Character, filter and
envelope are performed or configuration parameters. Embedded bytes and names are durable
non-parameter asset state.

**The loop crossfade is not source-owned.** Source-owned ids describe *this* recording and are
normalised to its frames; a crossfade is seconds of source and means the same thing on any audio, as
Loop mode does, so recipes and Init write it and an import keeps it — **unless the source arrives
looping**: a generated source, or a WAV carrying a usable loop, arrives at zero, for the reason under
*A generated source arrives looping*. It is an **amount**, so Init is zero — the hard wrap, which is
what keeps the owner-selected Init bit-identical.

The shared preset UI uses its opt-in durable-content seam. **Init and factory recipes preserve A/B
payload plus each layer's root and source extents; user saves, bank exports and host state carry
both.** A user capture writes the versioned `AssetPayload` into `Preset::state`; a factory recipe's
`state = null` preserves current content. Preflight parses and bounds a complete payload before any
parameter gesture, and dirty comparison includes its deterministic fingerprint without cloning the
audio every frame. This is failure-atomic — rejected content emits no gestures and is not marked
loaded — and it now has the process-boundary commit barrier the plan required (*A drop is a
transaction*, above).

### The instrument starts on a source, and the init patch is a sound

A fresh instance must be playable. `asset::default_source` generates a band-limited saw and
`AssetField::new` publishes it to A before any file is dropped; the four `state = null` recipes
therefore have content to treat. Imports replace it through the ordinary path, guarded by
`an_import_replaces_the_starting_source`.

**Generated rather than shipped**, so there is no third-party audio in the repository and no binary
in the tree: additive synthesis, every harmonic at `1/n`, normalised to 0.85.

It is the static entry in the seventeen-source generated bank below; its content is fixed because
the owner-selected Init treatment is calibrated against it.

Three constraints fix its otherwise odd frame and cycle counts, and none of them survives rounding
them to something tidier — `the_generated_source_loops_seamlessly_and_is_in_tune` asserts all three:

- **Whole cycles**, because the init patch loops it, and a loop that closes part way up a ramp
  clicks once a second.
- **A whole MIDI note**, because a generated source is tuned by its name, and a name says a note:
  Root has held cents since D6, but `root_in_name` and Auto both answer whole notes. 142 cycles in
  52 105 frames is 130.81278 Hz — C3 to 0.0000 cents.
- **Band-limited to 12 kHz**, not to Nyquist, because the init saw gets played up the keyboard and a
  saw built out to 24 kHz folds as soon as it leaves its root. Measured by DFT on the bin the partial
  lands in, not inferred from the waveform's slope.

### The generated bank, and the two rules that make it loop

**The instrument makes seventeen sources** in `src/generate.rs`: a pulse sweeping its width 50% →
1%, a saw sweeping the same way, a triangle arriving at a sine from an overtone-rich start, and
thirteen more. Generated, not loaded, so they
stay lightweight and nothing enters the repository.

**The point is the readers.** Each one sweeps its *spectrum* across its length, so the scan position
becomes a timbre control — which is what Repitch, Stretch and Grain are for. A static waveform gives
them nothing to scan.

**Sixteen of the seventeen sweep**; `InitSaw` is deliberately static, and is the sound the instrument
has always started on.

**One additive engine, seventeen spectral recipes.** A recipe answers only *what is partial `n`'s
amplitude at position `t`*; mxm-kit's `docs/oscillators/11-additive-resynthesis.md` §11.1 is the argument —
*"a sawtooth is what you get when the amplitudes happen to be `1/k`. Every spectral shape costs the
same."* Two things follow. **Band limiting is structural rather than filtered**: no *carrier* past 12 kHz is
ever synthesised — the rule is *never emit it* rather than *filter it and trust the filter*. That is
not the whole claim, and the difference was worth catching in review: a moving amplitude is
modulation, and modulation puts sidebands either side of what it moves. The morph is slow enough
that they sit within a few hertz of their carriers, and `nothing_lands_above_the_band_limit`
transforms the rendered audio and integrates **every** bin of the guard band rather than sampling the
harmonic grid, which is where the carriers already were. And **it is
true spectral morphing**, which the obvious alternative is not: `09-wavetable.md` warns that
*"amplitude crossfading is not spectral morphing… the null is worst where the tables are most
similar"*. Moving the partials' own amplitudes has no null and needs no phase-locking pass.

**Everything loops, and two rules are why.**

- **Every partial completes a whole number of cycles**, harmonic or not — an inharmonic partial at
  ratio `r` takes `round(142 r)`, under 0.35% of ratio error at worst and inaudible as detuning.
  Without it the metallic and noisy recipes would click once a second.
- **The sweep goes out and back.** Whole cycles make the *carrier* periodic and say nothing about
  the envelope: a spectrum run 0 → 1 leaves the last frame sounding nothing like the first, which
  breaks the very loop the frame and cycle counts exist to protect. This was caught in review before
  it shipped, and `every_recipe_closes_its_loop` fails on a one-way sweep with *Pulse sweep* stepping
  0.273 at the wrap against 0.206 anywhere inside itself.

**A generated source knows its own root, and says so in its name.** Every recipe is C3 by
construction, but an arrival that knows nothing better lands Root on `NEUTRAL_ROOT`, because a
*dropped file's* pitch is unknown — which would leave the whole bank an octave out, the very
complaint that produced `Oct ±`. The names end in `C3`, so `root_in_name` tunes them through the path
a dropped file's name already takes, rather than a second mechanism deciding Root.

**The picker is one click and keeps no state.** Design system §7.4's shared caret `selector` — *"an
editor does not substitute a local dropdown"* — with the displayed entry **derived from the loaded
source's name** rather than remembered. A pending choice that had not been applied would be a
control saying something untrue, and would go stale the moment a drop landed on the layer beneath
it; derived, it is true by construction, survives the editor closing and the patch reloading, and
costs no parameter. A layer holding a dropped file shows the non-recipe **Choose source** entry.
That distinct index is a correctness rule, not placeholder decoration: falling back to recipe zero
made the file impersonate Init saw, so selecting Init saw reported no change and did nothing until
another recipe was selected first. Picking now replaces the selected layer directly, as `Load` and a
drop do — the layer's own cross already empties it on one click with nothing but a tooltip, so a menu
is the higher bar. It is **disabled while a load is in flight**, not merely ignored: the bank
deliberately does not fail on a second stage before a commit, so an unguarded repeat would hold the
editor busy for as long as the picks took.

**It stages, and it runs off the UI thread**, both for the same reason the import path does. Region
bounds are normalised, so publishing outright would let a callback read the new audio through the
*previous* sample's crop — a morph heard through the middle sixth of itself — before the reset
landed. And the render is the small half of the work: around it sit a base64 encode, a decode of
each layer into its frames, a fingerprint over the whole encoded string, and a copy of the other
layer's payload, which the cap allows to be 29.30 MiB of base64. Measured on the development machine,
release, when validation still decoded each layer a second time: **1.9 ms for the six-partial
Drawbars to 97.7 ms for Two vowels**, most of the bank between 20 and 65 ms, beside the 0.113 s a WAV
import already costs. Not re-measured since the decode became one pass.

**Recipe phase and sign are part of the waveform, not just its magnitudes.** A pulse is a *cosine*
series: `sin(pi n d)/n` is a pulse's spectrum at any phase, so a
sine-phase version measures identically on anything spectral while being a different waveform — not
flat-topped, wrong crest. A triangle's odd harmonics alternate in sign; without that the amplitudes
are a triangle's and the shape is not. It matters more here than in a synth, because Grain windows
these waveforms directly and Character's converter quantises them, so the shape *is* the thing being
sampled. Both are held against independently built references:
`the_pulse_sweep_really_is_a_pulse` scores **0.011** on the sine-phase version against 0.95+ on the
real one, and `the_overtone_triangle_passes_through_a_triangle` demands 0.99 where unsigned
harmonics reach 0.975. The triangle's roll-off went with the fix, so the midpoint is exactly a
triangle rather than near one.

**No two partials may share a cycle count**, or they sum instead of beating and a crossfade between
them is a level change wearing an inharmonicity's name.
`no_recipe_collides_two_partials_on_one_cycle_count` found two: `StretchedHarmonics` (`1^1.04` is 1)
and `BellToTone` (a bell set containing 1.0 and 2.00 against the harmonic series' own). Both now
share and *hold* the fundamental rather than crossfading it, which is what a struck bar does anyway —
the hum note stays while the partials above it settle.

**The picker does not set the Sample card's floor; the layer bar does.** The picker is a caret
selector on its own row under the layer bar, and its widest recipe is narrower than the bar's two
chips, their crosses and Load.

### The Init patch and the factory recipes

**Init is the current live sound captured at the owner's request** (2026-09-13), on the generated
Init saw in A. The layer is Grain at 60% level: 0.30x scan, 0.21626087 s size, 36.094425 grains/s,
effectively zero pitch variation, 10.5% position variation, 18.5% onset timing, zero stereo width and
a Smooth window. Character is transparent at the native sample rate through the Linear converter:
16 bits, smooth reconstruction, no converter drive and no jitter. The low pass is 2693.449 Hz at 38%
resonance, and LFO 2 is a 2.6638079 Hz triangle. The live routes are LFO 2 and velocity into Amplitude
(the LFO depth is zero), and LFO 2 and Envelope into Cutoff at 25.510204% and 38.775516%. The absent
LFO 1 and LFO 2 → A Scan speed pairs retain their parked 76.53061% and 17.857146% amounts,
and the absent Pressure → Cutoff pair retains its parked 100%; adding one restores its selected
depth.

`src/init.rs::PATCH` is the single editable declaration of this sound. Parameter constructors read
its plain values, `AssetField` reads its generated-source choices and routing parameters read its
presence/amount pairs. The shared preset system still derives Init from the resulting CLAP parameter
defaults and gives it no file, so Init cannot be overwritten, renamed or deleted and host Reset
cannot disagree with it. `init_uses_the_current_live_sound_declared_in_one_patch` pins the distinctive
values; `a_fresh_instance_plays_the_init_patch_without_a_drop` proves it sounds through the real
bundle with no parameter touched and no state loaded.

**Closed:** the two `mxm-player` state-restore tests that this change broke now pass. The note was
never late — it was slept, mid-attack, and a suspended plugin is never called again. See *A held
note says "do not sleep me"* below; `plans/handover-2026-09-11-first-note-after-state-load.md`
records what the measurement got right and where it pointed wrong.

**Init still preserves whatever audio is loaded.** The starting source is the *initial* payload, not
a payload Init restores: a user who has dropped a recording and presses Init keeps it.

**A recipe brings no audio, so it may not write the numbers that describe audio.** The ten
source-owned ids are declared to the shared seam through `Instrument::is_source_owned`, and
`Preset::resolve` skips them for any preset whose `state` is null. Without that, a factory sound
authored against one recording would crop and detune whatever the player has actually dropped. A
user preset embeds its own audio and therefore does write them.

**Four factory recipes ship, not a bank**: *Long bass*, *Held pad*, *Struck hit* and *Drifting
texture*, each `state = null`, so they treat whatever is loaded four different ways. They exist to
make the S3 listening gate — which asks for exactly a bass, a pad, a hit and an evolving texture —
possible on the owner's own recordings; the authored bank of ≥50 categorised sounds with project
audio behind it is S4 and waits for the verdict. Their designs live in `src/preset.rs` in the
parameters' own units, with an ignored `rewrite_factory_files` test that regenerates the JSON. Init,
Save, Save As, Rename, Delete, favourites, categories and bank import/export are live. Direct
recording is permanently absent.

## Events, voices and activity

### Event, activity and tail

The surface is **178 provisional parameters**: 68 fixed controls and 110 routing pairs.

**Four tempo syncs** (2026-09-25, `plans/plan-tempo-sync-controls.md`), each the collection's
quarter note beside its knob: `lfo1sync` and `lfo2sync` on `params::LFO_SYNC` (1/32 to four bars),
and `agrainsync` and `bgrainsync` on `params::GRAIN_SYNC` (1/64 to a whole note), the top the fastest
on both. `process` resolves all four once a callback from the modulated positions (`synced_*` on the
params) into `MxmCreativeSampler::synced`; a synced LFO rate stands in for its smoother, which keeps
advancing, and a synced grain rate is written over the layer's `grain_density_hz` after
`params.layer`. `Telemetry::tempo` lets the knobs read their divisions. The Playback card's grain
row is one flat row — Transpose to Grain rate, the quarter note, Grain shape — so its spare width
stays at the end.

Voice count and grain budget are measurement-sized. Eight provisional voices share one
**256-grain budget** across sounding layers: one note reaches 128 grains per layer, four reach 32,
and eight reach 16. Compaction walks O(live) contiguous grains; the current worst-case probe measured
31.9% of one core. The DSP DOX owns the complete budget campaign, equivalence evidence and rejected
eight-tap kernel.
Supplied note IDs own release; ID-less duplicate key presses
release newest-first. Voice stealing chooses the quietest released voice, then the oldest live voice.
CC120 and CC123 panic. **Voices** chooses Poly, Mono or Legato and **Glide** is seconds per octave;
the ledger and the no-retrigger fall-back are the DSP's, documented there.

**A CC panic ends the notes and keeps the controllers** (audit D12). The wheel, the bender and
channel pressure are positions a controller sends only when they move, and neither control change
moves them, so the CC arm takes `Engine::performance` before `Engine::panic` and restores it after,
as the other instruments' panics leave their controller arrays alone. The engine's panic alone
returned all three sources to neutral while the plugin kept the bend it tunes new notes by: a note
after the panic was pitched by the bend while every route from Bend read zero, and the wheel and
pressure routes were lost until those controllers next moved. `reset` and `activate` still return
every controller to neutral. `a_cc_panic_keeps_every_controller_where_the_player_left_it` holds it
for CC 120 and CC 123 against a twin that received the same positions after its panic.

Channel pitch bend spans the **Bend range** parameter, 0–24 semitones, and updates voices already
held without erasing their per-note offsets. Voices carry accumulated deltas, so a range edited
under a held wheel is applied as the difference between what the wheel now means and what was
already applied. **The range is a signal, not a setting** (`docs/code-review-notes.md` §2): it is
smoothed over 20 ms, as `mxm-mono-02` and `mxm-mono-08` do, and `lib.rs::follow_bend_range` applies
the smoothed value once per sample — in inert spans too while it ramps, so a note started after
the edit begins where a sounding one would be. Read once per `process` call, a range edited under a
held bend was a pitch step on the callback's first sample (audit D11). A bend event scales its
position by the range the voices already hold, never the edited target, or it would finish the ramp
in one step; `reset` lands the ramp, because nothing is left to carry it.
`a_range_edit_under_a_held_bend_ramps_and_the_blocks_do_not_matter` reads the pitch off the voice
(`Engine::sounding_pitch`) after every sample and requires a bounded monotonic ramp that the block
partition does not change to the bit; `a_bend_moved_during_a_range_ramp_lands_on_the_range_already_applied`
holds the event. CLAP per-note tuning addresses the supplied voice ID or the newest matching
ID-less press.

**Volume and pan expression are live**, and both address a releasing voice as well as a held one.
**Pressure is no longer a fixed route**: it is a source, and its path to the filter is the
(Cutoff ← Pressure) pair. The route retains the old path's full-scale reach but is absent from the
owner-selected Init, matching the B treatment that was promoted.

All host events split rendering at their sample offset and internal spans are capped at 64 frames.
Inert output is exact zero and skips reader/filter work. Held voices report Normal; releases report a
recomputed conservative tail. Reset/panic clear all heads and grains. Debug builds retain
`assert_process_allocs` coverage.

### A held note says "do not sleep me", and a voice's first grain is immediate

Two `mxm-player` tests failed for a session under the heading *the first note after a state load
arrives 51 blocks late*. The note was never late. It was **slept**, and then it had nothing to say.

**`ProcessStatus::Normal` is CLAP's `CONTINUE_IF_NOT_QUIET`**, and this plugin returned it for a
live voice as well as an idle one — telling the host it may suspend an instrument whose key is still
down. That is a lie whenever the voice has not reached audibility yet, and this instrument has two
ordinary ways to spend a while at exact zero: the envelope's attack, 220 ms in the init patch, and
Grain's scheduler. The host counted eight silent buffers, suspended the voice mid-attack, and a
suspended plugin is not called again — so it could never climb out. `Activity::Live` now returns
`ProcessStatus::KeepAlive`; `Inert` still returns `Normal`, because an idle instrument *should* be
suspendable.

**A voice's first grain now lands on its first sample.** Left to its ordinary cycle the scheduler
opens a voice by rolling a jittered offset into the first slot and, above the hand-over, by handing
that slot to the stochastic process instead — so the first grain arrived anywhere inside the opening
slot, or nowhere. At forty grains a second that is up to 25 ms of silence before a key does anything,
a different number every time, on top of the attack. `GrainScheduler::prime` arms the first onset at
offset zero and unsuppressed; a `slot_length` of one sample makes it exactly one onset, and the
cloud's statistics from the second onset onwards are untouched.

**The host was not at fault and its change was reverted.** Resetting the quiet counter on *any* wake
rather than only on the way out of sleep was tried, and it broke five unrelated player tests: a
plugin that wants to stay awake is supposed to say so, which is what `KeepAlive` is for. The fix
belongs in the layer that was lying.

`a_sample_shorter_than_one_grain_still_sounds` is the regression test, built from the exact patch
and source printed out of the failing run. It asserts both halves: a grain on the voice's first
sample, and audibility inside the eight blocks the player's tests listen for.

## Root, arrival and loops

### A drop arrives in tune when the file name says how

**There is a standard for this and it is empty.** RIFF's `smpl` chunk carries `dwMIDIUnityNote`, the
MIDI note at which a file plays unpitched — exactly the field this instrument wants. Measured across
600 files drawn at random from the owner's library: **17 carried a `smpl` chunk at all, and all
seventeen had the field set to 0.** Zero is C-1, five octaves below anything in a bass pack. It is
the field being present and unset, so reading it would be correct by the specification and would
tune every sample to the bottom of the keyboard. `naming.rs` does not read it, and says why. **A loop
the chunk carries is another matter**: its length is a pitch, to the cent, and `wav_loop.rs` reads the
unity note only to count that loop's cycles, and only where it is set (*A WAV that carries its loop
arrives looping*, below).

**The de-facto standard is the file name.** Across 6,941 WAV files, about 15% end in a key tag:
338 bare letters, 338 note-plus-mode, 142 note-plus-accidental, 186 with both, and 26 with an
octave. Two facts from that survey decide the design. The octave is almost never there, so a name
gives a **pitch class** and not a note. And every one of the 338 bare single letters is **uppercase
and in A–G**, with no lowercase last token anywhere in the library — which is what makes a bare `_A`
safe to read as a key rather than as a take marker, the false positive that would otherwise sink the
idea. The parser requires uppercase for that reason, and reads **only the last token**: a key
mentioned mid-name is a guess, and a wrong Root retunes the keyboard.

**The name gives the class and the audio gives the octave.** That pairing is better than either
half: the class corrects the detector's whole-note mistakes, and the detector keeps the answer in
the register the sample actually sits in. On the owner's bass pack it closes the last gap —
`soul_E` was heard as D2 and its name moves it to E2, two semitones, in the right octave. The folder
goes to **10 right, 0 wrong, 0 declined**, from 4 right when this work started.

**Nothing is guessed from the audio alone, and an unpitched sample is never analysed.** A file whose
name says nothing leaves Root where the import put it, and the name is consulted *before* the audio
so no detector runs at all — the owner's reason is the right one: plenty of music is made from
material with no pitch, and running a detector over a drum loop on every drop is work whose answer
is thrown away, or worse, kept. A detector is confident on a bass note and wrong on a chord, and
Auto is the place for that question — asked, not assumed. Detection happens in exactly one case: a
name that claims a key and leaves out the octave, which is the one question the audio is being
asked. A name carrying its own octave is already a note and is not checked against anything. **The
one narrowing is a loop the file carries** (below): the loop's length states the pitch, the detector
runs only where the file leaves its unity note unset and only to count whole cycles in a loop fast
enough to be a pitch, and an unsure count sets nothing.

**Root is chosen from the staged sample and written before commit with the rest of the arrival.**
`AssetBank::with_staged` makes the sample available without making it audible; publishing Root later
would expose callbacks with new audio and old tuning.

Publication itself names its revision: `AssetBank::commit_revision(expected)` publishes the staged
snapshot only if it is the one the caller decided about. The editor decides in one step and commits
in another, and a background load can land in between; a bare `commit` published whatever it found.

### Auto hears the first note that holds still, and two harnesses say how well

A single 20 ms onset window scored **4 right, 5 wrong, 1 declined of 10** in
`examples/measure_root.rs`, chiefly because pitch envelopes were measured before settling. This is
the rejected baseline, not the current detector.

**The rule is the owner's: the pitch at the region start, held until the first pitch change after
it.** *"The sound closest to the start position, so I can move it and get a new pitch if I don't
want the start of the sample to be the pitch."* Windows at a 25 ms hop from the start; three that
agree within a quarter-tone make a note, and from there the run keeps taking windows for as long as
they agree, so the answer is the median of the **whole note** rather than of its first tenth of a
second. What ends it is the pitch change. A run is broken by disagreement with its *own first
window*, not with the previous one: a glide moves a little at a time, and comparing neighbours would
let it walk any distance while every step looked like the same note — which is also why an 808's
envelope is skipped for free, since it cannot hold still until it has settled.

`PITCH_SEARCH_S` bounds how far past the start a note is looked for, and it matters: at six seconds
a twelve-second loop could report a note from the middle of a progression and call it the root.
`root_follows_the_region_start` is the test that holds the control itself — a two-note phrase read
from the top gives the first note and read from past the change gives the second.

Same pack, measuring the whole note: **9 right, 1 wrong, 0 declined**, and every cents error
shrank — averaging a note beats sampling the front of it.

**The collection renders its own ground truth.** `dsp-lab`'s `root_spike` plays `mxm-mono-01`'s
oscillator at pitches chosen before the audio exists, so an error is the detector's and nothing
else's — a pack's key tag names a pitch class with no octave and no guarantee beyond what its maker
typed. Eleven steady saws from C1 to C7 come back exact. The awkward cases are there on purpose:
pitch envelopes, a sub oscillator, and a phrase whose first note is not its key.

**The octave error is not fixed, and the page says so rather than pretending.** A bass patch with a
sub genuinely repeats at the sub's period, so YIN is right about the signal and wrong about the
note. Preferring a half, third or quarter of the winning lag when it scores nearly as well was
built and measured: it changed no verdict at 0.25, 0.45 or 0.70 on either harness and made two more
wrong at 1.00, so it was removed rather than left in unearned. It cannot work here for a structural
reason — E1 is 41 Hz, its sub is 20.6 Hz, and the signal's true period is **below `PITCH_FLOOR_HZ`**:
no lag in the search range describes it, and dividing a winner never reaches a period the search
never held. A fix is a different detector — harmonic summation over a spectrum, or an octave
decided by energy — not a tuning constant.

**Root and key are different questions.** Auto answers *what note is sounding where the region starts*. A loop's file
name usually carries the *key*, which needs chroma over the whole file. They agree when a loop opens
on its tonic and disagree when it does not, and no amount of work on this detector closes that gap.

### Auto finds the root, and says when it cannot

`auto_root` runs `mxm_creative_sampler_dsp::detect_root` on the selected layer from the region start
and writes the rounded note to that layer's Root. It is an editor action rather than a parameter, so
it opens no `navigation::at` scope — the same as the card locks did before them, and the same reason.

**It reports what it found, including nothing.** A button that silently does nothing cannot be told
apart from a broken one, and this one legitimately declines: silence, a drum, a chord and noise have
no single pitch. Declining is correct there — a wrong Root retunes the whole keyboard on a guess the
player never made — but only if it is visible, so the outcome is written beside the button.

### An import resets Root to C4, and what Auto heard retires with the sample

**Root does not go back to its parameter default on an import.** `a_root` starts at C3 so a fresh
instance is in tune with the generated saw and `b_root` at C4 because layer B has no saw to be in
tune with — right until a file is dropped, at which point the saw is gone, C3 means nothing to the
new audio, and the same file dropped on A and on B came out an octave apart. Both land on
`NEUTRAL_ROOT`, C4, **when nothing better is known** — a loop's own pitch, then a key in the name,
comes first — and Auto is there for the sample that knows better. Start and End still reset to their
own defaults, because theirs mean the same thing whatever is loaded, and so do the loop points unless
the file carries its own loop.

**Auto’s result is keyed by layer revision, not layer alone.** Drop, Browse, preset load and host
state restore all retire a result produced for replaced audio.

The editor writes an arrival only when a new revision is visible with `Ready` status, and only the
arrival record staged with that revision (`AssetField::arrival`).
`import_wav` publishes status before releasing the revision; `consecutive_imports_each_announce_
themselves_as_ready` checks this ordering across three imports.

### Root carries an octave shift, because the octave is the error that actually happens

**Every route to a Root guesses the octave last and worst.** A file name almost never carries one —
26 of 600 in the survey behind `naming.rs` — so the name supplies the pitch class and the *audio*
supplies the octave; and pitch detection's characteristic failure is a whole octave rather than a
wrong note, a rich harmonic series inviting the octave below and a weak fundamental the one above.
The note is usually right and the register is not, which makes *the same note, an octave over* the
repair a player reaches for. The owner reported it as the common case (2026-09-12).

`Oct −` and `Oct +` sit beside `Auto` and move Root twelve semitones through the same
begin/set/end gesture the knob uses, so a host records an ordinary edit. **They refuse rather than
clamp at the ends**: clamping would land on a different note than the one asked for and say nothing
about it, so each button disables itself instead and the range is visible in the control.
`octave_target` is the arithmetic, held by
`an_octave_button_moves_twelve_semitones_and_stops_at_the_ends`. **An octave keeps Root's cents**
(D6): a Root between notes is an imported loop's pitch, and rounding it on the way to the right octave
would detune the loop the import tuned.

**They are not host parameters and do not need to be**, the same as `Auto`: they write `a_root` /
`b_root`, which the knob already covers for the keyboard cursor and the control map, so a second
scope for the same id would report as an unreachable duplicate. Design system §7.2 asks text buttons
to use verbs; a verb here would be *Transpose down an octave*, which cannot sit beside a knob, and
the shared noun with the two signs is what makes the pair read at that width.

### A generated source arrives looping, and a route stack belongs under the control it moves

**Two owner reports, and both are corrections to the conversion rather than to the routing model.**

**Picking a generated source left the loop alone, and nothing had ever written it.** Reported as
*Supersaw does not loop under Repitch and Stretch* — and the asymmetry is the diagnosis, because
**Grain never reads the loop at all**: it schedules grains inside the region, so only the two readers
that wrap were affected. The import's reset wrote the region, the loop *points* and the root; the
loop **mode** was written by nothing, so a generated source inherited whatever the layer was left on.
It looked correct only because a fresh instance defaults to Forward.

The generated bank exists to loop — every partial takes a whole number of cycles for that one
reason, and the odd frame and cycle counts are chosen around it — so a generated source now asks for
a forward loop as it lands, **and for no loop crossfade**. Its loop is the whole buffer, so a
crossfade has no room outside it and borrows from inside, moving the return point off a whole cycle
and fading against a cycle offset: the one thing a crossfade can do to a loop built to close is harm
it. **A dropped file does neither unless it carries its own loop**, which arrives the same way at its
own points (*A WAV that carries its loop arrives looping*, below): a drum hit that suddenly repeats is
a worse defect than a waveform that does not, and a plain file's crossfade is the player's. Every
write is one full host gesture before commit; `a_generated_source_arrives_looping_and_a_dropped_file_does_not`
holds them. `AssetField` carries an arrival record together with the revision it describes, because
every path shares `stage_layer` and the editor could not otherwise tell.

### A WAV that carries its loop arrives looping, at its own frames

**The owner's words, of a folder of JD-800 waves: *they were made for looping*** (2026-09-14,
`plans/plan-sampler-wav-loop-import.md`, in the private archive). 91 of those 108
files carry a RIFF `smpl` loop, and the importer read none of them, so an attack wave on a looping
layer repeated the whole file, attack included.

**The chunk is read beside the decoder, on the task thread, for a WAV only, and never fails an import**
(`wav_loop::read_loop`). The decoder returns audio and no chunk, so the RIFF list is walked header by
header, bounded at 1 024 chunks and a 64 KiB `smpl` body. No chunk, a truncated or oversized one,
zero loops, a start after its end, an end past the last frame, or a type with no mode here all mean
*no loop*; `a_chunk_list_that_lies_never_panics` fuzzes the list. The first loop is taken. Type 0 is
Forward and 1 Alternate, and **type 2, backward, is unusable**, because Reverse would reverse the
attack too. `dwFraction` is ignored, and a finite play count loops for as long as the note is held.

**One arrival record per staged revision.** `asset::Arrival` is `Generated` or
`File(Option<FileLoop>)`, stored with the revision it describes and replaced whole by a later stage;
`AssetField::arrival` answers only for that revision. `an_arrival_record_belongs_to_its_revision`
stages a looped file, then a plain one, before any settle: the stale settle writes nothing, and the
current one writes no loop mode, crossfade or loop pitch.

**Resolved once, then written once.** `editor::resolve_arrival` decides every value from the staged
sample, and `write_arrival` writes each as one complete gesture before the commit:

| Arrival | Start, End | Loop points | Loop mode | Crossfade | Root |
|---|---|---|---|---|---|
| Generated | defaults | whole buffer | Forward | 0 | the name's key, else C4 |
| A WAV with a usable loop | defaults | its frames | Forward or Alternate | 0 | the loop's pitch, else the name's key, else C4 |
| A WAV without one | defaults | whole file | the layer's | the layer's | the name's key, else C4 |

`an_arrival_writes_each_parameter_once_with_its_final_value` counts the gestures and holds the
precedence: a loop's pitch beats a generated name's key. `a_wav_with_a_loop_arrives_looping_at_its_own_frames`
runs the real decoder over files it writes, and a truncated chunk imports exactly as no chunk.

**Final on the first callback that hears it.** Region, loop points, crossfade and Root are smoothed,
so the arrival's gestures played the first 20 ms of new audio through a ramp from the previous
source's crop. Each snapshot carries its layers' revisions; `lib.rs::settle_arrivals` compares them
with the last it heard, and `params::settle_source` resets a changed layer's smoothers onto their
targets before rendering — on the audio thread, with no allocation and no lock. It covers every
publication that replaces a layer. `a_new_source_is_final_on_the_first_callback_that_hears_it`
checks it after an import and after a preset load, with a control showing that a moved crop still
ramps. **A publication that replaces both layers records both** in the ledger a later one-layer stage
copies from. A host state restore once did not, so an import after it snapped the other layer's
smoothers (code review round 1; `a_restore_then_a_new_source_on_one_layer_leaves_the_other_layer_alone`).

**Loop frames survive the parameter.** The points are normalised `f32`s, which cannot place every
frame of a long file, so `Region::new` lands a point that is exactly a frame's own normalised value on
that frame, and leaves every other point where it lies (the DSP DOX's half-open loop section). A
one-frame loop is a loop, and a back-and-forth one arrives Forward, which plays that frame exactly.

**The parameters are the durable record.** The payload stays schema 1 and never carries the chunk.
`embedded_audio_restores_in_a_fresh_host_after_the_source_is_deleted` round-trips fractional loop
points and a fractional Root through a fresh host, on a Repitch layer, **and hears them play**: the
restored patch renders the same twice in fresh hosts and differently with Root on the note or with
the loop moved.

### Root from a loop: the loop gives the cents, the unity note the octave

**A loop that repeats at 20 Hz or faster is a pitch; a slower one is a phrase.** The owner's rule:
*a drum loop might also have a loop but that is not pitch as it lasts longer than audible
frequencies.* A loop's length gives its note to the cent, but a loop may hold several cycles, so the
cycles are counted against a guide note of frequency `f`. The loop holds `r = f × P / rate` cycles,
`n = round(r)` is believed within `WHOLE_CYCLE_TOLERANCE`, and Root is the note of `n × rate / P`
(`wav_loop::loop_pitch`). It is measured only for a loop fast enough, so a phrase never builds a copy
of itself, and **on the canonical 16-bit audio a layer plays** (`asset::canonical_frame`, shared with
the snapshot), not the decoded floats: quantisation can move a quiet loop's peak under the
detector's level floor, so the floats can be pitched where the audio played is not (code review
round 2).

**The guide is the file's unity note, and the detector only where the file leaves it unset.** The
approved plan had the detector (revision 6); measuring it changed that (revision 7).
`examples/measure_loop_pitch.rs` on the owner's JD-800 set (release, 2026-09-14), whose 71 loops fast
enough to be a pitch all name unity note 60:

- **Counted against the unity note:** 68 accepted, every count within **0.049** of whole
  (`063 Digi Bell` the widest), and three refused — `069 Strat Atk` at 0.159, `050 AgogoBells` at
  0.243, `035 Fine Wine` at 0.249. The tolerance of 0.1 sits between.
- **Counted against the detector**, hearing the loop repeated for a second: **16 of the 71 differ.**
  Nine land an octave or more out (`029 Can Wave 1` two octaves low, `051 Bottle Hit` two high), four
  that hold whole cycles are declined, and all three the unity count refuses get a note.

The detector's failure is structural: repeating a loop makes the signal periodic at the loop, so
where the cycles inside it differ slightly it answers the loop. `the_unity_note_places_a_loop_whose_cycles_differ`
holds that case. Where a chunk leaves the unity note unset — every chunk in the owner's wider library
(*A drop arrives in tune when the file name says how*) — the detector is still the guide:
`a_loop_holding_several_cycles_lands_on_its_own_note` covers one to four cycles either way, and
`a_loop_that_is_not_one_note_has_no_pitch` refuses a slow loop, noise with no unity note, and a
non-whole count.

**A set unity note is trusted (D7, the orchestrator's, reversible by the owner).** An octave-wrong
unity note gives an octave-wrong Root, and `Oct ±`, which keeps the cents, is the recovery. The
owner's files are the evidence for trusting it: right on every loop there, where the detector was not.

**`055 Tabla` is a pitch.** The plan listed it as a fast loop that is not a note. Measured, its 26 Hz
loop holds 9.996 cycles of C4 by the unity note and 9.987 by the detector, and it lands on C4
+0.6 ct.

**Root holds cents (D6).** Root has no step size. It reads `C3 · 48` on a note and `C4 −25 ct · 59.75`
off one, and takes typed cents (`root_holds_cents_and_reads_a_whole_note_as_before`). An editor drag
or key press still moves it by whole notes — `Bound::stepped`, the one addition this plugin's copy of
the binding carries (`a_stepped_control_lands_a_drag_on_its_step_and_keeps_a_typed_value`) — and
`Oct ±` keeps the cents. Auto still answers a whole note. A whole-note Root reads and renders as it
did.

**Falsified by mutation** (2026-09-14), each against the test named in these two sections:

- the arrival record ignoring its revision: the stale settle writes five parameters;
- the loop points written twice: `A Loop start` appears twice;
- `settle_arrivals` resetting nothing: start, loop start, loop end and crossfade still ramp;
- the import dropping the file's loop: it arrives as `File(None)`;
- `loop_pitch` ignoring the unity note: the detector is asked;
- type 2 accepted: a backward loop is read;
- the tolerance not applied: 4.84 cycles tunes to 60.56;
- an unsnapped drag: Root stays at 59.6;
- a rounding octave: 59.75 goes to 72;
- a restore leaving the ledger stale: layer B's moving crop is snapped by layer A's arrival;
- a one-frame loop left Alternate: it reads `(42, 42, true)`;
- the loop's pitch measured in the decoded floats: a loop under the detector's floor as played sets Root 59.955;
- through a rebuilt release bundle, the plugin ignoring Root, then Loop end: the variant renders the
  same as the restored patch, a largest difference of 0.

## The converter (Character)

### The converter is the instrument, so it reads in the machines' own numbers

The converter’s stages remain directly visible on the card; there is no macro or Details
disclosure hiding the instrument’s central sound-shaping path.

**Each reads as the number the machine was sold on**, not as a percentage of a knob:

| Control | Law | Reads |
|---|---|---|
| Converter type | uniform steps ↔ µ-law, µ = 255 | `Linear`, `Companding` |
| Sample rate | `grid = 2^(5 × rate)` | `44.1 kHz` … `1.38 kHz` — the source's rate over the grid |
| Bit depth | `round(16 − √converter × 11)`, clamped 5..16 | `16 bits` … `5 bits` |
| Reconstruction | linear interpolation ↔ zero-order hold | `smooth`, `45% stepped`, `stepped` |
| Converter drive | `1 + input × 15` into a soft clip | `+0.0 dB` … `+24.1 dB` |
| Jitter | clock wander, `JITTER_CELLS` of a cell at full | `0%` … `100%` |

### The converter is a character, not a resolution — so it compands, and its clock wanders

**mxm-kit's `docs/oscillators/14-samplers.md` §14.8's recommendation is taken.** That chapter measured what
makes a 1984 sampler remembered fondly and a bit-crusher a toy, and it is not the bit count: a
*linear* converter degrades one-for-one with level, so at −36 dBFS an 8-bit linear converter sits at
−15.2 dB of junk, and by −54 dBFS the signal is gone entirely. A companding converter holds about
−36 dB across a 30 dB span instead. TAL's *DAC modes* — Emu II, AM6070 — are companding parts rather
than depths, which is the thing this instrument was missing while its DOX already recorded the
varispeed difference.

**Two additions, acting on different halves of the converter.** Companding changes the *domain the
quantiser's steps are uniform in*; **Jitter changes when the converter samples at all**:

```text
read position → cell boundary + jitter → hold / reconstruct     (per layer, in the reader)
voice sum → compand → quantise → expand → drive → filter        (once per voice)
```

**Dither was built in this slot first, and the owner removed it**: *"I cannot hear what it does so it
adds no creative character to the sound."* That is this collection's standard — the same one that
removed Blend, the macros and Nudge — and it was right twice over, because **Jitter is what was asked
for and dither is not the same thing**. Dither adds noise before the quantiser and leaves the clock
alone; jitter unsteadies the clock and leaves the quantiser alone. A real sampler's DAC clock never
kept perfect time, and TAL's interface calls the control Jitter.

**Jitter lives in the reader, not in `convert`**, because the clock *is* the virtual grid: `grid`
counts source frames and a cell boundary is the instant the converter samples. Moving that boundary
is the whole mechanism. The wobble is a **hash of the cell's own index**, so it carries no state —
it cannot perturb the streams that schedule grains, which is the trap the dither work had to route
around with two extra streams; it is identical however the block is split; and it is reproducible
across runs. `turning_jitter_does_not_disturb_any_other_draw` pins that anyway, because *by
construction* is a property of today's code and this defect was found the hard way once.

**It also changes what `transparent` means.** That flag skips the hold path entirely, and a wandering
clock is a reason not to skip it even at the native rate — otherwise the control would be dead at the
setting most of a session is spent at, which is exactly the failing that removed its predecessor.

**The range is just under one cell, and the bound is structural.** A clock's sampling instants are
*ordered*: instant `n` happens before instant `n + 1`. Cell `n` is displaced by `±JITTER_CELLS/2`, so
consecutive instants stay ordered exactly while `JITTER_CELLS < 1`. A first version used `4.0`
because it was four times louder — and at that width the instants **crossed**, so frames came back
out of order. That is address scrambling wearing this control's name, and the review caught it.
`a_jittering_clock_never_runs_backwards` walks ten thousand cells at four rates, both sides of zero
for a reversed layer, and holds the bound.

**Both ends of the interval are displaced, which is the other half of being a clock.** The first
version displaced only the cell the position fell in and then stepped a *nominal* grid to find its
neighbour, interpolating across a nominal interval — two instants that disagreed with each other.
Asking the same function for instant `n` and instant `n + 1` makes the reconstruction a crossfade
between the samples the converter actually took.

**Measured at `0.9`**: the deviation climbs from 2.5% of the signal at a twentieth of the knob to
12.4% at the top, monotonically, with no saturation anywhere in the travel. **12% rather than the
72% the scrambling version reached, and that is the price of being a clock** — worth knowing if a
wilder sound is ever wanted, because that would be a *different* effect and should carry a different
name.

**The drive stage did not move.** It is after the converter, which is where it has always been, and
moving it would move every shipped sound. Its field was documented as an *"analytic input companding
drive"* and is neither companding nor an input stage; that comment is corrected, because a real
compander beside it made the wrong name misleading rather than merely inaccurate.

**Companding saturates at full scale rather than wrapping or rescaling.** A voice sums two layers
whose `level` each reach 200%, so out-of-range input is the ordinary case here and not an edge. Of
the three available answers, saturation is the conservative one — wrapping folds a loud note to a
quiet one, and rescaling makes the character depend on how hard the converter is driven — and it is
what a DAC with a full-scale code did anyway.

**Clean is bit-exact.** At `Linear` the converter matches the pre-companding arithmetic over the
rate × depth × reconstruction × drive grid, and zero Jitter leaves the cell boundary untouched.
`a_linear_converter_with_no_dither_is_the_converter_this_instrument_shipped` and
`examples/measure_baseline.rs` protect that endpoint. The owner-selected Init deliberately sits away
from it; Clean remains available by returning Character controls to zero.

**Sample rate reads in hertz, quoted against the loaded sample.** `params.source_rate` is an
`Arc<AtomicU32>` captured by the formatter and published by `process` from the first loaded layer;
a dimensionless divisor is not an honest value for a control named Sample rate.

**It is quoted against the source rate and never the host's, and the audio agrees.** A patch has to
sound the same at 44.1 and 96 kHz. It does, and not by care: the grid is counted in *source frames*,
so the converter's rate works out to `pitch_ratio × source_rate / grid` and the host rate cancels
out of the arithmetic. Measured — one 44.1 kHz sample rendered at 44100, 48000 and 96000, the
converter's image stays at 978 Hz at magnitude 0.0698 / 0.0692 / 0.0644, and the fundamental does
not move either. Quoting this readout against the host rate would have shown a number the sound does
not depend on, which is the one way it could lie about a preset.

**Reconstruction is `Bit smoothing`, on the owner's naming.** They read the description, said *"I
understand what reconstruction is now — it is bit smoothing"*, and named it. Recorded because the
name is a shade off what the stage does: it smooths between *samples in time*, not between bit
levels, so sitting beside Bit depth it can be read as smoothing the quantisation steps, which is a
different stage. The owner was told and chose the name; the `#[id]` stays `reconstruction`.

**`Converter drive` draws as `Drive`.** The card is titled, so the knob under it does not repeat the
word. **The host name is untouched** — the Filter card also has a `Drive` and a DAW's automation
list has to tell the two apart — so this is `layer_label`'s doing, the same `name`/`label` split
design system §7.1 draws for the `A `/`B ` prefix, with the sentence case restored after the strip.

**Control descriptions say what is heard, not how it is implemented.** Reconstruction uses
*smooth* ↔ *stepped* vocabulary; internal terms such as virtual rate or analytic curve stay out of
player-facing copy.

**Rate and depth use perceptually useful curves.** Geometric rate and square-rooted bits put 12 kHz
and 8 bits at mid-travel; linearized alternatives spend much of the lower half above audible change.
`examples/measure_converter.rs` prints both tapers and the residual each stage leaves, which is how
this was found and how a future change to either law should be checked. Measured through the
plugin, each stage at 50%: bit depth `0.244`, converter drive `0.322`, sample rate `0.089` of the
render unexplainable by any level trim. Both endpoints are unchanged, so Clean is still exactly
transparent and the floor is still 5 bits.

**The virtual converter varispeeds by construction.** `grid` counts source frames, so one cell lasts
`grid / ratio` output samples and effective converter rate is `ratio × host_rate / grid`: it follows
the note like a per-voice DAC clock. Measured, 400 Hz sine at full hold and grid 32, the first image moves from 1100 Hz at
the root to 550 Hz an octave down at the same magnitude. The full table and the evidence caveat on
the historical claim are [`docs/oscillators/14-samplers.md`](https://github.com/mxm-audio/mxm-kit/blob/main/docs/oscillators/14-samplers.md)
§14.6. **TAL-Sampler decouples them instead** — down-sample, model the DAC, resample for pitch
(§14.1) — so this instrument is not copying that plugin here, and the difference is deliberate now
that it is known. The Sample rate reading is therefore the root's: every other note's converter clock
runs somewhere else, which a knob's kilohertz cannot show and this paragraph records.

**Reconstruction has nothing to do at unity rate.** It shapes how the reader fills the space
*between* source frames, so a note sounding at exactly the root — the init patch's own case, and the
first thing anyone plays — lands on whole frames and leaves the stage a measured `0.0000` residual,
against `0.0723` a fourth up. That is correct physics and a bad surprise. A caption once said so on
the card; it went under the no-prose ruling above, and the fact is kept here.

`character_rate_divisor`, `character_bits` and `character_drive_db` are **public in the DSP and
called by the formatters**, so the panel cannot drift from what the audio does. The divisor is the
honest absolute for the control — a source's own rate is not the panel's to assume — and the Sample
rate reading resolves it against the loaded sample's rate (*Sample rate reads in hertz*, above),
which is where kilohertz can be told truthfully.

## Grain shape and the grain display

### Grain shape is a control now, and the normalisation follows it

**The grain envelope is a tone control**: under a
periodic scheduler it *is* the waveform of an amplitude modulator whose period is the grain, so its
shape decides how many sidebands there are and how far they reach. `agrainshape`/`bgrainshape` read
`Hard`, `Triangle`, `Smooth` with percentages between, the way grain-fx's does — a bare "62%" says
nothing about how a grain sounds.

**The law is grain-fx's `window::window`, reproduced rather than shared.** Both DSP crates depend on
nothing but core, and a dependency between two plugins' engines is the wrong shape to carry one
function; if a third instrument wants it, that is the moment for a shared crate. The midpoint is an
exact triangle from either side — the published implementation the research read has a silent point
there where every grain renders at zero gain, and defects are not reproduced.

**The smooth end cost no new table.** `sin²(ramp·π/2)` is this crate's own Hann read at half the
ramp, so the 1024-point table already here is the whole upper branch.

Window mean and mean-square depend on shape; fixed Hann values would change cloud level by about
6 dB toward Hard. `window_moments` therefore integrates both at control rate using a 128-point
midpoint rule. Measured
through the engine on a 40 Hz cloud: **0.72 dB** of level change from Hard to Smooth while the
residual at Hard is **0.5963** — six-tenths of the render that no level trim explains. That is the
shape of a tone control, and the pair of numbers is what a future change to either law should be
checked against.

**Stretch keeps the plain Hann.** Its two-head crossfade is a different window doing a different
job, and a grain control has no business in it.

The curve is drawn beside the knobs by `visuals::envelope`, reading the DSP's own `grain_window` so
the picture cannot drift from the envelope the grains are given. Shape sits on the first grain row
with size and rate — the three are what a grain *is*, and the row below is what varies from one
grain to the next.

### The grains are drawn on the waveform, not on a plot of their own

The grain display shares the Sample waveform’s `0..=1` source axis and sits beneath region handles.
A separate time/position plot would duplicate the sample picture and force the reader to correlate
two axes.

**A grain is the span of source it is reading** — a segment from where it began to where it has got
to — so the picture answers both halves at once: where grains start, and where they go. Forward
reads draw to the right of their onset and reversed ones to the left, which is what makes Reverse
chance and a negative Scan speed visible rather than only audible. They travel: the editor repaints
at 40 ms, each frame's segment is longer than the last, and a grain sweeps out of its onset and
fades as its window closes.

**Live state, not a log.** An overlay needs current position, not onset history. `Engine::grain_views`
reads each grain’s start, age, step and length from voices once per block; there is no accumulating
ring or second source of truth.

**The marks are real.** A picture computed from the controls would draw an even comb exactly where
Onset timing has handed over to a per-sample draw and the sound is a cloud — the lie a display
exists to prevent, as `mxm-chorus-06`'s Sweep records.

**A stable row per grain, taken from its start position.** Stacking every grain on one line would
hide the one thing the picture is for, that a cloud is many reads at once. The row cannot come from
the grain's slot, because the engine fills a dead slot by moving the last grain into it and the row
would jump; the start position does not change for the life of the grain, so a row derived from it
does not either.

**Grain trails are two points wide.** A hairline disappears into the waveform and a heavier mark
turns a 256-read texture into a block. The trail carries width and the head carries contrast: the head is drawn taller and at full weight, so one grain is findable
without every grain having to shout. The waveform under it
was thinned in the same pass — 640 bins over a 440-point canvas make a bin 0.69 wide, and forcing
the stroke to a full point made every bar overlap its neighbours by nearly half.

**One layer, and the editor says which.** `Telemetry::show_layer` is the handshake: publishing both
would double the traffic for a picture that can only be drawn over one waveform at a time. The
callback publishes only the layer being looked at, and `telemetry` reaches the card as an `Option`
so a layout test may draw nothing — an overlay costs no height either way, which is why this one
cannot move a card floor the way the old plot did.

## Editor

### Editor and native drop

The editor uses shared theme, app bar, controls, navigation and space-derived paging. It opens on
Source and has seven cards from the §14 brief; there is no macro rail. Nothing is drawn to measure
a card, so a drop, a load or a generate is scheduled only by the one paint of its card. Waveform data
is rebuilt only when selected layer/revision changes.

**Every card body is a `mxm_ui::tree`** (`sections::card`), built each frame from the parameters
and `sections::Shown` — the selected layer, which layers hold a sample, their names and the load
state — measured for its floor and height and drawn leaf by leaf through the bindings
(`sections::paint`, `paging::editor::show`). **Floors are computed**, never typed:
`page_items` takes each from its card's tree, and no usability minimum is declared; each card is
exactly as wide as its floor (`plans/plan-editor-standard.md` A1). The opening size is derived, not
typed: the budget hugged around every page (`REFERENCE`, held by
`the_opening_size_is_the_budget_hugged`). **Painted names** drop the layer the layer bar names and
the module a card is titled by (`binding::layer_label`): *Rate 1* and *Shape 1* on *LFOs*, *Mode* on
Playback and Filter; a route stack's title drops the layer too (*Scan speed*). **The Grain overlap
line reserves its longest reading**, so dragging Grain size or rate never re-wraps it. An empty selected layer drops the region, level and Root rows; a name
wider than `CHIP_WIDTH` widens its chip and the floor; the resting status line wraps and a status
carrying a file name truncates. **The Playback card reserves its tallest reader** (`tree::reserve`):
it draws the selected reader's controls and is always as tall as Grain's, so the rows below never
jump. What the editor states for what it draws itself: the waveform is `sections::WAVEFORM_HEIGHT`
tall and the amplitude envelope `sections::ENVELOPE_HEIGHT`, both filling their card; the grain
envelope is `visuals::ENVELOPE_HEIGHT` tall and at least `visuals::envelope_min_width()`; a chip is
at least `CHIP_WIDTH` × `MIN_TARGET`, its cross a `MIN_TARGET` square and Load `MIN_TARGET` tall,
the row's pinned interact height; knob columns are `KNOB_COLUMN_MIN`.

**Load's dialog never runs inside a frame.** It opened synchronously until 2026-09-15, when the same
code in mxm-fx-convolution aborted the player: the dialog's modal loop re-entered egui-baseview
mid-frame (`RefCell already mutably borrowed` inside a window procedure). Load now starts it through
`mxm_ui::offthread` under `sections::load_picker`, is disabled while it is open, and the editor frame
collects the `(layer, path)` pick — waiting out a load already in flight — and loads it exactly as a
drop.

**Its master output sits outside the paging renderer, in the app bar (design system §3.1).**
`mxm_ui::navigation::paged` derives card order and geometry from the paging report and cannot see
controls above that surface, so `sections::master` draws inside `mxm_ui::navigation::bar_card` and
the editor drives the cursor with `paged_with_bar`. The seam moved into `mxm-ui` on 2026-09-18 when
`mxm-drum-machine` became its second user.

**Coverage is proved twice, once per selected layer.** The Source and Reader cards show one layer at
a time—the brief collapses unselected detail—so no single pass can reach the whole declared
parameter surface, and asserting `Coverage::Within` would prove nothing. Each pass asserts
`Exactly` the ids that layer selection reaches.

### A fully-routed card does not fit a page, and that is open

**A fully-routed card does not fit a page, and that is open.** Eleven sources times one row each is
more height than any card has beside its own controls: with every route revealed the Keyboard card
is taller than the pager's viewport at `REFERENCE`, so the page scrolls, and
`mxm_plugin_test::paging_checks` forbids scrolling at the reference size. The paging test therefore paints at
the init patch, which is a **known limit rather than an oversight** — the honest options are that a
fully-routed card is allowed to scroll, or that the source list is shorter, and both are the owner's.

### A developer channel, off unless the environment asks for it

`plugins/AGENTS.md`'s channel, in the collection's shape: `MXM_DEV_CC` read once into `dev_cc`
when an instance is made, four `MidiCC` arms guarded by it, request slots on `Telemetry` taken once,
and the editor honouring them at the top of `panel`, after `hold` and before the cursor moves.

- **CC 119 values 0–5 select a category's first card** through `paging::editor::developer_request`:
  Performance is Keyboard, Modulators Envelope, Generators Sample, Tone Filter. Sequencers and
  Effects have no cards here and move nothing.
- **CC 119 value 127 is the tabless Parameters surface**: every parameter the preset system
  declares, the 110 routing pairs included, as sliders. A musician page draws an absent route as
  nothing, so this is the one surface a script can reach all of them on. The cursor is stopped there
  — the app bar master's card scope included — so the sliders keep their bare arrows.
- **CC 118 is answered with nothing**, because this editor has no expander: the converter's stages
  sit on their card, and an unselected layer's detail follows the layer bar rather than a disclosure.
- **CC 117** opens and closes the preset browser; **CC 116** applies a theme and never saves it.
- The editor already asks for a frame every 40 ms while open, inside the contract's 50.

`the_developer_channel_is_off_unless_the_environment_asked_for_it` holds the gate, and
`developer_requests_reach_each_category_the_parameters_surface_the_browser_and_the_theme` holds the
editor half on the real panel.

### Everything that belongs to a layer lives under the layer bar

The Sample card carries selection, waveform, then two knob rows and the Root helpers, in this order
(owner, 2026-09-13):

- **Start · End · Loop start · Loop end · Crossfade** — the region in the order it plays, ending on
  the fade that joins the loop back to itself.
- **Level · Pan · Root.**
- **Oct − · Oct + · Auto**, with Auto's outcome. They act on Root and nothing else, so they sit
  under the row that holds it; left where they were, Level and Pan would sit between them and the
  knob they move.

The layer bar supplies context, so local labels are simply *Level*, *Pan* and *Root*.
`the_sample_card_puts_root_after_pan_and_the_crossfade_after_loop_end` asserts the order on the
painted labels, row by row.

**The crossfade knob reads `Crossfade`; its host name is `A Loop crossfade`.** At full length it was
the one Sample-card label that wrapped to two lines in a knob column, which made the region row
taller than its neighbours — the layout test measured 32 points against 16. The row already names
the loop twice beside it; an automation list has no row, so the host name keeps the word.
`binding::layer_label` does the strip, the same `name`/`label` split as `Converter drive`.

**A layer chip is a button at least `CHIP_WIDTH` wide.** The name elides inside it to
`CHIP_NAME_BUDGET` characters, so a loaded file name moves neighboring controls only when those
characters are wider than the chip — and then the card's computed floor grows with it.

**Each chip has its own cross, and the row is one height.** One cross acting on the selection meant
reaching a layer in two steps — select it, then remove it — and the owner's case is the direct one:
clear the second layer while looking at the first. So the cross sits after the chip it empties, and
`Load` alone still follows the selection, because it needs a single target and the bar names one.
The square is allocated whether or not it draws, so the row does not reflow as layers load and the
card's floor does not depend on what is loaded. Emptying the only loaded layer is allowed and leaves
a silent instrument with its drop target intact, which is the state a sampler with nothing in it has
anyway.

It is `mxm_ui::control::remove_mark` and not `remove_button`: the mark alone, no frame, because a
framed cross at the end of a row of framed chips reads as another chip rather than as an action on
the one before it. The row pins `interact_size.y` to `MIN_TARGET`, so chips, crosses and Load are
all that tall. The row is the Sample card's widest, so it sets the card's computed floor.

### The layer bar names files, not letters

Layer chips and local labels use loaded file names and selected-layer context rather than repeating
A/B on the panel. Host parameter names retain A/B for automation identity.

**A chip is a button at least `CHIP_WIDTH` wide.** `CHIP_WIDTH` and name elision keep source names
from moving neighboring controls, unless the elided characters are wider than the chip.

**Remove draws its cross rather than spelling one.** A `✕` came out as `?` — the editor's font has
no glyph for U+2715 — so this uses `mxm_ui::control::remove_button`, which paints two line segments.
A control whose meaning is a codepoint depends on a font it does not ship. The same caution is why
the rate divisor reads `rate / 4.0` rather than using a division sign.

### A source name is the one string this instrument does not choose

A source name is unbounded external text. With `mxm_ui::flow` and `flex_shrink: 0`, extending it can
paint through the next card even when allocation rectangles look valid. **Truncate or elide; never
extend user-provided names.**

Both places that show it are bounded now, and the full name is always one hover away:

- The **A/B chips** elide from the middle at `CHIP_NAME_BUDGET`, keeping both ends. A library name
  carries meaning at the front *and* the back — instrument, then key or tempo and the extension — so
  `SIG_GV_75_vibra…_Bbmin.wav` stays identifiable where a tail-truncated one does not.
- The **status line** and the failure message use `Label::truncate`, which measures against the
  card's real allocation. That is the pixel backstop; the chips' character budget only approximates
  it.

`elision_tests` holds the bound, both ends, and that elision counts characters rather than bytes —
a name with multi-byte glyphs must not panic or split one.

### The editor follows the acquisition-to-performance workflow

The surface follows one hierarchy: acquire a layer, edit its source, choose a reader, shape and
perform it. Current workflow contracts:

- **The waveform is the region editor.** Start, End, Loop start and Loop end are handles on the
  canvas with the numeric knobs kept beneath, and a playhead runs over them. Brief §8 asked for all
  four and the code drew a bare polyline. The handles follow their *parameter*, so dragging End past
  Start swaps their roles exactly as `Region::new` does, and a loop handle outside the region is
  drawn where the DSP will actually read it. **A drag takes the circle it is pressed on** (owner,
  2026-09-14): the press's half of the canvas picks the row — filled Start and End at the top, hollow
  loop points at the bottom — and its position the circle, so two handles on one line never move
  together; two circles on one spot go the way the drag goes. The grab once measured only the
  distance across the waveform, so a tie went to Start or End, and a loop handle drawn on their line
  followed them. `sections::grab` holds the rule. **The drag writes from its first frame**: the
  handle taken is used on the frame it is taken, not remembered for the next, so the shortest drag —
  press, one move, release — writes on its one moving frame. egui 0.36 never starts a drag in the
  frame the button is released, so a press and release in one frame opens no gesture at all.
  `a_waveform_drag_writes_from_its_first_frame_and_always_closes` drives both with real pointer
  events. Bins carry min **and** max, because one signed value
  per bin draws a barcode rather than a waveform.
- **A playhead needs a channel, and there was none.** `Engine::playhead` reports the newest sounding
  voice's read position and `Telemetry` carries one normalised scalar per layer. Still no sample
  asset on that channel.
- **A mode's controls belong to that mode.** The Playback card shows scan speed for Stretch and
  Grain and the grain controls only for Grain. The card's tree reserves the tallest reader
  (`tree::reserve`), or it would be planned at its shortest and the rows below would jump; its floor
  is the widest reader's.
- **Two overloaded controls became four honest ones.** Scatter drove both the read-position draw and
  the onset schedule; Chaos drove both stereo spread and per-grain reversal. They are Position
  variation, Onset timing, Stereo width and Reverse chance. Reverse chance is now a plain
  probability rather than a law that only opened past a shared knob's halfway point, and the
  normalisation law gates on the position draw alone — deleting a compromise the DSP DOX had to
  record when one knob drove both.
- **Host names keep their A/B prefix; the labels drop it.** Two layers need two distinct automatable
  parameters, so `A Transpose` and `B Transpose` both exist; the layer bar is what lets the knob
  read `Transpose`.
- **Master moved to the app bar**, design system §3.1 slot 6, still `#[id = "master"]` and still
  reachable by the keyboard cursor through its own card scope.
- **`clear_layer` finally has a caller.** The layer bar's `✕` is the interface for it; it had been
  in `AssetField` since the first slice with nothing able to reach it.

### No explanatory prose under the knobs

**No explanatory prose under the knobs** — the owner's ruling on the Converter card's two captions:
*not good interface*, documentation printed on a panel. Their values carry the operating
information; durable explanation belongs here and in oscillator chapter §14.6. The ruling reaches
past that card: the loop crossfade shows where it is limited or inert through the waveform's
shading, not a caption. **The collection's rule since 2026-09-27** (design system §7.6), and this
editor is clear of it: the Voices line, the Envelope, LFO and Filter notes and the overlap
reading's *shorten or slow down* are gone. The facts are the tooltips of the controls they are about —
Voices, Attack (the filter's Envelope route follows it), the LFO rates (free-running, added from
any control's modulate menu), Cutoff (the envelope and pressure open it further), the grain sizes and
rates (what to do when more overlap than can play). Live readings — the overlap count, what the
converter heard — remain.

### Card descriptions name their current destinations

**The former Travel LFO's reaches stay on the routes it became.** LFO 1 reaches the layers’ **Scan
speed** in opposite directions: `FULL_SCALE` adds `0.6` to A's `stretch_travel` and subtracts `0.47`
from B's. It is inert in Repitch because only Stretch and Grain read `stretch_travel`; the route
stack lives beside that destination.

**Pressure is available at its destination.** The Filter card's route stack can connect note
pressure to Cutoff beside the Envelope route that affects the same control; the owner-selected Init
leaves Pressure absent.

Owed, and deliberately not taken here: the amplitude envelope's drawing is modelled on
`mxm-mono-01`'s `visuals::envelope_shape` rather than sharing it, because that one carries a live
position marker this instrument publishes no telemetry for. Unify them when mono-01 is next opened.

### A route stack belongs under the control it moves

**The LFO card carried the scan-speed stacks, and that was the wrong card.** The owner's words:
*the faders don't modulate the LFOs, so they are placed poorly*. Quite right — a route stack belongs
**under the control it moves**, which is the rule mxm-mono-00's `plugins/mxm-mono-00/AGENTS.md` records and design
system §7.4 makes normative, and the first version of this conversion broke it by leaving Scan speed
A and B on the card whose knobs are the two LFOs. They are on the **Playback** card now, under the
Scan speed knob, drawn under the same condition the knob is — so the *needs Stretch or Grain* caption
is gone too: in Repitch there is no scan speed on the card and no stack under it either.

**What the card is now is an LFO module**, which is what *fold the Travel LFO in* meant: the Travel
LFO stops being a module with a destination and becomes two ordinary sources. Its sound survives the
move because the opposed direction lives in the route's **scale**, not in the knob that was lost.

### The loop has a crossfade, and the waveform draws it

**One per layer, `aloopcrossfade` and `bloopcrossfade`, in seconds of source up to
`MAX_LOOP_CROSSFADE_S`.** The seam itself — the virtual loop signal, room order and borrowing, the
gain law, Grain's exclusion — belongs to the DSP and is documented in
[`crates/mxm-creative-sampler-dsp/NOTES.md`](../../crates/mxm-creative-sampler-dsp/NOTES.md#a-forward-loops-seam-is-a-property-of-the-frames-read). The
plugin owns the parameter, its reading and where it is drawn. Delivered by
`plans/plan-sampler-loop-crossfade.md`, in the private archive.

- **It reads in milliseconds below a second and in seconds above**, the branch decided on the rounded
  millisecond reading so the two units partition the travel; `every_parameter_round_trips_its_own_text`
  holds it.
- **Smoothed like the loop points**, because it moves the same geometry they do, the return point
  included — and settled on its target with them on the first callback that hears a new source.
- **The waveform shades the fade where it is heard** — the stretch before Loop end that fades out, and
  the loop's opening from Loop start that fades in over it (owner, 2026-09-14: the fade's material
  comes from inside the loop only). No line marks the return point: one did, and it read as a start
  marker, where a note starts at Start (at End, reversed). `region_handles` takes that geometry from
  `mxm_creative_sampler_dsp::loop_crossfade`, the arithmetic the readers fit the seam with, and reads
  the parameters' values rather than their smoothers, so drawing never advances one.
- **Shading shorter than the knob asks for, or none at all, is the indication** that the loop limits
  the fade to half its length, or that the layer is not a forward loop on Repitch or Stretch. The plan
  asked for a caption; *No explanatory prose under the knobs*, above, decided against it.
- **Heard through the host**: `a_loop_crossfade_smooths_the_seam_through_the_host`, in
  `plugins/mxm-creative-sampler/host-tests/tests/behaviour.rs`, renders the bundle over a seam that does
  not close with the crossfade off and on, **in Repitch and in Stretch** — Stretch's heads and
  alignment search reach the seam by a path Repitch's does not prove. `routing_is_block_partition_invariant` carries a
  crossfaded Stretch loop short enough that its seam falls inside the rendered blocks.

### Editor tree checks and review pictures

`every_card_passes_the_tree_checks_in_every_state` (`src/editor.rs`) runs
`mxm_plugin_test::tree_checks`'s per-card checks — floor holds, content floor exact, stated height
drawn, nothing outside its leaf, grains and playhead forced on — over this editor's structural
states: Init (A loaded, B empty) with each layer selected, both loaded, every route revealed at full
negative depth, each reader, Grain asking for more than it plays, each voice mode, each load state
with a long name or message, Auto's longest answer and long names on both chips. Tests take floors
from `sections::test_items`, which runs `page_items` in a scratch editor context. Review pictures of
every page, light and dark, at the init patch:
`MXM_PICTURES=after cargo test -p mxm-creative-sampler --lib tree_pictures -- --ignored`, written to
`target/layout-tree/mxm-creative-sampler/after/`.

## Modulation

### The modulation conversion, and what it cost

**This is the collection's second instrument on `plans/plan-modulation-routing.md` (in the private archive), and the first polyphonic one.**
Five targets — Amplitude, Cutoff, Pitch, and each layer's Scan speed — by eleven sources, a presence
and a signed amount each. `crates/mxm-creative-sampler-dsp/src/routing.rs` is the instrument's own
declaration; `src/routes.rs` is its parameters, their ids mechanically derived from one
`#[nested(id_prefix = …)]` per target so they cannot drift from another instrument's. **The prefix
carries no trailing underscore** — the derive inserts its own separator, and a prefix ending in one
produces `mod_amp__lfo1`; the id table's own test caught that on the first run.

**Three knobs retired into routes, and each is the route that replaces it, not an approximation.**

| Retired | Became | Why it is the same sound |
|---|---|---|
| `velsense` | (Amplitude ← Velocity) | With Velocity the standard's `v − 1`, the amplitude factor `1 + sense × (v − 1)` *is* `1 − sense × (1 − velocity)` term for term |
| `envamount` | (Cutoff ← Envelope) | The route's scale is `FILTER_ENV_OCTAVES`, the reach the fixed path had |
| `traveldepth` | (Scan A ← LFO 1) and (Scan B ← LFO 1) | The two reaches `+0.6` and `−0.47` are the route's **scale**, so the layers still pull apart |

**The amplitude target is the collection's factor on the envelope** (`plans/plan-modulation-standard.md`),
and that is load-bearing twice. `plan-modulation-routing.md` §4.1 requires the target that ends a
voice's life to take its routes as a factor — a latched source *added* to the gain keeps a voice
audible after its envelope ends. And with Velocity published as `v − 1` the factor **is** the
velocity knob, where a raw velocity could not reach it at any amount: it would give unity at rest
rising *above* unity as you play harder, where the knob gives unity at full velocity falling to
silence. It replaced a `product` law on 2026-09-26; for every factory design the two render the same
samples.

**Velocity to the filter is the one genuinely new route.** It remains available as an ordinary
pair; the owner-selected Init leaves it absent, matching the promoted B patch. Pressure → Cutoff is
also absent there rather than carrying the earlier fixed full-depth reach.

**Scope: one global frame and one voice frame per voice**, which is §4.1's model and the reason it
exists — that section names this instrument. The two LFOs are evaluated once per sample by the engine
and published into each voice's frame, which §4.1 sanctions as *"simply the same value in every
voice"*; the per-channel wheel and bend are projected the same way, narrowed to the voice's own
channel, so **no third frame scope is introduced**. A voice frame ceases with its voice, so a latched
key cannot hold a patch open. Topology is compacted **once on the engine**, not once per voice, and
a started voice is armed from it — `Voice::start` resets the graph, so without that a note allocated
after the last topology change would render with nothing routed.

**The audio sources project through `Sample::mono_mix`, never `(L + R) / 2`**, because that sum is
silence on an anti-phase source — the failure this crate already met in the alignment search and
already solved by picking a mix at load. **When** they publish is below, and it is the end of the
sample rather than the moment each layer produces its tap.

**A mono fallback restores a press, and the ledger is what makes that possible.** Releasing the top
of a trill hands the voice back to a key pressed earlier — a different note, with its own pressure,
its own random draw and its own per-note expression. Only the pitch used to change, so the released
key's state rode along: wrong on a same-channel trill as well as across channels, and it made the
random per *phrase* against its declared per-note lifetime. `Ledger` holds a `Press` — note,
pressure, draw and expression — and per-note expression events are recorded against the press as well
as the voice, or an uncovered key would jump back to what it started with rather than what it had.

**The press path is deliberately the other way round**: a *new* key covering a held one carries the
held expression forward, because a controller position should not vanish mid-phrase. Restoring runs
on release, carrying on press; the two directions are not the same rule and the code says which is
which.

**One test proves the timed half this repository can prove, and one names what it cannot.**
`a_presence_written_between_blocks_is_live_on_the_next_blocks_first_sample` shows the plugin reads
topology at the start of a block rather than carrying a stale one, and
`routing_is_block_partition_invariant` shows that where the boundaries fall changes no sample — which
is the property the wrapper's parameter-event split depends on. **What has no test is the wrapper's
own split**: `SAMPLE_ACCURATE_AUTOMATION` is true here and nice-plug consumes parameter events
itself, splitting the outer buffer at each one before `process` is called
(`handle_in_events_until`), so proving *that* is proving the vendored wrapper rather than this
plugin. It would need a CLAP host
harness able to inject a parameter event at a sample offset, which no plugin in this repository has.
**Recorded as uncovered rather than implied to be covered**, and it is a harness task if it is wanted.

**A depth is read per sample, not per block, and that is correctness rather than polish.**
`ErasedParam::normalised` is explicitly the *unmodulated* value, so reading a route's amount through
it once a block leaves automation a block stale and makes a host's parameter modulation inaudible
entirely. `Routes::advance` pulls `smoothed.next()` for the **live** routes only — five of fifty-five
at the init patch — and `Engine::set_amounts` carries them across. Topology is the other half and is
set **once per block** by `Engine::set_topology`, because comparing fifty-five presences is not free
and the answer cannot change inside a block. **The two are paired and neither replaces the other**:
topology returns early when nothing moved, so it never delivers a changed depth.

**A route that comes back owes two resets, and both are made** (`docs/code-review-notes.md` §7).
`Routes::routing_from` snaps a newly present route's amount smoother to its stored value, because an
absent route's smoother is not advanced while a host write still moves its target — resuming it
would ramp the route in from a stale depth over a span set by how long it was absent.
`routing::Graph::set_topology` clears, in every voice's frame, a source that has just become needed:
publication is gated on `Routing::needs`, so without the clear a re-added backward route (either
layer's audio) reads the tap from before the gap for one sample.
`a_re_added_route_arrives_at_its_stored_depth_rather_than_ramping_from_a_stale_one` and the DSP's
`a_re_added_backward_route_reads_zero_rather_than_the_tap_from_before_the_gap` hold them.

**`mxm-mono-01` solves this the same way, and that is now two honest copies.** It cannot move into
`mxm-modulation-params` as things stand, because smoothing lives on the concrete `FloatParam` field
rather than on the `Param` trait `ErasedParam` blankets — sharing it needs that trait to gain a
"next smoothed value" method first. Recorded so the third conversion does not rediscover it, and so
`mxm_modulation_params::amounts` is understood for what it is: right only for a route nothing is
moving.

**The performance inputs are per channel, and every voice reads its own channel's.** A single scalar
made every voice read channel 0, so two voices sounding on different channels shared one wheel —
wrong on any MPE controller. `Performance` is an array the engine owns, set by events like every
other piece of live state. **Channel pressure is handled too**: this instrument's own pressure is
per note, which is the right scope for a polyphonic instrument, and a controller without MPE would
otherwise never move the Pressure source at all.

**Both audio sources publish at the very end of a sample**, after every target has been read. They
were published as each layer rendered, which made layer A *current* for anything read later in the
same sample — layer B's scan speed, and the cutoff — while the declared order says every audio route
is backward. One of the two had to be wrong and it was the code: a route is one sample late only if
nothing reads the value in the sample that produced it.

**Each route row's keyboard scope is its parameter's own id.** The shared stack used the literals
`present` and `amount`, so every row in every stack registered the same two keys — the cursor
registry is keyed by name, so fifty-five rows collapsed into two entries and no row could be told
from another. `Route` carries both ids now, which is what lets the **shared** keyboard-coverage check
see routes at all: it matches on parameter ids, and it covers all 110 with no allowance and no
bespoke substitute. `mxm-mono-01` gains the same fix.

**Measured, through the real plugin** (`examples/measure_baseline.rs`, release, this machine):

| | before | after |
|---|---|---|
| then-default patch, one voice | 473.6 ns/sample (2.27% of a core) | **515.1 ns/sample (2.47%)** |
| then-default patch, eight voices | 3712.9 ns/sample (17.82%) | **3785.9 ns/sample (18.17%)** |

These are the modulation conversion's pre-retune measurements; the 2026-09-13 Init selection changed
the default sound after they were taken. The owner's stated requirement was *"best case the default
should not use more cycles than now"*, and
**this does not meet it** — about **9% more on one voice and 1% at full polyphony**. Recorded as a
number rather than absorbed, and it is the owner's to accept or refuse.

**Two obvious savings are already taken and a third was measured and did not help.** The `Routing` no
longer travels inside `Params`, which was a fifty-five-pair copy built by the caller and copied again
by the engine on *every sample*; and the presence comparison moved to once per block. Splitting that
comparison out measured within noise, so what remains is the per-sample smoother advance itself —
five calls at the init patch — and the `Params` rebuild the instrument already did before any of this.

**Three of the four factory recipes render bit-identically** to renders captured before the
conversion. `Held pad` does not, and the cause is understood and inaudible: **a signed amount does
not round-trip through normalised storage for every value.** A bipolar amount is read as
`normalised × 2 − 1`, which is exact for `0.36` and one ULP off for `0.34`; the unipolar percentages
these replaced had no such step. That is the concrete form of §5.1's *the re-used amounts become
signed by migration*. It is one ULP on a depth control, `Held pad`'s peak moved by five parts in a
million, and `a_signed_amount_does_not_always_survive_normalised_storage` pins the mechanism so the
mismatch is not later "fixed" by changing a shared crate the pilot ships pinned digests against.

**A route reads what its pair delivers, in its target's own unit** — `plan-modulation-routing.md`
decision 1.11 — and not as a bare percentage of its amount. The cutoff is why: its three scales
differ, so the Envelope, Pressure and Key rows at one position move the filter four octaves, two
octaves and one octave per octave of keyboard, and all three used to read `100 %`. `routes.rs`'s
`reading` takes the reach from the DSP's `FULL_SCALE`: `oct` on the cutoff, `st` on the pitch, `x`
on Scan speed (the unit its knob reads in, so LFO 1's opposed reaches read `+0.60x` and `-0.47x`),
per octave of keyboard for a Key route, whose source unit is sixty semitones. **Amplitude reads a
percentage of the level**, `+100 %` at full — the velocity-sensitivity knob it replaced — and
`%/oct` from Key, as on every instrument. A route this sampler did not wire before routing reads the
collection's standard reach: Pitch ← Key `+12.00 st/oct`, where it read `+2.40`. Every amount is the
collection's one route parameter, `mxm_modulation_params::reading::amount_param_at` (Init's compiled
amounts are its starting values), with `reading::TIMES` for Scan speed's `x`.
`a_route_reads_what_its_pair_delivers_and_reads_back` pins the readings, and
`every_route_parameter_says_what_the_dsp_does` holds every pair's travel and reading to
`mxm_creative_sampler_dsp::conformance` (`mxm_plugin_test::routing_checks`).

**A route's reading never prints a negative zero.** It formats through `mxm_modulation_params::signed`
(mxm-kit's `crates/mxm-modulation-params/AGENTS.md`); the percentage it replaced printed `-0 %` for any amount
in (−0.005, 0), which parses to zero and prints `0 %`, so the text did not survive the host's round
trip. `every_reading_survives_the_hosts_round_trip_a_rounded_zero_included` walks all 55 amounts
either side of zero. The four factory files' `text` fields were regenerated with the readings; no
`v` moved.

**The control map's `filter.env_amount` role follows its id to the replacement route**, which is §8's
rule: *a role names a (target, source) pair's amount*, and the map changes only where an amount id
retired. `a_control_map_role_never_points_at_a_dead_route` holds that every amount the map names is
a route Init wires, since a knob on an absent pair would turn and do nothing.

**Two LFOs now, both global and free-running**, each with its own rate and a shape — sine, triangle,
both ramps, square and sample-and-hold. They tick whether or not anything reads them, because an LFO
that parked while unrouted would resume from wherever it stopped and a free-running control would
depend on when it was wired; only the *publication* is skipped. Each LFO's rate, and each layer's
grain rate, has the collection's tempo sync (see *Four tempo syncs* above).

**Sample-and-hold draws its first value at reset, not at its first wrap.** It only draws on a phase
wrap, so a held value left at zero meant the shape produced *nothing* until the first cycle
completed — a hundred seconds at the slowest rate this control offers.

## Validation and deliberate exclusions

### Validator quirks this plugin met

**The validator is part of the gate, not an optional extra** — see the run rule in
[`../AGENTS.md`](../AGENTS.md). The local regressions below preserve the product-specific seams it
exercises.

- A `value_to_string` **without a matching `string_to_value`** breaks `param-conversions`. Six
  parameters had one: Direction on each of the two layers, and the four Character stages. A `FloatParam`
  falls back to "strip `with_unit`, then parse", which none of the Character readings survive
  because their units are inside a hand-written `format!`; a `BoolParam` falls back to a
  true/false reader, which never answers to "Reverse". `mxm-grain-fx` recorded this same defect
  before this plugin was written, which is the reason the pin here is
  `every_parameter_round_trips_its_own_text` — it walks [`Params::param_map`] rather than a list
  someone has to remember to extend, and so covers parameters added after it.
- **A formatter's branches have to partition its travel.** Bit smoothing read "smooth" below
  0.001 and printed integer percentages above it, so 0.002 read "0% stepped" — text that parses
  back to a value reading "smooth". One `round() as i32` decides all three branches now.

**Parameter text is idempotent through the host's normalized conversion**: formatted with the unit,
parsed, normalized and formatted again, it is the same string. The twenty-step grid alone passed
five parameters that were not (audit D6), and the rules they taught are:

- **A unit, a precision or a name is chosen from the rounding the finer branch prints**, never from
  the raw value. Cutoff used nice-plug's `v2s_f32_hz_then_khz`, which switches at a raw 1000 Hz: a
  hair above read `1.0 kHz`, parsed to a thousand hertz whose inverse lands under it, and came back
  `1000.0 Hz`. Sample rate switched at raw 10 kHz and 1 kHz (`10.0 kHz` ↔ `10.00 kHz`, and on a
  source under 32 kHz `1000 Hz` ↔ `1.00 kHz`). Both decide on the finer reading's rounding now.
- **Everything one reading shows comes from one rounding.** Root rounded its note from the raw
  value and its cents and trailing number separately, so 59.496 read `B3 +50 ct` and came back
  `C4 −50 ct`, and 0.005 read `+1 ct · 0.00` and came back `· 0.01`. All three come from the whole
  cents now.
- **No reading prints a negative zero.** Pan (`v2s_f32_percentage`) printed `-0%` and Scan speed
  `-0.00x` a hair below centre; `params::unsigned_zero` removes that sign and nothing else. The route
  amounts' own rule is `mxm_modulation_params::signed` (*A route reads what its pair delivers*).

`every_parameter_round_trips_its_own_text` holds it for every parameter in `param_map`, at three
source rates, at the twenty-step grid, `clap-validator` 0.4.1's own grid, both sides of every branch
point its formatters have (`branch_plains`, every note and half note for Root) and a hair either
side of zero. The validator's grid hit none of these slivers, which is why its runs were clean.

### Search bounds remain ordered at the smallest region

`f32::clamp` panics when `min > max`; Start and End are automatable, so Stretch search bounds must
remain ordered even when the region reaches `MIN_REGION_FRAMES`. `param-fuzz-basic` covers the
interior enum variants `Reader::Stretch` and `LoopMode::Forward`; the bounds, modulation and
sample-accurate fuzz tests exercise range ends and cannot replace it.
`no_random_parameter_combination_panics_the_engine` also sweeps each reader explicitly.

### Deliberate exclusions

- **No Blend.** Layers sum through their own Level controls; a missing layer does not attenuate the
  other (`layers_sum_and_a_missing_one_costs_the_other_nothing`).
- **No fixed Air, Body, Motion or Dirt macros.** The host already provides visible, unrestricted
  parameter automation without hiding destinations.
- **No Nudge/randomizer, undo or editor lock taxonomy.** Every panel control is host-visible sound
  state; `explore.rs` is outside the product.

**What this leaves is the point.** Every control on the panel is now a host parameter. There is no
editor-only state to keep consistent, no lock whose scope has to be explained, and no surface that is
not the sound — which is why the paging revision hash no longer hashes lock states, and why the
keyboard cursor's one out-of-band card is the master output rather than a rail.
