# mxm-creative-sampler — UI design brief

Required by `MXM_DESIGN_SYSTEM.md` §14, and written before the editor it describes. This is an
original small-sample sound-design instrument: drop one sound, play it immediately, then turn it
into an instrument. Workflow evidence is `research:interfaces/creative-sampler-workflows.md`.

**Plugin:** `mxm-creative-sampler`; CLAP id `dk.mxm.mxm-creative-sampler`. Stereo instrument output,
no audio input. Product plan: `plans/plan-mxm-creative-sampler.md`.

## 1. Primary sound-design task

**Put one sound under the keyboard and find the instrument inside it.** Native file drop is the
primary acquisition gesture. The large Source surface and the explicit A/B targets accept files;
Browse is secondary. A valid first drop produces an immediately playable A layer without requiring
mapping, slicing, analysis or a second source. B adds optional depth.

## 2. Controls reached for most

1. **Playback mode** — Repitch, Stretch or Grain: three different intentions, not quality levels.
   **Each mode's controls belong to it**, so Grain's are not on the card while Repitch is selected.
   Grain's random axes are **four separate controls**, not two macros: Pitch variation detunes,
   Position variation decorrelates, Onset timing moves the schedule from periodic to stochastic,
   and Stereo width and Reverse chance are the thrown-about character. Grain rate thickens the
   cloud rather than only changing its texture — *"a short piece of violin sounds like a symphony
   orchestra"* is the owner's test for it, and it is what the mode is for.
2. **Start / End** — choose the useful region without entering a mapping editor.
3. **The converter** — a type (linear or companding), then sample rate, bit depth, reconstruction, drive and jitter, reading in kilohertz, bits and decibels. Not one macro over them: this is the instrument's reason for existing, and the machines it emulates are remembered by those numbers. The type is the one that decides what the bits *mean*, because a vintage sampler's DAC was a distortion character rather than a resolution.
4. **Level and Pan, per layer** — the two layers sum and each one's Level says how loud it is. There was an equal-power Blend across the pair as well and it was one control too many: at Blend 0 a layer's own Level did nothing, so the two disagreed about how loud B was. B stays optional by being empty, not by being faded out.
5. **Filter** — turn the source into a playable tone.
6. **Shape** — the amplitude envelope.

**There is no Layers card.** Level and pan belong to a layer, so they sit under the layer bar that
chooses one — which is also what lets them be called *Level* and *Pan* rather than *A Level* and
*B Level*. A card holding two knobs per layer, each named for its layer, restated what the bar above
it already said.

**There are no macros, and there is no Nudge.** Air, Body, Motion and Dirt were fixed offsets onto
Cutoff, Resonance, scan speed and Character, and every host that loads this instrument already has a
better version of them: `mxm-player`'s sequencer moves any parameter, and a DAW builds whatever macro
set a track actually wants. Four knobs that hid which parameter they moved were worth less than the
parameters. **Nudge** — a bounded randomiser over the patch, with a one-step undo and a per-card
lock — went with them on the owner's ruling. It was approved in the revision 3 blueprint and never
asked for again, and it cost a persistent rail, eight lock toggles and a `Scope` taxonomy to carry.

What is left on the panel is parameters. There is no editor-only state, no surface that is not the
sound.

## 3. Signal flow that must be visible

```text
A sample ─▶ region/loop ─▶ Repitch | Stretch | Grain ─▶ level/pan ─┐
                                                                   ├─▶ sum ─▶ converter
B sample ─▶ region/loop ─▶ Repitch | Stretch | Grain ─▶ level/pan ─┘
       ─▶ driven multimode filter ─▶ amplitude envelope / VCA ─▶ output
```

**The converter is one per voice, after the two layers sum** — where the hardware put its DAC, and
what makes quantise-then-sum genuinely different from sum-then-quantise. It is virtual input-rate conversion, converter
law and reconstruction — not an output bit crusher. **The converter is one per voice, applied to
what the whole voice is sounding**, because quantising is nonlinear and sixteen grains each rounded to
their own grid average toward smoothness instead of giving the stepped edge the hardware had.

## 4. Play view

There is no separate Play tab. The app bar is the performance surface;
space-derived paging shows the cards. The editor opens anchored on **Source** in Generators so load
and play is the first visible task, even though category navigation still follows the collection
order.

## 5. Advanced controls and disclosure

| Card | Controls / state | Job |
|---|---|---|
| **Keyboard** | Poly/Mono/Legato, glide, bend range, and Pitch's route stack | Keyboard response |
| **Envelope** | The drawn shape, attack, decay, sustain, release, then Amplitude's route stack | Final amplitude and exact idle — the Filter's Envelope route is the same shape, which is why this is not called the amplitude envelope |
| **LFOs** | Two rates and two shapes, and nothing else | Movement. A route stack belongs under the control it moves, and these knobs move nothing on this card |
| **Sample** | The layer bar, a picker for the seventeen generated sources, the waveform **with region, loop and playhead handles**, load and remove, and beneath the canvas one row of extents ending on the loop crossfade, then the layer's own level, pan and root, then Auto and the octave shift under Root | Acquisition, source truth, and everything that belongs to the selected layer |
| **Playback** | Mode, loop, direction, transpose, and **only the selected mode's own controls** — scan speed for Stretch and Grain with its own route stack, the grain controls for Grain, with how much of the budget the setting uses | How the source becomes time |
| **Converter** | Type (linear or companding), then sample rate, bit depth, reconstruction, drive and jitter, each reading as the number the machine was sold on | The virtual converter — the part that made a Fairlight or an Emulator sound like itself. The type decides what the bits *mean*, because a vintage DAC was a distortion character rather than a resolution |
| **Filter** | Mode, cutoff, resonance, drive, then Cutoff's route stack | Tone and bounded nonlinear colour, and where the envelope, pressure and velocity reach it |

**Master is not on a card.** Design system §3.1 slot 6 puts the master output control in the app bar
beside the level and clip indicator, which is where it is. It remains an ordinary automatable host
parameter; only where it is drawn is the app bar's business.

The source canvas gives named and numeric alternatives for every handle. Unsupported or failed drops
show the reason and leave the previous sound untouched. Empty B stays visible as a direct drop target
but its detailed source controls collapse.

## 6. Categories, cards, and grouping

| Category | Cards |
|---|---|
| Performance | Keyboard |
| Modulators | Envelope, LFO 1, LFO 2 |
| Generators | Sample, Playback, Converter |
| Tone | Filter |

Cards are ordered by **signal flow inside each category** — Sample, then Playback, then Character;
then Filter. Across categories the collection's own order stands, because a player who
learns one instrument's navigation has learned all thirteen; §4's opening anchor on Sample is what
makes load-and-play the first visible task regardless.

No Sequencers or Effects category is invented. Cards follow the shared paging renderer and reflow
from available space; these responsibilities do not prescribe fixed geometry.

## 7. Identity accent

The collection accent, unchanged in both themes. The sample waveform uses semantic canvas, quiet,
accent and warning tokens; it does not introduce a product-specific chrome colour or imitate a
hardware sampler.

## 8. Live visualization

The selected layer’s bounded waveform overview shows source extent, region, loop and current voice
playhead. Grain mode may add a bounded summary of active read regions, never one GUI object per
grain. Peak telemetry max-combines and clipping latches. Loading, invalid input and an empty layer
have explicit semantic states rather than being inferred from a blank graph.

## 9. Control map

`plugins/mxm-creative-sampler/control-map.json` ships and claims **only roles the standard already
declares** — filter, amplitude envelope, LFO rate, the two layer trims and transposes, and the voice
group including master. An instrument map naming an undeclared role is refused whole, so the
instrument-defining surface stays off it until sampler roles enter the standard, the compiled data
and the player together. That is the one dedicated sampler page this section always wanted —
the converter stages, grain size and rate, and the region extents — and it is a change to the
standard rather than to this plugin.

Root reads as a note name beside its number, `C3 · 48`, on the **C4 = 60** convention that
`mxm-poly-06` uses. `mxm-player`'s sequencer uses one an octave below; the collection disagrees with
itself and this brief does not settle it beyond this instrument.

## 10. Product and Init boundaries

- Two flat whole-keyboard layers only: no zones, realistic multisampling or round robins.
- Repitch, Stretch and Grain; no v1 slicing.
- No direct or retrospective recording, permanently.
- No workstation sequencer, stem workflow, giant-library streaming or conventional built-in effect.
- User patches and host state embed canonical sample audio. Paths are provenance only.
- Factory recipes and Init preserve loaded A/B audio plus root and source extents. Init resets reader,
  character, mix, modulation, filter and envelope processing to defaults.
- **The instrument can make its own sources**: seventeen generated waveforms, sixteen of them
  sweeping their spectrum across their length so the readers turn scan position into timbre, plus
  the static saw the instrument has always started on. The first two are the two ends of one idea —
  the plain saw, to learn the instrument on, and a supersaw that opens from one voice to seven, to
  make the big sounds with. Picking one replaces the
  selected layer exactly as a drop does; a dropped file remains the primary gesture.
- **A fresh instance starts on a generated band-limited saw on A, and the init patch reads it as a
  detuned grain cloud — a string section.** An instrument with nothing loaded cannot be played, and
  four `state = null` recipes have nothing to treat. It is the starting payload, not one Init
  restores: a dropped sample replaces it, and Init keeps whatever the player has loaded.
- Root note, Master, Voices, Glide and Bend range are off the exploration
  surface because a sample's tuning truth and the keyboard in front of the instrument are not
  things to roll dice on — and there are no dice to roll any more.
- Four factory recipes ship for the listening gate — a bass, a pad, a hit and a texture — each
  `state = null`, so they are four treatments of whatever is loaded rather than four sounds. The
  authored bank follows the gate.

The brief does not approve playback quality. Reader constants, capacity, polyphony and exact card
geometry are fixed by implementation measurements and the owner’s playable listening gate.
