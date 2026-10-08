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
| [#3718](https://github.com/Pumpkin-MC/Pumpkin/pull/3718) by KBDL | Fish swim underwater and flop on land; squid and glow squid move with their tentacle strokes. Ported to pumpkin-core and checked against vanilla 26.3. |
| [#3863](https://github.com/Pumpkin-MC/Pumpkin/pull/3863) | Scheduled ticks ran one tick late, ticks saved with a chunk never ran after loading, and observers misbehaved. |
| [#3853](https://github.com/Pumpkin-MC/Pumpkin/pull/3853) | Falling out of the world killed instantly instead of in steps, because void damage skipped the hurt cooldown. |
| [#3904](https://github.com/Pumpkin-MC/Pumpkin/pull/3904) | Sleeping players were not woken when hurt. |
| [#3813](https://github.com/Pumpkin-MC/Pumpkin/pull/3813) | A datapack function that called itself crashed the server with a stack overflow. |
| [#3876](https://github.com/Pumpkin-MC/Pumpkin/pull/3876) | Mace smash damage, knockback and sounds did not follow vanilla. |
| [#3859](https://github.com/Pumpkin-MC/Pumpkin/pull/3859) | Block entity data lingered after its block was removed, so a later block of the same kind could inherit old contents. |
| [#3905](https://github.com/Pumpkin-MC/Pumpkin/pull/3905) | Wind charges could not be thrown at blocks, launched players ever higher, and had no burst effects. |
| [#3845](https://github.com/Pumpkin-MC/Pumpkin/pull/3845) | Entities loaded from disk were frozen, `/forceload` did not keep chunks loaded, attribute changes were lost on reload, and melee knockback ignored the knockback attribute. |
| [#3861](https://github.com/Pumpkin-MC/Pumpkin/pull/3861) | Mobs spawned with equal odds instead of vanilla weights, so rare mobs were as common as zombies. |
| [#3891](https://github.com/Pumpkin-MC/Pumpkin/pull/3891) by Rennex07 | Spawn-potential distances use floating-point arithmetic before subtraction and squaring, avoiding overflow far from the origin. |
| [#3804](https://github.com/Pumpkin-MC/Pumpkin/pull/3804) | Hoppers took dropped stacks one item at a time and never picked items out of their own bowl. |
| [#3348](https://github.com/Pumpkin-MC/Pumpkin/pull/3348) by JulesB40 | Goat horn instrument holders preserve their references and inline definitions. Adapted to the generated instrument registry and vanilla 26.3's durability damage field. |
| [#3897](https://github.com/Pumpkin-MC/Pumpkin/pull/3897) by ToffyMTA | Charged crossbows preserve projectile items and intangible-projectile NBT. Adapted to vanilla 26.3's item templates and 1,024-projectile bound. |

## Upstream issues addressed here

| Upstream issue | Fixed by | Checked in-game |
|:--|:--|:--|
| [#3511](https://github.com/Pumpkin-MC/Pumpkin/issues/3511) Player saves truncate the last good file | durable temporary replacement, backup recovery and retained retries | Not yet |
| [#3512](https://github.com/Pumpkin-MC/Pumpkin/issues/3512) Older snapshots overwrite disconnect saves | ordered snapshots, a tick barrier during disconnect capture/removal and a UUID gate through final publication | Not yet |
| [#3468](https://github.com/Pumpkin-MC/Pumpkin/issues/3468) Aquatic mob AI (fish and squid movement only) | port of upstream PR #3718 | Yes, 2026-10-07 |
| [#3388](https://github.com/Pumpkin-MC/Pumpkin/issues/3388) Arrows have glitchy particles | remove server-generated arrow trails | Not yet |
| [#3520](https://github.com/Pumpkin-MC/Pumpkin/issues/3520) Shields cannot block player melee | `fix(combat): let shields block melee hits` | Yes, 2026-10-07 (blocked a zombie) |
| [#3105](https://github.com/Pumpkin-MC/Pumpkin/issues/3105) Observers don't work | upstream PR #3863 | Not yet |
| [#877](https://github.com/Pumpkin-MC/Pumpkin/issues/877) Naturally generated water sometimes doesn't flow | upstream PR #3863 | Not yet |
| [#3113](https://github.com/Pumpkin-MC/Pumpkin/issues/3113) Goat horns disconnect inventory users | adapted upstream PR #3348 | Not yet |
| [#3108](https://github.com/Pumpkin-MC/Pumpkin/issues/3108) Picking up paintings disconnects inventory users | painting item components encode registry holders | Not yet |
| [#3844](https://github.com/Pumpkin-MC/Pumpkin/issues/3844) Loading a crossbow disconnects its user | adapted upstream PR #3897 | Not yet |
| [#3571](https://github.com/Pumpkin-MC/Pumpkin/issues/3571) Written books lose content | raw readers already merged; saved titles and writable pages now use string codecs | Not yet |
| [#3272](https://github.com/Pumpkin-MC/Pumpkin/issues/3272) Duplicate entity UUIDs | existing spawn reservation reused by loads and commands; each rejection logs once and retains the original | Not yet |
| [#3777](https://github.com/Pumpkin-MC/Pumpkin/issues/3777), [#3319](https://github.com/Pumpkin-MC/Pumpkin/issues/3319), [#3065](https://github.com/Pumpkin-MC/Pumpkin/issues/3065) Malformed particle payloads disconnect clients | malformed 26.3 payloads rejected; eyeblossom, creaking, mooshroom and command senders supply typed options, adapting upstream #3079/#3509 | Not yet |
| [#3382](https://github.com/Pumpkin-MC/Pumpkin/issues/3382), [#3561](https://github.com/Pumpkin-MC/Pumpkin/issues/3561), [#1800](https://github.com/Pumpkin-MC/Pumpkin/issues/1800) Attacking dragons or vanished entities disconnects players | dragon parts resolve while tracked; atomically reserved IDs and silent missing-target returns adapt upstream #3407/#3661/#3583 | Not yet |

## Known regressions under investigation

| Report | Status |
|:--|:--|
| Chest and hopper contents vanished after `stop` and restart (2026-10-07, first start after switching from the pre-harvest build) | Not reproduced in three headless investigations (same build, player-style unload, old-build world opened by new build); container NBT format verified unchanged across the harvest; the owner's later tests (hand-filled chest, autosave, restart with a build switch) kept their items. Kept open; report any recurrence with the build and steps. |

## Fork fixes checked in-game

Fixes with no upstream issue number, and what the owner saw when testing them.

| Change | Checked in-game |
|:--|:--|
| Hoppers keep their facing after a restart | Yes, 2026-10-07 |
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
| Shields respect piercing shots, cooldowns and hand changes; death protectors use their configured effects (combat task 2 review follow-up, related to #3520) | Not yet |
| Disconnect counts games quit once and keeps the saved statistic consistent with plugin changes and the scoreboard | Not yet |

## Native plugin API 6

The native plugin API has moved three times in this fork; rebuild native plugins against this checkout. The Wasm WIT is unchanged throughout.

- API 4: shield and totem integration changed native trait/component layouts. `EntityDamageByEntityEvent.damager_id` now identifies the direct projectile for projectile hits; it previously identified the shooter. Resolve the projectile's owner for player attribution. Mob damage hooks carry separate direct and causing entities.
- API 5: the crash-codec audit changes `InstrumentImpl` from a unit component to registered or inline instrument data.
- API 6: durable storage. `ChunkData.dirty` and `ChunkEntityData.dirty` are `DirtyFlag` instead of `AtomicBool` (carried by the chunk load, save and send events); `ChunkSections.randomly_ticking_mask` is `RandomTickMembership`; `Server` gains `tick_gate` and a session lock and `Player` gains `storage_session` (layout changes); `Server::add_player` is async; `Level::shutdown`, `get_entity_chunk`, `get_or_fetch_chunk` and `get_or_fetch_entity_chunk` return `Result`.

The second shield/totem review adds authoritative item transactions, effect lifecycle corrections, projectile owner handling and teleport destination checks. Play testing remains **Not yet**. Hurt-cooldown admission, full-block damage history and specialized `blockUsingItem`/`blockedByItem` orchestration still require the separate damage-pipeline integration; this branch preserves that boundary.
