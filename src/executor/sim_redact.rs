//! Seeded simulations of what leaves for a model (larry#589). Source: OWASP ASVS
//! 4.0.3 §8.3.4 and CWE-200; the token prefixes each provider documents; RFC 7468
//! (PEM labels). Each drives the real [`Redactor`] and, where a cut matters, the
//! real [`clip`].

use lgwks_std::seeded::Seeded;

use super::super::run::clip;
use super::super::sim_tree::scratch;
use super::Redactor;

/// What every simulation returns.
type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// The configured key every case hides.
const KEY: &str = "zq7-route-key-0123456789abcdef";

/// Seeds each sweep runs.
const SEEDS: u64 = 2_000;

/// A redactor of [`KEY`] that knows no directory.
fn keyed() -> Redactor {
    Redactor::new([String::from(KEY)])
}

/// A drawn number of characters, under 64, that hold no part of the key and no
/// token prefix.
macro_rules! words {
    ($rng:expr) => {{
        let mut text = String::new();
        for _ in 0..$rng.below(64)? {
            text.push(match $rng.below(6)? {
                0 => 'a',
                1 => 'b',
                2 => ' ',
                3 => '\n',
                4 => 'é',
                _ => '=',
            });
        }
        text
    }};
}

/// A key anywhere, cut at any bound afterwards, leaves none of its head: the
/// mask keeps every other byte where it was.
#[test]
fn sim_a_key_cut_at_any_bound_leaves_no_head() -> TestResult {
    let redactor = keyed();
    for seed in 0..SEEDS {
        let mut rng = Seeded::from_seed(seed);
        let before = words!(rng);
        let after = words!(rng);
        let text = format!("{before}{KEY}{after}");
        let mut shown = redactor.observed(&text);
        let masked = format!("{before}{}{after}", "*".repeat(KEY.len()));
        assert_eq!(shown, masked, "seed {seed}: only the key is masked");
        let bound = usize::try_from(rng.below(200)?)?;
        clip(&mut shown, bound);
        assert!(!shown.contains("zq7"), "seed {seed}: {shown}");
    }
    Ok(())
}

/// A key shorter than eight bytes is a placeholder and is left; eight is masked.
#[test]
fn sim_only_a_key_of_eight_bytes_or_more_is_masked() {
    let short = Redactor::new([String::from("ollama1")]);
    assert_eq!(short.observed("run ollama1 now"), "run ollama1 now");
    assert!(!short.holds_secret("run ollama1 now"));
    let whole = Redactor::new([String::from("ollama12")]);
    assert_eq!(whole.observed("run ollama12 now"), "run ******** now");
    assert!(whole.holds_secret("run ollama12 now"));
}

/// Every documented token shape is masked at its shortest and kept one byte
/// short of it; a prefix inside a longer word is a word.
#[test]
fn sim_every_token_shape_is_masked_and_words_are_kept() {
    let redactor = Redactor::default();
    for shape in super::SHAPES {
        let token = format!("{}{}", shape.0, "7".repeat(shape.1));
        let held = format!("key: \"{token}\" end");
        let hidden = format!("key: \"{}\" end", "*".repeat(token.len()));
        assert_eq!(redactor.observed(&held), hidden, "{}", shape.0);
        let short = format!("{}{}", shape.0, "7".repeat(shape.1.saturating_sub(1)));
        assert_eq!(redactor.observed(&short), short, "{} one short", shape.0);
        let inside = format!("x{token}");
        assert_eq!(redactor.observed(&inside), inside, "{} inside", shape.0);
    }
}

/// An AWS access key id is a prefix and exactly sixteen capitals or digits, so a
/// constant that merely starts the same way is kept.
#[test]
fn sim_an_access_key_id_is_exactly_its_documented_shape() {
    let redactor = Redactor::default();
    for prefix in super::AWS_PREFIXES {
        let id = format!("{prefix}ABCDEFGHIJ012345");
        let shown = redactor.observed(&format!("{id}."));
        assert_eq!(shown, format!("{}.", "*".repeat(20)));
        for kept in [
            format!("{prefix}ABCDEFGHIJ01234"),
            format!("{prefix}ABCDEFGHIJ0123456"),
            format!("{prefix}abcdefghij012345"),
            format!("{prefix}_PACIFIC_REGIONS"),
        ] {
            assert_eq!(redactor.observed(&kept), kept);
        }
    }
}

/// A private key block is masked with its lines kept, to the end of the text when
/// its closing label was cut away; a certificate is public and kept.
#[test]
fn sim_a_private_key_block_is_masked_even_unclosed() {
    let redactor = Redactor::default();
    let label = ["PRIVATE", "KEY"].join(" ");
    let open = format!("-----BEGIN RSA {label}-----");
    let close = format!("-----END RSA {label}-----");
    let stars = |line: &str| "*".repeat(line.len());
    let shown = redactor.observed(&format!("a\n{open}\nMIIB\n{close}\nz"));
    let masked = format!("a\n{}\n****\n{}\nz", stars(&open), stars(&close));
    assert_eq!(shown, masked);
    let cut = redactor.observed(&format!("a\n{open}\nMIIB\nmore"));
    assert_eq!(cut, format!("a\n{}\n****\n****", stars(&open)));
    let public = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----";
    assert_eq!(redactor.observed(public), public);
}

/// A directory takes its name wherever it stands whole, the inner one first, as
/// given and as resolved; a longer sibling keeps its name and `/` is no
/// directory.
#[test]
fn sim_a_directory_is_renamed_inner_first_and_never_a_sibling() -> TestResult {
    let root = scratch("redact-paths")?;
    let home = root.join("u");
    let state = home.join("state");
    std::fs::create_dir_all(&state)?;
    let link = root.join("link");
    std::os::unix::fs::symlink(&home, &link)?;
    let mut redactor = Redactor::default();
    redactor.hide(std::path::Path::new("/"), "<root>");
    redactor.hide(&link, "~");
    redactor.hide(&state, "<state>");
    let sibling = root.join("user");
    let text = format!(
        "{}/ws {}/notes {}/n {} /etc",
        state.display(),
        link.display(),
        std::fs::canonicalize(&home)?.display(),
        sibling.display()
    );
    let want = format!("<state>/ws ~/notes ~/n {} /etc", sibling.display());
    assert_eq!(redactor.observed(&text), want);
    assert!(!redactor.holds_secret(&text), "a path is not a secret");
    Ok(())
}
