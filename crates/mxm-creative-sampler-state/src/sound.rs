//! The plain values the DSP reads, and their mapping onto [`mxm_creative_sampler_dsp::Params`].
//!
//! **One mapping, for the plugin's callback and for a state alike.** The plugin fills a [`Sound`]
//! from its smoothed parameters every sample and calls [`Sound::dsp`]; a [`crate::State`] fills one
//! from its stored values. The field renames and the two clamps happen here and nowhere else, so
//! what a state plays and what the plugin plays cannot drift apart.

use mxm_creative_sampler_dsp::{
    ConverterType, FilterMode, LayerParams, LfoShape, LoopMode, Params, Reader, VoiceMode,
};

/// One layer's values, in the controls' plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layer {
    pub reader: Reader,
    pub root: f32,
    pub transpose: f32,
    pub start: f32,
    pub end: f32,
    pub loop_start: f32,
    pub loop_end: f32,
    /// Seconds of source. An amount, so Init is zero: the hard wrap.
    pub loop_crossfade_s: f32,
    pub loop_mode: LoopMode,
    pub reverse: bool,
    pub level: f32,
    pub pan: f32,
    pub scan_speed: f32,
    pub grain_size_s: f32,
    pub grain_rate_hz: f32,
    pub pitch_variation_st: f32,
    pub position_variation: f32,
    pub onset_timing: f32,
    pub stereo_width: f32,
    pub reverse_chance: f32,
    pub grain_shape: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Character {
    pub sample_rate: f32,
    pub bit_depth: f32,
    pub bit_smoothing: f32,
    pub converter_drive: f32,
    pub converter_type: ConverterType,
    pub jitter: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub mode: FilterMode,
    pub drive: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Envelope {
    pub attack_s: f32,
    pub decay_s: f32,
    pub sustain: f32,
    pub release_s: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lfo {
    pub rate_hz: f32,
    pub shape: LfoShape,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Performance {
    pub master: f32,
    pub voices: VoiceMode,
    pub glide_s_per_octave: f32,
    /// Semitones per unit of channel bend. Not in [`Params`]: the plugin turns a bend into channel
    /// tuning itself.
    pub bend_range_st: f32,
}

/// Every value the DSP reads but the routes, in the controls' plain units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sound {
    pub layers: [Layer; 2],
    pub character: Character,
    pub filter: Filter,
    pub envelope: Envelope,
    pub lfos: [Lfo; 2],
    pub performance: Performance,
}

impl Layer {
    /// What the DSP reads for this layer.
    #[inline]
    #[must_use]
    pub fn dsp(&self) -> LayerParams {
        LayerParams {
            reader: self.reader,
            root_note: self.root,
            tune_semitones: self.transpose,
            start: self.start,
            end: self.end,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
            loop_crossfade_s: self.loop_crossfade_s,
            loop_mode: self.loop_mode,
            reverse: self.reverse,
            level: self.level,
            pan: self.pan,
            stretch_travel: self.scan_speed,
            grain_size_s: self.grain_size_s,
            grain_density_hz: self.grain_rate_hz,
            grain_position_variation: self.position_variation,
            grain_onset_timing: self.onset_timing,
            grain_stereo_width: self.stereo_width,
            grain_reverse_chance: self.reverse_chance,
            grain_shape: self.grain_shape,
            grain_spread_semitones: self.pitch_variation_st,
        }
    }
}

impl Sound {
    /// What the DSP reads: the free rates, before any tempo sync (`crate::params::TEMPO_SYNCS`).
    ///
    /// The cutoff and the resonance are clamped here, to the filter's own safe span, and nothing
    /// else is: every other value reaches the DSP as the parameter holds it, which bounds it itself.
    #[inline]
    #[must_use]
    pub fn dsp(&self) -> Params {
        let [lfo1, lfo2] = self.lfos;
        Params {
            layers: [self.layers[0].dsp(), self.layers[1].dsp()],
            character: mxm_creative_sampler_dsp::Character {
                rate: self.character.sample_rate,
                converter: self.character.bit_depth,
                reconstruction: self.character.bit_smoothing,
                input: self.character.converter_drive,
                converter_type: self.character.converter_type,
                jitter: self.character.jitter,
            },
            attack_s: self.envelope.attack_s,
            decay_s: self.envelope.decay_s,
            sustain: self.envelope.sustain,
            release_s: self.envelope.release_s,
            filter_mode: self.filter.mode,
            cutoff_hz: self.filter.cutoff_hz.clamp(20.0, 22_000.0),
            resonance: self.filter.resonance.clamp(0.0, 0.98),
            filter_drive: self.filter.drive,
            lfo_rate_hz: lfo1.rate_hz,
            lfo_shape: lfo1.shape,
            lfo2_rate_hz: lfo2.rate_hz,
            lfo2_shape: lfo2.shape,
            master: self.performance.master,
            voice_mode: self.performance.voices,
            glide_s: self.performance.glide_s_per_octave,
        }
    }
}

/// A choice's permanent id, which a state stores, and the name it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    pub id: &'static str,
    pub name: &'static str,
}

const fn variant(id: &'static str, name: &'static str) -> Variant {
    Variant { id, name }
}

/// A choice parameter's variants in index order, and the DSP's value for each.
#[derive(Debug, Clone, Copy)]
pub struct Choices<T: 'static> {
    pub variants: &'static [Variant],
    pub values: &'static [T],
}

impl<T: Copy + PartialEq> Choices<T> {
    /// The index of `value`. Every value the DSP has is offered, so this always finds one.
    #[must_use]
    pub fn index_of(&self, value: T) -> usize {
        self.values
            .iter()
            .position(|candidate| *candidate == value)
            .expect("every DSP value is offered")
    }

    /// The DSP's value at `index`, the last one past the end.
    #[must_use]
    pub fn value(&self, index: usize) -> T {
        self.values[index.min(self.values.len() - 1)]
    }
}

/// A layer's Playback mode.
pub const READERS: Choices<Reader> = Choices {
    variants: &[
        variant("repitch", "Repitch"),
        variant("stretch", "Stretch"),
        variant("grain", "Grain"),
    ],
    values: &[Reader::Repitch, Reader::Stretch, Reader::Grain],
};

/// A layer's Loop.
pub const LOOP_MODES: Choices<LoopMode> = Choices {
    variants: &[
        variant("off", "Off"),
        variant("forward", "Forward"),
        variant("alternate", "Alternate"),
    ],
    values: &[LoopMode::Off, LoopMode::Forward, LoopMode::Alternate],
};

/// Voices.
pub const VOICE_MODES: Choices<VoiceMode> = Choices {
    variants: &[
        variant("poly", "Poly"),
        variant("mono", "Mono"),
        variant("legato", "Legato"),
    ],
    values: &[VoiceMode::Poly, VoiceMode::Mono, VoiceMode::Legato],
};

/// What an LFO draws. See [`mxm_creative_sampler_dsp::LfoShape`].
pub const LFO_SHAPES: Choices<LfoShape> = Choices {
    variants: &[
        variant("sine", "Sine"),
        variant("triangle", "Triangle"),
        variant("rampup", "Ramp up"),
        variant("rampdown", "Ramp down"),
        variant("square", "Square"),
        variant("samplehold", "Sample & hold"),
    ],
    values: &[
        LfoShape::Sine,
        LfoShape::Triangle,
        LfoShape::RampUp,
        LfoShape::RampDown,
        LfoShape::Square,
        LfoShape::SampleHold,
    ],
};

/// Which converter the voice's DAC is. See [`mxm_creative_sampler_dsp::ConverterType`].
///
/// **Named for the machines, not for the mathematics.** mxm-kit's `docs/oscillators/14-samplers.md`
/// §14.1: the vintage DAC names are companding parts rather than resolutions, and the card already
/// reads in the numbers those machines were sold on.
pub const CONVERTER_TYPES: Choices<ConverterType> = Choices {
    variants: &[
        variant("linear", "Linear"),
        variant("companding", "Companding"),
    ],
    values: &[ConverterType::Linear, ConverterType::Companding],
};

/// Filter mode.
pub const FILTER_MODES: Choices<FilterMode> = Choices {
    variants: &[
        variant("off", "Off"),
        variant("lowpass", "Low pass"),
        variant("bandpass", "Band pass"),
        variant("highpass", "High pass"),
    ],
    values: &[
        FilterMode::Off,
        FilterMode::LowPass,
        FilterMode::BandPass,
        FilterMode::HighPass,
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every choice names one DSP value per variant**, and every DSP value has its variant.
    #[test]
    fn every_choice_names_one_dsp_value_per_variant() {
        fn check<T: Copy + PartialEq + std::fmt::Debug>(choices: Choices<T>) {
            assert_eq!(choices.variants.len(), choices.values.len());
            for (index, value) in choices.values.iter().enumerate() {
                assert_eq!(
                    choices.index_of(*value),
                    index,
                    "{value:?} is offered twice"
                );
            }
        }
        check(READERS);
        check(LOOP_MODES);
        check(VOICE_MODES);
        check(LFO_SHAPES);
        check(CONVERTER_TYPES);
        check(FILTER_MODES);
    }
}
