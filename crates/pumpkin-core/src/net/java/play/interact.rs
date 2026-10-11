#[allow(clippy::wildcard_imports)]
use super::*;

impl JavaClient {
    #[expect(clippy::too_many_lines)]
    pub fn handle_interact(
        &self,
        player: &Arc<Player>,
        interact: &SInteract,
        server: &Arc<Server>,
    ) {
        if !player.has_client_loaded() {
            return;
        }
        player.update_last_action_time();
        let entity_id = interact.entity_id;

        let sneaking = interact.sneaking;
        let player_entity = &player.get_entity();
        if player_entity.is_sneaking() != sneaking {
            player_entity.set_sneaking(sneaking);
        }
        let Ok(action) = ActionType::try_from(interact.r#type.0) else {
            self.try_kick(&TextComponent::text("Invalid action type"));
            return;
        };

        // Resolve the target entity for the event
        let world = player_entity.world.load_full();
        let player_target = world.get_player_by_id(entity_id.0);
        let target = world.get_entity_or_part(entity_id.0);

        if let Some(target) = target {
            if player.gamemode.load() == GameMode::Spectator {
                // ServerGamePacketListenerImpl.handleInteract does not set spectator cameras.
                return;
            }
            send_cancellable_blocking! {{
                server;
                PlayerInteractEntityEvent::new(
                    player,
                    Arc::clone(&target),
                    action,
                    interact.target_position,
                    sneaking,
                );

                'after: {
                    match event.action {
                        ActionType::Attack => {
                            let config = &server.advanced_config.pvp;
                            if !config.enabled {
                                return;
                            }

                            if entity_id.0 == player.entity_id() {
                                self.try_kick(&TextComponent::translate_cross(translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED, translation::java::MULTIPLAYER_DISCONNECT_INVALID_ENTITY_ATTACKED, []));
                                return;
                            }

                            if let Some(player_victim) = &player_target {
                                if player_victim.living_entity.health.load() <= 0.0 {
                                    return;
                                }
                                if config.protect_creative
                                    && player_victim.gamemode.load() == GameMode::Creative
                                {
                                    world
                                        .play_sound(
                                            Sound::EntityPlayerAttackNodamage,
                                            SoundCategory::Players,
                                            &player_victim.position(),
                                        )
                                        ;
                                    return;
                                }
                            }
                            player.attack(&event.target);
                        }
                        ActionType::Interact | ActionType::InteractAt => {
                            if event.action == ActionType::InteractAt
                                && let Some(pos) = event.target_position
                            {
                                let mut at_event = crate::plugin::api::events::player::player_interact_at_entity::PlayerInteractAtEntityEvent::new(
                                    player.clone(),
                                    event.target.get_entity().entity_id,
                                    pos.x,
                                    pos.y,
                                    pos.z,
                                    u8::from(interact.hand.map_or(0, |h| h.0) != 0),
                                );
                                server.plugin_manager.fire_blocking(server, &mut at_event);
                                if at_event.cancelled {
                                    return;
                                }
                                // Honor both interaction events' target and hit-position changes.
                                let Some(adjusted_target) = world.get_entity_or_part(at_event.entity_id) else { return; };
                                event.target = adjusted_target;
                                event.target_position = Some(Vector3::new(at_event.clicked_x, at_event.clicked_y, at_event.clicked_z));
                            }
                            let Ok(hand) = Hand::from_packet_id(interact.hand.map_or(0, |hand| hand.0)) else {
                                self.try_kick(&TextComponent::text("InvalidHand"));
                                return;
                            };
                            let source_slot = super::hand_use_result::hand_slot(player, hand);
                            let mut stack = player.inventory().get_stack_in_hand(hand);

                            let before = stack.clone();
                            let interacted = if event.action == ActionType::InteractAt {
                                match event.target_position {
                                    Some(position) => {
                                        event.target.interact_at_with_hand(player, &mut stack, position, hand)
                                    }
                                    None => event.target.interact_with_hand(player, &mut stack, hand),
                                }
                            } else {
                                event.target.interact_with_hand(player, &mut stack, hand)
                            };
                            let stand_swap = interacted && event.target.cast_any().is::<crate::entity::decoration::armor_stand::ArmorStandEntity>();
                            if !interacted {
                                server
                                    .item_registry
                                    .use_on_entity(&mut stack, player, event.target);
                            }
                            super::hand_use_result::write_back_hand_item(player, hand, source_slot, &before, &stack,
                                if stand_swap { super::hand_use_result::HandMutation::EquipmentTransfer }
                                else { super::hand_use_result::HandMutation::ItemUse });
                            // Player.interactOn does not award a generic Used statistic;
                            // item implementations own any item-specific awards.
                        }
                    }
                }
            }}
        } else {
            // Entity not found
            send_cancellable_blocking! {{
                server;
                PlayerInteractUnknownEntityEvent::new(player, entity_id.0, action);

                'after: {
                    // ServerGamePacketListenerImpl.handleInteract silently ignores missing targets.
                }
            }}
        }
    }
}
