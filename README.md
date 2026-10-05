# sts2-sim

High-performance combat simulator and solver for Slay the Spire 2, from [Relay the Spire](https://relaythespire.com).

Not affiliated with or endorsed by Mega Crit. No game files included; some tooling requires you to have your own copy of sts2 on your machine.

## About

* Combat only. Simulates single combats from fully specified starting states. Does not currently simulate non-combat events.
* Optimized for search. High-throughput Rust-based engine implementation. up to ~1m transitions/core/second, ~100k t/c/s more typical (see [PERF.md](v0.111.0/engine/PERF.md))
* High fidelity with actual game, but not guaranteed. Please file issues if you find any divergence from actual game.

## Comparison with other sims

* https://github.com/iRyougi/sts2-sim - Full game sim in C#, 10x slower as of 2026-10-04
* [Spirebird](https://spirebird.com/sandbox.html) sim - Not currently open source

## Repo structure

Core sim/solver will be forked per game build (`vX.Y.Z/`).

- `v0.111.0/engine/`: the combat engine and solver code
- `v0.111.0/python/`: python tooling: `.run` parsing, RNG and map
  reconstruction, `.mcr` decoding, and replay/review adapters
- `v0.111.0/eval/`: eval fixtures
- `builds.json`: game build manifest
- `dll-archive/`: tools for archiving dlls locally
- `meta/`: versioning stuff

## Build

```bash
cd v0.111.0/engine
cargo build --release
```

## Certification census

The census replays captured fights (`.mcr` files, currently dumped from live games via the RelayTheSpire companion mod) through the release binary and compares sim actions with game actions.

The 597 eval fixtures under `v0.111.0/eval/fights/` each include the raw capture and its entry save (`capture.mcr.gz`, `entry.save.gz`), so you can re-run their certification against the game's own checksums from this repo:

```bash
python3 v0.111.0/engine/tools/eval_suite.py census --fixtures
```

That reproduces 594 certified of 597 fixtures (the other 3 are refusal fixtures), and exits non-zero if it doesn't. `v0.111.0/eval/README.md` has the details.

The sim currently replays 1,196 of 1,206 captured fights from my personal census correctly. That larger corpus is not in the repo; the fixtures are selected from it. To run the census over your own captures:

```bash
python3 v0.111.0/engine/tools/eval_suite.py census --captures PATH_TO_CAPTURES
```

`--self-test` runs the decoder and tally checks without a corpus.

If you find fights with invalid simulations, please send them my way via a GitHub issue or pull request!

## Notes

This repo is synced from the private monorepo for RelayTheSpire;
`SYNCED_FROM` names the commit the tree was exported from. PRs welcome against this repo; PRs that are accepted will be commited into the RelayTheSpire monorepo and synced back to this repo (for now).

Issue and PR numbers cited throughout (`#1282` etc.) refer to the RelayTheSpire monorepo.

## License

MIT. See [LICENSE](LICENSE).
