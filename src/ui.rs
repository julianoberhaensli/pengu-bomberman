//! Shared UI widgets: buttons, panels, text fields, text helpers and the loading screen.

use bomberpengu::i18n::{tr, Key, Lang};
use macroquad::prelude::*;

pub const INK: Color = Color::new(0.08, 0.16, 0.28, 1.0);
pub const ERROR: Color = Color::new(0.75, 0.1, 0.05, 1.0);
const PANEL: Color = Color::new(0.93, 0.97, 1.0, 1.0);
const FOCUS: Color = Color::new(1.0, 0.82, 0.15, 1.0);
const ICE: Color = Color::new(0.75, 0.88, 1.0, 1.0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Normal,
    /// The suggested action of a screen.
    Primary,
}

pub fn text(s: &str, x: f32, y: f32, size: f32, color: Color) {
    draw_text(s, x, y, size, color);
}

/// Shrinks the text when it would reach past the 640 px window.
pub fn text_centered(s: &str, cx: f32, y: f32, size: f32, color: Color) {
    let max_w = 2.0 * cx.min(640.0 - cx) - 16.0;
    let mut sz = size;
    let mut d = measure_text(s, None, sz as u16, 1.0);
    while sz > 9.0 && d.width > max_w {
        sz -= 1.0;
        d = measure_text(s, None, sz as u16, 1.0);
    }
    draw_text(s, (cx - d.width / 2.0).round(), y, sz, color);
}

/// Draws `s` with the largest size <= `size` that fits into `max_w`.
pub fn text_fit(s: &str, x: f32, y: f32, max_w: f32, size: f32, color: Color) {
    let mut sz = size;
    while sz > 9.0 && measure_text(s, None, sz as u16, 1.0).width > max_w {
        sz -= 1.0;
    }
    draw_text(s, x, y, sz, color);
}

pub fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn drain_chars() {
    while get_char_pressed().is_some() {}
}

fn mouse() -> Vec2 {
    let (x, y) = mouse_position();
    vec2(x, y)
}

pub fn hovered(r: Rect) -> bool {
    r.contains(mouse())
}

pub fn clicked(r: Rect) -> bool {
    is_mouse_button_pressed(MouseButton::Left) && hovered(r)
}

fn lift(c: Color, d: f32) -> Color {
    Color::new((c.r + d).min(1.0), (c.g + d).min(1.0), (c.b + d).min(1.0), 1.0)
}

fn inset(r: Rect, d: f32) -> Rect {
    Rect::new(r.x + d, r.y + d, r.w - 2.0 * d, r.h - 2.0 * d)
}

/// Rounded rectangle; opaque colours only (the pieces overlap).
pub fn round_rect(r: Rect, rad: f32, c: Color) {
    let rad = rad.min(r.w / 2.0).min(r.h / 2.0);
    draw_rectangle(r.x + rad, r.y, r.w - 2.0 * rad, r.h, c);
    draw_rectangle(r.x, r.y + rad, r.w, r.h - 2.0 * rad, c);
    for (cx, cy) in [(r.x + rad, r.y + rad), (r.right() - rad, r.y + rad), (r.x + rad, r.bottom() - rad), (r.right() - rad, r.bottom() - rad)] {
        draw_circle(cx, cy, rad, c);
    }
}

pub fn panel(x: f32, y: f32, w: f32, h: f32) {
    let r = Rect::new(x, y, w, h);
    round_rect(r, 14.0, INK);
    round_rect(inset(r, 3.0), 11.0, PANEL);
}

/// The one button look used everywhere. `focused` marks the keyboard selection.
pub fn draw_button(r: Rect, label: &str, style: Style, enabled: bool, focused: bool) {
    let hover = enabled && hovered(r);
    let down = hover && is_mouse_button_down(MouseButton::Left);
    let (mut base, mut gloss) = match (enabled, style) {
        (false, _) => (Color::new(0.62, 0.67, 0.72, 1.0), Color::new(0.7, 0.75, 0.8, 1.0)),
        (true, Style::Normal) => (Color::new(0.2, 0.44, 0.8, 1.0), Color::new(0.32, 0.56, 0.9, 1.0)),
        (true, Style::Primary) => (Color::new(0.8, 0.16, 0.08, 1.0), Color::new(0.93, 0.3, 0.18, 1.0)),
    };
    if hover {
        base = lift(base, 0.07);
        gloss = lift(gloss, 0.07);
    }
    const DEPTH: f32 = 3.0;
    let top = if down { DEPTH - 1.0 } else { 0.0 };
    let body = Rect::new(r.x, r.y + top, r.w, r.h - DEPTH);
    if focused && enabled {
        round_rect(Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0), 13.0, FOCUS);
    }
    if !down {
        round_rect(Rect::new(r.x, r.y + DEPTH, r.w, r.h - DEPTH), 10.0, INK);
    }
    round_rect(body, 10.0, INK);
    round_rect(inset(body, 2.0), 8.0, base);
    let g = inset(body, 2.0);
    round_rect(Rect::new(g.x, g.y, g.w, g.h * 0.5), 8.0, gloss);
    let size = if r.h >= 40.0 { 22.0 } else { 18.0 };
    let mut sz = size;
    while sz > 10.0 && measure_text(label, None, sz as u16, 1.0).width > r.w - 16.0 {
        sz -= 1.0;
    }
    let cx = body.x + body.w / 2.0;
    let y = (body.y + body.h / 2.0 + sz * 0.27).round();
    text_centered(label, cx + 1.0, y + 1.0, sz, INK);
    text_centered(label, cx, y, sz, if enabled { WHITE } else { Color::new(0.9, 0.93, 0.96, 1.0) });
}

/// Draws a button and reports a mouse click on it.
pub fn button(r: Rect, label: &str, style: Style, enabled: bool) -> bool {
    draw_button(r, label, style, enabled, false);
    enabled && clicked(r)
}

pub fn name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.')
}

pub fn any_char(_: char) -> bool {
    true
}

const FIELD_PAD: f32 = 10.0;
const SELECTION: Color = Color::new(0.62, 0.78, 1.0, 1.0);

fn field_size(r: Rect) -> f32 {
    (r.h * 0.6).floor()
}

/// Key-repeat: fires on press, then after 0.4 s every 35 ms while held. `next` is the per-key timer.
fn repeating(key: KeyCode, next: &mut f64) -> bool {
    let now = get_time();
    if is_key_pressed(key) {
        *next = now + 0.4;
        true
    } else if is_key_down(key) && *next > 0.0 && now >= *next {
        *next = now + 0.035;
        true
    } else {
        if !is_key_down(key) {
            *next = 0.0;
        }
        false
    }
}

/// Single-line text input with cursor, selection (keyboard and mouse), key repeat and clipboard.
pub struct Field {
    pub text: String,
    max: usize,
    allow: fn(char) -> bool,
    /// Char indices; the selection is `anchor..cursor` in either order.
    cursor: usize,
    anchor: usize,
    scroll: f32,
    repeat: [f64; 4],
    dragging: bool,
    last_click: f64,
    last_input: f64,
    paste_pending: bool,
}

impl Field {
    pub fn new(text: &str, max: usize, allow: fn(char) -> bool) -> Self {
        let text: String = text.chars().filter(|c| allow(*c)).take(max).collect();
        let n = text.chars().count();
        Self { text, max, allow, cursor: n, anchor: n, scroll: 0.0, repeat: [0.0; 4], dragging: false, last_click: f64::NEG_INFINITY, last_input: 0.0, paste_pending: false }
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.anchor = 0;
        self.scroll = 0.0;
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, char_idx: usize) -> usize {
        self.text.char_indices().nth(char_idx).map_or(self.text.len(), |(b, _)| b)
    }

    fn range(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    fn selection(&self) -> &str {
        let (a, b) = self.range();
        &self.text[self.byte(a)..self.byte(b)]
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.range();
        if a == b {
            return false;
        }
        let (ba, bb) = (self.byte(a), self.byte(b));
        self.text.replace_range(ba..bb, "");
        self.cursor = a;
        self.anchor = a;
        true
    }

    fn delete_range(&mut self, from: usize, to: usize) {
        let (bf, bt) = (self.byte(from), self.byte(to));
        self.text.replace_range(bf..bt, "");
        self.cursor = from;
        self.anchor = from;
    }

    /// Replaces the selection with `s` (filtered, cut to the length limit).
    fn insert(&mut self, s: &str) {
        self.delete_selection();
        let room = self.max.saturating_sub(self.len());
        let ins: String = s.chars().filter(|c| !c.is_control() && (self.allow)(*c)).take(room).collect();
        let b = self.byte(self.cursor);
        self.text.insert_str(b, &ins);
        self.cursor += ins.chars().count();
        self.anchor = self.cursor;
    }

    fn word_left(&self, from: usize) -> usize {
        let cs: Vec<char> = self.text.chars().collect();
        let mut i = from;
        while i > 0 && cs[i - 1] == ' ' {
            i -= 1;
        }
        while i > 0 && cs[i - 1] != ' ' {
            i -= 1;
        }
        i
    }

    fn word_right(&self, from: usize) -> usize {
        let cs: Vec<char> = self.text.chars().collect();
        let mut i = from;
        while i < cs.len() && cs[i] != ' ' {
            i += 1;
        }
        while i < cs.len() && cs[i] == ' ' {
            i += 1;
        }
        i
    }

    /// x offset (relative to the text start) of every char boundary.
    fn widths(&self, size: f32) -> Vec<f32> {
        let mut w = vec![0.0];
        let mut prefix = String::new();
        for c in self.text.chars() {
            prefix.push(c);
            w.push(measure_text(&prefix, None, size as u16, 1.0).width);
        }
        w
    }

    fn visible_scroll(&self, widths: &[f32], inner_w: f32) -> f32 {
        let cx = widths[self.cursor.min(widths.len() - 1)];
        let total = widths[widths.len() - 1];
        let mut s = self.scroll;
        if cx - s > inner_w {
            s = cx - inner_w;
        }
        if cx < s {
            s = cx;
        }
        s.min((total + 2.0 - inner_w).max(0.0)).max(0.0)
    }

    /// Handles keyboard and mouse input for the focused field drawn in `r`.
    pub fn update(&mut self, r: Rect) {
        let size = field_size(r);
        let now = get_time();
        let down = |ks: [KeyCode; 2]| ks.iter().any(|k| is_key_down(*k));
        let ctrl = down([KeyCode::LeftControl, KeyCode::RightControl]) || down([KeyCode::LeftSuper, KeyCode::RightSuper]);
        let shift = down([KeyCode::LeftShift, KeyCode::RightShift]);
        let mut touched = false;

        while let Some(c) = get_char_pressed() {
            if !ctrl && !c.is_control() {
                self.insert(&c.to_string());
                touched = true;
            }
        }
        if self.paste_pending {
            self.paste_pending = false;
            if let Some(s) = macroquad::miniquad::window::clipboard_get() {
                self.insert(&s);
                touched = true;
            }
        }
        if ctrl {
            if is_key_pressed(KeyCode::A) {
                self.anchor = 0;
                self.cursor = self.len();
                touched = true;
            }
            let cut = is_key_pressed(KeyCode::X);
            if (cut || is_key_pressed(KeyCode::C)) && self.range().0 != self.range().1 {
                macroquad::miniquad::window::clipboard_set(self.selection());
                if cut {
                    self.delete_selection();
                }
                touched = true;
            }
            if is_key_pressed(KeyCode::V) {
                // the browser delivers the pasted text after the key event, so read it next frame there
                if cfg!(target_arch = "wasm32") {
                    self.paste_pending = true;
                } else if let Some(s) = macroquad::miniquad::window::clipboard_get() {
                    self.insert(&s);
                }
                touched = true;
            }
        }
        #[cfg(target_arch = "wasm32")]
        if self.range().0 != self.range().1 {
            // the browser copies on the key event, before this frame runs
            macroquad::miniquad::window::clipboard_set(self.selection());
        }

        if repeating(KeyCode::Backspace, &mut self.repeat[0]) {
            if !self.delete_selection() && self.cursor > 0 {
                let from = if ctrl { self.word_left(self.cursor) } else { self.cursor - 1 };
                self.delete_range(from, self.cursor);
            }
            touched = true;
        }
        if repeating(KeyCode::Delete, &mut self.repeat[1]) {
            if !self.delete_selection() && self.cursor < self.len() {
                let to = if ctrl { self.word_right(self.cursor) } else { self.cursor + 1 };
                self.delete_range(self.cursor, to);
            }
            touched = true;
        }
        let (sel_a, sel_b) = self.range();
        let collapse = !shift && sel_a != sel_b;
        let mut target = None;
        if repeating(KeyCode::Left, &mut self.repeat[2]) {
            target = Some(if collapse {
                sel_a
            } else if ctrl {
                self.word_left(self.cursor)
            } else {
                self.cursor.saturating_sub(1)
            });
        }
        if repeating(KeyCode::Right, &mut self.repeat[3]) {
            target = Some(if collapse {
                sel_b
            } else if ctrl {
                self.word_right(self.cursor)
            } else {
                (self.cursor + 1).min(self.len())
            });
        }
        if is_key_pressed(KeyCode::Home) {
            target = Some(0);
        }
        if is_key_pressed(KeyCode::End) {
            target = Some(self.len());
        }
        if let Some(t) = target {
            self.cursor = t;
            if !shift {
                self.anchor = t;
            }
            touched = true;
        }

        let widths = self.widths(size);
        let (mx, my) = mouse_position();
        let x0 = r.x + FIELD_PAD;
        let index_at = |x: f32| (0..widths.len()).min_by(|&a, &b| (widths[a] - x).abs().total_cmp(&(widths[b] - x).abs())).unwrap_or(0);
        if is_mouse_button_pressed(MouseButton::Left) && r.contains(vec2(mx, my)) {
            if now - self.last_click < 0.35 {
                self.anchor = 0;
                self.cursor = self.len();
                self.dragging = false;
            } else {
                self.cursor = index_at(mx - x0 + self.scroll);
                if !shift {
                    self.anchor = self.cursor;
                }
                self.dragging = true;
            }
            self.last_click = now;
            touched = true;
        } else if self.dragging {
            if is_mouse_button_down(MouseButton::Left) {
                self.cursor = index_at(mx - x0 + self.scroll);
            } else {
                self.dragging = false;
            }
        }

        if touched {
            self.last_input = now;
        }
        self.scroll = self.visible_scroll(&widths, r.w - 2.0 * FIELD_PAD);
    }
}

pub fn draw_field(r: Rect, f: &Field, active: bool) {
    round_rect(r, 8.0, if active { FOCUS } else { INK });
    round_rect(inset(r, if active { 3.0 } else { 2.0 }), 6.0, WHITE);
    let size = field_size(r);
    let inner = r.w - 2.0 * FIELD_PAD;
    let x0 = r.x + FIELD_PAD;
    let w = f.widths(size);
    let sc = if active { f.visible_scroll(&w, inner) } else { 0.0 };
    let (a, b) = f.range();
    if active && a != b {
        let (xa, xb) = ((w[a] - sc).clamp(0.0, inner), (w[b] - sc).clamp(0.0, inner));
        draw_rectangle(x0 + xa, r.y + 5.0, xb - xa, r.h - 10.0, SELECTION);
    }
    // only whole characters inside the field are drawn
    let n = w.len() - 1;
    let first = (0..=n).find(|&i| w[i] >= sc - 0.5).unwrap_or(n);
    let last = (first..=n).rev().find(|&i| w[i] <= sc + inner + 0.5).unwrap_or(first);
    let shown: String = f.text.chars().skip(first).take(last - first).collect();
    text(&shown, x0 + w[first] - sc, (r.y + r.h / 2.0 + size * 0.3).round(), size, INK);
    let now = get_time();
    if active && (now - f.last_input < 0.6 || (now * 2.0) as i64 % 2 == 0) {
        draw_rectangle((x0 + w[f.cursor] - sc).round(), r.y + 5.0, 2.0, r.h - 10.0, INK);
    }
}

// ---- language flags ---------------------------------------------------------------------------------------------

/// Diagonal band of vertical half-extent `half` from corner to corner, drawn per pixel column so it stays inside `r`.
fn diagonal(r: Rect, rising: bool, half: f32, c: Color) {
    for i in 0..r.w as i32 {
        let x = i as f32 + 0.5;
        let center = if rising { r.h - x * r.h / r.w } else { x * r.h / r.w };
        let (y0, y1) = ((center - half).max(0.0), (center + half).min(r.h));
        if y1 > y0 {
            draw_rectangle(r.x + i as f32, r.y + y0, 1.0, y1 - y0, c);
        }
    }
}

fn draw_flag(lang: Lang, r: Rect) {
    let stripes = |cols: [Color; 3], w: [f32; 3]| {
        let mut y = r.y;
        for (c, f) in cols.into_iter().zip(w) {
            draw_rectangle(r.x, y, r.w, r.h * f, c);
            y += r.h * f;
        }
    };
    match lang {
        Lang::De => stripes([BLACK, Color::new(0.87, 0.0, 0.0, 1.0), Color::new(1.0, 0.81, 0.0, 1.0)], [1.0 / 3.0; 3]),
        Lang::Es => stripes([Color::new(0.78, 0.07, 0.13, 1.0), Color::new(1.0, 0.77, 0.0, 1.0), Color::new(0.78, 0.07, 0.13, 1.0)], [0.25, 0.5, 0.25]),
        Lang::En => {
            let red = Color::new(0.8, 0.07, 0.17, 1.0);
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.0, 0.14, 0.49, 1.0));
            for rising in [false, true] {
                diagonal(r, rising, r.h * 0.2, WHITE);
                diagonal(r, rising, r.h * 0.08, red);
            }
            draw_rectangle(r.x, r.y + r.h * 0.5 - r.h * 0.17, r.w, r.h * 0.34, WHITE);
            draw_rectangle(r.x + r.w * 0.5 - r.h * 0.17, r.y, r.h * 0.34, r.h, WHITE);
            draw_rectangle(r.x, r.y + r.h * 0.5 - r.h * 0.1, r.w, r.h * 0.2, red);
            draw_rectangle(r.x + r.w * 0.5 - r.h * 0.1, r.y, r.h * 0.2, r.h, red);
        }
    }
}

/// A language flag as a button; the selected one is framed, the others are dimmed until hovered. Returns a click.
pub fn flag_button(r: Rect, lang: Lang, selected: bool) -> bool {
    let hover = hovered(r);
    round_rect(r, 8.0, if selected { FOCUS } else { INK });
    let inner = inset(r, 3.0);
    draw_flag(lang, inner);
    if !selected && !hover {
        draw_rectangle(inner.x, inner.y, inner.w, inner.h, Color::new(1.0, 1.0, 1.0, 0.5));
    }
    clicked(r)
}

// ---- loading screen ---------------------------------------------------------------------------------------------

pub fn draw_loading(done: usize, total: usize) {
    clear_background(ICE);
    text_centered(tr(Key::GameTitle), 320.0, 250.0, 56.0, INK);
    let bar = Rect::new(120.0, 300.0, 400.0, 30.0);
    round_rect(bar, 15.0, INK);
    round_rect(inset(bar, 3.0), 12.0, WHITE);
    let frac = if total == 0 { 0.0 } else { (done as f32 / total as f32).min(1.0) };
    if frac > 0.0 {
        let inner = inset(bar, 5.0);
        let w = (inner.w * frac).max(inner.h);
        round_rect(Rect::new(inner.x, inner.y, w, inner.h), inner.h / 2.0, Color::new(0.2, 0.44, 0.8, 1.0));
        round_rect(Rect::new(inner.x + 2.0, inner.y + 2.0, (w - 4.0).max(2.0), inner.h / 2.0 - 2.0), 4.0, Color::new(0.32, 0.56, 0.9, 1.0));
    }
    let label = if total == 0 { tr(Key::Loading).to_string() } else { format!("{done} / {total}  ({}%)", (frac * 100.0).round() as u32) };
    text_centered(&label, 320.0, 365.0, 24.0, INK);
}

// ---- after-match screens ----------------------------------------------------------------------------------------

const AGAIN_BTN: Rect = Rect { x: 100.0, y: 380.0, w: 210.0, h: 46.0 };
const MENU_BTN: Rect = Rect { x: 330.0, y: 380.0, w: 210.0, h: 46.0 };

/// "Play again" and "back" buttons; returns which of them was clicked.
pub fn after_buttons(again_enabled: bool, menu_label: &str) -> (bool, bool) {
    (button(AGAIN_BTN, tr(Key::PlayAgain), Style::Primary, again_enabled), button(MENU_BTN, menu_label, Style::Normal, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(s: &str) -> Field {
        Field::new(s, 10, any_char)
    }

    #[test]
    fn typing_replaces_the_selection_and_respects_the_limit() {
        let mut f = field("hello");
        f.anchor = 1;
        f.cursor = 4;
        f.insert("XY");
        assert_eq!(f.text, "hXYo");
        assert_eq!(f.cursor, 3);
        f.insert("0123456789");
        assert_eq!(f.text.chars().count(), 10);
    }

    #[test]
    fn deleting_a_selection_and_words() {
        let mut f = field("ab cd ef");
        f.anchor = 3;
        f.cursor = 6;
        assert!(f.delete_selection());
        assert_eq!(f.text, "ab ef");
        assert_eq!(f.word_left(5), 3);
        assert_eq!(f.word_right(0), 3);
        f.delete_range(0, 3);
        assert_eq!(f.text, "ef");
    }

    #[test]
    fn filter_applies_to_pasted_text() {
        let mut f = Field::new("", 20, name_char);
        f.insert("a<b>\nc");
        assert_eq!(f.text, "abc");
    }
}
