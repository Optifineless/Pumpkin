use pumpkin_data::Block;
use pumpkin_macros::pumpkin_block;
use pumpkin_world::world::BlockFlags;

use crate::block::{BlockBehaviour, BonemealArgs};

#[pumpkin_block("minecraft:rooted_dirt")]
pub struct RootedDirtBlock;

impl BlockBehaviour for RootedDirtBlock {
    fn is_valid_bonemeal_target(&self, args: BonemealArgs<'_>) -> bool {
        let below = args.position.down();
        args.world.is_in_height_limit(below.0.y)
            && args.world.is_loaded(&below)
            && args.world.get_block_state(&below).is_air()
    }

    fn perform_bonemeal(&self, args: BonemealArgs<'_>) -> bool {
        {
            let below_state = args.world.get_block_state_id(&args.position.down());
            if !below_state.to_state().is_air() {
                return false;
            }
            args.world.set_block_states_from_observed(&[
                (
                    *args.position,
                    args.state_id,
                    args.state_id,
                    BlockFlags::empty(),
                ),
                (
                    args.position.down(),
                    below_state,
                    Block::HANGING_ROOTS.default_state.id,
                    BlockFlags::NOTIFY_ALL,
                ),
            ])
        }
    }
}
