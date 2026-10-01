# sts2-sim

Combat simulator and solver for Slay the Spire 2, from [Relay the Spire](https://relaythespire.com).

Not affiliated with or endorsed by Mega Crit. No game files included; some tooling requires you to have your own copy of sts2 on your machine.

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

The census replays captured fights (`.mcr` files, currently dumped from live games via the RelayTheSpire companion mod) through the release binary and compares sim actions with game actions. Census corpus  not included (only the test suite's own captures under `v0.111.0/python/testdata/` are); point it at your own:

```bash
python3 v0.111.0/engine/tools/eval_suite.py census --captures PATH_TO_CAPTURES
```

`--self-test` runs the decoder and tally checks without a corpus.

The sim currently replays 1,196 of 1,206 captured fights from my personal census correctly. If you find fights with invalid simulations, please send them my way via a GitHub issue or pull request!

## Notes

This repo is synced from the private monorepo for RelayTheSpire;
`SYNCED_FROM` names the commit the tree was exported from. PRs welcome against this repo; PRs that are accepted will be commited into the RelayTheSpire monorepo and synced back to this repo (for now).

Issue and PR numbers cited throughout (`#1282` etc.) refer to the RelayTheSpire monorepo.

## License

MIT. See [LICENSE](LICENSE).
