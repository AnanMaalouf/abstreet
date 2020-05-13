use crate::app::App;
use crate::game::{State, Transition};
use crate::helpers::{cmp_duration_shorter, color_for_mode};
use crate::info::Tab;
use crate::sandbox::dashboards::DashTab;
use crate::sandbox::SandboxMode;
use abstutil::prettyprint_usize;
use ezgui::{
    Btn, Checkbox, Composite, EventCtx, GeomBatch, GfxCtx, Line, Outcome, Text, TextExt, Widget,
};
use geom::{Duration, Polygon, Time};
use maplit::btreemap;
use sim::{TripEndpoint, TripID, TripMode};
use std::collections::BTreeSet;

const ROWS: usize = 20;

// TODO Hover over a trip to preview its route on the map

pub struct TripTable {
    composite: Composite,
    opts: Options,
}

struct Options {
    sort_by: SortBy,
    descending: bool,
    modes: BTreeSet<TripMode>,
    off_map_starts: bool,
    off_map_ends: bool,
    skip: usize,
}

impl Options {
    fn change(&mut self, value: SortBy) {
        self.skip = 0;
        if self.sort_by == value {
            self.descending = !self.descending;
        } else {
            self.sort_by = value;
            self.descending = true;
        }
    }
}

// TODO Is there a heterogenously typed table crate somewhere?
#[derive(Clone, Copy, PartialEq)]
enum SortBy {
    Departure,
    Duration,
    RelativeDuration,
    PercentChangeDuration,
    Waiting,
    PercentWaiting,
}

impl TripTable {
    pub fn new(ctx: &mut EventCtx, app: &App) -> Box<dyn State> {
        let opts = Options {
            sort_by: SortBy::PercentWaiting,
            descending: true,
            modes: TripMode::all().into_iter().collect(),
            off_map_starts: true,
            off_map_ends: true,
            skip: 0,
        };
        Box::new(TripTable {
            composite: make(ctx, app, &opts),
            opts,
        })
    }

    fn recalc(&mut self, ctx: &mut EventCtx, app: &App) {
        let mut new = make(ctx, app, &self.opts);
        new.restore(ctx, &self.composite);
        self.composite = new;
    }
}

impl State for TripTable {
    fn event(&mut self, ctx: &mut EventCtx, app: &mut App) -> Transition {
        match self.composite.event(ctx) {
            Some(Outcome::Clicked(x)) => match x.as_ref() {
                "Departure" => {
                    self.opts.change(SortBy::Departure);
                    self.recalc(ctx, app);
                }
                "Duration" => {
                    self.opts.change(SortBy::Duration);
                    self.recalc(ctx, app);
                }
                "Comparison" => {
                    self.opts.change(SortBy::RelativeDuration);
                    self.recalc(ctx, app);
                }
                "Normalized" => {
                    self.opts.change(SortBy::PercentChangeDuration);
                    self.recalc(ctx, app);
                }
                "Time spent waiting" => {
                    self.opts.change(SortBy::Waiting);
                    self.recalc(ctx, app);
                }
                "Percent waiting" => {
                    self.opts.change(SortBy::PercentWaiting);
                    self.recalc(ctx, app);
                }
                "previous trips" => {
                    self.opts.skip -= ROWS;
                    self.recalc(ctx, app);
                }
                "next trips" => {
                    self.opts.skip += ROWS;
                    self.recalc(ctx, app);
                }
                x => {
                    if let Ok(idx) = x.parse::<usize>() {
                        let trip = TripID(idx);
                        let person = app.primary.sim.trip_to_person(trip);
                        return Transition::PopWithData(Box::new(move |state, app, ctx| {
                            let sandbox = state.downcast_mut::<SandboxMode>().unwrap();
                            let mut actions = sandbox.contextual_actions();
                            sandbox.controls.common.as_mut().unwrap().launch_info_panel(
                                ctx,
                                app,
                                Tab::PersonTrips(person, btreemap! { trip => true }),
                                &mut actions,
                            );
                        }));
                    }
                    return DashTab::TripTable.transition(ctx, app, x);
                }
            },
            None => {
                let mut modes = BTreeSet::new();
                for m in TripMode::all() {
                    if self.composite.is_checked(m.ongoing_verb()) {
                        modes.insert(m);
                    }
                }
                if modes != self.opts.modes {
                    self.opts.skip = 0;
                    self.opts.modes = modes;
                    self.recalc(ctx, app);
                }
                let off_map_starts = self.composite.is_checked("starting off-map");
                let off_map_ends = self.composite.is_checked("ending off-map");
                if self.opts.off_map_starts != off_map_starts
                    || self.opts.off_map_ends != off_map_ends
                {
                    self.opts.off_map_starts = off_map_starts;
                    self.opts.off_map_ends = off_map_ends;
                    self.opts.skip = 0;
                    self.recalc(ctx, app);
                }
            }
        };

        Transition::Keep
    }

    fn draw(&self, g: &mut GfxCtx, app: &App) {
        State::grey_out_map(g, app);
        self.composite.draw(g);
    }
}

struct Entry {
    trip: TripID,
    mode: TripMode,
    departure: Time,
    duration_after: Duration,
    duration_before: Duration,
    waiting: Duration,
    percent_waiting: usize,
}

fn make(ctx: &mut EventCtx, app: &App, opts: &Options) -> Composite {
    // Gather raw data
    let mut data = Vec::new();
    let sim = &app.primary.sim;
    let mut aborted = 0;
    for (_, id, maybe_mode, duration_after) in &sim.get_analytics().finished_trips {
        let mode = if let Some(m) = maybe_mode {
            if !opts.modes.contains(m) {
                continue;
            }
            *m
        } else {
            aborted += 1;
            continue;
        };
        let (_, start, end, _) = sim.trip_info(*id);
        if !opts.off_map_starts {
            if let TripEndpoint::Border(_, _) = start {
                continue;
            }
        }
        if !opts.off_map_ends {
            if let TripEndpoint::Border(_, _) = end {
                continue;
            }
        }

        let (_, waiting) = sim.finished_trip_time(*id).unwrap();
        let (departure, _, _, _) = sim.trip_info(*id);
        let duration_before = if app.has_prebaked().is_some() {
            if let Some(dt) = app.prebaked().finished_trip_time(*id) {
                dt
            } else {
                // Aborted
                aborted += 1;
                continue;
            }
        } else {
            Duration::ZERO
        };

        data.push(Entry {
            trip: *id,
            mode,
            departure,
            duration_after: *duration_after,
            duration_before,
            waiting,
            percent_waiting: (100.0 * waiting / *duration_after) as usize,
        });
    }

    // Sort
    match opts.sort_by {
        SortBy::Departure => data.sort_by_key(|x| x.departure),
        SortBy::Duration => data.sort_by_key(|x| x.duration_after),
        SortBy::RelativeDuration => data.sort_by_key(|x| x.duration_after - x.duration_before),
        SortBy::PercentChangeDuration => {
            data.sort_by_key(|x| (100.0 * (x.duration_after / x.duration_before)) as isize)
        }
        SortBy::Waiting => data.sort_by_key(|x| x.waiting),
        SortBy::PercentWaiting => data.sort_by_key(|x| x.percent_waiting),
    }
    if opts.descending {
        data.reverse();
    }
    let total_rows = data.len();

    // Render data
    let mut rows = Vec::new();
    for x in data.into_iter().skip(opts.skip).take(ROWS) {
        let mut row = vec![
            Text::from(Line(x.trip.0.to_string())).render_ctx(ctx),
            Text::from(Line(x.mode.ongoing_verb()).fg(color_for_mode(app, x.mode))).render_ctx(ctx),
            Text::from(Line(x.departure.ampm_tostring())).render_ctx(ctx),
            Text::from(Line(x.duration_after.to_string())).render_ctx(ctx),
        ];
        if app.has_prebaked().is_some() {
            row.push(
                Text::from_all(cmp_duration_shorter(x.duration_after, x.duration_before))
                    .render_ctx(ctx),
            );
            if x.duration_after == x.duration_before {
                row.push(Text::from(Line("same")).render_ctx(ctx));
            } else if x.duration_after < x.duration_before {
                row.push(
                    Text::from(Line(format!(
                        "{}% faster",
                        (100.0 * (1.0 - (x.duration_after / x.duration_before))) as usize
                    )))
                    .render_ctx(ctx),
                );
            } else {
                row.push(
                    Text::from(Line(format!(
                        "{}% slower ",
                        (100.0 * ((x.duration_after / x.duration_before) - 1.0)) as usize
                    )))
                    .render_ctx(ctx),
                );
            }
        }
        row.push(Text::from(Line(x.waiting.to_string())).render_ctx(ctx));
        row.push(Text::from(Line(format!("{}%", x.percent_waiting))).render_ctx(ctx));

        rows.push((x.trip.0.to_string(), row));
    }

    let btn = |value, name| {
        if opts.sort_by == value {
            Btn::text_bg2(format!(
                "{} {}",
                name,
                if opts.descending { "↓" } else { "↑" }
            ))
            .build(ctx, name, None)
        } else {
            Btn::text_bg2(name).build_def(ctx, None)
        }
    };
    let mut headers = vec![
        Line("Trip ID").draw(ctx),
        Line("Type").draw(ctx),
        btn(SortBy::Departure, "Departure"),
        btn(SortBy::Duration, "Duration"),
    ];
    if app.has_prebaked().is_some() {
        headers.push(btn(SortBy::RelativeDuration, "Comparison"));
        headers.push(btn(SortBy::PercentChangeDuration, "Normalized"));
    }
    headers.push(btn(SortBy::Waiting, "Time spent waiting"));
    headers.push(btn(SortBy::PercentWaiting, "Percent waiting"));

    let mut col = vec![DashTab::TripTable.picker(ctx)];
    let mut filters = Vec::new();
    for m in TripMode::all() {
        filters.push(
            Checkbox::colored(
                ctx,
                m.ongoing_verb(),
                color_for_mode(app, m),
                opts.modes.contains(&m),
            )
            .margin_right(5),
        );
        filters.push(m.ongoing_verb().draw_text(ctx).margin_right(10));
    }
    col.push(Widget::row(filters).margin_below(5));
    col.push(
        Widget::row(vec![
            Checkbox::text(ctx, "starting off-map", None, opts.off_map_starts).margin_right(10),
            Checkbox::text(ctx, "ending off-map", None, opts.off_map_ends),
        ])
        .margin_below(5),
    );
    col.push(
        format!(
            "{} trips aborted due to simulation glitch",
            prettyprint_usize(aborted)
        )
        .draw_text(ctx)
        .margin_below(5),
    );
    col.push(
        Widget::row(vec![
            if opts.skip > 0 {
                Btn::text_fg("<").build(ctx, "previous trips", None)
            } else {
                Btn::text_fg("<").inactive(ctx)
            }
            .margin_right(10),
            format!(
                "{}-{} of {}",
                if total_rows > 0 {
                    prettyprint_usize(opts.skip + 1)
                } else {
                    "0".to_string()
                },
                prettyprint_usize((opts.skip + 1 + ROWS).min(total_rows)),
                prettyprint_usize(total_rows)
            )
            .draw_text(ctx)
            .margin_right(10),
            if opts.skip + 1 + ROWS < total_rows {
                Btn::text_fg(">").build(ctx, "next trips", None)
            } else {
                Btn::text_fg(">").inactive(ctx)
            },
        ])
        .margin_below(5),
    );

    col.extend(make_table(
        ctx,
        app,
        headers,
        rows,
        0.88 * ctx.canvas.window_width,
    ));

    Composite::new(Widget::col(col).bg(app.cs.panel_bg).padding(10))
        .max_size_percent(90, 90)
        .build(ctx)
}

// TODO Figure out a nicer API to construct generic sortable tables.
pub fn make_table(
    ctx: &mut EventCtx,
    app: &App,
    headers: Vec<Widget>,
    rows: Vec<(String, Vec<GeomBatch>)>,
    total_width: f32,
) -> Vec<Widget> {
    let mut width_per_col: Vec<f32> = headers.iter().map(|w| w.get_width_for_forcing()).collect();
    for (_, row) in &rows {
        for (col, width) in row.iter().zip(width_per_col.iter_mut()) {
            *width = width.max(col.get_dims().width);
        }
    }
    let extra_margin = ((total_width - width_per_col.clone().into_iter().sum::<f32>())
        / (width_per_col.len() - 1) as f32)
        .max(0.0);

    let mut col = vec![Widget::row(
        headers
            .into_iter()
            .enumerate()
            .map(|(idx, w)| {
                let margin = extra_margin + width_per_col[idx] - w.get_width_for_forcing();
                if idx == width_per_col.len() - 1 {
                    w.margin_right((margin - extra_margin) as usize)
                } else {
                    w.margin_right(margin as usize)
                }
            })
            .collect(),
    )
    .bg(app.cs.section_bg)];

    for (label, row) in rows {
        let mut batch = GeomBatch::new();
        batch.autocrop_dims = false;
        let mut x1 = 0.0;
        for (col, width) in row.into_iter().zip(width_per_col.iter()) {
            batch.add_translated(col, x1, 0.0);
            x1 += *width + extra_margin;
        }

        let rect = Polygon::rectangle(total_width, batch.get_dims().height);
        let mut hovered = GeomBatch::new();
        hovered.push(app.cs.hovering, rect.clone());
        hovered.append(batch.clone());

        col.push(
            Btn::custom(batch, hovered, rect)
                .tooltip(Text::new())
                .build(ctx, label, None),
        );
    }

    col
}
