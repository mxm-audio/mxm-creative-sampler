//! `mxm-creative-sampler`'s routing parameters: one presence and one amount per *(target, source)*
//! pair.
//!
//! `plans/plan-modulation-routing.md` §4.3, and this instrument's
//! `plans/plan-sampler-modulation-and-converter.md` §3. The derive needs concrete fields and this
//! instrument's source list is its own, so the struct is declared here rather than generated —
//! which is the shape `mxm-mono-08` already uses for its 120 routing ids. What is shared is
//! everything around these fields: [`mxm_modulation_params`] reads them, and
//! [`mxm_creative_sampler_dsp::routing`] evaluates them.
//!
//! **What each pair is comes from the state crate** (`mxm_creative_sampler_state::routes` and its
//! parameter table): its ids, names, Init presence and depth, travel and reach. The ids are still
//! derived by one `#[nested(id_prefix = …)]` per target here, so a pair's id is `<target>_<source>`
//! and `<target>_<source>on`, and `the_id_table_is_what_the_derive_actually_produces` holds the
//! crate's written-out table to the derive. **Provisional like every id in this plugin.**

use mxm_creative_sampler_dsp::routing::{Routing, SOURCE_NAMES, SOURCES, TARGETS, source, target};
use mxm_creative_sampler_state::params::{self as table, Kind, Smoothing};
use mxm_creative_sampler_state::text::{self, Fader};
use mxm_modulation_params::Route;
use mxm_modulation_params::reading::{self, Reach, Unit};
use nice_plug::prelude::*;

pub use mxm_creative_sampler_state::routes::ROUTE_IDS;

/// One target's routes: a presence and a signed amount for every source the instrument declares.
///
/// **Presence is the enable and the amount is the depth**, and nothing else. No selector, because a
/// pair *is* its source; no polarity switch, because the amount is signed; no "via" modifier,
/// because a gesture scaling a route is a multiplier module rather than a third parameter here.
#[derive(Params)]
pub struct TargetRoutes {
    #[id = "lfo1on"]
    pub lfo1_on: BoolParam,
    #[id = "lfo1"]
    pub lfo1: FloatParam,
    #[id = "lfo2on"]
    pub lfo2_on: BoolParam,
    #[id = "lfo2"]
    pub lfo2: FloatParam,
    #[id = "envon"]
    pub env_on: BoolParam,
    #[id = "env"]
    pub env: FloatParam,
    #[id = "velon"]
    pub vel_on: BoolParam,
    #[id = "vel"]
    pub vel: FloatParam,
    #[id = "keyon"]
    pub key_on: BoolParam,
    #[id = "key"]
    pub key: FloatParam,
    #[id = "wheelon"]
    pub wheel_on: BoolParam,
    #[id = "wheel"]
    pub wheel: FloatParam,
    #[id = "presson"]
    pub press_on: BoolParam,
    #[id = "press"]
    pub press: FloatParam,
    #[id = "bendon"]
    pub bend_on: BoolParam,
    #[id = "bend"]
    pub bend: FloatParam,
    #[id = "rndon"]
    pub rnd_on: BoolParam,
    #[id = "rnd"]
    pub rnd: FloatParam,
    #[id = "layaon"]
    pub laya_on: BoolParam,
    #[id = "laya"]
    pub laya: FloatParam,
    #[id = "laybon"]
    pub layb_on: BoolParam,
    #[id = "layb"]
    pub layb: FloatParam,
}

impl TargetRoutes {
    /// One target's routes, all absent, every depth at zero.
    fn new(target: usize) -> Self {
        Self {
            lfo1_on: presence(target, source::LFO1),
            lfo1: amount(target, source::LFO1),
            lfo2_on: presence(target, source::LFO2),
            lfo2: amount(target, source::LFO2),
            env_on: presence(target, source::ENVELOPE),
            env: amount(target, source::ENVELOPE),
            vel_on: presence(target, source::VELOCITY),
            vel: amount(target, source::VELOCITY),
            key_on: presence(target, source::KEY),
            key: amount(target, source::KEY),
            wheel_on: presence(target, source::WHEEL),
            wheel: amount(target, source::WHEEL),
            press_on: presence(target, source::PRESSURE),
            press: amount(target, source::PRESSURE),
            bend_on: presence(target, source::BEND),
            bend: amount(target, source::BEND),
            rnd_on: presence(target, source::RANDOM),
            rnd: amount(target, source::RANDOM),
            laya_on: presence(target, source::LAYER_A),
            laya: amount(target, source::LAYER_A),
            layb_on: presence(target, source::LAYER_B),
            layb: amount(target, source::LAYER_B),
        }
    }

    /// This target's routes, in declared source order, for [`mxm_modulation_params`].
    ///
    /// **`target` is this group's index.** Each row's keyboard-cursor scope is its parameter's own
    /// permanent id rather than a shared literal, so a row can be told from every other row — and
    /// those ids live in [`ROUTE_IDS`], keyed by target.
    fn routes(&self, target: usize) -> [Route<'_>; SOURCES] {
        [
            Route {
                source: SOURCE_NAMES[0],
                present: &self.lfo1_on,
                amount: &self.lfo1,
                present_id: ROUTE_IDS[target][0].1,
                amount_id: ROUTE_IDS[target][0].0,
            },
            Route {
                source: SOURCE_NAMES[1],
                present: &self.lfo2_on,
                amount: &self.lfo2,
                present_id: ROUTE_IDS[target][1].1,
                amount_id: ROUTE_IDS[target][1].0,
            },
            Route {
                source: SOURCE_NAMES[2],
                present: &self.env_on,
                amount: &self.env,
                present_id: ROUTE_IDS[target][2].1,
                amount_id: ROUTE_IDS[target][2].0,
            },
            Route {
                source: SOURCE_NAMES[3],
                present: &self.vel_on,
                amount: &self.vel,
                present_id: ROUTE_IDS[target][3].1,
                amount_id: ROUTE_IDS[target][3].0,
            },
            Route {
                source: SOURCE_NAMES[4],
                present: &self.key_on,
                amount: &self.key,
                present_id: ROUTE_IDS[target][4].1,
                amount_id: ROUTE_IDS[target][4].0,
            },
            Route {
                source: SOURCE_NAMES[5],
                present: &self.wheel_on,
                amount: &self.wheel,
                present_id: ROUTE_IDS[target][5].1,
                amount_id: ROUTE_IDS[target][5].0,
            },
            Route {
                source: SOURCE_NAMES[6],
                present: &self.press_on,
                amount: &self.press,
                present_id: ROUTE_IDS[target][6].1,
                amount_id: ROUTE_IDS[target][6].0,
            },
            Route {
                source: SOURCE_NAMES[7],
                present: &self.bend_on,
                amount: &self.bend,
                present_id: ROUTE_IDS[target][7].1,
                amount_id: ROUTE_IDS[target][7].0,
            },
            Route {
                source: SOURCE_NAMES[8],
                present: &self.rnd_on,
                amount: &self.rnd,
                present_id: ROUTE_IDS[target][8].1,
                amount_id: ROUTE_IDS[target][8].0,
            },
            Route {
                source: SOURCE_NAMES[9],
                present: &self.laya_on,
                amount: &self.laya,
                present_id: ROUTE_IDS[target][9].1,
                amount_id: ROUTE_IDS[target][9].0,
            },
            Route {
                source: SOURCE_NAMES[10],
                present: &self.layb_on,
                amount: &self.layb,
                present_id: ROUTE_IDS[target][10].1,
                amount_id: ROUTE_IDS[target][10].0,
            },
        ]
    }

    /// One source's amount parameter, concretely, so its smoother can be advanced.
    fn amount_param(&self, source: usize) -> &FloatParam {
        match source {
            0 => &self.lfo1,
            1 => &self.lfo2,
            2 => &self.env,
            3 => &self.vel,
            4 => &self.key,
            5 => &self.wheel,
            6 => &self.press,
            7 => &self.bend,
            8 => &self.rnd,
            9 => &self.laya,
            10 => &self.layb,
            _ => unreachable!("source is not declared"),
        }
    }
}

/// Every target's routes.
#[derive(Params)]
pub struct Routes {
    #[nested(id_prefix = "mod_amp")]
    pub amp: TargetRoutes,
    #[nested(id_prefix = "mod_cutoff")]
    pub cutoff: TargetRoutes,
    #[nested(id_prefix = "mod_pitch")]
    pub pitch: TargetRoutes,
    #[nested(id_prefix = "mod_scana")]
    pub scana: TargetRoutes,
    #[nested(id_prefix = "mod_scanb")]
    pub scanb: TargetRoutes,
}

impl Default for Routes {
    fn default() -> Self {
        Self {
            amp: TargetRoutes::new(target::AMPLITUDE),
            cutoff: TargetRoutes::new(target::CUTOFF),
            pitch: TargetRoutes::new(target::PITCH),
            scana: TargetRoutes::new(target::SCAN_A),
            scanb: TargetRoutes::new(target::SCAN_B),
        }
    }
}

impl Routes {
    /// This target's routes, for the editor and for reading.
    #[must_use]
    pub fn target(&self, target: usize) -> [Route<'_>; SOURCES] {
        match target {
            target::AMPLITUDE => self.amp.routes(target),
            target::CUTOFF => self.cutoff.routes(target),
            target::PITCH => self.pitch.routes(target),
            target::SCAN_A => self.scana.routes(target),
            target::SCAN_B => self.scanb.routes(target),
            _ => unreachable!("target {target} is not declared"),
        }
    }

    /// Reads every pair into the [`Routing`] the DSP evaluates.
    ///
    /// **Presences and amounts both, once per processing interval.** `Engine::render` rebuilds its
    /// compacted lists only when `present` actually changes, so calling this per block costs a walk
    /// of the grid and nothing more when the topology is stable.
    #[must_use]
    pub fn routing(&self) -> Routing {
        let mut routing = Routing::new();
        for t in 0..TARGETS {
            let routes = self.target(t);
            let present = mxm_modulation_params::presences(&routes);
            // Read in the **target's own domain**: the amount is a signed fraction of one, and the
            // per-route full scale is applied by the DSP's `sum_scaled` as a third multiply. Folding
            // the scale in here is exactly what `plan-modulation-routing.md` §6.2 warns against, and
            // what moved every one of the pilot's pinned digests when its params layer did it.
            let amounts = mxm_modulation_params::amounts(&routes, 1.0);
            routing.present[t] = present;
            routing.amounts[t] = amounts;
        }
        routing.compact();
        routing
    }

    /// [`Routes::routing`], with every **newly present** route's amount smoother snapped to its
    /// stored value.
    ///
    /// **An absent route's smoother is not advanced, so it must not be resumed either.**
    /// [`Routes::advance`] walks the live routes only, but an absent pair's amount stays editable: a
    /// host automating it, or a preset load, moves the smoother's *target* and leaves its current
    /// value wherever the last live sample left it. Resuming from there ramps the route in from a
    /// stale depth over a span set by how long it was absent — the host's buffer sizes deciding a
    /// sound. mxm-kit's `docs/code-review-notes.md` §7; mxm-mono-pr1's
    /// `plugins/mxm-mono-pr1/src/routes.rs`'s `topology_from` is the same rule on that instrument.
    ///
    /// `previous` is the routing the last interval ran, which the caller keeps across buffers.
    #[must_use]
    pub fn routing_from(&self, previous: &Routing) -> Routing {
        let routing = self.routing();
        for target in 0..TARGETS {
            for source in 0..SOURCES {
                if routing.present[target][source] && !previous.present[target][source] {
                    let param = self.group(target).amount_param(source);
                    param.smoothed.reset(param.value());
                }
            }
        }
        routing
    }

    /// Advances the **live** routes' amount smoothers by one sample.
    ///
    /// **Per sample, and only the live ones — this is not an optimisation, it is correctness.**
    /// Reading a depth once per block leaves route automation a block stale, and leaves a host's
    /// parameter *modulation* inaudible entirely: `ErasedParam::normalised` is explicitly the
    /// unmodulated value, and `smoothed.next()` is the one that carries both smoothing and
    /// modulation. `mxm_modulation_params::amounts` therefore reads the right number only for a
    /// route nothing is moving, which is why it is used here to seed the block and never to carry a
    /// live depth.
    ///
    /// **Walking the compacted list keeps it O(live).** The init patch wires four pairs of
    /// fifty-five, so this touches four smoothers a sample rather than fifty-five.
    ///
    /// **`mxm-mono-01` already solves this exact problem the same way**, and that is now two honest
    /// copies — the repository's own threshold for extracting. It cannot move into
    /// `mxm-modulation-params` as things stand, because smoothing lives on the concrete
    /// `FloatParam` field rather than on the `Param` trait that `ErasedParam` blankets; sharing it
    /// needs that trait to gain a "next smoothed value" method first. Recorded rather than
    /// re-discovered by whichever instrument converts third.
    pub fn advance(&self, routing: &mut Routing) {
        for i in 0..routing.live().len() {
            let (target, source) = routing.live()[i];
            routing.amounts[target as usize][source as usize] = self
                .group(target as usize)
                .amount_param(source as usize)
                .smoothed
                .next();
        }
    }

    /// One target's routes as a concrete struct, for [`Routes::advance`].
    fn group(&self, target: usize) -> &TargetRoutes {
        match target {
            target::AMPLITUDE => &self.amp,
            target::CUTOFF => &self.cutoff,
            target::PITCH => &self.pitch,
            target::SCAN_A => &self.scana,
            target::SCAN_B => &self.scanb,
            _ => unreachable!("target is not declared"),
        }
    }

    /// Every pair's presence, concretely, in `[target][source]` order.
    ///
    /// **An absent route draws nothing at all** — the routing interface's own rule — so a panel
    /// painted at the init patch shows the four rows the selected patch wires and hides the rest. A
    /// coverage or floor check that needs the whole surface turns these on first, which is the
    /// same reason the pilot measures its card floors with every route revealed rather than at
    /// Init.
    #[must_use]
    pub fn presences(&self) -> Vec<&BoolParam> {
        let mut out = Vec::with_capacity(TARGETS * SOURCES);

        out.push(&self.amp.lfo1_on);
        out.push(&self.amp.lfo2_on);
        out.push(&self.amp.env_on);
        out.push(&self.amp.vel_on);
        out.push(&self.amp.key_on);
        out.push(&self.amp.wheel_on);
        out.push(&self.amp.press_on);
        out.push(&self.amp.bend_on);
        out.push(&self.amp.rnd_on);
        out.push(&self.amp.laya_on);
        out.push(&self.amp.layb_on);
        out.push(&self.cutoff.lfo1_on);
        out.push(&self.cutoff.lfo2_on);
        out.push(&self.cutoff.env_on);
        out.push(&self.cutoff.vel_on);
        out.push(&self.cutoff.key_on);
        out.push(&self.cutoff.wheel_on);
        out.push(&self.cutoff.press_on);
        out.push(&self.cutoff.bend_on);
        out.push(&self.cutoff.rnd_on);
        out.push(&self.cutoff.laya_on);
        out.push(&self.cutoff.layb_on);
        out.push(&self.pitch.lfo1_on);
        out.push(&self.pitch.lfo2_on);
        out.push(&self.pitch.env_on);
        out.push(&self.pitch.vel_on);
        out.push(&self.pitch.key_on);
        out.push(&self.pitch.wheel_on);
        out.push(&self.pitch.press_on);
        out.push(&self.pitch.bend_on);
        out.push(&self.pitch.rnd_on);
        out.push(&self.pitch.laya_on);
        out.push(&self.pitch.layb_on);
        out.push(&self.scana.lfo1_on);
        out.push(&self.scana.lfo2_on);
        out.push(&self.scana.env_on);
        out.push(&self.scana.vel_on);
        out.push(&self.scana.key_on);
        out.push(&self.scana.wheel_on);
        out.push(&self.scana.press_on);
        out.push(&self.scana.bend_on);
        out.push(&self.scana.rnd_on);
        out.push(&self.scana.laya_on);
        out.push(&self.scana.layb_on);
        out.push(&self.scanb.lfo1_on);
        out.push(&self.scanb.lfo2_on);
        out.push(&self.scanb.env_on);
        out.push(&self.scanb.vel_on);
        out.push(&self.scanb.key_on);
        out.push(&self.scanb.wheel_on);
        out.push(&self.scanb.press_on);
        out.push(&self.scanb.bend_on);
        out.push(&self.scanb.rnd_on);
        out.push(&self.scanb.laya_on);
        out.push(&self.scanb.layb_on);
        out
    }
}

/// A pair's presence. **Present in the init patch only where the selected patch wires it.**
fn presence(target: usize, source: usize) -> BoolParam {
    crate::params::switch(ROUTE_IDS[target][source].1)
}

/// A pair's depth. **Signed and drawn bipolar**, because its centre is *no modulation*.
///
/// A half-filled unipolar track would read as "half on" when it means "off" — recorded as a shipped
/// defect once already in mxm-mono-01's `plugins/mxm-mono-01/src/editor/binding.rs`. It is the
/// collection's one route parameter (`mxm_modulation_params::reading`), starting where the compiled
/// Init patch puts it, and it reads as the state crate's `routes::reach` says: what its pair
/// delivers, in the target's own unit.
fn amount(target: usize, source: usize) -> FloatParam {
    let id = ROUTE_IDS[target][source].0;
    let param = table::get(id).unwrap_or_else(|| panic!("`{id}` is not in the table"));
    let Kind::Float(float) = &param.kind else {
        panic!("`{id}` is not continuous");
    };
    let text::Reading::Route(reach, fader) = float.reading else {
        panic!("`{id}` does not read as a route");
    };
    let Smoothing::Linear(smoothing_ms) = float.smoothing else {
        panic!("`{id}` is not smoothed linearly");
    };
    reading::amount_param_at(
        param.name.clone(),
        float.default,
        kit_reach(reach),
        kit_fader(fader),
        smoothing_ms,
    )
}

/// The state crate's reach, as the kit's reading takes it.
fn kit_reach(reach: text::Reach) -> Reach {
    let unit = Unit {
        suffix: reach.unit.suffix,
        places: reach.unit.places,
        scale: reach.unit.scale,
    };
    let kit = if reach.per_octave {
        Reach::per_octave(reach.full, unit)
    } else {
        Reach::new(reach.full, unit)
    };
    kit.below(reach.below)
}

/// The state crate's fader, as the kit's reading takes it.
fn kit_fader(fader: Fader) -> reading::Fader {
    match fader {
        Fader::Linear => reading::Fader::Linear,
        Fader::SquareLaw => reading::Fader::SquareLaw,
        Fader::PositiveOnly => reading::Fader::PositiveOnly,
        Fader::NegativeOnly => reading::Fader::NegativeOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::params::Params;

    /// **The written-out id table is what the derive actually produces.**
    ///
    /// `ROUTE_IDS` exists because a preset's parameter list needs `&'static str` and because these
    /// ids are what a saved patch carries. Two sources of truth for the same names is exactly how
    /// they drift, so this is the one that says they cannot.
    #[test]
    fn the_id_table_is_what_the_derive_actually_produces() {
        let routes = Routes::default();
        let declared: Vec<String> = routes
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        for ids in &ROUTE_IDS {
            for (amount, presence) in ids {
                assert!(
                    declared.iter().any(|id| id == amount),
                    "{amount} is in the table and not in the derive"
                );
                assert!(
                    declared.iter().any(|id| id == presence),
                    "{presence} is in the table and not in the derive"
                );
            }
        }
        assert_eq!(
            declared.len(),
            TARGETS * SOURCES * 2,
            "the derive produced ids the table does not name: {declared:?}"
        );
    }

    /// **The init patch wires the owner-selected routes and retains its parked amounts.**
    ///
    /// The default parameters and the declarative Init are one patch, so this holds both halves:
    /// every pair has the presence and amount declared by `init::PATCH`, including parked depths.
    #[test]
    fn a_fresh_instance_wires_exactly_the_owner_selected_routes() {
        let routing = Routes::default().routing();
        for target in 0..TARGETS {
            for source in 0..SOURCES {
                let declared = crate::init::PATCH.route(target, source);
                let present = declared.present;
                assert_eq!(
                    routing.present[target][source], present,
                    "target {target} source {source}"
                );
                let amount = declared.amount;
                assert!(
                    (routing.amounts[target][source] - amount).abs() < 1.0e-6,
                    "target {target} source {source}: {} is not {amount}",
                    routing.amounts[target][source]
                );
            }
        }
    }

    /// **Remove, edit while absent, re-add: the route arrives at the depth the player set.**
    ///
    /// While the pair is absent nothing advances its smoother, but a host write still moves the
    /// target, so the smoother sits mid-ramp from the old depth. Resuming it would ramp the re-added
    /// route in from that stale number. See [`Routes::routing_from`].
    ///
    /// **Falsified before trusted**: with `routing_from`'s snap removed, the first sample after
    /// re-adding reads a point on the ramp down from `0.5` instead of the stored `-0.5`.
    #[test]
    fn a_re_added_route_arrives_at_its_stored_depth_rather_than_ramping_from_a_stale_one() {
        use nice_plug::params::InternalParamMut;
        const RATE: f32 = 48_000.0;

        let routes = Routes::default();
        assert!(
            !crate::init::PATCH
                .route(target::PITCH, source::WHEEL)
                .present,
            "this pair starts absent, which is what the test needs"
        );

        // Present it, settled at one depth. `_internal_update_smoother(_, true)` settles; `false`,
        // which the wrapper calls after a host parameter change, moves the target only.
        unsafe {
            routes.pitch.wheel_on._internal_set_plain_value(true);
            routes.pitch.wheel._internal_set_plain_value(0.5);
            routes.pitch.wheel._internal_update_smoother(RATE, true);
        }
        let mut live = routes.routing_from(&Routing::new());
        routes.advance(&mut live);
        assert_eq!(live.amounts[target::PITCH][source::WHEEL], 0.5);

        // Remove it, then edit the depth while it is absent.
        unsafe { routes.pitch.wheel_on._internal_set_plain_value(false) };
        let absent = routes.routing_from(&live);
        unsafe {
            routes.pitch.wheel._internal_set_plain_value(-0.5);
            routes.pitch.wheel._internal_update_smoother(RATE, false);
        }
        assert!(
            routes.pitch.wheel.smoothed.is_smoothing(),
            "the edit must leave the smoother mid-ramp, or this proves nothing"
        );

        // Re-add it. The very first sample must be the stored depth.
        unsafe { routes.pitch.wheel_on._internal_set_plain_value(true) };
        let mut back = routes.routing_from(&absent);
        routes.advance(&mut back);
        assert_eq!(
            back.amounts[target::PITCH][source::WHEEL],
            -0.5,
            "a re-added route must arrive at its stored depth"
        );
    }

    /// **A route reads what its pair delivers, in its target's own unit, and reads back** —
    /// `plans/plan-modulation-routing.md` decision 1.11. The cutoff's three scales are the case that
    /// needs it: the Envelope reaches four octaves, Pressure two and Key one per octave of keyboard,
    /// so the same position on three rows under one target moves the filter three different
    /// distances, and a bare percentage read all three as `100 %`. Scan speed B's LFO 1 reach is
    /// negative, so a positive amount reads negative, which is what it does.
    #[test]
    fn a_route_reads_what_its_pair_delivers_and_reads_back() {
        let r = Routes::default();
        let cases: [(&FloatParam, f32, &str); 14] = [
            (&r.cutoff.env, 1.0, "+4.00 oct"),
            (&r.cutoff.env, 0.0, "-4.00 oct"),
            (&r.cutoff.env, 0.5, "+0.00 oct"),
            (&r.cutoff.press, 1.0, "+2.00 oct"),
            (&r.cutoff.key, 1.0, "+1.00 oct/oct"),
            (&r.pitch.lfo1, 1.0, "+12.00 st"),
            (&r.pitch.key, 1.0, "+12.00 st/oct"),
            (&r.scana.lfo1, 1.0, "+0.60x"),
            (&r.scanb.lfo1, 1.0, "-0.47x"),
            (&r.scana.env, 1.0, "+1.00x"),
            (&r.scanb.key, 1.0, "+0.20x/oct"),
            (&r.amp.vel, 1.0, "+100 %"),
            (&r.amp.key, 0.0, "-20 %/oct"),
            (&r.amp.lfo2, 0.75, "+50 %"),
        ];
        for (param, normalised, expected) in cases {
            let text = param.normalized_value_to_string(normalised, true);
            assert_eq!(text, expected, "{} at {normalised}", param.name());
            let back = param
                .string_to_normalized_value(&text)
                .unwrap_or_else(|| panic!("{}: {text} does not parse", param.name()));
            assert!(
                (back - normalised).abs() < 1e-3,
                "{}: {text} read back as {back}",
                param.name()
            );
        }
    }

    use mxm_plugin_test::routing_checks;

    /// **Every route parameter says what the DSP does** — the modulation standard's plugin half:
    /// each pair's travel is its offer's, its reading carries its target's unit and states what
    /// `mxm_creative_sampler_dsp::conformance` measures the voice's own graph delivering, and every
    /// reading survives the host's round trip.
    ///
    /// Falsified before trusted: with Pitch ← Key read at the pitch column's twelve semitones per
    /// unit, it names that pair.
    #[test]
    fn every_route_parameter_says_what_the_dsp_does() {
        let routes = Routes::default();
        if let Err(failures) = routing_checks::amounts(
            &mxm_creative_sampler_dsp::conformance::Declared,
            |target, source| Some(routes.group(target).amount_param(source)),
        ) {
            panic!(
                "{} failure(s):
{}",
                failures.len(),
                failures.join(
                    "
"
                )
            );
        }
    }

    /// **Every reading survives the host's round trip, a rounded zero included**: printed, parsed
    /// and printed again, it is the same text — at amounts either side of zero, not only at the
    /// round numbers. `format!("{:.0}", v * 100.0)` printed `-0 %` for any amount in (−0.005, 0),
    /// which parses to zero and prints `0 %`; `clap-validator`'s `param-conversions` fails on that
    /// whenever its grid lands in the sliver (`mxm_params::text::signed`).
    #[test]
    fn every_reading_survives_the_hosts_round_trip_a_rounded_zero_included() {
        let routes = Routes::default();
        let mut failures = Vec::new();
        for target in 0..TARGETS {
            for source in 0..SOURCES {
                let param = routes.group(target).amount_param(source);
                for normalised in [
                    0.0f32, 0.25, 0.4995, 0.4999, 0.49999, 0.5, 0.50001, 0.5001, 0.5005, 0.75, 1.0,
                ] {
                    let text = param.normalized_value_to_string(normalised, true);
                    let Some(back) = param.string_to_normalized_value(&text) else {
                        failures.push(format!("{}: {text} does not parse", param.name()));
                        continue;
                    };
                    let again = param.normalized_value_to_string(back, true);
                    if again != text {
                        failures.push(format!(
                            "{} at {normalised}: {text:?} came back {again:?}",
                            param.name()
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// **A control-map role never points at a dead route.**
    ///
    /// An absent pair contributes nothing whatever its amount holds, so a controller knob bound to
    /// the amount of a route Init does not wire would turn and do nothing. A presence may be bound
    /// either way, because turning it on is what creates the route. mxm-kit's
    /// `docs/code-review-notes.md` §7; mxm-mono-00's `plugins/mxm-mono-00/src/routes.rs` holds the
    /// same check.
    #[test]
    fn a_control_map_role_never_points_at_a_dead_route() {
        let text = include_str!("../control-map.json");
        let mut checked = 0;
        for (target, ids) in ROUTE_IDS.iter().enumerate() {
            for (source, (amount, presence)) in ids.iter().enumerate() {
                if text.contains(&format!("\"{presence}\"")) {
                    checked += 1;
                }
                if text.contains(&format!("\"{amount}\"")) {
                    checked += 1;
                    assert!(
                        crate::init::PATCH.route(target, source).present,
                        "the control map binds a role to {amount}, which Init does not wire: a \
                         knob on it would do nothing"
                    );
                }
            }
        }
        // `filter.env_amount` → the (Cutoff ← Envelope) route, and nothing else.
        assert_eq!(
            checked, 1,
            "the map names {checked} routing ids; it named one when this was written, so either \
             the map or this check has drifted"
        );
    }
}
