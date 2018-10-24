// Copyright 2018 Google LLC, licensed under http://www.apache.org/licenses/LICENSE-2.0

use ezgui::{Canvas, Color, GfxCtx, InputResult, Menu};
use objects::{Ctx, SETTINGS};
use piston::input::Key;
use plugins::{Plugin, PluginCtx};

// TODO assumes minimum screen size
const WIDTH: u32 = 255;
const HEIGHT: u32 = 255;
const TILE_DIMS: u32 = 2;

// TODO parts of this should be in ezgui
pub enum ColorPicker {
    Inactive,
    Choosing(Menu<()>),
    // Remember the original modified color in case we revert.
    ChangingColor(String, Option<Color>),
}

impl ColorPicker {
    pub fn new() -> ColorPicker {
        ColorPicker::Inactive
    }
}

impl Plugin for ColorPicker {
    fn event(&mut self, ctx: PluginCtx) -> bool {
        let (input, canvas, cs) = (ctx.input, ctx.canvas, ctx.cs);

        let mut new_state: Option<ColorPicker> = None;
        match self {
            ColorPicker::Inactive => {
                if input.unimportant_key_pressed(Key::D8, SETTINGS, "configure colors") {
                    new_state = Some(ColorPicker::Choosing(Menu::new(
                        "Pick a color to change",
                        cs.color_names(),
                    )));
                }
            }
            ColorPicker::Choosing(ref mut menu) => {
                match menu.event(input) {
                    InputResult::Canceled => {
                        new_state = Some(ColorPicker::Inactive);
                    }
                    InputResult::StillActive => {}
                    InputResult::Done(name, _) => {
                        new_state = Some(ColorPicker::ChangingColor(
                            name.clone(),
                            cs.get_modified(&name),
                        ));
                    }
                };
            }
            ColorPicker::ChangingColor(name, orig) => {
                if input.key_pressed(
                    Key::Escape,
                    &format!("stop changing color for {} and revert", name),
                ) {
                    cs.reset_modified(name, *orig);
                    new_state = Some(ColorPicker::Inactive);
                } else if input
                    .key_pressed(Key::Return, &format!("finalize new color for {}", name))
                {
                    info!("Setting color for {}", name);
                    new_state = Some(ColorPicker::Inactive);
                }

                if let Some((m_x, m_y)) = input.get_moved_mouse() {
                    // TODO argh too much casting
                    let (start_x, start_y) = get_screen_offset(canvas);
                    let x = (m_x - (start_x as f64)) / (TILE_DIMS as f64) / 255.0;
                    let y = (m_y - (start_y as f64)) / (TILE_DIMS as f64) / 255.0;
                    if x >= 0.0 && x <= 1.0 && y >= 0.0 && y <= 1.0 {
                        cs.override_color(name, get_color(x as f32, y as f32));
                    }
                }
            }
        };
        if let Some(s) = new_state {
            *self = s;
        }
        match self {
            ColorPicker::Inactive => false,
            _ => true,
        }
    }

    fn draw(&self, g: &mut GfxCtx, ctx: Ctx) {
        match self {
            ColorPicker::Inactive => {}
            ColorPicker::Choosing(menu) => {
                menu.draw(g, ctx.canvas);
            }
            ColorPicker::ChangingColor(_, _) => {
                let (start_x, start_y) = get_screen_offset(ctx.canvas);

                for x in 0..WIDTH {
                    for y in 0..HEIGHT {
                        let color = get_color((x as f32) / 255.0, (y as f32) / 255.0);
                        let corner = ctx.canvas.screen_to_map((
                            (x * TILE_DIMS + start_x) as f64,
                            (y * TILE_DIMS + start_y) as f64,
                        ));
                        g.draw_rectangle(
                            color,
                            [corner.x(), corner.y(), TILE_DIMS as f64, TILE_DIMS as f64],
                        );
                    }
                }
            }
        }
    }
}

fn get_screen_offset(canvas: &Canvas) -> (u32, u32) {
    let total_width = TILE_DIMS * WIDTH;
    let total_height = TILE_DIMS * HEIGHT;
    let start_x = (canvas.window_size.width - total_width) / 2;
    let start_y = (canvas.window_size.height - total_height) / 2;
    (start_x, start_y)
}

fn get_color(x: f32, y: f32) -> Color {
    assert!(x >= 0.0 && x <= 1.0);
    assert!(y >= 0.0 && y <= 1.0);
    Color::rgb_f(x, y, (x + y) / 2.0)
}
