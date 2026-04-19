#!/usr/bin/env -S uv run --script
#
# /// script
# requires-python = ">=3.12"
# ///
"""Print the pending (fragments-only) changelog to stdout.

Between releases `CHANGELOG.md` intentionally contains only released
versions — that's what stops PRs from conflicting on it. To see what's
accumulated under `changelog.d/` run this script. It writes nothing.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from changelog_lib import load_fragments, render_unreleased_md  # type: ignore[import-not-found]


def main() -> int:
    fragments = load_fragments()
    print(render_unreleased_md(fragments), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
