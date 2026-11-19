//! Minimal static SVG charts for the paper's figures.
//!
//! Small multiples of line or dot-and-whisker panels on one shared legend.
//! Light mode only (journal print). Identity never rests on colour alone:
//! each series also has a dash pattern and a marker shape, and series are
//! labelled directly at their last point when there are four or fewer.
//! Colours: the validated categorical slots 1 to 3 of the dataviz default
//! palette. No function here reads data.

use std::fmt::Write as _;

pub const SURFACE: &str = "#ffffff";
pub const INK: &str = "#0b0b0b";
pub const INK_2: &str = "#52514e";
pub const MUTED: &str = "#898781";
pub const GRID: &str = "#e1e0d9";
pub const AXIS: &str = "#c3c2b7";
pub const BAND: &str = "#f0efec";
pub const SERIES: [&str; 3] = ["#2a78d6", "#eb6834", "#1baf7a"];
const DASH: [&str; 3] = ["", "6 3", "2 3"];

#[derive(Clone, Copy)]
pub enum Marker {
    Circle,
    Square,
    Triangle,
}
const MARKERS: [Marker; 3] = [Marker::Circle, Marker::Square, Marker::Triangle];

#[derive(Clone)]
pub struct Series {
    pub name: String,
    /// Slot 0, 1 or 2: colour, dash and marker.
    pub slot: usize,
    pub points: Vec<(f64, f64)>,
    /// Optional interval for each point.
    pub interval: Option<Vec<(f64, f64)>>,
    /// Draw a line through the points.
    pub line: bool,
}

#[derive(Clone, Default)]
pub struct Panel {
    pub title: String,
    pub x_label: String,
    pub y_label: String,
    pub series: Vec<Series>,
    /// Shaded vertical bands: (from, to, label).
    pub bands: Vec<(f64, f64, String)>,
    /// Horizontal reference lines: (y, label).
    pub hlines: Vec<(f64, String)>,
    /// Tick positions and labels on x; empty means automatic.
    pub x_ticks: Vec<(f64, String)>,
    pub y_range: Option<(f64, f64)>,
    pub x_range: Option<(f64, f64)>,
    /// Draw direct labels at the last point of each series.
    pub direct_labels: bool,
}

fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let span = (hi - lo).max(1e-12);
    let raw = span / target as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| span / s <= target as f64).unwrap_or(10.0 * mag);
    let start = (lo / step).ceil() * step;
    let mut t = Vec::new();
    let mut v = start;
    while v <= hi + 1e-9 * step {
        t.push((v / step).round() * step);
        v += step;
    }
    t
}

fn fmt_tick(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else if (v * 10.0 - (v * 10.0).round()).abs() < 1e-9 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn marker(out: &mut String, m: Marker, x: f64, y: f64, color: &str) {
    let r = 4.0;
    match m {
        Marker::Circle => {
            let _ = write!(out, r#"<circle cx="{x:.1}" cy="{y:.1}" r="{r}" fill="{color}" stroke="{SURFACE}" stroke-width="1.5"/>"#);
        }
        Marker::Square => {
            let _ = write!(out, r#"<rect x="{:.1}" y="{:.1}" width="{}" height="{}" fill="{color}" stroke="{SURFACE}" stroke-width="1.5"/>"#, x - r, y - r, 2.0 * r, 2.0 * r);
        }
        Marker::Triangle => {
            let _ = write!(
                out,
                r#"<polygon points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}" fill="{color}" stroke="{SURFACE}" stroke-width="1.5"/>"#,
                x, y - r * 1.2, x - r * 1.1, y + r * 0.8, x + r * 1.1, y + r * 0.8
            );
        }
    }
}

/// Render panels in a grid with `cols` columns. `legend` lists the series
/// names by slot (shown when two or more series exist).
pub fn render(title: &str, panels: &[Panel], cols: usize, legend: &[(usize, String)]) -> String {
    render_sized(title, panels, cols, legend, 340.0, 250.0)
}

/// As `render`, with the size of each panel given.
pub fn render_sized(title: &str, panels: &[Panel], cols: usize, legend: &[(usize, String)], pw: f64, ph: f64) -> String {
    let (ml, mr, mt, mb) = (52.0, 70.0, 30.0, 44.0);
    let rows = panels.len().div_ceil(cols);
    let top = if legend.len() >= 2 { 58.0 } else { 34.0 };
    let width = cols as f64 * pw;
    let height = top + rows as f64 * ph;
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" font-family="Helvetica, Arial, sans-serif"><rect width="100%" height="100%" fill="{SURFACE}"/>"#
    );
    let _ = write!(s, r#"<text x="8" y="20" font-size="14" font-weight="600" fill="{INK}">{}</text>"#, esc(title));
    if legend.len() >= 2 {
        let mut x = 8.0;
        for (slot, name) in legend {
            let c = SERIES[*slot];
            let _ = write!(s, r#"<line x1="{x}" y1="40" x2="{}" y2="40" stroke="{c}" stroke-width="2" stroke-dasharray="{}"/>"#, x + 22.0, DASH[*slot]);
            marker(&mut s, MARKERS[*slot], x + 11.0, 40.0, c);
            let _ = write!(s, r#"<text x="{}" y="44" font-size="11" fill="{INK_2}">{}</text>"#, x + 28.0, esc(name));
            x += 40.0 + 6.5 * name.len() as f64;
        }
    }
    for (i, p) in panels.iter().enumerate() {
        let ox = (i % cols) as f64 * pw;
        let oy = top + (i / cols) as f64 * ph;
        let (x0, x1) = (ox + ml, ox + pw - mr);
        let (y0, y1) = (oy + mt, oy + ph - mb);
        let all_x: Vec<f64> = p.series.iter().flat_map(|q| q.points.iter().map(|p| p.0)).collect();
        let mut all_y: Vec<f64> = p.series.iter().flat_map(|q| q.points.iter().map(|p| p.1)).collect();
        for q in &p.series {
            if let Some(iv) = &q.interval {
                all_y.extend(iv.iter().flat_map(|(a, b)| [*a, *b]));
            }
        }
        all_y.extend(p.hlines.iter().map(|h| h.0));
        let (xa, xb) = p.x_range.unwrap_or_else(|| {
            let a = all_x.iter().copied().fold(f64::INFINITY, f64::min);
            let b = all_x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let pad = 0.04 * (b - a).max(1.0);
            (a - pad, b + pad)
        });
        let (ya, yb) = p.y_range.unwrap_or_else(|| {
            let a = all_y.iter().copied().fold(f64::INFINITY, f64::min).min(0.0);
            let b = all_y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            (a, b + 0.06 * (b - a).max(1e-9))
        });
        let sx = |x: f64| x0 + (x - xa) / (xb - xa) * (x1 - x0);
        let sy = |y: f64| y1 - (y - ya) / (yb - ya) * (y1 - y0);
        let _ = write!(s, r#"<text x="{x0}" y="{}" font-size="12" font-weight="600" fill="{INK}">{}</text>"#, oy + 18.0, esc(&p.title));
        for (k, (a, b, label)) in p.bands.iter().enumerate() {
            // Successive bands get darker shades, with a rule at each start.
            let shade = if k % 2 == 0 { BAND } else { GRID };
            let _ = write!(s, r#"<rect x="{:.1}" y="{y0}" width="{:.1}" height="{:.1}" fill="{shade}"/>"#, sx(*a), sx(*b) - sx(*a), y1 - y0);
            let _ = write!(s, r#"<line x1="{:.1}" y1="{y0}" x2="{:.1}" y2="{y1}" stroke="{AXIS}" stroke-width="1"/>"#, sx(*a), sx(*a));
            let _ = write!(s, r#"<text x="{:.1}" y="{}" font-size="10" fill="{INK_2}">{}</text>"#, sx(*a) + 4.0, y0 + 12.0, esc(label));
        }
        for t in nice_ticks(ya, yb, 5) {
            let y = sy(t);
            let _ = write!(s, r#"<line x1="{x0}" y1="{y:.1}" x2="{x1}" y2="{y:.1}" stroke="{GRID}" stroke-width="1"/>"#);
            let _ = write!(s, r#"<text x="{}" y="{:.1}" font-size="10" fill="{MUTED}" text-anchor="end">{}</text>"#, x0 - 6.0, y + 3.5, fmt_tick(t));
        }
        let xt: Vec<(f64, String)> = if p.x_ticks.is_empty() {
            nice_ticks(xa, xb, 6).into_iter().map(|t| (t, fmt_tick(t))).collect()
        } else {
            p.x_ticks.clone()
        };
        for (t, label) in &xt {
            let _ = write!(s, r#"<text x="{:.1}" y="{}" font-size="10" fill="{MUTED}" text-anchor="middle">{}</text>"#, sx(*t), y1 + 14.0, esc(label));
        }
        let _ = write!(s, r#"<line x1="{x0}" y1="{:.1}" x2="{x1}" y2="{:.1}" stroke="{AXIS}" stroke-width="1"/>"#, sy(ya.max(0.0).min(yb)), sy(ya.max(0.0).min(yb)));
        let _ = write!(s, r#"<text x="{:.1}" y="{}" font-size="10.5" fill="{INK_2}" text-anchor="middle">{}</text>"#, 0.5 * (x0 + x1), y1 + 32.0, esc(&p.x_label));
        let _ = write!(s, r#"<text transform="translate({},{:.1}) rotate(-90)" font-size="10.5" fill="{INK_2}" text-anchor="middle">{}</text>"#, ox + 14.0, 0.5 * (y0 + y1), esc(&p.y_label));
        for (y, label) in &p.hlines {
            let _ = write!(s, r#"<line x1="{x0}" y1="{:.1}" x2="{x1}" y2="{:.1}" stroke="{INK_2}" stroke-width="1" stroke-dasharray="3 3"/>"#, sy(*y), sy(*y));
            let _ = write!(s, r#"<text x="{}" y="{:.1}" font-size="10" fill="{INK_2}">{}</text>"#, x1 + 4.0, sy(*y) + 3.5, esc(label));
        }
        for q in &p.series {
            let c = SERIES[q.slot];
            if let Some(iv) = &q.interval {
                for ((x, _), (a, b)) in q.points.iter().zip(iv) {
                    let _ = write!(s, r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{c}" stroke-width="1.5"/>"#, sx(*x), sy(*a), sx(*x), sy(*b));
                }
            }
            if q.line && q.points.len() > 1 {
                let d: Vec<String> = q.points.iter().map(|(x, y)| format!("{:.1},{:.1}", sx(*x), sy(*y))).collect();
                let _ = write!(s, r#"<polyline points="{}" fill="none" stroke="{c}" stroke-width="2" stroke-dasharray="{}" stroke-linejoin="round"/>"#, d.join(" "), DASH[q.slot]);
            }
            for (x, y) in &q.points {
                marker(&mut s, MARKERS[q.slot], sx(*x), sy(*y), c);
            }
            if p.direct_labels {
                if let Some((x, y)) = q.points.last() {
                    let _ = write!(s, r#"<text x="{:.1}" y="{:.1}" font-size="10" fill="{INK_2}">{}</text>"#, sx(*x) + 7.0, sy(*y) + 3.5, esc(&q.name));
                }
            }
        }
    }
    s.push_str("</svg>");
    s
}
