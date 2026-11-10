//! Seeded simulations and falsifiers for the harness value (larry#61 part 1, ADR 0028).
//! One seed drives every generated tree, edit and byte flip; same seed, same trace. There is no
//! clock, disk or network here, and no proptest in this estate (crates#838), so a property is a
//! fixed seed list.

use larry_contracts::harness::{
    HarnessAuthor, HarnessComponent, HarnessEdit, HarnessError, HarnessOp, HarnessPolicy,
    HarnessTree, HarnessVersion, validate_edit,
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// Seeds every property runs over (at least 256 per the larry#61 acceptance).
const SEEDS: u64 = 320;
/// Name alphabet; the first 36 bytes plus `_` are valid first characters.
const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz0123456789_-";

/// A splitmix64 stream: wrapping arithmetic only, no clock, no OS entropy.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(7))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    /// A value in `0..bound` (bound under 2^32), by multiply-shift: no modulo, no division.
    fn below(&mut self, bound: usize) -> usize {
        let Ok(wide) = u64::try_from(bound) else {
            return 0;
        };
        let scaled = (self.next() >> 32).wrapping_mul(wide) >> 32;
        let Ok(value) = usize::try_from(scaled) else {
            return 0;
        };
        value
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        while out.len() < len {
            out.extend_from_slice(&self.next().to_le_bytes());
        }
        out.truncate(len);
        out
    }

    /// A valid name of 1 to 64 bytes, never starting with `-`.
    fn name(&mut self) -> String {
        let len = self.below(64).saturating_add(1);
        let mut out = String::new();
        for position in 0..len {
            let pick = self.below(ALPHABET.len());
            let Some(letter) = ALPHABET.chars().nth(pick) else {
                break;
            };
            out.push(if position == 0 && letter == '-' {
                'x'
            } else {
                letter
            });
        }
        out
    }
}

/// A seeded tree touching every kind: empty, one entry, a few, one at the byte bound, a full count.
fn generate(seed: u64, small: bool) -> Result<HarnessTree, HarnessError> {
    let mut rng = Rng::new(seed);
    let mut tree = HarnessTree::empty();
    for component in HarnessComponent::ALL {
        let mode = if small { rng.below(3) } else { rng.below(6) };
        let (count, size) = match mode {
            0 => (0, 0),
            1 => (1, rng.below(48)),
            2 => (
                rng.below(5).saturating_add(1).min(component.max_entries()),
                rng.below(300),
            ),
            3 => (1, component.max_entry_bytes()),
            4 => (component.max_entries(), 0),
            _ => (component.max_entries(), rng.below(200)),
        };
        for index in 0..count {
            let name = format!(
                "e{index}-{}",
                rng.name().chars().take(8).collect::<String>()
            );
            let len = if mode == 3 {
                size
            } else {
                rng.below(size.saturating_add(1))
            };
            tree.insert(component, &name, &rng.bytes(len))?;
        }
    }
    Ok(tree)
}

fn path(component: HarnessComponent, name: &str) -> String {
    format!("{}/{name}", component.dir())
}

fn put(path: &str, bytes: &[u8]) -> HarnessOp {
    HarnessOp::Put {
        path: path.to_owned(),
        bytes: bytes.to_vec(),
    }
}

fn remove(path: &str) -> HarnessOp {
    HarnessOp::Remove {
        path: path.to_owned(),
    }
}

fn apply(
    tree: &HarnessTree,
    ops: Vec<HarnessOp>,
    policy: &HarnessPolicy,
) -> Result<HarnessTree, HarnessError> {
    validate_edit(tree, &HarnessEdit::new(ops)?, policy)
}

/// Acceptance: encode, decode and rehash is identity across generated trees.
#[test]
fn sim_harness_roundtrip_is_identity_over_seeded_trees() -> Outcome {
    let mut at_bound = 0_usize;
    let mut seen = [false; 8];
    for seed in 0..SEEDS {
        let tree = generate(seed, false)?;
        let bytes = tree.encode();
        let decoded = HarnessTree::decode(&bytes)?;
        assert_eq!(
            decoded, tree,
            "seed {seed}: decode(encode(tree)) is the tree"
        );
        assert_eq!(
            decoded.encode(),
            bytes,
            "seed {seed}: encode(decode(bytes)) is the bytes"
        );
        assert_eq!(
            decoded.hash(),
            tree.hash(),
            "seed {seed}: rehash is identity"
        );
        assert_eq!(
            generate(seed, false)?.hash(),
            tree.hash(),
            "seed {seed}: same seed, same hash"
        );
        for component in HarnessComponent::ALL {
            let slot = seen.get_mut(usize::from(component.tag()));
            if let Some(flag) = slot {
                *flag = *flag || tree.count(component) > 0;
            }
            at_bound = at_bound.saturating_add(usize::from(
                tree.count(component) == component.max_entries(),
            ));
        }
    }
    assert!(
        seen.iter().all(|flag| *flag),
        "all eight kinds were generated"
    );
    assert!(at_bound > 0, "some trees hold a full component");
    assert!(
        HarnessTree::decode(&HarnessTree::empty().encode())?.is_empty(),
        "the empty tree roundtrips"
    );
    Ok(())
}

/// Every strict prefix of an encoding is truncated, never a different valid tree.
#[test]
fn sim_harness_every_truncation_is_refused() -> Outcome {
    for seed in 0..64 {
        let bytes = generate(seed, true)?.encode();
        for cut in 0..bytes.len() {
            let prefix = bytes.get(..cut).ok_or("cut within bounds")?;
            let refused = HarnessTree::decode(prefix);
            assert!(
                matches!(refused, Err(HarnessError::Truncated)),
                "seed {seed} cut {cut}: {refused:?}"
            );
        }
    }
    Ok(())
}

/// A flipped, inserted or dropped byte is either refused or still canonical.
#[test]
fn sim_harness_mutated_encodings_are_refused_or_canonical() -> Outcome {
    let (mut refused, mut accepted) = (0_usize, 0_usize);
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed ^ 0xA5A5);
        let mut bytes = generate(seed, true)?.encode();
        for _ in 0..4 {
            let at = rng.below(bytes.len().saturating_add(1));
            match rng.below(3) {
                0 => {
                    if let Some(byte) = bytes.get_mut(at) {
                        *byte ^= 1 << rng.below(8);
                    }
                }
                1 => {
                    let fresh = rng.bytes(1).first().copied().ok_or("byte")?;
                    bytes.insert(at, fresh);
                }
                _ => {
                    if at < bytes.len() {
                        bytes.drain(at..=at);
                    }
                }
            }
            match HarnessTree::decode(&bytes) {
                Ok(tree) => {
                    accepted = accepted.saturating_add(1);
                    assert_eq!(
                        tree.encode(),
                        bytes,
                        "seed {seed}: an accepted input is canonical"
                    );
                }
                Err(_) => refused = refused.saturating_add(1),
            }
        }
    }
    assert!(
        refused > 0 && accepted > 0,
        "both outcomes occur: {refused} refused, {accepted} accepted"
    );
    Ok(())
}

#[test]
fn sim_harness_insertion_order_never_changes_the_hash() -> Outcome {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let mut entries = Vec::new();
        for index in 0..rng.below(12).saturating_add(2) {
            let component = HarnessComponent::ALL
                .get(rng.below(7).saturating_add(1))
                .copied()
                .ok_or("component")?;
            let size = rng.below(120);
            let suffix: String = rng.name().chars().take(40).collect();
            entries.push((component, format!("e{index}-{suffix}"), rng.bytes(size)));
        }
        let build =
            |order: &[(HarnessComponent, String, Vec<u8>)]| -> Result<HarnessTree, HarnessError> {
                let mut tree = HarnessTree::empty();
                for entry in order {
                    tree.insert(entry.0, &entry.1, &entry.2)?;
                }
                Ok(tree)
            };
        let forward = build(&entries)?;
        let mut reversed = entries.clone();
        reversed.reverse();
        let mut rotated = entries.clone();
        rotated.rotate_left(rng.below(entries.len()));
        for other in [build(&reversed)?, build(&rotated)?] {
            assert_eq!(other, forward, "seed {seed}: same entries, same tree");
            assert_eq!(other.hash(), forward.hash(), "seed {seed}: same hash");
            assert_eq!(other.encode(), forward.encode(), "seed {seed}: same bytes");
        }
    }
    Ok(())
}

/// Random edits, valid and hostile: success is canonical, refusal leaves the input as it was.
#[test]
fn sim_harness_random_edits_are_atomic_and_canonical() -> Outcome {
    let garbage = [
        "",
        "skills",
        "skills/a/b",
        "skills/..",
        "bin/x",
        "skills/UP",
        "scripts/setup",
        "subagents/root",
    ];
    let (mut done, mut refused) = (0_usize, 0_usize);
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed ^ 0x5EED);
        let current = generate(seed, true)?;
        let before = current.clone();
        let mut policy = HarnessPolicy::new();
        if rng.below(4) == 0 {
            policy = policy.forbidding(HarnessComponent::Plugins);
        }
        let mut ops = Vec::new();
        for _ in 0..rng.below(6).saturating_add(1) {
            let component = HarnessComponent::ALL
                .get(rng.below(8))
                .copied()
                .ok_or("component")?;
            let target = if rng.below(3) == 0 {
                garbage
                    .get(rng.below(garbage.len()))
                    .copied()
                    .ok_or("garbage")?
                    .to_owned()
            } else {
                path(component, &format!("n{}", rng.below(3)))
            };
            ops.push(if rng.below(4) == 0 {
                remove(&target)
            } else {
                let len = rng.below(40);
                put(&target, &rng.bytes(len))
            });
        }
        let first = apply(&current, ops.clone(), &policy);
        assert_eq!(
            first,
            apply(&current, ops, &policy),
            "seed {seed}: the reducer is pure"
        );
        assert_eq!(
            current, before,
            "seed {seed}: the input tree is never touched"
        );
        match first {
            Ok(next) => {
                done = done.saturating_add(1);
                assert_eq!(
                    HarnessTree::decode(&next.encode())?,
                    next,
                    "seed {seed}: result is canonical"
                );
            }
            Err(_) => refused = refused.saturating_add(1),
        }
    }
    assert!(
        done > 0 && refused > 0,
        "both outcomes occur: {done} applied, {refused} refused"
    );
    Ok(())
}

/// Every kind accepts its limit and refuses one past it, by bytes and by count.
#[test]
fn sim_harness_bounds_accept_the_limit_and_refuse_one_past() -> Outcome {
    for seed in 0..32 {
        let mut rng = Rng::new(seed);
        for component in HarnessComponent::ALL {
            let name = rng.name();
            let target = path(component, &name);
            let policy = HarnessPolicy::new().allowing_reserved();
            let empty = HarnessTree::empty();
            let limit = component.max_entry_bytes();
            let at_limit = apply(&empty, vec![put(&target, &rng.bytes(limit))], &policy)?;
            assert_eq!(
                at_limit.get(component, &name).map(<[u8]>::len),
                Some(limit),
                "{component:?} accepts {limit}"
            );
            let over = apply(
                &empty,
                vec![put(&target, &rng.bytes(limit.saturating_add(1)))],
                &policy,
            );
            assert_eq!(
                over,
                Err(HarnessError::Oversize {
                    component,
                    limit,
                    actual: limit.saturating_add(1)
                }),
                "{component:?} refuses limit + 1"
            );
            let mut full = HarnessTree::empty();
            for index in 0..component.max_entries() {
                full.insert(component, &format!("f{index}"), b"x")?;
            }
            let before = full.clone();
            let extra = path(component, "extra");
            assert_eq!(
                apply(&full, vec![put(&extra, b"y")], &policy),
                Err(HarnessError::TooManyEntries {
                    component,
                    limit: component.max_entries()
                }),
                "{component:?} refuses one entry past its count"
            );
            let replaced = apply(&full, vec![put(&path(component, "f0"), b"z")], &policy)?;
            assert_eq!(
                replaced.count(component),
                component.max_entries(),
                "replacing at the limit is not growth"
            );
            let freed = apply(
                &full,
                vec![remove(&path(component, "f0")), put(&extra, b"y")],
                &policy,
            )?;
            assert_eq!(
                freed.count(component),
                component.max_entries(),
                "a removal frees a slot"
            );
            assert_eq!(full, before, "the input tree is unchanged");
        }
    }
    Ok(())
}

/// The falsifier: an edit off the surface is refused with its specific error.
#[test]
fn harness_path_rules_separate_off_surface_from_invalid_names() -> Outcome {
    let current = HarnessTree::empty();
    let policy = HarnessPolicy::new();
    let long = format!("skills/{}", "a".repeat(65));
    let off_surface = [
        "",
        "skills",
        "skills/",
        "skills/a/b",
        "skills/..",
        "skills/.",
        "/skills/a",
        "bin/a",
        "../skills/a",
        "skills\\a",
        "skills/a\\b",
        "Skills/a",
        "skills//a",
        "memory/x/../y",
    ];
    for target in off_surface {
        assert_eq!(
            apply(&current, vec![put(target, b"x")], &policy),
            Err(HarnessError::OffSurface),
            "put {target:?}"
        );
        assert_eq!(
            apply(&current, vec![remove(target)], &policy),
            Err(HarnessError::OffSurface),
            "remove {target:?}"
        );
    }
    let component = HarnessComponent::Skills;
    let invalid = [
        "skills/Upper",
        "skills/-lead",
        "skills/a b",
        "skills/.hidden",
        "skills/a.b",
        "skills/\u{fc}",
        "skills/a\0",
        long.as_str(),
    ];
    for target in invalid {
        assert_eq!(
            apply(&current, vec![put(target, b"x")], &policy),
            Err(HarnessError::InvalidName { component }),
            "put {target:?}"
        );
    }
    let longest = format!("skills/{}", "a".repeat(64));
    for target in ["skills/_x", "skills/a-b", "skills/0", longest.as_str()] {
        assert!(
            apply(&current, vec![put(target, b"x")], &policy).is_ok(),
            "{target:?} is on the surface"
        );
    }
    Ok(())
}

#[test]
fn harness_policy_refusals_are_typed_and_atomic() -> Outcome {
    let mut current = HarnessTree::empty();
    current.insert(HarnessComponent::Scripts, "lint", b"echo")?;
    let before = current.clone();
    let strict = HarnessPolicy::new();
    for (component, name) in [
        (HarnessComponent::Scripts, "setup"),
        (HarnessComponent::Scripts, "entrypoint"),
        (HarnessComponent::Subagents, "root"),
        (HarnessComponent::Subagents, "larry"),
    ] {
        assert_eq!(
            apply(&current, vec![put(&path(component, name), b"x")], &strict),
            Err(HarnessError::Reserved {
                component,
                name: name.to_owned()
            }),
            "{component:?}/{name} is reserved"
        );
        assert!(
            apply(
                &current,
                vec![put(&path(component, name), b"x")],
                &HarnessPolicy::new().allowing_reserved()
            )
            .is_ok(),
            "the policy can allow {component:?}/{name}"
        );
    }
    let no_scripts = HarnessPolicy::new().forbidding(HarnessComponent::Scripts);
    let forbidden = HarnessError::Forbidden {
        component: HarnessComponent::Scripts,
    };
    assert_eq!(
        apply(&current, vec![put("scripts/new", b"x")], &no_scripts),
        Err(forbidden.clone()),
        "forbidden kind"
    );
    assert_eq!(
        apply(&current, vec![put("scripts/setup", b"x")], &no_scripts),
        Err(forbidden),
        "forbidden is checked before reserved"
    );
    assert!(
        apply(&current, vec![put("skills/ok", b"x")], &no_scripts).is_ok(),
        "other kinds are unaffected"
    );
    let purged = apply(&current, vec![remove("scripts/lint")], &no_scripts)?;
    assert!(purged.is_empty(), "a forbidden kind can still be purged");
    // A later refused operation discards the earlier accepted ones.
    let mixed = apply(
        &current,
        vec![put("skills/ok", b"x"), put("skills/Bad", b"x")],
        &strict,
    );
    assert_eq!(
        mixed,
        Err(HarnessError::InvalidName {
            component: HarnessComponent::Skills
        }),
        "second op refused"
    );
    let many = vec![remove("memory/a"); 129];
    assert_eq!(
        HarnessEdit::new(many),
        Err(HarnessError::EditTooLarge {
            limit: 128,
            actual: 129
        }),
        "edit bound"
    );
    assert_eq!(current, before, "refusals leave the current tree untouched");
    Ok(())
}

fn entry(tag: u8, name: &[u8], data: &[u8]) -> Result<Vec<u8>, std::num::TryFromIntError> {
    let mut out = vec![tag, u8::try_from(name.len())?];
    out.extend_from_slice(name);
    out.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    out.extend_from_slice(data);
    Ok(out)
}

fn document(version: u8, count: u32, entries: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&count.to_be_bytes());
    for item in entries {
        out.extend_from_slice(item);
    }
    out
}

#[test]
fn harness_non_canonical_bytes_are_refused() -> Outcome {
    let guard = entry(1, b"a", b"x")?;
    let skill = entry(5, b"b", b"y")?;
    let good = document(1, 2, &[guard.clone(), skill.clone()]);
    let tree = HarnessTree::decode(&good)?;
    assert_eq!(tree.encode(), good, "the helper builds canonical bytes");
    let refuse = |bytes: Vec<u8>, want: HarnessError| {
        assert_eq!(
            HarnessTree::decode(&bytes),
            Err(want.clone()),
            "expected {want:?}"
        );
    };
    refuse(
        document(1, 2, &[skill.clone(), guard.clone()]),
        HarnessError::Unsorted,
    );
    refuse(
        document(1, 2, &[entry(1, b"b", b"x")?, entry(1, b"a", b"x")?]),
        HarnessError::Unsorted,
    );
    refuse(
        document(1, 2, &[guard.clone(), guard.clone()]),
        HarnessError::Duplicate,
    );
    refuse([good, vec![0]].concat(), HarnessError::TrailingBytes);
    refuse(
        document(1, 1, &[guard.clone(), skill.clone()]),
        HarnessError::TrailingBytes,
    );
    refuse(document(1, 3, &[guard, skill]), HarnessError::Truncated);
    refuse(Vec::new(), HarnessError::Truncated);
    refuse(
        document(1, 1, &[entry(8, b"a", b"x")?]),
        HarnessError::UnknownComponent { tag: 8 },
    );
    refuse(
        document(1, 1, &[entry(255, b"a", b"x")?]),
        HarnessError::UnknownComponent { tag: 255 },
    );
    for wrong in [0_u8, 2, 255] {
        refuse(
            document(wrong, 0, &[]),
            HarnessError::UnsupportedVersion { got: wrong },
        );
    }
    refuse(
        document(1, 290, &[]),
        HarnessError::TreeTooLarge {
            limit: 289,
            actual: 290,
        },
    );
    let bad_name = HarnessError::InvalidName {
        component: HarnessComponent::Guardrails,
    };
    for name in [&b"Up"[..], b"", b"-x", b"a b", b"\xff\xfe", &[b'a'; 65][..]] {
        refuse(document(1, 1, &[entry(1, name, b"x")?]), bad_name.clone());
    }
    let limit = HarnessComponent::Guardrails.max_entry_bytes();
    let mut oversize = vec![1, 1, b'a'];
    oversize.extend_from_slice(&u32::try_from(limit.saturating_add(1))?.to_be_bytes());
    refuse(
        [vec![1, 0, 0, 0, 1], oversize].concat(),
        HarnessError::Oversize {
            component: HarnessComponent::Guardrails,
            limit,
            actual: limit.saturating_add(1),
        },
    );
    refuse(
        document(1, 2, &[entry(0, b"a", b"x")?, entry(0, b"b", b"x")?]),
        HarnessError::TooManyEntries {
            component: HarnessComponent::SystemPrompt,
            limit: 1,
        },
    );
    assert_eq!(
        HarnessTree::empty().encode(),
        vec![1, 0, 0, 0, 0],
        "the empty tree is five bytes"
    );
    Ok(())
}

/// Known answer: a format change moves this hash and fails loudly.
#[test]
fn harness_hash_is_pinned_to_a_known_answer() -> Outcome {
    let mut tree = HarnessTree::empty();
    tree.insert(HarnessComponent::Scripts, "lint", b"echo ok")?;
    tree.insert(HarnessComponent::SystemPrompt, "main", b"You are Larry.")?;
    tree.insert(HarnessComponent::Skills, "refactor", b"step one")?;
    tree.insert(
        HarnessComponent::Guardrails,
        "no-secrets",
        b"Never print secrets.",
    )?;
    assert_eq!(
        tree.hash().to_hex(),
        "9536e5e53aafcd57a8ffadf96c99b94e1e057f35806ec50244fb1bba1fb61c36",
        "tree hash known answer"
    );
    assert_eq!(
        HarnessTree::empty().hash().to_hex(),
        "59832bb56cf525180ad61f497aacfb14596b5037d7b14521f73aba9dbafef148",
        "empty tree hash known answer"
    );
    let mut edited = tree.clone();
    edited.insert(HarnessComponent::Skills, "refactor", b"step two")?;
    assert_ne!(
        edited.hash(),
        tree.hash(),
        "one changed byte moves the hash"
    );
    Ok(())
}

#[test]
fn harness_version_records_hash_parent_author_and_reason() -> Outcome {
    let mut first = HarnessTree::empty();
    first.insert(HarnessComponent::SystemPrompt, "main", b"v1")?;
    let genesis = HarnessVersion::new(&first, None, HarnessAuthor::Human, "first harness")?;
    let second = apply(
        &first,
        vec![put("system-prompt/main", b"v2")],
        &HarnessPolicy::new(),
    )?;
    let next = HarnessVersion::new(
        &second,
        Some(genesis.hash()),
        HarnessAuthor::Proposer,
        "tighten",
    )?;
    assert_eq!(
        next.parent(),
        Some(genesis.hash()),
        "parent links the chain"
    );
    assert_eq!(next.author(), HarnessAuthor::Proposer, "author recorded");
    assert_eq!(next.reason(), "tighten", "reason recorded");
    assert!(
        next.describes(&second) && !next.describes(&first),
        "a version names exactly its tree"
    );
    assert_eq!(genesis.parent(), None, "the first version has no parent");
    let tags = [
        HarnessAuthor::Human,
        HarnessAuthor::Proposer,
        HarnessAuthor::Composer,
    ]
    .map(HarnessAuthor::tag);
    assert_eq!(tags, [0, 1, 2], "author tags are stable");
    assert_eq!(
        HarnessVersion::new(&first, None, HarnessAuthor::Human, ""),
        Err(HarnessError::ReasonEmpty),
        "empty reason"
    );
    let long = "r".repeat(513);
    assert_eq!(
        HarnessVersion::new(&first, None, HarnessAuthor::Composer, &long),
        Err(HarnessError::ReasonTooLong {
            limit: 512,
            actual: 513
        }),
        "reason bound"
    );
    assert!(
        HarnessVersion::new(
            &first,
            None,
            HarnessAuthor::Composer,
            long.get(..512).ok_or("slice")?
        )
        .is_ok(),
        "reason at the bound"
    );
    let text = lgwks_std::json::to_string(&next)?;
    let back: HarnessVersion = lgwks_std::json::from_str(&text)?;
    assert_eq!(back, next, "a version roundtrips through JSON");
    Ok(())
}
