use crate::colors;
use crate::common::CommonState;
use crate::edit::{apply_map_edits, close_intersection};
use crate::game::{msg, DrawBaselayer, State, Transition, WizardState};
use crate::helpers::plain_list_names;
use crate::managed::{WrappedComposite, WrappedOutcome};
use crate::options::TrafficSignalStyle;
use crate::render::{draw_signal_phase, DrawOptions, DrawTurnGroup, BIG_ARROW_THICKNESS};
use crate::sandbox::{spawn_agents_around, SpeedControls, TimePanel};
use crate::ui::{ShowEverything, UI};
use abstutil::Timer;
use ezgui::{
    hotkey, lctrl, Button, Choice, Color, Composite, EventCtx, EventLoopMode, GeomBatch, GfxCtx,
    HorizontalAlignment, Key, Line, ManagedWidget, Outcome, RewriteColor, Text, VerticalAlignment,
};
use geom::{Distance, Duration, Polygon};
use map_model::{ControlTrafficSignal, EditCmd, IntersectionID, Phase, TurnGroupID, TurnPriority};
use sim::Sim;
use std::collections::BTreeSet;

// TODO Warn if there are empty phases or if some turn is completely absent from the signal.
pub struct TrafficSignalEditor {
    i: IntersectionID,
    current_phase: usize,
    composite: Composite,
    top_panel: Composite,

    groups: Vec<DrawTurnGroup>,
    group_selected: Option<TurnGroupID>,

    suspended_sim: Sim,
    // The first ControlTrafficSignal is the original
    command_stack: Vec<ControlTrafficSignal>,
    redo_stack: Vec<ControlTrafficSignal>,
}

impl TrafficSignalEditor {
    pub fn new(
        id: IntersectionID,
        ctx: &mut EventCtx,
        ui: &mut UI,
        suspended_sim: Sim,
    ) -> TrafficSignalEditor {
        ui.primary.current_selection = None;
        TrafficSignalEditor {
            i: id,
            current_phase: 0,
            composite: make_diagram(id, 0, ui, ctx),
            top_panel: make_top_panel(false, false, ctx),
            groups: DrawTurnGroup::for_i(id, &ui.primary.map),
            group_selected: None,
            suspended_sim,
            command_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    fn change_phase(&mut self, idx: usize, ui: &UI, ctx: &mut EventCtx) {
        if self.current_phase == idx {
            let preserve_scroll = self.composite.preserve_scroll();
            self.composite = make_diagram(self.i, self.current_phase, ui, ctx);
            self.composite.restore_scroll(ctx, preserve_scroll);
        } else {
            self.current_phase = idx;
            self.composite = make_diagram(self.i, self.current_phase, ui, ctx);
            // TODO Maybe center of previous member
            self.composite
                .scroll_to_member(ctx, format!("delete phase #{}", idx + 1));
        }
    }
}

impl State for TrafficSignalEditor {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Transition {
        let orig_signal = ui.primary.map.get_traffic_signal(self.i);

        ctx.canvas_movement();

        // TODO Buttons for these...
        if self.current_phase != 0 && ctx.input.new_was_pressed(hotkey(Key::UpArrow).unwrap()) {
            self.change_phase(self.current_phase - 1, ui, ctx);
        }

        if self.current_phase != ui.primary.map.get_traffic_signal(self.i).phases.len() - 1
            && ctx.input.new_was_pressed(hotkey(Key::DownArrow).unwrap())
        {
            self.change_phase(self.current_phase + 1, ui, ctx);
        }

        match self.composite.event(ctx) {
            Some(Outcome::Clicked(x)) => match x {
                x if x == "Edit offset" => {
                    return Transition::Push(change_offset(orig_signal.offset));
                }
                x if x == "Reset to default" => {
                    let new_signal =
                        ControlTrafficSignal::get_possible_policies(&ui.primary.map, self.i)
                            .remove(0)
                            .1;
                    self.command_stack.push(orig_signal.clone());
                    self.redo_stack.clear();
                    self.top_panel = make_top_panel(true, false, ctx);
                    change_traffic_signal(new_signal, ui, ctx);
                    // Don't use change_phase; it tries to preserve scroll
                    self.current_phase = 0;
                    self.composite = make_diagram(self.i, self.current_phase, ui, ctx);
                    return Transition::Keep;
                }
                x if x == "Use preset" => {
                    return Transition::Push(change_preset(self.i));
                }
                x if x == "Close intersection for construction" => {
                    return close_intersection(ctx, ui, self.i);
                }
                x if x == "Make all-walk" => {
                    let mut new_signal = orig_signal.clone();
                    if new_signal.convert_to_ped_scramble(&ui.primary.map) {
                        self.command_stack.push(orig_signal.clone());
                        self.redo_stack.clear();
                        self.top_panel = make_top_panel(true, false, ctx);
                        change_traffic_signal(new_signal, ui, ctx);
                        self.change_phase(0, ui, ctx);
                    }
                    return Transition::Keep;
                }
                x if x.starts_with("change duration of #") => {
                    let idx = x["change duration of #".len()..].parse::<usize>().unwrap() - 1;
                    return Transition::Push(change_phase_duration(
                        idx,
                        orig_signal.phases[idx].duration,
                    ));
                }
                x if x.starts_with("delete phase #") => {
                    let idx = x["delete phase #".len()..].parse::<usize>().unwrap() - 1;
                    let mut new_signal = orig_signal.clone();
                    new_signal.phases.remove(idx);
                    let num_phases = new_signal.phases.len();
                    self.command_stack.push(orig_signal.clone());
                    self.redo_stack.clear();
                    self.top_panel = make_top_panel(true, false, ctx);
                    change_traffic_signal(new_signal, ui, ctx);
                    // Don't use change_phase; it tries to preserve scroll
                    self.current_phase = if idx == num_phases { idx - 1 } else { idx };
                    self.composite = make_diagram(self.i, self.current_phase, ui, ctx);
                    return Transition::Keep;
                }
                x if x.starts_with("move up phase #") => {
                    let idx = x["move up phase #".len()..].parse::<usize>().unwrap() - 1;
                    let mut new_signal = orig_signal.clone();
                    new_signal.phases.swap(idx, idx - 1);
                    self.command_stack.push(orig_signal.clone());
                    self.redo_stack.clear();
                    self.top_panel = make_top_panel(true, false, ctx);
                    change_traffic_signal(new_signal, ui, ctx);
                    self.change_phase(idx - 1, ui, ctx);
                    return Transition::Keep;
                }
                x if x.starts_with("move down phase #") => {
                    let idx = x["move down phase #".len()..].parse::<usize>().unwrap() - 1;
                    let mut new_signal = orig_signal.clone();
                    new_signal.phases.swap(idx, idx + 1);
                    self.command_stack.push(orig_signal.clone());
                    self.redo_stack.clear();
                    self.top_panel = make_top_panel(true, false, ctx);
                    change_traffic_signal(new_signal, ui, ctx);
                    self.change_phase(idx + 1, ui, ctx);
                    return Transition::Keep;
                }
                x if x.starts_with("add new phase after #") => {
                    let idx = x["add new phase after #".len()..].parse::<usize>().unwrap() - 1;
                    let mut new_signal = orig_signal.clone();
                    new_signal.phases.insert(idx + 1, Phase::new());
                    self.command_stack.push(orig_signal.clone());
                    self.redo_stack.clear();
                    self.top_panel = make_top_panel(true, false, ctx);
                    change_traffic_signal(new_signal, ui, ctx);
                    self.change_phase(idx + 1, ui, ctx);
                    return Transition::Keep;
                }
                x if x.starts_with("phase ") => {
                    let idx = x["phase ".len()..].parse::<usize>().unwrap() - 1;
                    self.change_phase(idx, ui, ctx);
                }
                _ => unreachable!(),
            },
            None => {}
        }

        if ctx.redo_mouseover() {
            self.group_selected = None;
            if let Some(pt) = ctx.canvas.get_cursor_in_map_space() {
                for g in &self.groups {
                    if g.block.contains_pt(pt) {
                        self.group_selected = Some(g.id);
                        break;
                    }
                }
            }
        }

        if let Some(id) = self.group_selected {
            let mut new_signal = orig_signal.clone();
            let phase = &mut new_signal.phases[self.current_phase];
            // Just one key to toggle between the 3 states
            let next_priority = match phase.get_priority_of_group(id) {
                TurnPriority::Banned => {
                    if phase.could_be_protected(id, &orig_signal.turn_groups) {
                        Some(TurnPriority::Protected)
                    } else if id.crosswalk.is_some() {
                        None
                    } else {
                        Some(TurnPriority::Yield)
                    }
                }
                TurnPriority::Yield => Some(TurnPriority::Banned),
                TurnPriority::Protected => {
                    if id.crosswalk.is_some() {
                        Some(TurnPriority::Banned)
                    } else {
                        Some(TurnPriority::Yield)
                    }
                }
            };
            if let Some(pri) = next_priority {
                if ui.per_obj.left_click(
                    ctx,
                    format!(
                        "toggle from {:?} to {:?}",
                        phase.get_priority_of_group(id),
                        pri
                    ),
                ) {
                    phase.edit_group(
                        &orig_signal.turn_groups[&id],
                        pri,
                        &orig_signal.turn_groups,
                        &ui.primary.map,
                    );
                    self.command_stack.push(orig_signal.clone());
                    self.redo_stack.clear();
                    self.top_panel = make_top_panel(true, false, ctx);
                    change_traffic_signal(new_signal, ui, ctx);
                    self.change_phase(self.current_phase, ui, ctx);
                    return Transition::Keep;
                }
            }
        }

        match self.top_panel.event(ctx) {
            Some(Outcome::Clicked(x)) => match x.as_ref() {
                "Finish" => {
                    return check_for_missing_groups(
                        orig_signal.clone(),
                        &mut self.composite,
                        ui,
                        ctx,
                    );
                }
                "Preview" => {
                    // TODO These're expensive clones :(
                    return Transition::Push(make_previewer(
                        self.i,
                        self.current_phase,
                        self.suspended_sim.clone(),
                    ));
                }
                "undo" => {
                    self.redo_stack.push(orig_signal.clone());
                    change_traffic_signal(self.command_stack.pop().unwrap(), ui, ctx);
                    self.top_panel = make_top_panel(!self.command_stack.is_empty(), true, ctx);
                    self.change_phase(0, ui, ctx);
                    return Transition::Keep;
                }
                "redo" => {
                    self.command_stack.push(orig_signal.clone());
                    change_traffic_signal(self.redo_stack.pop().unwrap(), ui, ctx);
                    self.top_panel = make_top_panel(true, !self.redo_stack.is_empty(), ctx);
                    self.change_phase(0, ui, ctx);
                    return Transition::Keep;
                }
                _ => unreachable!(),
            },
            None => {}
        }

        Transition::Keep
    }

    fn draw_baselayer(&self) -> DrawBaselayer {
        DrawBaselayer::Custom
    }

    fn draw(&self, g: &mut GfxCtx, ui: &UI) {
        {
            let mut opts = DrawOptions::new();
            opts.suppress_traffic_signal_details = Some(self.i);
            ui.draw(g, opts, &ui.primary.sim, &ShowEverything::new());
        }

        let signal = ui.primary.map.get_traffic_signal(self.i);
        let phase = &signal.phases[self.current_phase];
        let mut batch = GeomBatch::new();
        draw_signal_phase(
            g.prerender,
            phase,
            self.i,
            None,
            &mut batch,
            ui,
            ui.opts.traffic_signal_style.clone(),
        );

        for g in &self.groups {
            if Some(g.id) == self.group_selected {
                batch.push(ui.cs.get_def("solid selected", Color::RED), g.block.clone());
                // Overwrite the original thing
                batch.push(
                    ui.cs.get("solid selected"),
                    signal.turn_groups[&g.id]
                        .geom
                        .make_arrow(BIG_ARROW_THICKNESS)
                        .unwrap(),
                );
            } else {
                batch.push(
                    ui.cs.get_def("turn block background", Color::grey(0.6)),
                    g.block.clone(),
                );
            }
            let arrow_color = match phase.get_priority_of_group(g.id) {
                TurnPriority::Protected => ui.cs.get("turn protected by traffic signal"),
                TurnPriority::Yield => ui
                    .cs
                    .get("turn that can yield by traffic signal")
                    .alpha(1.0),
                TurnPriority::Banned => ui.cs.get_def("turn not in current phase", Color::BLACK),
            };
            batch.push(arrow_color, g.arrow.clone());
        }
        batch.draw(g);

        self.composite.draw(g);
        self.top_panel.draw(g);
        if let Some(id) = self.group_selected {
            let osd = if id.crosswalk.is_some() {
                Text::from(Line(format!(
                    "Crosswalk across {}",
                    ui.primary.map.get_r(id.from).get_name()
                )))
            } else {
                Text::from(Line(format!(
                    "Turn from {} to {}",
                    ui.primary.map.get_r(id.from).get_name(),
                    ui.primary.map.get_r(id.to).get_name()
                )))
            };
            CommonState::draw_custom_osd(ui, g, osd);
        } else {
            CommonState::draw_osd(g, ui, &None);
        }
    }
}

fn make_top_panel(can_undo: bool, can_redo: bool, ctx: &mut EventCtx) -> Composite {
    let row = vec![
        WrappedComposite::text_button(ctx, "Finish", hotkey(Key::Escape)),
        WrappedComposite::text_button(ctx, "Preview", lctrl(Key::P)),
        (if can_undo {
            WrappedComposite::svg_button(
                ctx,
                "../data/system/assets/tools/undo.svg",
                "undo",
                lctrl(Key::Z),
            )
        } else {
            ManagedWidget::draw_svg_transform(
                ctx,
                "../data/system/assets/tools/undo.svg",
                RewriteColor::ChangeAll(Color::WHITE.alpha(0.5)),
            )
        })
        .margin(15),
        (if can_redo {
            WrappedComposite::svg_button(
                ctx,
                "../data/system/assets/tools/redo.svg",
                "redo",
                // TODO ctrl+shift+Z!
                lctrl(Key::Y),
            )
        } else {
            ManagedWidget::draw_svg_transform(
                ctx,
                "../data/system/assets/tools/redo.svg",
                RewriteColor::ChangeAll(Color::WHITE.alpha(0.5)),
            )
        })
        .margin(15),
    ];
    Composite::new(ManagedWidget::row(row).bg(colors::PANEL_BG))
        .aligned(HorizontalAlignment::Center, VerticalAlignment::Top)
        .build(ctx)
}

fn make_diagram(i: IntersectionID, selected: usize, ui: &UI, ctx: &mut EventCtx) -> Composite {
    // Slightly inaccurate -- the turn rendering may slightly exceed the intersection polygon --
    // but this is close enough.
    let bounds = ui.primary.map.get_i(i).polygon.get_bounds();
    // Pick a zoom so that we fit some percentage of the screen
    let zoom = 0.2 * ctx.canvas.window_width / bounds.width();
    let bbox = Polygon::rectangle(zoom * bounds.width(), zoom * bounds.height());

    let signal = ui.primary.map.get_traffic_signal(i);
    let mut col = vec![
        ManagedWidget::draw_text(ctx, {
            let mut txt = Text::new();
            let road_names = ui
                .primary
                .map
                .get_i(i)
                .roads
                .iter()
                .map(|r| ui.primary.map.get_r(*r).get_name())
                .collect::<BTreeSet<_>>();
            // TODO Style inside here. Also 0.4 is manually tuned and pretty wacky, because it
            // assumes default font.
            txt.add_wrapped(plain_list_names(road_names), 0.4 * ctx.canvas.window_width);
            txt.add(Line(""));
            txt.add(Line(format!("{} phases", signal.phases.len())));
            txt.add(Line(format!("Signal offset: {}", signal.offset)));
            txt.add(Line(format!("One cycle lasts {}", signal.cycle_length())));
            txt.add(Line(""));
            txt
        }),
        WrappedComposite::text_button(ctx, "Edit offset", hotkey(Key::O)),
        // TODO Icons
        WrappedComposite::text_button(ctx, "Reset to default", hotkey(Key::R)),
        WrappedComposite::text_button(ctx, "Use preset", hotkey(Key::P)),
    ];
    let has_sidewalks = ui
        .primary
        .map
        .get_turns_in_intersection(i)
        .iter()
        .any(|t| t.between_sidewalks());
    if has_sidewalks {
        col.push(WrappedComposite::text_button(
            ctx,
            "Make all-walk",
            hotkey(Key::B),
        ));
    }
    col.push(WrappedComposite::text_button(
        ctx,
        "Close intersection for construction",
        None,
    ));

    for (idx, phase) in signal.phases.iter().enumerate() {
        col.push(
            ManagedWidget::draw_batch(
                ctx,
                GeomBatch::from(vec![(
                    Color::WHITE,
                    Polygon::rectangle(0.2 * ctx.canvas.window_width, 2.0),
                )]),
            )
            .margin(15)
            .centered_horiz(),
        );
        let mut row = vec![
            ManagedWidget::draw_text(
                ctx,
                Text::from(Line(format!("Phase #{}: {}", idx + 1, phase.duration))),
            ),
            WrappedComposite::svg_button(
                ctx,
                "../data/system/assets/tools/edit.svg",
                &format!("change duration of #{}", idx + 1),
                if selected == idx {
                    hotkey(Key::D)
                } else {
                    None
                },
            )
            .margin(10),
        ];
        if signal.phases.len() > 1 {
            row.push(
                WrappedComposite::svg_button(
                    ctx,
                    "../data/system/assets/tools/delete.svg",
                    &format!("delete phase #{}", idx + 1),
                    if selected == idx {
                        hotkey(Key::Backspace)
                    } else {
                        None
                    },
                )
                .align_right(),
            );
        }
        col.push(ManagedWidget::row(row).margin(5).centered());

        let mut orig_batch = GeomBatch::new();
        draw_signal_phase(
            ctx.prerender,
            phase,
            i,
            None,
            &mut orig_batch,
            ui,
            TrafficSignalStyle::Sidewalks,
        );

        let mut normal = GeomBatch::new();
        // TODO Ideally no background here, but we have to force the dimensions of normal and
        // hovered to be the same. For some reason the bbox is slightly different.
        if idx == selected {
            normal.push(Color::RED.alpha(0.15), bbox.clone());
        } else {
            normal.push(Color::BLACK, bbox.clone());
        }
        // Move to the origin and apply zoom
        for (color, poly) in orig_batch.consume() {
            normal.push(
                color,
                poly.translate(-bounds.min_x, -bounds.min_y).scale(zoom),
            );
        }

        let mut hovered = GeomBatch::new();
        hovered.append(normal.clone());
        hovered.push(Color::RED, bbox.to_outline(Distance::meters(5.0)));

        let mut move_phase = Vec::new();
        if idx != 0 {
            move_phase.push(WrappedComposite::nice_text_button(
                ctx,
                Text::from(Line("↑")),
                if selected == idx {
                    hotkey(Key::K)
                } else {
                    None
                },
                &format!("move up phase #{}", idx + 1),
            ));
        }
        if idx != signal.phases.len() - 1 {
            move_phase.push(WrappedComposite::nice_text_button(
                ctx,
                Text::from(Line("↓")),
                if selected == idx {
                    hotkey(Key::J)
                } else {
                    None
                },
                &format!("move down phase #{}", idx + 1),
            ));
        }

        col.push(
            ManagedWidget::row(vec![
                ManagedWidget::btn(Button::new(
                    ctx,
                    normal,
                    hovered,
                    None,
                    &format!("phase {}", idx + 1),
                    bbox.clone(),
                ))
                .margin(5),
                ManagedWidget::col(move_phase).centered(),
            ])
            .margin(5),
        );
        col.push(
            WrappedComposite::text_button(ctx, &format!("add new phase after #{}", idx + 1), None)
                .centered_horiz(),
        );
    }

    Composite::new(ManagedWidget::col(col).bg(colors::PANEL_BG))
        .aligned(HorizontalAlignment::Left, VerticalAlignment::Top)
        .max_size_percent(30, 90)
        .build(ctx)
}

fn change_traffic_signal(signal: ControlTrafficSignal, ui: &mut UI, ctx: &mut EventCtx) {
    let mut edits = ui.primary.map.get_edits().clone();
    // TODO Only record one command for the entire session. Otherwise, we can exit this editor and
    // undo a few times, potentially ending at an invalid state!
    if edits
        .commands
        .last()
        .map(|cmd| match cmd {
            EditCmd::ChangeTrafficSignal(ref s) => s.id == signal.id,
            _ => false,
        })
        .unwrap_or(false)
    {
        edits.commands.pop();
    }
    edits.commands.push(EditCmd::ChangeTrafficSignal(signal));
    apply_map_edits(ctx, ui, edits);
}

fn change_phase_duration(idx: usize, current_duration: Duration) -> Box<dyn State> {
    WizardState::new(Box::new(move |wiz, ctx, _| {
        let new_duration = wiz.wrap(ctx).input_something(
            "How long should this phase be (seconds)?",
            Some(format!("{}", current_duration.inner_seconds() as usize)),
            Box::new(|line| {
                line.parse::<usize>()
                    .ok()
                    .and_then(|n| if n != 0 { Some(n) } else { None })
            }),
        )?;
        Some(Transition::PopWithData(Box::new(move |state, ui, ctx| {
            let editor = state.downcast_mut::<TrafficSignalEditor>().unwrap();
            let mut signal = ui.primary.map.get_traffic_signal(editor.i).clone();
            editor.command_stack.push(signal.clone());
            editor.redo_stack.clear();
            editor.top_panel = make_top_panel(true, false, ctx);
            signal.phases[idx].duration = Duration::seconds(new_duration as f64);
            change_traffic_signal(signal, ui, ctx);
            editor.change_phase(idx, ui, ctx);
        })))
    }))
}

fn change_offset(current_duration: Duration) -> Box<dyn State> {
    WizardState::new(Box::new(move |wiz, ctx, _| {
        let new_duration = wiz.wrap(ctx).input_usize_prefilled(
            "What should the offset of this traffic signal be (seconds)?",
            format!("{}", current_duration.inner_seconds() as usize),
        )?;
        Some(Transition::PopWithData(Box::new(move |state, ui, ctx| {
            let editor = state.downcast_mut::<TrafficSignalEditor>().unwrap();
            let mut signal = ui.primary.map.get_traffic_signal(editor.i).clone();
            editor.command_stack.push(signal.clone());
            editor.redo_stack.clear();
            editor.top_panel = make_top_panel(true, false, ctx);
            signal.offset = Duration::seconds(new_duration as f64);
            change_traffic_signal(signal, ui, ctx);
            editor.change_phase(editor.current_phase, ui, ctx);
        })))
    }))
}

fn change_preset(i: IntersectionID) -> Box<dyn State> {
    WizardState::new(Box::new(move |wiz, ctx, ui| {
        let (_, new_signal) =
            wiz.wrap(ctx)
                .choose("Use which preset for this intersection?", || {
                    Choice::from(ControlTrafficSignal::get_possible_policies(
                        &ui.primary.map,
                        i,
                    ))
                })?;
        Some(Transition::PopWithData(Box::new(move |state, ui, ctx| {
            let editor = state.downcast_mut::<TrafficSignalEditor>().unwrap();
            editor
                .command_stack
                .push(ui.primary.map.get_traffic_signal(editor.i).clone());
            editor.redo_stack.clear();
            editor.top_panel = make_top_panel(true, false, ctx);
            change_traffic_signal(new_signal, ui, ctx);
            editor.change_phase(0, ui, ctx);
        })))
    }))
}

fn check_for_missing_groups(
    mut signal: ControlTrafficSignal,
    composite: &mut Composite,
    ui: &mut UI,
    ctx: &mut EventCtx,
) -> Transition {
    let mut missing: BTreeSet<TurnGroupID> = signal.turn_groups.keys().cloned().collect();
    for phase in &signal.phases {
        for g in &phase.protected_groups {
            missing.remove(g);
        }
        for g in &phase.yield_groups {
            missing.remove(g);
        }
    }
    if missing.is_empty() {
        let i = signal.id;
        if let Err(err) = signal.validate() {
            panic!("Edited traffic signal {} finalized with errors: {}", i, err);
        }
        return Transition::Pop;
    }
    let num_missing = missing.len();
    let mut phase = Phase::new();
    for g in missing {
        if g.crosswalk.is_some() {
            phase.protected_groups.insert(g);
        } else {
            phase.yield_groups.insert(g);
        }
    }
    signal.phases.push(phase);
    let last_phase = signal.phases.len() - 1;
    let id = signal.id;
    change_traffic_signal(signal, ui, ctx);
    *composite = make_diagram(id, last_phase, ui, ctx);

    Transition::Push(msg(
        "Error: missing turns",
        vec![
            format!("{} turns are missing from this traffic signal", num_missing),
            "They've all been added as a new last phase. Please update your changes to include \
             them."
                .to_string(),
        ],
    ))
}

// TODO I guess it's valid to preview without all turns possible. Some agents are just sad.
fn make_previewer(i: IntersectionID, phase: usize, suspended_sim: Sim) -> Box<dyn State> {
    WizardState::new(Box::new(move |wiz, ctx, ui| {
        let random = "random agents around just this intersection".to_string();
        let right_now = format!("change the traffic signal live at {}", suspended_sim.time());
        match wiz
            .wrap(ctx)
            .choose_string(
                "Preview the traffic signal with what kind of traffic?",
                || vec![random.clone(), right_now.clone()],
            )?
            .as_str()
        {
            x if x == random => {
                // Start at the current phase
                let signal = ui.primary.map.get_traffic_signal(i);
                // TODO Use the offset correctly
                let mut step = Duration::ZERO;
                for idx in 0..phase {
                    step += signal.phases[idx].duration;
                }
                ui.primary.sim.step(&ui.primary.map, step);

                // This should be a no-op
                ui.primary
                    .map
                    .recalculate_pathfinding_after_edits(&mut Timer::throwaway());
                spawn_agents_around(i, ui);
            }
            x if x == right_now => {
                ui.primary.sim = suspended_sim.clone();
            }
            _ => unreachable!(),
        };
        Some(Transition::Replace(Box::new(PreviewTrafficSignal::new(
            ctx, ui,
        ))))
    }))
}

// TODO Show diagram, auto-sync the phase.
// TODO Auto quit after things are gone?
struct PreviewTrafficSignal {
    composite: Composite,
    speed: SpeedControls,
    time_panel: TimePanel,
    orig_sim: Sim,
}

impl PreviewTrafficSignal {
    fn new(ctx: &mut EventCtx, ui: &UI) -> PreviewTrafficSignal {
        PreviewTrafficSignal {
            composite: Composite::new(
                ManagedWidget::col(vec![
                    ManagedWidget::draw_text(ctx, Text::from(Line("Previewing traffic signal"))),
                    WrappedComposite::text_button(ctx, "back to editing", hotkey(Key::Escape)),
                ])
                .bg(colors::PANEL_BG)
                .padding(10),
            )
            .aligned(HorizontalAlignment::Center, VerticalAlignment::Top)
            .build(ctx),
            speed: SpeedControls::new(ctx),
            time_panel: TimePanel::new(ctx, ui),
            orig_sim: ui.primary.sim.clone(),
        }
    }
}

impl State for PreviewTrafficSignal {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Transition {
        ctx.canvas_movement();

        match self.composite.event(ctx) {
            Some(Outcome::Clicked(x)) => match x.as_ref() {
                "back to editing" => {
                    ui.primary.clear_sim();
                    return Transition::Pop;
                }
                _ => unreachable!(),
            },
            None => {}
        }

        self.time_panel.event(ctx, ui);
        match self.speed.event(ctx, ui) {
            Some(WrappedOutcome::Transition(t)) => {
                return t;
            }
            Some(WrappedOutcome::Clicked(x)) => match x {
                x if x == "reset to midnight" => {
                    ui.primary.sim = self.orig_sim.clone();
                    // TODO drawmap
                }
                _ => unreachable!(),
            },
            None => {}
        }
        if self.speed.is_paused() {
            Transition::Keep
        } else {
            Transition::KeepWithMode(EventLoopMode::Animation)
        }
    }

    fn draw(&self, g: &mut GfxCtx, _: &UI) {
        self.composite.draw(g);
        self.speed.draw(g);
        self.time_panel.draw(g);
    }
}
