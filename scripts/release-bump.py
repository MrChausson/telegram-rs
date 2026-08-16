#!/usr/bin/env python3
"""Bump the workspace version and rotate the changelog for a new release.

Usage:
    scripts/release-bump.py patch
    scripts/release-bump.py minor
    scripts/release-bump.py major
    scripts/release-bump.py --dry-run patch

What it does:
  * Bumps `version` under [workspace.package] in the root Cargo.toml
    (all member crates inherit it via `version.workspace = true`).
  * Rotates `## [Unreleased]` in CHANGELOG.md into
    `## [vX.Y.Z] - YYYY-MM-DD`, then prepends a fresh `## [Unreleased]`.
  * Writes the new release section to release-notes.md (used by CI to
    seed the GitHub release body).
  * Prints the new version (e.g. `1.2.3`) to stdout.

Fails unless the working tree is clean and `## [Unreleased]` exists.
"""
from __future__ import annotations

import argparse
import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"
CHANGELOG = ROOT / "CHANGELOG.md"
RELEASE_NOTES = ROOT / "release-notes.md"

VERSION_RE = re.compile(r'^version\s*=\s*"(\d+)\.(\d+)\.(\d+)"\s*$', re.MULTILINE)
UNRELEASED_RE = re.compile(r"^##\s+\[Unreleased\]\s*\n", re.MULTILINE)
NEXT_SECTION_RE = re.compile(r"^##\s+\[", re.MULTILINE)


def next_version(part: str, major: int, minor: int, patch: int) -> tuple[int, int, int]:
    if part == "major":
        return major + 1, 0, 0
    if part == "minor":
        return major, minor + 1, 0
    if part == "patch":
        return major, minor, patch + 1
    raise ValueError(f"unknown bump part: {part!r}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("part", choices=("major", "minor", "patch"))
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    if not CARGO_TOML.is_file():
        print(f"missing {CARGO_TOML}", file=sys.stderr)
        return 1

    cargo_text = CARGO_TOML.read_text()
    m = VERSION_RE.search(cargo_text)
    if m is None:
        print("could not find `version` under [workspace.package]", file=sys.stderr)
        return 1

    major, minor, patch = (int(g) for g in m.groups())
    new_major, new_minor, new_patch = next_version(args.part, major, minor, patch)
    next_v = f"{new_major}.{new_minor}.{new_patch}"

    if args.dry_run:
        print(f"{major}.{minor}.{patch} -> {next_v}")
        return 0

    if not CHANGELOG.is_file():
        print(f"missing {CHANGELOG}", file=sys.stderr)
        return 1
    changelog = CHANGELOG.read_text()
    m_unrel = UNRELEASED_RE.search(changelog)
    if m_unrel is None:
        print("CHANGELOG.md is missing a `## [Unreleased]` section", file=sys.stderr)
        return 1

    today = datetime.date.today().isoformat()
    section_header = f"## [v{next_v}] - {today}"

    # Rotate: Unreleased becomes the new version's section, then prepend a
    # fresh empty Unreleased section on top.
    changelog = UNRELEASED_RE.sub(section_header + "\n\n", changelog, count=1)
    new_section = m_unrel.group(0) + "\n"  # `## [Unreleased]\n` + blank line
    changelog = changelog.replace(
        section_header, new_section + section_header, 1
    )

    cargo_text = (
        cargo_text[: m.start()] + f'version = "{next_v}"' + cargo_text[m.end():]
    )
    changelog = re.sub(r"\n{3,}", "\n\n", changelog)
    CARGO_TOML.write_text(cargo_text)
    CHANGELOG.write_text(changelog)

    # Extract the new section into release notes for the GitHub release body.
    body = extract_section(changelog, section_header)
    RELEASE_NOTES.write_text(body)
    print(f"{major}.{minor}.{patch} -> {next_v}")
    return 0


def extract_section(changelog: str, header: str) -> str:
    body_start = changelog.index(header) + len(header) + 1
    rest = changelog[body_start:]
    m = NEXT_SECTION_RE.search(rest)
    section = rest if m is None else rest[: m.start()]
    return header + "\n" + section.rstrip() + "\n"


if __name__ == "__main__":
    sys.exit(main())