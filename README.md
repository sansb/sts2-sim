# sts2-sim

<!-- SEAN: one or two sentences in your own words: what this is and why you built it. -->

A combat simulator and solver for Slay the Spire 2, from [Relay the Spire](https://relaythespire.com).

Not affiliated with or endorsed by Mega Crit. No game files are included; tools that
read the game's `sts2.dll` expect you to point them at your own install.

## What's here

- `v0.111.0/engine/`: the combat engine and exact solver (`sts-sim`, Rust), certified
  against the game's own `.mcr` replay checksums.
- `v0.111.0/python/`: the Python layer around it: `.run` parsing, RNG and map
  reconstruction, `.mcr` decoding, and replay/review adapters.
- `v0.111.0/eval/`: the eval fixtures the certification census runs over.
- `builds.json`: the game builds the engine is admitted for.
- `dll-archive/`: hashes of each archived game build, and the script that archives
  your own install's files there (they are never committed).
- `meta/`: how builds are versioned, and what changed between them.

Each `vX.Y.Z/` tree is keyed to the game build it is certified against.

## Build

```bash
cd v0.111.0/engine
cargo build --release
```

## Certification census

The census replays captured fights (the game's `.mcr` files, recorded with the
Relay the Spire mod) through the release binary and checks every action against
the game's own checksums. The census corpus is not included (only the test suite's
own captures under `v0.111.0/python/testdata/` are); point it at your own:

```bash
python3 v0.111.0/engine/tools/eval_suite.py census --captures PATH_TO_CAPTURES
```

`--self-test` runs the decoder and tally checks without a corpus.

<!-- SEAN: current certified count, how to contribute, where to talk about it (Discord?). -->

## Notes

This repo is synced from the private monorepo Relay the Spire is developed in;
`SYNCED_FROM` names the commit the tree was exported from. Pull requests are
welcome here and are landed upstream, then arrive in the next sync.

Issue and PR numbers cited throughout (`#1282` etc.) refer to the private repo this
code was developed in; they are kept as a record, not as links.

## License

MIT. See [LICENSE](LICENSE).
