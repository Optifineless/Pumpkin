# PR #390 review, round 2

Starting head: `d48ed5a282ab1d0c900d9d5abf8581b3fa0cc59a`.
Branch: `fix-b1020c-g06-r2`, created from the explicitly fetched PR branch.
All four BACKLOG findings were independently CONFIRMED before source edits.
The coordinator implemented the fixes; the four verifiers also reviewed their
resulting deltas. No new transaction ownership or lock-order design was added.

## Findings and fixes

1. **B1 — CONFIRMED-FIXED.** At the starting head,
   `inventory_request_beacon.rs:47-54` admitted only one action. The parser now
   admits BeaconPayment alone or exactly one following Destroy with count 1,
   BeaconPayment container, no dynamic ID, UI index 27 and the live payment UID.
   Validation precedes the existing Java `update_effects` operation, which remains
   the only debit. Duplicate/unrelated suffixes and wrong counts/identities fail
   without changing payment, powers or sound. The paid real-entrypoint regression
   repeats source-derived BeaconPayment/Destroy requests at all four beacon tiers.

2. **B2 — CONFIRMED-FIXED.** At the starting head,
   `inventory_request_loom.rs:40-72,163-169` rejected result metadata and matched
   Java names. The parser now translates the 43 exact Bedrock banner identifiers
   from pinned Geyser `BannerPattern`, then authorizes the translated name against
   the staged live selectable registry. One optional CraftResultsDeprecated is
   accepted immediately after CraftLoom with one result and the matching craft
   count; empty, duplicate, misplaced and mismatched metadata fails. Its proposed
   item/count/NBT never supplies output. Existing staging, destination checks and
   production `safe_take` pay each result. Ordinary `bs` and item-required `flo`
   cases retain names/layers and the pattern item, with repeated crafts and a
   cursor destination. A forged diamond/count/NBT proposal still yields only the
   authoritative paid banner.

3. **B3 — CONFIRMED-FIXED.** At the starting head,
   `inventory_request_beacon.rs:76` and `inventory_request_loom.rs:274-277`
   fabricated zero response indices. Specialized responses now use payment 27,
   loom input/dye/material 9/10/11 and preview/CreatedOutput 50. Inputs no longer
   derive response indices from optional Consume metadata. Tests decode both
   response index fields, complete container sets, dynamic IDs, counts, UIDs,
   names, durability corrections and payload exhaustion. The omitted-Consume
   case also requires the correct authoritative input addresses.

4. **B4 — CONFIRMED-FIXED.** At the starting head, the five focused tests used
   singleton beacons, Java loom names, zero UI indices and no result metadata;
   `loom_workstation_test_support.rs:280-288` checked correction presence only.
   Their production menu-open, input-move and request entrypoints remain. Fixtures
   now use the primary-source action forms and addresses above. Ordinary banner
   metadata includes the original Bedrock pattern layers and display name plus
   the requested pattern and reversed Bedrock dye ID in versioned network NBT.
   Complete inventory, armor, offhand and menu correction vectors are decoded and
   compared with authoritative stock, including item IDs, counts, UIDs, aux values,
   block states, extra data, container IDs and absent dynamic IDs. A dirty cursor
   rejection requires its exact Cursor/window-59 correction. Two additional
   regressions cover disconnect admission for both menus and a cancelled loom
   statistic callback after the committed payment. The existing callback test
   still proves released menu/selection ownership and response-before-close order.

## Primary protocol evidence

These are source-derived request fixtures, not captured target-client sessions.
The versioned schema identifies protocol 2193 / 1.26.51-beta.1; it establishes
fields, while the independent maintained implementations establish action forms.

- Dragonfly [beacon handler, pinned 1793369](https://github.com/df-mc/dragonfly/blob/1793369709ceed934675d69e86ff6c828e4216c8/server/session/handler_beacon.go#L12-L20)
  defines payment slot 27; lines 55-59 document the predicted following Destroy
  and suppress a second debit. Its [request loop](https://github.com/df-mc/dragonfly/blob/1793369709ceed934675d69e86ff6c828e4216c8/server/session/handler_item_stack_request.go#L64-L75)
  bounds that suppression to the request.
- Geyser [loom translator, pinned 87c9c59](https://github.com/GeyserMC/Geyser/blob/87c9c597b4f3ed6039a2394f5e47ef4692de71f7/core/src/main/java/org/geysermc/geyser/translator/inventory/LoomInventoryTranslator.java#L91-L117)
  receives CraftLoom followed by CraftResultsDeprecated and resolves the Bedrock
  identifier. Lines 151-168 map the four loom UI slots to 9/10/11/50.
- Geyser [BannerPattern](https://github.com/GeyserMC/Geyser/blob/87c9c597b4f3ed6039a2394f5e47ef4692de71f7/core/src/main/java/org/geysermc/geyser/inventory/item/BannerPattern.java#L37-L106)
  supplies the edition mapping, including `bs` and `flo`.
- Geyser [BannerItem](https://github.com/GeyserMC/Geyser/blob/87c9c597b4f3ed6039a2394f5e47ef4692de71f7/core/src/main/java/org/geysermc/geyser/item/type/BannerItem.java#L145-L160)
  supplies the Patterns tag shape and reversed dye IDs.
- Geyser [beacon translator](https://github.com/GeyserMC/Geyser/blob/87c9c597b4f3ed6039a2394f5e47ef4692de71f7/core/src/main/java/org/geysermc/geyser/translator/inventory/BeaconInventoryTranslator.java#L175-L185)
  maps BeaconPayment to 27.
- Dragonfly [player.go](https://github.com/df-mc/dragonfly/blob/1793369709ceed934675d69e86ff6c828e4216c8/server/session/player.go#L278-L284)
  defines `craftingResult = 50`; [result creation](https://github.com/df-mc/dragonfly/blob/1793369709ceed934675d69e86ff6c828e4216c8/server/session/handler_item_stack_request.go#L277-L309)
  uses that CreatedOutput address, including [loom output](https://github.com/df-mc/dragonfly/blob/1793369709ceed934675d69e86ff6c828e4216c8/server/session/handler_loom.go#L86-L101).
- Mojang [v1.26.51 CraftResultsDeprecated schema](https://github.com/Mojang/bedrock-protocol-docs/blob/v1.26.51/json/ItemStackRequestCraftResultsDeprecatedAction.json)
  retains result descriptors and a craft count of 1-255.

## Preserved behavior and scope

Compared the unchanged payment/result chain with official Java 26.3
`BeaconMenu.updateEffects`, `LoomMenu` result-slot `onTake` and `setupResultSlot`,
and `Slot.tryRemove/safeTake`, decompiled using Vineflower 1.12.0. Inner server
JAR SHA-256: `a362163eec5d1612d520772bc16e5b39c09e3b234fdc045f56bf544284ee8ae6`.
The existing ordered inventory owner set, screen/selection lifetime checks,
deferred publication/statistic callbacks and single authoritative payment remain.
The generic validator/commit/finalization policy is unchanged. No saved-data
meaning, public native type/layout/signature or Wasm WIT changes; API 8 stays 8.

**PRE-EXISTING-BACKLOG:** generic menu correction still uses LevelEntity with a
menu-local vector (`inventory_request_correction.rs:54-63`, `player.rs:6878`).
PR #382 owns that policy. Tests assert the existing exact contents/address;
this round changes specialized response addressing only.

Searched upstream open beacon/loom PRs before implementation. #3860 is general
beacon parity; #4119/#4118 are generic inventory work. No replacement for these
local Bedrock parser fixes was found. No GitHub issue, comment, review or pull
request was created; the requested new branch is the only remote write.

## Verification

Rust/Cargo 1.99.1, `CARGO_BUILD_JOBS=2`; all commands below are targeted.

- `cargo test --locked -p pumpkin-core --lib b1020c_g06 -- --nocapture`:
  **7 passed**, including the five updated PR regressions and two new cases.
- `cargo test --locked -p pumpkin-core --test lifecycle_coverage_scan --test sync_guardrail_scan`:
  **46 lifecycle + 34 synchronization checks passed**. No manifest refresh.
- `cargo test --locked -p pumpkin-core --lib inventory_exploit_tests -- --nocapture`:
  **10 passed**.
- `cargo test --locked -p pumpkin-core --lib created_output_tests -- --nocapture`:
  **6 passed**.

The beacon validity regression deliberately catches its injected panic and passes;
this is not an unexpected server panic. Cargo reports the existing
`proc-macro-error2 v2.0.1` future-compatibility advisory.

### Recorded negative controls

Retained the final fixtures and production entrypoints. Temporarily changed only
the described implementation behavior, executed the filters below with
`cargo test --locked -p pumpkin-core --lib <filter> -- --nocapture`, and restored
the final production bytes in a `finally` block. These are reverted/mutated
implementation controls, not a claim that the full base branch suite was run.

| Control | Filter | Recorded runtime failure |
| --- | --- | --- |
| Restore both original `d48ed5a28` parser files | `b1020c_g06_bedrock_` | Both positives fail: beacon payment remains, triggering `paid selection must consume exactly one payment`; loom output item is AIR instead of WHITE_BANNER. |
| Restore Java-name interpretation while retaining metadata admission and correct addresses | `b1020c_g06_bedrock_loom_creates_and_pays_selected_banner` | Output item is AIR instead of WHITE_BANNER. |
| Restore zero specialized response indices with working parsers | `b1020c_g06_bedrock_` | Beacon response index is 0 instead of 27; loom fails `complete authoritative loom response` with zero UI addresses. |
| Omit one slot from the existing menu correction vector, retaining the packet itself | `b1020c_g06_loom_failed_sequences_conserve_all_stock` | `correction slot vector length`: 3 received versus 4 expected. |

After **each** control, the unaffected filter
`inventory_transactions_bedrock_recipe_commits_virtual_output_after_validation`
passed (1/1, four executions). All six intended failing test executions reached
behavioral assertions; none is a compile failure. Final production files were
restored after the fourth control. `inventory_request_correction.rs` has no final
diff. Logs were kept outside the repository; no test-only production mutation is
committed.

- `RUSTFLAGS=-Dwarnings cargo clippy -p pumpkin-core --all-targets`:
  **passed**, no source warnings. Four fixture literal-style diagnostics from
  the first lint pass were corrected before this successful run.
- `cargo fmt --all -- --check`: **passed**.
- `git diff --check`: **passed**.
- `SIZE_CHECK_BASE=origin/murgicraft python3 .fork/check-sizes.py`:
  **0 failures**; two inherited warnings for the original PR's two-line changes
  to each Wasm host's generated packet file. Neither file changes in round 2.

- `cargo build --locked -p pumpkin --bin pumpkin`: **passed**.
- Bounded scratch boot of that fresh debug binary with `RUST_LOG=info`:
  **passed**, ready in 3517 ms with Java and Bedrock listeners. Sent console
  `stop`; save and shutdown completed with exit 0. No ERROR or unexpected panic.
  Only expected debug/development/new-world/non-TTY warnings appeared. The
  command-local log filter overrides the host's unrelated suppressed INFO filter;
  proxy and CA settings remain inherited. No player login or real-client play
  test is claimed.

The final handover also reruns focused tests, both scans, strict Clippy, format,
size and a fresh build/boot after creating the single commit, so the checked
revision is the exact branch head rather than just its pre-commit source tree.

## Files changed

Under `crates/pumpkin-core/src/net/bedrock/play/`:
`inventory_request_beacon.rs`, `inventory_request_beacon_test_support.rs`,
`inventory_request_beacon_tests.rs`, `inventory_request_loom.rs`,
`loom_workstation_test_support.rs`, `loom_workstation_tests.rs`, `mod.rs`, and
new `workstation_correction_test_support.rs`. Bookkeeping: `FORK.md` and this
`report-review-b1020c-g06.md`.

## Real-client checklist — Not yet verified in-game

- On Bedrock 1.26.51, pay for each available beacon tier and reselect a power.
  Expect exactly one payment, saved powers and one selection sound each time.
  Reopen the menu and restart the server; expect the same selected powers.
- Apply an ordinary banner pattern, then a flower pattern with its pattern item.
  Repeat crafts into inventory and the cursor. Expect one banner and dye per
  result, the original name/layers, a retained pattern item and synchronized counts.
- Fill or replace the destination, close the menu, and disconnect around selection
  or crafting. Expect no free output, duplicate payment or lost input.
- Record the menus, counts, beacon effects/sound and repeat operations after each
  acknowledgment. Capture the actual action sequence, identifiers, result NBT,
  stack IDs and UI addresses for comparison with the source-derived fixtures.

The owner must perform that real-client check and attach a screen recording to
the existing PR before treating #272/#275 as verified. This round does not open
another PR, deploy a binary or claim a human play test.
