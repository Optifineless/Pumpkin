#!/usr/bin/env python3
"""Fork-only check: keep new code in small files and stop upstream's giants from growing.

Usage:
  python .fork/check-sizes.py [base-ref]        check (default base: upstream/master)
  python .fork/check-sizes.py --write-baseline  record current sizes of files over the giant threshold

Rules (see the fork section of AGENTS.md):
- A .rs file that does not exist on the base ref must be <= 1000 lines (fail) and should be <= 600 (warn).
- A .rs file that is already > 3000 lines may grow by at most 150 lines (fail); any growth is listed.
  Growth is measured from .fork/size-baseline.json when the file is listed there (sizes recorded when the
  rule was adopted, so growth that predates the rule is not counted), otherwise from the base ref.
- Generated code (any path containing /generated/) and tests under tests/ are ignored.
Exit code 1 on any failure.
"""
import json
import os
import subprocess
import sys

NEW_WARN = 600
NEW_FAIL = 1000
GIANT = 3000
GIANT_GROWTH = 150
BASELINE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "size-baseline.json")


def git(*args):
    return subprocess.run(
        ["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace", check=False
    ).stdout


def line_count_at(ref, path):
    out = subprocess.run(
        ["git", "show", f"{ref}:{path}"],
        capture_output=True, text=True, encoding="utf-8", errors="replace", check=False,
    )
    if out.returncode != 0:
        return None
    return out.stdout.count("\n")


def line_count(path):
    with open(path, encoding="utf-8", errors="replace") as fh:
        return sum(1 for _ in fh)


def tracked_sources():
    return [
        f for f in git("ls-files", "--cached", "--others", "--exclude-standard", "*.rs").split("\n")
        if f and "/generated/" not in f and not f.startswith("tests/")
    ]


def write_baseline(files):
    giants = {}
    for path in files:
        n = line_count(path)
        if n > GIANT:
            giants[path] = n
    with open(BASELINE, "w", encoding="utf-8") as fh:
        json.dump(giants, fh, indent=2, sort_keys=True)
        fh.write("\n")
    print(f"wrote baseline for {len(giants)} files over {GIANT} lines")


def main():
    positional = [a for a in sys.argv[1:] if not a.startswith("--")]
    base = positional[0] if positional else "upstream/master"
    if not git("rev-parse", "--verify", base).strip():
        base = "master"
    files = tracked_sources()
    if "--write-baseline" in sys.argv:
        write_baseline(files)
        return 0

    baseline = {}
    if os.path.exists(BASELINE):
        with open(BASELINE, encoding="utf-8") as fh:
            baseline = json.load(fh)

    failures, warnings = [], []
    for path in files:
        try:
            ours = line_count(path)
        except OSError:
            continue
        theirs = baseline.get(path)
        if theirs is None:
            theirs = line_count_at(base, path)
        if theirs is None:
            if ours > NEW_FAIL:
                failures.append(f"new file {path} has {ours} lines (limit {NEW_FAIL}); split it")
            elif ours > NEW_WARN:
                warnings.append(f"new file {path} has {ours} lines (aim for <= {NEW_WARN})")
        elif theirs > GIANT and ours - theirs > GIANT_GROWTH:
            failures.append(
                f"{path} grew by {ours - theirs} lines ({theirs} -> {ours}); move new logic into a sibling module"
            )
        elif theirs > GIANT and ours > theirs:
            warnings.append(f"{path} grew by {ours - theirs} lines ({theirs} -> {ours})")

    for w in warnings:
        print(f"warning: {w}")
    for f in failures:
        print(f"error: {f}")
    print(f"checked {len(files)} files: {len(failures)} failure(s), {len(warnings)} warning(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
