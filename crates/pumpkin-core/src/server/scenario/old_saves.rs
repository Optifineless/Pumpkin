use super::*;
use pumpkin_data::{damage::DamageType, item::Item, item_stack::ItemStack};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::{
    codec::item_stack_seralizer::ItemStackSerializer, java::server::play::SSetCreativeSlot,
};
use pumpkin_util::GameMode;
use pumpkin_world::data::player_data::PlayerDataStorage;
use std::sync::atomic::Ordering;

const SAVED_POSITION: Vector3<f64> = Vector3::new(7.5, 64.0, 9.5);
const SAVED_HEALTH: f32 = 13.0;
const SAVED_SLOT: usize = 4;
const SAVED_COUNT: u8 = 17;

fn old_player_save(legacy: bool) -> NbtCompound {
    // Player.addAdditionalSaveData/ServerPlayer.addAdditionalSaveData: the former fork
    // wrote playerGameType as a byte; vanilla writes an int. No sleeping_pos/bed exists.
    let mut nbt = NbtCompound::new();
    if legacy {
        nbt.put_byte("playerGameType", GameMode::Survival as i8);
    } else {
        nbt.put_int("playerGameType", GameMode::Survival as i32);
    }
    nbt.put_string("Dimension", Dimension::OVERWORLD.minecraft_name.into());
    nbt.put(
        "Pos",
        NbtTag::List(vec![
            NbtTag::Double(SPAWN.x),
            NbtTag::Double(SPAWN.y),
            NbtTag::Double(SPAWN.z),
        ]),
    );
    nbt.put_float("Health", 20.0);
    nbt.put_short("SleepTimer", 0);
    nbt
}

fn distinctive_player_save(legacy: bool) -> NbtCompound {
    let mut nbt = old_player_save(legacy);
    nbt.put_list(
        "Pos",
        [SAVED_POSITION.x, SAVED_POSITION.y, SAVED_POSITION.z]
            .map(NbtTag::Double)
            .to_vec(),
    );
    nbt.put_float("Health", SAVED_HEALTH);
    // Player.readAdditionalSaveData / ItemStackWithSlot.CODEC load inventory entries.
    let mut item = NbtCompound::new();
    item.put_byte("Slot", SAVED_SLOT as i8);
    ItemStack::new(SAVED_COUNT, &Item::EMERALD).write_item_stack(&mut item);
    nbt.put_list("Inventory", vec![NbtTag::Compound(item)]);
    nbt
}

fn assert_save_restored(session: &Session) {
    assert_eq!(
        session.player.position(),
        SAVED_POSITION,
        "saved position was not restored through login"
    );
    assert_eq!(
        session.player.living_entity.health.load(),
        SAVED_HEALTH,
        "saved health was not restored through login"
    );
    let stack = session.player.inventory.get_slot(SAVED_SLOT);
    assert_eq!(
        stack.item,
        &Item::EMERALD,
        "saved inventory item was not restored through login"
    );
    assert_eq!(
        stack.item_count, SAVED_COUNT,
        "saved inventory count was not restored through login"
    );
}

fn write_save(scenario: &Scenario, id: Uuid, nbt: NbtCompound) {
    // Publish a real compressed player file; login must recover and decode it normally.
    let storage = PlayerDataStorage::new(scenario.directory.path().join("players"), true);
    let snapshot = storage.snapshot(&id, || nbt);
    storage.save_snapshot(snapshot).unwrap();
}

#[tokio::test]
async fn s7_old_fork_survival_invulnerable_save_recovers_via_login() {
    run(async {
        // Fork migration of #130's overloaded Invulnerable tag; ServerPlayer.readAdditionalSaveData
        // derives abilities through ServerPlayerGameMode.setGameModeForPlayer/GameType.updatePlayerAbilities.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut nbt = distinctive_player_save(true);
        nbt.put_bool("Invulnerable", true); // Old fork cached mode/loading immunity in the entity tag.
        let mut abilities = NbtCompound::new();
        abilities.put_bool("invulnerable", true);
        abilities.put_bool("instabuild", true);
        abilities.put_bool("mayfly", true);
        nbt.put_compound("abilities", abilities);
        write_save(&scenario, id, nbt);
        let player = scenario.join(id, "LegacyImmunity").await;
        assert_save_restored(&player);
        assert_eq!(player.player.gamemode.load(), GameMode::Survival);
        assert!(
            !player
                .player
                .get_entity()
                .invulnerable
                .load(Ordering::Relaxed)
        );
        assert!(!player.player.abilities.lock().unwrap().invulnerable);
        let health = player.player.living_entity.health.load();
        player
            .player
            .living_entity
            .damage(player.player.as_ref(), 2.0, DamageType::GENERIC);
        assert!(
            player.player.living_entity.health.load() < health,
            "#130: old fork save restores permanent Survival immunity through login"
        );
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn s7_vanilla_survival_save_derives_abilities_via_login() {
    run(async {
        // ServerPlayer.readAdditionalSaveData invokes gameMode.setGameModeForPlayer after Player loads abilities.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut nbt = old_player_save(false);
        let saved_position = Vector3::new(5.5, 64.0, 5.5);
        nbt.put_list(
            "Pos",
            [saved_position.x, saved_position.y, saved_position.z]
                .map(NbtTag::Double)
                .to_vec(),
        );
        nbt.put_float("Health", 16.0);
        let mut abilities = NbtCompound::new();
        abilities.put_bool("invulnerable", false);
        abilities.put_bool("mayfly", false);
        abilities.put_bool("instabuild", false);
        nbt.put_compound("abilities", abilities);
        write_save(&scenario, id, nbt);
        let player = scenario.join(id, "VanillaSave").await;
        assert_eq!(player.player.gamemode.load(), GameMode::Survival);
        assert_eq!(player.player.position(), saved_position);
        assert_eq!(player.player.living_entity.health.load(), 16.0);
        assert!(!player.player.abilities.lock().unwrap().invulnerable);
        let health = player.player.living_entity.health.load();
        player
            .player
            .living_entity
            .damage(player.player.as_ref(), 2.0, DamageType::GENERIC);
        assert!(player.player.living_entity.health.load() < health);
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
#[ignore = "new: login clears explicit Invulnerable in vanilla player saves"]
async fn s7_vanilla_explicit_invulnerable_save_preserves_entity_flag() {
    run(async {
        // Entity.load restores explicit Invulnerable; GameType.updatePlayerAbilities only changes abilities.
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let mut nbt = old_player_save(false);
        nbt.put_bool("Invulnerable", true);
        write_save(&scenario, id, nbt);
        let player = scenario.join(id, "VanillaImmunity").await;
        assert_eq!(player.player.gamemode.load(), GameMode::Survival);
        assert!(!player.player.abilities.lock().unwrap().invulnerable);
        let health = player.player.living_entity.health.load();
        player
            .player
            .living_entity
            .damage(player.player.as_ref(), 2.0, DamageType::GENERIC);
        let after = player.player.living_entity.health.load();
        scenario.stop().await;
        assert_eq!(
            after, health,
            "vanilla login loses explicitly saved entity Invulnerable"
        );
    })
    .await;
}

async fn sleep_timer_without_bed(legacy: bool) {
    // Player.readAdditionalSaveData keeps a sleep counter; ServerPlayer.readAdditionalSaveData
    // clears actual sleeping state. SleepTimer alone never establishes a sleeping player.
    let scenario = Scenario::new().await;
    let id = Uuid::new_v4();
    let mut nbt = distinctive_player_save(legacy);
    nbt.put_short("SleepTimer", 100);
    write_save(&scenario, id, nbt);
    let player = scenario.join(id, "OldSleep").await;
    assert_save_restored(&player);
    let sleeping = player.player.sleeping_since.load();
    let night_skip = player.player.world().should_skip_night();
    assert!(player.player.sleeping_bed_pos.load().is_none());
    scenario.ticks(2).await;
    scenario.stop().await;
    assert!(
        sleeping.is_none(),
        "audit row 35: login restores sleeping state from SleepTimer with no bed: {sleeping:?}"
    );
    assert!(
        !night_skip,
        "audit row 35: bedless returning player counts towards night skip"
    );
}

#[tokio::test]
#[ignore = "known bug: audit row 35"]
async fn s7_old_fork_sleep_timer_does_not_restore_sleeping() {
    run(async {
        sleep_timer_without_bed(true).await;
    })
    .await;
}

#[tokio::test]
#[ignore = "known bug: audit row 35"]
async fn s7_vanilla_sleep_timer_does_not_restore_sleeping() {
    run(async {
        sleep_timer_without_bed(false).await;
    })
    .await;
}

#[tokio::test]
async fn s8_late_creative_slot_packet_after_survival_command_is_ignored() {
    run(async {
        // ServerGamePacketListenerImpl.handleSetCreativeModeSlot admits edits only with infinite materials.
        let scenario = Scenario::new().await;
        let player = scenario.join(Uuid::new_v4(), "LateSlot").await;
        scenario.command("gamemode creative LateSlot");
        assert_eq!(player.player.gamemode.load(), GameMode::Creative);
        player
            .player
            .inventory
            .set_slot(0, ItemStack::new(1, &Item::STONE));
        scenario.command("gamemode survival LateSlot");
        let packet = SSetCreativeSlot::new(
            36,
            ItemStackSerializer::from(ItemStack::new(64, &Item::DIAMOND)),
        );
        player.send(&scenario.server, &packet);
        scenario.ticks(2).await;
        assert!(
            !player.client().is_closed(),
            "audit row 6: a late creative packet kicked a Survival player"
        );
        assert_eq!(player.player.inventory.get_slot(0).item, &Item::STONE);
        assert_eq!(player.player.inventory.get_slot(0).item_count, 1);
        scenario.stop().await;
    })
    .await;
}
