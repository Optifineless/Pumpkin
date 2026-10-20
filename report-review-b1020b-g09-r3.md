# PR #330 review round 4

Base: `414022a952ae22a444f8adab607ce948e33957d6`. Branch: `fix-b1020b-g09-r4`.
The requested r3 removal-claim design, native API 8 version and saved-data meanings are retained.

## Independent finding verification, before code changes

One agent per finding verified the following against the required base; a separate tests agent audited the real-entry fixtures and bounded synchronization. The coordinator implements the fixes.

| Finding | Disposition | Evidence at base |
| --- | --- | --- |
| B3: skip-removal-callback flag | CONFIRMED-FIXED | `world/decorated_pot_break.rs:277` unconditionally invokes the inventory callback although flags were forwarded at `:195-202`; `pumpkin-world/src/world.rs:34-36` documents moves. |
| B4: piston ignores rejected claim | CONFIRMED-FIXED | `world/mod.rs:5042-5043` returns claim rejection as `None`; `block/blocks/piston/piston.rs:406-410` discards it and writes moving terrain at `:423-427,453-457`. |
| Owner 1: Bedrock placement actor | REFUTED | Already fixed: `block/blocks/decorated_pot.rs:51-66` sends post-application actor data; real second-observer regression `net/bedrock/play/inventory_action_pot_placement_tests.rs:29-118`. |
| Owner 2: projectile THIS_ENTITY loot | REFUTED | Already fixed: `world/decorated_pot_break.rs:71,176-183` retains actual projectile type/state; projectile cause is `None` and does not borrow shooter tool. Regression `block/blocks/decorated_pot_projectile_context_tests.rs:40-54`. |
| Owner 3: non-arrow hit cancellation | REFUTED | Already fixed: `entity/projectile/hit_event.rs:6-16`, `trident.rs:309,324-329`, `collision.rs:95-96,151-170`. Real egg/trident tick regressions `decorated_pot_projectile_context_tests.rs:161-197`. |
| Owner 4: empty-pot shattering | PRE-EXISTING-BACKLOG | `block/blocks/decorated_pot.rs:166-183` and empty tests retain deliberate compatibility, documented `FORK.md:289-294,364-365`. Identical guard predates #330 (`080cf4274:decorated_pot.rs:141-148`, original #13 `3e4ab0d7d:124-131`). Changing it would alter old-save behavior. |
| Owner 5: successful BlockDestroy event | REFUTED | Already fixed: `world/decorated_pot_break.rs:72-86` emits only after success; real regression `decorated_pot_projectile_context_tests.rs:72-117`. Existing API has no event source (`plugin/api/events/world/generic_game.rs:7-12`, `world/mod.rs:6646-6653`). |
| Owner 6: pre-removal world visibility in drop events | PRE-EXISTING-BACKLOG | Removal at `world/decorated_pot_break.rs:157,238-239` precedes drop callbacks at `:185,264`; event has no snapshot (`plugin/api/events/block/block_drop_item.rs:11-15`). Same ordering exists in `fd5ac1f40` at `:87,96,144-145`. Snapshot/in-flight redesign conflicts with the requested preserved claim design. |
| Owner 7: pending-only face stock | REFUTED | Already fixed: `world/decorated_pot_break.rs:229-235` rejects pending-only stock before removal/loot. Real invalidated-load regression `block/blocks/decorated_pot_pending_hit_tests.rs:5-61`; the exact break-event publisher scenario is protected by the same explicit guard. |
| Owner 8: single-flight before break admission | PRE-EXISTING-BACKLOG | `world/mod.rs:5028-5043` still fires the break event before claiming, also present in `fd5ac1f40:world/mod.rs:5027-5042`. Only one removal publishes stock. Pre-event single-flight would prevent the explicitly required nested-arrow winner; cancellation/reentry semantics need separate design. |

Rust paths above are under `crates/pumpkin-core/src/` unless another crate is named. The last two backlog classifications follow the brief's scope and design instructions; FORK does not literally label them BACKLOG. All three are deliberately unchanged.

## Vanilla and prior-work checks

Downloaded Mojang 26.3 server object `33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c` from its version manifest and decompiled only relevant classes with Vineflower 1.12.0 outside the repository. `LevelChunk.setBlockState:310-322` suppresses `preRemoveSideEffects` with its side-effects flag while removing the entity regardless. `PistonBaseBlock.moveBlocks:283-321` destroys and drops original stock before moving blocks/arm; the fork additionally must respect its cancellable/reentrant removal result. `Level.destroyBlock:278-296` and `DecoratedPotBlock.playerWillDestroy/onProjectileHit:188-215` confirm the existing r3 loot/event behavior and the separately documented EMPTY divergence.

Upstream open-PR search found adjacent #3859 (entity NBT removal), #3507 (loot components), and #3728 (component encoding), with no equivalent skip-flag/piston-rejection fix. Existing fork PR #330 is the work being updated.

## Implemented changes and caller audit

- B3: `world/decorated_pot_break.rs:278-280` checks `SKIP_BLOCK_ENTITY_REPLACED_CALLBACK` only around the retained callback. Terrain/live/pending/cache retirement and inventory-owner retirement remain unconditional. Projectile and explosion entrypoints supply ordinary `NOTIFY_ALL` flags and share this same finish function; their normal contents scatter is unchanged.
- B4: `world/neighbor_updater.rs:263-274` adds an internal helper that propagates rejection and also rejects a callback replacement after an otherwise successful break. `block/blocks/piston/piston.rs:408-437` aborts before pushed-block/arm writes and revalidates every destruction cell after all break callbacks.
- A failed extension also cancels only its matching queued retry (`world/block_placer.rs:15-32`, `piston.rs:172-176`). Arrow-removal notifications can queue that retry against intermediate air; `flush_synced_block_events` runs newly queued events in the same flush. Other positions, event types and data remain queued.
- `/setblock destroy` (`command/commands/setblock.rs:50-56`) fails on rejected nonair destruction; `/fill destroy` (`command/commands/fill.rs:134-140`) skips that cell. Air placement remains permitted. Redundant command-wide mutation permits were removed: break/state mutation and `block_input::place` retain their own admission, with no command permit spanning break callbacks.
- Fluid spread (`block/fluid/flowing_trait.rs:338-359`) returns on rejected destruction and rechecks terrain after its subsequent flow event.
- End-portal interior placement already checks `None` (`world/portal/end.rs:95-100`). Java/Bedrock digging, respawn anchors and neighbor destruction either respect rejection or perform no following terrain setter. Other production break callers only destroy, publish unrelated effects or use separate branches.
- Bucket emptying cannot enter this pot-removal claim in its admitted serial path: all 16 extracted pot states are solid/nonreplaceable; water uses the liquid-container branch, lava is rejected, and powder snow accepts only air (`item/items/bucket.rs:316-329,381-404`). Bucket source pickup begins with fluid/powder snow, not a pot (`:219-240`). Its pre-existing unrelated cancellation behavior is unchanged.
- The end-portal caller's successful-break/drop-callback replacement behavior also predates this fix; it already respects failed destruction and is left outside this rejected-claim change.

No new lock instances, lock classes, lock-order edges, fields or public layouts/signatures were added. Retry cancellation uses the existing queue mutex only for immutable event comparisons. The conditional claim ends its publication gate and mutation permit before loot/events/callbacks/sends. No saved formats or saved-data meanings changed. No native API bump, Wasm WIT change, deployment or PR creation.

## Regressions and recorded negative controls

`decorated_pot_removal_tests.rs` exercises public `World::break_block` with both skip flags, restores the captured NBT at another position, verifies exactly one stored diamond, and then normally breaks the destination to verify one contents scatter and one original face set.

Its powered-piston test admits an already-launched arrow inside the outer noncancelled `BlockBreakEvent`, installs a new uncracked pot with HEART/SHEAF/EXPLORER/BLADE faces and an emerald, and checks original stock once plus intact replacement stock and a retracted piston. It uses the actual redstone/synced-event path and a five-second completion channel. Command and production fluid-tick regressions exercise the same rejected-claim order. The existing piston-success test remains; the existing successful-break/drop-callback replacement test now includes Piston too. No sleep-based synchronization was added.

Before any production fix, the new tests were built against unchanged r3 production code and run:

- `cargo test -p pumpkin-core --lib projectile_tests::removal:: -- --nocapture`: exit 101, 0 passed / 4 failed. B3 observed one ground diamond instead of zero. Piston and fluid observed one ground emerald instead of zero after the replacement was consumed. The command case reached the real Setblock break and incorrectly accepted placement; the worker assertion failed (the test reports its disconnected bounded completion channel). The combined command negative control stops at Setblock; its Fill iteration is exercised by the positive run.
- `cargo test -p pumpkin-core --lib b1020b_g09_mixed_drop_callback_replacement_survives -- --nocapture`: exit 101, 0 passed / 1 failed after adding Piston; one replacement emerald was scattered instead of staying stored.

An additional negative control retaining the callback/caller fixes but omitting retry cancellation passes 41/42 tests: the required piston case sees three break admissions instead of two. The arrow queues a second extension against intermediate air; the same flush then breaks the replacement. Matching queued-retry cancellation closes that path without weakening the regression.

Independent read-only review of both fixes and their tests found no blocking issue.

## Validation

All Cargo commands use the provided Rust 1.99.0 toolchain (`PATH=/workspace/.cargo/bin:$PATH`, `CARGO_HOME=/workspace/.cargo`, `RUSTUP_TOOLCHAIN=/workspace/.rustup/toolchains/stable-x86_64-unknown-linux-gnu`). Checks cover this source tree; the exact committed head is verified before push and recorded in the handover. No whole-workspace test suite was run.

| Command | Result |
| --- | --- |
| `cargo test -p pumpkin-core --lib b1020b_g09` | PASS: 42 tests, including all four new tests and the expanded piston replacement case. |
| `cargo test -p pumpkin-core --lib b1019_g03` | PASS: 14 compatibility tests. |
| `cargo test -p pumpkin-core --lib placement_regression_` | PASS: 6 command-placement tests. |
| `cargo test -p pumpkin-core --lib piston` | PASS: 9 tests, including retired-chunk movement and both pot piston regressions. These filters overlap; counts are not unique-test totals. |
| `cargo test -p pumpkin-core --test lifecycle_coverage_scan --test sync_guardrail_scan` | PASS: 46 lifecycle tests and 34 guardrail tests. |
| `RUSTFLAGS=-Dwarnings cargo clippy -p pumpkin-core --all-targets` | PASS. |
| `cargo fmt --all -- --check` | PASS. |
| `SIZE_CHECK_BASE=origin/murgicraft python3 .fork/check-sizes.py` | PASS: 3,462 files, zero failures. Explicit positional `origin/murgicraft` verification gives the same result. Two existing warnings concern untouched Wasm `generated_packets.rs` files growing by two lines against their baseline. |
| `cargo build -p pumpkin` | PASS, including all native/Wasm host dependencies. |
| `RUST_LOG=info /workspace/Pumpkin/target/debug/pumpkin`, scratch working directory, then stdin `stop` | PASS: startup marker in 5,824ms, clean save/shutdown, exit 0. Only expected debug-build, development and non-TTY warnings; no errors or panics. |

Cargo also reports the existing future-incompatibility notice for third-party `proc-macro-error2 v2.0.1`; no new compiler/clippy warnings were added.

The initial lifecycle scan identified only changed `pumpkin-core/src/command` and `pumpkin-core/src/world` family fingerprints. `LIFECYCLE_MANIFEST_WRITE=1 cargo test -p pumpkin-core --test lifecycle_coverage_scan` refreshed the observed hashes, after which every original status and note was restored. The committed manifest changes only those two hashes; there are no new, removed or moved rows and no status upgrades. Both scans then pass without the writer flag.

## Files changed

- `FORK.md`: round-4 behavior, unchanged API/save meaning and owner play-test checklist.
- `crates/pumpkin-core/src/world/decorated_pot_break.rs`: honor skip-removal-callback flag.
- `crates/pumpkin-core/src/world/neighbor_updater.rs`: internal destruction-for-replacement result check.
- `crates/pumpkin-core/src/world/block_placer.rs`: exact queued-event cancellation.
- `crates/pumpkin-core/src/block/blocks/piston/piston.rs`: abort/revalidate before movement and discard matching failed-move retry.
- `crates/pumpkin-core/src/command/commands/setblock.rs` and `fill.rs`: respect rejected destruction and release command-wide permits.
- `crates/pumpkin-core/src/block/fluid/flowing_trait.rs`: respect rejection and flow-event replacement.
- `crates/pumpkin-core/src/block/blocks/decorated_pot_removal_tests.rs`: new move/piston/command/fluid regressions.
- `crates/pumpkin-core/src/block/blocks/decorated_pot_mixed_break_tests.rs`: reuse real piston setup, preserve original movement assertion and cover successful-break callback replacement.
- `crates/pumpkin-core/src/block/blocks/decorated_pot_projectile_tests.rs`: wire the new test module.
- `crates/pumpkin-core/tests/lifecycle_coverage_manifest.txt`: two family hashes only.
- `report-review-b1020b-g09-r3.md`: this requested round-4 report.

## Owner play test

In-game verification remains **Not yet**. Use a real client before updating PR #330 and attach a recording:

1. Move a diamond-filled, decorated pot with a plugin using both skip flags. Expect no ground diamond at the source and one stored diamond at the destination. Break the destination normally; expect its contents once and its faces preserved in its pot drop.
2. With the callback test plugin, let an arrow win during a powered piston's outer break and install the distinctly decorated emerald-filled replacement. Expect the piston retracted, one original face set and diamond on the ground, and the replacement pot/emerald intact.
3. Repeat ordinary piston, player, arrow and explosion destruction. Expect one complete original face set and one contents stack. Capture the original and replacement pot views and collected drops.

No real-client rendering/play test or capture is claimed. No PR was opened and nothing was deployed.
