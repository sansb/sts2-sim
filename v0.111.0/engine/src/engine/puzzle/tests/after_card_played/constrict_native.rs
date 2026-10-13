//! #3756: native traces of a choice inside Constrict's side-end damage,
//! replayed against this engine. The helpers are `after_card_played`'s.
//!
//! `fixtures/issue3756_native/*.json` were recorded from the archived
//! v0.111.0 `sts2.dll` (SHA-256 `9cb4f1ad…`) on the real
//! `ENCOUNTER.SLITHERING_STRANGLER_NORMAL`: the Strangler's own CONSTRICT
//! applies Constrict 3 on turn 1, the turn-2 layout is arranged, and the
//! game's own `EndPlayerTurnAction` runs with `TestCardSelector` as the
//! local selector. The README beside them says which mode each trace was
//! recorded in. [`replay`] rebuilds the root snapshot here, replays the
//! script, and reports, per recorded point, whether this engine's state
//! equals the game's and which legal answer reaches the game's next state,
//! from the warm state and from a cold reload.

// The replayer sits in a `tests` module of its own so the production-source
// censuses of `tests/hot_path_contract.rs`, which read a file up to its
// `mod tests {`, do not count its root builder as call sites.
mod tests {
    use super::super::*;

    const SIDE_END_POWERS: &[(&str, PowerId)] = &[
        ("NoDrawPower", PowerId::NoDraw),
        ("ThornsPower", PowerId::Thorns),
        ("BurstPower", PowerId::Burst),
        ("HellraiserPower", PowerId::Hellraiser),
        ("StrengthPower", PowerId::Strength),
        ("DexterityPower", PowerId::Dexterity),
        ("StratagemPower", PowerId::Stratagem),
        ("RagePower", PowerId::Rage),
        ("DoubleDamagePower", PowerId::DoubleDamage),
        ("DarkEmbracePower", PowerId::DarkEmbrace),
        ("DoomPower", PowerId::Doom),
        ("RetainHandPower", PowerId::RetainHand),
        ("ConsumingShadowPower", PowerId::ConsumingShadow),
        ("DisintegrationPower", PowerId::Disintegration),
        ("JugglingPower", PowerId::Juggling),
        ("PlatingPower", PowerId::Plating),
        ("TangledPower", PowerId::Tangled),
        ("VulnerablePower", PowerId::Vulnerable),
        ("WeakPower", PowerId::Weak),
    ];

    fn side_end_token(name: &str) -> Option<crate::hot::AfterSideTurnEndPowerToken> {
        use crate::hot::AfterSideTurnEndPowerToken as Token;
        Some(match name {
            "BurstPower" => Token::Burst,
            "ConsumingShadowPower" => Token::ConsumingShadow,
            "DarkEmbracePower" => Token::DarkEmbrace,
            "DoomPower" => Token::Doom,
            "DoubleDamagePower" => Token::DoubleDamage,
            "HellraiserPower" => Token::Hellraiser,
            "JugglingPower" => Token::Juggling,
            "NoDrawPower" => Token::NoDraw,
            "RagePower" => Token::Rage,
            "RetainHandPower" => Token::RetainHand,
            "TangledPower" => Token::Tangled,
            _ => return None,
        })
    }

    const SIDE_END_MOVES: [&str; 3] = ["CONSTRICT", "THWACK", "LASH"];

    fn side_end_snapshot(
        state: &HotState,
        catalog: &Catalog,
        started: i64,
        finished: i64,
    ) -> Value {
        let mut ours = snapshot(state, catalog, started, finished);
        let mut powers = ours["powers"].as_array().unwrap().clone();
        for row in &mut powers {
            let name = row[0].as_str().unwrap().to_owned();
            if let Some((native, _)) = SIDE_END_POWERS
                .iter()
                .find(|(_, id)| format!("ENGINE:{id:?}") == name)
            {
                row[0] = json!(native);
            }
        }
        if state.fanouts.constrict_amount() != 0 {
            powers.push(json!(["ConstrictPower", state.fanouts.constrict_amount()]));
        }
        if state.temp_strength != 0 {
            powers.push(json!(["FlexPotionPower", state.temp_strength]));
            let total = state.powers.value(PowerId::Strength) + state.temp_strength;
            powers.retain(|row| row[0] != "StrengthPower");
            if total != 0 {
                powers.push(json!(["StrengthPower", total]));
            }
        }
        powers.sort_by_key(|row| row[0].as_str().unwrap().to_owned());
        ours["powers"] = Value::Array(powers);
        for monster in ours["mons"].as_array_mut().unwrap() {
            monster.as_object_mut().unwrap().remove("id");
        }
        ours.as_object_mut().unwrap().remove("relics");
        // An unplayable card's cost is -1 here and 0 in the probe's GetResolved.
        for cost in ours["hand_costs"].as_array_mut().unwrap() {
            if cost.as_i64() == Some(-1) {
                *cost = json!(0);
            }
        }
        ours["stars"] = json!(state.stars);
        ours["turn"] = json!(state.turn);
        let next = state
            .monsters
            .iter()
            .find(|monster| monster.kind == MonsterKind::SlitheringStrangler && monster.hp > 0)
            .and_then(|monster| monster.random_ai.next())
            .map(|index| SIDE_END_MOVES[usize::from(index)]);
        ours["next"] = json!(next);
        ours
    }

    fn side_end_normalized(native: &Value) -> Value {
        let mut out = serde_json::Map::new();
        for key in [
            "hp",
            "max_hp",
            "block",
            "energy",
            "powers",
            "mons",
            "hand_costs",
            "piles",
            "plays_started",
            "plays_finished",
            "over",
            "rng",
            "stars",
            "turn",
        ] {
            out.insert(key.to_owned(), native[key].clone());
        }
        let mut out = Value::Object(out);
        for monster in out["mons"].as_array_mut().unwrap() {
            monster.as_object_mut().unwrap().remove("id");
        }
        let next = native["monster_ai"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "MONSTER.SLITHERING_STRANGLER" && row["dead"] == false)
            .map(|row| row["next"].clone())
            .unwrap_or(Value::Null);
        out["next"] = next;
        out
    }

    fn side_end_cold(state: &HotState, catalog: &Catalog) -> Result<(HotState, Catalog), String> {
        let wire =
            HotBoundary::try_to_canonical(state, catalog).map_err(|e| format!("export: {e:?}"))?;
        let loaded_catalog =
            HotBoundary::catalog_from_canonical(&wire).map_err(|e| format!("catalog: {e:?}"))?;
        let loaded = HotBoundary::from_canonical(&wire, &loaded_catalog)
            .map_err(|e| format!("import: {e:?}"))?;
        let again = HotBoundary::try_to_canonical(&loaded, &loaded_catalog)
            .map_err(|e| format!("re-export: {e:?}"))?;
        if again != wire {
            return Err("re-export differs".to_owned());
        }
        admit(&wire, &loaded, &loaded_catalog).map_err(|e| format!("ADMISSION: {e:?}"))?;
        Ok((loaded, loaded_catalog))
    }

    fn side_end_root(spec: &Value, native: &Value) -> Result<Root, String> {
        let mut identities = std::collections::BTreeSet::new();
        for label in spec["deck"].as_array().unwrap() {
            identities.insert(identity_of(label.as_str().unwrap()));
        }
        let order = native["power_order"].as_array().unwrap();
        let has = |name: &str| order.iter().any(|row| row[0] == name);
        let mut builder = CatalogBuilder::new();
        builder.intern_private_constrict_strangler().unwrap();
        if has("HellraiserPower") {
            builder.mark_live_hellraiser_reachable();
        }
        if has("DarkEmbracePower") {
            builder.mark_live_dark_embrace_reachable();
        }
        let atoms = identities
            .iter()
            .map(|identity| (*identity, builder.intern_reachable(*identity).unwrap()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut relics = vec![RelicId::RelicBurningBlood];
        for relic in spec["relics"].as_array().unwrap() {
            let name = format!("RELIC.{}", relic.as_str().unwrap());
            relics.push(RelicId::from_str(&name).unwrap_or_else(|| panic!("{name}")));
        }
        builder.set_relics_ordered(&relics, true).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        let int = |key: &str| native[key].as_i64().unwrap();
        state.hp = i32::try_from(int("hp")).unwrap();
        state.max_hp = i32::try_from(int("max_hp")).unwrap();
        state.block = i32::try_from(int("block")).unwrap();
        state.energy = i16::try_from(int("energy")).unwrap();
        state.stars = i16::try_from(int("stars")).unwrap();
        state.turn = i16::try_from(int("turn")).unwrap();
        state.player_phase = PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        assert!(state.fanouts.set_ascension(0));
        if let Some(used) = native["puzzle_used"].as_array().unwrap().first() {
            state.fanouts.set_puzzle_armed(used == false);
        }
        for (name, stream) in STREAMS {
            let live = &native["rng"][name];
            let mut words = [0; 4];
            for (word, row) in words.iter_mut().zip(live["words"].as_array().unwrap()) {
                *word = row[1].as_u64().unwrap();
            }
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words,
                    counter: live["counter"].as_u64().unwrap(),
                },
            );
        }
        // The Strangler first: Constrict's boundary needs its living owner.
        let ai = native["monster_ai"].as_array().unwrap();
        let mut monsters = Vec::new();
        for row in native["mons"].as_array().unwrap() {
            let ai_row = ai.iter().find(|a| a["id"] == row["id"]).unwrap();
            if ai_row["kind"] != "MONSTER.SLITHERING_STRANGLER" {
                return Err(format!("unsupported living monster {}", ai_row["kind"]));
            }
            assert!(row["powers"].as_array().unwrap().is_empty());
            let hp = i32::try_from(row["hp"].as_i64().unwrap()).unwrap();
            let mut monster = HotMonster::new(MonsterKind::SlitheringStrangler, hp);
            monster.max_hp = i32::try_from(row["max_hp"].as_i64().unwrap()).unwrap();
            monster.block = i32::try_from(row["block"].as_i64().unwrap()).unwrap();
            let index = |name: &Value| {
                u8::try_from(SIDE_END_MOVES.iter().position(|m| name == *m).unwrap()).unwrap()
            };
            let mut machine = crate::hot::RandomAiState::new();
            assert!(machine.set_next(Some(index(&ai_row["next"]))));
            let log = ai_row["log"]
                .as_array()
                .unwrap()
                .iter()
                .map(index)
                .collect::<Vec<_>>();
            assert!(machine.set_log(&log));
            monster.random_ai = machine;
            monsters.push(monster);
        }
        state.monsters = std::sync::Arc::new(monsters);
        let orbs = native["orbs"].as_array().unwrap();
        let capacity = u8::try_from(native["orb_capacity"].as_u64().unwrap()).unwrap();
        if capacity != 0 {
            state.orbs.set_slots(capacity);
            state.orbs.set_orbs(
                orbs.iter()
                    .map(|orb| {
                        assert_eq!(orb, "LightningOrb");
                        crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Lightning, None)
                            .unwrap()
                    })
                    .collect(),
            );
        }
        for row in order {
            let name = row[0].as_str().unwrap();
            let amount = i32::try_from(row[1].as_i64().unwrap()).unwrap();
            if name == "FlexPotionPower" {
                state.temp_strength = amount;
                state
                    .fanouts
                    .register_after_side_turn_end_power(
                        crate::hot::AfterSideTurnEndPowerToken::TemporaryStrength,
                    )
                    .unwrap();
                continue;
            }
            if name == "ConstrictPower" {
                assert!(state.fanouts.set_constrict_amount(amount));
                state
                    .fanouts
                    .register_after_side_turn_end_power(
                        crate::hot::AfterSideTurnEndPowerToken::Constrict,
                    )
                    .unwrap();
                continue;
            }
            let Some((_, id)) = SIDE_END_POWERS.iter().find(|(native, _)| *native == name) else {
                return Err(format!("unmapped root power {name}"));
            };
            state.powers.set(*id, SlotWire::Int, amount);
            if let Some(token) = side_end_token(name) {
                state
                    .fanouts
                    .register_after_side_turn_end_power(token)
                    .unwrap();
            }
        }
        if state.temp_strength != 0 {
            // The engine holds the temporary contribution apart from Strength.
            let permanent = state.powers.value(PowerId::Strength) - state.temp_strength;
            state
                .powers
                .set(PowerId::Strength, SlotWire::Int, permanent);
        }
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        if has("DarkEmbracePower") {
            assert!(
                state
                    .fanouts
                    .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
            );
        }
        let mut uid = 1;
        for (name, pile) in PILES {
            for label in native["piles"][name].as_array().unwrap() {
                let atom = atoms[&identity_of(label.as_str().unwrap())];
                state.piles.get_mut(pile).make_mut().push(card(uid, atom));
                uid += 1;
            }
        }
        state.next_card_uid = uid;
        let (state, catalog) = side_end_cold(&state, &catalog)?;
        Ok(Root {
            state,
            catalog,
            started: native["plays_started"].as_i64().unwrap(),
            finished: native["plays_finished"].as_i64().unwrap(),
        })
    }

    fn side_end_diff(ours: &Value, theirs: &Value) -> String {
        let mut out = String::new();
        for (key, value) in ours.as_object().unwrap() {
            if *value != theirs[key] {
                if key == "rng" {
                    for (stream, live) in value.as_object().unwrap() {
                        if *live != theirs[key][stream] {
                            out.push_str(&format!(
                                "\n      rng.{stream}: ours counter {} theirs {}",
                                live["counter"], theirs[key][stream]["counter"]
                            ));
                        }
                    }
                } else {
                    out.push_str(&format!(
                        "\n      {key}: ours {value}\n      {key}: game {}",
                        theirs[key]
                    ));
                }
            }
        }
        out
    }

    fn side_end_matches(
        root: &Root,
        state: &HotState,
        started: &Started,
        native: &Value,
    ) -> Result<(), String> {
        let mut ours = side_end_snapshot(
            state,
            &root.catalog,
            root.started + started.total(),
            root.finished,
        );
        let mut theirs = side_end_normalized(native);
        if theirs["over"] == json!(true) {
            for key in [
                "piles",
                "hand_costs",
                "plays_started",
                "plays_finished",
                "turn",
                "energy",
                "block",
                "stars",
                "powers",
            ] {
                ours[key] = Value::Null;
                theirs[key] = Value::Null;
            }
        }
        if ours == theirs {
            Ok(())
        } else {
            Err(side_end_diff(&ours, &theirs))
        }
    }

    /// Replay one trace; the report is one line per recorded point.
    fn replay(text: &str) -> Vec<String> {
        let file: Value = serde_json::from_str(text).unwrap();
        let spec = &file["spec"];
        let runs = file["runs"].as_array().unwrap();
        let mut report = Vec::new();
        for (run_index, run) in runs.iter().enumerate() {
            let trace = run["trace"].as_array().unwrap();
            let entry = |label: &str| trace.iter().find(|entry| entry["label"] == label);
            let mut root: Option<Root> = None;
            let mut state: Option<HotState> = None;
            let mut started = Started::default();
            let who = format!("run {run_index} answers {}", run["answers"]);
            'script: for (step_index, step) in spec["script"].as_array().unwrap().iter().enumerate()
            {
                let kind = step[0].as_str().unwrap();
                if kind == "snap" {
                    let label = step[1].as_str().unwrap();
                    let Some(native) = entry(label) else {
                        report.push(format!("{who}: no native snap {label}"));
                        break 'script;
                    };
                    let native = &native["snap"];
                    if root.is_none() {
                        match side_end_root(spec, native) {
                            Ok(built) => {
                                state = Some(built.state.clone());
                                root = Some(built);
                            }
                            Err(error) => {
                                report.push(format!("{who}: ROOT {error}"));
                                break 'script;
                            }
                        }
                    }
                    match side_end_matches(
                        root.as_ref().unwrap(),
                        state.as_ref().unwrap(),
                        &started,
                        native,
                    ) {
                        Ok(()) => report.push(format!("{who}: snap {label} equal")),
                        Err(diff) => {
                            report.push(format!("{who}: snap {label} DIFFERS{diff}"));
                            break 'script;
                        }
                    }
                    continue;
                }
                if kind != "play" || root.is_none() {
                    continue;
                }
                let root = root.as_ref().unwrap();
                let card_name = step[1].as_str().unwrap();
                let label = format!("{step_index}:{card_name}");
                let current = state.take().unwrap();
                let play = if card_name == "@END" {
                    Action::EndTurn
                } else {
                    Action::Play {
                        uid: uid_of(&current, &root.catalog, card_name),
                        target: step[2].as_u64().map(|_| 0),
                        selection: SelectionRef::NONE,
                    }
                };
                crate::engine::play::take_test_body_starts();
                let mut next = match apply_action(&current, &root.catalog, &play) {
                    Ok(applied) => applied.state,
                    Err(error) => {
                        let native_parks = trace
                            .iter()
                            .filter(|e| {
                                e["label"]
                                    .as_str()
                                    .unwrap()
                                    .starts_with(&format!("{label}:parked-"))
                            })
                            .count();
                        report.push(format!(
                            "{who}: {label} ENGINE REFUSES {error:?} (game: {native_parks} parks)"
                        ));
                        break 'script;
                    }
                };
                started.observe();
                let mut park = 0;
                loop {
                    let parked = entry(&format!("{label}:parked-{park}"));
                    let Some(parked) = parked else {
                        if next.pending.is_some() {
                            report
                                .push(format!("{who}: {label} ENGINE EXTRA CHOICE at park {park}"));
                            break 'script;
                        }
                        break;
                    };
                    if next.pending.is_none() && parked["snap"]["over"] == json!(true) {
                        // The probe's selector was asked after the combat had ended;
                        // the answer changes nothing in the game.
                        match side_end_matches(root, &next, &started, &parked["snap"]) {
                        Ok(()) => report.push(format!("{who}: {label} game asks its selector with the combat already over; engine ended with no choice, states equal")),
                        Err(diff) => report.push(format!("{who}: {label} combat over in the game at the selector; engine has no choice and DIFFERS{diff}")),
                    }
                        break;
                    }
                    if next.pending.is_none() {
                        report.push(format!(
                            "{who}: {label} ENGINE HAS NO CHOICE {park} (game parks)"
                        ));
                        break 'script;
                    }
                    match side_end_matches(root, &next, &started, &parked["snap"]) {
                        Ok(()) => report.push(format!("{who}: {label} park {park} equal")),
                        Err(diff) => {
                            report.push(format!("{who}: {label} park {park} DIFFERS{diff}"));
                            break 'script;
                        }
                    }
                    let (loaded, loaded_catalog) = match side_end_cold(&next, &root.catalog) {
                        Ok(pair) => pair,
                        Err(error) => {
                            report.push(format!(
                                "{who}: {label} park {park} COLD RELOAD FAILS {error}"
                            ));
                            break 'script;
                        }
                    };
                    let legal = legal_actions(&next, &root.catalog);
                    if legal_actions(&loaded, &loaded_catalog) != legal {
                        report.push(format!(
                            "{who}: {label} park {park} cold legal actions differ"
                        ));
                    }
                    let after = entry(&format!("{label}:parked-{}", park + 1))
                        .or_else(|| entry(&format!("{label}:done")))
                        .unwrap();
                    let mut matched = None;
                    let mut diffs = Vec::new();
                    for action in &legal {
                        crate::engine::play::take_test_body_starts();
                        let warm = match apply_action(&next, &root.catalog, action) {
                            Ok(applied) => applied.state,
                            Err(error) => {
                                diffs.push(format!("\n    {action:?}: ENGINE REFUSES {error:?}"));
                                continue;
                            }
                        };
                        let mut candidate = started.clone();
                        candidate.observe();
                        match apply_action(&loaded, &loaded_catalog, action) {
                            Ok(from_cold) => {
                                let a = HotBoundary::try_to_canonical(
                                    &from_cold.state,
                                    &loaded_catalog,
                                )
                                .map(|d| d.differential_digest());
                                let b = HotBoundary::try_to_canonical(&warm, &root.catalog)
                                    .map(|d| d.differential_digest());
                                if a.is_err() || b.is_err() || a.ok() != b.ok() {
                                    diffs.push(format!(
                                        "\n    {action:?}: cold/warm digests differ"
                                    ));
                                }
                            }
                            Err(error) => {
                                diffs.push(format!("\n    {action:?}: COLD REFUSES {error:?}"))
                            }
                        }
                        match side_end_matches(root, &warm, &candidate, &after["snap"]) {
                            Ok(()) => {
                                if matched.is_none() {
                                    matched = Some((*action, warm, candidate));
                                }
                            }
                            Err(diff) => diffs.push(format!("\n    {action:?}:{diff}")),
                        }
                    }
                    if matched.is_none() {
                        // A multi-card pick may be several engine answers: search
                        // answer sequences up to four deep (warm only).
                        let mut work = vec![(next.clone(), Vec::<Action>::new())];
                        'search: while let Some((at_state, path)) = work.pop() {
                            if path.len() >= 4 {
                                continue;
                            }
                            for action in legal_actions(&at_state, &root.catalog) {
                                let Ok(applied) = apply_action(&at_state, &root.catalog, &action)
                                else {
                                    continue;
                                };
                                let mut candidate = started.clone();
                                candidate.observe();
                                let mut path = path.clone();
                                path.push(action);
                                if side_end_matches(
                                    root,
                                    &applied.state,
                                    &candidate,
                                    &after["snap"],
                                )
                                .is_ok()
                                {
                                    report.push(format!("{who}: {label} park {park}: matched by the answer sequence {path:?}"));
                                    matched = Some((action, applied.state, candidate));
                                    break 'search;
                                }
                                if applied.state.pending.is_some() && !path.is_empty() {
                                    work.push((applied.state, path));
                                }
                            }
                        }
                    }
                    crate::engine::play::take_test_body_starts();
                    let Some((action, warm, candidate)) = matched else {
                        report.push(format!(
                        "{who}: {label} park {park}: NO ENGINE ANSWER ({} legal) REACHES THE GAME'S NEXT STATE{}",
                        legal.len(),
                        diffs.concat()
                    ));
                        break 'script;
                    };
                    report.push(format!(
                    "{who}: {label} park {park}: {} legal, answer {action:?} reaches the game's next state",
                    legal.len()
                ));
                    next = warm;
                    started = candidate;
                    park += 1;
                }
                let done = entry(&format!("{label}:done")).unwrap();
                match side_end_matches(root, &next, &started, &done["snap"]) {
                    Ok(()) => report.push(format!(
                        "{who}: {label} done equal (game state {})",
                        done["state"]
                    )),
                    Err(diff) => {
                        report.push(format!("{who}: {label} done DIFFERS{diff}"));
                        break 'script;
                    }
                }
                started.settle();
                if next.pending.is_none()
                    && !next.history.over
                    && let Err(error) = side_end_cold(&next, &root.catalog)
                {
                    report.push(format!("{who}: {label} done COLD RELOAD FAILS {error}"));
                }
                state = Some(next);
            }
        }
        report
    }

    /// Every line of a clean replay is one of these.
    fn assert_clean(report: &[String]) {
        for line in report {
            assert!(
                line.ends_with(" equal")
                    || line.contains(" done equal (game state ")
                    || line.ends_with("reaches the game's next state")
                    || line.ends_with("engine ended with no choice, states equal"),
                "{}",
                report.join("\n")
            );
        }
    }

    fn count(report: &[String], needle: &str) -> usize {
        report.iter().filter(|line| line.contains(needle)).count()
    }

    /// #3756, the case the first pass inferred: a later listener ends the
    /// combat by killing the LAST ENEMY while the Stratagem pick waits.
    /// Consuming Shadow's side-end evoke of a Lightning orb kills the only
    /// enemy; the game shows its selector with the combat already over and the
    /// answer changes nothing. The engine ends with no choice, in the game's
    /// state, for each of the three recorded answers.
    #[test]
    fn a_later_listener_that_kills_the_last_enemy_ends_the_combat_unanswered() {
        let report = replay(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/issue3756_native/a6_shadow_kill.json"
        )));
        assert_clean(&report);
        assert_eq!(count(&report, "snap root equal"), 3);
        assert_eq!(
            count(&report, "engine ended with no choice, states equal"),
            3
        );
        assert_eq!(count(&report, "snap end equal"), 3);
    }

    /// #3756: Constrict's hit beside an earlier Dark Embrace's waiting prompt
    /// is refused only where the Puzzle fires. With 50 Block the hit loses no
    /// HP, the Puzzle stays armed, and the turn end runs through as the game
    /// records it: the park on Dark Embrace's pick, its answer, and the next
    /// turn, warm and from cold reloads.
    #[test]
    fn a_fully_blocked_constrict_hit_beside_a_waiting_prompt_runs_through() {
        let report = replay(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/issue3756_native/r4_blocked_grown.json"
        )));
        assert_clean(&report);
        assert_eq!(count(&report, "@END park 0 equal"), 1);
        assert_eq!(count(&report, "@END done equal"), 1);
        assert_eq!(count(&report, "snap end equal"), 1);
    }

    /// The same root without the Block: the hit fires the Puzzle beside the
    /// waiting prompt, which refuses by name (the game draws the Puzzle's cards
    /// out of the prompt's options, and that is not built).
    #[test]
    fn an_unblocked_constrict_hit_beside_a_waiting_prompt_refuses_where_the_puzzle_fires() {
        let report = replay(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/issue3756_native/r4b_grown_unblocked.json"
        )));
        assert_eq!(
            report.last().unwrap(),
            "run 0 answers [\"0+1+2\"]: 11:@END ENGINE REFUSES \
         PowerOrderNotModeled(\"deferred hook action choice source moved\") (game: 1 parks)",
            "{}",
            report.join("\n")
        );
        assert_clean(&report[..report.len() - 1]);
    }

    /// #3756, #3931: a select that resolves without a prompt under Constrict's
    /// listener. Hellraiser plays a drawn Seeker Strike whose pile holds one
    /// candidate, with Rage 3 as a later side-end listener. The two native
    /// traces start from the same root and disagree: headless, the listener's
    /// Draws finish before Rage is removed and the owner is at 71 HP at the
    /// next turn's first prompt; with `Cmd.Wait` running as a timer Rage is
    /// removed first and the owner is at 65. Neither order is taken as
    /// the game's (`turn::side_end_prompt_starts_later_listeners`): the engine
    /// refuses the turn end by name.
    #[test]
    fn an_unprompted_select_under_constrict_refuses_where_the_two_native_modes_disagree() {
        let headless: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/issue3756_native/d3_seeker_one_rage.headless.json"
        )))
        .unwrap();
        let waits: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/issue3756_native/d3_seeker_one_rage.waits.json"
        )))
        .unwrap();
        let snap = |file: &Value, suffix: &str| {
            file["runs"][0]["trace"]
                .as_array()
                .unwrap()
                .iter()
                .rev()
                .find(|entry| {
                    let label = entry["label"].as_str().unwrap_or("");
                    label == suffix || label.ends_with(suffix)
                })
                .unwrap()["snap"]
                .clone()
        };
        // One root, two recorded orders.
        assert_eq!(
            side_end_normalized(&snap(&headless, "root")),
            side_end_normalized(&snap(&waits, "root"))
        );
        assert_eq!(snap(&headless, "@END:parked-0")["hp"], 71);
        assert_eq!(snap(&waits, "@END:parked-0")["hp"], 65);
        for file in [&headless, &waits] {
            let root = side_end_root(&file["spec"], &snap(file, "root")).unwrap();
            assert_eq!(
                apply_action(&root.state, &root.catalog, &Action::EndTurn)
                    .map(|_| ())
                    .unwrap_err(),
                EngineRefusal::PowerOrderNotModeled(
                    crate::engine::puzzle::AUTO_RESOLVED_IN_DEFERRED_LISTENER
                )
            );
        }
    }
}
