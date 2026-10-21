//! Every parameter the plugin has: its permanent id, the name a host shows, how it travels, its
//! default, its unit and its text both ways.
//!
//! **The plugin's parameters are built from this table** (`plugins/mxm-creative-sampler/src/params.rs`
//! and `routes.rs`), and its tests hold every one to it: the ids its `#[id]` attributes declare, in
//! the order a host lists them, each name, travel, default and reading. So a list of these rows,
//! name and text, is the plugin's own generic parameter list, character for character — what the
//! web tracker shows for the sampler (`plans/plan-web-tracker.md`, W6).
//!
//! **A value here is a plain `f32`**, as nice-plug's `ParamPtr` hands one over, with one
//! difference: a switch is `0.0` or `1.0`, never its normalised value. A choice is its index.
//!
//! **All parameter ids are provisional until the S3 owner listening gate** (the plugin's
//! `AGENTS.md`). They move here and in the plugin's `#[id]` attributes together.

use std::sync::LazyLock;

use mxm_creative_sampler_dsp::routing::{SOURCES, TARGETS};

use crate::range::{self, Range, skew_factor};
use crate::routes::{self, ROUTE_IDS};
use crate::sound::{
    CONVERTER_TYPES, FILTER_MODES, LFO_SHAPES, LOOP_MODES, READERS, VOICE_MODES, Variant,
};
use crate::text::{Reading, SwitchReading};

/// One parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    /// Its permanent id: the key a state stores its value under.
    pub id: &'static str,
    /// The name a host shows.
    pub name: String,
    /// The group a host lists it under (CLAP's module): empty, or [`routes::GROUP`].
    pub group: &'static str,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A continuous value.
    Float(Float),
    /// On or off.
    Switch(Switch),
    /// One of a list.
    Choice(Choice),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Float {
    pub range: Range,
    /// A step the stored value snaps to, where there is one. Still continuous to a host: nice-plug
    /// reports no step count for a stepped float.
    pub step: Option<f32>,
    pub default: f32,
    /// What follows the reading, separator included: `" Hz"`, `"%"`, or nothing where the reading
    /// carries its own.
    pub unit: &'static str,
    /// How the plugin smooths a change of it.
    pub smoothing: Smoothing,
    pub reading: Reading,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    pub default: bool,
    pub reading: SwitchReading,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// In index order: what a state stores is the variant's id.
    pub variants: &'static [Variant],
    pub default: usize,
}

/// How the plugin smooths a change: nice-plug's `SmoothingStyle`, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Smoothing {
    None,
    Linear(f32),
    Logarithmic(f32),
}

impl Param {
    /// The default, plain.
    #[must_use]
    pub fn default_plain(&self) -> f32 {
        match &self.kind {
            Kind::Float(float) => float.default,
            Kind::Switch(switch) => f32::from(u8::from(switch.default)),
            Kind::Choice(choice) => choice.default as f32,
        }
    }

    /// The default as the host's `0..=1`.
    #[must_use]
    pub fn default_normalized(&self) -> f32 {
        self.normalized(self.default_plain())
    }

    /// How many steps a host sees: `None` for a continuous value, one for a switch, and one fewer
    /// than its variants for a choice. A host's value is `normalised × steps`.
    #[must_use]
    pub fn step_count(&self) -> Option<usize> {
        match &self.kind {
            Kind::Float(_) => None,
            Kind::Switch(_) => Some(1),
            Kind::Choice(choice) => Some(choice.variants.len() - 1),
        }
    }

    /// `plain` as the host's `0..=1`.
    #[must_use]
    pub fn normalized(&self, plain: f32) -> f32 {
        match &self.kind {
            Kind::Float(float) => float.range.normalize(plain),
            Kind::Switch(_) => f32::from(u8::from(plain > 0.5)),
            Kind::Choice(choice) => range::index_normalized(plain, choice.variants.len()),
        }
    }

    /// The plain value a host's `0..=1` stores: **what the plugin keeps when a host sets it**.
    #[must_use]
    pub fn plain(&self, normalized: f32) -> f32 {
        match &self.kind {
            Kind::Float(float) => {
                let value = float.range.unnormalize(normalized);
                match float.step {
                    Some(step) => float.range.snap_to_step(value, step),
                    None => value,
                }
            }
            Kind::Switch(_) => f32::from(u8::from(normalized > 0.5)),
            Kind::Choice(choice) => range::index_plain(normalized, choice.variants.len()) as f32,
        }
    }

    /// `plain` as the plugin shows it, its unit included. `source_rate` is the loaded sample's,
    /// which only the converter's Sample rate reads ([`crate::State::source_rate`]).
    #[must_use]
    pub fn format(&self, plain: f32, source_rate: u32) -> String {
        match &self.kind {
            Kind::Float(float) => {
                format!("{}{}", float.reading.show(plain, source_rate), float.unit)
            }
            Kind::Switch(switch) => switch.reading.show(plain > 0.5),
            Kind::Choice(choice) => {
                let index = (plain.max(0.0) as usize).min(choice.variants.len() - 1);
                choice.variants[index].name.to_owned()
            }
        }
    }

    /// **What a host shows for its `0..=1`**: the plain value it stores, formatted. nice-plug's
    /// `normalized_value_to_string(normalized, true)`, which CLAP's `value_to_text` calls.
    #[must_use]
    pub fn text(&self, normalized: f32, source_rate: u32) -> String {
        self.format(self.plain(normalized), source_rate)
    }

    /// What typed `text` means as the host's `0..=1`, or `None` where it means nothing: nice-plug's
    /// `string_to_normalized_value`, which CLAP's `text_to_value` calls.
    #[must_use]
    pub fn parse(&self, text: &str, source_rate: u32) -> Option<f32> {
        let text = text.trim();
        match &self.kind {
            Kind::Float(float) => {
                let plain = float.reading.read(text, float.unit, source_rate)?;
                Some(float.range.normalize(plain))
            }
            Kind::Switch(switch) => {
                let on = switch.reading.read(text)?;
                Some(f32::from(u8::from(on)))
            }
            Kind::Choice(choice) => {
                let index = choice
                    .variants
                    .iter()
                    .position(|variant| variant.name == text)?;
                Some(range::index_normalized(index as f32, choice.variants.len()))
            }
        }
    }

    /// The continuous half, for a value the plugin builds as a `FloatParam`.
    #[must_use]
    pub fn float(&self) -> Option<&Float> {
        match &self.kind {
            Kind::Float(float) => Some(float),
            _ => None,
        }
    }
}

impl Float {
    /// `plain` as the plugin shows it, without its unit: nice-plug's `value_to_string`.
    #[must_use]
    pub fn show(&self, plain: f32, source_rate: u32) -> String {
        self.reading.show(plain, source_rate)
    }

    /// What `text`, trimmed, means as a plain value: nice-plug's `string_to_value`.
    #[must_use]
    pub fn read(&self, text: &str, source_rate: u32) -> Option<f32> {
        self.reading.read(text, self.unit, source_rate)
    }
}

/// Every parameter, **in the order a host lists them**: the plugin's declaration order, layer A,
/// layer B, the shared controls, then the routes, a presence before its amount.
#[must_use]
pub fn all() -> &'static [Param] {
    &TABLE
}

/// The parameter `id` names.
#[must_use]
pub fn get(id: &str) -> Option<&'static Param> {
    TABLE.iter().find(|param| param.id == id)
}

/// Where `id` sits in [`all`].
#[must_use]
pub fn index(id: &str) -> Option<usize> {
    TABLE.iter().position(|param| param.id == id)
}

/// The parameter `id` names, which the table is known to hold.
pub(crate) fn known(id: &str) -> &'static Param {
    get(id).unwrap_or_else(|| panic!("`{id}` is not in the parameter table"))
}

static TABLE: LazyLock<Vec<Param>> = LazyLock::new(build);

/// The ids of one layer's controls. **The two layers differ only by their prefix**, `a` and `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerIds {
    pub reader: &'static str,
    pub root: &'static str,
    pub transpose: &'static str,
    pub start: &'static str,
    pub end: &'static str,
    pub loop_start: &'static str,
    pub loop_end: &'static str,
    pub loop_crossfade: &'static str,
    pub loop_mode: &'static str,
    pub reverse: &'static str,
    pub level: &'static str,
    pub pan: &'static str,
    pub scan_speed: &'static str,
    pub grain_size: &'static str,
    pub grain_rate: &'static str,
    pub grain_sync: &'static str,
    pub pitch_variation: &'static str,
    pub position_variation: &'static str,
    pub onset_timing: &'static str,
    pub stereo_width: &'static str,
    pub reverse_chance: &'static str,
    pub grain_shape: &'static str,
}

/// Layer A's ids, then layer B's.
pub const LAYERS: [LayerIds; 2] = [
    LayerIds {
        reader: "areader",
        root: "aroot",
        transpose: "atranspose",
        start: "astart",
        end: "aend",
        loop_start: "aloopstart",
        loop_end: "aloopend",
        loop_crossfade: "aloopcrossfade",
        loop_mode: "aloop",
        reverse: "areverse",
        level: "alevel",
        pan: "apan",
        scan_speed: "ascanspeed",
        grain_size: "agrainsize",
        grain_rate: "agrainrate",
        grain_sync: "agrainsync",
        pitch_variation: "apitchvar",
        position_variation: "aposvar",
        onset_timing: "atiming",
        stereo_width: "awidth",
        reverse_chance: "areversechance",
        grain_shape: "agrainshape",
    },
    LayerIds {
        reader: "breader",
        root: "broot",
        transpose: "btranspose",
        start: "bstart",
        end: "bend",
        loop_start: "bloopstart",
        loop_end: "bloopend",
        loop_crossfade: "bloopcrossfade",
        loop_mode: "bloop",
        reverse: "breverse",
        level: "blevel",
        pan: "bpan",
        scan_speed: "bscanspeed",
        grain_size: "bgrainsize",
        grain_rate: "bgrainrate",
        grain_sync: "bgrainsync",
        pitch_variation: "bpitchvar",
        position_variation: "bposvar",
        onset_timing: "btiming",
        stereo_width: "bwidth",
        reverse_chance: "breversechance",
        grain_shape: "bgrainshape",
    },
];

/// The shared controls' ids.
pub mod id {
    pub const SAMPLE_RATE: &str = "rate";
    pub const BIT_DEPTH: &str = "converter";
    /// `Bit smoothing`, the owner's painted name; the id is the mechanism's.
    pub const BIT_SMOOTHING: &str = "reconstruction";
    pub const CONVERTER_DRIVE: &str = "input";
    pub const CONVERTER_TYPE: &str = "convtype";
    pub const JITTER: &str = "jitter";
    pub const CUTOFF: &str = "cutoff";
    pub const RESONANCE: &str = "resonance";
    pub const FILTER_MODE: &str = "filtermode";
    pub const DRIVE: &str = "drive";
    pub const ATTACK: &str = "attack";
    pub const DECAY: &str = "decay";
    pub const SUSTAIN: &str = "sustain";
    pub const RELEASE: &str = "release";
    pub const LFO1_RATE: &str = "lforate";
    pub const LFO1_SYNC: &str = "lfo1sync";
    pub const LFO1_SHAPE: &str = "lfo1shape";
    pub const LFO2_RATE: &str = "lfo2rate";
    pub const LFO2_SYNC: &str = "lfo2sync";
    pub const LFO2_SHAPE: &str = "lfo2shape";
    pub const MASTER: &str = "master";
    pub const VOICE_MODE: &str = "voicemode";
    pub const GLIDE: &str = "glide";
    pub const BEND_RANGE: &str = "bendrange";
}

/// **The LFO rate's tempo sync** (`plans/plan-tempo-sync-controls.md`): every LFO's ladder, 1/32 to
/// four bars, the top the fastest.
pub const LFO_SYNC: mxm_tempo::Ladder =
    mxm_tempo::Ladder::new(mxm_tempo::Span::LFO, mxm_tempo::Direction::Rate);

/// **A grain rate's tempo sync**: 1/64 to a whole note, the slice of the ladder a grain rate's
/// 0.5 – 200 per second holds at 120 bpm, the top the fastest.
pub const GRAIN_SYNC: mxm_tempo::Ladder = mxm_tempo::Ladder::new(
    mxm_tempo::Span::new(mxm_tempo::Division::SixtyFourth, mxm_tempo::Division::Whole),
    mxm_tempo::Direction::Rate,
);

/// One *<Control> sync* button and the rate it changes.
#[derive(Debug, Clone, Copy)]
pub struct TempoSync {
    /// The switch's id.
    pub switch: &'static str,
    /// The rate's id.
    pub rate: &'static str,
    pub ladder: mxm_tempo::Ladder,
}

impl TempoSync {
    /// The rate while the sync follows the tempo, or `None` for its free value: the rate's
    /// `normalized` position picks a division on the ladder. **What the plugin resolves once a
    /// callback**, from the rate's modulated position.
    #[must_use]
    pub fn resolve(&self, on: bool, tempo: Option<f64>, normalized: f32) -> Option<f32> {
        let rate = known(self.rate);
        self.ladder
            .resolve(
                on,
                tempo,
                normalized,
                f64::from(rate.plain(0.0)),
                f64::from(rate.plain(1.0)),
            )
            .map(|hz| hz as f32)
    }
}

/// The four tempo syncs, in the order the plugin resolves them: LFO 1, LFO 2, A's and B's grain
/// rates.
pub const TEMPO_SYNCS: [TempoSync; 4] = [
    TempoSync {
        switch: id::LFO1_SYNC,
        rate: id::LFO1_RATE,
        ladder: LFO_SYNC,
    },
    TempoSync {
        switch: id::LFO2_SYNC,
        rate: id::LFO2_RATE,
        ladder: LFO_SYNC,
    },
    TempoSync {
        switch: LAYERS[0].grain_sync,
        rate: LAYERS[0].grain_rate,
        ladder: GRAIN_SYNC,
    },
    TempoSync {
        switch: LAYERS[1].grain_sync,
        rate: LAYERS[1].grain_rate,
        ladder: GRAIN_SYNC,
    },
];

/// **The tempo syncs this plugin gained on 2026-09-25** (`plans/plan-tempo-sync-controls.md`). A
/// state or preset written before them was written unsynced, so each loads off rather than keeping
/// what the instance had.
pub const TEMPO_SYNC_IDS: [&str; 4] = [
    TEMPO_SYNCS[0].switch,
    TEMPO_SYNCS[1].switch,
    TEMPO_SYNCS[2].switch,
    TEMPO_SYNCS[3].switch,
];

fn float(
    id: &'static str,
    name: String,
    range: Range,
    default: f32,
    unit: &'static str,
    smoothing: Smoothing,
    reading: Reading,
) -> Param {
    Param {
        id,
        name,
        group: "",
        kind: Kind::Float(Float {
            range,
            step: None,
            default,
            unit,
            smoothing,
            reading,
        }),
    }
}

fn switch(id: &'static str, name: String, default: bool, reading: SwitchReading) -> Param {
    Param {
        id,
        name,
        group: "",
        kind: Kind::Switch(Switch { default, reading }),
    }
}

fn choice(id: &'static str, name: String, variants: &'static [Variant], default: usize) -> Param {
    Param {
        id,
        name,
        group: "",
        kind: Kind::Choice(Choice { variants, default }),
    }
}

const UNIT: Range = Range::Linear { min: 0.0, max: 1.0 };

/// A whole percentage of `0..=1`.
fn percent(id: &'static str, name: String, default: f32) -> Param {
    float(
        id,
        name,
        UNIT,
        default,
        "%",
        Smoothing::Linear(20.0),
        Reading::Percent,
    )
}

/// An LFO's rate. Skewed: the slow end is where a drift lives and the fast end is an effect.
fn lfo_rate(id: &'static str, name: String, default: f32) -> Param {
    float(
        id,
        name,
        Range::Skewed {
            min: 0.01,
            max: 40.0,
            factor: skew_factor(-1.5),
        },
        default,
        " Hz",
        Smoothing::Logarithmic(30.0),
        Reading::Decimals(2),
    )
}

/// A converter stage, `0..=1`, which **reads as the number the machine was sold on**, not as a
/// percentage of a knob: eight bits and twenty kilohertz are what a Fairlight or an Emulator is
/// remembered by. The laws live in the DSP, so the reading cannot drift from what the audio does.
fn converter_stage(id: &'static str, name: &str, default: f32, reading: Reading) -> Param {
    float(
        id,
        name.to_owned(),
        UNIT,
        default,
        "",
        Smoothing::Linear(20.0),
        reading,
    )
}

/// An envelope time, in seconds.
fn seconds(id: &'static str, name: &str, default: f32, max: f32) -> Param {
    float(
        id,
        name.to_owned(),
        Range::Skewed {
            min: 0.000_05,
            max,
            factor: skew_factor(-2.0),
        },
        default,
        " s",
        Smoothing::None,
        Reading::Decimals(3),
    )
}

fn build() -> Vec<Param> {
    let init = &crate::init::PATCH;
    let sound = &init.sound;
    let mut out = Vec::with_capacity(2 * 22 + 24 + 2 * TARGETS * SOURCES);
    for (layer, ids) in LAYERS.iter().enumerate() {
        let at = sound.layers[layer];
        let named = |what: &str| format!("{} {what}", ["A", "B"][layer]);
        out.extend([
            choice(
                ids.reader,
                named("Playback mode"),
                READERS.variants,
                READERS.index_of(at.reader),
            ),
            float(
                ids.root,
                named("Root"),
                Range::Linear {
                    min: 0.0,
                    max: 127.0,
                },
                at.root,
                "",
                Smoothing::None,
                Reading::Root,
            ),
            float(
                ids.transpose,
                named("Transpose"),
                Range::Linear {
                    min: -24.0,
                    max: 24.0,
                },
                at.transpose,
                " st",
                Smoothing::Linear(20.0),
                Reading::Decimals(2),
            ),
            percent(ids.start, named("Start"), at.start),
            percent(ids.end, named("End"), at.end),
            percent(ids.loop_start, named("Loop start"), at.loop_start),
            percent(ids.loop_end, named("Loop end"), at.loop_end),
            // **A loop crossfade reads in the unit a fade is heard in**: milliseconds where a seam is
            // being smoothed, seconds where a loop is being blurred into itself. Smoothed like the
            // loop points, because it moves the same geometry they do, the return point included.
            float(
                ids.loop_crossfade,
                named("Loop crossfade"),
                Range::Skewed {
                    min: 0.0,
                    max: mxm_creative_sampler_dsp::MAX_LOOP_CROSSFADE_S,
                    factor: skew_factor(-2.0),
                },
                at.loop_crossfade_s,
                "",
                Smoothing::Linear(20.0),
                Reading::Crossfade,
            ),
            choice(
                ids.loop_mode,
                named("Loop"),
                LOOP_MODES.variants,
                LOOP_MODES.index_of(at.loop_mode),
            ),
            switch(
                ids.reverse,
                named("Direction"),
                at.reverse,
                SwitchReading::Direction,
            ),
            percent(ids.level, named("Level"), at.level),
            float(
                ids.pan,
                named("Pan"),
                Range::Linear {
                    min: -1.0,
                    max: 1.0,
                },
                at.pan,
                "%",
                Smoothing::Linear(20.0),
                Reading::BipolarPercent,
            ),
            float(
                ids.scan_speed,
                named("Scan speed"),
                Range::Linear {
                    min: -2.0,
                    max: 2.0,
                },
                at.scan_speed,
                "",
                Smoothing::Linear(30.0),
                Reading::Speed,
            ),
            float(
                ids.grain_size,
                named("Grain size"),
                Range::Skewed {
                    min: 0.008,
                    max: 1.0,
                    factor: skew_factor(-1.8),
                },
                at.grain_size_s,
                " s",
                Smoothing::Linear(30.0),
                Reading::Decimals(3),
            ),
            float(
                ids.grain_rate,
                named("Grain rate"),
                Range::Skewed {
                    min: 0.5,
                    max: mxm_creative_sampler_dsp::MAX_GRAIN_DENSITY_HZ,
                    factor: skew_factor(-1.4),
                },
                at.grain_rate_hz,
                " /s",
                Smoothing::Linear(30.0),
                Reading::Decimals(1),
            ),
            switch(
                ids.grain_sync,
                named("Grain rate sync"),
                false,
                SwitchReading::OnOff,
            ),
            // Skewed hard toward the bottom: a section of players is a few cents apart, and the top
            // of this range is a chord. Without the skew the musical part of the control is the
            // first pixel.
            float(
                ids.pitch_variation,
                named("Pitch variation"),
                Range::Skewed {
                    min: 0.0,
                    max: mxm_creative_sampler_dsp::MAX_GRAIN_SPREAD_ST,
                    factor: skew_factor(-2.2),
                },
                at.pitch_variation_st,
                " st",
                Smoothing::Linear(30.0),
                Reading::Decimals(2),
            ),
            percent(
                ids.position_variation,
                named("Position variation"),
                at.position_variation,
            ),
            percent(ids.onset_timing, named("Onset timing"), at.onset_timing),
            percent(ids.stereo_width, named("Stereo width"), at.stereo_width),
            percent(
                ids.reverse_chance,
                named("Reverse chance"),
                at.reverse_chance,
            ),
            // **The window is a tone control, so it reads as one.** The three landmarks are named
            // and everything between them is a percentage, exactly as `mxm-grain-fx` does it.
            float(
                ids.grain_shape,
                named("Grain shape"),
                UNIT,
                at.grain_shape,
                "",
                Smoothing::Linear(20.0),
                Reading::GrainShape,
            ),
        ]);
    }
    let character = sound.character;
    let filter = sound.filter;
    let envelope = sound.envelope;
    let [lfo1, lfo2] = sound.lfos;
    let performance = sound.performance;
    out.extend([
        // **Sample rate reads in hertz** against the loaded sample's rate, so the reading is the
        // same at every host rate. See `text::DEFAULT_SOURCE_RATE`.
        converter_stage(
            id::SAMPLE_RATE,
            "Sample rate",
            character.sample_rate,
            Reading::SampleRate,
        ),
        converter_stage(
            id::BIT_DEPTH,
            "Bit depth",
            character.bit_depth,
            Reading::BitDepth,
        ),
        converter_stage(
            id::BIT_SMOOTHING,
            "Bit smoothing",
            character.bit_smoothing,
            Reading::BitSmoothing,
        ),
        converter_stage(
            id::CONVERTER_DRIVE,
            "Converter drive",
            character.converter_drive,
            Reading::ConverterDrive,
        ),
        choice(
            id::CONVERTER_TYPE,
            "Converter type".to_owned(),
            CONVERTER_TYPES.variants,
            CONVERTER_TYPES.index_of(character.converter_type),
        ),
        percent(id::JITTER, "Jitter".to_owned(), character.jitter),
        // **2.7 kHz, not 2693**, and kilohertz from the rounded tenth of a hertz, not from the raw
        // value, so every reading of a thousand hertz is `1.0 kHz`.
        float(
            id::CUTOFF,
            "Cutoff".to_owned(),
            Range::Skewed {
                min: 20.0,
                max: 22_000.0,
                factor: skew_factor(-2.2),
            },
            filter.cutoff_hz,
            "",
            Smoothing::Logarithmic(30.0),
            Reading::Cutoff,
        ),
        percent(id::RESONANCE, "Resonance".to_owned(), filter.resonance),
        choice(
            id::FILTER_MODE,
            "Filter mode".to_owned(),
            FILTER_MODES.variants,
            FILTER_MODES.index_of(filter.mode),
        ),
        percent(id::DRIVE, "Drive".to_owned(), filter.drive),
        seconds(id::ATTACK, "Attack", envelope.attack_s, 10.0),
        seconds(id::DECAY, "Decay", envelope.decay_s, 10.0),
        percent(id::SUSTAIN, "Sustain".to_owned(), envelope.sustain),
        seconds(id::RELEASE, "Release", envelope.release_s, 20.0),
        lfo_rate(id::LFO1_RATE, "LFO 1 rate".to_owned(), lfo1.rate_hz),
        switch(
            id::LFO1_SYNC,
            "LFO 1 sync".to_owned(),
            false,
            SwitchReading::OnOff,
        ),
        choice(
            id::LFO1_SHAPE,
            "LFO 1 shape".to_owned(),
            LFO_SHAPES.variants,
            LFO_SHAPES.index_of(lfo1.shape),
        ),
        // **A different rate and shape from LFO 1**, because two identical LFOs are one LFO until
        // somebody changes something.
        lfo_rate(id::LFO2_RATE, "LFO 2 rate".to_owned(), lfo2.rate_hz),
        switch(
            id::LFO2_SYNC,
            "LFO 2 sync".to_owned(),
            false,
            SwitchReading::OnOff,
        ),
        choice(
            id::LFO2_SHAPE,
            "LFO 2 shape".to_owned(),
            LFO_SHAPES.variants,
            LFO_SHAPES.index_of(lfo2.shape),
        ),
        percent(id::MASTER, "Master".to_owned(), performance.master),
        choice(
            id::VOICE_MODE,
            "Voices".to_owned(),
            VOICE_MODES.variants,
            VOICE_MODES.index_of(performance.voices),
        ),
        // Glide is the time to cover an octave, not the time between any two keys: an interval
        // measured in seconds-per-note makes a wide leap arrive at the same moment as a semitone.
        float(
            id::GLIDE,
            "Glide".to_owned(),
            Range::Skewed {
                min: 0.0,
                max: 5.0,
                factor: skew_factor(-2.0),
            },
            performance.glide_s_per_octave,
            " s/oct",
            Smoothing::None,
            Reading::Decimals(3),
        ),
        // Smoothed because it scales a held bend into every voice's pitch: a range edit under a
        // held bend would otherwise be a pitch step (mxm-kit's `docs/code-review-notes.md` §2).
        Param {
            id: id::BEND_RANGE,
            name: "Bend range".to_owned(),
            group: "",
            kind: Kind::Float(Float {
                range: Range::Linear {
                    min: 0.0,
                    max: 24.0,
                },
                step: Some(1.0),
                default: performance.bend_range_st,
                unit: " st",
                smoothing: Smoothing::Linear(20.0),
                reading: Reading::Decimals(0),
            }),
        },
    ]);
    // **Every routing pair, after the fixed surface**, so every parameter that existed before the
    // routing keeps its place in a host's list. A presence is present at Init only where the patch
    // wires it, and an amount starts where the patch puts it.
    for (target, ids) in ROUTE_IDS.iter().enumerate() {
        for (source, (amount, presence)) in ids.iter().enumerate() {
            let declared = init.route(target, source);
            out.push(Param {
                id: presence,
                name: routes::presence_name(target, source),
                group: routes::GROUP,
                kind: Kind::Switch(Switch {
                    default: declared.present,
                    reading: SwitchReading::Presence,
                }),
            });
            let fader = routes::fader(target, source);
            out.push(Param {
                id: amount,
                name: routes::amount_name(target, source),
                group: routes::GROUP,
                kind: Kind::Float(Float {
                    range: fader.range(),
                    step: None,
                    default: declared.amount,
                    unit: "",
                    smoothing: Smoothing::Linear(20.0),
                    reading: Reading::Route(routes::reach(target, source), fader),
                }),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::DEFAULT_SOURCE_RATE;

    /// **Every id is unique**, and every route pair has both of its ids.
    #[test]
    fn every_id_is_unique() {
        let mut ids: Vec<&str> = all().iter().map(|param| param.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "an id is declared twice");
        assert_eq!(count, 2 * 22 + 24 + 2 * TARGETS * SOURCES);
    }

    /// **Every parameter's default reads back as itself** through its own text.
    #[test]
    fn every_default_survives_its_own_text() {
        for param in all() {
            let normalized = param.default_normalized();
            let text = param.text(normalized, DEFAULT_SOURCE_RATE);
            let back = param
                .parse(&text, DEFAULT_SOURCE_RATE)
                .unwrap_or_else(|| panic!("{}: {text:?} does not parse", param.id));
            assert_eq!(
                param.text(back, DEFAULT_SOURCE_RATE),
                text,
                "{}: {text:?} came back different",
                param.id
            );
        }
    }

    /// **A tempo sync names a switch and a rate the table holds.**
    #[test]
    fn every_tempo_sync_names_its_switch_and_rate() {
        for sync in TEMPO_SYNCS {
            assert!(matches!(
                get(sync.switch).map(|p| &p.kind),
                Some(Kind::Switch(_))
            ));
            assert!(matches!(
                get(sync.rate).map(|p| &p.kind),
                Some(Kind::Float(_))
            ));
        }
    }
}
