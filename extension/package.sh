#!/usr/bin/env bash
# Crée les zips Chrome/Edge et Firefox dans dist/ (sans zip : python3 zipfile).
set -euo pipefail
cd "$(dirname "$0")"

VERSION=$(python3 -I -c 'import json;print(json.load(open("manifest.json"))["version"])')
mkdir -p dist

# Fichiers communs aux deux navigateurs
FILES=(common.js background.js content.js popup.html popup.css popup.js options.html options.css options.js picker.html picker.css picker.js ui.css)

python3 -I - "$VERSION" "${FILES[@]}" <<'PY'
import sys, zipfile
from pathlib import Path

version, files = sys.argv[1], sys.argv[2:]
extra = sorted(str(p) for d in ("_locales", "icons") for p in Path(d).rglob("*") if p.is_file())

def build(out, manifest):
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        z.write(manifest, "manifest.json")
        for f in files + extra:
            z.write(f, f)
    print(out)

build(f"dist/fluxdl-chrome-{version}.zip", "manifest.json")
build(f"dist/fluxdl-firefox-{version}.zip", "manifest.firefox.json")
PY
