# Full inventory and applied changes (compact)

Baseline line numbers refer to `origin/murgicraft` at `dec07fc010174173b999473240e6028bf73964b7`; paths are relative to `crates/pumpkin-core/src/`. Each bracketed number enumerates an original call, grouped only where file/function share the same verdict. `remove` includes unconditional break/removal helpers. Existing CAS controls are explicitly listed. Detailed per-function original observations remain in [the complete original inventory](stale-block-write-original-inventory.md).

Gap legend: L = state-read lock release; T = random/scheduled/tick computation; C = plugin callback; A = await; O = additional owner lock release; D = deferred cached apply. Every skipped conditional write must suppress its dependent drops, payment, sound/particles and game events. Every production raw-write exemption was reviewed separately.

| Baseline file | Function [original call lines] | Observed state | Gap | Applied change / verdict |
|---|---|---|---|---|
| `block/block_item.rs` | install_claimed_block [83] existing CAS | Original caller-observed state checked atomically | existing CAS | atomic expected-state batch |
| `block/blocks/amethyst.rs` | random_tick [108] | `relative_state_id` at `grow_pos` (air/source-water or previous bud stage) | L, T | expected-state CAS |
| `block/blocks/bed.rs` | placed [111]; set_occupied [316,332] | Placed foot `args.state_id`; head target admitted replaceable by earlier placement check; Properties retained from `state_id` | L | atomic expected-state batch |
| `block/blocks/beehive.rs` | finish_harvest [122] | Properties retained from `state`; reads `get_block_and_state_id(&position)` | L | expected-state CAS |
| `block/blocks/brushable_block.rs` | brush [55,66] | Properties retained from `state.id`; reads `get_block_state(pos)` | L, O | expected-state CAS |
| `block/blocks/bubble_column.rs` | update_column [115] existing CAS; update_column [131] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `block/blocks/cake.rs` | consume_if_hungry [59,72]; use_with_item [115]; on_scheduled_tick [145] remove | Properties retained from `state_id`; Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/candle_cakes.rs` | consume_and_drop_candle [79]; on_scheduled_tick [116] remove | Previously selected candle-cake `block` at `location`, before candle drop; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/candles.rs` | extinguish [113,120]; on_scheduled_tick [81] remove | Properties retained from `state`; reads `get_block_state_id(position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/carpet.rs` | on_scheduled_tick [33] remove; on_scheduled_tick [60] remove; on_scheduled_tick [87] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state break |
| `block/blocks/carved_pumpkin.rs` | spawn_golem [67] | Previously matched pumpkin/body states at every `pattern.blocks` position | L | atomic expected-state batch |
| `block/blocks/cauldron.rs` | use_with_item [96,141,172,201,207,246,252,285,291,330,336] | Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L | expected-state CAS |
| `block/blocks/chests.rs` | placed_chest_impl [114]; broken_chest_impl [316] | Properties retained from `args.state_id`; Properties retained from `args.state.id` | L | placement primary/partner batch; cleanup CAS |
| `block/blocks/composter.rs` | update_level_composter [120] | Properties retained from `state_id` | L | expected-state CAS |
| `block/blocks/coral/coral_block.rs` | on_scheduled_tick [35] | Caller-observed state/identity at the source and target of `on_scheduled_tick` | L, T | expected-state CAS |
| `block/blocks/coral/coral_fan.rs` | on_scheduled_tick [127] | Properties retained from `current_state.id`; reads `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/coral/coral_plant.rs` | on_scheduled_tick [47] | Properties retained from `current_state.id`; reads `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/creaking_heart.rs` | placed [62]; on_neighbor_update [99] | Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L | expected-state CAS |
| `block/blocks/decorated_pot.rs` | player_will_destroy [136] | Properties retained from `args.state.id` | L | expected-state CAS |
| `block/blocks/dirt_path.rs` | on_scheduled_tick [26] | Earlier `get_block_state_id(args.position)` | L, T | expected-state CAS |
| `block/blocks/doors.rs` | toggle_door [59,64]; set_open [198,207]; placed [241]; on_neighbor_update [332,337]; broken [287] remove; on_state_replaced [386] remove | Retained `block_state`, `other_state_id`, `args.state_id`, `block_state.id` | L | atomic expected-state batch; expected-state break |
| `block/blocks/dragon_egg.rs` | teleport [61,66] | Earlier `get_block_state_id(pos)`, `get_block_state(&target)` | L, O | atomic expected-state batch |
| `block/blocks/dried_ghast.rs` | place_liquid [39]; tick_waterlogged [70]; spawn_ghastling [80]; on_scheduled_tick [174] | Properties retained from `state`; Caller-observed state/identity at the source and target of `tick_waterlogged`; Caller-observed state/identity at the source and target of `spawn_ghastling`; Earlier `get_block_state_id(args.position)` | L, T | expected-state CAS |
| `block/blocks/dripstone.rs` | modify_dripstone_thickness_to [517]; on_scheduled_tick [72] remove | Properties retained from `support_block_state_id`; reads `get_block_and_state_id(pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/farmland.rs` | on_scheduled_tick [32]; random_tick [76]; change_moisture [116] | Caller-observed state/identity at the source and target of `on_scheduled_tick`; Earlier `get_block_state_id(args.position)`; Caller-observed state/identity at the source and target of `change_moisture` | L, T, C | expected-state CAS |
| `block/blocks/fence_gates.rs` | toggle_fence_gate [69]; on_neighbor_update [130] | Properties retained from `state`; reads `get_block_and_state_id(block_pos)`; Properties retained from `block_state.id`; reads `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/fire/fire.rs` | on_scheduled_tick [342,371,382,390,400]; try_spreading_fire [235] existing CAS; try_spreading_fire [239] existing CAS; on_scheduled_tick [498] existing CAS | Properties retained from `block_state.id`; reads `get_block_state(pos)` | L, T | expected-state CAS |
| `block/blocks/flower_pots.rs` | use_with_item [23]; normal_use [51]; random_tick [77] | Earlier empty-pot `args.block` at `args.position`; Earlier potted-flower `args.block` at `args.position`; Earlier potted eyeblossom `args.block` at `args.position` | L, T | expected-state CAS |
| `block/blocks/grass_block.rs` | perform_bonemeal [131]; place_tall_grass [143]; place_tall_grass_upper [154] | Properties retained from `target_state.id`, `state.id`; reads `get_block_state(&target)`, `get_block_state(&above)`; Earlier replaceable lower and air upper target states; `lower_state` plus earlier air upper target | L | atomic expected-state batch |
| `block/blocks/growing_plant.rs` | tick [121] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state break |
| `block/blocks/hopper.rs` | check_powered_state [137] | Properties retained from `state_id` | L | expected-state CAS |
| `block/blocks/ice.rs` | melt [20,22]; slightly_melt [49]; broken [72,87] | Caller-observed ordinary/frosted ice state at `position`; Caller-observed frosted-ice age at `pos`; Broken ice snapshot; live position is post-removal fluid/air | L | expected-state CAS |
| `block/blocks/lanterns.rs` | on_scheduled_tick [81] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state break |
| `block/blocks/leaves.rs` | on_scheduled_tick [97]; random_tick [130]; perform_bonemeal [145] | Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Mangrove-leaf source plus previously checked air at `args.position.down()` | L, T, C | atomic expected-state batch; expected-state CAS |
| `block/blocks/lectern.rs` | set_has_book [174]; on_scheduled_tick [278] | Properties retained from `state_id`; reads `get_block_and_state_id(position)`; Earlier `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/light.rs` | normal_use [39] | Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L | expected-state CAS |
| `block/blocks/netherrack.rs` | perform_bonemeal [74] | Earlier `get_block(&block_pos)` | L | expected-state CAS |
| `block/blocks/note.rs` | on_neighbor_update [83]; normal_use [95] | Properties retained from `block_state.id`; reads `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/nylium.rs` | random_tick [46]; place_crimson_vegetation [129]; place_warped_vegetation [182]; place_nether_sprouts [225]; place_twisting_vines [263,270] | Earlier `get_block_state(args.position)`; Earlier `get_block_state(&target_pos)`, `get_block(&below_pos)`; Earlier `get_block_state(&current_pos)` | L, T | atomic expected-state batch; expected-state CAS |
| `block/blocks/piston/piston.rs` | handle_synced_block_event [149,176,222,271,282]; move_piston [377,423,453,474]; broken [102,104] remove; move_piston [406] remove | Properties retained from `state.id`; reads `get_block_state(pos)`, `get_block_and_state(&pull_pos)`; Earlier `get_block_state(&block_pos)`, `get_block_state(&broken_block_pos)`, `get_block_state(&moved_block_pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, C | atomic movement batch in `piston/movement.rs`; conditional paired break |
| `block/blocks/piston/piston_extension.rs` | broken [26] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/piston/piston_head.rs` | broken [27] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/plant/bamboo.rs` | on_scheduled_tick [77]; update_leaves_and_grow [161,166,180]; on_scheduled_tick [75] remove | Retained `on_scheduled_tick`, `state_id`, `state_id_below`, `state_id_two_below` | L, T | atomic expected-state batch; expected-state CAS; expected-state break |
| `block/blocks/plant/bamboo_sapling.rs` | on_neighbor_update [42]; grow_bamboo [78] | Caller-observed state/identity at the source and target of `on_neighbor_update`; Caller-observed state/identity at the source and target of `grow_bamboo` | L | atomic expected-state batch; expected-state CAS |
| `block/blocks/plant/big_dripleaf.rs` | placed [128]; set_tilt [178] | Properties retained from `support_state_id`; reads `get_block_and_state_id(&support_pos)`; Properties retained from `state_id` | L | expected-state CAS |
| `block/blocks/plant/big_dripleaf_stem.rs` | handle_big_dripleaf_breaking [72] | Properties retained from `support_state_id`; reads `get_block_and_state_id(&support_pos)` | L | expected-state CAS |
| `block/blocks/plant/cactus.rs` | random_tick [44,51,58,66]; on_scheduled_tick [23] remove | Properties retained from `state.id`; reads `get_block_state(&block_up)`, `get_block_state(args.position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | atomic expected-state batch; expected-state CAS; expected-state break |
| `block/blocks/plant/cave_vines.rs` | normal_use [64,77]; perform_bonemeal [102,110] | Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Properties retained from `args.state_id` | L | expected-state CAS |
| `block/blocks/plant/chorus_flower.rs` | random_tick [108,146]; place_grown_flower [165]; place_dead_flower [176]; on_scheduled_tick [57] remove | Retained `state_id`, `place_grown_flower`, `place_dead_flower` | L, T | atomic expected-state batch; expected-state CAS; expected-state break |
| `block/blocks/plant/chorus_plant.rs` | on_scheduled_tick [66] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state break |
| `block/blocks/plant/cocoa.rs` | random_tick [92]; perform_bonemeal [115] | Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Properties retained from `args.state_id` | L, T | expected-state CAS |
| `block/blocks/plant/crop/gourds/stem.rs` | random_tick_with_rng [44,62,67] | Properties retained from `state`; reads `get_block_and_state_id(args.position)`, `get_block_state(&plant_block_pos)` | L, T | atomic expected-state batch; expected-state CAS |
| `block/blocks/plant/crop/mod.rs` | perform_bonemeal [105]; random_tick_with_rng [147,149] | Earlier `get_block_and_state_id(pos)` | L, T, C | expected-state CAS |
| `block/blocks/plant/crop/nether_wart.rs` | random_tick [77] | Earlier `get_block_and_state_id(pos)` | L, T | expected-state CAS |
| `block/blocks/plant/crop/pitcher_crop.rs` | grow_bonemeal [82,90]; random_tick [165,173] | Caller-observed state/identity at the source and target of `grow_bonemeal`; Properties retained from `state_id`, `above_state.id`; reads `get_block_state_id(args.position)`, `get_block_state(&above_pos)` | L, T | atomic expected-state batch |
| `block/blocks/plant/crop/sweet_berry_bush.rs` | normal_use [61]; random_tick [183] | Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Earlier `get_block_and_state_id(pos)` | L, C, T | expected-state CAS |
| `block/blocks/plant/dry_vegetation.rs` | perform_bonemeal [39,53] | Caller-observed state/identity at the source and target of `perform_bonemeal` | L | atomic expected-state batch; expected-state CAS |
| `block/blocks/plant/eyeblossom.rs` | try_changing_state [135]; on_scheduled_tick [60] remove | Caller-observed state/identity at the source and target of `try_changing_state`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/plant/lily_pad.rs` | on_entity_collision [27] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/plant/mangrove_propagule.rs` | advance_tree [67]; random_tick [125]; perform_bonemeal [152] | Caller-observed state/identity at the source and target of `advance_tree`; Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Properties retained from `args.state_id` | L, T | expected-state CAS |
| `block/blocks/plant/mushroom_plant.rs` | grow_mushroom [118]; place_mushroom_block [134]; random_tick [288] | Earlier `get_block_state(&check_pos)`, `get_block(&check_pos)`; Earlier `get_block(pos)`, `get_block_state(pos)`; Earlier `get_block_state_id(&pos)`, `get_block(&check_pos)`, `get_block_state(&offset)` | L, C, T | expected-state CAS |
| `block/blocks/plant/sapling.rs` | advance_tree [52] | Properties retained from `state_id` | L | expected-state CAS |
| `block/blocks/plant/sea_pickles.rs` | perform_bonemeal [56,75] | Properties retained from `args.state_id`; reads `get_block(&target)` | L | atomic expected-state batch |
| `block/blocks/plant/shelf_mushroom.rs` | perform_bonemeal [102] | Properties retained from `args.state_id` | L | expected-state CAS |
| `block/blocks/plant/short_plant.rs` | perform_bonemeal [35,38] | Clicked short-plant source and earlier replaceable upper target | L | atomic expected-state batch |
| `block/blocks/plant/small_dripleaf.rs` | placed [63] | Properties retained from `args.state_id` | L | atomic expected-state batch |
| `block/blocks/plant/sugar_cane.rs` | random_tick [35,42,49]; on_scheduled_tick [23] remove | Properties retained from `state_id`; reads `get_block_state(args.position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | atomic expected-state batch; expected-state CAS; expected-state break |
| `block/blocks/plant/tall_plant.rs` | placed [82]; broken [107] remove | Placed lower `args.state_id` and upper replaceability observed during placement; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | atomic expected-state batch; expected-state break |
| `block/blocks/plant/tree_grower.rs` | remove_saplings [189]; reset_saplings [199] | Caller-observed state/identity at the source and target of `remove_saplings`; Caller-observed state/identity at the source and target of `reset_saplings` | L | atomic expected-state batch |
| `block/blocks/player_destroy.rs` | remove_partner [18] | Earlier `get_block_state_id(&pos)` | L | expected-state CAS |
| `block/blocks/powder_snow.rs` | on_entity_collision [109] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/pumpkin.rs` | use_with_item [29] | Earlier uncarved `args.block` at `args.position` | L | expected-state CAS |
| `block/blocks/redstone/abstract_redstone_gate.rs` | on_neighbor_update [87] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/redstone/bell.rs` | on_neighbor_update [206] | Properties retained from `state.id`; reads `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/redstone/buttons.rs` | click_button [60]; on_scheduled_tick [104] | Properties retained from `state`; reads `get_block_and_state_id(block_pos)`; Properties retained from `state.id`; reads `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/comparator.rs` | normal_use [63]; update [350,353] | Properties retained from `state.id`; reads `get_block_state(args.position)`; Properties retained from `state.id` | L | expected-state CAS |
| `block/blocks/redstone/copper_bulb.rs` | on_neighbor_update [94] | Properties retained from `state.id`; reads `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/redstone/crafter.rs` | dispense_from [169]; on_neighbor_update [361,374] | Properties retained from `state.id`; reads `get_block_state(pos)`; Earlier `get_block_state(args.position)` | L, O | expected-state CAS |
| `block/blocks/redstone/daylight_detector.rs` | normal_use [46]; update_signal_strength [104] | Properties retained from `state.id`; reads `get_block_state(args.position)`; Properties retained from `state.id`; reads `get_block_and_state(block_pos)` | L, O | expected-state CAS |
| `block/blocks/redstone/dispenser.rs` | on_neighbor_update [236,243]; dispense_flint_and_steel [852]; shear_beehive [969]; dispense_glass_bottle [1050]; dispense_water_bottle [1091]; dispense_glowstone [1115]; dispense_mob_head [1168]; dispense_shulker_box [1196] | Earlier `get_block_state(args.position)`; Earlier `get_block(&front)`; Properties retained from `state_id`; reads `get_block_and_state_id(&target)`; Earlier `get_block(&target)`; Earlier `get_block_state(&target)` | L | expected-state CAS |
| `block/blocks/redstone/dropper.rs` | on_neighbor_update [138,145] | Earlier `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/redstone/lever.rs` | toggle_lever [30] | Properties retained from `state`; reads `get_block_and_state_id(block_pos)` | L | expected-state CAS |
| `block/blocks/redstone/lightning_rod.rs` | trigger [24]; on_scheduled_tick [72] | Properties retained from `state_id`; reads `get_block_and_state_id(pos)`; Properties retained from `state.id`; reads `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/observer.rs` | placed [36]; on_scheduled_tick [51,58] | Properties retained from `args.state_id`; Properties retained from `state.id`; reads `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/pressure_plate/mod.rs` | update_plate_state [97] | Caller-observed state/identity at the source and target of `update_plate_state` | L, C | expected-state CAS |
| `block/blocks/redstone/pressure_plate/plate.rs` | on_neighbor_update [69] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/redstone/pressure_plate/weighted.rs` | on_neighbor_update [77] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/redstone/rails/activator_rail.rs` | update_powered_state_internal [327]; on_neighbor_update [99] remove | Earlier `get_block_state_id(pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS; expected-state break |
| `block/blocks/redstone/rails/common.rs` | update_flanking_rails_shape [129] | Caller-observed state/identity at the source and target of `update_flanking_rails_shape` | L | expected-state CAS |
| `block/blocks/redstone/rails/detector_rail.rs` | check_pressed [85,92]; on_neighbor_update [127] remove | Properties retained from `state_id`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS; expected-state break |
| `block/blocks/redstone/rails/powered_rail.rs` | update_powered_state_internal [328]; on_neighbor_update [99] remove | Earlier `get_block_state_id(pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS; expected-state break |
| `block/blocks/redstone/rails/rail.rs` | on_neighbor_update [101] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state break |
| `block/blocks/redstone/redstone_lamp.rs` | on_neighbor_update [39]; on_scheduled_tick [59] | Properties retained from `state.id`; reads `get_block_state(args.position)`; Properties retained from `state.id`; reads `get_block_state(args.position)`, `get_block(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/redstone_ore.rs` | light_up [25]; random_tick [53] | Properties retained from `state.id`; Properties retained from `state.id`; reads `get_block_state(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/redstone_torch.rs` | set_lit [307] | Properties retained from `state_id` | L | expected-state CAS |
| `block/blocks/redstone/redstone_wire.rs` | normal_use [158]; update_power_strength [299]; on_neighbor_update [192] remove | Properties retained from `state.id`; reads `get_block_state(args.position)`; Properties retained from `state_id`; reads `get_block_state_id(pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS; expected-state break |
| `block/blocks/redstone/repeater.rs` | on_scheduled_tick [57,71]; on_use [224] | Properties retained from `state.id`; reads `get_block_and_state(args.position)`; Caller-observed state/identity at the source and target of `on_use` | L, T | expected-state CAS |
| `block/blocks/redstone/sculk_sensor.rs` | trigger [62,96]; on_scheduled_tick [187,202,216,231] | Properties retained from `state.id`, `back_state.id`; reads `get_block_state(pos)`, `get_block_state(&back_pos)`; Properties retained from `state.id`; reads `get_block_state(args.position)` | L, O, T | expected-state CAS |
| `block/blocks/redstone/target_block.rs` | on_projectile_hit [68]; on_scheduled_tick [88] | Properties retained from `args.state.id`; Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/tripwire.rs` | on_entity_collision [35]; broken [79]; on_scheduled_tick [123] | Properties retained from `args.state.id`; Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L, T | expected-state CAS |
| `block/blocks/redstone/tripwire_hook.rs` | update [215,248,269] | Properties retained from `start_hook_state_id`, `state_id`; reads `get_block(&current_pos)`, `get_block_state_id(&current_pos)` | L | expected-state CAS |
| `block/blocks/respawn_anchor.rs` | use_with_item [80]; normal_use [111] remove | Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS; expected-state break |
| `block/blocks/rooted_dirt.rs` | perform_bonemeal [20] | Rooted dirt source and previously checked air below | L | atomic expected-state batch |
| `block/blocks/sculk/sculk_shrieker.rs` | try_shriek [107]; on_scheduled_tick [203] | Properties retained from `state.id`; reads `get_block(pos)`, `get_block_state(pos)`; Properties retained from `state.id`; reads `get_block_state(args.position)` | L, O, T | expected-state CAS |
| `block/blocks/sculk/sculk_vein.rs` | perform_bonemeal [139] | Properties retained from `args.state_id` | L | expected-state CAS |
| `block/blocks/skull_block.rs` | on_neighbor_update [51] | Properties retained from `state.id`; reads `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/sniffer_egg.rs` | on_scheduled_tick [59]; on_scheduled_tick [77] remove | Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/snow.rs` | on_scheduled_tick [50] remove; random_tick [59] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state break |
| `block/blocks/sponge.rs` | placed [246]; absorb_nodes [88] existing CAS; absorb_node [113] existing CAS; absorb_node [121] existing CAS; drop_and_remove_plant [142] existing CAS | Placed dry-sponge state before absorption traversal/callbacks | L | expected-state CAS |
| `block/blocks/spreading_snowy_block.rs` | random_tick [130,153] | Properties retained from `default_block_state.id`; reads `get_block(&test_pos)`, `get_block_state(&above_test)` | L, T | expected-state CAS |
| `block/blocks/stairs.rs` | on_neighbor_update [61] | Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L | expected-state CAS |
| `block/blocks/straw_bed.rs` | placed [109] | Placed foot `args.state_id`; head target admitted replaceable by earlier placement check | L | atomic expected-state batch |
| `block/blocks/tnt.rs` | prime_with_owner [51] existing CAS; prime_with_owner [66] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `block/blocks/trapdoor.rs` | toggle_trapdoor [33]; on_neighbor_update [135] | Properties retained from `block_state`; reads `get_block_and_state_id(block_pos)`; Properties retained from `block_state.id`; reads `get_block_state(args.position)` | L | expected-state CAS |
| `block/blocks/turtle_egg.rs` | decrease_eggs [76]; random_tick [196]; decrease_eggs [72] remove; on_scheduled_tick [150] remove; random_tick [209] remove | Properties retained from `state_id`; Properties retained from `state_id`; reads `get_block_state_id(args.position)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `block/blocks/vault.rs` | use_with_item [83,107] | Properties retained from `state_id`; reads `get_block_state_id(args.position)` | L | expected-state CAS |
| `block/blocks/vine.rs` | use_with_item [365]; random_tick [424,434,454,469,483,493,506,536,568] | Properties retained from `state.id`; reads `get_block_state(args.position)`; Properties retained from `state_id`, `below_state.id`; reads `get_block(&above_pos)`, `get_block_and_state(&rel_pos)`, `get_block_and_state(&below_pos)` | L, T | expected-state CAS |
| `block/blocks/weathering_copper.rs` | change_over_time [541,552,569] | Properties retained from `current_state_id`; reads `get_block_state_id(position)`, `get_block_and_state_id(&top_pos)`, `get_block_and_state_id(&right_pos)` | L | atomic expected-state batch |
| `block/blocks/wither_skull.rs` | spawn_wither [65] | Previously matched skull/soul-sand states at every structure position | L | atomic expected-state batch |
| `block/entities/barrel.rs` | set_open [160] | Properties retained from `state.id`; reads `get_block_state(&self.position)` | L | expected-state CAS |
| `block/entities/brewing_stand.rs` | tick [615] | Properties retained from `state.id`; reads `get_block_and_state(&self.position)` | L, T, O | expected-state CAS |
| `block/entities/chiseled_bookshelf.rs` | update_state [147] | Caller-observed state/identity at the source and target of `update_state` | L | expected-state CAS |
| `block/entities/crafter.rs` | tick [106] | Properties retained from `state.id`; reads `get_block_state(&self.position)` | L, T | expected-state CAS |
| `block/entities/end_gateway.rs` | spawn_end_island [411]; spawn_gateway_portal [437,451,457,463] | Structure recipe; target state does not determine result | L | explicit structure recipe retained |
| `block/entities/furnace_like_block_entity.rs` | tick [619] | Properties retained from `furnace_block_state.id`; reads `get_block_and_state(&self.position)` | L, T, C | expected-state CAS |
| `block/entities/jukebox.rs` | notify_item_changed_in_jukebox [265] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `block/entities/piston.rs` | finish [204]; tick [234,242] | Earlier `get_block(&pos)` | L, T | expected-state CAS |
| `block/entities/trial_spawner.rs` | set_state [940]; set_ominous [956] | Properties retained from `block_id`; reads `get_block_state_id(&spawner_pos)`, `get_block(&spawner_pos)` | L | expected-state CAS |
| `block/fluid/flowing_trait.rs` | on_scheduled_tick_internal [100,111]; apply_spread [319,355,364]; apply_spread [339] remove | Properties retained from `current_block_state_id`; reads `get_block_state_id(block_pos)`; Earlier `get_block_state_id(pos)`, `get_block(pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T, C | expected-state CAS; expected-state break |
| `block/fluid/lava.rs` | ignite_fire_if_possible [87]; receive_neighbor_fluids [116,132]; random_tick [211]; spread_to [278] | Caller-observed state/identity at the source and target of `ignite_fire_if_possible`; Earlier `get_block_state_id(block_pos)`, `get_block(&neighbor_pos)`; Earlier `get_block_and_state_id(&test_pos)`; Properties retained from `state_id, fluid`, `current_state_id`; reads `get_block_state_id(pos)` | L, C, T | expected-state CAS |
| `block/registry.rs` | post_process_state [1422] | Earlier `get_block_state_id(position)`, `get_block_state_id(&neighbor_pos)` | L | expected-state CAS |
| `command/commands/block_input.rs` | place [31] | Command-supplied state; target is not its input | L | keep/filter/destroy retain expected target through shape callbacks and NBT; strict/replace preserve explicit intent |
| `command/commands/clone.rs` | execute [225,257] | Earlier source state/NBT plus target plan | L | atomic expected-state batch |
| `command/commands/fill.rs` | fill_blocks [139] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | keep/filter/destroy retain target snapshot; conditional placement/removal |
| `command/commands/place.rs` | execute [447,449]; execute [578,580]; apply_delta [675] | Private protochunk construction; Earlier `get_block_state(&pos)` | L, O | private proto construction; live adapter enforces observations |
| `command/commands/setblock.rs` | execute [54] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | keep/filter/destroy retain target snapshot; conditional placement/removal |
| `enchantment/effects/replace_disk.rs` | apply [149] | Per-position predicate-accepted state at replacement disk target | L | expected-state CAS |
| `entity/ai/goal/break_door.rs` | tick [162] | Earlier `get_block_and_state_id(&self.door_interact_goal.door_pos)` | L, T, C | atomic expected-state batch |
| `entity/ai/goal/eat_grass.rs` | tick [106]; tick [93] remove | Earlier `get_block(&block_pos)`, `get_block(&below_pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T | expected-state CAS; expected-state break |
| `entity/ai/goal/pick_up_block.rs` | tick [93] | Earlier `get_block(&target_pos)`, `get_block_state(block_pos)` | L, T, C | expected-state CAS |
| `entity/ai/goal/place_block.rs` | tick [84] | Earlier `get_block_state_id(&target_pos)`, `get_block_and_state(&below_pos)` | L, T | expected-state CAS |
| `entity/ai/goal/step_and_destroy_block.rs` | tick [155] | Goal-selected target block state before delayed destruction | L, T | expected-state CAS |
| `entity/boss/ender_dragon.rs` | tick_block_breaking [660] | Earlier `get_block(&pos)` | L, T, O | expected-state CAS |
| `entity/boss/wither.rs` | tick_wither [494] | Earlier `get_block(&bpos)` | L, T | expected-state CAS |
| `entity/effect/weaving.rs` | on_mob_death [51] | Earlier `get_block_state(&target_pos)`, `get_block_state(&below_pos)` | L | expected-state CAS |
| `entity/falling.rs` | replace_spawn [66]; land [253] | Caller-observed state/identity at the source and target of `replace_spawn`; Properties retained from `state_id`; reads `get_block_and_state(&landing_pos)` | L, C | expected-state CAS |
| `entity/lightning.rs` | spawn_fire [107]; clear_copper_on_lightning_strike [141]; random_step_cleaning_copper [185] | Earlier `get_block_state(&p)`; Earlier `get_block_state(&strike_pos)`; Earlier `get_block_state(&candidate)` | L | expected-state CAS |
| `entity/passive/rabbit_goals.rs` | tick [337] | Properties retained from `state`; reads `get_block_and_state_id(&crops)` | L, T, C, O | expected-state CAS |
| `entity/player.rs` | tick [2183] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, T, A, O | expected-state break |
| `entity/player/container_menu_block_effects.rs` | on_take_anvil [58,61] | Properties retained from `state.id`; reads `get_block_state(&pos)` | L | expected-state CAS |
| `entity/player/respawn_position.rs` | resolve_respawn_position [158] | Properties retained from `state_id`; reads `get_block_and_state_id(pos)`, `get_block_state(pos)` | L | expected-state CAS |
| `entity/player/sleeping.rs` | clear_occupied [191] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `entity/projectile/potion_water.rs` | douse_fire [113]; douse_fire [86] remove | Properties retained from `state.id`; reads `get_block_state(pos)`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS; expected-state break |
| `entity/projectile/small_fireball.rs` | on_hit [133] | Earlier `get_block_state(&block_to_place)` | L | expected-state CAS |
| `item/items/axe.rs` | transform_companions [57] existing CAS; transform_companions [103] existing CAS; use_on_block_with_hand [178] existing CAS | Original caller-observed state checked atomically | existing CAS | atomic expected-state batch |
| `item/items/brush.rs` | use_on_block [133,151] | Properties retained from `current_state_id`; reads `get_block_state_id(&location)` | L, C | expected-state CAS |
| `item/items/bucket.rs` | try_pickup_fluid_at [229,236]; empty_bucket_at_with_player [393,413]; use_observed_stack [647]; try_pickup_fluid_at [219,235] remove; empty_bucket_at_with_player [404] remove | Earlier `get_block_and_state_id(&block_pos)`; Earlier `get_block_and_state(&pos)`; Caller-observed state/identity at the source and target of `use_observed_stack`; Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, C | expected-state CAS; expected-state break |
| `item/items/debug_stick.rs` | handle_interaction [112] | Caller-observed state/identity at the source and target of `handle_interaction` | L, O | expected-state CAS |
| `item/items/ender_eye.rs` | use_on_block [63] | Properties retained from `state_id`; reads `get_block_state_id(&location)` | L | expected-state CAS |
| `item/items/hoe.rs` | use_on_block_with_hand [75] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `item/items/honeycomb.rs` | try_wax_block [70] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `item/items/ignite/fire_charge.rs` | use_on_block [63] | Ignition result computed from clicked/adjacent block observations before events | L, C | conditional shared boundary |
| `item/items/ignite/flint_and_steel.rs` | use_on_block [62] | Ignition result computed from clicked/adjacent block observations before events | L, C | conditional shared boundary |
| `item/items/place_on_water.rs` | use_observed_stack [78] | Properties retained from `state_id`; reads `get_block_state_id(pos)`, `get_block_state(&above_pos)` | L | expected-state CAS |
| `item/items/shears.rs` | handle_growing_plant [91]; handle_pumpkin [162] | Properties retained from `state_id`; Caller-observed state/identity at the source and target of `handle_pumpkin` | L | expected-state CAS |
| `item/items/shovel.rs` | use_on_block_with_hand [80] existing CAS; use_on_block_with_hand [109] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `net/bedrock/play/player_action.rs` | handle_player_action [52,84,187] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, O | expected-state break |
| `net/java/play/player_action.rs` | handle_player_action [53,85,187] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, C, O | expected-state break |
| `net/java/play/set_command_block.rs` | handle_set_command_block [158] existing CAS | Original caller-observed state checked atomically | existing CAS | expected-state CAS |
| `world/block_placer.rs` | set_block_state [82] remove | Structure/generation observed target; direct Level write | L | conditional generation commit; exact old entity detach/retirement |
| `world/dragon_fight.rs` | regenerate_spike [779,789,824]; spawn_new_gateway_at [926,932,938,944] | Structure recipe; target state does not determine result | L | explicit structure recipe retained |
| `world/dragon_fight/fight_actions.rs` | execute [121]; clear_portal [219] | Structure recipe; target state does not determine result; Earlier `b = world.get_block(&p)`, selected only BEDROCK/END_PORTAL for replacement | L | conditional selected removal; explicit egg recipe retained |
| `world/dragon_fight/respawn_actions.rs` | execute [36] | Earlier `block = world.get_block(&pos)`, selected non-BEDROCK/non-OBSIDIAN/non-AIR | L | expected-state CAS |
| `world/end_podium.rs` | set_block [33]; place [100] | Geometry recipe normally; `drop_previous_and_set_block` observes target block before calling shared setter; Structure recipe; target state does not determine result | L | expected-state CAS; explicit structure recipe retained |
| `world/explosion/block_triggers.rs` | trigger_block [58,80] | Properties retained from `state.id` | L | expected-state CAS |
| `world/explosion/blocks.rs` | interact_with_blocks [138]; create_fire [195] | Earlier `get_block_state(&pos)`; Earlier `get_block_state(pos)` | L, C | conditional removal/fire; retained loot/XP, physical outputs after admission |
| `world/generation_cache.rs` | apply [75] | Feature plan built from cached live chunk states | L, D | cached expected states; conditional apply/NBT |
| `world/mod.rs` | break_block_with_limit [5137] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L, C | conditional removal; pre-removal loot/XP events, stable retired contents |
| `world/neighbor_updater.rs` | execute_shape_update [331] remove | Caller eligibility state; break helper re-reads then runs BlockBreakEvent/drop callbacks | L | expected-state CAS |
| `world/portal/end.rs` | create_portal [102]; create_portal [97] remove | Portal frame geometry plus destination states | L | expected-state CAS; expected-state break |
| `world/portal/frame_effects.rs` | set_frame_block [172] | Explicit frame construction writes from structure recipe | L | explicit recipe; atomic exact old entity detach, deferred retirement/effects |
| `world/portal/mod.rs` | get_portal_destination [142] | End platform recipe computes OBSIDIAN/AIR solely from geometry; no target-state input | await before recipe; no retained target observation | atomic recipe batch; permits released before callbacks |
| `world/portal/nether.rs` | create [276] | Frame geometry plus empty interior observation | L | atomic expected-state batch |

Additional shared boundaries: `World::set_block_state` (`world/mod.rs:4784,4802`) remains an explicit replacement primitive; `set_block_state_if` (`4816,4833`) is the single-cell CAS. New observed-state helpers require the original expected state; paired helper commits all cells before callbacks. The depth-aware write and break seam (`world/neighbor_updater.rs:264,279,331`; `world/mod.rs:5045,5137`) must pass expected states. Existing conditional state writes in `world/cauldron.rs:26`, `lectern_update.rs:66`, `pending_block_entity.rs:753`, `straw_bed.rs:58`, and paired bed removal in `bed_break.rs:24` remain part of the boundary audit.

| Other caller group | Original sites | Observed state / gap | Verdict |
|---|---|---|---|
| Wasm world v0.1/v0.2 | `crates/pumpkin-wasm-host-v0_{1,2}/src/world.rs:169,1413,1426,1446` | Caller supplies explicit requested state; host performs no read-derived transition | Explicit replacement APIs retained; native read-derived callers use conditional API |
| Wasm proto generation v0.1/v0.2 | same files `1952,2001,2031,2063` | Exclusive proto data | No player-shared race |
| Entity block displays v0.1/v0.2 | `src/display.rs:491` | Display metadata only | No terrain mutation |
| World-generation and structure implementations | `crates/pumpkin-world/src/generation/**` and `chunk_system/generation_cache.rs:315,327` (all detailed original calls in long inventory) | Private proto generation, or a live-world adapter | Private generation cannot race players; live generation adapters retain expected target states |
| Low-level storage primitives | `pumpkin-world/src/level.rs:877,885`, `chunk/mod.rs:435,447,492,499,507,730,743,778,921,931,940,948` | Setter arguments are explicit; expected-state decision belongs to caller | Keep raw replacement API; observed-state runtime callers use atomic conditional chunk seam |
| Fixtures and game tests | Test/scenario modules and `pumpkin-gametest` | Deliberate setup data | Exempt from gameplay-transition conversion |

Additional delayed block-entity write inventory (same baseline). These are conditional terrain-and-registry admission, including native serialization/callback gaps; original explicit World/Wasm entity replacement APIs remain available.

| Baseline file:line | Observed terrain | Gap | Applied change / verdict |
|---|---|---|---|
| `block/blocks/banners.rs:19` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/barrel.rs:96` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/bed.rs:104,118` | Original foot state and computed installed head state | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/blast_furnace.rs:134` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/brewing_stand.rs:84` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/brushable_block.rs:88` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/campfire.rs:30` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/chests.rs:95` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/chiseled_bookshelf.rs:104` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/command.rs:348` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/conduit.rs:22` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/creaking_heart.rs:55` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/decorated_pot.rs:38` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/enchanting_table.rs:30` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/end_gateway.rs:60` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/end_portal.rs:31,34` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection; remove redundant pending-NBT write |
| `block/blocks/ender_chest.rs:108,126` | Original placement state; initial typed state for lazy recovery | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/furnace.rs:132` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/hopper.rs:103` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/jigsaw.rs:133` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/jukebox.rs:71` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/lectern.rs:182` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/piston/piston.rs:233,430,461` | Planned installed moving-piston state | L, C | conditional movement entity publication after atomic terrain admission |
| `block/blocks/redstone/bell.rs:139` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/redstone/comparator.rs:87` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/redstone/crafter.rs:336` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/redstone/daylight_detector.rs:23` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/redstone/dispenser.rs:222` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/redstone/dropper.rs:124` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/redstone/sculk_sensor.rs:119,122` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/shelf.rs:29` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/shulker_box.rs:95` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/signs.rs:343,346` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/skull_block.rs:33` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/smoker.rs:131` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/spawner.rs:21` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/straw_bed.rs:102,116` | Original foot state and computed installed head state | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/trial_spawner.rs:14` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/vault.rs:33` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/blocks/wither_skull.rs:93` | Original `PlacedArgs.state_id` | L, C | conditional typed initialization; skip dependent effects on rejection |
| `block/entities/end_gateway.rs:449` | Recipe-installed END_GATEWAY state | L, C | conditional return-gateway entity publication |
| `block/entities/sign.rs:561` | Original placed sign state, retained before item-component callbacks | L, C | conditional sign NBT publication |
| `block/registry.rs:1253` | Original placed state / typed fallback state | L, C | conditional fallback initialization and stale-dispatch rejection |
| `command/commands/block_input.rs:38` | Resolved installed command state | L, C | conditional initialization/NBT; strict terrain replacement intent retained |
| `command/commands/clone.rs:234` | Admitted clone destination state | L, C | conditional cloned NBT publication |
| `command/commands/data.rs:304` | Terrain retained when BlockDataAccessor is constructed | L, C | conditional NBT replacement; close/notify only after admission |
| `net/java/play/set_command_block.rs:32` | Original typed command-block state before native lookup, then installed state | L, C | conditional entity publication; release outer permit before callbacks |
| `world/block_placer.rs:60` | First source state or successfully installed state | L, C | conditional registry publication; retain unchanged-terrain NBT |
| `world/generation_cache.rs:79` | First cached source state or admitted overlay state | L, C | conditional registry publication; retain unchanged-terrain NBT |

Chest connection retains its original source guard after registration callbacks. Crafter placement and neighbor scheduling each publish one tick only after admission.
