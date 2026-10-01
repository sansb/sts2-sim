"""#138 Batch 293 / #1214: v0.111.0 version-impact gate.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

from admission import admitted_builds
import json
from pathlib import Path

from sts2_rng import GAME_BUILD_V0_111_0, seeding_scheme


HERE = Path(__file__).parent
BUILD = {
    "version": "v0.111.0",
    "commit": "41cef1ea",
    "sts2_dll_sha256":
        "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4",
}


def _load(name):
    return json.loads((HERE / name).read_text())


def test_v01110_rng_and_completed_combat_boundary_are_admitted():
    assert GAME_BUILD_V0_111_0 == BUILD["version"]
    assert seeding_scheme(BUILD["version"]) == "v109"

    # v0.111.0 combat is admitted. The Batch 298 remodel-issue tuple that
    # used to stand in for this went empty and became dead code; the
    # registry states it outright (#1258, I11).
    assert "v0.111.0" in admitted_builds()


def test_current_build_pool_censuses_are_exact_and_behavior_preserving():
    for name in ("card_pool_census.json", "potion_pool_census.json"):
        census = _load(name)
        assert census["game_build"] == BUILD
        assert census["source"]["kind"] == "headless-engine-oracle"


def test_like_for_like_impact_report_is_complete_and_build_exact():
    impact = _load("../../meta/version-impact/v0.111.0.json")
    assert impact["schema"] == 1
    assert impact["build"]["version"] == BUILD["version"]
    assert impact["build"]["commit"] == BUILD["commit"]
    assert impact["build"]["sts2_dll_sha256"] == \
        BUILD["sts2_dll_sha256"]
    assert impact["managed_comparison"]["parse_errors"] == {
        "baseline": 0, "build": 0}
    assert impact["managed_comparison"]["same_assembly_control_delta"] == 0
    assert impact["rng"]["getter_rows"] == [162, 163]
    assert impact["rng"]["new_consumers"] == [
        "BeautifulBracelet::AfterObtained -> RunRngSet::get_Niche"]

    delta = impact["like_for_like_artifact_delta"]
    assert len(delta["cards"]["shared_nullable_card_pile_result"]) == 12
    values = delta["game_values"]
    assert len(values["cards"]) == 15
    assert values["relics"] == ["REGALITE Block 6 -> 4"]
    assert values["enchantments"] == [
        "INKY removes Damage 1 and EnchantDamageAdditive; "
        "WeakPower 1 remains"]
    assert values["potions"] == values["colors"] == []
    assert delta["encounters"] == [
        "AXEBOT ONE_TWO 9/10 -> 10/11; HAMMER_UPPERCUT 12/14 -> 14/18",
        "AXEBOT respawn Max HP +10 per spent Stock",
        "ENTOMANCER A8+ HP 155 -> 165",
        "EXOSKELETON A8+ HP min 25 -> 26",
        "EXOSKELETON A8+ HP max 29 -> 30",
        "GLOBE_HEAD GalvanicPower 6 -> 8 at A9+",
        "LOUSE_PROGENITOR Curl And Grow Strength 5 -> 7 at A9+",
        "MECHA_KNIGHT Flamethrower gains 8/12 attack before 4 Burn",
    ]
    assert {"relics_census", "potions_census", "enchantments_census"} \
        <= set(delta["semantic_equal"])
    assert impact["children"] == {
        "cards": 1215, "encounters": 1216, "shared_combat": 1217,
        "rng_relic": 1218, "enchantment": 1222}


def test_current_build_viewer_values_are_exact_and_not_an_alias():
    values = json.loads((HERE.parents[1] / "data" / "game_values.json").read_text())
    current = values["builds"][BUILD["version"]]
    assert current["dll_sha256"] == BUILD["sts2_dll_sha256"]
    assert "alias" not in current
    assert current["cards"]["CARD.ALIGNMENT"]["stars"] == [2, 2]
    assert current["cards"]["CARD.HYPERBEAM"]["vars"]["Damage"] == [24, 30]
    assert current["relics"]["RELIC.REGALITE"] == {"vars": {"Block": 4}}
    assert current["enchantments"]["ENCHANTMENT.INKY"] == {
        "vars": {"WeakPower": 1}}


def test_generated_content_snapshots_remain_historical_and_exact():
    cards = _load("cards_census.json")
    assert cards["CARD.EXPECT_A_FIGHT"]["cost"] == 2
    assert cards["CARD.REND"]["cost"] == 2
    assert cards["CARD.TIMES_UP"]["keywords"] == ["Exhaust"]
    assert _load("card_templates.json")["refused"][
        "CARD.FORGOTTEN_RITUAL"] == \
        "call ForgottenRitual::get_WasCardExhaustedThisTurn"

    encounters = _load("encounters_census.json")["monsters"]
    assert encounters["Entomancer"]["hp_min_consts"] == [8, 155, 145]
    assert encounters["Exoskeleton"]["hp_min_consts"] == [8, 25, 24]
    assert encounters["Exoskeleton"]["hp_max_consts"] == [8, 29, 28]


def test_version_impact_docs_pin_archive_harness_and_new_niche_consumer():
    archive = (HERE.parents[1] / "dll-archive" / "README.md").read_text()
    rng = (HERE / "RNG_FINDINGS.md").read_text()
    consumers = (HERE / "STREAM_CONSUMERS.md").read_text()
    harness = (HERE / "harness" / "README.md").read_text()
    for text in (archive, rng, consumers, harness):
        assert BUILD["version"] in text and BUILD["commit"] in text
    assert BUILD["sts2_dll_sha256"] in archive
    assert "Niche (18 sites)" in consumers
    assert "BeautifulBracelet::AfterObtained" in consumers
    assert "NullPlatformUtilStrategy" in harness
    assert "SteamPlatformUtilStrategy" in harness
