//! Provisional host parameters for the pre-listening `mxm-creative-sampler` slice.

use crate::asset::AssetField;
use mxm_creative_sampler_dsp::{FilterMode, LayerParams, LoopMode, Reader, VoiceMode};
use mxm_preset::PresetIdentity;
use nice_plug::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

/// How the one sample-dependent converter stage reads a value, and reads one back.
///
/// Owned closures rather than `fn` pointers because both ends have to consult the loaded sample's
/// rate; aliased because the pair written out in full is past what clippy will read as one type.
type StageReading = Box<dyn Fn(f32) -> String + Send + Sync>;
type StageParser = Box<dyn Fn(&str) -> Option<f32> + Send + Sync>;

/// The source rate the converter's readout is quoted against, in hertz.
///
/// **The loaded sample's rate, never the host's.** A patch has to sound the same at 44.1 and 96 kHz,
/// and it does: the converter's grid is counted in *source frames*, so its rate works out to
/// `pitch_ratio × source_rate / grid` and the host rate cancels out of the arithmetic entirely.
/// Quoting the readout against the host rate would therefore be showing a number the audio does not
/// depend on — the one way this display could lie about a preset.
///
/// It is an atomic rather than a read of [`AssetField`] because the formatter is a closure built in
/// `Default::default()`, before the field it would need to borrow exists. 48 kHz is the rate every
/// generated source is built at, so the value is right before anything is loaded — it read 44.1 kHz
/// until 2026-09-13, which was the one thing this display exists to stop it doing.
pub const DEFAULT_SOURCE_RATE: u32 = 48_000;

/// `text` without the sign of a negative zero.
///
/// `{:.N}` prints `-0` or `-0.00` for a value a hair below zero. A host parses that to zero and
/// prints it unsigned, so the reading would not survive the host's round trip — and
/// `clap-validator`'s `param-conversions` fails whenever its grid lands in the sliver. Only a
/// reading that shows zero changes; `mxm_modulation_params::signed` is the same rule for a reading
/// that carries a `+`.
fn unsigned_zero(text: String) -> String {
    match text.strip_prefix('-') {
        Some(digits) if digits.bytes().all(|b| b == b'0' || b == b'.') => digits.to_owned(),
        _ => text,
    }
}

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

/// Which converter the voice's DAC is. See [`mxm_creative_sampler_dsp::ConverterType`].
///
/// **Named for the machines, not for the mathematics.** `docs/oscillators/14-samplers.md` §14.1: the
/// vintage DAC names are companding parts rather than resolutions, and the card already reads in the
/// numbers those machines were sold on.
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

impl Default for MxmCreativeSamplerParams {
    fn default() -> Self {
        // **One declaration owns the whole Init sound.** These constructors still define each
        // parameter's range and display law, but every default value comes from the same protected
        // patch the generated source and routing parameters read.
        let init = &crate::init::PATCH;
        let a = init.layers[0];
        let b = init.layers[1];

        // **A percentage carries its unit** — design system §7, *"units are part of the formatted
        // value"*. Every one of these used to render as a bare integer, so Master read `80`.
        let percent = |name: &str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_unit("%")
                .with_value_to_string(formatters::v2s_f32_percentage(0))
                .with_string_to_value(formatters::s2v_f32_percentage())
        };
        // Skewed: the slow end is where a drift lives and the fast end is an effect.
        let lfo_rate = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Skewed {
                    min: 0.01,
                    max: 40.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(30.0))
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(2))
        };
        let bipolar = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            // A bipolar percentage carries its unit too, and its sign: design system §8.5 wants a
            // centre reference and an explicit signed value on a bipolar depth. **Never a negative
            // zero**: `v2s_f32_percentage` printed `-0` a hair left of centre, which parses to zero
            // and prints `0`.
            .with_unit("%")
            .with_value_to_string(Arc::new(|value| {
                unsigned_zero(format!("{:.0}", value * 100.0))
            }))
            .with_string_to_value(formatters::s2v_f32_percentage())
        };
        let root = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Linear {
                    min: 0.0,
                    max: 127.0,
                },
            )
            // **Fractional** (owner, 2026-09-14, D6): a loop that plays between notes is tuned to the
            // cent, which a whole-note step could not hold. The editor's drag still steps by whole
            // notes; see `sections::all_parameters`.
            .with_value_to_string(Arc::new(|value| {
                // Whole notes read exactly as they always have; cents appear only off a note.
                //
                // **Everything is read off one rounding, the whole cents**: the note, the cents
                // beside it and the number after the dot. The note was rounded from the raw value
                // and the cents and number separately, so 59.496 read `B3 +50 ct` and parsed to
                // 59.5, which reads `C4 −50 ct`; and 0.005 read `+1 ct · 0.00` and came back
                // `+1 ct · 0.01`. A host's round trip changed the text both times.
                let total = (value * 100.0).round();
                let note = (total / 100.0).round();
                let cents = total - note * 100.0;
                if cents == 0.0 {
                    format!("{} · {:.0}", note_name(note), note)
                } else {
                    let sign = if cents < 0.0 { '\u{2212}' } else { '+' };
                    format!(
                        "{} {sign}{:.0} ct · {:.2}",
                        note_name(note),
                        cents.abs(),
                        total / 100.0
                    )
                }
            }))
            .with_string_to_value(Arc::new(|text| {
                let text = text.trim();
                // The formatter writes `C3 \u{b7} 48` or `C4 −25 ct \u{b7} 59.75`, so a typed note
                // name — with its cents, if it has any — arrives with the number after it: take the
                // name first, then the number after the dot, then a bare number.
                let mut parts = text.split('\u{b7}');
                if let Some(note) = parts.next().and_then(parse_note_with_cents) {
                    return Some(note);
                }
                if let Some(number) = parts
                    .next()
                    .and_then(|rest| rest.trim().parse::<f32>().ok())
                {
                    return Some(number);
                }
                text.trim_start_matches("MIDI").trim().parse::<f32>().ok()
            }))
        };

        /// A note name with optional cents after it — `C4`, `C4 −25 ct`, `c#3 +10ct` — as MIDI.
        fn parse_note_with_cents(text: &str) -> Option<f32> {
            let text = text.trim();
            let (name, cents) = match text.find(char::is_whitespace) {
                Some(at) => (&text[..at], text[at..].trim()),
                None => (text, ""),
            };
            let note = parse_note_name(name)?;
            if cents.is_empty() {
                return Some(note);
            }
            let cents: f32 = cents
                .trim_end_matches("ct")
                .trim()
                .replace('\u{2212}', "-")
                .parse()
                .ok()?;
            let value = note + cents / 100.0;
            (0.0..=127.0).contains(&value).then_some(value)
        }
        /// A MIDI number as a note name, **C4 = 60**.
        ///
        /// The collection currently disagrees with itself: `mxm-poly-06` uses this convention and
        /// `mxm-player`'s sequencer uses one an octave below, where 60 is C3. This follows poly-06, which
        /// is the one that makes the generated init source — MIDI 48, and audibly a C — read as C3.
        fn note_name(value: f32) -> String {
            const NAMES: [&str; 12] = [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ];
            let note = value.round().clamp(0.0, 127.0) as i32;
            format!("{}{}", NAMES[(note % 12) as usize], note / 12 - 1)
        }

        /// The inverse, so a note name can be typed into the field that shows one.
        fn parse_note_name(text: &str) -> Option<f32> {
            const NAMES: [&str; 12] = [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ];
            let text = text.trim();
            let split = text.find(|c: char| c.is_ascii_digit() || c == '-')?;
            let (name, octave) = text.split_at(split);
            let name = name.trim().to_ascii_uppercase();
            let index = NAMES.iter().position(|n| *n == name)?;
            let octave: i32 = octave.trim().parse().ok()?;
            let note = (octave + 1) * 12 + index as i32;
            (0..=127).contains(&note).then_some(note as f32)
        }

        let tune = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Linear {
                    min: -24.0,
                    max: 24.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" st")
            .with_value_to_string(formatters::v2s_f32_rounded(2))
        };
        let direction = |name: &str, default: bool| {
            // **A formatter needs its reader.** `param-conversions` round-trips every value through
            // `value_to_string` and back: without a parser the default reader answers 0.0 for
            // "Reverse", so the label came back as "Forward" and the validator failed the plugin.
            // `mxm-grain-fx` met exactly this with Freeze and recorded it under *Validator quirks
            // this plugin met*; it was live here the whole time because nothing ran the validator
            // over this bundle after the parameter landed.
            BoolParam::new(name, default)
                .with_value_to_string(Arc::new(|reverse| {
                    if reverse { "Reverse" } else { "Forward" }.to_owned()
                }))
                .with_string_to_value(Arc::new(|text| {
                    match text.trim().to_ascii_lowercase().as_str() {
                        "reverse" | "true" | "on" | "1" => Some(true),
                        "forward" | "false" | "off" | "0" => Some(false),
                        _ => None,
                    }
                }))
        };
        let travel = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Linear {
                    min: -2.0,
                    max: 2.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            // A rate against the source's own: 1x plays it at its recorded speed, 0 holds it, and
            // a negative one runs it backwards. It was the last value on the surface reading as a
            // bare number. Never a negative zero, which a host round-trips to `0.00x`.
            .with_value_to_string(Arc::new(|rate| {
                format!("{}x", unsigned_zero(format!("{rate:.2}")))
            }))
            .with_string_to_value(Arc::new(|text| {
                text.trim().trim_end_matches(['x', 'X']).trim().parse().ok()
            }))
        };
        let grain_size = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Skewed {
                    min: 0.008,
                    max: 1.0,
                    factor: FloatRange::skew_factor(-1.8),
                },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3))
        };
        let grain_rate = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Skewed {
                    min: 0.5,
                    max: mxm_creative_sampler_dsp::MAX_GRAIN_DENSITY_HZ,
                    factor: FloatRange::skew_factor(-1.4),
                },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_unit(" /s")
            .with_value_to_string(formatters::v2s_f32_rounded(1))
        };
        // Skewed hard toward the bottom: a section of players is a few cents apart, and the top of
        // this range is a chord. Without the skew the musical part of the control is the first
        // pixel.
        let spread = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Skewed {
                    min: 0.0,
                    max: mxm_creative_sampler_dsp::MAX_GRAIN_SPREAD_ST,
                    factor: FloatRange::skew_factor(-2.2),
                },
            )
            .with_smoother(SmoothingStyle::Linear(30.0))
            .with_unit(" st")
            .with_value_to_string(formatters::v2s_f32_rounded(2))
        };
        // **A converter stage reads as the number the machine was sold on**, not as a percentage of
        // a knob. Eight bits and twenty kilohertz are what a Fairlight or an Emulator is remembered
        // by, and design system §7 already asks for the unit to be part of the value. The laws live
        // in the DSP and are called here, so the panel cannot drift from what the audio does.
        // **The window is a tone control, so it reads as one.** The three landmarks are named and
        // everything between them is a percentage, exactly as `mxm-grain-fx` does it: "Hard" is a
        // near-boxcar and audibly clicky, "Triangle" is the midpoint, "Smooth" is an exact Hann.
        // A bare "62%" says nothing about how a grain sounds; "Triangle" does.
        let grain_shape = |name: &str, default: f32| {
            FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(20.0))
                .with_value_to_string(Arc::new(|v: f32| match (v * 100.0).round() as i32 {
                    0 => "Hard".to_owned(),
                    50 => "Triangle".to_owned(),
                    100 => "Smooth".to_owned(),
                    p => format!("{p}%"),
                }))
                .with_string_to_value(Arc::new(|text: &str| {
                    match text.trim().to_ascii_lowercase().as_str() {
                        "hard" => Some(0.0),
                        "triangle" => Some(0.5),
                        "smooth" => Some(1.0),
                        other => other
                            .trim_end_matches('%')
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .map(|v| v / 100.0),
                    }
                }))
        };
        // **A reading and its parser are one thing, so the builder takes both.** These four stages
        // shipped with formatters and no readers, and a `FloatParam` without `string_to_value`
        // falls back to "strip `with_unit`, then `parse`" — which none of these four outputs
        // survive, because their units are part of a hand-written `format!`. `param-conversions`
        // fails the whole plugin on that, and it did. The earlier signature made the omission easy:
        // it had nowhere to put a parser, so leaving one out did not look like leaving anything
        // out. The parsers below invert the published `character_*` laws, and
        // `every_parameter_round_trips_its_own_text` holds each against its own formatter, so a law
        // that changes without its inverse fails a test rather than a validator run nobody made.
        let converter_stage =
            |name: &str, default: f32, show: fn(f32) -> String, read: fn(&str) -> Option<f32>| {
                FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                    .with_smoother(SmoothingStyle::Linear(20.0))
                    .with_value_to_string(Arc::new(show))
                    .with_string_to_value(Arc::new(read))
            };
        // The same, for a stage whose reading depends on the loaded sample and so needs closures
        // that own something rather than plain fn pointers.
        let converter_stage_shared =
            |name: &str, default: f32, show: StageReading, read: StageParser| {
                FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
                    .with_smoother(SmoothingStyle::Linear(20.0))
                    .with_value_to_string(Arc::from(show))
                    .with_string_to_value(Arc::from(read))
            };
        let source_rate = Arc::new(AtomicU32::new(DEFAULT_SOURCE_RATE));
        // **A loop crossfade reads in the unit a fade is heard in**: milliseconds where a seam is being
        // smoothed, seconds where a loop is being blurred into itself. The branch is decided on the
        // rounded millisecond reading, so the two units partition the travel and every reading has one
        // value behind it — the rule Bit smoothing's formatter had to learn. Smoothed like the loop
        // points, because it moves the same geometry they do, the return point included.
        let crossfade = |name: &str, default: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Skewed {
                    min: 0.0,
                    max: mxm_creative_sampler_dsp::MAX_LOOP_CROSSFADE_S,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_value_to_string(Arc::new(|seconds: f32| {
                let ms = (seconds * 10_000.0).round() / 10.0;
                if ms < 1_000.0 {
                    format!("{ms:.1} ms")
                } else {
                    format!("{seconds:.2} s")
                }
            }))
            .with_string_to_value(Arc::new(|text: &str| {
                let text = text.trim().to_ascii_lowercase();
                if let Some(ms) = text.strip_suffix("ms") {
                    return ms.trim().parse::<f32>().ok().map(|ms| ms / 1_000.0);
                }
                text.trim_end_matches('s').trim().parse::<f32>().ok()
            }))
        };
        let cutoff = FloatParam::new(
            "Cutoff",
            init.filter.cutoff_hz,
            FloatRange::Skewed {
                min: 20.0,
                max: 22_000.0,
                factor: FloatRange::skew_factor(-2.2),
            },
        )
        .with_smoother(SmoothingStyle::Logarithmic(30.0))
        // **2.7 kHz, not 2693.** The unit belongs in the value, and a four-digit number was the
        // half of the truncation defect that a wider knob column does not fix.
        //
        // **Kilohertz from the rounded tenth of a hertz, not from the raw value.** nice-plug's
        // `v2s_f32_hz_then_khz` switches at a raw 1000, so a cutoff a hair above it read `1.0 kHz`,
        // parsed to a thousand hertz whose normalized inverse lands just under it, and came back
        // `1000.0 Hz`. Deciding on the tenth the hertz branch prints makes every reading of a
        // thousand hertz `1.0 kHz`. The same readings as before everywhere else; the vendor parser,
        // which takes either unit, is kept.
        .with_value_to_string(Arc::new(|hz| {
            if (hz * 10.0).round() >= 10_000.0 {
                format!("{:.1} kHz", hz / 1_000.0)
            } else {
                format!("{hz:.1} Hz")
            }
        }))
        .with_string_to_value(formatters::s2v_f32_hz_then_khz());
        let seconds = |name: &str, default: f32, max: f32| {
            FloatParam::new(
                name,
                default,
                FloatRange::Skewed {
                    min: 0.000_05,
                    max,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s")
            .with_value_to_string(formatters::v2s_f32_rounded(3))
        };
        Self {
            a_reader: EnumParam::new("A Playback mode", a.reader),
            a_root: root("A Root", a.root),
            a_transpose: tune("A Transpose", a.transpose),
            a_start: percent("A Start", a.start),
            a_end: percent("A End", a.end),
            a_loop_start: percent("A Loop start", a.loop_start),
            a_loop_end: percent("A Loop end", a.loop_end),
            a_loop_crossfade: crossfade("A Loop crossfade", a.loop_crossfade_s),
            a_loop: EnumParam::new("A Loop", a.loop_mode),
            // **A direction reads as a direction everywhere**, not only on the panel. The segments
            // said Forward/Reverse while the parameter itself said Off/On, so an automation lane
            // and any host's generic list showed "Off" for a direction, which names nothing.
            a_reverse: direction("A Direction", a.reverse),
            a_level: percent("A Level", a.level),
            a_pan: bipolar("A Pan", a.pan),
            a_scan_speed: travel("A Scan speed", a.scan_speed),
            a_grain_size: grain_size("A Grain size", a.grain_size_s),
            a_grain_rate: grain_rate("A Grain rate", a.grain_rate_hz),
            a_grain_sync: BoolParam::new("A Grain rate sync", false),
            a_pitch_var: spread("A Pitch variation", a.pitch_variation_st),
            a_position_var: percent("A Position variation", a.position_variation),
            a_timing: percent("A Onset timing", a.onset_timing),
            a_width: percent("A Stereo width", a.stereo_width),
            a_reverse_chance: percent("A Reverse chance", a.reverse_chance),
            a_grain_shape: grain_shape("A Grain shape", a.grain_shape),
            b_reader: EnumParam::new("B Playback mode", b.reader),
            b_root: root("B Root", b.root),
            b_transpose: tune("B Transpose", b.transpose),
            b_start: percent("B Start", b.start),
            b_end: percent("B End", b.end),
            b_loop_start: percent("B Loop start", b.loop_start),
            b_loop_end: percent("B Loop end", b.loop_end),
            b_loop_crossfade: crossfade("B Loop crossfade", b.loop_crossfade_s),
            b_loop: EnumParam::new("B Loop", b.loop_mode),
            b_reverse: direction("B Direction", b.reverse),
            b_level: percent("B Level", b.level),
            b_pan: bipolar("B Pan", b.pan),
            b_scan_speed: travel("B Scan speed", b.scan_speed),
            b_grain_size: grain_size("B Grain size", b.grain_size_s),
            b_grain_rate: grain_rate("B Grain rate", b.grain_rate_hz),
            b_grain_sync: BoolParam::new("B Grain rate sync", false),
            b_pitch_var: spread("B Pitch variation", b.pitch_variation_st),
            b_position_var: percent("B Position variation", b.position_variation),
            b_timing: percent("B Onset timing", b.onset_timing),
            b_width: percent("B Stereo width", b.stereo_width),
            b_reverse_chance: percent("B Reverse chance", b.reverse_chance),
            b_grain_shape: grain_shape("B Grain shape", b.grain_shape),
            // **"rate / 5.7" is a ratio, not a value.** A control named Sample rate has to read in
            // hertz — that is the number the machines were sold on and the only form a player can
            // act on. The divisor was shown because the formatter cannot see the loaded sample; the
            // shared `source_rate` is what fixes that, and it is the *sample's* rate, so the
            // reading is identical at every host rate. See [`DEFAULT_SOURCE_RATE`].
            rate: {
                let shown = Arc::clone(&source_rate);
                let parsed = Arc::clone(&source_rate);
                converter_stage_shared(
                    "Sample rate",
                    init.character.sample_rate,
                    Box::new(move |v| {
                        let divisor = mxm_creative_sampler_dsp::character_rate_divisor(v);
                        let hz = shown.load(Ordering::Relaxed) as f32 / divisor;
                        // **Each unit is chosen from the rounding the finer one prints**, never
                        // from the raw hertz: at `hz >= 10_000.0`, 9997 Hz read `10.00 kHz`,
                        // parsed to ten thousand and came back `10.0 kHz`, and at `hz >= 1_000.0`
                        // 999.6 Hz read `1000 Hz` and came back `1.00 kHz`.
                        if (hz / 10.0).round() >= 1_000.0 {
                            format!("{:.1} kHz", hz / 1000.0)
                        } else if hz.round() >= 1_000.0 {
                            format!("{:.2} kHz", hz / 1000.0)
                        } else {
                            format!("{hz:.0} Hz")
                        }
                    }),
                    // `divisor = 2^(5v)`, so `v = log2(source / hz) / 5`. **Read against the same
                    // loaded rate the formatter used**, which is why this one needs the handle: a
                    // reading of "24.0 kHz" means half speed on a 48 kHz source and full speed on a
                    // 24 kHz one, and answering with the wrong one would move the sound.
                    Box::new(move |text: &str| {
                        let text = text.trim().to_ascii_lowercase();
                        let (number, scale) = match text.strip_suffix("khz") {
                            Some(rest) => (rest, 1000.0),
                            None => (text.trim_end_matches("hz"), 1.0),
                        };
                        let hz = number.trim().parse::<f32>().ok()? * scale;
                        if hz <= 0.0 {
                            return None;
                        }
                        let source = parsed.load(Ordering::Relaxed) as f32;
                        Some(((source / hz).max(1.0).log2() / 5.0).clamp(0.0, 1.0))
                    }),
                )
            },
            converter: converter_stage(
                "Bit depth",
                init.character.bit_depth,
                |v| format!("{:.0} bits", mxm_creative_sampler_dsp::character_bits(v)),
                // `bits = 16 - 11*sqrt(c)`, so `c = ((16 - bits) / 11)^2`.
                |text| {
                    let bits = text
                        .trim()
                        .trim_end_matches("bits")
                        .trim_end_matches("bit")
                        .trim()
                        .parse::<f32>()
                        .ok()?;
                    Some(((16.0 - bits) / 11.0).clamp(0.0, 1.0).powi(2))
                },
            ),
            // **"45% hold" names the mechanism, not the sound.** A converter has to decide what the
            // output does *between* two stored frames: slide smoothly to the next one, or sit on
            // the old one and jump. Sitting and jumping is what a cheap output stage did, and the
            // steps it leaves are extra high content that was never in the recording — the buzz on
            // top. So the ends are named for what you hear, and the middle says which way it is
            // leaning rather than quoting a percentage of a word nobody outside this file knows.
            // **The two words and the percentages partition the travel rather than overlapping
            // it.** The thresholds used to be 0.001 and 0.999, which left 0.002 reading "0%
            // stepped" - a value whose own text parses back to something that reads "smooth". One
            // integer percent decides all three branches now, so every reading has exactly one
            // value behind it. It is also the shape the dither builder above already uses.
            reconstruction: converter_stage(
                "Bit smoothing",
                init.character.bit_smoothing,
                |v| match (v.clamp(0.0, 1.0) * 100.0).round() as i32 {
                    0 => "smooth".to_owned(),
                    100 => "stepped".to_owned(),
                    percent => format!("{percent}% stepped"),
                },
                |text| match text.trim().to_ascii_lowercase().as_str() {
                    "smooth" => Some(0.0),
                    "stepped" => Some(1.0),
                    other => other
                        .trim_end_matches("stepped")
                        .trim()
                        .trim_end_matches('%')
                        .trim()
                        .parse::<f32>()
                        .ok()
                        .map(|percent| (percent / 100.0).clamp(0.0, 1.0)),
                },
            ),
            input: converter_stage(
                "Converter drive",
                init.character.converter_drive,
                |v| format!("{:+.1} dB", mxm_creative_sampler_dsp::character_drive_db(v)),
                // `dB = 20*log10(1 + 15i)`, so `i = (10^(dB/20) - 1) / 15`. The sign the formatter
                // prints is read back by `f32::from_str`, which accepts a leading `+`.
                |text| {
                    let db = text
                        .trim()
                        .trim_end_matches("dB")
                        .trim_end_matches("db")
                        .trim()
                        .parse::<f32>()
                        .ok()?;
                    Some(((10.0f32.powf(db / 20.0) - 1.0) / 15.0).clamp(0.0, 1.0))
                },
            ),
            converter_type: EnumParam::new("Converter type", init.character.converter_type),
            jitter: percent("Jitter", init.character.jitter),
            cutoff,
            resonance: percent("Resonance", init.filter.resonance),
            filter_mode: EnumParam::new("Filter mode", init.filter.mode),
            drive: percent("Drive", init.filter.drive),
            attack: seconds("Attack", init.envelope.attack_s, 10.0),
            decay: seconds("Decay", init.envelope.decay_s, 10.0),
            sustain: percent("Sustain", init.envelope.sustain),
            release: seconds("Release", init.envelope.release_s, 20.0),
            lfo_rate: lfo_rate("LFO 1 rate", init.lfos[0].rate_hz),
            lfo1_sync: BoolParam::new("LFO 1 sync", false),
            lfo_shape: EnumParam::new("LFO 1 shape", init.lfos[0].shape),
            // **A different rate and shape from LFO 1**, because two identical LFOs are one LFO
            // until somebody changes something, and the first thing a player does with a second
            // modulator is check that it is not the first one.
            lfo2_rate: lfo_rate("LFO 2 rate", init.lfos[1].rate_hz),
            lfo2_sync: BoolParam::new("LFO 2 sync", false),
            lfo2_shape: EnumParam::new("LFO 2 shape", init.lfos[1].shape),
            master: percent("Master", init.performance.master),
            voice_mode: EnumParam::new("Voices", init.performance.voices),
            // Glide is the time to cover an octave, not the time between any two keys: an interval
            // measured in seconds-per-note makes a wide leap arrive at the same moment as a
            // semitone, which is not what a player hears as portamento.
            glide: FloatParam::new(
                "Glide",
                init.performance.glide_s_per_octave,
                FloatRange::Skewed {
                    min: 0.0,
                    max: 5.0,
                    factor: FloatRange::skew_factor(-2.0),
                },
            )
            .with_unit(" s/oct")
            .with_value_to_string(formatters::v2s_f32_rounded(3)),
            bend_range: FloatParam::new(
                "Bend range",
                init.performance.bend_range_st,
                FloatRange::Linear {
                    min: 0.0,
                    max: 24.0,
                },
            )
            .with_step_size(1.0)
            // Smoothed because it scales a held bend into every voice's pitch: a range edit under a
            // held bend would otherwise be a pitch step (`docs/code-review-notes.md` §2). The 20 ms
            // matches `mxm-mono-02` and `mxm-mono-08`.
            .with_smoother(SmoothingStyle::Linear(20.0))
            .with_unit(" st")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),
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

    pub fn layer(&self, index: usize) -> LayerParams {
        if index == 0 {
            LayerParams {
                reader: self.a_reader.value().dsp(),
                root_note: self.a_root.smoothed.next(),
                tune_semitones: self.a_transpose.smoothed.next(),
                start: self.a_start.smoothed.next(),
                end: self.a_end.smoothed.next(),
                loop_start: self.a_loop_start.smoothed.next(),
                loop_end: self.a_loop_end.smoothed.next(),
                loop_crossfade_s: self.a_loop_crossfade.smoothed.next(),
                loop_mode: self.a_loop.value().dsp(),
                reverse: self.a_reverse.value(),
                level: self.a_level.smoothed.next(),
                pan: self.a_pan.smoothed.next(),
                stretch_travel: self.a_scan_speed.smoothed.next(),
                grain_size_s: self.a_grain_size.smoothed.next(),
                grain_density_hz: self.a_grain_rate.smoothed.next(),
                grain_position_variation: self.a_position_var.smoothed.next(),
                grain_onset_timing: self.a_timing.smoothed.next(),
                grain_stereo_width: self.a_width.smoothed.next(),
                grain_reverse_chance: self.a_reverse_chance.smoothed.next(),
                grain_shape: self.a_grain_shape.smoothed.next(),
                grain_spread_semitones: self.a_pitch_var.smoothed.next(),
            }
        } else {
            LayerParams {
                reader: self.b_reader.value().dsp(),
                root_note: self.b_root.smoothed.next(),
                tune_semitones: self.b_transpose.smoothed.next(),
                start: self.b_start.smoothed.next(),
                end: self.b_end.smoothed.next(),
                loop_start: self.b_loop_start.smoothed.next(),
                loop_end: self.b_loop_end.smoothed.next(),
                loop_crossfade_s: self.b_loop_crossfade.smoothed.next(),
                loop_mode: self.b_loop.value().dsp(),
                reverse: self.b_reverse.value(),
                level: self.b_level.smoothed.next(),
                pan: self.b_pan.smoothed.next(),
                stretch_travel: self.b_scan_speed.smoothed.next(),
                grain_size_s: self.b_grain_size.smoothed.next(),
                grain_density_hz: self.b_grain_rate.smoothed.next(),
                grain_position_variation: self.b_position_var.smoothed.next(),
                grain_onset_timing: self.b_timing.smoothed.next(),
                grain_stereo_width: self.b_width.smoothed.next(),
                grain_reverse_chance: self.b_reverse_chance.smoothed.next(),
                grain_shape: self.b_grain_shape.smoothed.next(),
                grain_spread_semitones: self.b_pitch_var.smoothed.next(),
            }
        }
    }
}

impl MxmCreativeSamplerParams {
    /// LFO 1's rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`LFO_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_lfo1_rate(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.lfo_rate;
        LFO_SYNC
            .resolve(
                self.lfo1_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|hz| hz as f32)
    }

    /// LFO 2's rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`LFO_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_lfo2_rate(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.lfo2_rate;
        LFO_SYNC
            .resolve(
                self.lfo2_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|hz| hz as f32)
    }

    /// A's grain rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`GRAIN_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_a_grain_rate(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.a_grain_rate;
        GRAIN_SYNC
            .resolve(
                self.a_grain_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|hz| hz as f32)
    }

    /// B's grain rate while its sync follows the host, or `None` for its free value: the modulated
    /// position picks a division on [`GRAIN_SYNC`]. Resolved once a buffer by the plugin.
    pub fn synced_b_grain_rate(&self, tempo: Option<f64>) -> Option<f32> {
        let param = &self.b_grain_rate;
        GRAIN_SYNC
            .resolve(
                self.b_grain_sync.value(),
                tempo,
                param.modulated_normalized_value(),
                f64::from(param.preview_plain(0.0)),
                f64::from(param.preview_plain(1.0)),
            )
            .map(|hz| hz as f32)
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
            init.layers[0].grain_size_s,
            "grain size",
        );
        close(
            params.a_grain_rate.default_plain_value(),
            init.layers[0].grain_rate_hz,
            "grain rate",
        );
        close(
            params.a_pitch_var.default_plain_value(),
            init.layers[0].pitch_variation_st,
            "pitch variation",
        );
        close(
            params.a_position_var.default_plain_value(),
            init.layers[0].position_variation,
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
            init.filter.cutoff_hz,
            "cutoff",
        );
        close(params.resonance.default_plain_value(), 0.38, "resonance");
        close(
            params.lfo2_rate.default_plain_value(),
            init.lfos[1].rate_hz,
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

    /// One trip through the host, as `vendor/nice-plug`'s CLAP wrapper makes it: the CLAP value is
    /// the normalized value times the step count, the text carries the unit, and the parsed text
    /// comes back through the parameter's normalized conversion before it is formatted again.
    /// Returns the failure, if the text changed or did not parse.
    ///
    /// # Safety
    ///
    /// `ptr` must point at a parameter that outlives the call.
    unsafe fn host_round_trip(id: &str, ptr: ParamPtr, clap_value: f64) -> Option<String> {
        unsafe {
            let steps = ptr.step_count().unwrap_or(1);
            let normalised = clap_value as f32 / steps as f32;
            let first = ptr.normalized_value_to_string(normalised, true);
            let Some(parsed) = ptr.string_to_normalized_value(&first) else {
                return Some(format!("{id}: {first:?} does not parse"));
            };
            let back = parsed as f64 * steps as f64;
            let second = ptr.normalized_value_to_string(back as f32 / steps as f32, true);
            (second != first).then(|| {
                format!(
                    "{id} at plain {}: {first:?} came back {second:?}",
                    ptr.preview_plain(normalised)
                )
            })
        }
    }

    /// **Every parameter's text survives the host's normalized conversion unchanged** — not a list
    /// of the ones someone remembered.
    ///
    /// `mxm-grain-fx` pinned the same property over a hand-written list, and that shape is why a
    /// defect lived here: Direction shipped with a `value_to_string` and no reader, so
    /// `param-conversions` walked 0.5152 to "Reverse", back through the default reader to 0.0, and
    /// out again as "Forward". Walking [`Params::param_map`] asks the question of every parameter,
    /// the 110 routing pairs and every one added later included.
    ///
    /// **Branch points, not only a grid.** On the twenty-step grid alone this passed Cutoff and
    /// Sample rate switching unit at a raw boundary — a reading of `1.0 kHz` parses to a thousand
    /// hertz whose normalized inverse lands under it — Root changing its note name at a raw half
    /// note, and Pan and Scan speed printing a negative zero a hair below zero. A validator run
    /// checks a grid of its own, so a clean run proves nothing about a sliver between its points.
    ///
    /// Probes, for every parameter and at three source rates (Sample rate's reading depends on the
    /// loaded sample's): the twenty-step grid; `clap-validator` 0.4.1's own grid, whose size follows
    /// the parameter count; both sides of every branch point in [`branch_plains`]; and a hair
    /// either side of zero on a range that crosses it.
    #[test]
    fn every_parameter_round_trips_its_own_text() {
        let params = MxmCreativeSamplerParams::default();
        let map = params.param_map();
        let validator_points = 4000usize.div_ceil(map.len()).clamp(5, 100);
        let mut failures = Vec::new();
        for source_rate in [DEFAULT_SOURCE_RATE, 44_100, 22_050] {
            params.source_rate.store(source_rate, Ordering::Relaxed);
            for (id, ptr, _) in &map {
                // SAFETY: `params` owns every parameter these pointers refer to and outlives the
                // loop; this is the same access `param-conversions` makes through CLAP.
                unsafe {
                    let steps = ptr.step_count().unwrap_or(1) as f64;
                    let mut probes: Vec<f64> = (0..=19).map(|i| steps * i as f64 / 19.0).collect();
                    probes.extend(
                        (0..validator_points)
                            .map(|i| steps * (i as f64 / (validator_points - 1) as f64)),
                    );
                    let mut plains = branch_plains(id, source_rate as f32);
                    let (low, high) = (ptr.preview_plain(0.0), ptr.preview_plain(1.0));
                    if low.min(high) < 0.0 && low.max(high) > 0.0 {
                        plains.extend([-1.0e-3, -1.0e-4, 0.0, 1.0e-4, 1.0e-3]);
                    }
                    probes.extend(
                        plains
                            .into_iter()
                            .map(|plain| ptr.preview_normalized(plain) as f64 * steps),
                    );
                    failures.extend(probes.into_iter().filter_map(|value| {
                        host_round_trip(id, *ptr, value).map(|f| format!("{source_rate} Hz: {f}"))
                    }));
                }
            }
        }
        // One line per parameter and source rate is enough to read; the count says the rest.
        let total = failures.len();
        failures.dedup_by(|b, a| a.split(" at plain").next() == b.split(" at plain").next());
        assert!(
            failures.is_empty(),
            "{total} failures, first per parameter:\n{}",
            failures.join("\n")
        );
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
}
