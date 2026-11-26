#!/usr/bin/env python3
"""Notices des dépendances Rust compilées dans fluxdl.exe (audit L16) → THIRD-PARTY.txt.

Usage : python3 -I packaging/third_party.py > THIRD-PARTY.txt (depuis la racine du dépôt).
Lit `cargo metadata` (cible Windows, dépendances normales seulement) et recopie le texte de
licence de chaque crate (LICENSE*, COPYING*, NOTICE*) trouvé dans son dossier source.
"""
import json
import os
import subprocess
import sys

meta = json.loads(subprocess.run(
    ["cargo", "metadata", "--format-version", "1", "--locked",
     "--filter-platform", "x86_64-pc-windows-gnu"],
    check=True, capture_output=True, text=True).stdout)
paquets = {p["id"]: p for p in meta["packages"]}
noeuds = {n["id"]: n for n in meta["resolve"]["nodes"]}
racine = meta["resolve"]["root"]

# Dépendances normales atteignables depuis l'exe (ni build ni dev).
vus, pile = set(), [racine]
while pile:
    i = pile.pop()
    if i in vus:
        continue
    vus.add(i)
    for d in noeuds[i]["deps"]:
        if any(k["kind"] is None for k in d["dep_kinds"]):
            pile.append(d["pkg"])
vus.discard(racine)

print("fluxdl — third-party notices\n")
print("fluxdl.exe includes the following Rust crates. Each one's license text follows.\n")
print("Fonts: Open Sans (SIL OFL 1.1, assets/fonts/OFL-OpenSans.txt), DejaVu Sans subset")
print("(Bitstream Vera license, assets/fonts/LICENSE-DejaVu.txt).\n")
deja = {}  # texte de licence → crate où il est recopié en entier
for p in sorted((paquets[i] for i in vus), key=lambda p: (p["name"], p["version"])):
    dossier = os.path.dirname(p["manifest_path"])
    print("=" * 78)
    print(f'{p["name"]} {p["version"]} — {p.get("license") or p.get("license_file") or "?"}')
    if p.get("repository"):
        print(p["repository"])
    textes = sorted(f for f in os.listdir(dossier)
                    if f.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE")))
    for f in textes:
        chemin = os.path.join(dossier, f)
        if os.path.isfile(chemin):
            with open(chemin, encoding="utf-8", errors="replace") as t:
                texte = t.read().rstrip()
            cle = " ".join(texte.split())
            if cle in deja:
                print(f"\n--- {f} : same text as {deja[cle]} ---")
                continue
            deja[cle] = f'{p["name"]} {p["version"]} ({f})'
            print(f"\n--- {f} ---\n")
            print(texte)
    if not textes:
        print("\n(no license file shipped with the crate; see the license expression above)")
    print()
sys.stderr.write(f"{len(vus)} crates\n")
