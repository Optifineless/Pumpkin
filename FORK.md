# Murgicraft fork of Pumpkin

This is a fork of [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) maintained for the Murgicraft server. It tracks upstream `master` and adds fixes aimed at running a survival server: redstone, combat, world generation, commands and the plugin API.

## Accepted travel game events, Warden/sculk slice 3 (#162)

Accepted Java position packets and actual clipped nonplayer travel share transient
float distance accounting for STEP and the same vanilla SWIM/FLAP branches.
Plugin-mutated destinations, support/effect block snapshots, Player flight/sneak
policy and entity overrides follow vanilla 26.3 `Entity.move`,
`applyMovementEmissionAndPlaySound` and `getMovementEmission`. Java vehicle packets
account the vehicle's accepted displacement; passenger placement stays silent.
Bat, Allay, Bee and Phantom FLAP use their existing resting/ground state and
transient mob tick clock. Chicken and parrot flap animation state is not
implemented, so it is not synthesized here.
Existing sound, fall, glide, statistics, physics and save owners remain in place.

```toml
[movement_game_events]
enabled = true
```

The default-ON switch controls only this slice's new accounting and notifications.
`sculk_game_events.enabled = false` still leaves plugin notifications enabled.
Counters reset at teleport/new life/reload; no movement counter is serialized.
Direct placement, piston displacement and Bedrock player position packets do not
enter the new movement emitter. Existing s1 source lookup cost remains until s2.
Loaded X/Z columns use vanilla void air beyond build height.
Terrain is sampled only from current loaded chunks; the affected block state is
an immutable notification snapshot. The unchanged s1 boundary revalidates the
source after callbacks, but has no separate terrain-incarnation callback guard.
Concurrent retained-native movement during the inherited unload registry-to-mark
window can still issue a Generic-only callback; s1 rejects typed forwarding for the
retired source. Closing that authority gap remains with #368/#344 and s2.

API 8 (same version): the private transient state changes public `Entity` layout;
the public `MovementGameEventsConfig` and `AdvancedConfiguration::movement_game_events`
field change native configuration layouts. Rebuild native plugins. Exhaustive
configuration literals must add the field or use `Default`; existing method
signatures, event payloads, plugin cancellation and Wasm WIT remain unchanged.
Both Wasm host teleport adapters continue through the existing teleport entry.
There are no new durable fields or migrations. Implemented by Codex with AI review.
In-game verification: Not yet.

Listener lifecycle/selection, travel, sensor/shrieker activation and Warden
consumers remain later slices. This partial emission work does not close #162.

Play test: walk normally, then sneak/fly over stone and wool; expect ordinary
movement and unchanged sounds, with eligible STEP notifications at distance
thresholds. Swim and watch a flying bat; expect their branch-specific notifications.
Repeat with movement notifications disabled; expect unchanged travel, fall,
statistics and fishing notifications. Sensors still do not activate through this
unfinished path. Capture a moving player and a second client's view with plugin
notification counts, including a boat rider and a restart.

## Fresh ordinary-horse statistics (#115, slice 1)

Fresh ordinary horses initialize individual max health, movement speed and jump
strength before existing ageable/mob finalization, matching vanilla 26.3
`Horse.randomizeAttributes` and `AbstractHorse.finalizeSpawn`. Health uses the two
bounded integer draws (15–30 HP); speed and jump retain three independent draws
and Java's widened float constants. Current health clamps to the new maximum
without healing damaged structure templates. Coat and age behavior remain.

The default-ON emergency switch affects only fresh finalization:

```toml
[horse_spawn_stats]
enabled = true
```

Setting it false restores generated defaults for new births.
Saved horses, including old 53-HP, custom damaged and legacy no-attribute records,
retain their existing meaning under either setting. Explicit-NBT summons still
bypass finalization; egg entity data still applies afterward. No migration,
rolled marker, riding simulation, taming or other #115 behavior is included.

API 8 (same version): the additive public `HorseSpawnStatsConfig` and
`AdvancedConfiguration::horse_spawn_stats` field change native configuration and
containing layouts. Rebuild native plugins against this checkout; source users
constructing exhaustive configuration literals must supply the new field or use
`Default`. The provided `Mob::mob_java_spawn_attributes` hook adds a native
vtable entry; source implementations inherit its empty default. Only ordinary
horses supply the initial health/speed/jump bases and modifiers on every Java
spawn pairing, including loaded and explicitly overridden values. Tracker
ownership and other species are unchanged. Existing method signatures and Wasm
WIT are unchanged. Implemented by
GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

Play test: spawn fresh natural, egg and no-NBT summon horses; expect varied
15–30 max HP and full current HP. Save a custom damaged horse and restart; expect
its UUID, health, bases and modifiers unchanged. Disable the switch and create
new horses; expect prior generated defaults, while saved horses remain unchanged.
Record the births and restart, and capture UUID, Health and attribute values.

## Game-event source context foundation, slice 1 (#162)

Fishing use, item release and death-protection game events retain their originating
entity through the existing plugin callback. Successful death-protection teleports
retain their departure position. Built-in forwarding resolves the callback's final
key and position, honors cancellation, and rejects invalid coordinates or a stale
entity, life, world or player session. Source-less emissions remain valid and
unknown keys remain visible to plugins. `[sculk_game_events] enabled = true` is the
default; false disables only the new typed forwarding. No saved-data format changes.

API 8 (same version): the additive public `AdvancedConfiguration.sculk_game_events`
field changes native configuration layout. The public
`sculk_game_events::SculkGameEventsConfig` type is new. Native plugins must rebuild;
source users constructing `AdvancedConfiguration` with a
complete struct literal must add the field or use `..Default::default()`. Existing
event types, constructors, public method signatures and Wasm WIT are unchanged.
Implemented by GPT-6.1 Sol (Codex Cloud).
In-game verification: Not yet.

This is partial groundwork: STEP emission, listener lifecycle, vibration admission
and travel, ordinary/calibrated sensor activation, shrieker/Warden consumers, anger,
sonic attacks and world generation remain separate work. It does not close #162
or fix #156/#128, and walking still does not activate sensors through this path.

Play test: cast and reel a rod, stop using an item, and trigger a supported
death-protection teleport; expect the same fishing, item, consumption and teleport
behavior. Repeat with the setting false; expect the same gameplay and plugin
notification counts. Record those actions and plugin notifications on the candidate
build. No sensor activation repair is claimed.

## Stable Bedrock score identities (#289)

The shared scoreboard assigns transient IDs to logical holder/objective pairs.
Updates, display tracking, cloned snapshots and joins retain the same live ID;
single resets, holder resets and objective deletion retire it without reuse.
Loaded vanilla scores receive IDs before publication. Exhaustion omits new
Bedrock entries while retaining existing IDs and Java/logical score behavior.
This adapts vanilla 26.3 `Scoreboard.getOrCreatePlayerScore`, score resets and
`ServerScoreboard.getStartTrackingPackets` to the existing Bedrock codec.
The owner notification queue, Java packets, saved-data schema and Wasm WIT are
unchanged. The separate legacy `BedrockScoreboard` still publishes empty entry
lists and does not use this path. Implemented by GPT-6.1 Sol (Codex Cloud).

API 8 (same version): private runtime state changes the public `Scoreboard`
layout; native plugins must rebuild. Public method signatures and source
compatibility are unchanged. In-game verification: Not yet.

Play test: display two objectives containing the same holder to Java and Bedrock
observers in different dimensions, repeatedly change the values, then rejoin;
expect one current line per objective. Reset one score, reset the holder, hide
and show an objective, then delete/recreate it and its scores; expect only the
intended lines to disappear and no duplicates after reconstruction. Import an
existing scoreboard, save and restart; expect the logical scores, formatting
and display slots to survive. Record both clients and capture score update and
removal IDs, including the join and restart snapshots.

## Canonical furnace timers and legacy saves (#299)

Furnaces, blast furnaces and smokers load and save all four timer fields as
vanilla 26.3 Ints and retain signed i32 values throughout ticking, speed scaling
and menu properties. Old fork Short tags retain their historical unsigned-u16
bit meaning, including values above 32767, and export as Ints. Missing timers
default to zero; missing speed remains 1.0. Signed cooling mirrors
`AbstractFurnaceBlockEntity.serverTick` and `Mth.clamp`, including progress one
cooling to zero. Items, recipes used, speed, remainder ordering and existing
inventory ownership stay intact. Both persistent capture and chunk packet NBT
use Int tags. Java container packets keep their existing short wire encoding.

API 8 (same version): the public furnace/blast/smoker layouts replace four
`AtomicU16` fields with `AtomicI32`. `CookingBlockEntityBase` timer getters,
setters and `get_total_cook_time` now use i32, changing native signatures and
trait-object ABI. Rebuild all native plugins against this checkout; source
users must update u16 arguments/results and exported cooking-macro backing
fields to i32/AtomicI32. Wasm WIT stays unchanged: its existing u16 getters
project the low 16 bits without changing durable state. Event payloads stay
u32; nonpositive signed burn durations cannot consume fuel. Implemented by
GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

Play test: import lit vanilla furnaces, blast furnaces and smokers containing
partly cooked items, then restart; expect cooking and remaining heat to resume
with the same stock. Load an old fork save with a high timer, restart again,
and expect preserved progress and item counts. Record all three menus before
and after restart and capture the saved timer tags as Ints.

## Experimental minecart creation and overlap refusal (#316)

Minecart item placement follows vanilla 26.3 `MinecartItem.useOn`,
`AbstractMinecart.createMinecart/getCurrentBlockPosOrRailBelow` and
`NewMinecartBehavior.adjustToRails/isDecending`: the existing minecart improvements
feature selects initial rail alignment, including slopes, curves and rotation.
Experimental placement rejects intersecting minecarts using the full aligned box;
legacy height and overlap behavior stay unchanged. In-flight item placements reserve
their aligned space without retaining a mutex through callbacks. Spawn admission
revalidates the actor, life, world, source hand and rail incarnation before payment;
cancelled or stale creation keeps the item. Dispenser, load/summon constructors and
movement physics are unchanged. API 8 (same version): public layouts/signatures,
native source compatibility, Wasm WIT and saved-data meaning are unchanged.
Implemented by GPT-6.1 Sol (Codex Cloud).
In-game verification: Not yet.

Play test: with minecart improvements enabled, place a cart on flat, ascending and
curved rails; expect the initial alignment and rotation to follow the track. Use
another cart on occupied rail in Survival; expect one cart and no second charge.
Repeat with both hands, cart variants and two players placing together. Disable the
feature; expect legacy placement height and overlapping placement. Record cart
position/rotation and item counts, including a plugin-cancelled spawn if available.

## Entity-item hooks do not start use cooldowns (#154)

Passing Java and Bedrock entity interactions no longer start a stack's
`USE_COOLDOWN` from the registry after the item hook. This matches vanilla
26.3 `Player.interactOn` and `ItemStack.interactLivingEntity`, which call the
entity-item hook without `applyAfterUseComponentSideEffects`. Existing cooldown
admission and ordinary successful-use cooldowns remain in place. Real Java
handler regressions cover both hands, Survival and Creative, named and
ineligible name-tag cases, cancellation, active cooldowns and close pearl impact.
Native API 8, public signatures/layouts, Wasm WIT and saved data are unchanged.
The reporter's original client sequence has not been captured.

In-game verification: Not yet.

Play test: Hold pearls and look at a nearby cow. Right-click its hitbox, then
right-click air; expect the first action to leave the pearl and cooldown alone,
and the second to throw one pearl and start its cooldown. Repeat in each hand and
in Survival and Creative. Capture the hand counts, projectile insertion, visible
cooldown and, for a close throw, the impact and teleport.

## Extinguished campfire cooking progress (#304)

Unlit campfires subtract two progress ticks, then clamp each positive slot to
zero and its own saved cooking duration. This mirrors vanilla 26.3
`CampfireBlockEntity.cooldownTick` and preserves the existing cooking events,
transaction owner and NBT fields. Negative plugin or imported durations follow
Java's clamp order without panicking. Native API 8, public layouts and
signatures, Wasm WIT and saved-data meaning are unchanged. In-game verification:
Not yet.

Play test: use a plugin that cancels campfire cooking, let progress pass its
duration, extinguish the campfire with a shovel, then relight it. Expect cooling
to stop at the duration, resume cooking normally, and preserve the uncooked
food until cooking succeeds. Capture the campfire NBT, food stacks and any drops.

## Bed respawn collision surfaces (#36)

Bed respawn now follows the vanilla 26.3 yaw-selected candidate order, including
bunk beds and both bed-surface candidates. It derives landing height from the
collision shape and checks player clearance, invalid-spawn blocks and the full
world-border bounds. Anchor and forced-spawn behavior, saved spawn data and
respawn ownership are unchanged. Mirrors `AbstractBedBlock.findStandUpPosition`,
`findBunkBedStandUpPosition`, `DismountHelper.findSafeDismountLocation` and
`BlockGetter.getBlockFloorHeight`. Native API 8, public signatures and Wasm WIT
are unchanged.

PR #391 review round 2: lower bunk candidates now include straw beds, matching
vanilla 26.3 `AbstractBedBlock.isBunkBed`, which checks the `AbstractBedBlock`
family; `StrawBedBlock` extends that class but is not in the `minecraft:beds`
tag. The mixed white-over-straw death-respawn regression exercises the Java
respawn command and leaves only a lower exit clear. The saved-bed End-return
fixture now expects the yaw-selected safe exit at floor height: x=9.5 for yaw
27 and 180, x=7.5 for yaw 0, matching `AbstractBedBlock.findStandUpPosition`
and `ServerPlayer.findRespawnAndUseSpawnBlock` (vanilla 26.3). Negative controls:
the old geometry assertion failed with actual `(9.5, 64.0, 8.5)` versus
expected `(7.5, 64.1, 8.5)`; the tag-only bunk check landed on the upper bed at
`(8.5, 64.5625, 7.5)` instead of the lower exit `(9.5, 63.0, 7.5)`. Saved-data
meaning, API 8, public signatures and Wasm WIT are unchanged.

In-game verification: Not yet.

Play test: set a bed spawn, enclose the bed in stone while leaving clear space
above it, then die and respawn; expect to land on the bed surface with no block
collision. Repeat with an open exit, a white bed above a straw bed, and different
saved yaws; expect the first safe exit in the saved-yaw order. Capture the
respawn position and the bed layout.

## Minecart placement centers once (#129)

Minecart items use the X/Z center already returned by `BlockPos::to_f64()` and
retain the vanilla rail height and ascending-rail offset. Positive and negative
coordinates now place carts over the clicked rail. Mirrors vanilla 26.3
`MinecartItem.useOn`. Upstream draft PR #3794 contains the same placement
correction within broader minecart physics work; this fork change is limited to
placement. Native API 8, Wasm WIT and saved data are unchanged.
In-game verification: Not yet.

Play test: place each minecart item on isolated flat rails at positive and
negative coordinates, then place one on an ascending rail; expect centered
carts, with the ascending cart 0.5 blocks higher. Capture the rail and cart
positions, and verify Survival consumes one item while Creative consumes none.

## Owner review follow-ups, PRs #235, #238, #240 and #243 (PR #257 review round 2)

Fire spread now publishes the new fire state to nearby clients. Players switching
to Spectator while asleep wake immediately and must use a bed again after returning
to Survival. If the bed-leave callback completes another gamemode change during
that wake, the outer Spectator transition preserves the callback's final mode,
abilities, physics and client state. Jukebox playback restored from active NBT
powers adjacent redstone; starting and stopping playback notify neighbours when
the power changes. Beacon beam scans fall back to build height when a stale surface
height is below or level with the beacon, so they still find clear sky and an
obstruction above the beacon. Saved-data meaning is unchanged. Mirrors vanilla 26.3
`FireBlock` spread flag 3, `ServerPlayerGameMode.setGameMode` ability updates,
`JukeboxBlockEntity` neighbour updates, and `BeaconBlockEntity.tick` height
scanning. Immediate waking on a Spectator change is a fork behavior. In-game
verification: **Not yet**.

API 8 (same version): `JukeboxBlockEntity` gains runtime transition state for
neighbour power publication; rebuild native plugins. No public method signatures,
Wasm WIT or saved-data formats change.

Play test: spread fire beside a nearby client and confirm the fire appears; sleep
through the maturity threshold, switch to Spectator and back to Survival, then use
a bed again to skip night. With a test bed-leave plugin that restores Survival,
sleep in a bed and switch to Spectator; expect to wake with normal Survival damage
and building actions. Reload a jukebox with an active record beside redstone dust
and a comparator; expect full dust power while playing, zero after it stops, and
comparator output matching the inserted record. On a beacon with a stale surface
map at or just below the beacon, wait for the scan, select a paid power, then place
stone above it; expect the beam and selection sound to stop after the scan reaches
the stone. Vanilla's `ServerPlayerGameMode.setGameMode` updates mode and abilities
without directly stopping sleep. No real-client or in-game verification is claimed
here.

## Owner review follow-ups for #239, #242 and #243

PR #256 review round 2: axe use retries a rejected clicked write up to three
total attempts, recomputing from the live state while retaining the original
block identity. A neighboring stair's shape update no longer loses wax removal;
replacement blocks still reject the action. Wear, statistics and effects occur
once after success. Twice-panicking Wither deliveries retain failed actions and
their per-recipient successors, finish other recipients, and get one independent
recovery dispatch. Further failures remain queued for a later entry rather than
spawning an unbounded retry chain.

Checked against official vanilla 26.3 `BlockTransformer.transformBlock`,
`CopyPropertiesProvider.getState`, `StairBlock.updateShape/getStairsShape`
and `ServerBossEvent.removePlayer/removeAllPlayers` (embedded server SHA-256
`a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`).
Conditional retries and panic recovery adapt those semantics to fork concurrency
and native callbacks. Native API 8, public layouts, WIT and saved-data meaning
are unchanged. Written by GPT-6.1 Sol (Codex Cloud). In-game verification: **Not yet**.

Real Java use and Wither tick/retirement regressions cover the neighboring stair,
axe retry exhaustion, terminal delivery, persistent failure parking and returning
sessions. Negative controls against the original production code leave the stair
waxed and exhaust the bounded wait for terminal REMOVE. Test synchronization uses
write seams and cooperative poll budgets, without sleeps. Restricting ordering to
one subscription instead of its player session delivers ADD/HEALTH/REMOVE; the
returning-session regression requires REMOVE/ADD/HEALTH.

Play test: sneak-use an axe on a waxed copper stair while another player places
an adjoining stair; expect the wax to disappear, the corner shape to remain and
one durability use. Capture the stair shape, wax-removal effects and durability.

Axe transformations condition every clicked and companion write on its observed
state. Rejected clicked writes do not consume durability or publish transform
effects; hoe and shovel transformations use the same admission, including rooted
dirt drops and campfire extinction. Honeycomb already used conditional writes.
Door companion ordering remains unchanged to preserve paired-block callbacks.

Final lectern retirement reconciles `has_book` and clears `powered` when its
captured inventory has no book, keeping a queued take consistent across reload.
Wither dispatch releases ownership after native packet panics, preserves pending
successors and retries abandoned batches independently of another tick or save.
Registration and update packets share the block-entity publication order, skip
replaced incarnations, and send after mutation ownership releases. Committed tags
also survive an abandoned outer effect batch.

These preserve saved-data meaning. API 8 (same version): private World admission
state gains an ordered client-notification queue; rebuild native plugins. Public
signatures and Wasm WIT are unchanged. Mirrors the concurrency boundaries around
vanilla 26.3 axe/hoe/shovel use, `LecternBlock.resetBookState`,
`LevelChunk.getBlockEntityNbtForSaving` and `ServerBossEvent` delivery. Decompiled
26.3 source was unavailable; no new Java-source parity verification is claimed.
Written by GPT-6.1 Sol (Codex Cloud). In-game verification: **Not yet**.

All ten real-entry regressions have recorded failing negative controls. Bypassing
tool predicates overwrites replacements and produces stale root drops/wear;
omitting retirement reconciliation reloads an occupied empty lectern. Original
Wither dispatch strands health/terminal cleanup, original registration delivery
sends older text last, and canceling a committed notification loses its final tag.
The controls retain the test seams and restore production files afterward.

Play test: scrape both halves of a stocked copper chest and a copper door while
another player replaces a half; expect the replacement to remain and each
successful use to wear the tool once. Take a lectern book near simulation distance
and restart; expect one book returned and an empty, unpowered lectern. Kill a
Wither and replace a sign while another player watches; expect the bar to disappear
and the final sign text to remain. Record the item counts, reload, HUD and sign.

## Configured biome source for structure lookup (#121)

Cherry-picked upstream [#4029](https://github.com/Pumpkin-MC/Pumpkin/pull/4029),
`8677986a5876cdcc77bf674d4114aa75b5e32329` by WhiteProject1, preserving its author.
Locate uses the generator's configured biome source and generation column heights,
including End, Nether and fixed-biome worlds. Fork regressions exercise the registered
locate executor, cold locate/start/reference ordering, real End City templates,
invalid sites and bounded concurrent publication. Start and reference contexts use
the raw generator bounds from vanilla 26.3 `WorldGenerationContext`, independently
of column clipping. Concurrent cache misses return the first published mutable
piece collection or absent result; upstream #3335's broader concurrency change is
not included. Saved-data meaning, plugin events, public API 8 and Wasm WIT are
unchanged. Fork follow-up and tests implemented by Codex (GPT-6.1 Sol).
PR #315 review: locate resolves the set's first successful weighted entry before
matching requested structures, including failed-entry fallback. Impossible requests
are rejected against the configured biome source before scanning; vanilla's terrain
column, rotated jigsaw center and monument surrounding-biome checks precede deferred
noise-height sampling. Shared cache keys, generation/reference behavior, saved-data
meaning and API 8 layouts remain unchanged. Full-radius command regressions enforce
zero sampler constructions for impossible sources; winner/order and ordinary village
controls cover the successful path. Follow-up implemented by GPT-6.1 Sol (Codex Cloud).
Guardrail bookkeeping refreshes only the command-family hash and moves the existing
jigsaw collector mutex key; prior statuses remain intact and no lock is added.
Disabled-set filtering and precomputed per-dimension placement preparation remain
separate backlog work; existing generated chunks are not repaired.

Play test: in the Overworld run `/locate structure minecraft:end_city`; expect not
found promptly while the world keeps ticking. Locate a village and visit the reported
position in newly generated terrain; expect a village. In a Nether test world with
seed 0 and fixed nether_wastes, stand at X=64, Z=896 and locate a fortress; expect a
different chunk, with a bastion at the original chunk. Capture command feedback and
the visited structures. In-game verification: Not yet.

## Live player state after drop callbacks and skin settings (#163, #291, #296)

Held drops reconcile live equipment after cancelled or aborted callbacks, without
consuming replacement stock. The existing tick publisher rejects retired player
instances. Java Q and Control-Q reset attack recharge after every drop attempt,
including empty hands and cancellation. Initial tab portraits use the actual hat
setting; accepted changes reach online observers in every dimension through the
existing player-list owner. Settings guards release before callbacks and sends.
Mirrors vanilla 26.3 `ServerPlayer.drop`, `LivingEntity.detectEquipmentUpdates`,
`ClientboundPlayerInfoUpdatePacket.Entry`, and
`ServerGamePacketListenerImpl.handleClientInformation`.
API 8 (same version): public signatures/layouts and native source compatibility
are unchanged; no new rebuild requirement. Wasm WIT and saved-data meaning are
unchanged. Implemented by GPT-6.1 Sol (Codex Cloud).
In-game verification: Not yet.

Play test: with a nearby observer, use a drop hook that replaces a spear with a
sword, then one that cancels and selects another slot; expect preserved stock,
matching sword damage/recharge and the observer's current hand. Press Q and
Control-Q after charging an attack, including an empty hand or denied drop;
expect recharge to restart. Disable the hat layer before an observer joins, then
toggle it with observers in the Overworld and Nether; expect every tab portrait
to follow the setting without duplicate entries. Record both player views,
inventory/ground counts, the recharge indicator and both tabs.

## Enchanting offer minima and additional rolls (#270, #271)

Enchanting offers below their slot's minimum cost are disabled before their
clues and properties are published. Additional enchantments compare the next
random roll with the full adjusted cost, then halve the cost after each
successful extra selection. Mirrors vanilla 26.3 `EnchantmentMenu.slotsChanged`
and `EnchantmentHelper.selectEnchantment`. Existing payment, seed conversion,
book removal, one-book slot limit and plugin events are unchanged. No plugin API,
WIT or saved-data changes. Implemented by Codex (GPT-6).

In-game verification: Not yet.

Play test: Use an enchanting table with zero or one bookshelf and enough XP;
expect offers below their slot minimum to appear disabled and remain unchanged
after clicking. Add fifteen bookshelves and enchant a sword and a stack of books;
expect higher offers to work, compatible additional enchantments to match Java,
and only one book to be enchanted from the stack. Capture the table properties,
resulting items and XP/lapis changes.

## Captain banner drops and ominous bottles (#279, #280)

Patrol finalization, raid-wave leadership, replacement leadership and
`/raid spawnleader` preserve captain head banners with vanilla's 2.0 drop chance.
The registered pillager loot table now admits the raider predicate and ominous
bottle amplifier modifier. Death snapshots require the patrol-leader flag and
the actual canonical head stack, and include both raid membership and a raid at
the position. Amplifiers use the existing named loot stream and integer provider,
clamped to 0..4. An unsaved raid tick view keeps positional loot queries valid
during callbacks while the tick owns the raid map, and stops become visible
before their callbacks. Mirrors vanilla 26.3 `PatrollingMonster.finalizeSpawn`,
`Raid.setLeader`, `Raider.isCaptain/hasRaid`, `RaiderPredicate.matches` and
`SetOminousBottleAmplifierFunction.run`. Existing drop gates, plugin events and
saved tags retain their meaning. The canonical banner provider is shared with
PR #221; its appearance and codecs remain that PR's responsibility.
Written by GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

API 8 (same version): public `EntityLootState` adds two optional raider snapshot
fields; `Raid` and `Raids` gain private runtime tick-view state. Rebuild native
plugins. Exhaustive loot-state literals must supply the fields or use
`..Default::default()`. External raid struct literals cease to compile; use
`Raid::new` or `Raids::default()` instead. Constructor/default callers remain
source compatible. Native API version, trait vtables, Wasm WIT and save formats
are unchanged.

Play test: kill a patrol captain outside a raid; expect one captain banner and
one ominous bottle. Kill raid-wave and replacement captains; expect the banner
and no bottle during the raid. Kill an ordinary pillager or a leader wearing a
plain white banner; expect no bottle. Disable mob drops and repeat; expect no
equipment or bottle drops. Capture each death, ground items and bottle level.
Existing patrol finalization has no automatic spawn caller on this base; these
changes fix its equipment assignment without adding a separate spawn feature.
PR #332 review: Raid Omen releases the raid mutex before statistic callbacks.
Replacement leaders require an active raid, vacant leadership and a living,
pickup-eligible exact captain banner; nearby pillagers preserve bottles and junk.
Saved RaidId resolves through the registry, and death removes current membership
before loot, including during raid-tick callbacks. Both omitted raider predicate
fields default to false, as in the vanilla codec. API 8 (same version): Raids adds
private pending membership state; rebuild native plugins. Saved tags and native
API version remain unchanged. Source bodies, state audit and regression results:
[PR #332 round 2](.fork/pr332-review.md). In-game verification: Not yet.

Play test: kill a patrol captain with another pillager beside it; expect the bottle
and banner to remain outside a raid. In a raid, let a pillager collect a dropped
captain banner after its pickup delay; expect one replacement captain, while the
bottle remains. Capture the deaths, ground items and replacement leader.

## Detached framed-map markers (#294)

Retired item-frame contents remove their saved frame record and FRAME icon through
the map manager, retaining banners, other frames, pixels, names and unknown NBT.
Cold maps keep one cleanup obligation per map/attachment until normal loading;
shutdown drains reads and retries unresolved cleanup once before the existing map
save gate. Attachment incarnations reject stale cleanup after replacement, and
live NBT edits retain the old position. Creative, disabled-drop, Fixed and cancelled
branches retain their supplied vanilla 26.3 `ItemFrame.dropItem/removeFramedMap`,
`onKilled/readAdditionalSaveData` and `MapItemSavedData.removedFromFrame` behavior.
No live frame-marker creation or rendering feature is added. Written by GPT-6.1 Sol
(Codex Cloud). In-game verification: Not yet.

API 8 (same version): `ItemFrameEntity` and `MapManager` gain private runtime
attachment/cleanup state; rebuild native plugins. Public signatures, native API
version, Wasm WIT and saved-data meanings stay compatible. Unknown saved root/map
fields now survive map writes; no new saved fields are introduced.

PR #331 review: existing-ID `MapManager::create_map` again returns the cached
handle without locking the map, including when its native caller already holds
that mutex. Deferred frame cleanup remains with retirement, load completion and
shutdown retry, preserving its obligation while the map is busy. API 8 (same
version); public signatures, layouts and saved-data meanings are unchanged by
this follow-up. In-game verification: Not yet.

PR #331 round 3: loader cleanup only tries a busy map mutex and retains pending
work for the next server tick, load or save. Frame destruction queues runtime
ownership retirement without taking the cleanup mutex; unattached frames do
nothing. Position-only live NBT edits retire the old attachment, while metadata
edits retaining the same map and position preserve its saved icon. Failed map
reads wait five seconds before automatic retry; creation and explicit import/load
can recover immediately. API 8 (same version): `MapManager` gains private runtime
queue/backoff state; rebuild native plugins. Public signatures, Wasm WIT and saved
fields/meanings are unchanged. Written by GPT-6.1 Sol (Codex Cloud).
In-game verification: Not yet. Duplicate-UUID summon ownership, current-attack
cancellation and the pre-existing missing live marker rotation/ID refresh remain
backlog.

Play test: display an imported map in an ordinary/glow frame and break its support;
expect one frame/map drop and its icon gone after save/restart, with another frame
and a named banner unchanged. Repeat Creative attacks and support loss with
entity drops disabled; expect cleanup without item drops. Fixed and cancelled
frames retain their contents and markers. Capture the map, drops and restart result.

## Item-provided block sounds (#18, #265, #266)

Player heads retain their valid `note_block_sound` component through placement,
ordinary drops and reload. Note events play that identifier from the block center
in RECORDS at volume 3 and pitch 1; unconfigured heads fail the event silently.
Tagged heads used on a note block's top face reach ordinary placement and its
existing hand payment and cancellation rules. Other clicks retain tuning.
Mirrors vanilla 26.3 `NoteBlock.useItemOn/triggerEvent/getCustomSoundId` and
`SkullBlockEntity` implicit components and load/save methods.

Explicit registered `jukebox_playable` patches save as vanilla's direct song
identifier and accept earlier fork compounds. Existing invalid fork references
retain their stopped-record meaning. Real insertion and disk reload preserve the
chosen song and active elapsed ticks; ordinary default discs are unchanged.
Mirrors `JukeboxPlayable.CODEC/tryInsertIntoJukebox` and
`JukeboxBlockEntity.loadAdditional`. Arbitrary inline songs are not represented.
Native API 8, public layouts/signatures and Wasm WIT are unchanged.

PR #336 review: Java item packets now use the synced jukebox-song registry's
holder ID plus one, with matching registered-reference decoding. Stopped legacy
song patches stay in saves and use the item's default component on clients;
all stack, template and trade-cost writers omit those unregistered additions.
Bedrock's existing item conversion sends no song patch. Regressions restore saved
jukebox NBT, eject through a real right-click and independently decode the actual
item metadata and full inventory packets, including the stopped legacy case.
Mirrors vanilla 26.3 `JukeboxPlayable/JukeboxSong.STREAM_CODEC` and
`ByteBufCodecs.holder`. No public API or saved-data meaning changes.
Written by GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

Review play test: with a second Java player nearby, restart with a disc configured
to play Cat in a jukebox, then eject it, pick it up and open the inventory. Expect
both players to stay connected; reinsert the disc and expect Cat. Repeat with an
old stopped record; expect it to remain stopped without disconnecting either
player. Capture the ejection and inventory on both clients.

Upstream #3728 partially overlaps with legacy jukebox writing, and #3507 includes
skull components in a broader redesign. This change keeps the scoped existing
paths. The separate restored-power/D17 follow-up remains owned by fork #257;
the coordinator must settle that subset before closing all of #18.
Written by GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

Play test: place a configured player head on a note block in Survival; expect one
head payment, no tuning, and its custom sound when attacked or powered. Repeat
after breaking/replacing the head and after a restart; expect the same sound.
Use an unconfigured head; expect silence. Insert a disc with a registered-song
patch, save and restart mid-song; expect the same disc/song and retained progress.
Capture sound, placement/payment and the reload sequence with Java and Bedrock
clients before opening the PR yourself.

## Departed dragon and Wither bossbar recipients (#118)

Dragon fights and Withers retain weak handles to exact player subscriptions, so
ordinary dimension changes, cross-world respawns and boss death deliver REMOVE
through the departed session. Retired Withers also drain recipients during their
existing unload serialization, with destruction as a final cleanup fallback.
Replacement sessions with the same account UUID
receive their own ADD. Ordered owner dispatch releases guards before packet
callbacks and revalidates ADD/UPDATE at delivery, including outer effect batches;
departed subscriptions receive no queued health/title updates. The terminal dragon
zero-health packet still precedes REMOVE for a viewer remaining in the source.
Radius, scan cadence,
custom assigned bars, fog attributes, boss AI and saved-data meaning are unchanged.
Mirrors vanilla 26.3 `ServerBossEvent.addPlayer/removePlayer/removeAllPlayers/broadcast`,
`EnderDragonFight.updatePlayers` and `WitherBoss.startSeenByPlayer/stopSeenByPlayer`.
Written by GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

API 8 (same version): `DragonFight` and `WitherEntity` runtime layouts change;
rebuild native plugins. `WitherEntity::bossbar_players` is now private owner state
instead of a public UUID vector; native source that accessed it must stop mutating
recipient membership directly. Public methods, native API version and Wasm WIT
are unchanged. No recipient handles are saved.
Direct plugin discard while retaining a Wither handle can delay cleanup until
that handle is released; normal unload and death paths are covered. A shared
removal notification hook is outside this change's transfer/removal boundary.

Play test: leave an active End fight or nearby Wither for another dimension;
expect its bar to disappear while an assigned custom bar stays. Die near a Wither
and respawn in another dimension; expect the same cleanup. Return or reconnect;
expect one fresh bar, with normal health/title changes and death cleanup. Capture
the HUD and boss-event UUID/order across each transition, including cancellation
and a destination boss. Capture fog/sky appearance too; client-rendered fog remains
unverified, and independent ambient fog needs a separate reproduction.

## Owner review follow-ups, PRs #191 and #213 (#172, #63)

Deferred lectern page and book-removal effects reserve mutation admission before
touching the retained entity and terrain, then release it before notifications.
Rejected block-entity registration images no longer broadcast stale client tags.
Entity unloads keep one asynchronous storage recapture per chunk until completion,
including across watcher generations, and recheck whether completion already supplied
storage before claiming again; failures return to the ordinary tick retry.
Paid beacon selections play the power-selection sound when the completed beam
scan is active, including reselecting the same power. Beam obstruction checks
scan ten blocks per tick, retaining bedrock and colored-glass behavior. Mirrors
vanilla 26.3 `LecternBlockEntity.setPage`, `LecternBlock.signalPageChange/resetBookState`,
`LevelChunk.getBlockEntityNbtForSaving`, `PersistentEntitySectionManager.processUnloads`
and `BeaconBlockEntity.tick/dataAccess.set`.

The two reported beacon persistence races are already prevented by full persistent
captures and publication stamps on this base. New real-handler regressions cover
overlapping paid selections and contended custom-name/lock captures through disk
save/reload. Saved NBT meaning and Wasm WIT are unchanged. API 8 (same version):
`EntityUnloads` adds recapture ownership, `Level` adds begin/finish recapture methods,
and `BeaconBlockEntity` adds runtime beam-scan state; rebuild native plugins.
Implemented by Codex (GPT-6.1 Sol). In-game verification: **Not yet**.

Reviewed broader upstream [PR #3860](https://github.com/Pumpkin-MC/Pumpkin/pull/3860).
Its full beacon port changes loaded `Levels` behavior; this scoped follow-up keeps
the existing saved meaning and adds only the beam activity needed by selection sound.

Lifecycle manifest refresh retains existing statuses and notes, updates three
family hashes and moves twenty existing boundary keys. The only new row is
`boundary:crates/pumpkin-core/src/world/lectern_update.rs:20::World::prepare_lectern_state_update`,
marked `BoundaryCheckedLegacy`; it does not certify snapshot writer exclusion.
The sync guardrail adds one test-only recapture callback slot, whose mutex guard
drops before the callback runs or storage is awaited.

Play test: turn pages and take a lectern book near simulation distance, return and
restart; expect the final page/book state to remain and page turns to pulse redstone.
Have two players pay for different beacon powers, save and restart; expect the final
selection to remain. Pay on a beacon with a visible beam, then cover it with stone
and wait for the beam to stop; expect the selection sound only with the beam.
Record the page pulse, both beacon menus and the audible selection before/after
covering the beacon. No deployment or real-client verification is claimed here.

## Live timed world borders (#181)

Timed borders advance once per game tick. Queries, clamps and orb collision
filtering share vanilla 26.3 `WorldBorder.MovingBorderExtent` previous/current-size
bounds. Commands and arriving clients use the current size, target and remaining
ticks; timed add retains the remaining interval. Immediate set and reset retire
the transition. Saved border tags and startup defaults retain their existing meaning.

API 8 (same version): `Worldborder` gains private moving-extent state and live-query
methods; rebuild native plugins. Existing native setter signatures remain source
compatible; `new_diameter` remains the target, and `get_size()` returns the current
size. Native `set_diameter`/`add_diameter` durations and `speed` retain milliseconds,
rounded up to game ticks. Both Wasm hosts now honor the existing WIT's tick
durations and live-size getters; WIT and packet layouts are unchanged.
Written by GPT-6.1 Sol (Codex Cloud).
In-game verification: Not yet.

## Border and End fallback geometry (#181, #295, #305, #306)

Hanging survival and shared End-return search use the live border bounds,
including its absolute portal limit. Block checks test the coordinate itself;
portal clamps floor the float-clamped position and keep the center-block result
for collapsed sub-block ranges, consistent with upstream PR #3475. End fallback
checks contained fluid height against the player body while retaining the
existing full-column fluid rejection. End-return candidates follow vanilla's
`PlayerSpawnFinder.findSpawnPosition` border exemption for a null entity, so a
border outside the shared spawn cannot force the vertical fallback to the build
ceiling. Non-finite border bounds use a finite center-block fallback in
`clamp_block`. Mirrors vanilla 26.3 `WorldBorder` and `PlayerSpawnFinder`.
Plugin API 8, Wasm WIT and saved-data meanings are unchanged.
In-game verification: Not yet.

PR #335 integration onto `batch-1019k2`: the half-block border centered at
(1, -1) clamps (100, -100) to block (1, -2). Vanilla 26.3
`WorldBorder.clampToBounds` clamps floating coordinates before
`BlockPos.containing` floors them; the valid Z minimum is -1.25, so the old
(1, -1) expectation was incorrect. End-return fixtures await chunk publication
before installing their stone floors, with empty flat generation keeping setup
bounded and independent of terrain-generation ordering. API 8 (same version).
Integration by GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

Play test: Set a timed border resize with a four-wide painting near its edge;
expect it to survive while the live border still contains it, then retire when
the border reaches it. Return from the End with a waterlogged sign in the shared
spawn body space; expect a dry arrival, while a dry sign and fluid below the
player remain valid controls. At a diameter-10 border, use a water bottle on
source water at x=4; expect it to fill, with x=3 also filling and x=5 denied.
Set a diameter-10.4 border and enter a Nether portal from a scaled coordinate
beyond it; expect the destination search to retain upper block 5. Return from
the End at shared spawn (6,64,0) with a diameter-10 border centered at (0,0),
then use `/worldborder set 1` with spawn (0,64,0); expect a Y64 arrival without
fall damage in both cases.

## Held-item drop equipment publication (#163)

Accepted Java and Bedrock held drops queue the current main hand for the normal
equipment publication tick. Empty hands lose weapon modifiers and observers see
the committed hand; partial stacks, cancelled drops and callback source/selection
changes retain existing withdrawal and plugin contracts. Mirrors vanilla 26.3
`LivingEntity.detectEquipmentUpdates/collectEquipmentChanges/handleEquipmentChanges`.
Native API 8, Wasm WIT and saved-data formats are unchanged. Implemented by
Codex (GPT-6.1 Sol). In-game verification: Not yet.

Play test: drop the last selected spear without changing slots or picking it up;
expect fist damage/recharge and an empty hand for a nearby observer. Capture both
views and the single ground item; repeat with partial/whole stacks and cancellation.
## Server-wide tab-list membership (#117)

Login initializes and announces online players across all dimensions. Dimension
changes retain tab entries; only real logout removes them globally. Gamemode,
listed status, latency, order and display updates reach every dimension, while
entity tracking remains world scoped and Java initialization precedes spawning.
Ordered owner notifications keep concurrent logout behind pending initialization.
Retains weak player handles in the existing admission session table, so a
world detach cannot withdraw an online identity; no separate online cache or
saved format changes. Adapts the roster routing from JulesB40's upstream PR #3432,
with metadata and lifecycle packet regressions added by Codex (GPT-6.1 Sol).
Native API 8 and Wasm WIT are unchanged. In-game verification: Not yet.

Play test: keep three clients in the Overworld and Nether, join and reconnect,
then move between dimensions; expect every tab to retain all online players and
remove a player only on disconnect. Capture tabs and player-info packets during
login, a portal trip, metadata changes and logout.

## Villager persistence, breeding slice 1

Villagers load vanilla 26.3 registry-name `VillagerData`, the finalized flag and
byte `FoodLevel`, while accepting the earlier fork's numeric fields and int food.
Saves use the vanilla form. Unknown registry names and out-of-range legacy food
remain preserved with a warning. Inventory retains eight usable slots and saves
nonempty stacks in order; unsupported and excess entries retain lossless NBT tag
content, not literal bytes: compound-key order is not kept.
Gossip UUIDs use the shared four-int UUID codec, malformed targets are skipped
with a warning, and the fork's saved `LastGossipShare` is restored. Offers, restock
state, home and Brain memories keep their existing persistence. Each save captures
state under separate locks and encodes after releasing the guards.

Checked against vanilla 26.3 `VillagerData.CODEC`,
`Villager.readAdditionalSaveData/addAdditionalSaveData`, `InventoryCarrier`,
`SimpleContainer` and `GossipContainer.GossipEntry.CODEC`. Zombie villagers use the
same registry-name codec, accept legacy numeric data, and retain unknown names.
Retained names are saved only while the live field equals its decoded fallback;
later type or profession changes discard that field's retained name, including
direct native/Wasm writes. Level-only changes preserve both retained names.
Curing applies villager data before restoring offers, matching
`ZombieVillager.finishConversion`, so remembered trades survive a profession
change; retained unknown registry names also transfer to the cured villager.
Existing zombie Offers, Gossips, Xp and conversion-field behavior is
unchanged. Pickup, breeding, goals and biological age are unchanged.
Written by GPT-6.1 Sol (Codex Cloud). In-game verification: Not yet.

The original eight bounded server scenarios exercise chunk activation and two real shutdown/
restart saves, including unknown registry names and both finalized-flag cases.
All eight fail on `95cb23667` (the short gossip target panics) and pass with this
change. Raw-entry comparisons use canonical NBT bytes because the existing NBT
parser discards compound-key order; tag types, values and list order are retained.
Six round-2 scenarios add setter/direct-write changes, independent retained
fields, zombie vanilla/legacy/unknown data and carried fields, and known/unknown
cures. Each exercises two shutdown/restart cycles, with isolated negative
controls for fallback validation, zombie serialization and cure restoration.

PR #226 review round 3 keeps inventory entries raw when decoding would drop
unsupported top-level or component data. It skips only canonical legacy
`minecraft:air` zero-count placeholders, retaining other zero-count entries
across two restarts. Zombie-villager conversion finalizes villager data for
known and unknown registry names while keeping restored offers. Chunk activation
initializes Java tracked data and Bedrock actor metadata from loaded villager
data and XP. No public plugin API or saved-data meaning changes. In-game
verification: **Not yet**. The new persistence assertions fail on `854cb8cfd`
for lossy inventory rewrites, the dropped zero-count entry and the known cure's
false finalized flag.

API 8 (same version): `VillagerEntity` and `ZombieVillagerEntity` gain persistence
state (including recorded fallback values), and the public villager
inventory now holds `VillagerInventory`, whose usable slots are a fixed array of
eight stacks. Native plugins must be rebuilt; Wasm WIT is unchanged.

Play test: Import villagers with professions, trades, food and player reputation,
restart the server twice, and expect the same villagers, trades and discounts.
Give a villager with an unknown saved profession a lectern; expect its librarian
profession to survive two restarts. Import a zombie librarian, cure it, and expect
the same remembered trades before and after restarting.

## Hanging support lifecycle and painting attacker (#44, #47)

Paintings and ordinary item frames check support, block collision and hanging
coexistence every 101 world ticks, matching vanilla 26.3
`BlockAttachedEntity.tick`, `HangingEntity.survives` and `ItemFrame.survives`.
The full painting area is checked; Fixed frames remain immune. Missing neighboring
terrain postpones validation until it is loaded. Break callbacks run before the
existing first-removal claim; only its winner drops the decoration and frame
contents. Painting damage uses the actual causing entity, so Creative attacks
suppress the painting item while Survival and support loss retain their drops.

API 8 (same version): painting and frame layouts gain a runtime check counter,
and frame items use shared snapshots so component callbacks run after unlocking.
Survival queries are additive. Native plugins must rebuild; existing source
signatures, saved tags and Wasm WIT stay compatible. The check counter is not
saved, as in vanilla. Implemented by Codex. In-game verification: Not yet.

PR #232 review: coexistence checks prefilter distant entities conservatively,
including decorations whose cached bounds were reset by teleport. Frame geometry
reads shared items without component cloning. Item metadata publication rejects
superseded revisions after unlocked component callbacks; no new nested lock
ownership is introduced. Runtime painting variant changes preserve the attachment
when width/height parity changes; saved `Pos` keeps its existing center meaning,
and live NBT edits still refresh bounds and metadata. API 8 (same version): frame
layout gains a runtime item revision; native plugins must rebuild. Saved tags and
Wasm WIT are unchanged. Implemented by Codex. In-game verification: Not yet.

Backlog: timed world borders still expose their target size to server collision
and containment. The pre-existing shared border model has no transition clock,
effective-size accessor or remaining-duration initialization. Correcting that
model, interrupted transitions and command readers is a separate border change.

Play test: place filled ordinary/glow frames and a large painting, remove their
support (including one painting corner), and expect each eligible item once after
about five seconds. Cancel break hooks, attack while support breaks, and unload
and reload; expect canceled/Fixed frames and their contents to remain, with broken
decorations staying gone. Change a painting between odd/even sizes; expect its
attachment to stay aligned. Survival painting attacks drop one item; Creative
attacks drop none. Capture item counts, reload results and alignment in a recording.

## Portal return destination and axis rotation (#73, #76)

Returning players resolve their saved bed or anchor in its own dimension without
spending an anchor charge. Missing, obstructed and absent personal spawns use an
adjusted shared spawn on loaded terrain, preserving saved spawn coordinates and
supported administrator-selected heights. Invalid saved spawns send the existing
missing-spawn notification without rewriting the record. Entering-End platforms,
non-player exits, credits and cancellable world-change events keep their paths.
Mirrors Java 26.3 `EndPortalBlock.getPortalDestination`,
`ServerPlayer.findRespawnPositionAndUseSpawnBlock(false)` and
`PlayerSpawnFinder.findSpawn/fixupSpawnHeight`.

PR #233 review: living End returns exhaust safe bed/anchor candidates before
allowing dangerous cells or floors, and require collision support instead of
legacy solidity. Wall signs over shafts cannot become respawn floors. Arrival
height follows that support and yaw faces the saved spawn block, mirroring
`AbstractBedBlock.findStandUpPosition`, `RespawnAnchorBlock.findStandUpPosition`,
`DismountHelper.findSafeDismountLocation` and `ServerPlayer.RespawnPosAngle`.
Consuming death respawn retains its existing selection and yaw. Tests cover
registered End contact and actual Java/Bedrock transfers, lava and burning
floors, sign shafts, and saved-spawn changes/world unload at the chunk-fetch
boundary. Living End returns also require a charged anchor in a dimension where
anchors work, or a bed whose dimension allows setting spawn, before searching
for a stand-up position; ineligible records use shared spawn without changing
the saved record or anchor charge. Implemented by Codex (GPT-6.1 Sol).
In-game verification: Not yet.

Play test: In Survival, return from the End with lava beside a charged Nether
anchor and a dry supported alternative; expect the dry arrival, no new damage
or fire, and the same charge count. Repeat with a bed whose first side has a
wall sign over a deep shaft and whose opposite side has a stone floor; expect
arrival on stone, facing the bed. Capture a recording of each arrival and the
anchor charge count before and after. Also set an Overworld bed spawn, break the
bed, place and charge an anchor at its saved head once, and arrange lava as the
only stand-up candidate under a low roof; return from the End and expect the
safe shared spawn, no new damage or fire, and the anchor still charged.

Either Nether portal axis change adds 90 degrees to Pumpkin's absolute yaw;
same-axis travel retains yaw. This narrowly adapts Zinedin Polic's correction in
upstream PR [#3819](https://github.com/Pumpkin-MC/Pumpkin/pull/3819), commit
`5cd3fe4d3d30ffbd7596b985b6eaf9a97c25add4`, preserving this fork's optional source
axis, pitch, velocity and residency contracts. Mirrors
`NetherPortalBlock.createDimensionTransition`.

API 8 (same version): the shared collision predicates gain crate-internal
visibility only; external public layouts/signatures, Wasm WIT and saved-data
meanings are unchanged. Rebuild native plugins against this checkout; existing
plugin sources remain compatible with the unchanged respawn method signature.
The wider asynchronous portal/session race (#180) and death-respawn work (#205/#202) remain separate. Implemented by Codex.
In-game verification: Not yet.

## Decorated-pot loot alternatives (#13)

Intact pots drop one decorated pot with its decorations; their stored item still
drops once. Tools tagged to break pots crack them before loot selection, which
drops the stored sherd decorations instead. This removes the extra unconditional
four bricks and preserves Silk Touch protection, Creative, spawn protection and
cancelled-break behavior. Mirrors vanilla 26.3 `DecoratedPotBlock.getDrops`,
`DecoratedPotBlock.playerWillDestroy`, `DecoratedPotRecipe.assemble` and
`blocks/decorated_pot.json`. Crafting retains one component-preserving template
per input face. Pre-upgrade block entities with no face data keep the historical
four-brick drops. Pots with all four faces empty, including bare saved items and
`/give` pots, break intact and return the pot plus its contents. This deliberate
compatibility policy has no provenance tracking and differs from vanilla, where
a shattering tool drops only present sherds. Empty decorations remain empty;
vanilla's named `sherds` compound and the fork's legacy list remain readable and
are preserved.
API 8 (same version): `DecoratedPotBlockEntity`, `PotDecorationsImpl` and
`LootEntryKind` public layouts change; `DecoratedPotBlockEntity.sherds` now
retains the raw NBT tag; rebuild native plugins. In-game verification: Not yet.

Play test: Break an empty decorated pot with a diamond pickaxe, with and without
a diamond stored inside; expect one pot item back and the stored diamond when
present. Break a pot with sherds the same way; expect the sherds instead.


## Cauldron filled-bucket transactions (#29, #30, #31)

Filled water, lava and powder-snow buckets apply to all four cauldron kinds;
water buckets top up levels 1/2 to level 3. The existing BucketEmpty event can
veto every fill before effects. Water above, including flowing or waterlogged
water, consumes lava/snow use without filling or exchanging items. Source-slot
identity, player life and target checks preserve callback replacements. Input
and result commit under one inventory owner; payment and statistics precede
world callbacks. Mirrors Java 26.3
`CauldronInteractions.addDefaultInteractions`, `emptyBucket`, the three fill
interactions and `isUnderWater`. Bottle/washing behavior is unchanged.

Written by Codex, with focused fixture patterns adapted from Optifineless420k’s
[PR #116](https://github.com/Optifineless/Pumpkin/pull/116); supersedes only that
PR’s #29–#31 portion. API 8 (same version): exported layouts, shared item-exchange
and block-hand-result signatures, WIT and saved formats are unchanged.
In-game verification: Not yet.

Conditional terrain admission precedes item/stat publication, preserving the
fork's existing filled-terrain observation in those callbacks. Physics callbacks
wait for payment and statistics. This differs from serial vanilla's intermediate
terrain visibility during item/stat callbacks; no event framework is redesigned.

## Consumable fallback after refused crop placement (#158)

Java carrot and potato use starts eating in the requested hand when crop
placement refuses without consuming the action. Planting, consuming block
interactions, explicit failures, protection and cancelled events retain
precedence. Hunger, cooldown and existing active-use admission are shared with
air use; callback replacements and selection changes are revalidated before
starting. Mirrors vanilla 26.3 `BlockItem.useOn`, `Item.useOn/use` and
`ServerPlayerGameMode.useItemOn`. API 8 (same version): the richer placement
outcome is internal; public signatures, layouts, saved data and Wasm WIT are
unchanged. Written by Codex (GPT-6.1 Sol). In-game verification: Not yet.

## Ground mob collision step-up (#87)

Grounded mobs retry horizontally blocked movement at actual collision-shape
heights within their effective STEP_HEIGHT, choosing the first candidate that
improves horizontal progress. This lets ordinary mobs leave dirt paths and lets
Endermen use their one-block allowance. Mirrors vanilla 26.3 `Entity.collide`,
`collectCandidateStepUpHeights` and `collideWithShapes`. Final-footprint support
and incident vertical velocity reach the existing fall/landing hooks; ceilings,
airborne collisions and bounded collision sweeps remain respected. Player
position authority, plugin events, native API 8, WIT and save formats are unchanged.
Written by Codex Cloud. In-game verification: Not yet.

Play test: lure a cow off a dirt path onto stone; expect smooth ascent without
jumping. Repeat over slabs/stairs and with an Enderman at a one-block ledge;
expect ascent within each mob's step allowance. Lower STEP_HEIGHT below the lip;
expect no automatic step. Add a low ceiling; expect obstruction. Record both.


## Campfire storage and start admission (#85, #86)

Lit and unlit campfires accept valid food into a free slot. Start-event admission
runs before payment; cancellation and callback replacements preserve input and
storage. Accepted durations commit with the existing atomic held-item transfer.
Cold food waits for lighting and retains vanilla cooldown. Mirrors vanilla 26.3
`CampfireBlock.useItemOn` and `CampfireBlockEntity.placeFood`.

The optional `CookingStarted` byte array records admitted slots, including literal
zero/negative plugin durations, so reload does not repeat their start event.
Missing entries default to false: old positive totals/progress remain unchanged,
and old nonpositive totals retain the cancellable fallback. Existing item and
timer tags keep their meaning. API 8 (same version): `CampfireBlockEntity` gains
private admission state; rebuild native plugins. Public signatures and Wasm WIT
are unchanged. Implemented by Codex (GPT-6.1 Sol).
In-game verification: Not yet.

## Admitted movement and landing bookkeeping (#112, #77, #74)

Accepted Java movement counts the last descending segment before landing, including
supported zero-displacement ground-only and rotation-only packets. All landings
retain the actual support block's callback; neighboring noncontact slime no longer
cancels a stone landing. Position and position+rotation use the same grounded-to-airborne,
upward takeoff predicate for jump statistics and exhaustion.

PR #231 review: admitted Java ground updates refresh the existing runtime support
cache from collision shapes beneath the whole player footprint, including all
zero-displacement variants. Edge contact with slime stays safe even at one health;
noncontact slime still permits ordinary stone damage, and noncolliding shapes are
excluded. Airborne updates clear cached support. This ports the selection from
`Entity.checkSupportingBlock` and `CollisionGetter.findSupportingBlock`, including
the nearest-center and greatest Y/Z/X tie rule. When the final footprint has no
support, the probe retries at the prior horizontal position while keeping final Y,
so moving off a slime edge during landing retains its callback. Unsupported
zero-displacement ground claims defer fall consumption. Takeoff commits airborne
state before jump/exhaustion callbacks, so reentrant movement counts it once.
No public signature, layout or saved-data change.

Round 3 backlog: B3's second complete landing during a fall callback needs a
reservation handoff; the pre-existing lifecycle gap before damage admission
(owner comment 4315144035) remains out of scope.

Mirrors vanilla 26.3 `Entity.checkFallDamage`,
`ServerGamePacketListenerImpl.handlePlayerPositionChange` and
`ServerPlayer.jumpFromGround`. Landing callbacks retain their distance until they
finish; private combat-owner reservations prevent duplicate consumption and keep
new descent, replacement lives and replacement sessions safe across callbacks.
Mace landing particles retain the ServerPlayer override's pre-accumulation distance.
Water, climbable, honey and scaffolding paths, damage admission, plugin cancellation
and the existing floor/epsilon formula remain in use. `FallDistance`, statistics
and hunger saves retain their existing tags, defaults and meaning.

API 8 (same version): public signatures, vtables and embedded layouts are unchanged;
new runtime reservation state lives behind the existing private damage-owner Arc.
Native plugin source remains compatible; rebuild native plugins against this
checkout. Wasm WIT is unchanged. Implemented by Codex (GPT-6.1 Sol).
In-game verification: Not yet.

Play test: drop exactly four blocks onto stone with and without turning; expect
half a heart lost once. Repeat beside noncontact slime, then onto actual slime;
expect normal stone damage and no slime damage. Repeat with only the edge of the
player's feet over slime and the center over air, including at half a heart;
expect unchanged health and survival. Also move past that edge during the final
descent at full and half-a-heart health; expect no damage or death. Compare
walking/sprinting jumps with and without turning; expect identical jump statistics
and hunger costs. Record the measured drop, landing blocks, health and jump-statistic
changes.

## Sponge fluid traversal and depth (#80, #81)

Sponge absorption follows vanilla 26.3 `SpongeBlock.removeWaterBreadthFirstSearch` and `BlockPos.breadthFirstTraversal`: connected water uses graph depth six and at most 64 absorbed blocks. Waterlogged hosts keep their block while losing water; kelp and seagrass use their normal loot handling and are removed. The cancellable sponge event remains before mutation, with current states rechecked after callbacks. Aquatic plants are conditionally removed before their drops are published, so synchronous item-spawn callbacks cannot duplicate the observed plant. Cancelled drops still leave the plant removed, and callback replacements survive. No saved-data or public API change.

In-game verification: Not yet.

Play test: Place a dry sponge beside a waterlogged slab, kelp, seagrass and a seven-block water line. Expect the slab to stay dry, kelp to drop normally, seagrass to be removed, the first six water blocks to disappear, and the seventh to remain.

## Sponge wet conversion (#325)

When a dry sponge absorbs water, its wet-state change notifies clients and runs normal shape updates without sending an extra ordinary neighbor callback from the sponge. Deferred callbacks from drained water remain. Mirrors vanilla 26.3 `SpongeBlock.tryAbsorbWater`; no API or saved-data change.

In-game verification: Not yet.

Play test: Place a dry sponge beside a short connected water line with redstone
or another neighbor-sensitive block beside it; expect the water to drain, the
sponge to turn wet, and the sound to play once. Place water beside a dry sponge;
expect it to absorb. Put water beside a wet sponge; expect the water to remain.
## Server scoreboard persistence

Every dimension shares the server's scoreboard. Objective, score, display-slot and
team updates reach players across dimensions; joining players receive the shared
state once, and dimension changes retain it. Autosave, `/save-all` and shutdown
snapshot dirty state and atomically save the Overworld's
`data/minecraft/scoreboard.dat`. A failed load disables scoreboard saves for that
run, preserving the original file. Older versions load matching fields with one
warning and no data fixer; newer unsupported versions are preserved unchanged.

The saved codec follows vanilla 26.3 `ScoreboardSaveData.Packed`, `Objective.Packed`,
`Scoreboard.PackedScore`, `Score.Packed`, `PlayerTeam.Packed` and `DisplaySlot.CODEC`.
It retains unknown criteria, full text/style data, number formats, score locks,
all team options and memberships, and all 19 display slots. Display and format
commands expose the persisted settings. The world-data version is corrected from
26.2's 4903 to 26.3's 5023, checked against the official server jar's `version.json`.

Review round 3 separates storage from display. Every saved component keeps its
exact raw NBT in the existing persistence sidecar, including unusual or malformed
text and full events. Runtime displays and Java scoreboard packet serializers
project only literal/translated text, fallback, recursive arguments/extras, named
or ASCII RGB colors, five boolean decorations and valid resource-location fonts.
Events, insertion, selectors, score/NBT/keybind/object contents, shadow colors and
unknown fields stay on disk and never enter imported displays or command feedback.
Unsupported empty objective/team names display their storage name. Styled and fixed
number formats use the same subset. Session-created subset text retains its display.
UTF-8 hex colors return parse errors instead of panicking in either color parser.
Malformed NBT or wrong record/component outer types still fail the whole load and
disable saves. This is a deliberate display restriction, without changing saved
component meaning; it replaces round 2's incomplete client-codec validation gate.

Review round 4 discards a field's saved raw data when runtime update methods
change it or a command explicitly replaces it, including A to B to A before or
between saves. Other components retain their original NBT. Display projections stop
at 128 emitted container levels and use a conservative 32 KiB budget that includes modified
UTF-8 strings and component overhead. Raw storage remains complete. Login sends
teams followed by deduplicated displayed objectives and their scores; hidden
updates stay server-side until a display slot starts tracking them. Tracking
mirrors `PlayerList.updateEntireScoreboard` and `ServerScoreboard`'s tracking
methods. Native API 8 (same version), its public surface and on-disk codecs are
unchanged by this round. In-game verification: Not yet.

Accepted API 8 exception: a same-value component assignment through a whole-settings
plugin update may retain that field's original raw NBT because the API carries no
field-replacement intent. The affected setters are:

- Wasm `scoreboard::Scoreboard::update_team` for `TeamSettings.display_name`,
  `prefix` and `suffix`, including SDK `Team::update_settings`,
  `Team::set_display_name`, `Team::set_prefix` and `Team::set_suffix`;
  Wasm `scoreboard::Scoreboard::update_objective` for `display_name`.
- Native `Scoreboard::update_team` for `display_name`, `player_prefix` and
  `player_suffix`; `Scoreboard::update_objective` for `display_name` and
  `number_format`; `Scoreboard::update_objective_with_number_format` for `display_name`;
  `Scoreboard::update_score` for `display_name` and `number_format`, and
  `Scoreboard::update_score_with_number_format` for `display_name`.

Native `Scoreboard::get_objective_mut` and `Scoreboard::get_team_mut` compare
component fields with their imported baseline before each borrow. Only fields
already changed lose their raw sidecars; unrelated and no-op borrows preserve
them. Save packing also detects the final outstanding changes. Separate-borrow
A to B to A replacements stay replaced. A same-value replacement, or A to B to A
entirely within one borrow, carries no observable replacement intent and may keep
the original raw field. Commands and explicit number-format replacement setters
invalidate only the replaced field, including same-value replacements.

Review round 5 also constructs Bedrock titles only during Bedrock packet
serialization. Rendered text shares one 32 KiB output budget across the complete
component tree without materializing argument strings; bounded traversal also stops repeated
empty arguments. The scoreboard file reader counts each compound and list once,
including the root, with vanilla NbtAccounter's 512-container limit. Other NBT
readers and their limits are unchanged. Full raw text remains on disk.
NBT serialization keeps scalar encoding off recursive container stack frames so
newly accepted 512-container documents can also be saved on normal worker stacks.
Native API 8 (same version), public methods and layouts are unchanged in this round.
In-game verification: Not yet.

Review round 6 uses one private output/traversal budget for plain text, styled
console feedback and logs, both translation-string helpers, Java legacy text,
Bedrock raw/legacy text and titles, and custom translation materialization
(including hover text). Substitutions are visited by reference; formatting and
hyperlink bytes count toward the 32 KiB limit, and empty expansions stop at the
shared traversal limit. Translation table copies are bounded before allocation.
Raw persistence codecs remain lossless. API 8 (same version): no public methods,
types or layouts change in this round.
Native components needing no server-only custom translation pass unchanged
through serialization, including entity names and text-display saves. The input
scan is iterative and does not expand or copy component payloads.

Review round 7 preserves native parents, siblings, extras and styles in storage
even when they contain Custom descendants. Only generated Custom expansions
consume the materialization budget; entity CustomName, text-display JSON and the
public NBT storage encoders retain unrelated native payloads. Display and
console rendering retain their shared limits. Materialized click/hover events
and font identifiers are complete or omitted, never shortened; this keeps valid
UUIDs, registry IDs, URLs and commands valid at budget boundaries. Mirrors
`ComponentSerialization.createCodec`, `HoverEvent.EntityTooltipInfo.CODEC` and
`ClickEvent` codecs in vanilla 26.3. Native mutable scoreboard borrows now retain
untouched raw text as described above. API 8 (same version): no public methods,
types or layouts change. In-game verification: Not yet.

The native renderer retains the fork's existing missing-argument clamping;
vanilla 26.3 `TranslatableContents.decompose/getArgument` instead displays an
invalid template literally. This round bounds that existing behavior rather
than changing ordinary translations. A saved native S20 objective is covered
through startup, Java join, the actual Console `scoreboard players get` command,
mutation and save; safe S5 controls reproduce the old unbounded output.
In-game verification: Not yet. Play test: query an imported translated objective
from the console, then join and save; expect prompt, shortened feedback, a normal
join and unchanged saved display data. Capture the console query and sidebar.

Not addressed (backlog): live `displayautoupdate` holder-name resolution; numeric
translation arguments in display projection; Bedrock team-coloured slots,
objective lifecycle slot assignments. Allocation-derived shared-scoreboard
score-entry IDs are addressed by #289 above.
The Wasm whole-settings API also cannot distinguish a round-tripped default
white from an intentional white update on an uncoloured team. Its existing
preservation rule remains: to explicitly set white, update to a nonwhite colour
first, then white (or create the team with white). A one-call intent distinction
needs a separate design. These parity gaps do not change the retained raw save.

If the namespaced file is absent, supported pre-26.1 worlds import
`data/scoreboard.dat`; successful later saves publish the namespaced file without
removing the legacy original. A corrupt namespaced file never falls back to legacy.
Death announcements now obey the victim's saved server-team visibility across all
dimensions, preserving the victim's death-screen packet. Routing mirrors 26.3
`ServerPlayer.die` and `PlayerList.broadcastSystemToTeam/AllExceptTeam` (including
excluding the victim from the restricted own-team announcement). Projection is a
fork display policy; storage/migration follow `ScoreboardSaveData.Packed` and
`DimensionStorageFileFix`. Checked against the official 26.3 inner server JAR
SHA-256 `a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`,
decompiled with Vineflower 1.11.2.

Backlog: the new `displayautoupdate` flag remains persisted but does not refresh
live holder names. Complete parity with `Scoreboard`'s `ScoreAccess.set` needs
pre-lock live-holder snapshots across numeric commands, triggers, criteria,
execute-store callbacks and plugin-facing writes. The existing entity/player
name renderers also omit team decoration. A partial command-only update would
leave those paths inconsistent; this nonblocking work is outside this review fix.

API 8 (same version): `Server`, `World`, `Scoreboard`, `ScoreboardObjective` and
`Team` layouts change, including the shared `World.scoreboard` handle; native
plugins must be rebuilt. Wasm API versions and existing player display overlays
are retained. Automatic criterion updates and `/trigger` semantics are unchanged.
Written by Codex. In-game verification: Not yet.

Play test: show an objective in the sidebar, put players in the Overworld and
Nether on the same team, and change a score and team prefix; expect both players
to see the changes. Move between dimensions; expect one team and the same sidebar.
Stop and restart; expect scores, formatting, team settings and membership to
survive. Capture the cross-dimension updates and the restored sidebar after restart.
With unusual saved text, start the server and join; expect safe text (or the
objective/team name), no disconnect, and the original component data retained
through `/save-all` and shutdown. For a structurally corrupt scoreboard, expect
an error, a successful join with an empty imported scoreboard and saves disabled.
Import a pre-26.1 world and expect its old scoreboard before making any changes.
Set `deathMessageVisibility` to `never`, `hideForOwnTeam` and `hideForOtherTeams`;
kill a member and check recipients in another dimension. Capture the safe sidebar,
restored legacy scoreboard and each announcement audience. In-game verification:
Not yet. Review fixes by Codex; native API remains 8 with no additional layout
changes in this round.

Round 4 play test: import a prefix A with an old click action, run `/team modify
red prefix B`, optionally `/save-all`, then `/team modify red prefix A`; save and
restart, and expect plain A with no old click action. Join a world containing a
large hidden objective and expect a normal login with no hidden sidebar. Display
that objective and expect safely shortened text; clear its last display slot and
expect it to disappear. Capture the successful join and sidebar transitions.



## Trigger score formatting (#307)

Simple, add and set `/trigger` now mutate the existing unlocked score's value and
lock it, retaining its display component, number format and saved opaque fields.
Additions use wrapping `i32` arithmetic, matching Java integer semantics. Mirrors
vanilla 26.3 `TriggerCommand.simpleTrigger/addValue/setValue`; no native API, WIT,
packet format or saved-data schema changes. In-game verification: Not yet.

Play test: give a trigger score a custom display and fixed or styled number format,
enable it, and run the simple, add and set forms; expect the value to change, the
score to lock again, and the sidebar formatting to remain visible. Capture the
score update from a nearby observer.

## Cat, wolf and cow hand interactions (#41, #42, #48)

Healthy tame cats reach Animal feeding before the owner's sitting fallback.
Wolf food heals twice its effective nutrition, with the vanilla missing-FOOD
fallback; bones remain wild-wolf taming items. Adult cows exchange the actual
used hand's bucket for milk, including stacked and Creative inputs; babies refuse.
Mob uses conditionally commit their originating inventory slot before callbacks,
retain component-bearing food remainders, and preserve callback inventory edits.
Existing interaction and regain-health events retain their order and cancellation.
The species age clock, breeding inheritance and saved-data meanings are unchanged.

PR #249 review restores handless `Mob::mob_interact`, `EntityBase::interact` and
default `EntityBase::interact_at` calls on caller-supplied stacks, including
detached stacks, through the shared species handlers. Direct
`Animal::animal_interact` retains caller-stack feeding. Alive and shearing checks
remain ahead of species dispatch; packet hand transactions retain their existing
validation. API 8 (same version), with no new public API or layout changes.
In-game verification: Not yet.

Mirrors vanilla 26.3 `Cat.mobInteract`, `Wolf.mobInteract`, `TamableAnimal.feed`,
`Animal.mobInteract`, `AbstractCow.mobInteract` and `ItemUtils.createFilledResult`.
Selectively reimplements the relevant #116/#176 work with current inventory owners;
the cow result follows BarilTomasDev's upstream #3940 idea (`81441b4eee3b66ce`).
API 8 (same version): additive provided `Mob::mob_interact_with_hand` and
`Animal::animal_interact_with_hand` methods change native trait vtables. Existing
implementations retain their defaults; rebuild native plugins against this tree.
Generic exchange signatures and Wasm WIT are unchanged. Written by Codex.
In-game verification: Not yet.


## Brewing remainders and Java 26.3 menu (#16, #119)

Successful brewing returns the reagent's crafting remainder to its emptied slot,
or drops it after inventory ownership is released when reagents remain. Fuel uses
the same remainder rule. The real brewing menu publishes remaining brew time,
remaining fuel, total brew time and total fuel in vanilla order. Fuel component
providers supply uses and speed; brewing remains a countdown, including after
loading old fork short/byte saves. New saves use vanilla int counters and optional
totals/speed default to 400, 20 and 1.0 on old saves. Existing fuel, start and brew
events retain cancellation, with stock revalidated after callbacks.

Mirrors vanilla 26.3 `BrewingStandBlockEntity.serverTick/doBrew/getUses/getSpeedMultiplier/loadAdditional/saveAdditional`,
`BrewingStandMenu`, and `BrewingFuel`/`ResolvableInt`/`ResolvableFloat` codecs.
The context-provider evaluator handles the binomial rule used by blaze powder's
default 20-use provider. A fuel `uses` provider that errors or resolves to zero
or less leaves the fuel item and fuel totals unchanged, preventing repeated
payment on later ticks. Numeric-looking holder references and other unsupported
provider forms can still leave the stand idle. This addresses the payment-loss
part of PR #246 review; broader context-provider parity and the reviewed event
payload widths remain outside this fix.

API 8 (same version): rebuild native plugins for the brewing stand layout and
`BrewingFuelImpl` marker-to-payload change (replace unit construction with
`Default` or explicit providers; `read_data` is no longer const). `Eq`/`Hash`
remain available and speed constants compare and hash their float bits. The
crafting-remainder helper is now public and the brewing menu's `DATA_COUNT` is four. Wasm WIT is
unchanged. Written by Codex (GPT-6.1 Sol). In-game verification: Not yet.
## Bed removal, sleeping lifecycle and unsafe dimensions (#114, #35, #153)

Breaking either ordinary or straw bed half yields one bed in Survival and none in Creative.
Normal player ticking wakes a sleeper whose remembered bed disappeared, without
restoring the bed or overwriting a callback's replacement. Ordinary beds consume
the dimension's `destroy_on_use` rule before sleep admission and use the existing
bad-respawn explosion; straw beds retain their own rule and destruction behavior.
Mirrors vanilla 26.3 `AbstractBedBlock.useWithoutItem/playerWillDestroy`,
`BedBlock.destroyOnUse`, and `LivingEntity.tick/checkBedExists/stopSleeping`.
This supersedes only these issues' portion of old fork PR #102, whose shared
destruction hook was already integrated; villager and other admission work stays
separate. Reporter credits: visionglitcher and Uelq (#114), kalancelot / devilos
(#153). Implemented by Codex (GPT-6.1 Sol).

API 8 (same version): `Player` gains private runtime sleep-operation ownership;
native plugins must rebuild against this checkout. Existing plugin source call
sites remain compatible; public signatures, Wasm WIT, saved fields and existing
save meanings are unchanged.
Malformed use pairs with mismatched facing are rejected to avoid removing an
unrelated half; this is stricter than the supplied vanilla opposite-part check.
Removal uses the existing per-position chunk and block-entity publication owners;
arbitrary raw native writes and global two-position atomicity remain legacy limits.
PR #230 review: bed loot listeners again observe the bed before removal, preserving
conditional `BlockDropItemEvent` cancellation. A nonblocking operation snapshot
rejects recursive and concurrent duplicate breaks; changed cells retain callback
replacements and discard uncommitted loot. Callbacks and item spawning hold no
publication guard. API 8 (same version): `World` gains private runtime bed-break
admission state under its existing publication gate; rebuild native plugins.
Saved fields and Wasm WIT are unchanged.
PR #230 round 3: player breaks admit both halves with the original actor and
spawn protection, retaining partner cancellation without an environmental retry.
Wake ownership uses operation/lifecycle identity rather than the ticking sleep
timer, and cancelled Standing transitions retain the remembered sleep state.
Unsafe use explodes after any committed half; unrelated neighbour shape updates
still run when a captured partner changed. Checked against vanilla 26.3
`BedBlock.destroyOnUse`, `LivingEntity.hurtServer/stopSleeping` and `Level.setBlock`.
B4 remains BACKLOG by coordinator decision: no existing per-cell mutation identity
distinguishes an equal-state replacement during loot callbacks; new revision
infrastructure is out of scope. This limitation was introduced in round 2.
The older raw-removal spawn-boundary gap (L1) remains outside this change;
player-driven partner admission now checks protection explicitly.
PR #230 round 4: bed-specific `SKIP_DROPS` no longer reaches unrelated neighbour
shape updates, so attached blocks that lose support still drop their items.
The bed pair's own teardown continues to suppress its duplicate drop. Mirrors
vanilla 26.3 `Level.setBlock`; a Java regression pays for a spore blossom under
an elevated straw-bed head, then breaks the head in Survival and checks one of
each item. In-game verification: Not yet.
Unsafe-dimension bed use also reserves both halves across its break events, so a
recursive or concurrent use cannot dispatch the same pair's events repeatedly.
The regression re-enters Java bed use from a break callback and checks for one
pair admission. In-game verification: Not yet.
In-game verification: Not yet.

Play test: place ordinary and straw beds, break either half in Survival and expect
one bed item; repeat in Creative and expect none. With a plugin that cancels bed
loot by checking the event's world and position, break the head and expect no item.
Record the breaks and resulting drops on the candidate build.
Repeat with Java and Bedrock and a protection plugin; expect both break events to
name the breaker and a cancelled/protected partner to remain. Leave an intact bed
while another player hits the sleeper; expect waking to finish. Use a bed in the
Nether or End and expect the explosion, including when only one half is consumed.
Place a straw bed on temporary supports, remove them, crouch-place a spore blossom
under the head, then break the head in Survival; expect one bed and one blossom.

## Elytra durability and flight validity (#72)

Admitted gliding now checks usable equipped gliders on normal living ticks,
emits `elytra_glide` every 10 flight ticks and applies one durability wear every
20 through the existing equipment damage event and slot transaction. Landing,
removing or invalidating the glider ends flight through the cancellable glide
toggle event and synchronizes the owner and observers. Callback replacements
and life/session changes reject stale wear. Flight transitions reset the runtime
cadence; existing `FallFlying` saves and client-authoritative motion are unchanged.
Mirrors vanilla 26.3 `LivingEntity.updateFallFlying/canGlide/canGlideUsing`.
`StartFlyingElytra` also follows `Player.tryToStartFallFlying`: it starts only
when the player is airborne, not already gliding or in liquid, and has a usable
equipped glider without levitation.
The recurring update retains `Entity.checkFallDistanceAccumulation`, using
accepted client movement for players and existing velocity for other entities.
PR #253 rebase retains #231's supported-landing admission and the single bounded
player-tick equipment publisher. Wear still commits every 20 flight ticks;
observers receive that queued change on the following normal detection, matching
`LivingEntity.tick` before `aiStep/updateFallFlying`. The glide-start regression
uses a real stone landing while retaining its health and fall-distance assertions.
The current-base rebase uses #202's exact active-world instance check for flight
continuations. #195's UUID lookup can still find a detached player through tracking;
that feedback lookup does not admit another wear or glide event. The regression
covers both tracking-only detachment and #202's `mark_departing` removal during
the damage callback. Existing removal/life checks and storage retirement remain.
The reported horizontal freeze remains unverified. Written by Codex.
In-game verification: Not yet.

API 8 (same version): `Entity` gains private runtime flight cadence state, changing
its layout and containing entity layouts. Rebuild native plugins against this
checkout; existing public signatures remain source compatible, and WIT is unchanged.

Play test: glide in Survival with an unenchanted elytra; expect one durability
lost per second of continuous flight. Land and restart; expect a fresh second
before the next wear. Pick up or shift-click the chest elytra while airborne;
expect both your view and a second player's view to leave the glide pose. Record
the durability display and both views around removal; report horizontal motion
separately. Creative flight and standing with an elytra should cost no durability.

## Empty map centers (#45)

New empty maps center on the scale-aligned tile that contains the player's block
position, matching vanilla 26.3 `EmptyMapItem.use`, `MapItem.create` and
`MapItemSavedData.createFresh`. Existing map centers and saved map data keep
their current meaning. Written by Codex (GPT-6 Luna). In-game verification:
**Not yet**.

Play test: Use empty maps at X/Z positions 10 and 30; expect both maps to show
the same 128 by 128 area. Use one just beyond -64; expect it to show the tile
centered at -128.

## Cross-dimension map sampling (#46)

Held maps only sample terrain when the player's world matches the map dimension.
Map decorations and periodic map packets continue to update while the map is
carried elsewhere, matching vanilla 26.3 `MapItem.update` and
`MapItemSavedData.tickCarriedBy`. Written by Codex (GPT-6 Luna). In-game
verification: **Not yet**.

Play test: Hold an Overworld map in either hand and enter the Nether; expect its
pixels to stay unchanged and existing markers to remain visible. Return to the
Overworld and explore new ground; expect the map to fill in normally.

## Note block instrument classification (#17)

Pling and copper support instruments remain selected when a note block is
placed or its vertical support changes. Diamond blocks retain vanilla's Harp
instrument; Glowstone is assigned Pling. Mob-head instruments retain their
above-block override and Harp fallback. Mirrors vanilla 26.3
`NoteBlockInstrument.Type.BASE_BLOCK`, `NoteBlock.setInstrument` and
`NoteBlock.triggerEvent`. In-game verification: **Not yet**.

## Jukebox playback restoration (#18)

An unfinished jukebox song restores its saved elapsed ticks and duration from
the record component; ended, stopped, empty and invalid-song records stay stopped.
Vanilla's 20-tick finish padding is preserved when loading and re-saving playback.
Comparator output still follows the inserted record, while weak redstone power
follows playback. Mirrors vanilla 26.3 `JukeboxBlockEntity.loadAdditional`,
`JukeboxSong.hasFinished` and `JukeboxSongPlayer.setSongWithoutPlaying`. Adapted
from upstream PR #3549 by HPnetwork (`d425daf19c4`); loading does not replay
insertion or start effects.
In-game verification: Not yet.

Play test: Play a cat disc, save and reload after 20 ticks; expect playback power
to stay on and turn off when the song ends, while the comparator still reads the
inserted disc.

## Fence and wall connections (#52, #53)

Fence and wall solid-face connections honor `Block.isExceptionForConnection`, so
melons, leaves, barriers, pumpkins and shulker boxes do not create generic links.
Same-family connections and aligned fence gates remain valid. Wall arm heights
and the center post use the block above's downward collision face; the post also
honors `minecraft:wall_post_override` and vanilla's corner and tall-wall rules.
Follows vanilla 26.3 methods: `FenceBlock.connectsTo`, `WallBlock.connectsTo`,
`WallBlock.updateSides` and `WallBlock.shouldRaisePost`. In-game verification:
**Not yet**. Written by Codex (GPT-6 Luna).
## Enchanting table fixes (#64, #65, #66)

Ordinary books now receive enchanting offers, are converted to enchanted books with stored enchantments, and keep at most one randomly selected result from a multi-enchantment book roll. The middle offer uses vanilla's level requirement; non-Creative payment remains two levels and two lapis. Bookshelf providers are checked against the transmitter at the matching height, so an obstruction at one height does not hide a shelf at the other. Mirrors vanilla 26.3 `EnchantmentHelper.getAvailableEnchantmentResults`, `EnchantmentMenu.getEnchantmentList/clickMenuButton`, and `EnchantingTableBlock.BOOKSHELF_OFFSETS/isValidBookShelf`. In-game verification: **Not yet**.
Leash fence knots (follow-up to #192, fixes #37) use vanilla's centered leash
search area, reject attachments outside each mob's snap distance, keep knot
creation and empty cleanup safe during concurrent interactions, and clean up
released leash links. Mirrors vanilla 26.3 `LeashFenceKnotEntity.interact`,
`LeashFenceKnotEntity.getOrCreateKnot`, `LeadItem.bindPlayerMobs` and
`Leashable` leash methods. In-game verification: **Not yet**.
Leash fence knots (follow-up to #192, fixes #37; PR #199 review) use vanilla's
centered leash search area, reject attachments outside each mob's snap distance,
and settle snaps, conversion handoffs and knot removal exactly once. Removed knots
reject attachments, and a live leashee settles a link to a removed holder on its
next tick, including after player logout. Invalid-fence cleanup leaves contended or
cancelled links for that settle; player holders are revalidated after leash events.
Snap/elastic distances follow each mob's vanilla values; the public `Entity::leash_to`
signature remains unchanged. Mirrors vanilla 26.3
`LeashFenceKnotEntity.interact`, `LeashFenceKnotEntity.getOrCreateKnot`,
`LeadItem.bindPlayerMobs` and `Leashable` leash methods. In-game verification:
**Not yet**.
## Binding Curse armor removal (#198)

Java inventory clicks and Bedrock stack requests keep armor with the generated
`prevent_armor_change` enchantment effect equipped outside Creative, mirroring
vanilla 26.3 `ArmorSlot.mayPickup` and `EnchantmentHelper.has`. Bedrock checks
removal under the same inventory owners as the write and corrects rejected client
predictions after release. Packet regressions cover cursed armor, ordinary armor
and Creative removal. Implemented by Codex (GPT-6.1 Sol).
Owner-review follow-up (#203) checks every request's take rules before its first
action, including armor equipped earlier in the same request. Rejections after
shape validation correct the active container as well as player stock; malformed
container, preview and alias requests receive only their error response.
API 8 (same version): `Slot::can_take_stack` adds staged pickup validation and
`ResultSlot::simulate_crafts` exposes the shared detached-grid payment simulation;
native plugins must be rebuilt. No saved-data meaning changes.

General request rollback on a concurrent writer's late conditional-commit failure
remains a pre-existing backlog: murgicraft's `inventory_commit::commit` already
rejects changed UIDs/content after earlier actions can commit. This follow-up
retains live revalidation and does not hold owners across crafting/trade callbacks.
Vanilla 26.3 decompiled sources were unavailable for this follow-up; the armor
rule follows the existing `ArmorSlot.mayPickup` / `EnchantmentHelper.has` port,
and Bedrock corrections follow the fork's existing container addressing.
In-game verification: **Not yet**.

Issue #207: denied legacy Bedrock armor drops now return an error and force the
unchanged authoritative armor, inventory, crafting and cursor contents. Rejected
multi-action requests correct the active 2x2/3x3 crafting input/output or generic
container, preserving any earlier valid commits. Correction snapshots use ordered
inventory owners and revalidate the live player/menu after release; each packet
reads current stock, including an unchanged cursor. Java QuickMove rechecks
`can_take_stack` on the lock-pinned source before withdrawal. Mirrors the supplied
vanilla 26.3 `ArmorSlot.mayPickup`, `Slot.tryRemove/safeTake` and
`AbstractContainerMenu.doClick/sendAllDataToRemote`; Bedrock addresses reuse the
existing packet encoders and mappings. Implemented by GPT-6.1 Sol (Codex Cloud).

API 8 (same version): the player-checked transfer entry is additive within the
inventory crate; the existing merchant transfer signature, public trait vtables,
layouts, Wasm WIT and saved-data meanings are unchanged. Existing native source
remains compatible; no new rebuild requirement. In-game verification: Not yet.

Play test: in Survival, try legacy drop and shift-click on Binding Curse armor;
expect it to remain equipped with the cursor and crafting grid showing server
contents. Repeat with ordinary armor and with cursed armor in Creative; expect
removal to work. Craft, then attempt a denied armor take in one Bedrock request;
expect any paid craft to remain, all ingredient/output counts to correct and no
extra items. Repeat at a crafting table and chest. Record the inventory, cursor,
armor and ground items before and after rejection, with a nearby observer.

Ender Dragon contact ignores Creative and Spectator players and sends Survival damage through the victim with the dragon recorded as attacker ([#161](https://github.com/Optifineless/Pumpkin/issues/161)). In-game verification: **Not yet**.

## Door and trapdoor placement (#22, #23)

Door hinge scoring checks full-block obstructions beside the door at both lower and upper levels, while preserving adjacent-door precedence and the click-position tie-break. Mirrors vanilla 26.3 `DoorBlock.getHinge`. Fixes [#22](https://github.com/Optifineless/Pumpkin/issues/22). In-game verification: **Not yet**.

Door hinge ties reconstruct the absolute hit from the clicked block and relative
packet hit, then use vanilla's inclusive midpoint comparisons. Lanterns follow
the vanilla nearest-looking placement context, select the first supported floor
or ceiling side, waterlog in source or flowing water, and return air immediately
when their selected support is removed. Mirrors vanilla 26.3
`DoorBlock.getHinge` and `LanternBlock.getStateForPlacement/canSurvive/updateShape`.
Fixes [#267](https://github.com/Optifineless/Pumpkin/issues/267),
[#268](https://github.com/Optifineless/Pumpkin/issues/268), and
[#269](https://github.com/Optifineless/Pumpkin/issues/269). In-game verification: Not yet.

An adjacent side placement makes a trapdoor face the horizontal block face that was clicked. Replacing the clicked block and clicking a vertical face retain vanilla's opposite-player-facing branch. Mirrors vanilla 26.3 `TrapDoorBlock.getStateForPlacement`. Fixes [#23](https://github.com/Optifineless/Pumpkin/issues/23). In-game verification: **Not yet**.
Ender Dragon wing contact pushes any living entity and can deal 5 mob-attack
damage; head/neck contact deals 10. Part dimensions and contact ordering mirror
vanilla 26.3 `EnderDragon.aiStep/knockBack/hurt` and `EnderDragonPart`, including
Creative/Spectator, hurt-timer and team exclusions. Each effect rechecks the live
target's world, presence, life and eligibility after earlier callbacks (#201),
preserving PR #194. Implemented by Codex (GPT-6.1 Sol). In-game verification:
**Not yet**. Saved formats, native layouts and reserved part IDs are unchanged.
Dragon contact now collects candidates once from the union of its four part
boxes and filters out distant entities before cloning their Arcs. The global
registry still inspects each entry once because it has no spatial index.
NoAI dragons skip part contact effects. Reflected Thorns damage is admitted only
when a Player caused it or its damage type is tagged to hurt dragons, and it uses
body damage scaling. Mirrors vanilla 26.3 `EnderDragon.aiStep/hurtServer/hurt`;
real-tick NoAI and Thorns regressions cover the new contact paths. In-game
verification: **Not yet**.
The registered `/kill` command removes a dragon directly and settles the tracked
Ender Dragon fight, matching vanilla `EnderDragon.kill`; ordinary damage
admission remains unchanged. A regression dispatches the real command against a
healthy dragon and checks removal and fight state. Implemented by Codex (GPT-6
Luna). In-game verification: **Not yet**.

Infinity bow shots mark free ordinary arrows as creative-only pickup, while Infinity
still consumes tipped and spectral arrows. Mirrors vanilla 26.3
`BowItem.releaseUsing`, `ProjectileWeaponItem.draw/useAmmo` and
`AbstractArrow` construction. Fixes #92. In-game verification: **Not yet**.

Lantern survival uses the support side selected by the stored `HANGING` state,
and ladder placement retains replaced water as `WATERLOGGED`.
Mirrors vanilla 26.3 `LanternBlock.canSurvive/getConnectedDirection/updateShape`
and `LadderBlock.getStateForPlacement/updateShape`. Fixes #51 and #54.
In-game verification: **Not yet**.

Play test: hang a lantern below a ceiling with a separate floor beneath it, then
remove only the ceiling; expect the lantern to drop. Place a standing lantern on
a floor with a separate ceiling above it, then remove only the floor; expect the
lantern to drop. Place a ladder against a wall into source water; expect the
ladder to retain water, then place one against the wall in dry air and expect
the ordinary facing and item consumption.
## Mob visibility, avoidance and navigation follow-up (#2)

The 26.3 `mob_visibility` item component is parsed, saved, generated and encoded
through the normal item paths. Target ranges share
`LivingEntity.getVisibilityPercent`: sneaking, invisible armor cover and matching
equipped-item components apply before the two-block minimum. Player MAIN_HAND
comes from the selected hotbar stack, including before equipment publication.
`TargetingConditions.test` and piglin targetability preserve Java `Math.max` NaN
propagation. The ownership audit is in
[inventory-equipment-ownership.md](.fork/inventory-equipment-ownership.md).

`AvoidEntityGoal.canUse` retains nearest valid threat selection and now matches
Zombie subclasses and trader llamas. Cats stop avoiding players when tamed;
ocelots stop when trusting. `Wolf.WolfAvoidEntityGoal.canUse` first selects the
nearest valid llama and finds its escape path, then checks that the wolf is
untamed and calls `avoidLlama`: that llama's strength must be at least a random
integer from zero through four. A failed strength roll does not select a farther
llama. Wolf avoidance clears its combat target on start and tick. Villager Zombie
avoidance excludes zombified piglins. Wandering traders reuse the shared hostile
table while retaining their seven vanilla class goals, eight-block Vindicator
range and lack of Ravager avoidance.

Direct paths and cancellation clear Pumpkin's queued coordinate request and
delayed recomputation, so an old target cannot replace a new route or restart a
cancelled one. Completed coordinate requests already consumed their pending goal;
that behavior remains. The default destination test now uses solid rendering
below the target, preserving the existing ground-navigation check. Navigation
target positions and wall-climber fallback behavior retain their vanilla meaning.

Mirrors vanilla 26.3 `LivingEntity.getVisibilityPercent`, `PlayerEquipment.get`,
`MobVisibility.CODEC/STREAM_CODEC`, `TargetingConditions.test`,
`VillagerHostilesSensor`, species avoidance overrides and
`PathNavigation.moveTo/isStableDestination`. Deferred cancellation also covers
Pumpkin's queued-movement adapter. No public plugin API or event changes.
Native API 8 is retained; rebuild native plugins for the private avoidance state.
Implemented by Codex. In-game verification: **Not yet**.

## Spawn protection bypass through block use (#19)

Java and Bedrock block use now reject a protected clicked block before interaction
events and block/item hooks, then correct the clicked block, adjacent block,
block-entity data and used hand. Bucket pickup and emptying check the same
protected clicked position before their effects. Spawn protection follows the
vanilla operator-list membership, radius and shared respawn-dimension rules.
Mirrors vanilla 26.3 `DedicatedServer.isUnderSpawnProtection`,
`ServerGamePacketListenerImpl.handleUseItemOn`, `BucketItem.use` and
`ServerLevel.mayInteract`. In-game verification: **Not yet**.

## Glow ink on blank signs (#50)

Glow ink now applies only when the selected sign face has visible text, respecting
the player's text-filtering setting. Blank opposite faces do not qualify, and
honeycomb can still wax a blank sign. Mirrors vanilla 26.3
`SignBlock.useItemOn` and `SignApplicator.canApplyToSign`. In-game verification:
**Not yet**.

## Pressure-plate entity eligibility (#82)

Stone pressure plates count living entities; wooden and weighted plates count all
eligible entities. Every variant excludes spectators, removed entities and entities
that ignore block triggers, with weighted plates filtering before saturation. The
shared calculation covers collision activation and scheduled recounts. Mirrors vanilla
26.3 `PressurePlateBlock.getSignalStrength`,
`WeightedPressurePlateBlock.getSignalStrength`,
`BasePressurePlateBlock.getEntityCount` and `Entity.isIgnoringBlockTriggers`.
Vanilla block-trigger overrides are retained for bats, marker armor stands,
displays, interactions, markers and ominous item spawners; spectators remain
separately filtered. Player spectator mode also maps to the new capability.
Native API 8 (same version): `EntityBase` gains a default method; rebuild native
plugins. In-game verification: Not yet.

Management-enabled shutdown (#173) now stops the listener, WebSocket sessions and
heartbeat so the final player, world and level.dat saves can finish. The authenticated
`minecraft:server/stop` RPC signals shutdown before its bounded, best-effort success
reply. WebSocket clients subscribe before the upgrade response, and pending upgrades
are counted by the shutdown drain. The server broadcasts `server/stopping` once before
player and world saves; delivery remains best-effort. Incomplete ordinary HTTP requests
no longer hold final saves: after the five-second management grace, remaining listener
connections are closed and awaited. Later requests in a stop batch receive a
server-stopping error. Concurrent shutdown callers share one atomic claim, so
the stopping notification is queued before session cancellation. The public
`ManagementHub::broadcast_server_stopping()` completes both steps, so a plugin
cannot consume the claim and strand the listener or heartbeat before final saves.
Regression children now have a parent-side kill deadline and must match one exact
test. Native API 8 (same version): `ManagementHub` shares the server's runtime
management shutdown token; `Server` has runtime management shutdown state, and
`ManagementServer` exposes listener binding separately from serving; rebuild native
plugins. Written by Codex.
The idle stdin task also stops on cancellation, using umceko's upstream
[PR #3788](https://github.com/Pumpkin-MC/Pumpkin/pull/3788) approach.
In-game verification: **Not yet**.

Management scheduling and startup gates ([#179](https://github.com/Optifineless/Pumpkin/issues/179))
keep incoming RPCs and pings serviceable during notification floods. A separate tracked
WebSocket writer drains the bounded, drop-oldest notification queue and a bounded reply
queue; reply saturation disconnects the peer without awaiting capacity in the reader.
An accepted stop signals shutdown before its bounded, best-effort acknowledgement.
Shutdown drains a fixed notification budget even while producers remain active.
A stop accepted during plugin initialization lets the in-flight plugin finish, skips
later plugin initializations, hot-reload setup and startup announcements, then uses
the normal shutdown owner. Indefinitely blocked in-flight plugins are not cancelled.
Vanilla 26.3 `Connection.channelRead0/sendNotification` and `ManagementServer` provide
the transport comparison; the plugin startup gate is a fork lifecycle safeguard.
Decompiled 26.3 was unavailable for this review follow-up. No plugin API or saved-data
change. Written by Codex (GPT-6.1 Sol). In-game verification: **Not yet**.

Close-event listeners can reopen a menu without closing its predecessor recursively. The closing-menu identity survives callbacks, and an outer close preserves a replacement menu and position. Native API 8 (same version): Player has runtime close ownership state; rebuild native plugins. In-game verification: **Not yet**.

## Entity unload ownership, #150 split PR-1

Entity retirement runs in the owning world's entity tick phase. Rewatch cancels the
pending generation; the commit checks that generation, visibility and the entity
storage instance. Terrain stays published until retirement commits. Native API 8
(same version): Level, World and entity storage layouts change; rebuild native plugins.
In-game verification: **Not yet**.

Explicitly unprotected after this PR: Tokio-side mutation during serialization
(a stale snapshot can still win until PR-2's mark-removed-first); ordinary
`/save-all` versus retirement overwrite (PR-3); the block-entity rewatch-between-
snapshot-and-detach gap (PR-2); equipment/cargo/invulnerability writers (named
follow-up after PR-3). Block-entity saving and detachment retain the pre-PR path.

## Entity unload retirement, #150 split PR-2 (v2)

Registry detachment succeeds before the irrevocable first-reason removal mark.
Detached entities serialize outside admission, including unloaded passenger trees;
terrain/cleanup authority stays retained until their records are published.
Later position, teleport, world, health and riding writes are rejected at entry.
Forced tickets publish authoritative watches synchronously, and final terrain
unpublication rechecks watch authority, residency tickets and the expected chunk
incarnation together. The shared ticket wrapper stays in an Arc for portal residency.
API 8 (same version): existing force/task/ticket locks are instrumented and the
force transitions share unload admission; rebuild native plugins. Written by Codex.
In-game verification: **Not yet**.

**EXPLICITLY UNPROTECTED** until the named follow-ups:

- A writer that passed its `is_removed` check before the mark and writes after
  serialization (tick-phase window): entity writer ownership follow-up. Entry
  checks are best-effort rejection, not exclusion of an already admitted writer.
- A block-entity handle (`get_block_entity` Arc) retained across a rewatch, and
  the existing block-entity snapshot/detach gap: **PR-2b: block-entity leaf snapshot
  contract**. Block-entity save/detach ordering is unchanged in this PR.
- Ordinary `/save-all` versus retirement overwrite: **PR-3: shared ordinary-save
  admission**. This race remains accepted and unprotected here.
- Equipment/cargo/invulnerability and other persisted writer families: named
  writer coverage follow-up after PR-3.
- Per-chunk registry rescan cost and unordered movement storage-watcher deltas:
  P2 backlog. Empty membership now skips the replacement registry allocation,
  and completed hidden never-installed storage forgets its generation without
  resetting the monotonic generation counter.
Bedrock inventory requests reject forged hand and legacy slot predictions, Survival creative-item requests, unpaid preview-slot operations and cursor aliases ([#168](https://github.com/Optifineless/Pumpkin/issues/168)). Implemented by Codex (GPT-6.1 Sol). In-game verification: **Not yet**.
Bedrock inventory requests reject forged hand and legacy slot predictions, Survival creative-item requests, unpaid preview-slot operations and cursor aliases ([#168](https://github.com/Optifineless/Pumpkin/issues/168)). Client slot predictions do not suppress authoritative block, air or entity uses. Mirrors vanilla `ServerGamePacketListenerImpl.handleContainerClick`, `handleSetCreativeModeSlot`, `handleUseItemOn` and `handleUseItem`. Implemented by Codex (GPT-6.1 Sol). In-game verification: **Not yet**.

Bedrock click-block and click-air use the hand declared by the packet, including an empty offhand beside an occupied main hand. Hand descriptors check item identity (including mapping metadata), not predicted counts: client stacks and components are never installed. The existing Bedrock component codec does not decode component NBT for comparison. Transactions without slot predictions immediately correct only changed inventory slots; unchanged transactions send no inventory correction. Changed hands enqueue through the shared per-tick equipment publisher, which refreshes modifiers and corrects owner offhand window 119/slot 0 and tracking players once per changed hand, matching `LivingEntity.detectEquipmentUpdates/collectEquipmentChanges/handleEquipmentChanges`. Full corrections remain for discarded slot predictions or rejected hand identities. Mirrors `ServerGamePacketListenerImpl.handleUseItemOn/handleUseItem`, `ServerPlayerGameMode.useItemOn/useItem` and `AbstractContainerMenu.broadcastChanges` ([#169](https://github.com/Optifineless/Pumpkin/pull/169) follow-up). Implemented by Codex (GPT-6.1 Sol). In-game verification: **Not yet**.

## Chest restart regression, #172

Deterministic regressions exercise two stale-publication windows through real
container, entity-unload, save and reload paths. A captured chest image is discarded
if its live instance was replaced before publication, mirroring
`LevelChunk.getBlockEntityNbtForSaving`. An ordinary entity save drops a captured
live image after retirement has marked the entity removed, leaving the tick-owned
retirement record authoritative, mirroring
`PersistentEntitySectionManager.storeChunkSections/unloadEntity`. The stop path
joins the ticker and packet/server tasks before world shutdown captures and saves
the final chest state. The earlier terrain unpublication route is already closed
by #170 `claim_terrain_unload`, which retains terrain under admission, rechecks
tickets and holder targets, and removes only the expected chunk incarnation,
mirroring `ChunkMap.scheduleUnload` identity checks. Written by Codex. In-game
verification: **Not yet**.

The #191 follow-up closes the ordinary `/save-all` replacement window too: if the
captured block entity was replaced, save retries from the currently registered
instance and attaches its current `PumpkinCustomData` before publishing. This keeps
replacement state for entities whose `chunk_data_nbt()` is `None`, and matches
`LevelChunk.getBlockEntityNbtForSaving` resolving the current live entity. In-game
verification: **Not yet**.

The #191 round-three follow-up orders block-entity NBT publications by capture
stamp, so an older save of the same live chest cannot overwrite a newer image.
Chunk unload and server shutdown seal replacement admission before the final
capture is published; a same-position replacement already in flight is captured,
and later saves or replacements cannot overwrite that final image. The saved NBT
layout and meaning are unchanged. In-game verification: **Not yet**.

The #191 round-four follow-up keeps the loaded-chunk eviction fence until a new
block-entity image is marked dirty, and retires each replacement inventory before
the final unload or shutdown capture. Tests cover the actual `/item replace block`
command during chunk unload and a retained-handle transfer during server stop,
then reload the captured chest contents. In-game verification: **Not yet**.

The #191 round-five follow-up publishes only complete persistent block-entity
images; `chunk_data_nbt()` remains packet data. Capture stamps reset with the live
instance, and ordinary saves keep retrying until they capture the current one.
Unload and stop close mutation admission before their final capture. Block placement,
replacement, sign edits, block interactions, jigsaw edits and custom block-entity data
are fenced at their mutation entry points, including native sign/map setters and
both Wasm host versions. Terrain stays retained through the final capture, and its
incarnation remains sealed until activation or reload. Concurrent replacements retire
the exact predecessor before swapping. Chunk packets project full saved images through
each block entity's client serializer. Regression scenarios exercise deferred chest loot,
late chest placement, sign edits during unload, replacement churn and concurrent
container replacement through save or unload and reload. Entity saves also retry
when saved records are activated into the live registry during serialization. The
saved NBT layout, plugin method signatures and WIT are unchanged. The private native
layouts and hidden host coordination use API 8 (same version); rebuild native plugins.
Direct public field writes bypass the supported native setters' admission contract.
In-game verification: **Not yet**.

The #191 round-seven stopgap reopens completed block-entity retirement when the
same terrain is still loaded. Writers restore the captured entity before replacing
its block, and a fresh retirement generation retains terrain until the next entity
phase captures the reopened state. This also works after entity storage has been
cleaned; new roots request storage through the existing asynchronous unload path.
Mutation admission temporarily excludes captures across the whole world so nested
physics callbacks cannot lose blocks at another chunk boundary. Off-tick entry
points wait at most 250 ms for an active capture; capture callbacks never wait on
their own thread, and Wasm reports rejected writes or explosions as errors. End
platform construction preflights its footprint before transit. Native API remains
8 (same version); rebuild native plugins for the private admission layout change.
Saved NBT and WIT are unchanged. Vanilla 26.3 `LevelChunk.setBlockState`,
`getBlockEntityNbtForSaving` and `ChunkMap.scheduleUnload` provide the ordering;
the full terrain-lifetime redesign remains issue #224. Written by Codex.
In-game verification: **Not yet**.

Play test: put items in a chest at the edge of simulation distance, return after
it stops ticking, then explode it; expect one chest and its contents exactly once.
Push a block across that edge with a piston and collect water there with a bucket;
expect the block to move and the water to disappear only when the bucket fills.
Place and fill a chest there, save, leave the area and return, then restart; expect
the contents to remain. Record these border interactions and the reload.

The #191 round-7d follow-up runs lectern page and book-removal world effects after
the menu guard releases, using the same effect batch as anvils. Deferred effects
check the original block entity is still live; a removed book still reaches the
player's inventory or drops once when it is full, even if the lectern disappears.
Mirrors vanilla 26.3 `LecternMenu.clickMenuButton`, `LecternBlockEntity.setPage`
and `LecternBlock.signalPageChange/resetBookState`. Written by Codex
(GPT-6.1 Sol). Saved NBT, native API and WIT are unchanged.
In-game verification: **Not yet**.

Play test: open a lectern with a multi-page book and turn pages; expect the page
and redstone pulse to update without a server error. Take the book with space in
your inventory, then repeat with a full inventory; expect exactly one book in
your inventory or on the ground, and an empty, unpowered lectern. Record both
book returns and a page-turn pulse.

## World normal/frozen service boundary, ticks M1

Normal World ticking and each frozen flush/player batch now require exclusive
per-World service entry after restored World age initialization. Rejected entry
runs no body and advances no service epoch; receipts release on return or unwind.
This mirrors `MinecraftServer.tickChildren` / `ServerLevel.tick` serialization
while retaining Pumpkin's parallel ticking of different Worlds. Written by Codex
(GPT-6.1 Sol). Native API 8 (same version): World layout changes; rebuild native
plugins. In-game verification: **Not yet**.

Adapted after M0 on `4020a2658`: initialization compares the recorded restored age
with the shared Level scheduling clock before admitting service. A mismatch logs
both ages and leaves service unavailable; initialization cannot be retried. Real
saved-data loading and mismatched-clock rejection have regression coverage.

Level publication/admission sealing remains M2/M4/M10. Public Level APIs and
`Level::new` stay open. M1 establishes service exclusion only, not scheduler writer,
terrain lifecycle, coherent image, block-entity or shutdown ownership. U1/U2/U5/U8/
U9/U13/U14/U17 and the remaining U15/U16 limitations in the ownership map remain.
The next architectural stage is M3 transport, followed by M2 lifecycle routing.

Play test: run a repeating redstone circuit, freeze ticking, place/use blocks,
then unfreeze; expect player operations to work while frozen and simulation to
resume. Stop/restart with pending redstone or water updates; expect the existing
restored-time behavior. Record freeze → place/use → unfreeze and report any
World service entry invariant error in the log.

## Lifecycle capture boundary, PR-2b

BE saving enters a checked leaf serialization boundary, following vanilla
`ChunkMap.save`, `SerializableChunkData.copyOf` and
`LevelChunk.getBlockEntityNbtForSaving`. In debug builds, capture reentry into
mapped world, packet, plugin and effect boundaries is rejected, including virtual
item-component calls. The reviewed lifecycle manifest detects changed capture,
live-NBT and mutable API coverage. Release checks compile out with the existing
lock checker; serialization and saved NBT semantics remain legacy.

Existing live writers, terrain publication, image ordering and detach remain
legacy until the ownership map's ordered migrations activate. Retained chest
handles, final capture/detach and ordinary-save stale images remain unprotected;
this does not fix S6 or complete rule F3. Public native capture/debug interfaces
are additive API. API 8 (same version): rebuild native plugins; WIT and saved-data
formats are unchanged. Implemented by Codex (GPT-6.1 Sol).
In-game verification: **Not yet**.

Straw-bed wake cleanup preserves blocks and container contents replaced by plugin
callbacks, and honors cancellation for each half before removal. Committed removal
keeps normal neighbor work, requested drops and Java/Bedrock destroy effects.
Mirrors `StrawBedBlock.destroyBed`; implemented by Codex (GPT-6.1 Sol).
In-game verification: **Not yet**.

## How the code here is written

The changes in this fork are written by AI coding agents, not by hand. The fork owner tests them in-game but does not review the code line by line. Treat every commit that isn't from upstream accordingly.

Each change is still held to upstream's [AGENTS.md](AGENTS.md) rules: it is ported from the decompiled vanilla source, stays one topic per commit, passes `cargo fmt`, `clippy` and the tests, and comes with a regression test where one can catch the bug. That keeps the commits small enough for upstream to take any of them if they want to.

None of this is sent upstream as pull requests. Upstream maintainers are welcome to cherry-pick anything they find useful.

## Staying in sync

Upstream is merged in regularly. When upstream fixes something this fork also fixed, the fork drops its own version and keeps upstream's.

## Tick stall watchdog

Watchdog report directories are created or secured to Unix mode0700; symlink directories are refused. Final reports retain mode0600. This complements the bounded `ServerWatchdog.createWatchdogCrashReport` phase diagnostics. Optional GNU debugger fixtures explicitly skip absent or incompatible utilities; isolated restricted-PATH regressions verify that disposition. In-game verification **Not yet**.

Vanilla 26.3's `ServerWatchdog` now has a dedicated thread that reports a stalled
tick and exits without saving. `max_tick_time` in `pumpkin.toml` defaults to
60000 milliseconds; non-positive values disable it. Every executing tick is
monitored, including frozen player/packet work, `/tick step` and sprints. Startup
generation and intentional waits between ticks are unarmed. Shutdown remains
monitored until the ticker exits, then disarms before saving. Reports include
active phases, entity IDs and types, block entity positions, dispatch-snapshot
player/chunk counts and Tokio
metrics. Linux stack capture tries `eu-stack`, then `gdb` on failed or unusable
output, preserving both failure reasons, with an external timeout;
Windows reports phase markers. Owner verification: Not yet.

The watchdog and its parked halt helper are ready before gameplay ticking,
including ticks awaited by plugin `on_load` hooks; this mirrors the lifecycle
ordering in `DedicatedServer.initServer` and `MinecraftServer.runServer`.
Failure detection arms the fallback without allocating; the bounded report
retains world UUIDs and restores parent block positions after nested work.
The phase report is saved before optional stack capture. If the halt helper
cannot start, startup fails before the ticker is spawned; gameplay never runs
without its configured fallback.

Linux Yama `ptrace_scope=1` capture authorizes only a gated launcher PID with
`PR_SET_PTRACER` and revokes it after capture; its debugger descendants are
allowed. Absent Yama and scope 0 use ordinary ptrace permissions without the
exception. `PR_SET_PTRACER_ANY` is never used. The separate launcher waits for
`timeout --foreground` to reap the debugger, then resumes the server with
`SIGCONT` before returning; it remains runnable if the tracer left the server
stopped. Capture requires `/bin/sh`, GNU `timeout`, an executable debugger and
normal same-user/dumpable permissions.
Yama modes 2/3 and other security policies can still deny capture. The attachment
integration test is opt-in on hosts that permit ptrace; ordinary tests cover
capture failure and fallback.
A standalone Linux probe validated mode-1 attachment, a timed-out debugger's
group stop and server resumption. Scratch boot and in-game verification remain
separate checks. Final reports and temporary stack dumps are readable only by
their owner, and enrichment appends without truncating the saved phase report.
The external supervisor and its descendants have a 4 MiB file-size limit;
overflow cannot terminate the supervisor before it reaps and resumes the server.
A supervised real tick-start callback regression checks fatal exit and the saved
phase report with a short positive watchdog limit.

The synthetic slot benchmark measures warm, pre-registered workers, including
per-batch scopes and per-subject markers. It excludes first-use registration,
catalog replacement, chunk-stat sampling and the tick clock.

## Upstream pull requests included early

These are open upstream PRs merged here before upstream merges them. Each is dropped from the fork once upstream merges its own version. Fork candidates are listed by source branch and commit.

| Upstream PR | What it fixes |
|:--|:--|
| [#3821](https://github.com/Pumpkin-MC/Pumpkin/pull/3821) by towner-10 | Casting showed no bobber because its spawn packet lacked the caster id. Hook and rod behaviour adapted from this PR to vanilla 26.3; the fishing loot context, open-water treasure predicate and XP rewards are fork work. Follow-up matches the bite kick rounding, honours failed-attempt cancellation and rod vibration settings, keeps owner grace around dying passengers, and clears the caster immediately when a hook is removed externally. In-game verification: Not yet. |
| [#3718](https://github.com/Pumpkin-MC/Pumpkin/pull/3718) by KBDL | Fish swim underwater and flop on land; squid and glow squid move with their tentacle strokes. Ported to pumpkin-core and checked against vanilla 26.3. |
| [#3863](https://github.com/Pumpkin-MC/Pumpkin/pull/3863) | Scheduled ticks ran one tick late, ticks saved with a chunk never ran after loading, and observers misbehaved. |
| [#3853](https://github.com/Pumpkin-MC/Pumpkin/pull/3853) | Falling out of the world killed instantly instead of in steps, because void damage skipped the hurt cooldown. |
| [#3904](https://github.com/Pumpkin-MC/Pumpkin/pull/3904) | Sleeping players were not woken when hurt. |
| [#3813](https://github.com/Pumpkin-MC/Pumpkin/pull/3813) | Harvested recursion guards, now replaced by vanilla's iterative execution queue and generated command quota. |
| [#3876](https://github.com/Pumpkin-MC/Pumpkin/pull/3876) | Mace smash damage, knockback and sounds did not follow vanilla. |
| [#3859](https://github.com/Pumpkin-MC/Pumpkin/pull/3859) | Block entity data lingered after its block was removed, so a later block of the same kind could inherit old contents. |
| [#3905](https://github.com/Pumpkin-MC/Pumpkin/pull/3905) | Wind charges could not be thrown at blocks, launched players ever higher, and had no burst effects. |
| [#3845](https://github.com/Pumpkin-MC/Pumpkin/pull/3845) | Entities loaded from disk were frozen, `/forceload` did not keep chunks loaded, attribute changes were lost on reload, and melee knockback ignored the knockback attribute. |
| [#3861](https://github.com/Pumpkin-MC/Pumpkin/pull/3861) | Mobs spawned with equal odds instead of vanilla weights, so rare mobs were as common as zombies. |
| [#3891](https://github.com/Pumpkin-MC/Pumpkin/pull/3891) by Rennex07 | Spawn-potential distances use floating-point arithmetic before subtraction and squaring, avoiding overflow far from the origin. |
| [#3804](https://github.com/Pumpkin-MC/Pumpkin/pull/3804) | Hoppers took dropped stacks one item at a time and never picked items out of their own bowl. |
| [#3348](https://github.com/Pumpkin-MC/Pumpkin/pull/3348) by JulesB40 | Goat horn instrument holders preserve their references and inline definitions. Adapted to the generated instrument registry and vanilla 26.3's durability damage field. |
| [#3897](https://github.com/Pumpkin-MC/Pumpkin/pull/3897) by ToffyMTA | Charged crossbows preserve projectile items and intangible-projectile NBT. Adapted to vanilla 26.3's item templates and 1,024-projectile bound. |
| [#3827](https://github.com/Pumpkin-MC/Pumpkin/pull/3827) by luisakrivonogih | Block-state argument parsing, extended with block entity SNBT and vanilla placement callbacks. |
| [#3763](https://github.com/Pumpkin-MC/Pumpkin/pull/3763) by AdmerPRO | `/tick sprint` announces its start without printing an incomplete completion report. |
| [#3642](https://github.com/Pumpkin-MC/Pumpkin/pull/3642) by CocofireHD | Edition-specific `/gamemode` feedback argument order. |
| [#3638](https://github.com/Pumpkin-MC/Pumpkin/pull/3638) by CocofireHD | Hardcore difficulty override, including startup before worlds are initialized. |
| [#3394](https://github.com/Pumpkin-MC/Pumpkin/pull/3394) by JulesB40 | Defer datapack load functions until ticking resumes; keep the pending flag with the function library. |
| [#3807](https://github.com/Pumpkin-MC/Pumpkin/pull/3807) by 4d1cksupmya55-source | Apply command NBT to live block entities, close stale inventory screens and notify comparators. |
| WhiteProject1/fix/rayon-panic-handler, `15a5fa8fc` | Detached worker panics reach the existing shutdown hook instead of aborting the process. In-game verification: Not yet. |
| [#3950](https://github.com/Pumpkin-MC/Pumpkin/pull/3950) by qhashofficial, `416a1bd88` | Wasm plugins can reenter blocking host calls after the call is handed to another thread. In-game verification: Not yet. |
| [#3932](https://github.com/Pumpkin-MC/Pumpkin/pull/3932) by saranxzi, `041d2ce47` | Exposed farmland hydrates in rain; roofed or snowy farmland stays dry. In-game verification: Not yet. |
| [#3931](https://github.com/Pumpkin-MC/Pumpkin/pull/3931) by saranxzi, `5596ea875` | Farmland uses its survival tag; paths retain the fence gate exception, and delayed conversion rechecks support. In-game verification: Not yet. |
| The-Hypnos/feat/brain-phase4, `973d8a8e9` | Sleeping mobs remain still when the shared push entry point runs. In-game verification: Not yet. |
| Alb11747/feat/fobbitmc-preservation, `a35b075c6` (related alternative: [#3765](https://github.com/Pumpkin-MC/Pumpkin/pull/3765)) | Villager trades finish without recursively locking the trading screen; payment and offer use are retained. In-game verification: Not yet. |
| Alb11747/feat/fobbitmc-preservation, `f11a3f31a` | Villagers restock only at their assigned workstation, without consuming a pending-job cooldown. In-game verification: Not yet. |
| [#3656](https://github.com/Pumpkin-MC/Pumpkin/pull/3656) by creeperkatze, `8ad42f95f` | Container open/close events follow the first viewer opening and the last viewer closing. In-game verification: Not yet. |
| [#3660](https://github.com/Pumpkin-MC/Pumpkin/pull/3660) by creeperkatze, `6ded5c330` | Jukebox record changes emit one block-change event for players and automation; playback ending keeps the record. In-game verification: Not yet. |
| [#3843](https://github.com/Pumpkin-MC/Pumpkin/pull/3843) by oystrpj, `cb97bb831` | Button presses exclude the actual presser from the predicted sound; wind charges and releases broadcast normally. In-game verification: Not yet. |
| Mirkrog/chest-fixes, `5bcc98c67` | Hoppers access both valid double-chest halves with atomic extraction, insertion and failed-transfer rollback. In-game verification: Not yet. |
| [#3645](https://github.com/Pumpkin-MC/Pumpkin/pull/3645) by AdmerPRO, `4568d117f` | Dispensers launch experience bottles as projectiles. In-game verification: Not yet. |
| AdmerPRO/fixexpdrop, `f815e681d` | Experience bottles award collectible XP on impact through the existing orb implementation. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `eb08d84d5` | Sheep, snow golems and mooshrooms use shared shearing with reloadable loot and leash-snipping priority. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `fe82fb390` | Bogged skeletons can be sheared for their mushroom loot. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `07d9188e7` | Plugins can cancel mooshroom conversion without losing the mob or producing loot and sounds. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `a88878fea` | Shearing honors cancelled or adjusted item-damage events before changing the actual hand. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `192d93be2` | Atomic shear claims prevent duplicate loot and cancelled conversions release their claim. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `1b1ec0afc` | Mooshroom conversion transfers the vehicle and first passenger before evaluating shearing loot, including boarding state. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `b810aee7a` | Mooshroom conversion transfers its scoreboard team to the cow. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `3b594fb87` | Dispensers skip ordinary shearing of mobs during their death animation after checking leashes. In-game verification: Not yet. |
| [#3829](https://github.com/Pumpkin-MC/Pumpkin/pull/3829) by tom-devv, `585eb0dd5` | Dyeing and shearing preserve the sheep color and shear state atomically. In-game verification: Not yet. |
| [#3506](https://github.com/Pumpkin-MC/Pumpkin/pull/3506) by Thijs226, `56200c279` | Hardcore death respawns at a valid saved bed or anchor before switching to spectator mode. In-game verification: Not yet. |
| [#3506](https://github.com/Pumpkin-MC/Pumpkin/pull/3506) by Thijs226, `ae755dfd5` | Hardcore respawn uses the normal cancellable game-mode change and one in-flight respawn claim. In-game verification: Not yet. |
| [#3506](https://github.com/Pumpkin-MC/Pumpkin/pull/3506) by Thijs226, `512c0133a` | Duplicate respawn requests leave the original in-flight claim held. In-game verification: Not yet. |
| [#3684](https://github.com/Pumpkin-MC/Pumpkin/pull/3684) by Thijs226, `22afc2a29` | Armor stands exchange equipment atomically using the actual inventory hand, including concurrent removals and insertions. In-game verification: Not yet. |
| [#3684](https://github.com/Pumpkin-MC/Pumpkin/pull/3684) by Thijs226, `f63565443` | Armor stand clicks honor plugin-adjusted target and hit position. In-game verification: Not yet. |
| [#3684](https://github.com/Pumpkin-MC/Pumpkin/pull/3684) by Thijs226, `bc3ed49aa` | Armor stands drop equipment on break except vanishing equipment, and preserve the stand item name. In-game verification: Not yet. |
| [#3684](https://github.com/Pumpkin-MC/Pumpkin/pull/3684) by Thijs226, `67cb93a2f` | Armor stand transfers increment neither item-use nor broken-item statistics; genuine durability breaks retain their status and statistics. In-game verification: Not yet. |
| [#3684](https://github.com/Pumpkin-MC/Pumpkin/pull/3684) by Thijs226, `8274cc939` | Armor stand disabled slots, scaled click regions and equipment events follow vanilla. In-game verification: Not yet. |
| [#3684](https://github.com/Pumpkin-MC/Pumpkin/pull/3684) by Thijs226, `687d7b161` | Bedrock armor stand swaps retain known transaction semantics and separate item-use statistics. In-game verification: Not yet. |
| [#3705](https://github.com/Pumpkin-MC/Pumpkin/pull/3705) by Q2297045667, `2082caa93` | 26.3 spectator clicks decode optional entity IDs; legacy Wasm UUID records remain unchanged. In-game verification: Not yet. |
| [#3744](https://github.com/Pumpkin-MC/Pumpkin/pull/3744) by Q2297045667, `e262814c4` | Spectator cameras validate loaded state, current-world entities and dragon parts, border, range and shared pickability. Ordinary interaction packets do not set spectator cameras. In-game verification: Not yet. |
| [#3884](https://github.com/Pumpkin-MC/Pumpkin/pull/3884) by tom-devv, `ebf8b8ed8` | Java and Bedrock survival attacks teleport dragon eggs and play note blocks after protection, game-mode restrictions and plugin cancellation. In-game verification: Not yet. |
| [#3951](https://github.com/Pumpkin-MC/Pumpkin/pull/3951) by AdmerPRO, `2003d0def` | Banners read current support during queued updates and drop their standing banner item with patterns when support is removed. In-game verification: Not yet. |
| Phoenixxo/fix/block-update-stack-overflow, `df2fe1710` | Neighbor cascades run iteratively with per-world state and resumable vanilla direction order. In-game verification: Not yet. |
| Phoenixxo/fix/block-update-stack-overflow, `f8099507e` | Creative support removal still drops dependent blocks rather than carrying the original no-drop flag. In-game verification: Not yet. |
| Phoenixxo/fix/block-update-stack-overflow, `7994c6f66` | Queued shape updates capture neighbor state, retain depth limits and clean up after callback panics. In-game verification: Not yet. |

## Shared inventory duplication fixes

The `inventory-transactions` branch addresses shared-state audit finding 3.1 and consolidated
row 8: two viewers shift-clicking one chest slot, player pickup competing with a hopper minecart,
and stale rollback after a failed cart transfer. Transfers claim the shared source, update
storage atomically and dispatch effects after releasing ownership. Vanilla 26.3 `ChestMenu`,
`AbstractContainerMenu`, `ItemEntity` and `HopperBlockEntity` define the resulting counts.
In-game verification: Not yet. AI implementation: Codex (GPT-6).

## Upstream issues addressed here

| Upstream issue | Fixed by | Checked in-game |
|:--|:--|:--|
| [#3092](https://github.com/Pumpkin-MC/Pumpkin/issues/3092) XP orbs do not absorb, [#3624](https://github.com/Pumpkin-MC/Pumpkin/issues/3624) XP orbs sink in water | vanilla 26.3 orb award/merge, motion, collection, damage, metadata, persistence and Mending port; upstream #3644's buoyancy approach checked, with 26.3's eye-in-water eligibility | Not yet |
| [#3511](https://github.com/Pumpkin-MC/Pumpkin/issues/3511) Player saves truncate the last good file | durable temporary replacement, backup recovery and retained retries | Not yet |
| [#3512](https://github.com/Pumpkin-MC/Pumpkin/issues/3512) Older snapshots overwrite disconnect saves | ordered snapshots, a tick barrier during disconnect capture/removal and a UUID gate through final publication | Not yet |
| [#3468](https://github.com/Pumpkin-MC/Pumpkin/issues/3468) Aquatic mob AI (fish and squid movement only) | port of upstream PR #3718 | Yes, 2026-10-07 |
| [#3468](https://github.com/Pumpkin-MC/Pumpkin/issues/3468) Aquatic mob controls and navigation beyond fish/squid | Species movement controls, autonomous travel, contextual navigation rays, strider lava support and aquatic air handling; full routes, mounted movement and block-change path recomputation remain incomplete | Not yet |
| [#3388](https://github.com/Pumpkin-MC/Pumpkin/issues/3388) Arrows have glitchy particles | remove server-generated arrow trails | Not yet |
| [#3520](https://github.com/Pumpkin-MC/Pumpkin/issues/3520) Shields cannot block player melee | `fix(combat): let shields block melee hits` | Yes, 2026-10-07 (blocked a zombie) |
| [#3105](https://github.com/Pumpkin-MC/Pumpkin/issues/3105) Observers don't work | upstream PR #3863 | Not yet |
| [#877](https://github.com/Pumpkin-MC/Pumpkin/issues/877) Naturally generated water sometimes doesn't flow | upstream PR #3863 | Not yet |
| [#3113](https://github.com/Pumpkin-MC/Pumpkin/issues/3113) Goat horns disconnect inventory users | adapted upstream PR #3348 | Not yet |
| [#3108](https://github.com/Pumpkin-MC/Pumpkin/issues/3108) Picking up paintings disconnects inventory users | painting item components encode registry holders | Not yet |
| [#3844](https://github.com/Pumpkin-MC/Pumpkin/issues/3844) Loading a crossbow disconnects its user | adapted upstream PR #3897 | Not yet |
| [#3847](https://github.com/Pumpkin-MC/Pumpkin/issues/3847) Books duplicate when inserted into chiseled bookshelves | vanilla hand changes persist before consuming block returns; direct inventory replacements take precedence (overlaps upstream PR #3849) | Not yet |
| [#3571](https://github.com/Pumpkin-MC/Pumpkin/issues/3571) Written books lose content | raw readers already merged; saved titles and writable pages now use string codecs | Not yet |
| [#3272](https://github.com/Pumpkin-MC/Pumpkin/issues/3272) Duplicate entity UUIDs | existing spawn reservation reused by loads and commands; each rejection logs once and retains the original | Not yet |
| [#3777](https://github.com/Pumpkin-MC/Pumpkin/issues/3777), [#3319](https://github.com/Pumpkin-MC/Pumpkin/issues/3319), [#3065](https://github.com/Pumpkin-MC/Pumpkin/issues/3065) Malformed particle payloads disconnect clients | malformed 26.3 payloads rejected; eyeblossom, creaking, mooshroom and command senders supply typed options, adapting upstream #3079/#3509 | Not yet |
| [#3382](https://github.com/Pumpkin-MC/Pumpkin/issues/3382), [#3561](https://github.com/Pumpkin-MC/Pumpkin/issues/3561), [#1800](https://github.com/Pumpkin-MC/Pumpkin/issues/1800) Attacking dragons or vanished entities disconnects players | dragon parts resolve while tracked; atomically reserved IDs and silent missing-target returns adapt upstream #3407/#3661/#3583 | Not yet |
| [#3759](https://github.com/Pumpkin-MC/Pumpkin/issues/3759) Recursive functions overflow the native stack | iterative vanilla function queue with shared command and fork limits | Not yet |
| [#1985](https://github.com/Pumpkin-MC/Pumpkin/issues/1985) Effect commands crash | existing effect lifecycle fixes plus vanilla command arguments and instant durations | Not yet |
| [#3758](https://github.com/Pumpkin-MC/Pumpkin/issues/3758) Execute store rejects valid commands | score, bossbar, block, entity and storage result callbacks | Not yet |
| [#3779](https://github.com/Pumpkin-MC/Pumpkin/issues/3779) Setblock rejects block-state data | upstream PR #3827 approach with SNBT support | Not yet |
| [#3502](https://github.com/Pumpkin-MC/Pumpkin/issues/3502) Hardcore does not force Hard | startup difficulty override | Not yet |
| [#3538](https://github.com/Pumpkin-MC/Pumpkin/issues/3538) Gamemode feedback uses wrong variables | upstream PR #3642 | Not yet |
| [#3760](https://github.com/Pumpkin-MC/Pumpkin/issues/3760) Tick sprint prints wrong start message | upstream PR #3763 | Not yet |
| [#3874](https://github.com/Pumpkin-MC/Pumpkin/issues/3874) Fill cannot create snow golems | synchronous BlockInput placement through existing block callbacks | Not yet |
| [#3276](https://github.com/Pumpkin-MC/Pumpkin/issues/3276) Reload runs load functions while frozen | upstream PR #3394 approach | Not yet |
| [#3503](https://github.com/Pumpkin-MC/Pumpkin/issues/3503) Hardcore death cannot enter spectator mode | adapted upstream #3506, normal saved respawn followed by cancellable spectator transition | Not yet |
| [#67](https://github.com/Pumpkin-MC/Pumpkin/issues/67) Java crafter slot toggles never disable empty slots | validate the current Crafter menu packet and synchronize its nine disabled flags and powered property; the existing saved `disabled_slots` format is unchanged | Not yet |
| [#69](https://github.com/Pumpkin-MC/Pumpkin/issues/69) Shift-clicking loom output crafts only one of the available banners | refresh the Loom preview immediately after consuming inputs, before awarding crafted statistics and repeating the quick-move click | Not yet |

G10 implementation: Codex (GPT-6 Luna).
| [#3614](https://github.com/Pumpkin-MC/Pumpkin/issues/3614) Armor stand item interactions | adapted upstream #3684 with actual-hand writes, adjusted clicks, disabled slots and equipment-drop fixes | Not yet |

## Murgicraft issues addressed here

| Fork issue | Change | Checked in-game |
|:--|:--|:--|
| [#27](https://github.com/Optifineless/Pumpkin/issues/27) | Excludes Spectator players from the night-skip denominator and sleeper count while keeping Creative players active. Recounts the live roster and gamemodes each environment tick, mirroring vanilla 26.3 `SleepStatus.update/sleepersNeeded`. No public plugin API or saved-data changes. In-game verification: Not yet. | Not yet |
| [#20](https://github.com/Optifineless/Pumpkin/issues/20) | Axe, hoe and shovel transformations generate from the `blocks` array in the 26.3 asset wrapper; Java item-use-on applies stripping, copper scraping/unwaxing, tilling, rooted-dirt loot and path creation. Copper chest stage changes now update both connected block states while retaining both inventories; door transformations preserve both halves without writing above them. Shovels lose one durability when extinguishing a lit campfire in Survival, and none in Creative. Mirrors vanilla 26.3 `CopperChestBlock.updateShape`, `CampfireBlock` and `ShovelItem`. In-game verification: Not yet. | Not yet |
| [#70](https://github.com/Optifineless/Pumpkin/issues/70) Ender pearl teleports keep old fall distance | Vanilla 26.3 `ThrownEnderpearl.onHit`: accepted teleports clear fall distance and mace impulse context before five damage and teleport sound. Cross-world transfers preserve health, register hurt feedback before damage, and send the destination position before its spatial sound; cancelled transfers preserve state and stay silent. | Not yet |
| [#21](https://github.com/Optifineless/Pumpkin/issues/21) Waxing copper slabs preserves top, bottom and double types and compatible waterlogging; shared waxed block conversion retains other matching state properties. Mirrors vanilla 26.3 `HoneycombItem.getWaxed` and `Block.withPropertiesOf`. | Not yet |
| [#159](https://github.com/Optifineless/Pumpkin/issues/159) Always Active command blocks evaluate their activation condition before the first scheduled impulse tick; conditional blocks still require a successful predecessor, and repeating blocks keep their current-condition snapshot and next-condition refresh ordering. Mirrors vanilla 26.3 CommandBlockEntity.scheduleTick/markConditionMet and CommandBlock.tick. The reporter's sustained Always Active repeating silence remains unconfirmed and is not established as fixed by this scoped repair. | Not yet |
| [#159](https://github.com/Optifineless/Pumpkin/issues/159) Always Active command blocks capture the predecessor predicate before the first scheduled activation and publish that state with the replacement block entity. New activations mirror vanilla 26.3 `CommandBlockEntity.setAutomatic/onModeSwitch/scheduleTick`, invoked in `ServerGamePacketListenerImpl.handleSetCommandBlock`; `CommandBlock.tick` consumes the captured predicate for this tick before refreshing the next repeating predicate. Unchanged automatic saves and switches away from repeating preserve any queued predicate without scheduling a new activation. The reporter's sustained Always Active repeating silence remains unconfirmed and is not established as fixed by this scoped repair. | Not yet |
| [#83](https://github.com/Optifineless/Pumpkin/issues/83) Target blocks keep their weak hit signal but do not strongly power adjacent solid blocks; [#84](https://github.com/Optifineless/Pumpkin/issues/84) arrows, spectral arrows and tridents pulse for 20 ticks, while other projectiles such as snowballs pulse for 8. A second hit does not extend the first scheduled clear time. Mirrors vanilla 26.3 `TargetBlock.ownSignal`, inherited `BlockBehaviour.getDirectSignal` and `TargetBlock.updateRedstoneOutput`. In-game verification: Not yet. | Not yet |
| [#37](https://github.com/Optifineless/Pumpkin/issues/37) Right-clicking a leash knot deadlocks interaction | Knot clicks release leash-holder locks before leash changes, transfer eligible leashes in either direction, defer empty-knot cleanup until after player packet handling, and preserve leads when tying existing leashes to fences. Also fixes the existing-knot downcast used by player-to-knot transfers. Mirrors vanilla 26.3 `LeashFenceKnotEntity.interact/getKnot/notifyLeasheeRemoved` and `LeadItem.bindPlayerMobs`; species-specific `canBeLeashed` eligibility is not yet ported. | Not yet |
| [#37](https://github.com/Optifineless/Pumpkin/issues/37) Right-clicking a leash knot deadlocks interaction | Knot clicks release leash-holder locks before leash changes, transfer eligible leashes in either direction, defer empty-knot cleanup until after player packet handling, and preserve leads when tying existing leashes to fences. Also applies species `canBeLeashed` rules before transfers, avoids publishing fence knots for ineligible leashes, treats cancelled knot shearing as handled without durability loss, and skips leash distance physics across worlds. Mirrors vanilla 26.3 `LeashFenceKnotEntity.interact/getKnot/notifyLeasheeRemoved`, `LeadItem.bindPlayerMobs`, `Leashable.canHaveALeashAttachedTo/canBeLeashed/tickLeash`, and `Mob.canBeLeashed`. | Not yet |
| [#63](https://github.com/Optifineless/Pumpkin/issues/63) Right-clicking a beacon cannot open its payment menu | Beacon interactions open a per-viewer menu for powered and unpowered beacons. The menu accepts only singleton items from `minecraft:beacon_payment_items`, validates effect selections, syncs levels and encoded effects, and returns unpaid payment on close, replacement or disconnect. Payment remains transient and is not added to beacon saves. Paid selections refresh the saved block-entity image after releasing the menu lock; effect pairs are committed and captured together, following vanilla 26.3 `BeaconMenu.updateEffects`. Native API 8 stays at the same version; rebuild native plugins for the added runtime fields. Implemented by Codex (GPT-6 Luna), with the PR #213 round 3 persistence fix by Codex (GPT-6.1 Sol). | Not yet |

PR #227 owner-review follow-up. No native plugin API or saved-data meaning changes. Bedrock axe transformation sounds and particles remain a backlog item because no existing Bedrock transformation mapping is available. In-game verification: **Not yet**.

Play test:
- Sneak-use an axe on one half of a stocked double copper chest; expect both halves to reach the same copper stage, both inventories to remain stocked, and a hopper to pull from either half. Capture both block states, the inventories, and the hopper transfer.
- Use a shovel on a lit campfire in Survival; expect it to go out and the shovel durability to drop by one. Repeat in Creative; expect no durability loss. An unlit campfire should stay unchanged. Capture the hotbar before and after.

| [#14](https://github.com/Optifineless/Pumpkin/issues/14) Lava bucket fuel remainder | Fuel consumption uses the item's crafting remainder independently of stack use-remainder overrides; lava returns a bucket and overflow drops once. The shared mapping preserves all five vanilla 26.3 item templates (count one, default components). Mirrors vanilla 26.3 `AbstractFurnaceBlockEntity.consumeFuel`. | Not yet |
| [#15](https://github.com/Optifineless/Pumpkin/issues/15) Smoker and blast furnace cooking speed | The actual appliance recipe kind is used throughout; the admitted fuel's resolved `CookingFuel` speed scales cook duration, preserves partial completion across fuel changes, and is reused after crafting. With coal, smokers and blast furnaces cook eight admitted items per fuel; normal furnaces retain speed 1. Saved speed uses vanilla `speed_multiplier`, defaulting to 1.0 when absent. Mirrors vanilla 26.3 `AbstractFurnaceBlockEntity.serverTick/getTotalCookTime/loadAdditional/saveAdditional`. | Not yet |

In-game verification: Not yet.

Play test (#14/#15): smelt raw iron with a lava bucket; expect one empty bucket
in the fuel slot. Cook eight potatoes in a smoker and eight iron ore in a blast
furnace using one coal each; expect all eight to finish. Save and restart midway
through a smoker's coal burn; expect the same cooking pace afterward. Capture
the fuel slot and cooking before and after restart.

## Known regressions under investigation

Offline Java identities now follow `UUIDUtil.createOfflinePlayerUUID`: MD5 of UTF-8
`OfflinePlayer:<case-sensitive name>`, with UUID version 3 and RFC variant bits. This
matches vanilla/Paper offline worlds. Earlier Pumpkin identities used the first
16 SHA-256 bytes of the bare name, without changing the version or variant bits.
Existing fork offline worlds therefore need an administrator migration before use.
Stop the server and back up the world, player data, advancements, statistics,
`usercache.json`, whitelist, bans and operator files. For each exact saved name,
map the old UUID to the new UUID using:

```python
import hashlib, uuid
name = "Steve"  # exact capitalization from usercache.json
old = uuid.UUID(bytes=hashlib.sha256(name.encode("utf-8")).digest()[:16])
new = uuid.UUID(bytes=hashlib.md5(("OfflinePlayer:" + name).encode("utf-8")).digest(), version=3)
print(old, new)
```

Rename UUID-keyed player-data, advancement and statistic files, update the UUID
fields in the cache/admission/operator files, and update UUID references in saved
NBT (including player UUID and ownership of tamed animals) with an NBT editor.
Do not overwrite a destination file; resolve existing identities from the backup
first. Verify inventory, location, advancements, pets and permissions on a copy
of the world before starting the public server. Never apply this map to online
Java accounts. In-game migration verification: **Not yet**.

Bedrock authenticated XUID identities retain Pumpkin's `pocket-auth-1-xuid:`
namespace. Offline and self-signed Bedrock identities now use the separate
`OfflineBedrockPlayer:<display name>` MD5/version-3 namespace. Login claims no
longer use `leguuid` for either authentication mode. Previously linked authenticated
Bedrock profiles need an ownership-verified migration to their XUID-derived UUID.
Migrate old weak Bedrock
identities only after verifying the player's ownership, using the same backup
and UUID-reference procedure. Cross-edition linking requires a separately verified
linking mechanism and is not provided by login claims. BungeeCord forwarding now
requires a configured BungeeGuard secret on the proxy and backend.
Native plugins must be rebuilt against plugin API version 8 (see Native plugin API below). The Wasm API is unchanged.

Before upgrading an existing offline-mode world, run the UUID migration documented
above and verify the migrated copy, including inventories and permissions.

Bedrock online mode over RakNet is not yet cryptographically bound to the client's
`cpk`: the `ServerToClientHandshake` key exchange is missing. Do not grant operator
to Bedrock accounts on a public server until that exchange is implemented.

| Report | Status |
|:--|:--|
| Chest and hopper contents vanished after `stop` and restart (2026-10-07, first start after switching from the pre-harvest build) | Not reproduced in three headless investigations (same build, player-style unload, old-build world opened by new build); container NBT format verified unchanged across the harvest; the owner's later tests (hand-filled chest, autosave, restart with a build switch) kept their items. Kept open; report any recurrence with the build and steps. |
| [#2016](https://github.com/Pumpkin-MC/Pumpkin/issues/2016) Sapling fills crash | Oversized fills are rejected using the generated block limit and overflow-safe volume. Allowed-size fills still need the world neighbour queue from upstream PR #3924, outside the command lane's ownership. |
| Effect command success counts for instant effects | The entity API returns no admission result, so the command cannot observe an instant effect rejected by a plugin or boss immunity. Duration and argument fixes do not resolve that entity API limitation. |

## Fork fixes checked in-game

Fixes with no upstream issue number, and what the owner saw when testing them.

| Change | Checked in-game |
|:--|:--|
| Destroyed mounts eject surviving passengers before removal so each rider becomes a saved root and survives chunk reload; chunk unload still serializes the complete riding tree before detaching it. Older fork and vanilla saves keep the same passenger-tree NBT and still load. Mirrors `Entity.remove` / `Entity.setRemoved` and `Entity.ejectPassengers` (audit row 5) | Not yet |
| Logout marks only the departing player instance removed, so target goals drop that stale target and mobs stop attacking its old location. Mirrors `PlayerList.remove` and `TargetGoal.canContinueToUse` (R31, audit row 7) | Not yet |
| Respawn removal and cross-world publication use the exact player instance, so a late task cannot remove or republish a rejoined session. Rejected destination admission preserves source membership and chunk streaming without rollback publication. Accepted transfers commit the saved dimension and position together before the respawn hook, including disconnect during the hook. Hardcore respawn revalidates the exact active session after game-mode callbacks before either tracker publication, so completed callback removal cannot resurrect a player (PR #202 round 2, written by Codex). Mirrors `PlayerList.remove`, `PlayerList.respawn` and `ServerGamePacketListenerImpl.handleClientCommand` (audit row 15; PR #202 owner review). Native API 8 and saved-data meanings stay unchanged. | Not yet |
| Respawn releases the old riding links before publishing the reset player, and entering Spectator dismounts immediately so position packets move the player. Like Vanilla `ServerPlayer.die`, death processing keeps the riding link until `PlayerList.respawn` removes the old entity. Mirrors `PlayerList.respawn`, `Entity.setRemoved` / `Entity.unRide`, and `ServerPlayer.setGameMode` / `Entity.stopRiding` (audit row 16) | Not yet |
| Scheduled block and fluid ticks use elapsed game time while chunks are outside simulation distance; overdue ticks run when simulation resumes, with a 65,536 budget per type, clean chunks resave elapsed or drained tick lists, both schedulers retire before public chunk removal without allocating empty wheels, late producers append to the frozen image and newer mutations retry publication, and completed unload/save/reload preserves relative delays until first simulation. Pending reload copies run off the Tokio worker, clock-only progress permits a finite save to finish while queue mutations still retry, block/biome/light edits version saved content independently of clock dirtiness, and both saved tick lists share one game-time snapshot (PR #147; PR #136 review) | Not yet |
| [#110] Charged respawn anchors set a Nether spawn or explode in unsafe dimensions when used without glowstone | Not yet; positional `RESPAWN_ANCHOR_WORKS` environment-attribute overrides are not evaluated |
| Bedrock block use and placement consume from the packet's hand once, with off-hand owner slots, observer equipment and modifiers synchronized after transaction actions; mirrors `ServerPlayerGameMode.useItemOn` and `LivingEntity.detectEquipmentUpdates` | Not yet |
| Bedrock default block interactions use only the main hand, preserving distinct held items and off-hand placement components; mirrors `ServerPlayerGameMode.useItemOn` | Not yet |
| Bedrock block and air use reject forged held-item types before transaction actions or gameplay callbacks, correcting the originating hand; mirrors `ServerGamePacketListenerImpl.handleUseItemOn/handleUseItem` | Not yet |
| [#141] Bedrock block and air use treat originating-hand predictions as advisory, preserving components and consuming only the server result. Callback hotbar changes skip stale item use, and final equipment publication rechecks the live hands before the next attack; mirrors `ServerPlayerGameMode.useItem/useItemOn`, `EggItem.use` and `LivingEntity.detectEquipmentUpdates` | Not yet |
| [#141] Java and Bedrock placements claim the pinned source after cancellable hooks, reject moved/replaced stock before installation, and refund a failed conditional world write; mirrors `BlockItem.place/placeBlock/updateBlockEntityComponents` | Not yet |
| [#109] Firework Stars craft with colors, shapes and effects, can receive fade colors, and can be used in rockets | Not yet |
| Powered activator rails eject rideable minecart passengers without freezing the tick; normal dismount events, cooldown and repositioning apply; mirrors `Minecart.activateMinecart` -> `Entity.ejectPassengers` ([fork #38](https://github.com/Optifineless/Pumpkin/issues/38); regression audit item 1) | Not yet |
| Creative inventory packets arriving after a switch to Survival are ignored instead of disconnecting the player; mirrors `ServerGamePacketListenerImpl.handleSetCreativeModeSlot` (regression audit item 6) | Not yet |
| Hoppers keep their facing after a restart | Yes, 2026-10-07 |
| Minecarts limit explosion-driven movement using vanilla old and experimental speed limits, with water and furnace factors; off-rail carts halve motion before moving on the ground, and fluid flags refresh after movement. Rider input only nudges carts below 0.1 blocks/tick; displacement clamps preserve collision momentum. The speed command accepts 1-1000, and invalid saved values are bounded at use. Experimental powered-rail starts require a conducting end block. Mirrors `OldMinecartBehavior.moveAlongTrack/getMaxSpeed`, `NewMinecartBehavior.calculateTrackSpeed/calculateBoostTrackSpeed/getMaxSpeed`, `AbstractMinecart.getRedstoneDirection/tick/comeOffTrack/pushOtherMinecart`, `GameRules.MAX_MINECART_SPEED` and `Entity.move` (R37, fork #139). Temporary experimental limitation: rail speeds above the default remain capped at the default until `NewMinecartBehavior.stepAlongTrack` is ported; off-rail rule limits still apply. Deliberate safety divergence: vanilla has no generic movement cap; collision queries here reject non-finite motion, more than 128 blocks on any axis, or more than 262,144 additional swept block cells excluding the entity's own box, and clear the rejected impulse. Fireworks do not restore rejected impulses. | Not yet |
| Player saves clear legacy game-mode `Invulnerable:1b` while persisting explicit plugin entity invulnerability; loaded abilities are re-derived for the saved game mode, fire ignition applies the one-tick ability clamp, and invulnerable mobs keep their protection. Mirrors `Entity.load/saveWithoutId`, `ServerPlayer.readAdditionalSaveData`, `GameType.updatePlayerAbilities`, `Player.hurtServer`, `BaseFireBlock.fireIgnite` and `Player.setRemainingFireTicks` ([#130](https://github.com/Optifineless/Pumpkin/issues/130)) | Not yet |
| Village ground uses biomes at the actual structure position, even across chunk borders; terrain shaping includes nearby chunks within vanilla's expanded bounds. Regression fixtures compare 24,064 surface columns (height and top two blocks) and 144,384 biome cells. Coverage stops before paths, buildings and decoration and does not check underground ancient-city shaping. The biome registry separately matches 67 captured vanilla network entries; this does not establish a colouring fix. | Not yet |
| Redstone torches burn out when toggled too fast | Yes, 2026-10-07 |
| Sticky pistons pull back in every direction | Yes, 2026-10-07 |
| Pistons tell the client which block is moving, so the animation shows it | Yes, 2026-10-07 |
| Arrows, tridents, splash potions, evoker fangs and shulker bullets can hit players | Partly: skeleton arrows confirmed 2026-10-07 |
| Arrows and tridents follow vanilla flight timing; bow trails, shooter movement and off-hand use are corrected | Not yet |
| Land mobs bob at the surface instead of being pushed up out of the water | Yes, 2026-10-07 |
| Zombies sink in water like vanilla instead of floating | Yes, 2026-10-07 |
| Surface monsters use nighttime sky darkening; livestock and leashed or riding mobs keep vanilla distance persistence | Not yet |
| Distant idle mobs can despawn randomly again, with vanilla light additions and damage or crossbow resets | Not yet |
| Mob equipment is finalized before spawning and restored without re-rolling on reload | Not yet |
| Villagers, golems, farm animals and bucket-released aquatic mobs keep their species' distance persistence | Not yet |
| Generated creatures read the generation chunk and enter the world when their chunk is published | Not yet |
| Parched skeletons receive bows; skeleton weapons choose bow or melee attacks with vanilla intervals | Not yet |
| Spawners preserve configured entities and weighted potentials, and enforce their nearby-mob limit on each attempt | Not yet |
| Generation spawning uses region light at night and selects variants before saving new mobs; farm-animal babies inherit a parent's variant | Not yet |
| Slimes and magma cubes finalize size with local difficulty and load size before health | Not yet |
| Trial spawners enforce custom light limits and keep configured equipment drop chances | Not yet |
| Chicken jockey flags survive reload and suppress eggs; drowned can spawn on zombie nautiluses | Not yet |
| Generated structure starts and references supply runtime mob spawn overrides | Not yet |
| Natural jockey mounts are admitted together; ordinary unload and restart keep riders and equipment | Not yet |
| Structure births keep cancellable plugin events; trial-spawner callbacks can read NBT without freezing the tick | Not yet |
| Scheduled ticks run on time and survive a restart | Not yet |
| Dust removed by water or explosions updates neighbours | Not yet |
| Incoming damage follows difficulty, PvP and team rules; callbacks release combat ownership; cooldown excess, shield responses and melee/spear motion and statistics follow vanilla; instant healing and harming invert for undead mobs (combat task 1, related upstream PRs #3637, #3544 and #3525) | Not yet |
| Shields respect piercing shots, cooldowns and hand changes; death protectors use their configured effects (combat task 2 review follow-up, related to #3520) | Not yet |
| Disconnect counts games quit once and keeps the saved statistic consistent with plugin changes and the scoreboard | Not yet |
| Projectile and TNT owners persist by UUID; explosions, fireworks, splash potions and lingering clouds follow 26.3 damage and timing rules (combat task 4) | Not yet |
| Hand use consumes jukebox discs, compost and snow layers; off-hand buckets, milk, books, signs, pots, honey and consumable remainders keep their source hand and success rules (survival audit 1, 4, 16–18, 42, 44–46, 51–52, 55; overlaps upstream #3849) | Not yet |
| Villagers remember nearby/summoned golems for 600 ticks, require recent sleep, and place summons on clear collider tops; killed creepers stop their fuse and living creepers defuse when targets die (volunteer play-test bugs 2) | Not yet |
| Crafting Q/right-click keeps the whole output; exhausted crafter slots produce no remainders; mined shulker menus close; styled keys and processed maps survive saves (survival task 1 deep review 1–5) | Not yet |
| Cartography drags/clones and repeated shift-clicks, bounded drag selection, occupied smithing input routing and occupied-hive bundle weight (survival task 1 deep review 6–10) | Not yet |
| Ender Dragon fight spawning, portal/gateway placement, pillar regeneration, explosions, removals and boss-bar/respawn packet callbacks run after the fight lock is released. Crystal destruction cannot slip through final completion checks and is reconciled after spawn callbacks, so destroying a respawn crystal aborts without creating a dragon. Re-entrant spawn callbacks create only one dragon; rejected respawns finish and consume their crystals; cleanup survives a death callback during publication; cancelled ordinary spawns retry after the vanilla recovery interval; repeated kill callbacks cannot duplicate rewards or restore the dead dragon's boss bar. Untracked deaths cannot consume fight rewards; ordinary spawn identity is published after spawn events and before admission packets, preserving a publication death and its rewards. Stationary respawn spectators regain the boss bar, and aborted queued pillar regeneration is discarded. Mirrors `EnderDragonFight.createNewDragon`, `EnderDragonFight.setDragonKilled`, `EnderDragonFight.tryRespawn`, `EnderDragonFight.respawnDragon`, `EnderDragonFight.setRespawnStage`, `EnderDragonFight.onCrystalDestroyed`, `EnderDragonFight.tick`; `DragonRespawnStage.SUMMONING_DRAGON.tick` and `DragonRespawnStage.SUMMONING_PILLARS.tick`; `EndCrystal.hurtServer`; `ServerBossEvent.addPlayer` and `ServerBossEvent.setVisible`. (Singapore hang 2026-10-10, PR #122 follow-up, PR #146 review). | Not yet |
| Stale teleport confirmations (pearl into a remote End gateway, back-to-back /tp) are ignored like vanilla instead of kicking with "Wrong teleport id"; repeated confirmations of the latest accepted ID disconnect, matching `ServerGamePacketListenerImpl.handleAcceptTeleportPacket` (tester R20, PR #123) | Not yet |
| Ender Dragon respawn explosions run after the fight lock is released, and crystals destroyed while it is held are queued, so the respawn sequence no longer freezes the server (Singapore hang 2026-10-10, PR #122) | Not yet |
| Plugin-written sign text also replaces the filtered text, styled plain-text signs stay editable, and meadow tall grass generates as whole plants (PR #103 review follow-up) | Not yet |
| `/clear` updates worn armor and held items for Java and Bedrock observers and the owning inventory, and removes cleared item attributes (issue #98). Equipment refresh reads current slots on the player tick, including concurrent slot switches and replacement armor; attribute updates reach only the owner and tracking players. Mirrors `ClearInventoryCommands.clearInventory`, `LivingEntity.detectEquipmentUpdates/collectEquipmentChanges`, `ServerEntity.sendDirtyEntityData` and `PlayerEquipment.get/set`; Wasm main-hand equipment uses the selected slot, API 8 (same version). Bedrock owner armor/offhand corrections are also tested with a tracking observer. Known limitation, tracked as the equipment-freshness PR: switching from a sword to an empty hand and attacking in the same inbound batch can use the old item's damage and attack-speed modifiers for one tick; cleared armor can also protect until the publisher runs. Vanilla `Player.attack/getAttackStrengthScale` reads the live held item and its attack attributes; `LivingEntity.getDamageAfterArmorAbsorb` reads current armor attributes. | Not yet |

- R2-01: syntax-error context budgets ten UTF-16 units on safe UTF-8 boundaries, including cursors inside a character. In-game verification: Not yet.
- R2-02: implicit command targets follow the executing player through `execute as`; command positions follow the execution context, while permissions and feedback stay with the sender. In-game verification: Not yet.
- R2-11: regression coverage protects the existing function frames for failed, nested, redirected and `return run` returns. In-game verification: Not yet.
- R2-12: disabled commands remain unavailable through root redirects, repeated `execute ... run` chains, function bodies, cached parsing and command suggestions. In-game verification: Not yet.
- R2-13: natural weather and command durations advance when `advance_weather` is enabled; disabling it freezes timers while visual transitions continue. In-game verification: Not yet.

- Command review follow-up 1–3, 12: authoritative weather flags and all timers are shared across dimensions and round-trip through `weather.dat`; only weather-bearing dimensions advance/interpolate; tick sends ordered transitions and sleep uses visual rain before advancing time. In-game verification: Not yet.
- Command review follow-up 4–6, 10: local coordinates use the context position and numeric `positioned` resets the anchor; `execute in` scales X/Z; all rotation modifiers use pitch then yaw; entity UUID selectors include players without relaxing player-only arguments. In-game verification: Not yet.
- Command review follow-up 7–9, 13: top-level `return run` keeps first-result semantics, waypoint icon changes persist on the transmitter, signed and unsigned private messages use the executing player with output fallback and silence, and Unicode context preserves the styled HERE marker. In-game verification: Not yet.
- Opus command verification: normal shutdown saves live weather, player command sources use pitch then yaw, outgoing chat fallback uses its bound chat type, and top-level `return` discards remaining forked sources. In-game verification: Not yet.
- Command PR #100 review: waypoint modifications refresh the connected command sender with vanilla's untrack/track pair and 26.3 packet format; saved icons live in a living-entity component; unavailable redirects report the child start; top-level return discards pending callbacks. In-game verification: Not yet.
- Waypoint divergence: no tracking lifecycle is implemented yet. Icon commands persist entity data and refresh only the connected command sender, matching the original command's audience; console commands have no client audience. A command removes the sender's old marker before rechecking range, but movement updates and automatic range/death removal still await a server waypoint manager.
- Unicode context divergence: when Java's ten-unit UTF-16 window starts inside a surrogate pair, Pumpkin omits that entire character to preserve valid UTF-8 (`abcdefghi😀123456789` shows `...123456789`), instead of Java's isolated low surrogate.
- Deferred function gap: `execute if|unless function`, `/function ... with`, macro instantiation not implemented.

## Small world shared-state fixes

| Issue | In-game verification |
|:--|:--|
| Concurrent lazy chest opens share one inventory, so deposits survive unload and reload (shared-state finding 2). | Not yet |
| Scheduling and loaded-container registration cannot lose block or fluid ticks during collector removal (audit row 11). | Not yet |
| Entity tick callbacks can force chunks without deadlocking active-chunk membership (audit row 22). | Not yet |
| Metadata writes during a tracker flush remain dirty for the next flush (shared-state finding 3). | Not yet |
| A lazy chest load cannot resurrect a removed block entity or publish an invalidated chunk snapshot (PR #148 review). | Not yet |
| Losing lazy chest loads preserve newer plugin custom data, including after unload and reload (PR #148 review). | Not yet |
| Overlapping metadata flushes and refreshes enqueue older changes before newer changes, and an unwound sender retains queued refreshes for an ordinary tracker retry (PR #148 review). | Not yet |

## Native plugin API 8

- API 8 (same version): block-item use adds `ItemBehaviour::use_on_block_with_hand` and `ItemRegistry::use_on_block_with_hand` so item callbacks receive the packet hand; the default trait method preserves existing callback behavior, while the trait vtable changes. Rebuild native plugins against this checkout; Wasm WIT is unchanged. In-game verification: Not yet.

- API 8 (same version): issues #14 and #15 add private speed state to furnace block entity layouts and methods to the public `CookingBlockEntityBase` trait/vtable. Native plugins using these types or trait objects must rebuild against this checkout; saved speed uses vanilla `speed_multiplier` and defaults to 1.0 for old saves. The exported cooking macros remain usable from external crates through a public, documentation-hidden fuel helper and the shared item crafting-remainder resolver. Wasm WIT is unchanged. In-game verification: Not yet.

- API 8 (same version): #141 adds `PlayerInventory::replace_stack_if_unchanged` for identity-checked hand writeback. Rebuild native plugins against this checkout; saved data and Wasm WIT are unchanged. In-game verification **Not yet**.

- API 8 (same version): the tick watchdog changes `Server` layout through its crate-private `watchdog` field and adds `Server::start_watchdog`. Rebuild native plugins; Wasm WIT is unchanged. In-game verification **Not yet**.
- API 8 (same version): raw scheduled-tick index mutation is replaced by `Level` registration/clearing methods, and metadata clearing acknowledges a captured `DirtyBatch`; native plugins built against this fork must be rebuilt. In-game verification **Not yet**.
- API 8 (same version): PR #148 follow-up adds private lazy-load invalidation, metadata-sender state and reusable tick buffers to `World`, `Entity` and `Level`. Overlapping metadata flushes/refreshes defer to the active sender. Rebuild native plugins; saved NBT and Wasm WIT are unchanged. In-game verification **Not yet**.
- API 8 (same version): the clock changes add saved fields and the serde-skipped `dimension_clocks` snapshot map to `LevelData`, a metadata-save mutex to `Server`, and fixed default-root clock ownership to `World`. The obsolete `write_random_sequences_stub` helper is removed; use the live `RandomSequences::save` path. The synchronous `data_files::write_saved_data` helper requires an existing parent and serialized callers. Rebuild native plugins against this checkout. Wasm WIT is unchanged.
- API 8 (same version): the clock changes add saved fields and the serde-skipped `dimension_clocks` snapshot map to `LevelData`, a metadata-save mutex to `Server`, and fixed default-root clock ownership to `World`, and the generated optional `Dimension::default_clock` field. The obsolete `write_random_sequences_stub` helper is removed; use the live `RandomSequences::save` path. The synchronous `data_files::write_saved_data` helper requires an existing parent and serialized callers. Rebuild native plugins against this checkout. Wasm WIT is unchanged.
- API 8 (same version): the clock changes add saved fields and the serde-skipped `dimension_clocks` snapshot map to `LevelData`, a metadata-save mutex and per-root clock retention to `Server`, and fixed default-root clock ownership plus cached physical root identity to `World`, and the generated optional `Dimension::default_clock` field. The obsolete `write_random_sequences_stub` helper is removed; use the live `RandomSequences::save` path. The synchronous `data_files::write_saved_data` helper requires an existing parent and serialized callers. Rebuild native plugins against this checkout. Wasm WIT is unchanged.
- API 8 (same version): dragon-fight callback dispatch, crystal notifications and pending dragon creation change the `DragonFight` and `World` layouts. Native plugins must be rebuilt against this checkout; Wasm WIT is unchanged.

- API 8 (same version): scheduled ticks now use world game time. `ChunkTickScheduler::step_tick` takes the current game tick; `to_vec` returns signed `SavedTick` delays, and tick NBT codecs live on `SavedTick`. `Level::set_game_time` publishes the clock after time advances; `Level`'s existing registration owner safely refreshes empty chunk markers. `Level` and `ChunkTickScheduler` layouts changed; scheduler save tracking now accounts for elapsed delays and queue mutations, and world collection retains backlog beyond the per-type tick budget. Saved delays remain pending until first simulation; publication only binds the clock. Rebuild native plugins; Wasm WIT is unchanged. In-game verification **Not yet**.
- API 8 (same version): scheduled-tick collection mirrors `LevelTicks.drainFromCurrentContainer` and the `nextTickForContainer` cache, batching due runs and skipping future containers without changing saved delays or budgets. The #147 follow-up uses the fork lock classes (index 20, scheduler 30, save observation 40) and sorts prepared DRAIN_ORDER entries once for constant-time `LevelChunkTicks.poll` moves. In-game verification **Not yet**.
- API 8 (same version): the #147 integration uses an instrumented chunk-ticket registry (level 20). Terrain retirement checks entity retention, tickets and the exact published chunk under one admission hold before freezing both schedulers and removing public ownership. Native plugins must rebuild; in-game verification **Not yet**.
- API 8 (same version), PR #147 collection invariants: clearing a prepared head reorders it before consumption; unloaded tick containers reject stale collectors; pending-cache reload creates fresh schedulers; world loading publishes restored game time before initial scheduling. No new lock, atomic, queue, lease or generation counter. Codex GPT-6 implemented this follow-up. In-game verification **Not yet**.
- API 8 (same version), PR #147 round 5c: retirement time lives beside the optional wheel under the existing scheduler owner. Empty retirement allocates no wheel; late schedules append relative to the original unload clock and invalidate an in-flight save through existing mutation observation. A retained Arc scheduling after the final publication mutation check (including after acknowledgement) can still lose work; it must use loaded Level scheduling instead. Vanilla `LevelTicks.schedule` requires a registered container and logs an absent one; callers that require an unloaded chunk must load it first. Rebuild native plugins for the scheduler layout change; Wasm WIT is unchanged. Codex GPT-6 implemented this follow-up. In-game verification **Not yet**.
- API 8 (same version), PR #147 round 5b: `Dirtiable::clear_published` now returns whether the published version was acknowledged; native implementations must return false when newer mutations remain. Pending detach/decode uses the existing blocking facility. Encoded tick images retain their clock observation, so clock progress requests a later save without invalidating the current publication. Codex GPT-6 implemented this follow-up. In-game verification **Not yet**.
- API 8 (same version), [#7](https://github.com/Optifineless/Pumpkin/issues/7): candle stacks now use ordinary item placement, consuming one candle in survival while preserving the lit state and four-candle limit. In-game verification **Not yet**.
- API 8 (same version), [#8](https://github.com/Optifineless/Pumpkin/issues/8): sugar cane accepts tagged adjacent water fluids, including waterlogged blocks, or tagged frosted-ice support. In-game verification **Not yet**.
- API 8 (same version), [#9](https://github.com/Optifineless/Pumpkin/issues/9): newly grown sugar-cane segments start at age zero instead of inheriting the mature source age. In-game verification **Not yet**.
- API 8 (same version), [#55](https://github.com/Optifineless/Pumpkin/issues/55): sea-pickle bonemeal uses the ordinary item lifecycle, consumes one bone meal, and preserves vanilla coral and water eligibility. In-game verification **Not yet**.
- API 8 (same version), [#56](https://github.com/Optifineless/Pumpkin/issues/56): sea pickles accept a nonempty upper collision face or sturdy upper face during placement and neighbor updates, while surviving waterlogged pickles continue scheduling water ticks. In-game verification **Not yet**.
- API 8 (same version), [#57](https://github.com/Optifineless/Pumpkin/issues/57): an empty-hand upper-half click extinguishes a lit candle cake without eating it; lower clicks retain cake eating. Extinguishing uses the shared candle state, sound and block-change event path, without server particle packets. In-game verification **Not yet**.
- API 8 (same version), [#58](https://github.com/Optifineless/Pumpkin/issues/58): farmland crops require raw brightness 8 to survive and crops and gourd stems require raw brightness 9 to grow; nether wart remains light-independent. In-game verification **Not yet**.
- API 8 (same version): falling hazards add `Entity` permanent-invulnerability state, change `FallingEntity` layout, and add `OnLandedUponArgs.position`. Additive helpers: `block::push_entities_up` (call before the state write; returns the new state), `FallingEntity::set_hurts_entities`, and `LivingEntity::handle_fall_damage_from`. Rebuild native plugins; Wasm WIT is unchanged.
- API 8 (same version): unsupported kelp and growing-vine columns now break one segment per scheduled tick and produce their normal drops. In-game verification **Not yet**.
- API 8 (same version): lily pads use water fluid state or the lily-pad support block tag and ignore empty-fluid blocks above. In-game verification **Not yet**.
- API 8 (same version): mining a turtle-egg cluster removes one egg while preserving the remaining cluster. In-game verification **Not yet**.
- API 8 (same version): sniffer eggs rely exclusively on their loot table, avoiding duplicate survival drops and creative drops. In-game verification **Not yet**.
- API 8 (same version): shelf mushrooms attach to the sturdy face behind their outward-facing cap. In-game verification **Not yet**.
- API 8 (same version): dried ghasts face the placer, accept water, hydrate or dry in 5000-tick steps, and hatch a baby happy ghast facing the block direction. Scheduled delays are capped at the signed-int save range, and equal-priority tick order survives saves across short and long queues. `OrderedTick.sub_tick_order` and `ChunkTickScheduler::schedule_tick` use signed orders so loaded ticks precede fresh ticks. In-game verification **Not yet**.
- API 8 (same version): unsupported snow and any non-air block whose placement resolves to air are rejected before placement side effects. In-game verification **Not yet**.
- API 8 (same version): adding a glow-lichen, sculk-vein or resin-clump face uses ordinary item placement, consuming one item outside Creative and refusing occupied faces without redirecting to a neighbour while any face is vacant. Attachments use the actual support state and face, including top and double slabs. In-game verification **Not yet**.

- API 8 (same version): the shared text decoder retains translated names with fallback and typed numeric arguments in `TextContent::Translatable`. Item names and entity reload use the same lossless decoder; server text uses vanilla fallback precedence and substitutions. Rebuild native plugins for the enum layout. Translated-name anvil repairs, entity reload and equivalent lock encodings: in-game verification **Not yet**.

- API 8 (same version): survival task 1 second review adds opaque text contents, vanilla custom-name equality, dye recipe metadata, a menu tick hook, a disk-backed map cache and MapIndex allocation; map decoration names retain components. Rebuild native plugins for the changed enum, vtable and layouts. Map imports/restarts, whole cartography/stonecutter results, named map markers, hive occupant transfer and transmute/dye displays: in-game verification **Not yet**.

Rebuild native plugins against this checkout. The Wasm WIT is unchanged throughout.

- API 8 (same version): `VillagerEntity` changes layout with sensor timing and a group reservation lock, and its Brain registers sleep, recent-golem and interaction memories. `SpawnStrategy` changes from a single-variant enum to a discriminated enum with `OnTopOfColliderNoLeaves` (update exhaustive matches). Rebuild native plugins; Wasm is unchanged.
- API 8 (same version): `ShulkerBoxScreenHandler` adds a validity callback for its original world/entity and distance; `ScreenHandlerBehaviour` stores drag state and unique destinations; `BeesImpl` now holds occupant entity data and tick counts (`Eq`/`Hash` removed); `MapData` keeps saved tracking/marker fields and `MapManager` adds a save gate and `load`/async `save`. Rebuild native plugins. Map files use 26.3's `data/minecraft/maps/<id>.dat`, accepting legacy `data/map_<id>.dat` on load. Existing world storage replacement and parent-sync helpers are now exported for map saves. The second-review follow-up adds the previously deferred recipe displays, marker reconstruction and MapIndex migration (`util/filefix/fixes/DimensionStorageFileFix.java:54-55`); broader historical DataFixer conversion remains unimplemented.

- API 4: shield and totem integration changed native trait/component layouts. `EntityDamageByEntityEvent.damager_id` now identifies the direct projectile for projectile hits; it previously identified the shooter. Resolve the projectile's owner for player attribution. Mob damage hooks carry separate direct and causing entities.
- API 5: the crash-codec audit changes `InstrumentImpl` from a unit component to registered or inline instrument data.
- API 6: durable storage. `ChunkData.dirty` and `ChunkEntityData.dirty` are `DirtyFlag` instead of `AtomicBool` (carried by the chunk load, save and send events); `ChunkSections.randomly_ticking_mask` is `RandomTickMembership`; `Server` gains `tick_gate` and a session lock and `Player` gains `storage_session` (layout changes); `Server::add_player` is async; `Level::shutdown`, `get_entity_chunk`, `get_or_fetch_chunk` and `get_or_fetch_entity_chunk` return `Result`. Mob movement (same version) changes `MobEntity.look_control` to `Box<dyn LookControlTrait>`, adds public movement fields, and changes `PathNavigationTrait::tick` to take the owning mob and its collision context.
- API 6 (same version): command robustness adds methods to `CommandSource`, `ReturnValueCallable` and both `CommandExecutor` traits, the public `RedirectModifier::CustomUncharged` variant (update exhaustive matches), a `FunctionRunError::EmptyTag` variant and a new `DatapackManager` field, removes `MAX_FUNCTION_CHAIN_DEPTH`/`MAX_FUNCTION_CHAIN_COMMANDS`, and `reload_datapacks` no longer runs `#minecraft:load` synchronously. `CommandDispatcher::execute` installs a command quota context; a plugin's reentrant `handle_command` joins the caller's quota. `/function` and nested `execute_function` calls return 0 when scheduled; `/function` sends "Running function..." feedback before execution and reports an explicit function return later, including its result callbacks.
- API 7: harvest integration changes mob death hooks, bucket data, item lifetime fields, chunk repair fields, teleport outcomes and the occupancy-free inventory predicate (retained from the fork head).
- API 8 (same version): command review follow-up adds server-wide `Server.weather_data`, `Weather::data()` snapshots (replacing per-world public flags/timers), saved weather fields in `LevelData`, a top-level execution frame and persistent living-entity waypoint icons. `World::is_raining`/`is_thundering` report visible weather, so `is_raining` becomes true about 21 ticks after `set_raining(true)` from clear weather; thunder also waits for its visual threshold. `CommandSource::with_world` scales X/Z by the dimension coordinate-scale ratio and preserves `silent`. Command sources gain an execution-stop query to discard remaining sources after a return. Native plugins must rebuild against this checkout; Wasm is unchanged.
- API 8: XP orbs. `Player.experience_pick_up_delay` is now an `AtomicU32`, replacing `Mutex<u32>`, and `EnchantmentHelper::modify_durability_to_repair_from_xp` takes and returns `i32`. `World` gains a temporary orb snapshot (layout change). Additive helpers: `ExperienceOrbEntity::{award, award_with_direction, spawn_single, new_empty, get_value}`, `collect_nearby_orbs`, `Entity::move_towards_closest_space`, `EnchantmentHelper::get_random_item_with_repair_effect`, and `furnace_experience::recipe_experience`. `InventoryPlayer::award_experience` now drops collectible orbs at the player for furnace output. Native plugins must be rebuilt; Wasm is unaffected.
- API 8 (same version): projectile-state and explosion hooks change `EntityBase` vtables; `ExplodeArgs` carries the explosion context. Projectile ownership, cloud and explosion layouts also change. Rebuild native plugins against this checkout; API 7 from the harvest branch is incompatible. Wasm WIT is unchanged.
- API 8 (same version): crafting results now carry component-preserving stacks and remainders; recipe templates retain material bounds and components; bundle contents retain transient selection, dyes retain their color, written books retain generation/resolution, and containers retain locks. InventoryPlayer exposes saved-map validation/post-processing, recipe synchronization and bundle sounds; Slot exposes result post-processing. Native plugins should rebuild. Survival inventory findings 2, 3, 5, 6, 7, 10, 13, 14, 31, 39, 40 and 54: in-game verification **Not yet**.
- API 8 (same version): hand-use persistence adds `UseRemainderImpl.template` and `create()`; `read_data` is no longer const and equality compares full item templates (`Hash` and `Eq` are removed). `UseRemainderImpl` and `AdvancementTrigger` retain `Debug`; `ItemStack` gains template diagnostics. `Advancement.action_criteria` and the `ItemUsedOnBlock` / `DefaultBlockUse` trigger variants are added (update exhaustive matches). The legacy `MilkBucketItem` hook is removed. Additive helpers: `item::item_utils::{create_filled_result, give_or_drop}`, `ItemRegistry::records_item_use_stat`, and `beehive::{is_smokey_pos, finish_harvest}`. Native API stays at 8 because no external plugin binaries target this fork's API 8 yet; rebuild against this checkout. Wasm WIT is unchanged.
- API 8 (same version): damage orchestration adds combat ownership and a hurt-animation timer to `LivingEntity`, and a separate hurt-motion flag to `Entity`. These layouts are embedded in native player and mob types. Additive public helpers: `LivingEntity::{knockback, with_damage_owned}`, `Entity::{push_impulse, acknowledge_motion_delivery}`, and `Player::send_hurt_motion`. Health/absorption setters take combat ownership; native and Wasm callbacks release it before dispatch, and resumed damage, equipment wear during hurt processing and weapon effects revalidate the admitted life. `reset_state` and NBT loads abort in-flight hits. `combat::handle_knockback` no longer damps the attacker; melee, spear and mace paths serialize attacker motion separately. `send_velocity` clears pending hurt-motion flags; player melee sends and restores predicted victim motion while hurt-marked. Spear enchantment knockback retains server motion without sending a second impulse to the victim's client. Projectile punch and explosion follow-ups retain victim ownership and life checks; `ExplosionResult` carries player lifecycles through delayed packet delivery (layout change). Interrupted death drops finish delivering removed/generated stacks, and healing a dying life permits a later death. Native plugins must rebuild; event payloads and Wasm WIT remain unchanged.
- API 8 (same version): rabbit review adds a default no-op `Mob::play_attack_sound` hook and changes `AvoidEntityGoal` to retain its admitted escape path. Rebuild native plugins; Wasm WIT is unchanged.

Rabbit hopping review (vanilla 26.3 `Rabbit`): the fork already has hop delays, speed-dependent jump power, jump sounds and event 1 after the impulse. The follow-up restores the constructor's initial zero-speed request, water-avoiding strolls, food-tag-based temptation, and Killer Bunny armor, damage and default name. Cancelling `EntityChangeBlockEvent` when eating carrots now preserves both the crop and the rabbit's appetite. The deep review corrects path-node jump heights, Killer Bunny target ordering and successful-hit sounds, and shared avoidance, home-restricted block searches and solid-render destination admission. This addresses the rabbit portion of [mob AI tracking #3468](https://github.com/Pumpkin-MC/Pumpkin/issues/3468). In-game verification is **Not yet**; native API changes are listed above and Wasm WIT is unchanged.

- API 8 (same version): harvest round 3 adds hand-aware methods and shearing access to `EntityBase` vtables, shearable mob and thrown-item state, a block attack hook, `World::bind_block_entity_context`, a per-world neighbor queue, and `PrepareArgs.update_limit`. `Shearable::shear` reports success, `Mob::transform` reports event acceptance, shearing loot accepts a live context, and `ExperienceBottleEntity::set_item_stack` borrows the source stack. Rebuild native plugins against this checkout. Both Wasm WIT versions retain their spectator UUID records and variant layouts; 26.3 spectator actions use the existing Unknown variant and raw payload. Typed access needs a future versioned API.

The XP orb review fixes breeding/trading single-orb rewards, furnace collection and fractional XP, summon defaults, follow selection and collection after a dimension change. In-game verification remains **Not yet**. Merging is selective: only equal values in the same one-of-40 entity ID group combine; a small mob kill pile normally retains many visible orbs.

Command execution contexts are thread-local, as in vanilla `Commands.executeCommandInContext`. Work handed to another Rayon worker starts a separate context, while unrelated work invoked on the same worker during dispatch can join the active context. Reentrant command dispatch currently runs inline with the shared quota; vanilla queues it at the front for execution after the current command. Plugins must account for that ordering difference.

The second shield/totem review adds authoritative item transactions, effect lifecycle corrections, projectile owner handling and teleport destination checks. Damage-pipeline integration now orders blocking before cooldown, serializes cooldown/absorption/health/death with healing, centralizes player difficulty and PvP checks, and sends completed melee motion to player victims. Play testing remains **Not yet**.


Known incomplete (movement): Movement behavior hooks for bee pollination, turtle homeward travel and migration, phantom swoops and drowned land searches are stubs until their species goals or Brain behaviors set them. Parrot taming and sitting still need separate tests. These hooks do not implement the missing behaviors by themselves.

## Harvest round two follow-up

The following ports and corrections are checked against the supplied vanilla 26.3 source. The owner has not yet confirmed their in-game behavior.

| Source | Change | Checked in-game |
|:--|:--|:--|
| [#3865](https://github.com/Pumpkin-MC/Pumpkin/pull/3865), ydw1904 | Withers drop one star at their position during death processing; its extra five minutes survive a restart. | Not yet|
| [#3666](https://github.com/Pumpkin-MC/Pumpkin/pull/3666) | Damageability respects unbreakability and removed damage components; damage reads are clamped before stacking decisions. | Not yet|
| [#3920](https://github.com/Pumpkin-MC/Pumpkin/pull/3920) | Dispensers release the full mob bucket contents, including names, variants, health and bucket persistence; shared fluid placement accepts replaceable plants and honours emptying cancellation before placement. | Not yet|
| [#3916](https://github.com/Pumpkin-MC/Pumpkin/pull/3916) | Saved heightmaps use the dimension's height; discarded heightmaps and lighting are repaired before normal chunk exposure. | Not yet|
| [#3706](https://github.com/Pumpkin-MC/Pumpkin/pull/3706) | Incoming item stacks check persistent count/component constraints; unknown item registry ids fail decoding. | Not yet|
| [#3775](https://github.com/Pumpkin-MC/Pumpkin/pull/3775) | Painting metadata uses registry holder ids, including the first variant. | Not yet|
| [#3897](https://github.com/Pumpkin-MC/Pumpkin/pull/3897), selective port | Saved item counts default to one and stay within 1–99; empty templates are rejected. Charged-projectile codecs come from the crash-codec merge. | Not yet|
| [#3524](https://github.com/Pumpkin-MC/Pumpkin/pull/3524) | Plugins can cancel the final falling-block placement without creating a block or item drop. Both existing piston fixes remain. | Not yet|
| [#3412](https://github.com/Pumpkin-MC/Pumpkin/pull/3412), selective port | Chained brewing fires a fresh start event; recipe inputs, reagents and fuel govern slots and hopper faces. | Not yet|
| Alb11747 `contrib/teleport-chunk-view`, reimplemented from vanilla | Accepted same-world teleports refresh the chunk view using the accepted destination; cancellation suppresses movement and broadcasts. | Not yet|

Mob death hooks, bucket-data layouts, item lifetime fields, chunk repair fields, teleport outcomes and the occupancy-free inventory predicate change the native API, raising `PLUGIN_API_VERSION` from storage API 6 to 7. Rebuild native plugins. The Wasm WIT is unchanged.

Combat task 4 changes projectile ownership, cloud and explosion layouts and adds projectile-state access to `EntityBase`, part of native API 8 (see the API section). Rebuild native plugins. Wasm WIT is unchanged. The 26.3 `deflects_projectiles` tag contains Breeze, not wind-charge entities.

Combat task 4 leaves axolotl rehydration and detached weapon item-break callbacks to their mob and enchantment systems, which do not expose those operations on this base. Server and real-client play tests have not been run.

Combat task 4 deep review: **Not yet** play tested. Healing splashes ignore zero amounts and dead targets; explosions use complete block loot; weapon enchantments survive arrow reloads; projectile ticks run the base lifecycle; trident pickups wait and respect ownership; spit continues after entity hits; strafing dragons launch travelling fireballs. Custom crystal and minecart damage attribution is independent of chained TNT ownership. Explosion rays stop at missing chunks and unknown terrain occludes exposure, a deliberate clamp until synchronous loading reads are available. Native API is 8. The unused melee deflection helper was removed; melee integration remains with its owning task.

Combat task 4 verification follow-up: **Not yet** play tested. Pearl impacts teleport to the start of the impact tick; shulker bullets, rockets and fishing hooks run the base lifecycle. Explosion redirection uses the custom damage cause; broken unstable TNT and ownerless burning-arrow minecart blasts carry no owner. Hits are skipped after dimension changes, and Java shift-overflow healing can lower health. Missing projectile owners are retried once per world tick and after an owner or dimension change.

## Survival hand-use independent review

In-game verification: **Not yet**. Off-hand XP bottles, eyes, eggs, pearls, boats, rockets and knowledge books consume only the requested hand. Raising blocking items no longer awards `Used`. Flower pots resolve contents by name, planting reuses parsed criteria, and glass bottles inherit the passing block hook. Hive release preserves newly arriving occupants; shears apply the smoke/angry-bee harvest behavior. Sign distance checks release the editor lock and preserve a replacement editor.

- Fork issue #90: Java air-use equips only swappable items allowed for the player entity type; stacked equippables install one and leave the remainder in hand, while displaced equipment goes to inventory or drops if full. Survival swaps respect `PREVENT_ARMOR_CHANGE`. Mirrors vanilla 26.3 `Item.use`, `Equippable.swapWithEquipmentSlot/canBeEquippedBy` and `LivingEntity.isEquippableInSlot`. In-game verification: **Not yet**.

Block interactions returning `Fail` stop before item `useOn`, preserving cancellable plugin harvest/cauldron events; vanilla `ServerPlayerGameMode.useItemOn` continues to item use after a non-consuming block result. This applies to all block `Fail` results in the Java handler.

Bedrock block-use cloned-hand writeback and held-item type validation are addressed by #141; validation precedes transaction actions, including last-item use. See the #141 bullet under Native plugin API 8 for rebuild requirements. Full validation of inventory action contents remains separate. Java entity interactions still need `isWithinEntityInteractionRange(3.0)` admission (finding 12). Mob capture bucket data and `FILLED_BUCKET` advancement remain a follow-up (finding 4).

Finding 10 does not require a change for 26.3: `CauldronInteractions.bootStrap` calls `PotionContents.is(WATER)`, whose `isPotionWithoutCustomEffects` requires empty custom effects. Preserve that check.

Mobs play-test bugs 2: **Not yet** play tested. The supplied 26.3 `Villager.golemSpawnConditionsMet` checks only `LAST_SLEPT` within 24,000 ticks, not work history; the removed work/bed-ownership and nitwit gates were not vanilla conditions. The requested `ON_TOP_OF_COLLIDER_NO_LEAVES` strategy and whole-body collision check are intentionally stricter than 26.3 `spawnGolemIfNeeded`, which uses `LEGACY_IRON_GOLEM` with `checkCollisions=false`. Placement rejects unloaded terrain instead of loading chunks on a tick worker. Roomy interiors with sufficient clearance remain valid, as in vanilla.

The goal-based villager still lacks the full social behavior packages: the adapter tracks a visible villager interaction partner and only runs the five-villager gossip summon trigger during data-backed IDLE/MEET activities, with vanilla's missing-job-site/meeting-point fallback to IDLE. Recent damage/visible hostiles supply the three-villager panic trigger. One nearby-entity scan per step serves detection, panic and partner selection. Existing Brain storage persists sleep and golem-detection memories, including the remaining TTL across a villager NBT round trip, with the vanilla 599 TTL expiring on tick 600. Summons retain `finalize_spawn_with_reason(MobSummoned)` and cancellable spawn events. Placement scans padded collision cells on every axis, accepts the union of contextual top faces (also for the shared warden strategy), and treats positions outside the dimension's build height as void air.

Creeper power remains 3, or 6 when charged; `ServerExplosion.hurtEntities` (lines 173–186) admits damage through distance 6 or 12 inclusive, with zero knockback at the cutoff. No range multiplier deviation was found.

## Sign, bookshelf and natural tree fixes

- Fork issue #6: player-submitted sign text stays literal, and standing/hanging sign commands require the persisted `allow_op_features` flag. Trusted command NBT and creative gamemaster sign items retain rich text; wax remains independent. Mirrors vanilla 26.3 `SignBlockEntity.updateMessages`, `executeClickCommandsIfPresent`, `saveAdditional`/`loadAdditional`, and `BlockItem.updateCustomBlockEntityTag`. In-game verification: Not yet.
- Sign messages stored as NBT strings are literal, including JSON-looking text on a trusted sign; rich components must use compound NBT (`SignText.CODEC` / `ComponentSerialization.CODEC`). Signs saved by the fork before the trust flag was added load untrusted. This deliberately differs from vanilla's data-version-5002 `SignsOpFlagFix`, which grants trust to pre-existing signs during upgrades. In-game verification: Not yet.
- Both Wasm API versions keep the existing WIT unchanged: sign setters treat plugin strings as literal text, and getters return flattened component text rather than the former raw JSON representation. Trusted selector, score and NBT text resolution (`SignBlockEntity.resolveLines`) is not ported. Inline dialog and custom click actions share the sign trust gate; registry-defined dialogs remain unsupported as in `/dialog`. In-game verification: Not yet.
- Sign item data follows vanilla 26.3 `TypedEntityData.loadInto`: merge custom NBT into saved fields, reload only when changed, and retain the editing session. In-game verification: Not yet.
- Fork issue #5: chiseled bookshelves use ordinary/enchanted insertion sounds when adding books and keep the distinct pickup sounds when removing them, matching `ChiseledBookShelfBlock.addBook`/`removeBook`. In-game verification: Not yet.
- Floating natural trees: ported [upstream PR #3903](https://github.com/Pumpkin-MC/Pumpkin/pull/3903) by Szabolcs05 for upstream issues [#3889](https://github.com/Pumpkin-MC/Pumpkin/issues/3889) and [#3724](https://github.com/Pumpkin-MC/Pumpkin/issues/3724). Resolve named block-state providers from the vanilla datapack instead of substituting air, matching `BlockStateProvider.CODEC` and `TrunkPlacer.placeBelowTrunkBlock`. New trees retain their supporting soil; existing chunks are not repaired. In-game verification: Not yet.
- `configured_features_generated.rs` retains its workspace-formatted layout from the tree fix. This is a formatting divergence from the generator's standalone `rustfmt` output; the generated provider data is unchanged.
## Falling hazards hunt

- Falling anvils hurt living entities beneath them, can crack or break on impact, and play their landing or break event. Landing blocks retain their distance adjustments and damage suppression; waterloggable falling blocks become waterlogged when landing in source water (`FallingBlockEntity.tick`). Mirrors `AnvilBlock.falling`, `FallingBlockEntity.causeFallDamage`, `Block.fallOn`, and `AnvilBlock.onLand/onBrokenAfterFall`. In-game verification: **Not yet**.
- Unsupported stalactites detach after two ticks, including speleothem-tagged segments in the falling column, with column-sized impact damage on their tip; unsupported stalagmites break with drops after one tick. Mirrors `SpeleothemBlock.updateShape/tick/spawnFallingStalactite`. In-game verification: **Not yet**.
- Landing on an upward dripstone tip deals stalagmite damage. Adapted from [#3416](https://github.com/Pumpkin-MC/Pumpkin/pull/3416) by Laggy60, retaining its landing-position and damage-source approach with vanilla 26.3's `PointedDripstoneBlock.fallOn` offset of 2.5. Addresses [#3377](https://github.com/Pumpkin-MC/Pumpkin/issues/3377). In-game verification: **Not yet**.
- Honey side contact slows sliding entities and resets fall distance, including accepted player movement before a later landing packet; slide sounds, particles and the vanilla advancement are dispatched. Mirrors `HoneyBlock.entityInside/isSlidingDown/doSlideMovement`. In-game verification: **Not yet**.
- Paths turning into dirt lift occupants out of the added collision volume, including placement fallback. Mirrors `PathBlock.getStateForPlacement/turnToBaseBlock` and `Block.pushEntitiesUp`. Farmland still has a TODO rather than a separate push implementation; its owning lane can use the shared helper. In-game verification: **Not yet**.
- Inserting an Ender Eye lifts occupants out of the eye's added collision volume before changing the frame. Mirrors `EnderEyeItem.useOn` and `Block.pushEntitiesUp`. In-game verification: **Not yet**.
- Opening an End portal drops interior blocks and container contents before replacing them with portal blocks, retaining cancellable break events. A cancelled `BlockBreakEvent` leaves that interior cell unchanged and unfilled, unlike vanilla, which always places the portal block. Mirrors `EnderEyeItem.useOn`. In-game verification: **Not yet**.
- Wither roses give Creative players the 40-tick Wither effect without damaging them, while permanent entity invulnerability and equipped-enchantment damage immunity still block the effect. Mirrors `WitherRoseBlock.entityInside`, `LivingEntity.isInvulnerableTo`, `Entity.isInvulnerableToBase`, and `Player.hurtServer`; mob-specific effect immunity comes from effect admission. Player NBT saves game-mode abilities separately; legacy player `Invulnerable:true` tags are cleared on load by the #130 hotfix above. Non-player tags remain permanent. In-game verification: **Not yet**.

Living fall damage applies the `fall_damage_multiplier` attribute, including stalagmite landings. Mirrors `LivingEntity.calculateFallDamage`. In-game verification: **Not yet**.

## Guard boundaries (2026-10-11)

The shared-state audit findings 6-11 are addressed by debug lock-order and callback
boundary checks, split enchantment callbacks with input revalidation, and owner
notifications delivered after unlocking menus, weather, scoreboards, borders and
advancements. API 8 (same version); native plugins built against this fork must
be rebuilt for the typed locks and advancement notification methods. No saved
format changes.
The final-review follow-up moves merchant rewards, trade sounds and Bedrock drops
outside menu guards, covers deferred execute-score and mooshroom team updates,
and preserves border, scoreboard and advancement notification order across
concurrent mutations and callback re-entry. API 8 (same version): merchant
callbacks use shared ownership, offer selection accepts the player, and
`InventoryPlayer` gains a deferred menu callback hook. Native plugins must rebuild
for these signatures and the added notification queues; Wasm WIT is unchanged.
The delta-review follow-up gives each merchant opening a unique session token,
serializes replacement and close cleanup, and resumes ready owner notifications
when an earlier batch is abandoned. API 8 (same version): villager and wandering
trader layouts gain private session ownership, world scoreboard/border locks
start their own notification batches, and `InventoryPlayer::run_menu_callback`
requires an implementation that schedules callbacks outside the menu guard.
Native plugins must rebuild. Closing-menu events again expose the closing
inventory, and slot tracking retains every shared player-inventory index using
inventory allocation identity.
In-game verification: Not yet. This is fork audit work, not an upstream PR port;
GitHub PR checks were unavailable through the configured proxy.

Ordered owner notifications recover an already committed ready successor after a native packet callback unwinds. Recovery retains the owner, releases queue ownership before dispatch, preserves one active drainer and never replays the failed effect. Actual border, scoreboard and advancement packet callbacks exercise commit-then-panic without a third mutation. In-game verification: **Not yet**.
## Baseline scenario fixes

- A zero interaction location consumes only its one-byte vector encoding, leaving the next packet field intact. Mirrors vanilla 26.3 `LpVec3.read`. In-game verification: **Not yet**.
- Global game time and each dimension's declared optional clock, including rate, pause and fractional ticks, are restored separately after restart. Live canonical default-root clocks persist by their declared registered clock identity on autosave, `/save-all` and shutdown; dimensions without a default clock emit no clock record. Final clocks are captured after unload callbacks and retained across failed saves and canonical reloads; newer canonical owners supersede old unload completions. Distinct custom roots restore and persist their own registered clocks, retain final unload callback changes and failed-write snapshots, and migrate only their own valid legacy day-time (or own game time when absent). Named autosaves write only that root's clock document; canonical Overworld autosaves schedule main full metadata once. Physical path aliases share a named owner and cannot acquire main clock ownership. Spawn, difficulty and game-rule setters merge only their fields into current metadata, preserving retained clocks through callbacks and concurrent changes. Only known unregistered fork-created Nether/caves aliases migrate away, preserving custom and opaque unloaded records. Plugin-created worlds, including dot and absolute aliases of the default root, cannot replace its canonical clocks, even after canonical dimension unload. Other custom-world full metadata retains its existing persistence limitation; clock writes preserve that root's seed, generation metadata and opaque unloaded clock records. Int-shaped clock ticks migrate to Long without resetting. Invalid fractional ticks fall back to zero. Clock, RNG and game-rule/world-generation/weather/trader saves force a complete temporary file before atomic replacement; interrupted writes retain the previous document and independent metadata documents and player/advancement/world save families are attempted before combined contextual failures reach save-all. Metadata saves leave `random_sequences.dat` to the live RNG writer, preventing first-autosave truncation races. Mirrors `PrimaryLevelData.parse/setTagData`, `ServerClockManager.init/packState`, `ClockState.CODEC` and `MinecraftServer`'s `RandomSequences.TYPE` loading through `SavedDataStorage`, with fork guards against malformed fractions and interrupted writes. In-game verification: **Not yet**.
- Vanilla-shaped player saves retain explicit `Invulnerable` entity immunity; unmarked older fork saves clear the overloaded mode flag when `playerGameType` is a byte, or an int accompanied by the fork-only boolean `HasPlayedBefore` tag (true or false). The v2 marker takes precedence. Mirrors `Entity.load` and `ServerPlayer.readAdditionalSaveData`. In-game verification: **Not yet**.
## Interaction and hurt sound delivery (#99)

Reports by **uelq** (bucket interactions and creeper ignition) and **latent.exe** (hurt sounds). In-game verification: **Not yet**.

- Bucket filling excludes the acting player and uses the player's position and Players sound category (`BucketItem.use` / `Player.playSound`). Emptying and Nether evaporation exclude the actor and use the destination block center (`BucketItem.emptyContents/playEmptySound`, `MobBucketItem.playEmptySound`, `SolidBucketItem` and `Level.playSound`). Dispensers keep the actorless broadcast path. Existing cancellation, item exchanges, game events and mob spawning remain in their existing order.
- Creeper flint-and-steel ignition excludes the igniter (`Creeper.mobInteract`). The later primed/fuse sound still broadcasts to both players.
- Primary player hurt/death sounds exclude the victim; observers receive the source-specific hurt sound or player death sound (`Player.getHurtSound/getDeathSound`). Damage and death-status packets remain unchanged, so the victim's client plays its local feedback (`LivingEntity.handleDamageEvent/handleEntityEvent`, `Player.playSound`, `LocalPlayer.playSound`). The player sound override bypasses `Silent`; non-player primary sounds retain the normal silence guard. Secondary Thorns sounds still reach the victim and observers.

Checked against the official Java 26.3 server (inner JAR SHA-256 `a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`) and client (SHA-1 `e877b6a07acd633fb3bb475002175cec036e7b87`), decompiled with Vineflower 1.12.0. The existing World sound helpers keep their Java-only recipient grouping and chunk-based audible range. TNT doubling remains unconfirmed and separate; creeper fire-charge support and broader sound-call-site parity are outside this fix. No native layout, trait, Wasm WIT or saved-data change.
## Bubble-column runtime activation (#62)

Source water checks the current magma/soul-sand support below on placement and ordinary neighbor updates, and the captured lower neighbor on a shape update, then schedules WATER after 20 ticks. Existing columns schedule their survival, direction and extension checks after five ticks. Mirrors Java 26.3 `LiquidBlock.onPlace/neighborChanged/updateShape/tryScheduleBubbleBlockColumn` and `BubbleColumnBlock.updateShape/canSurvive`.

`BubbleColumnBlock.updateColumn` reconciles the whole eligible run in one callback using listener/shape writes. It continues after an unchanged first write, stops at a later unchanged write, and uses conditional writes to avoid overwriting a changed or unloaded cell. Eligibility retains the literal liquid-block class boundary, fluid tag and full-source amount; waterlogged obstructions and flowing water are not converted. Bubble shape updates use the existing normalized water-fluid family and flow delay.

Ordinary neighbor callbacks retain the existing cancellable `BlockPhysicsEvent`; shape callbacks retain their separate contract. This ports the column-scheduling part of WATER shape handling, not the complete fluid-shape behavior. Entity collision, movement and breath handling are outside this change.

Addresses runtime findings A and C in [fork issue #62](https://github.com/Optifineless/Pumpkin/issues/62), reported by **VisionGlitcher**. Generated-chunk initialization in finding B remains with [PR #107](https://github.com/Optifineless/Pumpkin/pull/107), so the issue stays open. In-game verification: **Not yet**.
## Portals

- PR #151 retired-pair follow-up: deferred bed, straw-bed and door teardown checks the partner's captured full block state before breaking it, preserving replacement pairs placed by physics plugins and still removing unchanged old partners. Mirrors vanilla 26.3 `LevelChunk.setBlockState`, `AbstractBedBlock.getConnectedDirection` and `DoorBlock`/`DoublePlantBlock` half lookup. Non-destructive pair notifications remain enabled. In-game verification: **Not yet**.
- PR #151 follow-up: deferred old-sign cleanup cannot delete a callback's replacement chest, and old portal cleanup preserves rebuilt portal POIs. Mirrors vanilla 26.3 `LevelChunk.setBlockState` retirement before `affectNeighborsAfterRemoval`. Replacement inventories are covered through unload/save/reload. In-game verification: **Not yet**.
- Portal frame and POI creation completes before block callbacks and neighbour updates run, releasing creation ownership before native physics handlers can request the same destination again. Mirrors vanilla 26.3 `NetherPortalBlock.getExitPortal` and `PortalForcer.createPortal`, using the fork's deferred effect batches. Replaced containers retire before callbacks; their captured contents drop once without removing a new container placed by a callback (`LevelChunk.setBlockState`/`BlockEntity.preRemoveSideEffects`). API 8 (same version), rebuild native plugins. In-game verification: **Not yet**.

- Extracted the destination deadlock and cancelled chunk-request ticket leak fixes from fork PR #127. Portal destination preparation runs on the server's Tokio task spawner; POI reads, terrain scans and builds run in tracked `spawn_blocking` jobs, with scoped chunk tickets and a queued-unload ticket recheck. Mirrors vanilla 26.3 `NetherPortalBlock.getExitPortal`, `PortalForcer.findClosestPortalPosition`/`createPortal`, `ServerChunkCache.getChunkFutureMainThread` and `ChunkMap.updateChunkScheduling`; failed creation leaves the traveler in place. API 8 (same version): async portal lookup signatures, `World` layout and `Level.chunk_loading` ownership change; native plugins must rebuild. In-game verification: **Not yet**.
Bedrock offhand equipment updates use the protocol offhand container 119, including the empty-hand update after `/clear`; the separate armor container is 120. The tracker regression exercises both count-only and actual clear commands with a real Bedrock observer. In-game verification: **Not yet**.

## PR #102 shared block destruction hook

The shared `player_will_destroy` hook runs after break-event admission and before loot or removal; the paired-block helper suppresses drops and sends destroy effects to Java and Bedrock clients. It provides the vanilla ordering used by `ServerPlayerGameMode.destroyBlock`, `AbstractBedBlock.playerWillDestroy`, and `DoublePlantBlock.preventDropFromBottomPart`. API 8 (same version); native plugins should rebuild for the `BlockBehaviour` trait layout change. Wasm WIT is unchanged. In-game verification: **Not yet**.
Inventory transactions now use per-container ownership and conditional withdrawals across menu clicks, hoppers, dispensers, crafting and plugin writes. Player inventory and ender-chest saves share a snapshot boundary; removed containers reject refunds, and hopper extraction updates jukebox record state after releasing ownership. Mirrors `AbstractContainerMenu.doClick/moveItemStackTo`, `HopperBlockEntity.tryTakeInItemFromSlot/addItem`, `DispenserBlock.dispenseFrom`, `DropperBlock.dispenseFrom`, `CrafterBlock.dispenseFrom`, `Player.addAdditionalSaveData` and `JukeboxBlockEntity.setTheItem`. Native plugin layouts and slot methods use API 8 (same version). Addresses the four final-review inventory-transaction findings. In-game verification: **Not yet**.

Crossbow firing claims and removes the observed charge before sound, projectile or plugin effects; loading validates the observed weapon and commits the actual ammunition payment and loaded component under the same player inventory owner. Launch callbacks retain their unrelated component edits, and wear follows the admitted physical hand slot. Unpaid drawn projectiles retain `IntangibleProjectile` and the shared arrow factory gives them creative-only pickup, matching vanilla 26.3 `CrossbowItem.performShooting/tryLoadProjectiles`, `ProjectileWeaponItem.draw/useAmmo` and `AbstractArrow`'s constructor. The fork's existing release-time loading and projectile behavior remain. Native API 8 (same version): rebuild native plugins against this checkout; Wasm WIT is unchanged. In-game verification: **Not yet**.

PR #145 follow-up: failed hand claims suppress throwable effects, use statistics and cooldowns; paid items return if a plugin cancels entity spawning. Mirrors vanilla 26.3 throwable `use` methods; Pumpkin claims before effects because inventory edits can run concurrently. API 8 (same version): additive `ItemBehaviour::use_observed_stack`; rebuild native plugins. Wasm WIT and saved inventory formats are unchanged. In-game verification: **Not yet**.

PR #145 Bedrock follow-up: virtual crafting output pays one generated batch per request, rejects unmapped or full destinations, and debits only an exact committed insertion. Paid leftovers return to inventory or drop after releasing the menu guard; unpaid previews cannot become items. Mirrors vanilla 26.3 `ResultSlot.onTake`, `AbstractContainerMenu.doClick` and `Slot.safeInsert`. API 8 (same version): additive `Slot::craft_into` and `CraftingCommit::finish`; rebuild native plugins. Wasm WIT and saved inventory formats are unchanged. In-game verification: **Not yet**.

PR #145 integration preserves #169 request authority before inventory writes, including Creative authorization, preview/physical-alias rejection and cursor identity. Admitted crafting still pays every repetition atomically, and Bedrock equipment transitions use the existing per-tick publisher. Mirrors vanilla 26.3 `ServerGamePacketListenerImpl.handleContainerClick/handleSetCreativeModeSlot`, `ResultSlot.onTake` and `LivingEntity.detectEquipmentUpdates`. API 8 (same version); rebuild native plugins. In-game verification: **Not yet**.

PR #145 Java conservation follow-up: eating and drinking claim the live hand before effects, with bottle/bowl delivery preserving callback edits. Java crafting and merchant quick-move pay inputs and deliver results under one ordered inventory transaction; trade and crafting effects follow ownership release. Dragon-breath bottling claims its bottle before shrinking the cloud, and number-key cartography swaps post-process only admitted stock. Mirrors vanilla 26.3 `Consumable.onConsume`, `ItemStack.applyAfterUseComponentSideEffects`, `CraftingMenu/InventoryMenu.quickMoveStack`, `ResultSlot.onTake`, `MerchantMenu.quickMoveStack`, `MerchantResultSlot.onTake`, `BottleItem.use` and `AbstractContainerMenu.doClick`. API 8 (same version): additive `Slot::quick_move_craft`; rebuild native plugins. Wasm WIT and saved inventory formats are unchanged. In-game verification: **Not yet**.

## split-102-dimension-infra: dimension bed rules

The generated dimensions now include ordinary-bed and straw-bed `BedRule` data, including vanilla `destroy_on_use`, `destroy_on_leave` and error-message fields. The regression checks these values and respawn-anchor rules against vanilla 26.3 `BedRule` and `DimensionTypes.bootstrap`. Existing gameplay readers retain their prior output until `split-102-beds-r2` uses the new fields; this branch adds no gameplay behavior.

## Bedrock hand transactions (#141)

Bedrock predicted placement retains the validated originating item through block-entity component application, including a last offhand shulker or a nonempty container remainder. Placement hooks keep their cancellation point; moved or replaced sources reject installation without restoring stale stock, while selection-only changes still pay the retained slot. The public placement entry retains its current hand before placement hooks, matching BlockItem.place / UseOnContext; the internal Bedrock entry receives the validated snapshot. Hand equipment updates to Bedrock observers now encode the current player's selected hotbar index (main hand uses that index; offhand uses container-local slot zero). Non-player defaults and owner offhand container119/slot0 remain unchanged. Real handler tests cover original-byte contents/lock/observer failures, callback replacement and cancellation. In-game verification: Not yet.

Bedrock non-placement block-use writeback keeps the originating hotbar slot across interaction callbacks and atomically checks stack identity and contents before saving the consumed clone. Placements instead claim the pinned source before world effects. Callback replacements take precedence, including equal stacks with a new identity. Mirrors `ServerPlayerGameMode.useItemOn` and `BlockItem.place`. PR #141 callback slot-switch and offhand replacement verification in-game: Not yet.

No-op Bedrock actions retain the originating stack UID. For block and air use, originating-hand predictions are advisory: server use consumes once from the retained stack, and denied use keeps the original contents. Selection and stack-identity checks skip stale item dispatch after callbacks; final hand publication rechecks both live hands and selection so combat modifiers follow the current item. Mirrors `ServerPlayerGameMode.useItem/useItemOn`, `EggItem.use`, `ItemStack.consume` and `LivingEntity.detectEquipmentUpdates`. In-game verification: **Not yet**.

Use-item transactions reconcile their hotbar index through the held-item-change event before admitting the server stack, so a quick switch followed by use charges the requested slot even before its equipment packet arrives. Invalid slots and cancelled selection changes reject use and correct the owner's inventory and selection. Mirrors `ServerGamePacketListenerImpl.handleSetCarriedItem` and Geyser's `BedrockInventoryTransactionTranslator.translate`. PR #141 round 4b in-game verification: **Not yet**.

Denied Bedrock block placements correct both the clicked block and its face-adjacent position with current server block states, alongside the hand correction. Mirrors `ServerGamePacketListenerImpl.handleUseItemOn` after `ServerPlayerGameMode.useItemOn`, including replacement of the clicked block. PR #141 round 4b ghost-block verification in-game: **Not yet**.

Entity-use and active-hand release predictions are also advisory, so feeding an animal consumes once and cancellation preserves the original stack. Inactive release predictions cannot debit either hand, and release callbacks may begin a newer use without old cleanup stopping it. Gameplay completion retains both hands from admission, then compares both live hands, publishes an opposite-hand replacement even when selection stayed unchanged, and keeps an offhand use valid when only the main-hand selection moved. Equipment swaps commit the checked hand and equipment slots before announcement callbacks, so combat modifiers describe actual equipped items. Mirrors `ServerGamePacketListenerImpl.handleInteract`, `Player.interactOn`, `Animal.mobInteract`, `LivingEntity.releaseUsingItem/detectEquipmentUpdates/collectEquipmentChanges` and `Equippable.swapWithEquipmentSlot`. PR #141 round 4c in-game verification: **Not yet**.

Direct Bedrock equipment publication reconciles a selection changed by its packet callbacks to the live hotbar and hand. Rejected click-block admission also corrects the clicked and adjacent block states, and final durability use commits its checked hand before broken-item statistics or animations can call plugins. Mirrors `ServerGamePacketListenerImpl.handleSetCarriedItem/handleUseItemOn`, `LivingEntity.detectEquipmentUpdates/collectEquipmentChanges/onEquippedItemBroken` and `ItemStack.applyDamage`. PR #141 round 4d in-game verification: **Not yet**. API 8 (same version); the hotbar publication helper is internal.

PR #141 rebased onto #169 rounds 1–3 and #124: all client action descriptors stay prediction-only; legitimate use runs on the admitted server stack and pinned slot. Unchanged uses send no equipment or inventory correction; changed slots are corrected selectively, with full correction for discarded actions or rejected hand identity. Final equipment derives from live slots in the tick publisher, rechecks item values and selection after packet callbacks, permits one corrective publication, and carries later changes to the next tick while refreshing live modifiers before returning. Use callbacks that move selection also correct the owner's hotbar. Item-use failure corrects clicked and adjacent blocks. Equipping a stackable item installs one and retains its remainder, returning displaced equipment to inventory or dropping it if full. Mirrors `ServerGamePacketListenerImpl.handleContainerClick/handleUseItemOn`, `AbstractContainerMenu.broadcastChanges`, `LivingEntity.detectEquipmentUpdates/collectEquipmentChanges` and `Equippable.swapWithEquipmentSlot`. API 8 (same version); native plugins must rebuild. In-game verification: **Not yet**.

The #141 rebase preserves #169 prediction-only descriptors and #145 conditional inventory ownership. Entity use with an omitted item descriptor still uses the server hand; click-block and click-air validate supplied identity regardless of predicted count. Committed egg statistics follow payment and spawning; pre-use statistic callbacks still reject stale admission. API 8 (same version). In-game verification: **Not yet**.

Callback reconciliation retains changed attribute identities in the existing equipment queue, so removing a held weapon corrects owner and observer attributes on the next publication even after a real attack reads the refreshed value. This mirrors `AttributeMap.onAttributeModified` and `ServerEntity.sendDirtyEntityData`; publications remain bounded to one synchronous correction. API 8 (same version): the private player equipment queue layout changes; native plugins must rebuild. In-game verification: **Not yet**.

PR #141 round 6: Java and Bedrock placements claim the pinned source after cancellable placement callbacks and before installing the block. Moved or replaced stacks reject placement and correct predicted blocks; a changed target refunds the claim without overwriting callback stock. Block-entity components come only from the claimed source, and Creative keeps its item. Mirrors vanilla 26.3 `BlockItem.place/placeBlock/updateBlockEntityComponents` and `ServerGamePacketListenerImpl.handleUseItemOn`; Pumpkin claims before effects to preserve vanilla's same-stack payment across callbacks. API 8 (same version); native plugins calling `BlockRegistry::place_block` must stop performing an additional payment. Saved data and Wasm WIT are unchanged. In-game verification: **Not yet**.

## Pitcher-crop bonemeal admission (#10)

Pitcher-crop bonemeal now shares vanilla 26.3 `PitcherCropBlock.canGrow` checks
for age, light, upper build height and available space. It resolves clicks on
either half, rechecks after `BlockFertilizeEvent`, and keeps an obstructing block
and the bone meal unchanged when growth is rejected. In-game verification: Not yet.

Play test: Bone meal an age-two pitcher crop with open space above it; expect a
two-block crop and one bone meal used. Put a diamond block above an age-two crop
and bone meal it; expect the diamond, crop age and bone-meal count to stay the
same. Bone meal the upper half of an age-three crop; expect it to reach age four.

## PR #177 glass-bottle source-water pickup

Glass bottles use the nearest block-outline or source-fluid hit with the main-hand collision context and the player's block interaction range. Only a source-water block can be collected; flowing water and waterlogged-only fluid remain unchanged. The cancellable pre-use event sees the same hit used by the bottle action. World-border and spawn protection are checked before effects; the neutral fill sound, `FLUID_PICKUP`, statistic and exact-slot bottle transaction follow vanilla 26.3 order. Dragon-breath pickup keeps precedence, and overflow potion drops use the player's drop position through `create_filled_result`. Mirrors `BottleItem.use/turnBottleIntoItem`, `Item.getPlayerPOVHitResult`, `BlockGetter.clip` and `ServerLevel.mayInteract`. No public plugin API changes. In-game verification: **Not yet**.

## Dolphin swim-with-player goal (#157)

Dolphins acquire the nearest noncombat player within 10 blocks, ignoring line of sight, when that player is swimming and is not the dolphin's combat target. They look toward and follow that player at speed 4, stop within 2.5 blocks, and continue while the player stays within 16 blocks. Starting the goal and its one-in-six swimming refresh grant 100 ticks of Dolphin's Grace through the existing effect event and packet path; the player is revalidated after callbacks before the effect commits. Mirrors vanilla 26.3 `Dolphin.SWIM_WITH_PLAYER_TARGETING`, `DolphinSwimWithPlayerGoal` and `Dolphin.registerGoals`. No public plugin API, saved-data or WIT changes. In-game verification: Not yet.

Issue #157 follow-up: visibility applies the target predicate's normal distance scaling while line of sight remains ignored; initial Grace accepts exactly 10 blocks, continuation remains strictly inside 16 blocks, and the follow goal at priority 2 can preempt melee at priority 6. Regression tests cover invisible and visible acquisition, both distance boundaries, an already-running melee goal, ordinary combat-target continuation, and panic-goal interruption. Public API 8, saved data and WIT are unchanged. In-game verification: Not yet.

PR #339 round 2 removes random jump, wander and look candidates from the melee-priority setup and resets entity age before each direct tick so the full selector runs. The companion continuation and panic scenario retains the intact selector. No runtime behavior changed.

Play test: With a sprint-swimming player and a dolphin, verify an invisible player at five blocks is not acquired while a visible player at five blocks and an invisible player nearby are acquired. Verify initial Grace at exactly 10 blocks, no acquisition just outside 10, continued following just inside 16, and stopping at exactly 16. Give the dolphin a combat target, then bring a different swimmer nearby; following should preempt melee and grant Grace. Finally hit the dolphin with a panic-causing source near water; escape behavior should interrupt following.

PR #228 review: effect insertion, dependent attributes, visibility, particle metadata
and the absorption start hook now commit under one synchronous combat owner before
packet publication. A concurrent `/effect clear` cannot leave permanent Strength
damage or restore cleared effect metadata. Mirrors vanilla 26.3
`LivingEntity.addEffect/onEffectAdded/onEffectUpdated` and
`MobEffect.addAttributeModifiers`. Public API 8, layouts and saved-data meaning are
unchanged. In-game verification: **Not yet**.

## Scheduled fire rules and client extinction updates

Issues #78, #79 and #160: scheduled fire checks the fire-spread radius at its source after scheduling its next tick, measuring from the block corner with a strict radius; -1 remains unrestricted, radius 0 admits no players, and spectators do not count. Adjacent burnout reads the target block state, so waterlogged wood has no burn odds. Burn and spread callbacks retain a replacement block instead of overwriting it; dry burning, cancellable events and TNT priming remain. Support and rain extinction notify clients as well as neighbors. Mirrors vanilla 26.3 `FireBlock.tick/canSpreadFireAround/getBurnOdds/checkBurnOut` and `Level.removeBlock`. Upstream PR #3669 independently contains the same two notification flags for #160; this grouped fork change records that overlap alongside #78 and #79. No public API or saved-data changes. In-game verification: Not yet.

Play test: Set the fire-spread radius to 0, place a supported flame beside an air gap and a dry wooden block beyond it, and keep players centered over the flame and gap; expect no new flame or wood loss. Set the radius to -1 and repeat; expect fire to spread, dry wood to burn, waterlogged wood to remain, and TNT to prime once.

## Lodestone compass component preservation (#43)

Survival singleton binding updates the used stack in place. Stack and Creative
binding copy the original component patch with a fresh stack UID and replace its
lodestone tracker; a full Survival inventory drops the bound copy, while a full
Creative inventory discards it. Mirrors vanilla 26.3 `CompassItem.useOn`,
`ItemStack.transmuteCopyIgnoreEmpty` and `Inventory.add`. No public API, WIT or
saved-data changes. Survival stacked binding withdraws from the exact hand slot
before insertion, so a bound copy that matches its source can merge without
increasing the total count. A real `handle_use_item_on` regression covers both
hands, repeated same-target uses, and free/full inventory layouts. In-game
verification: Not yet.

Play test: Bind a named singleton in each hand with a full inventory; expect the
same named compass to remain held with the new tracker and no drop. Combine two
identically named compasses bound to the same lodestone, then click it repeatedly
in either hand with space and with a full inventory; expect exactly two
compasses with the name and custom settings intact. Bind a named stack and
rebind it; expect the copy to retain its name and component removals.
Repeat in Creative with space and with a full inventory; expect a bound copy in
the free slot and no ground copy when full. Capture the holder and a nearby
observer.

## PR #195 owner review

World setup sends current health instead of healing dimension travelers to twenty. Respawn still restores maximum health explicitly. Pearl arrivals register entity tracking before damage while remaining outside destination player ticks until the effects finish. Lethal hits resolve the tracked arrival and retain its player reference for the cancellable death event, inventory drops and death packet before publication. Damage admission retires its pending hurt before releasing the serial owner. No saved-data meaning, native public type/layout, or plugin API version changes. Written by Codex (GPT-6.1 Sol). In-game verification: **Not yet**.

Round 2 keeps login and respawn loading immunity for every damage type. Only an accepted pearl transfer whose player was already loaded receives permission for that individual hit, including callback revalidation; damage triggered inside a callback gets no such permission. Public entity/player lookup resolves the tracked arrival in its current world before destination ticks begin, so protection handlers can identify and cancel its damage. The death path uses that same identity-checked lookup. No saved-data meaning or public native type/layout changes; API 8 stays unchanged. In-game verification: **Not yet**.

Checked against official Java 26.3 `ServerPlayer.isInvulnerableTo/teleport`, `ServerGamePacketListenerImpl.hasClientLoaded/teleport` and `ThrownEnderpearl.onHit`, decompiled with Vineflower 1.12.0 (inner server JAR SHA-256 `a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`).

Play test: with four hearts, land a pearl in another dimension; expect one and a half hearts, one hurt animation and the teleport sound on arrival. Have a nearby spectator record the arrival and hurt animation. Change dimensions normally at four hearts; expect four hearts afterward. Cancel a pearl transfer with a teleport plugin; expect unchanged health and fall state and no teleport sound. After a mace smash, pearl upward and then fall normally; expect ordinary fall damage. Rejoin at two hearts while a friend releases your stasis pearl before the loading screen finishes; expect to survive at two hearts. With a protection plugin cancelling pearl damage after looking up the victim, land a cross-dimension pearl at two hearts; expect to survive at two hearts.
