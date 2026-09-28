//! Experimental, observation-only baseline for offer reviews.
//!
//! Each root is a fresh, independently shuffled fight. The action policy sees
//! only legal actions at the current observation; it never searches or retains
//! a line for a known shuffle. An engine refusal invalidates the whole batch.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::{env, fs};
use sts_sim::{
    boundary::HotBoundary,
    canonical::CanonicalStateV2,
    catalog::Catalog,
    engine::{self, Action, Event, LegalActionBuffer},
    hot::{HotState, PileId},
    solo_v1::admit_exact_solve,
};

#[path = "coach/common.rs"]
mod common;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        ((self.next() >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

fn choose(actions: &[Action], state: &HotState, catalog: &Catalog, rng: &mut Rng) -> Action {
    // Current hand, energy, block and living target HP are observable. Neither
    // draw pile nor RNG nor a hypothetical action successor is inspected.
    let hand = state.piles.get(PileId::Hand).as_slice();
    let weight = |a: &Action| -> f64 {
        match a {
            Action::EndTurn => {
                if state.energy > 0 {
                    0.02
                } else {
                    0.35
                }
            }
            Action::Select { .. } => 1.0,
            Action::UsePotion { .. } => 0.01, // pilot assumes an empty belt
            Action::Play { uid, target, .. } => {
                let Some(card) = hand.iter().find(|c| c.uid == *uid) else {
                    return 0.1;
                };
                let Some(spec) = catalog.spec(card.atom) else {
                    return 0.1;
                };
                let kind = if spec.is_power {
                    6.0
                } else if spec.is_attack {
                    5.0
                } else if spec.is_skill {
                    if state.block < 12 { 4.0 } else { 2.0 }
                } else {
                    0.2
                };
                let target_factor =
                    target
                        .and_then(|i| state.monsters.get(i as usize))
                        .map_or(1.0, |m| {
                            if m.hp > 0 {
                                1.0 + 8.0 / f64::from(m.hp.max(8))
                            } else {
                                0.1
                            }
                        });
                (kind + (spec.cost.clamp(0, 3) as f64) * 0.4) * target_factor
            }
        }
    };
    let mut sample = rng.unit() * actions.iter().map(weight).sum::<f64>();
    for a in actions {
        sample -= weight(a);
        if sample < 0.0 {
            return *a;
        }
    }
    *actions.last().unwrap()
}

fn effective_damage(before_hp: i32, unblocked: i32) -> i32 {
    before_hp.max(0).min(unblocked.max(0))
}

fn trial(
    doc: CanonicalStateV2,
    policy_seed: u64,
    resample_index: Option<u64>,
) -> Result<Value, String> {
    admit_exact_solve(&doc).map_err(|e| format!("admission: {e:?}"))?;
    let catalog: Catalog =
        HotBoundary::catalog_from_canonical(&doc).map_err(|e| format!("catalog: {e:?}"))?;
    let mut state =
        HotBoundary::from_canonical(&doc, &catalog).map_err(|e| format!("root: {e:?}"))?;
    engine::admit(&doc, &state, &catalog).map_err(|e| format!("engine admission: {e:?}"))?;
    if let Some(index) = resample_index {
        state = common::resample(&state, index);
    }
    let starting_hp = state.hp;
    let mut rng = Rng(policy_seed);
    let mut legal = LegalActionBuffer::new();
    let mut events = vec![];
    let mut actions_taken = 0;
    let (mut damage, mut block_generated) = (0_i64, 0_i64);
    while !state.history.over && state.hp > 0 && state.turn <= 20 && actions_taken < 256 {
        let actions = engine::legal_actions_into(&state, &catalog, &mut legal);
        if actions.is_empty() {
            return Err("no legal action in a live state".into());
        }
        let action = choose(actions, &state, &catalog, &mut rng);
        let mut enemy_hp: HashMap<_, _> = state.monsters.iter().map(|m| (m.uid, m.hp)).collect();
        state = engine::apply_action_into(&state, &catalog, &action, &mut events)
            .map_err(|e| format!("combat transition after {actions_taken} actions: {e:?}"))?;
        for event in &events {
            match event {
                Event::MonsterDamaged {
                    uid, unblocked, hp, ..
                } => {
                    let before = enemy_hp
                        .get(uid)
                        .ok_or_else(|| format!("damage to untracked spawned monster uid {uid}"))?;
                    damage += i64::from(effective_damage(*before, *unblocked));
                    enemy_hp.insert(*uid, *hp);
                }
                Event::PlayerBlockGained { amount, .. } => block_generated += i64::from(*amount),
                _ => {}
            }
        }
        actions_taken += 1;
    }
    let won = state.history.over && state.hp > 0 && state.monsters.iter().all(|m| m.hp <= 0);
    let cutoff = !state.history.over && state.hp > 0;
    Ok(
        json!({"won":won,"cutoff":cutoff,"hp_lost":starting_hp-state.hp.max(0),
        "remaining_hp":state.hp.max(0),"turn":state.turn,
        "damage_dealt":damage,"block_generated":block_generated}),
    )
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let result = (|| -> Result<Value, String> {
        if args.len() != 2 && args.len() != 3 {
            return Err("usage: coach_policy ROOTS.json [POLICY_SEED_OFFSET]".into());
        }
        let offset: u64 = args
            .get(2)
            .map_or(Ok(0), |x| x.parse())
            .map_err(|_| "invalid policy seed offset")?;
        let roots: Vec<Value> =
            serde_json::from_str(&fs::read_to_string(&args[1]).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        // COACH_RESAMPLE=1: eval mode, a fresh shuffle per root (coach/common.rs).
        let resample = env::var("COACH_RESAMPLE").is_ok_and(|v| v == "1");
        let mut out = vec![];
        for (i, root) in roots.into_iter().enumerate() {
            let doc = serde_json::from_value(root).map_err(|e| format!("root {i}: {e}"))?;
            out.push(
                trial(
                    doc,
                    0x434f414348_u64.wrapping_add(offset).wrapping_add(i as u64),
                    resample.then(|| offset.wrapping_add(i as u64)),
                )
                .map_err(|e| format!("root {i}: {e}"))?,
            );
        }
        Ok(json!({"policy":"visible-card-priority-v1","trials":out}))
    })();
    match result {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sts_sim::hot::RngStream;

    #[test]
    fn hidden_draw_order_and_game_rng_cannot_change_policy_action() {
        let corpus: Value =
            serde_json::from_str(include_str!("../fixtures/exact_solve_corpus_v1.json")).unwrap();
        let doc: CanonicalStateV2 =
            serde_json::from_value(corpus["rows"][0]["entry"].clone()).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&doc).unwrap();
        let state = HotBoundary::from_canonical(&doc, &catalog).unwrap();
        let mut hidden = state.clone();
        hidden.piles.get_mut(PileId::Draw).make_mut().reverse();
        let mut game_rng = hidden.rng.get(RngStream::Ai);
        game_rng.counter += 13;
        hidden.rng.set(RngStream::Ai, game_rng);
        let mut buffer = LegalActionBuffer::new();
        let actions = engine::legal_actions_into(&state, &catalog, &mut buffer).to_vec();
        let hidden_actions = engine::legal_actions_into(&hidden, &catalog, &mut buffer).to_vec();
        assert_eq!(actions, hidden_actions);
        let mut left = Rng(7);
        let mut right = Rng(7);
        assert_eq!(
            choose(&actions, &state, &catalog, &mut left),
            choose(&hidden_actions, &hidden, &catalog, &mut right)
        );
    }

    #[test]
    fn damage_count_excludes_lethal_overkill() {
        assert_eq!(effective_damage(5, 40), 5);
        assert_eq!(effective_damage(0, 40), 0);
        assert_eq!(effective_damage(12, 3), 3);
    }
}
