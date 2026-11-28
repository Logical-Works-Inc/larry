//! Seeded adversarial simulations of the tenant store (larry#211): ids, keys
//! and rotation. One seed drives every draw and rename; same seed, same trace.

use super::super::Ledger;
use crate::executor::sim_tree;
use crate::sim_error::SimError;
use lgwks_std::seeded::Seeded;

/// Two tenants hold disjoint keys, rotate without crossing, and history answers
/// only its own ids.
fn tenant_round(seed: u64) -> Result<(), SimError> {
    let mut rng = Seeded::from_seed(seed);
    let dir = sim_tree::scratch("larry-tenant-keys")?;
    let ledger = Ledger::open(&dir)?;
    let tenants = ["alpha".to_owned(), format!("t-{}", rng.below(1_000_000)?)];
    let [ref alpha, ref other] = tenants;
    let mut actives = Vec::with_capacity(tenants.len());
    for tenant in &tenants {
        let (id, secret) = ledger.signing_key_for(tenant)?;
        if id != 1 {
            return Err(SimError::Missing("the first seal mints key id 1"));
        }
        let again = ledger.signing_key_for(tenant)?;
        if again.0 != 1 || again.1.public_key() != secret.public_key() {
            return Err(SimError::Missing(
                "a second use keeps key id 1 and its secret",
            ));
        }
        let read = ledger
            .active_receipt_key(tenant)?
            .ok_or(SimError::Missing("a minted key reads back"))?;
        if read.0 != 1 || read.1 != secret.public_key() {
            return Err(SimError::Missing(
                "the read-only lookup names the active key",
            ));
        }
        actives.push(secret.public_key().to_bytes());
    }
    let first = actives
        .first()
        .ok_or(SimError::Missing("two tenants draw two keys"))?;
    let second = actives
        .get(1)
        .ok_or(SimError::Missing("two tenants draw two keys"))?;
    if first == second {
        return Err(SimError::Missing("two tenants never share a key"));
    }
    let (second_id, _) = ledger.rotate_receipt_key(alpha)?;
    if second_id != 2 {
        return Err(SimError::Missing("a rotation appends the next id"));
    }
    let history = ledger.receipt_public_keys(alpha)?;
    let ids: Vec<i64> = history.iter().map(|entry| entry.0).collect();
    if ids.as_slice() != [2, 1] {
        return Err(SimError::Missing("history keeps both ids, newest first"));
    }
    // The other tenant's history is untouched by the rotation.
    if ledger.receipt_public_keys(other)?.len() != 1 {
        return Err(SimError::Missing("a rotation never reaches another tenant"));
    }
    // A retry answers from the exact row its receipt names: id 1 stays listed
    // with the secret that sealed it, and an unminted id answers none.
    let old_key = history
        .iter()
        .find(|entry| entry.0 == 1)
        .map(|entry| entry.1.to_bytes())
        .ok_or(SimError::Missing("key id 1 stays listed"))?;
    assert_eq!(old_key.len(), 32, "key id 1 stays sealed as 32 bytes");
    if ledger
        .receipt_public_keys(alpha)?
        .iter()
        .any(|entry| entry.0 == 9)
    {
        return Err(SimError::Missing("an unminted id answers none"));
    }
    Ok(())
}

/// Two tenants on one store keep disjoint keys across rotations.
#[test]
fn sim_two_tenants_hold_disjoint_keys_for_every_seed() -> Result<(), SimError> {
    for seed in 0..64_u64 {
        tenant_round(seed.wrapping_add(0x2110))?;
    }
    Ok(())
}

/// A fresh directory draws its tenant: the stored id is never the legacy
/// path-derived one, so no caller can predict it from the path (larry#211).
#[test]
fn sim_a_fresh_directory_draws_its_tenant() -> Result<(), SimError> {
    let dir = sim_tree::scratch("larry-tenant-draw")?;
    let ledger = Ledger::open(&dir)?;
    let stored = ledger.dir_tenant(&dir)?;
    let legacy = crate::executor::identity::legacy_tenant_of(&dir);
    if stored.as_bytes() == legacy.as_bytes() {
        return Err(SimError::Missing("a fresh directory draws, never derives"));
    }
    Ok(())
}

/// One directory through two spellings is one tenant and one worker: the
/// stored record answers every spelling, so `./state`, `/abs/state` and a
/// symlink to it never triple the budgets over one directory (larry#211).
#[test]
fn sim_one_directory_through_two_spellings_holds_one_tenant() -> Result<(), SimError> {
    let dir = sim_tree::scratch("larry-tenant-spell")?;
    let dotted = dir.join("./");
    let plain = Ledger::open(&dir)?.dir_tenant(&dir)?;
    let again = Ledger::open(&dotted)?.dir_tenant(&dotted)?;
    if plain.as_bytes() != again.as_bytes() {
        return Err(SimError::Missing(
            "one directory is one tenant whatever the spelling",
        ));
    }
    if crate::executor::identity::worker_of(&plain) != crate::executor::identity::worker_of(&again)
    {
        return Err(SimError::Missing(
            "one tenant is one worker whatever the spelling",
        ));
    }
    Ok(())
}

/// A store an old binary left rows in is renamed onto a drawn tenant: the
/// record is made once, never derived from the path, and the surviving config
/// keeps working under the new id (larry#211).
#[test]
fn sim_a_legacy_store_is_rekeyed_onto_a_drawn_tenant() -> Result<(), SimError> {
    use std::collections::BTreeSet;

    use larry_contracts::authority::Scope;

    let dir = sim_tree::scratch("larry-tenant-migrate")?;
    let legacy = crate::executor::identity::legacy_tenant_of(&dir);
    let legacy_hex = lgwks_std::hex::encode(legacy.as_bytes());
    let policy = [Scope::FileRead].into_iter().collect::<BTreeSet<_>>();
    {
        let ledger = Ledger::open(&dir)?;
        ledger.set_tenant_policy(&legacy_hex, &policy)?;
        ledger
            .conn
            .execute("DELETE FROM dir_tenant", [])
            .map_err(crate::executor::ExecError::Sql)?;
    }
    let reopened = Ledger::open(&dir)?;
    let stored = reopened.dir_tenant(&dir)?;
    if stored.as_bytes() == legacy.as_bytes() {
        return Err(SimError::Missing("the record is drawn, never derived"));
    }
    if reopened.tenant_policy(&lgwks_std::hex::encode(stored.as_bytes()))? != policy {
        return Err(SimError::Missing(
            "legacy rows are renamed onto the drawn tenant",
        ));
    }
    Ok(())
}

/// The stored default tenant survives a move of the state directory: the id
/// before the rename is the id after it.
#[test]
fn sim_a_stored_tenant_survives_a_move() -> Result<(), SimError> {
    let dir = sim_tree::scratch("larry-tenant-move")?;
    let before = Ledger::open(&dir)?.dir_tenant(&dir)?;
    let moved = dir.with_extension("moved-state");
    std::fs::rename(&dir, &moved)?;
    let after = Ledger::open(&moved)?.dir_tenant(&moved)?;
    let kept = before.as_bytes() == after.as_bytes()
        && lgwks_std::hex::encode(before.as_bytes()).len() == 32;
    std::fs::remove_dir_all(&moved)?;
    if !kept {
        return Err(SimError::Missing("a move kept every tenant id"));
    }
    Ok(())
}

/// A receipt key id of one is the first seal: zero is refused typed and a
/// missing member reads as one for legacy seals, so `>= one` never becomes
/// `> one` (larry#211).
#[test]
fn sim_a_receipt_key_id_boundary_keeps_one() -> Result<(), Box<dyn std::error::Error>> {
    use crate::executor::receipt::{ReceiptError, key_id_of};
    use lgwks_std::json::json;
    let one = json!({ "receipt": { "kid": 1 } });
    assert_eq!(key_id_of(&one)?, 1, "key id one is kept");
    let legacy = json!({ "receipt": {} });
    assert_eq!(key_id_of(&legacy)?, 1, "legacy seals read as one");
    for refused in [0, -1] {
        let body = json!({ "receipt": { "kid": refused } });
        assert!(
            matches!(key_id_of(&body), Err(ReceiptError::BadKeyId)),
            "zero is refused typed"
        );
    }
    Ok(())
}

/// A sealed receipt carries its seal, so a retry answers from it (larry#182).
#[test]
fn sim_a_receipt_seal_is_carried() {
    use crate::executor::Report;
    use lgwks_std::seeded::Seeded;
    use lgwks_std::sign::SecretKey;
    let seed = Seeded::from_seed(3).next_u64().to_le_bytes();
    let mut full = [0_u8; 32];
    full[..8].copy_from_slice(&seed);
    let signer = SecretKey::from_seed(full);
    let body = lgwks_std::json::json!({ "task_id": "t" });
    assert!(
        Report::sealed(body, &signer, 1).carries_a_seal(),
        "a sealed receipt carries its seal"
    );
}

/// A rotated tenant verifies its old receipt by key id: the lookup finds the
/// exact id its receipt names, never the newest one (larry#211).
#[test]
fn sim_a_receipt_lookup_finds_the_named_key() -> Result<(), Box<dyn std::error::Error>> {
    use crate::executor::{Report, verify_state_report};
    let dir = sim_tree::scratch("larry-receipt-lookup")?;
    let ledger = Ledger::open(&dir)?;
    let tenant = ledger.dir_tenant(&dir)?;
    let tenant_hex = lgwks_std::hex::encode(tenant.as_bytes());
    let (_, first) = ledger.signing_key_for(&tenant_hex)?;
    let (_, second) = ledger.rotate_receipt_key(&tenant_hex)?;
    assert_ne!(
        first.public_key().to_bytes(),
        second.public_key().to_bytes(),
        "rotation mints a new key"
    );
    let body = lgwks_std::json::json!({
        "task_id": "lookup-task",
        "task_digest": "lookup-digest",
        "diff": "x",
        "results": [{"argv": ["true"], "exit_code": 0, "timed_out": false}],
        "denied": [],
    });
    let sealed = Report::sealed(body, &first, 1);
    let state = dir.to_string_lossy().into_owned();
    let verdict = verify_state_report(
        |name| match name {
            "LARRY_STATE_DIR" => Some(state.clone()),
            _ => None,
        },
        sealed.bytes(),
    )?;
    assert_eq!(
        verdict.task_id(),
        "lookup-task",
        "the named key id verifies"
    );
    Ok(())
}

/// A truncated seed never lands: the store's CHECK refuses a seed that is not
/// 32 bytes, so no read ever sees it (CWE-732, larry#211).
#[test]
fn sim_a_truncated_key_is_refused_typed() -> Result<(), Box<dyn std::error::Error>> {
    let dir = sim_tree::scratch("larry-tenant-truncated")?;
    let ledger = Ledger::open(&dir)?;
    let refused = ledger.conn.execute(
        "INSERT INTO receipt_keys (tenant, key_id, seed) VALUES (?1, 1, ?2)",
        lgwks_deps::storefront::rusqlite::params!["alpha", vec![7_u8; 31].as_slice()],
    );
    assert!(refused.is_err(), "a 31-byte seed is refused, never stored");
    assert!(
        ledger.active_receipt_key("alpha")?.is_none(),
        "a refused insert mints nothing"
    );
    Ok(())
}

/// One thousand concurrent provisions on one state directory each hold one
/// distinct key: no two tenants share a secret, and no provision overwrites
/// another (larry#211).
#[test]
fn sim_a_thousand_concurrent_tenant_provisions_hold_distinct_keys()
-> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Barrier};
    let dir = sim_tree::scratch("larry-tenant-thousand")?;
    let barrier = Arc::new(Barrier::new(32));
    let all = std::thread::scope(|scope| {
        let racers: Vec<_> = (0..32_u32)
            .map(|batch| {
                let barrier = Arc::clone(&barrier);
                let dir_path = dir.to_path_buf();
                scope.spawn(move || {
                    barrier.wait();
                    let ledger = Ledger::open(&dir_path).map_err(|error| error.to_string())?;
                    let mut ids = Vec::new();
                    for index in 0..32_u32 {
                        let tenant = format!("tenant-{batch}-{index}");
                        let (id, secret) = ledger
                            .signing_key_for(&tenant)
                            .map_err(|error| error.to_string())?;
                        if id != 1 {
                            return Err("each tenant mints its own key id 1".to_owned());
                        }
                        ids.push(secret.public_key().to_bytes());
                    }
                    Ok::<_, String>(ids)
                })
            })
            .collect();
        let mut all = Vec::with_capacity(1024);
        for racer in racers {
            match racer.join() {
                Ok(Ok(ids)) => all.extend(ids),
                Ok(Err(cause)) => return Err(cause.into()),
                Err(_panicked) => return Err("a provision thread panicked".into()),
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(all)
    })?;
    // One thousand tenants is 1024 draws here (32 batches of 32); all distinct.
    assert_eq!(all.len(), 1024, "one thousand provisions all land");
    let distinct = all.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        distinct.len(),
        all.len(),
        "no two tenants share a key (CWE-639)"
    );
    Ok(())
}
