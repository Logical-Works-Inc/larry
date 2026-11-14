//! The identities one run derives (larry#182 defect 3): the state directory's
//! tenant, the run, operation, command and worker ids of one task, and the run id
//! itself.
//!
//! Source: RFC 9562 section 5.7 (a version 7 UUID carries a 48-bit millisecond
//! stamp, a 12-bit counter and 62 bits of OS entropy) and
//! draft-ietf-httpapi-idempotency-key-header-07 section 2 (the client's key is
//! what lets a server recognize a retry). Requirement proved: a run id never
//! repeats across processes, and a task that names an idempotency key maps to the
//! one run id its retry names, so the runtime reports the retry as a duplicate.
//!
//! A run id carries no process id: the scheme this replaces hashed the task's
//! BLAKE3 prefix with `std::process::id()` and a per-process counter, so a retry
//! got new ids and the duplicate rule never fired, and a fixed pid in a container
//! could mint the id of a killed earlier run (larry#182 defect 3).

use std::path::Path;

use larry_contracts::ids::{CommandId, OperationId, RunId, TenantId, WorkerId};

use super::ExecError;
use super::Task;
use super::digest_hex;

/// The domain tags separating every identity from every other, so no two
/// derivations collide and none collides with another repository's.
/// The state directory's tenant.
const TENANT_TAG: &[u8] = b"larry/executor/tenant/v1\0";
/// The run's.
const RUN_TAG: &[u8] = b"larry/executor/run/v1\0";
/// The operation's.
const OPERATION_TAG: &[u8] = b"larry/executor/operation/v1\0";
/// The command's.
const COMMAND_TAG: &[u8] = b"larry/executor/command/v1\0";
/// The worker's.
const WORKER_TAG: &[u8] = b"larry/executor/worker/v1\0";
/// The keyed run id's.
const KEY_TAG: &[u8] = b"larry/executor/idempotency/v1\0";

/// The first 16 bytes of the BLAKE3 digest over `parts`: the bytes a stable,
/// reproducible identity is built from (as `digest_hex` hashes).
fn identity16(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = lgwks_std::hash::Hasher::new();
    for part in parts {
        hasher.update(part);
    }
    match hasher.finalize().as_bytes().first_chunk::<16>() {
        Some(chunk) => *chunk,
        None => [0_u8; 16],
    }
}

/// The tenant one state directory is: its canonical bytes hashed under
/// [`TENANT_TAG`], so two directories never derive one tenant and no caller
/// supplies its own.
pub(super) fn tenant_of(state: &Path) -> TenantId {
    TenantId::from_bytes(identity16(&[
        TENANT_TAG,
        state.as_os_str().as_encoded_bytes(),
    ]))
}

/// The worker id one state directory's runs are held by.
pub(super) fn worker_of(state: &Path) -> WorkerId {
    WorkerId::from_bytes(identity16(&[
        WORKER_TAG,
        state.as_os_str().as_encoded_bytes(),
    ]))
}

/// The run, operation and command ids one task id names.
pub(super) fn ids_of(task_id: &str) -> (RunId, OperationId, CommandId) {
    let bytes = task_id.as_bytes();
    (
        RunId::from_bytes(identity16(&[RUN_TAG, bytes])),
        OperationId::from_bytes(identity16(&[OPERATION_TAG, bytes])),
        CommandId::from_bytes(identity16(&[COMMAND_TAG, bytes])),
    )
}

/// The one identity of a run that carries no process id at all: a UUID version 7
/// (RFC 9562 section 5.7), so a value never repeats across processes, a fixed
/// process id in a container cannot mint a killed run's id, and the text is the
/// canonical 36-byte hyphenated form on every target.
///
/// # Errors
///
/// [`ExecError::Id`] when the wall clock or the OS entropy refuses; nothing is
/// substituted for a missing value.
pub(super) fn fresh_id() -> Result<String, ExecError> {
    lgwks_std::id::Uuid::new_v7()
        .map(|id| id.to_string())
        .map_err(ExecError::Id)
}

/// The run id of one task under one state directory.
///
/// A task that names no idempotency key is a fresh identifier, so two deliberate
/// runs of one task are two runs. A task that names one is derived from the task
/// digest, that key and the state directory's tenant, so a client retry of the
/// same document maps to the same run id and the runtime reports it as a
/// duplicate instead of running the task again (larry#182 defect 3).
///
/// # Errors
///
/// [`ExecError::Id`] when the OS entropy or the clock refuses a fresh identifier.
pub(super) fn run_id(task: &Task, state: &Path) -> Result<String, ExecError> {
    let Some(key) = task.idempotency_key() else {
        return fresh_id();
    };
    let digest = digest_hex(task.bytes());
    let tenant = tenant_of(state);
    Ok(lgwks_std::id::Uuid::from(identity16(&[
        KEY_TAG,
        tenant.as_bytes(),
        digest.as_bytes(),
        key.as_bytes(),
    ]))
    .to_string())
}

/// Seeded adversarial simulations of the identity rules (larry#182 defect 3).
#[cfg(test)]
#[path = "sim_identity.rs"]
mod sim;

/// Seeded simulations of the identity rules: every fresh run id is the estate's
/// canonical version 7 value, and a keyed id is one function of its three parts.
#[cfg(test)]
mod tests {
    use lgwks_std::id::Uuid;

    use super::{KEY_TAG, fresh_id, identity16, run_id};

    /// A task document over `repo` whose acceptance is `acceptance`, under `key`
    /// when it is given.
    fn document(
        repo: &str,
        key: Option<&str>,
        acceptance: &str,
    ) -> Result<super::Task, Box<dyn std::error::Error>> {
        let mut rendered = lgwks_std::json::json!({
            "repo": repo,
            "base": "HEAD",
            "acceptance": lgwks_std::json::from_str::<lgwks_std::json::Value>(acceptance)?,
        });
        if let Some(text) = key
            && let Some(object) = rendered.as_object_mut()
        {
            object.insert(
                String::from("idempotency_key"),
                lgwks_std::json::Value::String(text.into()),
            );
        }
        Ok(super::Task::parse(rendered.to_string().as_bytes())?)
    }

    /// Two starts that see one process id never mint one run id twice: the fresh
    /// identifiers of one process are strictly increasing and distinct, a keyed
    /// id is pinned by its parts alone, another tenant never mints it, and a
    /// different key or content is a different request (larry#182 defect 3).
    #[test]
    fn sim_two_starts_that_see_one_process_id_mint_different_run_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut previous: Option<Uuid> = None;
        let mut drawn = Vec::with_capacity(256);
        for _ in 0..256 {
            let id = fresh_id()?;
            let parsed = Uuid::parse(&id)?;
            assert_eq!(
                parsed.version(),
                Some(7),
                "a run id is the estate's canonical version 7 value: {id}"
            );
            assert_eq!(id.len(), 36, "the canonical text is 36 bytes: {id}");
            if let Some(ref last) = previous {
                assert!(
                    *last < parsed,
                    "one process's run ids strictly increase: {last} then {parsed}"
                );
            }
            previous = Some(parsed);
            drawn.push(id);
        }
        let distinct = drawn.iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            distinct.len(),
            drawn.len(),
            "no two of 256 fresh run ids repeat"
        );

        // One keyed document under one key in one state directory is one run id,
        // whatever the process that asks.
        let state = std::env::temp_dir();
        let task = document("/tmp/a", Some("retry-a"), "[[\"true\"]]")?;
        let first = run_id(&task, &state)?;
        assert_eq!(
            first,
            run_id(&task, &state)?,
            "one keyed request maps to one run id"
        );

        // Another state directory is another tenant, so it never mints that id.
        let other = std::path::Path::new("/tmp/other-state");
        assert_ne!(
            first,
            run_id(&task, other)?,
            "two state directories never share a keyed run id"
        );

        // A different key, and the same key over different content, are
        // different requests.
        let rekeyed = document("/tmp/a", Some("retry-b"), "[[\"true\"]]")?;
        assert_ne!(
            first,
            run_id(&rekeyed, &state)?,
            "a different key is a different request"
        );
        let recontented = document("/tmp/b", Some("retry-a"), "[[\"true\"]]")?;
        assert_ne!(
            first,
            run_id(&recontented, &state)?,
            "one key over different content is a different request"
        );

        // A document with no key is a fresh run every time.
        let unkeyed = document("/tmp/a", None, "[[\"true\"]]")?;
        assert_ne!(
            run_id(&unkeyed, &state)?,
            run_id(&unkeyed, &state)?,
            "two unkeyed submissions are two runs"
        );
        Ok(())
    }

    /// The keyed derivation reads its parts in one order and no other: swapping
    /// the tenant and the key changes the value.
    #[test]
    fn a_keyed_id_is_one_function_of_its_parts() -> Result<(), Box<dyn std::error::Error>> {
        let task = document("/tmp/a", Some("abc"), "[[\"true\"]]")?;
        let digest = super::digest_hex(task.bytes());
        let tenant = [3_u8; 16];
        let forward = identity16(&[KEY_TAG, &tenant, digest.as_bytes(), b"abc"]);
        let swapped = identity16(&[KEY_TAG, b"abc", digest.as_bytes(), &tenant]);
        assert_ne!(forward, swapped, "the parts are not interchangeable");
        Ok(())
    }
}
