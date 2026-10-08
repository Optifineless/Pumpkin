use super::*;
use crate::entity::ai::pathfinder::{
    GroundPathNavigation, PathNavigationTrait, node::Node, node_evaluator::MobData,
};

#[test]
#[expect(clippy::unwrap_used, reason = "the test installs a nonempty path")]
fn waypoints_complete_at_commanded_height_and_large_mob_offset() {
    let node = BlockPos::new(4, 60, -2);
    // Each tick reaches the commanded integer Y. The old node.y + 0.5 sphere never advanced.
    let mut navigation = PathNavigation {
        mob_width: 0.6,
        path: Some(Path::new(
            vec![Node::new(node), Node::new(node.up())],
            node.up(),
            true,
        )),
        ..PathNavigation::default()
    };
    for y in [60.0, 61.0] {
        let (target, _) = navigation.next_move_target().unwrap();
        assert_eq!(target, Vector3::new(4.5, y, -1.5));
        navigation.follow_the_path_at(target, target, 1.0, |_| false);
    }
    assert!(navigation.next_move_target().is_none());
    // Elder guardian: width 1.9975 => target offset 1, tolerance 0.99875 on each horizontal axis.
    assert!(waypoint_reached(
        Vector3::new(5.0, 60.0, -1.0),
        node,
        1.9975,
        0.5
    ));
    // Square horizontal tolerance, not a circle. Water vertical tolerance is strictly <0.5.
    assert!(waypoint_reached(
        Vector3::new(4.9, 60.49, -1.1),
        node,
        0.6,
        0.5
    ));
    assert!(!waypoint_reached(
        Vector3::new(4.5, 60.5, -1.5),
        node,
        0.6,
        0.5
    ));
}

#[test]
fn navigator_update_gates_match_vanilla() {
    assert!(!NavigationKind::Ground.can_update(false, false, false));
    assert!(NavigationKind::Ground.can_update(false, false, true));
    assert!(NavigationKind::Ground.can_update(false, true, false));
    assert!(!NavigationKind::Flying { can_float: true }.can_update(true, false, true));
    assert!(NavigationKind::Flying { can_float: true }.can_update(false, true, true));
    assert!(NavigationKind::Flying { can_float: false }.can_update(false, false, false));
    assert!(
        !NavigationKind::Water {
            allow_breaching: false
        }
        .can_update(true, false, false)
    );
    assert!(
        NavigationKind::Water {
            allow_breaching: true
        }
        .can_update(false, false, false)
    );
    assert!(NavigationKind::Amphibious.can_update(false, false, true));
}

#[test]
fn floating_keeps_default_costs_and_constructor_overrides() {
    let mut navigation = GroundPathNavigation::new();
    navigation.set_can_float(true);
    let mut data = MobData::new(Vector3::new(0.0, 0.0, 0.0), 0.6, 1.8, 0.6);
    navigation.inner.apply_pathfinding_maluses(&mut data);
    // A route through one water or fire-neighbor node costs 1 + 8; fire costs 1 + 16.
    assert_eq!(1.0 + data.get_pathfinding_malus(PathType::Water), 9.0);
    assert_eq!(1.0 + data.get_pathfinding_malus(PathType::DangerFire), 9.0);
    assert_eq!(1.0 + data.get_pathfinding_malus(PathType::DamageFire), 17.0);
    assert!(data.get_pathfinding_malus(PathType::Lava) < 0.0);
    navigation.set_pathfinding_malus(PathType::Water, 0.0);
    navigation.inner.apply_pathfinding_maluses(&mut data);
    assert_eq!(1.0 + data.get_pathfinding_malus(PathType::Water), 1.0);
}

#[test]
fn corners_use_direction_and_strict_two_block_limit() {
    let node = BlockPos::new(0, 0, 0);
    let path = Path::new(
        vec![Node::new(node), Node::new(BlockPos::new(1, 0, 0))],
        node,
        true,
    );
    assert!(should_target_next_node_in_direction(
        Vector3::new(1.0, 0.0, 0.5),
        &path,
        || false
    ));
    assert!(!should_target_next_node_in_direction(
        Vector3::new(-1.5, 0.0, 0.5),
        &path,
        || true
    ));
    assert!(should_target_next_node_in_direction(
        Vector3::new(-1.4, 0.0, 0.5),
        &path,
        || true
    ));
    assert!(!should_target_next_node_in_direction(
        Vector3::new(-1.4, 0.0, 0.5),
        &path,
        || false
    ));
}

#[test]
fn spider_finishes_by_three_dimensional_block_center() {
    assert!(!wall_target_reached(
        Vector3::new(0.5, 4.0, 0.5),
        BlockPos::new(0, 0, 0),
        0.4
    ));
    assert!(wall_target_reached(
        Vector3::new(0.5, 4.5, 0.5),
        BlockPos::new(0, 0, 0),
        0.4
    ));
}

#[test]
fn path_follower_does_not_cut_hazard_corners() {
    let pos = Vector3::new(1.1, 0.0, 0.5);
    let make_path = |path_type| {
        let mut first = Node::new(BlockPos::new(0, 0, 0));
        first.path_type = path_type;
        Path::new(
            vec![first, Node::new(BlockPos::new(1, 0, 0))],
            BlockPos::new(1, 0, 0),
            true,
        )
    };
    let mut navigation = PathNavigation {
        path: Some(make_path(PathType::DangerOther)),
        ..PathNavigation::default()
    };
    navigation.follow_the_path_at(pos, pos, 1.0, |_| true);
    assert_eq!(
        navigation.path.as_ref().map(Path::get_next_node_index),
        Some(0)
    );
    navigation.path = Some(make_path(PathType::Walkable));
    navigation.follow_the_path_at(pos, pos, 1.0, |_| false);
    assert_eq!(
        navigation.path.as_ref().map(Path::get_next_node_index),
        Some(1)
    );
}
