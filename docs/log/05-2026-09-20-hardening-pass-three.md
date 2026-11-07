---
type: Log
title: log.md, part 5 of 15
description: Part 5 of /log.md, in order; the index lists every part.
resource: /docs/log/05-2026-09-20-hardening-pass-three.md
tags: [governance, decisions, evidence]
generated: {by: opencode, at: 2026-09-15}
verified: {by: opencode, method: explicit conversation decisions and source inspection}
status: active
stale_after: 2026-10-15
sources: [Director conversations 2026-09-15 through 2026-09-17, /specification.md, /research-ledger.md]
---
## 2026-09-20: hardening pass — three counterexamples, and the witness the journal lacks

Director direction: find real defects in the C02/E71 slices by adversarial
reading, fix each at its lowest invariant, and add falsifiers that would fail
if the property broke. No churn: only defects and the tests that catch them.

Three defects were found, all real, and each is now fixed with a falsifier.
The hardening commits land on `feat/dispatch-fence`, on top of the stack, and
never by rebasing a fix to a lower layer.

**1. Recovery was impossible for a legal journal.** `restore` derived each
run's tenant from the first reserved intent, so a run the journal names but
which reserved nothing — a submission the reducer denied, a steering command,
a cancellation that arrived before any work — made restore fail with
`RunNotTracked`, and with it the whole journal. That inverts the error's own
documented meaning ("the journal never named this run") and breaks the one
recovery story this slice claims: state is a fold over recorded facts (D04).
The fix reads the tenant from the run's own recorded events:
`ProjectionBackend::run_tenant`, backed by a `tenant` field on `RunView` that
is fixed by the run's first recorded event, so a later event cannot
re-attribute a run either. Falsifier:
`a_run_that_reserved_no_effect_does_not_block_restore`.

**2. A proposal could declare its own tenant.** `WorkProposal.tenant` was
documented as "re-validated against the envelope's tenant rather than trusted
from the body", but nothing validated it: `admit_work` recorded the *body's*
declared tenant into the durable `EffectIntent`. That is a caller-declared
security fact reaching a durable record, which D42 forbids, and it compounded
defect 1 — a restored runtime re-attributed the run to the declared tenant.
The fix is the one already used for the admission epoch: the tenant is an
argument to `WorkProposal::intent`, read from the runtime's own run table,
never a value the proposal supplies. Falsifier:
`e63_a_proposal_cannot_declare_its_own_tenant`, which submits an envelope for
tenant 1 carrying a body claiming tenant 2 and asserts the recorded intent,
the receipt and the restored run table all say tenant 1.

**3. An observation resolved an effect that provably never left the process.**
This corrects the reasoning the entry below records, and it is the reason this
entry exists. That entry concluded that a fenced first dispatch is
*indistinguishable from a crash in the dispatch window* because `EventKind`
has no "dispatched" fact, and therefore that the runtime must query such an
effect and take the answer. The premise is half true. The *journal* cannot
tell the two apart, but the running process can: a refused reach calls no
owner at all, so `LiveEffect::sent` stays false, while a reservation restored
from a crash is deliberately restored as possibly-sent and keeps its full
treatment. The unused witness cost real money: with the offline bound spent
and a queryable effect, an endpoint answering `Succeeded` to a status query
about an effect it was never asked to perform produced a terminal receipt
reading `Succeeded` and carried the run to `Completed` — a manufactured
success for an effect that provably never happened, which is exactly what R88
forbids and what the module doc promised could not occur ("a refused reach
calls no owner, records no outcome and publishes no receipt").

The repair is one witness, set in one place. `LiveEffect::sent` is set when a
mutating reach is taken and by `restore` for every recorded
reservation; `settle` records an observed outcome but withholds the
disposition when nothing was ever sent, and `is_reconcilable` keeps such an
effect open so the refusal stays reported rather than settling into a
disposition no reach earned. The status query itself stays ungated — the
already-dispatched treatment D48 names is untouched, and a restored effect
still resolves from its endpoint's answer. Falsifier:
`e71_an_observation_cannot_resolve_an_effect_that_was_never_sent`, which
asserts the endpoint is queried, is never dispatched, that no tick in the
effect's life publishes a receipt, and that the run stays `Reconciling`.

Two further items were examined and are **not** changed here, deliberately.
The `revoked` set on `RevocationFence` is unbounded, and that is a deliberate
fail-closed choice: evicting a withdrawn operation would re-authorize it, and
the fence documents that withdrawal is one-way for the fence's life. It is
recorded as a bound the host owns by advancing the epoch (D49's anti-rollback
territory) rather than being papered over. Separately, an effect whose outcome
was recorded and whose *receipt* write was refused (a full journal between the
two writes) is not re-attempted after restore, because `is_reconcilable`
treats a resolved observation as closed. It fails closed — no receipt, no
invented disposition — and is left visible here rather than repaired without a
decision, since re-attempting it means re-querying an owner for an event the
journal already holds.

**Hosted CI, recorded precisely because the page must not overstate what was
verified.** This repository has 61 recorded workflow runs and every one
concluded `failure`; no run has ever passed, and none has ever executed a step.
The jobs are created and complete in about two seconds with an empty `steps`
array, and the job log returns `BlobNotFound`, so no step ran and no log was
ever produced. Actions is enabled and the repository is private, which makes
this an account-level runner or spending-limit condition rather than a defect
in this code — the same signature the earlier entry recorded for PR #1's runs.
Nothing about it was disabled, edited or weakened to make a check green, and no
attempt was made to repair the account-level cause. Every gate this change
claims is a local lane, and the tracker says so.

Authority: Director's hardening order, 2026-09-20. Affected: C02, E71, E63
(partially), D42, D48, R88, R72. Evidence: 164 workspace tests pass (161
before; three added), clippy `-D warnings` clean, `cargo fmt --check` clean,
`lgwks-deps check .` OK with no new edge, governance inventory 96/53/80/13/9
PASS with 55 python tests OK, local-CI receipt naming the head. Unresolved:
E63 and E65 remain NOT RUN as acceptance cases, so C02's exit gate is still
unmet and the tracker still says so; no independent review; the two items
above are recorded, not repaired.
