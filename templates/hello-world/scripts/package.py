#!/usr/bin/env python3
"""Build a deterministic Paraco app release package."""

import argparse
import base64
import hashlib
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
PACKAGE_FILES = ("main.ts", "paraco.json")
VERSION_PATTERN = re.compile(r"(?:0|[1-9][0-9]{0,8})\.(?:0|[1-9][0-9]{0,8})\.(?:0|[1-9][0-9]{0,8})\Z")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="release version (MAJOR.MINOR.PATCH)")
    parser.add_argument("--output", required=True, type=Path, help="package output path")
    args = parser.parse_args()

    if not VERSION_PATTERN.fullmatch(args.version):
        parser.error("--version must have three numeric segments without leading zeros")

    files = []
    for relative_path in sorted(PACKAGE_FILES):
        content = (ROOT / relative_path).read_bytes()
        files.append({
            "path": relative_path,
            "content": base64.b64encode(content).decode("ascii"),
        })

    package = {
        "schemaVersion": 1,
        "name": "hello-world",
        "version": args.version,
        "files": files,
    }
    output = json.dumps(package, sort_keys=True, separators=(",", ":")) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(output, encoding="utf-8", newline="\n")
    print(hashlib.sha256(output.encode("utf-8")).hexdigest())


if __name__ == "__main__":
    main()
