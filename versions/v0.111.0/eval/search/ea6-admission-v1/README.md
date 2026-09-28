# EA6YVY5X1QM1 early-fight admission

Captured v0.111.0 Necrobinder starts for seanb's floors 6 (Seapunk) and 7
(Sludge Spinner). Floor 6 was blocked solely by Clash, floor 7 by Clash/Parse.
The exact captured start saves supplied deck order, relic state, HP and RNG.

Source run: `dc8c0560-5599-4900-8264-d57894da322f`.
Floor 6 save SHA-256: `5367610e66beeba2ce3bc0fefcc32bab6e63d9ee723662d97ceaef74033b8930`.
Floor 7 save SHA-256: `8a9f1939fb5fc01865163031e0b4aa8f0e67cddabccf7d86a1ca29a5b75f914e`.
DLL: `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.

The recorded action fixtures contain 13 and 9 completed native checkpoints,
respectively. Native player resources, Osty HP/max HP, living enemies,
card identities/upgrades/pile order, and all nine RNG streams are compared
against Rust, independently of the Rust digest regression pins. End-turn
phase timing and complete native power/cost serialization are not certified
by this limited comparison. Recorded outcomes are wins at 36 and 29 HP.

Native target IDs are assigned in creature creation order, which need not
match the serialized roster order. These fixtures resolve their known targets;
they do not certify a general target-ID reconstruction for the site adapter.

Local 3-second bounded searches also found retained wins at 36 HP on both
floors. These are modeled outcomes, not claims of optimality or in-game
verification of a solver line. The entire run is not admitted by this slice:
generation composition and later enemy mechanics are separate work.

## Character potion generation

`floor31-skill-potion.json` isolates the captured first Skill Potion choice:
Necrobinder's native pool offers Pull Aggro at index 1. The integration test
compares resources, ordered piles and all nine RNG streams after selection.
It invokes the exact body directly; it does not claim full floor 31 admission.

`alchemize-native-factory.json` extracts the before/after PotionGeneration
states and generated potion from completed Alchemize checkpoints on floors
30, 31, 35 and 38 of the same uploaded recording. These certify the factory
result and RNG progression, not the surrounding combat trajectory. They
produce Skill Potion, Powdered Demise, Flex Potion and Gigantification Potion.

Bone Brew and Pot of Ghouls bodies have current-DLL-derived seam tests, not
native in-game checkpoint certification. Their tests cover existing/fresh
Osty, HP overflow refusal, full Hand overflow, generated history and power
callbacks. All-character typed card-potion pools and Orobic draw counts are
checked against the generated DLL facts. Random-potion factories preserve
owner-first/shared-second order and refuse whole-fight admission when an
owner's reachable factory still contains an unsupported potion body.

`floor20-abundance-native.json` contains the exact floor-20 start and the full
20-action recorded line, with 15 completed native checkpoints. It detects the
Abundance factory's exclusion of Ancient powers: choice 0 must yield Spirit of
Ash+, not Danse Macabre+. Checks cover HP/block/energy/turn, living enemies,
Osty, ordered card identities/upgrades and all nine RNG streams; powers and
cost serialization are outside this comparison. The final fight state is a
win at 50 HP. The recorded target on this fixture is the sole Tunneler; no
general MCR target-ID reconstruction is claimed here.

## Nested damage admission

`floor12.canonical.json` is the captured Sewer Clam opening, save SHA-256
`d2b977d8dfcbd139d6be58565536e757868e9dcc608f77d14be888b3229d3361`.
Its Abundance closure reaches Sleight of Flesh together with damage-given
powers. The root now serves as an admission and cold projection regression;
no full recorded native action trajectory is claimed for this fixture.

`floor24.canonical.json` is the captured Slumbering Beetle encounter start,
save SHA-256 `9f45d4a8d105708835e8fe24a39b31e270c358e1f53edd490e3bc332608c3b54`.
It is an admission regression for the native Beetle sleep/wake machine.
The uploaded bundle has no recorded action replay for this floor; native
trajectory certification is not claimed. Public seam tests independently pin
current-DLL damage countdown, natural expiry, Plating removal, Rollout,
post-damage intent and cold reload after each action.

## The six Entropy floors (#2637)

`floor30/31/35/38/39/40.canonical.json` are the captured starts for the six
EA6 fights that #2637's five-class Entropy closure is about — the ones the
Entropy draft claims now admit. They are committed because the claim was
previously unverifiable from the repo: an independent review of PR #2724 could
establish neither the admission nor the closure sizes, because the roots lived
only in a scratch directory.

Same source run as the rest of this directory
(`dc8c0560-5599-4900-8264-d57894da322f`, EA6YVY5X1QM1, seanb's own run), same
DLL (`9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`).
Encounters and per-floor start-save SHA-256:

| floor | encounter | start save SHA-256 |
|---:|---|---|
| 30 | `CHOMPERS_NORMAL` | `215a239797a3b55371196a2ab6c08ebe145095cde3055f63f8e844d2b8171218` |
| 31 | `BOWLBUGS_NORMAL` | `4e8a18b8314eae3be6d2cc0db7bad17aa19afbb9bd8b549351912ea258654068` |
| 35 | `DEVOTED_SCULPTOR_WEAK` | `423bc57d9f19bb5209067f0bc7518985f4ee4cc1f441be88f8a91814113cad3c` |
| 38 | `TURRET_OPERATOR_WEAK` | `0e1c86b06ca78e4e47d3a0b09d0fa9788ded120449f1bccaa796af74d27d1256` |
| 39 | `SCROLLS_OF_BITING_NORMAL` | `86e459527d77fb1017c9f42068bc0cdade4a911f74b5965286a5c047edca1fbf` |
| 40 | `KNIGHTS_ELITE` | `1a51b05e23e9bb4de959209def71019e329efe40deb028b5a750059461dfb4c9` |

`floor33.canonical.json` joined them with #2654 (`fight_index` 12, node index
32), built the same way from the same run. It was the one floor in #2637's
scope that still refused after the Entropy landing — at the canonical boundary
on `player.kaiser_facing`, then on the `Kaiser Crab Surrounded lifecycle`
admission gate — and it is pinned by its own test rather than by the six-floor
loop because it belongs to a different slice.

| floor | encounter | start save SHA-256 |
|---:|---|---|
| 33 | `KAISER_CRAB_BOSS` | `4acade722bd126846694223056883f7bfe645ae8a2ac9957c6196a9d8d88930e` |

### How they were regenerated

`versions/v0.111.0/solver/rust_review.py`'s `build_root(run, fight_index,
saves)` — the same entry point the site's review pipeline uses — over the
uploaded `.run` and that floor's `start` snapshot, with the checkout's current
projector. Roots regenerated this way are byte-identical to the set the
2026-09-22 frontier measurement and PR #2724's cycle-2 measurement used, which
is how they were checked before committing. A root must be regenerated whenever
the projection adds a wire field (`/tmp/EA6-floorN-root.json` artifacts from
before #2697 are stale for exactly that reason).

### What they certify, and what they do not

`tests/ea6_review.rs::the_six_entropy_floors_admit_with_zero_refusals_and_pinned_closures`
asserts, per floor: `engine::admit` returns `Ok(())` with zero refusals; the
cold projection round-trips; and the interned closure is exactly

| floor | specs | distinct ids |
|---:|---:|---:|
| 30, 31 | 941 | 471 |
| 35 | 944 | 472 |
| 38, 39 | 945 | 473 |
| 40 | 947 | 473 |

No recorded native action line exists for these six floors in the uploaded
bundle, so nothing here is replay-certified. Admission means the gate found
nothing unmodeled; it is not a correctness certificate, and the five-class
Entropy transform still has no replay witness on any floor.
