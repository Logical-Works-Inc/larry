#!/usr/bin/env python3
"""Render the Rocco progress tracker from its data file.

The tracker is generated, never hand-edited. `tracker.json` is the single
hand-maintained file; `template.html` is the page; `index.html` is the render
that is committed and published.

    python3 tools/tracker/generate.py           # write index.html
    python3 tools/tracker/generate.py --check   # exit 1 if it is stale

`tools/check_governance.py` calls `tracker_errors`, so a tracker that has
drifted from its data, from the specification's cell inventory, or from the
specification's falsifier inventory fails the repository gate rather than
quietly going stale.

Every completion percentage in the data is an ESTIMATE. This tool renders; it
does not measure, and nothing here may be cited as runtime proof.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

TRACKER_DIRECTORY = Path("tools") / "tracker"
DATA_NAME = "tracker.json"
TEMPLATE_NAME = "template.html"
OUTPUT_NAME = "index.html"
DATA_TOKEN = "__TRACKER_DATA__"

# An execution-cell row: `| C02 | inputs | output | exit |`.
CELL_ROW = re.compile(r"^\|\s*(C\d{2})\s*\|")
# A falsifier-inventory row: `| E55 | scenario | falsified by |`.
FALSIFIER_ROW = re.compile(r"^\|\s*(E\d{2})\s*\|")


def load_data(directory: Path) -> dict:
    """Read and parse the tracker data file."""
    with (directory / DATA_NAME).open(encoding="utf-8") as source:
        return json.load(source)


def render(data: dict, template: str) -> str:
    """Return the page for `data`, deterministically.

    `<` is escaped so that a value can never close the surrounding `script`
    element, and the dump is key-sorted so the same data always renders to the
    same bytes.
    """
    payload = json.dumps(data, indent=2, sort_keys=True, ensure_ascii=True)
    payload = payload.replace("<", "\\u003c")
    if DATA_TOKEN not in template:
        raise ValueError(f"template is missing the {DATA_TOKEN} token")
    return template.replace(DATA_TOKEN, payload)


def build(root: Path) -> str:
    """Return the page the tracker should currently contain."""
    directory = root / TRACKER_DIRECTORY
    with (directory / TEMPLATE_NAME).open(encoding="utf-8") as source:
        template = source.read()
    return render(load_data(directory), template)


def _rows(path: Path, pattern: re.Pattern[str]) -> list[str]:
    if not path.is_file():
        return []
    found: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = pattern.match(line)
        if match and match.group(1) not in found:
            found.append(match.group(1))
    return found


def spec_cell_ids(root: Path) -> list[str]:
    """The execution-cell ids as declared by specification.md."""
    return _rows(root / "specification.md", CELL_ROW)


def spec_falsifier_ids(root: Path) -> list[str]:
    """The falsifier ids as declared by specification.md."""
    return _rows(root / "specification.md", FALSIFIER_ROW)


def tracker_errors(root: Path) -> list[str]:
    """Return every way the tracker has drifted, for the governance gate."""
    directory = root / TRACKER_DIRECTORY
    errors: list[str] = []

    try:
        data = load_data(directory)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        return [f"progress tracker: unreadable data ({error})"]

    try:
        expected = build(root)
    except (OSError, UnicodeError, ValueError) as error:
        return [f"progress tracker: cannot render ({error})"]

    rendered = directory / OUTPUT_NAME
    if not rendered.is_file():
        errors.append(f"progress tracker: {TRACKER_DIRECTORY / OUTPUT_NAME} is missing")
    else:
        actual = rendered.read_text(encoding="utf-8")
        if actual != expected:
            errors.append(
                f"progress tracker: {TRACKER_DIRECTORY / OUTPUT_NAME} is stale "
                f"— regenerate with `python3 {TRACKER_DIRECTORY / 'generate.py'}`"
            )

    declared_cells = spec_cell_ids(root)
    tracked_cells = [cell.get("id") for cell in data.get("cells", [])]
    if declared_cells and tracked_cells != declared_cells:
        missing = sorted(set(declared_cells) - set(tracked_cells))
        extra = sorted(set(tracked_cells) - set(declared_cells))
        errors.append(
            "progress tracker: cell inventory does not match specification.md "
            f"(missing {missing or 'none'}, unexpected {extra or 'none'}, "
            "order must match the specification)"
        )

    declared_falsifiers = set(spec_falsifier_ids(root))
    if declared_falsifiers:
        unknown = sorted(
            {f.get("id") for f in data.get("falsifiers", [])} - declared_falsifiers
        )
        if unknown:
            errors.append(
                f"progress tracker: falsifier ids not in specification.md: {unknown}"
            )
        expected_total = data.get("falsifier_total")
        if expected_total != len(declared_falsifiers):
            errors.append(
                "progress tracker: falsifier_total is "
                f"{expected_total}, specification.md declares {len(declared_falsifiers)}"
            )

    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--check", action="store_true", help="verify, do not write")
    args = parser.parse_args()

    directory = args.root / TRACKER_DIRECTORY
    try:
        rendered = build(args.root)
    except (OSError, UnicodeError, ValueError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1

    output = directory / OUTPUT_NAME
    if args.check:
        if output.is_file() and output.read_text(encoding="utf-8") == rendered:
            print(f"PASS: {TRACKER_DIRECTORY / OUTPUT_NAME} is current")
            return 0
        print(
            f"FAIL: {TRACKER_DIRECTORY / OUTPUT_NAME} is stale — run "
            f"`python3 {TRACKER_DIRECTORY / 'generate.py'}`",
            file=sys.stderr,
        )
        return 1

    output.write_text(rendered, encoding="utf-8")
    print(f"wrote {TRACKER_DIRECTORY / OUTPUT_NAME} ({len(rendered)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
