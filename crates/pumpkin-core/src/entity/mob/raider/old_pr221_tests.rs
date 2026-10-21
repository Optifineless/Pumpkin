#![expect(
    clippy::unwrap_used,
    reason = "Real banner regression fixtures require valid packets and entities"
)]

mod ingress;
mod review2;
mod support;

use super::create_ominous_banner;
use pumpkin_data::{
    data_component::DataComponent,
    data_component_impl::{
        BannerPatternLayer, BannerPatternsImpl, CustomNameImpl, DataComponentImpl, EquipmentSlot,
    },
    dye_color::DyeColor,
};
use pumpkin_util::text::TextComponent;
use support::{
    Fixture, Item, ItemStack, NbtCompound, POSITION, TestPlayer, assert_canonical, click,
    equipment, head, mismatched_click,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_raid_captain_banner_reaches_existing_and_late_java_observers() {
    Fixture::new()
        .verify(|fixture| {
            let mut raid =
                crate::world::raid::Raid::new(1, POSITION, pumpkin_util::Difficulty::Easy);
            raid.raid_cooldown_ticks = 0;
            raid.wave_spawn_pos = Some(POSITION);
            raid.tick(&fixture.world);
            assert_eq!(raid.groups_spawned, 1, "real raid tick spawned a wave");
            let captain = fixture
                .world
                .get_entity_by_uuid(raid.group_to_leader_map[&1])
                .unwrap();
            let canonical = head(&captain);
            assert_canonical(&canonical);
            let spawned = equipment(&mut fixture.client, captain.get_entity().entity_id);
            assert!(
                spawned.contains_key(&EquipmentSlot::HEAD.discriminant()),
                "existing observer receives equipment from real raid-wave insertion"
            );
            assert_canonical(&spawned[&EquipmentSlot::HEAD.discriminant()]);
            fixture.world.raids.lock().unwrap().raid_map.insert(1, raid);
            let armor = ItemStack::new(1, &Item::IRON_CHESTPLATE);
            let living = captain.get_living_entity().unwrap();
            living
                .entity_equipment
                .lock()
                .unwrap()
                .put(&EquipmentSlot::CHEST, armor.clone());
            let tracked = fixture
                .world
                .entity_tracker
                .get_tracked_entity(captain.get_entity().entity_id)
                .unwrap();
            tracked.seen_by.insert(fixture.client.player.gameprofile.id);
            fixture.client.take_packets();
            living.send_equipment_changes(&[
                (EquipmentSlot::HEAD, canonical.clone()),
                (EquipmentSlot::CHEST, armor.clone()),
            ]);
            let sent = equipment(&mut fixture.client, captain.get_entity().entity_id);
            assert_canonical(&sent[&EquipmentSlot::HEAD.discriminant()]);
            assert!(sent[&EquipmentSlot::CHEST.discriminant()].are_equal(&armor));
            let mut late = TestPlayer::new(&fixture.world);
            late.take_packets();
            tracked.add_pairing(&late.player);
            let paired = equipment(&mut late, captain.get_entity().entity_id);
            assert_canonical(&paired[&EquipmentSlot::HEAD.discriminant()]);
            assert!(paired[&EquipmentSlot::CHEST.discriminant()].are_equal(&armor));
            let mut saved = NbtCompound::new();
            captain.write_nbt(&mut saved);
            let restored = fixture.pillager();
            restored.read_nbt_non_mut(&saved);
            assert_canonical(&head(&restored));
            assert_eq!(
                restored
                    .get_mob()
                    .unwrap()
                    .get_mob_entity()
                    .drop_chance(&EquipmentSlot::HEAD),
                2.0
            );
            fixture.kill(&restored);
            assert!(
                fixture
                    .stacks(&Item::WHITE_BANNER)
                    .iter()
                    .any(|stack| stack.are_equal(&canonical)),
                "death conserved canonical stack"
            );
            let patrol = fixture.pillager();
            let patrol_mob = patrol.get_mob().unwrap().as_patrolling_monster().unwrap();
            patrol_mob.set_patrol_leader(true);
            patrol_mob.finalize_patrol_spawn(true);
            assert_canonical(&head(&patrol));
            check_saved_controls(fixture);
        })
        .await;
}

fn check_saved_controls(fixture: &Fixture) {
    let plain = ItemStack::new(1, &Item::WHITE_BANNER);
    let mut legacy = plain.clone();
    legacy.patch.push((
        DataComponent::CustomName,
        Some(
            CustomNameImpl {
                name: TextComponent::translate("block.minecraft.ominous_banner", []),
            }
            .to_dyn(),
        ),
    ));
    let mut custom = plain.clone();
    custom.patch.push((
        DataComponent::BannerPatterns,
        Some(
            BannerPatternsImpl {
                layers: vec![BannerPatternLayer {
                    pattern: "minecraft:border".into(),
                    color: DyeColor::Red,
                    translation_key: None,
                }],
            }
            .to_dyn(),
        ),
    ));
    for stack in [plain, legacy, custom] {
        let mob = fixture.pillager();
        mob.get_living_entity()
            .unwrap()
            .entity_equipment
            .lock()
            .unwrap()
            .put(&EquipmentSlot::HEAD, stack.clone());
        let mut saved = NbtCompound::new();
        mob.write_nbt(&mut saved);
        let loaded = fixture.pillager();
        loaded.read_nbt_non_mut(&saved);
        assert!(
            head(&loaded).are_equal(&stack),
            "ordinary, custom and old named banners retain saved identity"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_complete_banner_click_hash_matches_wire_projection() {
    Fixture::new()
        .verify(|fixture| {
            let captain = fixture.pillager();
            let patrol = captain.get_mob().unwrap().as_patrolling_monster().unwrap();
            patrol.set_patrol_leader(true);
            patrol.finalize_patrol_spawn(true);
            let canonical = head(&captain);
            // Supplied HashOps CODECs independently executed in pr221_independent_fixtures.py.
            click(
                fixture,
                &canonical,
                &[
                    (74, 0x5349_c6bc),
                    (18, 0x4c2d_2266),
                    (9, 0x0fc5_df16),
                    (12, 0xb3cc_bad2),
                ],
            );
            let mut projected = ItemStack::new(1, &Item::WHITE_BANNER);
            projected.patch.push((
                DataComponent::BannerPatterns,
                Some(
                    BannerPatternsImpl {
                        layers: vec![
                            BannerPatternLayer {
                                pattern: "border".into(),
                                color: DyeColor::Black,
                                translation_key: None,
                            },
                            BannerPatternLayer {
                                pattern: "example:unsupported".into(),
                                color: DyeColor::Red,
                                translation_key: None,
                            },
                        ],
                    }
                    .to_dyn(),
                ),
            ));
            let armor = ItemStack::new(1, &Item::IRON_CHESTPLATE);
            let living = captain.get_living_entity().unwrap();
            living
                .entity_equipment
                .lock()
                .unwrap()
                .put(&EquipmentSlot::HEAD, projected.clone());
            living
                .entity_equipment
                .lock()
                .unwrap()
                .put(&EquipmentSlot::CHEST, armor.clone());
            let tracked = fixture
                .world
                .entity_tracker
                .get_tracked_entity(captain.get_entity().entity_id)
                .unwrap();
            fixture.client.take_packets();
            tracked.add_pairing(&fixture.client.player);
            let published = equipment(&mut fixture.client, captain.get_entity().entity_id);
            assert_eq!(
                published[&EquipmentSlot::HEAD.discriminant()]
                    .get_data_component::<BannerPatternsImpl>()
                    .unwrap()
                    .layers
                    .len(),
                1
            );
            let visible = &published[&EquipmentSlot::HEAD.discriminant()]
                .get_data_component::<BannerPatternsImpl>()
                .unwrap()
                .layers[0];
            assert_eq!(visible.pattern, "minecraft:border");
            assert_eq!(visible.color, DyeColor::Black);
            assert!(
                published[&EquipmentSlot::CHEST.discriminant()].are_equal(&armor),
                "unsupported layer must preserve another equipment slot"
            );
            click(fixture, &projected, &[(74, 0x6188_5d54)]);
            let mut saved = NbtCompound::new();
            projected.write_item_stack(&mut saved);
            let loaded = ItemStack::read_item_stack(&saved).unwrap();
            assert_eq!(
                loaded
                    .get_data_component::<BannerPatternsImpl>()
                    .unwrap()
                    .layers[1]
                    .pattern,
                "example:unsupported"
            );
            let direct = ingress::direct_stack();
            click(fixture, &direct, &[(74, 0xb185_4b2a)]);
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_existing_captain_admission_and_malformed_name_controls() {
    Fixture::new()
        .verify(|fixture| {
            let candidate = fixture.pillager();
            let raider = candidate.get_mob().unwrap().as_raider().unwrap();
            raider.get_raider_data().raid_id.store(Some(9));
            let mut raid =
                crate::world::raid::Raid::new(9, POSITION, pumpkin_util::Difficulty::Easy);
            raid.join_raid(0, candidate.get_entity().entity_uuid, &candidate);
            fixture.world.raids.lock().unwrap().raid_map.insert(9, raid);
            fixture
                .world
                .drop_stack(&POSITION, ItemStack::new(1, &Item::WHITE_BANNER));
            fixture.release_items();
            ai_steps(&candidate);
            assert!(
                !raider.is_patrol_leader(),
                "plain banner cannot grant canonical leadership"
            );
            assert_eq!(fixture.stacks(&Item::WHITE_BANNER).len(), 1);
            fixture.world.drop_stack(&POSITION, create_ominous_banner());
            fixture.release_items();
            ai_steps(&candidate);
            assert!(raider.is_patrol_leader());
            assert_canonical(&head(&candidate));
            assert_eq!(
                fixture.stacks(&Item::WHITE_BANNER).len(),
                1,
                "pickup transfers only canonical stock"
            );
            fixture.world.raids.lock().unwrap().raid_map.clear();
            raider.get_raider_data().raid_id.store(None);
            fixture.kill(&candidate);
            assert_eq!(
                fixture.stacks(&Item::WHITE_BANNER).len(),
                2,
                "guaranteed captain drop conserves stock"
            );
            assert_eq!(
                fixture.stacks(&Item::OMINOUS_BOTTLE).len(),
                1,
                "outside-raid captain retains bottle loot"
            );
            ingress::malformed_name_control(fixture);
        })
        .await;
}

fn ai_steps(candidate: &std::sync::Arc<dyn support::EntityBase>) {
    let mob = candidate.get_mob().unwrap();
    // GoalSelector updates admission periodically; advance a bounded number of real AI steps.
    for _ in 0..4 {
        mob.get_mob_entity().server_ai_step(mob, candidate.as_ref());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_plain_custom_legacy_and_malformed_name_preservation_controls() {
    Fixture::new()
        .verify(|fixture| {
            click(fixture, &ItemStack::new(1, &Item::WHITE_BANNER), &[]);
            check_saved_controls(fixture);
            ingress::malformed_name_control(fixture);
        })
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_pr221_mismatched_click_claim_corrects_cursor_without_losing_authoritative_items() {
    Fixture::new()
        .verify(|fixture| {
            let mut banner = ItemStack::new(1, &Item::WHITE_BANNER);
            banner.set_data_component(BannerPatternsImpl {
                layers: vec![BannerPatternLayer {
                    pattern: "minecraft:border".into(),
                    color: DyeColor::Black,
                    translation_key: None,
                }],
            });
            // Independent border/black checksum 0x61885d54, deliberately changed by one bit.
            mismatched_click(fixture, &banner, &[(74, 0x6188_5d55)]);
        })
        .await;
}
