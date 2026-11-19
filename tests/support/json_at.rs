//! One checked read of a JSON reply for every journey and simulation (larry#341). `value["a"][0]`
//! indexing is refused by the workspace lint floor (`clippy::indexing_slicing`), so a test reads a
//! member through an RFC 6901 JSON pointer instead. A member the reply lacks is a typed
//! [`Missing`] refusal naming the pointer, so the test fails on the member it needed rather than
//! comparing against a substituted `null`. A test that proves a member is absent asks
//! `value.at(..).is_err()` directly. Included by path into each test crate that reads
//! replies; every use is an assertion in those tests.

use std::fmt;

use lgwks_std::json::Value;

/// A JSON reply lacked the member a test read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Missing {
    /// The RFC 6901 pointer that named no member.
    pointer: String,
}

impl fmt::Display for Missing {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "the reply has no member at {}", self.pointer)
    }
}

impl std::error::Error for Missing {}

/// Reads a member of a JSON value by an RFC 6901 pointer such as `/result/content/0/text`.
pub(crate) trait At {
    /// The member `pointer` names.
    ///
    /// # Errors
    ///
    /// [`Missing`] when the value has no member at `pointer`.
    fn at(&self, pointer: &str) -> Result<&Value, Missing>;

    /// The member `pointer` names, or `None`: for a reader whose own answer to an absent member
    /// is already a refusal or a mismatch it reports (`lgwks_std::json::Value` has no
    /// `pointer`).
    fn member(&self, pointer: &str) -> Option<&Value>;

    /// Whether `pointer` names a member that is present and not `null`; the negation of the
    /// `value["a"].is_null()` check it replaces, which read absent and `null` alike.
    fn holds(&self, pointer: &str) -> bool;
}

impl At for Value {
    fn at(&self, pointer: &str) -> Result<&Value, Missing> {
        match self.member(pointer) {
            Some(found) => Ok(found),
            None => Err(Missing {
                pointer: String::from(pointer),
            }),
        }
    }

    fn member(&self, pointer: &str) -> Option<&Value> {
        let mut node = self;
        for escaped in pointer.split('/').skip(1) {
            // RFC 6901 section 4: `~1` is a `/` inside a key and `~0` a `~`, decoded in that order.
            let segment = escaped.replace("~1", "/").replace("~0", "~");
            node = match *node {
                Value::Object(_) => node.get(segment.as_str())?,
                Value::Array(ref items) => match segment.parse::<usize>() {
                    Ok(position) => items.get(position)?,
                    Err(_not_an_index) => return None,
                },
                _ => return None,
            };
        }
        Some(node)
    }

    fn holds(&self, pointer: &str) -> bool {
        matches!(self.member(pointer), Some(found) if !found.is_null())
    }
}

/// A present member reads as itself, at any depth and through arrays; an absent one, a wrong
/// index and a step through a scalar are each the typed refusal naming the pointer.
#[test]
fn a_pointer_reads_present_members_and_refuses_absent_ones() -> Result<(), Missing> {
    let reply = lgwks_std::json::json!({"result": {"content": [{"text": "hi"}], "n": 3}});
    assert_eq!(
        reply.at("/result/content/0/text")?,
        &lgwks_std::json::json!("hi"),
        "a nested member through an array"
    );
    assert_eq!(
        reply.at("/result/n")?,
        &lgwks_std::json::json!(3),
        "a number"
    );
    for absent in [
        "/error",
        "/result/content/1",
        "/result/n/deeper",
        "/result/content/x",
    ] {
        assert_eq!(
            reply.at(absent),
            Err(Missing {
                pointer: String::from(absent)
            }),
            "{absent} is refused by name"
        );
        assert!(!reply.holds(absent), "{absent} holds nothing");
    }
    let nulled = lgwks_std::json::json!({"error": null});
    assert!(!nulled.holds("/error"), "a null member holds nothing");
    assert!(reply.holds("/result/n"), "a present member holds");
    let escaped = lgwks_std::json::json!({"_meta": {"io.mcp/serverInfo": {"a~b": 1}}});
    assert_eq!(
        escaped.at("/_meta/io.mcp~1serverInfo/a~0b")?,
        &lgwks_std::json::json!(1),
        "RFC 6901 escapes: ~1 is a slash and ~0 a tilde inside a key"
    );
    Ok(())
}
