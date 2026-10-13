# Inventory ownership source coverage

This integration scan is E1 from `briefs/ownership-map-inventory.md` v3.1. It
checks inventory and cooperating-state source coverage only. Every manifest row
is currently `legacy`; a passing scan does not establish write authority,
complete owner sets, conservation, receipts, or durable publication.

## Record format

`manifest.tsv` has the following tab-separated columns, in order:

1. `site_key`: stable workspace path, enclosing type/function, source-site kind,
   field or method, and occurrence within that item. It contains no line number.
2. `workspace_relative_path`
3. `enclosing_item`
4. `site_kind`
5. `field_or_method`
6. `state_family`
7. `value_class`
8. `actor_role`
9. `authority_status`
10. `boundary_kind`
11. `lifecycle_context`
12. `alias_or_dependency`
13. `migration_item`
14. `source_fingerprint`
15. `review_note`

The scanner walks Rust source under every workspace crate and tool directory. It finds
ItemStack-bearing fields and methods, the cooperating fields listed by the map,
direct field references, stock mutators, NBT/component calls, lifecycle sites,
and mapped callbacks, packet sends, waits and awaits. This discovery is driven
by source tokens and scanner policy, not by the manifest. The manifest owns the
audited classifications. Candidate records use placeholders for classification;
the scan compares only stable source identity and fingerprint. The ignored
printer emits review material and never writes or accepts manifest rows.

Every candidate item and every mapped module has a SHA-256 fingerprint over
token-normalized source. Whitespace and comments do not affect the fingerprint;
string literal tokens do. A changed item or module requires a fresh source audit even
when its key remains unchanged. Line numbers are diagnostics only.

`migration_item` names the applicable P row and later E enforcement item from
map section 7, or a specific debt that blocks the touching cohort. These are
migration assignments, not a claim that later work has shipped.
`authority_status=legacy` means the runtime path has no E1 ownership
enforcement. NBT sites remain context-dependent unless caller evidence
establishes live mutation; method names alone do not establish detached
initialization. Macro definitions are listed, and unexpanded call sites remain
an explicit audit debt.

## Exclusions and limits

Test-only source under `tests/`, files ending in `_tests.rs`, `test_*.rs`, and
`#[cfg(test)]` items inside source files are excluded from production discovery.
Scanner fixtures cover comments, strings, aliases, macros, direct writers,
fingerprint changes and stale sites. Rust macros are token-scanned, not
expanded; unsupported or indirect expansion remains a named debt and cannot be
used to claim an exclusive cohort. The synchronization guardrail continues to
own raw-lock discovery and its existing allowlist policy.

## Update and review procedure

1. Run `inventory_ownership_scan` and read each file:line diagnostic in the
   source context. Identify the state owner, actor, direct/indirect readers and
   writers, callbacks, packet sends, waits, NBT/component callers and lifecycle
   path using the full ownership map. Do not copy scanner-side classification
   hints into the manifest without that audit.
2. For a new site, add one row per field, mutator, boundary or lifecycle site.
   Keep source keys independent of line numbers. For changed fingerprints,
   repeat the reader/writer/lifecycle audit before updating the fingerprint.
   Remove stale rows only after confirming the site was removed or moved and
   that no behaviour moved outside the scanned source set.
3. Keep unresolved aliases and legacy debt explicit. Never label a runtime
   path `admitted` in E1. An unresolved row blocks exclusivity for the touching
   cohort.
4. Run the inventory fixtures and the synchronization scan with its existing
   negative fixtures. There is no CI mode that accepts or writes all current
   candidates. The ignored `print_source_inventory_for_manual_manifest_review`
   test prints candidate identities and fingerprints only; it does not write
   the manifest or change scan policy.

## Scanner policy

Candidate discovery uses the tokenizer shared with `sync_guardrail_scan` and
scanner-owned identifier families for mapped stock fields, mutators, lifecycle
operations and boundaries. It walks all workspace crates, so it covers core,
inventory, data, native plugin and both Wasm host surfaces. It excludes only
test-only source as described above. New candidates, missing classifications,
changed source fingerprints and stale rows fail with the path and line plus the
required manifest action.
