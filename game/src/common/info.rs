use crate::colors;
use crate::common::{ColorLegend, Warping};
use crate::game::{msg, Transition};
use crate::helpers::{rotating_color_map, ID};
use crate::managed::WrappedComposite;
use crate::render::{dashed_lines, Renderable, MIN_ZOOM_FOR_DETAIL};
use crate::sandbox::SpeedControls;
use crate::ui::UI;
use abstutil::prettyprint_usize;
use ezgui::{
    hotkey, Button, Color, Composite, Drawable, EventCtx, GeomBatch, GfxCtx, HorizontalAlignment,
    Key, Line, ManagedWidget, Outcome, Plot, RewriteColor, Series, Text, VerticalAlignment,
};
use geom::{Circle, Distance, Duration, Statistic, Time};
use map_model::{IntersectionID, RoadID};
use sim::{AgentID, CarID, TripEnd, TripID, TripMode, TripStart, VehicleType};
use std::collections::BTreeSet;

pub struct InfoPanel {
    pub id: ID,
    pub time: Time,
    pub composite: Composite,

    also_draw: Drawable,
    // (unzoomed, zoomed)
    trip_details: Option<(Drawable, Drawable)>,

    actions: Vec<(Key, String)>,
}

impl InfoPanel {
    pub fn new(
        id: ID,
        ctx: &mut EventCtx,
        ui: &UI,
        mut actions: Vec<(Key, String)>,
        maybe_speed: Option<&mut SpeedControls>,
    ) -> InfoPanel {
        if maybe_speed.map(|s| s.is_paused()).unwrap_or(false)
            && id.agent_id().is_some()
            && actions
                .get(0)
                .map(|(_, a)| a != "follow agent")
                .unwrap_or(true)
        {
            actions.insert(0, (Key::F, "follow agent".to_string()));
        }

        let mut col = info_for(id.clone(), ctx, ui);

        let trip_details = if let Some(trip) = match id {
            ID::Trip(t) => Some(t),
            ID::Car(c) => {
                if c.1 == VehicleType::Bus {
                    None
                } else {
                    ui.primary.sim.agent_to_trip(AgentID::Car(c))
                }
            }
            ID::Pedestrian(p) => ui.primary.sim.agent_to_trip(AgentID::Pedestrian(p)),
            _ => None,
        } {
            let (rows, unzoomed, zoomed) = trip_details(trip, ctx, ui);
            col.push(rows);
            Some((unzoomed, zoomed))
        } else {
            None
        };

        for (key, label) in &actions {
            let mut txt = Text::new();
            txt.append(Line(key.describe()).fg(ezgui::HOTKEY_COLOR));
            txt.append(Line(format!(" - {}", label)));
            col.push(
                ManagedWidget::btn(Button::text_bg(
                    txt,
                    colors::SECTION_BG,
                    colors::HOVERING,
                    hotkey(*key),
                    label,
                    ctx,
                ))
                .margin(5),
            );
        }

        // Follow the agent. When the sim is paused, this lets the player naturally pan away,
        // because the InfoPanel isn't being updated.
        // TODO Should we pin to the trip, not the specific agent?
        if let Some(pt) = id
            .agent_id()
            .and_then(|a| ui.primary.sim.canonical_pt_for_agent(a, &ui.primary.map))
        {
            ctx.canvas.center_on_map_pt(pt);
        }

        let mut batch = GeomBatch::new();
        // TODO Handle transitions between peds and crowds better
        if let Some(obj) = ui.primary.draw_map.get_obj(
            id.clone(),
            ui,
            &mut ui.primary.draw_map.agents.borrow_mut(),
            ctx.prerender,
        ) {
            // Different selection styles for different objects.
            match id {
                ID::Car(_) | ID::Pedestrian(_) | ID::PedCrowd(_) => {
                    // Some objects are much wider/taller than others
                    let multiplier = match id {
                        ID::Car(c) => {
                            if c.1 == VehicleType::Bike {
                                3.0
                            } else {
                                0.75
                            }
                        }
                        ID::Pedestrian(_) => 3.0,
                        ID::PedCrowd(_) => 0.75,
                        _ => unreachable!(),
                    };
                    // Make a circle to cover the object.
                    let bounds = obj.get_outline(&ui.primary.map).get_bounds();
                    let radius = multiplier * Distance::meters(bounds.width().max(bounds.height()));
                    batch.push(
                        ui.cs.get_def("current object", Color::WHITE).alpha(0.5),
                        Circle::new(bounds.center(), radius).to_polygon(),
                    );
                    batch.push(
                        ui.cs.get("current object"),
                        Circle::outline(bounds.center(), radius, Distance::meters(0.3)),
                    );

                    // TODO And actually, don't cover up the agent. The Renderable API isn't quite
                    // conducive to doing this yet.
                }
                _ => {
                    batch.push(Color::BLUE, obj.get_outline(&ui.primary.map));
                }
            }
        }

        // Show relationships between some objects
        if let ID::Car(c) = id {
            if let Some(b) = ui.primary.sim.get_owner_of_car(c) {
                // TODO Mention this, with a warp tool
                batch.push(
                    ui.cs
                        .get_def("something associated with something else", Color::PURPLE),
                    ui.primary.draw_map.get_b(b).get_outline(&ui.primary.map),
                );
            }
        }
        if let ID::Building(b) = id {
            for p in ui.primary.sim.get_parked_cars_by_owner(b) {
                batch.push(
                    ui.cs.get("something associated with something else"),
                    ui.primary
                        .draw_map
                        .get_obj(
                            ID::Car(p.vehicle.id),
                            ui,
                            &mut ui.primary.draw_map.agents.borrow_mut(),
                            ctx.prerender,
                        )
                        .unwrap()
                        .get_outline(&ui.primary.map),
                );
            }
        }

        InfoPanel {
            id,
            actions,
            trip_details,
            time: ui.primary.sim.time(),
            composite: Composite::new(ManagedWidget::col(col).bg(colors::PANEL_BG).padding(10))
                .aligned(
                    HorizontalAlignment::Percent(0.02),
                    VerticalAlignment::Percent(0.2),
                )
                .max_size_percent(35, 60)
                .build(ctx),
            also_draw: batch.upload(ctx),
        }
    }

    // (Are we done, optional transition)
    pub fn event(
        &mut self,
        ctx: &mut EventCtx,
        ui: &mut UI,
        maybe_speed: Option<&mut SpeedControls>,
    ) -> (bool, Option<Transition>) {
        // Can click on the map to cancel
        if ctx.canvas.get_cursor_in_map_space().is_some()
            && ui.primary.current_selection.is_none()
            && ui.per_obj.left_click(ctx, "stop showing info")
        {
            return (true, None);
        }

        // Live update?
        if ui.primary.sim.time() != self.time {
            if let Some(a) = self.id.agent_id() {
                if !ui.primary.sim.does_agent_exist(a) {
                    // TODO Get a TripResult, slightly more detail?
                    return (
                        true,
                        Some(Transition::Push(msg(
                            "Closing info panel",
                            vec![format!("{} is gone", a)],
                        ))),
                    );
                }
            }
            // TODO Detect crowds changing here maybe

            let preserve_scroll = self.composite.preserve_scroll();
            *self = InfoPanel::new(self.id.clone(), ctx, ui, self.actions.clone(), maybe_speed);
            self.composite.restore_scroll(ctx, preserve_scroll);
            return (false, None);
        }

        match self.composite.event(ctx) {
            Some(Outcome::Clicked(action)) => {
                if action == "X" {
                    return (true, None);
                } else if action == "jump to object" {
                    return (
                        false,
                        Some(Transition::Push(Warping::new(
                            ctx,
                            self.id.canonical_point(&ui.primary).unwrap(),
                            Some(10.0),
                            Some(self.id.clone()),
                            &mut ui.primary,
                        ))),
                    );
                } else if action == "follow agent" {
                    maybe_speed.unwrap().resume_realtime(ctx);
                    return (false, None);
                } else {
                    ui.primary.current_selection = Some(self.id.clone());
                    return (true, Some(Transition::ApplyObjectAction(action)));
                }
            }
            None => (false, None),
        }
    }

    pub fn draw(&self, g: &mut GfxCtx) {
        self.composite.draw(g);
        if let Some((ref unzoomed, ref zoomed)) = self.trip_details {
            if g.canvas.cam_zoom < MIN_ZOOM_FOR_DETAIL {
                g.redraw(unzoomed);
            } else {
                g.redraw(zoomed);
            }
        }
        g.redraw(&self.also_draw);
    }
}

fn info_for(id: ID, ctx: &EventCtx, ui: &UI) -> Vec<ManagedWidget> {
    let (map, sim, draw_map) = (&ui.primary.map, &ui.primary.sim, &ui.primary.draw_map);
    let name_color = ui.cs.get("OSD name color");
    let header_btns = ManagedWidget::row(vec![
        ManagedWidget::btn(Button::rectangle_svg(
            "../data/system/assets/tools/locate.svg",
            "jump to object",
            hotkey(Key::J),
            RewriteColor::Change(Color::hex("#CC4121"), colors::HOVERING),
            ctx,
        )),
        WrappedComposite::text_button(ctx, "X", hotkey(Key::Escape)),
    ])
    .align_right();

    let mut rows = vec![];

    match id {
        ID::Road(_) => unreachable!(),
        ID::Lane(id) => {
            let l = map.get_l(id);
            let r = map.get_r(l.parent);

            // Header
            {
                let label = if l.is_sidewalk() { "Sidewalk" } else { "Lane" };
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line(label).roboto_bold())),
                    header_btns,
                ]));
                rows.push(ManagedWidget::draw_text(
                    ctx,
                    Text::from(Line(format!("@ {}", r.get_name()))),
                ));
            }

            // Properties
            {
                let mut kv = Vec::new();

                if !l.is_sidewalk() {
                    kv.push(("Type".to_string(), l.lane_type.describe().to_string()));
                }

                if l.is_parking() {
                    kv.push((
                        "Parking".to_string(),
                        format!("{} spots, parallel parking", l.number_parking_spots()),
                    ));
                } else {
                    kv.push(("Speed limit".to_string(), r.get_speed_limit().to_string()));
                }

                kv.push(("Length".to_string(), l.length().describe_rounded()));

                if ui.opts.dev {
                    kv.push(("Parent".to_string(), r.id.to_string()));

                    if l.is_driving() {
                        kv.push((
                            "Parking blackhole redirect".to_string(),
                            format!("{:?}", l.parking_blackhole),
                        ));
                    }

                    if let Some(types) = l.get_turn_restrictions(r) {
                        kv.push(("Turn restrictions".to_string(), format!("{:?}", types)));
                    }
                    for (restriction, to) in &r.turn_restrictions {
                        kv.push((
                            format!("Restriction from this road to {}", to),
                            format!("{:?}", restriction),
                        ));
                    }

                    for (k, v) in &r.osm_tags {
                        kv.push((k.to_string(), v.to_string()));
                    }
                }

                rows.extend(make_table(ctx, kv));
            }

            if !l.is_parking() {
                let mut txt = Text::from(Line(""));
                txt.add(Line("Throughput (entire road)").roboto_bold());
                txt.add(Line(format!(
                    "Since midnight: {} agents crossed",
                    prettyprint_usize(sim.get_analytics().thruput_stats.count_per_road.get(r.id))
                )));
                txt.add(Line(format!("In 20 minute buckets:")));
                rows.push(ManagedWidget::draw_text(ctx, txt));

                rows.push(
                    road_throughput(
                        ui.primary.map.get_l(id).parent,
                        Duration::minutes(20),
                        ctx,
                        ui,
                    )
                    .margin(10),
                );
            }
        }
        ID::Intersection(id) => {
            let i = map.get_i(id);

            // Header
            {
                let label = if i.is_border() {
                    "Border"
                } else {
                    "Intersection"
                };
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line(label).roboto_bold())),
                    header_btns,
                ]));
            }

            let mut txt = Text::from(Line("Connecting"));
            let mut road_names = BTreeSet::new();
            for r in &i.roads {
                road_names.insert(map.get_r(*r).get_name());
            }
            for r in road_names {
                // TODO The spacing is ignored, so use -
                txt.add(Line(format!("- {}", r)));
            }

            let cnt = sim.count_trips_involving_border(id);
            if cnt.nonzero() {
                txt.add(Line(""));
                for line in cnt.describe() {
                    txt.add(Line(line));
                }
            }

            txt.add(Line(""));
            txt.add(Line("Throughput").roboto_bold());
            txt.add(Line(format!(
                "Since midnight: {} agents crossed",
                prettyprint_usize(
                    sim.get_analytics()
                        .thruput_stats
                        .count_per_intersection
                        .get(id)
                )
            )));
            txt.add(Line(format!("In 20 minute buckets:")));
            rows.push(ManagedWidget::draw_text(ctx, txt));

            rows.push(intersection_throughput(id, Duration::minutes(20), ctx, ui).margin(10));

            if ui.primary.map.get_i(id).is_traffic_signal() {
                let mut txt = Text::from(Line(""));
                txt.add(Line("Delay").roboto_bold());
                txt.add(Line(format!("In 20 minute buckets:")));
                rows.push(ManagedWidget::draw_text(ctx, txt));

                rows.push(intersection_delay(id, Duration::minutes(20), ctx, ui).margin(10));
            }
        }
        ID::Turn(_) => unreachable!(),
        ID::Building(id) => {
            let b = map.get_b(id);

            // Header
            {
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line("Building").roboto_bold())),
                    header_btns,
                ]));
            }

            // Properties
            {
                let mut kv = Vec::new();

                kv.push(("Address".to_string(), b.just_address(map)));
                if let Some(name) = b.just_name() {
                    kv.push(("Name".to_string(), name.to_string()));
                }

                if let Some(ref p) = b.parking {
                    kv.push((
                        "Parking".to_string(),
                        format!("{} spots via {}", p.num_stalls, p.name),
                    ));
                } else {
                    kv.push(("Parking".to_string(), "None".to_string()));
                }

                if ui.opts.dev {
                    kv.push((
                        "Dist along sidewalk".to_string(),
                        b.front_path.sidewalk.dist_along().to_string(),
                    ));

                    for (k, v) in &b.osm_tags {
                        kv.push((k.to_string(), v.to_string()));
                    }
                }

                rows.extend(make_table(ctx, kv));
            }

            let mut txt = Text::new();
            let cnt = sim.count_trips_involving_bldg(id);
            if cnt.nonzero() {
                txt.add(Line(""));
                for line in cnt.describe() {
                    txt.add(Line(line));
                }
            }

            let cars = sim.get_parked_cars_by_owner(id);
            if !cars.is_empty() {
                txt.add(Line(""));
                txt.add(Line(format!(
                    "{} parked cars owned by this building",
                    cars.len()
                )));
                // TODO Jump to it or see status
                for p in cars {
                    txt.add(Line(format!("- {}", p.vehicle.id)));
                }
            }

            if !b.amenities.is_empty() {
                txt.add(Line(""));
                if b.amenities.len() > 1 {
                    txt.add(Line(format!("{} amenities:", b.amenities.len())));
                }
                for (name, amenity) in &b.amenities {
                    txt.add(Line(format!("- {} (a {})", name, amenity)));
                }
            }

            if !txt.is_empty() {
                rows.push(ManagedWidget::draw_text(ctx, txt))
            }
        }
        ID::Car(id) => {
            // Header
            {
                let label = match id.1 {
                    VehicleType::Car => "Car",
                    VehicleType::Bike => "Bike",
                    VehicleType::Bus => "Bus",
                };
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line(label).roboto_bold())),
                    header_btns,
                ]));
            }

            let (kv, extra) = sim.car_properties(id, map);
            rows.extend(make_table(ctx, kv));
            if !extra.is_empty() {
                let mut txt = Text::from(Line(""));
                for line in extra {
                    txt.add(Line(line));
                }
                rows.push(ManagedWidget::draw_text(ctx, txt));
            }
        }
        ID::Pedestrian(id) => {
            // Header
            {
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line("Pedestrian").roboto_bold())),
                    header_btns,
                ]));
            }

            let (kv, extra) = sim.ped_properties(id, map);
            rows.extend(make_table(ctx, kv));
            if !extra.is_empty() {
                let mut txt = Text::from(Line(""));
                for line in extra {
                    txt.add(Line(line));
                }
                rows.push(ManagedWidget::draw_text(ctx, txt));
            }
        }
        ID::PedCrowd(members) => {
            // Header
            {
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(
                        ctx,
                        Text::from(Line("Pedestrian crowd").roboto_bold()),
                    ),
                    header_btns,
                ]));
            }

            let mut txt = Text::new();
            txt.add(Line(format!("Crowd of {}", members.len())));
            rows.push(ManagedWidget::draw_text(ctx, txt))
        }
        ID::BusStop(id) => {
            // Header
            {
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line("Bus stop").roboto_bold())),
                    header_btns,
                ]));
            }

            let mut txt = Text::new();
            let all_arrivals = &sim.get_analytics().bus_arrivals;
            for r in map.get_routes_serving_stop(id) {
                txt.add_appended(vec![Line("- Route "), Line(&r.name).fg(name_color)]);
                let arrivals: Vec<(Time, CarID)> = all_arrivals
                    .iter()
                    .filter(|(_, _, route, stop)| r.id == *route && id == *stop)
                    .map(|(t, car, _, _)| (*t, *car))
                    .collect();
                if let Some((t, _)) = arrivals.last() {
                    // TODO Button to jump to the bus
                    txt.add(Line(format!("  Last bus arrived {} ago", sim.time() - *t)));
                } else {
                    txt.add(Line("  No arrivals yet"));
                }
                // TODO Kind of inefficient...
                if let Some(hgram) = sim
                    .get_analytics()
                    .bus_passenger_delays(sim.time(), r.id)
                    .remove(&id)
                {
                    txt.add(Line(format!("  Waiting: {}", hgram.describe())));
                }
            }
            rows.push(ManagedWidget::draw_text(ctx, txt))
        }
        ID::Area(id) => {
            // Header
            {
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line("Area").roboto_bold())),
                    header_btns,
                ]));
            }

            let a = map.get_a(id);
            let mut kv = Vec::new();
            for (k, v) in &a.osm_tags {
                kv.push((k.to_string(), v.to_string()));
            }
            rows.extend(make_table(ctx, kv));
        }
        ID::ExtraShape(id) => {
            // Header
            {
                rows.push(ManagedWidget::row(vec![
                    ManagedWidget::draw_text(
                        ctx,
                        Text::from(Line("Extra GIS shape").roboto_bold()),
                    ),
                    header_btns,
                ]));
            }

            let es = draw_map.get_es(id);
            let mut kv = Vec::new();
            for (k, v) in &es.attributes {
                kv.push((k.to_string(), v.to_string()));
            }
            rows.extend(make_table(ctx, kv));
        }
        // No info here, trip_details will be used
        ID::Trip(_) => {}
    };
    rows
}

fn make_table(ctx: &EventCtx, rows: Vec<(String, String)>) -> Vec<ManagedWidget> {
    rows.into_iter()
        .map(|(k, v)| {
            ManagedWidget::row(vec![
                ManagedWidget::draw_text(ctx, Text::from(Line(k).roboto_bold())),
                // TODO not quite...
                ManagedWidget::draw_text(ctx, Text::from(Line(v)))
                    .centered_vert()
                    .align_right(),
            ])
        })
        .collect()

    // Attempt two
    /*let mut keys = Text::new();
    let mut values = Text::new();
    for (k, v) in rows {
        keys.add(Line(k).roboto_bold());
        values.add(Line(v));
    }
    vec![ManagedWidget::row(vec![
        ManagedWidget::draw_text(ctx, keys),
        ManagedWidget::draw_text(ctx, values).centered_vert().bg(Color::GREEN),
    ])]*/
}

fn intersection_throughput(
    i: IntersectionID,
    bucket: Duration,
    ctx: &EventCtx,
    ui: &UI,
) -> ManagedWidget {
    Plot::new_usize(
        ui.primary
            .sim
            .get_analytics()
            .throughput_intersection(ui.primary.sim.time(), i, bucket)
            .into_iter()
            .map(|(m, pts)| Series {
                label: m.to_string(),
                color: color_for_mode(m, ui),
                pts,
            })
            .collect(),
        ctx,
    )
}

fn road_throughput(r: RoadID, bucket: Duration, ctx: &EventCtx, ui: &UI) -> ManagedWidget {
    Plot::new_usize(
        ui.primary
            .sim
            .get_analytics()
            .throughput_road(ui.primary.sim.time(), r, bucket)
            .into_iter()
            .map(|(m, pts)| Series {
                label: m.to_string(),
                color: color_for_mode(m, ui),
                pts,
            })
            .collect(),
        ctx,
    )
}

fn intersection_delay(
    i: IntersectionID,
    bucket: Duration,
    ctx: &EventCtx,
    ui: &UI,
) -> ManagedWidget {
    let mut series: Vec<(Statistic, Vec<(Time, Duration)>)> = Statistic::all()
        .into_iter()
        .map(|stat| (stat, Vec::new()))
        .collect();
    for (t, distrib) in ui
        .primary
        .sim
        .get_analytics()
        .intersection_delays_bucketized(ui.primary.sim.time(), i, bucket)
    {
        for (stat, pts) in series.iter_mut() {
            if distrib.count() == 0 {
                pts.push((t, Duration::ZERO));
            } else {
                pts.push((t, distrib.select(*stat)));
            }
        }
    }

    Plot::new_duration(
        series
            .into_iter()
            .enumerate()
            .map(|(idx, (stat, pts))| Series {
                label: stat.to_string(),
                color: rotating_color_map(idx),
                pts,
            })
            .collect(),
        ctx,
    )
}

fn color_for_mode(m: TripMode, ui: &UI) -> Color {
    match m {
        TripMode::Walk => ui.cs.get("unzoomed pedestrian"),
        TripMode::Bike => ui.cs.get("unzoomed bike"),
        TripMode::Transit => ui.cs.get("unzoomed bus"),
        TripMode::Drive => ui.cs.get("unzoomed car"),
    }
}

// (extra rows to display, unzoomed view, zoomed view)
fn trip_details(trip: TripID, ctx: &mut EventCtx, ui: &UI) -> (ManagedWidget, Drawable, Drawable) {
    let map = &ui.primary.map;
    let phases = ui.primary.sim.get_analytics().get_trip_phases(trip, map);
    let (trip_start, trip_end) = ui.primary.sim.trip_endpoints(trip);

    let mut col = vec![ManagedWidget::draw_text(ctx, {
        let mut txt = Text::from(Line(""));
        txt.add(Line("Trip timeline").roboto_bold());
        txt
    })];
    let mut unzoomed = GeomBatch::new();
    let mut zoomed = GeomBatch::new();

    // Start
    {
        let color = rotating_color_map(col.len() - 1);
        match trip_start {
            TripStart::Bldg(b) => {
                let bldg = map.get_b(b);
                col.push(ColorLegend::row(
                    ctx,
                    color,
                    format!(
                        "{}: leave {}",
                        phases[0].start_time.ampm_tostring(),
                        bldg.just_address(map)
                    ),
                ));
                unzoomed.push(color, bldg.polygon.clone());
                zoomed.push(color, bldg.polygon.clone());
            }
            TripStart::Border(i) => {
                let i = map.get_i(i);
                // TODO How to name the intersection succinctly?
                col.push(ColorLegend::row(
                    ctx,
                    color,
                    format!(
                        "{}: start at {}",
                        phases[0].start_time.ampm_tostring(),
                        i.id
                    ),
                ));
                unzoomed.push(color, i.polygon.clone());
                zoomed.push(color, i.polygon.clone());
            }
        };
    }

    let mut end_time = None;
    for p in phases {
        let color = rotating_color_map(col.len() - 1);
        col.push(ColorLegend::row(
            ctx,
            color,
            if let Some(t2) = p.end_time {
                format!("+{}: {}", t2 - p.start_time, p.description)
            } else {
                format!("ongoing: {}", p.description)
            },
        ));

        // TODO Could really cache this between live updates
        if let Some((dist, ref path)) = p.path {
            if let Some(trace) = path.trace(map, dist, None) {
                unzoomed.push(color, trace.make_polygons(Distance::meters(10.0)));
                zoomed.extend(
                    ui.cs.get_def("route", Color::ORANGE.alpha(0.5)),
                    dashed_lines(
                        &trace,
                        Distance::meters(0.75),
                        Distance::meters(1.0),
                        Distance::meters(0.4),
                    ),
                );
            }
        }
        end_time = p.end_time;
    }

    // End
    {
        let color = rotating_color_map(col.len() - 1);
        let time = if let Some(t) = end_time {
            format!("{}: ", t.ampm_tostring())
        } else {
            String::new()
        };
        match trip_end {
            TripEnd::Bldg(b) => {
                let bldg = map.get_b(b);
                col.push(ColorLegend::row(
                    ctx,
                    color,
                    format!("{}end at {}", time, bldg.just_address(map)),
                ));
                unzoomed.push(color, bldg.polygon.clone());
                zoomed.push(color, bldg.polygon.clone());
            }
            TripEnd::Border(i) => {
                let i = map.get_i(i);
                // TODO name it better
                col.push(ColorLegend::row(
                    ctx,
                    color,
                    format!("{}end at {}", time, i.id),
                ));
                unzoomed.push(color, i.polygon.clone());
                zoomed.push(color, i.polygon.clone());
            }
            TripEnd::ServeBusRoute(_) => unreachable!(),
        };
    }

    (
        ManagedWidget::col(col),
        unzoomed.upload(ctx),
        zoomed.upload(ctx),
    )
}
