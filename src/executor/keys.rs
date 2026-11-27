//! The operator's key surface: provisioning a tenant's ledger, reading the
//! active receipt key without minting one, trusting a key, verifying a sealed
//! report and rotating a key. This is the second job beside the run path in
//! `super`: those functions authorize and execute tasks, while these hand the
//! operator the keys the reports seal under, read-only unless the run path
//! itself seals (larry#211).

use lgwks_std::sign::PublicKey;

use super::receipt::{ReceiptError, Verdict, key_id_of, parse_public_key, verify};
use super::{ExecError, Ledger, gate, state_dir};

/// The state directory's store, opened (and created or migrated) under its lock: the
/// one home of the receipt key and the tenant key digests (larry#89).
pub(crate) fn state_ledger(read: &impl Fn(&str) -> Option<String>) -> Result<Ledger, ExecError> {
    Ledger::open(&state_dir(read)?)
}

/// The public half of the selected tenant's active receipt key, read-only: a
/// tenant with no key yet is refused, and no seed is ever drawn here.
/// `larry key` and `larry verify` on an empty state directory refuse without
/// creating the store or minting a secret (larry#211).
pub fn state_public_key(read: impl Fn(&str) -> Option<String>) -> Result<PublicKey, ExecError> {
    let directory = state_dir(&read)?;
    if !directory.join("executor.db").exists() {
        return Err(ExecError::NoStore);
    }
    let ledger = Ledger::open(&directory)?;
    let (_, tenant) = ledger.selected_tenant(&directory, &read)?;
    ledger
        .active_receipt_key(&tenant)?
        .map(|(_, public)| public)
        .ok_or(ExecError::NoReceiptKey)
}

/// Provisions the tenant-owned executor ledger before a local or remote door runs.
/// The policy text uses the same comma-separated scope names as `LARRY_POLICY`;
/// roots are canonicalized and bounded by the ledger owner. The tenant is the
/// selected one: `LARRY_TENANT` when it names one, else the store's default.
pub fn provision_tenant(
    read: impl Fn(&str) -> Option<String>,
    policy_text: &str,
    roots: &[&std::path::Path],
) -> Result<(), ExecError> {
    let state = state_dir(&read)?;
    let ledger = Ledger::open(&state)?;
    let (_, tenant) = ledger.selected_tenant(&state, &read)?;
    let policy = gate::policy_names(Some(policy_text))?;
    ledger.set_tenant_policy(&tenant, &policy)?;
    for root in roots {
        ledger.set_tenant_repo_root(&tenant, root)?;
    }
    Ok(())
}

/// The key a verifier trusts: `LARRY_RECEIPT_KEY` (64 hex characters) when it is
/// set, which needs no state directory, else the state directory's own key.
pub fn trusted_key(read: impl Fn(&str) -> Option<String>) -> Result<PublicKey, ExecError> {
    match read("LARRY_RECEIPT_KEY").filter(|value| !value.is_empty()) {
        Some(text) => parse_public_key(&text).map_err(ExecError::Receipt),
        None => state_public_key(read),
    }
}

/// Verifies a sealed report against the selected tenant's keys in the state
/// directory: the receipt's key id selects the key, so a rotated tenant
/// verifies old receipts by id and new ones under the new id, while another
/// tenant's receipt is refused as untrusted (larry#211).
///
/// # Errors
///
/// [`ExecError::NoStore`] when the store is absent, [`ExecError::NoReceiptKey`]
/// when the named key id is gone, and [`ExecError::Receipt`] when the report
/// is not a report this tenant sealed.
pub fn verify_state_report(
    read: impl Fn(&str) -> Option<String>,
    bytes: &[u8],
) -> Result<Verdict, ExecError> {
    let directory = state_dir(&read)?;
    if !directory.join("executor.db").exists() {
        return Err(ExecError::NoStore);
    }
    let ledger = Ledger::open(&directory)?;
    let (_, tenant) = ledger.selected_tenant(&directory, &read)?;
    let body = lgwks_std::json::from_slice::<lgwks_std::json::Value>(bytes)
        .map_err(|cause| ExecError::Receipt(ReceiptError::Malformed(cause)))?;
    let key_id = key_id_of(&body).map_err(ExecError::Receipt)?;
    let keys = ledger.receipt_public_keys(&tenant)?;
    let trusted = keys
        .iter()
        .find(|entry| entry.0 == key_id)
        .map(|entry| &entry.1)
        .ok_or(ExecError::NoReceiptKey)?;
    verify(bytes, trusted).map_err(ExecError::Receipt)
}

/// Rotates the selected tenant's receipt key: appends the next key id and
/// returns it with the new public key. Old ids stay verifiable up to the kept
/// history; sealing moves to the new id at once (larry#211).
pub fn rotate_receipt_key(
    read: impl Fn(&str) -> Option<String>,
) -> Result<(i64, PublicKey), ExecError> {
    let directory = state_dir(&read)?;
    let ledger = Ledger::open(&directory)?;
    let (_, tenant) = ledger.selected_tenant(&directory, &read)?;
    let (id, secret) = ledger.rotate_receipt_key(&tenant)?;
    Ok((id, secret.public_key()))
}

#[cfg(test)]
mod tests {
    use super::super::sim_tree::scratch;
    use super::Ledger;

    /// The first seal mints key id 1 and read-only lookups never mint: an empty
    /// state directory has no key to read, the run path's mint is id 1, and a
    /// second mint moves the active id to 2 while id 1 still reads (larry#211).
    #[test]
    fn keys_first_seal_mints_key_id_one_and_reads_never_mint()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = scratch("keys-first-seal")?;
        let ledger = Ledger::open(&state)?;
        assert!(
            ledger.active_receipt_key("tenant")?.is_none(),
            "a read-only lookup mints nothing"
        );
        let (first, _) = ledger.signing_key_for("tenant")?;
        assert_eq!(first, 1, "the run path mints key id 1 first");
        assert!(
            ledger.active_receipt_key("tenant")?.is_some(),
            "the minted key reads back"
        );
        let (second, _) = ledger.rotate_receipt_key("tenant")?;
        assert_eq!(second, 2, "rotation moves the active id to 2");
        let ids: Vec<i64> = ledger
            .receipt_public_keys("tenant")?
            .into_iter()
            .map(|entry| entry.0)
            .collect();
        assert!(ids.contains(&1), "id 1 still reads after rotation: {ids:?}");
        Ok(())
    }
}
