use crate::colors;
use crate::common::{tool_panel, Minimap, Overlays, Warping};
use crate::edit::EditMode;
use crate::game::{msg, State, Transition};
use crate::helpers::ID;
use crate::managed::WrappedComposite;
use crate::sandbox::gameplay::{GameplayMode, GameplayState};
use crate::sandbox::SandboxMode;
use crate::sandbox::{spawn_agents_around, AgentMeter, SpeedControls, TimePanel};
use crate::ui::UI;
use abstutil::Timer;
use ezgui::{
    hotkey, lctrl, Button, Color, Composite, EventCtx, GeomBatch, GfxCtx, HorizontalAlignment, Key,
    Line, ManagedWidget, Outcome, ScreenPt, Text, VerticalAlignment,
};
use geom::{Distance, Duration, PolyLine, Polygon, Pt2D, Statistic, Time};
use map_model::{BuildingID, IntersectionID, IntersectionType, LaneType, RoadID};
use sim::{AgentID, BorderSpawnOverTime, CarID, OriginDestination, Scenario, VehicleType};
use std::collections::BTreeSet;

pub struct Tutorial {
    top_center: Composite,
    last_finished_task: Task,

    msg_panel: Option<Composite>,
    warped: bool,
    exit: bool,
}

impl Tutorial {
    pub fn new(ctx: &mut EventCtx, ui: &mut UI, current: usize) -> Box<dyn GameplayState> {
        if ui.session.tutorial.is_none() {
            ui.session.tutorial = Some(TutorialState::new(ctx, ui));
        }
        let mut tut = ui.session.tutorial.take().unwrap();
        tut.current = current;
        tut.latest = tut.latest.max(current);
        let state = tut.make_state(ctx, ui);
        ui.session.tutorial = Some(tut);
        state
    }

    // True if we should exit
    pub fn event_with_speed(
        &mut self,
        ctx: &mut EventCtx,
        ui: &mut UI,
        maybe_speed: Option<&mut SpeedControls>,
    ) -> (Option<Transition>, bool) {
        let tut = ui.session.tutorial.as_mut().unwrap();

        if tut.interaction() == Task::PauseResume {
            let is_paused = maybe_speed.unwrap().is_paused();
            if tut.was_paused && !is_paused {
                tut.was_paused = false;
            }
            if !tut.was_paused && is_paused {
                tut.num_pauses += 1;
                tut.was_paused = true;
                self.top_center = tut.make_top_center(ctx, false);
            }
            if tut.num_pauses == 3 {
                tut.next();
                return (Some(transition(ctx, ui)), false);
            }
        }
        (None, std::mem::replace(&mut self.exit, false))
    }
}

impl GameplayState for Tutorial {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Option<Transition> {
        let mut tut = ui.session.tutorial.as_mut().unwrap();

        // First of all, might need to initiate warping
        if !self.warped {
            match tut.stage() {
                Stage::Msg { ref warp_to, .. } | Stage::Interact { ref warp_to, .. } => {
                    if let Some((id, zoom)) = warp_to {
                        self.warped = true;
                        return Some(Transition::Push(Warping::new(
                            ctx,
                            id.canonical_point(&ui.primary).unwrap(),
                            Some(*zoom),
                            None,
                            &mut ui.primary,
                        )));
                    }
                }
            }
        }

        match self.top_center.event(ctx) {
            Some(Outcome::Clicked(x)) => match x.as_ref() {
                "Quit" => {
                    self.exit = true;
                    return None;
                }
                "Start over" => {
                    tut.current = 0;
                    return Some(transition(ctx, ui));
                }
                "Last completed step" => {
                    tut.current = tut.latest;
                    return Some(transition(ctx, ui));
                }
                "previous tutorial screen" => {
                    tut.current -= 1;
                    return Some(transition(ctx, ui));
                }
                "next tutorial screen" => {
                    tut.current += 1;
                    return Some(transition(ctx, ui));
                }
                "edit map" => {
                    // TODO Ideally this would be an inactive button in message states
                    if self.msg_panel.is_none() {
                        let mode = GameplayMode::Tutorial(tut.current);
                        return Some(Transition::Push(Box::new(EditMode::new(ctx, ui, mode))));
                    }
                }
                _ => unreachable!(),
            },
            None => {}
        }

        if let Some(ref mut msg) = self.msg_panel {
            match msg.event(ctx) {
                Some(Outcome::Clicked(x)) => match x.as_ref() {
                    "OK" => {
                        tut.next();
                        if tut.current == tut.stages.len() {
                            // TODO Clear edits?
                            ui.primary.clear_sim();
                            return Some(Transition::Pop);
                        } else {
                            return Some(transition(ctx, ui));
                        }
                    }
                    _ => unreachable!(),
                },
                None => {
                    // Don't allow other interactions
                    return Some(Transition::Keep);
                }
            }
        }

        // Interaction things
        if tut.interaction() == Task::Camera {
            if ui.primary.current_selection == Some(ID::Building(BuildingID(9)))
                && ui.per_obj.left_click(ctx, "put out the... fire?")
            {
                tut.next();
                return Some(transition(ctx, ui));
            }
        } else if tut.interaction() == Task::InspectObjects {
            match ui.primary.current_selection {
                Some(ID::Lane(l)) => {
                    if ui.per_obj.action(ctx, Key::I, "inspect the lane") {
                        tut.inspected_lane = true;
                        self.top_center = tut.make_top_center(ctx, false);
                        return Some(Transition::Push(msg(
                            "Inspection",
                            match ui.primary.map.get_l(l).lane_type {
                                LaneType::Driving => vec![
                                    "This is a regular lane for driving.",
                                    "Cars, bikes, and buses all share it.",
                                ],
                                LaneType::Parking => vec!["This is an on-street parking lane."],
                                LaneType::Sidewalk => {
                                    vec!["This is a sidewalk. Only pedestrians can use it."]
                                }
                                LaneType::Biking => vec!["This is a bike-only lane."],
                                LaneType::Bus => vec!["This is a bus lane. Bikes may also use it."],
                                LaneType::SharedLeftTurn => vec![
                                    "This is a lane where either direction of traffic can turn \
                                     left.",
                                ],
                                LaneType::Construction => {
                                    vec!["This lane is currently closed for construction."]
                                }
                            },
                        )));
                    }
                }
                Some(ID::Building(_)) => {
                    if ui.per_obj.action(ctx, Key::I, "inspect the building") {
                        tut.inspected_building = true;
                        self.top_center = tut.make_top_center(ctx, false);
                        return Some(Transition::Push(msg(
                            "Inspection",
                            vec![
                                "Knock knock, anyone home?",
                                "Did you know: most trips begin and end at a building.",
                            ],
                        )));
                    }
                }
                Some(ID::Intersection(i)) => {
                    if ui.per_obj.action(ctx, Key::I, "inspect the intersection") {
                        match ui.primary.map.get_i(i).intersection_type {
                            IntersectionType::StopSign => {
                                tut.inspected_stop_sign = true;
                                self.top_center = tut.make_top_center(ctx, false);
                                return Some(Transition::Push(msg(
                                    "Inspection",
                                    vec!["Most intersections are regulated by stop signs."],
                                )));
                            }
                            IntersectionType::TrafficSignal => {
                                return Some(Transition::Push(msg(
                                    "Inspection",
                                    vec![
                                        "This intersection is controlled by a traffic signal. \
                                         You'll learn more about these soon.",
                                    ],
                                )));
                            }
                            IntersectionType::Border => {
                                tut.inspected_border = true;
                                self.top_center = tut.make_top_center(ctx, false);
                                return Some(Transition::Push(msg(
                                    "Inspection",
                                    vec![
                                        "This is a border of the map. Vehicles appear and \
                                         disappear here.",
                                    ],
                                )));
                            }
                            IntersectionType::Construction => {
                                return Some(Transition::Push(msg(
                                    "Inspection",
                                    vec!["This intersection is currently closed for construction."],
                                )));
                            }
                        }
                    }
                }
                _ => {}
            }
            if tut.inspected_lane
                && tut.inspected_building
                && tut.inspected_stop_sign
                && tut.inspected_border
            {
                tut.next();
                return Some(transition(ctx, ui));
            }
        } else if tut.interaction() == Task::TimeControls {
            if ui.primary.sim.time() >= Time::START_OF_DAY + Duration::hours(17) {
                tut.next();
                return Some(transition(ctx, ui));
            }
        } else if tut.interaction() == Task::Escort {
            let target = CarID(30, VehicleType::Car);
            let is_parked = ui.primary.sim.agent_to_trip(AgentID::Car(target)).is_none();
            if !tut.car_parked && is_parked && tut.following_car {
                tut.car_parked = true;
                self.top_center = tut.make_top_center(ctx, false);
            }

            if let Some(ID::Car(c)) = ui.primary.current_selection {
                // TODO Need to plumb along CommonState and see if we actually have the panel open
                // or not.
                if !tut.following_car && c == target {
                    tut.following_car = true;
                    self.top_center = tut.make_top_center(ctx, false);
                }

                if ui.per_obj.action(ctx, Key::C, "draw WASH ME") {
                    if c == target {
                        if is_parked {
                            tut.next();
                            return Some(transition(ctx, ui));
                        } else {
                            return Some(Transition::Push(msg(
                                "Not yet!",
                                vec![
                                    "You're going to run up to an occupied car and draw on their \
                                     windows?",
                                    "Sounds like we should be friends.",
                                    "But, er, wait for the car to park. (You can speed up time!)",
                                ],
                            )));
                        }
                    } else if c.1 == VehicleType::Bike {
                        return Some(Transition::Push(msg(
                            "That's a bike",
                            vec![
                                "Achievement unlocked: You attempted to draw WASH ME on a cyclist.",
                                "This game is PG-13 or something, so I can't really describe what \
                                 happens next.",
                                "But uh, don't try this at home.",
                            ],
                        )));
                    } else {
                        return Some(Transition::Push(msg(
                            "Wrong car",
                            vec![
                                "You're looking at the wrong car.",
                                "Use the 'reset to midnight' (key binding 'X') to start over, if \
                                 you lost the car to follow.",
                            ],
                        )));
                    }
                }
            }
        } else if tut.interaction() == Task::LowParking {
            if let Some(ID::Lane(l)) = ui.primary.current_selection {
                if ui
                    .per_obj
                    .action(ctx, Key::C, "check the parking availability")
                {
                    let lane = ui.primary.map.get_l(l);
                    if !lane.is_parking() {
                        return Some(Transition::Push(msg(
                            "Uhh..",
                            vec!["That's not even a parking lane"],
                        )));
                    }
                    let percent = (ui.primary.sim.get_free_spots(l).len() as f64)
                        / (lane.number_parking_spots() as f64);
                    if percent > 0.1 {
                        return Some(Transition::Push(msg(
                            "Not quite",
                            vec![
                                format!("This lane has {:.0}% spots free", percent * 100.0),
                                "Try using the 'parking availability' layer from the minimap \
                                 controls"
                                    .to_string(),
                            ],
                        )));
                    }
                    tut.next();
                    return Some(transition(ctx, ui));
                }
            }
        } else if tut.interaction() == Task::WatchBikes {
            if ui.primary.sim.time() >= Time::START_OF_DAY + Duration::minutes(2) {
                tut.next();
                return Some(transition(ctx, ui));
            }
        } else if tut.interaction() == Task::FixBikes {
            if ui.primary.sim.is_done() {
                let (all, _, _) = ui
                    .primary
                    .sim
                    .get_analytics()
                    .all_finished_trips(ui.primary.sim.time());
                let max = all.select(Statistic::Max);

                if !tut.score_delivered {
                    tut.score_delivered = true;
                    if ui.primary.map.get_edits().commands.is_empty() {
                        return Some(Transition::Push(msg(
                            "All trips completed",
                            vec![
                                "You didn't change anything!",
                                "Try editing the map to create some bike lanes.",
                            ],
                        )));
                    }
                    // TODO Prebake results and use the normal differential stuff
                    let baseline = Duration::minutes(7) + Duration::seconds(15.0);
                    if max > baseline {
                        return Some(Transition::Push(msg(
                            "All trips completed",
                            vec![
                                "Your changes made things worse!".to_string(),
                                format!(
                                    "The slowest trip originally took {}, but now it took {}",
                                    baseline, max
                                ),
                                "".to_string(),
                                "Try again!".to_string(),
                            ],
                        )));
                    }
                    // TODO Tune. The real solution doesn't work because of sim bugs.
                    if max > Duration::minutes(6) + Duration::seconds(40.0) {
                        return Some(Transition::Push(msg(
                            "All trips completed",
                            vec![
                                "Nice, you helped things a bit!".to_string(),
                                format!(
                                    "The slowest trip originally took {}, but now it took {}",
                                    baseline, max
                                ),
                                "".to_string(),
                                "See if you can do a little better though.".to_string(),
                            ],
                        )));
                    }
                    return Some(Transition::Push(msg(
                        "All trips completed",
                        vec![format!(
                            "Awesome! The slowest trip originally took {}, but now it only took {}",
                            baseline, max
                        )],
                    )));
                }
                if max <= Duration::minutes(6) + Duration::seconds(30.0) {
                    tut.next();
                }
                return Some(transition(ctx, ui));
            }
        } else if tut.interaction() == Task::WatchBuses {
            if ui.primary.sim.time() >= Time::START_OF_DAY + Duration::minutes(5) {
                tut.next();
                return Some(transition(ctx, ui));
            }
        }

        None
    }

    fn draw(&self, g: &mut GfxCtx, ui: &UI) {
        let tut = ui.session.tutorial.as_ref().unwrap();

        if self.msg_panel.is_some() {
            State::grey_out_map(g);
        }

        self.top_center.draw(g);

        if let Some(ref msg) = self.msg_panel {
            // Arrows underneath the message panel, but on top of other panels
            if let Stage::Msg { point_to, .. } = tut.stage() {
                if let Some(fxn) = point_to {
                    let pt = (fxn)(g, ui);
                    g.fork_screenspace();
                    g.draw_polygon(
                        Color::RED,
                        &PolyLine::new(vec![
                            self.msg_panel.as_ref().unwrap().center_of("OK").to_pt(),
                            pt,
                        ])
                        .make_arrow(Distance::meters(20.0))
                        .unwrap(),
                    );
                    g.unfork();
                }
            }

            msg.draw(g);
        }

        // Special things
        if tut.interaction() == Task::Camera {
            g.draw_polygon(
                Color::hex("#e25822"),
                &ui.primary.map.get_b(BuildingID(9)).polygon,
            );
        }
    }

    fn can_examine_objects(&self) -> bool {
        self.last_finished_task >= Task::WatchBikes
    }
    fn has_common(&self) -> bool {
        self.last_finished_task >= Task::Camera
    }
    fn has_tool_panel(&self) -> bool {
        self.last_finished_task >= Task::Camera
    }
    fn has_time_panel(&self) -> bool {
        self.last_finished_task >= Task::InspectObjects
    }
    fn has_speed(&self) -> bool {
        self.last_finished_task >= Task::InspectObjects
    }
    fn has_agent_meter(&self) -> bool {
        self.last_finished_task >= Task::PauseResume
    }
    fn has_minimap(&self) -> bool {
        self.last_finished_task >= Task::Escort
    }
}

#[derive(PartialEq, PartialOrd, Clone, Copy)]
enum Task {
    Nil,
    Camera,
    InspectObjects,
    TimeControls,
    PauseResume,
    Escort,
    LowParking,
    WatchBikes,
    FixBikes,
    WatchBuses,
    FixBuses,
}

impl Task {
    fn top_txt(self, ctx: &EventCtx, state: &TutorialState) -> Text {
        let simple = match self {
            Task::Nil => unreachable!(),
            Task::Camera => "Put out the fire at the Montlake Market",
            Task::InspectObjects => {
                let mut txt = Text::from(Line("Inspect one of each: ").fg(Color::CYAN));
                txt.append(Line("lane").fg(if state.inspected_lane {
                    Color::GREEN
                } else {
                    Color::CYAN
                }));
                txt.append(Line(", "));
                txt.append(
                    Line("intersection with stop sign").fg(if state.inspected_stop_sign {
                        Color::GREEN
                    } else {
                        Color::CYAN
                    }),
                );
                txt.append(Line(", "));
                // That manual word wrap theaux
                txt.add(Line("building").fg(if state.inspected_building {
                    Color::GREEN
                } else {
                    Color::CYAN
                }));
                txt.append(Line(", "));
                txt.append(
                    Line("intersection on the map border").fg(if state.inspected_border {
                        Color::GREEN
                    } else {
                        Color::CYAN
                    }),
                );
                return txt;
            }
            Task::TimeControls => "Wait until 5pm",
            Task::PauseResume => {
                let mut txt = Text::from(Line("Pause/resume ").fg(Color::CYAN));
                txt.append(Line(format!("{} times", 3 - state.num_pauses)).fg(Color::GREEN));
                return txt;
            }
            Task::Escort => {
                // Inspect the target car, wait for them to park, draw WASH ME on the window
                let mut txt = Text::new();
                txt.append(Line("Follow the target car").fg(if state.following_car {
                    Color::GREEN
                } else {
                    Color::CYAN
                }));
                txt.append(Line(", ").fg(Color::CYAN));
                txt.append(Line("wait for them to park").fg(if state.car_parked {
                    Color::GREEN
                } else {
                    Color::CYAN
                }));
                txt.append(Line(",").fg(Color::CYAN));
                txt.add(Line("then draw WASH ME on the window").fg(Color::CYAN));
                return txt;
            }
            Task::LowParking => "Find a road with almost no parking spots available",
            Task::WatchBikes => "Watch for 2 minutes",
            Task::FixBikes => "Make better use of the road space",
            Task::WatchBuses => "Watch the buses for 5 minutes",
            Task::FixBuses => "Speed up bus 43 and 48",
        };

        let mut txt = Text::new();
        txt.add_wrapped(simple.to_string(), 0.6 * ctx.canvas.window_width);
        txt.change_fg(Color::CYAN)
    }
}

enum Stage {
    Msg {
        lines: Vec<&'static str>,
        point_to: Option<Box<dyn Fn(&GfxCtx, &UI) -> Pt2D>>,
        warp_to: Option<(ID, f64)>,
        spawn: Option<Box<dyn Fn(&mut UI)>>,
    },
    Interact {
        task: Task,
        warp_to: Option<(ID, f64)>,
        spawn: Option<Box<dyn Fn(&mut UI)>>,
    },
}

impl Stage {
    fn msg(lines: Vec<&'static str>) -> Stage {
        Stage::Msg {
            lines,
            point_to: None,
            warp_to: None,
            spawn: None,
        }
    }

    fn interact(task: Task) -> Stage {
        Stage::Interact {
            task,
            warp_to: None,
            spawn: None,
        }
    }

    fn arrow(self, pt: ScreenPt) -> Stage {
        self.dynamic_arrow(Box::new(move |_, _| pt.to_pt()))
    }
    fn dynamic_arrow(mut self, cb: Box<dyn Fn(&GfxCtx, &UI) -> Pt2D>) -> Stage {
        match self {
            Stage::Msg {
                ref mut point_to, ..
            } => {
                assert!(point_to.is_none());
                *point_to = Some(cb);
                self
            }
            Stage::Interact { .. } => unreachable!(),
        }
    }

    fn warp_to(mut self, id: ID, zoom: Option<f64>) -> Stage {
        match self {
            Stage::Msg {
                ref mut warp_to, ..
            }
            | Stage::Interact {
                ref mut warp_to, ..
            } => {
                assert!(warp_to.is_none());
                *warp_to = Some((id, zoom.unwrap_or(4.0)));
                self
            }
        }
    }

    fn spawn(mut self, cb: Box<dyn Fn(&mut UI)>) -> Stage {
        match self {
            Stage::Msg { ref mut spawn, .. } | Stage::Interact { ref mut spawn, .. } => {
                assert!(spawn.is_none());
                *spawn = Some(cb);
                self
            }
        }
    }

    fn spawn_around(self, i: IntersectionID) -> Stage {
        self.spawn(Box::new(move |ui| spawn_agents_around(i, ui)))
    }

    fn spawn_randomly(self) -> Stage {
        self.spawn(Box::new(|ui| {
            Scenario::small_run(&ui.primary.map).instantiate(
                &mut ui.primary.sim,
                &ui.primary.map,
                &mut ui.primary.current_flags.sim_flags.make_rng(),
                &mut Timer::throwaway(),
            )
        }))
    }
}

pub struct TutorialState {
    stages: Vec<Stage>,
    latest: usize,
    pub current: usize,

    // Goofy state for just some stages.
    inspected_lane: bool,
    inspected_building: bool,
    inspected_stop_sign: bool,
    inspected_border: bool,

    was_paused: bool,
    num_pauses: usize,

    following_car: bool,
    car_parked: bool,

    score_delivered: bool,
}

fn start_bike_lane_scenario(ui: &mut UI) {
    let mut s = Scenario::empty(&ui.primary.map, "car/bike contention");
    s.border_spawn_over_time.push(BorderSpawnOverTime {
        num_peds: 0,
        num_cars: 10,
        num_bikes: 10,
        percent_use_transit: 0.0,
        start_time: Time::START_OF_DAY,
        stop_time: Time::START_OF_DAY + Duration::seconds(10.0),
        start_from_border: RoadID(303).backwards(),
        goal: OriginDestination::GotoBldg(BuildingID(3)),
    });
    s.instantiate(
        &mut ui.primary.sim,
        &ui.primary.map,
        &mut ui.primary.current_flags.sim_flags.make_rng(),
        &mut Timer::throwaway(),
    );
    ui.primary.sim.step(&ui.primary.map, Duration::seconds(0.1));
}

fn start_bus_lane_scenario(ui: &mut UI) {
    let mut s = Scenario::empty(&ui.primary.map, "car/bus contention");
    let mut routes = BTreeSet::new();
    routes.insert("43".to_string());
    routes.insert("48".to_string());
    s.only_seed_buses = Some(routes);
    for src in vec![
        RoadID(61).backwards(),
        RoadID(240).forwards(),
        RoadID(56).forwards(),
    ] {
        s.border_spawn_over_time.push(BorderSpawnOverTime {
            num_peds: 100,
            num_cars: 100,
            num_bikes: 0,
            percent_use_transit: 1.0,
            start_time: Time::START_OF_DAY,
            stop_time: Time::START_OF_DAY + Duration::seconds(10.0),
            start_from_border: src,
            goal: OriginDestination::EndOfRoad(RoadID(0).forwards()),
        });
    }
    s.instantiate(
        &mut ui.primary.sim,
        &ui.primary.map,
        &mut ui.primary.current_flags.sim_flags.make_rng(),
        &mut Timer::throwaway(),
    );
    ui.primary.sim.step(&ui.primary.map, Duration::seconds(0.1));
}

fn transition(ctx: &mut EventCtx, ui: &mut UI) -> Transition {
    let tut = ui.session.tutorial.as_mut().unwrap();
    tut.reset_state();
    let mode = GameplayMode::Tutorial(tut.current);
    Transition::Replace(Box::new(SandboxMode::new(ctx, ui, mode)))
}

impl TutorialState {
    // These're mutex to each state, but still important to reset. Otherwise if you go back to a
    // previous interaction stage, it'll just be automatically marked done.
    fn reset_state(&mut self) {
        self.inspected_lane = false;
        self.inspected_building = false;
        self.inspected_stop_sign = false;
        self.inspected_border = false;
        self.was_paused = true;
        self.num_pauses = 0;
        self.score_delivered = false;
        self.following_car = false;
        self.car_parked = false;
    }

    fn stage(&self) -> &Stage {
        &self.stages[self.current]
    }

    fn interaction(&self) -> Task {
        match self.stage() {
            Stage::Msg { .. } => Task::Nil,
            Stage::Interact { task, .. } => *task,
        }
    }

    fn next(&mut self) {
        self.current += 1;
        self.latest = self.latest.max(self.current);
    }

    fn make_top_center(&self, ctx: &mut EventCtx, edit_map: bool) -> Composite {
        let mut col = vec![ManagedWidget::row(vec![
            ManagedWidget::draw_text(ctx, Text::from(Line("Tutorial").size(26))).margin(5),
            ManagedWidget::draw_batch(
                ctx,
                GeomBatch::from(vec![(Color::WHITE, Polygon::rectangle(2.0, 50.0))]),
            )
            .margin(5),
            ManagedWidget::draw_text(
                ctx,
                Text::from(Line(format!("{}/{}", self.current + 1, self.stages.len())).size(20)),
            )
            .margin(5),
            if self.current == 0 {
                Button::inactive_button(ctx, "<")
            } else {
                WrappedComposite::nice_text_button(
                    ctx,
                    Text::from(Line("<")),
                    None,
                    "previous tutorial screen",
                )
            }
            .margin(5),
            if self.current == self.latest {
                Button::inactive_button(ctx, ">")
            } else {
                WrappedComposite::nice_text_button(
                    ctx,
                    Text::from(Line(">")),
                    None,
                    "next tutorial screen",
                )
            }
            .margin(5),
            if self.current == 0 {
                Button::inactive_button(ctx, "Start over")
            } else {
                WrappedComposite::text_button(ctx, "Start over", None)
            }
            .margin(5),
            if self.current == self.latest {
                Button::inactive_button(ctx, "Last completed step")
            } else {
                WrappedComposite::text_button(ctx, "Last completed step", None)
            }
            .margin(5),
            WrappedComposite::text_button(ctx, "Quit", None).margin(5),
        ])
        .centered()];
        if let Stage::Interact { task, .. } = self.stage() {
            col.push(ManagedWidget::draw_text(ctx, task.top_txt(ctx, self)).margin(5));
        }
        if edit_map {
            col.push(
                WrappedComposite::svg_button(
                    ctx,
                    "../data/system/assets/tools/edit_map.svg",
                    "edit map",
                    lctrl(Key::E),
                )
                .margin(5),
            );
        }

        Composite::new(ManagedWidget::col(col).bg(colors::PANEL_BG))
            .aligned(HorizontalAlignment::Center, VerticalAlignment::Top)
            .build(ctx)
    }

    fn make_state(&self, ctx: &mut EventCtx, ui: &mut UI) -> Box<dyn GameplayState> {
        if let Stage::Msg { .. } = self.stage() {
            ui.primary.current_selection = None;
        }

        // TODO Should some of this always happen?
        ui.primary.clear_sim();
        ui.overlay = Overlays::Inactive;
        if let Some(cb) = match self.stage() {
            Stage::Msg { ref spawn, .. } => spawn,
            Stage::Interact { ref spawn, .. } => spawn,
        } {
            let old = ui.primary.current_flags.sim_flags.rng_seed;
            ui.primary.current_flags.sim_flags.rng_seed = Some(42);
            (cb)(ui);
            ui.primary.current_flags.sim_flags.rng_seed = old;
            ui.primary.sim.step(&ui.primary.map, Duration::seconds(0.1));
        }

        let mut last_finished_task = Task::Nil;
        for stage in &self.stages[0..self.current] {
            if let Stage::Interact { task, .. } = stage {
                last_finished_task = *task;
            }
        }

        Box::new(Tutorial {
            top_center: self.make_top_center(ctx, last_finished_task >= Task::WatchBikes),
            last_finished_task,

            msg_panel: match self.stage() {
                Stage::Msg { ref lines, .. } => Some(
                    Composite::new(
                        ManagedWidget::col(vec![
                            ManagedWidget::draw_text(ctx, {
                                let mut txt = Text::new();
                                for l in lines {
                                    txt.add(Line(*l));
                                }
                                txt
                            }),
                            WrappedComposite::text_button(ctx, "OK", hotkey(Key::Enter))
                                .centered_horiz()
                                .padding(10),
                        ])
                        .bg(colors::PANEL_BG)
                        .outline(5.0, Color::WHITE)
                        .padding(5),
                    )
                    .aligned(HorizontalAlignment::Center, VerticalAlignment::Center)
                    .build(ctx),
                ),
                Stage::Interact { .. } => None,
            },
            exit: false,
            warped: false,
        })
    }

    fn new(ctx: &mut EventCtx, ui: &mut UI) -> TutorialState {
        let mut state = TutorialState {
            stages: Vec::new(),
            latest: 0,
            current: 0,

            inspected_lane: false,
            inspected_building: false,
            inspected_stop_sign: false,
            inspected_border: false,
            was_paused: true,
            num_pauses: 0,
            following_car: false,
            car_parked: false,
            score_delivered: false,
        };

        let tool_panel = tool_panel(ctx);
        let time = TimePanel::new(ctx, ui);
        let speed = SpeedControls::new(ctx);
        let agent_meter = AgentMeter::new(ctx, ui);
        // The minimap is hidden at low zoom levels
        let orig_zoom = ctx.canvas.cam_zoom;
        ctx.canvas.cam_zoom = 100.0;
        let minimap = Minimap::new(ctx, ui);
        ctx.canvas.cam_zoom = orig_zoom;

        let osd = ScreenPt::new(
            0.1 * ctx.canvas.window_width,
            0.97 * ctx.canvas.window_height,
        );

        state.stages.extend(vec![Stage::msg(vec![
            "Welcome to your first day as a contract traffic engineer --",
            "like a paid assassin, but capable of making WAY more people cry.",
            "Seattle is a fast-growing city, and nobody can decide how to fix the traffic.",
        ])
        .warp_to(ID::Intersection(IntersectionID(141)), None)]);

        state.stages.extend(vec![
            Stage::msg(vec![
                "Let's start with the controls.",
                "Click and drag to pan around the map, and use your scroll wheel or touchpad to \
                 zoom in and out.",
            ]),
            Stage::msg(vec![
                "Let's try that ou--",
                "WHOA THE MONTLAKE MARKET IS ON FIRE!",
                "GO CLICK ON IT, QUICK!",
            ]),
            Stage::msg(vec!["(Hint: Look around for an unusually red building)"]),
            // TODO Just zoom in sufficiently on it, maybe don't even click it yet.
            Stage::interact(Task::Camera),
        ]);

        state.stages.extend(vec![
            Stage::msg(vec![
                "Er, sorry about that.",
                "Just a little joke we like to play on the new recruits.",
            ]),
            Stage::msg(vec![
                "If you're going to storm out of here, you can always go back towards the main \
                 screen using this button",
                "(But please continue with the training.)",
            ])
            .arrow(tool_panel.inner.center_of("back")),
            Stage::msg(vec![
                "Now, let's learn how to inspect and interact with objects in the map.",
                "Select something with your mouse, then click on it.",
            ]),
            Stage::msg(vec![
                "(By the way, the bottom of the screen shows keyboard shortcuts",
                "for whatever you're selecting; you don't have to click an object first.",
            ])
            .arrow(osd),
            Stage::msg(vec![
                "I wonder what kind of information is available for different objects? Let's find \
                 out!",
            ]),
            Stage::interact(Task::InspectObjects),
        ]);

        state.stages.extend(vec![
            Stage::msg(vec![
                "Inspection complete!",
                "",
                "You'll work day and night, watching traffic patterns unfold.",
            ])
            .arrow(time.composite.center_of_panel())
            .warp_to(ID::Intersection(IntersectionID(64)), None),
            Stage::msg(vec!["You can pause or resume time"])
                .arrow(speed.composite.inner.center_of("pause")),
            Stage::msg(vec!["Speed things up"]).arrow(speed.composite.inner.center_of("60x")),
            Stage::msg(vec!["Advance time by certain amounts"])
                .arrow(speed.composite.inner.center_of("step forwards 1 hour")),
            Stage::msg(vec!["And reset to the beginning of the day"])
                .arrow(speed.composite.inner.center_of("reset to midnight")),
            Stage::msg(vec!["Let's try these controls out. Just wait until 5pm."]),
            Stage::interact(Task::TimeControls),
        ]);

        state.stages.extend(vec![
            Stage::msg(vec!["Whew, that took a while! (Hopefully not though...)"]),
            Stage::msg(vec![
                "You might've figured it out already,",
                "But you'll be pausing/resuming time VERY frequently",
            ])
            .arrow(speed.composite.inner.center_of("pause")),
            Stage::msg(vec![
                "Again, most controls have a key binding shown at the bottom of the screen.",
                "Press SPACE to pause/resume time.",
            ])
            .arrow(osd),
            Stage::msg(vec![
                "Just reassure me and pause/resume time a few times, alright?",
            ]),
            Stage::interact(Task::PauseResume),
        ]);

        state.stages.extend(vec![
            Stage::msg(vec!["Alright alright, no need to wear out your spacebar."]),
            // Don't center on where the agents are, be a little offset
            Stage::msg(vec![
                "Oh look, some people appeared!",
                "We've got pedestrians, bikes, and cars moving around now.",
            ])
            .warp_to(ID::Building(BuildingID(813)), Some(10.0))
            .spawn_around(IntersectionID(247)),
            Stage::msg(vec![
                "You can see the number of them in the top-right corner.",
            ])
            .arrow(agent_meter.composite.center_of_panel())
            .spawn_around(IntersectionID(247)),
            Stage::msg(vec![
                "Why don't you follow this car to their destination,",
                "see where they park, and then play a little... prank?",
            ])
            .spawn_around(IntersectionID(247))
            .warp_to(ID::Building(BuildingID(813)), Some(10.0))
            .dynamic_arrow(Box::new(|g, ui| {
                g.canvas
                    .map_to_screen(
                        ui.primary
                            .sim
                            .canonical_pt_for_agent(
                                AgentID::Car(CarID(30, VehicleType::Car)),
                                &ui.primary.map,
                            )
                            .unwrap(),
                    )
                    .to_pt()
            })),
            Stage::msg(vec![
                "You don't have to manually chase them; just click to follow.",
                "(If you do lose track of them, just reset)",
            ])
            .spawn_around(IntersectionID(247))
            .warp_to(ID::Building(BuildingID(813)), Some(10.0))
            .arrow(speed.composite.inner.center_of("reset to midnight")),
            Stage::interact(Task::Escort)
                .spawn_around(IntersectionID(247))
                .warp_to(ID::Building(BuildingID(813)), Some(10.0)),
        ]);

        state.stages.extend(vec![
            Stage::msg(vec![
                "What an immature prank. You should re-evaluate your life decisions.",
                "",
                "The map is quite large, so to help you orient",
                "the minimap shows you an overview of all activity.",
            ])
            .arrow(minimap.composite.center_of("minimap")),
            Stage::msg(vec!["Find addresses here"]).arrow(minimap.composite.center_of("search")),
            Stage::msg(vec!["Set up shortcuts to favorite areas"])
                .arrow(minimap.composite.center_of("shortcuts")),
            Stage::msg(vec!["View different data about agents"])
                .arrow(minimap.composite.center_of("change agent colorscheme")),
            Stage::msg(vec!["Apply different heatmap layers to the map"])
                .arrow(minimap.composite.center_of("change overlay")),
            Stage::msg(vec![
                "Let's try these out.",
                "There are lots of cars parked everywhere.",
                "Can you find a road that's almost out of parking spots?",
            ]),
            Stage::interact(Task::LowParking).spawn_randomly(),
        ]);

        state.stages.extend(vec![
            Stage::msg(vec![
                "Well done!",
                "",
                "Let's see what's happening over here.",
                "(Just watch for a moment.)",
            ])
            .warp_to(ID::Building(BuildingID(543)), None)
            .spawn(Box::new(start_bike_lane_scenario)),
            Stage::interact(Task::WatchBikes).spawn(Box::new(start_bike_lane_scenario)),
        ]);

        let top_center = state.make_top_center(ctx, true);
        state.stages.extend(vec![
            Stage::msg(vec![
                "Looks like lots of cars and bikes trying to go to the playfield.",
                "When lots of cars and bikes share the same lane,",
                "cars are delayed (assuming there's no room to pass) and",
                "the cyclist probably feels unsafe too.",
            ]),
            Stage::msg(vec![
                "Luckily, you have the power to modify lanes!",
                "What if you could transform the parking lanes that aren't being used much",
                "into a protected bike lane?",
            ]),
            Stage::msg(vec![
                "To edit lanes, click 'edit map', choose a paintbrush, then apply it to a lane.",
            ])
            .arrow(top_center.center_of("edit map")),
            Stage::msg(vec![
                "Some changes you make can't take effect until the next day;",
                "like what if you removed a parking lane while there are cars on it?",
                "So when you leave edit mode, the day will always reset to midnight.",
                "People are on fixed schedules: every day, everybody leaves at exactly the same \
                 time,",
                "making the same decision to drive, walk, bike, or take a bus.",
                "All you can influence is how their experience will be in the short term.",
            ]),
            Stage::msg(vec![
                "So try to speed up all of the trips. When all trips are done, you'll get your \
                 final score.",
            ])
            .arrow(agent_meter.composite.center_of_panel()),
            Stage::interact(Task::FixBikes).spawn(Box::new(start_bike_lane_scenario)),
        ]);

        if false {
            // TODO There's no clear measurement for how well the buses are doing.
            // TODO Probably want a steady stream of the cars appearing

            state.stages.extend(vec![
                Stage::msg(vec![
                    "Alright, now it's a game day at the University of Washington.",
                    "Everyone's heading north across the bridge.",
                    "Watch what happens to the bus 43 and 48.",
                ])
                .warp_to(ID::Building(BuildingID(1979)), Some(0.5))
                .spawn(Box::new(start_bus_lane_scenario)),
                Stage::interact(Task::WatchBuses).spawn(Box::new(start_bus_lane_scenario)),
            ]);
            // 9 interacts

            state.stages.extend(vec![
                Stage::msg(vec![
                    "Let's speed up the poor bus! Why not dedicate some bus lanes to it?",
                ])
                .warp_to(ID::Building(BuildingID(1979)), Some(0.5))
                .spawn(Box::new(start_bus_lane_scenario)),
                // TODO By how much?
                Stage::interact(Task::FixBuses).spawn(Box::new(start_bus_lane_scenario)),
            ]);
            // 10 interacts
        }

        state.stages.push(Stage::msg(vec![
            "Training complete!",
            "Use sandbox mode to explore larger areas of Seattle and try out any ideas you have.",
            "Or try your skills at a particular challenge!",
            "",
            "Go have the appropriate amount of fun.",
        ]));

        // For my debugging sanity
        if ui.opts.dev {
            state.latest = state.stages.len() - 1;
        }

        state

        // TODO Multi-modal trips -- including parking. (Cars per bldg, ownership)
        // TODO Explain the finished trip data
        // The city is in total crisis. You've only got 10 days to do something before all hell
        // breaks loose and people start kayaking / ziplining / crab-walking / cartwheeling / to
        // work.
    }
}
