//! Every parameter's text, both ways: the reading it shows and the parser that reads what a person
//! types.
//!
//! **Character for character as the plugin shows it**, because the plugin's parameters take their
//! formatting from here (`plugins/mxm-creative-sampler/src/params.rs`). Three kinds of reading
//! meet here:
//!
//! - the plugin's own, moved from its `params.rs` with the reasons they read as they do;
//! - nice-plug's formatters this plugin used, `v2s_f32_percentage`, `s2v_f32_percentage`,
//!   `v2s_f32_rounded` and `s2v_f32_hz_then_khz`, ported from nice-plug-core 0.4.2's
//!   `src/formatters.rs` (ISC, Copyright (c) 2022-2024 Robbert van der Helm);
//! - a route amount's, which is mxm-kit's `mxm_modulation_params::reading` (MIT): the plugin builds
//!   those parameters with the kit's own function, and its tests hold this copy to it, so a change
//!   to the collection's route reading shows here the day the plugin is relocked.

use mxm_modulation::standard::Offer;

use crate::range::Range;

/// The source rate the converter's readout is quoted against when nothing is loaded, in hertz.
///
/// **The loaded sample's rate, never the host's.** A patch has to sound the same at 44.1 and 96 kHz,
/// and it does: the converter's grid is counted in *source frames*, so its rate works out to
/// `pitch_ratio × source_rate / grid` and the host rate cancels out of the arithmetic entirely.
/// Quoting the readout against the host rate would therefore be showing a number the audio does not
/// depend on — the one way this display could lie about a preset.
///
/// 48 kHz is the rate every generated source is built at, so the value is right before anything is
/// loaded — it read 44.1 kHz until 2026-09-13, which was the one thing this display exists to stop
/// it doing.
pub const DEFAULT_SOURCE_RATE: u32 = 48_000;

/// How a continuous parameter reads. Each shows its value without its unit, as nice-plug's
/// `value_to_string` does; [`crate::Param::format`] adds the unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reading {
    /// A whole percentage of `0..=1`, the unit `%`: nice-plug's `v2s_f32_percentage(0)` and
    /// `s2v_f32_percentage`. **A percentage carries its unit** — design system §7, *"units are
    /// part of the formatted value"*. Every one of these used to render as a bare integer, so
    /// Master read `80`.
    Percent,
    /// A whole percentage of `-1..=1`, the unit `%`, **never a negative zero**: design system §8.5
    /// wants a centre reference and an explicit signed value on a bipolar depth, and
    /// `v2s_f32_percentage` printed `-0` a hair left of centre, which parses to zero and prints `0`.
    BipolarPercent,
    /// nice-plug's `v2s_f32_rounded(places)`, read back by stripping the unit and parsing.
    Decimals(usize),
    /// A layer's Root: a note name and its MIDI number, with cents off a whole note.
    Root,
    /// Scan speed: a rate against the source's own, `1.00x` its recorded speed.
    Speed,
    /// Grain shape: `Hard`, `Triangle` and `Smooth` at its landmarks, a percentage between.
    GrainShape,
    /// A loop crossfade, in milliseconds below a second and seconds from one.
    Crossfade,
    /// The filter's cutoff, in hertz below a kilohertz and kilohertz from one.
    Cutoff,
    /// The converter's Sample rate, in hertz against the loaded sample's rate.
    SampleRate,
    /// The converter's Bit depth, in bits.
    BitDepth,
    /// The converter's Bit smoothing: `smooth`, `stepped`, or how stepped between.
    BitSmoothing,
    /// The converter's drive, in decibels.
    ConverterDrive,
    /// A route's amount: what its pair delivers, in the target's own unit.
    Route(Reach, Fader),
}

/// How a switch reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchReading {
    /// nice-plug's own: `On` and `Off`, and `true` or `on` read as on, anything else as off.
    OnOff,
    /// A layer's Direction: `Forward` and `Reverse`.
    Direction,
    /// A route's presence: `On` and `Off`, and `on`, `true`, `1` or `yes` read as on.
    Presence,
}

impl Reading {
    /// `plain` as this reading shows it, without its unit. `source_rate` is the loaded sample's
    /// rate, which only [`Reading::SampleRate`] reads.
    #[must_use]
    pub fn show(self, plain: f32, source_rate: u32) -> String {
        match self {
            Reading::Percent => format!("{:.0}", plain * 100.0),
            Reading::BipolarPercent => unsigned_zero(format!("{:.0}", plain * 100.0)),
            Reading::Decimals(digits) => rounded(plain, digits),
            Reading::Root => root_text(plain),
            // It was the last value on the surface reading as a bare number. Never a negative
            // zero, which a host round-trips to `0.00x`.
            Reading::Speed => format!("{}x", unsigned_zero(format!("{plain:.2}"))),
            Reading::GrainShape => match (plain * 100.0).round() as i32 {
                0 => "Hard".to_owned(),
                50 => "Triangle".to_owned(),
                100 => "Smooth".to_owned(),
                p => format!("{p}%"),
            },
            Reading::Crossfade => {
                let ms = (plain * 10_000.0).round() / 10.0;
                if ms < 1_000.0 {
                    format!("{ms:.1} ms")
                } else {
                    format!("{plain:.2} s")
                }
            }
            Reading::Cutoff => {
                if (plain * 10.0).round() >= 10_000.0 {
                    format!("{:.1} kHz", plain / 1_000.0)
                } else {
                    format!("{plain:.1} Hz")
                }
            }
            Reading::SampleRate => {
                let divisor = mxm_creative_sampler_dsp::character_rate_divisor(plain);
                let hz = source_rate as f32 / divisor;
                // **Each unit is chosen from the rounding the finer one prints**, never from the
                // raw hertz: at `hz >= 10_000.0`, 9997 Hz read `10.00 kHz`, parsed to ten
                // thousand and came back `10.0 kHz`, and at `hz >= 1_000.0` 999.6 Hz read
                // `1000 Hz` and came back `1.00 kHz`.
                if (hz / 10.0).round() >= 1_000.0 {
                    format!("{:.1} kHz", hz / 1000.0)
                } else if hz.round() >= 1_000.0 {
                    format!("{:.2} kHz", hz / 1000.0)
                } else {
                    format!("{hz:.0} Hz")
                }
            }
            Reading::BitDepth => {
                format!(
                    "{:.0} bits",
                    mxm_creative_sampler_dsp::character_bits(plain)
                )
            }
            Reading::BitSmoothing => match (plain.clamp(0.0, 1.0) * 100.0).round() as i32 {
                0 => "smooth".to_owned(),
                100 => "stepped".to_owned(),
                percent => format!("{percent}% stepped"),
            },
            Reading::ConverterDrive => format!(
                "{:+.1} dB",
                mxm_creative_sampler_dsp::character_drive_db(plain)
            ),
            Reading::Route(reach, _) => route_reading(reach, plain),
        }
    }

    /// What `text` means as a plain value, or `None` where it means nothing. `text` arrives trimmed,
    /// as nice-plug hands it over; `unit` is the parameter's, which [`Reading::Decimals`] strips.
    #[must_use]
    pub fn read(self, text: &str, unit: &str, source_rate: u32) -> Option<f32> {
        match self {
            Reading::Percent | Reading::BipolarPercent => text
                .trim_end_matches([' ', '%'])
                .parse()
                .ok()
                .map(|x: f32| x / 100.0),
            Reading::Decimals(_) => text.trim().trim_end_matches(unit).parse().ok(),
            Reading::Root => parse_root(text),
            Reading::Speed => text.trim().trim_end_matches(['x', 'X']).trim().parse().ok(),
            Reading::GrainShape => match text.trim().to_ascii_lowercase().as_str() {
                "hard" => Some(0.0),
                "triangle" => Some(0.5),
                "smooth" => Some(1.0),
                other => other
                    .trim_end_matches('%')
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .map(|v| v / 100.0),
            },
            Reading::Crossfade => {
                let text = text.trim().to_ascii_lowercase();
                if let Some(ms) = text.strip_suffix("ms") {
                    return ms.trim().parse::<f32>().ok().map(|ms| ms / 1_000.0);
                }
                text.trim_end_matches('s').trim().parse::<f32>().ok()
            }
            Reading::Cutoff => hz_then_khz(text),
            // `divisor = 2^(5v)`, so `v = log2(source / hz) / 5`. **Read against the same loaded
            // rate the reading used**: "24.0 kHz" means half speed on a 48 kHz source and full speed
            // on a 24 kHz one, and answering with the wrong one would move the sound.
            Reading::SampleRate => {
                let text = text.trim().to_ascii_lowercase();
                let (number, scale) = match text.strip_suffix("khz") {
                    Some(rest) => (rest, 1000.0),
                    None => (text.trim_end_matches("hz"), 1.0),
                };
                let hz = number.trim().parse::<f32>().ok()? * scale;
                if hz <= 0.0 {
                    return None;
                }
                let source = source_rate as f32;
                Some(((source / hz).max(1.0).log2() / 5.0).clamp(0.0, 1.0))
            }
            // `bits = 16 - 11*sqrt(c)`, so `c = ((16 - bits) / 11)^2`.
            Reading::BitDepth => {
                let bits = text
                    .trim()
                    .trim_end_matches("bits")
                    .trim_end_matches("bit")
                    .trim()
                    .parse::<f32>()
                    .ok()?;
                Some(((16.0 - bits) / 11.0).clamp(0.0, 1.0).powi(2))
            }
            Reading::BitSmoothing => match text.trim().to_ascii_lowercase().as_str() {
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
            // `dB = 20*log10(1 + 15i)`, so `i = (10^(dB/20) - 1) / 15`. The sign the reading prints
            // is read back by `f32::from_str`, which accepts a leading `+`.
            Reading::ConverterDrive => {
                let db = text
                    .trim()
                    .trim_end_matches("dB")
                    .trim_end_matches("db")
                    .trim()
                    .parse::<f32>()
                    .ok()?;
                Some(((10.0f32.powf(db / 20.0) - 1.0) / 15.0).clamp(0.0, 1.0))
            }
            Reading::Route(reach, fader) => route_parse(reach, fader, text),
        }
    }
}

impl SwitchReading {
    /// `on` as this reading shows it.
    #[must_use]
    pub fn show(self, on: bool) -> String {
        match self {
            SwitchReading::OnOff | SwitchReading::Presence => {
                if on {
                    "On"
                } else {
                    "Off"
                }
            }
            // **A direction reads as a direction everywhere**, not only on the panel: an
            // automation lane and any host's generic list showed "Off" for a direction, which
            // names nothing.
            SwitchReading::Direction => {
                if on {
                    "Reverse"
                } else {
                    "Forward"
                }
            }
        }
        .to_owned()
    }

    /// What `text` means, or `None` where it means nothing. `text` arrives trimmed.
    #[must_use]
    pub fn read(self, text: &str) -> Option<bool> {
        match self {
            SwitchReading::OnOff => {
                Some(text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("on"))
            }
            // **A reading needs its reader.** Without one the default answered off for "Reverse",
            // so the label came back as "Forward" and `clap-validator` failed the plugin.
            SwitchReading::Direction => match text.trim().to_ascii_lowercase().as_str() {
                "reverse" | "true" | "on" | "1" => Some(true),
                "forward" | "false" | "off" | "0" => Some(false),
                _ => None,
            },
            SwitchReading::Presence => Some(matches!(
                text.trim().to_ascii_lowercase().as_str(),
                "on" | "true" | "1" | "yes"
            )),
        }
    }
}

/// `text` without the sign of a negative zero.
///
/// `{:.N}` prints `-0` or `-0.00` for a value a hair below zero. A host parses that to zero and
/// prints it unsigned, so the reading would not survive the host's round trip — and
/// `clap-validator`'s `param-conversions` fails whenever its grid lands in the sliver. Only a
/// reading that shows zero changes; [`signed`] is the same rule for a reading that carries a `+`.
#[must_use]
pub fn unsigned_zero(text: String) -> String {
    match text.strip_prefix('-') {
        Some(digits) if digits.bytes().all(|b| b == b'0' || b == b'.') => digits.to_owned(),
        _ => text,
    }
}

/// nice-plug's `v2s_f32_rounded`: `digits` decimals, and never a negative zero.
fn rounded(value: f32, digits: usize) -> String {
    let rounding_multiplier = 10u32.pow(digits as u32) as f32;
    if (value * rounding_multiplier).round() / rounding_multiplier == 0.0 {
        format!("{:.digits$}", 0.0)
    } else {
        format!("{value:.digits$}")
    }
}

/// Root's reading. **Fractional** (owner, 2026-09-14, D6): a loop that plays between notes is tuned
/// to the cent, which a whole-note step could not hold.
fn root_text(value: f32) -> String {
    // Whole notes read exactly as they always have; cents appear only off a note.
    //
    // **Everything is read off one rounding, the whole cents**: the note, the cents beside it and
    // the number after the dot. The note was rounded from the raw value and the cents and number
    // separately, so 59.496 read `B3 +50 ct` and parsed to 59.5, which reads `C4 −50 ct`; and 0.005
    // read `+1 ct · 0.00` and came back `+1 ct · 0.01`. A host's round trip changed the text both
    // times.
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
}

/// Root's parser. The reading writes `C3 \u{b7} 48` or `C4 −25 ct \u{b7} 59.75`, so a typed note
/// name — with its cents, if it has any — arrives with the number after it: take the name first,
/// then the number after the dot, then a bare number.
fn parse_root(text: &str) -> Option<f32> {
    let text = text.trim();
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
}

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

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A MIDI number as a note name, **C4 = 60**.
///
/// The collection currently disagrees with itself: `mxm-poly-06` uses this convention and
/// `mxm-player`'s sequencer uses one an octave below, where 60 is C3. This follows poly-06, which
/// is the one that makes the generated init source — MIDI 48, and audibly a C — read as C3.
fn note_name(value: f32) -> String {
    let note = value.round().clamp(0.0, 127.0) as i32;
    format!("{}{}", NOTE_NAMES[(note % 12) as usize], note / 12 - 1)
}

/// The inverse, so a note name can be typed into the field that shows one.
fn parse_note_name(text: &str) -> Option<f32> {
    let text = text.trim();
    let split = text.find(|c: char| c.is_ascii_digit() || c == '-')?;
    let (name, octave) = text.split_at(split);
    let name = name.trim().to_ascii_uppercase();
    let index = NOTE_NAMES.iter().position(|n| *n == name)?;
    let octave: i32 = octave.trim().parse().ok()?;
    let note = (octave + 1) * 12 + index as i32;
    (0..=127).contains(&note).then_some(note as f32)
}

/// nice-plug's `s2v_f32_hz_then_khz`: hertz with or without a unit, kilohertz, or a key name with
/// optional cents.
fn hz_then_khz(string: &str) -> Option<f32> {
    let string = string.trim();
    let mut segments = string.split(',');
    let segments = (segments.next(), segments.next(), segments.next());

    if let (_, Some(midi_key_number_str), Some(cents_str))
    | (Some(midi_key_number_str), Some(cents_str), None) = segments
    {
        let cents_str = cents_str
            .trim_start_matches([' ', '+'])
            .trim_end_matches([' ', 'C', 'c', 'E', 'e', 'N', 'n', 'T', 't', 'S', 's', '.']);

        if let (Some(midi_key_number), Ok(cents)) =
            (key_number(midi_key_number_str), cents_str.parse::<i32>())
        {
            let plain_key_freq = midi_key_to_freq(midi_key_number as f32);
            let cents_multiplier = 2.0f32.powf(cents as f32 / 100.0 / 12.0);
            return Some(plain_key_freq * cents_multiplier);
        }
    }

    if let (_, Some(midi_key_number_str), _) | (Some(midi_key_number_str), None, None) = segments
        && let Some(midi_key_number) = key_number(midi_key_number_str)
    {
        return Some(midi_key_to_freq(midi_key_number as f32));
    }

    let frequency_segment = segments.0?;
    let cleaned_string = frequency_segment
        .trim_end_matches([' ', 'k', 'K', 'h', 'H', 'z', 'Z'])
        .parse()
        .ok();
    match frequency_segment.get(frequency_segment.len().saturating_sub(3)..) {
        Some(unit) if unit.eq_ignore_ascii_case("khz") => cleaned_string.map(|x: f32| x * 1000.0),
        _ => cleaned_string,
    }
}

/// nice-plug's `s2v_i32_key_formatter`: a key name, `C4` or `C# 4`, as a MIDI key, 60 being C4.
fn key_number(string: &str) -> Option<i32> {
    let string = string.trim();
    if string.len() < 2 {
        return None;
    }
    let (key_name, octave) = string
        .split_once(|c: char| c.is_whitespace())
        .unwrap_or_else(|| {
            if string.len() > 2 && &string[1..2] == "#" {
                (&string[..2], &string[2..])
            } else {
                (&string[..1], &string[1..])
            }
        });
    let key_number = NOTE_NAMES
        .iter()
        .position(|&candidate| key_name.eq_ignore_ascii_case(candidate))?
        as i32;
    let octave: i32 = octave.trim().parse().ok()?;
    Some(key_number + (12 * (octave + 1)))
}

/// nice-plug's `util::f32_midi_key_to_freq`: A4 = 440 Hz, equal temperament.
fn midi_key_to_freq(key: f32) -> f32 {
    2.0f32.powf((key - 69.0) / 12.0) * 440.0
}

/// A route reading's unit: what it says, how many decimals it shows, and what a target unit is
/// worth in it. mxm-kit's `mxm_modulation_params::reading::Unit`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    /// What follows the number, separator included: `" st"`, but `"x"` for a multiplier.
    pub suffix: &'static str,
    /// Decimals shown.
    pub places: usize,
    /// How many displayed units one target unit is: a hundred for a percentage.
    pub scale: f32,
}

/// Semitones, to two decimals.
pub const SEMITONES: Unit = Unit {
    suffix: " st",
    places: 2,
    scale: 1.0,
};

/// Octaves, to two decimals.
pub const OCTAVES: Unit = Unit {
    suffix: " oct",
    places: 2,
    scale: 1.0,
};

/// A percentage of the target's own range, whole numbers.
pub const PERCENT: Unit = Unit {
    suffix: " %",
    places: 0,
    scale: 100.0,
};

/// A multiplier on a rate, to two decimals, read against the source's own rate — `+0.60x`.
pub const TIMES: Unit = Unit {
    suffix: "x",
    places: 2,
    scale: 1.0,
};

/// What a route delivers at an amount of one, in its target's unit, and how to say it.
/// mxm-kit's `mxm_modulation_params::reading::Reach`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reach {
    /// Delivered at an amount of one with the source at its peak — per octave of keyboard for a
    /// Key route.
    pub full: f32,
    /// The reach a **negative** amount multiplies by.
    pub below: f32,
    /// The target's unit.
    pub unit: Unit,
    /// Whether this is a Key route, which reads per octave: `st/oct`, `oct/oct`, `%/oct`.
    pub per_octave: bool,
}

impl Reach {
    /// A reach of `full` target units at an amount of one.
    #[must_use]
    pub const fn new(full: f32, unit: Unit) -> Self {
        Self {
            full,
            below: full,
            unit,
            per_octave: false,
        }
    }

    /// A Key route's reach: `full` target units per octave of keyboard.
    #[must_use]
    pub const fn per_octave(full: f32, unit: Unit) -> Self {
        Self {
            full,
            below: full,
            unit,
            per_octave: true,
        }
    }

    /// The reach that applies at `amount`: `below` for a negative amount.
    #[must_use]
    pub fn at(&self, amount: f32) -> f32 {
        if amount < 0.0 { self.below } else { self.full }
    }
}

/// How the fader for an amount travels, and which halves it has.
/// mxm-kit's `mxm_modulation_params::reading::Fader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fader {
    /// −1…+1, linear.
    Linear,
    /// −1…+1, square-law about zero.
    SquareLaw,
    /// 0…+1: a one-sided target whose live half is positive.
    PositiveOnly,
    /// −1…0: a one-sided target whose live half is negative.
    NegativeOnly,
}

impl Fader {
    /// The fader an offer asks for: a one-sided offer has one half, and a two-sided one is linear
    /// unless `square_law`.
    #[must_use]
    pub const fn for_offer(offer: Offer, square_law: bool) -> Self {
        match offer {
            Offer::PositiveOnly => Self::PositiveOnly,
            Offer::NegativeOnly => Self::NegativeOnly,
            Offer::Both | Offer::Refused if square_law => Self::SquareLaw,
            Offer::Both | Offer::Refused => Self::Linear,
        }
    }

    /// The amount's range.
    #[must_use]
    pub const fn range(self) -> Range {
        match self {
            Self::Linear => Range::Linear {
                min: -1.0,
                max: 1.0,
            },
            Self::SquareLaw => Range::SymmetricalSkewed {
                min: -1.0,
                max: 1.0,
                factor: 0.5,
                center: 0.0,
            },
            Self::PositiveOnly => Range::Linear { min: 0.0, max: 1.0 },
            Self::NegativeOnly => Range::Linear {
                min: -1.0,
                max: 0.0,
            },
        }
    }

    /// The amount's lowest and highest values.
    #[must_use]
    pub const fn bounds(self) -> (f32, f32) {
        match self {
            Self::Linear | Self::SquareLaw => (-1.0, 1.0),
            Self::PositiveOnly => (0.0, 1.0),
            Self::NegativeOnly => (-1.0, 0.0),
        }
    }
}

/// mxm-kit's `mxm_params::text::signed`: signed, to `places` decimals, never a negative zero.
#[must_use]
pub fn signed(value: f32, places: usize) -> String {
    let text = format!("{value:+.places$}");
    match text.strip_prefix('-') {
        Some(digits) if digits.bytes().all(|b| b == b'0' || b == b'.') => format!("+{digits}"),
        _ => text,
    }
}

/// A route's reading at `amount`: what it delivers, signed and never a negative zero.
#[must_use]
pub fn route_reading(reach: Reach, amount: f32) -> String {
    let shown = amount * reach.at(amount) * reach.unit.scale;
    let number = signed(shown, reach.unit.places);
    if reach.per_octave {
        format!("{number}{}/oct", reach.unit.suffix)
    } else {
        format!("{number}{}", reach.unit.suffix)
    }
}

/// [`route_reading`]'s inverse: a number typed in the reading's unit, back to an amount within the
/// fader's range. The unit's text is ignored, so `7`, `+7 st` and `7.00 st` land on one amount.
#[must_use]
pub fn route_parse(reach: Reach, fader: Fader, text: &str) -> Option<f32> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+'))
        .collect();
    let entered: f32 = cleaned.parse().ok()?;
    let delivered = entered / reach.unit.scale;
    if !delivered.is_finite() {
        return None;
    }
    let above = (reach.full != 0.0).then(|| delivered / reach.full);
    let below = (reach.below != 0.0).then(|| delivered / reach.below);
    let amount = match (above, below) {
        (Some(amount), _) if amount >= 0.0 => amount,
        (_, Some(amount)) if amount < 0.0 => amount,
        _ => return None,
    };
    let (low, high) = fader.bounds();
    Some(amount.clamp(low, high))
}
