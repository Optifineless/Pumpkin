# Murgicraft fork of Pumpkin

This is a fork of [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) maintained for the Murgicraft server. It tracks upstream `master` and adds fixes aimed at running a survival server: redstone, combat, world generation, commands and the plugin API.

## How the code here is written

The changes in this fork are written by AI coding agents, not by hand. The fork owner tests them in-game but does not review the code line by line. Treat every commit that isn't from upstream accordingly.

Each change is still held to upstream's [AGENTS.md](AGENTS.md) rules: it is ported from the decompiled vanilla source, stays one topic per commit, passes `cargo fmt`, `clippy` and the tests, and comes with a regression test where one can catch the bug. That keeps the commits small enough for upstream to take any of them if they want to.

None of this is sent upstream as pull requests. Upstream maintainers are welcome to cherry-pick anything they find useful.

## Staying in sync

Upstream is merged in regularly. When upstream fixes something this fork also fixed, the fork drops its own version and keeps upstream's.

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
| [#3902](https://github.com/Pumpkin-MC/Pumpkin/issues/3902) Chest contents disappear after unload or stop | retain terrain through live block-entity snapshots and durable publication; cancel obsolete cleanup on rewatch | Not yet |
| [#3758](https://github.com/Pumpkin-MC/Pumpkin/issues/3758) Execute store rejects valid commands | score, bossbar, block, entity and storage result callbacks | Not yet |
| [#3779](https://github.com/Pumpkin-MC/Pumpkin/issues/3779) Setblock rejects block-state data | upstream PR #3827 approach with SNBT support | Not yet |
| [#3502](https://github.com/Pumpkin-MC/Pumpkin/issues/3502) Hardcore does not force Hard | startup difficulty override | Not yet |
| [#3538](https://github.com/Pumpkin-MC/Pumpkin/issues/3538) Gamemode feedback uses wrong variables | upstream PR #3642 | Not yet |
| [#3760](https://github.com/Pumpkin-MC/Pumpkin/issues/3760) Tick sprint prints wrong start message | upstream PR #3763 | Not yet |
| [#3874](https://github.com/Pumpkin-MC/Pumpkin/issues/3874) Fill cannot create snow golems | synchronous BlockInput placement through existing block callbacks | Not yet |
| [#3276](https://github.com/Pumpkin-MC/Pumpkin/issues/3276) Reload runs load functions while frozen | upstream PR #3394 approach | Not yet |
| [#3503](https://github.com/Pumpkin-MC/Pumpkin/issues/3503) Hardcore death cannot enter spectator mode | adapted upstream #3506, normal saved respawn followed by cancellable spectator transition | Not yet |
| [#3614](https://github.com/Pumpkin-MC/Pumpkin/issues/3614) Armor stand item interactions | adapted upstream #3684 with actual-hand writes, adjusted clicks, disabled slots and equipment-drop fixes | Not yet |

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
| [#110] Charged respawn anchors set a Nether spawn or explode in unsafe dimensions when used without glowstone | Not yet; positional `RESPAWN_ANCHOR_WORKS` environment-attribute overrides are not evaluated |
| [#109] Firework Stars craft with colors, shapes and effects, can receive fade colors, and can be used in rockets | Not yet |
| Hoppers keep their facing after a restart | Yes, 2026-10-07 |
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
| Ender Dragon respawn explosions run after the fight lock is released, and crystals destroyed while it is held are queued, so the respawn sequence no longer freezes the server (Singapore hang 2026-10-10, PR #122) | Not yet |

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

## Native plugin API 8

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
- API 8 (same version): chunk unload hooks carry a lifecycle generation and cancelled snapshots release their retained entity identities. Entity gains identity admission that follows outstanding writers across moves. Native chunk custom-data writes use `Level::set_retained_chunk_custom_data` / `remove_retained_chunk_custom_data`, which reject stale Arcs. Writes through canonical loaded handles cancel a pending unload and are resaved. Other native retained-chunk writes must hold `try_mutate_retained_chunk`; synchronous raw entity state writes hold the borrowed `Entity::try_begin_mutation` permit; asynchronous callers transferring their entity handle hold `Entity::try_begin_owned_mutation` through completion. The public `vehicle` and `passengers` fields are read-only to callers: all writes use the mount/dismount methods or unpublished-tree link helpers to maintain the riding generation. World writes use `set_world` to maintain the world generation. Rebuild native plugins for the changed Entity/World/Level layouts and portal vtable; the Wasm WIT remains unchanged.

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

Block interactions returning `Fail` stop before item `useOn`, preserving cancellable plugin harvest/cauldron events; vanilla `ServerPlayerGameMode.useItemOn` continues to item use after a non-consuming block result. This applies to all block `Fail` results in the Java handler.

Bedrock inventory actions still need caller-owned cloned-hand write-back and authoritative validation of client-supplied stacks (independent review finding 11). Java entity interactions still need `isWithinEntityInteractionRange(3.0)` admission (finding 12). Mob capture bucket data and `FILLED_BUCKET` advancement remain a follow-up (finding 4).

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
Chunk lifecycle/piston lease composition: this lane owns generation cancellation, retained snapshots, durable publication and detachment. The piston lane must acquire per-position lifecycle admission before its global lease mutex, validate canonical handles and cancel obsolete unloads for writes, then publish its lease counts. Install the short `ChunkLifecycles::set_unload_gate` check (`can_begin_unload(pos)`) to defer snapshot admission while leased. Release the global lease mutex before returning from that check; never carry it through serialization, block updates or plugin callbacks. Multi-chunk operations own per-position counters, never several `at(pos)` mutexes at once; ordering positions does not authorize nested lifecycle locks. A stack-held `try_mutate_retained_chunk` permit also serves as a lease. Release lease counts before releasing admission. Gating final unpublication alone is insufficient.

Known test race: `data::datapack::structure_loader::tests::load_structure_template_and_template_pool` uses the process-global template-pool registry. Parallel `DatapackManager::load_all` tests can clear it between registration and discovery; serial test execution avoids this pre-existing race. Registry isolation or shared synchronization belongs in a separate change.


## Chunk lifecycle review, follow-up 3

In-game verification: **Not yet**. Ordinary mutation admission uses a per-position atomic counter and a closing flag. Block-entity add/update/remove, NBT publication, retained-handle writes and pending block-entity promotion also take the lifecycle mutex; air lookup misses do not. Synchronous tick setters check the closing flag under the tick read barrier without incrementing counters, including scoped Rayon callbacks. Entity identity counters remain counted across changes of world. Tick-scoped permits must end before the read barrier; asynchronous writers acquire outside that scope and remain counted. Entities retain up to two admission cells from the same level; border oscillation reuses them, and world changes replace them. Retrying a stale completed save keeps admission closed through the next snapshot gate, including a writer delayed between its first flag read and its second. Generations come from one counter shared across levels and retired/recreated cells. Lookup never creates lifecycle entries for never-loaded positions. Rewatch releases the stale entity snapshot immediately.

Unload preparation and completion retry at the start of `World::tick`, before the live tick read guard. This mirrors vanilla `ServerChunkCache.tick` / `ChunkMap.processUnloads`, plus `PersistentEntitySectionManager.processUnloads`. The drain reserves 5 ms and forces the excess over 2,000 queued requests when time expires. A bounded admission pass selects at most the forced excess plus 32 terrain requests; indexing and one atomic cleanup operation can overrun the allowance. Positions close before the shared root index is built; active writers defer indexing and later writes invalidate its generation. One root index and custom-data index serve a drain, and entity-list removals publish together. Still-saving requests skip the ticket mutex and root indexing. Continuous simulated 50 ms ticks must finish a queued unload within 20 tick boundaries; shutdown cancels entity-only boundary waits. Unread prepared replies own their cancellation guard, and invalidated saves remain scheduled for retry. Completed or deliberately refused cleanup does not request another unload. Never call `begin_chunk_mutation` while holding `at(pos)` for that same position; acquire admission first.

The Wasm WIT is unchanged, and Native plugin API stays 8. Setters through removed entity handles and replaced but still upgradable terrain handles complete successfully without changing state. Canonical retained terrain writes cancel unloading and are saved. Expired weak terrain handles retain the existing `Chunk unloaded` trap; noncanonical native retained handles still fail validation. No new guest trap is introduced for removed entities, replaced upgradable chunks or quiescing loaded terrain.

The prior follow-up verified 23 positive regression scenes and 28 failing full negative controls. Of 22 disk/live-set-only controls, nine failed: captured identity, retained-quiescing refusal, cancelled cleanup retry, terrain detachment order, busy-tick terrain and entity-only queue admission, unread prepared replies, invalidated cleanup retry and tick-boundary draining. In those scenes the other 13 controls were backed by remaining guards: spawn/move/passenger/state/removal admission, retained invalidation/canonical validation, rewatch generation/captured-snapshot cancellation, the lease gate, identity admission after movement, successful-cleanup retry and shutdown cancellation. All protections remain; endpoint redundancy does not establish that a guard is unnecessary. The historical performance figures below used fixed seed 0, preset terrain, two Rayon workers, 200 movement-only pig entities in one chunk, 64 circulating hoppers and a fixed 16-tick clock driving 32 lamps. It excludes random AI, crowding, networking and generation workers; its logical-tick measurements are not whole-server MSPT. Admission off is a same-tree proxy for `ea5598963`, not a separate build of that commit. Those historical runs disabled release LTO and used 16 codegen units. Each historical result is the median of five 1,000-tick samples after 200 warmup ticks.


| Preset scene | Admission off, before / after (ns per logical tick) | Branch before (ns) | Branch after (ns) |
|:--|--:|--:|--:|
| 200 pig movement setters, one chunk | 136,583 / 125,166 | 290,377 | 164,451 |
| 64 circulating hoppers | 6,934 / 5,548 | 7,005 | 6,945 |
| Clock driving 32 lamps | 2,769,561 / 2,517,253 | 2,741,281 | 2,416,883 |

These historical measurements used sequential modes, five samples and disabled LTO, so they do not establish a performance improvement. The less-than-1% hopper comparison was before/after (7,005 vs 6,945 ns), not admission on/off: the latter was 6,945 vs 5,548 ns, about +25% with overlapping ranges. The lamp median and admission-off baseline both shifted. Background load varied; an earlier post-fix movement median was 205,923 ns. Historical logs: `briefs/lane-e-admission-before.log` and `briefs/lane-e-admission-after-final.log` outside this checkout.

Follow-up 6 uses the production profile, fat LTO and one codegen unit with the same package overrides as follow-up 5. Fifteen samples per mode each warm for 200 ticks and time 320 ticks. Off, the actual `ea9bdfa10` baseline executable and Final rotate within every round at two and eight workers. The baseline is the verified follow-up 5 final executable, rerun for this comparison; historical samples are not reused. Off and Final use the new final executable. Values are median / minimum nanoseconds per logical tick, not whole-server MSPT. Ratios divide Final by Off within each round, then report the median and range of those 15 ratios. Every sample is retained. Background work in other lanes shares this PC.

| Scene | Workers | Off median / min (ns) | ea9bdfa10 median / min (ns) | Final median / min (ns) | Paired Final / Off median (range) |
|:--|--:|--:|--:|--:|--:|
| 200 pigs, one chunk | 2 | 126,725 / 123,485 | 138,377 / 130,061 | 132,898 / 130,211 | 1.0517 (0.9281–1.0973) |
| 64 circulating hoppers | 2 | 4,985 / 4,172 | 5,198 / 4,391 | 5,553 / 4,291 | 1.0835 (0.9037–1.4549) |
| Clock / 32 lamps | 2 | 1,640,490 / 1,612,968 | 1,623,338 / 1,593,970 | 1,691,016 / 1,603,010 | 0.9990 (0.9763–1.0739) |
| 200 pigs, crossing borders | 2 | 144,660 / 139,680 | 163,619 / 155,777 | 155,829 / 150,592 | 1.0883 (1.0027–1.2450) |
| 200 pigs, one chunk | 8 | 126,391 / 103,021 | 129,637 / 106,902 | 130,430 / 108,155 | 1.0326 (0.8090–1.2361) |
| 64 circulating hoppers | 8 | 6,674 / 6,073 | 7,424 / 6,698 | 7,736 / 7,024 | 1.1670 (0.9294–1.9890) |
| Clock / 32 lamps | 8 | 1,738,011 / 1,714,784 | 1,736,169 / 1,637,583 | 1,760,769 / 1,714,289 | 1.0080 (0.9713–1.0642) |
| 200 pigs, crossing borders | 8 | 132,075 / 127,619 | 138,890 / 134,842 | 135,036 / 131,403 | 1.0274 (0.5395–1.4178) |

The isolated slot comparison keeps the final world-generation, registry-purge and neighbour fixes in both executables, changing only the two cached-cell access sites. Benchmark-only flag probes are identical. Guard/candidate order rotates over 15 paired rounds, with 200 warmup and 320 timed ticks:

| Scene | Workers | Slot guards median (ns) | Candidate median (ns) | Paired Candidate / guards median (range) |
|:--|--:|--:|--:|--:|
| 200 pigs, one chunk | 2 | 135,933 | 131,818 | 0.9609 (0.9270–0.9961) |
| 200 pigs, crossing borders | 2 | 160,805 | 154,290 | 0.9538 (0.5563–1.0222) |
| 200 pigs, one chunk | 8 | 135,155 | 135,194 | 1.0007 (0.8526–1.1838) |
| 200 pigs, crossing borders | 8 | 138,419 | 135,282 | 0.9917 (0.7952–1.0250) |

The cheaper slot check is retained based on the two-worker paired medians. The eight-worker one-chunk result is effectively flat. These are measured point estimates from the preset fixtures. They do not establish a general server or unload-latency speedup. Paired ratios for every individual round, raw samples, executable hashes and profile commands are in `briefs/lane-e-followup6-production-paired.md`, `lane-e-followup6-production-samples.json`, `lane-e-followup6-slot-paired.md` and `report-concurrency-3-followup-6-lane-e.md` outside this checkout. Test-only benchmark probes compile out of ordinary production builds.

Follow-up 6 brackets serialized world stores with odd/even generations and uses plain generation loads during admission revalidation. Registered caches retain admission state and use a final SeqCst validity check; counted readers prevent retirement, while tick readers retain the tick barrier. Quiescence clears validity before releasing canonical-cell ownership, so cleared caches cannot admit through retired cells after a reopen. Registration still holds the lifecycle mutex to serialize with clearing; dead registrations are purged only when the list doubles. Never admit an entity while holding any lifecycle mutex. Refilling a cleared current slot preserves its actual neighbour. Retirement is crate-visible, with test hooks; the left-chunk regression executes the scheduler's own retirement path. In-game verification remains **Not yet**.

Follow-up 6 validation passed 1,168 core tests, 334 world tests and five tests in each Wasm host version (1,512 total); 13 unit tests and one doctest were ignored, and the two task-named tests were skipped. Formatting, all-target/all-feature Clippy with denied warnings, size checks (zero errors) and whitespace checks passed. All seven section-membership regressions remain passing. Five targeted regressions have failing rollback controls, including scheduler retirement, stale cache admission, world-change retry, amortized registry purging and neighbour preservation. Temporary controls and the untimed fixture selector are absent from repository sources. No server or real client was started. Upstream PR lookup remained unavailable because the configured proxy refused the connection.

The earlier border diagnosis recorded 4,000 cache replacements and 4,000 vector paths over 20 ticks before the two-cell fix, then zero of each after warmup with identity admissions unchanged at 16,000. Those follow-up 4 counters compared the one-cell/vector path with the two-cell/inline path. The cache-reuse regression failed in that earlier rollback and remains passing. Follow-up 5 replaces riding mutex reads with a generation checked by plain loads, borrows identity admission for ordinary setters, shares the cache load across source and destination, and releases registered cached-cell ownership when quiescence starts. Live cache readers still prevent canonical-cell retirement. Async handle transfers retain an owned identity permit through `try_begin_owned_mutation`. A tick-wide permit was not introduced because destinations, riding trees, and world changes require per-setter admission and revalidation.

Follow-up 5 debug validation passed 1,166 core tests, 332 world tests, and five tests in each Wasm host version (1,508 total); 13 unit tests and one doctest were ignored, and the two task-named tests were skipped. Final formatting, all-target/all-feature Clippy with denied warnings, the size check (zero failures), and whitespace checks passed. Cargo retains its existing proc-macro future-compatibility notice. Seven focused release regressions passed. Seven distinct regressions have failing rollback controls (eight failed control executions), including a mount paused between admission checks, A to B with no subsequent setter while A unloads, current-only cache validation, and owned identity transfer to another world. All rollback edits and the untimed fixture selector were removed from repository sources. Raw samples, executable provenance, helper contracts, commands and final validation are in `briefs/lane-e-followup5-production-bench.txt`, `lane-e-followup5-production-samples.json` and `report-concurrency-3-followup-5-lane-e.md` outside this checkout. In-game verification remains **Not yet**.

The historical follow-up 3 unload fixture interleaves three before/after pairs, dropping 512 real terrain chunks with 512 pigs, a simulated 45 ms guarded workload and a 50 ms cadence. Budgeting lowers the largest drain from 19.852–26.446 ms to 2.813–3.370 ms in these runs, while all-chunk unload latency rises from 1.060–1.631 s to 2.241–2.528 s. These are fixture timings, not server play tests. The release artifact predates the final entity-only index-overrun progress fix, missing-terrain custom-data guard and identity-counter restoration; those edge cases are not exercised by these terrain timings.

| Round | Mode | All 512 unload (ms) | Boundaries | Median / max fixture MSPT (ms) | Median / max drain (ms) |
|--:|:--|--:|--:|--:|--:|
| 0 | Before | 1060 | 20 | 45.289 / 71.446 | 0.288 / 26.446 |
| 0 | After | 2241 | 41 | 46.820 / 47.814 | 1.818 / 2.813 |
| 1 | After | 2506 | 46 | 46.909 / 48.371 | 1.908 / 3.370 |
| 1 | Before | 1522 | 29 | 45.308 / 64.852 | 0.308 / 19.852 |
| 2 | Before | 1631 | 31 | 45.327 / 67.552 | 0.326 / 22.551 |
| 2 | After | 2528 | 46 | 46.719 / 48.029 | 1.719 / 3.028 |

Fifteen distinct follow-up regressions passed, and 16 isolated rollback controls failed, including retained-cell missing-terrain writes and progress when indexing consumes the budget. Temporary controls were removed. Raw 15-sample sets, release command, test summaries and limitations are recorded in `briefs/lane-e-followup3-production-bench.txt` and `briefs/report-concurrency-3-followup-3-lane-e.md` outside this checkout.

Follow-up 4 clean continuation checks passed on the preserved final implementation: formatting, six-package Clippy with warnings denied, and one completed combined Cargo test run. Unit results: core 1,162 passed and 13 ignored, world 331 passed, and each of the three Wasm hosts 5 passed; only the two requested core tests were skipped. In total 1,508 unit tests passed with zero failures. World's one doc test is ignored. The size check covered 3,107 files with zero failures and three existing warnings; whitespace checks passed. The clean run includes border cache reuse, chunk/world cache refresh and cross-world identity protection. Exact commands, completed logs and the restart inventory are in `briefs/report-concurrency-3-followup-4-lane-e.md` outside this checkout. In-game verification remains **Not yet**.
