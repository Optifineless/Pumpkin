# Issue #413 validation

Base: `origin/murgicraft` at `dec07fc010174173b999473240e6028bf73964b7`.
The [inventory](stale-block-write-inventory.md) enumerates original call lines and
applied changes; [the original audit](stale-block-write-original-inventory.md)
keeps per-function baseline observations and exemptions.

## Recorded negative controls

A detached checkout of the base received only the new regression module and its
module registration. Running
`cargo test -p pumpkin-core --lib block::blocks::stale_block_write_tests -- --nocapture`
produced `FAILED. 1 passed; 6 failed; 0 ignored` (18.84 seconds).

| Real entry point / interference | Origin result |
|---|---|
| Single copper chest random tick / Java FinishedDigging | Fails: stale oxidation resurrected the mined single copper chest |
| Double chest left half / Java FinishedDigging | Fails: stale oxidation resurrected a mined double-chest half |
| Double chest right half / Java FinishedDigging | Fails: stale oxidation resurrected a mined double-chest half |
| Copper trapdoor random tick / Java FinishedDigging | Fails: stale oxidation resurrected the mined copper trapdoor |
| Copper door random tick / Java FinishedDigging | Fails: stale oxidation resurrected the mined copper door's lower half |
| Wheat random tick / Java FinishedDigging | Fails: stale crop growth resurrected the harvested wheat |
| Uncontested single/double copper chest oxidation | Passes |

The existing origin control
`axe_review_java_partner_replacement_survives_transform` also passed alone
(1 passed, 6.23 seconds). The fixed new module first passed all seven tests in
26.62 seconds; final-tree results are recorded below.

The old upper-door mining path gives zero door items, because lower-half cleanup
uses `SKIP_DROPS` before the broken hook. The regression preserves that baseline
count while requiring both halves air. Lower-half mining still yields one item.

## Review and compatibility

Reviewed official Java 26.3 with Vineflower 1.12.0, including
`ChangeOverTimeBlock.changeOverTime`, copper chest/door random ticks and shape
updates, `CropBlock.randomTick/growCrops`, `BlockTransformer.transformBlock`,
`DoorBlock.setOpen/neighborChanged`, `Level.setBlock/destroyBlock`, and piston
movement/finalization. Inner server JAR SHA-256:
`a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`.

Independent review identified and resolved delayed block-entity retirement,
structure/frame replacement ownership, pre-removal loot/XP plugin context, and
paired bed admission. The later whole-class review also converted 57 delayed block-entity
writes across 49 files to atomic terrain-and-registry publication: typed placement,
structure/feature NBT (including unchanged-terrain NBT), clone/data/sign commands,
piston movement and command-block packets retain their original expected state.
Chest connection rechecks its original source after registration callbacks; crafter
placement and neighbor scheduling each publish exactly one tick after admission.
Existing decorated-pot and explosion-banner regressions now
assert that the plugin sees the original block and entity. Loot prepared before
commit is published only after successful removal; changed typed contents are
reevaluated from the exact retired owner, preserving cancellation. No owner or
mutation permit crosses those callbacks. Final independent review had no
unresolved concrete finding; remaining 22 raw sites are documented exemptions.

Related upstream PR [#3465](https://github.com/Pumpkin-MC/Pumpkin/pull/3465)
guards block-entity property decoding. It does not make the subsequent terrain
write conditional or cover this task’s paired writes, loot admission, and full
caller inventory. The handover search checked 300 open PR titles plus targeted
stale-block and oxidation searches; no complete equivalent was found.

## Final checks

All final targeted tests use `CARGO_PROFILE_DEV_OPT_LEVEL=1` and one test thread,
executing the final three-package core test binary by filter. The required final
full Cargo run uses two test threads. The `weathering` filter has no matches.

| Filter | Result |
|---|---|
| `block::blocks::stale_block_write_tests` | 7 passed |
| `weathering` | 0 matches; real copper random ticks covered by the new module |
| `chest` | 70 passed |
| `axe` | 14 passed, including #256's named control |
| `block_transformer` | 13 passed |
| `piston` | 7 passed |
| `world::bed_use::tests::round3` | 2 passed |
| decorated-pot original-context regression | 1 passed |
| explosion-banner original-context regression | 1 passed |
| structure/feature NBT admission | 4 passed |
| portal frame pair cleanup | 6 passed |
| command-block paths | 7 passed |
| sign loading | 5 passed |
| crafter | 2 passed |
| data commands | 3 passed |
| command capture window | 3 passed |

`cargo test -p pumpkin-core --test lifecycle_coverage_scan --test sync_guardrail_scan`
passed 46 lifecycle and 34 synchronization tests. The manifest refresh was needed
for changed family hashes, moved keys, and two new conditional-write boundaries. All
92 existing statuses/notes remain; both new rows are `BoundaryCheckedLegacy`.
No synchronization allowlist was regenerated.

`RUSTFLAGS=-Dwarnings cargo clippy -p pumpkin-core -p pumpkin-world -p pumpkin-wasm-host-v0_1 -p pumpkin-wasm-host-v0_2 --all-targets`
passed. `cargo fmt --all -- --check` and `git diff --check` passed.
`SIZE_CHECK_BASE=origin/murgicraft python3 .fork/check-sizes.py` passed:
`checked 3640 files: 0 failure(s), 2 warning(s)`. The warnings are existing
3907-line generated Wasm packet files (the size baseline says 3905); neither file
changed. An explicit positional `origin/murgicraft` size comparison also passed.
The compiler reported only the existing `proc-macro-error2` future-incompatibility
notice, not a Rust/Clippy warning from this change.

`cargo nextest` is unavailable (`error: no such command: nextest`). The final
required fallback command is
`CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo test -p pumpkin-core -p pumpkin-world -p pumpkin --no-fail-fast`.
Aggregate Summary: **3027 passed; 2 failed; 30 ignored across seven suites**.
Cargo exited 101. Core unit tests: 2537 passed, 2 failed, 28 ignored (1142.55 s).
World unit tests: 408 passed, 0 failed, 1 ignored (123.97 s). The server unit,
external cooking-macro, lifecycle and synchronization suites passed; one world
code example remains ignored.

Both failures were scenario setup poll timeouts:

| Test | Panic | One isolated rerun |
|---|---|---|
| `entity::boss::wither::bossbar_tests::review_tests::b1019d_wither_twice_panicking_terminal_remove_is_retried_without_mutation` | `scenario join packets did not complete after 4000000 cooperative polls` | 1 passed, 0.70 s |
| `world::entity_unload::terrain_tests::entity_unload_retains_terrain_until_final_block_entity_serialization_finishes` | `scenario join packets did not complete after 4000000 cooperative polls` | 1 passed, 1.39 s |

Each failing test was rerun alone exactly once using the same final core test
executable, and both passed. These load-sensitive setup failures are reported,
not counted as a green full run. The issue #413 regressions, NBT admission,
retired-pair cleanup and revised callback-ownership test all passed in the full run.

An earlier complete full run reported 3022 passed, 5 failed, 30 ignored across
seven suites. Each failing test was rerun alone once. Three failures were caused
by this change and fixed: unchanged-terrain structure NBT was incorrectly skipped,
and both ordinary/copper retired-door pair callbacks lost their non-destructive
physics notification. The new conditional block-entity publication keeps unchanged
NBT while closing its post-serialization race; stale pair replay preserves that
notification. New structure and feature publication races, the original NBT
admission case, and pair-cleanup checks are included in final targeted validation.
Two load-sensitive failures passed alone: Wither terminal-removal retry panicked
`scenario join packets did not complete after 4000000 cooperative polls`, and
`entity_unload_retains_terrain_until_final_block_entity_serialization_finishes`
panicked `scenario fixture terrain save did not complete after 4000000 cooperative polls`.
A subsequent complete run reported 3026 passed, 3 failed, 30 ignored. The Wither
and entity-unload failures both panicked `scenario join packets did not complete
after 4000000 cooperative polls` and each passed its required isolated rerun.
The remaining failure reproduced alone: the old command capture-window test
expected mutation ownership across a registration callback (`cross-chunk
retirement entered a command's callback window`), contrary to this task's explicit
callback rule. The test now proves independent neighbor retirement finishes inside
the callback while the command still publishes chest terrain, a live chest entity,
and persistent NBT at the target. Independent review approved this stronger contract
check. Production code is unchanged from the successful scratch boot.

An environment restart interrupted a still earlier broad run; it is not counted
as a completed Summary.

The optimized debug server build passed. A scratch server outside the repo
started ready in 801 ms, forceloaded the test chunk, placed a copper chest with
three diamonds, changed it to exposed copper without losing those contents,
destroyed it, saved 121 chunks and shut down with exit 0. The log had no new
warnings, errors or panics; only the standard development-build banners.

One load-sensitive earlier chest run had 69 passes and one failure:
`overlapping_same_arc_saves_keep_newer_chest_contents_after_reload` panicked
`scenario join packets did not complete after 4000000 cooperative polls`.
Its required isolated rerun passed (15.89 seconds). A retirement test rendezvous
and the old bed partial-write rendezvous were updated to the new commit boundary;
their intermediate failures were fixed, rather than classified as baseline. The
strengthened explosion-banner test initially panicked `can call blocking only when
running on the multi-threaded runtime`; its isolated rerun reproduced that panic.
The test now requests Tokio’s multithreaded runtime so it can invoke the real
plugin callback, and its isolated rerun passes. This was a test harness fix.

Real-client rendering, sounds and inventory GUI behavior are not verified.
The owner checklist in `FORK.md` remains **Not yet**.

## Environment (before the first build)

```text
5
               total        used        free      shared  buff/cache   available
Mem:              17           0          16           0           0          16
Swap:              0           0           0
Filesystem      Size  Used Avail Use% Mounted on
overlay          32G  187M   30G   1% /
model name	: INTEL(R) XEON(R) PLATINUM 8573C
```
