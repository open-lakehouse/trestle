#!/usr/bin/env python3
"""Scrub raw Neon CLI captures into committable fixtures.

    scrub.py <raw-dir> <fixture-dir>

Every file in <raw-dir> is copied to <fixture-dir> with secrets and personal data replaced by
stable placeholders: the same raw value always maps to the same placeholder, so fixtures stay
consistent with each other (a credential's token id in `credentials create` matches the one in
`credentials list`). What is replaced:

- passwords inside connection URIs and `password` fields
- `api_token`, `s3_secret_access_key` and every `nak_live_…` credential id
- the account's login, names, e-mail addresses and avatar URLs

Files are kept byte-for-byte otherwise, including the non-JSON text some `--output json` verbs
print, because a fake replays them as the CLI printed them.
"""

import json
import pathlib
import re
import sys

SKIP = {"s3env.sh", "obj.txt"}

placeholders: dict[str, str] = {}


def placeholder(kind: str, value: str) -> str:
    if value not in placeholders:
        n = sum(1 for v in placeholders.values() if v.startswith(f"<{kind}"))
        placeholders[value] = f"<{kind}-{n + 1}>"
    return placeholders[value]


def token(value: str) -> str:
    """A credential id: keeps the `nak_live_` shape (32 hex) so parsers still accept it."""
    if value not in placeholders:
        n = sum(1 for v in placeholders.values() if v.startswith("nak_live_"))
        placeholders[value] = f"nak_live_{n + 1:032x}"
    return placeholders[value]


def collect_personal(raw: pathlib.Path) -> list[str]:
    """The account's identifying strings, read from `neon me`."""
    me = raw / "me.json"
    if not me.exists():
        return []
    data = json.loads(me.read_text())
    values = set()
    for obj in [data, *data.get("auth_accounts", [])]:
        for key in ("login", "email", "name", "last_name", "image", "id"):
            v = obj.get(key)
            if isinstance(v, str) and len(v) >= 3:
                values.add(v)
    return sorted(values, key=len, reverse=True)


def scrub(text: str, personal: list[str]) -> str:
    # Passwords in URIs: the part between `<role>:` and `@<host>`.
    text = re.sub(
        r"(://[^:/\s\"]+:)([^@\s\"]+)(@)",
        lambda m: m.group(1) + placeholder("password", m.group(2)) + m.group(3),
        text,
    )
    for key in ("password", "api_token", "s3_secret_access_key"):
        text = re.sub(
            rf'("{key}"\s*:\s*")([^"]+)(")',
            lambda m, k=key: m.group(1) + placeholder(k.replace("_", "-"), m.group(2)) + m.group(3),
            text,
        )
    text = re.sub(r"nak_live_[0-9a-f]{32}", lambda m: token(m.group(0)), text)
    # The short form of a token id is its first 12 hex digits.
    for raw_id, fake in list(placeholders.items()):
        if raw_id.startswith("nak_live_"):
            text = text.replace(f'"{raw_id[9:21]}"', f'"{fake[9:21]}"')
    for value in personal:
        text = text.replace(value, placeholder("personal", value))
    return text


def main() -> None:
    raw, out = map(pathlib.Path, sys.argv[1:3])
    out.mkdir(parents=True, exist_ok=True)
    personal = collect_personal(raw)
    for path in sorted(raw.iterdir()):
        if path.name in SKIP or not path.is_file():
            continue
        (out / path.name).write_text(scrub(path.read_text(), personal))
    print(f"scrubbed {len(list(out.iterdir()))} files into {out}")


if __name__ == "__main__":
    main()
