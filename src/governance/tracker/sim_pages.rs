//! Adversarial controls of the tracker render, write and drift report (larry#554): hostile data,
//! templates and pages, and counts at the edges of their types.
//! Source: the Python generator the tracker replaced, CWE-59 and CWE-400. Requirement: every
//! failure names its file, and the drift report names the exact mismatch.

use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use lgwks_std::json::{self, Value};

use super::super::patterns::Patterns;
use super::super::pyjson;
use super::super::sim_support::{Scratch, SimResult};
use super::{
    MAX_TRACKER_BYTES, TrackerError, counts_equal, drift_errors, ids_of, is_current, load_data,
    or_none, read_bounded, relative, render, render_with, write,
};

/// The refusal for a page that is not there.
const MISSING_PAGE: &str = "progress tracker: tools/tracker/index.html is missing";

/// Sets the mode of `path`.
fn chmod(path: &Path, mode: u32) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

/// The tracker's relative path of `name`.
fn at(name: &str) -> PathBuf {
    relative(name)
}

/// A tracker tree holding `data` and a template that is only the token.
fn tracker(tag: &str, data: &Value) -> Result<Scratch, super::super::sim_support::SimError> {
    let tree = Scratch::new(tag)?;
    tree.write(
        "tools/tracker/tracker.json",
        pyjson::dumps(data, None).as_bytes(),
    )?;
    tree.write("tools/tracker/template.html", b"__TRACKER_DATA__")?;
    Ok(tree)
}

/// A total equals the count only as a whole number of that value, and an empty list prints
/// `none`.
#[test]
fn sim_a_total_equals_a_count_only_as_that_whole_number() {
    assert_eq!(or_none(&[]), "none");
    assert_eq!(
        or_none(&[String::from("'a'"), String::from("'b'")]),
        "['a', 'b']"
    );
    for equal in [json::json!(3), json::json!(3.0)] {
        assert!(counts_equal(Some(&equal), 3), "{equal:?}");
    }
    for unequal in [
        json::json!(4),
        json::json!(-3),
        json::json!(3.5),
        json::json!("3"),
        json::json!(true),
        json::json!(null),
    ] {
        assert!(!counts_equal(Some(&unequal), 3), "{unequal:?}");
    }
    assert!(!counts_equal(Some(&json::json!(true)), 1));
    assert!(!counts_equal(None, 0));
}

/// The page escapes every `<`, so no value closes its script element (CWE-79), and a template
/// without the token is refused.
#[test]
fn sim_no_data_value_closes_the_page_script() -> SimResult {
    let hostile = json::json!({"a": "</script><b>"});
    let page = render_with(&hostile, "<script>__TRACKER_DATA__</script>")?;
    assert_eq!(
        page,
        "<script>{\n  \"a\": \"\\u003c/script>\\u003cb>\"\n}</script>"
    );
    let refused = render_with(&hostile, "no token");
    assert!(
        matches!(refused, Err(TrackerError::MissingToken)),
        "{refused:?}"
    );
    Ok(())
}

/// Every unreadable tracker file is its own refusal naming the file.
#[test]
fn sim_each_unreadable_tracker_file_is_named_by_its_own_refusal() -> SimResult {
    let tree = Scratch::new("tracker-faults")?;
    let data = at("tracker.json");
    let read = |relative: &Path| read_bounded(tree.path(), relative);
    let refused = read(&data);
    assert!(
        matches!(&refused, Err(TrackerError::Read { path, .. }) if *path == data),
        "{refused:?}"
    );
    tree.write("tools/tracker/tracker.json", b"{")?;
    let refused = load_data(tree.path());
    assert!(matches!(refused, Err(TrackerError::Data(_))), "{refused:?}");
    tree.write("tools/tracker/template.html", b"no token")?;
    tree.write("tools/tracker/tracker.json", b"{}")?;
    let refused = render(tree.path());
    assert!(
        matches!(refused, Err(TrackerError::MissingToken)),
        "{refused:?}"
    );

    let directory = tree.path().join("tools/tracker");
    symlink(directory.join("tracker.json"), directory.join("link.json"))?;
    let link = at("link.json");
    let refused = read(&link);
    assert!(
        matches!(&refused, Err(TrackerError::Symlink { path }) if *path == link),
        "{refused:?}"
    );
    fs::create_dir(directory.join("folder.json"))?;
    let folder = at("folder.json");
    let refused = read(&folder);
    assert!(
        matches!(&refused, Err(TrackerError::NotRegular { path }) if *path == folder),
        "{refused:?}"
    );
    tree.write("tools/tracker/raw.json", &[b'{', 0xff])?;
    let refused = read(&at("raw.json")).map_err(|cause| cause.to_string());
    assert!(
        matches!(&refused, Err(told)
            if told.starts_with("'utf-8' codec can't decode byte 0xff in position 1")),
        "{refused:?}"
    );
    let bound = usize::try_from(MAX_TRACKER_BYTES)?;
    tree.write("tools/tracker/big.json", &vec![b' '; bound])?;
    assert_eq!(read(&at("big.json"))?.len(), bound, "the bound is admitted");
    tree.write(
        "tools/tracker/big.json",
        &vec![b' '; bound.saturating_add(1)],
    )?;
    let refused = read(&at("big.json"));
    assert!(
        matches!(&refused, Err(TrackerError::TooLarge { limit, .. }) if *limit == MAX_TRACKER_BYTES),
        "{refused:?}"
    );
    Ok(())
}

/// A page is current only when it is the render byte for byte; the drift report names a missing,
/// a stale, an unreadable and an unrenderable tracker apart.
#[test]
fn sim_a_missing_stale_or_unreadable_page_is_reported_apart() -> SimResult {
    let patterns = Patterns::new()?;
    let tree = tracker("tracker-page", &json::json!({}))?;
    let drift = || drift_errors(tree.path(), "", &patterns);
    assert!(!is_current(tree.path())?);
    assert_eq!(drift(), [MISSING_PAGE]);
    assert_eq!(write(tree.path())?, 2);
    assert!(is_current(tree.path())?);
    assert_eq!(drift(), Vec::<String>::new());

    tree.write("tools/tracker/index.html", b"{ }")?;
    assert!(!is_current(tree.path())?);
    assert_eq!(
        drift(),
        ["progress tracker: tools/tracker/index.html is stale — regenerate with `larry tracker`"]
    );

    let page = tree.path().join("tools/tracker/index.html");
    chmod(&page, 0o000)?;
    let unreadable = drift();
    let current = is_current(tree.path());
    chmod(&page, 0o644)?;
    assert!(
        matches!(unreadable.as_slice(), [one]
            if one.starts_with("progress tracker: cannot read tools/tracker/index.html: ")),
        "{unreadable:?}"
    );
    assert!(
        matches!(current, Err(TrackerError::Read { .. })),
        "{current:?}"
    );

    tree.write("tools/tracker/template.html", b"no token")?;
    assert_eq!(
        drift(),
        ["progress tracker: cannot render (template is missing the __TRACKER_DATA__ token)"]
    );
    fs::remove_file(tree.path().join("tools/tracker/tracker.json"))?;
    let missing = drift();
    assert!(
        matches!(missing.as_slice(), [one]
            if one.starts_with("progress tracker: unreadable data (cannot read tools/tracker/tracker.json: ")),
        "{missing:?}"
    );
    Ok(())
}

/// A write into a directory that refuses it is a typed write refusal and leaves the page as it
/// was.
#[test]
fn sim_a_refused_write_keeps_the_page_that_was_there() -> SimResult {
    let tree = tracker("tracker-write", &json::json!({}))?;
    tree.write("tools/tracker/index.html", b"before")?;
    let directory = tree.path().join("tools/tracker");
    chmod(&directory, 0o555)?;
    let refused = write(tree.path());
    chmod(&directory, 0o755)?;
    assert!(
        matches!(refused, Err(TrackerError::Write { .. })),
        "{refused:?}"
    );
    assert_eq!(tree.read("tools/tracker/index.html")?, "before");
    Ok(())
}

/// The drift report names a cell inventory that differs, an unknown falsifier id and a total
/// that is not the declared count; an item with no id equals none.
#[test]
fn sim_a_drifted_inventory_is_named_exactly() -> SimResult {
    let patterns = Patterns::new()?;
    let spec = "| E01 | Kill it |\n| C00 | Owns E01. |\n| C01 | Later. |\n| C01 | Again. |\n";
    let drifted = json::json!({
        "falsifier_total": 2,
        "cells": [{"id": "C00"}, {"id": "C09"}],
        "falsifiers": [{"id": "E02"}, {"no": 1}],
    });
    let tree = tracker("tracker-drift", &drifted)?;
    write(tree.path())?;
    assert_eq!(
        drift_errors(tree.path(), spec, &patterns),
        [
            "progress tracker: cell inventory does not match specification.md (missing ['C01'], unexpected ['C09'], order must match the specification)",
            "progress tracker: falsifier ids not in specification.md: ['E02', None]",
            "progress tracker: falsifier_total is 2, specification.md declares 1",
        ]
    );
    assert_eq!(ids_of(&drifted, "absent", &patterns).len(), 0);

    let reordered = json::json!({"cells": [{"id": "C01"}, {"id": "C00"}]});
    let tree = tracker("tracker-order", &reordered)?;
    write(tree.path())?;
    assert_eq!(
        drift_errors(tree.path(), spec, &patterns),
        [
            "progress tracker: cell inventory does not match specification.md (missing none, unexpected none, order must match the specification)",
            "progress tracker: falsifier_total is None, specification.md declares 1",
        ]
    );

    let settled = json::json!({
        "falsifier_total": 1,
        "cells": [{"id": "C00"}, {"id": "C01"}],
        "falsifiers": [{"id": "E01"}],
    });
    let tree = tracker("tracker-settled", &settled)?;
    write(tree.path())?;
    assert_eq!(
        drift_errors(tree.path(), spec, &patterns),
        Vec::<String>::new()
    );
    Ok(())
}
