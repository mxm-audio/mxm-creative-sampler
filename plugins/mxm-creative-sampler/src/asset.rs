//! Decode, persist and publish complete A/B sample snapshots.
//!
//! The callback only takes `AssetReadGuard`s. Writers own a mutex that the callback never touches,
//! reserve an inactive slot, replace/drop its old allocation away from audio, then publish one
//! index. Reader counts keep a later writer from reclaiming a slot still borrowed by audio.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use mxm_creative_sampler_dsp::{Sample, SampleError};
use nice_plug::params::persist::PersistentField;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cell::UnsafeCell;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub const MAX_FRAMES_PER_LAYER: usize = 5_760_000;
const SLOT_COUNT: usize = 3;
const WRITER: usize = 1usize << (usize::BITS - 1);

/// What a refused reservation reports. The previous sound is kept, which every caller guarantees.
const NOT_ENOUGH_MEMORY: &str =
    "not enough memory for the embedded audio; the previous sound was kept";

/// Reserves exactly `additional` more elements, or refuses.
///
/// **Every buffer sized by a payload, a preset or an imported file is reserved here** (audit D14;
/// `docs/code-review-notes.md` §1). The frame cap bounds a layer at 43.95 MiB of frames, and a cap
/// does not make that allocation succeed: an infallible one that fails aborts the host. `what` names
/// the buffer for the refusal tests.
fn reserve<T>(buffer: &mut Vec<T>, additional: usize, what: &'static str) -> Result<(), String> {
    if refused_for_test(what) {
        return Err(NOT_ENOUGH_MEMORY.to_owned());
    }
    buffer
        .try_reserve_exact(additional)
        .map_err(|_| NOT_ENOUGH_MEMORY.to_owned())
}

/// [`reserve`], for text.
fn reserve_text(buffer: &mut String, additional: usize, what: &'static str) -> Result<(), String> {
    if refused_for_test(what) {
        return Err(NOT_ENOUGH_MEMORY.to_owned());
    }
    buffer
        .try_reserve_exact(additional)
        .map_err(|_| NOT_ENOUGH_MEMORY.to_owned())
}

/// Whether a test has asked for this reservation to be refused. Always `false` outside tests.
#[cfg(test)]
fn refused_for_test(what: &'static str) -> bool {
    refusal::refuses(what)
}

#[cfg(not(test))]
fn refused_for_test(_: &'static str) -> bool {
    false
}

/// Refusals injected at the reservation seam, per thread so parallel tests cannot see each other's.
#[cfg(test)]
pub(crate) mod refusal {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static ARMED: Cell<bool> = const { Cell::new(false) };
        static REFUSE_AT: Cell<Option<usize>> = const { Cell::new(None) };
        static SEEN: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        static REFUSED: Cell<bool> = const { Cell::new(false) };
    }

    /// Starts recording reservations, refusing the `at`-th (from zero) if there is one.
    pub fn arm(at: Option<usize>) {
        ARMED.set(true);
        REFUSE_AT.set(at);
        REFUSED.set(false);
        SEEN.with_borrow_mut(Vec::clear);
    }

    /// Stops refusing; returns every reservation asked for since [`arm`] and whether one was refused.
    pub fn disarm() -> (Vec<&'static str>, bool) {
        ARMED.set(false);
        REFUSE_AT.set(None);
        (SEEN.with_borrow_mut(std::mem::take), REFUSED.replace(false))
    }

    pub(super) fn refuses(what: &'static str) -> bool {
        if !ARMED.get() {
            return false;
        }
        let index = SEEN.with_borrow_mut(|seen| {
            seen.push(what);
            seen.len() - 1
        });
        let refused = REFUSE_AT.get() == Some(index);
        if refused {
            REFUSED.set(true);
        }
        refused
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncodedSample {
    pub name: String,
    pub sample_rate: u32,
    pub frames: u32,
    /// Canonical little-endian interleaved stereo i16, base64 encoded.
    pub pcm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetPayload {
    pub schema: u32,
    pub layers: [Option<EncodedSample>; 2],
}

impl Default for AssetPayload {
    fn default() -> Self {
        Self {
            schema: 1,
            layers: [None, None],
        }
    }
}

/// An [`AssetPayload`] read in place, its text borrowed from the JSON it was read out of.
///
/// **Reading a payload does not copy its audio** (audit D14). Checking a restored state parsed its
/// base64 into owned strings, and a preset cloned its whole JSON value first, before a byte of either
/// was checked. Borrowed, a check copies nothing, and preparation copies only what it keeps, through
/// [`reserve_text`]. A string with an escape in it cannot be borrowed and is copied by the parser;
/// canonical base64 has none.
#[derive(Deserialize)]
struct PayloadView<'a> {
    schema: u32,
    #[serde(borrow)]
    layers: [Option<LayerView<'a>>; 2],
}

#[derive(Deserialize)]
struct LayerView<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    sample_rate: u32,
    frames: u32,
    #[serde(borrow)]
    pcm: Cow<'a, str>,
}

impl<'a> PayloadView<'a> {
    fn from_value(state: &'a serde_json::Value) -> Result<Self, String> {
        Self::deserialize(state).map_err(|error| format!("invalid embedded sample state: {error}"))
    }

    fn layers(&self) -> [Option<Layer<'_>>; 2] {
        self.layers.each_ref().map(|layer| {
            layer.as_ref().map(|layer| Layer {
                name: &layer.name,
                sample_rate: layer.sample_rate,
                frames: layer.frames,
                pcm: &layer.pcm,
            })
        })
    }

    /// The payload to keep, every string of it reserved fallibly.
    fn to_payload(&self) -> Result<AssetPayload, String> {
        let [a, b] = self.layers();
        Ok(AssetPayload {
            schema: self.schema,
            layers: [
                a.map(Layer::to_encoded).transpose()?,
                b.map(Layer::to_encoded).transpose()?,
            ],
        })
    }
}

/// One layer's fields, whether an [`EncodedSample`] or a [`PayloadView`] holds them.
#[derive(Clone, Copy)]
struct Layer<'a> {
    name: &'a str,
    sample_rate: u32,
    frames: u32,
    pcm: &'a str,
}

impl Layer<'_> {
    /// An owned copy, every buffer reserved fallibly.
    fn to_encoded(self) -> Result<EncodedSample, String> {
        let mut name = String::new();
        reserve_text(&mut name, self.name.len(), "encoded name")?;
        name.push_str(self.name);
        let mut pcm = String::new();
        reserve_text(&mut pcm, self.pcm.len(), "encoded audio")?;
        pcm.push_str(self.pcm);
        Ok(EncodedSample {
            name,
            sample_rate: self.sample_rate,
            frames: self.frames,
            pcm,
        })
    }
}

impl EncodedSample {
    fn layer(&self) -> Layer<'_> {
        Layer {
            name: &self.name,
            sample_rate: self.sample_rate,
            frames: self.frames,
            pcm: &self.pcm,
        }
    }
}

impl AssetPayload {
    fn layer_refs(&self) -> [Option<Layer<'_>>; 2] {
        self.layers
            .each_ref()
            .map(|layer| layer.as_ref().map(EncodedSample::layer))
    }

    /// A copy without layer `left_out`, every buffer reserved fallibly: the part of the payload a
    /// one-layer change carries across.
    fn copy_without(&self, left_out: usize) -> Result<Self, String> {
        let [a, b] = self.layer_refs();
        Ok(Self {
            schema: self.schema,
            layers: [
                a.filter(|_| left_out != 0)
                    .map(Layer::to_encoded)
                    .transpose()?,
                b.filter(|_| left_out != 1)
                    .map(Layer::to_encoded)
                    .transpose()?,
            ],
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct AssetSnapshot {
    pub samples: [Option<Sample>; 2],
    pub names: [Option<String>; 2],
    pub revision: u64,
    /// The revision that last replaced each layer, so the callback can tell a layer that arrived from
    /// one carried across unchanged, and settle only the arrival's smoothers.
    pub layer_revisions: [u64; 2],
}

/// What a staged source says about its own loop and pitch, for the editor to write as source-owned
/// defaults before anyone hears it (`plans/plan-sampler-wav-loop-import.md` §1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arrival {
    /// A generated source: built to loop over its whole buffer, with its root in its name.
    Generated,
    /// A dropped file, and the loop it carries when it carries a usable one.
    File(Option<crate::wav_loop::FileLoop>),
}

struct Slot {
    state: AtomicUsize,
    value: UnsafeCell<Option<Box<AssetSnapshot>>>,
}

impl Slot {
    fn empty() -> Self {
        Self {
            state: AtomicUsize::new(0),
            value: UnsafeCell::new(None),
        }
    }
}

// `value` is read only while its slot has a reader count and replaced only after the writer changes
// state from exactly zero to WRITER. The writer mutex serializes publication. Those rules are the
// synchronization that UnsafeCell itself cannot express.
unsafe impl Sync for Slot {}

pub struct AssetBank {
    slots: [Slot; SLOT_COUNT],
    active: AtomicUsize,
    /// A complete revision that is written but not yet audible, or [`NOTHING_STAGED`].
    staged: AtomicUsize,
    writer: Mutex<()>,
}

/// `staged` when there is no uncommitted revision.
const NOTHING_STAGED: usize = usize::MAX;

impl std::fmt::Debug for AssetBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetBank")
            .field("active", &self.active.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Default for AssetBank {
    fn default() -> Self {
        let bank = Self {
            slots: std::array::from_fn(|_| Slot::empty()),
            active: AtomicUsize::new(0),
            staged: AtomicUsize::new(NOTHING_STAGED),
            writer: Mutex::new(()),
        };
        // The object is not shared yet, so this initialization needs no state transition.
        unsafe {
            *bank.slots[0].value.get() = Some(Box::new(AssetSnapshot::default()));
        }
        bank
    }
}

impl AssetBank {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Acquire the complete active snapshot without allocation, locks or reference-count traffic.
    pub fn read(&self) -> AssetReadGuard<'_> {
        loop {
            let index = self.active.load(Ordering::Acquire);
            let slot = &self.slots[index];
            let mut state = slot.state.load(Ordering::Acquire);
            loop {
                if state & WRITER != 0 {
                    core::hint::spin_loop();
                    break;
                }
                debug_assert!(state < WRITER - 1, "asset reader count exhausted");
                match slot.state.compare_exchange_weak(
                    state,
                    state + 1,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        if self.active.load(Ordering::Acquire) == index {
                            return AssetReadGuard { bank: self, index };
                        }
                        slot.state.fetch_sub(1, Ordering::Release);
                        break;
                    }
                    Err(seen) => state = seen,
                }
            }
        }
    }

    /// Write a complete revision from a non-audio thread **without making it audible**.
    ///
    /// This is the first half of the commit barrier. A revision that arrives with parameters —
    /// a preset's region and root, an import's reset endpoints — must not be heard before them,
    /// or a hostile or merely busy host can render one block of the new audio through the old
    /// crop. Staging does all of the expensive work (allocation, copying, destroying the retired
    /// `Box`) off the audio thread and leaves exactly one atomic store for [`Self::commit`].
    ///
    /// With three slots and one serialized process callback there is always a reclaimable
    /// inactive slot; returning `false` preserves the old sound if an unusual external reader
    /// holds the retired generation. An uncommitted staged slot is itself reclaimable — it has no
    /// readers by construction, so a second stage before a commit replaces the revision nobody has
    /// heard yet rather than failing.
    pub fn stage(&self, snapshot: AssetSnapshot) -> bool {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let active = self.active.load(Ordering::Acquire);
        for index in 0..SLOT_COUNT {
            if index == active {
                continue;
            }
            let slot = &self.slots[index];
            if slot
                .state
                .compare_exchange(0, WRITER, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            // No reader can begin on an inactive WRITER slot, and a zero-to-WRITER transition proves
            // every previous reader is gone. Replacement and destruction therefore happen here.
            unsafe {
                *slot.value.get() = Some(Box::new(snapshot));
            }
            slot.state.store(0, Ordering::Release);
            self.staged.store(index, Ordering::Release);
            return true;
        }
        false
    }

    /// Make the staged revision audible. One atomic store, and idempotent when nothing is staged.
    ///
    /// **The barrier is the process boundary, not this call.** `process` takes one guard for the
    /// whole callback, so a commit that lands mid-block is not seen until the next one; a commit
    /// that lands between blocks is seen whole, with the parameters that were written before it.
    /// It is taken under the writer mutex so a concurrent stage cannot reserve the slot this is
    /// about to activate.
    pub fn commit(&self) -> bool {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staged = self.staged.swap(NOTHING_STAGED, Ordering::AcqRel);
        if staged == NOTHING_STAGED {
            return false;
        }
        self.active.store(staged, Ordering::Release);
        true
    }

    /// Publish the staged revision **only if it is the one the caller decided about**.
    ///
    /// The editor decides in one step and publishes in another: it reads the revision, writes
    /// that revision's region and root, then commits. A background load can stage in between,
    /// and a plain [`Self::commit`] publishes whatever it finds — which is how a revision could
    /// become audible carrying the *previous* one's crop, the one thing this barrier exists to
    /// prevent. Naming the revision closes the gap: a snapshot that arrived after the decision
    /// stays staged, and the editor settles it on the next frame, when it can see it.
    ///
    /// Returns whether anything became audible.
    pub fn commit_revision(&self, expected: u64) -> bool {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staged = self.staged.load(Ordering::Acquire);
        if staged == NOTHING_STAGED {
            return false;
        }
        // SAFETY: a staged slot has no readers by construction — it has never been active — and
        // the writer mutex excludes every other writer. This is the same rule `stage` relies on.
        let matches = unsafe { &*self.slots[staged].value.get() }
            .as_ref()
            .is_some_and(|snapshot| snapshot.revision == expected);
        if !matches {
            return false;
        }
        self.staged.store(NOTHING_STAGED, Ordering::Release);
        self.active.store(staged, Ordering::Release);
        true
    }

    /// Look at the revision that is written but not yet audible.
    ///
    /// **A source's root has to be decided before anyone can hear it**, and deciding it needs the
    /// audio — which, by the whole point of the barrier, is not in the active slot yet. Reading
    /// the staged slot is what lets the root be settled on the correct side of publication
    /// instead of a callback or two after it.
    pub fn with_staged<R>(&self, act: impl FnOnce(&AssetSnapshot) -> R) -> Option<R> {
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let staged = self.staged.load(Ordering::Acquire);
        if staged == NOTHING_STAGED {
            return None;
        }
        // SAFETY: as in `commit_revision` — a staged slot has no readers, and the writer mutex is
        // held for the whole of `act`, so nothing can replace the snapshot underneath it.
        unsafe { &*self.slots[staged].value.get() }
            .as_ref()
            .map(|snapshot| act(snapshot))
    }

    /// Stage and commit in one step, for a change that carries no parameter gestures with it.
    pub fn publish(&self, snapshot: AssetSnapshot) -> bool {
        self.stage(snapshot) && self.commit()
    }
}

pub struct AssetReadGuard<'a> {
    bank: &'a AssetBank,
    index: usize,
}

impl std::ops::Deref for AssetReadGuard<'_> {
    type Target = AssetSnapshot;

    fn deref(&self) -> &Self::Target {
        // Publication never makes an empty slot active. The reader count prevents replacement for
        // this guard's lifetime.
        unsafe {
            (*self.bank.slots[self.index].value.get())
                .as_deref()
                .expect("the active asset slot is populated")
        }
    }
}

impl Drop for AssetReadGuard<'_> {
    fn drop(&mut self) {
        let old = self.bank.slots[self.index]
            .state
            .fetch_sub(1, Ordering::Release);
        debug_assert!(old > 0 && old & WRITER == 0);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Idle,
    Loading { layer: usize, name: String },
    Ready { layer: usize, name: String },
    Failed { layer: usize, message: String },
}

#[derive(Debug)]
pub struct AssetField {
    payload: RwLock<AssetPayload>,
    bank: Arc<AssetBank>,
    status: RwLock<LoadState>,
    revision: AtomicU64,
    /// What the staged revision says about its loop, **stored with the revision it describes**.
    ///
    /// **A generated source is built to loop, and so is a file that carries a loop; a file without
    /// one is not.** Every partial in `crate::generate` takes a whole number of cycles so the buffer
    /// joins to itself, and a WAV's `smpl` chunk says where its maker meant it to repeat. Arriving with
    /// the layer's loop switched off contradicts both: Repitch and Stretch play once and stop, while
    /// Grain keeps sounding because it never reads the loop — exactly how the generated case was
    /// reported. A file with no loop must not be forced to repeat: a drum hit that suddenly repeats
    /// is a worse defect than a waveform that does not.
    ///
    /// **Paired with its revision, not beside it.** A later stage replaces both at once, and the
    /// editor asks for the record *of the revision it is settling*, so another revision's loop can
    /// never be written — the boolean this replaced could only say which kind the latest stage was.
    staged_arrival: Mutex<(u64, Arrival)>,
    /// The revision that last replaced each layer, carried into every snapshot. See
    /// [`AssetSnapshot::layer_revisions`].
    layer_revisions: Mutex<[u64; 2]>,
    /// The revision whose source-owned defaults have been written.
    ///
    /// **Staging and *handling* are different events, and only one of them was tracked.** The
    /// editor writes a new layer's region, loop and root defaults when it sees a new revision and
    /// commits after them; if the editor is not there to see it, the revision was staged, committed
    /// blind by `editor_closed`, and the new audio played through the previous source's crop — or
    /// was never committed at all, and a reopened editor started from the revision already current
    /// so the transition never came. This says which revision has actually been dealt with, so both
    /// paths can ask.
    handled: AtomicU64,
    fingerprint: AtomicU64,
}

impl Default for AssetField {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetField {
    /// A fresh instance starts on the generated saw, already audible.
    ///
    /// Published rather than staged: there are no parameter gestures to sequence behind it, since
    /// the init patch's own defaults already describe this source. If generation or encoding ever
    /// failed to round-trip, the bank keeps its empty state and the instrument is silent until
    /// something is dropped on it — the same failure-atomic rule every import follows.
    pub fn new() -> Self {
        let payload = initial_payload();
        let bank = AssetBank::shared();
        let published = payload_to_snapshot(&payload, 1)
            .map(|snapshot| bank.publish(snapshot))
            .unwrap_or(false);
        let (payload, status) = if published {
            (
                payload,
                LoadState::Ready {
                    layer: 0,
                    name: default_source_name(),
                },
            )
        } else {
            (AssetPayload::default(), LoadState::Idle)
        };
        let fingerprint = fingerprint_payload(&payload);
        Self {
            payload: RwLock::new(payload),
            bank,
            status: RwLock::new(status),
            revision: AtomicU64::new(1),
            // Revision 1 *is* a generated source — the init saw — and the init patch already loops
            // it, which is why this never surfaced until a different source was picked.
            staged_arrival: Mutex::new((1, Arrival::Generated)),
            layer_revisions: Mutex::new([1, 1]),
            // The init patch's own defaults already describe this source, so revision 1 arrives
            // handled: a freshly opened editor must not reset a region the player has edited.
            handled: AtomicU64::new(1),
            fingerprint: AtomicU64::new(fingerprint),
        }
    }

    pub fn bank(&self) -> Arc<AssetBank> {
        Arc::clone(&self.bank)
    }

    /// The revision whose defaults have been written. See [`AssetField::handled`].
    pub fn handled(&self) -> u64 {
        self.handled.load(Ordering::Acquire)
    }

    /// Record that `revision`'s source-owned defaults are written.
    pub fn mark_handled(&self, revision: u64) {
        self.handled.store(revision, Ordering::Release);
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn fingerprint(&self) -> u64 {
        self.fingerprint.load(Ordering::Acquire)
    }

    pub fn preset_state(&self) -> serde_json::Value {
        let payload = self
            .payload
            .read()
            .unwrap_or_else(|error| error.into_inner());
        serde_json::to_value(&*payload).expect("AssetPayload serialization is infallible")
    }

    /// Checks a preset's embedded audio completely, reading it in place: no copy of the preset and
    /// no buffer for its audio.
    pub fn validate_preset_state(state: &serde_json::Value) -> Result<(), String> {
        let view = PayloadView::from_value(state)?;
        validate_layers(view.schema, view.layers())
    }

    /// Make everything staged audible. Idempotent, and safe to call on a revision that carried no
    /// content of its own.
    pub fn commit(&self) -> bool {
        self.bank.commit()
    }

    /// See [`AssetBank::commit_revision`].
    pub fn commit_revision(&self, expected: u64) -> bool {
        self.bank.commit_revision(expected)
    }

    /// See [`AssetBank::with_staged`].
    pub fn with_staged<R>(&self, act: impl FnOnce(&AssetSnapshot) -> R) -> Option<R> {
        self.bank.with_staged(act)
    }

    /// Prepare a preset's audio **without publishing it**; [`Self::commit`] is called after the
    /// preset's parameter gestures, so the new content is never heard through the old patch.
    pub fn apply_preset_state(&self, state: &serde_json::Value) -> Result<(), String> {
        let view = PayloadView::from_value(state)?;
        let revision = self.revision.load(Ordering::Relaxed).wrapping_add(1);
        // A preset replaces both layers, so both arrive. Prepared from the preset's own JSON, which
        // checks it as it decodes; the copy kept for saving is made after, and reserved too.
        let snapshot = prepare(view.schema, view.layers(), revision)?;
        let payload = view.to_payload()?;
        if !self.bank.stage(snapshot) {
            return Err("sample publication is busy; the previous sound was kept".to_owned());
        }
        *self
            .layer_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = [revision, revision];
        let fingerprint = fingerprint_payload(&payload);
        *self
            .payload
            .write()
            .unwrap_or_else(|error| error.into_inner()) = payload;
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Idle;
        self.fingerprint.store(fingerprint, Ordering::Release);
        self.revision.store(revision, Ordering::Release);
        Ok(())
    }

    pub fn status(&self) -> LoadState {
        self.status
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn set_loading(&self, layer: usize, path: &Path) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sample");
        self.set_loading_named(layer, name);
    }

    /// The same, for a source that has a name but no file.
    ///
    /// A generated pick has nothing on disk, and `Path::new("Pulse sweep C3")` would work while
    /// saying something untrue about where the sound came from.
    pub fn set_loading_named(&self, layer: usize, name: &str) {
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Loading {
            layer,
            name: name.to_owned(),
        };
        self.revision.fetch_add(1, Ordering::Release);
    }

    pub fn fail(&self, layer: usize, message: impl Into<String>) {
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Failed {
            layer,
            message: message.into(),
        };
        self.revision.fetch_add(1, Ordering::Release);
    }

    pub fn import_wav(&self, layer: usize, path: &Path) -> Result<(), String> {
        // **A file loops when it says it does.** Its `smpl` loop arrives with it; a file with none is
        // not forced to loop, because a drum hit that suddenly repeats is a worse defect than a
        // waveform that does not.
        let (encoded, file_loop) = decode_wav(path)?;
        self.stage_layer(layer, encoded, Arrival::File(file_loop))
    }

    /// Put one of the generated sources into a layer, replacing whatever is there.
    ///
    /// **The same path a dropped file takes**, deliberately: it stages rather than publishes, so
    /// the editor writes the layer's region and root defaults before the sound changes, and the
    /// name carries the root so `root_in_name` tunes it. Publishing here instead would let
    /// at least one callback read the new audio through the *previous* sample's crop — normalised
    /// region bounds map proportionally, so a morph could be heard through the middle 16% of
    /// itself before the reset landed.
    pub fn generate_source(
        &self,
        layer: usize,
        recipe: crate::generate::Recipe,
    ) -> Result<(), String> {
        self.stage_layer(layer, encode_generated(recipe), Arrival::Generated)
    }

    /// What `revision` said about its loop when it was staged, or `None` when the record belongs to a
    /// later revision. **The editor writes a source's defaults only for the revision it is settling.**
    pub fn arrival(&self, revision: u64) -> Option<Arrival> {
        let (staged, arrival) = *self
            .staged_arrival
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (staged == revision).then_some(arrival)
    }

    /// Replaces the staged revision's arrival record, so a test can reach a file's arrival without a
    /// real WAV on disk. The distinction under test is what the publication said about its loop.
    #[cfg(test)]
    pub fn set_arrival_for_test(&self, arrival: Arrival) {
        *self
            .staged_arrival
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = (self.revision(), arrival);
    }

    /// Stage an already-encoded sample into a layer: the ordering every publication path shares.
    ///
    /// The revision is read, incremented and carried *into* the snapshot before being stored last,
    /// because the editor treats seeing a new revision as a promise that the status beside it is
    /// already true — which is what `consecutive_imports_each_announce_themselves_as_ready` exists
    /// to hold.
    fn stage_layer(
        &self,
        layer: usize,
        encoded: EncodedSample,
        arrival: Arrival,
    ) -> Result<(), String> {
        if layer >= 2 {
            return Err("the destination layer does not exist".to_owned());
        }
        // The layer left alone is copied, fallibly; the one being replaced is not copied at all.
        let mut payload = self
            .payload
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .copy_without(layer)?;
        payload.layers[layer] = Some(encoded);
        let revision = self.revision.load(Ordering::Relaxed).wrapping_add(1);
        let mut snapshot = payload_to_snapshot(&payload, revision)?;
        let mut layer_revisions = *self
            .layer_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        layer_revisions[layer] = revision;
        snapshot.layer_revisions = layer_revisions;
        // Staged, not published: the editor writes this layer's root, region and loop defaults
        // when it sees the new revision, and commits after them.
        if !self.bank.stage(snapshot) {
            return Err("sample publication is busy; the previous sound was kept".to_owned());
        }
        *self
            .layer_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = layer_revisions;
        let name = payload.layers[layer]
            .as_ref()
            .map(|sample| sample.name.clone())
            .unwrap_or_default();
        let fingerprint = fingerprint_payload(&payload);
        *self
            .payload
            .write()
            .unwrap_or_else(|error| error.into_inner()) = payload;
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Ready { layer, name };
        // **Before the revision, because the revision is the barrier.** Everything the editor reads
        // beside a new revision has to be true by the time it can see that revision — the comment on
        // this function says so about the status, and this is the same promise. Setting it *after*
        // `stage_layer` returned left a window where the editor could settle a freshly staged source
        // against the previous publication's answer: a generated source left one-shot, or a dropped
        // file forced to loop.
        *self
            .staged_arrival
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = (revision, arrival);
        self.fingerprint.store(fingerprint, Ordering::Release);
        self.revision.store(revision, Ordering::Release);
        Ok(())
    }

    pub fn clear_layer(&self, layer: usize) -> Result<(), String> {
        if layer >= 2 {
            return Err("the destination layer does not exist".to_owned());
        }
        let payload = self
            .payload
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .copy_without(layer)?;
        let revision = self.revision.load(Ordering::Relaxed).wrapping_add(1);
        let mut snapshot = payload_to_snapshot(&payload, revision)?;
        let mut layer_revisions = *self
            .layer_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        layer_revisions[layer] = revision;
        snapshot.layer_revisions = layer_revisions;
        if !self.bank.publish(snapshot) {
            return Err("sample publication is busy; the previous sound was kept".to_owned());
        }
        *self
            .layer_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = layer_revisions;
        let fingerprint = fingerprint_payload(&payload);
        *self
            .payload
            .write()
            .unwrap_or_else(|error| error.into_inner()) = payload;
        *self
            .status
            .write()
            .unwrap_or_else(|error| error.into_inner()) = LoadState::Idle;
        self.fingerprint.store(fingerprint, Ordering::Release);
        self.revision.store(revision, Ordering::Release);
        Ok(())
    }
}

impl<'a> PersistentField<'a, AssetPayload> for AssetField {
    fn set(&self, payload: AssetPayload) {
        let revision = self.revision.load(Ordering::Relaxed).wrapping_add(1);
        // `Plugin::filter_state` validates this exact payload before nice-plug mutates any field.
        // Keep this defensive branch because direct `PersistentField::set` callers get the same
        // preserve-on-error semantics.
        //
        // **Published, not staged.** `deserialize_object` writes every parameter and only then
        // hands the fields to `deserialize_fields`, so a restore already arrives in the barrier's
        // order and has no later gesture to wait for.
        match payload_to_snapshot(&payload, revision) {
            Ok(snapshot) => {
                if self.bank.publish(snapshot) {
                    // **A restore replaces both layers, and the ledger has to say so** (code review
                    // round 1). A later one-layer stage copies the ledger for the layer it leaves
                    // alone; left stale, that layer read as changed and the callback snapped its
                    // smoothers under whatever automation was moving them.
                    *self
                        .layer_revisions
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = [revision, revision];
                    let fingerprint = fingerprint_payload(&payload);
                    *self
                        .payload
                        .write()
                        .unwrap_or_else(|error| error.into_inner()) = payload;
                    *self
                        .status
                        .write()
                        .unwrap_or_else(|error| error.into_inner()) = LoadState::Idle;
                    self.fingerprint.store(fingerprint, Ordering::Release);
                    self.revision.store(revision, Ordering::Release);
                } else {
                    self.fail(0, "state publication was busy; the previous sound was kept");
                }
            }
            Err(error) => self.fail(0, error),
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&AssetPayload) -> R,
    {
        f(&self
            .payload
            .read()
            .unwrap_or_else(|error| error.into_inner()))
    }
}

fn fingerprint_payload(payload: &AssetPayload) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut add = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    add(&payload.schema.to_le_bytes());
    for layer in &payload.layers {
        match layer {
            None => add(&[0]),
            Some(sample) => {
                add(&[1]);
                add(&sample.sample_rate.to_le_bytes());
                add(&sample.frames.to_le_bytes());
                add(sample.name.as_bytes());
                add(&[0xff]);
                add(sample.pcm.as_bytes());
            }
        }
    }
    hash
}

/// Checks a payload completely — schema, counts, rate and every byte of its audio — **without a
/// buffer**. See [`decode_frames`].
pub fn validate_payload(payload: &AssetPayload) -> Result<(), String> {
    validate_layers(payload.schema, payload.layer_refs())
}

/// [`validate_payload`] on a serialized `samples` field, read in place, for `Plugin::filter_state`:
/// checking a restored state copies none of its audio.
pub fn validate_serialized_payload(serialized: &str) -> Result<(), String> {
    let view: PayloadView<'_> = serde_json::from_str(serialized)
        .map_err(|error| format!("invalid embedded sample state: {error}"))?;
    validate_layers(view.schema, view.layers())
}

fn validate_layers(schema: u32, layers: [Option<Layer<'_>>; 2]) -> Result<(), String> {
    check_schema(schema)?;
    for layer in layers.into_iter().flatten() {
        let frames = check_layer(layer)?;
        decode_frames(layer, frames, |_| {})?;
    }
    Ok(())
}

fn check_schema(schema: u32) -> Result<(), String> {
    if schema == 1 {
        Ok(())
    } else {
        Err(format!("unsupported sample-state schema {schema}"))
    }
}

/// The most of a name an error message quotes. A name is external text of any length, and a message
/// that copied all of it would be one more buffer the payload sized.
const QUOTED_NAME_BYTES: usize = 120;

fn quoted(name: &str) -> &str {
    let mut end = name.len().min(QUOTED_NAME_BYTES);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// A layer's frame count, once its count, rate and encoded length agree, before a byte is decoded.
fn check_layer(layer: Layer<'_>) -> Result<usize, String> {
    let frames = layer.frames as usize;
    if frames == 0 || frames > MAX_FRAMES_PER_LAYER {
        return Err(format!(
            "{} has an invalid or oversized frame count",
            quoted(layer.name)
        ));
    }
    if !(1_000..=768_000).contains(&layer.sample_rate) {
        return Err(format!("{} has an invalid sample rate", quoted(layer.name)));
    }
    // Canonical padded base64 of `4 × frames` bytes has exactly this length; the engine refuses a
    // string that omits its padding, and any other length decodes to a different byte count.
    if layer.pcm.len() != (frames * 4).div_ceil(3) * 4 {
        return Err(format!(
            "{} has inconsistent embedded audio",
            quoted(layer.name)
        ));
    }
    Ok(frames)
}

/// Base64 characters decoded at a time: whole quads, which decode to whole frames.
const DECODE_CHARS: usize = 4_096;
const DECODE_BYTES: usize = DECODE_CHARS / 4 * 3;

/// Decodes a checked layer's audio a chunk at a time and hands `each` every canonical frame.
///
/// **One decode, into no buffer of its own** (audit D14). Validation decoded the whole string into a
/// vector it then threw away — 21.97 MiB at the frame cap, reserved infallibly — and preparation
/// decoded it again before copying it into frames. A fixed stack chunk serves both: validation
/// discards the frames, preparation pushes them into a vector [`reserve`]d once.
///
/// It accepts exactly what one whole-string decode of the right length does. A chunk decodes whole
/// quads, so padding can only shorten its last quad, by one or two bytes: a chunk before the last
/// that held padding — which the whole decode refuses — decodes off the four-byte frame grid and is
/// refused here too. The running total must stay within the layer's bytes, checked before a chunk's
/// frames are handed over, so a caller's reservation is never exceeded.
fn decode_frames(
    layer: Layer<'_>,
    frames: usize,
    mut each: impl FnMut(&[u8; 4]),
) -> Result<(), String> {
    let expected = frames * 4;
    let invalid = || format!("{} contains invalid embedded audio", quoted(layer.name));
    let truncated = || format!("{} contains truncated embedded audio", quoted(layer.name));
    let mut bytes = [0u8; DECODE_BYTES];
    let mut decoded = 0;
    for chunk in layer.pcm.as_bytes().chunks(DECODE_CHARS) {
        let written = BASE64
            .decode_slice(chunk, &mut bytes)
            .map_err(|_| invalid())?;
        decoded += written;
        let (whole, partial) = bytes[..written].as_chunks::<4>();
        if decoded > expected || !partial.is_empty() {
            return Err(truncated());
        }
        whole.iter().for_each(&mut each);
    }
    if decoded == expected {
        Ok(())
    } else {
        Err(truncated())
    }
}

/// The name the generated starting source carries, in the browser and in saved state.
///
/// **It ends in `C3`**, which is not decoration: `root_in_name` reads a trailing key off a
/// loaded source's name and tunes Root from it, so every generated source arrives in tune through
/// the path a named file already uses. See [`crate::generate::Recipe::name`].
pub fn default_source_name() -> String {
    default_source_recipe().name()
}

/// The MIDI note that plays any generated source at its recorded rate. C3.
pub const DEFAULT_SOURCE_ROOT: f32 = crate::generate::ROOT;

/// The source the instrument starts on: a band-limited saw, generated rather than shipped.
///
/// **An instrument with nothing loaded cannot be played, and four of this plugin's factory recipes
/// carry no audio of their own** — they are `state = null` by design, so they treat whatever is
/// loaded. With an empty bank they treated nothing. This is the sound they treat out of the box.
///
/// It is now one of seventeen ([`crate::generate::Recipe`]) rather than the only one, and this is the
/// entry that does not move — the owner-selected init patch is tuned against it.
pub fn default_source() -> EncodedSample {
    encode_generated(default_source_recipe())
}

fn default_source_recipe() -> crate::generate::Recipe {
    crate::init::PATCH.sources[0].expect("Init must provide the one audible source on layer A")
}

/// Render a recipe and wrap it in the payload's canonical encoding.
///
/// Project-owned by construction: additive synthesis from a named technique, so there is no
/// third-party audio in the repository and no binary in the tree.
fn encode_generated(recipe: crate::generate::Recipe) -> EncodedSample {
    let frames = crate::generate::render(recipe);
    let mut pcm = Vec::with_capacity(frames.len() * 4);
    for value in &frames {
        let scaled = (value * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16;
        // Mono content in the canonical stereo payload: the width is the instrument's to make,
        // not the source's to bake in.
        pcm.extend_from_slice(&scaled.to_le_bytes());
        pcm.extend_from_slice(&scaled.to_le_bytes());
    }
    EncodedSample {
        name: recipe.name(),
        sample_rate: crate::generate::RATE,
        frames: frames.len() as u32,
        pcm: BASE64.encode(pcm),
    }
}

/// The payload a fresh instance starts with: the generated saw on A, nothing on B.
pub fn initial_payload() -> AssetPayload {
    AssetPayload {
        schema: 1,
        layers: crate::init::PATCH
            .sources
            .map(|recipe| recipe.map(encode_generated)),
    }
}

/// One canonical stereo i16 frame, as a layer plays it.
///
/// **The one place the reconstruction lives**, so what an import measures and what a layer plays
/// cannot drift apart: a loop's pitch was once heard in the decoded floats, and quantisation can move
/// a quiet loop's peak under the detector's level floor (code review round 2).
fn canonical_frame(bytes: &[u8; 4]) -> [f32; 2] {
    [
        i16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 32_768.0,
        i16::from_le_bytes([bytes[2], bytes[3]]) as f32 / 32_768.0,
    ]
}

fn payload_to_snapshot(payload: &AssetPayload, revision: u64) -> Result<AssetSnapshot, String> {
    prepare(payload.schema, payload.layer_refs(), revision)
}

/// The audible snapshot a payload describes, **each layer decoded once, into frames reserved
/// fallibly** (audit D14).
///
/// This checks everything [`validate_payload`] checks, as it goes, so it does not call it: that
/// decode was a second pass over the same audio. Any refusal — malformed audio, or memory the
/// frames, the name or the sample's own analysis cannot have — returns before anything is staged
/// or published, so the caller keeps the sound it had.
fn prepare(
    schema: u32,
    layers: [Option<Layer<'_>>; 2],
    revision: u64,
) -> Result<AssetSnapshot, String> {
    check_schema(schema)?;
    let mut samples: [Option<Sample>; 2] = [None, None];
    let mut names: [Option<String>; 2] = [None, None];
    for (index, layer) in layers.into_iter().enumerate() {
        let Some(layer) = layer else {
            continue;
        };
        let frames = check_layer(layer)?;
        let mut decoded: Vec<[f32; 2]> = Vec::new();
        reserve(&mut decoded, frames, "frames")?;
        decode_frames(layer, frames, |bytes| decoded.push(canonical_frame(bytes)))?;
        samples[index] = Some(
            Sample::new(decoded, layer.sample_rate as f32).map_err(|error| match error {
                SampleError::Allocation => NOT_ENOUGH_MEMORY.to_owned(),
                other => format!("{} could not be prepared: {other:?}", quoted(layer.name)),
            })?,
        );
        let mut name = String::new();
        reserve_text(&mut name, layer.name.len(), "name")?;
        name.push_str(layer.name);
        names[index] = Some(name);
    }
    Ok(AssetSnapshot {
        samples,
        names,
        revision,
        // Every layer arrives with a whole payload; a stage that replaces one layer says so after.
        layer_revisions: [revision, revision],
    })
}

/// The file's audio, canonical, and the loop it carries when it carries a usable one.
///
/// **Any format the collection's decoder reads** — WAV, AIFF, FLAC, ALAC, MP3, AAC in M4A and Ogg
/// Vorbis, judged by its content rather than its extension (`crates/mxm-audio-file-decode`). The
/// instrument's policy stays here: more than [`MAX_FRAMES_PER_LAYER`] frames is refused, the first two
/// channels are kept and mono is duplicated, and the audio is quantised once to canonical stereo i16.
/// Integer PCM arrives scaled by `2^(bits−1)`, the rule this import used when hound read it, so a WAV
/// imports to the same canonical state it did before.
fn decode_wav(path: &Path) -> Result<(EncodedSample, Option<crate::wav_loop::FileLoop>), String> {
    use mxm_audio_file_decode::{AtLimit, Container, Error, Keep, Limits};
    let decoded = mxm_audio_file_decode::decode_file(
        path,
        &Limits::new(MAX_FRAMES_PER_LAYER, AtLimit::Refuse, Keep::First(2)),
    )
    .map_err(|error| match error {
        Error::TooLong { .. } => format!(
            "too long for this small-sample instrument ({MAX_FRAMES_PER_LAYER} frames maximum)"
        ),
        other => other.to_string(),
    })?;
    let container = decoded.container;
    let sample_rate = decoded.sample_rate;
    let channels = decoded.channels;
    let actual_frames = decoded.frames();
    // The decoder's own buffer is `mxm-audio-file-decode`'s to reserve. Everything this import sizes
    // from the file after it is reserved fallibly, and the samples are clamped where they lie.
    let mut interleaved = decoded.interleaved;
    for value in &mut interleaved {
        *value = value.clamp(-1.0, 1.0);
    }
    let mut pcm = Vec::new();
    reserve(&mut pcm, actual_frames * 4, "import audio")?;
    for frame in interleaved.chunks_exact(channels) {
        let left = frame[0];
        let right = if channels == 1 { left } else { frame[1] };
        let left = (left * 32_767.0).round() as i16;
        let right = (right * 32_767.0).round() as i16;
        pcm.extend_from_slice(&left.to_le_bytes());
        pcm.extend_from_slice(&right.to_le_bytes());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sample.wav")
        .to_owned();
    // **Optional metadata adds no failure** (§1.2): the audio above is the import, and a chunk list
    // that cannot be read, or holds no usable loop, only means the file has none.
    // Only a RIFF WAVE carries a `smpl` chunk; any other container has no loop to read.
    let file_loop = std::fs::File::open(path)
        .ok()
        .filter(|_| container == Container::Wav)
        .and_then(|file| {
            crate::wav_loop::read_loop(&mut std::io::BufReader::new(file), actual_frames as u32)
        })
        .map(|mut found| {
            // Measured only for a loop that repeats fast enough to be a pitch, so a phrase-length loop
            // never builds a copy of itself — and **on the canonical audio a layer plays**, not the
            // decoded floats (`canonical_frame`).
            let span = (found.end - found.start + 1) as usize;
            let rate = sample_rate as f32;
            if span as f32 <= rate / crate::wav_loop::LOOP_PITCH_FLOOR_HZ {
                let cycle: Vec<[f32; 2]> = pcm
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .skip(found.start as usize)
                    .take(span)
                    .map(canonical_frame)
                    .collect();
                found.root = crate::wav_loop::loop_pitch(&cycle, rate, found.unity).root;
            }
            found
        });
    let mut encoded = String::new();
    let encoded_len = base64::encoded_len(pcm.len(), true).ok_or(NOT_ENOUGH_MEMORY)?;
    reserve_text(&mut encoded, encoded_len, "import encoding")?;
    BASE64.encode_string(&pcm, &mut encoded);
    Ok((
        EncodedSample {
            name,
            sample_rate,
            frames: actual_frames as u32,
            pcm: encoded,
        },
        file_loop,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A mono 16-bit WAV holding exactly `codes`, through the collection's encoder: its 16-bit rule
    /// maps `code / 32767` back to `code`.
    fn write_wav16(path: &Path, rate: u32, codes: &[i16]) {
        let samples: Vec<f32> = codes
            .iter()
            .map(|&code| f32::from(code) / 32_767.0)
            .collect();
        mxm_audio_file::write(
            path,
            &samples,
            1,
            rate,
            mxm_audio_file::Target::Wav(mxm_audio_file::Bits::Sixteen),
        )
        .unwrap();
    }

    /// **A loop's pitch is heard in the audio a layer plays, not in the decoded floats** (code review
    /// round 2). A float loop just over the detector's level floor is pitched as decoded; quantised to
    /// 16 bits its peak falls under that floor, so an import that listened to the floats would
    /// retune the keyboard from audio the layer does not play. The unity note is unset, so the
    /// detector is the guide.
    #[test]
    fn a_loop_pitch_is_heard_in_the_audio_a_layer_plays() {
        use std::f32::consts::TAU;
        let dir = std::env::temp_dir().join(format!(
            "mxm-creative-sampler-canonical-pitch-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        let cycle = 169u32;
        let tone =
            |amplitude: f32, n: u32| (TAU * (n % cycle) as f32 / cycle as f32).sin() * amplitude;
        let write = |name: &str, amplitude: f32| {
            let samples: Vec<f32> = (0..4_096u32).map(|n| tone(amplitude, n)).collect();
            let (mut bytes, _) =
                mxm_audio_file::encode(&samples, 1, 44_100, mxm_audio_file::Target::WavFloat32)
                    .unwrap();
            // One forward loop over the first cycle, unity note unset.
            let mut body = vec![0u8; 36];
            body[28..32].copy_from_slice(&1u32.to_le_bytes());
            for word in [0u32, 0, 0, cycle - 1, 0, 0] {
                body.extend_from_slice(&word.to_le_bytes());
            }
            bytes.extend_from_slice(b"smpl");
            bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&body);
            let riff = (bytes.len() - 8) as u32;
            bytes[4..8].copy_from_slice(&riff.to_le_bytes());
            let path = dir.join(name);
            std::fs::write(&path, bytes).expect("a scratch WAV");
            path
        };

        // Loud: the control, that a float import measures a pitch at all.
        let (_, loud) = decode_wav(&write("loud.wav", 0.5)).expect("a float WAV decodes");
        let root = loud
            .and_then(|found| found.root)
            .expect("a loud single cycle is a pitch");
        assert!(
            (root - 59.955).abs() < 0.01,
            "the loud cycle's root is {root}"
        );

        // Quiet: pitched as floats, a step under the floor as played.
        let amplitude = 1.05e-4;
        let floats: Vec<[f32; 2]> = (0..cycle)
            .map(|n| [tone(amplitude, n), tone(amplitude, n)])
            .collect();
        assert!(
            crate::wav_loop::loop_pitch(&floats, 44_100.0, None)
                .root
                .is_some(),
            "the quiet cycle is not pitched even as floats, so this case proves nothing"
        );
        let (_, quiet) = decode_wav(&write("quiet.wav", amplitude)).expect("a float WAV decodes");
        let found = quiet.expect("the quiet file's loop is still read");
        assert_eq!(
            found.root, None,
            "a loop under the detector's floor as played set Root from its decoded floats"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What a fresh field is playing before anything is dropped on it.
    ///
    /// These assertions used to read `samples[0].is_none()`, which said "nothing is audible yet"
    /// and quietly depended on the bank starting empty. The barrier's actual claim is stronger and
    /// is what the comments always said: **the previous sound is what plays.** Now that a fresh
    /// instance starts on the generated saw, that claim is testable rather than vacuous.
    fn still_the_init_saw(bank: &AssetBank) -> bool {
        let guard = bank.read();
        guard.names[0].as_deref() == Some(default_source_name().as_str())
            && guard.samples[0].as_ref().map(|sample| sample.len()) == Some(crate::generate::FRAMES)
    }

    fn payload(value: i16) -> AssetPayload {
        let mut bytes = Vec::new();
        for _ in 0..16 {
            bytes.extend_from_slice(&value.to_le_bytes());
            bytes.extend_from_slice(&(-value).to_le_bytes());
        }
        AssetPayload {
            schema: 1,
            layers: [
                Some(EncodedSample {
                    name: "portable.wav".to_owned(),
                    sample_rate: 48_000,
                    frames: 16,
                    pcm: BASE64.encode(bytes),
                }),
                None,
            ],
        }
    }

    #[test]
    fn payload_round_trip_builds_the_same_canonical_samples() {
        let payload = payload(12_345);
        let snapshot = payload_to_snapshot(&payload, 7).unwrap();
        let sample = snapshot.samples[0].as_ref().unwrap();
        assert_eq!(
            sample.frames()[0],
            [12_345.0 / 32_768.0, -12_345.0 / 32_768.0]
        );
        assert_eq!(snapshot.revision, 7);
    }

    #[test]
    fn durable_preset_state_round_trips_and_updates_its_dirty_fingerprint() {
        let source = AssetField::new();
        let value = serde_json::to_value(payload(9_876)).unwrap();
        let empty_fingerprint = source.fingerprint();
        source.apply_preset_state(&value).unwrap();
        assert_ne!(source.fingerprint(), empty_fingerprint);
        assert_eq!(source.preset_state(), value);

        let restored = AssetField::new();
        restored.apply_preset_state(&source.preset_state()).unwrap();
        assert_eq!(restored.fingerprint(), source.fingerprint());
        let bank = restored.bank();
        assert!(
            still_the_init_saw(&bank),
            "a preset's audio was audible before its parameters were written"
        );
        assert!(restored.commit());
        assert!(bank.read().samples[0].is_some());
        assert!(!restored.commit(), "committing twice published twice");
    }

    /// The commit barrier, from the audio thread's point of view: a callback that is already
    /// rendering keeps the sample it started with, and the next one sees the new revision whole.
    #[test]
    fn a_callback_in_flight_never_changes_sample_underneath_itself() {
        let field = AssetField::new();
        field
            .apply_preset_state(&serde_json::to_value(payload(1_000)).unwrap())
            .unwrap();
        field.commit();
        let bank = field.bank();

        let rendering = bank.read();
        let first = rendering.revision;
        assert!(rendering.samples[0].is_some());

        // A whole preset arrives mid-block: prepared, then the parameters, then the commit.
        field
            .apply_preset_state(&serde_json::to_value(payload(2_000)).unwrap())
            .unwrap();
        assert_eq!(rendering.revision, first, "the held guard changed content");
        assert!(field.commit());
        assert_eq!(
            rendering.revision, first,
            "a commit rewrote the snapshot a callback was already reading"
        );
        drop(rendering);

        let next = bank.read();
        assert_ne!(next.revision, first, "the next callback kept the old sound");
        assert_eq!(
            next.samples[0].as_ref().expect("a sample").frames()[0][0],
            2_000.0 / 32_768.0
        );
    }

    /// **A fresh instance can be played without dropping anything on it.**
    #[test]
    fn a_fresh_instance_starts_on_an_audible_source() {
        let field = AssetField::new();
        let bank = field.bank();
        let guard = bank.read();
        let sample = guard.samples[0].as_ref().expect("A starts loaded");
        assert_eq!(sample.len(), crate::generate::FRAMES);
        assert_eq!(
            guard.names[0].as_deref(),
            Some(default_source_name().as_str())
        );
        assert!(guard.samples[1].is_none(), "B starts empty");
        let energy: f32 = sample.frames().iter().map(|f| f[0] * f[0]).sum();
        assert!(energy > 1.0, "the starting source is silent: {energy}");
    }

    /// The generated saw loops without a click and sits exactly on the note Root is set to.
    ///
    /// Both properties come from the same choice of frame and cycle counts, and neither survives
    /// rounding them to something tidier — which is why they are asserted rather than trusted.
    #[test]
    fn the_generated_source_loops_seamlessly_and_is_in_tune() {
        let encoded = default_source();
        let payload = AssetPayload {
            schema: 1,
            layers: [Some(encoded), None],
        };
        let snapshot = payload_to_snapshot(&payload, 1).expect("the generated source decodes");
        let sample = snapshot.samples[0].as_ref().expect("a sample");
        let frames = sample.frames();

        // Seamless: the wrap from the last frame to the first is no larger a step than the
        // waveform takes anywhere else in its cycle.
        let cycle = crate::generate::FRAMES / crate::generate::CYCLES;
        let largest_internal = frames
            .windows(2)
            .map(|pair| (pair[1][0] - pair[0][0]).abs())
            .fold(0.0f32, f32::max);
        let wrap = (frames[0][0] - frames[frames.len() - 1][0]).abs();
        assert!(
            wrap <= largest_internal,
            "the loop point steps further than the waveform does: wrap {wrap} \
             against {largest_internal} over a {cycle}-frame cycle"
        );

        // In tune: the fundamental implied by the frame and cycle counts is the root, to a cent.
        let fundamental = crate::generate::RATE as f32 * crate::generate::CYCLES as f32
            / crate::generate::FRAMES as f32;
        let midi = 69.0 + 12.0 * (fundamental / 440.0).log2();
        assert!(
            (midi - DEFAULT_SOURCE_ROOT).abs() < 0.01,
            "the generated source is not on its declared root: {midi} against {DEFAULT_SOURCE_ROOT}"
        );

        // Band-limited, measured rather than inferred from the slope. The buffer holds a whole
        // number of cycles, so harmonic `n` lands exactly on DFT bin `n * cycles` and the
        // magnitude there is the partial's own amplitude with no leakage to argue about.
        let harmonic = |n: usize| {
            let mut real = 0.0f64;
            let mut imaginary = 0.0f64;
            for (index, frame) in frames.iter().enumerate() {
                let phase = -std::f64::consts::TAU
                    * n as f64
                    * crate::generate::CYCLES as f64
                    * index as f64
                    / crate::generate::FRAMES as f64;
                real += f64::from(frame[0]) * phase.cos();
                imaginary += f64::from(frame[0]) * phase.sin();
            }
            (real.hypot(imaginary) / crate::generate::FRAMES as f64) as f32
        };
        let fundamental_level = harmonic(1);
        let last_kept = (crate::generate::BANDLIMIT_HZ / fundamental).floor() as usize;
        assert!(
            fundamental_level > 0.1,
            "the fundamental is missing: {fundamental_level}"
        );
        // A saw falls as 1/n, so the last kept partial is small but must be there.
        assert!(
            harmonic(last_kept) > fundamental_level / (last_kept as f32 * 4.0),
            "the band limit cuts in early: partial {last_kept} is {}",
            harmonic(last_kept)
        );
        // And the first one past it must be gone outright, or an octave up folds.
        assert!(
            harmonic(last_kept + 1) < fundamental_level * 1.0e-4,
            "content past the band limit: partial {} is {}",
            last_kept + 1,
            harmonic(last_kept + 1)
        );
    }

    /// **The detector and the generated source agree about what note it is**, independently.
    ///
    /// `DEFAULT_SOURCE_ROOT` is asserted elsewhere from the frame and cycle counts — arithmetic on
    /// how the saw was built. This asks the question the other way round, from the audio, with the
    /// same code the Auto button runs. Two derivations meeting is worth more than either alone: if
    /// the generator is ever retuned and the constant is not, or the detector drifts, this fails.
    #[test]
    fn the_generated_source_is_heard_as_the_root_it_declares() {
        let payload = AssetPayload {
            schema: 1,
            layers: [Some(default_source()), None],
        };
        let snapshot = payload_to_snapshot(&payload, 1).expect("the generated source decodes");
        let sample = snapshot.samples[0].as_ref().expect("a sample");
        let heard = mxm_creative_sampler_dsp::detect_root(sample, 0.0)
            .expect("the generated saw has a pitch");
        assert!(
            (heard - DEFAULT_SOURCE_ROOT).abs() < 0.2,
            "the saw declares MIDI {DEFAULT_SOURCE_ROOT} and sounds like {heard}"
        );
    }

    /// **Every import in a row presents itself to the editor**, so Root is reset every time.
    ///
    /// The owner reported Root not clearing between consecutive samples. The editor resets the
    /// five source-owned fields when it sees a new revision *while the status reads `Ready`*, so
    /// the reset is lost if either the revision stops moving on a later import or the status has
    /// not caught up with it by the time the revision is visible. This walks three imports and
    /// checks that pair, in the order the editor reads them.
    #[test]
    fn consecutive_imports_each_announce_themselves_as_ready() {
        let directory = std::env::temp_dir().join("mxm-creative-sampler-consecutive");
        std::fs::create_dir_all(&directory).unwrap();
        let field = AssetField::new();
        let mut observed = field.revision();
        for round in 0..3u32 {
            let path = directory.join(format!("take-{round}.wav"));
            write_wav16(
                &path,
                48_000,
                &[0, 6_000 + round as i16 * 100, -6_000, 3_000],
            );

            field.import_wav(0, &path).unwrap();
            let revision = field.revision();
            assert_ne!(
                revision, observed,
                "import {round} did not move the revision"
            );
            // The editor reads the status in the same frame it notices the revision. `import_wav`
            // publishes the status before releasing the revision, so seeing the one guarantees the
            // other; if that order ever inverts, the reset silently stops happening and Root keeps
            // whatever the previous sample left it at.
            assert!(
                matches!(field.status(), LoadState::Ready { layer: 0, .. }),
                "import {round} moved the revision without reporting Ready, so the editor would \
                 see a new sample and skip resetting Root"
            );
            observed = revision;
            assert!(field.commit());
            let _ = std::fs::remove_file(&path);
        }
    }

    /// **Picking a generated source replaces the layer, through the staging path a file takes.**
    ///
    /// The commit is separate on purpose: until it happens the previous sound is still what plays,
    /// which is what lets the editor write the layer's region and root defaults first.
    #[test]
    fn generating_replaces_the_layer_and_carries_its_root_in_its_name() {
        for recipe in crate::generate::Recipe::ALL {
            let field = AssetField::new();
            assert!(still_the_init_saw(&field.bank()));
            field.generate_source(0, recipe).unwrap();
            if recipe != crate::generate::Recipe::InitSaw {
                assert!(
                    still_the_init_saw(&field.bank()),
                    "{} was audible before the commit",
                    recipe.label()
                );
            }
            assert!(field.commit());

            let bank = field.bank();
            let guard = bank.read();
            let expected = recipe.name();
            assert_eq!(guard.names[0].as_deref(), Some(expected.as_str()));
            let sample = guard.samples[0].as_ref().expect("a generated sample");
            assert_eq!(sample.len(), crate::generate::FRAMES);
            assert!(sample.frames().iter().flatten().all(|v| v.is_finite()));
            // The name is what tunes it, so the editor's own reader has to find the root in it.
            let key = crate::naming::key_in_name(&recipe.name()).expect("a key in the name");
            assert!((crate::naming::root_from_key(key, None) - DEFAULT_SOURCE_ROOT).abs() < 1.0e-3);
        }
    }

    /// **A staged revision is not a handled one, and both close timings depend on the difference.**
    ///
    /// Closing the editor mid-load used to commit blind, so new audio played through the previous
    /// source's crop and root; and a load that finished after the close was never committed at all,
    /// because a reopened editor started from the revision already current and so never saw the
    /// transition. `handled` is what tells the two apart — this holds the state machine both paths
    /// read, without a GUI.
    #[test]
    fn a_staged_source_is_not_marked_handled_until_its_defaults_are_written() {
        // A fresh field is handled: the init patch's own defaults describe the starting source, so
        // opening an editor must not reset a region the player has edited.
        let field = AssetField::new();
        assert_eq!(field.revision(), field.handled());

        // Staging a new source leaves work outstanding, whoever is or is not watching.
        field
            .generate_source(0, crate::generate::Recipe::Glass)
            .unwrap();
        assert_ne!(
            field.revision(),
            field.handled(),
            "a staged source claimed its defaults were already written"
        );

        // Which the editor — or `editor_closed` — clears only after writing them.
        let revision = field.revision();
        field.mark_handled(revision);
        assert_eq!(field.revision(), field.handled());
        assert!(field.commit());
    }

    /// **Every one of the seventeen survives the round trip through its payload without a seam.**
    ///
    /// The analytic test in `generate` evaluates an idealised `f64` expression, and the spectral
    /// one there measures `render`'s `f32` output — but neither sees what encoding to i16 and
    /// decoding back does, which is the form a player is handed. This runs the same seam
    /// measurement on the decoded frames, for all of them rather than only the init saw.
    #[test]
    fn every_generated_source_survives_its_payload_without_a_seam() {
        for recipe in crate::generate::Recipe::ALL {
            let payload = AssetPayload {
                schema: 1,
                layers: [Some(encode_generated(recipe)), None],
            };
            let snapshot = payload_to_snapshot(&payload, 1).expect("the generated source decodes");
            let sample = snapshot.samples[0].as_ref().expect("a sample");
            let mono: Vec<f32> = sample.frames().iter().map(|frame| frame[0]).collect();
            let seam = crate::generate::seam_out_of_band_db(&mono);
            assert!(
                seam < -55.0,
                "{} has a {seam:.1} dB seam once it has been through its payload",
                recipe.label()
            );
        }
    }

    /// **A staged revision is readable before it is audible, and that is what settles its root.**
    ///
    /// Root used to be written *after* the commit, on the reasoning that deciding a pitch needs an
    /// audible sample. It does need the sample — but the sample is staged, not absent. Publishing
    /// first left a window of one or more audio callbacks in which a generated source, every one
    /// of which is C3, played at `NEUTRAL_ROOT`'s C4: an octave out, on any note held across the
    /// pick. This is the accessor that lets the decision happen on the correct side of the
    /// barrier.
    #[test]
    fn a_staged_source_is_readable_before_it_is_audible() {
        let field = AssetField::new();
        let before = field.bank().read().names[0]
            .clone()
            .expect("the init source is published");

        field
            .generate_source(0, crate::generate::Recipe::Glass)
            .unwrap();

        // Not audible yet: the active slot still holds what it held.
        assert_eq!(
            field.bank().read().names[0].as_deref(),
            Some(before.as_str()),
            "staging must not change what is being heard"
        );
        // But readable, with its audio, which is what a root decision needs.
        let staged = field
            .with_staged(|snapshot| {
                (
                    snapshot.names[0].clone(),
                    snapshot.samples[0].is_some(),
                    snapshot.revision,
                )
            })
            .expect("a staged revision is there to be read");
        assert_ne!(staged.0.as_deref(), Some(before.as_str()));
        assert!(staged.1, "the staged revision must carry its audio");
        assert_eq!(staged.2, field.revision());

        assert!(field.commit_revision(field.revision()));
        assert_eq!(field.bank().read().names[0], staged.0);
        assert!(
            field.with_staged(|_| ()).is_none(),
            "nothing is staged once it has been published"
        );
    }

    /// **Publication names the revision it was asked for**, so a load that lands between the
    /// editor's decision and its commit is not published in its place.
    ///
    /// The editor reads a revision, writes that revision's region and root, then commits. A plain
    /// `commit` publishes whatever it finds — so a background load staging in between became
    /// audible carrying the previous source's crop and root, which is the one thing the staging
    /// barrier exists to prevent.
    #[test]
    fn publication_refuses_a_revision_it_was_not_asked_for() {
        let field = AssetField::new();
        let audible = field.bank().read().names[0].clone();

        field
            .generate_source(0, crate::generate::Recipe::Glass)
            .unwrap();
        let decided = field.revision();

        // The load the editor has not seen, landing after the decision.
        field
            .generate_source(0, crate::generate::Recipe::Drawbars)
            .unwrap();
        assert_ne!(decided, field.revision());

        assert!(
            !field.commit_revision(decided),
            "the revision the editor decided about is gone; nothing may be published for it"
        );
        assert_eq!(
            field.bank().read().names[0],
            audible,
            "a refused commit must leave the sound alone"
        );

        // And the newer one publishes normally once someone settles it.
        assert!(field.commit_revision(field.revision()));
        assert_ne!(field.bank().read().names[0], audible);
    }

    /// Generating into the layer that does not exist is refused rather than panicking.
    #[test]
    fn generating_into_a_missing_layer_is_refused() {
        let field = AssetField::new();
        assert!(
            field
                .generate_source(2, crate::generate::Recipe::Glass)
                .is_err()
        );
    }

    /// **Dropping a sample replaces the starting source** rather than sitting alongside it.
    #[test]
    fn an_import_replaces_the_starting_source() {
        let directory = std::env::temp_dir().join("mxm-creative-sampler-replaces-init");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("replacement.wav");
        write_wav16(&path, 48_000, &[0, 9_000, -9_000, 4_000]);

        let field = AssetField::new();
        assert!(still_the_init_saw(&field.bank()));
        field.import_wav(0, &path).unwrap();
        assert!(field.commit());

        let bank = field.bank();
        let guard = bank.read();
        let sample = guard.samples[0].as_ref().expect("the replacement");
        assert_eq!(sample.len(), 4, "the starting source was not replaced");
        assert_eq!(guard.names[0].as_deref(), Some("replacement.wav"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_second_stage_replaces_the_revision_nobody_has_heard() {
        let field = AssetField::new();
        let bank = field.bank();
        field
            .apply_preset_state(&serde_json::to_value(payload(11)).unwrap())
            .unwrap();
        field
            .apply_preset_state(&serde_json::to_value(payload(22)).unwrap())
            .unwrap();
        assert!(still_the_init_saw(&bank), "either stage was audible");
        assert!(field.commit());
        assert_eq!(
            bank.read().samples[0].as_ref().expect("a sample").frames()[0][0],
            22.0 / 32_768.0,
            "the committed revision was not the last one staged"
        );
    }

    #[test]
    fn malformed_payloads_are_rejected_before_publication() {
        let mut bad = payload(1);
        bad.layers[0].as_mut().unwrap().pcm.truncate(4);
        assert!(validate_payload(&bad).is_err());
        assert!(payload_to_snapshot(&bad, 1).is_err());

        let mut same_length_garbage = payload(1);
        let length = same_length_garbage.layers[0].as_ref().unwrap().pcm.len();
        same_length_garbage.layers[0].as_mut().unwrap().pcm = "!".repeat(length);
        assert!(validate_payload(&same_length_garbage).is_err());
    }

    #[test]
    fn imported_audio_restores_after_its_source_file_is_deleted() {
        let path = std::env::temp_dir().join(format!(
            "mxm-creative-sampler-{}-portable.wav",
            std::process::id()
        ));
        write_wav16(&path, 44_100, &[0, 12_345, -23_456, 1_000]);
        let field = AssetField::new();
        field.import_wav(0, &path).unwrap();
        // The import is staged, and the editor commits it after writing this layer's root, region
        // and loop defaults. Until then the previous sound is what plays.
        assert!(
            still_the_init_saw(&field.bank()),
            "an import was audible before its reset gestures"
        );
        assert!(field.commit());
        assert!(field.bank().read().samples[0].is_some());
        let serialized =
            field.map(|payload| nice_plug::params::persist::serialize_field(payload).unwrap());
        std::fs::remove_file(&path).unwrap();

        let payload: AssetPayload =
            nice_plug::params::persist::deserialize_field(&serialized).unwrap();
        let restored = AssetField::new();
        restored.set(payload);
        let bank = restored.bank();
        let snapshot = bank.read();
        let frames = snapshot.samples[0].as_ref().unwrap().frames();
        assert_eq!(frames.len(), 4);
        assert_eq!(frames[1], [12_345.0 / 32_768.0; 2]);
        assert_eq!(
            snapshot.names[0].as_deref(),
            Some(path.file_name().unwrap().to_str().unwrap())
        );
    }

    #[test]
    fn a_held_reader_prevents_reclamation_but_not_a_second_publish() {
        let bank = AssetBank::shared();
        assert!(bank.publish(payload_to_snapshot(&payload(10), 1).unwrap()));
        let held = bank.read();
        assert_eq!(held.revision, 1);
        assert!(bank.publish(payload_to_snapshot(&payload(20), 2).unwrap()));
        assert_eq!(
            held.samples[0].as_ref().unwrap().frames()[0][0],
            10.0 / 32_768.0
        );
        assert_eq!(bank.read().revision, 2);
        drop(held);
        assert!(bank.publish(payload_to_snapshot(&payload(30), 3).unwrap()));
    }

    #[test]
    fn concurrent_readers_only_see_complete_revisions() {
        let bank = AssetBank::shared();
        assert!(bank.publish(payload_to_snapshot(&payload(10), 1).unwrap()));
        let running = Arc::new(AtomicBool::new(true));
        let reader_bank = Arc::clone(&bank);
        let reader_running = Arc::clone(&running);
        let reader = std::thread::spawn(move || {
            while reader_running.load(Ordering::Relaxed) {
                let snapshot = reader_bank.read();
                assert!(matches!(snapshot.revision, 1..=100));
                let first = snapshot.samples[0].as_ref().unwrap().frames()[0][0];
                assert!(first.is_finite());
            }
        });
        for revision in 2..=100 {
            while !bank.publish(payload_to_snapshot(&payload(revision as i16), revision).unwrap()) {
                std::thread::yield_now();
            }
        }
        running.store(false, Ordering::Relaxed);
        reader.join().unwrap();
    }

    /// Two layers of `frames` frames each, every frame distinct from its neighbours and from
    /// another seed's, so a replaced payload cannot pass for a kept one. 2 000 frames spans three
    /// of the decoder's chunks.
    fn layered(seed: i16, frames: usize) -> AssetPayload {
        let layer = |offset: i16, name: &str| {
            let mut bytes = Vec::with_capacity(frames * 4);
            for frame in 0..frames {
                let value = seed.wrapping_add(offset).wrapping_add(frame as i16);
                bytes.extend_from_slice(&value.to_le_bytes());
                bytes.extend_from_slice(&value.wrapping_neg().to_le_bytes());
            }
            EncodedSample {
                name: name.to_owned(),
                sample_rate: 48_000,
                frames: frames as u32,
                pcm: BASE64.encode(bytes),
            }
        };
        AssetPayload {
            schema: 1,
            layers: [Some(layer(0, "left.wav")), Some(layer(7, "right.wav"))],
        }
    }

    /// Everything a rejected payload, preset or import must leave exactly as it was.
    #[derive(Debug, PartialEq)]
    struct Kept {
        payload: AssetPayload,
        fingerprint: u64,
        audible_revision: u64,
        audible_names: [Option<String>; 2],
        audible_frames: [Option<Vec<[f32; 2]>>; 2],
        layer_revisions: [u64; 2],
        staged: bool,
    }

    impl Kept {
        fn of(field: &AssetField) -> Self {
            let bank = field.bank();
            let audible = bank.read();
            Self {
                payload: field.map(AssetPayload::clone),
                fingerprint: field.fingerprint(),
                audible_revision: audible.revision,
                audible_names: audible.names.clone(),
                audible_frames: audible
                    .samples
                    .each_ref()
                    .map(|sample| sample.as_ref().map(|sample| sample.frames().to_vec())),
                layer_revisions: *field.layer_revisions.lock().unwrap(),
                staged: field.with_staged(|_| ()).is_some(),
            }
        }
    }

    /// Refuses each reservation `attempt` makes in turn, on a fresh field already holding `prior`,
    /// and requires every refusal to reject the attempt whole and keep the previous state. Returns
    /// the reservations a run with nothing refused asks for, in order, so a caller can hold the
    /// inventory: a buffer that bypasses the seam is missing from it.
    fn refuse_each_reservation(
        prior: &AssetPayload,
        attempt: impl Fn(&AssetField) -> bool,
    ) -> Vec<&'static str> {
        for at in 0.. {
            let field = AssetField::new();
            field.set(prior.clone());
            let kept = Kept::of(&field);
            refusal::arm(Some(at));
            let accepted = attempt(&field);
            let (seen, refused) = refusal::disarm();
            if !refused {
                assert!(accepted, "with nothing refused, the attempt was rejected");
                return seen;
            }
            assert!(
                !accepted,
                "refusing reservation {at} ({}) did not reject the whole attempt",
                seen[at]
            );
            assert_eq!(
                Kept::of(&field),
                kept,
                "refusing reservation {at} ({}) did not keep the previous state",
                seen[at]
            );
        }
        unreachable!("the reservations never ran out")
    }

    /// **A host state restore whose buffer cannot be reserved is rejected whole and keeps the sound
    /// that was playing** (audit D14; `docs/code-review-notes.md` §1). The frame cap bounds each
    /// layer, but a bounded allocation still fails, and an infallible one that fails aborts the host.
    #[test]
    fn a_refused_reservation_rejects_a_restored_state_whole_and_keeps_the_previous_one() {
        let prior = layered(1_000, 2_000);
        let restored = layered(-9_000, 2_000);
        let seen = refuse_each_reservation(&prior, |field| {
            let before = field.fingerprint();
            field.set(restored.clone());
            let accepted = field.fingerprint() != before;
            if !accepted {
                assert!(
                    matches!(field.status(), LoadState::Failed { ref message, .. }
                        if message == NOT_ENOUGH_MEMORY),
                    "a refused restore says {:?}",
                    field.status()
                );
            }
            accepted
        });
        assert_eq!(seen, ["frames", "name", "frames", "name"]);
    }

    /// **A preset whose buffers cannot be reserved stages nothing**, and checking one reserves nothing.
    #[test]
    fn a_refused_reservation_rejects_a_preset_whole_and_stages_nothing() {
        let prior = layered(1_000, 2_000);
        let preset = serde_json::to_value(layered(-9_000, 2_000)).unwrap();
        refusal::arm(None);
        assert!(AssetField::validate_preset_state(&preset).is_ok());
        assert!(
            refusal::disarm().0.is_empty(),
            "checking a preset reserved a buffer"
        );
        let seen =
            refuse_each_reservation(&prior, |field| field.apply_preset_state(&preset).is_ok());
        assert_eq!(
            seen,
            [
                "frames",
                "name",
                "frames",
                "name",
                "encoded name",
                "encoded audio",
                "encoded name",
                "encoded audio"
            ]
        );
    }

    /// **An import whose buffers cannot be reserved stages nothing and keeps the previous sound.**
    /// The decoder's own buffer belongs to `mxm-audio-file-decode`; everything after it is here.
    #[test]
    fn a_refused_reservation_rejects_an_import_whole_and_keeps_the_previous_sound() {
        let dir = std::env::temp_dir().join(format!(
            "mxm-creative-sampler-refused-import-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("import.wav");
        let codes: Vec<i16> = (0..2_000).map(|n| (n * 13 % 20_000) as i16).collect();
        write_wav16(&path, 48_000, &codes);
        let prior = layered(1_000, 2_000);
        let seen = refuse_each_reservation(&prior, |field| field.import_wav(0, &path).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            seen,
            [
                "import audio",
                "import encoding",
                "encoded name",
                "encoded audio",
                "frames",
                "name",
                "frames",
                "name"
            ]
        );
    }

    /// **The chunked decoder accepts exactly what one whole-string decode accepts, and reads the
    /// same frames.** It decodes a layer a few thousand characters at a time so validation needs no
    /// buffer and preparation needs only the frames; a chunk that decoded short in the middle of the
    /// stream (padding where none may be) would otherwise shift every frame after it.
    #[test]
    fn the_chunked_decode_accepts_exactly_what_a_whole_decode_accepts() {
        for frames in [1usize, 767, 768, 769, 1_536, 2_000] {
            let good = layered(3, frames).layers[0].clone().unwrap();
            let length = good.pcm.len();
            let mut cases = vec![good.pcm.clone()];
            let mut replaced = |at: usize, with: &str| {
                if at + with.len() <= length {
                    let mut pcm = good.pcm.clone();
                    pcm.replace_range(at..at + with.len(), with);
                    cases.push(pcm);
                }
            };
            // Padding closing the first chunk, a bad character in each place, and the last symbol
            // carrying bits the padding says are not there.
            replaced(4_096 - 1, "=");
            replaced(4_096 - 2, "==");
            replaced(0, "!");
            replaced(5_000, "*");
            replaced(length - 1, "A");
            replaced(length - 2, "AA");
            if good.pcm.ends_with("==") {
                replaced(length - 3, "B");
            } else if good.pcm.ends_with('=') {
                replaced(length - 2, "B");
            }
            // Padding closing the first chunk, paid for by filling in the padding at the end, so the
            // byte total still matches and only the frame grid can tell. The symbol before the
            // padding carries no stray bits, or the decode would refuse it for that instead.
            let pad = length - good.pcm.trim_end_matches('=').len();
            if length > 4_096 && pad > 0 {
                let mut pcm = good.pcm.clone();
                pcm.replace_range(4_096 - pad - 1..4_096, &format!("A{}", "=".repeat(pad)));
                pcm.replace_range(length - pad..length, &"A".repeat(pad));
                cases.push(pcm);
            }
            let mut short = good.pcm.clone();
            short.truncate(length - 4);
            cases.push(short);
            cases.push(format!("{}AAAA", good.pcm));

            for pcm in cases {
                let whole = BASE64
                    .decode(&pcm)
                    .ok()
                    .filter(|bytes| bytes.len() == frames * 4);
                let payload = AssetPayload {
                    schema: 1,
                    layers: [
                        Some(EncodedSample {
                            pcm: pcm.clone(),
                            ..good.clone()
                        }),
                        None,
                    ],
                };
                let tail = &pcm[pcm.len().saturating_sub(8)..];
                assert_eq!(
                    validate_payload(&payload).is_ok(),
                    whole.is_some(),
                    "{frames} frames ending {tail}: validation disagrees with a whole decode"
                );
                match (payload_to_snapshot(&payload, 1), whole) {
                    (Ok(snapshot), Some(bytes)) => {
                        let expected: Vec<[f32; 2]> = bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(canonical_frame)
                            .collect();
                        assert_eq!(
                            snapshot.samples[0].as_ref().unwrap().frames(),
                            &expected[..],
                            "{frames} frames: the chunked decode read different frames"
                        );
                    }
                    (Err(_), None) => {}
                    (snapshot, whole) => panic!(
                        "{frames} frames ending {tail}: preparation {} where a whole decode {}",
                        if snapshot.is_ok() {
                            "accepted"
                        } else {
                            "refused"
                        },
                        if whole.is_some() {
                            "accepted"
                        } else {
                            "refused"
                        }
                    ),
                }
            }
        }
    }
}
