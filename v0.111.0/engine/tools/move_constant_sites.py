#!/usr/bin/env python3
"""Which move-table argument each ascension-tiered monster constant feeds.

#2828 (part 2 of #2539). The assembly spells every tiered monster constant
as `AscensionHelper::GetValueIfAscension(gate, atOrAbove, below)`, and
`dll_content.DllFacts.monster_move_constants` reads each one — the triple,
its RVA and its shape — into the committed content manifest. What the
assembly does *not* say is which argument of which generated move row the
constant is: that is the port's modeling of each move body, so it is
declared here, by name, and `generate_content` joins the two.

The join is by name on purpose, never by value. A value match would pair
`AxeRubyRaider::get_SwingDamage` and `get_SwingBlock` arbitrarily (both
`(9, 6, 5)`) and, on the next build, would silently re-pair any constant
whose number moved. Keyed by name, a moved number arrives in the generated
tables from the assembly, and the frozen registry's disagreement is a
codegen failure naming the row (`generate_content.move_constant_tier`).

Four dicts, and together they account for every manifest row of every
modeled `MonsterKind` — `generate_content` refuses to write the tables
otherwise:

* `MOVE_CONSTANT_SITES` — a generated table argument (the generator's own
  site spelling: `<TABLE>[.<KIND>].<MOVE>[<index>]`) -> the constant;
* `ENGINE_MOVE_CONSTANTS` — a constant the engine reads inside a move body
  rather than from a table argument, with where;
* `ROSTER_CONSTANTS` — a constant an `src/encounters/` roster builder reads
  directly, as spawn-time roster state that is neither an HP getter nor an
  `Apply<XPower>` amount (so `MONSTER_MODELS` has no column for it), with
  which builder (#2535);
* `NOT_MOVE_CONSTANTS` — a tiered row that feeds no move, with what it feeds
  instead (spawn-time amounts, which `MONSTER_MODELS` and #2539 part 1
  already carry).

A constant is named `<MonsterKind>.<Getter>`, the getter without `get_`, or
`<MonsterKind>.<Type>::<Method>@IL_xxxx` for an inline call. The manifest is
keyed by the assembly's MONSTER entry; the one aggregate kind,
`DECIMILLIPEDE_SEGMENT`, resolves through
`generate_content.AGGREGATE_MONSTER_CLASSES` and requires its three classes
to agree.
"""

#: Generated move-table argument -> the tiered constant it is.
MOVE_CONSTANT_SITES = {
    "AEONGLASS_MOVES.EBB_MOVE[0]": "AEONGLASS.EbbDamage",
    "AEONGLASS_MOVES.EYE_LASERS_MOVE[0]": "AEONGLASS.EyeLasersDamage",
    "AEONGLASS_MOVES.INCREASING_INTENSITY_MOVE[0]":
        "AEONGLASS.IncreasingIntensityBaseStrength",
    "AEONGLASS_MOVES.INCREASING_INTENSITY_MOVE[1]": "AEONGLASS.WitherAmount",
    "EXOSKELETON_MOVES.MANDIBLES_MOVE[0]": "EXOSKELETON.MandiblesDamage",
    "EXOSKELETON_MOVES.SKITTER_MOVE[1]": "EXOSKELETON.SkitterRepeats",
    "FABRICATOR_BOT_MOVES.STABBOT[0]": "STABBOT.StabDamage",
    "FABRICATOR_BOT_MOVES.ZAPBOT[0]": "ZAPBOT.ZapDamage",
    "FABRICATOR_MOVES.DISINTEGRATE_MOVE[0]": "FABRICATOR.DisintegrateDamage",
    "FABRICATOR_MOVES.FABRICATING_STRIKE_MOVE[0]":
        "FABRICATOR.FabricatingStrikeDamage",
    "FAKE_MERCHANT_MOVES.SWIPE_MOVE[0]": "FAKE_MERCHANT_MONSTER.SwipeDamage",
    "FAKE_MERCHANT_MOVES.THROW_RELIC_MOVE[0]":
        "FAKE_MERCHANT_MONSTER.ThrowRelicDamage",
    "FLAIL_KNIGHT_MOVES.FLAIL_MOVE[0]": "FLAIL_KNIGHT.FlailDamage",
    "FLAIL_KNIGHT_MOVES.RAM_MOVE[0]": "FLAIL_KNIGHT.RamDamage",
    "FLYCONID_MOVES.FRAIL_SPORES_MOVE[0]": "FLYCONID.SporeDamage",
    "FLYCONID_MOVES.SMASH_MOVE[0]": "FLYCONID.SmashDamage",
    "FOSSIL_STALKER_MOVES.LASH_MOVE[0]": "FOSSIL_STALKER.LashDamage",
    "FOSSIL_STALKER_MOVES.LATCH_MOVE[0]": "FOSSIL_STALKER.LatchDamage",
    "FOSSIL_STALKER_MOVES.TACKLE_MOVE[0]": "FOSSIL_STALKER.TackleDamage",
    "HUNTER_KILLER_MOVES.BITE_MOVE[0]": "HUNTER_KILLER.BiteDamage",
    "HUNTER_KILLER_MOVES.PUNCTURE_MOVE[0]": "HUNTER_KILLER.PunctureDamage",
    "INKLET_MOVES.JAB_MOVE[0]": "INKLET.JabDamage",
    "INKLET_MOVES.PIERCING_GAZE_MOVE[0]": "INKLET.PiercingGazeDamage",
    "INKLET_MOVES.WHIRLWIND_MOVE[0]": "INKLET.WhirlwindDamage",
    "LEAF_SLIME_S_MOVES.TACKLE_MOVE[0]": "LEAF_SLIME_S.TackleDamage",
    "LOOPS.AEONGLASS.EBB_MOVE[0]": "AEONGLASS.EbbDamage",
    "LOOPS.AEONGLASS.EYE_LASERS_MOVE[0]": "AEONGLASS.EyeLasersDamage",
    "LOOPS.AEONGLASS.INCREASING_INTENSITY_MOVE[0]":
        "AEONGLASS.IncreasingIntensityBaseStrength",
    "LOOPS.AEONGLASS.INCREASING_INTENSITY_MOVE[1]": "AEONGLASS.WitherAmount",
    "LOOPS.ASSASSIN_RUBY_RAIDER.KILLSHOT_MOVE[0]":
        "ASSASSIN_RUBY_RAIDER.KillshotDamage",
    "LOOPS.AXEBOT.BOOT_UP[0]": "AXEBOT.BootUpBlock",
    "LOOPS.AXEBOT.BOOT_UP[1]": "AXEBOT.BootUpStrGain",
    "LOOPS.AXEBOT.HAMMER_UPPERCUT[0]": "AXEBOT.HammerUppercutDamage",
    "LOOPS.AXEBOT.ONE_TWO[0]": "AXEBOT.OneTwoDamage",
    "LOOPS.AXE_RUBY_RAIDER.BIG_SWING_MOVE[0]":
        "AXE_RUBY_RAIDER.BigSwingDamage",
    "LOOPS.AXE_RUBY_RAIDER.SWING_1_MOVE[0]": "AXE_RUBY_RAIDER.SwingDamage",
    "LOOPS.AXE_RUBY_RAIDER.SWING_1_MOVE[2]": "AXE_RUBY_RAIDER.SwingBlock",
    "LOOPS.AXE_RUBY_RAIDER.SWING_2_MOVE[0]": "AXE_RUBY_RAIDER.SwingDamage",
    "LOOPS.AXE_RUBY_RAIDER.SWING_2_MOVE[2]": "AXE_RUBY_RAIDER.SwingBlock",
    "LOOPS.BOWLBUG_EGG.BITE[0]": "BOWLBUG_EGG.BiteDamage",
    "LOOPS.BOWLBUG_EGG.BITE[2]": "BOWLBUG_EGG.ProtectBlock",
    "LOOPS.BOWLBUG_NECTAR.BUFF[0]": "BOWLBUG_NECTAR.BuffStrengthGain",
    "LOOPS.BOWLBUG_ROCK.HEADBUTT[0]": "BOWLBUG_ROCK.HeadbuttDamage",
    "LOOPS.BOWLBUG_SILK.THRASH_MOVE[0]": "BOWLBUG_SILK.ThrashDamage",
    "LOOPS.BRUTE_RUBY_RAIDER.BEAT_MOVE[0]": "BRUTE_RUBY_RAIDER.BeatDamage",
    "LOOPS.BYGONE_EFFIGY.SLASH[0]": "BYGONE_EFFIGY.SlashDamage",
    "LOOPS.BYRDONIS.PECK[0]": "BYRDONIS.PeckDamage",
    "LOOPS.BYRDONIS.PECK[1]": "BYRDONIS.PeckRepeat",
    "LOOPS.BYRDONIS.SWOOP[0]": "BYRDONIS.SwoopDamage",
    "LOOPS.CALCIFIED_CULTIST.DARK_STRIKE[0]":
        "CALCIFIED_CULTIST.DarkStrikeDamage",
    "LOOPS.CEREMONIAL_BEAST.CRUSH_MOVE[0]": "CEREMONIAL_BEAST.CrushDamage",
    "LOOPS.CEREMONIAL_BEAST.CRUSH_MOVE[2]": "CEREMONIAL_BEAST.CrushStrength",
    "LOOPS.CEREMONIAL_BEAST.PLOW_MOVE[0]": "CEREMONIAL_BEAST.PlowDamage",
    "LOOPS.CEREMONIAL_BEAST.STAMP_MOVE[0]": "CEREMONIAL_BEAST.PlowAmount",
    "LOOPS.CEREMONIAL_BEAST.STOMP_MOVE[0]": "CEREMONIAL_BEAST.StompDamage",
    "LOOPS.CHOMPER.CLAMP_MOVE[0]": "CHOMPER.ClampDamage",
    "LOOPS.CORPSE_SLUG.GLOMP_MOVE[0]": "CORPSE_SLUG.GlompDamage",
    "LOOPS.CROSSBOW_RUBY_RAIDER.FIRE_MOVE[0]":
        "CROSSBOW_RUBY_RAIDER.FireDamage",
    "LOOPS.CRUSHER.ADAPT_MOVE[0]": "CRUSHER.AdaptStrengthGain",
    "LOOPS.CRUSHER.BUG_STING_MOVE[0]": "CRUSHER.BugStingDamage",
    "LOOPS.CRUSHER.ENLARGING_STRIKE_MOVE[0]": "CRUSHER.EnlargingStrikeDamage",
    "LOOPS.CRUSHER.GUARDED_STRIKE_MOVE[0]": "CRUSHER.GuardedStrikeDamage",
    "LOOPS.CRUSHER.THRASH_MOVE[0]": "CRUSHER.ThrashDamage",
    "LOOPS.CUBEX_CONSTRUCT.EXPEL_MOVE[0]": "CUBEX_CONSTRUCT.ExpelDamage",
    "LOOPS.CUBEX_CONSTRUCT.REPEATER_BLAST_MOVE[0]":
        "CUBEX_CONSTRUCT.BlastDamage",
    "LOOPS.CUBEX_CONSTRUCT.REPEATER_BLAST_MOVE_2[0]":
        "CUBEX_CONSTRUCT.BlastDamage",
    "LOOPS.DAMP_CULTIST.DARK_STRIKE[0]": "DAMP_CULTIST.DarkStrikeDamage",
    "LOOPS.DAMP_CULTIST.INCANTATION[0]": "DAMP_CULTIST.IncantationAmount",
    "LOOPS.DECIMILLIPEDE_SEGMENT.BULK[0]": "DECIMILLIPEDE_SEGMENT.BulkDamage",
    "LOOPS.DECIMILLIPEDE_SEGMENT.CONSTRICT[0]":
        "DECIMILLIPEDE_SEGMENT.ConstrictDamage",
    "LOOPS.DECIMILLIPEDE_SEGMENT.WRITHE[0]":
        "DECIMILLIPEDE_SEGMENT.WritheDamage",
    "LOOPS.DEVOTED_SCULPTOR.SAVAGE[0]": "DEVOTED_SCULPTOR.SavageDamage",
    "LOOPS.ENTOMANCER.BEES[0]": "ENTOMANCER.BeesDamage",
    "LOOPS.ENTOMANCER.BEES[1]": "ENTOMANCER.BeesRepeat",
    "LOOPS.ENTOMANCER.SPEAR[0]": "ENTOMANCER.SpearMoveDamage",
    "LOOPS.FOGMOG.HEADBUTT_MOVE[0]": "FOGMOG.HeadbuttDamage",
    "LOOPS.FOGMOG.SWIPE_MOVE[0]": "FOGMOG.SwipeDamage",
    "LOOPS.FOGMOG.SWIPE_RANDOM_MOVE[0]": "FOGMOG.SwipeDamage",
    "LOOPS.FROG_KNIGHT.BEETLE_CHARGE[0]": "FROG_KNIGHT.BeetleChargeDamage",
    "LOOPS.FROG_KNIGHT.STRIKE_DOWN_EVIL[0]":
        "FROG_KNIGHT.StrikeDownEvilDamage",
    "LOOPS.FROG_KNIGHT.TONGUE_LASH[0]": "FROG_KNIGHT.TongueLashDamage",
    "LOOPS.FUZZY_WURM_CRAWLER.ACID_GOOP[0]":
        "FUZZY_WURM_CRAWLER.AcidGoopDamage",
    "LOOPS.FUZZY_WURM_CRAWLER.FIRST_ACID_GOOP[0]":
        "FUZZY_WURM_CRAWLER.AcidGoopDamage",
    "LOOPS.GAS_BOMB.EXPLODE[0]": "GAS_BOMB.ExplodeDamage",
    "LOOPS.GLOBE_HEAD.GALVANIC_BURST[0]": "GLOBE_HEAD.GalvanicBurstDamage",
    "LOOPS.GLOBE_HEAD.SHOCKING_SLAP[0]": "GLOBE_HEAD.ShockingSlapDamage",
    "LOOPS.GLOBE_HEAD.THUNDER_STRIKE[0]": "GLOBE_HEAD.ThunderStrikeDamage",
    "LOOPS.GREMLIN_MERC.DOUBLE_SMASH_MOVE[0]":
        "GREMLIN_MERC.DoubleSmashDamage",
    "LOOPS.GREMLIN_MERC.GIMME_MOVE[0]": "GREMLIN_MERC.GimmeDamage",
    "LOOPS.GREMLIN_MERC.HEHE_MOVE[0]": "GREMLIN_MERC.HeheDamage",
    "LOOPS.HAUNTED_SHIP.STOMP_MOVE[0]": "HAUNTED_SHIP.StompDamage",
    "LOOPS.HAUNTED_SHIP.SWIPE_MOVE[0]": "HAUNTED_SHIP.SwipeDamage",
    "LOOPS.INFESTED_PRISM.JAB[0]": "INFESTED_PRISM.JabDamage",
    "LOOPS.INFESTED_PRISM.PULSATE[0]": "INFESTED_PRISM.PulsateDamage",
    "LOOPS.INFESTED_PRISM.PULSATE[2]": "INFESTED_PRISM.PulsateBlock",
    "LOOPS.INFESTED_PRISM.PULSATE[3]": "INFESTED_PRISM.VitalSparkAmount",
    "LOOPS.INFESTED_PRISM.RADIATE[0]": "INFESTED_PRISM.RadiateDamage",
    "LOOPS.INFESTED_PRISM.RADIATE[2]": "INFESTED_PRISM.RadiateBlock",
    "LOOPS.INFESTED_PRISM.WHIRLWIND[0]": "INFESTED_PRISM.WhirlwindDamage",
    "LOOPS.KIN_FOLLOWER.BOOMERANG_MOVE[0]": "KIN_FOLLOWER.BoomerangDamage",
    "LOOPS.KIN_FOLLOWER.POWER_DANCE_MOVE[0]": "KIN_FOLLOWER.DanceStrength",
    "LOOPS.KIN_FOLLOWER.QUICK_SLASH_MOVE[0]": "KIN_FOLLOWER.QuickSlashDamage",
    "LOOPS.KIN_PRIEST.BEAM_MOVE[0]": "KIN_PRIEST.BeamDamage",
    "LOOPS.KIN_PRIEST.ORB_OF_FRAILTY_MOVE[0]":
        "KIN_PRIEST.OrbOfFrailtyDamage",
    "LOOPS.KIN_PRIEST.ORB_OF_WEAKNESS_MOVE[0]":
        "KIN_PRIEST.OrbOfWeaknessDamage",
    "LOOPS.KIN_PRIEST.RITUAL_MOVE[0]": "KIN_PRIEST.RitualStrength",
    "LOOPS.KNOWLEDGE_DEMON.KNOWLEDGE_OVERWHELMING_MOVE[0]":
        "KNOWLEDGE_DEMON.KnowledgeOverwhelmingDamage",
    "LOOPS.KNOWLEDGE_DEMON.PONDER_MOVE[0]": "KNOWLEDGE_DEMON.PonderDamage",
    "LOOPS.KNOWLEDGE_DEMON.PONDER_MOVE[3]": "KNOWLEDGE_DEMON.PonderStrength",
    "LOOPS.KNOWLEDGE_DEMON.SLAP_MOVE[0]": "KNOWLEDGE_DEMON.SlapDamage",
    "LOOPS.LAGAVULIN_MATRIARCH.DISEMBOWEL_MOVE[0]":
        "LAGAVULIN_MATRIARCH.DisembowelDamage",
    "LOOPS.LAGAVULIN_MATRIARCH.SLASH2_MOVE[0]":
        "LAGAVULIN_MATRIARCH.Slash2Damage",
    "LOOPS.LAGAVULIN_MATRIARCH.SLASH2_MOVE[2]":
        "LAGAVULIN_MATRIARCH.Slash2Block",
    "LOOPS.LAGAVULIN_MATRIARCH.SLASH_MOVE[0]":
        "LAGAVULIN_MATRIARCH.SlashDamage",
    "LOOPS.LEAF_SLIME_M.CLUMP_SHOT[0]": "LEAF_SLIME_M.ClumpDamage",
    "LOOPS.LIVING_FOG.ADVANCED_GAS[0]": "LIVING_FOG.AdvancedGasDamage",
    "LOOPS.LIVING_FOG.BLOAT[0]": "LIVING_FOG.BloatDamage",
    "LOOPS.LIVING_FOG.SUPER_GAS_BLAST[0]": "LIVING_FOG.SuperGasBlastDamage",
    "LOOPS.LIVING_SHIELD.SMASH_MOVE[0]": "LIVING_SHIELD.SmashDamage",
    "LOOPS.LOUSE_PROGENITOR.CURL_AND_GROW_MOVE[0]":
        "LOUSE_PROGENITOR.CurlBlock",
    "LOOPS.LOUSE_PROGENITOR.CURL_AND_GROW_MOVE[1]":
        "LOUSE_PROGENITOR.GrowStrength",
    "LOOPS.LOUSE_PROGENITOR.POUNCE_MOVE[0]": "LOUSE_PROGENITOR.PounceDamage",
    "LOOPS.LOUSE_PROGENITOR.WEB_CANNON_MOVE[0]": "LOUSE_PROGENITOR.WebDamage",
    "LOOPS.MAGI_KNIGHT.MAGIC_BOMB_MOVE[0]": "MAGI_KNIGHT.BombDamage",
    "LOOPS.MAGI_KNIGHT.POWER_SHIELD_MOVE[0]": "MAGI_KNIGHT.PowerShieldDamage",
    "LOOPS.MAGI_KNIGHT.POWER_SHIELD_MOVE[2]": "MAGI_KNIGHT.PowerShieldBlock",
    "LOOPS.MAGI_KNIGHT.PREP_MOVE[0]": "MAGI_KNIGHT.PowerShieldBlock",
    "LOOPS.MAGI_KNIGHT.RAM_MOVE[0]": "MAGI_KNIGHT.SpearDamage",
    "LOOPS.MECHA_KNIGHT.CHARGE_MOVE[0]": "MECHA_KNIGHT.ChargeDamage",
    "LOOPS.MECHA_KNIGHT.FLAMETHROWER_MOVE[0]":
        "MECHA_KNIGHT.FlamethrowerDamage",
    "LOOPS.MECHA_KNIGHT.HEAVY_CLEAVE_MOVE[0]":
        "MECHA_KNIGHT.HeavyCleaveDamage",
    "LOOPS.MYTE.BITE_MOVE[0]": "MYTE.BiteDamage",
    "LOOPS.MYTE.SUCK_MOVE[0]": "MYTE.SuckDamage",
    "LOOPS.MYTE.SUCK_MOVE[2]": "MYTE.SuckStrength",
    "LOOPS.NIBBIT.BUTT[0]": "NIBBIT.ButtDamage",
    "LOOPS.NIBBIT.HISS[0]": "NIBBIT.HissStrengthGain",
    "LOOPS.NIBBIT.SLICE[0]": "NIBBIT.SliceDamage",
    "LOOPS.NIBBIT.SLICE[2]": "NIBBIT.SliceBlock",
    "LOOPS.OVICOPTER.NUTRITIONAL_PASTE_MOVE[0]":
        "OVICOPTER.NutritionalPasteStrengthAmount",
    "LOOPS.OVICOPTER.SMASH_MOVE[0]": "OVICOPTER.SmashDamage",
    "LOOPS.OVICOPTER.TENDERIZER_MOVE[0]": "OVICOPTER.TenderizerDamage",
    "LOOPS.OWL_MAGISTRATE.MAGISTRATE_SCRUTINY[0]":
        "OWL_MAGISTRATE.ScrutinyDamage",
    "LOOPS.OWL_MAGISTRATE.PECK_ASSAULT[0]":
        "OWL_MAGISTRATE.PeckAssaultDamage",
    "LOOPS.OWL_MAGISTRATE.VERDICT[0]": "OWL_MAGISTRATE.VerdictDamage",
    "LOOPS.PARAFRIGHT.SLAM_MOVE[0]": "PARAFRIGHT.SlamDamage",
    "LOOPS.PHANTASMAL_GARDENER.BITE[0]": "PHANTASMAL_GARDENER.BiteDamage",
    "LOOPS.PHANTASMAL_GARDENER.ENLARGE[0]": "PHANTASMAL_GARDENER.EnlargeStr",
    "LOOPS.PHANTASMAL_GARDENER.FLAIL[1]": "PHANTASMAL_GARDENER.FlailRepeat",
    "LOOPS.PHANTASMAL_GARDENER.LASH[0]": "PHANTASMAL_GARDENER.LashDamage",
    "LOOPS.PHROG_PARASITE.LASH[0]": "PHROG_PARASITE.LashDamage",
    "LOOPS.PUNCH_CONSTRUCT.FAST_PUNCH[0]": "PUNCH_CONSTRUCT.FastPunchDamage",
    "LOOPS.PUNCH_CONSTRUCT.STRONG_PUNCH[0]":
        "PUNCH_CONSTRUCT.StrongPunchDamage",
    "LOOPS.QUEEN.BURN_BRIGHT_FOR_ME_MOVE[0]":
        "QUEEN.<BurnBrightForMeMove>d__43::MoveNext@IL_00aa",
    "LOOPS.QUEEN.EXECUTION_MOVE[0]": "QUEEN.ExecutionDamage",
    "LOOPS.QUEEN.OFF_WITH_YOUR_HEAD_MOVE[0]": "QUEEN.OffWithYourHeadDamage",
    "LOOPS.ROCKET.CHARGE_UP_MOVE[0]": "ROCKET.ChargeUpStrengthGain",
    "LOOPS.ROCKET.LASER_MOVE[0]": "ROCKET.LaserDamage",
    "LOOPS.ROCKET.PRECISION_BEAM_MOVE[0]": "ROCKET.PrecisionBeamDamage",
    "LOOPS.ROCKET.TARGETING_RETICLE_MOVE[0]": "ROCKET.TargetingReticleDamage",
    "LOOPS.SCROLL_OF_BITING.CHEW[0]": "SCROLL_OF_BITING.ChewDamage",
    "LOOPS.SCROLL_OF_BITING.CHOMP[0]": "SCROLL_OF_BITING.ChompDamage",
    "LOOPS.SEAPUNK.BUBBLE_BURP[0]": "SEAPUNK.BubbleBlock",
    "LOOPS.SEAPUNK.BUBBLE_BURP[1]": "SEAPUNK.BubbleStr",
    "LOOPS.SEAPUNK.SEA_KICK[0]": "SEAPUNK.SeaKickDamage",
    "LOOPS.SEWER_CLAM.JET_MOVE[0]": "SEWER_CLAM.JetDamage",
    "LOOPS.SHRINKER_BEETLE.CHOMP_MOVE[0]": "SHRINKER_BEETLE.ChompDamage",
    "LOOPS.SHRINKER_BEETLE.STOMP_MOVE[0]": "SHRINKER_BEETLE.StompDamage",
    "LOOPS.SKULKING_COLONY.INERTIA[0]": "SKULKING_COLONY.InertiaDamage",
    "LOOPS.SKULKING_COLONY.INERTIA[2]": "SKULKING_COLONY.InertiaStrengthGain",
    "LOOPS.SKULKING_COLONY.PIERCING_STABS[0]":
        "SKULKING_COLONY.PiercingStabsDamage",
    "LOOPS.SKULKING_COLONY.ZOOM[0]": "SKULKING_COLONY.ZoomDamage",
    "LOOPS.SKULKING_COLONY.ZOOM_2[0]": "SKULKING_COLONY.ZoomDamage",
    "LOOPS.SLIMED_BERSERKER.FURIOUS_PUMMELING_MOVE[0]":
        "SLIMED_BERSERKER.PummelingDamage",
    "LOOPS.SLIMED_BERSERKER.SMOTHER_MOVE[0]":
        "SLIMED_BERSERKER.SmotherDamage",
    "LOOPS.SNAPPING_JAXFRUIT.ENERGY_ORB_MOVE[0]":
        "SNAPPING_JAXFRUIT.EnergyDamage",
    "LOOPS.SNEAKY_GREMLIN.TACKLE_MOVE[0]": "SNEAKY_GREMLIN.TackleDamage",
    "LOOPS.SOUL_FYSH.DE_GAS[0]": "SOUL_FYSH.DeGasDamage",
    "LOOPS.SOUL_FYSH.GAZE[0]": "SOUL_FYSH.GazeDamage",
    "LOOPS.SOUL_FYSH.SCREAM[0]": "SOUL_FYSH.ScreamDamage",
    "LOOPS.SPINY_TOAD.SPIKE_EXPLOSION_MOVE[0]": "SPINY_TOAD.ExplosionDamage",
    "LOOPS.SPINY_TOAD.TONGUE_LASH_MOVE[0]": "SPINY_TOAD.LashDamage",
    "LOOPS.STABBOT.STAB_MOVE[0]": "STABBOT.StabDamage",
    "LOOPS.TERROR_EEL.CRASH[0]": "TERROR_EEL.CrashDamage",
    "LOOPS.TERROR_EEL.THRASH[0]": "TERROR_EEL.ThrashDamage",
    "LOOPS.TEST_SUBJECT.BITE_MOVE[0]": "TEST_SUBJECT.BiteDamage",
    "LOOPS.TEST_SUBJECT.BURNING_GROWL_MOVE[0]":
        "TEST_SUBJECT.BurningGrowlBurnCount",
    "LOOPS.TEST_SUBJECT.BURNING_GROWL_MOVE[1]":
        "TEST_SUBJECT.BurningGrowlStrengthGain",
    "LOOPS.TEST_SUBJECT.MULTI_CLAW_MOVE[0]": "TEST_SUBJECT.MultiClawDamage",
    "LOOPS.TEST_SUBJECT.PHASE3_LACERATE_MOVE[0]":
        "TEST_SUBJECT.Phase3LacerateDamage",
    "LOOPS.TEST_SUBJECT.SKULL_BASH_MOVE[0]": "TEST_SUBJECT.SkullBashDamage",
    "LOOPS.THE_FORGOTTEN.DREAD[0]": "THE_FORGOTTEN.DreadDamage",
    "LOOPS.THE_FORGOTTEN.MIASMA[0]":
        "THE_FORGOTTEN.DebilitatingSmogDexStealAmount",
    "LOOPS.THE_INSATIABLE.LUNGING_BITE_MOVE[0]": "THE_INSATIABLE.BiteDamage",
    "LOOPS.THE_INSATIABLE.SALIVATE_MOVE[0]":
        "THE_INSATIABLE.SalivateStrength",
    "LOOPS.THE_INSATIABLE.THRASH_MOVE[0]": "THE_INSATIABLE.ThrashDamage",
    "LOOPS.THE_INSATIABLE.THRASH_MOVE_2[0]": "THE_INSATIABLE.ThrashDamage",
    "LOOPS.THE_LOST.DEBILITATING_SMOG[0]":
        "THE_LOST.DebilitatingSmogStrengthStealAmount",
    "LOOPS.THE_LOST.EYE_LASERS[0]": "THE_LOST.EyeLasersDamage",
    "LOOPS.THIEVING_HOPPER.HAT_TRICK_MOVE[0]":
        "THIEVING_HOPPER.HatTrickDamage",
    "LOOPS.THIEVING_HOPPER.NAB_MOVE[0]": "THIEVING_HOPPER.NabDamage",
    "LOOPS.THIEVING_HOPPER.THIEVERY_MOVE[0]": "THIEVING_HOPPER.TheftDamage",
    "LOOPS.TOADPOLE.SPIKE_SPIT[0]": "TOADPOLE.SpikeSpitDamage",
    "LOOPS.TOADPOLE.WHIRL[0]": "TOADPOLE.WhirlDamage",
    "LOOPS.TORCH_HEAD_AMALGAM.BEAM_MOVE[0]":
        "TORCH_HEAD_AMALGAM.SoulBeamDamage",
    "LOOPS.TORCH_HEAD_AMALGAM.STRONG_TACKLE_MOVE[0]":
        "TORCH_HEAD_AMALGAM.StrongTackleDamage",
    "LOOPS.TORCH_HEAD_AMALGAM.TACKLE_2_MOVE[0]":
        "TORCH_HEAD_AMALGAM.TackleDamage",
    "LOOPS.TORCH_HEAD_AMALGAM.TACKLE_3_MOVE[0]":
        "TORCH_HEAD_AMALGAM.WeakTackleDamage",
    "LOOPS.TORCH_HEAD_AMALGAM.TACKLE_4_MOVE[0]":
        "TORCH_HEAD_AMALGAM.WeakTackleDamage",
    "LOOPS.TOUGH_EGG.NIBBLE_MOVE[0]": "TOUGH_EGG.NibbleDamage",
    "LOOPS.TRACKER_RUBY_RAIDER.HOUNDS_MOVE[0]":
        "TRACKER_RUBY_RAIDER.HoundsDamage",
    "LOOPS.TRACKER_RUBY_RAIDER.HOUNDS_MOVE[1]":
        "TRACKER_RUBY_RAIDER.HoundsRepeat",
    "LOOPS.TUNNELER.BELOW[0]": "TUNNELER.BelowDamage",
    "LOOPS.TUNNELER.BITE[0]": "TUNNELER.BiteDamage",
    "LOOPS.TUNNELER.BURROW[0]": "TUNNELER.BlockGain",
    "LOOPS.TURRET_OPERATOR.UNLOAD_MOVE[0]": "TURRET_OPERATOR.FireDamage",
    "LOOPS.TURRET_OPERATOR.UNLOAD_MOVE_2[0]": "TURRET_OPERATOR.FireDamage",
    "LOOPS.TWIG_SLIME_S.TACKLE_MOVE[0]": "TWIG_SLIME_S.TackleDamage",
    "LOOPS.VANTOM.DISMEMBER_MOVE[0]": "VANTOM.DismemberDamage",
    "LOOPS.VANTOM.INKY_LANCE_MOVE[0]": "VANTOM.InkyLanceDamage",
    "LOOPS.VANTOM.INK_BLOT_MOVE[0]": "VANTOM.InkBlotDamage",
    "LOOPS.VINE_SHAMBLER.CHOMP_MOVE[0]": "VINE_SHAMBLER.ChompDamage",
    "LOOPS.VINE_SHAMBLER.GRASPING_VINES_MOVE[0]":
        "VINE_SHAMBLER.GraspingVinesDamage",
    "LOOPS.VINE_SHAMBLER.SWIPE_MOVE[0]": "VINE_SHAMBLER.SwipeDamage",
    "LOOPS.WATERFALL_GIANT.PRESSURE_UP_MOVE[0]":
        "WATERFALL_GIANT.PressureUpDamage",
    "LOOPS.WATERFALL_GIANT.PRESSURIZE_MOVE[0]":
        "WATERFALL_GIANT.PressurizeAmount",
    "LOOPS.WATERFALL_GIANT.RAM_MOVE[0]": "WATERFALL_GIANT.RamDamage",
    "LOOPS.WATERFALL_GIANT.SIPHON_MOVE[0]": "WATERFALL_GIANT.SiphonHeal",
    "LOOPS.WATERFALL_GIANT.STOMP_MOVE[0]": "WATERFALL_GIANT.StompDamage",
    "LOOPS.WRIGGLER.NASTY_BITE[0]": "WRIGGLER.BiteDamage",
    "LOOPS.ZAPBOT.ZAP_MOVE[0]": "ZAPBOT.ZapDamage",
    "MAWLER_MOVES.CLAW_MOVE[0]": "MAWLER.ClawDamage",
    "MAWLER_MOVES.RIP_AND_TEAR_MOVE[0]": "MAWLER.RipAndTearDamage",
    "MYSTERIOUS_KNIGHT_MOVES.FLAIL_MOVE[0]": "MYSTERIOUS_KNIGHT.FlailDamage",
    "MYSTERIOUS_KNIGHT_MOVES.RAM_MOVE[0]": "MYSTERIOUS_KNIGHT.RamDamage",
    "OVICOPTER_MOVES.NUTRITIONAL_PASTE_MOVE[0]":
        "OVICOPTER.NutritionalPasteStrengthAmount",
    "OVICOPTER_MOVES.SMASH_MOVE[0]": "OVICOPTER.SmashDamage",
    "OVICOPTER_MOVES.TENDERIZER_MOVE[0]": "OVICOPTER.TenderizerDamage",
    "RAT_MOVES.DISEASE_BITE[0]": "TWO_TAILED_RAT.DiseaseBiteDamage",
    "RAT_MOVES.SCRATCH[0]": "TWO_TAILED_RAT.ScratchDamage",
    "SLITHERING_STRANGLER_MOVES.LASH_MOVE[0]":
        "SLITHERING_STRANGLER.LashDamage",
    "SLITHERING_STRANGLER_MOVES.THWACK_MOVE[0]":
        "SLITHERING_STRANGLER.ThwackDamage",
    "SLUDGE_MOVES.OIL_SPRAY[0]": "SLUDGE_SPINNER.OilSprayDamage",
    "SLUDGE_MOVES.RAGE[0]": "SLUDGE_SPINNER.RageDamage",
    "SLUDGE_MOVES.SLAM[0]": "SLUDGE_SPINNER.SlamDamage",
    "SOULNEXUS_MOVES.DRAIN_LIFE[0]": "SOUL_NEXUS.DrainLifeDamage",
    "SOULNEXUS_MOVES.MAELSTROM[0]": "SOUL_NEXUS.MaelstromDamage",
    "SOULNEXUS_MOVES.MAELSTROM[1]": "SOUL_NEXUS.MaelstromRepeat",
    "SOULNEXUS_MOVES.SOUL_BURN[0]": "SOUL_NEXUS.SoulBurnDamage",
    "SPECTRAL_KNIGHT_MOVES.SOUL_FLAME_MOVE[0]":
        "SPECTRAL_KNIGHT.SoulFlameDamage",
    "SPECTRAL_KNIGHT_MOVES.SOUL_SLASH_MOVE[0]":
        "SPECTRAL_KNIGHT.SoulSlashDamage",
    "THE_OBSCURA_MOVES.HARDENING_STRIKE_MOVE[0]":
        "THE_OBSCURA.HardeningStrikeDamage",
    "THE_OBSCURA_MOVES.HARDENING_STRIKE_MOVE[2]":
        "THE_OBSCURA.HardeningStrikeBlock",
    "THE_OBSCURA_MOVES.PIERCING_GAZE_MOVE[0]":
        "THE_OBSCURA.PiercingGazeDamage",
    "TOUGH_EGG_MOVES.NIBBLE_MOVE[0]": "TOUGH_EGG.NibbleDamage",
    "TWIG_SLIME_M_MOVES.POKEY_POUNCE_MOVE[0]": "TWIG_SLIME_M.ClumpDamage",
    "_RANDOM_MOVES.EXOSKELETON.MANDIBLES_MOVE[0]":
        "EXOSKELETON.MandiblesDamage",
    "_RANDOM_MOVES.EXOSKELETON.SKITTER_MOVE[1]": "EXOSKELETON.SkitterRepeats",
    "_RANDOM_MOVES.FABRICATOR.DISINTEGRATE_MOVE[0]":
        "FABRICATOR.DisintegrateDamage",
    "_RANDOM_MOVES.FABRICATOR.FABRICATING_STRIKE_MOVE[0]":
        "FABRICATOR.FabricatingStrikeDamage",
    "_RANDOM_MOVES.FAKE_MERCHANT_MONSTER.SWIPE_MOVE[0]":
        "FAKE_MERCHANT_MONSTER.SwipeDamage",
    "_RANDOM_MOVES.FAKE_MERCHANT_MONSTER.THROW_RELIC_MOVE[0]":
        "FAKE_MERCHANT_MONSTER.ThrowRelicDamage",
    "_RANDOM_MOVES.FLAIL_KNIGHT.FLAIL_MOVE[0]": "FLAIL_KNIGHT.FlailDamage",
    "_RANDOM_MOVES.FLAIL_KNIGHT.RAM_MOVE[0]": "FLAIL_KNIGHT.RamDamage",
    "_RANDOM_MOVES.FLYCONID.FRAIL_SPORES_MOVE[0]": "FLYCONID.SporeDamage",
    "_RANDOM_MOVES.FLYCONID.SMASH_MOVE[0]": "FLYCONID.SmashDamage",
    "_RANDOM_MOVES.FOSSIL_STALKER.LASH_MOVE[0]": "FOSSIL_STALKER.LashDamage",
    "_RANDOM_MOVES.FOSSIL_STALKER.LATCH_MOVE[0]":
        "FOSSIL_STALKER.LatchDamage",
    "_RANDOM_MOVES.FOSSIL_STALKER.TACKLE_MOVE[0]":
        "FOSSIL_STALKER.TackleDamage",
    "_RANDOM_MOVES.HUNTER_KILLER.BITE_MOVE[0]": "HUNTER_KILLER.BiteDamage",
    "_RANDOM_MOVES.HUNTER_KILLER.PUNCTURE_MOVE[0]":
        "HUNTER_KILLER.PunctureDamage",
    "_RANDOM_MOVES.INKLET.JAB_MOVE[0]": "INKLET.JabDamage",
    "_RANDOM_MOVES.INKLET.PIERCING_GAZE_MOVE[0]": "INKLET.PiercingGazeDamage",
    "_RANDOM_MOVES.INKLET.WHIRLWIND_MOVE[0]": "INKLET.WhirlwindDamage",
    "_RANDOM_MOVES.LEAF_SLIME_S.TACKLE_MOVE[0]": "LEAF_SLIME_S.TackleDamage",
    "_RANDOM_MOVES.MAWLER.CLAW_MOVE[0]": "MAWLER.ClawDamage",
    "_RANDOM_MOVES.MAWLER.RIP_AND_TEAR_MOVE[0]": "MAWLER.RipAndTearDamage",
    "_RANDOM_MOVES.MYSTERIOUS_KNIGHT.FLAIL_MOVE[0]":
        "MYSTERIOUS_KNIGHT.FlailDamage",
    "_RANDOM_MOVES.MYSTERIOUS_KNIGHT.RAM_MOVE[0]":
        "MYSTERIOUS_KNIGHT.RamDamage",
    "_RANDOM_MOVES.SLITHERING_STRANGLER.LASH_MOVE[0]":
        "SLITHERING_STRANGLER.LashDamage",
    "_RANDOM_MOVES.SLITHERING_STRANGLER.THWACK_MOVE[0]":
        "SLITHERING_STRANGLER.ThwackDamage",
    "_RANDOM_MOVES.SLUDGE_SPINNER.OIL_SPRAY[0]":
        "SLUDGE_SPINNER.OilSprayDamage",
    "_RANDOM_MOVES.SLUDGE_SPINNER.RAGE[0]": "SLUDGE_SPINNER.RageDamage",
    "_RANDOM_MOVES.SLUDGE_SPINNER.SLAM[0]": "SLUDGE_SPINNER.SlamDamage",
    "_RANDOM_MOVES.SOUL_NEXUS.DRAIN_LIFE[0]": "SOUL_NEXUS.DrainLifeDamage",
    "_RANDOM_MOVES.SOUL_NEXUS.MAELSTROM[0]": "SOUL_NEXUS.MaelstromDamage",
    "_RANDOM_MOVES.SOUL_NEXUS.MAELSTROM[1]": "SOUL_NEXUS.MaelstromRepeat",
    "_RANDOM_MOVES.SOUL_NEXUS.SOUL_BURN[0]": "SOUL_NEXUS.SoulBurnDamage",
    "_RANDOM_MOVES.SPECTRAL_KNIGHT.SOUL_FLAME_MOVE[0]":
        "SPECTRAL_KNIGHT.SoulFlameDamage",
    "_RANDOM_MOVES.SPECTRAL_KNIGHT.SOUL_SLASH_MOVE[0]":
        "SPECTRAL_KNIGHT.SoulSlashDamage",
    "_RANDOM_MOVES.THE_OBSCURA.HARDENING_STRIKE_MOVE[0]":
        "THE_OBSCURA.HardeningStrikeDamage",
    "_RANDOM_MOVES.THE_OBSCURA.HARDENING_STRIKE_MOVE[2]":
        "THE_OBSCURA.HardeningStrikeBlock",
    "_RANDOM_MOVES.THE_OBSCURA.PIERCING_GAZE_MOVE[0]":
        "THE_OBSCURA.PiercingGazeDamage",
    "_RANDOM_MOVES.TWIG_SLIME_M.POKEY_POUNCE_MOVE[0]":
        "TWIG_SLIME_M.ClumpDamage",
    "_RANDOM_MOVES.TWO_TAILED_RAT.DISEASE_BITE[0]":
        "TWO_TAILED_RAT.DiseaseBiteDamage",
    "_RANDOM_MOVES.TWO_TAILED_RAT.SCRATCH[0]": "TWO_TAILED_RAT.ScratchDamage",
}

#: Rust names for the inline sites, which have no getter to be named after.
INLINE_CONSTANT_NAMES = {
    "QUEEN.<BurnBrightForMeMove>d__43::MoveNext@IL_00aa":
        "BurnBrightStrength",
}

#: Tiered constants a move body reads directly rather than through a table
#: argument -> where the engine reads it.
ENGINE_MOVE_CONSTANTS = {
    "SLUMBERING_BEETLE.RolloutDamage": (
        "the awake ROLL_OUT_MOVE is not a table row "
        "(`SlumberingBeetle::GenerateMoveStateMachine` is override-driven "
        "while asleep): combat_sim SLUMBERING_BEETLE_ROLLOUT, and the Rust "
        "catalog's `BEETLE_ROLLOUT` row"),
}

#: Tiered rows that feed no move -> what they feed.
NOT_MOVE_CONSTANTS = {
    "CORPSE_SLUG.RavenousStr":
        "spawn-time RavenousPower: MONSTER_MODELS.initial_powers",
    "FROG_KNIGHT.PlatingAmount":
        "spawn-time PlatingPower: MONSTER_MODELS.initial_powers",
    "GLOBE_HEAD.GalvanicPowerAmount":
        "spawn-time GalvanicPower: MONSTER_MODELS.initial_powers",
    "PHANTASMAL_GARDENER.SkittishAmount":
        "spawn-time SkittishPower: MONSTER_MODELS.initial_powers",
    "SEWER_CLAM.<AfterAddedToRoom>d__9::MoveNext@IL_0083":
        "spawn-time PlatingPower, IL-read by #2539 part 1",
    "SLUMBERING_BEETLE.PlatingAmount":
        "spawn-time PlatingPower: MONSTER_MODELS.initial_powers",
    "TERROR_EEL.ShriekAmount":
        "spawn-time ShriekPower: MONSTER_MODELS.initial_powers",
    "TEST_SUBJECT.EnrageAmount":
        "spawn-time EnragePower, IL-read by #2539 part 1",
    "TEST_SUBJECT.FirstFormHp": "form HP, IL-read by #2539 part 1",
    "TEST_SUBJECT.SecondFormHp": "form HP, IL-read by #2539 part 1",
    "TEST_SUBJECT.ThirdFormHp": "form HP, IL-read by #2539 part 1",
    "TOUGH_EGG.HatchlingMaxHp": "Hatchling HP, IL-read by #2539 part 1",
    "TOUGH_EGG.HatchlingMinHp": "Hatchling HP, IL-read by #2539 part 1",
    "VANTOM.SlipperyAmt":
        "spawn-time SlipperyPower: MONSTER_MODELS.initial_powers",
}

#: Tiered constants a roster builder reads directly -> which builder, and the
#: native write it restates. Emitted into `content_tables::move_constants`
#: like the engine's, so the builder selects the fight's tier from the
#: assembly's triple rather than from a typed or registry-lifted number.
ROSTER_CONSTANTS = {
    "WATERFALL_GIANT.BasePressureGunDamage": (
        "encounters::boss::build_waterfall_giant_boss: "
        "`WaterfallGiant/<AfterAddedToRoom>d__64::MoveNext` (RVA `0x3755f4`) "
        "IL_0077-IL_007c is `set_CurrentPressureGunDamage("
        "get_BasePressureGunDamage())`, which combat_sim carries as "
        "`Monster.pressure_gun_damage`"),
}
