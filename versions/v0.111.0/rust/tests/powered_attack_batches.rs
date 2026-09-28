//! Native powered `AttackCommand` batches at the public action boundary (#2655).
//!
//! Authority: archived v0.111.0 `sts2.dll`, SHA-256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
//! `AttackCommand.<Execute>d__90::MoveNext` (RVA `0x3f19c0`) issues one
//! `CreatureCmd.Damage(IEnumerable<Creature>)` per hit at IL_0702-IL_0743, and
//! `CreatureCmd.<Damage>d__12::MoveNext` (RVA `0x3e96c8`) commits every
//! snapshotted receiver (IL_00c6 … IL_0aa4) before dispatching one frozen
//! result list (IL_0ad9-IL_0e84) and draining one `Kill` (IL_0ead-IL_0eb4).
use serde_json::json;
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    engine::{self, Action, SelectionRef},
};

fn root() -> CanonicalStateV2 {
    serde_json::from_str(include_str!(
        "../fixtures/canonical_state_v2_ironclad_toadpoles.json"
    ))
    .unwrap()
}

/// The canonical player map omits fields at their default, so a reader that
/// indexes it directly would panic rather than compare.
fn player_int(doc: &CanonicalStateV2, key: &str, default: i64) -> i64 {
    doc.player
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(default)
}

fn set_monster(doc: &mut CanonicalStateV2, index: usize, hp: i64) {
    doc.monsters[index].insert("hp".into(), json!(hp));
    doc.monsters[index].insert("max_hp".into(), json!(hp));
}

/// Play one card and prove the resulting document survives a cold reload —
/// the batch's private pending-death receipts must not reach the wire, and a
/// completed batch must normalize back to an importable state.
fn play_and_reload(doc: &CanonicalStateV2, uid: u32) -> CanonicalStateV2 {
    let catalog = HotBoundary::catalog_from_canonical(doc).unwrap();
    let state = HotBoundary::from_canonical(doc, &catalog).unwrap();
    engine::admit(doc, &state, &catalog).unwrap();
    let next = engine::apply_action_into(
        &state,
        &catalog,
        &Action::Play {
            uid,
            target: None,
            selection: SelectionRef::NONE,
        },
        &mut Vec::new(),
    )
    .unwrap();
    let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
    let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
    let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
    if !next.history.over {
        engine::admit(&wire, &cold, &cold_catalog).unwrap();
    }
    assert_eq!(
        HotBoundary::try_to_canonical(&cold, &cold_catalog).unwrap(),
        wire
    );
    wire
}

/// The #2654 Kaiser Crab shape, with Gremlin Horn standing in for CrabRage as
/// the death hook that reads the live roster.
///
/// A killing AoE commits HP to BOTH receivers in phase 1, so the first death
/// drained in phase 3 already sees a roster with no live enemy: native
/// `IsEnding` gates Horn's energy and draw (`finish_monster_death`'s
/// `damage_combat_is_ending`). Before this slice the first receiver's death
/// resolved while the second was still at full HP, and Horn drew a card and
/// gained a point of energy off a peer that was about to die anyway.
#[test]
fn a_killing_aoe_commits_both_receivers_before_the_first_death_hook_runs() {
    let mut doc = root();
    doc.player
        .insert("relics_entering".into(), json!(["RELIC.GREMLIN_HORN"]));
    doc.player.insert("gremlin_horn".into(), json!(true));
    set_monster(&mut doc, 0, 4);
    set_monster(&mut doc, 1, 4);
    doc.piles.get_mut("hand").unwrap()[0].id = "THUNDERCLAP".into();

    let next = play_and_reload(&doc, 0);

    assert_eq!(next.monsters[0]["hp"], 0);
    assert_eq!(
        next.monsters[1]["hp"], 0,
        "the second receiver takes its own committed damage"
    );
    assert_eq!(
        next.piles["draw"], doc.piles["draw"],
        "both deaths drain after every commit, so Horn is ending-gated"
    );
    assert_eq!(
        next.player["cards_drawn_combat"], 5,
        "no Gremlin Horn draw off an already-doomed peer"
    );
    assert_eq!(
        next.player["energy"], 2,
        "Thunderclap costs one and Horn gains none"
    );
}

/// The same roster, a peer that survives its committed hit: the first drained
/// death is then NOT terminal and Gremlin Horn fires exactly once. The batch
/// defers deaths; it does not suppress their hooks.
#[test]
fn a_surviving_peer_still_lets_the_first_drained_death_run_its_hook() {
    let mut doc = root();
    doc.player
        .insert("relics_entering".into(), json!(["RELIC.GREMLIN_HORN"]));
    doc.player.insert("gremlin_horn".into(), json!(true));
    set_monster(&mut doc, 0, 4);
    set_monster(&mut doc, 1, 20);
    doc.piles.get_mut("hand").unwrap()[0].id = "THUNDERCLAP".into();

    let next = play_and_reload(&doc, 0);

    assert_eq!(next.monsters[0]["hp"], 0);
    assert_eq!(next.monsters[1]["hp"], 16);
    assert_eq!(
        next.piles["draw"].len(),
        4,
        "the surviving peer keeps the first death off the ending gate"
    );
    assert_eq!(
        player_int(&next, "energy", 3),
        3,
        "one spent on Thunderclap, one returned by Gremlin Horn"
    );
}

/// `0x3e96c8` gates the dealer's liveness at Damage ENTRY (IL_0078-IL_00af) and
/// never again: the receiver loop's only skip is the receiver's own `IsDead`
/// (IL_0167-IL_0172), and the commit at IL_02be-IL_04c1 follows
/// `BeforeDamageReceived`'s resume (IL_02b9) unconditionally. A Thorns kill
/// during phase 1 therefore cancels the next HIT (`0x3f19c0` IL_0156-IL_0161)
/// and not the receivers already snapshotted at IL_00c6.
#[test]
fn a_thorns_kill_mid_batch_still_commits_the_remaining_receiver() {
    let mut doc = root();
    doc.player.insert("hp".into(), json!(5));
    set_monster(&mut doc, 0, 50);
    doc.monsters[0].insert("thorns".into(), json!(10));
    set_monster(&mut doc, 1, 50);
    doc.piles.get_mut("hand").unwrap()[0].id = "THUNDERCLAP".into();

    let next = play_and_reload(&doc, 0);

    assert_eq!(player_int(&next, "hp", 0), 0);
    assert_eq!(next.player["player_hooks_deactivated"], true);
    assert_eq!(next.monsters[0]["hp"], 46);
    assert_eq!(
        next.monsters[1]["hp"], 46,
        "the dealer's death cancels the next hit, not this hit's other receiver"
    );
}
