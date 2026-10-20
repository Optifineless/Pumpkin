# PR #332 round 2 verification

Starting commit: `bc205fa556200d62dc91169d8cea1a5397f1e957`.
All B1–B4 findings were independently confirmed before edits.

## Vanilla reference

Official Java server 26.3, SHA-1 `33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c`,
downloaded using Mojang's version manifest, unbundled, and decompiled with
Vineflower 1.12.0. These are the bodies mirrored by the changes. The jars and
decompilation stay outside the repository. Raider inherits `dropCustomDeathLoot`
from Mob; the bottle comes from the entity loot table first.

### Raider.die

```java
   public void die(final DamageSource source) {
      if (this.level() instanceof ServerLevel serverLevel) {
         Entity killer = source.getEntity();
         Raid raidWhenKilled = this.getCurrentRaid();
         if (raidWhenKilled != null) {
            if (this.isPatrolLeader()) {
               raidWhenKilled.removeLeader(this.getWave());
            }

            if (killer != null && killer.is(EntityTypes.PLAYER)) {
               raidWhenKilled.addHeroOfTheVillage(killer);
            }

            raidWhenKilled.removeFromRaid(serverLevel, this, false);
         }
      }

      super.die(source);
   }
```

### Raider.isCaptain

```java
   public boolean isCaptain() {
      ItemStack banner = this.getItemBySlot(EquipmentSlot.HEAD);
      boolean hasCaptainBanner = !banner.isEmpty()
         && ItemStack.matches(banner, Raid.getOminousBannerInstance(this.registryAccess().lookupOrThrow(Registries.BANNER_PATTERN)));
      boolean patrolLeader = this.isPatrolLeader();
      return hasCaptainBanner && patrolLeader;
   }
```

### Raider.hasRaid

```java
   public boolean hasRaid() {
      return !(this.level() instanceof ServerLevel serverLevel) ? false : this.getCurrentRaid() != null || serverLevel.getRaidAt(this.blockPosition()) != null;
   }
```

### Raider.hasActiveRaid

```java
   public boolean hasActiveRaid() {
      return this.getCurrentRaid() != null && this.getCurrentRaid().isActive();
   }
```

### Raider.readAdditionalSaveData

```java
   protected void readAdditionalSaveData(final ValueInput input) {
      super.readAdditionalSaveData(input);
      this.wave = input.getIntOr("Wave", 0);
      this.canJoinRaid = input.getBooleanOr("CanJoinRaid", false);
      if (this.level() instanceof ServerLevel level) {
         input.getInt("RaidId").ifPresent(raidId -> {
            this.raid = level.getRaids().get(raidId);
            if (this.raid != null) {
               this.raid.addWaveMob(level, this.wave, this, false);
               if (this.isPatrolLeader()) {
                  this.raid.setLeader(this.wave, this);
               }
            }
         });
      }
   }
```

### Raid.removeFromRaid

```java
   public void removeFromRaid(final ServerLevel level, final Raider raider, final boolean removeFromTotalHealth) {
      Set<Raider> raiders = this.groupRaiderMap.get(raider.getWave());
      if (raiders != null) {
         boolean couldRemove = raiders.remove(raider);
         if (couldRemove) {
            if (removeFromTotalHealth) {
               this.totalHealth = this.totalHealth - raider.getHealth();
            }

            raider.setCurrentRaid(null);
            this.updateBossbar();
            this.setDirty(level);
         }
      }
   }
```

### Raid.setLeader

```java
   public void setLeader(final int wave, final Raider raider) {
      this.groupToLeaderMap.put(wave, raider);
      raider.setItemSlot(EquipmentSlot.HEAD, getOminousBannerInstance(raider.registryAccess().lookupOrThrow(Registries.BANNER_PATTERN)));
      raider.setDropChance(EquipmentSlot.HEAD, 2.0F);
   }
```

### LivingEntity.dropAllDeathLoot

```java
   protected void dropAllDeathLoot(final ServerLevel level, final DamageSource source) {
      boolean playerKilled = this.lastHurtByPlayerMemoryTime > 0;
      if (this.shouldDropLoot(level)) {
         this.dropFromLootTable(level, source, playerKilled);
         this.dropCustomDeathLoot(level, source, playerKilled);
      }

      this.dropEquipment(level);
      this.dropExperience(level, source.getEntity());
   }
```

### Mob.dropCustomDeathLoot

```java
   protected void dropCustomDeathLoot(final ServerLevel level, final DamageSource source, final boolean killedByPlayer) {
      super.dropCustomDeathLoot(level, source, killedByPlayer);

      for (EquipmentSlot slot : EquipmentSlot.VALUES) {
         ItemStack itemStack = this.getItemBySlot(slot);
         float dropChance = this.dropChances.byEquipment(slot);
         if (dropChance != 0.0F) {
            boolean preserve = this.dropChances.isPreserved(slot);
            if (source.getEntity() instanceof LivingEntity livingSource && this.level() instanceof ServerLevel serverLevel) {
               dropChance = EnchantmentHelper.processEquipmentDropChance(serverLevel, livingSource, source, dropChance);
            }

            if (!itemStack.isEmpty()
               && !EnchantmentHelper.has(itemStack, EnchantmentEffectComponents.PREVENT_EQUIPMENT_DROP)
               && (killedByPlayer || preserve)
               && this.random.nextFloat() < dropChance) {
               if (!preserve && itemStack.isDamageableItem()) {
                  itemStack.setDamageValue(itemStack.getMaxDamage() - this.random.nextInt(1 + this.random.nextInt(Math.max(itemStack.getMaxDamage() - 3, 1))));
               }

               this.spawnAtLocation(level, itemStack);
               this.setItemSlot(slot, ItemStack.EMPTY);
            }
         }
      }
   }
```

### Raider.pickUpItem

```java
   protected void pickUpItem(final ServerLevel level, final ItemEntity entity) {
      ItemStack itemStack = entity.getItem();
      boolean hasRaidLeader = this.hasActiveRaid() && this.getCurrentRaid().getLeader(this.getWave()) != null;
      if (this.hasActiveRaid()
         && !hasRaidLeader
         && ItemStack.matches(itemStack, Raid.getOminousBannerInstance(this.registryAccess().lookupOrThrow(Registries.BANNER_PATTERN)))) {
         EquipmentSlot slot = EquipmentSlot.HEAD;
         ItemStack current = this.getItemBySlot(slot);
         double dropChance = this.getDropChances().byEquipment(slot);
         if (!current.isEmpty() && Math.max(this.random.nextFloat() - 0.1F, 0.0F) < dropChance) {
            this.spawnAtLocation(level, current);
         }

         this.onItemPickup(entity);
         this.setItemSlot(slot, itemStack);
         this.take(entity, itemStack.getCount());
         entity.discard();
         this.getCurrentRaid().setLeader(this.getWave(), this);
         this.setPatrolLeader(true);
      } else {
         super.pickUpItem(level, entity);
      }
   }
```

### ObtainRaidLeaderBannerGoal.cannotPickUpBanner

```java
      private boolean cannotPickUpBanner() {
         if (!this.mob.hasActiveRaid()) {
            return true;
         }

         if (this.mob.getCurrentRaid().isOver()) {
            return true;
         }

         if (!this.mob.canBeLeader()) {
            return true;
         }

         if (ItemStack.matches(
            this.mob.getItemBySlot(EquipmentSlot.HEAD), Raid.getOminousBannerInstance(this.mob.registryAccess().lookupOrThrow(Registries.BANNER_PATTERN))
         )) {
            return true;
         }

         Raider leader = Raider.this.raid.getLeader(this.mob.getWave());
         return leader != null && leader.isAlive();
      }
```

### RaidOmenMobEffect.applyEffectTick

```java
   public boolean applyEffectTick(final ServerLevel level, final LivingEntity mob, final int amplification) {
      if (mob instanceof ServerPlayer player && !mob.isSpectator()) {
         BlockPos raidOmenPosition = player.getRaidOmenPosition();
         if (raidOmenPosition != null) {
            level.getRaids().createOrExtendRaid(player, raidOmenPosition);
            player.clearRaidOmenPosition();
            return false;
         }
      }

      return true;
   }
```

### Raider.ALLOWED_ITEMS

```java
   private static final Predicate<ItemEntity> ALLOWED_ITEMS = e -> !e.hasPickUpDelay()
      && e.isAlive()
      && ItemStack.matches(e.getItem(), Raid.getOminousBannerInstance(e.registryAccess().lookupOrThrow(Registries.BANNER_PATTERN)));
```

### RaiderPredicate codec and matches

```java
public record RaiderPredicate(boolean hasRaid, boolean isCaptain) implements EntitySubPredicate {
   public static final Codec<RaiderPredicate> CODEC = RecordCodecBuilder.create(
      i -> i.group(
            Codec.BOOL.optionalFieldOf("has_raid", false).forGetter(RaiderPredicate::hasRaid),
            Codec.BOOL.optionalFieldOf("is_captain", false).forGetter(RaiderPredicate::isCaptain)
         )
         .apply(i, RaiderPredicate::new)
   );
   public static final RaiderPredicate CAPTAIN_WITHOUT_RAID = new RaiderPredicate(false, true);

   public boolean matches(final Entity entity, final ServerLevel level, final @Nullable Vec3 position) {
      return !(entity instanceof Raider raider) ? false : raider.hasRaid() == this.hasRaid && raider.isCaptain() == this.isCaptain;
   }
}
```

### Official entities/pillager loot table

```json
{
  "type": "minecraft:entity",
  "pools": [
    {
      "condition": {
        "type": "minecraft:entity_properties",
        "entity": "this",
        "predicate": {
          "minecraft:type_specific/raider": {
            "is_captain": true
          }
        }
      },
      "entries": [
        {
          "type": "minecraft:item",
          "modifier": [
            {
              "type": "minecraft:set_count",
              "count": 1
            },
            {
              "type": "minecraft:set_ominous_bottle_amplifier",
              "amplifier": {
                "type": "minecraft:uniform",
                "max": 4,
                "min": 0
              }
            }
          ],
          "name": "minecraft:ominous_bottle"
        }
      ],
      "rolls": 1
    }
  ],
  "random_sequence": "minecraft:entities/pillager"
}
```

## State and callback audit

- B1: Raid Omen releases the registry mutex before RaidTrigger statistics.
  The other registry-lock sites in commands, pathfinding and player omen checks
  already snapshot and release before effects, packet sends or native events.
  Raid ticks detach the map and publish a read view before callbacks.
- B2: banner eligibility checks full stack equality, pickup delay and liveness.
  Owned item withdrawal releases transaction/storage guards before equality.
  Final raid/leader admission uses a nonblocking equipment acquisition; failed
  admission returns the actual withdrawn stock. Equipment publication, removal
  and displaced-head drops occur after guards are released. Existing generic
  component snapshot/cloning conventions are retained.
- B3: saved RaidId resolves through the actual registry, including the tick view;
  registered members rejoin their saved wave. Death clears current membership
  and removes the wave member/leader before death statistics and loot snapshots.
  Tick-owned membership edits are applied before wave/victory decisions, with a
  merge fallback. Bossbar recipients/values are captured before dispatch.
- B4: the codec defaults both booleans to false. The predefined
  CAPTAIN_WITHOUT_RAID constant is separate, and the built-in table explicitly
  requests is_captain=true.

RaidId, Wave, captain flags, head equipment and drop_chances keep their vanilla
saved meanings. Runtime tick views and pending membership edits are not saved.
No native API version or Wasm WIT changes. API 8 (same version): Raids gains
private pending runtime membership state; rebuild native plugins.

Lifecycle transitions: spawning and saved loading establish membership;
normal death removes it before loot; unload/save/restart retain existing tags
and resolve the registry when loaded. Unrelated disconnect, player respawn and
dimension-transfer behavior is unchanged. General raid AI, patrol spawning and
canonical banner appearance remain backlog work.

## Validation

Targeted core filter `b1020b_g01` passes all 11 tests, including the five new
round-2 regressions. The existing pumpkin-util parser filter is also required.

Each isolated negative control recompiles and runs
`cargo test -p pumpkin-core --lib <filter> -- --nocapture`:

| Finding | Filter suffix after `b1020b_g01_r2_` | Temporarily removed fix | Observed assertion |
| --- | --- | --- | --- |
| B1 | `raid_omen_expiry_callback_death` | Restore old Raid Omen guard scope | Raid Omen guard spans statistic callback |
| B2 | `nearby_pillager_preserves_captain_bottle` | Restore unrestricted banner goal | nearby AI consumed captain bottle |
| B3 | `saved_raid_membership_death_order` | Omit death-time membership removal | Raider.die removes remote membership before loot; expected 1 bottle, got 0 |
| B4 | `raider_predicate_defaults_death` | Restore omitted is_captain=true | empty codec object; expected 1 emerald, got 0 |

All four controls exited 101 with exactly one intended regression failure.
Every source mutation was restored in a finally block. B1 fails before attempting
the recursive mutex acquisition, so the negative control is bounded without sleeps.

Positive eligibility controls cover ordinary raiders, positional raids, mob-drop
gates, noncanonical/stacked/delayed banners, contended equipment, successful raid
succession, explicit predicate booleans and nonraiders. Saved-load coverage also
exercises an actual raid-tick packet callback and legacy absent drop_chances.

The lifecycle precheck identified only the entity and world aggregate hashes.
The refresh retained all existing statuses and notes; there were no moved keys
or new rows. Both required size-check forms passed with 0 failures and the same
two pre-existing generated packet file warnings (untouched Wasm host files).

Final code checks:

- `cargo test -p pumpkin-core --lib b1020b_g01 -- --nocapture`: 11 passed.
- `cargo test -p pumpkin-util b1020b_g01 -- --nocapture`: 1 passed.
- `cargo test -p pumpkin-core --test lifecycle_coverage_scan --test sync_guardrail_scan`:
  46 and 34 passed; only the two aggregate hashes changed in the manifest.
- `RUSTFLAGS=-Dwarnings cargo clippy -p pumpkin-core --all-targets`: passed.
- `cargo fmt --all -- --check`: passed.
- `SIZE_CHECK_BASE=origin/murgicraft python3 .fork/check-sizes.py` and
  `python3 .fork/check-sizes.py origin/murgicraft`: passed, 0 failures and the
  two pre-existing untouched Wasm generated packet file growth warnings.

Independent final raid/membership and lock/event audits found no remaining
blocking issues. The last lint-only changes use explicit guard drops in the
effect and fixtures. No gameplay logic changed during lint cleanup.

`cargo build -p pumpkin` passed. The freshly built debug server booted in the
scratch directory, reached `Started server`, accepted console `list` and `stop`,
saved successfully and printed `The server has stopped.`; exit code 0. The log
contained only the existing debug-build, development and non-TTY warnings, with
no errors or panics. The scratch harness sets `RUST_LOG=info` because the inherited
error-only filter hid readiness during the first observation; that first server
also shut down with exit 0.
Real-client play testing and capture remain Not yet; follow the FORK.md checklist.

The final upstream open-PR search for `repo:Pumpkin-MC/Pumpkin is:pr is:open raid`
returned bell highlighting, the broader mob-AI foundation refactor and player
push/touch work. None is the scoped review fix here. Fork PR #221 remains the
shared canonical banner appearance work.

## Changed files

- `.fork/pr332-review.md`
- `FORK.md`
- `crates/pumpkin-core/src/entity/death_loot.rs`
- `crates/pumpkin-core/src/entity/death_loot/captain_banner_tests.rs`
- `crates/pumpkin-core/src/entity/effect/raid_omen.rs`
- `crates/pumpkin-core/src/entity/item/transfer.rs`
- `crates/pumpkin-core/src/entity/mob/raider.rs`
- `crates/pumpkin-core/src/entity/mob/raider/banner.rs`
- `crates/pumpkin-core/src/world/loot/captain_raid_tick_tests.rs`
- `crates/pumpkin-core/src/world/loot/captain_tests.rs`
- `crates/pumpkin-core/src/world/loot/captain_tests/review.rs`
- `crates/pumpkin-core/src/world/loot/context.rs`
- `crates/pumpkin-core/src/world/loot/predicates/entity.rs`
- `crates/pumpkin-core/src/world/mod.rs`
- `crates/pumpkin-core/src/world/raid.rs`
- `crates/pumpkin-core/src/world/raid_loot.rs`
- `crates/pumpkin-core/tests/lifecycle_coverage_manifest.txt`
