"""Shared card-template target/eligibility schema and admission guards (#439).

The DLL-facing translator produces this metadata for every card class, even
when its OnPlay body refuses translation. Composition and runtime use the
same conservative target/action admission check so a stale reviewed ledger
cannot turn an ally target into an enemy action.
"""

TARGET_TYPE_NAMES = {
    0: "None", 1: "Self", 2: "AnyEnemy", 3: "AllEnemies",
    4: "RandomEnemy", 5: "AnyPlayer", 6: "AnyAlly", 7: "AllAllies",
    8: "TargetedNoCreature", 9: "Osty",
}

MULTIPLAYER_CONSTRAINT_NAMES = {
    0: "None", 1: "MultiplayerOnly", 2: "SingleplayerOnly",
}

# MultiplayerOnly is descriptive eligibility metadata, not a blanket
# refusal. These are the target spaces the simulator can currently express;
# every card still needs its individual full effect/hook read.
SUPPORTED_TEMPLATE_TARGETS = frozenset({
    "None", "Self", "AnyEnemy", "AllEnemies",
})


def _validate_enum(label, value, names):
    if not isinstance(value, dict) or set(value) != {"name", "value"}:
        raise ValueError(f"{label} must contain exactly name/value")
    number = value["value"]
    if not isinstance(number, int) or names.get(number) != value["name"]:
        raise ValueError(f"{label} has unknown or mismatched enum value")


def validate_targeting_map(targeting, census):
    """Validate the universal checked schema against the full card census."""
    if not isinstance(targeting, dict) or set(targeting) != set(census):
        missing = sorted(set(census) - set(targeting)) \
            if isinstance(targeting, dict) else sorted(census)
        extra = sorted(set(targeting) - set(census)) \
            if isinstance(targeting, dict) else []
        raise ValueError("targeting metadata must cover the exact card census: "
                         f"missing={missing} extra={extra}")
    for cid, meta in targeting.items():
        if not isinstance(meta, dict) or set(meta) != {
                "constructor_rva", "constructor_target_type",
                "effective_target_type", "multiplayer_constraint"}:
            raise ValueError(f"{cid} targeting metadata has invalid keys")
        if not isinstance(meta["constructor_rva"], str) \
                or not meta["constructor_rva"].startswith("0x"):
            raise ValueError(f"{cid} constructor_rva is invalid")
        _validate_enum(f"{cid} constructor_target_type",
                       meta["constructor_target_type"], TARGET_TYPE_NAMES)

        effective = meta["effective_target_type"]
        if not isinstance(effective, dict) or effective.get("kind") \
                not in ("static", "dynamic"):
            raise ValueError(f"{cid} effective_target_type is invalid")
        if effective["kind"] == "static":
            required = {"kind", "name", "source", "value"}
            if set(effective) not in (required, required | {"getter_rva"}):
                raise ValueError(f"{cid} static target metadata has invalid keys")
            _validate_enum(f"{cid} effective_target_type",
                           {"name": effective["name"],
                            "value": effective["value"]}, TARGET_TYPE_NAMES)
        elif set(effective) != {"getter_rva", "kind", "source"}:
            raise ValueError(f"{cid} dynamic target metadata has invalid keys")
        if effective.get("source") not in ("constructor", "getter"):
            raise ValueError(f"{cid} effective target source is invalid")
        if effective["source"] == "constructor":
            if "getter_rva" in effective \
                    or {"name": effective.get("name"),
                        "value": effective.get("value")} \
                    != meta["constructor_target_type"]:
                raise ValueError(f"{cid} constructor target source is invalid")
        elif "getter_rva" not in effective:
            raise ValueError(f"{cid} getter target lacks evidence RVA")

        constraint = meta["multiplayer_constraint"]
        if not isinstance(constraint, dict) or constraint.get("kind") \
                not in ("static", "dynamic"):
            raise ValueError(f"{cid} multiplayer constraint is invalid")
        if constraint["kind"] == "static":
            required = {"kind", "name", "source", "value"}
            if set(constraint) not in (required, required | {"getter_rva"}):
                raise ValueError(f"{cid} static constraint metadata has invalid keys")
            _validate_enum(f"{cid} multiplayer_constraint",
                           {"name": constraint["name"],
                            "value": constraint["value"]},
                           MULTIPLAYER_CONSTRAINT_NAMES)
        elif set(constraint) != {"getter_rva", "kind", "source"}:
            raise ValueError(f"{cid} dynamic constraint metadata has invalid keys")
        if constraint.get("source") not in ("inherited", "getter"):
            raise ValueError(f"{cid} multiplayer constraint source is invalid")
        if constraint["source"] == "inherited":
            if "getter_rva" in constraint or constraint.get("value") != 0 \
                    or constraint.get("name") != "None":
                raise ValueError(f"{cid} inherited constraint is invalid")
        elif "getter_rva" not in constraint:
            raise ValueError(f"{cid} constraint getter lacks evidence RVA")


def template_target_refusal(meta, levels):
    """Return an I5 error for unsupported metadata or target/action drift."""
    target = meta["effective_target_type"]
    constraint = meta["multiplayer_constraint"]
    if constraint["kind"] != "static":
        return "card has a dynamic multiplayer constraint"
    if target["kind"] != "static":
        return "card has a dynamic target type"
    if target["name"] not in SUPPORTED_TEMPLATE_TARGETS:
        return ("card target is unsupported: "
                f"TargetType.{target['name']}({target['value']})")

    # `attack` and enemy-targeted power applications consume cardPlay.Target;
    # `attack_all` independently enumerates opponents and is not selected.
    # Check every level: Card.targeted controls whether the runtime action
    # carries a creature index, so an AnyEnemy level with no target consumer
    # is just as unsafe as an enemy-shaped Self level.
    for level, steps in levels.items():
        selected = any(
            step[0] == "attack"
            or (step[0] == "power" and len(step) > 2
                and step[2] == "enemy")
            for step in steps)
        all_enemies = any(step[0] == "attack_all" for step in steps)
        if target["name"] == "AnyEnemy" and not selected:
            return f"AnyEnemy level {level} has no selected-enemy step"
        if target["name"] != "AnyEnemy" and selected:
            return (f"level {level} is enemy-targeted but TargetType is "
                    f"{target['name']}")
        if target["name"] == "AllEnemies" and not all_enemies:
            return f"AllEnemies level {level} has no all-enemies step"
        if target["name"] != "AllEnemies" and all_enemies:
            return (f"level {level} is all-enemies but TargetType is "
                    f"{target['name']}")
    return None
