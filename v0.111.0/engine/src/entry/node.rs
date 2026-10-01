//! Which node the save is standing on, what kind it is, and which encounter
//! that node schedules.
//!
//! Oracle: `live_coach.current_node`, `live_coach.node_type` and
//! `live_coach.next_encounter` in `sim/v0.111.0/python/tools/live_coach.py`.
//! The same global numbering is spelled a second time as
//! `mcr_validate.global_node_index`.

use crate::entry::refusal::EntryRefusal;
use crate::entry::save::{MAP_POINT_TYPES, SerializedRun};

/// The node's index in the run's **global** numbering.
///
/// `visited_map_coords` resets at each act transition, so its length alone
/// gives only the per-act index; prior acts' node lists stay behind in
/// `map_point_history`, one completed list per finished act. The per-fight
/// Encounter stream's `total_floor` is derived from this value + 1, so a
/// per-act shortfall silently reseeds every act-2-or-later random-starter
/// encounter — the #826 `MonsterAi` chain drift, where a
/// `SCROLLS_OF_BITING_NORMAL` fight rolled starter 0 instead of the game's 2.
///
/// The oracle reaches `save["map_point_history"]` unguarded, so its absence on
/// an act-2+ save is an exception there; here it is the named
/// [`EntryRefusal::MissingMapPointHistory`]. The corpus carries
/// `map_point_history` on 3,027 of 3,092 schema-20 saves, so the gap is real.
pub fn current_node(run: &SerializedRun) -> Result<i64, EntryRefusal> {
    let act = run.act_index();
    let prior: usize = if act == 0 {
        0
    } else {
        let history = run
            .map_point_history
            .as_deref()
            .ok_or(EntryRefusal::MissingMapPointHistory)?;
        history
            .iter()
            .take(act)
            .map(|entry| entry.as_array().map_or(0, Vec::len))
            .sum()
    };
    if run.visited_map_coords.is_empty() {
        return Err(EntryRefusal::EmptyVisitedMapCoords);
    }
    Ok((prior + run.visited_map_coords.len() - 1) as i64)
}

/// The map-point kind of the current node, from the current act's saved map.
///
/// Returns the point's `type` lowercased, matching the oracle. The oracle
/// returns `None` both when the act has no `saved_map` and when the current
/// coordinate is not among its `points`; those are separated here, because the
/// second is a real "this save cannot say what kind of node it is" and the
/// first is an absent map.
///
/// Note what the corpus shows about the point list: `boss` and `ancient`
/// points live in `saved_map.boss` / `saved_map.start`, not in
/// `saved_map.points`, so a boss node reaches this function as
/// [`EntryRefusal::NodeNotOnSavedMap`] — exactly as the oracle reaches `None`
/// there — and the caller supplies the kind explicitly.
pub fn node_type(run: &SerializedRun) -> Result<Option<String>, EntryRefusal> {
    let act = run.act_index();
    let acts = run.acts.len();
    let act_model = run
        .acts
        .get(act)
        .ok_or(EntryRefusal::ActIndexOutOfRange { index: act, acts })?;
    let current = *run
        .visited_map_coords
        .last()
        .ok_or(EntryRefusal::EmptyVisitedMapCoords)?;
    let Some(map) = act_model.saved_map.as_ref() else {
        return Ok(None);
    };
    for point in &map.points {
        if point.coord != Some(current) {
            continue;
        }
        let raw = point.point_type.clone().unwrap_or_default();
        let kind = raw.to_lowercase();
        if !MAP_POINT_TYPES.contains(&kind.as_str()) {
            return Err(EntryRefusal::UnknownNodeType(raw));
        }
        return Ok(Some(kind));
    }
    Err(EntryRefusal::NodeNotOnSavedMap {
        col: current.col,
        row: current.row,
    })
}

/// The current node's saved **map point** type (`MapPoint.PointType`),
/// independent of any caller-supplied node kind (#3162).
///
/// [`node_type`] answers "which kind of room does this node schedule", and a
/// caller that already knows the room kind skips it — the eval census and the
/// review both pass `--node-type`, and an `unknown` (`?`) point that rolled a
/// combat reaches the opening as `"monster"`. A relic body that reads the
/// point rather than the room (`Planisphere`, `IRunState::get_CurrentMapPoint`
/// then `MapPoint::get_PointType`) needs the point itself, so this reads it
/// again from the act's saved map, including the three points
/// [`node_type`] leaves to the caller (`boss`, `second_boss`, `start`).
///
/// Never a refusal: `None` means the save does not say — no saved map, a
/// coordinate on none of its points, or a spelling outside
/// `MAP_POINT_TYPES` — and a reader that needs the point refuses on `None`
/// itself.
pub fn map_point_type(run: &SerializedRun) -> Option<String> {
    let map = run.acts.get(run.act_index())?.saved_map.as_ref()?;
    let current = *run.visited_map_coords.last()?;
    map.points
        .iter()
        .chain(map.boss.iter())
        .chain(map.second_boss.iter())
        .chain(map.start.iter())
        .find(|point| point.coord == Some(current))
        .and_then(|point| point.point_type.as_deref())
        .map(str::to_lowercase)
        .filter(|kind| MAP_POINT_TYPES.contains(&kind.as_str()))
}

/// The encounter the current act schedules for a node of this kind.
///
/// Combat nodes carry their encounter in the act's room lists, indexed by the
/// act's own visited counter. Event-room fights report map node type `unknown`
/// and their encounter is recorded only by a LATER save's `pre_finished_room`,
/// which is a different save than this one — so they refuse here and the
/// caller supplies `--encounter`.
pub fn next_encounter(run: &SerializedRun, kind: &str) -> Result<String, EntryRefusal> {
    let act = run.act_index();
    let acts = run.acts.len();
    let rooms = &run
        .acts
        .get(act)
        .ok_or(EntryRefusal::ActIndexOutOfRange { index: act, acts })?
        .rooms;
    match kind {
        "elite" => pick(
            "elite_encounter_ids",
            &rooms.elite_encounter_ids,
            rooms.elite_encounters_visited,
        ),
        "boss" => rooms
            .boss_id
            .clone()
            .ok_or_else(|| EntryRefusal::NoEncounterForNode {
                node_type: kind.to_string(),
            }),
        "monster" => pick(
            "normal_encounter_ids",
            &rooms.normal_encounter_ids,
            rooms.normal_encounters_visited,
        ),
        other => Err(EntryRefusal::NoEncounterForNode {
            node_type: other.to_string(),
        }),
    }
}

fn pick(list: &'static str, ids: &[String], index: usize) -> Result<String, EntryRefusal> {
    ids.get(index)
        .cloned()
        .ok_or(EntryRefusal::EncounterIndexOutOfRange {
            list,
            index,
            len: ids.len(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::save::SerializedMapPoint;

    fn run(text: &str) -> SerializedRun {
        SerializedRun::parse(text).unwrap()
    }

    const ACT_ZERO: &str = r#"{
        "schema_version": 20,
        "current_act_index": 0,
        "visited_map_coords": [{"col": 3, "row": 0}, {"col": 3, "row": 1}],
        "acts": [{"id": "ACT.ONE",
                  "rooms": {"normal_encounter_ids": ["ENCOUNTER.TOADPOLES_WEAK",
                                                     "ENCOUNTER.MAWLER_NORMAL"],
                            "normal_encounters_visited": 1,
                            "elite_encounter_ids": ["ENCOUNTER.TERROR_EEL_ELITE"],
                            "elite_encounters_visited": 0,
                            "boss_id": "ENCOUNTER.WATERFALL_GIANT_BOSS"},
                  "saved_map": {"points": [{"coord": {"col": 3, "row": 1},
                                            "type": "monster", "children": []}]}}],
        "rng": {"seed": "ZPJHU3WSH2", "rngs": {}},
        "players": []
    }"#;

    #[test]
    fn act_zero_node_index_is_the_per_act_index() {
        assert_eq!(current_node(&run(ACT_ZERO)).unwrap(), 1);
    }

    #[test]
    fn a_later_act_adds_every_prior_acts_history_length() {
        let text = ACT_ZERO
            .replace(r#""current_act_index": 0"#, r#""current_act_index": 1"#)
            .replace(
                r#""rng": {"seed""#,
                r#""map_point_history": [[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17], []],
                   "rng": {"seed""#,
            );
        let mut save = run(&text);
        // Two acts so the index is in range.
        save.acts.push(save.acts[0].clone());
        assert_eq!(current_node(&save).unwrap(), 17 + 2 - 1);
    }

    #[test]
    fn a_later_act_without_history_refuses_by_name() {
        let text = ACT_ZERO.replace(r#""current_act_index": 0"#, r#""current_act_index": 1"#);
        assert_eq!(
            current_node(&run(&text)),
            Err(EntryRefusal::MissingMapPointHistory)
        );
    }

    #[test]
    fn node_type_reads_the_saved_map_point() {
        assert_eq!(
            node_type(&run(ACT_ZERO)).unwrap().as_deref(),
            Some("monster")
        );
    }

    /// `map_point_type` reads the point itself: a listed point, the boss
    /// point `node_type` leaves to the caller, and `None` (never a refusal)
    /// for an off-map coordinate, an unknown spelling or no saved map.
    #[test]
    fn map_point_type_reads_the_point_and_never_refuses() {
        assert_eq!(map_point_type(&run(ACT_ZERO)).as_deref(), Some("monster"));
        let unknown = ACT_ZERO.replace(r#""type": "monster""#, r#""type": "Unknown""#);
        assert_eq!(map_point_type(&run(&unknown)).as_deref(), Some("unknown"));
        let elsewhere = Some(crate::entry::save::Coord { col: 0, row: 0 });
        let mut off_map = run(ACT_ZERO);
        off_map.acts[0].saved_map.as_mut().unwrap().points[0].coord = elsewhere;
        assert_eq!(map_point_type(&off_map), None);
        let mut boss = off_map.clone();
        boss.acts[0].saved_map.as_mut().unwrap().boss = Some(SerializedMapPoint {
            coord: Some(crate::entry::save::Coord { col: 3, row: 1 }),
            point_type: Some("boss".to_string()),
            can_modify: None,
            children: Vec::new(),
        });
        assert_eq!(map_point_type(&boss).as_deref(), Some("boss"));
        let misspelled = ACT_ZERO.replace(r#""type": "monster""#, r#""type": "dungeon""#);
        assert_eq!(map_point_type(&run(&misspelled)), None);
        let mut mapless = run(ACT_ZERO);
        mapless.acts[0].saved_map = None;
        assert_eq!(map_point_type(&mapless), None);
    }

    #[test]
    fn a_coordinate_off_the_saved_map_refuses_rather_than_defaulting() {
        let text = ACT_ZERO.replace(r#"{"col": 3, "row": 1}, {"col": 3, "row": 1}"#, "");
        let text = text.replace(
            r#""visited_map_coords": [{"col": 3, "row": 0}, {"col": 3, "row": 1}]"#,
            r#""visited_map_coords": [{"col": 6, "row": 9}]"#,
        );
        assert_eq!(
            node_type(&run(&text)),
            Err(EntryRefusal::NodeNotOnSavedMap { col: 6, row: 9 })
        );
    }

    #[test]
    fn encounters_come_from_the_acts_visited_counters() {
        let save = run(ACT_ZERO);
        assert_eq!(
            next_encounter(&save, "monster").unwrap(),
            "ENCOUNTER.MAWLER_NORMAL"
        );
        assert_eq!(
            next_encounter(&save, "elite").unwrap(),
            "ENCOUNTER.TERROR_EEL_ELITE"
        );
        assert_eq!(
            next_encounter(&save, "boss").unwrap(),
            "ENCOUNTER.WATERFALL_GIANT_BOSS"
        );
    }

    #[test]
    fn an_event_node_has_no_scheduled_encounter() {
        assert_eq!(
            next_encounter(&run(ACT_ZERO), "unknown"),
            Err(EntryRefusal::NoEncounterForNode {
                node_type: "unknown".to_string()
            })
        );
    }

    #[test]
    fn a_visited_counter_past_the_list_refuses_rather_than_wrapping() {
        let text = ACT_ZERO.replace(
            r#""normal_encounters_visited": 1"#,
            r#""normal_encounters_visited": 9"#,
        );
        assert_eq!(
            next_encounter(&run(&text), "monster"),
            Err(EntryRefusal::EncounterIndexOutOfRange {
                list: "normal_encounter_ids",
                index: 9,
                len: 2,
            })
        );
    }
}
