# mxm-creative-sampler

A small-sample sound-design instrument: drop one audio file on A or B, play immediately, then turn it into
a patch with Repitch, Stretch or Grain playback, a modelled converter, any-to-any modulation and a
multimode filter.

Part of the MXM collection, the MXM Synth Collection monorepo until the split (2026-10-06): see
the repository's [README](../../README.md). GPL-3.0-or-later — see the repository's
[`LICENSE`](../../LICENSE). CLAP only.

## First playable workflow

1. Play the fresh patch: A contains the generated Init saw at C3/MIDI 48 with the owner-selected
   granular treatment and a transparent converter. Init is compiled in and cannot be overwritten.
2. Drop a sound on the large Source canvas or directly on **A** — WAV, AIFF, FLAC, ALAC, MP3, AAC in M4A
   or Ogg Vorbis. Browse is the secondary path, and still lists WAV files only for now. A WAV
   that carries its own loop arrives looping at its loop points, and in tune when the loop is a
   pitch; Root holds the cents it lands on.
3. Choose Repitch (duration follows pitch), Stretch (travel and pitch separate), or Grain.
4. Add B only when the sound needs another layer.
5. Play it: Poly, Mono or Legato, with glide measured per octave, a bend range you set, and note
   pressure, volume and pan expression where your controller sends them.
6. Route something. Two free-running LFOs, the envelope, velocity, key, the wheel, pressure, bend,
   a per-note random and each layer's own audio reach five destinations: level, cutoff, pitch and
   each layer's scan speed. Every stack sits under the control it moves; `‹ modulate ›` adds a
   source and the cross removes it, keeping the depth for when you add it back.
7. Try the four factory recipes — Long bass, Held pad, Struck hit, Drifting texture. They carry no
   audio of their own: each is a different way of reading *your* sound.
8. Save the host project: canonical stereo sample audio is embedded in plugin state, so the original
   file can move or disappear.

Unsupported, empty, non-finite and oversized files are rejected without replacing the current sound.
Import reads WAV, AIFF, FLAC, ALAC, MP3, AAC in M4A and Ogg Vorbis through the collection's decoder,
judged by content rather than extension, always off the audio thread. Opus is not read. A compressed
file is decoded once into the same embedded stereo audio a WAV becomes, so saved projects do not
depend on the original file or its format.

## Status

**Playable pre-listening slice, complete enough to judge.** Host state and user presets embed A/B
audio; factory recipes and Init preserve it, and new audio is never heard through the patch it
replaces. Parameter IDs, reader constants and mappings are provisional and wait on the owner
listening gate in
`plans/plan-mxm-creative-sampler.md` (in the private archive). **Nudge and the
macro rail were removed** on the owner's ruling — a sequencer and a DAW both do that better — and the
**modulation routing and the second LFO are built**, so the instrument no longer has fixed
destinations to apologise for. Direct recording,
realistic multisampling, slicing, giant-library streaming, sequencing and conventional built-in
effects are intentionally outside the product.

## Building

```bash
cargo test -p mxm-creative-sampler-dsp -p mxm-creative-sampler
cargo xtask bundle mxm-creative-sampler
clap-validator validate "target/bundled/mxm-creative-sampler.clap"
```
