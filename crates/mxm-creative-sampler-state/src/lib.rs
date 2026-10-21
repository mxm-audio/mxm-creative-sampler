//! What an `mxm-creative-sampler` state means, with no plugin framework.
//!
//! **One crate, so the plugin and anything else that reads its state cannot disagree.** The web
//! tracker (`plans/plan-web-tracker.md` in newDAWn, W4) plays this sampler's DSP in a browser, and
//! reads and writes the plugin's own state inside tutti projects: the CLAP state stream newDAWn
//! stores, nice-plug's JSON of parameter values by id and the two persisted fields, `samples` and
//! `preset`. Everything that state means lives here, and the plugin takes it from here:
//!
//! - [`params`]: every parameter, its id, name, travel, default, unit and its text both ways,
//!   character for character as the plugin shows it;
//! - [`init`]: the compiled-in Init patch, its sources and its routes;
//! - [`sound`]: the plain values the DSP reads, and their mapping onto
//!   [`mxm_creative_sampler_dsp::Params`];
//! - [`payload`]: the embedded audio, canonical stereo i16 in base64, checked and decoded;
//! - [`import`]: a file decoded into that payload, and what the plugin decides when it arrives;
//! - [`state`]: the state itself, its JSON and its CLAP stream, in and out.
//!
//! No nice-plug and no egui: it builds for `wasm32-unknown-unknown`.

pub mod generate;
pub mod import;
pub mod init;
pub mod naming;
pub mod params;
pub mod payload;
pub mod range;
pub mod routes;
pub mod sound;
pub mod state;
pub mod text;
pub mod wav_loop;

pub use params::Param;
pub use state::{State, StateError, Values};

/// **The one literal of the product's name**: the plugin's `NAME` and `CLAP_ID` are [`NAME`] and
/// [`CLAP_ID`], and `bundler.toml` is the one external duplicate, pinned by the plugin's tests.
macro_rules! plugin_name {
    () => {
        "mxm-creative-sampler"
    };
}

/// The product's name.
pub const NAME: &str = plugin_name!();

/// The plugin's CLAP id, which a project names the device by: `clap:dk.mxm.mxm-creative-sampler`.
pub const CLAP_ID: &str = concat!("dk.mxm.", plugin_name!());

/// The plugin version a state written here carries, as the plugin writes its own
/// (`Plugin::VERSION`). Both crates take the workspace's version, which the plugin's tests hold.
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
