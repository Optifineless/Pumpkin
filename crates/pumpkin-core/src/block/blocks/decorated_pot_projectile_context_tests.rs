use super::*;
use crate::{
    entity::projectile::{egg::EggEntity, trident::TridentEntity},
    plugin::api::events::world::generic_game::GenericGameEvent,
};
use pumpkin_data::game_event::GameEvent;
use serde_json::json;

fn load_projectile_loot(fixture: &Fixture) {
    let pack = fixture
        .directory
        .path()
        .join("datapacks/projectile_context");
    let tables = pack.join("data/minecraft/loot_table/blocks");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"min_format":94,"max_format":94,"description":"projectile loot context"}}"#,
    )
    .unwrap();
    let table = json!({"pools":[
        {"rolls":1,"entries":[{"type":"minecraft:dynamic","name":"minecraft:sherds"}]},
        {"rolls":1,"conditions":[{
            "condition":"minecraft:entity_properties", "entity":"this",
            "predicate":{"type":"minecraft:arrow","flags":{"is_on_fire":false}}
        }],"entries":[{"type":"minecraft:item","name":"minecraft:emerald"}]},
        {"rolls":1,"conditions":[{
            "condition":"minecraft:match_tool", "predicate":{"items":"minecraft:diamond_pickaxe"}
        }],"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}]}
    ]});
    std::fs::write(tables.join("decorated_pot.json"), table.to_string()).unwrap();
    fixture.server.datapack_manager.load_all(
        fixture.directory.path(),
        &["file/projectile_context".to_owned()],
        &fixture.server.recipe_manager,
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_loot_retains_this_entity_type_and_state() {
    let fixture = Fixture::new();
    let position = crafted_content_pot(&fixture);
    load_projectile_loot(&fixture);
    let arrow = launch(&fixture, position, ItemStack::new(1, &Item::ARROW));
    fixture.give(ItemStack::new(1, &Item::DIAMOND_PICKAXE));
    collide(&fixture, &arrow);
    assert_shattered(&fixture, position);
    assert_eq!(item_count(&fixture.world, &Item::EMERALD), 1);
    assert_eq!(
        item_count(&fixture.world, &Item::GOLD_INGOT),
        0,
        "the projectile owner hand is never borrowed as the tool"
    );
    fixture.shutdown().await;
}

struct CountDestroy(Arc<AtomicUsize>);
impl EventHandler<GenericGameEvent> for CountDestroy {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == GameEvent::BlockDestroy.name() {
            self.0.fetch_add(1, Relaxed);
        }
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_projectile_block_destroy_event_only_after_success() {
    for admission in 0..4 {
        let cancelled = admission != 0;
        let fixture = Fixture::new();
        let position = crafted_content_pot(&fixture);
        let count = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<GenericGameEvent, _>(
                Arc::new(CountDestroy(count.clone())),
                EventPriority::Normal,
                true,
            );
        if admission == 1 {
            fixture
                .server
                .plugin_manager
                .register::<BlockBreakEvent, _>(Arc::new(CancelBreak), EventPriority::Normal, true);
        }
        if admission == 2 {
            fixture
                .server
                .plugin_manager
                .register::<ProjectileHitEvent, _>(
                    Arc::new(CancelHit),
                    EventPriority::Normal,
                    true,
                );
        } else if admission == 3 {
            fixture.world.level_info.rcu(|info| {
                let mut next = (**info).clone();
                next.game_rules.projectiles_can_break_blocks = false;
                next
            });
        }
        let arrow = shoot(&fixture, position, ItemStack::new(1, &Item::ARROW));
        arrow.on_hit(impact(position));
        assert_eq!(count.load(Relaxed), usize::from(!cancelled));
        if cancelled {
            assert_retained(&fixture, position);
        } else {
            assert_shattered(&fixture, position);
        }
        fixture.shutdown().await;
    }
}

struct AdmitHit {
    cancelled: bool,
    count: Arc<AtomicUsize>,
}
impl EventHandler<ProjectileHitEvent> for AdmitHit {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut ProjectileHitEvent,
    ) -> BoxFuture<'a, ()> {
        self.count.fetch_add(1, Relaxed);
        event.cancelled = self.cancelled;
        Box::pin(async {})
    }
}

fn non_arrow(fixture: &Fixture, trident: bool) -> Arc<dyn EntityBase> {
    let start = Vector3::new(5.5, 65.6, 8.5);
    let owner = fixture.player.player.get_entity();
    let projectile: Arc<dyn EntityBase> = if trident {
        Arc::new(TridentEntity::new_shot(
            Entity::new(fixture.world.clone(), start, &EntityType::TRIDENT),
            owner,
            ItemStack::new(1, &Item::TRIDENT),
            crate::entity::projectile::arrow::ArrowPickup::Disallowed,
        ))
    } else {
        Arc::new(EggEntity::new_shot(
            Entity::new(fixture.world.clone(), start, &EntityType::EGG),
            owner,
        ))
    };
    projectile.get_entity().set_pos(start);
    projectile
        .get_entity()
        .velocity
        .store(Vector3::new(3.0, 0.0, 0.0));
    assert!(fixture.world.spawn_entity(projectile.clone()));
    projectile
}

async fn verify_non_arrow_admission(trident: bool) {
    for cancelled in [true, false] {
        let fixture = Fixture::new();
        let position = crafted_content_pot(&fixture);
        fixture.approach(position);
        let count = Arc::new(AtomicUsize::new(0));
        fixture
            .server
            .plugin_manager
            .register::<ProjectileHitEvent, _>(
                Arc::new(AdmitHit {
                    cancelled,
                    count: count.clone(),
                }),
                EventPriority::Normal,
                true,
            );
        let projectile = non_arrow(&fixture, trident);
        projectile.tick(projectile.as_ref(), &fixture.server);
        if cancelled {
            assert_retained(&fixture, position);
        } else {
            assert_shattered(&fixture, position);
        }
        assert_eq!(count.load(Relaxed), 1, "one real flight impact is admitted");
        fixture.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_egg_hit_admission_preserves_cancelled_pots() {
    verify_non_arrow_admission(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b1020b_g09_trident_hit_admission_preserves_cancelled_pots() {
    verify_non_arrow_admission(true).await;
}
