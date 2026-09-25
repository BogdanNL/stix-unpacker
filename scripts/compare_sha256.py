#!/usr/bin/env python3
"""Compare extracted files with a possibly incomplete reference directory."""

import argparse
import hashlib
import json
from pathlib import Path


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def inventory(root, recode=None):
    files = {}
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        key = relative
        if recode:
            source, destination = recode.split(":", 1)
            key = key.encode(source).decode(destination)
        key = key.casefold()
        if key in files:
            raise ValueError(f"Case-insensitive filename collision: {relative}")
        files[key] = {"path": relative, "size": path.stat().st_size, "sha256": digest(path)}
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("actual", type=Path)
    parser.add_argument("reference", type=Path)
    parser.add_argument("--reference-recode", help="Reinterpret reference names, e.g. cp437:cp1251")
    parser.add_argument("--report", type=Path, help="Write a detailed JSON report")
    args = parser.parse_args()
    if not args.actual.is_dir() or not args.reference.is_dir():
        parser.error("Both inputs must be existing directories")
    actual = inventory(args.actual)
    reference = inventory(args.reference, args.reference_recode)
    matched, different = [], []
    for key in sorted(actual.keys() & reference.keys()):
        pair = {"actual": actual[key], "reference": reference[key]}
        if actual[key]["sha256"] == reference[key]["sha256"]:
            matched.append(pair)
        else:
            different.append(pair)
    report = {
        "actual_directory": str(args.actual),
        "reference_directory": str(args.reference),
        "reference_name_recoding": args.reference_recode,
        "actual_file_count": len(actual),
        "reference_file_count": len(reference),
        "matched_count": len(matched),
        "different_count": len(different),
        "matched": matched,
        "different": different,
        "actual_only": [actual[k] for k in sorted(actual.keys() - reference.keys())],
        "reference_only": [reference[k] for k in sorted(reference.keys() - actual.keys())],
    }
    if args.report:
        args.report.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"Actual files: {len(actual)}; reference files: {len(reference)}")
    print(f"SHA-256 matches: {len(matched)}; mismatches: {len(different)}")
    print(f"Actual only: {len(report['actual_only'])}; reference only: {len(report['reference_only'])}")
    for entry in report["reference_only"]:
        print(f"Reference only: {entry['path']}")
    return 1 if different or report["actual_only"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
