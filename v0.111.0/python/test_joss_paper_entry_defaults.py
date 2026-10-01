"""Native omitted-default regression from Y3NULJSNND7N floor 48.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""

import pytest

from tools.live_coach import _strict_joss_paper_properties, build_entry


@pytest.mark.parametrize("props", [
    False, 0, "", [],
    {"CardsExhausted": True}, {"CardsExhausted": "0"},
    {"EtherealCount": None}, {"IsWax": True}, {"Unknown": 0},
    {"ints": None}, {"ints": [{"name": "CardsExhausted", "value": False}]},
    {"ints": [{"name": "CardsExhausted", "value": 1}] * 2},
    {"ints": [{"name": [], "value": 0}]},
    {"ints": [{"name": "Other", "value": 0}]},
    {"ints": [{"name": "CardsExhausted", "value": 0, "extra": 0}]},
    {"bools": [{"name": "IsWax", "value": False}] * 2},
    {"strings": []}, {"ints": [], "CardsExhausted": 0},
])
def test_malformed_saved_payload_is_not_defaulted(props):
    assert _strict_joss_paper_properties(props) is None


def test_explicit_nonentry_values_are_preserved_for_refusal():
    assert _strict_joss_paper_properties({"CardsExhausted": 5, "EtherealCount": 1}) == {
        "CardsExhausted": 5, "EtherealCount": 1}
