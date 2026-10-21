# Game-event source membership (s2, refs #162)

`world/game_event_sources.rs` is a transient, world-local identity index. The
existing entity/player lists still own publication, ticking, selection and saves;
tracking audiences do not authorize game-event sources. Index values hold Weak
entity references. The Arc identity of each membership record is its admission
incarnation, including withdrawal/re-admission of the same Player Arc.

The checked source-index owner is level 20. Admission computes registry images,
IDs, Weak handles and base addresses before acquiring it. Registry CAS and index
publication occur together. The nonplayer update closure runs outside the owner;
a failed CAS recomputes outside it. Player withdrawal uses only Arc/Vec operations
under the owner. No EntityBase virtual method, lifecycle/client access, plugin,
packet, external callback or await runs under the index owner. Capture clones an
owned entry and releases the owner before upgrading/validating it. All unsealed
native getters resolve first; the final keyed token check and life/session scalar
checks follow them with no further virtual calls. Getter reentry can therefore
not conceal a same-Arc return transfer or a later life revision. Removal drops
retired entries after unlocking. No new nested state-lock acquisition is needed.

Publication owners: `spawn_insertion::insert_reserved_entity_batch` (fresh,
silent, restored and passenger trees), `server/player_list::add_pending_player`,
ordinary/deferred player transfers, and respawn's destination admission. Retirement
owners: marked entity cleanup, successful chunk retirement, source projectile
transfer, player removal and respawn rollback. Expected base-address deletion
cannot remove a replacement entry. Removed entities reject forwarding before
cleanup finishes. Runtime handles/entries are rebuilt through admission after load.
Projectile transfers retain the original membership token before destination
callbacks, even with forwarding OFF. Delayed cleanup compares that token, so a
same-Arc return transfer cannot lose its new admission. The retirement closure owns
no guard and never waits on reentry.
World shutdown closes and empties source authority before its first await. Late
registry publication cannot reopen that retired world's index; a fresh World
starts with an open, empty index.

The game-event boundary upgrades the server and reads the startup switch before
calling any context resolver. OFF retains the raw plugin event but captures no
source, state or action token. ON uses a bounded number of hash probes and scalar
identity/life/session checks; a missing indexed ID never scans the entity, player,
tracker, viewer or listener collections. Current getters used by commands/native
and Wasm APIs keep their prior semantics. SHOOT attribution reuses the existing
projectile ownership cache: cold UUID recovery can still scan the existing world,
player and loaded-world getters. That existing gameplay lookup is outside the
indexed capture/membership guarantee; total cold SHOOT/tick work is not claimed
to be population-independent. No new owner schema or global getter rewrite is made.

An action snapshot owns no guard. Projectile land captures it before hit callbacks;
flower pots, item consumption, transformations, bucket/bottle use, shear, roar and
replace-disk effects retain it around their local callbacks. Living damage uses the
existing HurtContext lifecycle admission, then retains the source incarnation around
armor/stat callbacks. The notification still snapshots source position and required
block state at its existing emission boundary. GenericGameEvent may mutate only
key/position/cancellation; forwarding revalidates both the emission and action
identities after its callback. Required action capture failure rejects forwarding.
Normal early projectile retirement remains deferred to the hit-admission owner;
its existing plugin notification is retained, with no source-less typed fallback.

The public raw `World.players`/`World.entities` ArcSwap fields are legacy uncertified
native mutation surfaces. Direct external stores bypass the supported owners and
cannot synchronize this index; their behavior is unsupported/unverified. Missing
entries fail closed; an externally retained alive Arc removed only by a raw store
cannot be certified as stale by this index. There is deliberately no scan fallback.
Native plugins must use existing admission/removal APIs and rebuild for the World
layout change. API 8 and both Wasm event mappings/WIT remain unchanged. No index,
action token, counter or lifecycle revision is written to NBT.

## Complete emitter inventory

Paths below are relative to `crates/pumpkin-core/src`. This inventories every
existing production event factory, including unchanged/deferred sites; it does
not claim the deferred contexts are implemented. The original audit found 61
mechanical calls including forwarding wrappers. The final notification-factory /
public-wrapper inventory has 59: the two dried-ghast boundaries share one factory
and the two fishing-hook LAND boundaries share one factory. No notification
boundary was removed. The table contains 57 external calls, followed by the two
existing s1 factories inside `game_events.rs`. Six new internal forwarding calls
are excluded from that count and listed separately below.

| Rust factory | Final disposition / owner |
| --- | --- |
| `block/blocks/beehive.rs:74` | Deferred: hive harvest action/admission owner; actual player known |
| `block/blocks/beehive.rs:226` | Deferred: hive release admission; released bee currently unpublished |
| `block/blocks/candles.rs:132` | Deferred: nullable candle/potion extinguish source owner |
| `block/blocks/cauldron_bucket.rs:246` | Deferred: #394 bucket/cauldron item payment + block mutation owner |
| `block/blocks/dried_ghast.rs:85` | Adapted; exact Java context below |
| `block/blocks/flower_pots.rs:32` | Adapted; exact Java context below |
| `block/blocks/flower_pots.rs:67` | Adapted; exact Java context below |
| `block/blocks/redstone/daylight_detector.rs:47` | Deferred: player block-state commit owner; actor+new-state needs action admission regression |
| `block/blocks/redstone/dispenser.rs:589` | Legitimately actorless; unchanged |
| `block/blocks/redstone/dispenser.rs:1025` | Legitimately actorless; unchanged |
| `block/blocks/redstone/lever.rs:45` | Legitimately actorless; unchanged |
| `block/blocks/respawn_anchor.rs:85` | Deferred: player block-state commit owner; actor+new-state needs action admission regression |
| `block/blocks/sculk/sculk_shrieker.rs:110` | Deferred: s4+ shrieker consumer/lifecycle owner |
| `block/blocks/tnt.rs:136` | Deferred: #330/#394 explosion/ignition/hit admission owner |
| `block/blocks/turtle_egg.rs:81` | Actorless; state-only adapter deferred to automatic block lifecycle owner |
| `block/entities/barrel.rs:127` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/barrel.rs:133` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/chest_like_block_entity.rs:282` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/chest_like_block_entity.rs:294` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/ender_chest.rs:60` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/ender_chest.rs:65` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/jukebox.rs:287` | Actorless; state-only adapter deferred to automatic block lifecycle owner |
| `block/entities/shulker_box.rs:162` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `block/entities/shulker_box.rs:167` | Deferred: container viewer lifecycle must thread actual ContainerUser; never infer viewer |
| `enchantment/effects/replace_block.rs:52` | Adapted; exact Java context below |
| `enchantment/effects/replace_disk.rs:154` | Adapted; exact Java context below |
| `enchantment/effects/set_block_properties.rs:56` | Adapted; exact Java context below |
| `entity/area_effect_cloud/bottle.rs:43` | Adapted; exact Java context below |
| `entity/death_protection.rs:107` | Already s1 |
| `entity/death_protection.rs:231` | Already s1 |
| `entity/decoration/armor_stand.rs:321` | Deferred: equipment commit owner |
| `entity/decoration/leash_knot.rs:443` | Deferred: leash link transaction/removal owner |
| `entity/item.rs:690` | Deferred: DamageCause item damage owner; do not attribute victim |
| `entity/leash_shearing.rs:110` | Deferred: leash link transaction/removal owner |
| `entity/living/blocking_response.rs:206` | Adapted; exact Java context below |
| `entity/living/fall_flying.rs:77` | Deferred: s3/#406 movement/flight owner |
| `entity/living/hurt_server.rs:170` | Adapted; exact Java context below |
| `entity/living/item_use.rs:150` | Adapted; exact Java context below |
| `entity/living.rs:582` | Already s1 |
| `entity/living.rs:2247` | Deferred: consumable teleport admission owner |
| `entity/passive/rabbit_goals.rs:339` | Deferred: rabbit raid AI/crop mutation owner (s3 independent) |
| `entity/projectile/collision.rs:208` | Adapted; exact Java context below |
| `entity/projectile/firework_rocket.rs:149` | Deferred: #330/#394 explosion/ignition/hit admission owner |
| `entity/projectile/fishing_bobber/collision.rs:123` | Adapted; exact Java context below |
| `entity/projectile/ownership.rs:158` | Adapted; exact Java context below |
| `entity/projectile/potion_water.rs:126` | Deferred: nullable candle/potion extinguish source owner |
| `entity/shearable.rs:66` | Adapted; exact Java context below |
| `item/items/block_transformer.rs:97` | Adapted; exact Java context below |
| `item/items/bucket.rs:143` | Adapted; exact Java context below |
| `item/items/bucket.rs:203` | Deferred: #394 bucket/cauldron item payment + block mutation owner |
| `item/items/fishing_rod.rs:144` | Already s1 |
| `item/items/glass_bottle.rs:237` | Adapted; exact Java context below |
| `item/items/lead.rs:80` | Deferred: leash link transaction/removal owner |
| `world/explosion/block_triggers.rs:42` | Legitimately actorless; unchanged |
| `world/explosion/block_triggers.rs:100` | Actorless; gate previous-state context deferred to #330 block-trigger owner (door/trapdoor null-state) |
| `world/explosion.rs:154` | Deferred: #330/#394 explosion/ignition/hit admission owner |
| `world/mod.rs:6693` | Actorless public wrapper; unchanged payload/API |
| `world/game_events.rs:216` item-interact-finish | Already s1; self, no state |
| `world/game_events.rs:228` teleport | Already s1; self and departure, no state |

Internal forwarding in `world/game_events.rs`: `emit_game_event_from_action:150`
calls `emit_game_event_after_action` for a required actor or the actorless public
wrapper for None; `emit_game_event_after_action:181` calls validation;
`emit_game_event_from:199` calls `emit_game_event_with_context`;
`emit_game_event_with_context:239` calls the lazy helper;
`emit_game_event_with_lazy_context:248` calls validation. These are the six
internal calls. `emit_game_event_with_validation:257` owns raw notification and
final routing. Only the
existing final dispatcher observes accepted typed events. The raw public wrapper
still produces one GenericGameEvent. Empty-bucket pickup and generic block
placement/break have no existing factory to adapt; no new events are introduced.

## All membership owners and states

- **Readers and derived state:** indexed capture clones an owned entry; capture
  and post-callback validation check pointer, runtime ID, UUID, world, removal,
  life revision and client closure. Public ID/UUID/name/box lookup, tracking
  audiences, ownership caches, spawn accounting, chunk activation, player area,
  inventory and equipment retain their existing owners. Zero tracking range does
  not prevent live registry admission. Missing IDs have no scan fallback.
- **Fresh/restored nonplayer publication:** `spawn_insertion` reserves UUIDs,
  fires spawn callbacks, validates pending trees, then publishes the complete
  admitted batch. Natural/command/egg/item/projectile/mob/raid/structure/portal
  callers share this owner. Silent insertion and restored passenger trees use
  the same batch. Cancelled, denied, pending and unloaded records have no entry.
- **Player/session publication:** `Server::add_player`, `World::add_player` and
  `add_pending_player`; ordinary/deferred dimension transfer; respawn destination
  admission and exact-pointer rollback. Source-world mismatch rejects an early
  destination record. Same-Arc respawn advances life; same-Arc re-admission
  replaces membership identity; same-UUID rejoin creates a distinct connection.
- **Destroyers:** `Entity::remove`/marked `finish_entity_removal`, painting removal,
  item-frame mark/finish, kill/death/despawn/discard/plugins; chunk retirement of
  roots and riders; successful projectile transfer; exact player withdrawal and
  respawn rollback. Marking still precedes external effects. Replacement cleanup
  checks expected instance; delayed projectile cleanup checks captured membership.
- **Save/unload/restart:** existing riding-root capture, retirement commit and
  entity/player codecs are unchanged. Index removal happens after irreversible
  chunk removal marks and outside its admission guard. Shutdown closes authority
  before await; late publishers cannot reopen it. Fresh World/load admission
  rebuilds entries. No index/token/revision/counter is durable.
- **Commands/native/plugins:** summon, kill, transfer, real login and supported
  public admission/removal APIs use these owners. Generic event key/position/
  cancellation remain mutable; source/state snapshots are private. Raw native
  ArcSwap stores have the unsupported limitation stated above. Wasm mappings and
  WIT are unchanged. The test-only replacement-player helper retires old fixture
  memberships before publishing through the real player owner.
- **Other destroyers and non-applicable cases:** explosions, pistons, redstone,
  fluids and plugins mutate terrain through existing block owners; they do not
  directly publish entity membership. A hit may discard its projectile through
  shared removal, which rejects typed forwarding while preserving the old raw
  notification. Block replacement inside GenericGameEvent cannot change captured
  state. Pending block-entity admission remains unchanged; there is no production
  vibration listener registry, listener persistence, traversal or sensor output.

## Exact Java 26.3 context evidence

These are line references into the read-only Mojang-named 26.3 reference used in
review (`0b127b181faf16fb505437471c2146c19fe5f1c5`); no Java source is copied here.

| Chain / factory | Java file and lines | Context |
| --- | --- | --- |
| Common | `world/entity/Entity.java:1461-1467`; `world/level/LevelAccessor.java:91-104`; `world/level/gameevent/GameEvent.java:92-104`; `server/level/ServerLevel.java:1132-1134`; `world/level/gameevent/GameEventDispatcher.java:22-61` | Entity delegation; independent nullable source/state; loaded-section dispatcher |
| Flower | `world/level/block/FlowerPotBlock.java:47-91` | Player; no state |
| Lever use / explosion | `world/level/block/LeverBlock.java:56-88` | Both explicitly null; the task's player-lever prose conflicts with its supplied exact body |
| Bucket / bottle | `world/item/BucketItem.java:157-161`; `world/item/BottleItem.java:39,59` | Nullable bucket user / bottle player; no state |
| Transform | `core/component/BlockTransformer.java:101,106` | Player plus committed updatedShape / current partner state |
| Consume / hurt / roar | `world/item/component/Consumable.java:90`; `world/entity/LivingEntity.java:1976`; `world/entity/monster/Ravager.java:238` | Self; no state |
| Shear | `world/entity/animal/sheep/Sheep.java:152`; `world/entity/animal/golem/SnowGolem.java:147`; `world/entity/animal/cow/MushroomCow.java:129`; `world/entity/monster/skeleton/Bogged.java:76` | Actual player; no state |
| Enchantment | `world/item/enchantment/effects/ReplaceBlock.java:30-36`; `SetBlockProperties.java:33-40`; `ReplaceDisk.java:40-54` | Passed effect entity, not owner; no state |
| Hydration | `world/level/block/DriedGhastBlock.java:85-107` | Null actor plus previous block state, wet and dry |
| Projectile | `world/entity/projectile/Projectile.java:130-139,348-365`; `world/entity/projectile/arrow/AbstractArrow.java:278-317,423-510,526-546`; `world/entity/projectile/LlamaSpit.java:43-91`; `world/entity/projectile/hurtingprojectile/DragonFireball.java:30-61`; `world/entity/projectile/FireworkRocketEntity.java:200-205,208-216`; `world/entity/projectile/FishingHook.java:266-291` | SHOOT owner; LAND projectile; entity hit no state; block hit state after specified callbacks; subclass order preserved |

Container user propagation, early-retired hit/admission, explosion/payment,
movement/flight and shrieker/listener owners remain separate follow-ups. Read-only
handover PR review found no covering indexed-membership implementation. #330 at
`e264115d1c4dc448c2debe966ec44c437a4ae34d` now supplies projectile source and
broken cracked-pot state for its successful decorated-pot BLOCK_DESTROY commit;
that adapter stays with #330 and is not duplicated here. When combining, retain
s2's original-action capture **before** #330's first hit callback and preserve its
cancellation argument threading. Both branches add `game_events/projectile_tests.rs`;
combine or rename that test sibling and retain one module declaration and both
families' assertions. #408 owns tool transaction/feedback; this slice only carries
event context.

#411 independently uses Scenario construction/restart's third boolean for horse
taming, while this slice uses it for game events. A future merge must preserve
both startup flags and both values across restarts; choosing either three-argument
version would silently couple the switches. #401's sleep/leave changes and #390's
inventory packet changes share `player.rs` but not s2's transfer publication hooks.

The final refresh also found #405 at `a960bb8d91931863f59f90787c40267193806679`
adding registry commits in `boat_destruction.rs` and `boat_portal.rs`, absent from
this base. A future merge must index prepared item/destination tree publication at
those commit owners, retire each source incarnation (including canonical same-Arc
container passengers), and preserve s2's pending-player publication in its reserved
arrival path. Publish membership before tracker/effects without carrying guards
across callbacks. Do not import those boat/portal transactions into this slice.
#417 (`5644d9dbafc53e0dd6c13db0fea619e56877e9c3`) owns unknown-NBT retention;
it supplies no source index and does not replace this task's pinned b48 baseline.
