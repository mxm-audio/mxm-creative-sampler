//! Persist and publish complete A/B sample snapshots.
//!
//! The callback only takes `AssetReadGuard`s. Writers own a mutex that the callback never touches,
//! reserve an inactive slot, replace/drop its old allocation away from audio, then publish one
//! index. Reader counts keep a later writer from reclaiming a slot still borrowed by audio.
//!
//! **What the payload is, how it is checked and decoded, and how a file is imported, is the state
//! crate's** (`mxm_creative_sampler_state::payload` and `import`), so the web tracker reads and
//! writes exactly what this publishes. This keeps the publication: the slots, the barrier and the
//! revision every arrival is settled by.

use mxm_creative_sampler_dsp::Sample;
use nice_plug::params::persist::PersistentField;
use std::cell::UnsafeCell;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub use mxm_creative_sampler_state::import::Arrival;
pub use mxm_creative_sampler_state::payload::{
    AssetPayload, DEFAULT_SOURCE_ROOT, EncodedSample, MAX_FRAMES_PER_LAYER, NOT_ENOUGH_MEMORY,
    default_source, default_source_name, initial_payload, validate_payload,
    validate_serialized_payload,
};
use mxm_creative_sampler_state::payload::{PayloadView, encode_generated};

const SLOT_COUNT: usize = 3;
const WRITER: usize = 1usize << (usize::BITS - 1);

#[derive(Debug, Clone, Default)]
pub struct AssetSnapshot {
    pub samples: [Option<Sample>; 2],
    pub names: [Option<String>; 2],
    pub revision: u64,
    /// The revision that last replaced each layer, so the callback can tell a layer that arrived from
    /// one carried across unchanged, and settle only the arrival's smoothers.
    pub layer_revisions: [u64; 2],
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
        let fingerprint = payload.fingerprint();
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
        PayloadView::from_value(state)?.validate()
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
        let snapshot = snapshot(view.prepare()?, revision);
        let payload = view.to_payload()?;
        if !self.bank.stage(snapshot) {
            return Err("sample publication is busy; the previous sound was kept".to_owned());
        }
        *self
            .layer_revisions
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = [revision, revision];
        let fingerprint = payload.fingerprint();
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
        let imported = mxm_creative_sampler_state::import::decode_file(path)?;
        let arrival = imported.arrival();
        self.stage_layer(layer, imported.sample, arrival)
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
        let fingerprint = payload.fingerprint();
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
        let fingerprint = payload.fingerprint();
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
                    let fingerprint = payload.fingerprint();
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

/// The audible snapshot a payload describes, at `revision`: each layer decoded once by the state
/// crate's `payload::prepare`, which checks everything as it goes and reserves every buffer
/// fallibly, so a refusal returns before anything is staged or published.
fn payload_to_snapshot(payload: &AssetPayload, revision: u64) -> Result<AssetSnapshot, String> {
    mxm_creative_sampler_state::payload::prepare(payload)
        .map(|prepared| snapshot(prepared, revision))
}

fn snapshot(
    prepared: mxm_creative_sampler_state::payload::Prepared,
    revision: u64,
) -> AssetSnapshot {
    AssetSnapshot {
        samples: prepared.samples,
        names: prepared.names,
        revision,
        // Every layer arrives with a whole payload; a stage that replaces one layer says so after.
        layer_revisions: [revision, revision],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use mxm_creative_sampler_state::payload::refusal;
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
    /// that was playing** (audit D14; mxm-kit's `docs/code-review-notes.md` §1). The frame cap
    /// bounds each layer, but a bounded allocation still fails, and an infallible one that fails
    /// aborts the host.
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
}
