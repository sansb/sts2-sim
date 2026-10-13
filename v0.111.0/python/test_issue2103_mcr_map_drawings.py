"""The .mcr decoder reads a run snapshot that carries player map drawings
(#2103).

Until this the decoder refused the whole capture at the first drawn line, so
every fight after a player first drew on the map was lost: 92 of the 1,204
captures in one uploader archive on 2026-10-10.

Two layers:

1. A real capture: seed 39D29WG6HMEK, 33 lines, four of them eraser strokes.
   The decode consumes the snapshot and then every later field, event and
   checksum to the last bit (`decode` raises otherwise). The drawings are the
   same strokes the site's map-ink fixture for this run records from the floor
   save: an erased pen dot still in the list, then eraser strokes of 126 and
   133 points over it.
2. A stream the test writes itself, with the writer's side of the IL
   (`PacketWriter::Quantize`, RVA 0x68a2b: `uint((v - min) / (max - min) *
   2^bits)`), covering what one capture cannot: two players, a 64-bit id past
   2^53, a line with no points, and both ends of both quantization ranges.

Run: solver/tools/pytest_lane.sh -- \
    sim/v0.111.0/python/test_issue2103_mcr_map_drawings.py
"""

import pathlib
import sys

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).parent))

import mcr_parser  # noqa: E402
from mcr_parser import BitReader, McrDecoder  # noqa: E402

MCR = (pathlib.Path(__file__).parent / "testdata"
       / "39D29WG6HMEK_d21_map_drawings.mcr")

X = (-3.0, 3.0, 13)
Y = (-2.0, 2.0, 16)


class BitWriter:
    """LSB-first, the inverse of `BitReader`."""

    def __init__(self):
        self.bits = []

    def u(self, value, width):
        assert 0 <= value < 1 << width
        self.bits.extend((value >> i) & 1 for i in range(width))

    def quantized(self, value, lo, hi, width):
        self.u(int((value - lo) / (hi - lo) * 2 ** width), width)

    def drawings(self, players):
        self.u(len(players), 32)
        for player_id, lines in players:
            self.u(player_id, 64)
            self.u(len(lines), 32)
            for is_eraser, points in lines:
                self.u(int(is_eraser), 1)
                self.u(len(points), 16)
                for x, y in points:
                    self.quantized(x, *X)
                    self.quantized(y, *Y)

    def to_bytes(self):
        padded = self.bits + [0] * (-len(self.bits) % 8)
        return bytes(sum(bit << i for i, bit in enumerate(padded[n:n + 8]))
                     for n in range(0, len(padded), 8))


def _drawings_decoder(data):
    # The constructor reads a replay header; this stream is the drawings
    # field alone.
    decoder = object.__new__(McrDecoder)
    decoder.r = BitReader(data)
    return decoder


def test_a_capture_with_map_drawings_decodes_to_its_last_bit():
    replay = mcr_parser.decode(MCR)
    assert replay["version"] == "v0.111.0"
    assert replay["run"]["rng"]["seed"] == "39D29WG6HMEK"
    # Everything after the drawings: a drift of one bit in the snapshot would
    # not leave 17 events and 25 checksums that end on the final byte.
    assert replay["run"]["extra_fields"] is not None
    assert len(replay["events"]) == 17
    assert len(replay["checksums"]) == 25


def test_the_decoded_drawings_are_the_strokes_the_player_made():
    run = mcr_parser.decode(MCR)["run"]
    drawings = run["map_drawings"]
    assert len(drawings) == 1
    assert drawings[0]["player_id"] == run["players"][0]["net_id"]
    lines = drawings[0]["lines"]
    assert len(lines) == 33
    assert sum(len(line["points"]) for line in lines) == 346
    assert [i for i, line in enumerate(lines) if line["is_eraser"]] \
        == [27, 28, 29, 30]
    assert [len(lines[i]["points"]) for i in (27, 28, 29, 30)] \
        == [126, 133, 2, 3]
    assert lines[0]["points"][:2] == [[-0.595458984375, 0.42401123046875],
                                      [-0.595458984375, 0.4241943359375]]
    for line in lines:
        for x, y in line["points"]:
            assert -3.0 <= x < 3.0 and -2.0 <= y < 2.0
            # On the grid: 13 bits over a span of 6, 16 bits over 4.
            assert (x + 3.0) * 8192 / 6 == int((x + 3.0) * 8192 / 6)
            assert (y + 2.0) * 65536 / 4 == int((y + 2.0) * 65536 / 4)


def test_a_written_stream_reads_back_exactly():
    players = [
        (76561198012718148, [
            (False, [(-3.0, -2.0), (0.0, 0.0), (0.75, -0.5)]),
            (True, []),
            (True, [(-0.62109375, 0.29595947265625)]),
        ]),
        (1, []),
        (2 ** 64 - 1, [(False, [(1.5, 1.25)])]),
    ]
    writer = BitWriter()
    writer.drawings(players)
    # The largest value each field can carry: one step below the range's top.
    writer.u(1, 32)
    writer.u(7, 64)
    writer.u(1, 32)
    writer.u(0, 1)
    writer.u(1, 16)
    writer.u(2 ** 13 - 1, 13)
    writer.u(2 ** 16 - 1, 16)
    total_bits = len(writer.bits)
    decoder = _drawings_decoder(writer.to_bytes())

    assert decoder.map_drawings() == [
        {"player_id": player_id,
         "lines": [{"is_eraser": is_eraser,
                    "points": [list(point) for point in points]}
                   for is_eraser, points in lines]}
        for player_id, lines in players]
    assert decoder.map_drawings() == [{"player_id": 7, "lines": [
        {"is_eraser": False,
         "points": [[3.0 - 6.0 / 8192, 2.0 - 4.0 / 65536]]}]}]
    assert decoder.r.pos == total_bits


def test_a_point_is_29_bits_x_first():
    writer = BitWriter()
    writer.u(1, 32)
    writer.u(1, 64)
    writer.u(1, 32)
    writer.u(1, 1)
    writer.u(1, 16)
    writer.u(1, 13)       # x: one step above -3
    writer.u(1 << 15, 16)  # y: the middle of [-2, 2]
    decoder = _drawings_decoder(writer.to_bytes())
    assert decoder.map_drawings()[0]["lines"] == [
        {"is_eraser": True, "points": [[-3.0 + 6.0 / 8192, 0.0]]}]
    assert decoder.r.pos == 32 + 64 + 32 + 1 + 16 + 29


def test_a_stream_that_ends_inside_a_line_is_refused():
    writer = BitWriter()
    writer.drawings([(1, [(False, [(0.0, 0.0)] * 4)])])
    data = writer.to_bytes()
    with pytest.raises(EOFError):
        _drawings_decoder(data[:-2]).map_drawings()
