//! mxm-creative-sampler through MXM Player's ordinary bundle and CLAP-state path.
//!
//! Build first with `cargo xtask bundle mxm-creative-sampler --release`. These tests construct the
//! same versioned payload the plugin writes, but link no sampler product code into the host.

use mxm_player_harness::app_harness;

use mxm_player::session::Session;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const PLUGIN: &str = "dk.mxm.mxm-creative-sampler";
const SKIP: &str = "skipping: run `cargo xtask bundle mxm-creative-sampler --release`";

fn bundle() -> Option<(PathBuf, PathBuf)> {
    let dir = app_harness::any_bundled_dir()?;
    let file = dir.join("mxm-creative-sampler.clap");
    file.exists().then_some((dir, file))
}

fn session(name: &str) -> Option<Session> {
    let (dir, file) = bundle()?;
    let mut session = Session::scratch(name, vec![dir]);
    session.load(&file, PLUGIN);
    Some(session)
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(TABLE[((word >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((word >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((word >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(word & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn source_and_payload(dir: &Path) -> (PathBuf, Value) {
    let path = dir.join("source-that-will-be-deleted.wav");
    let mut samples = Vec::new();
    let mut pcm = Vec::new();
    for frame in 0..8_192 {
        let phase = std::f32::consts::TAU * 220.0 * frame as f32 / 48_000.0;
        let value = (phase.sin() * 24_000.0) as i16;
        // Handed over as the code it is: the encoder's 16-bit rule maps value / 32767 back to value.
        samples.push(f32::from(value) / 32_767.0);
        pcm.extend_from_slice(&value.to_le_bytes());
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    mxm_audio_file::write(
        &path,
        &samples,
        1,
        48_000,
        mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
    )
    .unwrap();
    let payload = json!({
        "schema": 1,
        "layers": [{
            "name": "source-that-will-be-deleted.wav",
            "sample_rate": 48_000,
            "frames": 8_192,
            "pcm": encode_base64(&pcm)
        }, null]
    });
    (path, payload)
}

fn decode_state(state: &[u8]) -> Value {
    let size = u64::from_le_bytes(state[..8].try_into().expect("CLAP state length prefix"));
    assert_eq!(size as usize, state.len() - 8);
    serde_json::from_slice(&state[8..]).expect("uncompressed nice-plug state")
}

fn encode_state(state: &Value) -> Vec<u8> {
    let json = serde_json::to_vec(state).unwrap();
    let mut framed = Vec::with_capacity(json.len() + 8);
    framed.extend_from_slice(&(json.len() as u64).to_le_bytes());
    framed.extend_from_slice(&json);
    framed
}

fn with_payload(state: &[u8], payload: &Value) -> Vec<u8> {
    let mut state = decode_state(state);
    state["fields"]["samples"] =
        Value::String(serde_json::to_string(payload).expect("payload JSON"));
    encode_state(&state)
}

/// `state` with each named parameter's stored value replaced, keeping the shape the state stores it in.
fn with_params(state: &[u8], values: &[(&str, f64)]) -> Vec<u8> {
    let mut state = decode_state(state);
    for (id, value) in values {
        match &mut state["params"][*id] {
            Value::Object(tagged) => {
                for stored in tagged.values_mut() {
                    *stored = json!(value);
                }
            }
            other => *other = json!(value),
        }
    }
    encode_state(&state)
}

/// `state` with each named choice parameter set to the variant with that id, in the shape a state
/// stores a choice with ids in.
fn with_choices(state: &[u8], values: &[(&str, &str)]) -> Vec<u8> {
    let mut state = decode_state(state);
    for (id, variant) in values {
        let stored = &mut state["params"][*id];
        assert!(
            stored.get("string").is_some(),
            "`{id}` is not stored by variant id: {stored}"
        );
        *stored = json!({ "string": variant });
    }
    encode_state(&state)
}

/// Everything a held note renders over long enough to wrap a loop over a quarter of the test source
/// more than once.
fn render_note(session: &mut Session) -> Vec<f32> {
    session.clear_capture();
    session.app().note_on(60, 0.8);
    session.advance_blocks(32).unwrap();
    session.app().note_off(60);
    session.captured()
}

/// A parameter's stored value in `state`.
fn param_in(state: &[u8], id: &str) -> f64 {
    let state = decode_state(state);
    let stored = &state["params"][id];
    match stored {
        Value::Object(tagged) => tagged.values().next().and_then(Value::as_f64),
        other => other.as_f64(),
    }
    .unwrap_or_else(|| panic!("`{id}` is not in the state: {stored}"))
}

fn payload_in(state: &[u8]) -> Value {
    let state = decode_state(state);
    let field = state["fields"]["samples"]
        .as_str()
        .expect("persisted samples field");
    serde_json::from_str(field).expect("AssetPayload JSON")
}

fn load_dump(session: &mut Session, state: &[u8]) {
    std::fs::write(session.dir().join("preset.clapstate"), state).unwrap();
    let answer = session.app().run_cli_command("loadstate");
    assert!(answer.contains("\"ok\""), "{answer}");
    session.advance_blocks(2).unwrap();
}

fn sound_note(session: &mut Session) -> f32 {
    session.clear_capture();
    session.app().note_on(60, 0.8);
    session.advance_blocks(8).unwrap();
    session.app().note_off(60);
    session
        .captured()
        .into_iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

#[test]
fn embedded_audio_restores_in_a_fresh_host_after_the_source_is_deleted() {
    let Some(mut source_session) = session("creative-sampler-state-source") else {
        eprintln!("{SKIP}");
        return;
    };
    let (source, payload) = source_and_payload(source_session.dir());
    let default_state = source_session.app().engine_mut().capture_state().unwrap();
    // **The loop and Root a looped WAV arrives with are ordinary parameters**, so the patch restores
    // them with the audio (`plans/plan-sampler-wav-loop-import.md` §1.2) — a Root between notes, as
    // an import writes it, included.
    let arrived = [("aloopstart", 0.25), ("aloopend", 0.5), ("aroot", 59.75)];
    // On a reader that reads the loop: Init's layer A is Grain, which never does.
    let portable_state = with_choices(
        &with_params(&with_payload(&default_state, &payload), &arrived),
        &[("areader", "repitch"), ("aloop", "forward")],
    );
    std::fs::remove_file(&source).unwrap();

    let Some(mut restored_session) = session("creative-sampler-state-restored") else {
        unreachable!("the same bundle was present above");
    };
    load_dump(&mut restored_session, &portable_state);
    let peak = sound_note(&mut restored_session);
    assert!(
        peak > 0.01,
        "the restored sample did not sound: peak {peak}"
    );

    let round_trip = restored_session.app().engine_mut().capture_state().unwrap();
    assert_eq!(payload_in(&round_trip), payload);
    for (id, value) in arrived {
        let restored = param_in(&round_trip, id);
        assert!(
            (restored - value).abs() < 1.0e-4,
            "`{id}` restored as {restored}, not {value}"
        );
    }
    assert!(
        !String::from_utf8_lossy(&round_trip).contains(source.to_string_lossy().as_ref()),
        "provenance paths must never enter portable state"
    );

    // **And they are what plays** (code review round 1): a state round-trip alone passes a plugin
    // that ignores them. The restored patch plays twice in fresh hosts, the control, then with Root
    // on the note and with the loop moved; each of those must sound different from it.
    let heard = |state: &[u8], name: &str| {
        let mut played = session(name).expect("the same bundle was present above");
        load_dump(&mut played, state);
        render_note(&mut played)
    };
    let difference = |a: &[f32], b: &[f32]| {
        assert_eq!(
            a.len(),
            b.len(),
            "two renders of one length differ in length"
        );
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max)
    };
    let played = heard(&portable_state, "creative-sampler-state-played");
    let again = heard(&portable_state, "creative-sampler-state-played-again");
    let on_the_note = heard(
        &with_params(&portable_state, &[("aroot", 60.0)]),
        "creative-sampler-state-root-on-the-note",
    );
    let loop_moved = heard(
        &with_params(&portable_state, &[("aloopend", 0.75)]),
        "creative-sampler-state-loop-moved",
    );
    let control = difference(&played, &again);
    assert!(
        control < 1.0e-4,
        "the same restored patch rendered differently twice, by {control}"
    );
    for (what, variant) in [
        ("Root on the note", &on_the_note),
        ("the loop moved", &loop_moved),
    ] {
        let apart = difference(&played, variant);
        assert!(
            apart > 0.05,
            "{what} sounds the same as the restored patch (largest difference {apart}), so the restored value is not what plays"
        );
    }
}

#[test]
fn malformed_embedded_audio_preserves_the_working_sound_and_patch() {
    let Some(mut session) = session("creative-sampler-malformed-state") else {
        eprintln!("{SKIP}");
        return;
    };
    let (_source, payload) = source_and_payload(session.dir());
    let default_state = session.app().engine_mut().capture_state().unwrap();
    let valid_state = with_payload(&default_state, &payload);
    load_dump(&mut session, &valid_state);
    let before = session.app().engine_mut().capture_state().unwrap();

    let mut malformed = payload;
    malformed["layers"][0]["pcm"] = Value::String("!!!!".to_owned());
    let attempted = with_payload(&before, &malformed);
    load_dump(&mut session, &attempted);

    let after = session.app().engine_mut().capture_state().unwrap();
    assert_eq!(payload_in(&after), payload_in(&before));
    let peak = sound_note(&mut session);
    assert!(
        peak > 0.01,
        "the preserved sample did not sound: peak {peak}"
    );
}

/// Sets a parameter by its display name, as a position in its normalised range, and settles.
fn set_param(session: &mut Session, name: &str, fraction: f64) {
    let param = session
        .state()
        .param(name)
        .unwrap_or_else(|| panic!("`{name}` is not a parameter"))
        .clone();
    let value = param.min + fraction * (param.max - param.min);
    session
        .app()
        .engine_mut()
        .push_gui_event(mxm_player::events::input::Payload::ParamValue {
            param_id: param.id,
            value,
        });
    session.advance_blocks(4).expect("the session advances");
}

/// A session with the portable payload already restored, so notes have something to read.
fn loaded(name: &str) -> Option<Session> {
    let mut session = session(name)?;
    let (_source, payload) = source_and_payload(session.dir());
    let default_state = session.app().engine_mut().capture_state().unwrap();
    let state = with_payload(&default_state, &payload);
    load_dump(&mut session, &state);
    Some(session)
}

/// **A fresh instance makes a sound when you press a key, with nothing dropped on it.**
///
/// The instrument used to start with an empty bank, so the first thing anyone heard was silence and
/// the four factory recipes — all `state = null`, so they treat whatever is loaded — had nothing to
/// treat. A generated band-limited saw now loads on A and the init patch reads it as a detuned
/// grain cloud. Nothing here touches a parameter or loads state: that is the point.
#[test]
fn a_fresh_instance_plays_the_init_patch_without_a_drop() {
    let Some(mut session) = loaded("creative-sampler-init-patch") else {
        eprintln!("{SKIP}");
        return;
    };
    session.clear_capture();
    session.app().note_on(60, 0.9);
    session.advance_blocks(48).unwrap();
    session.app().note_off(60);
    session.advance_blocks(16).unwrap();
    let captured = session.captured();
    let peak = captured
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        peak > 0.005,
        "the init patch was silent out of the box: {peak}"
    );
    assert!(peak < 1.0, "the init patch is not a sane level: {peak}");
    assert!(
        captured.iter().all(|sample| sample.is_finite()),
        "the init patch produced a non-finite sample"
    );
}

/// **Every reader is audible through a real host, not only in a unit test.** The three engines are
/// three intentions, and the listening gate cannot judge them if one of them is silent once the
/// wrapper, the state path and the bundle are in the way.
#[test]
fn all_three_readers_sound_and_stay_bounded_in_a_real_instance() {
    let Some(mut session) = loaded("creative-sampler-readers") else {
        eprintln!("{SKIP}");
        return;
    };
    set_param(&mut session, "A Loop", 0.5);
    for (name, position) in [("Repitch", 0.0), ("Stretch", 0.5), ("Grain", 1.0)] {
        set_param(&mut session, "A Playback mode", position);
        session.clear_capture();
        session.app().note_on(64, 0.8);
        session.advance_blocks(24).unwrap();
        session.app().note_off(64);
        session.advance_blocks(8).unwrap();
        let captured = session.captured();
        let peak = captured
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak > 0.005, "{name} was silent through the host: {peak}");
        assert!(peak < 4.0, "{name} left the bounded range: {peak}");
        assert!(
            captured.iter().all(|sample| sample.is_finite()),
            "{name} produced a non-finite sample"
        );
    }
}

/// Mono, glide and the filter envelope, driven the way a host drives them.
#[test]
fn mono_glide_and_the_filter_envelope_survive_the_wrapper() {
    let Some(mut session) = loaded("creative-sampler-performance") else {
        eprintln!("{SKIP}");
        return;
    };
    set_param(&mut session, "A Loop", 0.5);
    set_param(&mut session, "Sustain", 1.0);
    set_param(&mut session, "Voices", 0.5); // Mono
    set_param(&mut session, "Glide", 0.2);

    // An overlapping phrase in Mono is one voice, and releasing the newer key keeps the older one.
    session.clear_capture();
    session.app().note_on(60, 0.8);
    session.advance_blocks(8).unwrap();
    session.app().note_on(67, 0.8);
    session.advance_blocks(8).unwrap();
    session.app().note_off(67);
    session.advance_blocks(8).unwrap();
    let held = session.captured();
    assert!(
        held.iter().fold(0.0f32, |p, s| p.max(s.abs())) > 0.005,
        "the phrase went silent when the newer key was released"
    );
    assert!(held.iter().all(|sample| sample.is_finite()));
    session.app().note_off(60);
    session.advance_blocks(8).unwrap();

    // A closed filter opened by the Shape envelope passes more than one left where it was set.
    set_param(&mut session, "Filter mode", 1.0 / 3.0); // Low pass
    set_param(&mut session, "Cutoff", 0.10);
    set_param(&mut session, "Voices", 0.0); // Poly
    let energy = |session: &mut Session| {
        session.clear_capture();
        session.app().note_on(60, 0.8);
        session.advance_blocks(24).unwrap();
        session.app().note_off(60);
        session.advance_blocks(8).unwrap();
        session
            .captured()
            .iter()
            .map(|sample| (*sample as f64) * (*sample as f64))
            .sum::<f64>()
    };
    // The filter envelope is a route now, not a fixed knob: the amount is `Cutoff from Envelope
    // amount`, still bipolar over -100%..+100%, so the same two fractions mean the same two things.
    // Its presence is wired by `routing::INIT_PRESENT`, so nothing has to switch it on here.
    set_param(&mut session, "Cutoff from Envelope amount", 0.5); // centre: no modulation
    let closed = energy(&mut session);
    set_param(&mut session, "Cutoff from Envelope amount", 1.0);
    let opened = energy(&mut session);
    assert!(
        opened > closed * 1.5,
        "the filter envelope did nothing through the host: {closed} -> {opened}"
    );
}

/// **A loop crossfade takes the step out of a seam that does not close, through a real instance.**
///
/// The payload is 8,192 frames of 220 Hz — 37.55 cycles — so looped whole its two ends do not meet
/// and every pass steps. The same patch with the crossfade raised must step far less. The DSP's own
/// tests hold the seam on the engine; this holds it through the bundle, the parameter path and the
/// host. Two sessions rather than two notes in one, so a release tail cannot sit under the second.
#[test]
fn a_loop_crossfade_smooths_the_seam_through_the_host() {
    let largest_step = |name: &str, reader: f64, crossfade: f64| -> Option<f32> {
        let mut session = loaded(name)?;
        set_param(&mut session, "A Playback mode", reader);
        set_param(&mut session, "A Loop", 0.5); // Forward
        // Off, so a step is not smoothed away by the init patch's low pass before it is measured.
        set_param(&mut session, "Filter mode", 0.0);
        set_param(&mut session, "A Loop crossfade", crossfade);
        session.clear_capture();
        session.app().note_on(60, 0.8);
        session.advance_blocks(96).unwrap();
        session.app().note_off(60);
        let captured = session.captured();
        assert!(
            captured.iter().all(|sample| sample.is_finite()),
            "{name} produced a non-finite sample"
        );
        // Interleaved stereo: one channel, past the attack.
        let left: Vec<f32> = captured.iter().step_by(2).copied().collect();
        let settled = &left[left.len() / 4..];
        Some(
            settled
                .windows(2)
                .map(|pair| (pair[1] - pair[0]).abs())
                .fold(0.0f32, f32::max),
        )
    };
    // **Both readers that read a loop**, each in its own pair of sessions. Grain never reads the
    // loop, and Stretch's path through the seam — heads that wrap on their own schedule, an alignment
    // search reading the same frames — is not proved by Repitch's.
    for (reader, position) in [("repitch", 0.0), ("stretch", 0.5)] {
        let Some(hard) = largest_step(
            &format!("creative-sampler-seam-{reader}-hard"),
            position,
            0.0,
        ) else {
            eprintln!("{SKIP}");
            return;
        };
        let faded = largest_step(
            &format!("creative-sampler-seam-{reader}-faded"),
            position,
            0.5,
        )
        .unwrap_or_else(|| unreachable!("the same bundle was present above"));
        assert!(
            hard > 0.01,
            "{reader}: the hard seam did not step, so this measures nothing: {hard}"
        );
        assert!(
            faded < hard * 0.5,
            "{reader}: the crossfade did not smooth the seam through the host: {hard} -> {faded}"
        );
    }
}
