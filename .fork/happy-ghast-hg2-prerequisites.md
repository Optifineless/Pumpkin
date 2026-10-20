# HG2 navigation prerequisite and remaining ownership work

Base: `ce84a33063e87956c9c11c134d2f557b329fb335`, fetched from
`origin/feat-happy-ghast-s1`. Compatibility baseline:
`b48caaf06e8d13dbf0672bfbddb14fb66e4ee3b3` (peeled
`murgicraft-baseline-202610-1`). This branch retains a bounded navigation
prerequisite. It does not implement or enable HG2's baby modes, flying travel,
core/idle brain, or `ghastling_flight` configuration field.

## Supplied vanilla and current Rust chain

Vanilla 26.3 `HappyGhast.java:598-610` supplies the baby navigation constructor
and `canMoveDirectly`; `PathNavigation.java:384-390` defines false as
`ClipContext.Fluid.NONE`, with COLLIDER shapes and destination Y plus half
height. The new sibling uses those semantics and rejects unavailable blocks.
The existing required length already reaches the real search via
`PathNavigation::mob_max_follow_range` and `compute_path`; no shared search
algorithm change is needed.

The remaining chain is constructor / loaded Age / age boundary -> baby setup ->
`customServerAiStep` -> HappyGhastAi CORE and IDLE -> sinks / RunOne -> navigation
-> FlyingMoveControl -> species travel. Current Rust's local
`HappyGhastEntity::after_base_tick` is before AI/travel. Its current
`custom_server_ai_step` is after navigation and before controls. That chain is
deliberately unchanged until the ownership requirements below are proved.

## Concrete prerequisites

- `MobEntity::tick_brain` calls behaviors under the public brain slot mutex.
  `write_mob_nbt` packs that same slot; `read_custom_nbt` replaces it before
  loading age. Taking the Brain and restoring it unconditionally can save an
  empty placeholder or overwrite a native write/load replacement. Current
  `MemoryStore` has no mutation revision or erase journal. A dedicated owner
  protocol must preserve writes, erases, replacement and durable save images,
  then validate the exact instance and mode generation before applying effects.
- `MobEntity::server_ai_step` detaches goal selectors and restores them
  unconditionally. Reentrant age/load changes need obsolete-mode rejection,
  without making callbacks wait on their own detached operation.
- `PositionTracker` exposes virtual position/visibility calls but no entity
  identity for the supplied sink's spectator check or player session/lifecycle
  validation. Any adapter must keep these calls outside private state guards.
- `FlyingMoveControl` uses the same no-gravity setter as native callers. The
  boolean has no provenance, so restoring a captured false can erase a later
  explicit true. Current Entity NBT also lacks a NoGravity codec. Prove a local
  ownership adapter before promising explicit-flag preservation.
- Generic packed-Brain loading discards unknown/unsupported/undecodable entries;
  the default species Brain ignores its input. Optional loaded Brain data must
  remain retained or unavailable rather than be overwritten with empty state.
- The supplied MoveToTargetSink fallback depends on `DefaultRandomPos`; obtain
  its exact 26.3 transitive source before porting it. Upstream open PR #4092
  already adds look/move sinks and AI ownership changes; inspect its latest
  interfaces read-only before a dedicated prerequisite rather than duplicate it.

These protocols are unimplemented and unproved, not claimed impossible. Folding
them and their lifecycle regressions into the species port would broaden the
requested small HG2 slice. No partial brain or baby-mode registration is enabled.

## Bounded navigation callers, state and lifetime

- `PathNavigation::new` initializes the private option false; Default delegates
  to it. Ground, Flying, WaterBound, WallClimber and Amphibious constructors,
  and Default-based test literals, preserve their old semantics. Only the new
  `Navigator::ghastling` sets it true, doors false, float true and length48.
- The internal kind's update gate, temporary feet position and waypoint
  tolerance match Flying. Only its direct corner-clearance branch differs.
  The existing generic geometry helper and fluid behavior remain unchanged.
- Existing production factory consumers: default MobEntity, villager reset,
  parrot, allay, bee, nautilus, fish, dolphin, tadpole, guardian, elder guardian,
  frog, axolotl, turtle, strider, spider, cave spider, drowned mode resets and
  both HG1 Happy Ghast modes. None opts into the new constructor. Hostile
  Ghast retains its separate controller/travel.
- `set_progress`, `create_path`, `recompute_path`, path following, `move_to_path`,
  waypoint propagation and `stop` keep their existing owners. Stop clears path
  and pending goals, not constructor options. `configure_movement` replaces and
  drops the whole navigator. No new locks, ownership edges, callback registration,
  packet effect, entity handle, registry, admission or durable writer is added.
- Save capture serializes Entity/Living/Mob/species NBT, not Navigator. Pending
  creation, summon, egg EntityData, dried-ghast birth, region restoration,
  natural growth, native age/NBT reload, removal, unload/reload, world transfer
  and restart therefore continue using their existing registrations and owners.
  No hidden unloaded tick or force-load is introduced.
- Explosions/damage, pistons and other-entity push can change position/velocity
  or remove the entity; they do not write this option. Existing mark-before-
  removal, UUID reservation, event-before-publication and riding-root saves are
  unchanged. Player death/respawn, disconnect/rejoin, dimension/session
  replacement are relevant to a future entity-tracked brain target; this
  bounded constructor retains no target or player.
- Both Wasm adapters call existing navigator `set_progress`; neither constructs
  PathNavigation or selects this option. Native API8 stays8. Public containing
  layouts change, requiring native rebuilds; constructors remain source-compatible.

## Compatibility limit and remaining verification

HG1 saves `home_pos` as IntArray and `home_radius` as Int. The exact baseline
HappyGhast reader/writer handles only `still_timeout`, so it loses those home
fields on re-save. HG2 must disclose this inherited failure, not repeat HG1's
old downgrade caveat as a no-data-loss claim. This prerequisite adds no durable
form and does not redesign age/home ownership to hide the loss.

The exact baseline production reader was executed with a scratch test-only
harness using `World::restore_entity_tree`, real region save and another
baseline load/save after one baseline restart. Its Happy Ghast source blob matches the tag:
`285f3846b4a94258f3360e159d67c60edf89c16d`. The probe retained Age Int(0),
ForcedAge Int(21), AgeLocked Byte(1), Health Float(20), still_timeout Int(0),
literal Pos/Motion/UUID, every saved attribute and the white BODY harness with
its components. It lost `home_pos` IntArray([8,100,-9]) and `home_radius` Int(32)
on the first re-save; those fields remained absent after restart. The production
Entity/Living/Mob/species readers were unmodified; only a test-module declaration
was added to the scratch baseline worktree. This proves an inherited downgrade
failure, not successful no-data-loss compatibility.

The new regressions cover actual navigation searches and follower ticks only.
They do not establish real birth/summon/egg flight, brain sink behavior,
age-mode transitions, stale generation rejection, media travel or runtime
switch behavior. Full HG2 acceptance and negative controls for those omitted
features remain pending. In-game verification: Not yet.
