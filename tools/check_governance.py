"""Read-only structural checks. These results are not product acceptance."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit


DOCUMENTS = (
    "README.md", "index.md", "specification.md", "research-ledger.md",
    "AGENTS.md", "GOVERNANCE.md", "log.md",
)
DOCS_DIRECTORY = "docs"
MAX_DOCUMENT_BYTES = 2 * 1024 * 1024
RERANKER_MODEL = "Qwen/Qwen3-Reranker-0.6B"
RERANKER_REVISION = "e61197ed45024b0ed8a2d74b80b4d909f1255473"
FIELDS = {
    "type", "title", "description", "resource", "tags", "generated",
    "verified", "status", "stale_after", "sources",
}
INVENTORIES = (
    ("R", "requirements", 70, 1, r"^\| (R\d{2,}) \|"),
    ("D", "decisions", 35, 1, r"^### (D\d{2,}) [—-]"),
    ("E", "falsifiers", 54, 1, r"^\| (E\d{2,}) \|"),
    ("C", "execution_cells", 13, 0, r"^\| (C\d{2,}) \|"),
    ("A", "axes", 9, 1, r"^\| (A\d+) "),
)


def frontmatter_errors(name, text):
    """Validate the repository's declared bounded frontmatter syntax."""
    errors = []
    match = re.match(r"\A---\n(.*?)\n---\n", text, re.S)
    if not match:
        return [f"{name}: missing frontmatter envelope"]
    keys = []
    last_key = None
    for line in match[1].splitlines():
        if not line.strip():
            continue
        if line.startswith("  - ") and last_key == "sources":
            continue
        field = re.fullmatch(r"([a-z_]+):\s*(.*)", line)
        if not field:
            errors.append(f"{name}: unsupported frontmatter line {line!r}")
            continue
        key, value = field.groups()
        keys.append(key)
        last_key = key
        if not value and key != "sources":
            errors.append(f"{name}: empty frontmatter field {key}")
        for opening, closing in (("[", "]"), ("{", "}")):
            if value.startswith(opening) and not value.endswith(closing):
                errors.append(f"{name}: unterminated frontmatter field {key}")
    if len(keys) != len(set(keys)):
        errors.append(f"{name}: duplicate frontmatter key")
    missing = sorted(FIELDS - set(keys))
    if missing:
        errors.append(f"{name}: missing frontmatter keys {missing}")
    return errors


def markdown_structure(text):
    visible, languages, errors = [], [], []
    opened = None
    for line in text.splitlines():
        fence = re.match(r"^ {0,3}(`{3,}|~{3,})(.*)$", line)
        if not fence:
            if opened is None:
                visible.append(line)
            continue
        marker, suffix = fence.groups()
        if opened is None:
            if marker[0] == "`" and "`" in suffix:
                errors.append("malformed backtick fence")
            opened = (marker[0], len(marker))
            languages.append(suffix.strip().split()[0] if suffix.strip() else "")
        elif marker[0] == opened[0] and len(marker) >= opened[1] and not suffix.strip():
            opened = None
    if opened:
        errors.append("unclosed code fence")
    return visible, languages, errors


def document_errors(name, text, documents):
    errors = frontmatter_errors(name, text)
    if len(text.encode("utf-8")) > MAX_DOCUMENT_BYTES:
        errors.append(f"{name}: document exceeds byte limit")
    visible, _, fence_errors = markdown_structure(text)
    errors.extend(f"{name}: {error}" for error in fence_errors)
    for target in re.findall(r"\]\(([^)]+)\)", "\n".join(visible)):
        parsed = urlsplit(target)
        if parsed.scheme in ("https", "http", "mailto"):
            continue
        if parsed.scheme or parsed.netloc:
            errors.append(f"{name}: unsupported link scheme {target}")
            continue
        path = unquote(parsed.path).lstrip("/")
        if not path:
            path = name
        if ".." in Path(path).parts or path not in documents:
            errors.append(f"{name}: unresolved or escaping local link {target}")
        elif parsed.fragment and unquote(parsed.fragment) not in heading_ids(documents[path]):
            errors.append(f"{name}: unresolved heading fragment {target}")
    return errors


def heading_ids(text):
    """GitHub-style IDs for the simple ATX headings used in this repository."""
    result, occurrences = set(), {}
    visible, _, _ = markdown_structure(text)
    for title in re.findall(r"^#{1,6}\s+(.+?)\s*#*\s*$", "\n".join(visible), re.M):
        slug = re.sub(r"[^\w\- ]", "", title.lower()).replace(" ", "-")
        suffix = occurrences.get(slug, 0)
        occurrences[slug] = suffix + 1
        result.add(f"{slug}-{suffix}" if suffix else slug)
    return result


def inspect_inventories(spec, errors):
    counts, defined = {}, {}
    for prefix, label, minimum, start, pattern in INVENTORIES:
        ids = re.findall(pattern, spec, re.M)
        defined[prefix] = set(ids)
        counts[label] = len(ids)
        expected = {
            f"{prefix}{i}" if prefix == "A" else f"{prefix}{i:02d}"
            for i in range(start, start + len(ids))
        }
        if len(ids) < minimum or len(ids) != len(set(ids)) or set(ids) != expected:
            errors.append(f"{label}: incomplete, duplicate or noncontiguous inventory")
    return counts, defined


def traceability_errors(documents, defined):
    errors = []
    for name, text in documents.items():
        for prefix, ids in defined.items():
            unresolved = set(re.findall(r"\b" + prefix + r"\d+\b", text)) - ids
            if unresolved:
                errors.append(f"{name}: undefined {prefix} references {sorted(unresolved)}")
    spec = documents.get("specification.md", "")
    for row in re.findall(r"^\| R\d{2,} \|.*$", spec, re.M):
        if not re.search(r"\bD\d{2,}\b", row) or not re.search(r"\bE\d{2,}\b", row):
            errors.append(f"unmapped requirement: {row}")
    return errors


def validate(documents):
    errors = [f"missing document: {name}" for name in DOCUMENTS if name not in documents]
    for name, text in documents.items():
        errors.extend(document_errors(name, text, documents))
    spec = documents.get("specification.md", "")
    counts, defined = inspect_inventories(spec, errors)
    errors.extend(traceability_errors(documents, defined))
    execution = re.search(r"## Cell 11\b.*?(?=## Cell 12\b)", spec, re.S)
    allocated = set()
    if execution:
        for row in re.findall(r"^\| C\d{2,} \|.*$", execution[0], re.M):
            allocated.update(re.findall(r"\bE\d{2,}\b", row.split("|")[-2]))
        for cases in re.findall(r"\bC\d{2,} (?:also )?owns (E\d{2,}(?:/E\d{2,})*)", execution[0]):
            allocated.update(cases.split("/"))
    if defined["E"] - allocated:
        errors.append(f"unallocated execution cases: {sorted(defined['E'] - allocated)}")
    log = documents.get("log.md", "")
    for label, count in counts.items():
        declared = re.findall(r"^" + label + r": (\d+)$", log, re.M)
        if declared != [str(count)]:
            errors.append(f"log inventory drift: {label}, observed {count}")
    crash_lines = re.findall(r"^planned_crash_cases: (.+)$", log, re.M)
    crash = crash_lines[0].split() if len(crash_lines) == 1 else []
    if not crash or len(set(crash)) != len(crash) or not set(crash) <= defined["E"]:
        errors.append("invalid planned crash inventory")
    if len(set(crash)) * 10 < counts["falsifiers"]:
        errors.append("planned crash fraction below 10 percent")
    rows = dict(re.findall(r"^\| (E\d{2,}) \| (.+)$", spec, re.M))
    for case in crash:
        if not re.search(r"\b(?:crash(?:es|ed|ing)?|kill(?:s|ed|ing)?|panic(?:s|ked|king)?)\b", rows.get(case, ""), re.I):
            errors.append(f"planned crash case lacks fault action: {case}")
    if markdown_structure(spec)[1].count("mermaid") < 4:
        errors.append("specification lacks required visual diagrams")
    if set(re.findall(r"\bP[1-8]\b", spec)) != {f"P{i}" for i in range(1, 9)}:
        errors.append("missing proof obligation IDs")
    agents = documents.get("AGENTS.md", "")
    for anchor in ("Logical-DB", "Sentinel", "Secure-Authority", "Nova", "Moo",
                   "Logicalworks-Crates", "Self-maintenance", "memory", "NOT RUN"):
        if anchor.casefold() not in agents.casefold():
            errors.append(f"AGENTS.md: missing governance anchor {anchor}")
    return {
        "scope": "document structure only; runtime and human results NOT RUN",
        "counts": counts,
        "planned_crash_cases": crash,
        "sha256": {name: hashlib.sha256(text.encode()).hexdigest()
                   for name, text in sorted(documents.items())},
        "errors": errors,
        "ok": not errors,
    }


def load_documents(root):
    documents = {}
    docs_root = root / DOCS_DIRECTORY
    nested = sorted(docs_root.rglob("*.md")) if docs_root.is_dir() else []
    for path in [root / name for name in DOCUMENTS] + nested:
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise ValueError(f"symbolic-link document is not admitted: {relative}")
        if not path.exists():
            continue
        with path.open("rb") as source:
            content = source.read(MAX_DOCUMENT_BYTES + 1)
        if len(content) > MAX_DOCUMENT_BYTES:
            raise ValueError(f"document exceeds byte limit: {relative}")
        documents[relative] = content.decode("utf-8")
    return documents


def receipt_errors(root):
    errors = []
    receipts = sorted((root / "tools" / "receipts").glob("*.json"))
    phases = set()
    if not receipts:
        return ["reranker receipts: none retained"]
    for path in receipts:
        relative = path.relative_to(root).as_posix()
        try:
            receipt = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            errors.append(f"reranker receipt {relative}: {error}")
            continue
        required = {"phase", "model", "revision", "query_sha256", "candidates_sha256", "candidate_count", "top_k", "results"}
        if (not required <= set(receipt) or set(receipt) - (required | {"max_seconds"})
                or receipt["phase"] not in {"pre-exec", "post-exec"}
                or receipt["model"] != RERANKER_MODEL or receipt["revision"] != RERANKER_REVISION):
            errors.append(f"reranker receipt {relative}: invalid identity or shape")
        elif (not isinstance(receipt["candidate_count"], int) or not isinstance(receipt["top_k"], int)
              or not 1 <= receipt["top_k"] <= receipt["candidate_count"] <= 256
              or ("max_seconds" in receipt and (not isinstance(receipt["max_seconds"], int) or not 1 <= receipt["max_seconds"] <= 3600))
              or not all(re.fullmatch(r"[0-9a-f]{64}", receipt[key]) for key in ("query_sha256", "candidates_sha256"))
              or len(receipt["results"]) != receipt["top_k"]
              or any(not isinstance(result, dict) or set(result) != {"id", "score"}
                     or not isinstance(result["id"], str) or not result["id"]
                     or not isinstance(result["score"], (int, float)) for result in receipt["results"])):
            errors.append(f"reranker receipt {relative}: invalid bounds or hashes")
        else:
            phases.add(receipt["phase"])
    if phases != {"pre-exec", "post-exec"}:
        errors.append("reranker receipts: both pre-exec and post-exec phases are required")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--report", action="store_true", help="emit structural JSON report")
    args = parser.parse_args()
    try:
        report = validate(load_documents(args.root))
        report["errors"].extend(receipt_errors(args.root))
        report["ok"] = not report["errors"]
    except (OSError, UnicodeError, ValueError) as error:
        report = {"ok": False, "errors": [str(error)]}
    if args.report:
        print(json.dumps(report, indent=2, sort_keys=True))
    elif report["ok"]:
        print("PASS: document structure and governance inventory")
        print(json.dumps(report["counts"], sort_keys=True))
        print("NOT CHECKED BY THIS GATE: Mermaid rendering")
        print("NOT RUN: runtime, model evaluations, hardware and human trials")
    else:
        for error in report["errors"]:
            print("FAIL: " + error, file=sys.stderr)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
