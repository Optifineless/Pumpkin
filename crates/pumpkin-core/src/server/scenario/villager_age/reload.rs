//! Full NBT loads and command reloads preserve vanilla age semantics.
use super::*;

const AGE_KEYS: [&str; 4] = ["Age", "ForcedAge", "AgeLocked", SCHEMA];

fn assert_age(scenario: &Scenario, id: Uuid, expected: (i32, i32, bool), quarantined: bool) {
    with_villager(scenario, id, |villager| {
        let value = villager.biological_age();
        assert_eq!((value.age, value.forced_age, value.locked), expected);
        assert_eq!(value.quarantined, quarantined);
        assert_eq!(villager.is_baby(), expected.0 < 0);
        let dimensions = if expected.0 < 0 {
            (0.49, 0.98)
        } else {
            <(f32, f32)>::from(EntityType::VILLAGER.dimension)
        };
        assert_eq!(
            (
                villager.get_entity().width(),
                villager.get_entity().height()
            ),
            dimensions
        );
    });
}

fn assert_canonical(scenario: &Scenario, id: Uuid, expected: (i32, i32, bool)) {
    with_villager(scenario, id, |villager| {
        let mut saved = NbtCompound::new();
        villager.write_nbt(&mut saved);
        assert_eq!(saved.get("Age"), Some(&NbtTag::Int(expected.0)));
        assert_eq!(saved.get("ForcedAge"), Some(&NbtTag::Int(expected.1)));
        assert_eq!(
            saved.get("AgeLocked"),
            Some(&NbtTag::Byte(i8::from(expected.2)))
        );
        assert_eq!(saved.get(SCHEMA), Some(&NbtTag::Int(1)));
    });
}

#[tokio::test]
async fn villager_s6_r2_numeric_forms_match_vanilla() {
    bounded(async {
        let scenario = Scenario::new().await;
        // Literal results from NumericTag.intValue/byteValue, including Mth.floor.
        let cases = [
            (NbtTag::Byte(-2), -2, true),
            (NbtTag::Short(256), 256, false),
            (NbtTag::Int(257), 257, true),
            (NbtTag::Long(4_294_967_295), -1, true),
            (NbtTag::Float(-0.5), -1, true),
            (NbtTag::Double(256.75), 256, false),
            (NbtTag::Byte(2), 2, true),
            (NbtTag::Long(i64::MIN), 0, false),
            (NbtTag::Long(i64::MAX), -1, true),
            (NbtTag::Float(f32::NAN), 0, false),
            (NbtTag::Double(f64::NAN), 0, false),
            (NbtTag::Float(f32::INFINITY), i32::MAX, true),
            (NbtTag::Double(f64::INFINITY), i32::MAX, true),
            (NbtTag::Float(f32::NEG_INFINITY), i32::MIN, false),
            (NbtTag::Double(f64::NEG_INFINITY), i32::MIN, false),
            (NbtTag::Double(-2.25), -3, true),
            (NbtTag::Float(0.75), 0, false),
        ];
        let ids: Vec<_> = cases.iter().map(|_| Uuid::new_v4()).collect();
        let records = cases
            .iter()
            .zip(&ids)
            .map(|((tag, _, _), id)| {
                let mut record = fixture(*id, 0, false);
                for key in ["Age", "ForcedAge", "AgeLocked"] {
                    record.put(key, tag.clone());
                }
                record
            })
            .collect();
        seed(&scenario, records).await;
        activate(&scenario, "AgeNumbers").await;
        for ((_, value, locked), id) in cases.iter().zip(&ids) {
            assert_age(&scenario, *id, (*value, *value, *locked), false);
            assert_canonical(&scenario, *id, (*value, *value, *locked));
        }
        // Exercise the SNBT parser, dispatcher and existing-entity reload as well.
        let id = ids[0];
        for (input, expected) in [
            ("Age:-4b,ForcedAge:3b,AgeLocked:2b", (-4, 3, true)),
            ("Age:-5s,ForcedAge:4s,AgeLocked:256s", (-5, 4, false)),
            ("Age:-6,ForcedAge:5,AgeLocked:257", (-6, 5, true)),
            (
                "Age:4294967295L,ForcedAge:4294967302L,AgeLocked:256L",
                (-1, 6, false),
            ),
            ("Age:-7.5f,ForcedAge:7.75f,AgeLocked:-0.5f", (-8, 7, true)),
            (
                "Age:-8.5d,ForcedAge:8.75d,AgeLocked:256.75d",
                (-9, 8, false),
            ),
        ] {
            assert_eq!(
                scenario.command(&format!("data merge entity {id} {{{input}}}")),
                1
            );
            assert_age(&scenario, id, expected, false);
            assert_canonical(&scenario, id, expected);
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_s6_r2_malformed_reload_preserves_and_recovers_same_owner() {
    bounded(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        let fresh = Uuid::new_v4();
        let mut malformed = fixture(fresh, -20, true);
        malformed.put_string("Age", "broken".into());
        seed(&scenario, vec![fixture(id, -20, true), malformed]).await;
        let (java, _) = activate(&scenario, "AgeRecovery").await;
        assert_age(&scenario, fresh, (0, 0, false), true);
        let owner = java.player.world().get_entity_by_uuid(id).unwrap();
        for (key, input, raw) in [
            ("Age", "\"broken\"", NbtTag::String("broken".into())),
            ("ForcedAge", "{}", NbtTag::Compound(NbtCompound::new())),
            ("AgeLocked", "\"broken\"", NbtTag::String("broken".into())),
            (SCHEMA, "99", NbtTag::Int(99)),
        ] {
            scenario.command(&format!("data merge entity {id} {{\"{key}\":{input}}}"));
            assert_age(&scenario, id, (-20, 17, true), true);
            with_villager(&scenario, id, |villager| {
                assert!(!villager.set_biological_age(0));
                assert!(!villager.is_age_ready());
                let mut saved = NbtCompound::new();
                villager.write_nbt(&mut saved);
                assert_eq!(saved.get(key), Some(&raw), "raw invalid tag retained");
            });
            scenario.ticks(1).await;
            assert_age(&scenario, id, (-20, 17, true), true);
            scenario.command(&format!(
                "data merge entity {id} {{Age:-9,ForcedAge:5,AgeLocked:0b,\"{SCHEMA}\":1}}"
            ));
            assert!(Arc::ptr_eq(
                &owner,
                &java.player.world().get_entity_by_uuid(id).unwrap()
            ));
            assert_age(&scenario, id, (-9, 5, false), false);
            assert_canonical(&scenario, id, (-9, 5, false));
            scenario.ticks(1).await;
            assert_age(&scenario, id, (-8, 5, false), false);
            scenario.command(&format!(
                "data merge entity {id} {{Age:-20,ForcedAge:17,AgeLocked:1b}}"
            ));
        }
        scenario.stop().await;
    })
    .await;
}

#[tokio::test]
async fn villager_s6_r2_boundary_effects_require_status_change() {
    bounded(async {
        let scenario = Scenario::new().await;
        let id = Uuid::new_v4();
        seed(&scenario, vec![fixture(id, -20, true)]).await;
        let (java, _) = activate(&scenario, "AgeBoundaries").await;
        // Watch after hydration but before admission: neither the reader nor the
        // newborn setter may leave a deferred initial boundary for the first tick.
        for configured in [true, false] {
            let world = java.player.world();
            let initial_id = Uuid::new_v4();
            let entity = if configured {
                crate::entity::mob::spawn::load_spawn_entity(
                    &world,
                    &fixture(initial_id, -20, true),
                    SPAWN,
                )
                .unwrap()
            } else {
                crate::entity::r#type::from_type(&EntityType::VILLAGER, SPAWN, &world, initial_id)
            };
            let villager = entity.cast_any().downcast_ref::<VillagerEntity>().unwrap();
            if !configured {
                assert!(villager.set_baby(true));
                villager.mob_entity.set_no_ai(true);
            }
            let stopped = Arc::new(AtomicBool::new(false));
            let observed = stopped.clone();
            navigation::watch_actions(
                villager,
                None,
                Some(Box::new(move || {
                    observed.store(true, Relaxed);
                })),
            );
            world.add_entity_silent(entity);
            scenario.ticks(1).await;
            assert!(
                !stopped.load(Relaxed),
                "initial projection ran boundary effects"
            );
            assert_age(
                &scenario,
                initial_id,
                if configured {
                    (-20, 17, true)
                } else {
                    (-23999, 0, false)
                },
                false,
            );
        }
        for (same_status, crossing) in [
            (
                ["Age:-30", "ForcedAge:9", "AgeLocked:0b", "NoAI:0b"],
                "Age:0",
            ),
            (
                ["Age:20", "Age:0", "ForcedAge:0", "AgeLocked:1b"],
                "Age:-20",
            ),
        ] {
            let stopped = Arc::new(AtomicBool::new(false));
            let observed = stopped.clone();
            with_villager(&scenario, id, |villager| {
                navigation::watch_actions(
                    villager,
                    None,
                    Some(Box::new(move || {
                        observed.store(true, Relaxed);
                    })),
                );
            });
            for input in same_status {
                scenario.command(&format!("data merge entity {id} {{{input}}}"));
                assert!(
                    !stopped.load(Relaxed),
                    "same-status reload stopped navigation: {input}"
                );
            }
            scenario.command(&format!("data merge entity {id} {{{crossing}}}"));
            assert!(
                stopped.load(Relaxed),
                "real boundary did not stop navigation"
            );
        }
        scenario.stop().await;
    })
    .await;
}

fn without_age(mut record: NbtCompound) -> NbtCompound {
    for key in AGE_KEYS {
        record.child_tags.remove(key);
    }
    record
}

#[tokio::test]
async fn villager_s6_r2_retaining_rollback_roundtrip() {
    bounded(async {
        let scenario = Scenario::new().await;
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        seed(
            &scenario,
            vec![fixture(ids[0], -24000, true), fixture(ids[1], 1234, true)],
        )
        .await;
        activate(&scenario, "AgeNewBuild").await;
        let scenario = scenario.restart().await;
        let mut source = Vec::new();
        for id in ids {
            source.push(saved_record(&scenario, id).await);
        }
        // Model #414's unknown-tag retention contract. This branch has no generic
        // unknown-tag owner yet: hide age input from the real loader, and replace
        // its canonical defaults with the opaque tags after the real shutdown save.
        replace_dormant_records(&scenario, source.iter().cloned().map(without_age).collect());
        activate(&scenario, "AgeRollback").await;
        for id in ids {
            assert_age(&scenario, id, (0, 0, false), false);
        }
        let scenario = scenario.restart().await;
        let mut retained = Vec::new();
        for (id, original) in ids.into_iter().zip(&source) {
            let mut saved = without_age(saved_record(&scenario, id).await);
            for key in AGE_KEYS {
                saved.put(key, original.get(key).unwrap().clone());
            }
            for key in [
                "UUID",
                "Offers",
                "Inventory",
                "Gossips",
                "Brain",
                "HomeX",
                "HomeY",
                "HomeZ",
            ] {
                assert_eq!(saved.get(key), original.get(key), "rollback retained {key}");
            }
            retained.push(saved);
        }
        replace_dormant_records(&scenario, retained);
        activate(&scenario, "AgeRollForward").await;
        for (id, expected) in ids.into_iter().zip([(-24000, 17, true), (1234, 17, true)]) {
            assert_age(&scenario, id, expected, false);
            assert_canonical(&scenario, id, expected);
        }
        scenario.stop().await;
    })
    .await;
}

fn replace_dormant_records(scenario: &Scenario, records: Vec<NbtCompound>) {
    let world = scenario.world(&Dimension::OVERWORLD);
    let storage = world
        .level
        .get_entity_chunk_sync(&Vector2::new(0, 0))
        .unwrap();
    assert!(!storage.live.load(Relaxed));
    *storage.data.lock().unwrap() = records;
}
