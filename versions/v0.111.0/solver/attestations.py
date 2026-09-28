"""Single-source attestation manifest for shared protected artifacts (#713).

Historical batch tests attest that shared inputs (censuses, templates,
ledgers, contract docs, engine-adjacent modules) have not drifted since
review. Before #713 every such test embedded its own sha256 literals, so
any engine batch that legitimately moved an artifact rewrote ~90 files of
mechanical pins. The values now live in exactly one checked-in manifest,
``attestations.json``; tests call :func:`attested` for the expected hash
and ``test_attestations.py`` keeps the manifest current against the live
files. Refreshing a pin is a deliberate one-file diff:

    python3 versions/v0.111.0/solver/tools/refresh_attestations.py

Batch-owned artifacts (a batch's own content file, its fight pins) keep
their inline hashes — this manifest is only for artifacts shared across
batches.
"""

import json
import hashlib
import pathlib

_HERE = pathlib.Path(__file__).resolve().parent
_MANIFEST_PATH = _HERE / "attestations.json"
_MANIFEST = json.loads(_MANIFEST_PATH.read_text())


def attested(name: str) -> str:
    """Return the manifest sha256 for a shared protected artifact."""
    try:
        return _MANIFEST[name]
    except KeyError:
        raise NotImplementedError(
            f"{name!r} has no attestation manifest entry "
            "(SOLVER_INVARIANTS.md I5 — add it deliberately via "
            "tools/refresh_attestations.py)") from None


def live_sha256(name: str) -> str:
    """sha256 of the live artifact, path relative to solver/."""
    return hashlib.sha256((_HERE / name).read_bytes()).hexdigest()


def manifest() -> dict:
    return dict(_MANIFEST)
