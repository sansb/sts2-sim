//! #3756: a player choice that opens inside Centennial Puzzle's Draw under
//! Constrict's `AfterSideTurnEnd` damage.
//!
//! Native order (headless harness, v0.111.0, `EndPlayerTurnAction`; see
//! `turn::side_end_prompt_starts_later_listeners`): the later
//! ordinary side-end listeners run before the choice is asked, and the rest
//! of the Puzzle's Draws, the Late pass and the enemy turn follow the answer.

// The cases sit in a `tests` module of their own so the production-source
// censuses of `tests/hot_path_contract.rs`, which read a file up to its
// `mod tests {`, do not count these fixtures as call sites.
mod tests {
    use super::super::{cold, constrict, every_answer_to_terminal, parked_draw_depth, plain};
    use crate::catalog::{CardAtom, Catalog, CatalogBuilder};
    use crate::engine::admission::PHASE_ORDINARY_ACTIONS;
    use crate::engine::turn::{STRANGLER_CONSTRICT_INDEX, STRANGLER_THWACK_INDEX};
    use crate::engine::{Action, EngineRefusal, apply_action};
    use crate::hot::{
        AfterSideTurnEndPowerToken as Token, CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotCard, HotMonster,
        HotState, PileId, RngStream, RngStreamState,
    };
    use crate::ids::{CardId, MonsterKind, PowerId, RelicId};
    use crate::powers::SlotWire;

    struct Fixture {
        state: HotState,
        catalog: Catalog,
        atoms: Vec<CardAtom>,
    }

    /// A Slithering Strangler about to Thwack, an armed Centennial Puzzle and
    /// `cards` interned in order. Constrict is not applied yet: the caller
    /// registers the side-end powers in the order under test.
    fn fixture(cards: &[CardId], relics: &[RelicId]) -> Fixture {
        fixture_with_live(cards, relics, &[])
    }

    /// [`fixture`], with `live` powers (Hellraiser, Dark Embrace) marked
    /// reachable in the catalog, applied at 1 and registered first, in order.
    fn fixture_with_live(cards: &[CardId], relics: &[RelicId], live: &[PowerId]) -> Fixture {
        let mut fixture = fixture_inner(cards, relics, live);
        for power in live {
            fixture.state.powers.set(*power, SlotWire::Int, 1);
            match power {
                PowerId::Hellraiser => register(&mut fixture.state, Token::Hellraiser),
                PowerId::DarkEmbrace => {
                    assert!(
                        fixture
                            .state
                            .fanouts
                            .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
                    );
                }
                other => panic!("unsupported live power {other:?}"),
            }
        }
        fixture
    }

    fn fixture_inner(cards: &[CardId], relics: &[RelicId], live: &[PowerId]) -> Fixture {
        let mut builder = CatalogBuilder::new();
        builder.intern_private_constrict_strangler().unwrap();
        if live.contains(&PowerId::Hellraiser) {
            builder.mark_live_hellraiser_reachable();
        }
        if live.contains(&PowerId::DarkEmbrace) {
            builder.mark_live_dark_embrace_reachable();
        }
        let atoms = cards
            .iter()
            .map(|id| builder.intern_reachable(plain(*id)).unwrap())
            .collect();
        builder.set_relics_ordered(relics, true).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 40;
        state.exact_piles = true;
        state
            .fanouts
            .set_puzzle_armed(relics.contains(&RelicId::RelicCentennialPuzzle));
        for stream in [
            RngStream::Rng,
            RngStream::Sel,
            RngStream::Targets,
            RngStream::Ai,
        ] {
            state.rng.set(
                stream,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        let mut owner = HotMonster::new(MonsterKind::SlitheringStrangler, 55);
        owner.max_hp = 55;
        let mut ai = crate::hot::RandomAiState::new();
        assert!(ai.set_next(Some(STRANGLER_THWACK_INDEX)));
        assert!(ai.set_log(&[STRANGLER_CONSTRICT_INDEX, STRANGLER_THWACK_INDEX]));
        owner.random_ai = ai;
        state.monsters_mut().push(owner);
        Fixture {
            state,
            catalog,
            atoms,
        }
    }

    fn burst(state: &mut HotState) {
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        state
            .fanouts
            .register_after_side_turn_end_power(Token::Burst)
            .unwrap();
    }

    fn put(state: &mut HotState, pile: PileId, cards: &[(u32, CardAtom)]) {
        state
            .piles
            .get_mut(pile)
            .make_mut()
            .extend(cards.iter().map(|(uid, atom)| HotCard {
                uid: *uid,
                atom: *atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
    }

    fn uids(state: &HotState, pile: PileId) -> Vec<u32> {
        state
            .piles
            .get(pile)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect()
    }

    /// Stratagem 1 against a four-card reshuffle inside the Puzzle's first Draw,
    /// with Burst registered before or after Constrict.
    fn stratagem_fixture(burst_first: bool) -> Fixture {
        stratagem_fixture_with(burst_first, None)
    }

    /// Where a live Dark Embrace sits relative to Constrict in the side-end
    /// ledger. Its tally comes from a Dazed the turn end exhausts.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Dark {
        BeforeConstrict,
        AfterConstrict,
    }

    fn stratagem_fixture_with(burst_first: bool, dark: Option<Dark>) -> Fixture {
        let cards = [
            CardId::DefendIronclad,
            CardId::Bash,
            CardId::ShrugItOff,
            CardId::TrueGrit,
            CardId::Dazed,
        ];
        let relics = [RelicId::RelicCentennialPuzzle, RelicId::RelicLunarPastry];
        let mut fixture = match dark {
            None => fixture(&cards, &relics),
            Some(_) => fixture_with_live(&cards, &relics, &[PowerId::DarkEmbrace]),
        };
        let atoms = fixture.atoms.clone();
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 1);
        if dark == Some(Dark::BeforeConstrict) {
            register(&mut fixture.state, Token::DarkEmbrace);
        }
        if burst_first {
            burst(&mut fixture.state);
        }
        constrict(&mut fixture.state, 3);
        if !burst_first {
            burst(&mut fixture.state);
        }
        if dark == Some(Dark::AfterConstrict) {
            register(&mut fixture.state, Token::DarkEmbrace);
        }
        if dark.is_some() {
            put(&mut fixture.state, PileId::Hand, &[(5, atoms[4])]);
        }
        put(
            &mut fixture.state,
            PileId::Hand,
            &[(1, atoms[0]), (2, atoms[1]), (3, atoms[2]), (4, atoms[3])],
        );
        fixture
    }

    /// The native `strat_burst_after` / `strat_burst_before` traces: at the
    /// paused choice Constrict has hit for 3, Burst is gone and Lunar Pastry's
    /// Star is gained on either side of Constrict, the Puzzle has drawn nothing,
    /// and the enemy has not acted. After the answer the pick and the Puzzle's
    /// three Draws are in Hand and the enemy turn has run.
    #[test]
    fn stratagem_pick_under_constrict_runs_the_later_listeners_first() {
        for burst_first in [false, true] {
            let fixture = stratagem_fixture(burst_first);
            let (state, catalog) = cold(&fixture.state, &fixture.catalog);
            // Admission rehearses the root's own actions.
            crate::engine::turn::take_side_end_deferrals_started_for_test();
            let parked = apply_action(&state, &catalog, &Action::EndTurn)
                .unwrap()
                .state;
            assert_eq!(
                crate::engine::turn::take_side_end_deferrals_started_for_test(),
                1
            );
            assert!(parked.pending.is_some());
            assert!(parked_draw_depth(&parked).is_some());
            assert!(
                parked
                    .pending
                    .as_deref()
                    .unwrap()
                    .stratagem_draw_record(&parked.frames)
                    .is_some()
            );
            assert_eq!(parked.hp, 47);
            assert_eq!(parked.powers.value(PowerId::Burst), 0);
            assert_eq!(parked.stars, 1);
            assert!(uids(&parked, PileId::Hand).is_empty());
            assert_eq!(parked.piles.get(PileId::Draw).len(), 4);
            assert!(parked.player_side_active);
            assert_eq!(parked.turn, state.turn);
            assert_eq!(parked.monsters[0].hp, 55);

            let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
            assert_eq!(visited, 1);
            assert_eq!(terminals.len(), 4);
            let mut hands = std::collections::BTreeSet::new();
            for terminal in &terminals {
                assert_eq!(terminal.turn, state.turn + 1);
                assert!(terminal.hp < 47, "Thwack follows the answer");
                assert_eq!(terminal.hp, terminals[0].hp);
                assert_eq!(terminal.stars, 1);
                assert_eq!(terminal.powers.value(PowerId::Burst), 0);
                assert_eq!(terminal.cards_drawn_combat, 3);
                let hand = uids(terminal, PileId::Hand);
                assert_eq!(hand.len(), 4);
                hands.insert(hand);
            }
            assert_eq!(hands.len(), 4, "the pick leads the Hand");
        }
    }

    fn register(state: &mut HotState, token: Token) {
        state
            .fanouts
            .register_after_side_turn_end_power(token)
            .unwrap();
    }

    fn end_turn(state: &HotState, catalog: &Catalog) -> Result<HotState, EngineRefusal> {
        crate::engine::turn::take_side_end_deferrals_started_for_test();
        apply_action(state, catalog, &Action::EndTurn).map(|applied| applied.state)
    }

    /// The native `seeker` trace: a Hellraiser-played Seeker Strike's pick
    /// inside the Puzzle's first Draw. At the prompt the Strike has hit and is
    /// in Play, Burst is gone and the Star is gained.
    #[test]
    fn seeker_strike_pick_under_constrict_runs_the_later_listeners_first() {
        let mut fixture = fixture_with_live(
            &[
                CardId::SeekerStrike,
                CardId::DefendIronclad,
                CardId::Bash,
                CardId::ShrugItOff,
                CardId::TrueGrit,
            ],
            &[RelicId::RelicCentennialPuzzle, RelicId::RelicLunarPastry],
            &[PowerId::Hellraiser],
        );
        let atoms = fixture.atoms.clone();
        constrict(&mut fixture.state, 3);
        burst(&mut fixture.state);
        put(
            &mut fixture.state,
            PileId::Draw,
            &[
                (1, atoms[0]),
                (2, atoms[1]),
                (3, atoms[2]),
                (4, atoms[3]),
                (5, atoms[4]),
            ],
        );
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let parked = end_turn(&state, &catalog).unwrap();
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            1
        );
        assert!(parked.pending.is_some());
        assert!(parked_draw_depth(&parked).is_some());
        assert!(
            parked
                .pending_card_play(parked.pending.as_deref().unwrap())
                .is_some()
        );
        assert_eq!(parked.hp, 47);
        assert_eq!(parked.monsters[0].hp, 55 - 9);
        assert_eq!(uids(&parked, PileId::Play), [1]);
        assert_eq!(uids(&parked, PileId::Draw), [2, 3, 4, 5]);
        assert_eq!(parked.powers.value(PowerId::Burst), 0);
        assert_eq!(parked.stars, 1);
        assert!(parked.player_side_active);

        let (terminals, visited) = every_answer_to_terminal(&parked, &catalog);
        assert!(visited >= 1);
        assert!(!terminals.is_empty());
        for terminal in &terminals {
            assert!(terminal.turn > state.turn || terminal.history.over);
        }
    }

    /// The native `strat_second_draw` trace: the Puzzle's first Draw takes the
    /// lone Draw-pile card, and its second reshuffles into the pick.
    #[test]
    fn a_prompt_in_the_second_puzzle_draw_runs_the_later_listeners_first() {
        let mut fixture = stratagem_fixture(false);
        let strike = fixture.atoms[0];
        put(&mut fixture.state, PileId::Draw, &[(9, strike)]);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let parked = end_turn(&state, &catalog).unwrap();
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            1
        );
        assert!(parked.pending.is_some());
        assert_eq!(uids(&parked, PileId::Hand), [9]);
        assert_eq!(parked.piles.get(PileId::Draw).len(), 4);
        assert_eq!(parked.powers.value(PowerId::Burst), 0);
        assert_eq!(parked.stars, 1);
        assert!(parked.player_side_active);
        let (terminals, _) = every_answer_to_terminal(&parked, &catalog);
        assert_eq!(terminals.len(), 4);
        for terminal in &terminals {
            assert_eq!(terminal.turn, state.turn + 1);
            // The first Draw's card, the pick, the two later Draws, and the
            // one card left for the next Hand Draw.
            assert_eq!(terminal.piles.get(PileId::Hand).len(), 5);
            assert_eq!(terminal.cards_drawn_combat, 4);
        }
    }

    /// Three Hellraiser plays of a Seeker Strike with one candidate and then
    /// none (the headless `seeker_auto` trace). No select prompts, but each
    /// one signals the listener's context, which releases the walk in play;
    /// the order of the Draws against the later listeners is then not
    /// established (`turn::side_end_prompt_starts_later_listeners`), so the
    /// turn end refuses by name when the listener settles. With the Puzzle
    /// spent there is no select and the turn ends.
    #[test]
    fn a_select_that_needs_no_prompt_refuses_by_name() {
        let fixture = unprompted_seeker_fixture(&[]);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        crate::engine::turn::take_side_end_deferrals_started_for_test();
        assert_eq!(
            end_turn(&state, &catalog).unwrap_err(),
            EngineRefusal::PowerOrderNotModeled(
                crate::engine::puzzle::AUTO_RESOLVED_IN_DEFERRED_LISTENER
            )
        );
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            0
        );
        let mut spent = fixture.state.clone();
        spent.fanouts.set_puzzle_armed(false);
        let (spent, spent_catalog) = cold(&spent, &fixture.catalog);
        let next = end_turn(&spent, &spent_catalog).unwrap();
        assert_eq!(next.powers.value(PowerId::Burst), 0);
        assert_eq!(next.stars, 1);
    }

    /// Hellraiser, a Seeker Strike on top of a one-card Draw pile, and
    /// `discard` behind it: the Puzzle's first Draw plays the Strike, whose
    /// select has one candidate and takes it without a prompt.
    fn unprompted_seeker_fixture(discard: &[u32]) -> Fixture {
        let mut fixture = fixture_with_live(
            &[CardId::SeekerStrike, CardId::Bash],
            &[RelicId::RelicCentennialPuzzle, RelicId::RelicLunarPastry],
            &[PowerId::Hellraiser],
        );
        let atoms = fixture.atoms.clone();
        constrict(&mut fixture.state, 3);
        burst(&mut fixture.state);
        put(
            &mut fixture.state,
            PileId::Draw,
            &[(1, atoms[0]), (2, atoms[1])],
        );
        let discard = discard
            .iter()
            .map(|uid| (*uid, atoms[0]))
            .collect::<Vec<_>>();
        put(&mut fixture.state, PileId::Discard, &discard);
        fixture
    }

    /// The same unprompted select followed by a prompt of the same listener:
    /// the Puzzle's second Draw reshuffles four Seeker Strikes, plays one,
    /// and its select has three candidates. The walk was released at the
    /// first select, so this prompt does not start the later listeners: it
    /// refuses by name, and nothing is published.
    #[test]
    fn a_prompt_after_a_select_that_needed_none_refuses_by_name() {
        let fixture = unprompted_seeker_fixture(&[3, 4, 5]);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        crate::engine::turn::take_side_end_deferrals_started_for_test();
        assert_eq!(
            end_turn(&state, &catalog).unwrap_err(),
            EngineRefusal::PowerOrderNotModeled(
                crate::engine::puzzle::AUTO_RESOLVED_IN_DEFERRED_LISTENER
            )
        );
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            0
        );
    }

    /// A second prompt of the same listener is sequential in its hook action:
    /// the later listeners are not started again. Stratagem's pick opens the
    /// first Draw, and a Seeker Strike drawn after it picks in turn.
    #[test]
    fn a_second_prompt_of_the_paused_listener_starts_nothing() {
        let mut fixture = fixture_with_live(
            &[CardId::SeekerStrike],
            &[RelicId::RelicCentennialPuzzle, RelicId::RelicLunarPastry],
            &[PowerId::Hellraiser],
        );
        let seeker = fixture.atoms[0];
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 1);
        constrict(&mut fixture.state, 3);
        burst(&mut fixture.state);
        put(
            &mut fixture.state,
            PileId::Hand,
            &[(1, seeker), (2, seeker), (3, seeker), (4, seeker)],
        );
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let parked = end_turn(&state, &catalog).unwrap();
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            1
        );
        assert!(
            parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .is_some()
        );
        let (loaded, loaded_catalog) = cold(&parked, &catalog);
        let first = crate::engine::legal_actions(&loaded, &loaded_catalog)[0];
        crate::engine::turn::take_side_end_deferrals_started_for_test();
        let second = apply_action(&loaded, &loaded_catalog, &first)
            .unwrap()
            .state;
        // The re-execution starts the later listeners once, at the first prompt;
        // the Strike's pick is the listener's second.
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            1
        );
        assert!(
            second
                .pending_card_play(second.pending.as_deref().unwrap())
                .is_some()
        );
        assert!(parked_draw_depth(&second).is_some());
        assert!(second.player_side_active);
        assert_eq!(second.powers.value(PowerId::Burst), 0);
        assert_eq!(second.stars, 1);
        cold(&second, &catalog);
    }

    /// The native `strat_doom_lethal` trace: a later Doom kills the owner while
    /// the pick waits. The combat is over and the pick is never applied.
    #[test]
    fn a_later_listener_that_kills_the_owner_ends_the_combat_unanswered() {
        let mut fixture = stratagem_fixture(true);
        fixture.state.powers.set(PowerId::Doom, SlotWire::Int, 99);
        register(&mut fixture.state, Token::Doom);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let dead = end_turn(&state, &catalog).unwrap();
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            1
        );
        assert!(dead.history.over);
        assert!(dead.hp <= 0);
        assert!(dead.pending.is_none());
        assert!(dead.frames.is_empty());
        assert!(uids(&dead, PileId::Hand).is_empty());
        assert_eq!(dead.piles.get(PileId::Draw).len(), 4);
        assert_eq!(dead.cards_drawn_combat, 0);
        assert_eq!(dead.monsters[0].hp, 55);
    }

    /// The native `strat_dark_after` trace: a later Dark Embrace draws the top
    /// card while the pick waits, and the pick keeps the options it opened
    /// with. Rust's prompt reads the live pile, so this refuses by name.
    #[test]
    fn a_later_listener_that_moves_the_option_source_refuses_by_name() {
        let fixture = stratagem_fixture_with(true, Some(Dark::AfterConstrict));
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert_eq!(
            end_turn(&state, &catalog).unwrap_err(),
            EngineRefusal::PowerOrderNotModeled(crate::engine::hook_action::CHOICE_SOURCE_MOVED)
        );
    }

    /// A later Dark Embrace whose own Draw prompts (a Seeker Strike it draws
    /// under Hellraiser) would be a second paused listener: refused by name.
    #[test]
    fn a_later_listener_that_prompts_refuses_by_name() {
        let mut fixture = fixture_with_live(
            &[CardId::SeekerStrike, CardId::Dazed],
            &[RelicId::RelicCentennialPuzzle],
            &[PowerId::Hellraiser, PowerId::DarkEmbrace],
        );
        let (seeker, dazed) = (fixture.atoms[0], fixture.atoms[1]);
        fixture
            .state
            .powers
            .set(PowerId::Stratagem, SlotWire::Int, 1);
        constrict(&mut fixture.state, 3);
        register(&mut fixture.state, Token::DarkEmbrace);
        put(
            &mut fixture.state,
            PileId::Hand,
            &[
                (1, seeker),
                (2, seeker),
                (3, seeker),
                (4, seeker),
                (5, dazed),
            ],
        );
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert_eq!(
            end_turn(&state, &catalog).unwrap_err(),
            EngineRefusal::PowerOrderNotModeled(crate::engine::hook_action::SECOND_HOOK_ACTION)
        );
    }

    /// An earlier Dark Embrace's waiting prompt beside an armed Puzzle under
    /// Constrict: the native `strat_dark_before` trace draws the Puzzle's three
    /// cards out of the pick's options. Refused by name before the hit.
    #[test]
    fn constrict_beside_an_earlier_waiting_prompt_refuses_by_name() {
        let fixture = stratagem_fixture_with(false, Some(Dark::BeforeConstrict));
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        assert_eq!(
            end_turn(&state, &catalog).unwrap_err(),
            EngineRefusal::PowerOrderNotModeled(crate::engine::hook_action::CHOICE_SOURCE_MOVED)
        );
        // With the Puzzle spent the same turn parks on Dark Embrace's own pick.
        let mut spent = fixture.state.clone();
        spent.fanouts.set_puzzle_armed(false);
        let (spent, spent_catalog) = cold(&spent, &fixture.catalog);
        let parked = end_turn(&spent, &spent_catalog).unwrap();
        assert!(parked.pending.is_some());
        assert_eq!(parked.hp, 47);
    }

    /// The listener guard: armed, a park refuses under the backstop wall and a
    /// select that needs no prompt is counted and refuses when the listener
    /// settles; once its later peers have started, a park is published and a
    /// later select is sequential, not counted; a nested ordinary listener is
    /// unaffected; leaving restores the outer listener.
    #[test]
    fn the_side_end_deferral_guard_backstops_a_park_until_it_has_started() {
        use super::super::super::{
            AFTER_DEATH_WALL, AFTER_SIDE_TURN_END_WALL, AUTO_RESOLVED_IN_DEFERRED_LISTENER,
            CONSTRICT_PUZZLE_WALL, CarrierGuard, DeferredChoiceListener, PARK_SITE,
            SideEndDeferral, deferred_choice_wall, park, side_end_deferral,
            side_end_deferral_released_unprompted, side_end_deferral_started,
        };
        use crate::engine::hook_action::note_unprompted_select;
        let fixture = stratagem_fixture(false);
        let state = fixture.state.clone();
        let _carrier = CarrierGuard::enter();
        assert_eq!(side_end_deferral(), SideEndDeferral::Inactive);
        // Outside such a listener the start is a no-op.
        side_end_deferral_started();
        assert_eq!(side_end_deferral(), SideEndDeferral::Inactive);
        assert!(!side_end_deferral_released_unprompted());
        {
            let _outer = DeferredChoiceListener::enter(Some(AFTER_SIDE_TURN_END_WALL));
            let listener = DeferredChoiceListener::enter_side_end_deferral(CONSTRICT_PUZZLE_WALL);
            assert_eq!(side_end_deferral(), SideEndDeferral::Armed);
            assert_eq!(
                park(&state),
                Err(EngineRefusal::PowerOrderNotModeled(CONSTRICT_PUZZLE_WALL))
            );
            assert!(!side_end_deferral_released_unprompted());
            {
                let _inner = DeferredChoiceListener::enter(Some(AFTER_DEATH_WALL));
                assert_eq!(side_end_deferral(), SideEndDeferral::Inactive);
                assert_eq!(
                    park(&state),
                    Err(EngineRefusal::PowerOrderNotModeled(AFTER_DEATH_WALL))
                );
                // The nested listener's select is its own.
                note_unprompted_select(true);
                assert!(!side_end_deferral_released_unprompted());
            }
            assert_eq!(side_end_deferral(), SideEndDeferral::Armed);
            assert!(!side_end_deferral_released_unprompted());
            side_end_deferral_started();
            assert_eq!(side_end_deferral(), SideEndDeferral::Started);
            assert_eq!(deferred_choice_wall(), None);
            assert_eq!(park(&state), Err(EngineRefusal::MalformedArgs(PARK_SITE)));
            // A select of the started listener is sequential in its hook
            // action.
            note_unprompted_select(true);
            assert!(!side_end_deferral_released_unprompted());
            assert_eq!(listener.settle(Ok(7)), Ok(7));
            assert_eq!(side_end_deferral(), SideEndDeferral::Inactive);
            assert_eq!(deferred_choice_wall(), Some(AFTER_SIDE_TURN_END_WALL));
        }
        assert_eq!(deferred_choice_wall(), None);
        // Armed, a select that needs no prompt releases the walk: the next
        // prompt sees it, and so does the listener when it settles.
        {
            let listener = DeferredChoiceListener::enter_side_end_deferral(CONSTRICT_PUZZLE_WALL);
            note_unprompted_select(false);
            assert!(!side_end_deferral_released_unprompted());
            note_unprompted_select(true);
            assert!(side_end_deferral_released_unprompted());
            assert_eq!(
                listener.settle(Ok(7)),
                Err(EngineRefusal::PowerOrderNotModeled(
                    AUTO_RESOLVED_IN_DEFERRED_LISTENER
                ))
            );
        }
        assert!(!side_end_deferral_released_unprompted());
    }

    /// A receipt-owned Draw that is not directly above the pass's owner frame
    /// is not one this model carries: the backstop wall names it.
    #[test]
    fn a_prompt_not_directly_above_the_owner_frame_refuses_under_the_wall() {
        use super::super::super::{CONSTRICT_PUZZLE_WALL, DeferredChoiceListener};
        use crate::engine::turn::side_end_prompt_starts_later_listeners as prompt;
        let fixture = stratagem_fixture(false);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let mut parked = end_turn(&state, &catalog).unwrap();
        let mut events = Vec::new();
        // Outside the listener the same call is a no-op.
        assert_eq!(prompt(&mut parked, &catalog, &mut events, 1, 0), Ok(false));
        let _listener = DeferredChoiceListener::enter_side_end_deferral(CONSTRICT_PUZZLE_WALL);
        assert_eq!(
            prompt(&mut parked, &catalog, &mut events, 1, 0),
            Err(EngineRefusal::PowerOrderNotModeled(CONSTRICT_PUZZLE_WALL))
        );
        // Directly above it, with the pass already started: a no-op.
        assert_eq!(prompt(&mut parked, &catalog, &mut events, 2, 0), Ok(false));
        // And with no prompt at all.
        parked.pending = None;
        assert_eq!(prompt(&mut parked, &catalog, &mut events, 1, 0), Ok(false));
    }

    /// Iron Club's Draw nested in the Puzzle's Draw under Constrict's listener
    /// (#3756): the Puzzle draws a Strike, Hellraiser AutoPlays it as Iron
    /// Club's fourth card, Iron Club draws a Seeker Strike, and that Strike's
    /// pick opens above a CardPlay instead of directly above the pass's owner
    /// frame. Not built: the root refuses at admission by the wall's name,
    /// and the same turn end run past admission refuses at runtime under it.
    /// Without Iron Club the root admits, and so does one whose Puzzle is
    /// spent.
    #[test]
    fn an_iron_club_draw_nested_under_constrict_refuses_at_admission_and_at_runtime() {
        use super::super::super::{CONSTRICT_PUZZLE_WALL, root_refuses_under_for_test};
        let club_root = |relics: &[RelicId]| {
            let mut fixture = fixture_with_live(
                &[
                    CardId::StrikeIronclad,
                    CardId::SeekerStrike,
                    CardId::Bash,
                    CardId::DefendIronclad,
                    CardId::ShrugItOff,
                ],
                relics,
                &[PowerId::Hellraiser],
            );
            let atoms = fixture.atoms.clone();
            constrict(&mut fixture.state, 3);
            burst(&mut fixture.state);
            put(
                &mut fixture.state,
                PileId::Draw,
                &[
                    (1, atoms[0]),
                    (2, atoms[1]),
                    (3, atoms[2]),
                    (4, atoms[3]),
                    (5, atoms[4]),
                ],
            );
            fixture
        };
        let mut club = club_root(&[RelicId::RelicCentennialPuzzle, RelicId::RelicIronClub]);
        assert!(club.state.fanouts.set_iron_club_cards(3));
        assert!(root_refuses_under_for_test(
            &club.state,
            &club.catalog,
            CONSTRICT_PUZZLE_WALL
        ));
        assert_eq!(
            end_turn(&club.state, &club.catalog).unwrap_err(),
            EngineRefusal::PowerOrderNotModeled(CONSTRICT_PUZZLE_WALL)
        );
        assert_eq!(
            crate::engine::turn::take_side_end_deferrals_started_for_test(),
            0
        );
        let mut spent = club.state.clone();
        spent.fanouts.set_puzzle_armed(false);
        assert!(!root_refuses_under_for_test(
            &spent,
            &club.catalog,
            CONSTRICT_PUZZLE_WALL
        ));
        let plain = club_root(&[RelicId::RelicCentennialPuzzle]);
        assert!(!root_refuses_under_for_test(
            &plain.state,
            &plain.catalog,
            CONSTRICT_PUZZLE_WALL
        ));
    }

    /// The published owner record: every captured listener started, no Dark
    /// Embrace child, one Constrict row. The boundary refuses the forged
    /// neighbours of that shape on load, and refuses to export an owner that has
    /// neither a Dark Embrace child nor a Constrict row.
    #[test]
    fn the_parked_owner_record_round_trips_and_refuses_its_forged_neighbours() {
        use crate::boundary::HotBoundary;
        let fixture = stratagem_fixture(true);
        let (state, catalog) = cold(&fixture.state, &fixture.catalog);
        let parked = end_turn(&state, &catalog).unwrap();
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let owner = &wire.continuations[1];
        assert_eq!(owner.frame_type, "AfterSideTurnEndPowerFrame");
        assert_eq!(owner.fields["dark_pending_uid"], serde_json::Value::Null);
        assert_eq!(
            owner.fields["ordinary_suffix_invoked"],
            serde_json::json!(true)
        );
        assert_eq!(owner.fields["cursor"], serde_json::json!(2));
        let rows = owner.fields["listeners"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][0], serde_json::json!("constrict"));
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        assert!(HotBoundary::from_canonical(&wire, &rebuilt_catalog).is_ok());

        let forge = |field: &str, value: serde_json::Value| {
            let mut forged = wire.clone();
            forged.continuations[1]
                .fields
                .insert(field.to_owned(), value);
            forged
        };
        for (field, value) in [
            ("cursor", serde_json::json!(1)),
            ("ordinary_suffix_invoked", serde_json::json!(false)),
            // A Dark Embrace child that no row names.
            ("dark_pending_uid", serde_json::json!(0)),
            ("dark_pending_uid", serde_json::json!("none")),
        ] {
            assert!(
                HotBoundary::from_canonical(&forge(field, value.clone()), &rebuilt_catalog)
                    .is_err(),
                "forged {field} {value}"
            );
        }
        // No Constrict row: the paused listener is not named.
        let mut no_constrict = wire.clone();
        {
            let rows = no_constrict.continuations[1]
                .fields
                .get_mut("listeners")
                .unwrap()
                .as_array_mut()
                .unwrap();
            rows.pop();
        }
        no_constrict.continuations[1]
            .fields
            .insert("cursor".to_owned(), serde_json::json!(1));
        assert!(HotBoundary::from_canonical(&no_constrict, &rebuilt_catalog).is_err());

        // Export: a fully invoked owner with neither a Dark Embrace child nor a
        // Constrict row is not a published shape.
        let mut hot = parked.clone();
        let Some(crate::frame::Frame::AfterSideTurnEndPower { record }) =
            hot.frames.as_slice().get(1).copied()
        else {
            panic!("owner frame");
        };
        let mut owned = hot
            .frames
            .after_side_turn_end_power(record)
            .unwrap()
            .to_owned();
        assert!(owned.ordinary_suffix_invoked && owned.dark_pending_uid.is_none());
        owned
            .listeners
            .retain(|listener| listener.object.token != Token::Constrict);
        owned.cursor = 1;
        let mut frames = crate::hot::Frames::default();
        assert!(frames.push_after_side_turn_end_power(&owned).is_some());
        hot.frames = frames;
        hot.pending = None;
        assert!(HotBoundary::try_to_canonical(&hot, &catalog).is_err());
    }
}
