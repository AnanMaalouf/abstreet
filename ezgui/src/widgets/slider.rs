use crate::{
    Color, Drawable, EventCtx, GeomBatch, GfxCtx, ScreenDims, ScreenPt, ScreenRectangle, Widget,
    WidgetImpl, WidgetOutput,
};
use geom::Polygon;

pub struct Slider {
    current_percent: f32,
    mouse_on_slider: bool,
    dragging: bool,

    horiz: bool,
    main_bg_len: f32,
    dragger_len: f32,

    draw: Drawable,

    top_left: ScreenPt,
    dims: ScreenDims,
}

const BG_CROSS_AXIS_LEN: f32 = 20.0;

impl Slider {
    pub fn horizontal(
        ctx: &EventCtx,
        width: f32,
        dragger_len: f32,
        current_percent: f32,
    ) -> Widget {
        let mut s = Slider {
            current_percent,
            mouse_on_slider: false,
            dragging: false,

            horiz: true,
            main_bg_len: width,
            dragger_len,

            draw: ctx.upload(GeomBatch::new()),

            top_left: ScreenPt::new(0.0, 0.0),
            dims: ScreenDims::new(0.0, 0.0),
        };
        s.recalc(ctx);
        Widget::new(Box::new(s))
    }

    pub fn vertical(ctx: &EventCtx, height: f32, dragger_len: f32, current_percent: f32) -> Widget {
        let mut s = Slider {
            current_percent,
            mouse_on_slider: false,
            dragging: false,

            horiz: false,
            main_bg_len: height,
            dragger_len,

            draw: ctx.upload(GeomBatch::new()),

            top_left: ScreenPt::new(0.0, 0.0),
            dims: ScreenDims::new(0.0, 0.0),
        };
        s.recalc(ctx);
        Widget::new(Box::new(s))
    }

    fn recalc(&mut self, ctx: &EventCtx) {
        // Full dims
        self.dims = if self.horiz {
            ScreenDims::new(self.main_bg_len, BG_CROSS_AXIS_LEN)
        } else {
            ScreenDims::new(BG_CROSS_AXIS_LEN, self.main_bg_len)
        };

        let mut batch = GeomBatch::new();

        // The background
        batch.push(
            Color::WHITE,
            Polygon::rectangle(self.dims.width, self.dims.height),
        );

        // The draggy thing
        batch.push(
            if self.mouse_on_slider {
                Color::grey(0.7).alpha(0.7)
            } else {
                Color::grey(0.7)
            },
            self.slider_geom(),
        );

        self.draw = ctx.upload(batch);
    }

    // Doesn't touch self.top_left
    fn slider_geom(&self) -> Polygon {
        if self.horiz {
            Polygon::rectangle(self.dragger_len, BG_CROSS_AXIS_LEN).translate(
                self.current_percent * (self.main_bg_len - self.dragger_len),
                0.0,
            )
        } else {
            Polygon::rectangle(BG_CROSS_AXIS_LEN, self.dragger_len).translate(
                0.0,
                self.current_percent * (self.main_bg_len - self.dragger_len),
            )
        }
    }

    pub fn get_percent(&self) -> f32 {
        self.current_percent
    }

    pub fn get_value(&self, num_items: usize) -> usize {
        (self.current_percent * (num_items as f32 - 1.0)) as usize
    }

    pub fn set_percent(&mut self, ctx: &EventCtx, percent: f32) {
        assert!(percent >= 0.0 && percent <= 1.0);
        self.current_percent = percent;
        self.recalc(ctx);
        // Just reset dragging, to prevent chaos
        self.dragging = false;
        if let Some(pt) = ctx.canvas.get_cursor_in_screen_space() {
            self.mouse_on_slider = self
                .slider_geom()
                .translate(self.top_left.x, self.top_left.y)
                .contains_pt(pt.to_pt());
        } else {
            self.mouse_on_slider = false;
        }
    }

    fn inner_event(&mut self, ctx: &mut EventCtx) -> bool {
        if self.dragging {
            if ctx.input.get_moved_mouse().is_some() {
                let percent = if self.horiz {
                    (ctx.canvas.get_cursor().x - self.top_left.x - (self.dragger_len / 2.0))
                        / (self.main_bg_len - self.dragger_len)
                } else {
                    (ctx.canvas.get_cursor().y - self.top_left.y - (self.dragger_len / 2.0))
                        / (self.main_bg_len - self.dragger_len)
                };
                self.current_percent = percent.min(1.0).max(0.0);
                return true;
            }
            if ctx.input.left_mouse_button_released() {
                self.dragging = false;
                return true;
            }
            return false;
        }

        if ctx.redo_mouseover() {
            let old = self.mouse_on_slider;
            if let Some(pt) = ctx.canvas.get_cursor_in_screen_space() {
                self.mouse_on_slider = self
                    .slider_geom()
                    .translate(self.top_left.x, self.top_left.y)
                    .contains_pt(pt.to_pt());
            } else {
                self.mouse_on_slider = false;
            }
            return self.mouse_on_slider != old;
        }
        if ctx.input.left_mouse_button_pressed() {
            if self.mouse_on_slider {
                self.dragging = true;
                return true;
            }

            // Did we click somewhere else on the bar?
            if let Some(pt) = ctx.canvas.get_cursor_in_screen_space() {
                if Polygon::rectangle(self.dims.width, self.dims.height)
                    .translate(self.top_left.x, self.top_left.y)
                    .contains_pt(pt.to_pt())
                {
                    let percent = if self.horiz {
                        (pt.x - self.top_left.x - (self.dragger_len / 2.0))
                            / (self.main_bg_len - self.dragger_len)
                    } else {
                        (pt.y - self.top_left.y - (self.dragger_len / 2.0))
                            / (self.main_bg_len - self.dragger_len)
                    };
                    self.current_percent = percent.min(1.0).max(0.0);
                    self.mouse_on_slider = true;
                    self.dragging = true;
                    return true;
                }
            }
        }
        false
    }
}

impl WidgetImpl for Slider {
    fn get_dims(&self) -> ScreenDims {
        self.dims
    }

    fn set_pos(&mut self, top_left: ScreenPt) {
        self.top_left = top_left;
    }

    fn event(&mut self, ctx: &mut EventCtx, _output: &mut WidgetOutput) {
        if self.inner_event(ctx) {
            self.recalc(ctx);
        }
    }

    fn draw(&self, g: &mut GfxCtx) {
        g.redraw_at(self.top_left, &self.draw);
        // TODO Since the sliders in Composites are scrollbars outside of the clipping rectangle,
        // this stays for now.
        g.canvas
            .mark_covered_area(ScreenRectangle::top_left(self.top_left, self.dims));
    }
}
