mod dashboards;
mod gameplay;
mod speed;

use crate::colors;
use crate::common::{tool_panel, CommonState, Minimap, Overlays, ShowBusRoute};
use crate::debug::DebugMode;
use crate::edit::{
    apply_map_edits, can_edit_lane, save_edits_as, EditMode, LaneEditor, StopSignEditor,
    TrafficSignalEditor,
};
use crate::game::{DrawBaselayer, State, Transition, WizardState};
use crate::helpers::ID;
use crate::managed::{WrappedComposite, WrappedOutcome};
use crate::pregame::main_menu;
use crate::render::DrawOptions;
use crate::sandbox::gameplay::Tutorial;
pub use crate::sandbox::gameplay::TutorialState;
use crate::ui::{ShowEverything, UI};
use ezgui::{
    hotkey, lctrl, Choice, Composite, EventCtx, EventLoopMode, GfxCtx, HorizontalAlignment, Key,
    Line, ManagedWidget, Outcome, Text, VerticalAlignment, Wizard,
};
pub use gameplay::spawner::spawn_agents_around;
pub use gameplay::GameplayMode;
use geom::Time;
use map_model::MapEdits;
use sim::TripMode;
pub use speed::{SpeedControls, TimePanel};

pub struct SandboxMode {
    gameplay: Box<dyn gameplay::GameplayState>,
    gameplay_mode: GameplayMode,

    pub common: Option<CommonState>,
    tool_panel: Option<WrappedComposite>,
    time_panel: Option<TimePanel>,
    speed: Option<SpeedControls>,
    agent_meter: Option<AgentMeter>,
    minimap: Option<Minimap>,
}

impl SandboxMode {
    pub fn new(ctx: &mut EventCtx, ui: &mut UI, mode: GameplayMode) -> SandboxMode {
        let gameplay = mode.initialize(ui, ctx);

        SandboxMode {
            common: if gameplay.has_common() {
                Some(CommonState::new())
            } else {
                None
            },
            tool_panel: if gameplay.has_tool_panel() {
                Some(tool_panel(ctx))
            } else {
                None
            },
            time_panel: if gameplay.has_time_panel() {
                Some(TimePanel::new(ctx, ui))
            } else {
                None
            },
            speed: if gameplay.has_speed() {
                Some(SpeedControls::new(ctx))
            } else {
                None
            },
            agent_meter: if gameplay.has_agent_meter() {
                Some(AgentMeter::new(ctx, ui))
            } else {
                None
            },
            minimap: if gameplay.has_minimap() {
                Some(Minimap::new(ctx, ui))
            } else {
                None
            },
            gameplay,
            gameplay_mode: mode,
        }
    }

    fn examine_objects(&self, ctx: &mut EventCtx, ui: &mut UI) -> Option<Transition> {
        if ui.opts.dev && ctx.input.new_was_pressed(lctrl(Key::D).unwrap()) {
            return Some(Transition::Push(Box::new(DebugMode::new(ctx))));
        }

        if let Some(ID::Building(b)) = ui.primary.current_selection {
            let cars = ui
                .primary
                .sim
                .get_offstreet_parked_cars(b)
                .into_iter()
                .map(|p| p.vehicle.id)
                .collect::<Vec<_>>();
            if !cars.is_empty()
                && ui.per_obj.action(
                    ctx,
                    Key::P,
                    format!("examine {} cars parked here", cars.len()),
                )
            {
                return Some(Transition::Push(WizardState::new(Box::new(
                    move |wiz, ctx, _| {
                        let _id = wiz.wrap(ctx).choose("Examine which car?", || {
                            cars.iter()
                                .map(|c| Choice::new(c.to_string(), *c))
                                .collect()
                        })?;
                        Some(Transition::Pop)
                    },
                ))));
            }
        }
        if let Some(ID::Intersection(i)) = ui.primary.current_selection {
            if ui.primary.map.get_i(i).is_traffic_signal()
                && ui.per_obj.action(ctx, Key::C, "show current demand")
            {
                ui.overlay = Overlays::intersection_demand(i, ctx, ui);
            }

            if ui.primary.map.get_i(i).is_traffic_signal()
                && ui.per_obj.action(ctx, Key::E, "edit traffic signal")
            {
                let edit = EditMode::new(ctx, ui, self.gameplay_mode.clone());
                let sim_copy = edit.suspended_sim.clone();
                return Some(Transition::PushTwice(
                    Box::new(edit),
                    Box::new(TrafficSignalEditor::new(i, ctx, ui, sim_copy)),
                ));
            }
            if ui.primary.map.get_i(i).is_stop_sign()
                && ui.per_obj.action(ctx, Key::E, "edit stop sign")
            {
                return Some(Transition::PushTwice(
                    Box::new(EditMode::new(ctx, ui, self.gameplay_mode.clone())),
                    Box::new(StopSignEditor::new(i, ctx, ui)),
                ));
            }
        }
        if let Some(ID::Lane(l)) = ui.primary.current_selection {
            if can_edit_lane(&self.gameplay_mode, l, ui)
                && ui.per_obj.action(ctx, Key::E, "edit lane")
            {
                return Some(Transition::PushTwice(
                    Box::new(EditMode::new(ctx, ui, self.gameplay_mode.clone())),
                    Box::new(LaneEditor::new(l, ctx, ui)),
                ));
            }
        }
        if let Some(ID::BusStop(bs)) = ui.primary.current_selection {
            let routes = ui.primary.map.get_routes_serving_stop(bs);
            if ui.per_obj.action(ctx, Key::E, "explore bus route") {
                return Some(Transition::Push(ShowBusRoute::make_route_picker(
                    routes.into_iter().map(|r| r.id).collect(),
                    true,
                )));
            }
        }
        if let Some(ID::Car(c)) = ui.primary.current_selection {
            if let Some(r) = ui.primary.sim.bus_route_id(c) {
                if ui.per_obj.action(ctx, Key::E, "explore bus route") {
                    return Some(Transition::Push(ShowBusRoute::make_route_picker(
                        vec![r],
                        true,
                    )));
                }
            }
        }

        None
    }
}

impl State for SandboxMode {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Transition {
        // Do this before gameplay
        ctx.canvas_movement();

        if let Some(t) = self.gameplay.event(ctx, ui) {
            return t;
        }
        // Sad hack. :(
        if let Some(ref mut tut) = self.gameplay.downcast_mut::<Tutorial>() {
            let (maybe_t, exit) = tut.event_with_speed(ctx, ui, self.speed.as_mut());
            if let Some(t) = maybe_t {
                return t;
            }
            if exit {
                return Transition::Push(WizardState::new(Box::new(exit_sandbox)));
            }
        }

        if ctx.redo_mouseover() {
            ui.recalculate_current_selection(ctx);
        }

        // Order here is pretty arbitrary
        if let Some(ref mut m) = self.minimap {
            if let Some(t) = m.event(ui, ctx) {
                return t;
            }
            if let Some(t) = Overlays::update(ctx, ui, &m.composite) {
                return t;
            }
        }

        if self.gameplay.can_examine_objects() {
            if let Some(t) = self.examine_objects(ctx, ui) {
                return t;
            }
        }

        if let Some(ref mut tp) = self.time_panel {
            tp.event(ctx, ui);
        }

        if let Some(ref mut s) = self.speed {
            match s.event(ctx, ui) {
                Some(WrappedOutcome::Transition(t)) => {
                    return t;
                }
                Some(WrappedOutcome::Clicked(x)) => match x {
                    x if x == "reset to midnight" => {
                        ui.primary.clear_sim();
                        return Transition::Replace(Box::new(SandboxMode::new(
                            ctx,
                            ui,
                            self.gameplay_mode.clone(),
                        )));
                    }
                    _ => unreachable!(),
                },
                None => {}
            }
        }

        if let Some(ref mut tp) = self.tool_panel {
            match tp.event(ctx, ui) {
                Some(WrappedOutcome::Transition(t)) => {
                    return t;
                }
                Some(WrappedOutcome::Clicked(x)) => match x.as_ref() {
                    "back" => {
                        return Transition::Push(WizardState::new(Box::new(exit_sandbox)));
                    }
                    _ => unreachable!(),
                },
                None => {}
            }
        }
        if let Some(ref mut am) = self.agent_meter {
            if let Some(t) = am.event(ctx, ui) {
                return t;
            }
        }

        if let Some(ref mut c) = self.common {
            if let Some(t) = c.event(ctx, ui, self.speed.as_mut()) {
                return t;
            }
        }

        if self.speed.as_ref().map(|s| s.is_paused()).unwrap_or(true) {
            Transition::Keep
        } else {
            Transition::KeepWithMode(EventLoopMode::Animation)
        }
    }

    fn draw_baselayer(&self) -> DrawBaselayer {
        DrawBaselayer::Custom
    }

    fn draw(&self, g: &mut GfxCtx, ui: &UI) {
        ui.draw(
            g,
            self.common
                .as_ref()
                .map(|c| c.draw_options(ui))
                .unwrap_or_else(DrawOptions::new),
            &ui.primary.sim,
            &ShowEverything::new(),
        );
        ui.overlay.draw(g);

        if let Some(ref c) = self.common {
            c.draw(g, ui);
        } else {
            CommonState::draw_osd(g, ui, &None);
        }
        if let Some(ref tp) = self.tool_panel {
            tp.draw(g);
        }
        if let Some(ref s) = self.speed {
            s.draw(g);
        }
        if let Some(ref tp) = self.time_panel {
            tp.draw(g);
        }
        if let Some(ref am) = self.agent_meter {
            am.draw(g);
        }
        if let Some(ref m) = self.minimap {
            m.draw(g, ui);
        }

        self.gameplay.draw(g, ui);
    }

    fn on_suspend(&mut self, ctx: &mut EventCtx, _: &mut UI) {
        if let Some(ref mut s) = self.speed {
            s.pause(ctx);
        }
    }

    fn on_destroy(&mut self, _: &mut EventCtx, ui: &mut UI) {
        ui.overlay = Overlays::Inactive;
    }
}

fn exit_sandbox(wiz: &mut Wizard, ctx: &mut EventCtx, ui: &mut UI) -> Option<Transition> {
    let mut wizard = wiz.wrap(ctx);
    let unsaved = ui.primary.map.get_edits().edits_name == "untitled edits"
        && !ui.primary.map.get_edits().commands.is_empty();
    let (resp, _) = wizard.choose("Sure you want to abandon the current challenge?", || {
        let mut choices = Vec::new();
        choices.push(Choice::new("keep playing", ()));
        if unsaved {
            choices.push(Choice::new("save edits and quit", ()));
        }
        choices.push(Choice::new("quit challenge", ()).key(Key::Q));
        choices
    })?;
    if resp == "keep playing" {
        return Some(Transition::Pop);
    }
    let map_name = ui.primary.map.get_name().to_string();
    if resp == "save edits and quit" {
        save_edits_as(&mut wizard, ui)?;
    }
    ctx.loading_screen("reset map and sim", |ctx, mut timer| {
        if ui.primary.map.get_edits().edits_name != "untitled edits"
            || !ui.primary.map.get_edits().commands.is_empty()
        {
            apply_map_edits(ctx, ui, MapEdits::new(map_name));
            ui.primary
                .map
                .recalculate_pathfinding_after_edits(&mut timer);
        }
        ui.primary.clear_sim();
        ui.set_prebaked(None);
    });
    ctx.canvas.save_camera_state(ui.primary.map.get_name());
    Some(Transition::Clear(vec![main_menu(ctx, ui)]))
}

pub struct AgentMeter {
    time: Time,
    pub composite: Composite,
}

impl AgentMeter {
    pub fn new(ctx: &mut EventCtx, ui: &UI) -> AgentMeter {
        let (finished, unfinished, by_mode) = ui.primary.sim.num_trips();

        let composite = Composite::new(
            ManagedWidget::col(vec![
                ManagedWidget::row(vec![
                    ManagedWidget::draw_svg(ctx, "../data/system/assets/meters/pedestrian.svg"),
                    ManagedWidget::draw_text(ctx, Text::from(Line(&by_mode[&TripMode::Walk]))),
                    ManagedWidget::draw_svg(ctx, "../data/system/assets/meters/bike.svg"),
                    ManagedWidget::draw_text(ctx, Text::from(Line(&by_mode[&TripMode::Bike]))),
                    ManagedWidget::draw_svg(ctx, "../data/system/assets/meters/car.svg"),
                    ManagedWidget::draw_text(ctx, Text::from(Line(&by_mode[&TripMode::Drive]))),
                    ManagedWidget::draw_svg(ctx, "../data/system/assets/meters/bus.svg"),
                    ManagedWidget::draw_text(ctx, Text::from(Line(&by_mode[&TripMode::Transit]))),
                ])
                .centered(),
                {
                    let mut txt = Text::new();
                    txt.add(Line(format!("Finished trips: {}", finished)));
                    txt.add(Line(format!("Unfinished trips: {}", unfinished)));
                    ManagedWidget::draw_text(ctx, txt)
                },
                // TODO The SVG button uses clip and doesn't seem to work
                WrappedComposite::text_button(ctx, "finished trip data", hotkey(Key::Q)),
            ])
            .bg(colors::PANEL_BG)
            .padding(20),
        )
        .aligned(HorizontalAlignment::Right, VerticalAlignment::Top)
        .build(ctx);

        AgentMeter {
            time: ui.primary.sim.time(),
            composite,
        }
    }

    pub fn event(&mut self, ctx: &mut EventCtx, ui: &UI) -> Option<Transition> {
        if self.time != ui.primary.sim.time() {
            *self = AgentMeter::new(ctx, ui);
            return self.event(ctx, ui);
        }
        match self.composite.event(ctx) {
            Some(Outcome::Clicked(x)) => match x.as_ref() {
                "finished trip data" => {
                    return Some(Transition::Push(dashboards::make(
                        ctx,
                        ui,
                        dashboards::Tab::FinishedTripsSummary,
                    )));
                }
                _ => unreachable!(),
            },
            None => {}
        }

        None
    }

    pub fn draw(&self, g: &mut GfxCtx) {
        self.composite.draw(g);
    }
}
