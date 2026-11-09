#!/usr/bin/env python3 -I
"""Regenerate library/manifest.toml (sha256 of every shipped file). usage: library-manifest.py LIBRARY_DIR VERSION"""
import sys, hashlib, os
root, version = sys.argv[1], sys.argv[2]
kinds = {"frames": "frame", "cards": "card", "templates": "template", "examples": "example", "regulations": "regulation"}
rows = []
for d, k in kinds.items():
    for f in sorted(os.listdir(os.path.join(root, d))):
        p = f"{d}/{f}"
        kk = "regulation_table" if (k == "regulation" and f.endswith(".toml")) else k
        rows.append((p, hashlib.sha256(open(os.path.join(root, p), "rb").read()).hexdigest(), kk))
rows.append(("defaults.toml", hashlib.sha256(open(os.path.join(root, "defaults.toml"), "rb").read()).hexdigest(), "defaults"))
out = f'version = "{version}"\nanonymized = true\nsearch = "none: substring only in P3; TF-IDF index is P5 (not neural)"\n'
for p, h, k in rows:
    out += f'\n[[file]]\npath = "{p}"\nsha256 = "{h}"\nkind = "{k}"\n'
open(os.path.join(root, "manifest.toml"), "w").write(out)
print(len(rows), "files")
