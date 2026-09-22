#!/usr/bin/env python3
"""Check local Markdown links and the committed synthetic fixture checksum."""
import hashlib
import pathlib
import re
import urllib.parse

root = pathlib.Path(__file__).resolve().parents[1]
errors = []
for path in root.rglob("*.md"):
    if any(part in ("target", ".git") for part in path.relative_to(root).parts):
        continue
    for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", path.read_text()):
        target = target.split("#", 1)[0]
        if not target or urllib.parse.urlparse(target).scheme:
            continue
        if not (path.parent / urllib.parse.unquote(target)).exists():
            errors.append(f"{path.relative_to(root)}: missing {target}")
expected = (root / "fixtures/SHA256SUMS").read_text().split()[0]
actual = hashlib.sha256((root / "fixtures/contract.bin.gz").read_bytes()).hexdigest()
if actual != expected:
    errors.append("fixture checksum changed; review and update provenance")
if errors:
    raise SystemExit("\n".join(errors))
print("Local documentation links and fixture checksum pass.")
