use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use std::f32::consts::{PI, TAU};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiIcon {
    Refresh,
    Pause,
    Play,
    MinimizeTray,
    Settings,
    Search,
    Stop,
    Save,
    Power,
    Plus,
    Edit,
    Trash,
    Close,
    StarFilled,
    Sun,
    Moon,
    Monitor,
    Warning,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// 精密網路儀表板色彩系統（非純黑純白，低調帶暖灰／冷岩灰底色搭配工業訊號橘紅強調色）
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub canvas: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub input_bg: Color32,
    pub hairline: Color32,
    pub ink_primary: Color32,
    pub ink_secondary: Color32,
    pub ink_muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_soft: Color32,
    pub accent_ink: Color32,
    pub online: Color32,
    pub online_bg: Color32,
    pub offline: Color32,
    pub offline_bg: Color32,
    pub waking: Color32,
    pub waking_bg: Color32,
    pub info: Color32,
    pub unknown: Color32,
    pub unknown_bg: Color32,
}

impl Palette {
    pub fn for_mode(dark: bool) -> Self {
        if dark {
            Self {
                canvas: Color32::from_rgb(20, 21, 23),         // #141517
                sidebar: Color32::from_rgb(26, 28, 32),        // #1A1C20
                surface: Color32::from_rgb(33, 36, 41),        // #212429
                surface_hover: Color32::from_rgb(43, 47, 54),  // #2B2F36
                input_bg: Color32::from_rgb(16, 17, 20),       // #101114
                hairline: Color32::from_rgb(46, 50, 58),       // #2E323A
                ink_primary: Color32::from_rgb(236, 234, 228), // #ECEAE4
                ink_secondary: Color32::from_rgb(162, 165, 172),
                ink_muted: Color32::from_rgb(112, 116, 125),
                accent: Color32::from_rgb(234, 106, 50),       // #EA6A32 訊號橘紅
                accent_hover: Color32::from_rgb(244, 124, 70),
                accent_soft: Color32::from_rgba_unmultiplied(234, 106, 50, 34),
                accent_ink: Color32::from_rgb(252, 250, 247),
                online: Color32::from_rgb(52, 211, 153),
                online_bg: Color32::from_rgba_unmultiplied(16, 185, 129, 28),
                offline: Color32::from_rgb(248, 113, 113),
                offline_bg: Color32::from_rgba_unmultiplied(239, 68, 68, 30),
                waking: Color32::from_rgb(251, 191, 36),
                waking_bg: Color32::from_rgba_unmultiplied(245, 158, 11, 28),
                info: Color32::from_rgb(125, 180, 245),
                unknown: Color32::from_rgb(138, 144, 154),
                unknown_bg: Color32::from_rgba_unmultiplied(148, 163, 184, 24),
            }
        } else {
            Self {
                canvas: Color32::from_rgb(244, 243, 239),      // #F4F3EF 溫潤工程紙
                sidebar: Color32::from_rgb(235, 233, 227),     // #EBE9E3
                surface: Color32::from_rgb(250, 249, 246),     // #FAF9F6
                surface_hover: Color32::from_rgb(228, 226, 219),
                input_bg: Color32::from_rgb(252, 251, 248),    // #FCFBF8
                hairline: Color32::from_rgb(214, 211, 201),    // #D6D3C9
                ink_primary: Color32::from_rgb(28, 27, 24),    // #1C1B18 深暖炭
                ink_secondary: Color32::from_rgb(92, 90, 82),
                ink_muted: Color32::from_rgb(138, 134, 124),
                accent: Color32::from_rgb(217, 83, 30),        // #D9531E Braun 工業橘紅
                accent_hover: Color32::from_rgb(196, 70, 22),
                accent_soft: Color32::from_rgba_unmultiplied(217, 83, 30, 24),
                accent_ink: Color32::from_rgb(253, 252, 250),
                online: Color32::from_rgb(21, 128, 61),
                online_bg: Color32::from_rgba_unmultiplied(22, 163, 74, 24),
                offline: Color32::from_rgb(185, 28, 28),
                offline_bg: Color32::from_rgba_unmultiplied(220, 38, 38, 22),
                waking: Color32::from_rgb(180, 83, 9),
                waking_bg: Color32::from_rgba_unmultiplied(217, 119, 6, 24),
                info: Color32::from_rgb(29, 78, 216),
                unknown: Color32::from_rgb(112, 109, 100),
                unknown_bg: Color32::from_rgba_unmultiplied(120, 113, 108, 20),
            }
        }
    }
}

/// 繪製 1.5px 單色線條風格向量圖示（任意 DPI 皆清晰、幾何精確對齊）
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: UiIcon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let r = s * 0.5;
    let sw = (s * 0.115).clamp(1.25, 1.85);
    let stroke = Stroke::new(sw, color);

    let p = |nx: f32, ny: f32| pos2(c.x + nx * r, c.y + ny * r);

    match icon {
        UiIcon::Refresh => {
            let rad = r * 0.72;
            let a0 = 0.45_f32;
            let sweep = TAU - 1.15;
            let a1 = a0 + sweep;
            draw_arc(painter, c, rad, a0, sweep, stroke, 20);

            let end = pos2(c.x + a1.cos() * rad, c.y + a1.sin() * rad);
            let tx = -a1.sin();
            let ty = a1.cos();
            let nx = a1.cos();
            let ny = a1.sin();
            let tip = pos2(end.x + tx * (sw * 0.35), end.y + ty * (sw * 0.35));
            let wl = r * 0.46;
            let wing1 = pos2(
                tip.x + (-tx * 0.85 + nx * 0.65) * wl,
                tip.y + (-ty * 0.85 + ny * 0.65) * wl,
            );
            let wing2 = pos2(
                tip.x + (-tx * 0.85 - nx * 0.65) * wl,
                tip.y + (-ty * 0.85 - ny * 0.65) * wl,
            );
            painter.line_segment([wing1, tip], stroke);
            painter.line_segment([wing2, tip], stroke);
        }
        UiIcon::Pause => {
            let w = r * 0.32;
            let h = r * 1.24;
            let left = Rect::from_center_size(p(-0.34, 0.0), vec2(w, h));
            let right = Rect::from_center_size(p(0.34, 0.0), vec2(w, h));
            painter.rect_filled(left, 1.5, color);
            painter.rect_filled(right, 1.5, color);
        }
        UiIcon::Play => {
            let pts = vec![p(-0.40, -0.64), p(0.68, 0.0), p(-0.40, 0.64)];
            painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        UiIcon::MinimizeTray => {
            painter.line_segment([p(-0.74, 0.36), p(-0.74, 0.72)], stroke);
            painter.line_segment([p(-0.74, 0.72), p(0.74, 0.72)], stroke);
            painter.line_segment([p(0.74, 0.72), p(0.74, 0.36)], stroke);
            painter.line_segment([p(0.0, -0.72), p(0.0, 0.24)], stroke);
            painter.line_segment([p(-0.36, -0.10), p(0.0, 0.28)], stroke);
            painter.line_segment([p(0.36, -0.10), p(0.0, 0.28)], stroke);
        }
        UiIcon::Settings => {
            painter.circle_stroke(c, r * 0.34, stroke);
            painter.circle_stroke(c, r * 0.60, Stroke::new(sw * 0.9, color));
            for i in 0..8 {
                let a = (i as f32) * (TAU / 8.0);
                let p1 = pos2(c.x + a.cos() * r * 0.60, c.y + a.sin() * r * 0.60);
                let p2 = pos2(c.x + a.cos() * r * 0.86, c.y + a.sin() * r * 0.86);
                painter.line_segment([p1, p2], Stroke::new(sw * 1.15, color));
            }
        }
        UiIcon::Search => {
            let lens_c = p(-0.14, -0.14);
            painter.circle_stroke(lens_c, r * 0.56, stroke);
            painter.line_segment([p(0.28, 0.28), p(0.78, 0.78)], Stroke::new(sw * 1.15, color));
        }
        UiIcon::Stop => {
            let box_rect = Rect::from_center_size(c, vec2(r * 1.14, r * 1.14));
            painter.rect_filled(box_rect, 2.0, color);
        }
        UiIcon::Save => {
            let pts = vec![
                p(-0.72, -0.72),
                p(0.40, -0.72),
                p(0.72, -0.40),
                p(0.72, 0.72),
                p(-0.72, 0.72),
                p(-0.72, -0.72),
            ];
            painter.add(egui::Shape::line(pts, stroke));
            let top_slot = Rect::from_min_max(p(-0.38, -0.72), p(0.26, -0.24));
            painter.rect_stroke(
                top_slot,
                0.5,
                Stroke::new(sw * 0.85, color),
                egui::StrokeKind::Middle,
            );
            let bottom_label = Rect::from_min_max(p(-0.42, 0.14), p(0.42, 0.72));
            painter.rect_stroke(
                bottom_label,
                0.5,
                Stroke::new(sw * 0.85, color),
                egui::StrokeKind::Middle,
            );
        }
        UiIcon::Power => {
            draw_arc(
                painter,
                p(0.0, 0.08),
                r * 0.66,
                -PI * 0.5 + 0.58,
                TAU - 1.16,
                stroke,
                18,
            );
            painter.line_segment([p(0.0, -0.78), p(0.0, 0.02)], Stroke::new(sw * 1.1, color));
        }
        UiIcon::Plus => {
            painter.line_segment([p(-0.62, 0.0), p(0.62, 0.0)], stroke);
            painter.line_segment([p(0.0, -0.62), p(0.0, 0.62)], stroke);
        }
        UiIcon::Edit => {
            painter.line_segment([p(-0.68, 0.68), p(-0.52, 0.20)], stroke);
            painter.line_segment([p(-0.68, 0.68), p(-0.20, 0.52)], stroke);
            painter.line_segment([p(-0.52, 0.20), p(0.38, -0.70)], stroke);
            painter.line_segment([p(-0.20, 0.52), p(0.70, -0.38)], stroke);
            painter.line_segment([p(0.38, -0.70), p(0.70, -0.38)], stroke);
        }
        UiIcon::Trash => {
            painter.line_segment([p(-0.74, -0.46), p(0.74, -0.46)], stroke);
            painter.line_segment([p(-0.28, -0.74), p(0.28, -0.74)], stroke);
            painter.line_segment([p(-0.28, -0.74), p(-0.28, -0.46)], stroke);
            painter.line_segment([p(0.28, -0.74), p(0.28, -0.46)], stroke);
            let body = vec![
                p(-0.56, -0.44),
                p(-0.46, 0.74),
                p(0.46, 0.74),
                p(0.56, -0.44),
            ];
            painter.add(egui::Shape::line(body, stroke));
            painter.line_segment([p(-0.18, -0.16), p(-0.14, 0.48)], Stroke::new(sw * 0.85, color));
            painter.line_segment([p(0.18, -0.16), p(0.14, 0.48)], Stroke::new(sw * 0.85, color));
        }
        UiIcon::Close => {
            painter.line_segment([p(-0.50, -0.50), p(0.50, 0.50)], stroke);
            painter.line_segment([p(0.50, -0.50), p(-0.50, 0.50)], stroke);
        }
        UiIcon::StarFilled => {
            let mut pts = Vec::with_capacity(10);
            for i in 0..10 {
                let angle = -PI * 0.5 + (i as f32) * (PI / 5.0);
                let rad = if i % 2 == 0 { r * 0.80 } else { r * 0.36 };
                pts.push(pos2(c.x + angle.cos() * rad, c.y + angle.sin() * rad));
            }
            for i in 0..10 {
                let tri = vec![c, pts[i], pts[(i + 1) % 10]];
                painter.add(egui::Shape::convex_polygon(tri, color, Stroke::NONE));
            }
        }
        UiIcon::Sun => {
            painter.circle_stroke(c, r * 0.38, stroke);
            for i in 0..8 {
                let a = (i as f32) * (TAU / 8.0);
                let p1 = pos2(c.x + a.cos() * r * 0.62, c.y + a.sin() * r * 0.62);
                let p2 = pos2(c.x + a.cos() * r * 0.88, c.y + a.sin() * r * 0.88);
                painter.line_segment([p1, p2], stroke);
            }
        }
        UiIcon::Moon => {
            let r1 = r * 0.74;
            let r2 = r * 0.58;
            let dx = r * 0.34;
            let dy = -r * 0.34;
            let d = (dx * dx + dy * dy).sqrt();
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = (r1 * r1 - a * a).max(0.0).sqrt();
            let ux = dx / d;
            let uy = dy / d;
            let mx = ux * a;
            let my = uy * a;

            let i1 = pos2(c.x + mx + uy * h, c.y + my - ux * h);
            let i2 = pos2(c.x + mx - uy * h, c.y + my + ux * h);
            let o2 = pos2(c.x + dx, c.y + dy);

            let mut pts = Vec::with_capacity(34);
            let o1_a1 = (i1.y - c.y).atan2(i1.x - c.x);
            let mut o1_a2 = (i2.y - c.y).atan2(i2.x - c.x);
            if o1_a2 > o1_a1 {
                o1_a2 -= TAU;
            }
            for step in 0..=18 {
                let t = step as f32 / 18.0;
                let ang = o1_a1 + (o1_a2 - o1_a1) * t;
                pts.push(pos2(c.x + ang.cos() * r1, c.y + ang.sin() * r1));
            }

            let o2_a1 = (i2.y - o2.y).atan2(i2.x - o2.x);
            let mut o2_a2 = (i1.y - o2.y).atan2(i1.x - o2.x);
            if o2_a2 < o2_a1 {
                o2_a2 += TAU;
            }
            for step in 1..14 {
                let t = step as f32 / 14.0;
                let ang = o2_a1 + (o2_a2 - o2_a1) * t;
                pts.push(pos2(o2.x + ang.cos() * r2, o2.y + ang.sin() * r2));
            }

            painter.add(egui::Shape::closed_line(pts, stroke));
        }
        UiIcon::Monitor => {
            let screen = Rect::from_min_max(p(-0.78, -0.66), p(0.78, 0.34));
            painter.rect_stroke(screen, 1.5, stroke, egui::StrokeKind::Middle);
            painter.line_segment([p(0.0, 0.36), p(0.0, 0.70)], stroke);
            painter.line_segment([p(-0.40, 0.70), p(0.40, 0.70)], stroke);
        }
        UiIcon::Warning => {
            let tri = vec![p(0.0, -0.78), p(0.80, 0.70), p(-0.80, 0.70), p(0.0, -0.78)];
            painter.add(egui::Shape::line(tri, stroke));
            painter.line_segment([p(0.0, -0.24), p(0.0, 0.24)], Stroke::new(sw * 1.05, color));
            painter.circle_filled(p(0.0, 0.48), sw * 0.62, color);
        }
        UiIcon::Network => {
            // 精簡網卡／拓樸節點圖示
            let top = Rect::from_center_size(p(0.0, -0.52), vec2(r * 0.68, r * 0.44));
            let bl = Rect::from_center_size(p(-0.48, 0.52), vec2(r * 0.52, r * 0.40));
            let br = Rect::from_center_size(p(0.48, 0.52), vec2(r * 0.52, r * 0.40));
            painter.rect_stroke(top, 1.2, stroke, egui::StrokeKind::Middle);
            painter.rect_stroke(bl, 1.2, stroke, egui::StrokeKind::Middle);
            painter.rect_stroke(br, 1.2, stroke, egui::StrokeKind::Middle);
            painter.line_segment([p(0.0, -0.28), p(0.0, 0.04)], stroke);
            painter.line_segment([p(-0.48, 0.04), p(0.48, 0.04)], stroke);
            painter.line_segment([p(-0.48, 0.04), p(-0.48, 0.30)], stroke);
            painter.line_segment([p(0.48, 0.04), p(0.48, 0.30)], stroke);
        }
    }
}

fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_rad: f32,
    sweep_rad: f32,
    stroke: Stroke,
    segments: usize,
) {
    let mut pts = Vec::with_capacity(segments + 1);
    for i in 0..=segments {
        let t = (i as f32) / (segments as f32);
        let a = start_rad + sweep_rad * t;
        pts.push(pos2(center.x + a.cos() * radius, center.y + a.sin() * radius));
    }
    painter.add(egui::Shape::line(pts, stroke));
}

/// 精簡狀態指示膠囊（附完美置中圓點與細微背景）
pub fn status_pill(
    ui: &mut egui::Ui,
    text: &str,
    fg: Color32,
    bg: Color32,
    filled_dot: bool,
) {
    let font_id = egui::FontId::proportional(11.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font_id, Color32::PLACEHOLDER);
    let pad_x = 8.0;
    let pad_y = 3.0;
    let dot_w = 8.0;
    let gap = 5.0;
    let desired = vec2(
        pad_x * 2.0 + dot_w + gap + galley.size().x,
        (pad_y * 2.0 + galley.size().y).max(20.0),
    );
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, rect.height() * 0.5, bg);
        let dot_center = pos2(rect.left() + pad_x + 3.5, rect.center().y);
        if filled_dot {
            ui.painter().circle_filled(dot_center, 3.4, fg);
        } else {
            ui.painter()
                .circle_stroke(dot_center, 3.1, Stroke::new(1.3, fg));
        }
        let text_pos = pos2(
            rect.left() + pad_x + dot_w + gap,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
}

/// 分層按鈕（Primary 強調實心 / Secondary 描邊次要 / Ghost 透明無框 / Danger 警示）
pub fn styled_button(
    ui: &mut egui::Ui,
    pal: &Palette,
    style: ButtonStyle,
    icon: Option<UiIcon>,
    text: &str,
    compact: bool,
) -> egui::Response {
    let icon_size = if compact { 12.5 } else { 13.5 };
    let gap = if icon.is_some() && !text.is_empty() {
        6.0
    } else {
        0.0
    };
    let pad: Vec2 = if compact {
        vec2(8.0, 4.0)
    } else {
        vec2(11.0, 6.0)
    };
    let font_id = egui::FontId::proportional(if compact { 12.0 } else { 13.0 });
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font_id, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { icon_size } else { 0.0 };
    let text_w = if text.is_empty() { 0.0 } else { galley.size().x };
    let desired = vec2(
        pad.x * 2.0 + icon_w + gap + text_w,
        (pad.y * 2.0 + galley.size().y.max(icon_size)).max(if compact { 24.0 } else { 30.0 }),
    );

    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && response.hovered();
        let active = enabled && response.is_pointer_button_down_on();

        let (bg, border, fg) = match style {
            ButtonStyle::Primary => {
                let bg = if !enabled {
                    pal.hairline
                } else if active || hovered {
                    pal.accent_hover
                } else {
                    pal.accent
                };
                let fg = if enabled { pal.accent_ink } else { pal.ink_muted };
                (bg, Stroke::NONE, fg)
            }
            ButtonStyle::Secondary => {
                let bg = if active || hovered {
                    pal.surface_hover
                } else {
                    pal.surface
                };
                let fg = if enabled {
                    pal.ink_primary
                } else {
                    pal.ink_muted
                };
                (bg, Stroke::new(1.0, pal.hairline), fg)
            }
            ButtonStyle::Ghost => {
                let bg = if active || hovered {
                    pal.surface_hover
                } else {
                    Color32::TRANSPARENT
                };
                let fg = if !enabled {
                    pal.ink_muted
                } else if hovered {
                    pal.ink_primary
                } else {
                    pal.ink_secondary
                };
                (bg, Stroke::NONE, fg)
            }
            ButtonStyle::Danger => {
                let bg = if active || hovered {
                    pal.offline_bg
                } else {
                    Color32::TRANSPARENT
                };
                let border = if hovered {
                    Stroke::new(1.0, pal.offline)
                } else {
                    Stroke::new(1.0, pal.hairline)
                };
                let fg = if enabled { pal.offline } else { pal.ink_muted };
                (bg, border, fg)
            }
        };

        let radius = if compact { 5.0 } else { 6.0 };
        ui.painter()
            .rect(rect, radius, bg, border, egui::StrokeKind::Inside);

        let mut cursor_x = rect.left() + pad.x;
        if let Some(ic) = icon {
            let icon_center = pos2(cursor_x + icon_size * 0.5, rect.center().y);
            let icon_rect = Rect::from_center_size(icon_center, vec2(icon_size, icon_size));
            paint_icon(ui.painter(), icon_rect, ic, fg);
            cursor_x = icon_rect.right() + gap;
        }

        if !text.is_empty() {
            let text_pos = pos2(cursor_x, rect.center().y - galley.size().y * 0.5);
            ui.painter().galley(text_pos, galley, fg);
        }
    }

    if response.hovered() && ui.is_enabled() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// 純圖示 Ghost 小按鈕（用於列表次要操作如編輯、刪除）
pub fn icon_only_ghost(
    ui: &mut egui::Ui,
    pal: &Palette,
    icon: UiIcon,
    danger_hover: bool,
) -> egui::Response {
    let desired = vec2(26.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let bg = if hovered {
            if danger_hover {
                pal.offline_bg
            } else {
                pal.surface_hover
            }
        } else {
            Color32::TRANSPARENT
        };
        let fg = if hovered {
            if danger_hover {
                pal.offline
            } else {
                pal.ink_primary
            }
        } else {
            pal.ink_secondary
        };
        ui.painter().rect_filled(rect, 5.0, bg);
        let icon_rect = Rect::from_center_size(rect.center(), vec2(13.0, 13.0));
        paint_icon(ui.painter(), icon_rect, icon, fg);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// 帶向量圖示的分段切換項（用於主題切換）
pub fn segment_choice(
    ui: &mut egui::Ui,
    pal: &Palette,
    selected: bool,
    icon: UiIcon,
    text: &str,
) -> egui::Response {
    let icon_size = 13.0;
    let gap = 5.0;
    let pad = vec2(10.0, 5.0);
    let font_id = egui::FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font_id, Color32::PLACEHOLDER);
    let desired = vec2(
        pad.x * 2.0 + icon_size + gap + galley.size().x,
        (pad.y * 2.0 + galley.size().y.max(icon_size)).max(26.0),
    );
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let (bg, border, fg) = if selected {
            (pal.accent_soft, Stroke::new(1.0, pal.accent), pal.accent)
        } else if hovered {
            (pal.surface_hover, Stroke::new(1.0, pal.hairline), pal.ink_primary)
        } else {
            (pal.surface, Stroke::new(1.0, pal.hairline), pal.ink_secondary)
        };
        ui.painter()
            .rect(rect, 6.0, bg, border, egui::StrokeKind::Inside);
        let icon_center = pos2(rect.left() + pad.x + icon_size * 0.5, rect.center().y);
        let icon_rect = Rect::from_center_size(icon_center, vec2(icon_size, icon_size));
        paint_icon(ui.painter(), icon_rect, icon, fg);
        let text_pos = pos2(
            icon_rect.right() + gap,
            rect.center().y - galley.size().y * 0.5,
        );
        ui.painter().galley(text_pos, galley, fg);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// 繪製 1px 精細結構水平線
pub fn hairline_h(ui: &mut egui::Ui, pal: &Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().line_segment(
            [pos2(rect.left(), rect.center().y), pos2(rect.right(), rect.center().y)],
            Stroke::new(1.0, pal.hairline),
        );
    }
}
