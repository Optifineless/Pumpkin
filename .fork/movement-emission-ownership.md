# Accepted movement emission ownership

`Entity::movement_emission` owns only transient `moveDist`, `flyDist`, `nextStep`
and an observation revision. Its checked mutex is level 30. No other lock is
acquired under it. Only local atomic/lifecycle and owned-image reads run under it;
no cross-object gameplay, virtual call, world lookup, callback, send or await does.
All policies, exact lifecycle/session identity and loaded support/effect images
are sampled before the brief counter commit. The threshold commits before the
existing s1 game-event callback, and the caller never writes counters afterward.

| Entry | New behavior and ownership |
|---|---|
| Java position and position+rotation | Capture original identity before PlayerMove; count final accepted displacement after existing fall/enchantment/statistic callbacks. |
| Concrete Entity::move_entity | Count committed clipped movement once after existing fall/block callbacks; default trait wrapper stays silent. |
| Java vehicle movement | Validate original driver/session and exact riding relationship; count only vehicle movement using accepted ground claim and current support. |
| Entity::teleport / Player accepted teleport / Java force_tp / Entity::set_world | Reset only transient emission state before placement, correction or world binding; no movement notification. |
| New life/world/position discontinuity | Reconcile observation; original damage lifecycle, client, tracker identity and teleport revision reject stale continuation. |
| Admission/removal/unload | Reuse current tracker and existing session admission; removed entities reject before effects. Reload constructs fresh state. No new registry. |

Existing position/support/physics caches, living fall/flight counters, species age,
NBT readers/writers, riding-root capture and block-entity admission are unchanged.
Direct set_pos, pistons, passengers, commands and native/Wasm placements never
emit. Explosions retain damage/velocity ownership; subsequent real travel enters
the ordinary path. NoAI retains its existing travel gate.

The source-rich s1 API and sculk gate are unchanged. Its inherited population
lookup remains; the new observation uses existing indexed tracker/session lookup.
Support/effect retain exact loaded chunk instances through decision/commit and
immutable BlockState IDs through notification. s1 revalidates source life/session
after callbacks; it has no post-callback terrain-incarnation input. No listener,
sensor, wool damping or vibration admission is added here.
