"""The .mcr decoder reads a capture that carries one zero byte after a stream
that ended on a byte boundary (#3756).

The game's writer (`CombatReplayWriter::WriteReplay`, RVA 0x69c7c) writes
`CeilToInt(BitPosition / 8f)` bytes: at most seven zero bits of padding, and
none when the stream ends on a boundary. 105 of the 6,376 captures uploaded to
the site by 2026-10-10 were one byte longer than that. They are every
byte-aligned capture of 33 runs; the other 360 runs have 709 byte-aligned
captures and none has the byte. Their streams decode to the last field; the
byte is zero in all of them.

Two layers:

1. The end-of-stream rule alone, at every bit position a stream can end on.
2. A real capture whose stream ends on a byte boundary (seed WDRG3QZP2Y7G,
   node 2, one of Sean's own): it decodes as the game wrote it, and to the
   same document with one zero byte appended; any other tail is refused.

Run: solver/tools/pytest_lane.sh -- \
    sim/v0.111.0/python/test_issue3756_mcr_trailing_byte.py
"""

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

from mcr_parser import BitReader, McrDecoder, McrError  # noqa: E402

MCR = (pathlib.Path(__file__).parent / "testdata"
       / "WDRG3QZP2Y7G_n02_byte_aligned.mcr")


def _end(data, pos):
    # The constructor reads a replay header; this is the tail alone.
    decoder = object.__new__(McrDecoder)
    decoder.r = BitReader(bytes(data))
    decoder.r.pos = pos
    decoder.end_of_stream()


def _refused(data, pos, left):
    with pytest.raises(McrError, match=f"^{left} unconsumed bits after decode$"):
        _end(data, pos)


def test_a_stream_that_ends_on_a_byte_boundary_may_carry_one_zero_byte():
    _end([0xFF], 8)
    _end([0xFF, 0x00], 8)


@pytest.mark.parametrize("bit", range(8))
def test_the_extra_byte_must_be_zero(bit):
    _refused([0xFF, 1 << bit], 8, 8)


def test_two_whole_bytes_are_never_padding():
    _refused([0xFF, 0x00, 0x00], 8, 16)


@pytest.mark.parametrize("used", range(1, 8))
def test_a_stream_that_ends_inside_a_byte_keeps_only_that_bytes_padding(used):
    low = (1 << used) - 1
    _end([low], used)
    # A set padding bit.
    _refused([0xFF], used, 8 - used)
    # A byte after a partly used one is a ninth bit or more.
    _refused([low, 0x00], used, 16 - used)


def test_a_real_byte_aligned_capture_decodes_with_and_without_the_byte():
    data = MCR.read_bytes()
    document = McrDecoder(data).combat_replay()
    assert document["version"] == "v0.111.0"
    assert document["run"]["rng"]["seed"] == "WDRG3QZP2Y7G"
    assert len(document["events"]) == 1
    assert len(document["checksums"]) == 2

    # The stream ends on the boundary: the last byte is data, not padding.
    with pytest.raises(EOFError):
        McrDecoder(data[:-1]).combat_replay()

    assert McrDecoder(data + b"\x00").combat_replay() == document
    for tail in (b"\x01", b"\x80", b"\x00\x00", b"\x00\x01"):
        with pytest.raises(McrError, match="unconsumed bits after decode"):
            McrDecoder(data + tail).combat_replay()
