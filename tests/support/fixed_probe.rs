//! The fixed probe value the replay journeys run with (larry#274): one value
//! per run, so the same seed replays the same prompts, gateway trace and card
//! (SD-4). Production runs the live folder probe instead: `LARRY_PROBE` is
//! unset there, and nothing here can change what a live run records.

/// The probe text a replay journey exports as `LARRY_PROBE`: one JSON object
/// holding every section, bounded like the live probe's record, naming no
/// machine of this host.
pub(crate) const FIXED_PROBE: &str = r###"{"cwd":"/probe-fixture/tree","tree":{"ok":true,"truncated":false,"entries":[{"path":"README","kind":"file","len":30},{"path":"src","kind":"dir","len":null}]},"toolchains":[{"tool":"rustc","ok":true,"version":"rustc 1.100.0 (namespace 00000000 2026-01-01)"},{"tool":"cargo","ok":true,"version":"cargo 1.100.0 (namespace 00000000 2026-01-01)"},{"tool":"node","ok":false,"error":"node is not on PATH"},{"tool":"npm","ok":false,"error":"npm is not on PATH"},{"tool":"python3","ok":false,"error":"python3 is not on PATH"},{"tool":"cc","ok":false,"error":"cc is not on PATH"},{"tool":"go","ok":false,"error":"go is not on PATH"},{"tool":"java","ok":false,"error":"java is not on PATH"}],"manifests":[{"path":"Cargo.toml","len":21}],"memory":{"ok":true,"total_kb":8000000,"avail_kb":4000000},"disk":{"ok":true,"avail_kb":600000},"git":{"ok":true,"lines":1,"truncated":false,"head":"## main"}}"###;
