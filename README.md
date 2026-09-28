# sts2-sim

<!-- SEAN: one or two sentences in your own words: what this is and why you built it. -->

A combat simulator and solver for Slay the Spire 2, from [Relay the Spire](https://relaythespire.com).

Not affiliated with or endorsed by Mega Crit. No game files are included; tools that
read the game's `sts2.dll` expect you to point them at your own install.

## What's here

- `versions/v0.111.0/rust/`: the combat engine and exact solver (`sts-sim`), certified
  against the game's own `.mcr` replay checksums.
- `versions/v0.111.0/solver/`: the Python layer around it: `.run` parsing, RNG and map
  reconstruction, `.mcr` decoding, and replay/review adapters.
- `versions/v0.111.0/eval/`: the eval fixtures the certification census runs over.

Each tree is keyed to the game build it is certified against (v0.111.0).

## Build

```bash
cd versions/v0.111.0/rust
cargo build --release
```

## Certification census

The census replays captured fights (the game's `.mcr` files, recorded with the
Relay the Spire mod) through the release binary and checks every action against
the game's own checksums. The captures themselves are not included; point it at
your own:

```bash
python3 versions/v0.111.0/rust/tools/eval_suite.py census --captures PATH_TO_CAPTURES
```

`--self-test` runs the decoder and tally checks without a corpus.

<!-- SEAN: current certified count, how to contribute, where to talk about it (Discord?). -->

## Notes

Issue and PR numbers cited throughout (`#1282` etc.) refer to the private repo this
code was developed in; they are kept as a record, not as links.

## License

MIT. See [LICENSE](LICENSE).
