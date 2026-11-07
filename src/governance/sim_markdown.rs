//! Seeded controls of `larry governance`'s frontmatter, fence, link, fragment, size and
//! docs-tree rules over the real documents, in memory only. Each ports one control of the
//! Python suite the checker replaced (`tests/test_governance.py`, larry#53), named in its doc
//! line, at a seeded document and place instead of one fixed one. Source: CommonMark 0.31.2
//! fenced code blocks (4.5), RFC 3986 references, GitHub heading anchors. Requirement: every
//! mutation is refused with its exact error, and every legal change passes.

use std::fmt::Write as _;

use lgwks_std::seeded::Seeded;

use super::documents::{Documents, MAX_DOCUMENT_BYTES};
use super::markdown::{self, FIELDS, MAX_LINES};
use super::patterns::{INVENTORIES, Patterns};
use super::sim_support::{
    SimError, SimResult, append, assert_refused, count_of, docs_tree, errors_of, loaded, names_of,
    pick, replace_all, replace_once, text_of, top_level, whole_of,
};

/// Seeds each control runs; each seed validates the whole tree once.
const SEEDS: u64 = 16;

/// Every `(document, target)` local `.md` link whose every occurrence is visible text, so the
/// first occurrence a control rewrites is one the checker reads.
fn local_links(
    documents: &Documents,
    patterns: &Patterns,
    only_docs_tree: bool,
) -> Vec<(String, String)> {
    let mut links = Vec::new();
    for (name, text) in documents.iter() {
        if only_docs_tree && !name.starts_with("docs/") {
            continue;
        }
        let visible = markdown::markdown_structure(text, patterns).visible_text();
        for target in patterns.link_targets(&visible) {
            let plain = target.ends_with(".md") && !target.contains(['#', '?', '%', ':', ' ']);
            let marker = format!("]({target})");
            if plain && text.matches(&marker).count() == visible.matches(&marker).count() {
                links.push((String::from(name), String::from(target)));
            }
        }
    }
    links
}

/// One seeded visible local link rewritten to `rewrite` of its target; returns the document,
/// the new target and the errors.
fn relinked(
    rng: &mut Seeded,
    base: &Documents,
    patterns: &Patterns,
    docs_only: bool,
    rewrite: &mut dyn FnMut(&mut Seeded, &str) -> Result<String, SimError>,
) -> Result<(String, String, Vec<String>), SimError> {
    let links = local_links(base, patterns, docs_only);
    let (name, target) = pick(rng, &links)?.clone();
    let changed = rewrite(rng, &target)?;
    let mut documents = base.clone();
    replace_once(
        &mut documents,
        &name,
        &format!("]({target})"),
        &format!("]({changed})"),
    )?;
    Ok((name, changed, errors_of(&documents)?))
}

/// The frontmatter key lines of `text`, those of the form `key: value`.
fn key_lines(text: &str) -> Vec<String> {
    let Some((body, _rest)) = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
    else {
        return Vec::new();
    };
    body.split('\n')
        .filter(|line| {
            line.split_once(':').is_some_and(|(key, _)| {
                !key.is_empty()
                    && key
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            })
        })
        .map(String::from)
        .collect()
}

/// Ports `test_missing_frontmatter`: a broken opening fence loses the envelope.
#[test]
fn sim_missing_frontmatter_is_named() -> SimResult {
    let base = loaded()?;
    let names = names_of(&base);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &names)?.clone();
        let broken = *pick(
            &mut rng,
            &["", "--- \n", "---\r\n", "\u{feff}---\n", "--\n"],
        )?;
        let mut documents = base.clone();
        replace_once(&mut documents, &name, "---\n", broken)?;
        assert_refused(
            &errors_of(&documents)?,
            &format!("{name}: missing frontmatter envelope"),
            seed,
        );
    }
    Ok(())
}

/// One seeded key line, among those `keep` admits, of one seeded document of `names`, with its
/// document.
fn seeded_key_line(
    rng: &mut Seeded,
    base: &Documents,
    names: &[String],
    keep: fn(&str) -> bool,
) -> Result<(String, String), SimError> {
    let name = pick(rng, names)?.clone();
    let lines: Vec<String> = key_lines(&text_of(base, &name)?)
        .into_iter()
        .filter(|line| keep(line))
        .collect();
    Ok((name, pick(rng, &lines)?.clone()))
}

/// Every key line.
fn any_key(_line: &str) -> bool {
    true
}

/// A key line whose key is one the checker requires; renaming any other key misses nothing.
fn required_key(line: &str) -> bool {
    line.split_once(':')
        .is_some_and(|(key, _)| FIELDS.contains(&key))
}

/// Ports `test_duplicate_frontmatter`: any key written twice is refused.
#[test]
fn sim_duplicate_frontmatter_key_is_refused() -> SimResult {
    let base = loaded()?;
    let names = names_of(&base);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (name, line) = seeded_key_line(&mut rng, &base, &names, any_key)?;
        let mut documents = base.clone();
        replace_once(
            &mut documents,
            &name,
            &format!("\n{line}\n"),
            &format!("\n{line}\n{line}\n"),
        )?;
        assert_refused(
            &errors_of(&documents)?,
            &format!("{name}: duplicate frontmatter key"),
            seed,
        );
    }
    Ok(())
}

/// Ports `test_docs_tree_frontmatter_is_enforced`: a renamed required key in any docs-tree
/// document is reported missing by name. An extra key such as `updated` is not required, so
/// renaming it misses nothing; seed 14 drew one once the tree gained ADR 0012.
#[test]
fn sim_docs_tree_frontmatter_is_enforced() -> SimResult {
    let base = loaded()?;
    let tree = docs_tree(&base);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (name, line) = seeded_key_line(&mut rng, &base, &tree, required_key)?;
        let Some((key, value)) = line.split_once(':') else {
            return Err(Box::new(SimError::Absent(String::from("a key line"))));
        };
        let mut documents = base.clone();
        replace_once(
            &mut documents,
            &name,
            &format!("\n{line}\n"),
            &format!("\n{key}z:{value}\n"),
        )?;
        let errors = errors_of(&documents)?;
        let named = errors.iter().any(|error| {
            error.starts_with(&format!("{name}: missing frontmatter keys"))
                && error.contains(&format!("'{key}'"))
        });
        assert!(named, "seed {seed}: {name} must miss {key}: {errors:?}");
    }
    Ok(())
}

/// A seeded fence opener: up to three spaces, three to six backticks or tildes, an info word.
fn opener(rng: &mut Seeded) -> Result<String, SimError> {
    let indent = " ".repeat(rng.index(4)?);
    let marker = if rng.below(2)? == 0 { "`" } else { "~" }.repeat(rng.index(4)?.saturating_add(3));
    let info = *pick(rng, &["", "mermaid", "text", "python", "sh title"])?;
    Ok(format!("{indent}{marker}{info}"))
}

/// Ports `test_unclosed_fence`: a mermaid fence left open at the end of the specification.
#[test]
fn sim_unclosed_fence_in_the_specification_is_refused() -> SimResult {
    let base = loaded()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let fence = "`".repeat(rng.index(4)?.saturating_add(3));
        let mut documents = base.clone();
        let last = append(
            &mut documents,
            "specification.md",
            &format!("\n{fence}mermaid\nflowchart TD\n"),
        )?;
        let errors = errors_of(&documents)?;
        assert!(
            errors.contains(&format!("{last}: unclosed code fence")),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_unclosed_indented_and_tilde_fences`: any opener left open in any document.
#[test]
fn sim_unclosed_indented_and_tilde_fences_are_refused() -> SimResult {
    let base = loaded()?;
    let names = names_of(&base);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &names)?.clone();
        let mut documents = base.clone();
        documents.set(
            &name,
            format!(
                "{}\n{}\nunterminated\n",
                text_of(&base, &name)?,
                opener(&mut rng)?
            ),
        );
        let errors = errors_of(&documents)?;
        assert!(
            errors.contains(&format!("{name}: unclosed code fence")),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_missing_visuals`: fewer than four mermaid fences in the specification.
#[test]
fn sim_missing_visuals_are_refused() -> SimResult {
    let base = loaded()?;
    let spec = whole_of(&base, "specification.md")?;
    let fences = spec.matches("```mermaid").count();
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let keep = rng.index(4)?;
        let mut documents = base.clone();
        for _ in 0..fences.saturating_sub(keep) {
            replace_once(&mut documents, "specification.md", "```mermaid", "```text")?;
        }
        let errors = errors_of(&documents)?;
        assert!(
            errors.contains(&String::from(
                "specification lacks required visual diagrams"
            )),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_valid_tilde_mermaid_fences`: every backtick fence rewritten as a tilde fence passes.
#[test]
fn sim_valid_tilde_mermaid_fences_pass() -> SimResult {
    let base = loaded()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let tildes = "~".repeat(rng.index(4)?.saturating_add(3));
        let mut documents = base.clone();
        replace_all(&mut documents, "specification.md", "```", &tildes)?;
        let errors = errors_of(&documents)?;
        assert!(
            errors.is_empty(),
            "seed {seed}: tilde fences must pass: {errors:?}"
        );
    }
    Ok(())
}

/// The 120-line law (larry#155): any governed document padded to exactly `MAX_LINES` lines
/// passes, and one line more is refused with its count. Source: the Director's size law
/// (2026-10-10), which this checker now enforces at the boundary it states.
#[test]
fn sim_the_line_law_holds_at_its_boundary_for_every_document() -> SimResult {
    let base = loaded()?;
    let names = names_of(&base);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &names)?.clone();
        let text = text_of(&base, &name)?;
        let lines = text.lines().count();
        let room = MAX_LINES
            .checked_sub(lines)
            .ok_or_else(|| SimError::Absent(format!("{name} within the law")))?;
        let mut documents = base.clone();
        documents.set(&name, format!("{text}{}", "\n".repeat(room)));
        let errors = errors_of(&documents)?;
        assert!(
            errors.is_empty(),
            "seed {seed}: {name} at {MAX_LINES} lines passes: {errors:?}"
        );
        documents.set(
            &name,
            format!("{text}{}", "\n".repeat(room.saturating_add(1))),
        );
        let errors = errors_of(&documents)?;
        let expected = format!(
            "{name}: {} lines, over the {MAX_LINES}-line law",
            MAX_LINES.saturating_add(1)
        );
        assert!(
            errors.contains(&expected),
            "seed {seed}: expected {expected:?} in {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_matching_long_fence_with_short_content_fence`: a shorter fence inside a longer
/// one is content, not a close.
#[test]
fn sim_long_fence_holds_a_shorter_content_fence() -> SimResult {
    let base = loaded()?;
    // The five lines this control appends must keep the document within the line law, or the
    // legal change it builds would not be legal (larry#155).
    let mut names = Vec::new();
    for name in top_level() {
        if text_of(&base, &name)?.lines().count().saturating_add(5) <= MAX_LINES {
            names.push(name);
        }
    }
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &names)?.clone();
        let character = if rng.below(2)? == 0 { "`" } else { "~" };
        let outer_len = rng.index(4)?.saturating_add(4);
        let inner_len = rng.index(outer_len.saturating_sub(3))?.saturating_add(3);
        let inner_char = if rng.below(2)? == 0 { character } else { "~" };
        let (outer, inner) = (character.repeat(outer_len), inner_char.repeat(inner_len));
        let mut documents = base.clone();
        documents.set(
            &name,
            format!(
                "{}\n{outer}text\n{inner}\ncontent\n{outer}\n",
                text_of(&base, &name)?
            ),
        );
        let errors = errors_of(&documents)?;
        assert!(
            errors.is_empty(),
            "seed {seed}: {outer} holding {inner} must pass: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_escaping_link` and `test_docs_tree_escaping_link_is_rejected`: any `..` part.
#[test]
fn sim_escaping_link_is_refused() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let docs_only = rng.below(2)? == 0;
        let (name, escaped, errors) =
            relinked(&mut rng, &base, &patterns, docs_only, &mut |rng, target| {
                let bare = target.trim_start_matches('/');
                Ok(format!(
                    "{}{bare}",
                    *pick(rng, &["../", "/../", "docs/../"])?
                ))
            })?;
        assert_refused(
            &errors,
            &format!("{name}: unresolved or escaping local link {escaped}"),
            seed,
        );
    }
    Ok(())
}

/// Ports `test_missing_link`: a link to a document that does not exist.
#[test]
fn sim_missing_link_is_refused() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (name, missing, errors) =
            relinked(&mut rng, &base, &patterns, false, &mut |rng, _target| {
                Ok(format!(
                    "{}missing-{}.md",
                    *pick(rng, &["", "/docs/", "docs/deep/"])?,
                    rng.below(1 << 20)?
                ))
            })?;
        assert_refused(
            &errors,
            &format!("{name}: unresolved or escaping local link {missing}"),
            seed,
        );
    }
    Ok(())
}

/// Ports `test_missing_heading_fragment` and `test_missing_same_document_fragment`.
#[test]
fn sim_missing_heading_fragment_is_refused() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (name, broken, errors) =
            relinked(&mut rng, &base, &patterns, false, &mut |rng, target| {
                let fragment = format!("definitely-missing-{}", rng.below(1 << 20)?);
                Ok(if rng.below(2)? == 0 {
                    format!("{target}#{fragment}")
                } else {
                    format!("#{fragment}")
                })
            })?;
        assert_refused(
            &errors,
            &format!("{name}: unresolved heading fragment {broken}"),
            seed,
        );
    }
    Ok(())
}

/// `text` with each ASCII character percent-encoded when the seed says so.
fn percent_encode_some(rng: &mut Seeded, text: &str) -> Result<String, SimError> {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii() && rng.below(3)? == 0 {
            if write!(out, "%{:02X}", u32::from(character)).is_err() {
                return Err(SimError::Absent(String::from("room to encode")));
            }
        } else {
            out.push(character);
        }
    }
    Ok(out)
}

/// Ports `test_valid_heading_fragment`: a link to any heading of its target passes, encoded or not.
#[test]
fn sim_valid_heading_fragment_passes() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    let mut fixed = base.clone();
    replace_once(
        &mut fixed,
        "README.md",
        "](index.md)",
        "](index.md#read-in-this-order)",
    )?;
    assert!(
        errors_of(&fixed)?.is_empty(),
        "the Python control's own fragment must pass"
    );
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (name, linked, errors) =
            relinked(&mut rng, &base, &patterns, false, &mut |rng, target| {
                let destination = text_of(&base, target.trim_start_matches('/'))?;
                let ids: Vec<String> = markdown::heading_ids(&destination, &patterns)
                    .into_iter()
                    .collect();
                let id = pick(rng, &ids)?.clone();
                let fragment = percent_encode_some(rng, &id)?;
                Ok(format!("{target}#{fragment}"))
            })?;
        assert!(
            errors.is_empty(),
            "seed {seed}: {name} linking {linked} must resolve: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_code_block_heading_is_not_destination`: a heading inside a fence is no target.
#[test]
fn sim_code_block_heading_is_not_destination() -> SimResult {
    let base = loaded()?;
    let names = top_level();
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &names)?.clone();
        let number = rng.below(1 << 20)?;
        let fence = *pick(&mut rng, &["```", "~~~~", "  ````"])?;
        let appended = format!(
            "{}\n{fence}text\n# Phantom heading {number}\n{fence}\n\n[x](#phantom-heading-{number})\n",
            text_of(&base, &name)?
        );
        let mut documents = base.clone();
        documents.set(&name, appended);
        assert_refused(
            &errors_of(&documents)?,
            &format!("{name}: unresolved heading fragment #phantom-heading-{number}"),
            seed,
        );
    }
    Ok(())
}

/// Ports `test_unsafe_link_scheme`: every scheme but http, https and mailto, and any netloc.
#[test]
fn sim_unsafe_link_scheme_is_refused() -> SimResult {
    let base = loaded()?;
    let patterns = Patterns::new()?;
    let unsafe_targets = [
        "javascript:bad",
        "JaVaScRiPt:alert(1",
        "data:text/html,x",
        "file:///etc/passwd",
        "vbscript:x",
        "ftp://host/x",
        "//evil.example/x",
        "c:/windows",
        "\u{1}javascript:x",
        "java\tscript:x",
    ];
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let (name, bad, errors) =
            relinked(&mut rng, &base, &patterns, false, &mut |rng, _target| {
                Ok(String::from(*pick(rng, &unsafe_targets)?))
            })?;
        assert_refused(
            &errors,
            &format!("{name}: unsupported link scheme {bad}"),
            seed,
        );
    }
    Ok(())
}

/// Ports `test_oversized_document`: any document past 2 MiB is refused by name.
#[test]
fn sim_oversized_document_is_refused() -> SimResult {
    let base = loaded()?;
    let names = top_level();
    for seed in 0..4 {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &names)?.clone();
        let text = text_of(&base, &name)?;
        let pad = MAX_DOCUMENT_BYTES
            .saturating_sub(text.len())
            .saturating_add(1)
            .saturating_add(rng.index(64)?);
        let mut documents = base.clone();
        documents.set(&name, format!("{text}{}", "x".repeat(pad)));
        let errors = errors_of(&documents)?;
        assert!(
            errors.contains(&format!("{name}: document exceeds byte limit")),
            "seed {seed}: {errors:?}"
        );
    }
    Ok(())
}

/// Ports `test_docs_tree_undefined_reference_is_rejected`, for every prefix: a stray ID in any
/// docs-tree document is named exactly.
#[test]
fn sim_docs_tree_undefined_reference_is_named_exactly() -> SimResult {
    let base = loaded()?;
    let tree = docs_tree(&base);
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let name = pick(&mut rng, &tree)?.clone();
        let inventory = pick(&mut rng, &INVENTORIES)?;
        let defined = count_of(&base, inventory.label)?;
        let number = defined
            .saturating_add(inventory.start)
            .saturating_add(rng.index(400)?);
        let id = if inventory.prefix == "A" {
            format!("A{number}")
        } else {
            format!("{}{number:02}", inventory.prefix)
        };
        let mut documents = base.clone();
        documents.set(
            &name,
            format!(
                "{}\nStray reference {id} must fail traceability.\n",
                text_of(&base, &name)?
            ),
        );
        let errors = errors_of(&documents)?;
        let expected = format!("{name}: undefined {} references ['{id}']", inventory.prefix);
        assert!(
            errors.contains(&expected),
            "seed {seed}: expected {expected:?} in {errors:?}"
        );
    }
    Ok(())
}
