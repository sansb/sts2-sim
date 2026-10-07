# Performance

This file gives the speed and memory use of the engine. It also tells you how
to measure them on your computer.

There are two measurements:

- The benchmark fight is one small fight. It gives the maximum speed.
- The eval fights are 594 fights from real games. They give the usual speed.

## Results for the benchmark fight

| Item | Value |
|---|---|
| Speed | 1,049,000 transitions per second |
| Time for each transition | 953 nanoseconds |
| Full fights | 61,000 per second |
| Maximum memory | 5.9 MB |
| Memory allocations | 13.3 for each transition |
| Allocated memory | 887 bytes for each transition |

Conditions:

- Date: 2026-10-01.
- Computer: Apple M5 Max, macOS 26.6.
- Compiler: rustc 1.97.1, release build.
- Threads: one.
- Speed is the best of five runs. The slowest run was 0.9% slower.

The file `benchmarks/2026-10-01-perf-floor-baseline-v1.json` contains the full
data for each run.

## Results for the eval fights

The directory `../eval/fights` contains fights from real games. All of them
are from Ascension 9 or Ascension 10. The engine accepts 594 of the 597
fights. We measured the speed of each of these 594 fights.

| Fights | Speed (transitions per second) |
|---|---|
| Slowest 10% | 144,000 or less |
| Slowest 25% | 208,000 or less |
| Median | 302,000 |
| Fastest 25% | 570,000 or more |
| Fastest 10% | 786,000 or more |

| Type of fight | Number of fights | Median speed (transitions per second) |
|---|---|---|
| Monster | 359 | 329,000 |
| Elite | 142 | 294,000 |
| Boss | 93 | 253,000 |

Conditions:

- Date: 2026-10-06.
- The computer, the build, and the thread count are the same as above.
- Each fight ran for 0.25 seconds.
- A second measurement gave a median of 300,000.

32 fights are slower than 100,000 transitions per second. Nine of these
fights are slower than 15,000 transitions per second. The slowest fight is
slower than 200 transitions per second.

The file `benchmarks/2026-10-06-eval-fight-throughput.jsonl` contains the
result for each fight.

## What the benchmark measures

A transition is one action that the engine applies to a fight. Examples are
"play a card", "use a potion", and "end the turn".

The benchmark does these steps for each transition:

1. It finds all the legal actions.
2. It selects one action at random.
3. It makes a copy of the fight state.
4. It applies the action to the copy.

A search does the same steps. Thus the result is the speed that a search gets
on one thread.

The benchmark uses one fight: an Ironclad with a starter deck against two
Toadpoles. It plays this fight 240,000 times from start to end. One fight has
17 transitions on average. The total is 4,078,800 transitions.

The random selection uses a fixed seed. Thus each run does the same work. The
output contains a `checksum` of the actions. If two runs have different
checksums, do not compare their speeds.

## Limits of the result

- The benchmark fight is small. A fight with more cards, relics, and enemies
  is slower. Use the results for the eval fights to estimate the usual speed.
- The actions are random. A search selects different actions, and its speed
  can be different.
- The result is for one thread. The benchmark does not measure a search on
  many threads.
- The result is for the native build. The benchmark does not measure the WASM
  build.

## How to measure

Build the engine:

```bash
cargo build --release
```

Run the benchmark:

```bash
./target/release/sts-sim bench
```

The run takes approximately four seconds. The output is one JSON object. These
are the important fields:

| Field | Meaning |
|---|---|
| `timing.transitions_per_second` | Speed |
| `timing.nanos_per_transition` | Time for each transition |
| `timing.complete_playouts_per_second` | Full fights per second |
| `work.transitions` | Number of transitions in the run |
| `work.checksum` | Checksum of the actions |

Speed changes when the computer does other work. Do the run three to five
times and use the best result.

For a short run, decrease the number of fights:

```bash
./target/release/sts-sim bench --trajectories 10000
```

To measure the eval fights, build and run this example:

```bash
cargo build --release --example eval_throughput
```

```bash
./target/release/examples/eval_throughput ../eval/fights
```

The run takes approximately three minutes. The output has one JSON line for
each fight. The last line gives the median and the other percentiles.

To measure allocations, build with the counter:

```bash
cargo build --release --features allocation-counting
```

Then run the benchmark again. The `allocation` fields now contain values. This
build is slower, so do not use its speed.

## Regression check

The file `benchmarks/floors.json` contains a limit for each measurement.

| Measurement | Limit | Calculation |
|---|---|---|
| Speed | 891,866 transitions per second minimum | Result minus 15% |
| Maximum memory | 8,871,936 bytes maximum | Result plus 50% |
| Allocations for each transition | 14.64 maximum | Result plus 10% |
| Allocated bytes for each transition | 975.3 maximum | Result plus 10% |

This command does the full check:

```bash
python3 tools/perf_floor.py
```

The command builds the engine and does five timed runs. Then it does one run
with the allocation counter and compares the results with the limits. It uses
`/usr/bin/time -l`, so it operates only on macOS.

| Exit code | Meaning |
|---|---|
| 0 | All results are in the limits. |
| 1 | One or more results are not in the limits. |
| 2 | The measurement is not valid. The message gives the cause. |

The limits apply to the computer in "Results". A slower computer can fail the
speed limit when the engine has no defect.

Our CI does this check after each merge. Change a limit only when you have new
measurements. Put the new data file in `benchmarks/` with the change.
