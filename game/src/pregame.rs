use crate::abtest::setup::PickABTest;
use crate::challenges::challenges_picker;
use crate::colors;
use crate::game::{State, Transition};
use crate::managed::{Callback, ManagedGUIState, WrappedComposite, WrappedOutcome};
use crate::mission::MissionEditMode;
use crate::sandbox::{GameplayMode, SandboxMode};
use crate::ui::UI;
use ezgui::{
    hotkey, Button, Color, Composite, EventCtx, EventLoopMode, GfxCtx, JustDraw, Key, Line,
    ManagedWidget, Text,
};
use geom::{Duration, Line, Pt2D, Speed};
use instant::Instant;
use map_model::{Map, MapEdits};
use rand::Rng;
use rand_xorshift::XorShiftRng;

pub struct TitleScreen {
    composite: WrappedComposite,
    screensaver: Screensaver,
    rng: XorShiftRng,
}

impl TitleScreen {
    pub fn new(ctx: &mut EventCtx, ui: &UI) -> TitleScreen {
        let mut rng = ui.primary.current_flags.sim_flags.make_rng();
        TitleScreen {
            composite: WrappedComposite::new(
                Composite::new(
                    ManagedWidget::col(vec![
                        JustDraw::image(ctx, "../data/system/assets/pregame/logo.png")
                            .bg(Color::GREEN.alpha(0.2)),
                        // TODO that nicer font
                        // TODO Any key
                        ManagedWidget::btn(Button::text_bg(
                            Text::from(Line("PLAY")),
                            Color::BLUE,
                            colors::HOVERING,
                            hotkey(Key::Space),
                            "start game",
                            ctx,
                        )),
                    ])
                    .centered(),
                )
                .build(ctx),
            )
            .cb(
                "start game",
                Box::new(|ctx, ui| Some(Transition::Replace(main_menu(ctx, ui)))),
            ),
            screensaver: Screensaver::start_bounce(&mut rng, ctx, &ui.primary.map),
            rng,
        }
    }
}

impl State for TitleScreen {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Transition {
        match self.composite.event(ctx, ui) {
            Some(WrappedOutcome::Transition(t)) => t,
            Some(WrappedOutcome::Clicked(_)) => unreachable!(),
            None => {
                self.screensaver.update(&mut self.rng, ctx, &ui.primary.map);
                Transition::KeepWithMode(EventLoopMode::Animation)
            }
        }
    }

    fn draw(&self, g: &mut GfxCtx, _: &UI) {
        self.composite.draw(g);
    }
}

pub fn main_menu(ctx: &mut EventCtx, ui: &UI) -> Box<dyn State> {
    let mut col = vec![
        WrappedComposite::svg_button(
            ctx,
            "../data/system/assets/pregame/quit.svg",
            "quit",
            hotkey(Key::Escape),
        )
        .align_left(),
        {
            let mut txt = Text::from(Line("A/B STREET").size(100));
            txt.add(Line("Created by Dustin Carlino"));
            ManagedWidget::draw_text(ctx, txt).centered_horiz()
        },
        ManagedWidget::row(vec![
            WrappedComposite::svg_button(
                ctx,
                "../data/system/assets/pregame/tutorial.svg",
                "Tutorial",
                hotkey(Key::T),
            ),
            WrappedComposite::svg_button(
                ctx,
                "../data/system/assets/pregame/sandbox.svg",
                "Sandbox mode",
                hotkey(Key::S),
            ),
            ManagedWidget::btn(Button::rectangle_img(
                "../data/system/assets/pregame/challenges.png",
                hotkey(Key::C),
                ctx,
                "Challenges",
            )),
            WrappedComposite::text_bg_button(ctx, "COMMUNITY PROPOSALS", hotkey(Key::P)),
        ])
        .centered(),
    ];
    if ui.opts.dev {
        col.push(
            ManagedWidget::row(vec![
                WrappedComposite::text_bg_button(ctx, "INTERNAL DEV TOOLS", hotkey(Key::M)),
                WrappedComposite::text_bg_button(ctx, "INTERNAL A/B TEST MODE", hotkey(Key::A)),
            ])
            .centered(),
        );
    }
    col.push(
        ManagedWidget::col(vec![
            WrappedComposite::text_bg_button(ctx, "About A/B Street", None),
            ManagedWidget::draw_text(ctx, built_info::time()),
        ])
        .centered(),
    );

    let mut c = WrappedComposite::new(
        Composite::new(ManagedWidget::col(col).evenly_spaced())
            .exact_size_percent(90, 90)
            .build(ctx),
    )
    .cb(
        "quit",
        Box::new(|_, _| {
            // TODO before_quit?
            std::process::exit(0);
        }),
    )
    .cb(
        "Tutorial",
        Box::new(|ctx, ui| {
            if ui.primary.map.get_name() != "montlake" {
                ui.switch_map(ctx, abstutil::path_map("montlake"));
            }

            Some(Transition::Push(Box::new(SandboxMode::new(
                ctx,
                ui,
                GameplayMode::Tutorial(
                    ui.session
                        .tutorial
                        .as_ref()
                        .map(|tut| tut.current)
                        .unwrap_or(0),
                ),
            ))))
        }),
    )
    .cb(
        "Sandbox mode",
        Box::new(|ctx, ui| {
            // We might've left tutorial mode with a synthetic map loaded.
            if !abstutil::list_all_objects(abstutil::path_all_maps())
                .contains(ui.primary.map.get_name())
            {
                ui.switch_map(ctx, abstutil::path_map("montlake"));
            }

            Some(Transition::Push(Box::new(SandboxMode::new(
                ctx,
                ui,
                GameplayMode::PlayScenario("random".to_string()),
            ))))
        }),
    )
    .cb(
        "Challenges",
        Box::new(|ctx, ui| Some(Transition::Push(challenges_picker(ctx, ui)))),
    )
    .cb(
        "About A/B Street",
        Box::new(|ctx, _| Some(Transition::Push(about(ctx)))),
    )
    .cb(
        "COMMUNITY PROPOSALS",
        Box::new(|ctx, _| Some(Transition::Push(proposals_picker(ctx)))),
    );
    if ui.opts.dev {
        c = c
            .cb(
                "INTERNAL DEV TOOLS",
                Box::new(|ctx, _| Some(Transition::Push(Box::new(MissionEditMode::new(ctx))))),
            )
            .cb(
                "INTERNAL A/B TEST MODE",
                Box::new(|_, _| Some(Transition::Push(PickABTest::new()))),
            );
    }
    ManagedGUIState::fullscreen(c)
}

fn about(ctx: &mut EventCtx) -> Box<dyn State> {
    let mut col = Vec::new();

    col.push(
        WrappedComposite::svg_button(
            ctx,
            "../data/system/assets/pregame/back.svg",
            "back",
            hotkey(Key::Escape),
        )
        .align_left(),
    );

    let mut txt = Text::new();
    txt.add(Line("A/B STREET").size(50));
    txt.add(Line("Created by Dustin Carlino, UX by Yuwen Li"));
    txt.add(Line(""));
    txt.add(Line("Contact: dabreegster@gmail.com"));
    txt.add(Line(
        "Project: http://github.com/dabreegster/abstreet (aliased by abstreet.org)",
    ));
    txt.add(Line("Map data from OpenStreetMap and King County GIS"));
    // TODO Add more here
    txt.add(Line(
        "See full credits at https://github.com/dabreegster/abstreet#credits",
    ));
    txt.add(Line(""));
    // TODO Word wrapping please?
    txt.add(Line(
        "Disclaimer: This game is based on imperfect data, heuristics ",
    ));
    txt.add(Line(
        "concocted under the influence of cold brew, a simplified traffic ",
    ));
    txt.add(Line(
        "simulation model, and a deeply flawed understanding of how much ",
    ));
    txt.add(Line(
        "articulated buses can bend around tight corners. Use this as a ",
    ));
    txt.add(Line(
        "conversation starter with your city government, not a final ",
    ));
    txt.add(Line(
        "decision maker. Any resemblance of in-game characters to real ",
    ));
    txt.add(Line(
        "people is probably coincidental, except for PedestrianID(42). ",
    ));
    txt.add(Line("Have the appropriate amount of fun."));
    col.push(
        ManagedWidget::draw_text(ctx, txt)
            .centered_horiz()
            .align_vert_center(),
    );

    ManagedGUIState::fullscreen(
        WrappedComposite::new(
            Composite::new(ManagedWidget::col(col))
                .exact_size_percent(90, 90)
                .build(ctx),
        )
        .cb("back", Box::new(|_, _| Some(Transition::Pop))),
    )
}

fn proposals_picker(ctx: &mut EventCtx) -> Box<dyn State> {
    let mut cbs: Vec<(String, Callback)> = Vec::new();
    let mut buttons: Vec<ManagedWidget> = Vec::new();
    for map_name in abstutil::list_all_objects(abstutil::path_all_maps()) {
        for (_, edits) in
            abstutil::load_all_objects::<MapEdits>(abstutil::path_all_edits(&map_name))
        {
            if !edits.proposal_description.is_empty() {
                let mut txt = Text::new();
                for l in &edits.proposal_description {
                    txt.add(Line(l));
                }
                let path = abstutil::path_edits(&edits.map_name, &edits.edits_name);
                buttons.push(WrappedComposite::nice_text_button(ctx, txt, None, &path));
                cbs.push((
                    path,
                    Box::new(move |ctx, ui| {
                        if ui.primary.map.get_name() != &edits.map_name {
                            ui.switch_map(ctx, abstutil::path_map(&edits.map_name));
                        }
                        // TODO apply edits
                        Some(Transition::Push(Box::new(SandboxMode::new(
                            ctx,
                            ui,
                            GameplayMode::PlayScenario("weekday".to_string()),
                        ))))
                    }),
                ));
            }
        }
    }

    let mut c = WrappedComposite::new(
        Composite::new(
            ManagedWidget::col(vec![
                WrappedComposite::svg_button(
                    ctx,
                    "../data/system/assets/pregame/back.svg",
                    "back",
                    hotkey(Key::Escape),
                )
                .align_left(),
                {
                    let mut txt = Text::from(Line("A/B STREET").size(100));
                    txt.add(Line("PROPOSALS").size(50));
                    txt.add(Line(""));
                    txt.add(Line(
                        "These are proposed changes to Seattle made by community members.",
                    ));
                    txt.add(Line("Contact dabreegster@gmail.com to add your idea here!"));
                    ManagedWidget::draw_text(ctx, txt)
                        .centered_horiz()
                        .bg(colors::PANEL_BG)
                },
                ManagedWidget::row(buttons)
                    .flex_wrap(ctx, 80)
                    .bg(colors::PANEL_BG)
                    .padding(10),
            ])
            .evenly_spaced(),
        )
        .exact_size_percent(90, 90)
        .build(ctx),
    )
    .cb("back", Box::new(|_, _| Some(Transition::Pop)));
    for (name, cb) in cbs {
        c = c.cb(&name, cb);
    }
    ManagedGUIState::fullscreen(c)
}

const SPEED: Speed = Speed::const_meters_per_second(20.0);

struct Screensaver {
    line: Line,
    started: Instant,
}

impl Screensaver {
    fn start_bounce(rng: &mut XorShiftRng, ctx: &mut EventCtx, map: &Map) -> Screensaver {
        let at = ctx.canvas.center_to_map_pt();
        let bounds = map.get_bounds();
        // TODO Ideally bounce off the edge of the map
        let goto = Pt2D::new(
            rng.gen_range(0.0, bounds.max_x),
            rng.gen_range(0.0, bounds.max_y),
        );

        ctx.canvas.cam_zoom = 10.0;
        ctx.canvas.center_on_map_pt(at);

        Screensaver {
            line: Line::new(at, goto),
            started: Instant::now(),
        }
    }

    fn update(&mut self, rng: &mut XorShiftRng, ctx: &mut EventCtx, map: &Map) {
        if ctx.input.nonblocking_is_update_event().is_some() {
            ctx.input.use_update_event();
            let dist_along = Duration::realtime_elapsed(self.started) * SPEED;
            if dist_along < self.line.length() {
                ctx.canvas
                    .center_on_map_pt(self.line.dist_along(dist_along));
            } else {
                *self = Screensaver::start_bounce(rng, ctx, map)
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(unused)]
mod built_info {
    use ezgui::{Color, Line, Text};

    include!(concat!(env!("OUT_DIR"), "/built.rs"));

    pub fn time() -> Text {
        let t = built::util::strptime(BUILT_TIME_UTC);

        let mut txt = Text::from(Line(format!("Built on {}", t.date().naive_local())));
        // Releases every Sunday
        if (chrono::Utc::now() - t).num_days() > 8 {
            txt.append(Line(format!(" (get the new release from abstreet.org)")).fg(Color::RED));
        }
        txt
    }
}

#[cfg(target_arch = "wasm32")]
mod built_info {
    pub fn time() -> ezgui::Text {
        ezgui::Text::new()
    }
}
