"""#138 Batch 242 / #989: The Architect structural event exclusion.

The current build has no combat-entry path for
ENCOUNTER.THE_ARCHITECT_EVENT_ENCOUNTER: the only generic instantiation
naming the encounter type is Encounter<T> (CanonicalEncounter / ModelDb),
never EnterCombatWithoutExitingEvent<T>, and TheArchitect is a scripted
dialogue whose LayoutType-1 scene hosts one 9999-HP do-nothing Architect
prop. Modeling a fight here would synthesize combat the game cannot produce,
so the census row is explicitly excluded from the solver denominator.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""
