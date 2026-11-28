//! What leaves for a model (larry#589, ADR 0151): the one owner of redaction.
//!
//! A secret is masked byte for byte, so the cut an observation gets afterwards
//! never leaves the head of one behind, and a directory of this host is renamed.
//! Source: OWASP ASVS 4.0.3 §8.3.4, CWE-200; RFC 7468 (the PEM labels) and
//! RFC 7519 (a JWT begins `eyJ`).

use std::borrow::Cow;
use std::ops::Range;
use std::path::{Component, Path};

use super::{ExecError, Ledger, Settings};
use crate::model::Route;

impl Settings {
    /// What a run under these settings keeps from a model: the operator's key,
    /// the key variable of each of this tenant's stored routes (another
    /// tenant's variable is never read), the state directory and HOME.
    pub(crate) fn redactor(&self, ledger: &Ledger) -> Result<Redactor, ExecError> {
        let tenant = self.tenant_text.as_str();
        let table = ledger.routes(tenant)?;
        let mut names = vec!["LARRY_MODEL_KEY"];
        names.extend(
            table
                .routes()
                .iter()
                .filter(|route| route.key_env_refusal(tenant).is_none())
                .map(Route::key_env),
        );
        let keys = names.into_iter().filter_map(|name| (self.read)(name));
        let mut redactor = Redactor::new(keys);
        redactor.hide(&self.state_dir, "<state>");
        if let Some(home) = (self.read)("HOME") {
            redactor.hide(Path::new(&home), "~");
        }
        Ok(redactor)
    }
}

/// Shortest configured key that is masked: a shorter one is a local gateway's
/// placeholder, and masking it would blank ordinary words.
const MIN_KEY_BYTES: usize = 8;

/// Token shapes: the prefix a provider documents and the fewest bytes after it.
const SHAPES: [(&str, usize); 11] = [
    ("sk-", 20),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("github_pat_", 20),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("AIza", 33),
    ("eyJ", 20),
];

/// The two prefixes of an AWS access key id, which is 16 capitals or digits more.
const AWS_PREFIXES: [&str; 2] = ["AKIA", "ASIA"];

/// What one run keeps from a model: its keys, and this host's directories with
/// the name each is given instead.
#[derive(Default)]
pub(crate) struct Redactor {
    /// The configured keys, each at least [`MIN_KEY_BYTES`] long.
    keys: Vec<String>,
    /// Directories and their names, longest first so a directory inside another
    /// takes its own name.
    paths: Vec<(String, &'static str)>,
}

impl Redactor {
    /// A redactor of `keys` that knows no directory yet.
    pub(crate) fn new(keys: impl IntoIterator<Item = String>) -> Self {
        Self {
            keys: keys
                .into_iter()
                .filter(|key| key.len() >= MIN_KEY_BYTES)
                .collect(),
            paths: Vec::new(),
        }
    }

    /// Calls `path` by `name` from now on, as given and as the file system
    /// resolves it. A path with no directory of its own (`/`) is left alone.
    pub(crate) fn hide(&mut self, path: &Path, name: &'static str) {
        if !path
            .components()
            .any(|part| matches!(part, Component::Normal(_)))
        {
            return;
        }
        let mut forms = vec![path.to_path_buf()];
        // A path that does not resolve has only the form it was given.
        if let Ok(resolved) = std::fs::canonicalize(path) {
            forms.push(resolved);
        }
        for form in forms {
            let text = form.to_string_lossy();
            let text = text.trim_end_matches('/');
            if !self.paths.iter().any(|known| known.0 == text) {
                self.paths.push((text.to_owned(), name));
            }
        }
        self.paths
            .sort_by_key(|known| std::cmp::Reverse(known.0.len()));
    }

    /// `text` with every key and token shape masked, byte for byte.
    pub(crate) fn masked<'text>(&self, text: &'text str) -> Cow<'text, str> {
        let mut found = Vec::new();
        for key in &self.keys {
            found.extend(
                text.match_indices(key.as_str())
                    .map(|(at, hit)| at..at.saturating_add(hit.len())),
            );
        }
        shaped(text, &mut found);
        pem_keys(text, &mut found);
        mask(text, found)
    }

    /// Whether `text` holds a key or a token shape.
    pub(crate) fn holds_secret(&self, text: &str) -> bool {
        matches!(self.masked(text), Cow::Owned(_))
    }

    /// What a model is shown of `text`: secrets masked, then directories
    /// renamed. A cut belongs after this, never before.
    pub(crate) fn observed(&self, text: &str) -> String {
        let mut shown = self.masked(text).into_owned();
        for &(ref path, name) in &self.paths {
            if shown.contains(path.as_str()) {
                shown = rename(&shown, path, name);
            }
        }
        shown
    }
}

/// Whether `character` can sit inside a token, so a token begins and ends beside
/// one that cannot.
fn in_token(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
}

/// Whether `run`, a run of token bytes with no closing punctuation, has a
/// documented token's shape.
fn is_token(run: &str) -> bool {
    SHAPES.iter().any(|shape| {
        run.strip_prefix(shape.0)
            .is_some_and(|rest| rest.len() >= shape.1)
    }) || AWS_PREFIXES.iter().any(|prefix| {
        run.strip_prefix(prefix).is_some_and(|rest| {
            rest.len() == 16
                && rest
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
    })
}

/// Adds to `found` every run of `text` shaped like a token.
fn shaped(text: &str, found: &mut Vec<Range<usize>>) {
    let mut from = None;
    let ends = text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')));
    for (at, character) in ends {
        if in_token(character) {
            from.get_or_insert(at);
        } else if let Some(start) = from.take()
            && let Some(run) = text.get(start..at)
        {
            let run = run.trim_end_matches(['.', '-']);
            if is_token(run) {
                found.push(start..start.saturating_add(run.len()));
            }
        }
    }
}

/// Adds to `found` every PEM private key block, to the end of `text` when its
/// closing label is missing.
fn pem_keys(text: &str, found: &mut Vec<Range<usize>>) {
    const LABEL: &str = "PRIVATE KEY-----";
    for (start, _) in text.match_indices("-----BEGIN ") {
        let Some(block) = text.get(start..) else {
            continue;
        };
        let first = match block.split_once('\n') {
            Some((line, _)) => line,
            None => block,
        };
        if !first.contains(LABEL) {
            continue;
        }
        let closed = block.find("-----END ").and_then(|close| {
            let label = block.get(close..)?.find(LABEL)?;
            Some(close.saturating_add(label).saturating_add(LABEL.len()))
        });
        let end = match closed {
            Some(length) => start.saturating_add(length),
            None => text.len(),
        };
        found.push(start..end);
    }
}

/// `text` with every byte inside `found` written as `*`, line ends kept.
fn mask(text: &str, mut found: Vec<Range<usize>>) -> Cow<'_, str> {
    if found.is_empty() {
        return Cow::Borrowed(text);
    }
    found.sort_by_key(|range| range.start);
    let mut ahead = found.into_iter().peekable();
    let mut until = 0;
    let mut shown = String::with_capacity(text.len());
    for (at, character) in text.char_indices() {
        while let Some(range) = ahead.next_if(|range| range.start <= at) {
            until = range.end.max(until);
        }
        if at < until && character != '\n' {
            shown.extend(std::iter::repeat_n('*', character.len_utf8()));
        } else {
            shown.push(character);
        }
    }
    Cow::Owned(shown)
}

/// `text` with the directory `path` called `name` wherever it is not merely the
/// start of a longer directory name (`/a/u` inside `/a/user` stays).
fn rename(text: &str, path: &str, name: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    let mut done = 0;
    for (at, hit) in text.match_indices(path) {
        let end = at.saturating_add(hit.len());
        let longer = text.get(end..).and_then(|rest| rest.chars().next());
        if longer.is_some_and(|next| next.is_alphanumeric() || matches!(next, '_' | '-')) {
            continue;
        }
        if let Some(before) = text.get(done..at) {
            shown.push_str(before);
        }
        shown.push_str(name);
        done = end;
    }
    if let Some(rest) = text.get(done..) {
        shown.push_str(rest);
    }
    shown
}

#[cfg(test)]
#[path = "sim_redact.rs"]
mod sim;

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::time::Instant;

    use super::Redactor;

    /// The cost of one observation at the capture bound (SD-5), printed.
    #[test]
    fn a_mebibyte_observation_prints_its_cost() -> std::io::Result<()> {
        let key = "zq7-route-key-0123456789abcdef";
        let mut redactor = Redactor::new([String::from(key)]);
        redactor.hide(std::path::Path::new("/srv/larry/state"), "<state>");
        let line = format!("plain words and /srv/larry/state/ws and {key} here\n");
        let text = line.repeat(1_048_576_usize.div_ceil(line.len()));
        let mut micros = Vec::new();
        for _ in 0..32 {
            let started = Instant::now();
            let shown = redactor.observed(&text);
            micros.push(started.elapsed().as_micros());
            assert!(!shown.contains(key));
        }
        micros.sort_unstable();
        writeln!(
            std::io::stdout().lock(),
            "redact {} bytes: p50 {:?} us p95 {:?} us p99 {:?} us",
            text.len(),
            micros.get(15),
            micros.get(30),
            micros.get(31)
        )
    }
}
