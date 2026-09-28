/* Canonical xoshiro256** + splitmix64 (Blackman/Vigna, public domain),
 * plus the .NET deterministic string hash, used to generate independent
 * test vectors for the Python port. Prints JSON to stdout.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

static uint64_t rotl(const uint64_t x, int k) {
    return (x << k) | (x >> (64 - k));
}

static uint64_t sm_state;
static uint64_t splitmix64(void) {
    uint64_t z = (sm_state += 0x9e3779b97f4a7c15ULL);
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return z ^ (z >> 31);
}

static uint64_t s[4];
static uint64_t next(void) {
    const uint64_t result = rotl(s[1] * 5, 7) * 9;
    const uint64_t t = s[1] << 17;
    s[2] ^= s[0]; s[3] ^= s[1]; s[1] ^= s[2]; s[0] ^= s[3];
    s[2] ^= t;
    s[3] = rotl(s[3], 45);
    return result;
}

static int32_t det_hash(const char *str) {
    int32_t h1 = 352654597, h2 = 352654597;
    size_t n = strlen(str);
    for (size_t i = 0; i < n; i += 2) {
        h1 = (int32_t)(((uint32_t)h1 << 5) + (uint32_t)h1) ^ str[i];
        if (i == n - 1) break;
        h2 = (int32_t)(((uint32_t)h2 << 5) + (uint32_t)h2) ^ str[i + 1];
    }
    return (int32_t)((uint32_t)h1 + (uint32_t)h2 * 1566083941u);
}

static void dump_seed(uint64_t seed) {
    sm_state = seed;
    for (int i = 0; i < 4; i++) s[i] = splitmix64();
    printf("  \"%llu\": {\"state\": [%llu, %llu, %llu, %llu], \"next\": [",
           (unsigned long long)seed,
           (unsigned long long)s[0], (unsigned long long)s[1],
           (unsigned long long)s[2], (unsigned long long)s[3]);
    for (int i = 0; i < 12; i++)
        printf("%s%llu", i ? ", " : "", (unsigned long long)next());
    printf("], \"doubles\": [");
    /* re-seed and emit .NET-style doubles and Next(max) draws */
    sm_state = seed;
    for (int i = 0; i < 4; i++) s[i] = splitmix64();
    for (int i = 0; i < 6; i++) {
        double d = (double)(next() >> 11) * 1.1102230246251565e-16;
        printf("%s%.17g", i ? ", " : "", d);
    }
    printf("], \"next100\": [");
    sm_state = seed;
    for (int i = 0; i < 4; i++) s[i] = splitmix64();
    for (int i = 0; i < 12; i++) {
        double d = (double)(next() >> 11) * 1.1102230246251565e-16;
        printf("%s%d", i ? ", " : "", (int)(d * 100));
    }
    printf("]}");
}

int main(void) {
    uint64_t seeds[] = {0ULL, 1ULL, 42ULL, 0xFFFFFFFFULL,
                        1234567890ULL, 0xDEADBEEFULL};
    printf("{\n\"seeds\": {\n");
    for (size_t i = 0; i < sizeof(seeds) / sizeof(*seeds); i++) {
        dump_seed(seeds[i]);
        printf(i + 1 < sizeof(seeds) / sizeof(*seeds) ? ",\n" : "\n");
    }
    printf("},\n\"hashes\": {\n");
    const char *strs[] = {"ZPJHU3WSH2", "shuffle", "up_front",
                          "combat_card_generation", "monster_ai", "niche",
                          "rewards", "shops", "transformations",
                          "unknown_map_point", "combat_potion_generation",
                          "combat_card_selection", "combat_energy_costs",
                          "combat_targets", "combat_orbs",
                          "treasure_room_relics", "", "a", "AB"};
    for (size_t i = 0; i < sizeof(strs) / sizeof(*strs); i++)
        printf("  \"%s\": %d%s\n", strs[i], det_hash(strs[i]),
               i + 1 < sizeof(strs) / sizeof(*strs) ? "," : "");
    printf("}\n}\n");
    return 0;
}
