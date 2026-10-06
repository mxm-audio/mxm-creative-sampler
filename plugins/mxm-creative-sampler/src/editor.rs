//! Rough but production-architecture MXM editor for the playable sampler slice.

use crate::asset::{Arrival, LoadState};
use crate::params::MxmCreativeSamplerParams;
use crate::telemetry::Telemetry;
use crate::{EditorTask, MxmCreativeSampler};
use egui::Ui;
use mxm_creative_sampler_dsp::Sample;
use mxm_ui::space::SPACE_5;
use mxm_ui::theme::Tokens;
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use nice_plug_egui::{EguiEditorState, NiceEguiApp, create_egui_editor};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The size the editor **opens** at — **derived, not chosen**: the quarter-4K budget hugged around
/// every page, which `tests::the_opening_size_is_the_budget_hugged` holds.
const REFERENCE: (u32, u32) = (1780, 1077);
pub(crate) const MINIMUM: (u32, u32) = (520, 420);
const WAVEFORM_POINTS: usize = 640;

pub type MxmCreativeSamplerEditor = nice_plug_egui::EguiEditor<MxmCreativeSamplerApp>;

pub fn create(
    params: Arc<MxmCreativeSamplerParams>,
    telemetry: Arc<Telemetry>,
    executor: AsyncExecutor<MxmCreativeSampler>,
) -> Option<MxmCreativeSamplerEditor> {
    let state = EguiEditorState::from_size(
        nice_plug::editor::dpi::LogicalSize::new(REFERENCE.0, REFERENCE.1),
        1.0,
    );
    create_egui_editor(
        state,
        nice_plug_egui::RepaintNotifier::new(),
        nice_plug_egui::EguiNiceSettings {
            title: crate::NAME.to_owned(),
            resize_hint: ResizeHint {
                size_constraints: nice_plug::editor::SizeConstraints::min_logical_size(
                    nice_plug::editor::dpi::LogicalSize::new(MINIMUM.0 as f32, MINIMUM.1 as f32),
                ),
                ..ResizeHint::RESIZABLE
            },
            ..Default::default()
        },
        MxmCreativeSamplerApp::new(params, telemetry, executor),
    )
}

pub struct MxmCreativeSamplerApp {
    params: Arc<MxmCreativeSamplerParams>,
    telemetry: Arc<Telemetry>,
    executor: AsyncExecutor<MxmCreativeSampler>,
    gui_context: Option<GuiContext>,
    selected_layer: usize,
    waveform: Vec<[f32; 2]>,
    waveform_revision: u64,
    waveform_layer: usize,
    observed_revision: u64,
    text_entry: HashMap<&'static str, Option<String>>,
    presets: mxm_preset::PresetUi,
    nav: mxm_ui::navigation::State,
    /// `0` for the musician pages, or [`mxm_ui::paging::PARAMETERS`] for the tabless developer
    /// surface CC 119 value 127 reaches. Transient editor state; nothing durable reads it.
    view: usize,
}

impl MxmCreativeSamplerApp {
    fn new(
        params: Arc<MxmCreativeSamplerParams>,
        telemetry: Arc<Telemetry>,
        executor: AsyncExecutor<MxmCreativeSampler>,
    ) -> Self {
        let handled = params.assets.handled();
        let presets = mxm_preset::PresetUi::new(params.as_ref());
        Self {
            params,
            telemetry,
            executor,
            gui_context: None,
            selected_layer: 0,
            waveform: Vec::with_capacity(WAVEFORM_POINTS),
            waveform_revision: u64::MAX,
            waveform_layer: usize::MAX,
            // **What was *handled*, not what is current.** Starting from the current revision means
            // a load that completed while no editor existed is never seen, so its defaults are
            // never written; starting from the handled one makes the first frame finish that work.
            observed_revision: handled,
            text_entry: HashMap::new(),
            presets,
            nav: mxm_ui::navigation::State::default(),
            view: 0,
        }
    }

    fn rebuild_waveform(
        &mut self,
        sample: Option<&mxm_creative_sampler_dsp::Sample>,
        revision: u64,
    ) {
        if self.waveform_revision == revision && self.waveform_layer == self.selected_layer {
            return;
        }
        self.waveform.clear();
        if let Some(sample) = sample {
            let frames = sample.frames();
            let bins = frames.len().min(WAVEFORM_POINTS);
            for bin in 0..bins {
                let start = bin * frames.len() / bins;
                let end = ((bin + 1) * frames.len() / bins)
                    .max(start + 1)
                    .min(frames.len());
                // **Both extremes of the bin, not the one signed peak.**
                //
                // A single signed value per bin draws one polyline through the middle of the
                // waveform, which for anything with a bipolar cycle — a saw above all — is a line
                // that jumps between the two rails and reads as a barcode rather than as a sound.
                // Min and max give the envelope the eye expects from a sample editor.
                let mut low = f32::INFINITY;
                let mut high = f32::NEG_INFINITY;
                for frame in &frames[start..end] {
                    let mono = (frame[0] + frame[1]) * 0.5;
                    low = low.min(mono);
                    high = high.max(mono);
                }
                self.waveform.push([low, high]);
            }
        }
        self.waveform_revision = revision;
        self.waveform_layer = self.selected_layer;
    }
}

impl NiceEguiApp for MxmCreativeSamplerApp {
    fn build(
        &mut self,
        context: egui::Context,
        gui_context: GuiContext,
        _frame: &mut nice_plug_egui::Frame,
    ) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        mxm_ui::theme::apply(&context);
        mxm_ui::typography::apply(&context);
        context.set_theme(mxm_ui::theme::preference());
        self.gui_context = Some(gui_context);
        // The default task is loading, even though category order still begins at Performance.
        mxm_ui::paging::editor::request_card(&context, mxm_ui::paging::Key(3));
        Ok(())
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut nice_plug_egui::Frame) {
        let Some(context) = self.gui_context.clone() else {
            return;
        };
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(40));
        let revision = self.params.assets.revision();
        let status = self.params.assets.status();
        if revision != self.observed_revision {
            match settle_action(revision, self.params.assets.handled(), &status) {
                SettleAction::Wait => {}
                action => {
                    self.observed_revision = revision;
                    settle(
                        self.params.as_ref(),
                        &context.param_setter(),
                        revision,
                        &status,
                        action,
                    );
                    // Scheduling on the GUI path requests a host callback even if a previously
                    // empty instrument was asleep. The task itself carries no state.
                    self.executor.execute_gui(EditorTask::Wake);
                }
            }
        }
        let bank = self.params.assets.bank();
        let snapshot = bank.read();
        self.rebuild_waveform(snapshot.samples[self.selected_layer].as_ref(), revision);
        let names = [
            snapshot.names[0].as_deref().unwrap_or(""),
            snapshot.names[1].as_deref().unwrap_or(""),
        ];
        let samples = [snapshot.samples[0].as_ref(), snapshot.samples[1].as_ref()];
        let params = Arc::clone(&self.params);
        let executor = self.executor.clone();
        let mut load = move |layer: usize, path: std::path::PathBuf| {
            params.assets.set_loading(layer, &path);
            executor.execute_background(EditorTask::Load {
                layer: layer as u8,
                path,
            });
        };
        // A Load pick finishes on its own thread (`sections::load_picker`) and is collected here. It
        // waits out an acquisition already in flight, as the button itself does.
        if !matches!(status, LoadState::Loading { .. })
            && let Some((layer, path)) = mxm_ui::offthread::take::<(usize, std::path::PathBuf)>(
                ui.ctx(),
                crate::sections::load_picker(),
            )
        {
            load(layer, path);
        }
        // Emptying a layer publishes directly — there are no parameter gestures to sequence behind
        // it, since nothing about the remaining layer changes.
        // **The callback publishes grains for one layer, and this is how it knows which.** The
        // selection is editor state; asking for it here keeps the audio thread from publishing a
        // layer nobody is looking at.
        self.telemetry.show_layer(self.selected_layer);
        let playhead = self.telemetry.playhead(self.selected_layer);
        let clearing = Arc::clone(&self.params);
        let mut clear = move |layer: usize| {
            if let Err(error) = clearing.assets.clear_layer(layer) {
                clearing.assets.fail(layer, error);
            }
        };
        // **A generated pick takes the same road as a dropped file**, down to announcing itself as
        // loading first: the render plus its encode, two decodes and a fingerprint is the same
        // shape of work as a decode, and the status is what disables the picker while it runs.
        let generating = Arc::clone(&self.params);
        let generate_executor = self.executor.clone();
        let mut generate = move |layer: usize, recipe: crate::generate::Recipe| {
            generating.assets.set_loading_named(layer, &recipe.name());
            generate_executor.execute_background(EditorTask::Generate {
                layer: layer as u8,
                recipe,
            });
        };
        panel(
            ui,
            self.params.as_ref(),
            &self.telemetry,
            &context.param_setter(),
            &mut self.text_entry,
            &mut self.presets,
            &mut self.nav,
            &mut self.view,
            &mut self.selected_layer,
            samples,
            names,
            &status,
            &self.waveform,
            playhead,
            &mut load,
            &mut clear,
            &mut generate,
        );
    }

    fn editor_closed(&mut self) {
        // **The same decision as a frame, with one thing taken away: there is no next frame.**
        // A `Wait` here would leave the revision staged for whenever an editor is opened again,
        // which is correct — the `handled` watermark is what makes a later editor finish the job —
        // so the two paths differ only in that this one cannot retry.
        let revision = self.params.assets.revision();
        let status = self.params.assets.status();
        let action = settle_action(revision, self.params.assets.handled(), &status);
        if let Some(context) = self.gui_context.clone() {
            settle(
                self.params.as_ref(),
                &context.param_setter(),
                revision,
                &status,
                action,
            );
        }
        self.gui_context = None;
    }
}

/// The whole editor surface, with every host and asset dependency passed in.
///
/// Separated from [`MxmCreativeSamplerApp::ui`] so the keyboard-cursor and paging checks can paint
/// the real panel headlessly. Anything that needs a `GuiContext`, a task executor or a lock on the
/// asset bank is resolved by the caller.
#[allow(clippy::too_many_arguments)]
pub fn panel(
    ui: &mut Ui,
    params: &MxmCreativeSamplerParams,
    telemetry: &Telemetry,
    setter: &ParamSetter<'_>,
    text_entry: &mut HashMap<&'static str, Option<String>>,
    presets: &mut mxm_preset::PresetUi,
    nav: &mut mxm_ui::navigation::State,
    view: &mut usize,
    selected_layer: &mut usize,
    samples: [Option<&Sample>; 2],
    names: [&str; 2],
    status: &LoadState,
    waveform: &[[f32; 2]],
    playhead: Option<f32>,
    load: &mut dyn FnMut(usize, PathBuf),
    clear: &mut dyn FnMut(usize),
    generate: &mut dyn FnMut(usize, crate::generate::Recipe),
) {
    let loading = matches!(status, LoadState::Loading { .. });
    let busy = text_entry.values().any(Option::is_some) || presets.holds_the_keyboard() || loading;
    mxm_ui::paging::editor::hold(ui.ctx(), busy);
    // **The developer channel, before the cursor moves.** A category request lands on that
    // category's first card; 127 is the tabless Parameters surface. The paging renderer defers either
    // while `hold` says another surface owns the keyboard.
    mxm_ui::paging::editor::developer_request(ui.ctx(), view, telemetry.take_view_request());
    if *view == mxm_ui::paging::PARAMETERS {
        // No cards, so no cursor: stopped, or an invisible cursor left from the musician page — the
        // master output's card included — would keep the arrows its sliders need.
        mxm_ui::navigation::stop(ui.ctx());
    } else {
        mxm_ui::navigation::paged_with_bar(ui.ctx(), nav, busy, &[crate::sections::MASTER]);
    }
    if let Some(open) = telemetry.take_browser_request() {
        presets.set_browser_open(open);
    }
    // Applied and never stored: a capture run must not rewrite the choice made in the app bar.
    if let Some(preference) = telemetry
        .take_theme_request()
        .and_then(mxm_ui::theme::from_index)
    {
        ui.ctx().set_theme(preference);
    }
    let tokens = tokens_for(ui);
    let peak = telemetry.take_peak();
    let clipped = telemetry.clipped();
    mxm_ui::AppBar::new(crate::NAME).show_with(
        ui,
        &tokens,
        |ui| {
            // **A preset is an acquisition too, so it waits for the one in flight.** Recalling
            // one rewrites the layer payloads and advances the asset revision, exactly as a load
            // does; taken while a background load is staged but not committed, the two interleave
            // and the editor answers one revision with the other's region and root. The source
            // card has been gated on `Loading` since the picker landed — this is the same door,
            // and it was left open because the card's gate never reached the app bar.
            ui.add_enabled_ui(!loading, |ui| {
                mxm_preset::ui::preset_row(ui, &tokens, params, setter, presets);
            });
            ui.label(
                egui::RichText::new(format!(
                    "{} voice{}",
                    telemetry.voices(),
                    if telemetry.voices() == 1 { "" } else { "s" }
                ))
                .color(tokens.text_secondary),
            );
        },
        |ui| {
            if mxm_ui::shell::level_meter(ui, &tokens, peak, clipped) {
                telemetry.clear_clip();
            }
            // **Design system §3.1 slot 6**: the master output control belongs beside the level and
            // clip indicator, not filed with voice allocation. It stays an ordinary host parameter
            // — automatable, and reachable by the keyboard cursor through `navigation::at` — so
            // only where it is drawn has changed.
            crate::sections::master(ui, &tokens, params, setter, text_entry);
            mxm_ui::shell::zoom_control(ui);
            mxm_ui::shell::editor_theme_control(ui);
        },
    );
    mxm_preset::ui::overlays(ui, &tokens, params, setter, presets);
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(tokens.canvas)
                .inner_margin(egui::Margin::same(SPACE_5 as i8)),
        )
        .show(ui, |ui| {
            if *view == mxm_ui::paging::PARAMETERS {
                parameters_view(ui, &tokens, params, setter, text_entry);
            } else {
                crate::sections::cards(
                    ui,
                    &tokens,
                    params,
                    setter,
                    text_entry,
                    selected_layer,
                    samples,
                    names,
                    status,
                    waveform,
                    playhead,
                    telemetry,
                    load,
                    clear,
                    generate,
                );
            }
        });
}

/// The developer Parameters surface: every parameter the preset system declares, in that order, as
/// a slider. It has no tab; developer CC 119 value 127 reaches it (`plugins/AGENTS.md`).
///
/// **Every parameter, including the 110 routing pairs** — a musician page draws an absent route as
/// nothing at all, so this is the one place a script can see and set all of them. A control keeps
/// its card's description, bipolar track and step where it has one, and a route gets a sentence of
/// its own.
fn parameters_view(
    ui: &mut Ui,
    tokens: &Tokens,
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    text_entry: &mut HashMap<&'static str, Option<String>>,
) {
    use crate::binding::Bound;
    let described = crate::sections::all_parameters(params);
    let entries: Vec<Bound<'_>> = mxm_preset::Instrument::parameters(params)
        .into_iter()
        .map(
            |(id, param)| match described.iter().find(|bound| bound.id == id) {
                Some(bound) => Bound {
                    id,
                    param,
                    description: bound.description,
                    panel: bound.panel.clone(),
                    bipolar: bound.bipolar,
                    stepped: bound.stepped,
                    law: bound.law,
                    details: bound.details,
                },
                None if id.starts_with("mod_") && id.ends_with("on") => {
                    Bound::new(id, param, "Whether this modulation route is present.")
                }
                None if id.starts_with("mod_") => {
                    Bound::new(id, param, "How far this route moves its target.").bipolar()
                }
                None => Bound::new(id, param, mxm_preset::ErasedParam::name(param)),
            },
        )
        .collect();
    mxm_ui::shell::scroll_list(ui).show(ui, |ui| {
        let columns = if ui.available_width() >= 1000.0 { 3 } else { 2 };
        let per_column = entries.len().div_ceil(columns);
        ui.columns(columns, |uis| {
            for (index, chunk) in entries.chunks(per_column).enumerate() {
                let Some(column) = uis.get_mut(index) else {
                    continue;
                };
                for entry in chunk {
                    entry.slider(column, tokens, setter, text_entry);
                }
            }
        });
    });
}

/// What to do about the revision the asset field is holding.
///
/// **Split out because the wrong answer here is silent, and because a frame and a close have to
/// give the same one.** Two earlier versions of this were wrong in the same direction. The first
/// committed unconditionally at the end of `editor_closed`; the second still committed and marked
/// handled while the status read `Loading`, which is the state that means *a payload is on its way
/// and has not landed* — so a revision that did owe its defaults could be published without them
/// and then recorded as settled, and nothing would ever go back for it. A pure function can be
/// asked about every case without a host, a GUI context or a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettleAction {
    /// The payload has landed and owes its region and root: write them, then publish.
    WriteThenCommit,
    /// Nothing is owed — a preset carries its own gestures — but a staged revision wants
    /// publishing. Publication is still named, so a payload that arrived after this decision is
    /// left for the frame that can see it.
    Commit,
    /// **A load is in flight.** Touch nothing: not the commit, not the `handled` watermark, not
    /// the observed revision. The status cannot say which revision it describes, so a `Ready`
    /// revision whose status has since been overwritten by a new `Loading` would otherwise be
    /// settled blind. Whatever is staged is superseded by the load that is coming — staging twice
    /// before a commit replaces the revision nobody has heard — so nothing is lost by waiting.
    Wait,
}

fn settle_action(revision: u64, handled: u64, status: &LoadState) -> SettleAction {
    match status {
        LoadState::Loading { .. } => SettleAction::Wait,
        LoadState::Ready { .. } if revision != handled => SettleAction::WriteThenCommit,
        _ => SettleAction::Commit,
    }
}

/// Carry out [`settle_action`]'s decision.
///
/// **Every write happens before the commit, root included.** Root used to be written after it, on
/// the reasoning that deciding a pitch needs an audible sample — but the sample is *staged*, not
/// absent, and `with_staged` reads it. Publishing first left a window of one or more audio
/// callbacks in which the new source played at `NEUTRAL_ROOT`, C4, while every generated source is
/// C3: a note held across a pick was heard an octave out for that window. That is the very failure
/// the region defaults are ordered against, so the root belongs on the same side of the barrier.
fn settle(
    params: &MxmCreativeSamplerParams,
    setter: &ParamSetter<'_>,
    revision: u64,
    status: &LoadState,
    action: SettleAction,
) {
    match action {
        SettleAction::WriteThenCommit => {
            let LoadState::Ready { layer, name } = status else {
                return;
            };
            // **The record this revision was staged with, and no other.** A later stage replaces the
            // record together with its revision, so a settle for a revision that is no longer staged
            // writes nothing and publishes nothing — the frame that sees the newer one settles it.
            let Some(arrival) = params.assets.arrival(revision) else {
                return;
            };
            // **Resolved, then written once.** Every source-owned value is decided first — from the
            // staged snapshot, because the point is to decide before anyone hears it — and then each
            // is one complete gesture. The whole-file defaults are no longer written and overwritten.
            let arrived = params
                .assets
                .with_staged(|snapshot| {
                    resolve_arrival(arrival, name, snapshot.samples[*layer].as_ref())
                })
                .unwrap_or_else(|| resolve_arrival(arrival, name, None));
            write_arrival(params, *layer, setter, &arrived);
            params.assets.mark_handled(revision);
            params.assets.commit_revision(revision);
        }
        SettleAction::Commit => {
            params.assets.commit_revision(revision);
        }
        SettleAction::Wait => {}
    }
}

/// Every source-owned value an arrival writes, decided before any of it is written.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Arrived {
    /// Normalised Loop start and Loop end, or `None` for the parameters' own whole-file defaults.
    loop_points: Option<(f32, f32)>,
    /// The loop mode the source asks for, or `None` to leave the layer's.
    loop_mode: Option<crate::params::LoopChoice>,
    /// The crossfade in seconds the source asks for, or `None` to leave the layer's.
    crossfade: Option<f32>,
    /// Root, as a MIDI note, fractional.
    root: f32,
}

/// What an arrival writes (`plans/plan-sampler-wav-loop-import.md` §1.2, §1.3).
///
/// **A generated source is built to loop**: every partial in [`crate::generate`] takes a whole number
/// of cycles so the buffer joins to itself. Publishing one into a layer whose loop is off made the
/// instrument contradict its own design — Repitch and Stretch played the buffer once and stopped while
/// Grain kept sounding, because it never reads the loop — which is how it was reported. So it arrives
/// Forward over the whole buffer.
///
/// **A file that carries a loop arrives looping at its own points**, in the mode its chunk names
/// (owner, 2026-09-14: *they were made for looping*). **A file without one keeps the layer's loop
/// mode and crossfade** — a drum hit that suddenly repeats is a worse defect than a waveform that
/// does not — and its loop points go to the whole file, as every import's do.
///
/// **Both looping arrivals come with no crossfade.** A loop made to close is only harmed by a fade,
/// which moves its return point off a whole cycle and fades against a cycle offset; the knob is
/// there for a loop that was not made to close.
///
/// **Root: the loop's own pitch, then the key in the name, then C4** (owner, 2026-09-14, D4). The loop
/// is measured on the task thread when the file is decoded, from the canonical audio a layer plays,
/// exact to the cent, and only for a loop
/// that repeats fast enough to be a pitch (`wav_loop::loop_pitch`). The name gives a pitch class and
/// the audio its octave ([`root_in_name`]). Neither, and Root lands on [`NEUTRAL_ROOT`].
fn resolve_arrival(
    arrival: Arrival,
    name: &str,
    sample: Option<&mxm_creative_sampler_dsp::Sample>,
) -> Arrived {
    use crate::params::LoopChoice;
    let named = || root_in_name(name, sample);
    match arrival {
        Arrival::Generated => Arrived {
            loop_points: None,
            loop_mode: Some(LoopChoice::Forward),
            crossfade: Some(0.0),
            root: named().unwrap_or(NEUTRAL_ROOT),
        },
        Arrival::File(None) => Arrived {
            loop_points: None,
            loop_mode: None,
            crossfade: None,
            root: named().unwrap_or(NEUTRAL_ROOT),
        },
        Arrival::File(Some(found)) => {
            let last = found.frames.saturating_sub(1).max(1) as f32;
            Arrived {
                loop_points: Some((found.start as f32 / last, found.end as f32 / last)),
                loop_mode: Some(if found.alternate {
                    LoopChoice::Alternate
                } else {
                    LoopChoice::Forward
                }),
                crossfade: Some(0.0),
                root: found.root.or_else(named).unwrap_or(NEUTRAL_ROOT),
            }
        }
    }
}

/// Writes an arrival: **one complete host gesture per source-owned parameter**, each carrying its final
/// value — the region, both loop points, Root, and the loop mode and crossfade where the arrival sets
/// them.
fn write_arrival(
    params: &MxmCreativeSamplerParams,
    layer: usize,
    setter: &nice_plug::prelude::ParamSetter<'_>,
    arrived: &Arrived,
) {
    let (start, end, loop_start, loop_end, loop_mode, crossfade, root) = if layer == 0 {
        (
            &params.a_start,
            &params.a_end,
            &params.a_loop_start,
            &params.a_loop_end,
            &params.a_loop,
            &params.a_loop_crossfade,
            &params.a_root,
        )
    } else {
        (
            &params.b_start,
            &params.b_end,
            &params.b_loop_start,
            &params.b_loop_end,
            &params.b_loop,
            &params.b_loop_crossfade,
            &params.b_root,
        )
    };
    let write = |param: &dyn mxm_preset::ErasedParam, normalised: f32| {
        param.begin(setter);
        param.set(setter, normalised);
        param.end(setter);
    };
    write(start, mxm_preset::ErasedParam::default_normalised(start));
    write(end, mxm_preset::ErasedParam::default_normalised(end));
    let (from, to) = arrived.loop_points.unwrap_or((
        mxm_preset::ErasedParam::default_normalised(loop_start),
        mxm_preset::ErasedParam::default_normalised(loop_end),
    ));
    write(loop_start, from);
    write(loop_end, to);
    // Asked for by value rather than by a hand-written 0.5, so adding a fourth loop mode cannot
    // silently re-point this at the wrong one.
    if let Some(mode) = arrived.loop_mode {
        write(loop_mode, loop_mode.preview_normalized(mode));
    }
    if let Some(seconds) = arrived.crossfade {
        write(crossfade, crossfade.preview_normalized(seconds));
    }
    // **Root does not go back to its parameter default, because A's default is the init saw's own
    // pitch.** `a_root` starts at C3 so a fresh instance is in tune with the generated saw, and
    // `b_root` at C4 because layer B has no saw to be in tune with. That is right until a file is
    // dropped: the saw is gone, C3 means nothing to the new audio, and the same file dropped on A
    // and on B came out an octave apart.
    write(root, arrived.root / 127.0);
}

/// Where Root lands when an arrival says nothing better: C4, middle of the keyboard, and the
/// convention Root itself displays, so an import is at least consistent.
///
/// Not either layer's parameter default — see [`write_arrival`].
const NEUTRAL_ROOT: f32 = 60.0;

/// Root from the key in the file's name, when the name carries one.
///
/// **A drop should arrive in tune when the file says how.** About one file in seven in a commercial
/// library ends its name with a key. A file's `smpl` chunk comes first when its loop is a pitch
/// ([`resolve_arrival`]); in that library the chunk's unity note is absent or set to zero — see
/// [`crate::naming`] for the survey.
///
/// The name gives a pitch class and the audio gives the octave, because a name almost never carries
/// one. That pairing is also what makes this better than either half: the class corrects the
/// detector's whole-note mistakes, and the detector keeps the answer in the register the sample
/// actually sits in.
///
/// A name with no key says nothing. **Nothing is guessed from the audio alone** — a detector's answer
/// is confident on a bass note and wrong on a chord, and a wrong Root retunes the whole keyboard.
/// That is what the Auto button is for: asked, not assumed. The one narrowing is a loop the file
/// carries, whose length states the pitch (`wav_loop::loop_pitch`).
fn root_in_name(name: &str, sample: Option<&mxm_creative_sampler_dsp::Sample>) -> Option<f32> {
    let stem = std::path::Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    // **The name is consulted before the audio, so an unpitched sample is never analysed.** Plenty
    // of music is made from material that has no pitch at all, and running a detector over a drum
    // loop on every drop is work whose answer is thrown away — or worse, kept. Detection happens
    // only when a name has claimed a key and left out the octave, which is the one question the
    // audio is actually being asked.
    let tag = crate::naming::key_in_name(stem)?;
    let heard = match tag.octave {
        // A name that carries its octave is already a note; there is nothing to listen for.
        Some(_) => None,
        None => sample.and_then(|sample| mxm_creative_sampler_dsp::detect_root(sample, 0.0)),
    };
    Some(crate::naming::root_from_key(tag, heard))
}

fn tokens_for(ui: &Ui) -> Tokens {
    if ui.visuals().dark_mode {
        mxm_ui::DARK
    } else {
        mxm_ui::LIGHT
    }
}

#[cfg(test)]
// The shared paging support is `include!`d whole and this editor uses only its `verify`; the
// opening-size half is dead here rather than wrong.
#[allow(dead_code)]
mod tests {
    use super::*;
    use crate::sections;
    use mxm_preset::PresetUi;

    use mxm_plugin_test::keyboard_checks;

    /// The coverage check's reveal hook, which has nothing to reveal.
    ///
    /// It used to open Character's "Details" so the four stages behind it were reachable. That
    /// disclosure went when the stages became a card of their own, and the body kept setting it
    /// anyway — writing a key nothing reads, under a comment saying it was inert. A no-op says the
    /// same thing without leaving a name that no longer exists in the source. This editor's one
    /// collapse, an empty layer's source detail, is opened by handing the check a loaded sample
    /// rather than by a keystroke. Delete the constant with the hook when the helper stops taking
    /// one.
    const REVEAL: fn(&egui::Context) = |_| {};

    fn silence(frames: usize) -> Sample {
        Sample::new(vec![[0.0, 0.0]; frames], 48_000.0).expect("a valid sample")
    }

    /// What the panel checks paint: both layers holding a sample, `layer` selected, nothing loading.
    fn both_loaded(layer: usize) -> sections::Shown<'static> {
        sections::Shown {
            layer,
            loaded: [true, true],
            names: ["a.wav", "b.wav"],
            status: &LoadState::Idle,
        }
    }

    /// The ids only reachable while that layer is the selected one. The Source and Reader cards
    /// show one layer at a time by design — the brief collapses the unselected layer's detail —
    /// so a single pass cannot see both, and pretending otherwise would mean asserting `Within`
    /// and proving nothing.
    fn layer_only(prefix: char) -> Vec<String> {
        [
            "root",
            "start",
            "end",
            "loopstart",
            "loopend",
            "loopcrossfade",
            "reader",
            "loop",
            "reverse",
            "transpose",
            "scanspeed",
            "grainsize",
            "grainrate",
            "grainsync",
            "pitchvar",
            "posvar",
            "timing",
            "width",
            "reversechance",
            "grainshape",
            "level",
            "pan",
        ]
        .into_iter()
        .map(|suffix| format!("{prefix}{suffix}"))
        .collect()
    }

    fn reachable_with(selected: usize) -> Vec<String> {
        let params = MxmCreativeSamplerParams::default();
        let hidden = layer_only(if selected == 0 { 'b' } else { 'a' });
        // **A scan-speed stack is a layer's control like the knob above it.** The stacks moved onto
        // the Playback card, under the Scan speed they move, and that card shows one layer at a
        // time — so the other layer's routes are not painted, exactly as its `scanspeed` is not.
        let hidden_stack = if selected == 0 {
            "mod_scanb_"
        } else {
            "mod_scana_"
        };
        mxm_preset::Instrument::parameters(&params)
            .into_iter()
            .map(|(id, _)| id.to_owned())
            .filter(|id| !hidden.contains(id))
            .filter(|id| !id.starts_with(hidden_stack))
            .collect()
    }

    // The shared support also carries the opening-size helpers, which this editor does not use.
    use mxm_plugin_test::{opening_size, paging_checks};

    /// **Every card fits its page at every size the editor can be at.**
    ///
    /// Seven plugins run this shared check and the newest instrument never got wired into it, which
    /// is why a card clipping at the window edge reached the owner rather than a test.
    /// **A generated source arrives looping; a dropped file keeps the loop it found.**
    ///
    /// Reported by the owner: picking Supersaw and playing it under Repitch or Stretch gave one pass
    /// and silence, while Grain kept sounding. That asymmetry is the diagnosis — **Grain never reads
    /// the loop at all**, because it schedules grains inside the region, so only the two readers that
    /// wrap were affected. Nothing in the publication path had ever written the loop mode, so a
    /// generated source inherited whatever the layer was left on; it only looked fine because a
    /// fresh instance defaults to Forward.
    ///
    /// The generated bank exists to loop — every partial takes a whole number of cycles for that one
    /// reason — so it asks for the loop, and for no crossfade, which on a loop built to close can
    /// only move the return point off a whole cycle. A file must not be forced either way: a drum hit
    /// that suddenly repeats is worse than a waveform that does not, and its crossfade is the
    /// player's.
    #[test]
    fn a_generated_source_arrives_looping_and_a_dropped_file_does_not() {
        use nice_plug::params::InternalParamMut;

        let check = |generated: bool| {
            let params = MxmCreativeSamplerParams::default();
            // Start from the state the report came from: the loop switched off. And a crossfade
            // already on the layer, so the arrival has something to keep or to clear.
            unsafe {
                params
                    .a_loop
                    ._internal_set_plain_value(crate::params::LoopChoice::Off);
                params.a_loop_crossfade._internal_set_plain_value(0.25);
            };
            if generated {
                params
                    .assets
                    .generate_source(0, crate::generate::Recipe::Supersaw)
                    .expect("staging a recipe failed");
            } else {
                // No file to decode in a unit test, so stage the same bytes and record the arrival a
                // file with no loop makes — this is about what the publication said about its loop.
                params
                    .assets
                    .generate_source(0, crate::generate::Recipe::Supersaw)
                    .expect("staging failed");
                params.assets.set_arrival_for_test(Arrival::File(None));
            }
            // **An applying host, not the counting one.** `Recorder` records that a gesture
            // happened; this asserts what the gesture *did*, so the write has to land.
            struct Applying;
            impl nice_plug::context::gui::GuiContextInner for Applying {
                // A test double has no host to ask for a restart (nice-plug 0.4).
                fn request_restart(&self) {}
                fn plugin_api(&self) -> nice_plug::prelude::PluginApi {
                    nice_plug::prelude::PluginApi::Clap
                }
                unsafe fn raw_begin_set_parameter(
                    &self,
                    _p: nice_plug::params::internals::ParamPtr,
                ) {
                }
                unsafe fn raw_set_parameter_normalized(
                    &self,
                    p: nice_plug::params::internals::ParamPtr,
                    v: f32,
                ) {
                    unsafe { p._internal_set_normalized_value(v) };
                }
                unsafe fn raw_end_set_parameter(&self, _p: nice_plug::params::internals::ParamPtr) {
                }
                fn get_state(&self) -> nice_plug::prelude::PluginState {
                    nice_plug::prelude::PluginState {
                        version: String::new(),
                        params: Default::default(),
                        fields: Default::default(),
                    }
                }
                fn set_state(&self, _: nice_plug::prelude::PluginState) {}
            }
            let host = Applying;
            let setter = ParamSetter::new(&host);
            let revision = params.assets.revision();
            settle(
                &params,
                &setter,
                revision,
                &params.assets.status(),
                SettleAction::WriteThenCommit,
            );
            (params.a_loop.value(), params.a_loop_crossfade.value())
        };

        let (loop_mode, crossfade) = check(true);
        assert_eq!(
            loop_mode,
            crate::params::LoopChoice::Forward,
            "a generated source did not turn its layer's loop on"
        );
        assert_eq!(
            crossfade, 0.0,
            "a generated source kept a crossfade that can only harm a loop built to close"
        );
        let (loop_mode, crossfade) = check(false);
        assert_eq!(
            loop_mode,
            crate::params::LoopChoice::Off,
            "a dropped file was forced to loop"
        );
        assert!(
            (crossfade - 0.25).abs() < 1.0e-5,
            "a dropped file lost the layer's crossfade: {crossfade}"
        );
    }

    /// A host that applies every write and records it by parameter name, **without settling the
    /// smoother** — the value lands and the smoother ramps toward it, as the wrapper does.
    #[derive(Default)]
    struct Ledger {
        begins: std::sync::Mutex<Vec<String>>,
        sets: std::sync::Mutex<Vec<(String, f32)>>,
        ends: std::sync::Mutex<Vec<String>>,
    }

    impl nice_plug::context::gui::GuiContextInner for Ledger {
        // A test double has no host to ask for a restart (nice-plug 0.4).
        fn request_restart(&self) {}
        fn plugin_api(&self) -> nice_plug::prelude::PluginApi {
            nice_plug::prelude::PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, param: nice_plug::params::internals::ParamPtr) {
            let name = unsafe { param.name() }.to_owned();
            self.begins.lock().unwrap().push(name);
        }
        unsafe fn raw_set_parameter_normalized(
            &self,
            param: nice_plug::params::internals::ParamPtr,
            value: f32,
        ) {
            unsafe {
                param._internal_set_normalized_value(value);
                param._internal_update_smoother(48_000.0, false);
            }
            let name = unsafe { param.name() }.to_owned();
            self.sets.lock().unwrap().push((name, value));
        }
        unsafe fn raw_end_set_parameter(&self, param: nice_plug::params::internals::ParamPtr) {
            let name = unsafe { param.name() }.to_owned();
            self.ends.lock().unwrap().push(name);
        }
        fn get_state(&self) -> nice_plug::prelude::PluginState {
            nice_plug::prelude::PluginState {
                version: String::new(),
                params: Default::default(),
                fields: Default::default(),
            }
        }
        fn set_state(&self, _: nice_plug::prelude::PluginState) {}
    }

    /// A loop a staged generated buffer can carry: frames 1 000..2 000, back and forth, 25 cents flat.
    fn a_loop() -> crate::wav_loop::FileLoop {
        crate::wav_loop::FileLoop {
            start: 1_000,
            end: 2_000,
            alternate: true,
            frames: crate::generate::FRAMES as u32,
            unity: Some(60.0),
            root: Some(59.75),
        }
    }

    /// Stages a source into layer A that says `arrival` about its loop.
    fn staged(params: &MxmCreativeSamplerParams, arrival: Arrival) {
        params
            .assets
            .generate_source(0, crate::generate::Recipe::Supersaw)
            .expect("staging a source failed");
        params.assets.set_arrival_for_test(arrival);
    }

    /// **Each source-owned parameter an arrival writes is written once, as one balanced gesture, with
    /// its final value** (`plans/plan-sampler-wav-loop-import.md` §1.2). The arrival used to write the
    /// whole-file loop defaults and then overwrite them, and Root twice when a name carried a key.
    #[test]
    fn an_arrival_writes_each_parameter_once_with_its_final_value() {
        use crate::params::LoopChoice;
        let region = ["A Start", "A End", "A Loop start", "A Loop end", "A Root"];
        let looping = [
            "A Start",
            "A End",
            "A Loop start",
            "A Loop end",
            "A Loop",
            "A Loop crossfade",
            "A Root",
        ];
        for (arrival, wanted) in [
            (Arrival::File(Some(a_loop())), &looping[..]),
            (Arrival::File(None), &region[..]),
            (Arrival::Generated, &looping[..]),
        ] {
            let params = MxmCreativeSamplerParams::default();
            staged(&params, arrival);
            let host = Ledger::default();
            settle(
                &params,
                &ParamSetter::new(&host),
                params.assets.revision(),
                &params.assets.status(),
                SettleAction::WriteThenCommit,
            );
            let sorted = |mut names: Vec<String>| {
                names.sort();
                names
            };
            let wanted = sorted(wanted.iter().map(|name| (*name).to_owned()).collect());
            let sets = host.sets.lock().unwrap().clone();
            assert_eq!(
                sorted(sets.iter().map(|(name, _)| name.clone()).collect()),
                wanted,
                "{arrival:?} did not write each of its parameters exactly once"
            );
            assert_eq!(
                sorted(host.begins.lock().unwrap().clone()),
                wanted,
                "{arrival:?}: begins do not match the writes"
            );
            assert_eq!(
                sorted(host.ends.lock().unwrap().clone()),
                wanted,
                "{arrival:?}: ends do not match the writes"
            );
        }

        // What a looped file arrives with.
        let params = MxmCreativeSamplerParams::default();
        staged(&params, Arrival::File(Some(a_loop())));
        let host = Ledger::default();
        settle(
            &params,
            &ParamSetter::new(&host),
            params.assets.revision(),
            &params.assets.status(),
            SettleAction::WriteThenCommit,
        );
        let last = (crate::generate::FRAMES - 1) as f32;
        assert_eq!(params.a_loop.value(), LoopChoice::Alternate);
        assert_eq!(params.a_loop_crossfade.unmodulated_plain_value(), 0.0);
        assert_eq!(
            (params.a_loop_start.unmodulated_plain_value() * last).round(),
            1_000.0
        );
        assert_eq!(
            (params.a_loop_end.unmodulated_plain_value() * last).round(),
            2_000.0
        );
        assert!(
            (params.a_root.unmodulated_plain_value() - 59.75).abs() < 1.0e-3,
            "Root arrived at {}, not the loop's 59.75",
            params.a_root.unmodulated_plain_value()
        );
    }

    /// **An arrival record belongs to its revision.** A file with a loop, then a file without, both
    /// staged before any settle: the second settles to today's arrival — the whole-file loop points
    /// and no loop mode, crossfade or loop pitch — and a settle for the first, now stale, writes
    /// nothing at all.
    #[test]
    fn an_arrival_record_belongs_to_its_revision() {
        let params = MxmCreativeSamplerParams::default();
        staged(&params, Arrival::File(Some(a_loop())));
        let first = params.assets.revision();
        staged(&params, Arrival::File(None));
        let second = params.assets.revision();
        let status = params.assets.status();

        let stale = Ledger::default();
        settle(
            &params,
            &ParamSetter::new(&stale),
            first,
            &status,
            SettleAction::WriteThenCommit,
        );
        assert!(
            stale.sets.lock().unwrap().is_empty(),
            "a settle for a superseded revision wrote {:?}",
            stale.sets.lock().unwrap()
        );

        let current = Ledger::default();
        settle(
            &params,
            &ParamSetter::new(&current),
            second,
            &status,
            SettleAction::WriteThenCommit,
        );
        let sets = current.sets.lock().unwrap().clone();
        let written = |name: &str| {
            sets.iter()
                .find(|(wrote, _)| wrote == name)
                .map(|(_, value)| *value)
        };
        assert_eq!(written("A Loop start"), Some(0.0));
        assert_eq!(written("A Loop end"), Some(1.0));
        assert_eq!(
            written("A Loop"),
            None,
            "a file without a loop set the mode"
        );
        assert_eq!(
            written("A Loop crossfade"),
            None,
            "a file without a loop set the crossfade"
        );
        assert!(
            written("A Root").is_some_and(|root| (root * 127.0 - 59.75).abs() > 0.1),
            "the superseded loop's pitch was written"
        );
    }

    /// **A new source's geometry is final on the first callback that hears it** (§1.2). The arrival's
    /// gestures move targets, and each is smoothed: without settling, the first 20 ms of the new audio
    /// played through a ramp from the previous source's crop, loop and crossfade. Checked through a
    /// host that ramps as the wrapper does, after an import and after a preset load, with a control
    /// that shows the check can see a ramp at all.
    #[test]
    fn a_new_source_is_final_on_the_first_callback_that_hears_it() {
        use nice_plug::params::InternalParamMut;
        let mut plugin = crate::MxmCreativeSampler::default();
        let params = plugin.params_for_test();
        for (_id, ptr, _group) in params.param_map() {
            unsafe { ptr._internal_update_smoother(48_000.0, true) };
        }
        // A crop and a crossfade already on layer A, settled and heard.
        for (param, plain) in [(&params.a_start, 0.2f32), (&params.a_loop_crossfade, 0.25)] {
            unsafe { param._internal_set_plain_value(plain) };
            param.smoothed.reset(plain);
        }
        let mut block = ([0.0f32; 64], [0.0f32; 64]);
        plugin.render_block_for_test(&mut block.0, &mut block.1);
        let ramping = |params: &MxmCreativeSamplerParams| {
            [
                ("start", &params.a_start),
                ("end", &params.a_end),
                ("loop start", &params.a_loop_start),
                ("loop end", &params.a_loop_end),
                ("crossfade", &params.a_loop_crossfade),
                ("root", &params.a_root),
            ]
            .into_iter()
            .filter(|(_, param)| (param.smoothed.previous_value() - param.value()).abs() > 1.0e-6)
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
        };
        let mut one = ([0.0f32; 1], [0.0f32; 1]);

        staged(&params, Arrival::File(Some(a_loop())));
        let host = Ledger::default();
        settle(
            &params,
            &ParamSetter::new(&host),
            params.assets.revision(),
            &params.assets.status(),
            SettleAction::WriteThenCommit,
        );
        plugin.render_block_for_test(&mut one.0, &mut one.1);
        assert!(
            ramping(&params).is_empty(),
            "after an import, still ramping on the first sample: {:?}",
            ramping(&params)
        );

        // The control: a crop moved with no new source still ramps, so the check can see one.
        let host = Ledger::default();
        mxm_preset::ErasedParam::set(&params.a_start, &ParamSetter::new(&host), 0.35);
        plugin.render_block_for_test(&mut one.0, &mut one.1);
        assert_eq!(
            ramping(&params),
            ["start"],
            "a moved crop did not ramp, so this check measures nothing"
        );

        // A preset load replaces the audio, and settles the same way.
        let state = params.assets.preset_state();
        params
            .assets
            .apply_preset_state(&state)
            .expect("re-staging the patch's audio");
        params.assets.commit();
        plugin.render_block_for_test(&mut one.0, &mut one.1);
        assert!(
            ramping(&params).is_empty(),
            "after a preset load, still ramping on the first sample: {:?}",
            ramping(&params)
        );
    }

    /// **A host state restore, then a new source on one layer, settles only that layer** (code review
    /// round 1). A restore publishes both layers under one revision, and the per-layer ledger a later
    /// stage copies from was left behind, so the untouched layer read as changed and had its smoothers
    /// snapped under whatever automation was moving them.
    #[test]
    fn a_restore_then_a_new_source_on_one_layer_leaves_the_other_layer_alone() {
        use nice_plug::params::persist::PersistentField;
        let mut plugin = crate::MxmCreativeSampler::default();
        let params = plugin.params_for_test();
        for (_id, ptr, _group) in params.param_map() {
            unsafe { ptr._internal_update_smoother(48_000.0, true) };
        }
        let mut one = ([0.0f32; 1], [0.0f32; 1]);

        // A host state restore: both layers at once, and heard.
        let restored = params.assets.map(|payload| payload.clone());
        params.assets.set(restored);
        plugin.render_block_for_test(&mut one.0, &mut one.1);

        // Layer B's crop is moving under automation when layer A receives a new source.
        let host = Ledger::default();
        mxm_preset::ErasedParam::set(&params.b_start, &ParamSetter::new(&host), 0.35);
        staged(&params, Arrival::File(Some(a_loop())));
        settle(
            &params,
            &ParamSetter::new(&host),
            params.assets.revision(),
            &params.assets.status(),
            SettleAction::WriteThenCommit,
        );
        plugin.render_block_for_test(&mut one.0, &mut one.1);

        let ramping = |param: &nice_plug::prelude::FloatParam| {
            (param.smoothed.previous_value() - param.value()).abs() > 1.0e-6
        };
        assert!(
            !ramping(&params.a_loop_start),
            "layer A's new source was not settled"
        );
        assert!(
            ramping(&params.b_start),
            "layer B's moving crop was snapped by layer A's arrival"
        );
    }

    /// **A WAV that carries a loop arrives with it, and one whose chunk cannot be used imports exactly
    /// as one without** — through the real decoder, from files written here.
    #[test]
    fn a_wav_with_a_loop_arrives_looping_at_its_own_frames() {
        let dir = std::env::temp_dir().join(format!(
            "mxm-creative-sampler-wav-loop-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let write = |name: &str, chunk: Option<Vec<u8>>| {
            // The same 16-bit codes the test always wrote, through the collection's encoder.
            let samples: Vec<f32> = (0..4_096u32)
                .map(|n| {
                    let phase = std::f32::consts::TAU * (n % 169) as f32 / 169.0;
                    f32::from((phase.sin() * 20_000.0) as i16) / 32_767.0
                })
                .collect();
            let (mut bytes, _) = mxm_audio_file::encode(
                &samples,
                1,
                44_100,
                mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
            )
            .unwrap();
            if let Some(body) = chunk {
                bytes.extend_from_slice(b"smpl");
                bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
                bytes.extend_from_slice(&body);
                let riff = (bytes.len() - 8) as u32;
                bytes[4..8].copy_from_slice(&riff.to_le_bytes());
            }
            let path = dir.join(name);
            std::fs::write(&path, bytes).expect("a scratch WAV");
            path
        };
        let smpl = |kind: u32, start: u32, end: u32| {
            let mut body = vec![0u8; 36];
            body[28..32].copy_from_slice(&1u32.to_le_bytes());
            for word in [0, kind, start, end, 0, 0] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            body
        };

        let params = MxmCreativeSamplerParams::default();
        params
            .assets
            .import_wav(0, &write("cycle.wav", Some(smpl(0, 0, 168))))
            .expect("a looped file imports");
        match params.assets.arrival(params.assets.revision()) {
            Some(Arrival::File(Some(found))) => {
                assert_eq!(
                    (found.start, found.end, found.alternate, found.frames),
                    (0, 168, false, 4_096)
                );
                // 169 frames at 44.1 kHz is 260.95 Hz: four cents flat of C4.
                let root = found.root.expect("a single cycle is a pitch");
                assert!((root - 59.955).abs() < 0.01, "the cycle's root is {root}");
            }
            other => panic!("a looped file arrived as {other:?}"),
        }

        let mut broken = smpl(0, 0, 168);
        broken.truncate(40);
        for (name, chunk) in [("broken.wav", Some(broken)), ("plain.wav", None)] {
            params
                .assets
                .import_wav(0, &write(name, chunk))
                .unwrap_or_else(|why| panic!("{name} failed to import: {why}"));
            assert_eq!(
                params.assets.arrival(params.assets.revision()),
                Some(Arrival::File(None)),
                "{name} arrived with a loop"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A drop arrives in tune when its name says how**, which is the gesture this wiring exists
    /// for and the half a unit test on the parser cannot reach.
    #[test]
    fn a_named_drop_sets_root_from_its_name_and_an_unnamed_one_does_not() {
        // **The name alone decides whether it says anything**, because no sample is handed over.
        // Which note it names is `naming::the_octave_comes_from_what_was_heard`'s to hold, the ten
        // real files in `examples/measure_root.rs` are the end-to-end evidence, and that an arrival
        // writes Root exactly once is `an_arrival_writes_each_parameter_once_with_its_final_value`'s.
        let wrote = |name: &str| root_in_name(name, None).is_some();

        for named in [
            "OS_VV2_bass_soul_E.wav",
            "jh_keys_piano_smoke_80_Cm.wav",
            "pad_Eb.wav",
            "hit_C3.wav",
            "loop_G#m.wav",
        ] {
            assert!(wrote(named), "{named} did not set Root from its name");
        }

        // **Nothing is guessed from the audio alone.** A detector is confident on a bass note and
        // wrong on a chord, and a wrong Root retunes the whole keyboard — so a file whose name
        // says nothing leaves Root where the import put it. That is what the Auto button is for:
        // asked, not assumed.
        for unnamed in [
            "vocal_chop.wav",
            "kick_loop_b.wav",
            "take_a.wav",
            "riser.wav",
        ] {
            assert!(
                !wrote(unnamed),
                "{unnamed} has no key in its name and must not move Root"
            );
        }
    }

    /// **The editor opens at the budget, hugged** (`plans/plan-editor-standard.md` F1): `REFERENCE`
    /// is derived, not typed, and this holds it.
    #[test]
    fn the_opening_size_is_the_budget_hugged() {
        let params = MxmCreativeSamplerParams::default();
        let telemetry = Telemetry::default();
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let sample = silence(256);
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let mut view = 0usize;
        let mut layer = 0usize;
        let mut load = |_: usize, _: PathBuf| {};
        let mut clear = |_: usize| {};
        let mut generate = |_: usize, _: crate::generate::Recipe| {};
        opening_size::is_the_budget_hugged(
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &REVEAL,
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut view,
                    &mut layer,
                    [Some(&sample), Some(&sample)],
                    ["a.wav", "b.wav"],
                    &LoadState::Idle,
                    &[[0.0, 0.0]; 64],
                    None,
                    &mut load,
                    &mut clear,
                    &mut generate,
                );
            },
        );
    }

    /// **The app bar holds in the narrowest window**: its `…` menu whole and nothing drawn over
    /// anything else, from `MINIMUM` up (`opening_size::bar_holds_from_the_minimum`).
    #[test]
    fn the_app_bar_holds_in_the_minimum_window() {
        let params = MxmCreativeSamplerParams::default();
        let telemetry = Telemetry::default();
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let sample = silence(256);
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let mut view = 0usize;
        let mut layer = 0usize;
        let mut load = |_: usize, _: PathBuf| {};
        let mut clear = |_: usize| {};
        let mut generate = |_: usize, _: crate::generate::Recipe| {};
        opening_size::bar_holds_from_the_minimum(
            egui::vec2(MINIMUM.0 as f32, MINIMUM.1 as f32),
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut view,
                    &mut layer,
                    [Some(&sample), Some(&sample)],
                    ["a.wav", "b.wav"],
                    &LoadState::Idle,
                    &[[0.0, 0.0]; 64],
                    None,
                    &mut load,
                    &mut clear,
                    &mut generate,
                );
            },
        );
    }

    /// `paging_checks::verify` lays the real panel out over both themes, both scales and each of
    /// these sizes, and asserts every card is reachable, sits inside its viewport, keeps its floor,
    /// and does not overlap its neighbours.
    #[test]
    fn every_dynamic_page_fits_and_every_card_is_reachable() {
        let params = MxmCreativeSamplerParams::default();
        // **Painted at the init patch, and that is a known limit rather than an oversight.**
        // Revealing every route makes the Keyboard card 947 points tall against an 804-point
        // viewport, so the page scrolls — and this shared check forbids scrolling at the reference
        // size. Eleven sources times a row each cannot fit any card beside its own controls, so a
        // fully-routed card scrolling is a **design question for the owner**, not something to hide
        // by weakening a check seven plugins share. Recorded in this plugin's AGENTS.md.
        let telemetry = Telemetry::default();
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let sample = silence(256);
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let mut view = 0usize;
        let mut layer = 0usize;
        let mut load = |_: usize, _: PathBuf| {};
        let mut clear = |_: usize| {};
        let mut generate = |_: usize, _: crate::generate::Recipe| {};
        paging_checks::verify(
            &sections::test_items(&params, &both_loaded(0)),
            &[
                egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
                egui::vec2(1880.0, 1040.0),
                egui::vec2(MINIMUM.0 as f32, MINIMUM.1 as f32),
            ],
            |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut view,
                    &mut layer,
                    [Some(&sample), Some(&sample)],
                    ["a.wav", "b.wav"],
                    &LoadState::Idle,
                    &[[0.0, 0.0]; 64],
                    None,
                    &mut load,
                    &mut clear,
                    &mut generate,
                );
            },
        );
    }

    use mxm_plugin_test::tree_checks;

    /// Runs the layout tree's checks (plans/plan-layout-tree.md §4.3, `tree_checks::card`) over
    /// `cards` in one state: each card's computed floor holds its content with nothing painted
    /// outside the card, its content floor is exact, the height its tree states is the height it
    /// draws — for Playback, the tallest reader's — and every leaf stays in the room it was given.
    /// The grains and the playhead are forced on over the waveform, so the overlays are checked
    /// too.
    fn check_cards(
        state: &str,
        params: &MxmCreativeSamplerParams,
        shown: &sections::Shown<'_>,
        setup: &dyn Fn(&egui::Context),
        cards: &[usize],
    ) {
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let sample = silence(256);
        let samples = shown.loaded.map(|loaded| loaded.then_some(&sample));
        let waveform: Vec<[f32; 2]> = (0..64)
            .map(|bin| {
                let level = (bin as f32 * 0.3).sin() * 0.8;
                [-level.abs(), level.abs()]
            })
            .collect();
        let telemetry = Telemetry::default();
        telemetry.publish_grains(&[(0.1, 0.3, 1.0), (0.6, 0.4, 0.5), (0.9, 1.0, 0.8)]);
        if state.ends_with("at a tempo") {
            telemetry.tempo.publish(Some(120.0));
        }
        let floors: Vec<f32> = sections::test_items(params, shown)
            .iter()
            .map(|item| item.card.floor)
            .collect();
        for &index in cards {
            let mut text = HashMap::new();
            let mut load = |_: usize, _: PathBuf| {};
            let mut clear = |_: usize| {};
            let mut generate = |_: usize, _: crate::generate::Recipe| {};
            let mut live = sections::Live {
                params,
                setter: &setter,
                text: &mut text,
                layer: shown.layer,
                chosen: None,
                samples,
                names: shown.names,
                status: shown.status,
                waveform: &waveform,
                playhead: Some(0.5),
                telemetry: Some(&telemetry),
                load: &mut load,
                clear: &mut clear,
                generate: &mut generate,
            };
            tree_checks::card(
                setup,
                state,
                sections::TITLES[index],
                floors[index],
                &|ui| sections::card(ui, index, params, shown),
                &mut |ui, leaf, rect| sections::paint(ui, &mxm_ui::LIGHT, leaf, rect, &mut live),
            );
        }
    }

    /// Every card, in every state that changes what it holds, passes the layout tree's checks.
    ///
    /// This editor's structural-state matrix: Init — layer A holding the saw it starts on, B
    /// empty — with A selected and with B selected; both layers loaded, B selected; every route
    /// revealed at full negative depth, where a reading carries its sign and every digit; each
    /// reader on the Playback card, whose `Reserve` holds the tallest; Grain asking for more grains
    /// than are played; each voice mode, whose line differs; each load state with a long file
    /// name or message; Auto's longest answer beside Root; and a long file name on both chips.
    #[test]
    fn every_card_passes_the_tree_checks_in_every_state() {
        use crate::params::{ReaderChoice, VoiceChoice};
        use nice_plug::params::InternalParamMut;
        const LONG: &str =
            "SIG_GV_75_vibraphone_melody_roll_ambient_jazz_ensemble_isitme_Bbmin.wav";
        let none = |_: &egui::Context| {};
        let every = [0, 1, 2, 3, 4, 5, 6];
        let init = |layer: usize| sections::Shown {
            layer,
            loaded: [true, false],
            names: ["Init saw C3", ""],
            status: &LoadState::Idle,
        };
        let params = MxmCreativeSamplerParams::default();
        check_cards("init", &params, &init(0), &none, &every);
        check_cards("an empty layer selected", &params, &init(1), &none, &[3, 4]);
        check_cards(
            "both loaded, B selected",
            &params,
            &both_loaded(1),
            &none,
            &[3, 4],
        );

        let revealed = MxmCreativeSamplerParams::default();
        reveal_every_mode(&revealed);
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        for target in 0..mxm_creative_sampler_dsp::routing::TARGETS {
            for route in &revealed.routes.target(target) {
                route.amount.set(&setter, 0.0);
            }
        }
        check_cards(
            "every route revealed",
            &revealed,
            &both_loaded(0),
            &none,
            &every,
        );

        for reader in [
            ReaderChoice::Repitch,
            ReaderChoice::Stretch,
            ReaderChoice::Grain,
        ] {
            let params = MxmCreativeSamplerParams::default();
            // SAFETY: a test owns these parameters outright; nothing else holds a reference.
            unsafe { params.a_reader._internal_set_plain_value(reader) };
            check_cards(&format!("{reader:?} on A"), &params, &init(0), &none, &[4]);
        }
        let crowded = MxmCreativeSamplerParams::default();
        // SAFETY: as above.
        unsafe {
            crowded
                .a_reader
                ._internal_set_plain_value(ReaderChoice::Grain);
            crowded.a_grain_size._internal_set_normalized_value(1.0);
            crowded.a_grain_rate._internal_set_normalized_value(1.0);
        }
        check_cards(
            "more grains asked for than played",
            &crowded,
            &init(0),
            &none,
            &[4],
        );

        for mode in [VoiceChoice::Poly, VoiceChoice::Mono, VoiceChoice::Legato] {
            let params = MxmCreativeSamplerParams::default();
            // SAFETY: as above.
            unsafe { params.voice_mode._internal_set_plain_value(mode) };
            check_cards(&format!("{mode:?}"), &params, &init(0), &none, &[0]);
        }

        let params = MxmCreativeSamplerParams::default();
        for status in [
            LoadState::Loading {
                layer: 1,
                name: LONG.to_owned(),
            },
            LoadState::Ready {
                layer: 0,
                name: LONG.to_owned(),
            },
            LoadState::Failed {
                layer: 1,
                message: format!("{LONG} could not be decoded: the stream ended early"),
            },
        ] {
            let shown = sections::Shown {
                status: &status,
                ..both_loaded(0)
            };
            check_cards(&format!("{status:?}"), &params, &shown, &none, &[3]);
        }

        let revision = params.assets.revision();
        let heard = move |ctx: &egui::Context| {
            ctx.data_mut(|data| {
                data.insert_temp(
                    sections::heard_id(0),
                    (revision, "no clear pitch — Root left alone".to_owned()),
                );
            });
        };
        check_cards("Auto heard nothing", &params, &init(0), &heard, &[3]);

        let named = sections::Shown {
            names: [LONG, LONG],
            ..both_loaded(0)
        };
        check_cards("long file names", &params, &named, &none, &[3]);

        // Every tempo sync on, A granular so its grain rate is drawn: with no tempo each knob reads
        // its free value, and at one its division.
        let synced = MxmCreativeSamplerParams::default();
        // SAFETY: as above.
        unsafe {
            synced
                .a_reader
                ._internal_set_plain_value(ReaderChoice::Grain);
            for sync in [
                &synced.lfo1_sync,
                &synced.lfo2_sync,
                &synced.a_grain_sync,
                &synced.b_grain_sync,
            ] {
                sync._internal_set_normalized_value(1.0);
            }
        }
        check_cards("every sync on", &synced, &init(0), &none, &every);
        check_cards(
            "every sync on, at a tempo",
            &synced,
            &init(0),
            &none,
            &every,
        );
    }

    /// Every page at the opening size, light and dark, for the owner's review of the layout-tree
    /// conversion (plans/plan-layout-tree.md §4.3): `target/layout-tree/mxm-creative-sampler/<tag>/`,
    /// where `MXM_PICTURES` names the tag — `before` on the unconverted editor, `after` on the tree.
    /// Drawn at the init patch: layer A holds the generated saw the instrument starts on, layer B is
    /// empty.
    ///
    /// `MXM_PICTURES=after cargo test -p mxm-creative-sampler --lib tree_pictures -- --ignored`
    #[test]
    #[ignore = "renders through wgpu; run by hand"]
    fn tree_pictures() {
        let tag = std::env::var("MXM_PICTURES").unwrap_or_else(|_| "after".to_owned());
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/layout-tree/mxm-creative-sampler")
            .join(tag);
        let params = MxmCreativeSamplerParams::default();
        let telemetry = Telemetry::default();
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let bank = params.assets.bank();
        let snapshot = bank.read();
        // The editor's own binning (`MxmCreativeSamplerApp::rebuild_waveform`): each bin's lowest
        // and highest mono sample.
        let waveform: Vec<[f32; 2]> = snapshot.samples[0]
            .as_ref()
            .map(|sample| {
                let frames = sample.frames();
                let bins = frames.len().min(WAVEFORM_POINTS);
                (0..bins)
                    .map(|bin| {
                        let start = bin * frames.len() / bins;
                        let end = ((bin + 1) * frames.len() / bins)
                            .max(start + 1)
                            .min(frames.len());
                        frames[start..end].iter().fold(
                            [f32::INFINITY, f32::NEG_INFINITY],
                            |[low, high], frame| {
                                let mono = (frame[0] + frame[1]) * 0.5;
                                [low.min(mono), high.max(mono)]
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let names = [
            snapshot.names[0].as_deref().unwrap_or(""),
            snapshot.names[1].as_deref().unwrap_or(""),
        ];
        let samples = [snapshot.samples[0].as_ref(), snapshot.samples[1].as_ref()];
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let mut view = 0usize;
        let mut layer = 0usize;
        let mut load = |_: usize, _: PathBuf| {};
        let mut clear = |_: usize| {};
        let mut generate = |_: usize, _: crate::generate::Recipe| {};
        tree_checks::pictures(
            &|_| {},
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &dir,
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut view,
                    &mut layer,
                    samples,
                    names,
                    &LoadState::Idle,
                    &waveform,
                    None,
                    &mut load,
                    &mut clear,
                    &mut generate,
                );
            },
        );
    }

    /// Put both layers into Grain so the Playback card shows every control it owns.
    ///
    /// **The contextual card is why this exists.** Grain's controls belong to Grain, so at the init
    /// patch B sits in Repitch and eight of its parameters are legitimately not painted. Coverage
    /// asks whether every parameter is *reachable*, not whether every parameter is on screen in one
    /// arbitrary mode, so the check reveals the mode that owns them — the same job `REVEAL` does
    /// for a disclosure elsewhere in the collection.
    fn reveal_every_mode(params: &MxmCreativeSamplerParams) {
        use nice_plug::params::InternalParamMut;
        for reader in [&params.a_reader, &params.b_reader] {
            // Safety: a test owns these parameters outright; nothing else holds a reference.
            unsafe { reader._internal_set_plain_value(crate::params::ReaderChoice::Grain) };
        }
        // **Every route present, because an absent one draws nothing at all.**
        //
        // That is the routing interface's own rule — a target with nothing wired draws no group —
        // so at the init patch four route rows exist and the other fifty-one do not. Painting the
        // panel with all of them revealed is the only way this check covers the parameters the
        // conversion added, and it is the same reason the pilot measures its card floors with every
        // route revealed rather than at Init.
        for presence in params.routes.presences() {
            unsafe { presence._internal_set_plain_value(true) };
        }
    }

    fn check_coverage(selected: usize) {
        let params = MxmCreativeSamplerParams::default();
        reveal_every_mode(&params);
        let telemetry = Telemetry::default();
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let sample = silence(256);
        let owned = reachable_with(selected);
        let ids: Vec<&str> = owned.iter().map(String::as_str).collect();
        let mut text_entry = HashMap::new();
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut nav = mxm_ui::navigation::State::default();
        let mut view = 0usize;
        let mut layer = selected;
        let mut load = |_: usize, _: PathBuf| {};
        let mut clear = |_: usize| {};
        let mut generate = |_: usize, _: crate::generate::Recipe| {};
        keyboard_checks::the_cursor_reaches_and_operates(
            egui::vec2(REFERENCE.0 as f32, REFERENCE.1 as f32),
            &sections::test_items(&params, &both_loaded(selected)),
            keyboard_checks::Coverage::Exactly(&ids),
            &REVEAL,
            &host,
            &mut |ui| {
                panel(
                    ui,
                    &params,
                    &telemetry,
                    &setter,
                    &mut text_entry,
                    &mut presets,
                    &mut nav,
                    &mut view,
                    &mut layer,
                    [Some(&sample), Some(&sample)],
                    ["a.wav", "b.wav"],
                    &LoadState::Idle,
                    &[[0.0, 0.0]; 64],
                    None,
                    &mut load,
                    &mut clear,
                    &mut generate,
                );
            },
        );
    }

    /// The rollout's own failure mode: a control whose `navigation::at` scope was forgotten paints
    /// exactly as before and is simply unreachable from the keyboard. Nothing else would say so.
    #[test]
    fn the_keyboard_cursor_reaches_and_operates_every_parameter_of_the_selected_layer() {
        check_coverage(0);
    }

    #[test]
    fn selecting_b_reaches_the_other_layers_parameters() {
        check_coverage(1);
    }
    /// A host that looks at what is *audible* every time a parameter is written.
    ///
    /// The counting `Recorder` can say that gestures happened; it cannot say they happened before
    /// the sound changed, which is the whole claim of the staging barrier. This records the
    /// published source name at each write, so the ordering becomes an assertion instead of a
    /// reading of the code.
    struct Witness {
        bank: std::sync::Arc<crate::asset::AssetBank>,
        audible: std::sync::Mutex<Vec<Option<String>>>,
    }

    impl Witness {
        fn new(bank: std::sync::Arc<crate::asset::AssetBank>) -> Self {
            Self {
                bank,
                audible: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn audible_during_gestures(&self) -> Vec<Option<String>> {
            self.audible.lock().unwrap().clone()
        }
    }

    impl nice_plug::context::gui::GuiContextInner for Witness {
        // A test double has no host to ask for a restart (nice-plug 0.4).
        fn request_restart(&self) {}
        fn plugin_api(&self) -> nice_plug::prelude::PluginApi {
            nice_plug::prelude::PluginApi::Clap
        }
        unsafe fn raw_begin_set_parameter(&self, _param: nice_plug::params::internals::ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(
            &self,
            _param: nice_plug::params::internals::ParamPtr,
            _normalized: f32,
        ) {
            let name = self.bank.read().names[0].clone();
            self.audible.lock().unwrap().push(name);
        }
        unsafe fn raw_end_set_parameter(&self, _param: nice_plug::params::internals::ParamPtr) {}
        fn get_state(&self) -> nice_plug::prelude::PluginState {
            nice_plug::prelude::PluginState {
                version: String::new(),
                params: Default::default(),
                fields: Default::default(),
            }
        }
        fn set_state(&self, _state: nice_plug::prelude::PluginState) {}
    }

    /// **Nothing is heard before the parameters that describe it — on a frame and on a close.**
    ///
    /// Round 5 asked for the timeline rather than the decision, and it is the timeline that the
    /// two earlier defects lived in: a revision published before its gestures, and a revision
    /// published while its payload was still in flight. Three cases, all three previously wrong.
    #[test]
    fn a_source_is_never_audible_before_the_parameters_that_describe_it() {
        // --- A load that lands, settled by an ordinary frame. ---
        let params = MxmCreativeSamplerParams::default();
        let was_audible = params.assets.bank().read().names[0].clone();
        params
            .assets
            .generate_source(0, crate::generate::Recipe::Glass)
            .unwrap();
        let revision = params.assets.revision();
        let status = params.assets.status();
        assert_ne!(revision, params.assets.handled());

        let witness = Witness::new(params.assets.bank());
        let setter = ParamSetter::new(&witness);
        settle(
            &params,
            &setter,
            revision,
            &status,
            settle_action(revision, params.assets.handled(), &status),
        );

        let during = witness.audible_during_gestures();
        assert!(
            !during.is_empty(),
            "settling a load must write its defaults"
        );
        assert!(
            during.iter().all(|name| *name == was_audible),
            "every gesture must land while the previous source is still the audible one: {during:?}"
        );
        assert_ne!(
            params.assets.bank().read().names[0],
            was_audible,
            "and the new source must be audible once they have"
        );
        assert_eq!(params.assets.handled(), revision);
        // The root came from the staged sample, not from `NEUTRAL_ROOT`: every generated source
        // names itself C3, and `DEFAULT_SOURCE_ROOT` is that note.
        assert!(
            (params.a_root.value() - crate::asset::DEFAULT_SOURCE_ROOT).abs() < 0.01,
            "the root was not settled from the staged sample: {}",
            params.a_root.value()
        );

        // --- A close while a load is still in flight. Nothing may move. ---
        let params = MxmCreativeSamplerParams::default();
        let was_audible = params.assets.bank().read().names[0].clone();
        params
            .assets
            .generate_source(0, crate::generate::Recipe::Drawbars)
            .unwrap();
        let revision = params.assets.revision();
        // Whoever starts the *next* acquisition overwrites the status, which is exactly how a
        // revision owing its defaults came to be settled blind.
        params.assets.set_loading_named(1, "something else");
        let status = params.assets.status();
        let witness = Witness::new(params.assets.bank());
        let setter = ParamSetter::new(&witness);
        settle(
            &params,
            &setter,
            revision,
            &status,
            settle_action(revision, params.assets.handled(), &status),
        );
        assert!(
            witness.audible_during_gestures().is_empty(),
            "a close during an acquisition must write nothing"
        );
        assert_eq!(
            params.assets.bank().read().names[0],
            was_audible,
            "nor may it publish anything"
        );
        assert_ne!(
            params.assets.handled(),
            revision,
            "and it must leave the revision owing its defaults, for the next editor to finish"
        );

        // --- The next editor finishes it, which is what the watermark is for. ---
        let status = LoadState::Ready {
            layer: 0,
            name: "Drawbars C3".to_owned(),
        };
        let witness = Witness::new(params.assets.bank());
        let setter = ParamSetter::new(&witness);
        settle(
            &params,
            &setter,
            revision,
            &status,
            settle_action(revision, params.assets.handled(), &status),
        );
        assert!(
            witness
                .audible_during_gestures()
                .iter()
                .all(|name| *name == was_audible),
            "the deferred revision's gestures must still precede its sound"
        );
        assert_ne!(params.assets.bank().read().names[0], was_audible);
        assert_eq!(params.assets.handled(), revision);
    }

    /// Every string one frame painted, from egui's own shape list.
    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// **The developer channel reaches every category's first card and the Parameters surface, and
    /// leaving that surface restores a musician page; CC 117 and CC 116 reach the preset browser and
    /// the theme** (mxm-kit's `docs/plugin-conventions.md`, *A developer channel in every editor*).
    ///
    /// Driven through the telemetry slots the plugin's CC arms fill, on the real panel. Categories
    /// are checked at the minimum window, where a page holds few enough cards that landing on the
    /// right one is not an accident of everything fitting on one. The Parameters surface is painted
    /// in a window tall enough to hold its whole list, and asked for every parameter's painted name.
    #[test]
    fn developer_requests_reach_each_category_the_parameters_surface_the_browser_and_the_theme() {
        use mxm_ui::paging::{Key, PARAMETERS};

        let params = MxmCreativeSamplerParams::default();
        let telemetry = Telemetry::default();
        let host = keyboard_checks::Recorder::default();
        let setter = ParamSetter::new(&host);
        let sample = silence(256);
        let mut presets = PresetUi::at(crate::preset::Library::at(None), &params);
        let mut view = 0usize;
        let mut text_entry = HashMap::new();
        let mut nav = mxm_ui::navigation::State::default();
        let mut layer = 0usize;
        let ctx = egui::Context::default();
        mxm_ui::theme::apply(&ctx);
        mxm_ui::typography::apply(&ctx);
        ctx.all_styles_mut(|style| style.animation_time = 0.0);

        let mut frames =
            |size: egui::Vec2, view: &mut usize, presets: &mut PresetUi| -> Vec<String> {
                let mut texts = Vec::new();
                for _ in 0..4 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    };
                    let mut output = ctx.run_ui(input, |ui| {
                        let mut load = |_: usize, _: PathBuf| {};
                        let mut clear = |_: usize| {};
                        let mut generate = |_: usize, _: crate::generate::Recipe| {};
                        panel(
                            ui,
                            &params,
                            &telemetry,
                            &setter,
                            &mut text_entry,
                            &mut *presets,
                            &mut nav,
                            &mut *view,
                            &mut layer,
                            [Some(&sample), Some(&sample)],
                            ["a.wav", "b.wav"],
                            &LoadState::Idle,
                            &[[0.0, 0.0]; 64],
                            None,
                            &mut load,
                            &mut clear,
                            &mut generate,
                        );
                    });
                    // As the shared `Session` does: an undelivered texture delta is a panic.
                    output.textures_delta.clear();
                    texts = painted_text(&output.shapes);
                }
                texts
            };
        let selected_page = || {
            let report = mxm_ui::paging::editor::report(&ctx).expect("a paging report");
            (
                report.selected,
                report.plan.pages[report.selected].cards.clone(),
            )
        };

        let minimum = egui::vec2(MINIMUM.0 as f32, MINIMUM.1 as f32);
        frames(minimum, &mut view, &mut presets);
        assert!(
            !selected_page().1.contains(&Key(6)),
            "the premise: the Filter card is not on the opening page at the minimum window"
        );

        // Tone, Generators, Modulators, Performance: each category's first card in authored order.
        for (request, first) in [(4u8, 6u64), (3, 3), (1, 1), (0, 0)] {
            telemetry.request_view(request);
            frames(minimum, &mut view, &mut presets);
            assert_eq!(view, 0, "category {request} left the musician pages");
            assert!(
                selected_page().1.contains(&Key(first)),
                "category {request} did not land on card {first}: {:?}",
                selected_page()
            );
        }
        // Sequencers and Effects have no cards here, so asking for them moves nothing.
        for request in [2u8, 5] {
            let before = selected_page();
            telemetry.request_view(request);
            frames(minimum, &mut view, &mut presets);
            assert_eq!(
                selected_page(),
                before,
                "empty category {request} moved the page"
            );
        }

        let tall = egui::vec2(REFERENCE.0 as f32, 8_000.0);
        telemetry.request_view(PARAMETERS as u8);
        let texts = frames(tall, &mut view, &mut presets);
        assert_eq!(view, PARAMETERS);
        // A flat list with no card to shorten a name: each parameter under its full name.
        for (id, param) in mxm_preset::Instrument::parameters(&params) {
            let label = mxm_preset::ErasedParam::name(param).to_owned();
            assert!(
                texts.contains(&label),
                "the Parameters surface lacks {id} ({label})"
            );
        }
        assert!(
            !texts
                .iter()
                .any(|text| text == "Keyboard" || text == "Playback"),
            "the Parameters surface still paints the musician cards"
        );
        assert!(
            mxm_ui::navigation::spots(&ctx).is_empty(),
            "the Parameters surface has no cards, so nothing registers with the cursor"
        );

        telemetry.request_view(0);
        let texts = frames(tall, &mut view, &mut presets);
        assert_eq!(
            view, 0,
            "leaving Parameters did not return to the musician pages"
        );
        assert!(
            texts.iter().any(|text| text == "Keyboard"),
            "leaving Parameters did not paint the Performance card"
        );
        assert!(
            !mxm_ui::navigation::spots(&ctx).is_empty(),
            "the musician page registers its controls with the cursor again"
        );

        telemetry.request_theme(1);
        frames(tall, &mut view, &mut presets);
        assert_eq!(
            ctx.options(|options| options.theme_preference),
            egui::ThemePreference::Dark,
            "CC 116 value 1 did not apply dark"
        );

        telemetry.request_browser(true);
        frames(tall, &mut view, &mut presets);
        assert!(presets.is_browser_open(), "CC 117 did not open the browser");
        telemetry.request_browser(false);
        frames(tall, &mut view, &mut presets);
        assert!(
            !presets.is_browser_open(),
            "CC 117 did not close the browser"
        );
    }

    /// **What a frame — or a close — does about the revision the field is holding.**
    ///
    /// The failure this protects against is silent, and two earlier versions of the code had it.
    /// The first committed unconditionally at the end of `editor_closed`. The second still
    /// committed *and marked handled* whenever the revision had moved, including while the status
    /// read `Loading` — and `Loading` is set by whoever starts the next acquisition, not by the
    /// revision that is outstanding, so a `Ready` revision owing its defaults could have its
    /// status overwritten, be published without them, and then be recorded as settled. Nothing
    /// would ever go back for it.
    ///
    /// The decision is a pure function, so every case can be asked without a host or a GUI.
    #[test]
    fn a_revision_is_settled_only_when_its_payload_has_landed() {
        let ready = LoadState::Ready {
            layer: 0,
            name: "Glass C3".to_owned(),
        };
        let loading = LoadState::Loading {
            layer: 0,
            name: "Glass C3".to_owned(),
        };
        let failed = LoadState::Failed {
            layer: 0,
            message: "no".to_owned(),
        };

        // A payload that landed and owes its region and root.
        assert_eq!(
            settle_action(8, 7, &ready),
            SettleAction::WriteThenCommit,
            "a landed source must have its defaults written before it is heard"
        );

        // Nothing outstanding. A preset stages with `Idle` and carries its own gestures, so
        // publishing is all that is left; it is idempotent where there is nothing staged.
        assert_eq!(settle_action(7, 7, &ready), SettleAction::Commit);
        assert_eq!(settle_action(8, 7, &LoadState::Idle), SettleAction::Commit);
        assert_eq!(settle_action(7, 7, &LoadState::Idle), SettleAction::Commit);
        assert_eq!(settle_action(8, 7, &failed), SettleAction::Commit);

        // **A load in flight stops everything**, whether or not this revision owes anything —
        // which is the case both earlier versions got wrong.
        assert_eq!(
            settle_action(8, 7, &loading),
            SettleAction::Wait,
            "a revision owing defaults must not be published while an acquisition is in flight"
        );
        assert_eq!(
            settle_action(7, 7, &loading),
            SettleAction::Wait,
            "nor may one that owes nothing — the status does not say which revision it describes"
        );
    }
}
