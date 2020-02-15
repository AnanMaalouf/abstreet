use crate::colors;
use crate::edit::EditMode;
use crate::game::{State, Transition, WizardState};
use crate::helpers::{nice_map_name, ID};
use crate::managed::{WrappedComposite, WrappedOutcome};
use crate::sandbox::gameplay::{spawner, GameplayMode, GameplayState};
use crate::sandbox::SandboxMode;
use crate::ui::UI;
use ezgui::{
    hotkey, lctrl, Choice, Color, Composite, EventCtx, GeomBatch, GfxCtx, HorizontalAlignment, Key,
    Line, ManagedWidget, ScreenRectangle, Text, VerticalAlignment,
};
use geom::Polygon;
use map_model::IntersectionID;
use std::collections::BTreeSet;

// TODO Maybe remember what things were spawned, offer to replay this later
pub struct Freeform {
    // TODO Clean these up later when done?
    pub spawn_pts: BTreeSet<IntersectionID>,
    top_center: WrappedComposite,
}

impl Freeform {
    pub fn new(ctx: &mut EventCtx, ui: &UI) -> Box<dyn GameplayState> {
        Box::new(Freeform {
            spawn_pts: BTreeSet::new(),
            top_center: freeform_controller(ctx, ui, GameplayMode::Freeform, "none"),
        })
    }
}

impl GameplayState for Freeform {
    fn event(&mut self, ctx: &mut EventCtx, ui: &mut UI) -> Option<Transition> {
        match self.top_center.event(ctx, ui) {
            Some(WrappedOutcome::Transition(t)) => {
                return Some(t);
            }
            Some(WrappedOutcome::Clicked(_)) => unreachable!(),
            None => {}
        }

        if let Some(new_state) = spawner::AgentSpawner::new(ctx, ui) {
            return Some(Transition::Push(new_state));
        }
        if let Some(new_state) = spawner::SpawnManyAgents::new(ctx, ui) {
            return Some(Transition::Push(new_state));
        }
        None
    }

    fn draw(&self, g: &mut GfxCtx, ui: &UI) {
        self.top_center.draw(g);
        // TODO Overriding draw options would be ideal, but...
        for i in &self.spawn_pts {
            g.draw_polygon(Color::GREEN.alpha(0.8), &ui.primary.map.get_i(*i).polygon);
        }

        if let Some(ID::Intersection(i)) = ui.primary.current_selection {
            if self.spawn_pts.contains(&i) {
                let cnt = ui.primary.sim.count_trips_involving_border(i);
                let mut txt = Text::new().with_bg();
                for line in cnt.describe() {
                    txt.add(Line(line));
                }
                g.draw_mouse_tooltip(txt);
            }
        }
    }
}

pub fn freeform_controller(
    ctx: &mut EventCtx,
    ui: &UI,
    gameplay: GameplayMode,
    scenario_name: &str,
) -> WrappedComposite {
    let c = Composite::new(
        ManagedWidget::row(vec![
            ManagedWidget::draw_text(ctx, Text::from(Line("Sandbox").size(26))).margin(5),
            ManagedWidget::draw_batch(
                ctx,
                GeomBatch::from(vec![(Color::WHITE, Polygon::rectangle(2.0, 50.0))]),
            )
            .margin(5),
            ManagedWidget::draw_text(ctx, Text::from(Line("Map:").size(18).roboto_bold()))
                .margin(5),
            WrappedComposite::nice_text_button(
                ctx,
                Text::from(
                    Line(format!("{} ▼", nice_map_name(ui.primary.map.get_name())))
                        .size(18)
                        .roboto(),
                ),
                lctrl(Key::L),
                "change map",
            )
            .margin(5),
            ManagedWidget::draw_text(ctx, Text::from(Line("Traffic:").size(18).roboto_bold()))
                .margin(5),
            WrappedComposite::nice_text_button(
                ctx,
                Text::from(Line(format!("{} ▼", scenario_name)).size(18).roboto()),
                hotkey(Key::S),
                "change traffic",
            )
            .margin(5),
            WrappedComposite::svg_button(
                ctx,
                "../data/system/assets/tools/edit_map.svg",
                "edit map",
                lctrl(Key::E),
            )
            .margin(5),
        ])
        .centered()
        .bg(colors::PANEL_BG),
    )
    .aligned(HorizontalAlignment::Center, VerticalAlignment::Top)
    .build(ctx);
    let map_picker = c.rect_of("change map").clone();
    let traffic_picker = c.rect_of("change traffic").clone();

    WrappedComposite::new(c)
        .cb("change map", {
            let gameplay = gameplay.clone();
            Box::new(move |_, _| {
                Some(Transition::Push(make_load_map(
                    map_picker.clone(),
                    gameplay.clone(),
                )))
            })
        })
        .cb(
            "change traffic",
            Box::new(move |_, _| {
                Some(Transition::Push(make_change_traffic(
                    traffic_picker.clone(),
                )))
            }),
        )
        .cb(
            "edit map",
            Box::new(move |ctx, ui| {
                Some(Transition::Push(Box::new(EditMode::new(
                    ctx,
                    ui,
                    gameplay.clone(),
                ))))
            }),
        )
}

fn make_load_map(btn: ScreenRectangle, gameplay: GameplayMode) -> Box<dyn State> {
    WizardState::new(Box::new(move |wiz, ctx, ui| {
        if let Some((_, name)) = wiz.wrap(ctx).choose_exact(
            (
                HorizontalAlignment::Centered(btn.center().x),
                VerticalAlignment::Below(btn.y2 + 15.0),
            ),
            None,
            || {
                let current_map = ui.primary.map.get_name();
                abstutil::list_all_objects(abstutil::path_all_maps())
                    .into_iter()
                    .filter(|n| n != current_map)
                    .map(|n| Choice::new(nice_map_name(&n), n.clone()))
                    .collect()
            },
        ) {
            ui.switch_map(ctx, abstutil::path_map(&name));
            // Assume a scenario with the same name exists.
            Some(Transition::PopThenReplace(Box::new(SandboxMode::new(
                ctx,
                ui,
                gameplay.clone(),
            ))))
        } else if wiz.aborted() {
            Some(Transition::Pop)
        } else {
            None
        }
    }))
}

fn make_change_traffic(btn: ScreenRectangle) -> Box<dyn State> {
    WizardState::new(Box::new(move |wiz, ctx, ui| {
        let (_, scenario_name) = wiz.wrap(ctx).choose_exact(
            (
                HorizontalAlignment::Centered(btn.center().x),
                VerticalAlignment::Below(btn.y2 + 15.0),
            ),
            None,
            || {
                let mut list = Vec::new();
                for name in abstutil::list_all_objects(abstutil::path_all_scenarios(
                    ui.primary.map.get_name(),
                )) {
                    let nice_name = if name == "weekday" {
                        "realistic weekday traffic".to_string()
                    } else {
                        name.clone()
                    };
                    list.push(Choice::new(nice_name, name));
                }
                list.push(Choice::new(
                    "random unrealistic trips",
                    "random".to_string(),
                ));
                list.push(Choice::new("just buses", "just buses".to_string()));
                list.push(Choice::new(
                    "none (you manually spawn traffic)",
                    "empty".to_string(),
                ));
                list
            },
        )?;
        ui.primary.clear_sim();
        Some(Transition::PopThenReplace(Box::new(SandboxMode::new(
            ctx,
            ui,
            if scenario_name == "empty" {
                GameplayMode::Freeform
            } else {
                GameplayMode::PlayScenario(scenario_name)
            },
        ))))
    }))
}
