use crate::{
    entity::{EntityBase, death_test_world::DeathTestWorld, player::Player},
    net::java::combat_test_support::TestPlayer,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        entity::{entity_spawn::EntitySpawnEvent, projectile_launch::ProjectileLaunchEvent},
        player::player_item_damage::PlayerItemDamageEvent,
    },
    server::Server,
};
use pumpkin_data::{
    Block,
    biome::Biome,
    data_component_impl::{ChargedProjectilesImpl, CustomDataImpl},
    item::Item,
    item_stack::ItemStack,
    statistic::StatisticCategory,
};
use pumpkin_inventory::{Inventory, inventory::slot_transaction::Transaction};
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_protocol::{
    VarInt,
    codec::item_stack_seralizer::ItemStackSerializer,
    java::server::play::{SPlayerAction, SUseItem, SUseItemOn, Status},
    ser::NetworkReadExt,
};
use pumpkin_util::{
    Hand,
    math::{position::BlockPos, vector3::Vector3},
    permission::PermissionLvl,
};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

pub use std::sync::atomic::Ordering::SeqCst;
pub type Edit = Arc<dyn Fn(&Arc<Player>) + Send + Sync>;
pub type AdmissionEdit = Arc<dyn Fn(&Arc<Player>, char, usize) + Send + Sync>;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Veto {
    None,
    Launch,
    Spawn,
}
#[derive(Default)]
pub struct State {
    pub veto: Option<Veto>,
    pub partial: bool,
    pub deny_index: Option<usize>,
    pub edit: Option<Edit>,
    pub observer: Option<Edit>,
    pub admission_observer: Option<AdmissionEdit>,
    pub damage_edit: Option<Edit>,
    pub break_edit: Option<Edit>,
    pub cast_edit: Option<Edit>,
    pub order: Vec<(char, i32)>,
    pub launches: usize,
    pub spawns: usize,
    pub damage: usize,
    pub damage_amounts: Vec<i32>,
    pub casts: usize,
    pub dispenses: usize,
    pub vibrations: usize,
    pub damage_amount: Option<i32>,
    pub cancel_damage: bool,
    pub owner: Option<Arc<Transaction>>,
    pub observe_any: bool,
    pub check_reservations: bool,
    pub packet_probes: usize,
}
pub struct Hooks {
    pub player: Arc<Player>,
    pub state: Mutex<State>,
}
impl Hooks {
    pub fn install(server: &Arc<Server>, player: &Arc<Player>) -> Arc<Self> {
        let hook = Arc::new(Self {
            player: player.clone(),
            state: Mutex::default(),
        });
        server.plugin_manager.register::<ProjectileLaunchEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server.plugin_manager.register::<EntitySpawnEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server
            .plugin_manager
            .register::<crate::plugin::api::events::block::block_dispense::BlockDispenseEvent, _>(
                hook.clone(),
                EventPriority::Normal,
                true,
            );
        server
            .plugin_manager
            .register::<crate::plugin::api::events::server::packet::PacketSentEvent, _>(
                hook.clone(),
                EventPriority::Normal,
                true,
            );
        server.plugin_manager.register::<PlayerItemDamageEvent, _>(
            hook.clone(),
            EventPriority::Normal,
            true,
        );
        server
            .plugin_manager
            .register::<crate::plugin::api::events::player::fish::PlayerFishEvent, _>(
                hook.clone(),
                EventPriority::Normal,
                true,
            );
        server
            .plugin_manager
            .register::<crate::plugin::api::events::world::generic_game::GenericGameEvent, _>(
                hook.clone(),
                EventPriority::Normal,
                true,
            );
        server.plugin_manager.register::<crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent, _>(hook.clone(), EventPriority::Normal, true);
        hook
    }
    pub fn configure(&self, veto: Veto, partial: bool, edit: Option<Edit>) {
        *self.state.lock().unwrap() = State {
            veto: Some(veto),
            partial,
            edit,
            ..State::default()
        };
    }
    pub fn counts(&self) -> (usize, usize, usize) {
        let s = self.state.lock().unwrap();
        (s.launches, s.spawns, s.damage)
    }
    fn admission(&self, stage: char, id: i32) -> bool {
        let (deny, edit, owner, admission_observer, index) = {
            let mut s = self.state.lock().unwrap();
            if stage == 'S' && !s.order.contains(&('L', id)) {
                return false;
            }
            s.order.push((stage, id));
            let n = if stage == 'L' {
                let n = s.launches;
                s.launches += 1;
                n
            } else {
                let n = s.spawns;
                s.spawns += 1;
                n
            };
            let selected = s.veto
                == Some(if stage == 'L' {
                    Veto::Launch
                } else {
                    Veto::Spawn
                });
            (
                selected
                    && s.deny_index
                        .map_or(!s.partial || n == 0, |index| n == index),
                if (selected || (s.veto == Some(Veto::None) && stage == 'L')) && n == 0 {
                    s.edit.clone()
                } else {
                    None
                },
                s.owner.clone(),
                s.admission_observer.clone(),
                n,
            )
        };
        assert_owner_released(self.player.inventory().slot_transaction(0));
        if let Some(owner) = owner {
            assert_owner_released(owner);
        }
        if self.state.lock().unwrap().check_reservations {
            assert_reservations_released(self.player.world());
        }
        let observer = self.state.lock().unwrap().observer.clone();
        if let Some(observer) = observer {
            observer(&self.player);
        }
        if let Some(observer) = admission_observer {
            observer(&self.player, stage, index);
        }
        if let Some(edit) = edit {
            edit(&self.player);
        }
        deny
    }
}
impl EventHandler<ProjectileLaunchEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut ProjectileLaunchEvent,
    ) -> BoxFuture<'a, ()> {
        if event
            .shooter_id
            .is_none_or(|id| id == self.player.entity_id())
            || self.state.lock().unwrap().observe_any
        {
            event.cancelled |= self.admission('L', event.entity_id);
        }
        Box::pin(async {})
    }
}
impl EventHandler<EntitySpawnEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut EntitySpawnEvent,
    ) -> BoxFuture<'a, ()> {
        event.cancelled |= self.admission('S', event.entity_id);
        Box::pin(async {})
    }
}
impl EventHandler<crate::plugin::api::events::block::block_dispense::BlockDispenseEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        _: &'a mut crate::plugin::api::events::block::block_dispense::BlockDispenseEvent,
    ) -> BoxFuture<'a, ()> {
        let owner = {
            let mut state = self.state.lock().unwrap();
            state.dispenses += 1;
            state.owner.clone()
        };
        if let Some(owner) = owner {
            assert_owner_released(owner);
        }
        Box::pin(async {})
    }
}
impl EventHandler<crate::plugin::api::events::server::packet::PacketSentEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::server::packet::PacketSentEvent,
    ) -> BoxFuture<'a, ()> {
        let probe = {
            let mut state = self.state.lock().unwrap();
            let probe =
                event.player.entity_id() == self.player.entity_id() && state.check_reservations;
            state.packet_probes += usize::from(probe);
            probe
        };
        if probe {
            assert_owner_released(self.player.inventory().slot_transaction(0));
            assert_reservations_released(self.player.world());
        }
        Box::pin(async {})
    }
}
impl EventHandler<PlayerItemDamageEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut PlayerItemDamageEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() != self.player.entity_id() {
            return Box::pin(async {});
        }
        let edit = {
            let mut s = self.state.lock().unwrap();
            s.damage += 1;
            s.damage_amounts.push(event.damage);
            event.cancelled |= s.cancel_damage;
            if let Some(amount) = s.damage_amount {
                event.damage = amount;
            }
            if s.damage == 1 {
                s.damage_edit.clone()
            } else {
                None
            }
        };
        assert_owner_released(self.player.inventory().slot_transaction(0));
        if let Some(edit) = edit {
            edit(&self.player);
        }
        Box::pin(async {})
    }
}
pub fn assert_owner_released(owner: Arc<Transaction>) {
    let (send, recv) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let acquired = owner.try_lock().is_some();
        let _ = send.send(acquired);
    });
    assert!(
        recv.recv_timeout(Duration::from_secs(2)).unwrap(),
        "projectile callback retained transaction ownership"
    );
}
pub fn assert_reservations_released(world: Arc<crate::world::World>) {
    let (send, recv) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let released = crate::world::projectile_test_support::reservations_released(&world);
        let _ = send.send(released);
    });
    assert!(
        recv.recv_timeout(Duration::from_secs(2)).unwrap(),
        "projectile callback or packet retained a UUID reservation"
    );
}
pub async fn run(action: impl FnOnce(&DeathTestWorld) + Send + 'static) {
    let fixture = Arc::new(
        tokio::time::timeout(Duration::from_secs(90), DeathTestWorld::new())
            .await
            .unwrap(),
    );
    let worker = fixture.clone();
    let runtime = tokio::runtime::Handle::current();
    let (send, recv) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _entered = runtime.enter();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(&worker)));
        let _ = send.send(result);
    });
    let result = recv.recv_timeout(Duration::from_secs(90)).unwrap();
    tokio::time::timeout(Duration::from_secs(30), fixture.server.shutdown())
        .await
        .unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
pub fn player(f: &DeathTestWorld) -> TestPlayer {
    let world = f.world();
    crate::world::spawn_test_support::publish(
        &world,
        crate::world::spawn_test_support::proto(&Biome::PLAINS, &Block::AIR),
    );
    let p = TestPlayer::new(&world);
    p.player.get_entity().set_pos(Vector3::new(8.5, 64.0, 8.5));
    p.player.permission_lvl.store(PermissionLvl::Four);
    p
}
pub fn stack(count: u8, item: &'static Item) -> ItemStack {
    let mut s = ItemStack::new(count, item);
    if count > s.get_max_stack_size() {
        s.set_data_component(pumpkin_data::data_component_impl::MaxStackSizeImpl { size: count });
    }
    let mut data = NbtCompound::new();
    data.put_int("launch_stock", 200);
    s.set_data_component(CustomDataImpl::new(data));
    s
}
pub fn use_item(p: &TestPlayer, f: &DeathTestWorld, hand: Hand) {
    p.client().handle_use_item(
        &p.player,
        &SUseItem {
            hand: VarInt(i32::from(hand != Hand::Right)),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
        &f.server,
    );
}
pub fn release(p: &TestPlayer, f: &DeathTestWorld) {
    p.player
        .living_entity
        .item_use_time
        .store(72000 - 30, SeqCst);
    p.client().handle_player_action(
        &p.player,
        &SPlayerAction {
            status: VarInt(Status::ReleaseItemInUse as i32),
            position: BlockPos::new(0, 0, 0),
            face: 0,
            sequence: VarInt(2),
        },
        &f.server,
    );
}
pub fn shoot(p: &TestPlayer, f: &DeathTestWorld, hand: Hand) {
    use_item(p, f, hand);
    release(p, f);
}
pub fn block_rocket(p: &TestPlayer, f: &DeathTestWorld, hand: Hand) {
    let position = BlockPos::new(8, 63, 8);
    f.world().set_block_state(
        &position,
        Block::STONE.default_state.id,
        pumpkin_world::world::BlockFlags::FORCE_STATE,
    );
    p.client()
        .handle_use_item_on(
            &p.player,
            &SUseItemOn {
                hand: VarInt(i32::from(hand != Hand::Right)),
                position,
                face: VarInt(1),
                cursor_pos: Vector3::new(0.5, 1.0, 0.5),
                inside_block: false,
                is_against_world_border: false,
                sequence: VarInt(3),
            },
            &f.server,
        )
        .unwrap();
}
pub fn slot(hand: Hand) -> usize {
    if hand == Hand::Right { 0 } else { 40 }
}
pub fn loose(p: &Player, item: &Item) -> usize {
    (0..p.inventory().size())
        .map(|i| p.inventory().get_stack(i))
        .filter(|s| s.item == item)
        .map(|s| usize::from(s.item_count))
        .sum()
}
pub fn charge(s: &ItemStack) -> usize {
    s.get_data_component::<ChargedProjectilesImpl>()
        .map_or(0, |c| c.projectiles.len())
}
pub fn used(p: &Player, item: &Item) -> i32 {
    p.stats
        .lock()
        .unwrap()
        .get(StatisticCategory::Used, i32::from(item.id))
}
pub fn packet_count(packets: &[bytes::Bytes], id: i32) -> usize {
    packets
        .iter()
        .filter(|p| p.as_ref().get_var_int().unwrap().0 == id)
        .count()
}
pub fn assert_slot(packets: &[bytes::Bytes], slot: usize, expected: &ItemStack) {
    let mut latest = None;
    for packet in packets {
        let mut data = packet.as_ref();
        if data.get_var_int().unwrap().0
            == pumpkin_data::packet::clientbound::play::SET_PLAYER_INVENTORY.0
            && data.get_var_int().unwrap().0 == slot as i32
        {
            latest = Some(ItemStackSerializer::read(&mut data).unwrap().0.into_owned());
        }
    }
    let actual = latest.unwrap_or_else(|| panic!("no authoritative packet for slot {slot}"));
    assert!(
        actual.are_equal(expected),
        "slot {slot}: packet {actual:?}, expected {expected:?}"
    );
}
pub fn assert_quiet(packets: &[bytes::Bytes]) {
    assert_eq!(
        packet_count(packets, pumpkin_data::packet::clientbound::play::SOUND.0),
        0,
        "veto published a successful launch sound"
    );
}

impl EventHandler<crate::plugin::api::events::player::fish::PlayerFishEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::fish::PlayerFishEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() == self.player.entity_id()
            && event.state == crate::plugin::api::events::player::fish::PlayerFishState::Fishing
        {
            let edit = {
                let mut s = self.state.lock().unwrap();
                s.casts += 1;
                if s.casts == 1 {
                    s.cast_edit.clone()
                } else {
                    None
                }
            };
            if let Some(edit) = edit {
                edit(&self.player);
            }
        }
        Box::pin(async {})
    }
}
impl EventHandler<crate::plugin::api::events::world::generic_game::GenericGameEvent> for Hooks {
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::world::generic_game::GenericGameEvent,
    ) -> BoxFuture<'a, ()> {
        if event.event_key == pumpkin_data::game_event::GameEvent::ItemInteractStart.name() {
            self.state.lock().unwrap().vibrations += 1;
        }
        Box::pin(async {})
    }
}

impl EventHandler<crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent>
    for Hooks
{
    fn handle_blocking<'a>(
        &'a self,
        _: &'a Arc<Server>,
        event: &'a mut crate::plugin::api::events::player::player_item_break::PlayerItemBreakEvent,
    ) -> BoxFuture<'a, ()> {
        if event.player.entity_id() == self.player.entity_id() {
            let edit = self.state.lock().unwrap().break_edit.clone();
            assert_owner_released(self.player.inventory().slot_transaction(0));
            if let Some(edit) = edit {
                edit(&self.player);
            }
        }
        Box::pin(async {})
    }
}

pub fn assert_launch_sound(packets: &[bytes::Bytes], admitted: bool) {
    assert_eq!(
        packet_count(packets, pumpkin_data::packet::clientbound::play::SOUND.0),
        usize::from(admitted)
    );
}
