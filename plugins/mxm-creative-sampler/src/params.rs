//! Provisional host parameters for the pre-listening `mxm-creative-sampler` slice.
//!
//! **Every parameter is built from the state crate's table** (`mxm_creative_sampler_state::params`):
//! its name, travel, default, unit, smoothing and its text both ways. What is left here is what
//! nice-plug needs written out — the `#[id]` attributes, the choice enums — and how the plugin reads
//! its parameters on the audio thread. `every_parameter_is_the_tables` holds the two together, the
//! ids in the order a host lists them included.

use crate::asset::AssetField;
use mxm_creative_sampler_dsp::{FilterMode, LoopMode, Reader, VoiceMode};
use mxm_creative_sampler_state::params::{self as table, Kind, LAYERS, Smoothing, TEMPO_SYNCS, id};
use mxm_creative_sampler_state::range::Range;
use mxm_creative_sampler_state::sound;
use mxm_preset::PresetIdentity;
use nice_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

pub use mxm_creative_sampler_state::params::{GRAIN_SYNC, LFO_SYNC};
/// The source rate the converter's readout is quoted against: see the state crate's
/// [`DEFAULT_SOURCE_RATE`]. It is an atomic here, [`MxmCreativeSamplerParams::source_rate`], rather
/// than a read of [`AssetField`], because the reading is a closure built in `Default::default()`,
/// before the field it would need to borrow exists.
pub use mxm_creative_sampler_state::text::DEFAULT_SOURCE_RATE;

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderChoice {
    #[id = "repitch"]
    #[name = "Repitch"]
    Repitch,
    #[id = "stretch"]
    #[name = "Stretch"]
    Stretch,
    #[id = "grain"]
    #[name = "Grain"]
    Grain,
}

impl ReaderChoice {
    pub const fn dsp(self) -> Reader {
        match self {
            Self::Repitch => Reader::Repitch,
            Self::Stretch => Reader::Stretch,
            Self::Grain => Reader::Grain,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopChoice {
    #[id = "off"]
    #[name = "Off"]
    Off,
    #[id = "forward"]
    #[name = "Forward"]
    Forward,
    #[id = "alternate"]
    #[name = "Alternate"]
    Alternate,
}

impl LoopChoice {
    pub const fn dsp(self) -> LoopMode {
        match self {
            Self::Off => LoopMode::Off,
            Self::Forward => LoopMode::Forward,
            Self::Alternate => LoopMode::Alternate,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceChoice {
    #[id = "poly"]
    #[name = "Poly"]
    Poly,
    #[id = "mono"]
    #[name = "Mono"]
    Mono,
    #[id = "legato"]
    #[name = "Legato"]
    Legato,
}

impl VoiceChoice {
    pub const fn dsp(self) -> VoiceMode {
        match self {
            Self::Poly => VoiceMode::Poly,
            Self::Mono => VoiceMode::Mono,
            Self::Legato => VoiceMode::Legato,
        }
    }
}

/// What an LFO draws. See [`mxm_creative_sampler_dsp::LfoShape`].
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeChoice {
    #[id = "sine"]
    #[name = "Sine"]
    Sine,
    #[id = "triangle"]
    #[name = "Triangle"]
    Triangle,
    #[id = "rampup"]
    #[name = "Ramp up"]
    RampUp,
    #[id = "rampdown"]
    #[name = "Ramp down"]
    RampDown,
    #[id = "square"]
    #[name = "Square"]
    Square,
    #[id = "samplehold"]
    #[name = "Sample & hold"]
    SampleHold,
}

impl ShapeChoice {
    pub fn dsp(self) -> mxm_creative_sampler_dsp::LfoShape {
        use mxm_creative_sampler_dsp::LfoShape;
        match self {
            Self::Sine => LfoShape::Sine,
            Self::Triangle => LfoShape::Triangle,
            Self::RampUp => LfoShape::RampUp,
            Self::RampDown => LfoShape::RampDown,
            Self::Square => LfoShape::Square,
            Self::SampleHold => LfoShape::SampleHold,
        }
    }
}

/// Which converter the voice's DAC is. See [`mxm_creative_sampler_dsp::ConverterType`] and the
/// state crate's `sound::CONVERTER_TYPES`, which says why it is named for the machines.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConverterChoice {
    #[id = "linear"]
    #[name = "Linear"]
    Linear,
    #[id = "companding"]
    #[name = "Companding"]
    Companding,
}

impl ConverterChoice {
    pub fn dsp(self) -> mxm_creative_sampler_dsp::ConverterType {
        match self {
            Self::Linear => mxm_creative_sampler_dsp::ConverterType::Linear,
            Self::Companding => mxm_creative_sampler_dsp::ConverterType::Companding,
        }
    }
}

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterChoice {
    #[id = "off"]
    #[name = "Off"]
    Off,
    #[id = "lowpass"]
    #[name = "Low pass"]
    LowPass,
    #[id = "bandpass"]
    #[name = "Band pass"]
    BandPass,
    #[id = "highpass"]
    #[name = "High pass"]
    HighPass,
}

impl FilterChoice {
    pub const fn dsp(self) -> FilterMode {
        match self {
            Self::Off => FilterMode::Off,
            Self::LowPass => FilterMode::LowPass,
            Self::BandPass => FilterMode::BandPass,
            Self::HighPass => FilterMode::HighPass,
        }
    }
}

#[derive(Params)]
pub struct MxmCreativeSamplerParams {
    #[id = "areader"]
    pub a_reader: EnumParam<ReaderChoice>,
    #[id = "aroot"]
    pub a_root: FloatParam,
    #[id = "atranspose"]
    pub a_transpose: FloatParam,
    #[id = "astart"]
    pub a_start: FloatParam,
    #[id = "aend"]
    pub a_end: FloatParam,
    #[id = "aloopstart"]
    pub a_loop_start: FloatParam,
    #[id = "aloopend"]
    pub a_loop_end: FloatParam,
    #[id = "aloopcrossfade"]
    pub a_loop_crossfade: FloatParam,
    #[id = "aloop"]
    pub a_loop: EnumParam<LoopChoice>,
    #[id = "areverse"]
    pub a_reverse: BoolParam,
    #[id = "alevel"]
    pub a_level: FloatParam,
    #[id = "apan"]
    pub a_pan: FloatParam,
    #[id = "ascanspeed"]
    pub a_scan_speed: FloatParam,
    #[id = "agrainsize"]
    pub a_grain_size: FloatParam,
    #[id = "agrainrate"]
    pub a_grain_rate: FloatParam,
    /// A's grain rate's tempo sync.
    #[id = "agrainsync"]
    pub a_grain_sync: BoolParam,
    #[id = "apitchvar"]
    pub a_pitch_var: FloatParam,
    #[id = "aposvar"]
    pub a_position_var: FloatParam,
    #[id = "atiming"]
    pub a_timing: FloatParam,
    #[id = "awidth"]
    pub a_width: FloatParam,
    #[id = "areversechance"]
    pub a_reverse_chance: FloatParam,
    #[id = "agrainshape"]
    pub a_grain_shape: FloatParam,

    #[id = "breader"]
    pub b_reader: EnumParam<ReaderChoice>,
    #[id = "broot"]
    pub b_root: FloatParam,
    #[id = "btranspose"]
    pub b_transpose: FloatParam,
    #[id = "bstart"]
    pub b_start: FloatParam,
    #[id = "bend"]
    pub b_end: FloatParam,
    #[id = "bloopstart"]
    pub b_loop_start: FloatParam,
    #[id = "bloopend"]
    pub b_loop_end: FloatParam,
    #[id = "bloopcrossfade"]
    pub b_loop_crossfade: FloatParam,
    #[id = "bloop"]
    pub b_loop: EnumParam<LoopChoice>,
    #[id = "breverse"]
    pub b_reverse: BoolParam,
    #[id = "blevel"]
    pub b_level: FloatParam,
    #[id = "bpan"]
    pub b_pan: FloatParam,
    #[id = "bscanspeed"]
    pub b_scan_speed: FloatParam,
    #[id = "bgrainsize"]
    pub b_grain_size: FloatParam,
    #[id = "bgrainrate"]
    pub b_grain_rate: FloatParam,
    /// B's grain rate's tempo sync.
    #[id = "bgrainsync"]
    pub b_grain_sync: BoolParam,
    #[id = "bpitchvar"]
    pub b_pitch_var: FloatParam,
    #[id = "bposvar"]
    pub b_position_var: FloatParam,
    #[id = "btiming"]
    pub b_timing: FloatParam,
    #[id = "bwidth"]
    pub b_width: FloatParam,
    #[id = "breversechance"]
    pub b_reverse_chance: FloatParam,
    #[id = "bgrainshape"]
    pub b_grain_shape: FloatParam,

    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "converter"]
    pub converter: FloatParam,
    #[id = "reconstruction"]
    pub reconstruction: FloatParam,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "convtype"]
    pub converter_type: EnumParam<ConverterChoice>,
    #[id = "jitter"]
    pub jitter: FloatParam,
    #[id = "cutoff"]
    pub cutoff: FloatParam,
    #[id = "resonance"]
    pub resonance: FloatParam,
    #[id = "filtermode"]
    pub filter_mode: EnumParam<FilterChoice>,
    #[id = "drive"]
    pub drive: FloatParam,
    // `envamount` retired here: it is the (Cutoff <- Envelope) route's amount now, and the
    // control map's `filter.env_amount` role follows it there.
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    // `velsense` retired here: it is the (Amplitude <- Velocity) route's amount. The target was a
    // `product` when it retired and is the collection's standard factor now; under either, the route
    // renders what the knob did at the same number (`plans/plan-modulation-standard.md`, rev. 8).
    #[id = "lforate"]
    pub lfo_rate: FloatParam,
    /// LFO 1's tempo sync: its rate's position picks a division of the host's tempo.
    #[id = "lfo1sync"]
    pub lfo1_sync: BoolParam,
    #[id = "lfo1shape"]
    pub lfo_shape: EnumParam<ShapeChoice>,
    #[id = "lfo2rate"]
    pub lfo2_rate: FloatParam,
    /// LFO 2's tempo sync.
    #[id = "lfo2sync"]
    pub lfo2_sync: BoolParam,
    #[id = "lfo2shape"]
    pub lfo2_shape: EnumParam<ShapeChoice>,
    // `traveldepth` retired here: it is two routes, LFO 1 into each layer's scan speed, and the
    // opposed direction that made the layers drift apart lives in the route's scale.
    #[id = "master"]
    pub master: FloatParam,
    #[id = "voicemode"]
    pub voice_mode: EnumParam<VoiceChoice>,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "bendrange"]
    pub bend_range: FloatParam,

    /// One presence and one amount per *(target, source)* pair.
    ///
    /// **Declared last**, so every parameter that existed before the conversion keeps its position
    /// in the preset file's declaration order and in a host's parameter list.
    #[nested(group = "Modulation")]
    pub routes: crate::routes::Routes,

    #[persist = "samples"]
    pub assets: AssetField,
    #[persist = "preset"]
    pub preset: RwLock<PresetIdentity>,

    /// See [`DEFAULT_SOURCE_RATE`]. Not persisted: it is derived from the audio, which is.
    pub source_rate: Arc<AtomicU32>,
}

impl MxmCreativeSamplerParams {
    /// Tells the converter's readout which sample it is quoting against.
    ///
    /// Called from `process` and from the editor, so the text is right whether or not the editor is
    /// open. The first loaded layer wins: the converter is one stage on the summed voice, so there
    /// is one rate to quote and no selection to consult.
    pub fn publish_source_rate(&self, rate: f32) {
        if rate.is_finite() && rate > 0.0 {
            self.source_rate.store(rate as u32, Ordering::Relaxed);
        }
    }
}

/// The table's entry for `id`, which the table holds: a parameter field is built from its own id.
fn spec(id: &str) -> &'static table::Param {
    table::get(id).unwrap_or_else(|| panic!("`{id}` is not in the state crate's table"))
}

/// A travel as nice-plug's range.
pub(crate) fn float_range(range: Range) -> FloatRange {
    match range {
        Range::Linear { min, max } => FloatRange::Linear { min, max },
        Range::Skewed { min, max, factor } => FloatRange::Skewed { min, max, factor },
        Range::SymmetricalSkewed {
            min,
            max,
            factor,
            center,
        } => FloatRange::SymmetricalSkewed {
            min,
            max,
            factor,
            center,
        },
    }
}

/// A continuous parameter, **everything about it from the table**: name, travel, step, default,
/// unit, smoothing, and its reading and parser.
///
/// **A reading and its parser are one thing, so both come from the same entry**: a `FloatParam`
/// without `string_to_value` falls back to "strip `with_unit`, then `parse`", which a reading whose
/// unit is part of its own text does not survive, and `param-conversions` fails the whole plugin on
/// that. Both read the loaded sample's rate, which only Sample rate's reading uses.
fn float(id: &str, source_rate: &Arc<AtomicU32>) -> FloatParam {
    let param = spec(id);
    let float = param
        .float()
        .unwrap_or_else(|| panic!("`{id}` is not continuous"));
    let shown = Arc::clone(source_rate);
    let parsed = Arc::clone(source_rate);
    let mut built = FloatParam::new(param.name.as_str(), float.default, float_range(float.range))
        .with_unit(float.unit)
        .with_value_to_string(Arc::new(move |plain| {
            float.show(plain, shown.load(Ordering::Relaxed))
        }))
        .with_string_to_value(Arc::new(move |text| {
            float.read(text, parsed.load(Ordering::Relaxed))
        }));
    if let Some(step) = float.step {
        built = built.with_step_size(step);
    }
    match float.smoothing {
        Smoothing::None => built,
        Smoothing::Linear(ms) => built.with_smoother(SmoothingStyle::Linear(ms)),
        Smoothing::Logarithmic(ms) => built.with_smoother(SmoothingStyle::Logarithmic(ms)),
    }
}

/// A switch, its name, default and text both ways from the table.
pub(crate) fn switch(id: &str) -> BoolParam {
    let param = spec(id);
    let Kind::Switch(switch) = &param.kind else {
        panic!("`{id}` is not a switch");
    };
    let reading = switch.reading;
    BoolParam::new(param.name.as_str(), switch.default)
        .with_value_to_string(Arc::new(move |on| reading.show(on)))
        .with_string_to_value(Arc::new(move |text| reading.read(text)))
}

/// A choice, its name and default from the table. Its variants' ids and names are the enum's own,
/// which nice-plug's derive needs written out; `every_choice_is_the_tables` holds them.
fn choice<T: Enum + PartialEq + 'static>(id: &str) -> EnumParam<T> {
    let param = spec(id);
    let Kind::Choice(choice) = &param.kind else {
        panic!("`{id}` is not a choice");
    };
    EnumParam::new(param.name.as_str(), T::from_index(choice.default))
}

impl Default for MxmCreativeSamplerParams {
    fn default() -> Self {
        // **One declaration owns the whole Init sound**: every default comes from the state crate's
        // `init::PATCH`, through its table, as the generated source and the routes do.
        let source_rate = Arc::new(AtomicU32::new(DEFAULT_SOURCE_RATE));
        let float = |id: &str| float(id, &source_rate);
        let [a, b] = LAYERS;
        Self {
            a_reader: choice(a.reader),
            a_root: float(a.root),
            a_transpose: float(a.transpose),
            a_start: float(a.start),
            a_end: float(a.end),
            a_loop_start: float(a.loop_start),
            a_loop_end: float(a.loop_end),
            a_loop_crossfade: float(a.loop_crossfade),
            a_loop: choice(a.loop_mode),
            a_reverse: switch(a.reverse),
            a_level: float(a.level),
            a_pan: float(a.pan),
            a_scan_speed: float(a.scan_speed),
            a_grain_size: float(a.grain_size),
            a_grain_rate: float(a.grain_rate),
            a_grain_sync: switch(a.grain_sync),
            a_pitch_var: float(a.pitch_variation),
            a_position_var: float(a.position_variation),
            a_timing: float(a.onset_timing),
            a_width: float(a.stereo_width),
            a_reverse_chance: float(a.reverse_chance),
            a_grain_shape: float(a.grain_shape),
            b_reader: choice(b.reader),
            b_root: float(b.root),
            b_transpose: float(b.transpose),
            b_start: float(b.start),
            b_end: float(b.end),
            b_loop_start: float(b.loop_start),
            b_loop_end: float(b.loop_end),
            b_loop_crossfade: float(b.loop_crossfade),
            b_loop: choice(b.loop_mode),
            b_reverse: switch(b.reverse),
            b_level: float(b.level),
            b_pan: float(b.pan),
            b_scan_speed: float(b.scan_speed),
            b_grain_size: float(b.grain_size),
            b_grain_rate: float(b.grain_rate),
            b_grain_sync: switch(b.grain_sync),
            b_pitch_var: float(b.pitch_variation),
            b_position_var: float(b.position_variation),
            b_timing: float(b.onset_timing),
            b_width: float(b.stereo_width),
            b_reverse_chance: float(b.reverse_chance),
            b_grain_shape: float(b.grain_shape),
            rate: float(id::SAMPLE_RATE),
            converter: float(id::BIT_DEPTH),
            reconstruction: float(id::BIT_SMOOTHING),
            input: float(id::CONVERTER_DRIVE),
            converter_type: choice(id::CONVERTER_TYPE),
            jitter: float(id::JITTER),
            cutoff: float(id::CUTOFF),
            resonance: float(id::RESONANCE),
            filter_mode: choice(id::FILTER_MODE),
            drive: float(id::DRIVE),
            attack: float(id::ATTACK),
            decay: float(id::DECAY),
            sustain: float(id::SUSTAIN),
            release: float(id::RELEASE),
            lfo_rate: float(id::LFO1_RATE),
            lfo1_sync: switch(id::LFO1_SYNC),
            lfo_shape: choice(id::LFO1_SHAPE),
            lfo2_rate: float(id::LFO2_RATE),
            lfo2_sync: switch(id::LFO2_SYNC),
            lfo2_shape: choice(id::LFO2_SHAPE),
            master: float(id::MASTER),
            voice_mode: choice(id::VOICE_MODE),
            glide: float(id::GLIDE),
            bend_range: float(id::BEND_RANGE),
            routes: crate::routes::Routes::default(),
            assets: AssetField::new(),
            preset: RwLock::new(PresetIdentity::none()),
            source_rate,
        }
    }
}

impl MxmCreativeSamplerParams {
    /// Settles a layer's source-owned geometry — region, loop points, Root — and its crossfade on
    /// their targets, for the first callback that hears a new source in that layer.
    ///
    /// **A new source's geometry is final when it is heard** (`plans/plan-sampler-wav-loop-import.md`
    /// §1.2). An arrival writes these as gestures before its commit, so their targets are the
    /// arrival; but each is smoothed, and without this the first 20 ms of new audio played through a
    /// ramp from the *previous* source's crop, loop and crossfade. A smoother reset: no allocation,
    /// no lock, safe on the audio thread.
    pub fn settle_source(&self, layer: usize) {
        let settled: [&FloatParam; 6] = if layer == 0 {
            [
                &self.a_start,
                &self.a_end,
                &self.a_loop_start,
                &self.a_loop_end,
                &self.a_loop_crossfade,
                &self.a_root,
            ]
        } else {
            [
                &self.b_start,
                &self.b_end,
                &self.b_loop_start,
                &self.b_loop_end,
                &self.b_loop_crossfade,
                &self.b_root,
            ]
        };
        for param in settled {
            param.smoothed.reset(param.value());
        }
    }

    /// One layer's values for this sample, each smoother advanced once.
    pub fn layer(&self, index: usize) -> sound::Layer {
        if index == 0 {
            sound::Layer {
                reader: self.a_reader.value().dsp(),
                root: self.a_root.smoothed.next(),
                transpose: self.a_transpose.smoothed.next(),
                start: self.a_start.smoothed.next(),
                end: self.a_end.smoothed.next(),
                loop_start: self.a_loop_start.smoothed.next(),
                loop_end: self.a_loop_end.smoothed.next(),
                loop_crossfade_s: self.a_loop_crossfade.smoothed.next(),
                loop_mode: self.a_loop.value().dsp(),
                reverse: self.a_reverse.value(),
                level: self.a_level.smoothed.next(),
                pan: self.a_pan.smoothed.next(),
                scan_speed: self.a_scan_speed.smoothed.next(),
                grain_size_s: self.a_grain_size.smoothed.next(),
                grain_rate_hz: self.a_grain_rate.smoothed.next(),
                pitch_variation_st: self.a_pitch_var.smoothed.next(),
                position_variation: self.a_position_var.smoothed.next(),
                onset_timing: self.a_timing.smoothed.next(),
                stereo_width: self.a_width.smoothed.next(),
                reverse_chance: self.a_reverse_chance.smoothed.next(),
                grain_shape: self.a_grain_shape.smoothed.next(),
            }
        } else {
            sound::Layer {
                reader: self.b_reader.value().dsp(),
                root: self.b_root.smoothed.next(),
                transpose: self.b_transpose.smoothed.next(),
                start: self.b_start.smoothed.next(),
                end: self.b_end.smoothed.next(),
                loop_start: self.b_loop_start.smoothed.next(),
                loop_end: self.b_loop_end.smoothed.next(),
                loop_crossfade_s: self.b_loop_crossfade.smoothed.next(),
                loop_mode: self.b_loop.value().dsp(),
                reverse: self.b_reverse.value(),
                level: self.b_level.smoothed.next(),
                pan: self.b_pan.smoothed.next(),
                scan_speed: self.b_scan_speed.smoothed.next(),
                grain_size_s: self.b_grain_size.smoothed.next(),
                grain_rate_hz: self.b_grain_rate.smoothed.next(),
                pitch_variation_st: self.b_pitch_var.smoothed.next(),
                position_variation: self.b_position_var.smoothed.next(),
                onset_timing: self.b_timing.smoothed.next(),
                stereo_width: self.b_width.smoothed.next(),
                reverse_chance: self.b_reverse_chance.smoothed.next(),
                grain_shape: self.b_grain_shape.smoothed.next(),
            }
        }
    }

    /// Every value the DSP reads but the routes, for this sample: each smoothed parameter advanced
    /// once, the rest as they stand. **Bend range is read, not advanced**: the plugin follows it
    /// sample by sample itself (`MxmCreativeSampler::follow_bend_range`).
    pub fn sound(&self) -> sound::Sound {
        sound::Sound {
            layers: [self.layer(0), self.layer(1)],
            character: sound::Character {
                sample_rate: self.rate.smoothed.next(),
                bit_depth: self.converter.smoothed.next(),
                bit_smoothing: self.reconstruction.smoothed.next(),
                converter_drive: self.input.smoothed.next(),
                converter_type: self.converter_type.value().dsp(),
                jitter: self.jitter.smoothed.next(),
            },
            filter: sound::Filter {
                cutoff_hz: self.cutoff.smoothed.next(),
                resonance: self.resonance.smoothed.next(),
                mode: self.filter_mode.value().dsp(),
                drive: self.drive.smoothed.next(),
            },
            envelope: sound::Envelope {
                attack_s: self.attack.smoothed.next(),
                decay_s: self.decay.smoothed.next(),
                sustain: self.sustain.smoothed.next(),
                release_s: self.release.smoothed.next(),
            },
            lfos: [
                sound::Lfo {
                    rate_hz: self.lfo_rate.smoothed.next(),
                    shape: self.lfo_shape.value().dsp(),
                },
                sound::Lfo {
                    rate_hz: self.lfo2_rate.smoothed.next(),
                    shape: self.lfo2_shape.value().dsp(),
                },
            ],
            performance: sound::Performance {
                master: self.master.smoothed.next(),
                voices: self.voice_mode.value().dsp(),
                glide_s_per_octave: self.glide.value(),
                bend_range_st: self.bend_range.value(),
            },
        }
    }
}

impl MxmCreativeSamplerParams {
    /// LFO 1's rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`LFO_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_lfo1_rate(&self, tempo: Option<f64>) -> Option<f32> {
        TEMPO_SYNCS[0].resolve(
            self.lfo1_sync.value(),
            tempo,
            self.lfo_rate.modulated_normalized_value(),
        )
    }

    /// LFO 2's rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`LFO_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_lfo2_rate(&self, tempo: Option<f64>) -> Option<f32> {
        TEMPO_SYNCS[1].resolve(
            self.lfo2_sync.value(),
            tempo,
            self.lfo2_rate.modulated_normalized_value(),
        )
    }

    /// A's grain rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`GRAIN_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_a_grain_rate(&self, tempo: Option<f64>) -> Option<f32> {
        TEMPO_SYNCS[2].resolve(
            self.a_grain_sync.value(),
            tempo,
            self.a_grain_rate.modulated_normalized_value(),
        )
    }

    /// B's grain rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`GRAIN_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_b_grain_rate(&self, tempo: Option<f64>) -> Option<f32> {
        TEMPO_SYNCS[3].resolve(
            self.b_grain_sync.value(),
            tempo,
            self.b_grain_rate.modulated_normalized_value(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **LFO 1's sync picks a division and is inert without a tempo**
    /// (`plans/plan-tempo-sync-controls.md`): off, or with no tempo, the knob's own hertz stand; on
    /// at 120 bpm the ends are the ladder's ends that the range can hold, the top the fastest.
    #[test]
    fn lfo_1_sync_picks_a_division_and_is_inert_without_a_tempo() {
        use nice_plug::params::InternalParamMut;
        fn set<P: InternalParamMut>(param: &P, normalized: f32) {
            unsafe {
                let _ = param._internal_set_normalized_value(normalized);
            }
        }
        let p = MxmCreativeSamplerParams::default();
        set(&p.lfo_rate, 1.0);
        assert_eq!(
            p.synced_lfo1_rate(Some(120.0)),
            None,
            "off is the free rate"
        );
        set(&p.lfo1_sync, 1.0);
        assert_eq!(p.synced_lfo1_rate(None), None, "no tempo is the free rate");

        let top = p.synced_lfo1_rate(Some(120.0)).expect("synced at a tempo");
        set(&p.lfo_rate, 0.0);
        let bottom = p.synced_lfo1_rate(Some(120.0)).expect("synced at a tempo");
        let (lo, hi) = (
            f64::from(p.lfo_rate.preview_plain(0.0)),
            f64::from(p.lfo_rate.preview_plain(1.0)),
        );
        assert!(
            top > bottom,
            "the top of a rate is the fastest: {bottom} to {top}"
        );
        let reach = LFO_SYNC.reachable(120.0, lo, hi).divisions();
        let fastest = reach[0].hz(120.0) as f32;
        let slowest = reach[reach.len() - 1].hz(120.0) as f32;
        assert!((top - fastest).abs() < 1e-4, "{top} against {fastest}");
        assert!(
            (bottom - slowest).abs() < 1e-4,
            "{bottom} against {slowest}"
        );
    }

    /// **A's grain-rate sync picks a division and is inert without a tempo**
    /// (`plans/plan-tempo-sync-controls.md`): off, or with no tempo, the knob's own hertz stand; on
    /// at 120 bpm the ends are the ladder's ends that the range can hold, the top the fastest.
    #[test]
    fn a_grain_sync_picks_a_division_and_is_inert_without_a_tempo() {
        use nice_plug::params::InternalParamMut;
        fn set<P: InternalParamMut>(param: &P, normalized: f32) {
            unsafe {
                let _ = param._internal_set_normalized_value(normalized);
            }
        }
        let p = MxmCreativeSamplerParams::default();
        set(&p.a_grain_rate, 1.0);
        assert_eq!(
            p.synced_a_grain_rate(Some(120.0)),
            None,
            "off is the free rate"
        );
        set(&p.a_grain_sync, 1.0);
        assert_eq!(
            p.synced_a_grain_rate(None),
            None,
            "no tempo is the free rate"
        );

        let top = p
            .synced_a_grain_rate(Some(120.0))
            .expect("synced at a tempo");
        set(&p.a_grain_rate, 0.0);
        let bottom = p
            .synced_a_grain_rate(Some(120.0))
            .expect("synced at a tempo");
        let (lo, hi) = (
            f64::from(p.a_grain_rate.preview_plain(0.0)),
            f64::from(p.a_grain_rate.preview_plain(1.0)),
        );
        assert!(
            top > bottom,
            "the top of a rate is the fastest: {bottom} to {top}"
        );
        let reach = GRAIN_SYNC.reachable(120.0, lo, hi).divisions();
        let fastest = reach[0].hz(120.0) as f32;
        let slowest = reach[reach.len() - 1].hz(120.0) as f32;
        assert!((top - fastest).abs() < 1e-4, "{top} against {fastest}");
        assert!(
            (bottom - slowest).abs() < 1e-4,
            "{bottom} against {slowest}"
        );
    }

    #[test]
    fn init_uses_the_current_live_sound_declared_in_one_patch() {
        let params = MxmCreativeSamplerParams::default();
        let init = &crate::init::PATCH;
        let close = |actual: f32, expected: f32, name: &str| {
            assert!(
                (actual - expected).abs() < 1.0e-6,
                "{name}: {actual} is not {expected}"
            );
        };

        assert_eq!(params.a_reader.default_plain_value(), ReaderChoice::Grain);
        close(params.a_root.default_plain_value(), 48.0, "root");
        close(params.a_level.default_plain_value(), 0.60, "level");
        close(
            params.a_scan_speed.default_plain_value(),
            0.30,
            "scan speed",
        );
        close(
            params.a_grain_size.default_plain_value(),
            init.sound.layers[0].grain_size_s,
            "grain size",
        );
        close(
            params.a_grain_rate.default_plain_value(),
            init.sound.layers[0].grain_rate_hz,
            "grain rate",
        );
        close(
            params.a_pitch_var.default_plain_value(),
            init.sound.layers[0].pitch_variation_st,
            "pitch variation",
        );
        close(
            params.a_position_var.default_plain_value(),
            init.sound.layers[0].position_variation,
            "position variation",
        );
        close(
            params.a_timing.default_plain_value(),
            0.185_000_02,
            "onset timing",
        );
        close(params.a_width.default_plain_value(), 0.0, "stereo width");
        // An amount, so zero — and zero is the hard wrap, which keeps this sound bit-identical.
        close(
            params.a_loop_crossfade.default_plain_value(),
            0.0,
            "A loop crossfade",
        );
        close(
            params.b_loop_crossfade.default_plain_value(),
            0.0,
            "B loop crossfade",
        );

        close(params.converter.default_plain_value(), 0.0, "bit depth");
        close(
            params.reconstruction.default_plain_value(),
            0.0,
            "bit smoothing",
        );
        close(params.input.default_plain_value(), 0.0, "converter drive");
        close(params.jitter.default_plain_value(), 0.0, "jitter");
        close(
            params.cutoff.default_plain_value(),
            init.sound.filter.cutoff_hz,
            "cutoff",
        );
        close(params.resonance.default_plain_value(), 0.38, "resonance");
        close(
            params.lfo2_rate.default_plain_value(),
            init.sound.lfos[1].rate_hz,
            "LFO 2 rate",
        );

        // Captured normalised values from the running player: these pin the two skewed controls to
        // the exact gestures the owner selected rather than merely to their rounded display text.
        close(
            params.a_grain_size.default_normalized_value(),
            0.638_738_63,
            "normalised grain size",
        );
        close(
            params.a_grain_rate.default_normalized_value(),
            0.520_413_4,
            "normalised grain rate",
        );
    }

    /// Plain values either side of a formatter's branch point: the point, and fractions of the
    /// **finer** branch's printed step around it, both halves of its rounding included.
    fn around(point: f32, step: f32) -> impl Iterator<Item = f32> {
        [
            -1.0, -0.6, -0.5, -0.49, -0.4, -0.1, 0.0, 0.1, 0.4, 0.49, 0.5, 0.6, 1.0,
        ]
        .into_iter()
        .map(move |k| point + k * step)
    }

    /// Plain values on both sides of every place a formatter here changes its unit, its precision
    /// or its words. **Sample rate's are hertz converted into its knob**, against the source rate its
    /// reading is quoted at, because the knob is the plain value and the kilohertz are not.
    fn branch_plains(id: &str, source_rate: f32) -> Vec<f32> {
        let knob = |hz: f32| (source_rate / hz).max(1.0).log2() / 5.0;
        let landmarks = || [0.0, 0.5, 1.0].into_iter().flat_map(|at| around(at, 0.01));
        match id {
            // Whole hertz below a kilohertz, hundredths of one below ten, tenths from ten.
            "rate" => around(1_000.0, 1.0)
                .chain(around(10_000.0, 10.0))
                .map(knob)
                .collect(),
            // Tenths of a hertz below a kilohertz, tenths of a kilohertz from one.
            "cutoff" => around(1_000.0, 0.1).collect(),
            // A note name alone on a whole note, with cents off one; the name changes at the half.
            "aroot" | "broot" => (0..=127)
                .flat_map(|note| around(note as f32, 0.01).chain(around(note as f32 + 0.5, 0.01)))
                .collect(),
            // Tenths of a millisecond below a second, hundredths of a second from one.
            "aloopcrossfade" | "bloopcrossfade" => around(1.0, 0.000_1).collect(),
            // Words at the two ends and the middle, whole percents between.
            "agrainshape" | "bgrainshape" => landmarks().collect(),
            "reconstruction" => [0.0, 1.0]
                .into_iter()
                .flat_map(|at| around(at, 0.01))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// **Every parameter's text survives the host's round trip**, with the unit on, across the
    /// collection's grids and switch points (`mxm_plugin_test::text_checks`) at three source rates,
    /// because Sample rate's reading depends on the loaded sample's, plus both sides of every
    /// branch point in [`branch_plains`].
    #[test]
    fn every_parameter_text_survives_the_hosts_round_trip() {
        let params = MxmCreativeSamplerParams::default();
        for source_rate in [DEFAULT_SOURCE_RATE, 44_100, 22_050] {
            params.source_rate.store(source_rate, Ordering::Relaxed);
            let boundaries: Vec<f32> = params
                .param_map()
                .iter()
                .flat_map(|(id, _, _)| branch_plains(id, source_rate as f32))
                .collect();
            mxm_plugin_test::text_checks::every_text_round_trips(&params, &boundaries);
        }
    }

    /// **Root holds cents without disturbing a whole-note Root** (D6). A whole note reads exactly as it
    /// always has; a Root between notes shows its cents and its number, and typed cents are taken.
    #[test]
    fn root_holds_cents_and_reads_a_whole_note_as_before() {
        let params = MxmCreativeSamplerParams::default();
        let root = &params.a_root;
        let text = |plain: f32| root.normalized_value_to_string(plain / 127.0, true);
        let parse = |typed: &str| {
            root.string_to_normalized_value(typed)
                .map(|normalised| normalised * 127.0)
        };
        assert_eq!(text(48.0), "C3 · 48", "a whole note changed how it reads");
        assert_eq!(text(59.75), "C4 \u{2212}25 ct · 59.75");
        assert_eq!(text(60.16), "C4 +16 ct · 60.16");
        for (typed, wanted) in [
            ("C4 \u{2212}25 ct", 59.75),
            ("C4 -25ct", 59.75),
            ("C4 +16 ct", 60.16),
            ("D4", 62.0),
            ("59.75", 59.75),
            ("C4 \u{2212}25 ct · 59.75", 59.75),
            ("C3 · 48", 48.0),
        ] {
            let got = parse(typed).unwrap_or_else(|| panic!("{typed:?} was refused"));
            assert!(
                (got - wanted).abs() < 1.0e-3,
                "{typed:?} parsed to {got}, not {wanted}"
            );
        }
    }

    /// **Every parameter is the state crate's**, so the plugin and anything reading its state cannot
    /// disagree: the ids its `#[id]` attributes declare, in the order a host lists them, each
    /// group, name, unit, step count and default, and at every point of the travel the plain value a
    /// host's normalised one stores, the text a host shows and what that text parses back to — at
    /// three source rates, because Sample rate's reading depends on the loaded sample's. The route
    /// amounts are built by mxm-kit's own `amount_param_at`, so this is also what holds the state
    /// crate's copy of the collection's route reading to the kit's.
    #[test]
    fn every_parameter_is_the_tables() {
        let params = MxmCreativeSamplerParams::default();
        let map = params.param_map();
        let table = table::all();
        assert_eq!(
            map.iter().map(|(id, _, _)| id.as_str()).collect::<Vec<_>>(),
            table.iter().map(|param| param.id).collect::<Vec<_>>(),
            "the declared ids are not the table's, in the table's order"
        );
        let typed = [
            "",
            "0",
            "1",
            "-1",
            "0.5",
            "50 %",
            "-25%",
            "+12 st",
            "C4",
            "c#3 +25 ct",
            "59.75",
            "1 kHz",
            "1000 Hz",
            "500 ms",
            "1.5 s",
            "On",
            "off",
            "yes",
            "Reverse",
            "Hard",
            "25% stepped",
            "8 bits",
            "+6.0 dB",
            "0.5x",
            "+4.00 oct",
            "-20 %/oct",
            "Grain",
            "Low pass",
            "Sample & hold",
            "garbage",
            "A4, +10 cents",
        ];
        for rate in [DEFAULT_SOURCE_RATE, 44_100, 22_050] {
            params.source_rate.store(rate, Ordering::Relaxed);
            for ((id, ptr, group), spec) in map.iter().zip(table) {
                unsafe {
                    assert_eq!(group.as_str(), spec.group, "{id}'s group");
                    assert_eq!(ptr.name(), spec.name, "{id}'s name");
                    let unit = spec.float().map_or("", |float| float.unit);
                    assert_eq!(ptr.unit(), unit, "{id}'s unit");
                    assert_eq!(ptr.step_count(), spec.step_count(), "{id}'s steps");
                    assert_eq!(
                        ptr.default_normalized_value().to_bits(),
                        spec.default_normalized().to_bits(),
                        "{id}'s default"
                    );
                    let mut grid: Vec<f32> = (0..=400).map(|i| i as f32 / 400.0).collect();
                    grid.extend([1.0e-6, 0.4999, 0.5001, 0.999_999]);
                    for normalized in grid {
                        if spec.float().is_some() {
                            assert_eq!(
                                ptr.preview_plain(normalized).to_bits(),
                                spec.plain(normalized).to_bits(),
                                "{id} at {normalized}"
                            );
                        }
                        let text = ptr.normalized_value_to_string(normalized, true);
                        assert_eq!(text, spec.text(normalized, rate), "{id} at {normalized}");
                        assert_eq!(
                            ptr.string_to_normalized_value(&text).map(f32::to_bits),
                            spec.parse(&text, rate).map(f32::to_bits),
                            "{id} reading {text:?}"
                        );
                    }
                    for text in typed {
                        assert_eq!(
                            ptr.string_to_normalized_value(text).map(f32::to_bits),
                            spec.parse(text, rate).map(f32::to_bits),
                            "{id} reading {text:?}"
                        );
                    }
                }
            }
        }
    }

    /// **Every choice enum is the table's**: its variants' ids and names in the table's order, and
    /// each the DSP value the table maps that index to.
    #[test]
    fn every_choice_is_the_tables() {
        use mxm_creative_sampler_state::sound::{self, Choices};
        fn check<T: Enum + Copy, D: Copy + PartialEq + std::fmt::Debug>(
            choices: Choices<D>,
            dsp: impl Fn(T) -> D,
        ) {
            let ids = T::ids().expect("every choice declares its ids");
            assert_eq!(T::variants().len(), choices.variants.len());
            for (index, variant) in choices.variants.iter().enumerate() {
                assert_eq!(ids[index], variant.id);
                assert_eq!(T::variants()[index], variant.name);
                assert_eq!(dsp(T::from_index(index)), choices.values[index]);
            }
        }
        check(sound::READERS, ReaderChoice::dsp);
        check(sound::LOOP_MODES, LoopChoice::dsp);
        check(sound::VOICE_MODES, VoiceChoice::dsp);
        check(sound::LFO_SHAPES, ShapeChoice::dsp);
        check(sound::CONVERTER_TYPES, ConverterChoice::dsp);
        check(sound::FILTER_MODES, FilterChoice::dsp);
    }
}
