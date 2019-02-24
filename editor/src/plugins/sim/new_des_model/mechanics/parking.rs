use crate::plugins::sim::new_des_model::{ParkedCar, ParkingSpot, Vehicle};
use abstutil::{deserialize_btreemap, serialize_btreemap};
use geom::{Angle, Distance, Pt2D};
use map_model;
use map_model::{Lane, LaneID, LaneType, Map, Position, Traversable};
use serde_derive::{Deserialize, Serialize};
use sim::{CarID, CarState, DrawCarInput, VehicleType};
use std::collections::{BTreeMap, BTreeSet};
use std::iter;

#[derive(Serialize, Deserialize, PartialEq)]
pub struct ParkingSimState {
    #[serde(
        serialize_with = "serialize_btreemap",
        deserialize_with = "deserialize_btreemap"
    )]
    cars: BTreeMap<CarID, ParkedCar>,
    // TODO hacky, but other types of lanes just mark 0 spots. :\
    lanes: Vec<ParkingLane>,
    reserved_spots: BTreeSet<ParkingSpot>,
}

impl ParkingSimState {
    pub fn new(map: &Map) -> ParkingSimState {
        ParkingSimState {
            cars: BTreeMap::new(),
            lanes: map
                .all_lanes()
                .iter()
                .map(|l| ParkingLane::new(l))
                .collect(),
            reserved_spots: BTreeSet::new(),
        }
    }

    /*pub fn edit_remove_lane(&mut self, id: LaneID) {
        assert!(self.lanes[id.0].is_empty());
        self.lanes[id.0] = ParkingLane {
            id,
            spots: Vec::new(),
            occupants: Vec::new(),
        };
    }

    pub fn edit_add_lane(&mut self, l: &Lane) {
        self.lanes[l.id.0] = ParkingLane::new(l);
    }

    // TODO bad name
    pub fn total_count(&self) -> usize {
        self.cars.len()
    }*/

    pub fn get_free_spots(&self, lane: LaneID) -> Vec<ParkingSpot> {
        let l = &self.lanes[lane.0];
        let mut spots: Vec<ParkingSpot> = Vec::new();
        for (idx, maybe_occupant) in l.occupants.iter().enumerate() {
            if maybe_occupant.is_none() {
                spots.push(ParkingSpot::new(l.id, idx));
            }
        }
        spots
    }

    pub fn remove_parked_car(&mut self, p: ParkedCar) {
        self.cars.remove(&p.vehicle.id);
        self.lanes[p.spot.lane.0].remove_parked_car(p.vehicle.id);
    }

    pub fn add_parked_car(&mut self, p: ParkedCar) {
        let spot = p.spot;
        assert!(self.reserved_spots.remove(&p.spot));
        assert_eq!(self.lanes[spot.lane.0].occupants[spot.idx], None);
        self.lanes[spot.lane.0].occupants[spot.idx] = Some(p.vehicle.id);
        self.cars.insert(p.vehicle.id, p);
    }

    pub fn reserve_spot(&mut self, spot: ParkingSpot) {
        self.reserved_spots.insert(spot);
    }

    pub fn get_draw_cars(&self, id: LaneID, map: &Map) -> Vec<DrawCarInput> {
        self.lanes[id.0]
            .occupants
            .iter()
            .filter_map(|maybe_occupant| {
                if let Some(car) = maybe_occupant {
                    Some(self.get_draw_car(*car, map).unwrap())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn get_draw_car(&self, id: CarID, map: &Map) -> Option<DrawCarInput> {
        let p = self.cars.get(&id)?;
        let lane = p.spot.lane;

        let front_dist = self.lanes[lane.0].spots[p.spot.idx].dist_along_for_car(&p.vehicle);
        Some(DrawCarInput {
            id: p.vehicle.id,
            waiting_for_turn: None,
            stopping_trace: None,
            state: CarState::Parked,
            vehicle_type: VehicleType::Car,
            on: Traversable::Lane(lane),

            body: map
                .get_l(lane)
                .lane_center_pts
                .slice(front_dist - p.vehicle.length, front_dist)
                .unwrap()
                .0,
        })
    }

    pub fn get_all_draw_cars(&self, map: &Map) -> Vec<DrawCarInput> {
        self.cars
            .keys()
            .map(|id| self.get_draw_car(*id, map).unwrap())
            .collect()
    }

    /*pub fn lookup_car(&self, id: CarID) -> Option<&ParkedCar> {
        self.cars.get(&id)
    }*/

    pub fn is_free(&self, spot: ParkingSpot) -> bool {
        self.lanes[spot.lane.0].occupants[spot.idx].is_none()
            && !self.reserved_spots.contains(&spot)
    }

    pub fn get_car_at_spot(&self, spot: ParkingSpot) -> CarID {
        self.lanes[spot.lane.0].occupants[spot.idx].unwrap()
    }

    pub fn get_first_free_spot(
        &self,
        parking_pos: Position,
        vehicle: &Vehicle,
    ) -> Option<ParkingSpot> {
        let lane = parking_pos.lane();
        let l = &self.lanes[lane.0];
        let idx = l.occupants.iter().enumerate().position(|(idx, x)| {
            x.is_none()
                && !self.reserved_spots.contains(&ParkingSpot::new(lane, idx))
                && parking_pos.dist_along() <= l.spots[idx].dist_along_for_car(vehicle)
        })?;
        Some(ParkingSpot::new(lane, idx))
    }

    /*pub fn get_car_at_spot(&self, spot: ParkingSpot) -> Option<ParkedCar> {
        let car = self.lanes[spot.lane.0].occupants[spot.idx]?;
        Some(self.cars[&car].clone())
    }*/

    pub fn spot_to_driving_pos(
        &self,
        spot: ParkingSpot,
        vehicle: &Vehicle,
        driving_lane: LaneID,
        map: &Map,
    ) -> Position {
        Position::new(spot.lane, self.get_spot(spot).dist_along_for_car(vehicle))
            .equiv_pos(driving_lane, map)
    }

    pub fn spot_to_sidewalk_pos(&self, spot: ParkingSpot, sidewalk: LaneID, map: &Map) -> Position {
        Position::new(spot.lane, self.get_spot(spot).dist_along_for_ped()).equiv_pos(sidewalk, map)
    }

    fn get_spot(&self, spot: ParkingSpot) -> &ParkingSpotGeometry {
        &self.lanes[spot.lane.0].spots[spot.idx]
    }

    /*pub fn tooltip_lines(&self, id: CarID) -> Vec<String> {
        let c = self.lookup_car(id).unwrap();
        vec![format!(
            "{} is parked, owned by {:?}",
            c.vehicle.id, c.owner
        )]
    }

    pub fn get_parked_cars_by_owner(&self, id: BuildingID) -> Vec<&ParkedCar> {
        let mut result: Vec<&ParkedCar> = Vec::new();
        for p in self.cars.values() {
            if p.owner == Some(id) {
                result.push(p);
            }
        }
        result
    }

    pub fn get_owner_of_car(&self, id: CarID) -> Option<BuildingID> {
        self.lookup_car(id).and_then(|p| p.owner)
    }

    pub fn count(&self, lanes: &HashSet<LaneID>) -> (usize, usize) {
        // TODO self.cars.len() works for cars_parked; we could maintain the other value easily too
        let mut cars_parked = 0;
        let mut open_parking_spots = 0;

        for id in lanes {
            for maybe_car in &self.lanes[id.0].occupants {
                if maybe_car.is_some() {
                    cars_parked += 1;
                } else {
                    open_parking_spots += 1;
                }
            }
        }

        (cars_parked, open_parking_spots)
    }*/
}

#[derive(Serialize, Deserialize, PartialEq)]
struct ParkingLane {
    id: LaneID,
    spots: Vec<ParkingSpotGeometry>,
    occupants: Vec<Option<CarID>>,
}

impl ParkingLane {
    fn new(l: &Lane) -> ParkingLane {
        if l.lane_type != LaneType::Parking {
            return ParkingLane {
                id: l.id,
                spots: Vec::new(),
                occupants: Vec::new(),
            };
        }

        ParkingLane {
            id: l.id,
            occupants: iter::repeat(None).take(l.number_parking_spots()).collect(),
            spots: (0..l.number_parking_spots())
                .map(|idx| {
                    let spot_start = map_model::PARKING_SPOT_LENGTH * (2.0 + idx as f64);
                    let (pos, angle) = l.dist_along(spot_start);
                    ParkingSpotGeometry {
                        dist_along: spot_start,
                        pos,
                        angle,
                    }
                })
                .collect(),
        }
    }

    fn remove_parked_car(&mut self, car: CarID) {
        let idx = self.occupants.iter().position(|x| *x == Some(car)).unwrap();
        self.occupants[idx] = None;
    }

    /*fn is_empty(&self) -> bool {
        !self.occupants.iter().any(|x| x.is_some())
    }*/
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ParkingSpotGeometry {
    // These 3 are of the front of the parking spot (farthest along)
    dist_along: Distance,
    pos: Pt2D,
    angle: Angle,
}

impl ParkingSpotGeometry {
    fn dist_along_for_ped(&self) -> Distance {
        // Always centered in the entire parking spot
        self.dist_along - (map_model::PARKING_SPOT_LENGTH / 2.0)
    }

    fn dist_along_for_car(&self, vehicle: &Vehicle) -> Distance {
        // Find the offset to center this particular car in the parking spot
        self.dist_along - (map_model::PARKING_SPOT_LENGTH - vehicle.length) / 2.0
    }
}