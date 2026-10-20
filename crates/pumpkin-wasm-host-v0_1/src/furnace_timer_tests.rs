use super::*;
use pumpkin_core::block::entities::{
    blasting_furnace::BlastingFurnaceBlockEntity as InternalBlastingFurnaceBlockEntity,
    furnace::FurnaceBlockEntity as InternalFurnaceBlockEntity,
    smoker::SmokerBlockEntity as InternalSmokerBlockEntity,
};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::position::BlockPos;

#[test]
fn b1020c_g08_wasm_timer_projection_preserves_durable_ints() {
    macro_rules! check {
        ($internal:ty, $resource:ty, $host:ident) => {{
            let mut nbt = NbtCompound::new();
            nbt.put_int("cooking_time_spent", 70_000);
            nbt.put_int("cooking_total_time", -1);
            nbt.put_int("lit_time_remaining", 123_456);
            nbt.put_int("lit_total_time", -23_456);
            let entity = Arc::new(<$internal>::from_nbt(&nbt, BlockPos::new(0, 64, 0)));
            let mut host = PluginHostState::new();
            let resource = host.add::<$resource>(entity.clone()).unwrap();
            let borrow = || Resource::new_borrow(resource.rep());
            assert_eq!(
                $host::get_cooking_time_spent(&mut host, borrow()).unwrap(),
                4_464
            );
            assert_eq!(
                $host::get_cooking_total_time(&mut host, borrow()).unwrap(),
                65_535
            );
            assert_eq!(
                $host::get_lit_time_remaining(&mut host, borrow()).unwrap(),
                57_920
            );
            assert_eq!(
                $host::get_lit_total_time(&mut host, borrow()).unwrap(),
                42_080
            );
            assert!(entity.is_burning());
            let mut saved = NbtCompound::new();
            entity.write_internal(&mut saved);
            for key in [
                "cooking_time_spent",
                "cooking_total_time",
                "lit_time_remaining",
                "lit_total_time",
            ] {
                assert_eq!(saved.get(key), nbt.get(key));
                assert!(matches!(saved.get(key), Some(NbtTag::Int(_))));
            }
        }};
    }
    check!(
        InternalFurnaceBlockEntity,
        FurnaceBlockEntity,
        HostFurnaceBlockEntity
    );
    check!(
        InternalBlastingFurnaceBlockEntity,
        BlastingFurnaceBlockEntity,
        HostBlastingFurnaceBlockEntity
    );
    check!(
        InternalSmokerBlockEntity,
        SmokerBlockEntity,
        HostSmokerBlockEntity
    );
}
