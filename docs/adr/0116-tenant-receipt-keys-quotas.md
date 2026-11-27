---
type: ADR
title: "ADR 0116: Per-tenant receipt keys, a stored tenant id and report quotas"
description: Why executor receipts are sealed per tenant with key ids, the tenant id is stored, and sealed reports carry retention and a byte quota.
resource: /docs/adr/0116-tenant-receipt-keys-quotas.md
tags: [larry, adr, executor, tenancy, receipts]
generated: {by: codex, at: 2026-11-26}
verified: {by: codex, method: lgwks-box cargo nextest and clippy, 2026-11-26}
status: accepted
stale_after: 2027-05-26
sources: [larry#211, ADR 0116]
---

# ADR 0116

## Context

One state directory held one receipt key for every tenant, the tenant id was
recomputed from the directory's path on every open, the tree followed the
process umask, and every keyed report was kept forever. Moving the directory
changed every tenant id, `larry key` and `larry verify` on an empty directory
created the store and minted a secret, and two tenants on one directory could
not hold separate keys (larry#211).

## Decision

`LARRY_TENANT` selects the tenant: a validated name, else the store's default.
The default is one stored random id, drawn once inside the state dir; a
directory that already filed rows under its legacy path-derived tenant has
those rows renamed onto the drawn id at migration, so surviving config keeps
working. The worker derives from the stored tenant, so one directory is one
worker whatever the spelling. `receipt_keys(tenant, key_id)` holds one seed per
id: `larry key` reads the active public key and never mints, a run path mints
at most key id 1 on first use, and `larry key rotate` appends the next id,
keeping 16 for verification. The receipt names its `kid`; verification loads
the tenant's keys read-only and refuses an unknown id as untrusted. Reports
are rows of (tenant, run id) with a sealed-at stamp: each keep prunes past 512
rows or 30 days, then refuses past 256 MiB per tenant with `ReportQuota`. Live
groups carry the tenant. The state tree, `ws/` and journals are owner-only.

Rejected: keep deriving the tenant from the path and migrate rows to a random
id. The derivation was the move defect, and re-keying breaks in-flight keyed
retries by minting new run ids for old requests.

## Why

A key every tenant shares signs every tenant's receipts; a path-derived tenant
cannot survive a move; a read path that mints creates secrets nobody asked
for; unbounded evidence grows forever; and an unmasked tree trusts the umask.

## Consequences

Two tenants share one state directory, each verifying only its own receipts;
a rotated key verifies old receipts by id; a moved directory keeps every id;
an empty directory is refused without creating or minting. A tenant over quota
keeps nothing new until old rows age out. Key history past 16 ids no longer
verifies, fail closed.

## Proof

`sim_two_tenants_on_one_state_directory_verify_only_their_own` runs two
tenants on one directory and refuses each cross-verification as untrusted.
`a_rotated_key_verifies_old_receipts_by_key_id_and_seals_new_ones_under_the_new_id`
rotates and verifies both generations. `moving_the_state_directory_keeps_every_tenant_id`
retries a keyed run after a move and answers from the kept report.
`one_dir_through_relative_absolute_and_symlink_paths_is_one_tenant` runs one
keyed task through all three spellings and lands on one run id and one key.
`sim_quota_and_retention_hold_for_every_seed` refuses past quota typed and
prunes oldest first per tenant. `live_budget_reads_the_gate_timeline` pins the
stamp to the gate's clock with no default (larry#436).
