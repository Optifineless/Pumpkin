# PR #390 fix round 3

The requested new branch is `fix-b1020c-g06-r3`. The starting PR head was
`079f3a2806546afa6ade6665c449e2bc3e94b434`. Both original PR commits were
rebased onto explicitly fetched `origin/batch-1020b2` at
`f6cd06e047d22747967d12efe3c96ca0a746baf2`. `git range-diff` shows only
FORK.md insertion context changed during the rebase; all production/test hunks
were preserved. The original remote branch is untouched. No pull request,
issue, review, comment or message was created on GitHub.

Seven separate finding verifiers and one dedicated test auditor completed their
read-only reviews before source edits. Every finding was independently CONFIRMED.
The coordinator implemented every fix. Follow-up reviews checked prediction
authority, codec framing, preview identity and deferred sound ordering. The codec
needs no new dependency or serialization fallback.

## Finding disposition

1. **B1: CONFIRMED-FIXED.** At the starting head,
   `inventory_request_beacon.rs:68-72` and
   `inventory_request_loom.rs:108-110,159-164` compared raw request IDs to live
   positive UIDs. Successful slot results were discarded into responses at
   `item_stack_request.rs:809-844`. The new private prediction history retains
   at most 64 accepted negative request IDs, with at most three relevant,
   deduplicated authoritative slot results each. It binds to weak player and menu
   allocation identities. Resolution requires the exact container, slot and absent
   dynamic ID, followed by equality to the current positive stack UID. Unknown,
   failed/reused, other-address, replaced-stack and retired-menu references reject.
   A current request ID is removed before any validation path; only successful
   commits supply history. History is snapshotted before menu guards and stored
   after release through ArcSwap. Earlier requests within one packet are available
   to later requests. Specialized Destroy/Consume still bypass the generic action
   loop, leaving existing payment/result-slot operations as the sole debit.

2. **B2: CONFIRMED-FIXED.** The old loom test retired its player at
   `loom_workstation_tests.rs:272-276` but unconditionally required corrections at
   `:296`. The requested base's `inventory_request_correction.rs:72-85,242-244`
   intentionally suppresses publication to retired sessions. Only that case now
   requires no inventory content/slot corrections. Live rejected requests still
   require exact authoritative correction contents. Compound NBT key order is
   immaterial: the helper compares all decoded NBT values (including list order),
   version framing and every remaining user-data byte, alongside all other item
   and container fields. PR #382 production protections are unchanged.

3. **Owner P1, outbound banner components: CONFIRMED-FIXED.** The starting
   `network_item.rs:148,261,356-357` discarded component NBT. All three outbound
   item conversions now share translation for ordered banner patterns and custom
   names. Patterns use the existing 43-entry edition mapping, shared forward and
   reverse instead of copied; colors use Bedrock's reversed dye IDs. Names use the
   existing Bedrock legacy renderer. The real-handler regression decodes emitted
   `CInventoryContent` bytes and independently requires `display.Name = "Keep my
   name"`, then `ts/4` and `bs/1` layers. A serialized dropped-item wrapper is
   checked too. Unsupported custom datapack pattern identifiers remain outside
   the vanilla edition map; no incoming client NBT becomes authoritative output.

4. **Owner P2, packet-start payment: CONFIRMED-FIXED.** The old snapshot at
   `inventory_request_workstations.rs:86-94` and equality gate at `:194-201`
   rejected payment placed by an earlier request in the same packet. That snapshot
   and gate are removed. Native validity restoration, live player/world/menu,
   block incarnation, range and ordered owner admission remain; the existing
   guarded beacon operation checks the current payment. The regression exercises
   placement, selection, replenishment and reselection in one packet, with singleton
   selections so this test does not depend on negative-reference resolution.

5. **Owner P2, duplicate preview identity: CONFIRMED-FIXED.** At
   `pumpkin-inventory/src/loom_screen_handler.rs:527-528`, cloning preserved the
   banner input's UID. Result creation now uses `copy_with_count(1)`; existing
   item/component equality guards keep an unchanged live preview's UID stable.
   Preview aliases compare to the live output slot, including its components,
   rather than the isolated staging stack. CreatedOutput remains bound to the
   current negative request ID. The repeated-craft regression checks distinct
   input/preview UIDs, refresh stability and successful subsequent live-preview use.

6. **Owner P2, source indices: CONFIRMED-FIXED, with corrected protocol indices.**
   The old parser at `inventory_request_loom.rs:70-87,108-110,221-223` omitted
   source indices. This branch's canonical indices are banner 9, dye 10 and all
   preview/CreatedOutput aliases 50, not the older review suggestion of zero.
   Invalid indices reject before any selection or payment. Regressions cover both
   inputs, all three output aliases, dynamic IDs, unknown/other-slot predictions,
   and substitution of the input UID for the preview UID, retaining exact live
   corrections. The old claim that forged Consume indices also determine input
   response indices was already superseded by round 2's canonical responses.

7. **Owner P3, loom take sound: CONFIRMED-FIXED for the existing world sound path.**
   Starting `LoomResultSlot::on_take_item` (`:558-575`) and the Bedrock result-take
   path omitted the sound. Successful Bedrock loom commits now defer the world
   sound until ownership releases and the response enqueues. A menu-bound game
   tick value deduplicates repetitions, same-packet and consecutive requests;
   its initial zero matches Java's default `lastSoundTime`. The real-handler test
   checks one sound for multiple paid crafts in one tick, another on the next tick,
   and none for rejection, plus category, position, volume and pitch.

**PRE-EXISTING-BACKLOG:** `World::play_sound_raw` publishes registered sounds to
Java recipients only. This round adds the requested loom world effect and verifies
its Java observer packet; it does not implement general Bedrock sound transport or
claim Bedrock audio verification. Java loom result sound parity outside this
specialized Bedrock path remains pre-existing. Neither requires expanding this PR.

## Source, state and caller review

Downloaded the official 26.3 bundler and decompiled only the relevant inventory
classes with Vineflower 1.12.0. Inner server SHA-256:
`a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`.
Compared `BeaconMenu.updateEffects`, `LoomMenu.setupResultSlot`, its result-slot
`onTake` (including `lastSoundTime`), and `Slot.safeTake`. Scratch source, tools,
controls and logs stay outside the repository.

Read the pinned [Dragonfly request implementation](https://github.com/df-mc/dragonfly/blob/1793369709ceed934675d69e86ff6c828e4216c8/server/session/handler_item_stack_request.go#L348-L370):
`resolveID` requires prior request/container/slot results. Read pinned
[Geyser BannerItem](https://github.com/GeyserMC/Geyser/blob/87c9c597b4f3ed6039a2394f5e47ef4692de71f7/core/src/main/java/org/geysermc/geyser/item/type/BannerItem.java#L145-L160),
which specifies ordered Pattern short IDs and `15 - colorId`.
Request shapes and slots remain the pinned
source-derived fixtures from round 2. They are semantic objects sent through the
real handler, not captured client traffic or incoming packet-reader tests.

Shared callers checked: `create_loom_result` serves normal setup and post-take
refresh, also used by Java clicks/shift-click. Its UID is transient, never saved.
The three outbound conversions supply inventory/window/cursor, equipment and
dropped-item packets; they now agree on banner/name components. Other components
remain outside this narrow translation. Prediction state has one constructor,
request reader/writer and menu/player invalidation boundary. It owns no player/menu
strong references and adds no lock-order edge. Deferred effects preserve response
before callback order; no guard spans an await, callback or send.

Native **API 8 (same version)** is retained. The private ArcSwap history changes
public `BedrockClient` layout, so FORK.md requires native plugins to rebuild.
Public signatures, native source compatibility, Wasm WIT and saved-data meaning
are unchanged. No persistent fields or codecs changed. Disconnect/rejoin,
closed/replaced menu and retired player tests preserve the existing admission and
return paths; no real-client lifecycle pass is claimed.

Searched open upstream beacon/loom PRs: #3860 is broad beacon parity; #4119/#4118
are generic inventory work; none replaces these local fixes. The fork's existing
#390 remains the delivery target. The final read-only searches for `beacon`, `loom`
and `inventory` returned the same relevant PRs; #4056 is generic hotbar bounds
validation and does not replace these workstation changes.

## Files changed

- `FORK.md`: round-3 behavior, API 8 same-version rebuild note and play checklist.
- `crates/pumpkin-core/src/net/bedrock/mod.rs` and `play/mod.rs`: private retained
  state, module wiring and regression registration.
- `play/inventory_request_{beacon,loom,workstations,predictions}.rs` and
  `play/item_stack_request.rs`: authoritative references, live admission, canonical
  addresses, preview validation and deferred sound.
- `play/inventory_request_beacon_{test_support,tests}.rs`,
  `play/loom_workstation_{test_support,tests}.rs`,
  `play/workstation_correction_test_support.rs` and
  `play/workstation_prediction_tests.rs`: real-entry fixtures, decoded response
  and correction checks, and the new regressions.
- `crates/pumpkin-inventory/src/loom_screen_handler.rs`: independent preview UID.
- `crates/pumpkin-protocol/src/bedrock/{mod,network_item,banner_pattern,item_components}.rs`:
  shared edition mapping and outbound NBT.
- `crates/pumpkin-core/tests/lifecycle_coverage_manifest.txt`: eight moved boundary
  keys and six changed family fingerprints; all 91 statuses/descriptions retained,
  no new rows.
- This report. No lockfile, generated data, saved-data codec or persistent field changed.

## Validation

Static checks passed on the corrected, formatted source:

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `RUSTFLAGS=-Dwarnings cargo clippy --locked -p pumpkin-core -p pumpkin-inventory -p pumpkin-protocol --all-targets` | PASS; post-manifest check 3m37s |
| `SIZE_CHECK_BASE=origin/murgicraft python3 .fork/check-sizes.py` | PASS: 3,632 files, 0 failures, 2 inherited warnings |
| Same size command with positional `origin/murgicraft` | PASS: same result; script reads its base from this argument, not the environment variable |

The inherited size warnings are the two generated Wasm packet files growing by
2 lines in the base stack. Cargo also reports the existing `proc-macro-error2`
future-compatibility advisory; strict Clippy emits no source diagnostic.

Validation exposed two fixture issues: raw NBT compound-key ordering in exact
correction checks, and a preview-source case looking up selectable patterns before
refreshing the live menu. Both fixtures were corrected. The resumed first optimized
focused run compiled in 49m34s and reached 12 passes / 1 fixture failure; the
corrected isolated test passed (1 passed, 0 failed, 62.48s) after the menu refresh
fix. One earlier optimized compilation was
interrupted by the workspace restart and is not counted as a test pass.

Negative controls used the cached unoptimized profile:
`CARGO_PROFILE_DEV_OPT_LEVEL=0 cargo test --locked -p pumpkin-core --lib <filter> -- --nocapture`.
Each reached a runtime assertion and exited 101; compiler failures were rejected
as evidence. The temporary source reversions were restored byte-for-byte, verified
against SHA-256 hashes of all 18 changed Rust files.

| Reverted behavior | Observed runtime failure |
| --- | --- |
| B2 retired-session expectation | Missing corrections: `[false, false, false, false, false]` |
| B1 beacon reference resolution | Requests `-1/-3` returned statuses `0/1`, expected `0/0` |
| B1 loom reference resolution | Requests `-1/-3/-5` returned `0/0/1`, expected all success |
| Current beacon payment | Same-packet statuses `0/1/1/1`, expected all success |
| Distinct preview UID | Input and preview UIDs both 3; distinct-identity assertion failed |
| Outbound banner NBT | Version marker 0, expected -1 |
| Canonical loom source indices | `WrongSourceSlot(2)` created a banner in empty inventory slot 12 |
| Deferred loom sound | 0 sounds, expected 1 |
| Live preview identity | Subsequent valid preview request returned error status 1, expected 0 |
| Positive dynamic-ID rejection | `DynamicConsume` created a banner in empty inventory slot 12 |

The independent generic crafting control
`inventory_transactions_bedrock_recipe_commits_virtual_output_after_validation`
still passed (1 passed, 0 failed) with the combined production reversions. The
index and dynamic-ID controls retain the full invalid-case table; failures were
checked to identify the intended forged source rather than an earlier case.

The final restored optimized focused command
`CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo test --locked -p pumpkin-core --lib b1020c_g06 -- --nocapture`
passed: **13 passed, 0 failed, 2,542 filtered out, 97.26s**. The expected bounded
validity-callback panic was caught by its passing regression.

The lifecycle scan initially reported eight stale line-addressed boundaries and six
changed family fingerprints in the combined tree. Regenerated only after that
failure with `LIFECYCLE_MANIFEST_WRITE=1`, then restored every prior status and
description. The final diff contains exactly eight moved keys and six hashes,
with 91 rows and no new rows; no coverage status was upgraded.

The final optimized neighboring checks passed:

| Filter / integration binaries | Result |
| --- | --- |
| `lifecycle_coverage_scan` / `sync_guardrail_scan` | 46 / 34 passed, no failures |
| `inventory_exploit_tests` | 10 passed, no failures |
| `created_output_tests` | 6 passed, no failures |
| `loom_shift_click_crafts_every_available_banner` | 1 passed, no failures |
| `cargo test --locked -p pumpkin-protocol bedrock::network_item` | 1 passed, no failures; 216 unrelated tests filtered |
| `git diff --check` | PASS |

The core checks used `cargo test --locked -p pumpkin-core` with the indicated
`--lib` filter or `--test` binaries. All Cargo tests above used
`CARGO_PROFILE_DEV_OPT_LEVEL=1`.

`CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo build --locked -p pumpkin --bin pumpkin`
passed in 39m14s. Launched that binary in fresh scratch directory
`/workspace/work/390-r3/boot`, reached “Server is now running,” sent `stop`, and
observed completed saves and exit 0. Java and Bedrock listeners started and all
three dimensions loaded. Read the complete startup/shutdown log: no errors or
panics. Warnings were the existing empty Bedrock username prefix, debug/development
build, fresh world with no level.dat, and non-TTY logger fallback.

The first full-suite attempt compiled in 22m20s, passed the main binary's one
test and 39 core tests, then lost its execution-service connection. The runner,
Cargo and test process disappeared before a core summary; no failed-test or panic
diagnostic was captured. Container memory peaked at its 16 GiB limit, but
`oom_kill` stayed zero, so the cause is undetermined. This interrupted attempt is
archived and is not counted as a completed suite.

Repeated the same three-package command with `RUST_TEST_THREADS=1` and detached,
durable file logging, using the already compiled binaries. This changes test
execution concurrency only; the source and optimized profile remain unchanged.

The final command passed:

```sh
RUST_TEST_THREADS=1 CARGO_PROFILE_DEV_OPT_LEVEL=1 cargo test --locked -p pumpkin-core -p pumpkin-world -p pumpkin --no-fail-fast
```

`cargo nextest --version` reported that nextest is unavailable, so this is the
requested Cargo fallback. Its literal per-target results were:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `pumpkin` main | 1 | 0 | 0 |
| `pumpkin-core` library | 2,527 | 0 | 28 |
| Core external cooking-macro test | 1 | 0 | 0 |
| Lifecycle coverage scan | 46 | 0 | 0 |
| Synchronization guardrail scan | 34 | 0 | 0 |
| `pumpkin-world` library | 408 | 0 | 1 |
| `pumpkin-world` documentation tests | 0 | 0 | 1 |

Fallback Summary: 3017 passed; 0 failed; 30 ignored; 0 measured; 0 filtered out across 7 result lines. Cargo exit 0; elapsed 2403.67s.

No test failed, so no isolated failing-test reruns were needed. All 13 PR tests,
including the six new round-3 regressions, passed again within this complete run.
All 18 changed Rust files matched the frozen SHA-256 hashes both before and after
execution. The existing ignored cases are unchanged. No whole-workspace suite ran.

Final branch checks confirmed the original remote remains
`079f3a2806546afa6ade6665c449e2bc3e94b434` and the requested new remote branch
was absent before delivery. The final commit contains only the 21 listed files;
its SHA and push verification accompany the delivered report.

## Owner play test and existing-PR description draft

On supported Bedrock, quickly insert a beacon payment and confirm before its
acknowledgment; expect one payment consumed and the chosen power. Repeat after
acknowledgment, reselect, reopen, save and restart. Rapidly place a named banner
and dye into a loom, then craft before acknowledgment; expect one banner and dye
per result, retained pattern materials and visible original/new layers and name.
Repeat into inventory and cursor, then carry, drop and reopen the banner; expect
its visible appearance and name to persist. Try a full destination, close/reopen
and reconnect; expect no duplicate output or lost input. Have a nearby Java player
listen for one loom take sound per crafting tick. Bedrock audio is separate backlog.
Capture both menus, counts, banner appearance/name, beacon powers and observer
sound, plus the request IDs/actions/slot references if packet capture is available.

The owner must run this real-client checklist and attach a screen recording to
the existing PR before marking #272/#275 verified. No deployment, human play test
or real-client capture occurred in this session. Do not open another PR.

### Description

Bedrock beacon and loom requests can refer to authoritative results of earlier
unacknowledged input moves. Payment placement and beacon selection can share a
packet; malformed source addresses still reject before payment. Loom previews
have distinct stable identities, and outgoing banner items retain their visible
patterns and custom names. Successful Bedrock loom crafts schedule the existing
world sound once per game tick. Rebased on the #382 combined tree, preserving its
retired-session correction protection. Native API stays 8; rebuild native plugins.

### Testing

Validated the frozen final source on the rebased combined tree: 13 focused PR
tests passed; all ten temporary negative controls failed at their intended runtime
assertions, with one unaffected generic crafting control passing. Neighbor filters
passed (10 inventory exploit, 6 created-output, 1 Java loom and 1 protocol NBT test).
Lifecycle/synchronization scans passed 46/34; strict three-crate all-target Clippy,
formatting and size checks passed (0 size failures, two inherited warnings).
Optimized server build and fresh startup/save/shutdown passed.

The complete optimized three-package Cargo fallback passed with one test thread:
3,017 passed, 0 failed, 30 existing ignored cases; no isolated reruns were needed.
The first default-concurrency attempt was interrupted by the execution service
before a core summary and was archived separately. Exact commands and results
appear above. The delivered report records the final commit SHA.

Real-client play testing and capture remain Not yet. Run the checklist above and
attach the recording to the existing #390 before marking #272/#275 verified.
General Bedrock sound transport remains pre-existing backlog. API 8 stays at the
same version; rebuild native plugins.

> This description was drafted by an AI agent (Codex, GPT-6).
