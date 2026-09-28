//! The hook dispatch framework (PORT_PLAN.md D5).
//!
//! Python fires listener hooks by walking every relic and every power on every
//! creature at ~30 fire points. That is affordable in Python only because the
//! sim is not searched; the port cannot pay it per node. So the subscriber
//! lists are built **once per combat**, from the relics and powers actually
//! present, and live here — beside the hot state, inside the per-fight
//! [`crate::catalog::Catalog`], never inside [`crate::hot::HotState`] and never
//! cloned per search node (D3).
//!
//! # The taxonomy
//!
//! [`HookEvent`] is unified across the two Python event families rather than
//! split in two:
//!
//! * the 12 relic-template hooks of `combat_sim._RELIC_HOOKS_SUPPORTED`, fired
//!   by `_fire_relic_templates` (and read as modifiers by
//!   `_relic_modifier_total` for the two `Modify*` events);
//! * the power fan-outs — the `_powers_*` functions of `combat_sim` plus the
//!   two native card-event walks that carry the same ordered-power contract.
//!
//! One enum, because the *machinery* is identical (an ordered subscriber list
//! per event, a presence bit per event, one walk to fire) and only the payload
//! differs — and the payload is already carried by the fire site's arguments,
//! not by the event id. A split would have duplicated the table, the bitset and
//! the walk to express nothing. [`HookCategory`] recovers the distinction where
//! it matters, and gates a whole family with a single mask test.
//!
//! PORT_PLAN §3 D5 says "the 8 `_powers_*` fan-outs". The v0.111.0 tree defines
//! **five** functions (`grep '^def _powers_' combat_sim.py`:
//! `_powers_after_card_drawn`, `_powers_after_energy_spent`,
//! `_powers_after_block_gained`, `_powers_before_card_played`,
//! `_powers_after_card_played`) and two additional native ordered power walks,
//! `AfterCardExhausted` and `BeforeHandDraw`. The enum follows the tree, and
//! [`tests::the_power_fanouts_match_the_python_functions`] is where that claim
//! is re-checked rather than inherited.
//!
//! # Ordering is part of the spec
//!
//! Same-event relic subscribers fire in **relic inventory order**, not id
//! order: v0.110.1 `CombatState/<IterateHookListeners>d__69::MoveNext`
//! enumerates `Player.Relics` by ascending index, and
//! `versions/v0.111.0/solver/test_relic_dispatch_order.py` is the Python
//! ordering spec built on exactly that reading (schema>=19 saves record the
//! list, and `start_combat` refuses a same-hook peer whose order was not
//! recorded). [`HookTable::build`] therefore takes the inventory as an ordered
//! slice and preserves that order inside each event's list.
//!
//! # What is here at R0.6, and what is not
//!
//! The framework is the deliverable; breadth is wave work (D4). Every template
//! relic's hook program is **compiled** here at admission — hook names, guard
//! verbs and effect verbs all resolve to typed enums, and the per-fight form
//! carries no text — but no effect body is implemented, so a fight that
//! actually has a subscriber refuses by name at the fire point ([`HookTable::fire`])
//! and, before that, at the admission gate, which admits no relic at all.

use crate::catalog::{CompiledArg, Span, compile_args};
use crate::content_tables::{RelicHook, template_relic};
use crate::ids::{PowerId, RelicId};

/// Which Python event family an event belongs to.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HookCategory {
    /// `_RELIC_HOOKS_SUPPORTED` — fired by `_fire_relic_templates`, and read
    /// by `_relic_modifier_total` for the two `Modify*` events.
    RelicTemplate,
    /// The `_powers_*` fan-outs.
    PowerFanout,
}

macro_rules! hook_events {
    ($(($variant:ident, $name:literal, $category:ident)),+ $(,)?) => {
        /// One dispatch event: a relic-template hook or a power fan-out.
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum HookEvent {
            $(
                #[doc = concat!("`", $name, "`.")]
                $variant
            ),+
        }

        impl HookEvent {
            /// Number of events.
            pub const COUNT: usize = [$(stringify!($variant)),+].len();
            /// Every event, in declaration order.
            pub const ALL: [HookEvent; Self::COUNT] = [$(HookEvent::$variant),+];
            /// The Python name of each event: the hook name for a relic
            /// template hook, the fan-out function for a power event.
            pub const NAMES: [&'static str; Self::COUNT] = [$($name),+];
            /// Each event's family.
            pub const CATEGORIES: [HookCategory; Self::COUNT] =
                [$(HookCategory::$category),+];

            /// This event's Python name.
            pub const fn as_str(&self) -> &'static str {
                Self::NAMES[*self as usize]
            }

            /// This event's family.
            pub const fn category(&self) -> HookCategory {
                Self::CATEGORIES[*self as usize]
            }
        }
    };
}

hook_events! {
    // `_RELIC_HOOKS_SUPPORTED` (combat_sim.py, above `_fire_relic_templates`),
    // in the order that set literal lists them — which is also the order the
    // fire points run in over a turn.
    (BeforeCombatStart, "BeforeCombatStart", RelicTemplate),
    (AfterRoomEntered, "AfterRoomEntered", RelicTemplate),
    (BeforeSideTurnStart, "BeforeSideTurnStart", RelicTemplate),
    (AfterSideTurnStart, "AfterSideTurnStart", RelicTemplate),
    (AfterPlayerTurnStart, "AfterPlayerTurnStart", RelicTemplate),
    (AfterPlayerTurnStartLate, "AfterPlayerTurnStartLate", RelicTemplate),
    (AfterEnergyReset, "AfterEnergyReset", RelicTemplate),
    (AfterBlockCleared, "AfterBlockCleared", RelicTemplate),
    (BeforeSideTurnEnd, "BeforeSideTurnEnd", RelicTemplate),
    (AfterSideTurnEnd, "AfterSideTurnEnd", RelicTemplate),
    (ModifyMaxEnergy, "ModifyMaxEnergy", RelicTemplate),
    (ModifyHandDraw, "ModifyHandDraw", RelicTemplate),
    // The `_powers_*` fan-outs, named by their Python function, followed by
    // native card-event walks that use the same ordered-power machinery.
    (AfterCardDrawn, "_powers_after_card_drawn", PowerFanout),
    (AfterEnergySpent, "_powers_after_energy_spent", PowerFanout),
    (AfterBlockGained, "_powers_after_block_gained", PowerFanout),
    (BeforeCardPlayed, "_powers_before_card_played", PowerFanout),
    (AfterCardPlayed, "_powers_after_card_played", PowerFanout),
    (AfterCardExhausted, "AfterCardExhausted", PowerFanout),
    (BeforeHandDraw, "BeforeHandDraw", PowerFanout),
}

impl HookEvent {
    /// Parse a relic-template hook name.
    ///
    /// Only the [`HookCategory::RelicTemplate`] names parse: the power fan-outs
    /// are engine call sites, not content strings, so accepting their names
    /// here would invent a content vocabulary that does not exist.
    pub fn from_hook_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|event| event.category() == HookCategory::RelicTemplate && event.as_str() == name)
    }

    /// The bit this event occupies in a presence mask.
    const fn bit(self) -> u32 {
        1u32 << (self as u32)
    }
}

/// A relic-template guard verb (`_relic_cond_eval`).
///
/// Vocabulary: `content_tables::TEMPLATE_RELIC_COND_VERBS`, which the
/// generator publishes from `relic_templates.json` itself.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TemplateCond {
    /// `is_owner`.
    IsOwner,
    /// `own_side`.
    OwnSide,
    /// `room_type`.
    RoomType,
    /// `turn`.
    Turn,
}

impl TemplateCond {
    /// Every verb, ascending by name.
    pub const ALL: [TemplateCond; 4] = [
        TemplateCond::IsOwner,
        TemplateCond::OwnSide,
        TemplateCond::RoomType,
        TemplateCond::Turn,
    ];

    /// The Python verb.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::IsOwner => "is_owner",
            Self::OwnSide => "own_side",
            Self::RoomType => "room_type",
            Self::Turn => "turn",
        }
    }

    fn parse(verb: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|cond| cond.as_str() == verb)
    }
}

/// A relic-template effect verb: every step kind `_fire_relic_templates`
/// interprets — `energy`, `block`, `heal`, `stars`, `power_self`, `power_all`,
/// `damage_all`, plus the two modifier kinds `max_energy` / `draw_bonus` that
/// `_relic_modifier_total` consumes instead of applying.
///
/// The enum follows the **interpreter**, which is the contract a body has to
/// satisfy; `content_tables::TEMPLATE_RELIC_EFFECT_VERBS` publishes the eight
/// verbs this build's `relic_templates.json` actually uses, and `power_all` is
/// the one interpreted verb no current template reaches. Naming it here is not
/// speculation — it is a branch of the Python function being ported — and the
/// test below pins the containment rather than an equality that would break on
/// the first template that uses it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TemplateEffect {
    /// `block`.
    Block,
    /// `damage_all`.
    DamageAll,
    /// `draw_bonus` — a `ModifyHandDraw` modifier, never applied directly.
    DrawBonus,
    /// `energy`.
    Energy,
    /// `heal`.
    Heal,
    /// `max_energy` — a `ModifyMaxEnergy` modifier, never applied directly.
    MaxEnergy,
    /// `power_all`.
    PowerAll,
    /// `power_self`.
    PowerSelf,
    /// `stars`.
    Stars,
}

impl TemplateEffect {
    /// Every verb, ascending by name.
    pub const ALL: [TemplateEffect; 9] = [
        TemplateEffect::Block,
        TemplateEffect::DamageAll,
        TemplateEffect::DrawBonus,
        TemplateEffect::Energy,
        TemplateEffect::Heal,
        TemplateEffect::MaxEnergy,
        TemplateEffect::PowerAll,
        TemplateEffect::PowerSelf,
        TemplateEffect::Stars,
    ];

    /// The effect verbs with a modeled body. `PowerAll` remains outside the
    /// v0.111.0 compiled template surface and is not claimed.
    pub const IMPLEMENTED: &'static [TemplateEffect] = &[
        TemplateEffect::Block,
        TemplateEffect::DamageAll,
        TemplateEffect::DrawBonus,
        TemplateEffect::Energy,
        TemplateEffect::Heal,
        TemplateEffect::MaxEnergy,
        TemplateEffect::PowerSelf,
        TemplateEffect::Stars,
    ];

    /// The Python verb.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::DamageAll => "damage_all",
            Self::DrawBonus => "draw_bonus",
            Self::Energy => "energy",
            Self::Heal => "heal",
            Self::MaxEnergy => "max_energy",
            Self::PowerAll => "power_all",
            Self::PowerSelf => "power_self",
            Self::Stars => "stars",
        }
    }

    fn parse(verb: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|effect| effect.as_str() == verb)
    }
}

/// One compiled guard or effect: a typed verb plus typed arguments.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CompiledCond {
    /// The guard verb.
    pub verb: TemplateCond,
    /// Its arguments, in the hook table's argument arena.
    pub args: Span,
}

/// One compiled effect step.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CompiledEffect {
    /// The effect verb.
    pub verb: TemplateEffect,
    /// Its arguments, in the hook table's argument arena.
    pub args: Span,
}

/// A half-open range into one of the hook table's op arenas.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct OpRange {
    start: u32,
    len: u16,
}

impl OpRange {
    fn of(start: usize, len: usize) -> Self {
        Self {
            start: start as u32,
            len: len as u16,
        }
    }

    fn bounds(self) -> (usize, usize) {
        let start = self.start as usize;
        (start, start + self.len as usize)
    }
}

/// One `(conditions, effects)` rule of a subscribed hook.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CompiledRule {
    /// All guards must hold.
    pub conds: OpRange,
    /// Effects applied in order when they do.
    pub effects: OpRange,
}

/// Whose subscription this is.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HookSubject {
    /// A relic in the player's inventory.
    Relic(RelicId),
    /// A power, subscribed through the static power-fan-out table.
    Power(PowerId),
}

/// One subscriber of one event.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Subscriber {
    /// The event subscribed to.
    pub event: HookEvent,
    /// Who subscribed.
    pub subject: HookSubject,
    /// The subscriber's compiled rules (empty for a power subscription).
    pub rules: OpRange,
}

/// Which modeled powers listen to which fan-out event.
///
/// Deliberately **static**, not derived from the live state: a power's amount
/// changes mid-combat (a `buff_thorns` move grants Thorns on turn 2 to a
/// monster that entered with none), so a presence bitset built from entry
/// values would go stale the moment the engine did its job. Presence here means
/// "this fight's *content* can reach this listener", which is what an immutable
/// per-combat table is allowed to claim.
///
/// Empty in this slice, and not vacuously: `Strength`, `Thorns` and `Vuln` are
/// damage-pipeline modifiers (`damage_monster` / `monster_attack_player` read
/// them directly), not `_powers_*` listeners, so there is nothing to register.
pub const POWER_SUBSCRIPTIONS: &[(PowerId, HookEvent)] = &[];

/// Why a hook table could not be built.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HookRefusal {
    /// A template hook name outside `_RELIC_HOOKS_SUPPORTED`.
    UnknownHook(&'static str),
    /// A guard verb outside `_relic_cond_eval`'s vocabulary.
    UnknownCond(&'static str),
    /// An effect verb outside `_fire_relic_templates`'s vocabulary.
    UnknownEffect(&'static str),
}

impl std::fmt::Display for HookRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHook(name) => write!(f, "unknown relic hook {name:?}"),
            Self::UnknownCond(verb) => write!(f, "unknown relic guard verb {verb:?}"),
            Self::UnknownEffect(verb) => write!(f, "unknown relic effect verb {verb:?}"),
        }
    }
}

impl std::error::Error for HookRefusal {}

/// The per-combat subscriber tables: immutable, built once, read by index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookTable {
    subscribers: Vec<Subscriber>,
    ranges: Vec<(u32, u16)>,
    presence: u32,
    rules: Vec<CompiledRule>,
    conds: Vec<CompiledCond>,
    effects: Vec<CompiledEffect>,
    args: Vec<CompiledArg>,
    relics: Vec<RelicId>,
    relic_presence: [u64; RelicId::COUNT.div_ceil(64)],
    dispatch_ordered: bool,
}

impl HookTable {
    /// The empty table: no relic, no power, every event absent.
    pub fn empty() -> Self {
        Self {
            ranges: vec![(0, 0); HookEvent::COUNT],
            ..Self::default()
        }
    }

    /// Build the per-combat tables from the relic inventory, **in inventory
    /// order** (see the module docs: that order is the dispatch spec).
    ///
    /// Boundary-time work: it allocates, sorts, and compiles. Nothing here runs
    /// again for the life of the fight.
    pub fn build(relics: &[RelicId]) -> Result<Self, HookRefusal> {
        let mut table = Self::empty();
        table.relics = relics.to_vec();
        for relic in relics {
            let index = *relic as usize;
            table.relic_presence[index / 64] |= 1_u64 << (index % 64);
        }

        // Relic subscriptions, in inventory order, grouped by event. The group
        // walk is over `HookEvent::ALL` and the inner walk over the inventory,
        // so within an event the inventory order survives by construction —
        // a stable sort would have had the same effect with more room to be
        // subtly wrong.
        for event in HookEvent::ALL {
            let start = table.subscribers.len();
            for relic in relics {
                let Some(program) = template_relic(*relic) else {
                    continue;
                };
                for hook in program {
                    if HookEvent::from_hook_name(hook.hook)
                        .ok_or(HookRefusal::UnknownHook(hook.hook))?
                        != event
                    {
                        continue;
                    }
                    let rules = table.compile_hook(hook)?;
                    table.subscribers.push(Subscriber {
                        event,
                        subject: HookSubject::Relic(*relic),
                        rules,
                    });
                }
            }
            for (power, subscribed) in POWER_SUBSCRIPTIONS {
                if *subscribed == event {
                    table.subscribers.push(Subscriber {
                        event,
                        subject: HookSubject::Power(*power),
                        rules: OpRange::default(),
                    });
                }
            }
            let len = table.subscribers.len() - start;
            table.ranges[event as usize] = (start as u32, len as u16);
            if len > 0 {
                table.presence |= event.bit();
            }
        }
        Ok(table)
    }

    fn compile_hook(&mut self, hook: &'static RelicHook) -> Result<OpRange, HookRefusal> {
        let mut compiled: Vec<CompiledRule> = Vec::with_capacity(hook.rules.len());
        for rule in hook.rules {
            let cond_start = self.conds.len();
            for cond in rule.conds {
                let verb =
                    TemplateCond::parse(cond.verb).ok_or(HookRefusal::UnknownCond(cond.verb))?;
                let args = compile_args(cond.args, &mut self.args, &[]);
                self.conds.push(CompiledCond { verb, args });
            }
            let effect_start = self.effects.len();
            for effect in rule.effects {
                let verb = TemplateEffect::parse(effect.verb)
                    .ok_or(HookRefusal::UnknownEffect(effect.verb))?;
                let args = compile_args(effect.args, &mut self.args, &[]);
                self.effects.push(CompiledEffect { verb, args });
            }
            compiled.push(CompiledRule {
                conds: OpRange::of(cond_start, self.conds.len() - cond_start),
                effects: OpRange::of(effect_start, self.effects.len() - effect_start),
            });
        }
        let start = self.rules.len();
        self.rules.extend(compiled);
        Ok(OpRange::of(start, self.rules.len() - start))
    }

    /// Whether this event has any subscriber. One bit test.
    pub fn has(&self, event: HookEvent) -> bool {
        self.presence & event.bit() != 0
    }

    /// Whether any event of this family has a subscriber. One mask test —
    /// the point of the bitset: an absent family costs one branch, not a walk.
    pub fn any(&self, category: HookCategory) -> bool {
        let mask: u32 = HookEvent::ALL
            .into_iter()
            .filter(|event| event.category() == category)
            .map(HookEvent::bit)
            .fold(0, |mask, bit| mask | bit);
        self.presence & mask != 0
    }

    /// This event's subscribers, in dispatch order.
    pub fn subscribers(&self, event: HookEvent) -> &[Subscriber] {
        let (start, len) = self.ranges[event as usize];
        let start = start as usize;
        &self.subscribers[start..start + len as usize]
    }

    /// Whether the authenticated inventory owns one relic.
    ///
    /// Hand-authored relic bodies use this immutable bitset instead of
    /// walking the inventory at every damage/resource hook. It is built once
    /// with the rest of the hook table and never enters [`HotState`].
    #[inline]
    pub fn owns(&self, relic: RelicId) -> bool {
        let index = relic as usize;
        self.relic_presence[index / 64] & (1_u64 << (index % 64)) != 0
    }

    /// One subscriber's compiled rules.
    pub fn rules(&self, subscriber: &Subscriber) -> &[CompiledRule] {
        let (start, end) = subscriber.rules.bounds();
        &self.rules[start..end]
    }

    /// One rule's guards.
    pub fn conds(&self, rule: &CompiledRule) -> &[CompiledCond] {
        let (start, end) = rule.conds.bounds();
        &self.conds[start..end]
    }

    /// One rule's effects.
    pub fn effects(&self, rule: &CompiledRule) -> &[CompiledEffect] {
        let (start, end) = rule.effects.bounds();
        &self.effects[start..end]
    }

    /// A compiled op's arguments — typed, never text.
    pub fn args(&self, range: Span) -> &[CompiledArg] {
        let (start, end) = range.bounds();
        &self.args[start..end]
    }

    /// The relic inventory this table was built from, in dispatch order.
    pub fn relics(&self) -> &[RelicId] {
        &self.relics
    }

    /// Whether this relic inventory has authenticated dispatch-order provenance.
    pub fn dispatch_ordered(&self) -> bool {
        self.dispatch_ordered
    }

    /// Build the per-combat tables with explicit dispatch-ordering provenance.
    pub fn build_with_provenance(
        relics: &[RelicId],
        dispatch_ordered: bool,
    ) -> Result<Self, HookRefusal> {
        let mut table = Self::build(relics)?;
        table.dispatch_ordered = dispatch_ordered;
        Ok(table)
    }

    /// How many subscriptions there are in total.
    pub fn len(&self) -> usize {
        self.subscribers.len()
    }

    /// Whether nothing subscribes to anything.
    pub fn is_empty(&self) -> bool {
        self.subscribers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_tables::{
        TEMPLATE_RELIC_COND_VERBS, TEMPLATE_RELIC_EFFECT_VERBS, TEMPLATE_RELIC_HOOKS,
        TEMPLATE_RELIC_STEPS,
    };

    fn every_template_relic() -> Vec<RelicId> {
        TEMPLATE_RELIC_STEPS
            .iter()
            .map(|(relic, _)| *relic)
            .collect()
    }

    /// The enum is a claim about `_RELIC_HOOKS_SUPPORTED`; the generated table
    /// is the registry's own list. They must agree in both directions.
    #[test]
    fn the_twelve_relic_hooks_cover_the_published_vocabulary() {
        for name in TEMPLATE_RELIC_HOOKS {
            assert!(
                HookEvent::from_hook_name(name).is_some(),
                "published hook {name:?} has no HookEvent"
            );
        }
        let relic_hooks = HookEvent::ALL
            .into_iter()
            .filter(|event| event.category() == HookCategory::RelicTemplate)
            .count();
        assert_eq!(relic_hooks, 12);
        assert_eq!(HookEvent::COUNT, 19);
    }

    /// The five `_powers_*` functions and two native ordered power walks,
    /// re-checked rather than inherited from PORT_PLAN's "8".
    #[test]
    fn the_power_fanouts_match_the_python_functions() {
        let fanouts: Vec<&str> = HookEvent::ALL
            .into_iter()
            .filter(|event| event.category() == HookCategory::PowerFanout)
            .map(|event| event.as_str())
            .collect();
        assert_eq!(
            fanouts,
            vec![
                "_powers_after_card_drawn",
                "_powers_after_energy_spent",
                "_powers_after_block_gained",
                "_powers_before_card_played",
                "_powers_after_card_played",
                "AfterCardExhausted",
                "BeforeHandDraw",
            ]
        );
    }

    #[test]
    fn the_guard_and_effect_vocabularies_are_covered_exactly() {
        for verb in TEMPLATE_RELIC_COND_VERBS {
            assert!(TemplateCond::parse(verb).is_some(), "guard verb {verb:?}");
        }
        for verb in TEMPLATE_RELIC_EFFECT_VERBS {
            assert!(
                TemplateEffect::parse(verb).is_some(),
                "effect verb {verb:?}"
            );
        }
        assert_eq!(TemplateCond::ALL.len(), TEMPLATE_RELIC_COND_VERBS.len());
        // The interpreter has one branch (`power_all`) no current template
        // reaches, so this is containment, not equality — see the enum docs.
        assert_eq!(TEMPLATE_RELIC_EFFECT_VERBS.len(), 8);
        assert_eq!(TemplateEffect::ALL.len(), 9);
        let unused: Vec<&str> = TemplateEffect::ALL
            .into_iter()
            .map(|effect| effect.as_str())
            .filter(|verb| !TEMPLATE_RELIC_EFFECT_VERBS.contains(verb))
            .collect();
        assert_eq!(unused, vec!["power_all"]);
    }

    #[test]
    fn the_empty_table_is_absent_everywhere() {
        let table = HookTable::empty();
        assert!(table.is_empty());
        for event in HookEvent::ALL {
            assert!(!table.has(event));
            assert!(table.subscribers(event).is_empty());
        }
        assert!(!table.any(HookCategory::RelicTemplate));
        assert!(!table.any(HookCategory::PowerFanout));
    }

    /// The framework against real content: every template relic in the build,
    /// compiled at once. This is what proves the 12-name enum, the guard and
    /// effect vocabularies, and the argument compiler cover the registry.
    #[test]
    fn every_template_relic_compiles() {
        let relics = every_template_relic();
        assert!(
            relics.len() > 10,
            "the build has template relics to compile"
        );
        let table = HookTable::build(&relics).unwrap();
        assert!(!table.is_empty());
        assert!(table.any(HookCategory::RelicTemplate));
        // Every compiled argument is typed: no text survives the compile.
        for event in HookEvent::ALL {
            for subscriber in table.subscribers(event) {
                for rule in table.rules(subscriber) {
                    for cond in table.conds(rule) {
                        assert!(
                            table
                                .args(cond.args)
                                .iter()
                                .all(|arg| !matches!(arg, CompiledArg::Unresolved(_))),
                            "{:?} guard argument did not compile",
                            subscriber.subject
                        );
                    }
                    for effect in table.effects(rule) {
                        assert!(
                            table
                                .args(effect.args)
                                .iter()
                                .all(|arg| !matches!(arg, CompiledArg::Unresolved(_))),
                            "{:?} effect argument did not compile",
                            subscriber.subject
                        );
                    }
                }
            }
        }
    }

    /// The ordering spec (`test_relic_dispatch_order.py`): same-event relics
    /// fire in inventory order, and reversing the inventory reverses them.
    #[test]
    fn same_event_subscribers_follow_inventory_order() {
        let mut relics = every_template_relic();
        let event = HookEvent::ALL
            .into_iter()
            .find(|event| HookTable::build(&relics).unwrap().subscribers(*event).len() > 1)
            .expect("some event has more than one subscriber");

        let forward: Vec<HookSubject> = HookTable::build(&relics)
            .unwrap()
            .subscribers(event)
            .iter()
            .map(|subscriber| subscriber.subject)
            .collect();
        relics.reverse();
        let mut backward: Vec<HookSubject> = HookTable::build(&relics)
            .unwrap()
            .subscribers(event)
            .iter()
            .map(|subscriber| subscriber.subject)
            .collect();
        backward.reverse();
        assert_eq!(forward, backward);
        assert!(forward.len() > 1);
    }

    #[test]
    fn presence_is_per_event_and_per_category() {
        let relics = every_template_relic();
        let table = HookTable::build(&relics).unwrap();
        for event in HookEvent::ALL {
            assert_eq!(table.has(event), !table.subscribers(event).is_empty());
        }
        // No power subscribes in this slice, so the whole family is absent —
        // one mask test, not a walk.
        assert!(!table.any(HookCategory::PowerFanout));
        assert!(POWER_SUBSCRIPTIONS.is_empty());
        assert_eq!(
            TemplateEffect::IMPLEMENTED,
            &[
                TemplateEffect::Block,
                TemplateEffect::DamageAll,
                TemplateEffect::DrawBonus,
                TemplateEffect::Energy,
                TemplateEffect::Heal,
                TemplateEffect::MaxEnergy,
                TemplateEffect::PowerSelf,
                TemplateEffect::Stars,
            ]
        );
    }
}
