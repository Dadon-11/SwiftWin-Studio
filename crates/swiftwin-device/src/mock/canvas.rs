//! Minimal RGBA software canvas used to draw the simulated iPhone screen.

use super::font::{glyph, GLYPH_H, GLYPH_W};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub struct Canvas {
    pub w: u32,
    pub h: u32,
    px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: u32, h: u32, bg: Rgb) -> Self {
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..w * h {
            px.extend_from_slice(&[bg.0, bg.1, bg.2]);
        }
        Canvas { w, h, px }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Rgb {
        let i = ((y * self.w + x) * 3) as usize;
        Rgb(self.px[i], self.px[i + 1], self.px[i + 2])
    }

    fn put(&mut self, x: i64, y: i64, c: Rgb) {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            return;
        }
        let i = ((y as u32 * self.w + x as u32) * 3) as usize;
        self.px[i] = c.0;
        self.px[i + 1] = c.1;
        self.px[i + 2] = c.2;
    }

    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Rgb) {
        self.fill_round_rect(x, y, w, h, 0.0, c);
    }

    pub fn fill_round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64, c: Rgb) {
        let (x0, y0) = (x.round() as i64, y.round() as i64);
        let (x1, y1) = ((x + w).round() as i64, (y + h).round() as i64);
        let r = r.min(w / 2.0).min(h / 2.0);
        for py in y0.max(0)..y1.min(self.h as i64) {
            for px in x0.max(0)..x1.min(self.w as i64) {
                if r > 0.0 && !inside_round(px as f64 + 0.5, py as f64 + 0.5, x, y, w, h, r) {
                    continue;
                }
                self.put(px, py, c);
            }
        }
    }

    pub fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Rgb) {
        self.fill_rect(x, y, w, 1.0, c);
        self.fill_rect(x, y + h - 1.0, w, 1.0, c);
        self.fill_rect(x, y, 1.0, h, c);
        self.fill_rect(x + w - 1.0, y, 1.0, h, c);
    }

    /// Width in pixels of `text` at integer `scale`.
    pub fn text_width(text: &str, scale: u32) -> f64 {
        let n = text.chars().count() as u32;
        if n == 0 {
            return 0.0;
        }
        (n * (GLYPH_W + 1) * scale - scale) as f64
    }

    pub fn text_height(scale: u32) -> f64 {
        (GLYPH_H * scale) as f64
    }

    pub fn draw_text(&mut self, x: f64, y: f64, text: &str, scale: u32, c: Rgb) {
        let mut cx = x.round() as i64;
        let cy = y.round() as i64;
        let s = scale as i64;
        for ch in text.chars() {
            let cols = glyph(ch);
            for (col, bits) in cols.iter().enumerate() {
                for row in 0..GLYPH_H as i64 {
                    if bits & (1 << row) != 0 {
                        for dy in 0..s {
                            for dx in 0..s {
                                self.put(cx + col as i64 * s + dx, cy + row * s + dy, c);
                            }
                        }
                    }
                }
            }
            cx += (GLYPH_W as i64 + 1) * s;
        }
    }

    /// Draw text horizontally centred inside a rect.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text_centered(&mut self, x: f64, y: f64, w: f64, h: f64, text: &str, scale: u32, c: Rgb) {
        let tw = Self::text_width(text, scale);
        let th = Self::text_height(scale);
        self.draw_text(x + (w - tw) / 2.0, y + (h - th) / 2.0, text, scale, c);
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.w, self.h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().expect("png header");
            wr.write_image_data(&self.px).expect("png data");
        }
        out
    }
}

fn inside_round(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64, r: f64) -> bool {
    let cx = px.clamp(x + r, x + w - r);
    let cy = py.clamp(y + r, y + h - r);
    let (dx, dy) = (px - cx, py - cy);
    dx * dx + dy * dy <= r * r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_and_encodes() {
        let mut c = Canvas::new(40, 20, Rgb(255, 255, 255));
        c.fill_rect(0.0, 0.0, 10.0, 10.0, Rgb(255, 0, 0));
        assert_eq!(c.pixel(5, 5), Rgb(255, 0, 0));
        assert_eq!(c.pixel(15, 5), Rgb(255, 255, 255));
        c.draw_text(12.0, 2.0, "I", 1, Rgb(0, 0, 0));
        // 'I' has its middle column (col 2) fully set.
        assert_eq!(c.pixel(14, 4), Rgb(0, 0, 0));
        let png = c.to_png();
        assert_eq!(swiftwin_core::png_dimensions(&png), Some((40, 20)));
        assert_eq!(Canvas::text_width("ab", 2), 22.0);
    }
}
