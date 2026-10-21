//! The creative sampler's compiled-in, read-only Init patch.
//!
//! This is the editable declaration of the sound. The parameter table ([`crate::params`]), the
//! generated source and the routing parameters all read this value; the plugin's shared preset
//! system then derives its protected Init row from those parameter defaults. Init deliberately has
//! no preset file, so it cannot be renamed, overwritten or deleted.

use crate::generate::Recipe;
use crate::sound::{Character, Envelope, Filter, Layer, Lfo, Performance, Sound};
use mxm_creative_sampler_dsp::routing::{source, target};
use mxm_creative_sampler_dsp::{ConverterType, FilterMode, LfoShape, LoopMode, Reader, VoiceMode};

/// One route's complete Init state. Presence and amount remain independent so a removed route can
/// retain the depth it will recover when it is added again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Route {
    pub target: usize,
    pub source: usize,
    pub present: bool,
    pub amount: f32,
}

impl Route {
    const OFF: Self = Self {
        target: usize::MAX,
        source: usize::MAX,
        present: false,
        amount: 0.0,
    };
}

#[derive(Debug, Clone, Copy)]
pub struct InitPatch {
    /// Generated source recipes. `None` is an empty layer.
    pub sources: [Option<Recipe>; 2],
    /// Every value but the routes, in the controls' plain units.
    pub sound: Sound,
    /// Only present routes and non-zero parked amounts need entries; every omitted pair is off at
    /// zero depth.
    pub routes: &'static [Route],
}

impl InitPatch {
    pub fn route(&self, wanted_target: usize, wanted_source: usize) -> Route {
        self.routes
            .iter()
            .copied()
            .find(|route| route.target == wanted_target && route.source == wanted_source)
            .unwrap_or(Route::OFF)
    }
}

/// The sound captured from the running player on 2026-09-13.
///
/// The generated Init saw is the source. The values are in the controls' plain units, not their
/// skewed host-normalised representation, so this reads the same way the factory-preset designs do.
pub const PATCH: InitPatch = InitPatch {
    sources: [Some(Recipe::InitSaw), None],
    sound: Sound {
        layers: [
            Layer {
                reader: Reader::Grain,
                root: crate::generate::ROOT,
                transpose: 0.0,
                start: 0.0,
                end: 1.0,
                loop_start: 0.0,
                loop_end: 1.0,
                loop_crossfade_s: 0.0,
                loop_mode: LoopMode::Forward,
                reverse: false,
                level: 0.60,
                pan: 0.0,
                scan_speed: 0.30,
                grain_size_s: 0.216_260_87,
                grain_rate_hz: 36.094_425,
                pitch_variation_st: 1.024_636_2e-8,
                position_variation: 0.105,
                onset_timing: 0.185_000_02,
                stereo_width: 0.0,
                reverse_chance: 0.0,
                grain_shape: 1.0,
            },
            Layer {
                reader: Reader::Repitch,
                root: 60.0,
                transpose: 0.0,
                start: 0.0,
                end: 1.0,
                loop_start: 0.0,
                loop_end: 1.0,
                loop_crossfade_s: 0.0,
                loop_mode: LoopMode::Off,
                reverse: false,
                level: 1.0,
                pan: 0.0,
                scan_speed: 1.0,
                grain_size_s: 0.090,
                grain_rate_hz: 14.0,
                pitch_variation_st: 0.0,
                position_variation: 0.25,
                onset_timing: 0.25,
                stereo_width: 0.0,
                reverse_chance: 0.0,
                grain_shape: 1.0,
            },
        ],
        character: Character {
            sample_rate: 0.0,
            bit_depth: 0.0,
            bit_smoothing: 0.0,
            converter_drive: 0.0,
            converter_type: ConverterType::Linear,
            jitter: 0.0,
        },
        filter: Filter {
            cutoff_hz: 2_693.449,
            resonance: 0.38,
            mode: FilterMode::LowPass,
            drive: 0.0,
        },
        envelope: Envelope {
            attack_s: 0.220,
            decay_s: 0.600,
            sustain: 0.88,
            release_s: 0.800,
        },
        lfos: [
            Lfo {
                rate_hz: 0.25,
                shape: LfoShape::Sine,
            },
            Lfo {
                rate_hz: 2.663_807_9,
                shape: LfoShape::Triangle,
            },
        ],
        performance: Performance {
            master: 0.8,
            voices: VoiceMode::Poly,
            glide_s_per_octave: 0.0,
            bend_range_st: 2.0,
        },
    },
    routes: &[
        Route {
            target: target::AMPLITUDE,
            source: source::LFO2,
            present: true,
            amount: 0.0,
        },
        Route {
            target: target::AMPLITUDE,
            source: source::VELOCITY,
            present: true,
            amount: 0.35,
        },
        Route {
            target: target::CUTOFF,
            source: source::LFO2,
            present: true,
            amount: 0.255_102_04,
        },
        Route {
            target: target::CUTOFF,
            source: source::ENVELOPE,
            present: true,
            amount: 0.387_755_16,
        },
        Route {
            target: target::CUTOFF,
            source: source::PRESSURE,
            present: false,
            amount: 1.0,
        },
        Route {
            target: target::SCAN_A,
            source: source::LFO1,
            present: false,
            amount: 0.765_306_1,
        },
        Route {
            target: target::SCAN_A,
            source: source::LFO2,
            present: false,
            amount: 0.178_571_46,
        },
    ],
};
