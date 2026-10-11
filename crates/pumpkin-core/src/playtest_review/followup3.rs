use super::*;
use crate::{
    entity::{Entity, RemovalReason, vehicle::boat::BoatEntity},
    net::{bedrock::combat_test_support::TestBedrockPlayer, java::combat_test_support::TestPlayer},
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        api::events::{
            entity::entity_portal::EntityPortalEvent, vehicle::vehicle_damage::VehicleDamageEvent,
        },
    },
};
use pumpkin_data::{BlockDirection, damage::DamageType};
use std::sync::atomic::{AtomicUsize, Ordering};

struct DecayFirstHit {
    boat: Arc<dyn EntityBase>,
    hits: AtomicUsize,
}

impl EventHandler<VehicleDamageEvent> for DecayFirstHit {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut VehicleDamageEvent,
    ) -> BoxFuture<'a, ()> {
        if self.hits.fetch_add(1, Ordering::Relaxed) == 0 {
            self.boat
                .cast_any()
                .downcast_ref::<BoatEntity>()
                .unwrap()
                .vehicle
                .tick();
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn predicted_lethal_hit_decayed_to_nonlethal_leaves_boat_breakable() {
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    let vehicle = &boat
        .cast_any()
        .downcast_ref::<BoatEntity>()
        .unwrap()
        .vehicle;
    vehicle.set_damage(31.0);
    fixture.server.plugin_manager.register(
        Arc::new(DecayFirstHit {
            boat: boat.clone(),
            hits: AtomicUsize::new(0),
        }),
        EventPriority::Normal,
        true,
    );
    assert!(boat.damage(boat.as_ref(), 1.0, DamageType::PLAYER_ATTACK));
    assert_eq!(vehicle.get_damage(), 40.0);
    assert!(boat.get_entity().is_alive());
    assert!(boat.damage(boat.as_ref(), 1.0, DamageType::PLAYER_ATTACK));
    assert!(boat.get_entity().is_removed());
    assert_eq!(fixture.drops(&Item::OAK_CHEST_BOAT), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trading_gossip_discount_reaches_the_same_trade_snapshot() {
    use crate::entity::passive::villager::{VillagerEntity, data::VillagerData};
    use pumpkin_data::villager::{VillagerProfession, VillagerType};
    use pumpkin_inventory::{
        merchant::merchant_screen_handler::MerchantScreenHandler,
        screen_handler::{ScreenHandler, ScreenHandlerFactory},
    };
    use pumpkin_protocol::java::server::play::SlotActionType;
    let fixture = Fixture::new();
    let player = TestPlayer::new(&fixture.world);
    let entity = fixture.entity(&EntityType::VILLAGER);
    let merchant = entity.cast_any().downcast_ref::<VillagerEntity>().unwrap();
    merchant.set_villager_data(VillagerData::new(
        VillagerType::Plains,
        VillagerProfession::Fletcher,
        1,
    ));
    *merchant.offers.lock().unwrap() = vec![pumpkin_protocol::java::client::play::MerchantOffer {
        base_cost_a: ItemStack::new(32, &Item::STICK).into(),
        output: ItemStack::new(1, &Item::EMERALD).into(),
        cost_b: None,
        reward_exp: false,
        uses: 0,
        max_uses: 16,
        xp: 0,
        special_price: 0,
        price_multiplier: 0.5,
        demand: 0,
    }];
    player
        .player
        .inventory
        .set_stack(9, ItemStack::new(32, &Item::STICK));
    let handler = merchant
        .create_screen_handler(1, &player.player.inventory, player.player.as_ref())
        .unwrap();
    *player.player.current_screen_handler.lock().unwrap() = handler.clone();
    let menu_special_price = {
        let mut guard = handler.lock().unwrap();
        let menu = guard
            .as_any_mut()
            .downcast_mut::<MerchantScreenHandler>()
            .unwrap();
        menu.set_selected_offer(0);
        menu.on_slot_click(2, 0, SlotActionType::QuickMove, player.player.as_ref());
        assert_eq!(menu.offers[0].uses, 1);
        // Villager.onReputationEventFrom adds two Trading points before updateSpecialPrices.
        menu.offers[0].special_price
    };
    assert_eq!(menu_special_price, -1);
    assert_eq!(merchant.offers.lock().unwrap()[0].special_price, -1);
    fixture.shutdown().await;
}

fn place_door(fixture: &Fixture, pos: BlockPos) {
    use pumpkin_data::block_properties::{DoubleBlockHalf, OakDoorLikeProperties};
    fixture.world.set_block_state(
        &pos.down(),
        Block::STONE.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    let mut props = OakDoorLikeProperties::default(&Block::OAK_DOOR);
    for (position, half) in [
        (pos, DoubleBlockHalf::Lower),
        (pos.up(), DoubleBlockHalf::Upper),
    ] {
        props.half = half;
        fixture.world.set_block_state(
            &position,
            props.to_state_id(&Block::OAK_DOOR),
            BlockFlags::FORCE_STATE,
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonplayer_no_drop_upper_break_still_drops_dependent_lower_door() {
    let fixture = Fixture::new();
    let pos = BlockPos::new(8, 64, 9);
    place_door(&fixture, pos);
    fixture
        .world
        .break_block(
            &pos.up(),
            None,
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
        )
        .unwrap();
    assert!(fixture.world.get_block_state(&pos).is_air());
    assert!(fixture.world.get_block_state(&pos.up()).is_air());
    assert_eq!(fixture.drops(&Item::OAK_DOOR), 1);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creative_upper_door_particles_reach_both_editions_except_breaker() {
    use pumpkin_protocol::{
        VarInt,
        bedrock::client::level_event::{CLevelEvent, LevelEvent},
        java::client::play::CWorldEvent,
    };
    let fixture = Fixture::new();
    let mut breaker = TestPlayer::new(&fixture.world);
    breaker
        .player
        .permission_lvl
        .store(pumpkin_util::permission::PermissionLvl::Four);
    let mut java = TestPlayer::new(&fixture.world);
    let mut bedrock = TestBedrockPlayer::new(&fixture.world).await;
    bedrock
        .player
        .watched_section
        .store(java.player.watched_section.load());
    fixture.world.players.store(Arc::new(vec![
        breaker.player.clone(),
        java.player.clone(),
        bedrock.player.clone(),
    ]));
    let pos = BlockPos::new(8, 64, 9);
    place_door(&fixture, pos);
    let state = fixture.world.get_block_state_id(&pos);
    let java_packet = CWorldEvent::new(
        pumpkin_data::world::WorldEvent::ParticlesDestroyBlock as i32,
        pos,
        i32::from(state.as_u16()),
        false,
    );
    let java_bytes = java.client().serialize_packet(&java_packet).unwrap();
    let bedrock_packet = CLevelEvent {
        event_id: VarInt(LevelEvent::ParticlesDestroyBlock as i32),
        position: pos.to_centered_f64().to_f32_lossy(),
        data: VarInt(pumpkin_data::BlockState::to_be_network_id(state) as i32),
    };
    let bedrock_bytes = bedrock.client().serialize_packet(&bedrock_packet).unwrap();
    breaker.take_packets();
    java.take_packets();
    bedrock.take_packets();
    breaker
        .player
        .gamemode
        .store(pumpkin_util::GameMode::Creative);
    fixture
        .world
        .break_block(
            &pos.up(),
            Some(&breaker.player),
            BlockFlags::NOTIFY_ALL | BlockFlags::SKIP_DROPS,
        )
        .unwrap();
    assert!(!breaker.take_packets().contains(&java_bytes));
    assert_eq!(
        java.take_packets()
            .iter()
            .filter(|packet| **packet == java_bytes)
            .count(),
        1
    );
    assert_eq!(
        bedrock
            .take_packets()
            .iter()
            .filter(|packet| **packet == bedrock_bytes)
            .count(),
        1
    );
    bedrock.close().await;
    fixture.shutdown().await;
}

#[test]
fn chest_vehicle_classification_uses_registry_identity_without_item_lookup() {
    let mut kind = EntityType::POPLAR_CHEST_BOAT;
    // EntityType equality is its registry ID; item names must not drive a per-tick classification.
    kind.resource_name = EntityType::ITEM.resource_name;
    assert!(BoatEntity::has_chest_inventory(&kind));
    kind.id = EntityType::ITEM.id;
    kind.resource_name = EntityType::POPLAR_CHEST_BOAT.resource_name;
    assert!(!BoatEntity::has_chest_inventory(&kind));
}

struct NativeRemovalEntity {
    entity: Entity,
    removals: AtomicUsize,
}

impl EntityBase for NativeRemovalEntity {
    fn get_entity(&self) -> &Entity {
        &self.entity
    }
    fn get_living_entity(&self) -> Option<&crate::entity::living::LivingEntity> {
        None
    }
    fn cast_any(&self) -> &dyn std::any::Any {
        self
    }
    fn on_removed(&self, reason: RemovalReason) {
        assert!(reason == RemovalReason::Discarded);
        self.removals.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_entity_removal_hook_survives_base_entity_dispatch() {
    let fixture = Fixture::new();
    let entity = Arc::new(NativeRemovalEntity {
        entity: Entity::new(
            fixture.world.clone(),
            Vector3::new(8.0, 64.0, 8.0),
            &EntityType::ITEM,
        ),
        removals: AtomicUsize::new(0),
    });
    fixture.world.add_entity_silent(entity.clone());
    assert!(entity.entity.remove());
    assert!(!entity.entity.remove());
    assert_eq!(entity.removals.load(Ordering::Relaxed), 1);
    assert!(fixture.world.entities.load().is_empty());
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bedrock_hoe_with_offhand_shield_tills_without_sneaking() {
    use crate::item::{ItemBehaviour, items::hoe::HoeItem};
    use pumpkin_inventory::player::player_inventory::PlayerInventory;
    let fixture = Fixture::new();
    let player = TestBedrockPlayer::new(&fixture.world).await;
    let pos = BlockPos::new(8, 64, 9);
    fixture.world.set_block_state(
        &pos,
        Block::GRASS_BLOCK.default_state.id,
        BlockFlags::FORCE_STATE,
    );
    player.player.inventory.set_stack(
        PlayerInventory::OFF_HAND_SLOT,
        ItemStack::new(1, &Item::SHIELD),
    );
    let mut stack = ItemStack::new(1, &Item::IRON_HOE);
    HoeItem.use_on_block(
        &mut stack,
        &player.player,
        pos,
        BlockDirection::Up,
        Vector3::new(0.5, 1.0, 0.5),
        &Block::GRASS_BLOCK,
        &fixture.server,
    );
    assert_eq!(fixture.world.get_block(&pos), &Block::FARMLAND);
    assert_eq!(stack.get_damage(), 1);
    player.close().await;
    fixture.shutdown().await;
}

struct CountPortalEvents(Arc<AtomicUsize>);
impl EventHandler<EntityPortalEvent> for CountPortalEvents {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut EntityPortalEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guarded_boat_clears_portal_contact_without_repeated_events() {
    use crate::world::portal::{PortalProcessor, PortalType};
    let fixture = Fixture::new();
    let boat = fixture.entity(&EntityType::OAK_CHEST_BOAT);
    let events = Arc::new(AtomicUsize::new(0));
    fixture.server.plugin_manager.register(
        Arc::new(CountPortalEvents(events.clone())),
        EventPriority::Normal,
        true,
    );
    let portal = BlockPos::new(8, 64, 8);
    *boat.get_entity().portal_manager.lock().unwrap() = Some(PortalProcessor::new(
        PortalType::Nether,
        portal,
        fixture.world.clone(),
    ));
    for _ in 0..3 {
        boat.tick(boat.as_ref(), &fixture.server);
        boat.get_entity()
            .try_use_portal(fixture.world.clone(), portal);
        assert!(boat.get_entity().portal_manager.lock().unwrap().is_none());
    }
    assert_eq!(events.load(Ordering::Relaxed), 0);
    fixture.shutdown().await;
}
