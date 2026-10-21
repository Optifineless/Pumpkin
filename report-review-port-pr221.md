# PR #393 review round 2

Base: `ce9dcbc9efc5a6c7834ad8fe27523e4d2f3dcef7` on `origin/port-pr221-1020`.
Branch: `port-pr221-1020-r2`. No PR opened.

The three requested blockers were independently verified by one agent per finding before edits. A fourth agent checked the real-entry test paths and reviewed the regressions. The coordinator implemented the fixes and the three finding agents reviewed the resulting changes without finding further blockers.

## B1 — CONFIRMED-FIXED

At r1, `crates/pumpkin-data/src/data_component_impl/banner_patterns.rs:137-139` rejects 1,025 saved layers. Component dispatch (`data_component_impl/mod.rs:689`) and `item_stack/mod.rs:877` propagate that failure. Container loading (`crates/pumpkin-inventory/src/inventory/inventory.rs:168-181`) and player loading (`crates/pumpkin-core/src/entity/player.rs:5805-5806`) skip the stack. The next save omits it. The parent reader (`basic.rs:495-508`) retained every layer and its writer (`:513-521`) saved every layer.

The related raw tooltip list check (`tooltip_display.rs:86-88`) rejects 257 duplicate valid component names before set conversion at `:89-93`.

Official Java 26.3 `BannerPatternLayers.java:30` uses unrestricted `Layer.CODEC.listOf()`; its stream codec at `:31-33` also has no such cap. `ByteBufCodecs.java:447-450,486-487` defaults the collection bound to `Integer.MAX_VALUE`. `TooltipDisplay.java:15-17` reads an unrestricted list and converts it to an ordered set. Actual vanilla layer caps belong to loom result creation (`LoomMenu.java:177-184`, six), tooltip presentation (`BannerPatternLayers.java:40-43`, six), and client rendering (`BannerRenderer.java:211-214`, sixteen). None authorizes saved-data deletion or server publication truncation.

Fix: `banner_patterns.rs:125-142` removes saved-list rejection and publication truncation; `tooltip_display.rs:82-93` deduplicates an unrestricted saved list. The existing bounded untrusted packet reader remains separate. The old banner reader's non-list/non-compound handling, black fallback for unknown colors and long opaque references are restored. Loom and renderer behavior are unchanged.

## B2 — CONFIRMED-FIXED

At r1, `tooltip_display.rs:78` uses Byte-only `extract_bool` (`crates/pumpkin-nbt/src/tag.rs:531-534`). `Int(0)` and other numeric tags therefore reach the same whole-item rejection chain as B1. The old unit reader accepted this component. `NbtTag::as_numeric_double` (`tag.rs:517-525`) supports all six numeric tag kinds.

Vanilla `TooltipDisplay.java:20` uses `Codec.BOOL`; `NbtOps.java:65-66,97-98` accepts every numeric tag and tests `doubleValue() != 0.0`.

Fix: `tooltip_display.rs:79` uses those numeric semantics and still rejects nonnumeric values. `item_stack/mod.rs:877-879` omits a recognized component that fails parsing while retaining the item, count and valid unrelated components, as the coordinator explicitly requires. Invalid item IDs/counts and unknown component-type names retain their pre-existing admission rules; no opaque component/public layout state was added.

## B3 — CONFIRMED-FIXED

At r1, `crates/pumpkin-protocol/src/codec/banner_patterns.rs:57` accepts a direct key with 16,384 `é` characters via the UTF-16-bounded reader (`ser/mod.rs:486-529`). Creative stack validation (`codec/item_stack_validation.rs:15`) and the actual handler (`crates/pumpkin-core/src/net/java/play/set_creative_slot.rs:35,75-82`) admit it. The shared data/hash projection (`data_component_impl/banner_patterns.rs:54,169`) retains it. Publication at `codec/banner_patterns.rs:39` instead calls the generic 32,767-byte writer (`ser/mod.rs:1002-1019`), rejecting its 32,768 UTF-8 bytes.

`CSetContainerContent` propagates the failure (`java/client/play/set_container_content.rs:66-67`); `Player::enqueue_inventory_packet` (`player.rs:6751-6753`) sends nothing, after the screen handler has recorded all slots as received (`crates/pumpkin-inventory/src/screen_handler.rs:463-470`). Equipment encoding fails for the same reason (`java/client/play/set_equipment.rs:83`). NBT accepts and preserves this key.

Vanilla `BannerPattern.java:21-22` uses `STRING_UTF8`; `ByteBufCodecs.java:185` sets 32,767 UTF-16 units and `Utf8String.java:35-51` allows their corresponding encoded byte length. The generic Pumpkin writer defect predates the PR; direct banner key retention/publication makes this regression newly reachable here. It is fixed locally rather than widening the generic writer's behavior.

Fix: `codec/banner_patterns.rs:40` uses the corresponding three-bytes-per-unit writer bound after the shared UTF-16 projection validation. Publication and hashing keep exactly the same supported layer values. The ingest persistence check is at `:60-62`.

B3 also exposed a local ingest edge during final review: keys such as `漢` × 21,846 or `😀` × 10,923 fit the native stream bound but exceed Java NBT's 65,535 modified-UTF-8-byte string length. Pumpkin's existing NBT writer rejects these strings (`serializer.rs:90-100`) while `Nbt::write` suppresses the error (`lib.rs:175`), which can produce a truncated save. The local fix rejects such newly incoming direct keys before authoritative ingestion using CESU-8 length, retaining existing saved readers. Vanilla `StringTag.java:63-64` uses `writeUTF`; `NbtIo.java:198-203` substitutes an empty string for this error. We preserve the admitted key's meaning by refusing unsaveable new input instead. PRE-EXISTING-BACKLOG: general NBT error propagation/fallback handling predates this PR; this new direct-key route is closed here.

## Verification

Eight new regressions have recorded negative controls:

- On unchanged r1, `cargo test -p pumpkin-core old_pr221_review2 --lib`: **0 passed, 7 failed**, as expected. Five saved-data witnesses failed with `saved item must retain its item count` (0 versus 1). The container and equipment witnesses failed with `opening chest must send its full contents, including unrelated stock` and `full equipment send must reach observer`. Production code was unchanged.
- Before adding the CESU-8 ingest check, `cargo test -p pumpkin-protocol old_pr221_review2_creative_direct_keys --lib`: **0 passed, 1 failed** with `Creative keys must be safely persistable` (true versus false).

Core regressions are in `crates/pumpkin-core/src/entity/mob/raider/old_pr221_tests/review2.rs`. They use binary NBT fixtures, actual inventory loading/saving, and binary reload. They verify all 1,025 stored layers, 257 repeated tooltip names, legacy saved shapes, all six numeric tag types, and malformed-component isolation. The numeric witnesses include the exact stone/Int-zero input and a non-Byte nonzero value. Rarity and seven unrelated diamonds survive. The multibyte witnesses additionally use actual bounded Creative decoding, the actual handler, full menu opening/publication and tracker equipment pairing, preserving unrelated diamonds/armor. The protocol regression checks exact 65,535-byte acceptance, NUL encoding, and rejection of CJK/supplementary strings that exceed the modified UTF-8 limit.

Toolchain environment: `PATH=/workspace/.cargo/bin:$PATH RUSTUP_HOME=/workspace/.rustup CARGO_HOME=/workspace/.cargo`. Rust 1.99.0. Cargo jobs are bounded to two or four; this does not change the test scope.

| Command | Result |
| --- | --- |
| `cargo test -p pumpkin-data -p pumpkin-protocol -p pumpkin-core old_pr221 --lib` | **26 passed**: core 13, data 4, protocol 9; zero failed. |
| `cargo test -p pumpkin-core --test lifecycle_coverage_scan --test sync_guardrail_scan` | **80 passed**: lifecycle 46, sync 34; zero failed. |
| `RUSTFLAGS=-Dwarnings cargo clippy -p pumpkin-data -p pumpkin-protocol -p pumpkin-core --all-targets` | Passed with warnings denied. |
| `cargo fmt --all -- --check` | Passed. |
| `SIZE_CHECK_BASE=origin/murgicraft python3 .fork/check-sizes.py` | Passed: 3,603 files, zero failures; two pre-existing generated Wasm packet growth warnings. The explicit positional-base check also passed. |
| Scratch-world boot | Passed: optimized dev binary reached `Server is now running`, accepted `stop`, saved and exited 0. Only the existing debug-build/heavy-development notices; no errors or panics. |
| `CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo test -p pumpkin-core -p pumpkin-world -p pumpkin --no-fail-fast` | Completed all seven targets in 42m 50.47s: **2,946 passed, 4 failed, 30 ignored**. All four failures passed their single isolated rerun below. Cargo fallback used because nextest is unavailable. |

The boot harness explicitly sets `RUST_LOG=info`: the inherited workspace filter hid the first attempt's readiness message, so that process was stopped cleanly and the observable check repeated. This was a harness correction, not a server failure. The debug-build notice keys off debug assertions even with dev optimization level 1.

**Summary (Cargo fallback aggregate): 2,946 passed; 4 failed; 30 ignored; all seven targets completed.** Cargo returned 101 for the original failures; there was no timeout. The four isolated reruns passed. Cargo does not emit nextest's literal `Summary` line, so this aggregate is calculated from its seven target result lines:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `pumpkin` binary | 1 | 0 | 0 |
| `pumpkin-core` library | 2,456 | 4 | 28 |
| `cooking_macro_external` | 1 | 0 | 0 |
| `lifecycle_coverage_scan` | 46 | 0 | 0 |
| `sync_guardrail_scan` | 34 | 0 | 0 |
| `pumpkin-world` library | 408 | 0 | 1 |
| `pumpkin-world` doctests | 0 | 0 | 1 |

The original core result line was `test result: FAILED. 2456 passed; 4 failed; 28 ignored; 0 measured; 0 filtered out; finished in 1214.79s`. All 13 core PR regressions, including the seven new core witnesses, passed in this optimized run.

### Isolated failure reruns

All four failures had the same panic at `crates/pumpkin-core/src/server/scenario/mod.rs:83:5`: `scenario join packets did not complete after 4000000 cooperative polls`. Each was rerun **once, alone**, from the exact optimized artifact with `/workspace/Pumpkin/target/debug/deps/pumpkin_core-d1235adedc402da8 --exact <full-test-name>`, using the core crate as working directory. Every rerun ran one test and passed, so these are flakes under the requested policy; no unrelated fix was made.

| Full test name | Isolated result |
| --- | --- |
| `entity::boss::wither::bossbar_tests::review_tests::b1019d_wither_persistent_remove_panic_parks_after_one_recovery` | 1 passed, 0 failed (0.77s) |
| `entity::boss::wither::bossbar_tests::review_tests::b1019d_wither_twice_panicking_remove_precedes_same_session_readd` | 1 passed, 0 failed (0.82s) |
| `entity::boss::wither::bossbar_tests::review_tests::b1019d_wither_twice_panicking_terminal_remove_is_retried_without_mutation` | 1 passed, 0 failed (0.72s) |
| `world::entity_unload::terrain_tests::entity_unload_retains_terrain_until_final_block_entity_serialization_finishes` | 1 passed, 0 failed (1.42s) |

An independent source audit also found these tests and their relevant production paths unchanged from r1. Wither REMOVE sends `CBossEvent` (`world/bossbar.rs:142-149`; `boss_event.rs:37,54`), without item component serialization. The terrain fixture uses plain stacks with empty component patches (`entity_unload_terrain_tests.rs:83-95`; `item_stack/mod.rs:118-123`), so it does not enter the changed component branch. All four original failures occurred at fixture join, before those asserted transitions. The unchanged test blob hashes are `9f5a0de1328885c4c1a2b1d3cc1b8b009c7baeb5` (Wither) and `e7d4af28d18e7c6fbd242b36057f3520386a40d3` (terrain).

The lifecycle scan initially requested three changed family fingerprints. `LIFECYCLE_MANIFEST_WRITE=1` refreshed them, then the original statuses and notes were restored. The committed delta contains only hashes for `pumpkin-core/src/entity`, `pumpkin-data/src/data_component_impl` and `pumpkin-data/src/item_stack`; no keys were added/removed and no status was upgraded.

## Compatibility and state audit

The authoritative state is the existing component patch. Readers are saved-item loading and Creative component decoding; writers are existing item/inventory saves and the Creative handler. Publication and click hashing derive from `java_layers`; invalid/unknown references continue to be saved and omitted from Java projection. There are no new caches or owners, no new lock-order edges, and no new guards crossing callbacks, awaits or sends. Save/reload tests exercise restart persistence; tracker pairing exercises late observer publication. Existing disconnect/death/dimension/respawn behavior is unchanged.

Removed behavior audit: the saved collection caps and publication truncation were deleted because vanilla does not apply them there; the network resource bounds remain at ingestion. Persistent string projection bounds remain in `java_projection`, while saved strings are preserved. The Byte-only boolean conversion became numeric conversion. Whole-item propagation of recognized component decode failures became per-component omission. Shared callers continue to use the same APIs; saved player/container/equipment readers all benefit, and nested NBT-backed stack decoding uses the same component normalization.

API 8 (same version): the public `java_layers` opaque iterator drops its `Take` wrapper, changing that return value's layout; native plugins must rebuild against this checkout. Named component/stack layouts and method signatures are unchanged. The r1 rebuild requirement also remains. Wasm WIT is unchanged. Valid saved values retain their meaning. Malformed recognized components are rejected individually rather than deleting their item.

## Vanilla provenance

Official version JSON: https://piston-meta.mojang.com/v1/packages/61fff1316bd50312624a858bbb5eb18fa45208ed/26.3.json

Inner server JAR SHA-256: `a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`. Decompiled with Vineflower 1.12.0. Source and acquisition provenance are outside the repository under `/workspace/work/vanilla`; Java line citations above refer to that extraction. The latest 100 open upstream PRs and a broader open banner/tooltip search were inspected. #4101 concerns armor-trim registry references; #3951 concerns banner placement. #3507's component patches still use Byte-only tooltip booleans and reject direct banner holders, so they do not resolve B2/B3. #3728 fixes named `NbtTag::End` serialization, a separate cause from overlong modified-UTF-8 strings. None supplies these review fixes.

## Owner play test

1. Open a chest containing a custom banner and ordinary items; expect every slot to appear and remain usable after reopening and restarting. Record the chest contents before and after.
2. Load the long-layer and numeric-tooltip saved fixtures; expect the original item counts and banner data to remain after a second save/restart.
3. Observe a custom direct-pattern banner worn alongside armor; expect both equipment slots to publish, including to a player joining later.

In-game verification: **Not yet**. No real-client rendering/capture is claimed. The owner should perform the checklist and attach a screenshot or recording to the existing PR before merging. No new PR should be opened for this review branch.

## Description draft for the existing PR

### Description

Review round 2 preserves saved banners with more than 1,024 layers and saved numeric tooltip booleans. A malformed recognized component no longer deletes its item. Accepted multibyte direct banner keys now publish complete container and equipment updates; unsaveable new keys are rejected before admission. API 8 stays at the same version; native plugins must rebuild for the changed opaque iterator layout.

### Testing

Official Java 26.3 codecs were compared. All 26 PR regressions and 80 lifecycle/sync scans pass, with eight recorded failing negative controls. Formatting, warnings-denied Clippy, size checks and scratch-world boot pass. The optimized suite completed with 2,946 passed, four fixture-join flakes and 30 ignored; each flake passed its single isolated rerun. Real-client verification remains outstanding; follow the owner checklist above.

> This description was drafted by an AI agent (Codex Cloud, GPT-6.1 Sol).
