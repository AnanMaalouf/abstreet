use geom::{ArrowCap, Distance, PolyLine, Polygon};
use map_model::{IntersectionID, LaneID, Map, TurnGroupID};
use std::collections::{HashMap, HashSet};

const TURN_ICON_ARROW_LENGTH: Distance = Distance::const_meters(1.5);

pub struct DrawTurnGroup {
    pub id: TurnGroupID,
    pub block: Polygon,
    pub arrow: Polygon,
}

impl DrawTurnGroup {
    pub fn for_i(i: IntersectionID, map: &Map) -> Vec<DrawTurnGroup> {
        // TODO Sort by angle here if we want some consistency
        // TODO Handle short roads
        let mut offset_per_lane: HashMap<LaneID, usize> = HashMap::new();
        let mut draw = Vec::new();
        for group in map.get_traffic_signal(i).turn_groups.values() {
            let offset = group
                .members
                .iter()
                .map(|t| *offset_per_lane.entry(t.src).or_insert(0))
                .max()
                .unwrap() as f32;
            let (pl, width) = group.src_center_and_width(map);
            let slice = if pl.length() >= (offset + 1.0) * TURN_ICON_ARROW_LENGTH {
                pl.exact_slice(
                    offset * TURN_ICON_ARROW_LENGTH,
                    (offset + 1.0) * TURN_ICON_ARROW_LENGTH,
                )
            } else {
                pl
            };
            let block = slice.make_polygons(width);

            let arrow = {
                let center = slice.middle();
                PolyLine::new(vec![
                    center.project_away(TURN_ICON_ARROW_LENGTH / 2.0, group.angle.opposite()),
                    center.project_away(TURN_ICON_ARROW_LENGTH / 2.0, group.angle),
                ])
                .make_arrow(Distance::meters(0.5), ArrowCap::Triangle)
                .unwrap()
            };

            let mut seen_lanes = HashSet::new();
            for t in &group.members {
                if !seen_lanes.contains(&t.src) {
                    *offset_per_lane.get_mut(&t.src).unwrap() += 1;
                    seen_lanes.insert(t.src);
                }
            }

            draw.push(DrawTurnGroup {
                id: group.id,
                block,
                arrow,
            });
        }
        draw
    }
}
