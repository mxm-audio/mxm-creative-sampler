//! The A0 capture for [`plans/plan-sampler-modulation-and-converter.md`]: what this instrument
//! costs and what it sounds like **before** the modulation conversion touches it.
//!
//! **Both numbers stop existing once the conversion starts**, which is the whole reason this runs
//! first. §1.6's cost gate is the owner's own requirement — *"best case the default should not use
//! more cycles than now"* — so the acceptance comparison is this plugin, this workload, before
//! against after. A framework-free DSP bench cannot see it: the per-sample `Params` rebuild and
//! every smoother advance are plugin-side, and that is exactly where the routing work lands.
//!
//! Run it release, on a quiet machine, and record the output in the plan's revision table:
//!
//! ```text
//! cargo run -p mxm-creative-sampler --release --example measure_baseline
//! ```
//!
//! The digests are **renders, not parameters**: each factory recipe is applied through the real
//! preset path and played, and the resulting audio is hashed. That is what "the recipes still
//! render as themselves" means, and a parameter comparison would not have caught the reorderings
//! the pilot's conversion actually produced.

use std::hint::black_box;
use std::time::Instant;

use mxm_creative_sampler::MxmCreativeSampler;
use mxm_creative_sampler::params::MxmCreativeSamplerParams;
use mxm_creative_sampler::preset::FACTORY_FILES;
use mxm_preset::format::Preset;
use nice_plug::params::Params;
use nice_plug::params::internals::ParamPtr;
use nice_plug::prelude::{ParamSetter, PluginApi, PluginState};

const RATE: f32 = 48_000.0;
/// Long enough for an attack, a sustain and the start of a decay on every recipe here.
///
/// **Four seconds, not one**, because `Held pad` has a 1.4 s attack and a 2 s decay: a one-second
/// render of it measured a peak of exactly zero, and a reference digest taken over silence would
/// have passed whatever the conversion did to it. `a_reference_render_must_contain_a_sound` below
/// is the guard that stops that recurring.
const FRAMES: usize = 4 * 48_000;

/// A host that applies what it is asked to, as `mxm-bucket-delay`'s examples already do.
struct ApplyingHost;

impl nice_plug::context::gui::GuiContextInner for ApplyingHost {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }
    unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
    unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, value: f32) {
        unsafe {
            param._internal_set_normalized_value(value);
            // A smoother left un-reset would ramp from the previous recipe's value and the render
            // would be a crossfade between two sounds rather than the sound being measured.
            param._internal_update_smoother(RATE, true);
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

/// A 64-bit digest of a render. Order-sensitive and exact — this is a bit-identity check, so a
/// tolerance would defeat its purpose.
fn digest(left: &[f32], right: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for value in left.iter().zip(right).flat_map(|(l, r)| [l, r]) {
        hash ^= value.to_bits() as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Plays one note through the plugin's own per-sample path and returns the audio.
fn render(plugin: &mut MxmCreativeSampler, key: u8, velocity: f32) -> (Vec<f32>, Vec<f32>) {
    prime_smoothers(&plugin.params_for_test());
    let mut left = vec![0.0; FRAMES];
    let mut right = vec![0.0; FRAMES];
    plugin.note_on_for_test(key, velocity);
    plugin.render_block_for_test(&mut left, &mut right);
    (left, right)
}

/// Primes every smoother, which is what a host does on `initialize`.
///
/// **Without this the whole harness measures silence.** A nice-plug smoother that has never been
/// updated returns zero rather than its parameter's value, so `master` and every layer `level`
/// read as zero and the render is digital silence — which also makes any timing taken over it
/// meaningless. Found the hard way: a factory recipe appeared to render nothing.
fn prime_smoothers(params: &MxmCreativeSamplerParams) {
    for (_id, ptr, _group) in params.param_map() {
        unsafe {
            ptr._internal_update_smoother(RATE, true);
        }
    }
}

fn main() {
    println!("mxm-creative-sampler — A0 baseline, before the modulation conversion");
    println!("{FRAMES} frames at {RATE} Hz per render\n");

    // ---- The default patch, which is the case the owner's cost requirement is about. ----
    //
    // §1.6: *the default patch is the one that must not regress*. It is also the case that stopped
    // being free on the pilot, where its own knobs becoming routes cost it 206 -> 263 ns before the
    // compaction work brought it back to 224.
    let mut plugin = MxmCreativeSampler::default();
    prime_smoothers(&plugin.params_for_test());
    let mut left = vec![0.0; FRAMES];
    let mut right = vec![0.0; FRAMES];
    plugin.note_on_for_test(60, 0.8);
    // One pass to warm the caches and the smoothers, then the measured pass.
    plugin.render_block_for_test(&mut left, &mut right);
    let started = Instant::now();
    plugin.render_block_for_test(&mut left, &mut right);
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "default patch, one voice : {:>8.1} ns/sample  ({:.2}% of one core realtime)",
        elapsed / FRAMES as f64 * 1.0e9,
        elapsed / (FRAMES as f64 / RATE as f64) * 100.0
    );
    black_box(&left);

    // ---- Full polyphony, which is the separate worst-case budget check. ----
    let mut plugin = MxmCreativeSampler::default();
    prime_smoothers(&plugin.params_for_test());
    for key in [48u8, 52, 55, 59, 62, 65, 69, 72] {
        plugin.note_on_for_test(key, 0.8);
    }
    plugin.render_block_for_test(&mut left, &mut right);
    let started = Instant::now();
    plugin.render_block_for_test(&mut left, &mut right);
    let elapsed = started.elapsed().as_secs_f64();
    println!(
        "default patch, eight     : {:>8.1} ns/sample  ({:.2}% of one core realtime)",
        elapsed / FRAMES as f64 * 1.0e9,
        elapsed / (FRAMES as f64 / RATE as f64) * 100.0
    );
    black_box(&left);

    // ---- The Init patch, rendered and hashed: what a fresh instance plays. ----
    //
    // Recorded beside the recipes because a change to how loops read must leave it alone, and a
    // recipe digest cannot say so: none of them is Init.
    let mut plugin = MxmCreativeSampler::default();
    let (left, right) = render(&mut plugin, 60, 0.8);
    println!(
        "\ninit patch render         digest {:016x}",
        digest(&left, &right)
    );

    // ---- The factory recipes, rendered and hashed. ----
    //
    // Applied through the **real** preset path — parse, resolve, write — rather than by poking
    // values in, so what is measured is the sound a player gets when they pick the recipe. The
    // designs in `preset.rs` are `#[cfg(test)]`; these shipped files are what the plugin loads.
    println!("\nfactory recipe renders (digest of the audio, not of the parameters):");
    for (name, text) in FACTORY_FILES {
        let mut plugin = MxmCreativeSampler::default();
        {
            let params = plugin.params_for_test();
            let preset = Preset::parse(text, mxm_creative_sampler::CLAP_ID)
                .unwrap_or_else(|why| panic!("{name}: {why:?}"));
            let (writes, problems) = preset.resolve(params.as_ref());
            assert!(problems.is_empty(), "{name}: {problems:?}");
            let host = ApplyingHost;
            let setter = ParamSetter::new(&host);
            for (_id, param, normalised) in writes {
                param.begin(&setter);
                param.set(&setter, normalised);
                param.end(&setter);
            }
        }
        let (left, right) = render(&mut plugin, 60, 0.8);
        let peak = left
            .iter()
            .chain(&right)
            .fold(0.0f32, |peak, v| peak.max(v.abs()));
        // **A silent reference is not a reference.** It would compare equal to anything the
        // conversion did, so it is a failure here rather than a row in the output.
        // **A silent reference is not a reference.** It would compare equal to anything the
        // conversion did, so it is a failure here rather than a row in the output.
        assert!(
            peak > 1.0e-4,
            "{name} rendered silence, so its digest would prove nothing"
        );
        println!(
            "  {name:<17} digest {:016x}  peak {peak:.6}",
            digest(&left, &right)
        );
    }
}
