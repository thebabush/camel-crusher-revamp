use crate::CrusherParams;
use dune_crusher_dsp::FACTORY_PRESETS;
use egui::{pos2, vec2, Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Stroke};
use nih_plug::prelude::*;
use nih_plug_egui::{create_egui_editor, egui};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

struct Theme;
impl Theme {
    const BACKGROUND: Color32 = Color32::from_rgb(10, 16, 32);
    const ACCENT: Color32 = Color32::from_rgb(79, 216, 240);
    const TEXT: Color32 = Color32::from_rgb(234, 246, 255);
    const SECONDARY: Color32 = Color32::from_rgb(214, 230, 242);
    const MUTED: Color32 = Color32::from_rgb(143, 166, 196);
    const TITLE: Color32 = Color32::from_rgb(169, 189, 214);
    const BORDER: Color32 = Color32::from_rgb(34, 53, 90);
    const DIVIDER: Color32 = Color32::from_rgb(23, 38, 74);
    const TRACK: Color32 = Color32::from_rgb(24, 40, 74);
    const BODY: Color32 = Color32::from_rgb(19, 34, 65);
}

struct State {
    header: Option<(f32, egui::TextureHandle)>,
    last_preset: usize,
    random: u64,
    drag_value: f32,
}

pub fn create(params: Arc<CrusherParams>) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        State {
            header: None,
            last_preset: 0,
            random: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
                | 1,
            drag_value: 0.0,
        },
        |ctx, _| {
            let mut fonts = egui::FontDefinitions::default();
            for (name, bytes) in [
                (
                    "archivo",
                    include_bytes!("../assets/fonts/Archivo-Medium.ttf").as_slice(),
                ),
                (
                    "archivo-italic",
                    include_bytes!("../assets/fonts/Archivo-BlackItalic.ttf").as_slice(),
                ),
                (
                    "bold",
                    include_bytes!("../assets/fonts/BricolageGrotesque-Bold.ttf").as_slice(),
                ),
                (
                    "extra",
                    include_bytes!("../assets/fonts/BricolageGrotesque-ExtraBold.ttf").as_slice(),
                ),
                (
                    "mono",
                    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice(),
                ),
            ] {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_static(bytes).into());
                fonts
                    .families
                    .insert(FontFamily::Name(name.into()), vec![name.into()]);
            }
            ctx.set_fonts(fonts);
        },
        move |ctx, setter, state| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(Theme::BACKGROUND))
                .show(ctx, |ui| {
                    draw(ui, setter, &params, state);
                });
        },
    )
}

fn font(name: &str, size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(name.into()))
}

fn text(
    ui: &egui::Ui,
    at: Pos2,
    align: Align2,
    value: &str,
    family: &str,
    size: f32,
    color: Color32,
) {
    ui.painter()
        .text(at, align, value, font(family, size), color);
}

fn spaced(
    ui: &egui::Ui,
    at: Pos2,
    value: &str,
    family: &str,
    size: f32,
    spacing: f32,
    color: Color32,
) -> f32 {
    let mut job = egui::text::LayoutJob::simple_singleline(value.into(), font(family, size), color);
    job.sections[0].format.extra_letter_spacing = spacing;
    let galley = ui.painter().layout_job(job);
    let width = galley.size().x;
    ui.painter().galley(at, galley, color);
    width
}

fn set<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

fn matched(values: &[f32; 12]) -> Option<usize> {
    FACTORY_PRESETS.iter().position(|p| {
        p.values
            .iter()
            .zip(values)
            .all(|(a, b)| (a - b).abs() <= 1e-4)
    })
}

fn continuous(params: &CrusherParams) -> [(&FloatParam, usize); 7] {
    [
        (&params.mech, 1),
        (&params.tube, 2),
        (&params.cutoff, 4),
        (&params.res, 5),
        (&params.comp_amount, 7),
        (&params.mix, 10),
        (&params.volume, 11),
    ]
}

fn apply(setter: &ParamSetter, params: &CrusherParams, index: usize) {
    let values = FACTORY_PRESETS[index].values;
    for (param, i) in continuous(params) {
        set(setter, param, values[i]);
    }
    for (param, i) in [
        (&params.dist_on, 0),
        (&params.filter_on, 3),
        (&params.comp_on, 6),
        (&params.phat, 8),
        (&params.master_on, 9),
    ] {
        set(setter, param, values[i] >= 0.5);
    }
}

impl State {
    fn random_value(&mut self) -> f32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        (self.random >> 40) as f32 / 16_777_215.0
    }

    fn header(&mut self, ctx: &egui::Context) -> &egui::TextureHandle {
        let scale = ctx.pixels_per_point();
        if self.header.as_ref().is_none_or(|(old, _)| *old != scale) {
            let tree = resvg::usvg::Tree::from_data(
                include_bytes!("../assets/header-night.svg"),
                &resvg::usvg::Options::default(),
            )
            .expect("valid header artwork");
            let mut pixels = resvg::tiny_skia::Pixmap::new(
                (460.0 * scale).ceil() as u32,
                (104.0 * scale).ceil() as u32,
            )
            .expect("header dimensions");
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixels.as_mut(),
            );
            let image = egui::ColorImage::from_rgba_premultiplied(
                [pixels.width() as usize, pixels.height() as usize],
                pixels.data(),
            );
            self.header = Some((
                scale,
                ctx.load_texture("night-header", image, egui::TextureOptions::LINEAR),
            ));
        }
        &self.header.as_ref().unwrap().1
    }
}

fn draw(ui: &mut egui::Ui, setter: &ParamSetter, params: &CrusherParams, state: &mut State) {
    let origin = ui.min_rect().min;
    let p = |x, y| origin + vec2(x, y);
    ui.painter().image(
        state.header(ui.ctx()).id(),
        Rect::from_min_size(origin, vec2(460.0, 104.0)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let width = spaced(
        ui,
        p(70.0, 16.0),
        "Dune",
        "archivo",
        30.0,
        -0.5,
        Theme::TEXT,
    );
    spaced(
        ui,
        p(76.0 + width, 16.0),
        "Crusher",
        "archivo-italic",
        30.0,
        -0.5,
        Theme::ACCENT,
    );

    let current = matched(&params.values());
    if let Some(index) = current {
        state.last_preset = index;
    }
    for (id, x, direction) in [("previous", 34.0, -1), ("next", 352.0, 1)] {
        let center = p(x, 124.0);
        let response = ui.interact(
            Rect::from_center_size(center, vec2(28.0, 28.0)),
            ui.id().with(id),
            Sense::click(),
        );
        ui.painter()
            .circle_stroke(center, 13.5, Stroke::new(1.0_f32, Theme::BORDER));
        let d = direction as f32;
        ui.painter().add(egui::Shape::line(
            vec![
                center + vec2(-d * 1.7, -3.3),
                center + vec2(d * 1.7, 0.0),
                center + vec2(-d * 1.7, 3.3),
            ],
            Stroke::new(1.5_f32, Theme::SECONDARY),
        ));
        if response.clicked() {
            state.last_preset = (state.last_preset as i32 + direction).rem_euclid(20) as usize;
            apply(setter, params, state.last_preset);
        }
    }
    text(
        ui,
        p(58.0, 123.0),
        Align2::LEFT_CENTER,
        current.map_or("Custom", |i| FACTORY_PRESETS[i].name),
        "bold",
        16.0,
        Theme::TEXT,
    );
    let number = current.map_or_else(|| "-- / 20".into(), |i| format!("{:02} / 20", i + 1));
    text(
        ui,
        p(328.0, 125.0),
        Align2::RIGHT_CENTER,
        &number,
        "mono",
        10.0,
        Theme::MUTED,
    );
    ui.painter().line_segment(
        [p(56.0, 138.0), p(330.0, 138.0)],
        Stroke::new(1.0_f32, Theme::BORDER),
    );
    let shuffle = Rect::from_min_size(p(374.0, 110.0), vec2(66.0, 28.0));
    ui.painter().rect_stroke(
        shuffle,
        14.0,
        Stroke::new(1.0_f32, Theme::BORDER),
        egui::StrokeKind::Inside,
    );
    text(
        ui,
        shuffle.center(),
        Align2::CENTER_CENTER,
        "Shuffle",
        "bold",
        11.0,
        Theme::SECONDARY,
    );
    if ui
        .interact(shuffle, ui.id().with("shuffle"), Sense::click())
        .clicked()
    {
        for (param, _) in continuous(params) {
            set(setter, param, state.random_value());
        }
    }

    ui.painter().line_segment(
        [p(230.0, 148.0), p(230.0, 488.0)],
        Stroke::new(1.0_f32, Theme::DIVIDER),
    );
    ui.painter().line_segment(
        [p(12.0, 318.0), p(448.0, 318.0)],
        Stroke::new(1.0_f32, Theme::DIVIDER),
    );
    for (title, x, y, param) in [
        ("DISTORTION", 22.0, 158.0, &params.dist_on),
        ("FILTER", 240.0, 158.0, &params.filter_on),
        ("COMPRESSOR", 22.0, 328.0, &params.comp_on),
        ("MASTER", 240.0, 328.0, &params.master_on),
    ] {
        spaced(ui, p(x, y), title, "bold", 12.0, 2.0, Theme::TITLE);
        let rect = Rect::from_min_size(p(x + 168.0, y), vec2(30.0, 16.0));
        ui.painter().rect_filled(
            rect,
            8.0,
            if param.value() {
                Theme::ACCENT
            } else {
                Theme::TRACK
            },
        );
        ui.painter().circle_filled(
            rect.min + vec2(if param.value() { 22.0 } else { 8.0 }, 8.0),
            6.0,
            Theme::BACKGROUND,
        );
        if ui
            .interact(rect, ui.id().with(title), Sense::click())
            .clicked()
        {
            set(setter, param, !param.value());
        }
    }
    for (label, x, y, param) in [
        ("Tube", 71.5, 231.0, &params.tube),
        ("Mech", 170.5, 231.0, &params.mech),
        ("Cutoff", 289.5, 231.0, &params.cutoff),
        ("Res", 388.5, 231.0, &params.res),
        ("Amount", 71.5, 401.0, &params.comp_amount),
        ("Volume", 289.5, 401.0, &params.volume),
        ("Mix", 388.5, 401.0, &params.mix),
    ] {
        knob(ui, setter, param, label, p(x, y), &mut state.drag_value);
    }
    let phat = Rect::from_center_size(p(170.5, 414.0), vec2(66.0, 30.0));
    let color = if params.phat.value() {
        Theme::ACCENT
    } else {
        Theme::MUTED
    };
    ui.painter().rect_stroke(
        phat,
        15.0,
        Stroke::new(1.5_f32, color),
        egui::StrokeKind::Inside,
    );
    spaced(
        ui,
        phat.min + vec2(14.0, 7.0),
        "PHAT",
        "extra",
        12.0,
        1.0,
        color,
    );
    if ui
        .interact(phat, ui.id().with("phat"), Sense::click())
        .clicked()
    {
        set(setter, &params.phat, !params.phat.value());
    }
}

fn arc(ui: &egui::Ui, center: Pos2, value: f32, color: Color32) {
    if value <= 0.0 {
        return;
    }
    let steps = (96.0 * value).ceil() as usize;
    let points: Vec<_> = (0..=steps)
        .map(|i| {
            let angle = (-135.0 + 270.0 * value * i as f32 / steps as f32).to_radians();
            center + vec2(angle.sin(), -angle.cos()) * 29.0
        })
        .collect();
    ui.painter().circle_filled(points[0], 2.5, color);
    ui.painter().circle_filled(points[steps], 2.5, color);
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(5.0_f32, color)));
}

fn knob(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &FloatParam,
    label: &str,
    center: Pos2,
    drag_value: &mut f32,
) {
    let rect = Rect::from_min_size(center - vec2(36.0, 36.0), vec2(72.0, 96.0));
    let response = ui.interact(rect, ui.id().with(label), Sense::click_and_drag());
    if response.drag_started() {
        setter.begin_set_parameter(param);
        *drag_value = param.unmodulated_normalized_value();
    }
    if response.dragged() {
        let delta =
            ui.input(|i| i.pointer.delta().y / if i.modifiers.shift { 2000.0 } else { 200.0 });
        *drag_value = (*drag_value - delta).clamp(0.0, 1.0);
        setter.set_parameter_normalized(param, *drag_value);
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if response.double_clicked() {
        set(setter, param, param.default_plain_value());
    }
    if response.hovered() && !response.dragged() {
        let scroll = ui.input_mut(|i| {
            let delta = i.raw_scroll_delta.y;
            i.raw_scroll_delta = egui::Vec2::ZERO;
            delta / if i.modifiers.shift { 2000.0 } else { 200.0 }
        });
        if scroll != 0.0 {
            setter.begin_set_parameter(param);
            setter.set_parameter_normalized(
                param,
                (param.unmodulated_normalized_value() + scroll).clamp(0.0, 1.0),
            );
            setter.end_set_parameter(param);
        }
    }
    let value = param.modulated_normalized_value();
    arc(ui, center, 1.0, Theme::TRACK);
    arc(ui, center, value, Theme::ACCENT);
    ui.painter().circle_filled(center, 21.0, Theme::BODY);
    let angle = (-135.0 + 270.0 * value).to_radians();
    let end = center + vec2(angle.sin(), -angle.cos()) * 15.0;
    ui.painter()
        .line_segment([center, end], Stroke::new(2.5_f32, Theme::TEXT));
    ui.painter().circle_filled(center, 1.25, Theme::TEXT);
    ui.painter().circle_filled(end, 1.25, Theme::TEXT);
    text(
        ui,
        center + vec2(0.0, 39.0),
        Align2::CENTER_CENTER,
        label,
        "bold",
        12.0,
        Theme::TEXT,
    );
    text(
        ui,
        center + vec2(0.0, 54.0),
        Align2::CENTER_CENTER,
        &param.normalized_value_to_string(value, true),
        "mono",
        10.0,
        Theme::MUTED,
    );
}

#[cfg(test)]
mod tests;
