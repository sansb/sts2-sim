#!/usr/bin/env python3
"""The frozen v0.111.0 Python modeling registry, as ``generate_content`` reads it.

Why this exists (#2827 item D)
------------------------------
``generate_content.py`` emits ``src/ids.rs``, ``src/content_tables.rs`` and the
dispatch trees from two halves. The *content* half already replays from the
committed DLL manifest (``data/dll_content.v0.111.0.json``, #2515). The
*modeling* half — card step programs, monster move tables, template relics,
dispatch sites, the op-language vocabularies (``dll_content.MODELING_ANNEX``) —
was read live out of ``combat_sim`` and the ``content/`` package. Item F of
#2827 deletes both outright, and ``combat_sim`` has been frozen at v0.111.0
since the #1282 authority flip, so every one of those facts is already a
constant. This module makes that constant a committed file:
``data/python_registry.v0.111.0.json``.

It is not a re-derivation and not a hand transcription. The snapshot is
**recorded** by running the real generator against the real registry through a
proxy that notes every attribute the generator reads and every source-reading
census it calls, and then **replayed** by handing the generator a registry
rebuilt from that recording. The acceptance test is byte identity: every file
the generator emitted was identical under the live registry and
``--registry frozen`` (the pre-deletion ``--verify-replay`` check). #2827 item
F deleted the simulator, and with it the recording (``--write``) and
live-replay modes: the snapshot is now frozen data, integrity-checked by
``--check`` and sha256-pinned by ``frozen_oracle_data.py``.

What is recorded
----------------
* ``attributes``: every ``combat_sim`` module global the generator (or a
  ``dll_content`` source it drives) read, with its value serialized exactly —
  tuples stay tuples, lists stay lists, sets and frozensets keep their type,
  dict keys keep their Python type (tuples included), dataclass and
  NamedTuple instances keep their class name and field order, and class
  objects (``AscensionTier``) are carried as references to one rebuilt class
  so ``type(v) is cs.AscensionTier`` still holds on replay.
* ``dir``: the module's ``dir()`` listing, which ``move_tables`` walks to find
  every ``*_MOVES`` table.
* ``censuses``: the return value (and the ``REFUSALS`` / ``DYNAMIC_SITES``
  rows appended) of each function that parses Python *source* or imports the
  ``content`` package — ``monster_kinds``, ``dispatch_sites``, the solo /
  multiplayer ``_card_can_play`` censuses, ``card_step_families``,
  ``monster_kind_pools`` and ``_pool_module_names``. Each call is keyed by the
  function name plus the sha256 of its non-registry arguments, so a replay
  whose inputs moved (a new DLL manifest changing ``axes``) refuses by name
  rather than returning a stale census (SOLVER_INVARIANTS.md I5).

A value of a type this codec does not know raises; nothing is ``str()``-ed.
An attribute the generator reads on replay that the recording never saw
raises ``FrozenRegistryMiss`` — a generator edit that needs a new registry
fact models it in Rust-owned generator code (see
``generate_content.apply_rust_overlays``); nothing can record one any more.

Standard library only.
"""

from __future__ import annotations

import argparse
import collections
import dataclasses
import enum
import hashlib
import json
import pathlib
import sys
import types
from fractions import Fraction
from typing import Any, Callable

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]
BUILD = "v0.111.0"
SCHEMA = "sts-sim-python-registry-snapshot-v1"
SNAPSHOT_PATH = RUST_DIR / "data" / f"python_registry.{BUILD}.json"


class FrozenRegistryMiss(NotImplementedError):
    """The generator asked the frozen registry for a fact never recorded."""


class SnapshotCodecError(ValueError):
    """A registry value this codec cannot represent exactly."""


# ---------------------------------------------------------------------------
# Encoding
# ---------------------------------------------------------------------------

def _is_named_tuple(value: Any) -> bool:
    return isinstance(value, tuple) and hasattr(type(value), "_fields")


class Encoder:
    """Serialize registry values; collects the class schemas they reference."""

    def __init__(self):
        self.classes: dict[str, dict] = {}

    def _class_ref(self, cls: type) -> str:
        name = cls.__qualname__
        if name in self.classes:
            return name
        if dataclasses.is_dataclass(cls):
            params = cls.__dataclass_params__
            schema = {
                "kind": "dataclass",
                "fields": [f.name for f in dataclasses.fields(cls)],
                "frozen": bool(params.frozen),
            }
        elif issubclass(cls, tuple) and hasattr(cls, "_fields"):
            schema = {"kind": "namedtuple", "fields": list(cls._fields)}
        elif issubclass(cls, enum.Enum):
            schema = {"kind": "enum",
                      "members": [[m.name, self.encode(m.value)] for m in cls]}
        else:
            raise SnapshotCodecError(f"class {name!r} is not a dataclass, "
                                     "NamedTuple or Enum")
        self.classes[name] = schema
        return name

    def encode(self, value: Any) -> Any:
        if value is None or type(value) in (bool, int, str):
            return value
        if type(value) is float:
            return {"__float__": repr(value)}
        if type(value) is Fraction:
            return {"__fraction__": [value.numerator, value.denominator]}
        if isinstance(value, type):
            return {"__class__": self._class_ref(value)}
        if isinstance(value, enum.Enum):
            return {"__enum__": [self._class_ref(type(value)), value.name]}
        if _is_named_tuple(value):
            return {"__nt__": self._class_ref(type(value)),
                    "v": [self.encode(v) for v in value]}
        if dataclasses.is_dataclass(value):
            cls = type(value)
            return {"__dc__": self._class_ref(cls),
                    "v": [self.encode(getattr(value, f.name))
                          for f in dataclasses.fields(cls)]}
        if type(value) is tuple:
            return [self.encode(v) for v in value]
        if type(value) is list:
            return {"__list__": [self.encode(v) for v in value]}
        if type(value) in (set, frozenset):
            members = [self.encode(v) for v in value]
            members.sort(key=lambda m: json.dumps(m, sort_keys=True))
            tag = "__set__" if type(value) is set else "__frozenset__"
            return {tag: members}
        if type(value) in (dict, collections.OrderedDict):
            return {"__dict__": [[self.encode(k), self.encode(v)]
                                 for k, v in value.items()]}
        if type(value) is collections.defaultdict:
            return {"__dict__": [[self.encode(k), self.encode(v)]
                                 for k, v in value.items()]}
        raise SnapshotCodecError(
            f"cannot represent {type(value).__module__}."
            f"{type(value).__qualname__} exactly")


# ---------------------------------------------------------------------------
# Decoding
# ---------------------------------------------------------------------------

class Decoder:
    """Rebuild values; one class object per recorded class name."""

    def __init__(self, classes: dict[str, dict]):
        self.schemas = classes
        self.built: dict[str, type] = {}

    def cls(self, name: str) -> type:
        if name in self.built:
            return self.built[name]
        schema = self.schemas[name]
        kind = schema["kind"]
        if kind == "dataclass":
            built = dataclasses.make_dataclass(
                name, [(f, Any) for f in schema["fields"]],
                frozen=schema["frozen"])
        elif kind == "namedtuple":
            built = collections.namedtuple(name, schema["fields"])
        elif kind == "enum":
            built = enum.Enum(name, [(m, self.decode(v))
                                     for m, v in schema["members"]])
        else:
            raise SnapshotCodecError(f"unknown class kind {kind!r}")
        self.built[name] = built
        return built

    def decode(self, obj: Any) -> Any:
        if obj is None or type(obj) in (bool, int, str):
            return obj
        if type(obj) is list:
            return tuple(self.decode(v) for v in obj)
        if type(obj) is not dict or len(obj) not in (1, 2):
            raise SnapshotCodecError(f"malformed snapshot value {obj!r:.80}")
        if "__float__" in obj:
            return float(obj["__float__"])
        if "__fraction__" in obj:
            return Fraction(*obj["__fraction__"])
        if "__class__" in obj:
            return self.cls(obj["__class__"])
        if "__enum__" in obj:
            cls_name, member = obj["__enum__"]
            return self.cls(cls_name)[member]
        if "__nt__" in obj:
            return self.cls(obj["__nt__"])(*(self.decode(v) for v in obj["v"]))
        if "__dc__" in obj:
            return self.cls(obj["__dc__"])(*(self.decode(v) for v in obj["v"]))
        if "__list__" in obj:
            return [self.decode(v) for v in obj["__list__"]]
        if "__set__" in obj:
            return {self.decode(v) for v in obj["__set__"]}
        if "__frozenset__" in obj:
            return frozenset(self.decode(v) for v in obj["__frozenset__"])
        if "__dict__" in obj:
            return {self.decode(k): self.decode(v) for k, v in obj["__dict__"]}
        raise SnapshotCodecError(f"unknown snapshot tag in {sorted(obj)}")


# ---------------------------------------------------------------------------
# Census keys
# ---------------------------------------------------------------------------

def census_key(name: str, args: tuple, kwargs: dict) -> str:
    """``name`` plus the sha256 of the non-registry arguments."""
    encoder = Encoder()
    payload = json.dumps(
        [[encoder.encode(a) for a in args],
         [[k, encoder.encode(v)] for k, v in sorted(kwargs.items())]],
        sort_keys=True, separators=(",", ":"))
    return f"{name}:{hashlib.sha256(payload.encode()).hexdigest()[:16]}"


# ---------------------------------------------------------------------------
# Recording and replay registries
# ---------------------------------------------------------------------------

class RecordingRegistry:
    """Wrap the live registry namespace; note every attribute read."""

    def __init__(self, raw):
        object.__setattr__(self, "_raw", raw)
        object.__setattr__(self, "_read", {})
        object.__setattr__(self, "_dir", None)
        object.__setattr__(self, "_overrides", {})
        object.__setattr__(self, "_censuses", {})

    def __getattr__(self, name):
        overrides = object.__getattribute__(self, "_overrides")
        if name in overrides:
            return overrides[name]
        raw = object.__getattribute__(self, "_raw")
        value = getattr(raw, name)
        object.__getattribute__(self, "_read")[name] = value
        return value

    def __setattr__(self, name, value):
        object.__getattribute__(self, "_overrides")[name] = value

    def __dir__(self):
        listing = sorted(dir(object.__getattribute__(self, "_raw")))
        object.__setattr__(self, "_dir", listing)
        return listing

    def raw(self):
        """The live namespace with this run's overlays applied, unrecorded."""
        import types  # noqa: PLC0415

        raw = object.__getattribute__(self, "_raw")
        overrides = object.__getattribute__(self, "_overrides")
        return types.SimpleNamespace(**{**vars(raw), **overrides})

    def record_census(self, key, result, refusals, dynamic):
        censuses = object.__getattribute__(self, "_censuses")
        row = (result, tuple(refusals), tuple(dynamic))
        if key in censuses and censuses[key] != row:
            raise SnapshotCodecError(
                f"census {key} returned two different values in one run")
        censuses[key] = row

    def snapshot(self) -> dict:
        encoder = Encoder()
        read = object.__getattribute__(self, "_read")
        attributes = {name: encoder.encode(read[name]) for name in sorted(read)}
        censuses = {}
        for key, (result, refusals, dynamic) in sorted(
                object.__getattribute__(self, "_censuses").items()):
            censuses[key] = {
                "result": encoder.encode(result),
                "refusals": [list(r) for r in refusals],
                "dynamic_sites": [list(d) for d in dynamic],
            }
        body = {
            "schema": SCHEMA,
            "build": BUILD,
            "provenance": (
                "recorded from versions/v0.111.0/solver/combat_sim.py and "
                "content/ by `registry_snapshot.py --write` (#2827 item D); "
                "frozen at v0.111.0 under the #1282 authority flip"),
            "classes": {name: encoder.classes[name]
                        for name in sorted(encoder.classes)},
            "dir": object.__getattribute__(self, "_dir"),
            "attributes": attributes,
            "censuses": censuses,
        }
        return body


class FrozenRegistry:
    """Serve registry attributes and censuses from a committed snapshot."""

    def __init__(self, payload: dict):
        if payload.get("schema") != SCHEMA:
            raise SnapshotCodecError(
                f"snapshot schema {payload.get('schema')!r} != {SCHEMA!r}")
        decoder = Decoder(payload["classes"])
        object.__setattr__(self, "_decoder", decoder)
        object.__setattr__(self, "_values", {
            name: decoder.decode(value)
            for name, value in payload["attributes"].items()})
        object.__setattr__(self, "_dir", payload["dir"])
        object.__setattr__(self, "_censuses", payload["censuses"])
        object.__setattr__(self, "_overrides", {})

    def __getattr__(self, name):
        overrides = object.__getattribute__(self, "_overrides")
        if name in overrides:
            return overrides[name]
        values = object.__getattribute__(self, "_values")
        if name not in values:
            raise FrozenRegistryMiss(
                f"combat_sim.{name} is not in the frozen registry snapshot "
                f"({SNAPSHOT_PATH.name}); the Python simulator is frozen and "
                "slated for deletion (#2827), so a new modeling fact belongs "
                "in Rust-owned generator code, not in the snapshot")
        return values[name]

    def __setattr__(self, name, value):
        object.__getattribute__(self, "_overrides")[name] = value

    def __dir__(self):
        listing = object.__getattribute__(self, "_dir")
        if listing is None:
            raise FrozenRegistryMiss("dir(combat_sim) was never recorded")
        return list(listing)

    def census(self, key):
        censuses = object.__getattribute__(self, "_censuses")
        if key not in censuses:
            raise FrozenRegistryMiss(
                f"census {key} is not in the frozen registry snapshot: either "
                "the generator calls a source-reading census it never called "
                "while recording, or that census's inputs (for example the "
                "DLL manifest's axes) moved since the recording")
        row = censuses[key]
        decoder = object.__getattribute__(self, "_decoder")
        return (decoder.decode(row["result"]),
                [tuple(r) for r in row["refusals"]],
                [tuple(d) for d in row["dynamic_sites"]])


def frozen_census(fn: Callable) -> Callable:
    """Record a source-reading census's result, or replay it.

    The registry is the census's first positional argument, or the
    generator's ``ACTIVE_REGISTRY`` for a census that takes none; it never
    contributes to the key. Under a live, unrecorded registry the census runs
    unchanged.

    The key includes the census's other arguments (the DLL-derived ``axes``),
    so a forked crate whose manifest moves an axis gets ``FrozenRegistryMiss``
    naming the census, never the v0.111.0 answer computed for other axes.
    """
    name = fn.__name__

    def wrapper(*args, **kwargs):
        # The generator's own globals, not `import generate_content`: run as
        # a script it is `__main__`, and a fresh import would be a second
        # module with its own REFUSALS list.
        gc = fn.__globals__
        active = gc["ACTIVE_REGISTRY"]
        # Every census that takes a registry takes it first. An explicit
        # non-proxy first argument — the live namespace, or the `None` a
        # synthetic-source control passes — means "run the census", never
        # "replay the recorded one"; only a census with no registry argument
        # (`monster_kind_pools`) consults the active registry.
        registry = args[0] if args else active
        # Any registry-shaped argument — the proxy, the live namespace a
        # nested census receives while recording — is the registry, never
        # part of the key.
        key_args = tuple(a for a in args
                         if not isinstance(a, (RecordingRegistry,
                                               FrozenRegistry,
                                               types.SimpleNamespace))
                         and a is not active)
        if isinstance(registry, FrozenRegistry):
            result, refusals, dynamic = registry.census(
                census_key(name, key_args, kwargs))
            gc["REFUSALS"].extend(refusals)
            gc["DYNAMIC_SITES"].extend(dynamic)
            return result
        if isinstance(registry, RecordingRegistry):
            raw = registry.raw()
            raw_args = tuple(raw if a is registry else a for a in args)
            refusals, dynamic = gc["REFUSALS"], gc["DYNAMIC_SITES"]
            before_r, before_d = len(refusals), len(dynamic)
            result = fn(*raw_args, **kwargs)
            registry.record_census(
                census_key(name, key_args, kwargs), result,
                refusals[before_r:], dynamic[before_d:])
            return result
        return fn(*args, **kwargs)

    wrapper.__name__ = name
    wrapper.__doc__ = fn.__doc__
    wrapper.__wrapped__ = fn
    return wrapper


# ---------------------------------------------------------------------------
# File I/O
# ---------------------------------------------------------------------------

def _compact(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def snapshot_text(body: dict) -> str:
    """Deterministic bytes with an embedded ``self_sha256`` over the body.

    One line per top-level scalar and one line per entry of each mapping
    section (``attributes``, ``censuses``, ``classes``), so a reviewer sees
    which registry fact moved rather than one 800 KB line.
    """
    unsealed = dict(body)
    unsealed.pop("self_sha256", None)
    digest = hashlib.sha256(_compact(unsealed).encode()).hexdigest()
    sealed = dict(unsealed, self_sha256=digest)
    lines = ["{"]
    keys = sorted(sealed)
    for index, key in enumerate(keys):
        comma = "," if index + 1 < len(keys) else ""
        value = sealed[key]
        if isinstance(value, dict) and value:
            lines.append(f"{_compact(key)}:{{")
            entries = sorted(value)
            for inner_index, inner in enumerate(entries):
                inner_comma = "," if inner_index + 1 < len(entries) else ""
                lines.append(f"{_compact(inner)}:{_compact(value[inner])}"
                             f"{inner_comma}")
            lines.append("}" + comma)
        else:
            lines.append(f"{_compact(key)}:{_compact(value)}{comma}")
    lines.append("}")
    return "\n".join(lines) + "\n"


def load_snapshot(path: pathlib.Path | None = None) -> FrozenRegistry:
    """Read and integrity-check the committed snapshot."""
    path = SNAPSHOT_PATH if path is None else path
    try:
        text = path.read_text()
    except OSError as exc:
        raise FrozenRegistryMiss(f"no frozen registry snapshot: {exc}")
    payload = json.loads(text)
    if snapshot_text(payload) != text:
        raise SnapshotCodecError(
            f"{path.name}: self_sha256 or byte layout does not match the "
            "body; the frozen registry was edited by hand")
    return FrozenRegistry(payload)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--check", action="store_true",
                       help="integrity-check the committed snapshot")
    # `--write` (re-record from the live registry) and `--verify-replay`
    # (live vs frozen byte identity) needed the Python simulator and were
    # deleted with it (#2827 item F).
    args = parser.parse_args(argv)
    sys.path.insert(0, str(HERE.parent))
    if args.check:
        registry = load_snapshot()
        values = object.__getattribute__(registry, "_values")
        censuses = object.__getattribute__(registry, "_censuses")
        print(f"{SNAPSHOT_PATH.name}: intact ({len(values)} attributes, "
              f"{len(censuses)} censuses)")
        return 0
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
