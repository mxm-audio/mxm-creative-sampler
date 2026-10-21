# NOTES.md — crates/mxm-creative-sampler-state/

The reasoning behind [AGENTS.md](AGENTS.md).

## Why a state crate (2026-10-21)

The web tracker plays this sampler on a phone and reads and writes the plugin's own state in tutti
projects, so a song moves between newDAWn, which hosts the plugin, and the phone with its sound
(`plans/plan-web-tracker.md` in newDAWn, *The sampler on the phone*, W4). Two programs reading one
state must not be able to disagree about it, so everything the state means moved out of the plugin
into this crate, and the plugin was rebuilt on it: its parameters from the table, its per-sample
mapping through `Sound::dsp`, its import, payload and arrival through `payload` and `import`.

**It was a refactor, proved as one.** Before anything moved, every parameter of the plugin was
dumped — name, unit, step count, default, flags, its text with and without its unit and what that
text parses back to at 1 007 normalised points, ~60 typed strings, at three source rates — together
with a fresh instance's whole state JSON. The same dump of the plugin built on this crate was byte
for byte the same (35.6 MB). The dump was a throwaway; the tests that stay are the plugin's
`every_parameter_is_the_tables` and the state round trips.

**nice-plug's laws and formatters are ported, not depended on**, because nice-plug brings the
plugin framework and the crate must build for the browser. They are copied to the bit, and the
plugin's tests compare each parameter's nice-plug travel and text with the table's at every point
they check, so a nice-plug upgrade that changed either fails there.

**A route amount is still built by mxm-kit's `amount_param_at`** in the plugin, because every
modulation means the same on every instrument (the owner, 2026-09-26): the collection's one route
reading is the kit's. This crate keeps a copy for the phone, and the same plugin test holds the copy
to the kit's at every pair.

**The `preset` field is kept, not used.** It is mxm-kit's `PresetIdentity` (which preset is loaded,
and its baseline for the Modified mark); the phone has no preset browser. It is mirrored as a type,
not kept as text, because the plugin rewrites it on load for a state from before the tempo syncs
(`mxm_preset::add_switches_off`), and a state has to write back what the plugin would.

**What the plugin refuses whole, a state refuses as an error**: malformed audio, a field the plugin
does not keep, a field that would not write back as asked (nice-plug's defect-9 acceptance check).
The plugin keeps the sound it had; a program without one has to say why it read nothing.
