//! The key variable the tenant binding gives one state directory (larry#328):
//! `LARRY_KEY_<TAG>_<SUFFIX>`. This is a reference reading of the written rule,
//! not a call into the app, so the tests judge the shipped derivation instead of
//! repeating it: the tag is the tenant text's first 16 characters, upper case,
//! where the tenant text is the BLAKE3 hex of the tenant id's 16 bytes
//! (`src/executor/identity.rs`: the state directory under `TENANT_TAG`).

use std::path::Path;

use lgwks_std::hash::Hasher;

/// Characters of the tenant text that make the tag (ADR 0084).
const TENANT_TAG_CHARS: usize = 16;

/// The domain tag the executor's tenant identity hashes first.
const TENANT_TAG: &[u8] = b"larry/executor/tenant/v1\0";

/// The BLAKE3 hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize().to_hex()
}

/// The key variable name tenant `state` may name for `suffix`, or why the tenant
/// id could not be read: BLAKE3 answers 32 bytes, so no case reaches the error.
pub(crate) fn key_env(state: &Path, suffix: &str) -> Result<String, &'static str> {
    let mut tenant = Hasher::new();
    tenant.update(TENANT_TAG);
    tenant.update(state.as_os_str().as_encoded_bytes());
    let digest = tenant.finalize();
    let Some(id) = digest.as_bytes().first_chunk::<16>() else {
        return Err("the tenant digest is shorter than a tenant id");
    };
    let tag: String = hex(id).chars().take(TENANT_TAG_CHARS).collect();
    Ok(format!("LARRY_KEY_{}_{suffix}", tag.to_ascii_uppercase()))
}
