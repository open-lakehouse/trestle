#!/usr/bin/env python3
"""Redact local-development credentials from committed environment goldens."""

from pathlib import Path
import re
import sys


def redact(text: str) -> str:
    text = re.sub(
        r"(postgres(?:ql)?://)[^@\s\"']+@",
        r"\1<redacted>@",
        text,
    )
    return re.sub(r"AccountKey=[^;]+;", "AccountKey=<redacted>;", text)


def main() -> None:
    root = Path(sys.argv[1])
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        try:
            original = path.read_text()
        except UnicodeDecodeError:
            continue
        redacted = redact(original)
        if redacted != original:
            path.write_text(redacted)


if __name__ == "__main__":
    main()
