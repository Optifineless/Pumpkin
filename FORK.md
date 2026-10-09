# Murgicraft fork of Pumpkin

This is a fork of [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) maintained for the Murgicraft server. It tracks upstream `master` and adds fixes aimed at running a survival server: redstone, combat, world generation, commands and the plugin API.

## How the code here is written

The changes in this fork are written by AI coding agents, not by hand. The fork owner tests them in-game but does not review the code line by line. Treat every commit that isn't from upstream accordingly.

Each change is still held to upstream's [AGENTS.md](AGENTS.md) rules: it is ported from the decompiled vanilla source, stays one topic per commit, passes `cargo fmt`, `clippy` and the tests, and comes with a regression test where one can catch the bug. That keeps the commits small enough for upstream to take any of them if they want to.

None of this is sent upstream as pull requests. Upstream maintainers are welcome to cherry-pick anything they find useful.

## Staying in sync

Upstream is merged in regularly. When upstream fixes something this fork also fixed, the fork drops its own version and keeps upstream's.

## Upstream pull requests included early

These are open upstream PRs merged here before upstream merges them. Each is dropped from the fork once upstream merges its own version.

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
| Hoppers keep their facing after a restart | Yes, 2026-10-07 |
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

## Native plugin API 8

Rebuild native plugins against this checkout. The Wasm WIT is unchanged throughout.

- API 8 (same version): `VillagerEntity` changes layout with sensor timing and a group reservation lock, and its Brain registers sleep, recent-golem and interaction memories. `SpawnStrategy` changes from a single-variant enum to a discriminated enum with `OnTopOfColliderNoLeaves` (update exhaustive matches). Rebuild native plugins; Wasm is unchanged.
- API 4: shield and totem integration changed native trait/component layouts. `EntityDamageByEntityEvent.damager_id` now identifies the direct projectile for projectile hits; it previously identified the shooter. Resolve the projectile's owner for player attribution. Mob damage hooks carry separate direct and causing entities.
- API 5: the crash-codec audit changes `InstrumentImpl` from a unit component to registered or inline instrument data.
- API 6: durable storage. `ChunkData.dirty` and `ChunkEntityData.dirty` are `DirtyFlag` instead of `AtomicBool` (carried by the chunk load, save and send events); `ChunkSections.randomly_ticking_mask` is `RandomTickMembership`; `Server` gains `tick_gate` and a session lock and `Player` gains `storage_session` (layout changes); `Server::add_player` is async; `Level::shutdown`, `get_entity_chunk`, `get_or_fetch_chunk` and `get_or_fetch_entity_chunk` return `Result`. Mob movement (same version) changes `MobEntity.look_control` to `Box<dyn LookControlTrait>`, adds public movement fields, and changes `PathNavigationTrait::tick` to take the owning mob and its collision context.
- API 6 (same version): command robustness adds methods to `CommandSource`, `ReturnValueCallable` and both `CommandExecutor` traits, the public `RedirectModifier::CustomUncharged` variant (update exhaustive matches), a `FunctionRunError::EmptyTag` variant and a new `DatapackManager` field, removes `MAX_FUNCTION_CHAIN_DEPTH`/`MAX_FUNCTION_CHAIN_COMMANDS`, and `reload_datapacks` no longer runs `#minecraft:load` synchronously. `CommandDispatcher::execute` installs a command quota context; a plugin's reentrant `handle_command` joins the caller's quota. `/function` and nested `execute_function` calls return 0 when scheduled; `/function` sends "Running function..." feedback before execution and reports an explicit function return later, including its result callbacks.
- API 7: harvest integration changes mob death hooks, bucket data, item lifetime fields, chunk repair fields, teleport outcomes and the occupancy-free inventory predicate (retained from the fork head).
- API 8: XP orbs. `Player.experience_pick_up_delay` is now an `AtomicU32`, replacing `Mutex<u32>`, and `EnchantmentHelper::modify_durability_to_repair_from_xp` takes and returns `i32`. `World` gains a temporary orb snapshot (layout change). Additive helpers: `ExperienceOrbEntity::{award, award_with_direction, spawn_single, new_empty, get_value}`, `collect_nearby_orbs`, `Entity::move_towards_closest_space`, `EnchantmentHelper::get_random_item_with_repair_effect`, and `furnace_experience::recipe_experience`. `InventoryPlayer::award_experience` now drops collectible orbs at the player for furnace output. Native plugins must be rebuilt; Wasm is unaffected.
- API 8 (same version): projectile-state and explosion hooks change `EntityBase` vtables; `ExplodeArgs` carries the explosion context. Projectile ownership, cloud and explosion layouts also change. Rebuild native plugins against this checkout; API 7 from the harvest branch is incompatible. Wasm WIT is unchanged.
- API 8 (same version): hand-use persistence adds `UseRemainderImpl.template` and `create()`; `read_data` is no longer const and equality compares full item templates (`Hash` and `Eq` are removed). `UseRemainderImpl` and `AdvancementTrigger` retain `Debug`; `ItemStack` gains template diagnostics. `Advancement.action_criteria` and the `ItemUsedOnBlock` / `DefaultBlockUse` trigger variants are added (update exhaustive matches). The legacy `MilkBucketItem` hook is removed. Additive helpers: `item::item_utils::{create_filled_result, give_or_drop}`, `ItemRegistry::records_item_use_stat`, and `beehive::{is_smokey_pos, finish_harvest}`. Native API stays at 8 because no external plugin binaries target this fork's API 8 yet; rebuild against this checkout. Wasm WIT is unchanged.
- API 8 (same version): damage orchestration adds combat ownership and a hurt-animation timer to `LivingEntity`, and a separate hurt-motion flag to `Entity`. These layouts are embedded in native player and mob types. Additive public helpers: `LivingEntity::{knockback, with_damage_owned}`, `Entity::{push_impulse, acknowledge_motion_delivery}`, and `Player::send_hurt_motion`. Health/absorption setters take combat ownership; native and Wasm callbacks release it before dispatch, and resumed damage, equipment wear during hurt processing and weapon effects revalidate the admitted life. `reset_state` and NBT loads abort in-flight hits. `combat::handle_knockback` no longer damps the attacker; melee, spear and mace paths serialize attacker motion separately. `send_velocity` clears pending hurt-motion flags; player melee sends and restores predicted victim motion while hurt-marked. Spear enchantment knockback retains server motion without sending a second impulse to the victim's client. Projectile punch and explosion follow-ups retain victim ownership and life checks; `ExplosionResult` carries player lifecycles through delayed packet delivery (layout change). Interrupted death drops finish delivering removed/generated stacks, and healing a dying life permits a later death. Native plugins must rebuild; event payloads and Wasm WIT remain unchanged.
- API 8 (same version): rabbit review adds a default no-op `Mob::play_attack_sound` hook and changes `AvoidEntityGoal` to retain its admitted escape path. Rebuild native plugins; Wasm WIT is unchanged.

Rabbit hopping review (vanilla 26.3 `Rabbit`): the fork already has hop delays, speed-dependent jump power, jump sounds and event 1 after the impulse. The follow-up restores the constructor's initial zero-speed request, water-avoiding strolls, food-tag-based temptation, and Killer Bunny armor, damage and default name. Cancelling `EntityChangeBlockEvent` when eating carrots now preserves both the crop and the rabbit's appetite. The deep review corrects path-node jump heights, Killer Bunny target ordering and successful-hit sounds, and shared avoidance, home-restricted block searches and solid-render destination admission. This addresses the rabbit portion of [mob AI tracking #3468](https://github.com/Pumpkin-MC/Pumpkin/issues/3468). In-game verification is **Not yet**; native API changes are listed above and Wasm WIT is unchanged.

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

Block interactions returning `Fail` stop before item `useOn`, preserving cancellable plugin harvest/cauldron events; vanilla `ServerPlayerGameMode.useItemOn` continues to item use after a non-consuming block result. This applies to all block `Fail` results in the Java handler.

Bedrock inventory actions still need caller-owned cloned-hand write-back and authoritative validation of client-supplied stacks (independent review finding 11). Java entity interactions still need `isWithinEntityInteractionRange(3.0)` admission (finding 12). Mob capture bucket data and `FILLED_BUCKET` advancement remain a follow-up (finding 4).

Finding 10 does not require a change for 26.3: `CauldronInteractions.bootStrap` calls `PotionContents.is(WATER)`, whose `isPotionWithoutCustomEffects` requires empty custom effects. Preserve that check.

Mobs play-test bugs 2: **Not yet** play tested. The supplied 26.3 `Villager.golemSpawnConditionsMet` checks only `LAST_SLEPT` within 24,000 ticks, not work history; the removed work/bed-ownership and nitwit gates were not vanilla conditions. The requested `ON_TOP_OF_COLLIDER_NO_LEAVES` strategy and whole-body collision check are intentionally stricter than 26.3 `spawnGolemIfNeeded`, which uses `LEGACY_IRON_GOLEM` with `checkCollisions=false`. Placement rejects unloaded terrain instead of loading chunks on a tick worker. Roomy interiors with sufficient clearance remain valid, as in vanilla.

The goal-based villager still lacks the full social behavior packages: the adapter tracks a visible villager interaction partner and only runs the five-villager gossip summon trigger during data-backed IDLE/MEET activities, with vanilla's missing-job-site/meeting-point fallback to IDLE. Recent damage/visible hostiles supply the three-villager panic trigger. One nearby-entity scan per step serves detection, panic and partner selection. Existing Brain storage persists sleep and golem-detection memories, including the remaining TTL across a villager NBT round trip, with the vanilla 599 TTL expiring on tick 600. Summons retain `finalize_spawn_with_reason(MobSummoned)` and cancellable spawn events. Placement scans padded collision cells on every axis, accepts the union of contextual top faces (also for the shared warden strategy), and treats positions outside the dimension's build height as void air.

Creeper power remains 3, or 6 when charged; `ServerExplosion.hurtEntities` (lines 173–186) admits damage through distance 6 or 12 inclusive, with zero knockback at the cutoff. No range multiplier deviation was found.
