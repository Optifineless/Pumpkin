use super::*;
use crate::{
    entity::{
        Entity,
        projectile::{
            ProjectileHit, arrow::ArrowEntity, experience_bottle::ExperienceBottleEntity,
        },
    },
    item::items::bow::BowItem,
    plugin::api::events::{
        block::block_break::BlockBreakEvent,
        entity::{projectile_hit::ProjectileHitEvent, projectile_launch::ProjectileLaunchEvent},
    },
};
use pumpkin_data::{data_component_impl::CanBreakImpl, entity::EntityType};
use pumpkin_inventory::{
    crafting::crafting_screen_handler::CraftingTableScreenHandler, screen_handler::ScreenHandler,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_protocol::java::server::play::SlotActionType;
use std::sync::atomic::AtomicUsize;

#[path = "decorated_pot_projectile_overlap_tests.rs"]
mod overlap;

#[path = "decorated_pot_projectile_context_tests.rs"]
mod context;
#[path = "decorated_pot_mixed_break_tests.rs"]
mod mixed_break;
#[path = "decorated_pot_pending_hit_tests.rs"]
mod pending_hit;
#[path = "decorated_pot_removal_tests.rs"]
mod removal;

const FACES: [(&Item, &str); 4] = [
    (&Item::ANGLER_POTTERY_SHERD, "arrow-back"),
    (&Item::BRICK, "arrow-left"),
    (&Item::ARCHER_POTTERY_SHERD, "arrow-right"),
    (&Item::BRICK, "arrow-front"),
];

fn crafted_content_pot(fixture: &Fixture) -> BlockPos {
    let mut menu = CraftingTableScreenHandler::new(1, fixture.player.player.inventory(), None);
    for (slot, (item, name)) in [2, 4, 6, 8].into_iter().zip(FACES) {
        let mut stack = ItemStack::new(1, item);
        stack.set_custom_name(name.to_owned());
        menu.get_behaviour().slots[slot].set_stack(stack);
    }
    menu.send_content_updates();
    menu.on_slot_click(0, 0, SlotActionType::Pickup, fixture.player.player.as_ref());
    let pot = {
        let mut cursor = menu
            .get_behaviour()
            .cursor_stack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *cursor, ItemStack::EMPTY.clone())
    };
    assert_eq!(pot.item, &Item::DECORATED_POT);
    for slot in [2, 4, 6, 8] {
        assert!(menu.get_behaviour().slots[slot].get_stack().is_empty());
    }
    let support = BlockPos::new(8, 64, 8);
    fixture.set_block(support, &Block::STONE);
    fixture.give(pot);
    fixture.use_on(support);
    fixture.insert(support.up(), &Item::DIAMOND);
    support.up()
}

fn shoot(fixture: &Fixture, position: BlockPos, ammunition: ItemStack) -> Arc<dyn EntityBase> {
    let arrow = launch(fixture, position, ammunition);
    collide(fixture, &arrow);
    arrow
}

fn launch(fixture: &Fixture, position: BlockPos, ammunition: ItemStack) -> Arc<dyn EntityBase> {
    let shooter = fixture.player.player.get_entity();
    shooter.set_pos(Vector3::new(
        f64::from(position.0.x) - 2.5,
        f64::from(position.0.y) + 0.6 - shooter.get_eye_height() + 0.1,
        f64::from(position.0.z) + 0.5,
    ));
    shooter.set_rotation(-90.0, 0.0);
    BowItem::shoot(
        fixture.player.player.as_ref(),
        &ItemStack::new(1, &Item::BOW),
        &[ammunition],
        1.0,
        0.0,
        false,
    );
    fixture
        .world
        .entities
        .load()
        .iter()
        .rev()
        .find(|entity| entity.get_entity().entity_type == &EntityType::ARROW)
        .expect("the real weapon launch was admitted into the world")
        .clone()
}

fn collide(fixture: &Fixture, arrow: &Arc<dyn EntityBase>) {
    for _ in 0..4 {
        arrow.tick(arrow.as_ref(), &fixture.server);
        if arrow
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap()
            .in_ground
            .load(Relaxed)
        {
            break;
        }
    }
    assert!(
        arrow
            .cast_any()
            .downcast_ref::<ArrowEntity>()
            .unwrap()
            .in_ground
            .load(Relaxed),
        "production arrow collision reached the pot"
    );
}

fn impact(position: BlockPos) -> ProjectileHit {
    ProjectileHit::Block {
        pos: position,
        world_border: false,
        face: BlockDirection::West,
        hit_pos: Vector3::new(8.0625, 65.6, 8.5),
        normal: Vector3::new(-1.0, 0.0, 0.0),
    }
}

fn assert_retained(fixture: &Fixture, position: BlockPos) {
    assert_eq!(fixture.world.get_block(&position), &Block::DECORATED_POT);
    assert!(
        !DecoratedPotLikeProperties::from_state_id(fixture.world.get_block_state_id(&position))
            .cracked
    );
    let entity = fixture.world.get_block_entity(&position).unwrap();
    assert_eq!(
        entity
            .as_any()
            .downcast_ref::<crate::block::entities::decorated_pot::DecoratedPotBlockEntity>()
            .unwrap()
            .get_item()
            .unwrap()
            .item,
        &Item::DIAMOND
    );
    assert_eq!(
        fixture
            .world
            .entities
            .load()
            .iter()
            .filter(|e| e.get_item_entity().is_some())
            .count(),
        0
    );
}

fn assert_shattered(fixture: &Fixture, position: BlockPos) {
    assert_eq!(
        fixture.world.get_block(&position),
        &Block::AIR,
        "an admitted arrow shatters the pot"
    );
    assert!(fixture.world.get_block_entity(&position).is_none());
    assert_eq!(item_count(&fixture.world, &Item::DECORATED_POT), 0);
    assert_eq!(item_count(&fixture.world, &Item::BRICK), 2);
    assert_eq!(item_count(&fixture.world, &Item::ANGLER_POTTERY_SHERD), 1);
    assert_eq!(item_count(&fixture.world, &Item::ARCHER_POTTERY_SHERD), 1);
    assert_eq!(item_count(&fixture.world, &Item::DIAMOND), 1);
    for (item, name) in FACES {
        let count: u32 = fixture
            .world
            .entities
            .load()
            .iter()
            .filter_map(|entity| entity.get_item_entity())
            .map(|entity| {
                let stack = entity.get_item_stack().lock().unwrap();
                if stack.item == item && stack.get_hover_name() == name {
                    u32::from(stack.item_count)
                } else {
                    0
                }
            })
            .sum();
        assert_eq!(count, 1, "face {name} drops once with its crafted patch");
    }
}

struct LaunchCount(Arc<AtomicUsize>);
impl EventHandler<ProjectileLaunchEvent> for LaunchCount {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        _event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        self.0.fetch_add(1, Relaxed);
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_hit_shatters_once() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    let launches = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .plugin_manager
        .register::<ProjectileLaunchEvent, _>(
            Arc::new(LaunchCount(launches.clone())),
            EventPriority::Normal,
            true,
        );
    let arrow = shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
    assert_eq!(launches.load(Relaxed), 1);
    assert_shattered(&fixture, position);
    arrow.on_hit(impact(position));
    arrow.tick(arrow.as_ref(), &fixture.server);
    assert_shattered(&fixture, position);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_denied_permission_retains_pot() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    fixture.server.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.spawn_x = position.0.x;
        next.spawn_z = position.0.z;
        next
    });
    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap()
        .ops
        .push(pumpkin_config::op::Op::new(
            uuid::Uuid::new_v4(),
            "pot-control".to_owned(),
            PermissionLvl::Zero,
            false,
        ));
    let arrow = shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
    assert_retained(&fixture, position);
    fixture
        .server
        .data
        .operator_config
        .write()
        .unwrap()
        .ops
        .clear();
    fixture.world.worldborder.lock().unwrap().new_diameter = 4.0;
    arrow.on_hit(impact(position));
    assert_retained(&fixture, position);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_may_break_false_retains_pot() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    fixture.world.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.game_rules.projectiles_can_break_blocks = false;
        next
    });
    shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
    assert_retained(&fixture, position);
    fixture.world.level_info.rcu(|info| {
        let mut next = (**info).clone();
        next.game_rules.projectiles_can_break_blocks = true;
        next
    });
    let bottle = ExperienceBottleEntity::new(Entity::new(
        fixture.world.clone(),
        position.to_centered_f64(),
        &EntityType::EXPERIENCE_BOTTLE,
    ));
    fixture.world.block_registry.on_projectile_hit(
        &Block::DECORATED_POT,
        &fixture.world,
        &bottle,
        &position,
        fixture.world.get_block_state(&position),
        &position.to_centered_f64(),
        &fixture.server,
    );
    // A bare entity with an impact entity type is still not a projectile.
    let bare = Entity::new(
        fixture.world.clone(),
        position.to_centered_f64(),
        &EntityType::ARROW,
    );
    fixture.world.block_registry.on_projectile_hit(
        &Block::DECORATED_POT,
        &fixture.world,
        &bare,
        &position,
        fixture.world.get_block_state(&position),
        &position.to_centered_f64(),
        &fixture.server,
    );
    assert_retained(&fixture, position);
    fixture.shutdown().await;
}

struct CancelHit;
impl EventHandler<ProjectileHitEvent> for CancelHit {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut ProjectileHitEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled = true;
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_cancelled_hit_retains_pot() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    fixture
        .server
        .plugin_manager
        .register::<ProjectileHitEvent, _>(Arc::new(CancelHit), EventPriority::Normal, true);
    shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
    assert_retained(&fixture, position);
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_cancelled_break_retains_pot() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    fixture
        .server
        .plugin_manager
        .register::<BlockBreakEvent, _>(Arc::new(CancelBreak), EventPriority::Normal, true);
    shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
    assert_retained(&fixture, position);
    fixture.shutdown().await;
}

fn permitted_arrow() -> ItemStack {
    let mut predicate = NbtCompound::new();
    predicate.put_string("blocks", "minecraft:decorated_pot".to_owned());
    let mut arrow = ItemStack::new(1, &Item::ARROW);
    arrow.set_data_component(CanBreakImpl {
        predicate: NbtTag::Compound(predicate),
    });
    arrow
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_adventure_uses_retained_ammunition() {
    let denied = Fixture::new();
    let position = crafted_content_pot(&denied);
    denied.player.player.gamemode.store(GameMode::Adventure);
    denied.give(permitted_arrow());
    shoot(&denied, position, ItemStack::new(1, &Item::ARROW));
    assert_retained(&denied, position);
    denied.shutdown().await;

    let allowed = Fixture::new();
    let position = crafted_content_pot(&allowed);
    allowed.player.player.gamemode.store(GameMode::Adventure);
    allowed.give(ItemStack::EMPTY.clone());
    shoot(&allowed, position, permitted_arrow());
    assert_shattered(&allowed, position);
    allowed.shutdown().await;
}

fn mob_arrow(fixture: &Fixture, position: BlockPos, ammunition: &ItemStack) {
    let start = Vector3::new(5.5, 65.6, 8.5);
    let owner: Arc<dyn EntityBase> =
        Arc::new(crate::entity::living::LivingEntity::new(Entity::new(
            fixture.world.clone(),
            Vector3::new(4.5, 64.0, 8.5),
            &EntityType::SKELETON,
        )));
    assert!(fixture.world.spawn_entity(owner.clone()));
    let arrow = ArrowEntity::new_shot(
        Entity::new(fixture.world.clone(), start, &EntityType::ARROW),
        owner.get_entity(),
        ammunition,
        crate::entity::projectile::arrow::ArrowPickup::Disallowed,
    );
    arrow.entity.set_pos(start);
    arrow.entity.velocity.store(Vector3::new(3.0, 0.0, 0.0));
    let arrow: Arc<dyn EntityBase> = Arc::new(arrow);
    assert!(fixture.world.spawn_entity(arrow.clone()));
    collide(fixture, &arrow);
    assert_eq!(position, BlockPos::new(8, 65, 8));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_mob_owner_denied_retains_pot() {
    for with_predicate in [false, true] {
        let fixture = Fixture::new();
        let position = crafted_content_pot(&fixture);
        fixture.world.level_info.rcu(|info| {
            let mut next = (**info).clone();
            next.game_rules.mob_griefing = !with_predicate;
            next
        });
        let ammunition = if with_predicate {
            permitted_arrow()
        } else {
            ItemStack::new(1, &Item::ARROW)
        };
        mob_arrow(&fixture, position, &ammunition);
        assert_retained(&fixture, position);
        fixture.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_mob_owner_allowed_shatters_pot() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    mob_arrow(&fixture, position, &permitted_arrow());
    assert_shattered(&fixture, position);
    fixture.shutdown().await;
}
