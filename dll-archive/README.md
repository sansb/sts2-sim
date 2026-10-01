# DLL archive — game version snapshots

Local-only archive of the game's managed assemblies, one directory per
game version (`vX.Y.Z/`, matching `release_info.json`). Contents are
gitignored (about 106 MB per version) — only this README is tracked.

**Archive on EVERY game version bump, immediately, before anything
else.**

This is now checked rather than remembered: `dll archive check` runs daily on
the self-hosted runner and fails loudly if the installed build is unarchived,
or if an archived version string's installed `sts2.dll` no longer matches
(a depot rebuilt under an unchanged version is a different build). The
machine-readable record is `index.json` — version, sha256, release date, and
commit per build; `sim/v0.111.0/python/test_dll_archive_index.py` pins
that every admitted build is archived, because admission promises the DLL is
available for re-verification (I11).

Archive with the tool rather than by hand — it verifies the copy's hash and
writes the index entry:

```sh
python3 sim/dll-archive/archive_build.py --archive   # from the MAIN checkout
```
 Steam auto-updates in place and deletes the old build; v0.108
is unrecoverable because nobody archived it, which is why all v0.108
IL reads can never be re-verified against their source (see #309).

To archive the current install:

```sh
SRC="$HOME/Library/Application Support/Steam/steamapps/common/Slay the Spire 2/SlayTheSpire2.app/Contents/Resources"
VER=$(python3 -c "import json;print(json.load(open('$SRC/release_info.json'))['version'])")
DEST="sim/dll-archive/$VER"
mkdir -p "$DEST"
cp -R "$SRC/data_sts2_macos_arm64" "$DEST/"
cp "$SRC/release_info.json" "$DEST/"
shasum -a 256 "$SRC/data_sts2_macos_arm64/sts2.dll" "$DEST/data_sts2_macos_arm64/sts2.dll"
```

Run it from the MAIN repo checkout, not a disposable worktree — a
gitignored copy inside a worktree is deleted when the worktree is
cleaned up.

Archived versions:

- `v0.103.3/` — archived 2026-09-26 UTC (commit 460a0ece, built
  2026-05-29T13:36:05-07:00, main assembly hash 418053415,
  `release_info.json` sha256
  `2f3888abf6061dd97987f6970b217fb4766765c9bdd13ca69a373141361d9de1`,
  `sts2.dll` sha256
  `348523fa6a3dccbc0635d1068d306ca9549785c398eefbbda0c9b5219a6548a0`).
  Recovered retroactively, not from an install: the Steam console's
  `download_depot 2868840 2868842 4577611564181546473` fetched the
  public-branch macOS depot manifest SteamDB first saw 2026-05-29, and
  `archive_build.archive()` was pointed at that download. The ARM64 runtime
  subtree is 203 files and 104,339,264 bytes. It was the PUBLIC build while
  public-beta ran v0.104-v0.106.1. Those beta builds are what most
  late-May/early-June runs carry, and they are not recoverable this way:
  `download_depot` cannot name a branch, and the v0.106.1 beta manifest
  5855589493078469321 fails with "No connection". The same console path
  re-fetched v0.107.1's public manifest byte-identical to its archive below,
  which is the control for this method. No version-impact comparison was run;
  nothing is certified or admitted against this build.
- `v0.107.1/` — archived 2026-08-05 (commit 59260271, built
  2026-06-18T15:43:56-07:00, main assembly hash -1718063421, `sts2.dll`
  sha256
  `e7ceb80669bfaf5c8fccabaa126ae2bb283aba514be5b5b55612579cfd285f18`).
  Archived from the installed Steam default-branch build (public branch
  build 23811903, macOS depot 2868842 manifest 8653035385353091849) for
  #974 (mainline v0.107.1 support). Contains `data_sts2_macos_arm64/` +
  `release_info.json`, 105 MB.
- `v0.109.0/` — archived 2026-07-16 (commit c12f634d, sts2.dll sha256
  06c78d946ca70658e85abb28f6dc2ee0a023a4467faf0708ff542180fe5f4c82)
- `v0.109.1/` — archived 2026-07-26 (commit c8c577f6, sts2.dll sha256
  2cb39e2eee651743829abcc0df4dd9cd7e65f46287c7ca264481115c9602382f).
  The #625 version-impact check found the same 204-file release layout and
  zero managed type, field, method-signature, or CIL-body changes from
  v0.109.0; only `sts2.dll` differed.
- `v0.110.1/` — archived 2026-07-31 (commit db5d3552, main assembly hash
  348485714, `sts2.dll` sha256
  `5a8fb7eb62510a86fd03653b9210cd8f67b511b632331a1f174042de39c92bd9`).
  Installed and archived `release_info.json` files were byte-identical; the
  exact macOS ARM64 runtime subtrees were also byte-for-byte equal at 206
  files and 110,826,139 bytes. The managed assembly changed materially
  from v0.109.1: TypeDef 9,627 -> 9,737, Field 41,144 -> 41,563,
  MethodDef 50,854 -> 51,443, and parsed RVA bodies 50,080 -> 50,669
  (zero parse errors). A signature/token-normalized comparison recorded
  182 removed / 292 added types, 1,137 removed / 1,556 added fields,
  654 removed / 1,243 added methods, and 1,013 changed method bodies.
  The 50,080-body baseline is not comparable to #625's 43,794-body figure:
  #625 used its earlier per-type hash scanner, while Batch 202's normalized
  scanner enumerates the complete nonzero-RVA MethodDef universe. Within the
  Batch 202 scanner the like-for-like comparison is 50,080 -> 50,669, with
  zero parse errors on both assemblies.
  Batch 202 / #793 classified the behavior-bearing changes under #794 and
  its child issues; unlike the v0.109.1 point release, this archive must not
  be treated as behaviorally interchangeable with its predecessor.
- `v0.111.0/` — archived 2026-08-13 (commit 41cef1ea, built
  2026-08-13T17:39:18-07:00, main assembly hash 1172974615,
  `release_info.json` sha256
  `97c0cef5a032ff6ed0826c161f5b72778753e9645c3f6dfe9503b5c3d03a8530`,
  `sts2.dll` sha256
  `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`).
  Installed/archive release metadata and DLL are byte-identical; both runtime
  subtrees contain 206 files and 110,898,550 bytes. Batch 293 / #1214's
  normalized comparison covered TypeDef 9,737 -> 9,760, Field 41,563 ->
  41,712, MethodDef 51,443 -> 51,602, and parsed RVA bodies 50,669 ->
  50,816 with zero parse errors. It classified 207 removed / 230 added types,
  1,107 removed / 1,256 added fields, 746 removed / 905 added methods, and
  744 removed / 891 added / 1,842 changed normalized bodies. Current-build
  display-value extraction enumerated 596 cards, 221 relics, 48 potions,
  5 enchantments, and 596 color rows. It found 15 changed card rows, Regalite
  Block 6 -> 4, and Inky's removed Damage 1; the exact distinct build is
  shipped through `data/game_values.json`. Batches 294-298 completed the card,
  encounter, RNG/relic, Inky, and shared-combat remodels under
  #1215/#1216/#1218/#1222/#1217. Exact v0.111.0 combat is admitted with the
  archive remaining its immutable provenance source.
