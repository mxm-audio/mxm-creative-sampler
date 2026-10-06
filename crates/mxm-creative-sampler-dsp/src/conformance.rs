//! The creative sampler's routing as the collection's modulation standard checks it
//! (`mxm_modulation::conformance`; `plans/plan-modulation-standard.md`).
//!
//! Behind the `conformance` feature, which only `[dev-dependencies]` enable — this crate's own
//! tests, and the plugin's, whose route readings are held to [`Declared::deliver`] — so no shipped
//! graph carries it. [`Declared`] answers every question through [`crate::routing`]'s own tables
//! and a real [`Graph`], never a copy of them.

use mxm_modulation::conformance::{Declaration, Kind};
use mxm_modulation::standard::{self, Offer, Performance};

use crate::routing::{
    self, AMPLITUDE_BOUND, Graph, KEY_UNIT_SEMITONES, Routing, SOURCE_NAMES, SOURCES, TARGET_NAMES,
    TARGETS, target,
};

/// What each target is, for the standard: the amplitude factor, a cutoff, a pitch, and the two scan
/// speeds in their own travel unit.
const KINDS: [Kind; TARGETS] = [
    Kind::Amplitude,
    Kind::Cutoff,
    Kind::Pitch,
    Kind::Control,
    Kind::Control,
];

/// The creative sampler's routing declaration.
#[derive(Debug, Clone, Copy, Default)]
pub struct Declared;

/// Exactly one route, at `amount`.
fn one_route(target: usize, source: usize, amount: f32) -> Routing {
    let mut routing = Routing::new();
    routing.present[target][source] = true;
    routing.amounts[target][source] = amount;
    routing.compact();
    routing
}

impl Declaration for Declared {
    fn sources(&self) -> usize {
        SOURCES
    }

    fn targets(&self) -> usize {
        TARGETS
    }

    fn performance(&self, source: usize) -> Option<Performance> {
        routing::PERFORMANCE[source]
    }

    fn kind(&self, target: usize) -> Kind {
        KINDS[target]
    }

    fn machine(&self, target: usize, source: usize) -> bool {
        routing::machine(target, source)
    }

    fn offered(&self, target: usize, source: usize) -> Offer {
        routing::offer(target, source)
    }

    fn key_unit(&self) -> f32 {
        KEY_UNIT_SEMITONES
    }

    /// One route alone through a voice's own [`Graph`], its source publishing `raw`; Amplitude
    /// through the factor the voice applies. The bound is generous on every summed target, as the
    /// voice's own are.
    fn deliver(&self, target: usize, source: usize, amount: f32, raw: f32) -> f32 {
        let routing = one_route(target, source, amount);
        let mut graph = Graph::new();
        graph.set_topology(&routing);
        graph.begin_sample();
        graph.write(source, raw);
        if target == target::AMPLITUDE {
            standard::amplitude_factor(graph.sum(target, &routing, AMPLITUDE_BOUND)) - 1.0
        } else {
            graph.sum(target, &routing, 64.0)
        }
    }

    fn name(&self, target: usize, source: usize) -> String {
        format!("{} from {}", TARGET_NAMES[target], SOURCE_NAMES[source])
    }
}

#[cfg(test)]
mod tests {
    use core::f32::consts::TAU;

    use mxm_modulation::conformance::{self, Case, Input};

    use super::*;
    use crate::routing::source;
    use crate::{Engine, Note, Params, Sample, Voice};

    fn report(result: Result<(), Vec<String>>) {
        if let Err(failures) = result {
            panic!("{} failure(s):\n{}", failures.len(), failures.join("\n"));
        }
    }

    fn tone() -> Sample {
        Sample::new(
            (0..48_000)
                .map(|n| {
                    let value = (TAU * 220.0 * n as f32 / 48_000.0).sin() * 0.5;
                    [value, value]
                })
                .collect(),
            48_000.0,
        )
        .unwrap()
    }

    fn sounding(engine: &Engine) -> &Voice {
        engine
            .voices
            .iter()
            .find(|voice| voice.active)
            .expect("a sounding voice")
    }

    /// **Every pair means what the standard says**: offered as `standard::offer` says, nothing at
    /// its source's rest, a meaningful move at full, and the standard reach for every pair this
    /// sampler did not wire before it had routing.
    ///
    /// Falsified before trusted: with Pitch ← Key left at the pitch column's twelve semitones —
    /// 2.4 per octave of keyboard — it names that pair.
    #[test]
    fn every_pair_means_what_the_standard_says() {
        report(conformance::check_declaration(&Declared));
    }

    /// **A voice publishes what the standard says**: Key from middle C over its unit, Velocity as
    /// `v − 1`, the gestures as they arrive, each exactly zero at rest.
    ///
    /// Falsified before trusted: publishing the raw velocity fails at every input.
    #[test]
    fn a_voice_publishes_what_the_standard_says() {
        let sample = tone();
        report(conformance::check_publishers(&Declared, |from, input| {
            let params = Params::default();
            let mut engine = Engine::default();
            engine.set_topology(&one_route(target::CUTOFF, from, 0.0));
            let (key, velocity) = match input {
                Input::Note(n) => (n, 1.0),
                Input::Normalised(value) if from == source::VELOCITY => (60, value),
                _ => (60, 1.0),
            };
            match input {
                Input::Normalised(value) if from == source::WHEEL => engine.set_wheel(0, value),
                Input::Lever(value) => engine.set_bend_position(0, value),
                _ => {}
            }
            engine.note_on(Note::midi(key, velocity), [Some(&sample), None], &params);
            if let Input::Normalised(value) = input
                && from == source::PRESSURE
            {
                engine.set_pressure(None, 0, key, value);
            }
            engine.render([Some(&sample), None], &params);
            sounding(&engine).graph.read_for_test(from)
        }));
    }

    /// **Random is `2u − 1`, one draw per press**: every draw in range, both ends reached.
    #[test]
    fn random_is_bipolar_and_spans_its_range() {
        let draws: Vec<f32> = (0..4_096u64)
            .map(|born| Voice::random_for(born, (born % 128) as u8))
            .collect();
        assert!(draws.iter().all(|d| (-1.0..=1.0).contains(d)));
        assert!(draws.iter().any(|&d| d < -0.99) && draws.iter().any(|&d| d > 0.99));
    }

    /// **Velocity is the press that last triggered the envelope**: a Legato press that re-pitches
    /// without retriggering keeps the phrase's; a Mono press, which retriggers, brings its own.
    ///
    /// Falsified before trusted: publishing the note's own velocity reads the Legato press's.
    #[test]
    fn velocity_is_the_press_that_last_triggered_the_envelope() {
        use crate::VoiceMode;
        let sample = tone();
        for (mode, expected) in [(VoiceMode::Legato, -0.75), (VoiceMode::Mono, -0.125)] {
            let params = Params {
                voice_mode: mode,
                ..Params::default()
            };
            let mut engine = Engine::default();
            engine.set_topology(&one_route(target::CUTOFF, source::VELOCITY, 0.0));
            engine.note_on(Note::midi(60, 0.25), [Some(&sample), None], &params);
            engine.render([Some(&sample), None], &params);
            engine.note_on(Note::midi(64, 0.875), [Some(&sample), None], &params);
            engine.render([Some(&sample), None], &params);
            assert_eq!(
                sounding(&engine).graph.read_for_test(source::VELOCITY),
                expected,
                "{mode:?}"
            );
        }
    }

    /// **After a release, no performance route holds a note open** — every pair, both halves,
    /// the softest and hardest notes and the keyboard's ends, gestures held at full through the
    /// note and let go at the release.
    #[test]
    fn after_a_release_no_performance_route_holds_a_note_open() {
        let sample = tone();
        report(conformance::check_release_silence(
            &Declared,
            &[],
            |case: Case| {
                let params = Params {
                    release_s: 0.05,
                    ..Params::default()
                };
                let mut engine = Engine::default();
                engine.set_topology(&one_route(case.target, case.source, case.amount));
                engine.set_wheel(0, 1.0);
                engine.set_bend_position(0, 1.0);
                engine.note_on(
                    Note::midi(case.key, case.velocity),
                    [Some(&sample), None],
                    &params,
                );
                engine.set_pressure(None, 0, case.key, 1.0);
                for _ in 0..4_800 {
                    engine.render([Some(&sample), None], &params);
                }
                engine.note_off(None, 0, case.key);
                engine.set_wheel(0, 0.0);
                engine.set_bend_position(0, 0.0);
                (0..96_000).any(|_| {
                    let out = engine.render([Some(&sample), None], &params);
                    out.left == 0.0 && out.right == 0.0 && engine.active_voices() == 0
                })
            },
        ));
    }
}
