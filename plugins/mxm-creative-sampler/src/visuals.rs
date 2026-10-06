//! The grains, drawn over the waveform they are reading.
//!
//! Plugin-local, following `mxm-grain-fx`'s buffer timeline and `mxm-chorus-06`'s Sweep. Not a
//! reusable type and not `crates/ui`'s: design system §13 keeps a widget out of the shared crate
//! until a second instrument needs it, and this one has no meaning away from the waveform it is
//! painted on.
//!
//! **A separate plot was the wrong answer.** The first attempt drew time against source position on
//! a canvas of its own under the grain knobs, the way `mxm-grain-fx` draws its buffer. The owner's
//! ruling was that it is confusing, and the reason is that a sampler already has the picture the
//! grains belong to: the waveform. A cloud on its own axes asks the reader to hold two pictures at
//! once and match them up; the same marks laid over the waveform need no matching, because they
//! share the x axis with the thing they are reading. So this shares the Sample card's canvas, its
//! `0..=1` across the whole sample, and its region handles.
//!
//! **Geometry comes from `mxm_ui::visual`'s tokens**, and **nothing here calls `navigation::at`** —
//! a display is not a control, and the keyboard cursor's coverage check would report it as a
//! parameter that is not one.

use egui::{Rect, Sense, Stroke, Ui, Vec2, pos2};
use mxm_ui::theme::Tokens;
use mxm_ui::visual;

use crate::telemetry::{GRAIN_SLOTS, Telemetry};

/// How wide a grain's trail is drawn.
///
/// **Two, after one was too thin and three too thick.** A grain is one of up to 256 reads, so the
/// picture is a texture rather than a set of objects and a heavy mark turns a cloud into a block —
/// but a hairline over a waveform is lost in the waveform. The trail carries the width and the head
/// carries the contrast: the head is drawn taller and at full weight, so a single grain is findable
/// without every grain having to shout.
const GRAIN_STROKE: f32 = 2.0;

/// How far the head stands out of the trail, each way.
const HEAD_REACH: f32 = 2.5;

/// The share of the canvas height the marks are spread over, centred.
///
/// Less than the whole, so the marks stay clear of the region handles at the corners and of the
/// waveform's own extremes, and so the band reads as an overlay rather than as more waveform.
const SPREAD: f32 = 0.72;

/// Every sounding grain, laid over the waveform as the span of source it is reading.
///
/// **Each grain is a segment from where it began to where it has got to**, so the picture answers
/// both halves of the question at once: where the grains start, and where they go. A grain reading
/// forward draws to the right of its start and one running backwards draws to the left, which is
/// what makes Reverse chance and a negative Scan speed visible rather than merely audible.
///
/// They travel. The editor repaints at 40 ms and each frame's segment is longer than the last, so a
/// grain is seen sweeping out of its onset — left to right at ordinary settings — and fading as its
/// window closes. Position variation scatters where the segments begin; Scan speed walks the whole
/// group along the sample; grain size sets how long each segment grows before it goes.
///
/// **The marks are real.** They are the engine's own grains, read off the voices each block, never
/// a pattern computed from the controls — at the top of Onset timing the scheduler is a per-sample
/// draw and a control-derived picture would show an even comb over a cloud. `mxm-chorus-06`'s Sweep
/// records the same rule: a picture with its own clock drifts from the sound, which is the lie a
/// display exists to prevent.
///
/// `telemetry` is optional because a layout test may have none. Nothing is drawn then, which costs
/// no height — this is an overlay on a canvas that is already allocated.
pub fn grains(ui: &Ui, tokens: &Tokens, telemetry: Option<&Telemetry>, rect: Rect) {
    let Some(telemetry) = telemetry else {
        return;
    };
    let mut found_buffer = [(0.0f32, 0.0f32, 0.0f32); GRAIN_SLOTS];
    let found = telemetry.grains(&mut found_buffer);
    if found == 0 {
        return;
    }

    let painter = ui.painter_at(rect);
    let band = rect.height() * SPREAD;
    let top = rect.center().y - band * 0.5;

    for (start, current, weight) in &found_buffer[..found] {
        if *weight <= 0.01 {
            continue;
        }
        let x_from = rect.left() + start.clamp(0.0, 1.0) * rect.width();
        let x_to = rect.left() + current.clamp(0.0, 1.0) * rect.width();

        // **A stable row per grain, derived from where it started.** Stacking every grain on one
        // line would hide exactly what the picture is for — that a cloud is many reads at once —
        // and taking the row from the grain's slot would make it jump, because the engine fills a
        // dead slot by moving the last grain into it. The start position does not change for the
        // life of the grain, so a row derived from it does not either.
        let row = fract(start * 9_781.0);
        let y = top + row * band;

        let ink = tokens.warning.gamma_multiply(weight.clamp(0.0, 1.0));
        painter.line_segment(
            [pos2(x_from, y), pos2(x_to, y)],
            Stroke::new(GRAIN_STROKE, ink.gamma_multiply(0.55)),
        );
        // The head, so the direction of travel is visible on a grain that has only just started and
        // whose trail is still a point. Taller than the trail rather than wider, which is what
        // makes one grain findable in a cloud without thickening every mark in it.
        painter.line_segment(
            [pos2(x_to, y - HEAD_REACH), pos2(x_to, y + HEAD_REACH)],
            Stroke::new(GRAIN_STROKE, ink),
        );
    }
}

/// The narrowest the envelope can be drawn.
pub fn envelope_min_width() -> f32 {
    96.0
}

/// How tall the envelope is drawn; it fills the width it is given.
pub const ENVELOPE_HEIGHT: f32 = visual::PLOT_HEIGHT;

/// One grain's envelope, at the shape the control is sitting at.
///
/// **The window is a tone control, and "62%" says nothing about a tone.** The curve does: a
/// near-rectangle with a hundredth of attack is visibly the clicking end, the midpoint is visibly a
/// triangle, and the top is visibly a Hann. `mxm-grain-fx` draws the same picture for the same
/// control and for the same reason.
///
/// It reads the DSP's own [`mxm_creative_sampler_dsp::grain_window`], so it cannot drift from the
/// envelope the grains are actually given — a display carrying its own copy of a law is a display
/// that will eventually disagree with the sound.
pub fn envelope(ui: &mut Ui, tokens: &Tokens, shape: f32) {
    let width = ui.available_width().max(envelope_min_width());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, ENVELOPE_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, visual::CANVAS_RADIUS, tokens.surface_1);

    let inner = rect.shrink(visual::INNER_GUTTER);
    // One point per point of width, so the trapezoid's corner is not rounded off by the sampling
    // and the near-boxcar reads as the rectangle it is.
    let steps = (inner.width().round() as usize).clamp(16, 512);
    let points: Vec<egui::Pos2> = (0..=steps)
        .map(|step| {
            let phase = step as f32 / steps as f32;
            let w = mxm_creative_sampler_dsp::grain_window(shape, phase);
            pos2(
                inner.left() + phase * inner.width(),
                inner.bottom() - w * inner.height(),
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(visual::TRACE_STROKE, tokens.accent),
    ));
    painter.line_segment(
        [
            pos2(inner.left(), inner.bottom()),
            pos2(inner.right(), inner.bottom()),
        ],
        Stroke::new(visual::REFERENCE_STROKE, tokens.text_secondary),
    );
}

/// A deterministic `0..1` from a position, for the row a grain is drawn on.
///
/// Cheap on purpose: this runs once per grain per frame and its only requirement is that the same
/// grain lands on the same row every time and that neighbouring starts do not land on the same one.
fn fract(value: f32) -> f32 {
    let v = value.abs();
    v - v.floor()
}
