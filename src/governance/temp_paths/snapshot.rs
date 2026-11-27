//! The bounded snapshot cache of the temp-path rule (larry#415, ADR 0125): parsed trees
//! and the pure per-file analysis `scan` derives from them, retained while one workspace
//! snapshot lives and keyed by exact source bytes, so repeated checks pay tree-sitter's parse
//! and the tree walks once. Follow names are cached; their resolution reads the live tree.

use std::sync::{Arc, Mutex};

use lgwks_ast::{Language, Parsed, try_parse};

use super::super::error::GovernanceError;
use super::syntax::{follow_names, is_test_code, verdicts};
use super::{may_follow, may_refuse};

/// The parsed source cache is bounded so a large workspace cannot turn one governance run into an
/// unbounded heap owner. A source that would exceed the budget is parsed without being retained.
const MAX_CACHED_SOURCE_BYTES: usize = 16 * 1024 * 1024;

/// One source's exact bytes, its parsed syntax tree, and the pure analysis `scan` derives
/// from those bytes: the test-code verdict, the follow names the tree spells, and the verdict
/// errors the file's own functions raise. Everything stored here is a function of the path and
/// bytes alone; the follow pass still resolves every name against the live tree, so a changed
/// tree is never judged from an old entry.
struct CachedParse {
    /// The bytes that produced `tree` and `analysis`.
    contents: Vec<u8>,
    /// The tree parsed from `contents`.
    tree: Arc<Parsed>,
    /// The analysis of `contents`.
    analysis: FileAnalysis,
}

/// The pure per-file analysis one source's bytes and tree determine.
#[derive(Clone, Default)]
pub(super) struct FileAnalysis {
    /// Whether the file is test code, from its path and tree alone.
    pub(super) test_code: bool,
    /// Whether `verdicts` was computed because the text could refuse one.
    verdicts_done: bool,
    /// Whether `follows` was computed because the text could name one.
    follows_done: bool,
    /// The `#[path]` and `include!` names the tree spells, each with its inline modules.
    pub(super) follows: Vec<(String, Vec<String>)>,
    /// The temp-path refusals the file's own functions raise.
    pub(super) verdicts: Vec<String>,
}

impl FileAnalysis {
    /// Analyses `text` over `tree`: the test-code verdict when a pass needs it, the follow
    /// names when the follow pass can name one, and the verdicts when the verdict pass can
    /// refuse one. The flags are a function of the text, so the same bytes analyse the same.
    fn of(relative: &str, text: &str, tree: &Parsed, needed: (bool, bool)) -> Self {
        let test_code = (needed.0 || needed.1) && is_test_code(relative, tree);
        let follows = if needed.1 { names(tree) } else { Vec::new() };
        let verdicts = if needed.0 {
            verdicts(relative, text, tree)
        } else {
            Vec::new()
        };
        Self {
            test_code,
            verdicts_done: needed.0,
            follows_done: needed.1,
            follows,
            verdicts,
        }
    }

    /// Whether the stored analysis covers what the text needs.
    fn covers(&self, verdicts_needed: bool, follows_needed: bool) -> bool {
        (!verdicts_needed || self.verdicts_done) && (!follows_needed || self.follows_done)
    }

    /// The heap the analysis owns beyond the source bytes.
    fn retained(&self) -> usize {
        self.verdicts
            .iter()
            .map(String::len)
            .sum::<usize>()
            .saturating_add(
                self.follows
                    .iter()
                    .map(|entry| {
                        entry
                            .0
                            .len()
                            .saturating_add(entry.1.iter().map(String::len).sum::<usize>())
                    })
                    .sum(),
            )
    }
}

/// Parsed trees retained for one workspace snapshot.
#[derive(Default)]
struct ParseCache {
    /// Trees indexed by their root-relative source path.
    entries: std::collections::BTreeMap<String, CachedParse>,
    /// Total source bytes retained in `entries`.
    bytes: usize,
}

impl ParseCache {
    /// Returns the tree and analysis when the path and exact source bytes match.
    fn get(&self, relative: &str, contents: &[u8]) -> Option<(Arc<Parsed>, FileAnalysis)> {
        self.entries
            .get(relative)
            .filter(|entry| entry.contents.as_slice() == contents)
            .map(|entry| (Arc::clone(&entry.tree), entry.analysis.clone()))
    }

    /// Replaces a path's entry when the bounded byte budget allows it. The budget counts the
    /// source bytes and the retained analysis beside them.
    fn insert(
        &mut self,
        relative: &str,
        contents: &[u8],
        tree: Arc<Parsed>,
        analysis: FileAnalysis,
    ) {
        let old = match self.entries.remove(relative) {
            Some(entry) => entry
                .contents
                .len()
                .saturating_add(entry.analysis.retained()),
            None => 0,
        };
        self.bytes = self.bytes.saturating_sub(old);
        let retained = contents.len().saturating_add(analysis.retained());
        if retained <= MAX_CACHED_SOURCE_BYTES
            && self.bytes.saturating_add(retained) <= MAX_CACHED_SOURCE_BYTES
        {
            self.bytes = self.bytes.saturating_add(retained);
            self.entries.insert(
                String::from(relative),
                CachedParse {
                    contents: contents.to_vec(),
                    tree,
                    analysis,
                },
            );
        }
    }
}

/// One workspace snapshot's bounded cache of parsed sources and their analyses.
#[derive(Default)]
pub(super) struct Snapshot {
    /// Trees and analyses indexed by root-relative source path, held across checks.
    cache: Mutex<ParseCache>,
}

impl Snapshot {
    /// Parses `relative` and analyses what the verdict and follow passes need, reusing an
    /// exact byte match from the snapshot's bounded cache. The checked parse always runs,
    /// so a syntax fault refuses whatever the text holds; the verdict and follow analysis
    /// runs only when the text can raise one, and a byte-matching entry whose analysis does
    /// not cover the need is re-analysed over its stored tree, never reparsed.
    pub(super) fn analyzed(
        &self,
        relative: &str,
        contents: &[u8],
        text: &str,
        gated: bool,
    ) -> Result<(Arc<Parsed>, FileAnalysis), GovernanceError> {
        let needed = (!gated || may_refuse(text), !gated || may_follow(text));
        let stored = self
            .cache
            .lock()
            .ok()
            .and_then(|cache| cache.get(relative, contents));
        if let Some((tree, analysis)) = stored.clone()
            && analysis.covers(needed.0, needed.1)
        {
            return Ok((tree, analysis));
        }
        let tree = match stored {
            Some((tree, _)) => tree,
            None => Arc::new(try_parse(text, Language::Rust).map_err(|cause| {
                GovernanceError::Source {
                    relative: String::from(relative),
                    cause,
                }
            })?),
        };
        let analysis = FileAnalysis::of(relative, text, &tree, needed);
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(relative, contents, Arc::clone(&tree), analysis.clone());
        }
        Ok((tree, analysis))
    }
}

/// The `#[path = "..."]` and `include!("...")` names `tree` spells, each with the inline
/// modules around it, outermost first.
fn names(tree: &Parsed) -> Vec<(String, Vec<String>)> {
    follow_names(tree)
        .into_iter()
        .map(|named| (named.name, named.inline))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One parsed tree for the cache tests: the tree and the bytes need not agree, because
    /// `insert` stores what it is given and the bound counts bytes alone.
    fn parsed(text: &str) -> Result<Arc<Parsed>, Box<dyn std::error::Error>> {
        Ok(Arc::new(try_parse(text, Language::Rust)?))
    }

    /// `covers` holds exactly what was done: nothing needed is covered by nothing done, and
    /// each needed pass fails without its own flag, so neither `||` can become `&&`.
    #[test]
    fn covers_distinguishes_each_needed_pass() {
        let none = FileAnalysis::default();
        assert!(none.covers(false, false), "nothing needed is covered");
        assert!(
            !none.covers(true, false),
            "a needed verdict pass is not covered"
        );
        assert!(
            !none.covers(false, true),
            "a needed follow pass is not covered"
        );
        assert!(
            !none.covers(true, true),
            "two needed passes are not covered"
        );

        let verdicts = FileAnalysis {
            verdicts_done: true,
            ..Default::default()
        };
        assert!(verdicts.covers(false, false));
        assert!(
            verdicts.covers(true, false),
            "a done verdict pass covers it"
        );
        assert!(!verdicts.covers(false, true));
        assert!(!verdicts.covers(true, true));

        let follows = FileAnalysis {
            follows_done: true,
            ..Default::default()
        };
        assert!(follows.covers(false, false));
        assert!(!follows.covers(true, false));
        assert!(follows.covers(false, true), "a done follow pass covers it");
        assert!(!follows.covers(true, true));

        let both = FileAnalysis {
            verdicts_done: true,
            follows_done: true,
            ..Default::default()
        };
        assert!(both.covers(false, false));
        assert!(both.covers(true, true), "two done passes cover two needs");
    }

    /// Inserts `contents` at `relative` into a fresh cache and reports whether the entry
    /// stayed: the bound counts bytes alone, so the tree and the bytes need not agree.
    fn retained(relative: &str, contents: &[u8]) -> Result<bool, Box<dyn std::error::Error>> {
        let tree = parsed("pub fn present() {}\n")?;
        let mut cache = ParseCache::default();
        cache.insert(relative, contents, tree, FileAnalysis::default());
        Ok(cache.get(relative, contents).is_some())
    }

    /// An entry of exactly the 16 MiB bound is retained: `<=` never becomes `<`.
    #[test]
    fn an_entry_of_exactly_the_bound_is_retained() -> Result<(), Box<dyn std::error::Error>> {
        let contents = vec![b'x'; MAX_CACHED_SOURCE_BYTES];
        assert!(
            retained("tests/bound.rs", &contents)?,
            "exactly the bound is within it"
        );
        Ok(())
    }

    /// An entry one past the bound is parsed but never retained.
    #[test]
    fn an_entry_one_past_the_bound_is_parsed_but_not_retained()
    -> Result<(), Box<dyn std::error::Error>> {
        let contents = vec![b'x'; MAX_CACHED_SOURCE_BYTES.saturating_add(1)];
        assert!(
            !retained("tests/bound.rs", &contents)?,
            "one past the bound is refused"
        );
        Ok(())
    }

    /// Half the 16 MiB bound, so two entries share exactly it without any division.
    const HALF_BOUND: usize = 8 * 1024 * 1024;

    /// A cache holding `first` at `tests/first.rs`, with its tree for the next insert.
    fn holding_first(
        first: &[u8],
    ) -> Result<(ParseCache, Arc<Parsed>), Box<dyn std::error::Error>> {
        let tree = parsed("pub fn present() {}\n")?;
        let mut cache = ParseCache::default();
        cache.insert(
            "tests/first.rs",
            first,
            Arc::clone(&tree),
            FileAnalysis::default(),
        );
        Ok((cache, tree))
    }

    /// Two entries sharing exactly the bound are both retained: the shared `<=` never
    /// becomes `<`.
    #[test]
    fn two_entries_sharing_exactly_the_bound_are_both_retained()
    -> Result<(), Box<dyn std::error::Error>> {
        let first = vec![b'x'; HALF_BOUND];
        let second = vec![b'y'; MAX_CACHED_SOURCE_BYTES.saturating_sub(HALF_BOUND)];
        let (mut cache, tree) = holding_first(&first)?;
        cache.insert("tests/second.rs", &second, tree, FileAnalysis::default());
        assert!(
            cache.get("tests/first.rs", &first).is_some(),
            "the first half stays"
        );
        assert!(
            cache.get("tests/second.rs", &second).is_some(),
            "the second half fills exactly the bound"
        );
        Ok(())
    }

    /// An entry past the shared bound is parsed but never retained, and the kept entries stay.
    #[test]
    fn an_entry_past_the_shared_bound_is_parsed_but_not_retained()
    -> Result<(), Box<dyn std::error::Error>> {
        let first = vec![b'x'; HALF_BOUND];
        let past = vec![b'y'; HALF_BOUND.saturating_add(1)];
        let (mut cache, tree) = holding_first(&first)?;
        cache.insert("tests/past.rs", &past, tree, FileAnalysis::default());
        assert!(
            cache.get("tests/first.rs", &first).is_some(),
            "the kept entry stays"
        );
        assert!(
            cache.get("tests/past.rs", &past).is_none(),
            "one past the shared bound is refused"
        );
        Ok(())
    }
}
