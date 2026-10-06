//! `mxm-creative-sampler` — drop a small sound, play it, then find the instrument inside it.

macro_rules! plugin_name {
    () => {
        "mxm-creative-sampler"
    };
}
pub const NAME: &str = plugin_name!();
pub const CLAP_ID: &str = concat!("dk.mxm.", plugin_name!());

pub mod asset;
mod binding;
pub mod editor;
pub mod generate;
mod init;
pub mod naming;
pub mod params;
pub mod preset;
pub mod routes;
mod sections;
pub mod telemetry;
mod visuals;
pub mod wav_loop;

use asset::AssetBank;
use mxm_creative_sampler_dsp::{Activity, Character, Engine, Note, Params as DspParams};
use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use params::MxmCreativeSamplerParams;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

/// A note's identity in the shape the voice logic was written for. nice-plug 0.4 types it
/// (`VoiceID`, `Channel`, `Key`, each with a wildcard); 0.3 handed over a host's wildcard (-1) as
/// 255 and a missing voice id as `None`. Converting here keeps every note decision, and every
/// recorded render, exactly what it was before the upgrade.
fn legacy_note(voice_id: VoiceID, channel: Channel, key: Key) -> (Option<i32>, u8, u8) {
    (
        voice_id.id(),
        channel.number().unwrap_or(u8::MAX),
        key.number().unwrap_or(u8::MAX),
    )
}

const MAX_BLOCK_SIZE: usize = 64;
const NUM_CHANNELS: usize = 16;

/// The developer channel's control changes, answered only when [`DEV_CC_ENV`] was set as the instance
/// was made (mxm-kit's `docs/plugin-conventions.md`, *A developer channel in every editor*). CC 119
/// is a view address, CC 117 the preset browser, CC 116 a theme applied and never saved.
const DEV_VIEW_CC: u8 = 119;
/// Answered with nothing: this editor has no expander to open.
const DEV_DISCLOSURE_CC: u8 = 118;
const DEV_BROWSER_CC: u8 = 117;
const DEV_THEME_CC: u8 = 116;
const DEV_CC_ENV: &str = "MXM_DEV_CC";

#[derive(Debug)]
pub enum EditorTask {
    Load {
        layer: u8,
        path: PathBuf,
    },
    /// Build one of the generated sources into a layer.
    ///
    /// **Off the audio thread and off the UI thread**, the same as a decode, because it is the same
    /// shape of work: the additive render is the small half, and around it sit a base64 encode, one
    /// base64 decode of each layer into its frames, a fingerprint over the whole encoded string, and a
    /// copy of the *other* layer's payload, which the cap allows to be 29.30 MiB of base64.
    Generate {
        layer: u8,
        recipe: generate::Recipe,
    },
    Wake,
}

pub struct MxmCreativeSampler {
    params: Arc<MxmCreativeSamplerParams>,
    assets: Arc<AssetBank>,
    engine: Engine,
    sample_rate: f32,
    /// Each channel's pitch-bender position, `-1..=1`.
    bends: [f32; NUM_CHANNELS],
    /// The routing as the parameters currently describe it.
    ///
    /// **Presences once per block, amounts once per sample.** Topology is discrete and changes only
    /// on a parameter event; a depth is smoothed and a host can modulate it, so reading it per block
    /// would leave automation a block stale and modulation inaudible.
    routing: mxm_creative_sampler_dsp::routing::Routing,
    /// The semitones that position has already been turned into, so a range change is a delta.
    applied_bends: [f32; NUM_CHANNELS],
    /// The Bend range, smoothed, that `applied_bends` were scaled by. A bend event scales by this
    /// too, so an event and a ramp never disagree about what a position means.
    bend_range_applied: f32,
    telemetry: Arc<telemetry::Telemetry>,
    /// The revision each layer's source was last heard at. A callback that reads a newer one settles
    /// that layer's source-owned smoothers before rendering. See [`Self::settle_arrivals`].
    heard_revisions: [u64; 2],
    /// Whether the developer channel answers. Read once, when the instance is made, so no host,
    /// preset or automation lane can switch it on.
    dev_cc: bool,
    /// LFO 1's and LFO 2's rates and A's and B's grain rates as their syncs resolved them for this
    /// callback, or `None` for their free values (`plans/plan-tempo-sync-controls.md`).
    synced: [Option<f32>; 4],
}

impl Default for MxmCreativeSampler {
    fn default() -> Self {
        let params = Arc::new(MxmCreativeSamplerParams::default());
        let assets = params.assets.bank();
        let routing = params.routes.routing();
        let bend_range_applied = params.bend_range.value();
        Self {
            params,
            assets,
            engine: Engine::default(),
            sample_rate: 48_000.0,
            bends: [0.0; NUM_CHANNELS],
            routing,
            applied_bends: [0.0; NUM_CHANNELS],
            bend_range_applied,
            telemetry: telemetry::Telemetry::shared(),
            heard_revisions: [0, 0],
            dev_cc: std::env::var_os(DEV_CC_ENV).is_some(),
            synced: [None; 4],
        }
    }
}

impl MxmCreativeSampler {
    fn next_params(&self) -> DspParams {
        // Synced, a grain rate runs at its division; its free smoother advances either way.
        let mut a = self.params.layer(0);
        let mut b = self.params.layer(1);
        if let Some(hz) = self.synced[2] {
            a.grain_density_hz = hz;
        }
        if let Some(hz) = self.synced[3] {
            b.grain_density_hz = hz;
        }
        DspParams {
            layers: [a, b],
            character: Character {
                rate: self.params.rate.smoothed.next(),
                converter: self.params.converter.smoothed.next(),
                reconstruction: self.params.reconstruction.smoothed.next(),
                input: self.params.input.smoothed.next(),
                converter_type: self.params.converter_type.value().dsp(),
                jitter: self.params.jitter.smoothed.next(),
            },
            attack_s: self.params.attack.smoothed.next(),
            decay_s: self.params.decay.smoothed.next(),
            sustain: self.params.sustain.smoothed.next(),
            release_s: self.params.release.smoothed.next(),
            filter_mode: self.params.filter_mode.value().dsp(),
            cutoff_hz: self.params.cutoff.smoothed.next().clamp(20.0, 22_000.0),
            resonance: self.params.resonance.smoothed.next().clamp(0.0, 0.98),
            filter_drive: self.params.drive.smoothed.next(),
            lfo_rate_hz: {
                let free = self.params.lfo_rate.smoothed.next();
                self.synced[0].unwrap_or(free)
            },
            lfo_shape: self.params.lfo_shape.value().dsp(),
            lfo2_rate_hz: {
                let free = self.params.lfo2_rate.smoothed.next();
                self.synced[1].unwrap_or(free)
            },
            lfo2_shape: self.params.lfo2_shape.value().dsp(),

            master: self.params.master.smoothed.next(),
            voice_mode: self.params.voice_mode.value().dsp(),
            glide_s: self.params.glide.value(),
        }
    }

    /// Sets a channel's wheel, for the timed-routing test.
    pub fn engine_set_wheel_for_test(&mut self, channel: u8, value: f32) {
        self.engine.set_wheel(channel, value);
    }

    /// The parameters, for the measurement seam below, so a harness can apply a preset the way a
    /// player would rather than reaching into the plugin's fields.
    pub fn params_for_test(&self) -> Arc<MxmCreativeSamplerParams> {
        Arc::clone(&self.params)
    }

    /// Starts a note, for the measurement seam below. **Not a second event path.**
    ///
    /// It calls exactly what `handle_event`'s `NoteOn` arm calls, so a measurement runs against the
    /// voices a real press would have made rather than against a construction of its own.
    pub fn note_on_for_test(&mut self, key: u8, velocity: f32) {
        let bank = Arc::clone(&self.assets);
        let assets = bank.read();
        self.settle_arrivals(assets.layer_revisions);
        let samples = [assets.samples[0].as_ref(), assets.samples[1].as_ref()];
        self.routing = self.params.routes.routing_from(&self.routing);
        let note = mxm_creative_sampler_dsp::Note::midi(key, velocity.clamp(0.0, 1.0));
        self.engine.note_on(note, samples, &self.next_params());
    }

    /// **A measurement seam, not a second `process()`.**
    ///
    /// It runs exactly the per-sample body `process()` runs — [`Self::render_frame`], which follows
    /// the Bend range, advances every smoother and rebuilds the `Params`, then calls `Engine::render`
    /// — and deliberately omits the wrapper's event handling and buffer plumbing, which are not what
    /// this work lands on. The pilot carries the same shape for the same reason, and so do the three
    /// effects.
    ///
    /// It exists because [`plans/plan-sampler-modulation-and-converter.md`]'s cost gate must measure
    /// the **plugin** path: the smoothers and the per-sample `Params` rebuild are where the routing
    /// work lands, and a framework-free DSP bench cannot reach them.
    pub fn render_block_for_test(&mut self, left: &mut [f32], right: &mut [f32]) {
        // **The per-block work too, not just the per-sample loop.** `process` reads the routing once
        // a block, so a seam that skipped it would render Init's amounts whatever the patch said —
        // which is exactly what it did, and it made a factory recipe measure three times its peak.
        self.routing = self.params.routes.routing_from(&self.routing);
        self.engine.set_topology(&self.routing);
        let bank = Arc::clone(&self.assets);
        let assets = bank.read();
        self.settle_arrivals(assets.layer_revisions);
        let samples = [assets.samples[0].as_ref(), assets.samples[1].as_ref()];
        for (left, right) in left.iter_mut().zip(right.iter_mut()) {
            let frame = self.render_frame(samples);
            *left = frame.left;
            *right = frame.right;
        }
    }

    /// One rendered sample: the body `process` runs per sample and the measurement seam runs too,
    /// so the two cannot drift apart.
    #[inline]
    fn render_frame(
        &mut self,
        samples: [Option<&mxm_creative_sampler_dsp::Sample>; 2],
    ) -> mxm_creative_sampler_dsp::Stereo {
        self.follow_bend_range();
        let params = self.next_params();
        // **Depths advance per sample**, because they are smoothed and a host can modulate them;
        // only the live ones are touched, which is O(live).
        self.params.routes.advance(&mut self.routing);
        self.engine.set_amounts(&self.routing);
        self.engine.render(samples, &params)
    }

    /// **A new source is final on the first callback that can hear it.** A layer whose published
    /// revision moved since the last callback had its source-owned parameters written as gestures
    /// before the commit, so their targets are the arrival; this settles their smoothers there before
    /// the first sample renders. Every publication that replaces a layer takes it — an import, a
    /// generated pick, a preset or bank load, a state restore. `plans/plan-sampler-wav-loop-import.md`
    /// §1.2.
    fn settle_arrivals(&mut self, layer_revisions: [u64; 2]) {
        for (layer, revision) in layer_revisions.into_iter().enumerate() {
            if revision != self.heard_revisions[layer] {
                self.heard_revisions[layer] = revision;
                self.params.settle_source(layer);
            }
        }
    }

    /// Semitones per unit of channel bend, as the patch currently says — the unsmoothed target, for
    /// activation and reset, where nothing is sounding to ramp.
    fn bend_range(&self) -> f32 {
        self.params.bend_range.value().clamp(0.0, 24.0)
    }

    /// Advances the Bend range smoother by one sample and brings the voices along with it.
    ///
    /// **Once per sample, in `process`'s rendering spans and in its inert ones alike**, so where the
    /// callbacks and the 64-frame spans fall changes nothing. The comparison is the whole cost while
    /// the range holds still.
    #[inline]
    fn follow_bend_range(&mut self) {
        let range = self.params.bend_range.smoothed.next().clamp(0.0, 24.0);
        if range != self.bend_range_applied {
            self.resync_bends(range);
        }
    }

    /// Brings every channel's applied bend up to date with `range`.
    ///
    /// A bend range edited while the wheel is held is an ordinary automation move, and the note
    /// has to follow it. Voices carry accumulated deltas rather than an absolute bend, so the
    /// range change is applied as the difference between what the wheel now means and what was
    /// already applied. **`range` is the smoothed value** ([`Self::follow_bend_range`]): the
    /// unsmoothed one arrived as the whole difference on one sample, a pitch step (audit D11).
    fn resync_bends(&mut self, range: f32) {
        for channel in 0..NUM_CHANNELS {
            let target = self.bends[channel] * range;
            let delta = target - self.applied_bends[channel];
            if delta != 0.0 {
                self.applied_bends[channel] = target;
                self.engine.add_channel_tuning(channel as u8, delta);
            }
        }
        self.bend_range_applied = range;
    }

    fn handle_event(
        &mut self,
        event: NoteEvent<()>,
        samples: [Option<&mxm_creative_sampler_dsp::Sample>; 2],
    ) {
        match event {
            NoteEvent::NoteOn {
                voice_id,
                channel,
                key,
                velocity,
                ..
            } if velocity.is_finite() && velocity > 0.0 => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                let params = self.next_params();
                self.engine.note_on(
                    Note {
                        id: voice_id.map(|id| id as u32),
                        channel,
                        key: note,
                        velocity: velocity.clamp(0.0, 1.0),
                        tuning: self.applied_bends[channel as usize % NUM_CHANNELS],
                    },
                    samples,
                    &params,
                );
            }
            NoteEvent::NoteOn {
                voice_id,
                channel,
                key,
                velocity,
                ..
            } if velocity.is_finite() => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                self.engine
                    .note_off(voice_id.map(|id| id as u32), channel, note);
            }
            NoteEvent::NoteOff {
                voice_id,
                channel,
                key,
                ..
            } => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                self.engine
                    .note_off(voice_id.map(|id| id as u32), channel, note)
            }
            NoteEvent::Choke {
                voice_id,
                channel,
                key,
                ..
            } => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                self.engine
                    .choke(voice_id.map(|id| id as u32), channel, note)
            }
            NoteEvent::PolyTuning {
                voice_id,
                channel,
                key,
                tuning,
                ..
            } if tuning.is_finite() => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                let bend = self.applied_bends[channel as usize % NUM_CHANNELS];
                self.engine.set_note_tuning(
                    voice_id.and_then(|id| u32::try_from(id).ok()),
                    channel,
                    note,
                    (tuning + bend).clamp(-128.0, 128.0),
                );
            }
            NoteEvent::MidiPitchBend { channel, value, .. } if value.is_finite() => {
                let index = channel as usize % NUM_CHANNELS;
                // **The range the voices already hold**, not the edited target: mid-ramp, the
                // target would move this channel the rest of the way on this one event.
                let range = self.bend_range_applied;
                let next = (2.0 * (value - 0.5)).clamp(-1.0, 1.0);
                let target = next * range;
                let delta = target - self.applied_bends[index];
                self.bends[index] = next;
                // The bender is a **source** as well as a pitch offset now, so the routing sees the
                // same position the tuning does. §5.2: an existing gesture path stays where it is
                // while the raw gesture also becomes a source.
                self.engine.set_bend_position(channel, next);
                self.applied_bends[index] = target;
                self.engine.add_channel_tuning(channel, delta);
            }
            NoteEvent::PolyPressure {
                voice_id,
                channel,
                key,
                pressure,
                ..
            } if pressure.is_finite() => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                self.engine.set_pressure(
                    voice_id.and_then(|id| u32::try_from(id).ok()),
                    channel,
                    note,
                    pressure,
                );
            }
            NoteEvent::PolyVolume {
                voice_id,
                channel,
                key,
                gain,
                ..
            } if gain.is_finite() => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                self.engine.set_expression_gain(
                    voice_id.and_then(|id| u32::try_from(id).ok()),
                    channel,
                    note,
                    gain,
                );
            }
            NoteEvent::PolyPan {
                voice_id,
                channel,
                key,
                pan,
                ..
            } if pan.is_finite() => {
                let (voice_id, channel, note) = legacy_note(voice_id, channel, key);
                self.engine.set_expression_pan(
                    voice_id.and_then(|id| u32::try_from(id).ok()),
                    channel,
                    note,
                    pan,
                );
            }
            NoteEvent::MidiCC { cc, .. }
                if cc == control_change::ALL_SOUND_OFF || cc == control_change::ALL_NOTES_OFF =>
            {
                // **A panic ends the notes, not the controllers** (audit D12). The wheel, the
                // bender and channel pressure are positions a controller sends only when they move,
                // and this plugin keeps the bend it tunes new notes by, so the sources those routes
                // read keep their positions too. `reset` and `activate` still return them to
                // neutral.
                let controllers = self.engine.performance();
                self.engine.panic();
                self.engine.restore_performance(controllers);
            }
            // **The developer channel**, inert unless the environment asked for it. The requests go to
            // the editor through telemetry and change nothing the DSP reads.
            NoteEvent::MidiCC {
                cc: DEV_VIEW_CC,
                value,
                ..
            } if self.dev_cc && value.is_finite() => self
                .telemetry
                .request_view((value.clamp(0.0, 1.0) * 127.0).round() as u8),
            NoteEvent::MidiCC {
                cc: DEV_DISCLOSURE_CC,
                ..
            } if self.dev_cc => {}
            NoteEvent::MidiCC {
                cc: DEV_BROWSER_CC,
                value,
                ..
            } if self.dev_cc && value.is_finite() => self.telemetry.request_browser(value >= 0.5),
            NoteEvent::MidiCC {
                cc: DEV_THEME_CC,
                value,
                ..
            } if self.dev_cc && value.is_finite() => self
                .telemetry
                .request_theme((value.clamp(0.0, 1.0) * 127.0).round() as u8),
            // **CC 1, the mod wheel**, which this plugin did not read at all before the conversion.
            // Retained per channel and projected into each voice's frame, which is §4.1's *the same
            // value in every voice* narrowed to every voice on that channel.
            NoteEvent::MidiCC {
                channel, cc, value, ..
            } if cc == control_change::MODULATION_MSB && value.is_finite() => {
                self.engine.set_wheel(channel, value.clamp(0.0, 1.0));
            }
            // **Channel pressure reaches every voice on its channel.** This instrument's own
            // pressure is per note — it reads `PolyPressure` — which is the right scope for a
            // polyphonic instrument and the one §4.1 wants. A keyboard without MPE sends channel
            // pressure instead, and without this the Pressure source would never move for it.
            NoteEvent::MidiChannelPressure {
                channel, pressure, ..
            } if pressure.is_finite() => {
                self.engine
                    .set_channel_pressure(channel, pressure.clamp(0.0, 1.0));
            }
            _ => {}
        }
    }
}

impl Plugin for MxmCreativeSampler {
    const NAME: &'static str = NAME;
    const VENDOR: &'static str = "mxm";
    const URL: &'static str = "https://mxm.dk";
    const EMAIL: &'static str = "plugins@mxm.dk";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = editor::MxmCreativeSamplerEditor;
    type SysExMessage = ();
    type BackgroundTask = EditorTask;

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = Arc::clone(&self.params);
        Box::new(move |task| match task {
            EditorTask::Load { layer, path } => {
                if let Err(error) = params.assets.import_wav(layer as usize, &path) {
                    params.assets.fail(layer as usize, error);
                }
            }
            EditorTask::Generate { layer, recipe } => {
                if let Err(error) = params.assets.generate_source(layer as usize, recipe) {
                    params.assets.fail(layer as usize, error);
                }
            }
            EditorTask::Wake => {}
        })
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        editor::create(self.params.clone(), self.telemetry.clone(), executor)
    }

    fn filter_state(state: &mut PluginState) {
        // First, so a state the payload check below clears stays a complete no-op.
        mxm_preset::add_switches_off(state, crate::preset::TEMPO_SYNC_IDS);
        // Read in place: the check copies none of the embedded audio and reserves no buffer for it.
        let valid = state
            .fields
            .get("samples")
            .is_none_or(|serialized| asset::validate_serialized_payload(serialized).is_ok());
        if !valid {
            // `filter_state` cannot reject in nice-plug 0.3. A complete no-op is the only
            // transactional response available: clear parameter and field writes before
            // deserialize_object sees either, preserving the currently published sound.
            state.params.clear();
            state.fields.clear();
        }
    }

    fn activate(
        &mut self,
        _layout: &AudioIOLayout,
        config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        // A new activation starts with no tempo and nothing resolved: the first callback reports
        // the tempo, so neither the audio nor an editor frame before it shows the last session's
        // divisions (`plans/plan-tempo-sync-controls.md`).
        self.telemetry.tempo.publish(None);
        self.synced = [None; 4];
        self.sample_rate = if config.sample_rate.is_finite() {
            config.sample_rate.clamp(8_000.0, 384_000.0)
        } else {
            48_000.0
        };
        self.engine.set_sample_rate(self.sample_rate);
        self.engine.panic();
        self.bends = [0.0; NUM_CHANNELS];
        self.applied_bends = [0.0; NUM_CHANNELS];
        self.bend_range_applied = self.bend_range();
        true
    }

    fn reset(&mut self) {
        self.engine.panic();
        self.bends = [0.0; NUM_CHANNELS];
        self.applied_bends = [0.0; NUM_CHANNELS];
        // With every bender centred there is nothing for a range ramp to carry, so it lands.
        self.params
            .bend_range
            .smoothed
            .reset(self.params.bend_range.value());
        self.bend_range_applied = self.bend_range();
        self.telemetry.publish(0.0, 0, [None, None]);
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let samples_count = buffer.samples();
        // The tempo syncs, once per callback, and the tempo in force for the editor's readings.
        let tempo = context.transport().tempo;
        self.synced = [
            self.params.synced_lfo1_rate(tempo),
            self.params.synced_lfo2_rate(tempo),
            self.params.synced_a_grain_rate(tempo),
            self.params.synced_b_grain_rate(tempo),
        ];
        self.telemetry.tempo.publish(tempo);
        let bank = Arc::clone(&self.assets);
        let assets = bank.read();
        self.settle_arrivals(assets.layer_revisions);
        let samples = [assets.samples[0].as_ref(), assets.samples[1].as_ref()];
        // **The converter's readout is quoted against the sample, not the host.** Published from
        // here so the host's own generic panel is right with the editor shut, and it is the loaded
        // audio's rate: the grid is counted in source frames, so the converter's rate works out
        // host-rate-independent and a patch reads and sounds the same at 44.1 and at 96 kHz.
        if let Some(sample) = samples[0].or(samples[1]) {
            self.params.publish_source_rate(sample.sample_rate());
        }
        // **Presences once per callback, and that is already sample-accurate.**
        //
        // `SAMPLE_ACCURATE_AUTOMATION` is `true` below, and the wrapper honours it by consuming the
        // parameter events itself and **splitting the outer buffer at each one** before calling
        // this (the nice-plug fork's `src/wrapper/clap/wrapper.rs`, `handle_in_events_until`). So
        // every call begins on a parameter boundary with the new values already applied, and a
        // presence written at a nonzero offset starts its own callback rather than waiting for the
        // next one.
        //
        // **The event loop below is note events**, which is why reading topology against *it* was
        // the wrong mechanism: it was extra work keyed off the wrong thing. The engine still rebuilds
        // its compacted lists only when a presence actually moved. The depths this seeds are
        // overwritten per sample by `advance`, which is what carries smoothing and host modulation.
        self.routing = self.params.routes.routing_from(&self.routing);
        self.engine.set_topology(&self.routing);
        let output = buffer.as_slice();
        let mut next_event = context.next_event();
        let mut start = 0usize;
        let mut peak = 0.0f32;
        while start < samples_count {
            let mut end = (start + MAX_BLOCK_SIZE).min(samples_count);
            loop {
                match next_event {
                    Some(event) if event.timing() as usize <= start => {
                        self.handle_event(event, samples);
                        next_event = context.next_event();
                    }
                    Some(event) if (event.timing() as usize) < end => {
                        end = event.timing() as usize;
                        break;
                    }
                    _ => break,
                }
            }
            if self.engine.activity() == Activity::Inert {
                for channel in output.iter_mut().take(2) {
                    channel[start..end].fill(0.0);
                }
                // A range ramp still runs while nothing sounds, sample by sample as it would
                // under a note, so the next note starts where a sounding one would be. At rest one
                // call is every call: it only lands a range a restore snapped.
                if self.params.bend_range.smoothed.is_smoothing() {
                    for _ in start..end {
                        self.follow_bend_range();
                    }
                } else {
                    self.follow_bend_range();
                }
            } else {
                let (left_channels, right_channels) = output.split_at_mut(1);
                let left = &mut left_channels[0][start..end];
                let right = &mut right_channels[0][start..end];
                for (left, right) in left.iter_mut().zip(right) {
                    let frame = self.render_frame(samples);
                    *left = frame.left;
                    *right = frame.right;
                    peak = peak.max(frame.left.abs()).max(frame.right.abs());
                }
            }
            start = end;
        }
        // Normalised here rather than in the DSP: the sample's length is the plugin's to know, and
        // the editor draws against the whole sample rather than the played region.
        let playhead = [0usize, 1].map(|layer| {
            let last = samples[layer].map(|s| (s.len().saturating_sub(1)).max(1) as f32)?;
            self.engine.playhead(layer).map(|frames| frames / last)
        });
        self.telemetry
            .publish(peak, self.engine.active_voices(), playhead);
        // **Once a block, and only for the layer the editor is showing.** The grains are read
        // straight off the voices rather than logged as they spawn: a grain's position now is
        // arithmetic on its start, age and step, so there is nothing to accumulate and nothing a
        // stalled editor can make the audio thread carry.
        let shown = self.telemetry.shown_layer();
        let mut views = [mxm_creative_sampler_dsp::GrainView::default(); telemetry::GRAIN_SLOTS];
        // The display's fade is the grain's own window, so it follows the shape control too.
        let shape = if shown == 0 {
            self.params.a_grain_shape.value()
        } else {
            self.params.b_grain_shape.value()
        };
        let found = self.engine.grain_views(shown, shape, &mut views);
        let length = samples[shown]
            .map(|s| (s.len().saturating_sub(1)).max(1) as f32)
            .unwrap_or(1.0);
        let mut published = [(0.0f32, 0.0f32, 0.0f32); telemetry::GRAIN_SLOTS];
        for (out, view) in published.iter_mut().zip(&views[..found]) {
            *out = (view.start / length, view.current / length, view.weight);
        }
        self.telemetry.publish_grains(&published[..found]);
        match self.engine.activity() {
            // Nothing is sounding and nothing is owed: the host may suspend this plugin.
            Activity::Inert => ProcessStatus::Normal,
            // **A held note says "do not sleep me", even before it has made a sound.**
            //
            // `Normal` is CLAP's `CONTINUE_IF_NOT_QUIET`, and it was returned for a live voice as
            // well as an idle one — which tells the host it may suspend an instrument whose key is
            // still down. That is a lie whenever the voice has not reached audibility yet, and this
            // instrument has two ordinary ways to spend a while at exact zero: the envelope's
            // attack, 220 ms in the init patch, and Grain's scheduler, which under stochastic timing
            // draws its first onset somewhere inside the opening slot rather than at its edge.
            //
            // A host counting consecutive silent buffers then suspends the voice mid-attack, and a
            // suspended plugin is not called again, so it can never climb out. Two `mxm-player`
            // tests failed on exactly that and were read for a session as *the first note after a
            // state load arrives late*. The note was never late; it was slept.
            Activity::Live => ProcessStatus::KeepAlive,
            Activity::Tailing => {
                let seconds = self.params.release.value().clamp(0.0, 60.0) + 1.0;
                ProcessStatus::Tail((seconds * self.sample_rate).min(u32::MAX as f32) as u32)
            }
        }
    }
}

impl ClapPlugin for MxmCreativeSampler {
    const CLAP_ID: &'static str = CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = Some(
        "A sound-design sampler that plays samples repitched, time-stretched or as grain clouds",
    );
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Sampler,
        ClapFeature::Stereo,
    ];
}

nice_export_clap!(MxmCreativeSampler);

#[cfg(test)]
mod routing_timing_tests {
    use super::*;
    use nice_plug::params::internals::ParamPtr;
    use nice_plug::prelude::{PluginApi, PluginState};

    /// A host that applies what it is asked to, so a test can move a parameter the way one does.
    struct ApplyingHost;

    impl nice_plug::context::gui::GuiContextInner for ApplyingHost {
        // A test double has no host to ask for a restart (nice-plug 0.4).
        fn request_restart(&self) {}
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, value: f32) {
            unsafe {
                param._internal_set_normalized_value(value);
                param._internal_update_smoother(48_000.0, true);
            }
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            PluginState {
                version: String::new(),
                params: Default::default(),
                fields: Default::default(),
            }
        }
        fn set_state(&self, _: PluginState) {}
    }

    fn prime(params: &MxmCreativeSamplerParams) {
        for (_id, ptr, _group) in params.param_map() {
            unsafe { ptr._internal_update_smoother(48_000.0, true) };
        }
    }

    /// **A presence written between two blocks is live on the first sample of the next one.**
    ///
    /// This is the plugin's half of the timed-routing contract, and it is the half this repository
    /// can actually test. The other half is the wrapper's: `SAMPLE_ACCURATE_AUTOMATION` is `true`
    /// here, and nice-plug consumes parameter events itself and **splits the outer buffer at each
    /// one** before calling `process` (`handle_in_events_until`). So a presence automated at a
    /// nonzero sample offset begins its own `process` call with the value already applied, and what
    /// remains to prove on this side is that `process` reads the topology at the start of the call
    /// rather than carrying a stale one — which is exactly what this measures.
    ///
    /// **What it does not prove**: the wrapper's split itself. That needs a CLAP host harness able to
    /// inject a parameter event at a sample offset, which this repository does not have for any
    /// plugin; it is named here rather than implied to be covered.
    ///
    /// [`routing_is_block_partition_invariant`] covers the other half of the same contract — that
    /// where the block boundaries fall makes no difference to the audio — which is what the wrapper's
    /// split relies on being true.
    /// **Where the block boundaries fall makes no difference to the audio.**
    ///
    /// This is the property the wrapper's parameter-event split *depends* on: it turns one callback
    /// into several at arbitrary offsets, so if the routing path rendered differently across a
    /// boundary, sample-accurate automation would change the sound in a way that had nothing to do
    /// with the automation. Rendered as one block and as eight, with every route present so the
    /// whole grid is live, and compared on the bits.
    #[test]
    fn routing_is_block_partition_invariant() {
        let render = |chunks: usize| {
            let mut plugin = MxmCreativeSampler::default();
            let params = plugin.params_for_test();
            // **A crossfaded forward loop on a reader that reads it**, short enough that four
            // thousand samples cross its seam, so the seam is inside the partition claim too. The
            // init patch's Grain layer never reads the loop.
            unsafe {
                use nice_plug::params::InternalParamMut;
                params
                    .a_reader
                    ._internal_set_plain_value(crate::params::ReaderChoice::Stretch);
                params
                    .a_loop
                    ._internal_set_plain_value(crate::params::LoopChoice::Forward);
                params.a_loop_end._internal_set_plain_value(0.02);
                params.a_loop_crossfade._internal_set_plain_value(0.01);
            }
            prime(&params);
            for presence in params.routes.presences() {
                unsafe {
                    nice_plug::params::InternalParamMut::_internal_set_plain_value(presence, true)
                };
            }
            plugin.note_on_for_test(60, 0.8);
            let total = 4_096;
            let mut left = vec![0.0f32; total];
            let mut right = vec![0.0f32; total];
            let size = total / chunks;
            for chunk in 0..chunks {
                let (l, r) = (
                    &mut left[chunk * size..(chunk + 1) * size],
                    &mut right[chunk * size..(chunk + 1) * size],
                );
                plugin.render_block_for_test(l, r);
            }
            (left, right)
        };
        let (one_l, one_r) = render(1);
        let (many_l, many_r) = render(8);
        for (index, ((a, b), (c, d))) in one_l
            .iter()
            .zip(&one_r)
            .zip(many_l.iter().zip(&many_r))
            .enumerate()
        {
            assert_eq!(
                (a.to_bits(), b.to_bits()),
                (c.to_bits(), d.to_bits()),
                "sample {index} differs between one block and eight"
            );
        }
    }

    #[test]
    fn a_presence_written_between_blocks_is_live_on_the_next_blocks_first_sample() {
        use mxm_creative_sampler_dsp::routing::{source, target};

        let mut plugin = MxmCreativeSampler::default();
        let params = plugin.params_for_test();
        prime(&params);

        // Amplitude from the wheel: absent at Init, and unambiguous when it arrives.
        let host = ApplyingHost;
        let setter = nice_plug::prelude::ParamSetter::new(&host);
        {
            let routes = params.routes.target(target::AMPLITUDE);
            let wheel = &routes[source::WHEEL];
            // Full negative depth with the wheel up. The standard amplitude factor is
            // `1 + clamp(amount × wheel, ±1)`, so amount −1 and a wheel at 1 is a factor of exactly
            // zero: the route silences the voice the moment it becomes present, which makes the
            // transition unmistakable rather than a level change to argue about.
            wheel.amount.begin(&setter);
            wheel.amount.set(&setter, -1.0);
            wheel.amount.end(&setter);
        }
        plugin.engine_set_wheel_for_test(0, 1.0);

        // Let the envelope open first: the init patch has a slow attack, so a 64-sample block taken
        // at the note's start is near silence whatever the routing does.
        let mut warm_l = vec![0.0; 48_000];
        let mut warm_r = vec![0.0; 48_000];
        plugin.note_on_for_test(60, 1.0);
        plugin.render_block_for_test(&mut warm_l, &mut warm_r);

        let mut left = vec![0.0; 64];
        let mut right = vec![0.0; 64];
        plugin.render_block_for_test(&mut left, &mut right);
        let before = left.iter().fold(0.0f32, |p, v| p.max(v.abs()));
        assert!(
            before > 1.0e-4,
            "the note was silent before the route arrived"
        );

        // Now add the presence, exactly as a host automating it would.
        {
            let routes = params.routes.target(target::AMPLITUDE);
            mxm_modulation_params::add(&routes[source::WHEEL], &setter);
        }
        plugin.render_block_for_test(&mut left, &mut right);
        assert!(
            left[0].abs() < 1.0e-6 && right[0].abs() < 1.0e-6,
            "the presence was not live on the first sample of the next block: {} {}",
            left[0],
            right[0]
        );
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    use nice_plug::params::InternalParamMut;

    fn prime(params: &MxmCreativeSamplerParams) {
        for (_id, ptr, _group) in params.param_map() {
            unsafe { ptr._internal_update_smoother(48_000.0, true) };
        }
    }

    /// One event through the plugin's own event path, against the audio it is playing.
    fn send(plugin: &mut MxmCreativeSampler, event: NoteEvent<()>) {
        let bank = Arc::clone(&plugin.assets);
        let assets = bank.read();
        let samples = [assets.samples[0].as_ref(), assets.samples[1].as_ref()];
        plugin.handle_event(event, samples);
    }

    fn note_on(key: u8) -> NoteEvent<()> {
        NoteEvent::NoteOn {
            timing: 0,
            voice_id: VoiceID::Wildcard,
            channel: Channel::Number(0),
            key: Key::Number(key),
            velocity: 1.0,
        }
    }

    /// The bender at `position`, `-1..=1`.
    fn bend(position: f32) -> NoteEvent<()> {
        NoteEvent::MidiPitchBend {
            timing: 0,
            channel: 0,
            value: position * 0.5 + 0.5,
        }
    }

    fn render(plugin: &mut MxmCreativeSampler, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut out = (vec![0.0; frames], vec![0.0; frames]);
        plugin.render_block_for_test(&mut out.0, &mut out.1);
        out
    }

    /// A Repitch note at key 60, the bender full up at the default two semitones, then Bend range
    /// moved to twelve the way a host moves a parameter.
    fn a_held_bend_whose_range_was_just_edited() -> MxmCreativeSampler {
        let mut plugin = MxmCreativeSampler::default();
        let params = plugin.params_for_test();
        unsafe {
            params
                .a_reader
                ._internal_set_plain_value(crate::params::ReaderChoice::Repitch);
        }
        prime(&params);
        send(&mut plugin, note_on(60));
        render(&mut plugin, 480);
        send(&mut plugin, bend(1.0));
        render(&mut plugin, 480);
        assert_eq!(
            plugin.engine.sounding_pitch(),
            Some(62.0),
            "a full bend at the default range"
        );
        unsafe {
            let ptr = params.bend_range.as_ptr();
            let _ = ptr._internal_set_normalized_value(0.5);
            ptr._internal_update_smoother(48_000.0, false);
        }
        assert_eq!(params.bend_range.value(), 12.0);
        plugin
    }

    /// **A Bend range edited under a held bend ramps the pitch; it never steps it, and where the
    /// blocks fall changes nothing** (audit D11; mxm-kit's `docs/code-review-notes.md` §2, *smooth
    /// the mutable scale*). Voices carry the bend as an accumulated tuning, so the range reaches
    /// them as deltas; read once per callback, a range edit arrived as one ten-semitone delta on
    /// the callback's first sample.
    ///
    /// The pitch is read off the voice after every sample, and the render is repeated in blocks of
    /// irregular sizes, which must agree to the bit — a smoother advanced per block, or a delta
    /// applied at block starts, fails that even where every block ramps.
    #[test]
    fn a_range_edit_under_a_held_bend_ramps_and_the_blocks_do_not_matter() {
        const AFTER: usize = 4_800;
        let run = |blocks: &[usize]| {
            let mut plugin = a_held_bend_whose_range_was_just_edited();
            let mut audio = (Vec::new(), Vec::new());
            let mut pitches = Vec::new();
            let mut done = 0;
            for &size in blocks.iter().cycle() {
                if done == AFTER {
                    break;
                }
                let size = size.min(AFTER - done);
                let (left, right) = render(&mut plugin, size);
                audio.0.extend(left);
                audio.1.extend(right);
                done += size;
                pitches.push((
                    done,
                    plugin.engine.sounding_pitch().expect("still sounding"),
                ));
            }
            (audio, pitches)
        };

        let (reference, trace) = run(&[1]);
        let per_sample = (72.0 - 62.0) / (0.020 * 48_000.0);
        let mut previous = 62.0f32;
        let mut largest_move = 0.0f32;
        let mut settled_at = None;
        for &(sample, pitch) in &trace {
            assert!(
                pitch >= previous && pitch <= 72.0 + 1.0e-4,
                "sample {sample}: {previous} -> {pitch} is not a monotonic ramp toward 72"
            );
            largest_move = largest_move.max(pitch - previous);
            if settled_at.is_none() && (pitch - 72.0).abs() <= 1.0e-4 {
                settled_at = Some(sample);
            }
            previous = pitch;
        }
        assert!(
            largest_move <= per_sample * 1.05,
            "a {largest_move} semitone move in one sample is a step, not a {per_sample} ramp"
        );
        let settled_at = settled_at.expect("the pitch never reached the new range");
        assert!(
            settled_at >= 900,
            "the new range arrived after {settled_at} samples, faster than the declared 20 ms"
        );

        for blocks in [&[4_800][..], &[64], &[7, 64, 333, 1, 128]] {
            let (audio, pitches) = run(blocks);
            for (index, (a, b)) in audio
                .0
                .iter()
                .zip(&audio.1)
                .zip(reference.0.iter().zip(&reference.1))
                .enumerate()
            {
                assert_eq!(
                    (a.0.to_bits(), a.1.to_bits()),
                    (b.0.to_bits(), b.1.to_bits()),
                    "blocks {blocks:?}: sample {index} differs from one-sample blocks"
                );
            }
            for (end, pitch) in pitches {
                assert_eq!(
                    pitch.to_bits(),
                    trace[end - 1].1.to_bits(),
                    "blocks {blocks:?}: the pitch after sample {end} differs"
                );
            }
        }
    }

    /// **A bend that moves during the ramp is scaled by the range the voices already hold**, so the
    /// event lands where the bender says and the ramp carries on from there. Scaling it by the
    /// edited target instead would step to the whole new range at the event.
    #[test]
    fn a_bend_moved_during_a_range_ramp_lands_on_the_range_already_applied() {
        let mut plugin = a_held_bend_whose_range_was_just_edited();
        for _ in 0..480 {
            render(&mut plugin, 1);
        }
        let before = plugin.engine.sounding_pitch().expect("sounding") - 60.0;
        assert!(
            before > 2.5 && before < 11.5,
            "the ramp is not under way at its midpoint: {before}"
        );
        send(&mut plugin, bend(0.5));
        let after = plugin.engine.sounding_pitch().expect("sounding") - 60.0;
        assert!(
            (after - before * 0.5).abs() < 1.0e-4,
            "half the bender at a range of {before} semitones sounds {after}"
        );
        let mut previous = after;
        for _ in 0..4_800 {
            render(&mut plugin, 1);
            let now = plugin.engine.sounding_pitch().expect("sounding") - 60.0;
            assert!(now >= previous, "{previous} -> {now} after the event");
            previous = now;
        }
        assert!(
            (previous - 6.0).abs() < 1.0e-4,
            "half the bender settles at {previous} rather than half of twelve"
        );
    }

    fn midi_cc(cc: u8, value: f32) -> NoteEvent<()> {
        NoteEvent::MidiCC {
            timing: 0,
            channel: 0,
            cc,
            value,
        }
    }

    /// **A CC panic ends the notes and leaves the controllers where the player left them** (audit
    /// D12; mxm-kit's `docs/code-review-notes.md` §2). The bender, the wheel and channel pressure
    /// are positions a controller sends only when they move, and CC 120 or 123 moves none of them.
    /// The engine's panic returned all three sources to neutral while the plugin kept the bend it
    /// tunes new notes by, so a note after the panic was pitched by the bend while every route from
    /// Bend read zero, and the wheel and pressure were lost until they next moved.
    ///
    /// The oracle is a twin that panicked first and received the same controller positions after:
    /// the two must render the same note to the bit, for both control changes. A third instance
    /// that panicked with every controller at rest must render differently, or the routes carry
    /// nothing and the comparison is empty.
    #[test]
    fn a_cc_panic_keeps_every_controller_where_the_player_left_it() {
        const FRAMES: usize = 4_800;
        let controllers = [
            bend(0.6),
            midi_cc(control_change::MODULATION_MSB, 0.8),
            NoteEvent::MidiChannelPressure {
                timing: 0,
                channel: 0,
                pressure: 0.7,
            },
        ];
        for panic in [control_change::ALL_SOUND_OFF, control_change::ALL_NOTES_OFF] {
            let run = |before: &[NoteEvent<()>], after: &[NoteEvent<()>]| {
                let mut plugin = MxmCreativeSampler::default();
                let params = plugin.params_for_test();
                // Three routes into the cutoff at amounts that cancel in no combination, so losing
                // any one source moves the filter.
                unsafe {
                    let cutoff = &params.routes.cutoff;
                    cutoff.bend_on._internal_set_plain_value(true);
                    cutoff.bend._internal_set_plain_value(0.5);
                    cutoff.wheel_on._internal_set_plain_value(true);
                    cutoff.wheel._internal_set_plain_value(-0.3);
                    cutoff.press_on._internal_set_plain_value(true);
                    cutoff.press._internal_set_plain_value(0.9);
                }
                prime(&params);
                for event in before {
                    send(&mut plugin, *event);
                }
                send(&mut plugin, midi_cc(panic, 0.0));
                for event in after {
                    send(&mut plugin, *event);
                }
                send(&mut plugin, note_on(60));
                let audio = render(&mut plugin, FRAMES);
                (audio, plugin.engine.sounding_pitch())
            };
            let (kept, kept_pitch) = run(&controllers, &[]);
            let (sent_after, sent_after_pitch) = run(&[], &controllers);
            let (at_rest, _) = run(&[], &[]);
            assert_eq!(
                kept_pitch, sent_after_pitch,
                "CC {panic}: the note is pitched differently"
            );
            assert!(
                at_rest.0.iter().zip(&sent_after.0).any(|(a, b)| a != b),
                "CC {panic}: the controllers change nothing, so this compares nothing"
            );
            for (index, ((a, b), (c, d))) in kept
                .0
                .iter()
                .zip(&kept.1)
                .zip(sent_after.0.iter().zip(&sent_after.1))
                .enumerate()
            {
                assert_eq!(
                    (a.to_bits(), b.to_bits()),
                    (c.to_bits(), d.to_bits()),
                    "CC {panic}: sample {index} differs from the bender, wheel and pressure sent \
                     after the panic"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_and_id_have_one_source() {
        assert_eq!(NAME, "mxm-creative-sampler");
        assert_eq!(CLAP_ID, format!("dk.mxm.{NAME}"));
    }

    #[test]
    fn the_developer_channel_is_off_unless_the_environment_asked_for_it() {
        let mut plugin = MxmCreativeSampler {
            dev_cc: false,
            ..Default::default()
        };
        let cc = |cc: u8, value: f32| -> NoteEvent<()> {
            NoteEvent::MidiCC {
                timing: 0,
                channel: 0,
                cc,
                value,
            }
        };
        for (number, value) in [
            (DEV_VIEW_CC, 2.0 / 127.0),
            (DEV_DISCLOSURE_CC, 1.0),
            (DEV_BROWSER_CC, 1.0),
            (DEV_THEME_CC, 1.0 / 127.0),
        ] {
            plugin.handle_event(cc(number, value), [None, None]);
        }
        assert_eq!(plugin.telemetry.take_view_request(), None);
        assert_eq!(plugin.telemetry.take_browser_request(), None);
        assert_eq!(plugin.telemetry.take_theme_request(), None);

        plugin.dev_cc = true;
        for (number, value) in [
            (DEV_VIEW_CC, 1.0),
            (DEV_DISCLOSURE_CC, 1.0),
            (DEV_BROWSER_CC, 1.0),
            (DEV_THEME_CC, 1.0 / 127.0),
        ] {
            plugin.handle_event(cc(number, value), [None, None]);
        }
        assert_eq!(
            plugin.telemetry.take_view_request(),
            Some(mxm_ui::paging::PARAMETERS),
            "127 is the Parameters surface"
        );
        assert_eq!(plugin.telemetry.take_browser_request(), Some(true));
        assert_eq!(
            plugin.telemetry.take_theme_request(),
            Some(1),
            "1 is dark, as mxm_ui::theme::from_index reads it"
        );
        assert_eq!(
            plugin.telemetry.take_view_request(),
            None,
            "a request is taken once"
        );
    }

    #[test]
    fn malformed_asset_state_becomes_a_complete_no_op() {
        let mut state = PluginState {
            version: "0.1.0".to_owned(),
            params: [("blend".to_owned(), nice_plug::plugin::ParamValue::F32(0.75))]
                .into_iter()
                .collect(),
            fields: [("samples".to_owned(), "{ definitely not JSON".to_owned())]
                .into_iter()
                .collect(),
        };
        MxmCreativeSampler::filter_state(&mut state);
        assert!(state.params.is_empty());
        assert!(state.fields.is_empty());
    }

    #[test]
    fn the_bundle_is_named_after_this_plugin() {
        mxm_plugin_test::bundle::is_named(env!("CARGO_MANIFEST_DIR"), env!("CARGO_PKG_NAME"), NAME);
    }
}

/// **A synced value reaches the patch** (`plans/plan-tempo-sync-controls.md`): what a sync resolved
/// for this callback is what the DSP is given, and with none the free value is.
#[cfg(test)]
mod tempo_sync_path {
    use super::*;

    #[test]
    fn the_synced_lfo_and_grain_rates_are_the_patchs() {
        let mut plugin = MxmCreativeSampler::default();
        let free = plugin.next_params();
        plugin.synced = [
            Some(free.lfo_rate_hz + 1.0),
            Some(free.lfo2_rate_hz + 1.0),
            Some(free.layers[0].grain_density_hz + 1.0),
            Some(free.layers[1].grain_density_hz + 1.0),
        ];
        let synced = plugin.next_params();
        assert_eq!(synced.lfo_rate_hz, free.lfo_rate_hz + 1.0);
        assert_eq!(synced.lfo2_rate_hz, free.lfo2_rate_hz + 1.0);
        assert_eq!(
            synced.layers[0].grain_density_hz,
            free.layers[0].grain_density_hz + 1.0
        );
        assert_eq!(
            synced.layers[1].grain_density_hz,
            free.layers[1].grain_density_hz + 1.0
        );
    }
}

/// **Activation forgets the last session's tempo and resolved syncs**: the first callback reports the
/// tempo, so nothing — the audio, or an editor frame before it — starts from the previous session's
/// divisions.
#[cfg(test)]
mod activation_forgets_the_tempo {
    use super::*;

    #[test]
    fn activation_forgets_the_last_tempo_and_resolved_syncs() {
        use nice_plug::prelude::Plugin as _;
        let mut plugin = MxmCreativeSampler::default();
        plugin.telemetry.tempo.publish(Some(120.0));
        plugin.synced = [Some(1.0); 4];
        let layout = MxmCreativeSampler::AUDIO_IO_LAYOUTS[0];
        let config = BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: None,
            max_buffer_size: 512,
            process_mode: ProcessMode::Realtime,
        };
        let _ = plugin.activate(&layout, &config, &mut NoInit);
        assert_eq!(plugin.telemetry.tempo.get(), None);
        assert_eq!(plugin.synced, [None; 4]);
    }

    /// An activation context that asks nothing of a host.
    struct NoInit;

    impl ActivateContext<MxmCreativeSampler> for NoInit {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _task: <MxmCreativeSampler as Plugin>::BackgroundTask) {}
        fn set_latency_samples(&self, _samples: u32) {}
        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }
}

/// What a player reads — on hover in the editor, and in a host's plugin browser — speaks to the
/// player about the sound, never about the machine or the code (`mxm_plugin_test::hover_text`).
#[cfg(test)]
mod speaks_to_the_player {
    #[test]
    fn hover_text() {
        mxm_plugin_test::hover_text::speaks_to_the_player(env!("CARGO_MANIFEST_DIR"));
    }

    #[test]
    fn host_description() {
        mxm_plugin_test::hover_text::host_description_speaks_to_the_player(env!(
            "CARGO_MANIFEST_DIR"
        ));
    }
}
