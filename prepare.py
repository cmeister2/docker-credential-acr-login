#!/usr/bin/env python3
"""Replace the sentinel crate version with the release version."""

import sys
from pathlib import Path


VERSION = sys.argv[1]

for path in (Path("Cargo.toml"), Path("Cargo.lock")):
    text = path.read_text()
    updated = text.replace('version = "0.0.0"', f'version = "{VERSION}"', 1)
    path.write_text(updated)
