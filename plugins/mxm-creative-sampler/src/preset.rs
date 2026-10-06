//! Asset-aware shared-preset integration for the pre-listening sampler.
//!
//! User saves carry `AssetPayload` in `Preset::state`. Factory files are recipes: `state = null`
//! preserves current A/B content. Init uses the same preservation policy for source-owned numeric
//! metadata and never creates a duplicate preset file.

use crate::asset::AssetField;
use crate::params::MxmCreativeSamplerParams;
use mxm_preset::{Instrument, PresetIdentity};
use std::sync::RwLock;

pub use mxm_preset::Library;

/// **The tempo syncs this plugin gained on 2026-09-25** (`plans/plan-tempo-sync-controls.md`). A
/// preset file written before them was written unsynced, so each loads off rather than keeping the
/// instance's sync, and without reporting a missing control.
pub(crate) const TEMPO_SYNC_IDS: &[&str] = &["lfo1sync", "lfo2sync", "agrainsync", "bgrainsync"];

impl Instrument for MxmCreativeSamplerParams {
    fn clap_id(&self) -> &'static str {
        crate::CLAP_ID
    }

    fn parameters(&self) -> Vec<(&'static str, &dyn mxm_preset::ErasedParam)> {
        let mut out: Vec<(&'static str, &dyn mxm_preset::ErasedParam)> = vec![
            ("areader", &self.a_reader),
            ("aroot", &self.a_root),
            ("atranspose", &self.a_transpose),
            ("astart", &self.a_start),
            ("aend", &self.a_end),
            ("aloopstart", &self.a_loop_start),
            ("aloopend", &self.a_loop_end),
            ("aloopcrossfade", &self.a_loop_crossfade),
            ("aloop", &self.a_loop),
            ("areverse", &self.a_reverse),
            ("alevel", &self.a_level),
            ("apan", &self.a_pan),
            ("ascanspeed", &self.a_scan_speed),
            ("agrainsize", &self.a_grain_size),
            ("agrainrate", &self.a_grain_rate),
            ("agrainsync", &self.a_grain_sync),
            ("apitchvar", &self.a_pitch_var),
            ("aposvar", &self.a_position_var),
            ("atiming", &self.a_timing),
            ("awidth", &self.a_width),
            ("areversechance", &self.a_reverse_chance),
            ("agrainshape", &self.a_grain_shape),
            ("breader", &self.b_reader),
            ("broot", &self.b_root),
            ("btranspose", &self.b_transpose),
            ("bstart", &self.b_start),
            ("bend", &self.b_end),
            ("bloopstart", &self.b_loop_start),
            ("bloopend", &self.b_loop_end),
            ("bloopcrossfade", &self.b_loop_crossfade),
            ("bloop", &self.b_loop),
            ("breverse", &self.b_reverse),
            ("blevel", &self.b_level),
            ("bpan", &self.b_pan),
            ("bscanspeed", &self.b_scan_speed),
            ("bgrainsize", &self.b_grain_size),
            ("bgrainrate", &self.b_grain_rate),
            ("bgrainsync", &self.b_grain_sync),
            ("bpitchvar", &self.b_pitch_var),
            ("bposvar", &self.b_position_var),
            ("btiming", &self.b_timing),
            ("bwidth", &self.b_width),
            ("breversechance", &self.b_reverse_chance),
            ("bgrainshape", &self.b_grain_shape),
            ("rate", &self.rate),
            ("converter", &self.converter),
            ("reconstruction", &self.reconstruction),
            ("input", &self.input),
            ("convtype", &self.converter_type),
            ("jitter", &self.jitter),
            ("cutoff", &self.cutoff),
            ("resonance", &self.resonance),
            ("filtermode", &self.filter_mode),
            ("drive", &self.drive),
            ("attack", &self.attack),
            ("decay", &self.decay),
            ("sustain", &self.sustain),
            ("release", &self.release),
            ("lforate", &self.lfo_rate),
            ("lfo1sync", &self.lfo1_sync),
            ("lfo1shape", &self.lfo_shape),
            ("lfo2rate", &self.lfo2_rate),
            ("lfo2sync", &self.lfo2_sync),
            ("lfo2shape", &self.lfo2_shape),
            ("master", &self.master),
            ("voicemode", &self.voice_mode),
            ("glide", &self.glide),
            ("bendrange", &self.bend_range),
        ];
        // **Every routing pair, appended after the fixed surface.** A preset is a full snapshot, so
        // a pair missing here is a route a saved patch silently forgets — which is why this walks
        // `ROUTE_IDS` rather than being typed out a second time.
        for (target, ids) in crate::routes::ROUTE_IDS.iter().enumerate() {
            let routes = self.routes.target(target);
            for (route, (amount, presence)) in routes.iter().zip(ids) {
                out.push((presence, route.present));
                out.push((amount, route.amount));
            }
        }
        out
    }

    fn identity(&self) -> &RwLock<PresetIdentity> {
        &self.preset
    }

    fn factory_files(&self) -> &'static [(&'static str, &'static str)] {
        FACTORY_FILES
    }

    fn default_missing_legacy_parameter(&self, id: &str) -> bool {
        TEMPO_SYNC_IDS.contains(&id)
    }

    fn capture_preset_state(&self) -> Option<serde_json::Value> {
        Some(self.assets.preset_state())
    }

    fn preset_state_fingerprint(&self) -> Option<u64> {
        Some(self.assets.fingerprint())
    }

    fn validate_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        if let Some(state) = state {
            AssetField::validate_preset_state(state)
        } else {
            // Factory recipes intentionally preserve whichever sources are currently loaded.
            Ok(())
        }
    }

    fn apply_preset_state(&self, state: Option<&serde_json::Value>) -> Result<(), String> {
        if let Some(state) = state {
            self.assets.apply_preset_state(state)
        } else {
            Ok(())
        }
    }

    fn commit_preset_state(&self) {
        self.assets.commit();
    }

    fn is_source_owned(&self, id: &str) -> bool {
        matches!(
            id,
            "aroot"
                | "astart"
                | "aend"
                | "aloopstart"
                | "aloopend"
                | "broot"
                | "bstart"
                | "bend"
                | "bloopstart"
                | "bloopend"
        )
    }
}

/// **Four recipes, not a bank.** Each one carries `state = null`, so it keeps whatever the player
/// has dropped and changes only how the instrument reads it: the S3 listening gate asks for a bass,
/// a pad, a hit and an evolving texture from the owner's own recordings, and flipping between four
/// treatments of one sound is the fastest way to hear whether Repitch, Stretch, Grain and Character
/// are actually different instruments. The authored bank of ≥50 categorised sounds with project
/// audio behind them is S4 work and waits for the listening verdict.
pub const FACTORY_FILES: &[(&str, &str)] = &[
    ("Long bass", include_str!("../presets/long-bass.json")),
    ("Held pad", include_str!("../presets/held-pad.json")),
    ("Struck hit", include_str!("../presets/struck-hit.json")),
    (
        "Drifting texture",
        include_str!("../presets/drifting-texture.json"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// **A project saved before the tempo syncs restores them Off** (`mxm_preset::add_switches_off`),
    /// whatever this instance had.
    #[test]
    fn an_older_state_restores_the_tempo_syncs_off() {
        use nice_plug::prelude::Plugin as _;
        let mut state = nice_plug::prelude::PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        };
        crate::MxmCreativeSampler::filter_state(&mut state);
        for id in TEMPO_SYNC_IDS {
            assert!(
                matches!(
                    state.params.get(*id),
                    Some(nice_plug::plugin::ParamValue::Bool(false))
                ),
                "{{id}} was not restored off"
            );
        }
    }

    /// **A preset saved before the tempo syncs loads them off, and cleanly** ([`TEMPO_SYNC_IDS`]).
    #[test]
    fn a_preset_from_before_the_tempo_syncs_loads_them_off() {
        let params = crate::params::MxmCreativeSamplerParams::default();
        let mut old = mxm_preset::Preset::init(&params);
        for id in TEMPO_SYNC_IDS {
            old.params.remove(*id);
        }
        let (writes, problems) = old.resolve(&params);
        assert!(problems.is_empty(), "{{problems:?}}");
        for id in TEMPO_SYNC_IDS {
            assert!(
                writes.iter().any(|(w, _, v)| w == id && *v == 0.0),
                "{{id}} was not written off"
            );
        }
    }

    use mxm_preset::{Category, Preset, Value};
    use nice_plug::prelude::Param;

    /// Factory designs in the parameters' own units: seconds, hertz, semitones, and a switch
    /// position for a stepped control. Keeping them here makes the bank reviewable without
    /// reverse-engineering each skewed range.
    ///
    /// **No source-owned parameter appears in any design.** A recipe preserves the loaded audio,
    /// and `Instrument::is_source_owned` keeps its root and extents from being written at all;
    /// naming one here would be a value that never arrives.
    pub type Design = (&'static str, Category, &'static [(&'static str, f32)]);

    const REPITCH: f32 = 0.0;
    const STRETCH: f32 = 1.0;
    const GRAIN: f32 = 2.0;
    const LOOP_OFF: f32 = 0.0;
    const LOOP_FORWARD: f32 = 1.0;
    const LOOP_ALTERNATE: f32 = 2.0;
    const FILTER_LOW: f32 = 1.0;
    const FILTER_BAND: f32 = 2.0;
    const POLY: f32 = 0.0;
    const LEGATO: f32 = 2.0;

    pub fn designs() -> &'static [Design] {
        DESIGNS
    }

    const DESIGNS: &[Design] = &[
        (
            "Long bass",
            Category::Bass,
            &[
                ("areader", REPITCH),
                ("aloop", LOOP_FORWARD),
                ("atranspose", -12.0),
                ("alevel", 1.0),
                ("converter", 0.18),
                ("input", 0.30),
                ("filtermode", FILTER_LOW),
                ("cutoff", 900.0),
                ("resonance", 0.30),
                ("drive", 0.32),
                ("mod_cutoff_env", 0.36),
                ("attack", 0.002),
                ("decay", 0.35),
                ("sustain", 0.58),
                ("release", 0.12),
                ("mod_amp_vel", 0.55),
                ("voicemode", LEGATO),
                ("glide", 0.06),
            ],
        ),
        (
            "Held pad",
            Category::Pad,
            &[
                ("areader", STRETCH),
                ("aloop", LOOP_ALTERNATE),
                ("ascanspeed", 0.16),
                ("breader", STRETCH),
                ("btranspose", 7.0),
                ("bscanspeed", -0.09),
                ("rate", 0.20),
                ("filtermode", FILTER_LOW),
                ("cutoff", 3_200.0),
                ("resonance", 0.18),
                ("mod_cutoff_env", 0.22),
                ("attack", 1.40),
                ("decay", 2.00),
                ("sustain", 0.84),
                ("release", 2.60),
                ("mod_amp_vel", 0.30),
                ("lforate", 0.11),
                ("mod_scana_lfo1", 0.34),
                ("mod_scanb_lfo1", 0.34),
                ("voicemode", POLY),
            ],
        ),
        (
            "Struck hit",
            Category::Percussion,
            &[
                ("areader", REPITCH),
                ("aloop", LOOP_OFF),
                ("alevel", 1.0),
                ("converter", 0.42),
                ("input", 0.38),
                ("filtermode", FILTER_BAND),
                ("cutoff", 5_600.0),
                ("resonance", 0.34),
                ("drive", 0.46),
                ("mod_cutoff_env", 0.62),
                ("attack", 0.000_5),
                ("decay", 0.22),
                ("sustain", 0.0),
                ("release", 0.18),
                ("mod_amp_vel", 0.85),
            ],
        ),
        (
            "Drifting texture",
            Category::Drone,
            &[
                ("areader", GRAIN),
                ("aloop", LOOP_FORWARD),
                ("agrainsize", 0.22),
                ("agrainrate", 18.0),
                ("apitchvar", 0.18),
                ("aposvar", 0.55),
                ("ascanspeed", 0.07),
                ("breader", GRAIN),
                ("btranspose", 12.0),
                ("bgrainsize", 0.10),
                ("bgrainrate", 40.0),
                ("bpitchvar", 0.42),
                ("bposvar", 0.30),
                ("bscanspeed", -0.19),
                ("rate", 0.30),
                ("reconstruction", 0.40),
                ("filtermode", FILTER_LOW),
                ("cutoff", 7_200.0),
                ("resonance", 0.14),
                ("attack", 0.62),
                ("decay", 1.50),
                ("sustain", 0.72),
                ("release", 1.90),
                ("lforate", 0.07),
                ("mod_scana_lfo1", 0.52),
                ("mod_scanb_lfo1", 0.52),
            ],
        ),
    ];

    /// A stepped parameter's normalised value from its position, or a float's from its own unit.
    fn normalised_for(params: &MxmCreativeSamplerParams, id: &str, plain: f32) -> f32 {
        let (_, param) = Instrument::parameters(params)
            .into_iter()
            .find(|(name, _)| *name == id)
            .unwrap_or_else(|| panic!("{id} is not a parameter"));
        match param.steps() {
            Some(steps) => {
                assert!(
                    plain >= 0.0 && plain <= steps as f32 && plain.fract() == 0.0,
                    "{id} has no position {plain}"
                );
                plain / steps as f32
            }
            None => float_normalised(params, id, plain),
        }
    }

    fn float_normalised(params: &MxmCreativeSamplerParams, id: &str, plain: f32) -> f32 {
        match id {
            "atranspose" | "btranspose" => params.a_transpose.preview_normalized(plain),
            "ascanspeed" | "bscanspeed" => params.a_scan_speed.preview_normalized(plain),
            "agrainsize" | "bgrainsize" => params.a_grain_size.preview_normalized(plain),
            "agrainrate" | "bgrainrate" => params.a_grain_rate.preview_normalized(plain),
            "apitchvar" | "bpitchvar" => params.a_pitch_var.preview_normalized(plain),
            "cutoff" => params.cutoff.preview_normalized(plain),
            "attack" => params.attack.preview_normalized(plain),
            "decay" => params.decay.preview_normalized(plain),
            "release" => params.release.preview_normalized(plain),
            "lforate" => params.lfo_rate.preview_normalized(plain),
            "glide" => params.glide.preview_normalized(plain),
            // **Every route amount is bipolar over -1..=1**, so a design's plain depth is not its
            // own normalised value the way a percentage is. Matched by prefix rather than listed,
            // because there are 55 of them and a list is what drifts.
            id if id.starts_with("mod_") => (plain + 1.0) / 2.0,
            "apan" | "bpan" => params.a_pan.preview_normalized(plain),
            // Everything else is a plain 0..1 percentage and is its own normalised value.
            _ => {
                assert!((0.0..=1.0).contains(&plain), "{id} is not a percentage");
                plain
            }
        }
    }

    pub fn generated(
        params: &MxmCreativeSamplerParams,
        name: &str,
        category: Category,
        overrides: &[(&str, f32)],
    ) -> Preset {
        let mut preset = Preset::init(params);
        preset.name = name.to_owned();
        preset.category = category;
        // A recipe preserves the loaded audio; `Preset::init` leaves `state` null, which is what
        // says so, and nothing here fills it in.
        for (id, plain) in overrides {
            let (_, param) = Instrument::parameters(params)
                .into_iter()
                .find(|(found, _)| found == id)
                .unwrap_or_else(|| panic!("{id} is not a parameter"));
            let value = normalised_for(params, id, *plain).clamp(0.0, 1.0);
            preset.params.insert(
                (*id).to_owned(),
                Value {
                    v: value,
                    text: param.format(value),
                },
            );
        }
        preset
    }

    fn meaningful_axes(left: &Preset, right: &Preset) -> usize {
        left.params
            .iter()
            .filter(|(id, value)| {
                (value.v - right.params.get(*id).expect("complete preset").v).abs() >= 0.06
            })
            .count()
    }

    #[test]
    fn the_factory_files_match_the_design() {
        let params = MxmCreativeSamplerParams::default();
        assert_eq!(FACTORY_FILES.len(), DESIGNS.len());
        for (name, category, overrides) in DESIGNS {
            let shipped = Preset::parse(
                FACTORY_FILES
                    .iter()
                    .find(|(found, _)| found == name)
                    .expect("listed")
                    .1,
                crate::CLAP_ID,
            )
            .expect("factory preset parses");
            assert_eq!(
                shipped,
                generated(&params, name, *category, overrides),
                "{name}"
            );
        }
    }

    #[test]
    fn every_recipe_is_complete_categorised_and_carries_no_audio_of_its_own() {
        let params = MxmCreativeSamplerParams::default();
        for (name, text) in FACTORY_FILES {
            let preset = Preset::parse(text, crate::CLAP_ID).expect(name);
            assert_ne!(preset.category, Category::Uncategorised, "{name}");
            assert!(preset.resolve(&params).1.is_empty(), "{name} is incomplete");
            assert!(
                preset.state.is_none(),
                "{name} embeds audio; a factory recipe preserves the loaded source"
            );
            // The recipe cannot crop or detune whatever is loaded.
            for (id, _, _) in preset.resolve(&params).0 {
                assert!(!params.is_source_owned(id), "{name} writes {id}");
            }
        }
    }

    /// Init is generated from `init::PATCH` through the CLAP defaults and has no writable file.
    /// The shared library protects factory origins; keeping Init out of this plugin's file list is
    /// what also makes it impossible to overwrite, rename or delete on disk.
    #[test]
    fn init_is_compiled_in_and_has_no_overwritable_file() {
        let params = MxmCreativeSamplerParams::default();
        assert!(
            FACTORY_FILES
                .iter()
                .all(|(name, _)| *name != mxm_preset::INIT_NAME)
        );
        assert_eq!(
            mxm_preset::factory(&params).first().expect("Init").name,
            mxm_preset::INIT_NAME
        );
    }

    #[test]
    fn every_factory_pair_is_a_different_treatment_of_the_same_sound() {
        let params = MxmCreativeSamplerParams::default();
        let presets: Vec<_> = DESIGNS
            .iter()
            .map(|(name, category, overrides)| generated(&params, name, *category, overrides))
            .collect();
        let init = Preset::init(&params);
        for (index, left) in presets.iter().enumerate() {
            let from_init = meaningful_axes(&init, left);
            assert!(
                from_init >= 6,
                "{} differs from Init on only {from_init} axes",
                left.name
            );
            for right in &presets[index + 1..] {
                let changed = meaningful_axes(left, right);
                assert!(
                    changed >= 6,
                    "{} and {} differ meaningfully on only {changed} axes",
                    left.name,
                    right.name
                );
            }
        }
    }

    #[test]
    fn every_parameter_is_declared_exactly_once() {
        let params = MxmCreativeSamplerParams::default();
        let declared = params.parameters();
        let mut ids: Vec<_> = declared.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "a parameter id is duplicated");
        // 68 fixed controls — four of them tempo syncs, 2026-09-25 — and 110 routing pairs: five targets by eleven sources, a presence and an
        // amount each. The routes dominate the surface and exist whether or not anyone uses them,
        // which is the cost the pair model trades for never re-pointing a stored value.
        assert_eq!(before, 178, "the provisional surface changed: {ids:?}");
    }

    #[test]
    fn init_preserves_only_root_and_source_extents() {
        let params = MxmCreativeSamplerParams::default();
        let preserved: Vec<_> = params
            .parameters()
            .into_iter()
            .filter_map(|(id, _)| params.is_source_owned(id).then_some(id))
            .collect();
        assert_eq!(
            preserved,
            [
                "aroot",
                "astart",
                "aend",
                "aloopstart",
                "aloopend",
                "broot",
                "bstart",
                "bend",
                "bloopstart",
                "bloopend",
            ]
        );
        assert!(!params.is_source_owned("aloop"));
        assert!(!params.is_source_owned("atranspose"));
    }

    #[test]
    fn factory_recipes_preserve_assets_but_user_capture_contains_them() {
        let params = MxmCreativeSamplerParams::default();
        assert!(params.validate_preset_state(None).is_ok());
        assert!(params.apply_preset_state(None).is_ok());
        let content = serde_json::json!({
            "schema": 1,
            "layers": [{
                "name": "inside-the-preset.wav",
                "sample_rate": 48_000,
                "frames": 1,
                "pcm": "AAAAAA=="
            }, null]
        });
        params.apply_preset_state(Some(&content)).unwrap();
        let captured =
            mxm_preset::Preset::capture("Portable", mxm_preset::Category::Uncategorised, &params);
        assert_eq!(captured.state.as_ref(), Some(&content));
        assert!(AssetField::validate_preset_state(captured.state.as_ref().unwrap()).is_ok());
        let round_trip = mxm_preset::Preset::parse(&captured.to_json(), crate::CLAP_ID).unwrap();
        assert_eq!(round_trip.state.as_ref(), Some(&content));
        assert_eq!(
            params.factory_files().len(),
            4,
            "the four pre-listening recipes; the authored bank waits for S4"
        );
    }
}

#[cfg(test)]
mod regenerate {
    use super::tests::*;

    #[test]
    #[ignore = "writes the factory preset files"]
    fn rewrite_factory_files() {
        let params = crate::params::MxmCreativeSamplerParams::default();
        for (name, category, overrides) in designs() {
            let slug = name.to_lowercase().replace(' ', "-");
            let path = format!("presets/{slug}.json");
            let json = generated(&params, name, *category, overrides).to_json();
            std::fs::write(&path, json + "\n").expect("write preset");
            println!("wrote {path}");
        }
    }
}
