use crate::{AgentID, CarID, Event, TripID, TripMode, VehicleType};
use abstutil::Counter;
use derivative::Derivative;
use geom::{Distance, Duration, DurationHistogram, PercentageHistogram, Time};
use map_model::{
    BusRouteID, BusStopID, IntersectionID, Map, Path, PathRequest, RoadID, Traversable,
};
use serde_derive::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Serialize, Deserialize, Derivative)]
pub struct Analytics {
    pub thruput_stats: ThruputStats,
    #[serde(skip_serializing, skip_deserializing)]
    pub(crate) test_expectations: VecDeque<Event>,
    pub bus_arrivals: Vec<(Time, CarID, BusRouteID, BusStopID)>,
    #[serde(skip_serializing, skip_deserializing)]
    pub total_bus_passengers: Counter<BusRouteID>,
    // TODO Hack: No TripMode means aborted
    // Finish time, ID, mode (or None as aborted), trip duration
    pub finished_trips: Vec<(Time, TripID, Option<TripMode>, Duration)>,
    // TODO This subsumes finished_trips
    pub trip_log: Vec<(Time, TripID, Option<PathRequest>, String)>,
    pub intersection_delays: BTreeMap<IntersectionID, Vec<(Time, Duration)>>,
}

#[derive(Serialize, Deserialize, Derivative)]
pub struct ThruputStats {
    #[serde(skip_serializing, skip_deserializing)]
    pub count_per_road: Counter<RoadID>,
    #[serde(skip_serializing, skip_deserializing)]
    pub count_per_intersection: Counter<IntersectionID>,

    raw_per_road: Vec<(Time, TripMode, RoadID)>,
    raw_per_intersection: Vec<(Time, TripMode, IntersectionID)>,
}

impl Analytics {
    pub fn new() -> Analytics {
        Analytics {
            thruput_stats: ThruputStats {
                count_per_road: Counter::new(),
                count_per_intersection: Counter::new(),
                raw_per_road: Vec::new(),
                raw_per_intersection: Vec::new(),
            },
            test_expectations: VecDeque::new(),
            bus_arrivals: Vec::new(),
            total_bus_passengers: Counter::new(),
            finished_trips: Vec::new(),
            trip_log: Vec::new(),
            intersection_delays: BTreeMap::new(),
        }
    }

    pub fn event(&mut self, ev: Event, time: Time, map: &Map) {
        // TODO Plumb a flag
        let raw_thruput = true;

        // Throughput
        if let Event::AgentEntersTraversable(a, to) = ev {
            let mode = match a {
                AgentID::Pedestrian(_) => TripMode::Walk,
                AgentID::Car(c) => match c.1 {
                    VehicleType::Car => TripMode::Drive,
                    VehicleType::Bike => TripMode::Bike,
                    VehicleType::Bus => TripMode::Transit,
                },
            };

            match to {
                Traversable::Lane(l) => {
                    let r = map.get_l(l).parent;
                    self.thruput_stats.count_per_road.inc(r);
                    if raw_thruput {
                        self.thruput_stats.raw_per_road.push((time, mode, r));
                    }
                }
                Traversable::Turn(t) => {
                    self.thruput_stats.count_per_intersection.inc(t.parent);
                    if raw_thruput {
                        self.thruput_stats
                            .raw_per_intersection
                            .push((time, mode, t.parent));
                    }
                }
            };
        }

        // Test expectations
        if !self.test_expectations.is_empty() && &ev == self.test_expectations.front().unwrap() {
            println!("At {}, met expectation {:?}", time, ev);
            self.test_expectations.pop_front();
        }

        // Bus arrivals
        if let Event::BusArrivedAtStop(bus, route, stop) = ev {
            self.bus_arrivals.push((time, bus, route, stop));
        }

        // Bus passengers
        if let Event::PedEntersBus(_, _, route) = ev {
            self.total_bus_passengers.inc(route);
        }

        // Finished trips
        if let Event::TripFinished(id, mode, dt) = ev {
            self.finished_trips.push((time, id, Some(mode), dt));
        } else if let Event::TripAborted(id) = ev {
            self.finished_trips.push((time, id, None, Duration::ZERO));
        }

        // Intersection delays
        if let Event::IntersectionDelayMeasured(id, delay) = ev {
            self.intersection_delays
                .entry(id)
                .or_insert_with(Vec::new)
                .push((time, delay));
        }

        // Trip log
        if let Event::TripPhaseStarting(id, maybe_req, metadata) = ev {
            self.trip_log.push((time, id, maybe_req, metadata));
        } else if let Event::TripAborted(id) = ev {
            self.trip_log
                .push((time, id, None, format!("trip aborted for some reason")));
        } else if let Event::TripFinished(id, _, _) = ev {
            self.trip_log
                .push((time, id, None, format!("trip finished")));
        }
    }

    pub fn record_backpressure(&mut self, path: &Path) {
    }

    // TODO If these ever need to be speeded up, just cache the histogram and index in the events
    // list.

    pub fn finished_trips(&self, now: Time, mode: TripMode) -> DurationHistogram {
        let mut distrib = DurationHistogram::new();
        for (t, _, m, dt) in &self.finished_trips {
            if *t > now {
                break;
            }
            if *m == Some(mode) {
                distrib.add(*dt);
            }
        }
        distrib
    }

    // Returns (all trips except aborted, number of aborted trips, trips by mode)
    pub fn all_finished_trips(
        &self,
        now: Time,
    ) -> (
        DurationHistogram,
        usize,
        BTreeMap<TripMode, DurationHistogram>,
    ) {
        let mut per_mode = TripMode::all()
            .into_iter()
            .map(|m| (m, DurationHistogram::new()))
            .collect::<BTreeMap<_, _>>();
        let mut all = DurationHistogram::new();
        let mut num_aborted = 0;
        for (t, _, m, dt) in &self.finished_trips {
            if *t > now {
                break;
            }
            if let Some(mode) = *m {
                all.add(*dt);
                per_mode.get_mut(&mode).unwrap().add(*dt);
            } else {
                num_aborted += 1;
            }
        }
        (all, num_aborted, per_mode)
    }

    pub fn bus_arrivals(&self, now: Time, r: BusRouteID) -> BTreeMap<BusStopID, DurationHistogram> {
        let mut per_bus: BTreeMap<CarID, Vec<(Time, BusStopID)>> = BTreeMap::new();
        for (t, car, route, stop) in &self.bus_arrivals {
            if *t > now {
                break;
            }
            if *route == r {
                per_bus
                    .entry(*car)
                    .or_insert_with(Vec::new)
                    .push((*t, *stop));
            }
        }
        let mut delay_to_stop: BTreeMap<BusStopID, DurationHistogram> = BTreeMap::new();
        for events in per_bus.values() {
            for pair in events.windows(2) {
                delay_to_stop
                    .entry(pair[1].1)
                    .or_insert_with(DurationHistogram::new)
                    .add(pair[1].0 - pair[0].0);
            }
        }
        delay_to_stop
    }

    // TODO Refactor!
    // For each stop, a list of (time, delay)
    pub fn bus_arrivals_over_time(
        &self,
        now: Time,
        r: BusRouteID,
    ) -> BTreeMap<BusStopID, Vec<(Time, Duration)>> {
        let mut per_bus: BTreeMap<CarID, Vec<(Time, BusStopID)>> = BTreeMap::new();
        for (t, car, route, stop) in &self.bus_arrivals {
            if *t > now {
                break;
            }
            if *route == r {
                per_bus
                    .entry(*car)
                    .or_insert_with(Vec::new)
                    .push((*t, *stop));
            }
        }
        let mut delays_to_stop: BTreeMap<BusStopID, Vec<(Time, Duration)>> = BTreeMap::new();
        for events in per_bus.values() {
            for pair in events.windows(2) {
                delays_to_stop
                    .entry(pair[1].1)
                    .or_insert_with(Vec::new)
                    .push((pair[1].0, pair[1].0 - pair[0].0));
            }
        }
        delays_to_stop
    }

    // Slightly misleading -- TripMode::Transit means buses, not pedestrians taking transit
    pub fn throughput_road(
        &self,
        now: Time,
        road: RoadID,
        bucket: Duration,
    ) -> BTreeMap<TripMode, Vec<(Time, usize)>> {
        let mut max_this_bucket = now.min(Time::START_OF_DAY + bucket);
        let mut per_mode = TripMode::all()
            .into_iter()
            .map(|m| (m, vec![(Time::START_OF_DAY, 0), (max_this_bucket, 0)]))
            .collect::<BTreeMap<_, _>>();
        for (t, m, r) in &self.thruput_stats.raw_per_road {
            if *r != road {
                continue;
            }
            if *t > now {
                break;
            }
            if *t > max_this_bucket {
                max_this_bucket = now.min(max_this_bucket + bucket);
                for vec in per_mode.values_mut() {
                    vec.push((max_this_bucket, 0));
                }
            }
            per_mode.get_mut(m).unwrap().last_mut().unwrap().1 += 1;
        }
        per_mode
    }

    // TODO Refactor!
    pub fn throughput_intersection(
        &self,
        now: Time,
        intersection: IntersectionID,
        bucket: Duration,
    ) -> BTreeMap<TripMode, Vec<(Time, usize)>> {
        let mut per_mode = TripMode::all()
            .into_iter()
            .map(|m| (m, vec![(Time::START_OF_DAY, 0)]))
            .collect::<BTreeMap<_, _>>();
        let mut max_this_bucket = Time::START_OF_DAY + bucket;
        for (t, m, i) in &self.thruput_stats.raw_per_intersection {
            if *i != intersection {
                continue;
            }
            if *t > now {
                break;
            }
            if *t > max_this_bucket {
                max_this_bucket = now.min(max_this_bucket + bucket);
                for vec in per_mode.values_mut() {
                    vec.push((max_this_bucket, 0));
                }
            }
            per_mode.get_mut(m).unwrap().last_mut().unwrap().1 += 1;
        }
        per_mode
    }

    pub fn get_trip_phases(&self, trip: TripID, map: &Map) -> Vec<TripPhase> {
        let mut phases: Vec<TripPhase> = Vec::new();
        for (t, id, maybe_req, md) in &self.trip_log {
            if *id != trip {
                continue;
            }
            if let Some(ref mut last) = phases.last_mut() {
                last.end_time = Some(*t);
            }
            if md == "trip finished" || md == "trip aborted for some reason" {
                break;
            }
            phases.push(TripPhase {
                start_time: *t,
                end_time: None,
                // Unwrap should be safe, because this is the request that was actually done...
                path: maybe_req
                    .as_ref()
                    .map(|req| (req.start.dist_along(), map.pathfind(req.clone()).unwrap())),
                description: md.clone(),
            })
        }
        phases
    }

    fn get_all_trip_phases(&self) -> BTreeMap<TripID, Vec<TripPhase>> {
        let mut trips = BTreeMap::new();
        for (t, id, _, md) in &self.trip_log {
            let phases: &mut Vec<TripPhase> = trips.entry(*id).or_insert_with(Vec::new);
            if let Some(ref mut last) = phases.last_mut() {
                last.end_time = Some(*t);
            }
            if md == "trip finished" {
                continue;
            }
            // Remove aborted trips
            if md == "trip aborted for some reason" {
                trips.remove(id);
                continue;
            }
            phases.push(TripPhase {
                start_time: *t,
                end_time: None,
                // Don't compute any paths
                path: None,
                description: md.clone(),
            })
        }
        trips
    }

    pub fn analyze_parking_phases(&self) -> Vec<String> {
        // Of all completed trips involving parking, what percentage of total time was spent as
        // "overhead" -- not the main driving part of the trip?
        // TODO This is misleading for border trips -- the driving lasts longer.
        let mut distrib = PercentageHistogram::new();
        for (_, phases) in self.get_all_trip_phases() {
            if phases.last().as_ref().unwrap().end_time.is_none() {
                continue;
            }
            let mut driving_time = Duration::ZERO;
            let mut overhead = Duration::ZERO;
            for p in phases {
                let dt = p.end_time.unwrap() - p.start_time;
                // TODO New enum instead of strings, if there'll be more analyses like this
                if p.description.starts_with("CarID(") {
                    driving_time += dt;
                } else if p.description == "parking somewhere else"
                    || p.description == "parking on the current lane"
                {
                    overhead += dt;
                } else if p.description.starts_with("PedestrianID(") {
                    overhead += dt;
                } else {
                    // Waiting for a bus. Irrelevant.
                }
            }
            // Only interested in trips with both
            if driving_time == Duration::ZERO || overhead == Duration::ZERO {
                continue;
            }
            distrib.add(overhead / (driving_time + overhead));
        }
        vec![format!("Consider all trips with both a walking and driving portion"), format!("The portion of the trip spent walking to the parked car, looking for parking, and walking from the parking space to the final destination are all overhead."), format!("So what's the distribution of overhead percentages look like? 0% is ideal -- the entire trip is spent just driving between the original source and destination."), distrib.describe()]
    }

    pub fn intersection_delays(&self, i: IntersectionID, t1: Time, t2: Time) -> DurationHistogram {
        let mut delays = DurationHistogram::new();
        // TODO Binary search
        if let Some(list) = self.intersection_delays.get(&i) {
            for (t, dt) in list {
                if *t < t1 {
                    continue;
                }
                if *t > t2 {
                    break;
                }
                delays.add(*dt);
            }
        }
        delays
    }

    pub fn intersection_delays_bucketized(
        &self,
        now: Time,
        i: IntersectionID,
        bucket: Duration,
    ) -> Vec<(Time, DurationHistogram)> {
        let mut max_this_bucket = now.min(Time::START_OF_DAY + bucket);
        let mut results = vec![
            (Time::START_OF_DAY, DurationHistogram::new()),
            (max_this_bucket, DurationHistogram::new()),
        ];
        if let Some(list) = self.intersection_delays.get(&i) {
            for (t, dt) in list {
                if *t > now {
                    break;
                }
                if *t > max_this_bucket {
                    max_this_bucket = now.min(max_this_bucket + bucket);
                    results.push((max_this_bucket, DurationHistogram::new()));
                }
                results.last_mut().unwrap().1.add(*dt);
            }
        }
        results
    }
}

pub struct TripPhase {
    pub start_time: Time,
    pub end_time: Option<Time>,
    // Plumb along start distance
    pub path: Option<(Distance, Path)>,
    pub description: String,
}

impl TripPhase {
    pub fn describe(&self, now: Time) -> String {
        if let Some(t2) = self.end_time {
            format!(
                "{} .. {} ({}): {}",
                self.start_time,
                t2,
                t2 - self.start_time,
                self.description
            )
        } else {
            format!(
                "{} .. ongoing ({} so far): {}",
                self.start_time,
                now - self.start_time,
                self.description
            )
        }
    }
}
