# AGENTS.md — crates/mxm-creative-sampler-state/

Parent: [`../../AGENTS.md`](../../AGENTS.md)

# Purpose

What an `mxm-creative-sampler` state means, with no plugin framework: the parameter table, the
Init patch, the state's JSON and CLAP stream in and out, the embedded audio, the import and what
the plugin decides when a source arrives, and the mapping onto `mxm_creative_sampler_dsp::Params`.

**One crate, so the plugin and anything else reading its state cannot disagree.** The plugin
(`plugins/mxm-creative-sampler`) is built from it, and MXM Tracker, the web tracker
(`plans/plan-web-tracker.md` in newDAWn, W4–W6), plays the DSP in a browser and reads and writes
the plugin's own state in tutti projects through it. The reasoning is in [NOTES.md](NOTES.md).

# Ownership

- `src/params.rs` — every parameter: id, name, group, travel, default, unit, smoothing, its text
  both ways (`Param::text`, `Param::parse`), the layers' ids (`LAYERS`), the shared ids (`id`), and
  the tempo syncs (`TEMPO_SYNCS`, `LFO_SYNC`, `GRAIN_SYNC`). **In the order a host lists them.**
- `src/range.rs`, `src/text.rs` — nice-plug's travel laws and every reading and parser, ported or
  moved; `text.rs` also holds the copy of mxm-kit's route reading (`route_reading`, `route_parse`).
- `src/init.rs` — the one declarative, compiled-in Init patch (sources, `Sound`, routes).
- `src/sound.rs` — the plain values the DSP reads (`Sound`), `Sound::dsp`, and each choice's
  variants and DSP values (`READERS`, `LOOP_MODES`, …).
- `src/routes.rs` — the route ids (`ROUTE_IDS`), names, reach and fader.
- `src/payload.rs` — the `samples` field: `AssetPayload`, its checks, its chunked decode, `prepare`,
  the generated sources' encoding, and every fallible reservation (`reserve`, the `refusal` seam).
- `src/import.rs` — decoding a file or its bytes (`decode_file`, `decode_bytes`), and the arrival
  (`Arrival`, `resolve`, `Arrived::writes`, `root_in_name`).
- `src/state.rs` — `State`: the JSON and the CLAP stream, values, rows' text, `dsp_params`,
  `routing`, `arrive`, `generate`, and the mirror of mxm-kit's `PresetIdentity`. `Values`, every
  parameter's plain value apart from the audio, gives the same `dsp_params` and `routing`: the
  state reads its own values through it, and the web tracker's AudioWorklet holds a track so.
- `src/generate.rs`, `src/naming.rs`, `src/wav_loop.rs` — moved from the plugin whole: the
  seventeen generated sources, what a file name says about its pitch, a WAV's `smpl` loop.

No nice-plug, egui, editor, host or audio-thread type belongs here. The plugin keeps publication
(`asset.rs`), the nice-plug parameters, smoothing and the editor.

# Local Contracts

- **No nice-plug and no egui; every dependency builds for `wasm32-unknown-unknown`.**
  `cargo tree -p mxm-creative-sampler-state` is the check.
- **The plugin's parameters are this table**, held by the plugin's `every_parameter_is_the_tables`
  (ids in order, groups, names, units, steps, defaults, travel, text and parse at three source
  rates) and `every_choice_is_the_tables`. A parameter is added here first, then in the plugin
  (its `AGENTS.md` lists where). **Ids are provisional until the S3 gate** and move together.
- **A value is a plain `f32`**: a switch `0.0` or `1.0`, a choice its index. What a host stores
  for its normalised value is `Param::plain`; what it shows is `Param::text` of the normalised
  value. `State::text` is that, against the state's own source rate.
- **A state reads as the plugin loads one** (`state.rs`'s module docs list the rules) **and writes
  back as the plugin saves one**, byte for byte, version included (`PLUGIN_VERSION`, the
  workspace's). What the plugin refuses whole is a `StateError`. Held by the plugin's
  `a_state_the_plugin_writes_reads_into_what_it_plays_and_writes_back_equal` and
  `a_fresh_instance_writes_the_crates_fresh_state`.
- **One mapping to the DSP**: `Sound::dsp`, which the plugin's callback calls every sample and
  `State::dsp_params` calls with the tempo syncs resolved as the plugin resolves them.
- **An arrival is the plugin's**: `import::resolve` decides, `Arrived::writes` lists each write as
  a normalised value, the plugin writes each as a gesture and `State::arrive` stores
  `Param::plain` of it. Held by the plugin's
  `an_import_arrives_through_the_state_crate_as_through_the_plugin`.
- **Ported code stays ported.** `range.rs` and nice-plug's formatters in `text.rs` are nice-plug
  0.4.2's to the bit; the route reading is mxm-kit's, which the plugin still builds its amounts
  with. Change them only with their originals; the plugin's tests fail if they part.
- **Every buffer a payload, a preset or an import sizes is reserved fallibly** (`payload::reserve`,
  named for the plugin's refusal tests, which enable the `refusal` feature from
  `[dev-dependencies]` only).

# Work Guidance

- Moved code keeps its tests here; the plugin keeps the tests of what it still owns.
- `State::source_rate` falls back to `text::DEFAULT_SOURCE_RATE` where the plugin keeps the last
  rate it published: the one reading that can differ, and only with both layers empty.

# Verification

```bash
cargo test -p mxm-creative-sampler-state
cargo clippy -p mxm-creative-sampler-state --all-targets -- -D warnings
cargo build -p mxm-creative-sampler-state --target wasm32-unknown-unknown
cargo tree -p mxm-creative-sampler-state -e normal    # no nice-plug, no egui
cargo test -p mxm-creative-sampler --lib              # the plugin's tests that hold it to the table
```

# Child DOX Index

None.
