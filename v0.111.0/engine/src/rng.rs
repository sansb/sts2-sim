//! STS2 xoshiro256** and build-dependent stream seeding.

use serde::{Deserialize, Serialize};

const XXP1: u64 = 0x9E37_79B1_85EB_CA87;
const XXP2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const XXP3: u64 = 0x1656_67B1_9E37_79F9;
const XXP4: u64 = 0x85EB_CA77_C2B2_AE63;
const XXP5: u64 = 0x27D4_EB2F_1656_67C5;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Xoshiro256StarStar {
    pub words: [u64; 4],
    pub counter: u64,
}

impl Xoshiro256StarStar {
    pub fn from_seed(seed: u64) -> Self {
        let mut splitmix = seed;
        let mut words = [0; 4];
        for word in &mut words {
            (*word, splitmix) = splitmix64_next(splitmix);
        }
        Self { words, counter: 0 }
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.words[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.words[1] << 17;
        self.words[2] ^= self.words[0];
        self.words[3] ^= self.words[1];
        self.words[1] ^= self.words[2];
        self.words[0] ^= self.words[3];
        self.words[2] ^= t;
        self.words[3] = self.words[3].rotate_left(45);
        self.counter += 1;
        result
    }

    pub fn next_i31(&mut self) -> i32 {
        (self.next_u64() >> 33) as i32
    }

    pub fn next_double(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) * 1.110_223_024_625_156_5e-16
    }

    /// Native `Rng.NextFloat(max)`: scale one 53-bit `NextDouble` result,
    /// then round the result to IEEE-754 binary32.
    pub fn next_float(&mut self, max: f32) -> f32 {
        (self.next_double() * f64::from(max)) as f32
    }

    pub fn next_bounded(&mut self, max: i32) -> Result<i32, &'static str> {
        if max < 1 {
            return Err("maxValue must be > 0");
        }
        Ok((self.next_double() * f64::from(max)) as i32)
    }

    /// Native `ListExtensions.UnstableShuffle`: one descending Fisher-Yates
    /// pass, consuming exactly `len - 1` draws.
    pub fn shuffle<T>(&mut self, values: &mut [T]) -> Result<(), &'static str> {
        for index in (1..values.len()).rev() {
            let bound = i32::try_from(index + 1).map_err(|_| "shuffle bound overflow")?;
            let picked = self.next_bounded(bound)?;
            values.swap(picked as usize, index);
        }
        Ok(())
    }
}

fn splitmix64_next(state: u64) -> (u64, u64) {
    let state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31), state)
}

pub fn deterministic_hash_v108(value: &str) -> i32 {
    let mut h1: u32 = 352_654_597;
    let mut h2: u32 = 352_654_597;
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        h1 = h1.wrapping_mul(33) ^ chars[i] as u32;
        if i == chars.len() - 1 {
            break;
        }
        h2 = h2.wrapping_mul(33) ^ chars[i + 1] as u32;
        i += 2;
    }
    h1.wrapping_add(h2.wrapping_mul(1_566_083_941)) as i32
}

fn xx_round(mut acc: u64, input: u64) -> u64 {
    acc = acc.wrapping_add(input.wrapping_mul(XXP2));
    acc = acc.rotate_left(31);
    acc.wrapping_mul(XXP1)
}

fn xx_merge_round(mut hash: u64, value: u64) -> u64 {
    hash ^= xx_round(0, value);
    hash.wrapping_mul(XXP1).wrapping_add(XXP4)
}

pub fn xxhash64(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    let mut index = 0;
    let mut hash = if len >= 32 {
        let mut v1 = seed.wrapping_add(XXP1).wrapping_add(XXP2);
        let mut v2 = seed.wrapping_add(XXP2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(XXP1);
        while index <= len - 32 {
            v1 = xx_round(
                v1,
                u64::from_le_bytes(data[index..index + 8].try_into().unwrap()),
            );
            v2 = xx_round(
                v2,
                u64::from_le_bytes(data[index + 8..index + 16].try_into().unwrap()),
            );
            v3 = xx_round(
                v3,
                u64::from_le_bytes(data[index + 16..index + 24].try_into().unwrap()),
            );
            v4 = xx_round(
                v4,
                u64::from_le_bytes(data[index + 24..index + 32].try_into().unwrap()),
            );
            index += 32;
        }
        let mut h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        h = xx_merge_round(h, v1);
        h = xx_merge_round(h, v2);
        h = xx_merge_round(h, v3);
        xx_merge_round(h, v4)
    } else {
        seed.wrapping_add(XXP5)
    };
    hash = hash.wrapping_add(len as u64);
    while index + 8 <= len {
        let word = u64::from_le_bytes(data[index..index + 8].try_into().unwrap());
        hash ^= xx_round(0, word);
        hash = hash.rotate_left(27).wrapping_mul(XXP1).wrapping_add(XXP4);
        index += 8;
    }
    if index + 4 <= len {
        let word = u32::from_le_bytes(data[index..index + 4].try_into().unwrap()) as u64;
        hash ^= word.wrapping_mul(XXP1);
        hash = hash.rotate_left(23).wrapping_mul(XXP2).wrapping_add(XXP3);
        index += 4;
    }
    while index < len {
        hash ^= (data[index] as u64).wrapping_mul(XXP5);
        hash = hash.rotate_left(11).wrapping_mul(XXP1);
        index += 1;
    }
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(XXP2);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(XXP3);
    hash ^ (hash >> 32)
}

pub fn deterministic_hash_v109(value: &str) -> u64 {
    xxhash64(value.as_bytes(), 0)
}

pub fn snake_case(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 4);
    for (index, character) in value.trim().chars().enumerate() {
        if index > 0 && character.is_ascii_uppercase() {
            output.push('_');
        }
        output.push(character.to_ascii_lowercase());
    }
    output
}

pub fn stream_seed(build: &str, run_seed: &str, stream: &str) -> Result<u64, String> {
    let name = snake_case(stream);
    match build {
        "v0.108.0" => {
            let set = deterministic_hash_v108(run_seed) as u32;
            let stream_hash = deterministic_hash_v108(&name) as u32;
            Ok(set.wrapping_add(stream_hash) as u64)
        }
        "v0.109.0" | "v0.109.1" | "v0.110.1" => {
            let set = if let Some(old) = run_seed.strip_prefix("old") {
                deterministic_hash_v108(old) as u32 as u64
            } else {
                deterministic_hash_v109(run_seed)
            };
            Ok(set.wrapping_add(deterministic_hash_v109(&name)))
        }
        _ => Err(format!("unverified RNG build {build}")),
    }
}

/// `RunRngSet.Seed` for a v0.109-scheme build (which v0.110.1 and v0.111.0
/// both are; `sts2_rng.SEEDING_SCHEME`).
///
/// `RunRngSet::.ctor` (RVA `0x50f70`) hashes the run seed string with
/// XxHash64, except for the legacy escape: a seed string prefixed `"old"`
/// hashes the **remainder** with the old djb2 and zero-extends it to `u64` via
/// CIL `conv.u8`.
///
/// This is deliberately a *different* function from [`stream_seed`], which has
/// no `v0.111.0` arm and no callers: the nine combat streams are **read** from
/// the save's recorded words rather than derived (`entry/counters.rs`), and
/// only the per-fight `Encounter` stream — which the save never records — is
/// derived. Giving `stream_seed` a `v0.111.0` arm here would quietly reopen
/// the derivation path that module's measurement retired.
pub fn run_set_seed_v109(run_seed: &str) -> u64 {
    match run_seed.strip_prefix("old") {
        Some(rest) => u64::from(deterministic_hash_v108(rest) as u32),
        None => deterministic_hash_v109(run_seed),
    }
}

/// One run stream at counter 0, derived the way `RunRngSet` seeds it.
///
/// `Rng::.ctor(seed, string name)` (RVA `0x61be9`) is `seed + hash(name)`,
/// where a v0.109-scheme build hashes the **snake_case** stream name with
/// XxHash64 and adds at 64 bits. `RunRngSet::.ctor` passes
/// `SnakeCase(enum.ToString())`, which is exactly the key a schema-20 save
/// writes, so `stream` is passed through unchanged.
///
/// # When this is used, and why it is not [`stream_seed`]
///
/// The nine combat streams are **read** from the save, not derived
/// (`entry/counters.rs`): a schema >= 19 save records each one's counter and
/// four words, and the derivation was checked against them on 3,092 saves with
/// zero disagreements. This function exists for the one case a save cannot
/// answer — an **optional** combat stream the save omits entirely.
/// `start_combat` still constructs it (`Rng(rs["CombatOrbs"].seed,
/// counter=combat_orb_generation_counter or 0)`, frozen Python, deleted #2827), so
/// its four words are in the projected document and cannot be defaulted to
/// zero.
///
/// No corpus save reaches it — all 3,092 record all twelve streams — but the
/// checked-in synthetic fixture omits `combat_orbs`, and inventing a refusal
/// the oracle does not have would break entry parity.
///
/// [`stream_seed`] is deliberately left alone, still with no `v0.111.0` arm
/// and no callers: it takes a build string, and a build-keyed seeding switch
/// is exactly the surface #1265 narrowed.
pub fn optional_combat_stream_at_zero(run_seed: &str, stream: &str) -> Xoshiro256StarStar {
    run_stream_at_zero(run_seed, stream)
}

/// Any run stream at counter 0: `RunRngSet::CreateRng` (v0.111.0 RVA
/// `0x4df00`) passes `SnakeCase(type.ToString())` and `Seed` to
/// `Rng::.ctor(ulong, string)` (`0x5eadd`), which adds
/// `GetDeterministicHashCode(name)` (`IL_0004`-`IL_0009`); `Seed` is
/// [`run_set_seed_v109`] (`RunRngSet::.ctor` `0x4de0c`, `IL_0024`-`IL_0067`).
///
/// The one reader besides [`optional_combat_stream_at_zero`] is the entry-facts
/// input (`entry/facts.rs`), which uses it to **check** the stream words a
/// facts document records, never to supply them. That is the same cross-check
/// `live_coach.verify_stream_seeding` runs over saves.
pub fn run_stream_at_zero(run_seed: &str, stream: &str) -> Xoshiro256StarStar {
    let seed = run_set_seed_v109(run_seed).wrapping_add(deterministic_hash_v109(stream));
    Xoshiro256StarStar::from_seed(seed)
}

/// The per-fight `Encounter` stream, at counter 0.
///
/// `EncounterModel::GenerateMonstersWithSlots` (v0.111.0 RVA `0x7f88c`) seeds
/// it as `RunRngSet.Seed + (long)RunState.TotalFloor +
/// StringHelper.GetDeterministicHashCode(Id.Entry)`, all three addends 64-bit
/// and unchecked, then hands the sum to `Rng::.ctor(UInt64)` (`0x61b81`). It is
/// not one of the named run streams and is not persisted, so it needs no
/// cross-fight counter accounting.
///
/// Oracle: `sts2_rng.Rng.for_encounter`, v0.109 branch. `entry` is the
/// `ModelId.Entry` half — `"SLIMES_WEAK"`, never `"ENCOUNTER.SLIMES_WEAK"`.
pub fn encounter_stream(run_set_seed: u64, total_floor: i64, entry: &str) -> Xoshiro256StarStar {
    let seed = run_set_seed
        .wrapping_add(total_floor as u64)
        .wrapping_add(deterministic_hash_v109(entry));
    Xoshiro256StarStar::from_seed(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors printed by the oracle, not re-derived from the same expression.
    ///
    /// ```text
    /// python3.12 -c "import sts2_rng as r; b='v0.111.0'
    /// rs = r.RunRngSet('ZPJHU3WSH2', build=b); print(rs.seed)
    /// e = r.Rng.for_encounter(rs.seed, 3, 'SLIMES_WEAK', build=b)
    /// print(e._random.s0, e._random.s1, e._random.s2, e._random.s3)
    /// print(e.next_int(0, 100))
    /// print(r.RunRngSet('oldZPJHU3WSH2', build=b).seed)"
    /// ```
    #[test]
    fn the_encounter_stream_matches_the_python_for_encounter_vectors() {
        let set_seed = run_set_seed_v109("ZPJHU3WSH2");
        assert_eq!(set_seed, 11_137_324_208_666_220_282);
        let mut stream = encounter_stream(set_seed, 3, "SLIMES_WEAK");
        assert_eq!(
            stream.words,
            [
                4_193_509_537_493_288_423,
                2_041_592_976_470_635_983,
                8_125_862_189_240_511_560,
                6_828_330_711_433_557_389,
            ]
        );
        assert_eq!(stream.counter, 0);
        assert_eq!(stream.next_bounded(100).unwrap(), 48);
    }

    #[test]
    fn the_old_prefix_escape_hashes_the_remainder_with_djb2() {
        assert_eq!(run_set_seed_v109("oldZPJHU3WSH2"), 1_584_024_330);
    }

    /// Also printed by the oracle, and independently the four words the
    /// Python-rooted Toadpoles document carries for `combat_orbs` — the one
    /// stream `fixtures/entry_builder_v1.json`'s synthetic save omits.
    ///
    /// ```text
    /// python3.12 -c "import sts2_rng as r
    /// g = r.RunRngSet('ZPJHU3WSH2', build='v0.111.0')['CombatOrbs']
    /// print(g.seed, g._random.s0, g._random.s1, g._random.s2, g._random.s3)"
    /// ```
    #[test]
    fn an_omitted_optional_stream_derives_to_the_oracles_words() {
        let stream = optional_combat_stream_at_zero("ZPJHU3WSH2", "combat_orbs");
        assert_eq!(
            stream.words,
            [
                16_452_943_454_101_452_638,
                5_525_124_908_612_069_843,
                10_373_153_506_651_793_454,
                1_658_981_437_511_799_834,
            ]
        );
        assert_eq!(stream.counter, 0);
    }

    #[test]
    fn xoshiro_reference_vectors_match_independent_c_corpus() {
        let expected = [
            11_091_344_671_253_066_420,
            13_793_997_310_169_335_082,
            1_900_383_378_846_508_768,
            7_684_712_102_626_143_532,
            13_521_403_990_117_723_737,
        ];
        let mut rng = Xoshiro256StarStar::from_seed(0);
        assert_eq!(expected.map(|_| rng.next_u64()), expected);
        assert_eq!(rng.counter, 5);
    }

    #[test]
    fn next_float_matches_the_python_binary32_rounding_pin() {
        let mut rng = Xoshiro256StarStar::from_seed(0);
        assert_eq!(rng.next_float(2.0).to_bits(), 1.202_526_f32.to_bits());
        assert_eq!(rng.counter, 1);
    }

    #[test]
    fn hash_and_stream_vectors_match_python_and_engine_pins() {
        assert_eq!(deterministic_hash_v108("ZPJHU3WSH2"), 1_584_024_330);
        assert_eq!(deterministic_hash_v109(""), 0xEF46_DB37_51D8_E999);
        assert_eq!(snake_case("CombatCardGeneration"), "combat_card_generation");
        assert_eq!(
            stream_seed("v0.108.0", "ZPJHU3WSH2", "Shuffle").unwrap(),
            3_892_305_005
        );
    }

    #[test]
    fn shuffle_consumes_one_draw_per_nonfinal_slot() {
        let mut rng = Xoshiro256StarStar::from_seed(0);
        let mut values = [0, 1, 2, 3, 4];
        rng.shuffle(&mut values).unwrap();
        assert_eq!(values, [1, 4, 0, 2, 3]);
        assert_eq!(rng.counter, 4);
    }
}
