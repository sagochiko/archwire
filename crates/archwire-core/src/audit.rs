//! The final check of the finished lines.

use crate::layout::{FrameKind, Layout, Rect};
use crate::measure::{frame_name_box, name_area, Measure};
use crate::model::Model;
use crate::path::grow;
use crate::route::{Port, Side, Wire};

// The final check. It looks at the finished lines with its own arithmetic, not the router's, so
// a mistake in how the router measures or searches cannot hide itself. Its measures follow the
// diagram checker (tools/geocheck.py): text width per character, the label boxes, an icon's box
// shrunk by its blank margin, and two strokes this close together read as one.

const AUDIT_CHAR_ASCII: i32 = 7;
/// Latin letters in a line's label, widest first, by their width at font size 11. Characters not
/// listed count at `AUDIT_WIRE_UNLISTED`.
const AUDIT_WIRE_LATIN: [(&str, i32); 7] = [
    ("@", 12),
    ("W", 11),
    ("Mm%", 10),
    ("GOQw", 9),
    ("ABCDEFHKLNPRSTUVXYZ&", 8),
    ("abdeghnopqu0123456789#$?_+=<>~^\"*`{}\\", 7),
    ("ckJsvxyz", 6),
];
const AUDIT_WIRE_NARROW: &str = "fijlrtI()[]/.,:;-'!| ";
const AUDIT_WIRE_UNLISTED: i32 = 12;
const AUDIT_WIRE_NARROW_W: i32 = 4;
const AUDIT_CHAR_WIDE: i32 = 12;
const AUDIT_WIRE_WIDE: i32 = 11;
const AUDIT_WIRE_PAREN: i32 = 6;
const AUDIT_ICON_MARGIN: i32 = 2;
const AUDIT_STROKES_MERGE: i32 = 2;

type Seg = ((i32, i32), (i32, i32));

fn audit_text_width(text: &str) -> i32 {
    text.lines().map(|l| l.chars().map(|c| if c.is_ascii() { AUDIT_CHAR_ASCII } else { AUDIT_CHAR_WIDE }).sum()).max().unwrap_or(0)
}

/// A line's label is drawn at font size 11, where Latin letters differ in width and a full-width
/// character is narrower; a full-width parenthesis shows only half its width where it opens a line
/// or closes it.
pub(crate) fn audit_wire_text_width(text: &str) -> i32 {
    let latin = |c: char| {
        if AUDIT_WIRE_NARROW.contains(c) {
            return AUDIT_WIRE_NARROW_W;
        }
        AUDIT_WIRE_LATIN.iter().find(|(set, _)| set.contains(c)).map_or(AUDIT_WIRE_UNLISTED, |&(_, w)| w)
    };
    let mut widest = 0;
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut sum = 0;
        for (i, &c) in chars.iter().enumerate() {
            let at_edge = (c == '（' && i == 0) || (c == '）' && i == chars.len() - 1);
            sum += if c.is_ascii() {
                latin(c)
            } else if at_edge {
                AUDIT_WIRE_PAREN
            } else {
                AUDIT_WIRE_WIDE
            };
        }
        widest = widest.max(sum);
    }
    widest
}

/// True if the segment enters the inside of the box; touching its edge does not count.
fn audit_enters(s: Seg, r: Rect) -> bool {
    let ((x1, y1), (x2, y2)) = s;
    x1.min(x2) < r.x + r.w && x1.max(x2) > r.x && y1.min(y2) < r.y + r.h && y1.max(y2) > r.y
}

/// How long two segments run on top of each other: both vertical or both horizontal, at most
/// `AUDIT_STROKES_MERGE` apart, measured along their common stretch.
fn audit_overlap(s: Seg, t: Seg) -> i32 {
    let ((a1, a2), (b1, b2)) = (s, t);
    let common = |p: i32, q: i32, r: i32, u: i32| p.max(q).min(r.max(u)) - p.min(q).max(r.min(u));
    if a1.0 == a2.0 && b1.0 == b2.0 && (a1.0 - b1.0).abs() <= AUDIT_STROKES_MERGE {
        common(a1.1, a2.1, b1.1, b2.1)
    } else if a1.1 == a2.1 && b1.1 == b2.1 && (a1.1 - b1.1).abs() <= AUDIT_STROKES_MERGE {
        common(a1.0, a2.0, b1.0, b2.0)
    } else {
        0
    }
}

/// The drawn line: its exit, waypoints and entry, a diagonal step drawn as draw.io draws it.
fn audit_path(w: &Wire, layout: &Layout) -> Vec<(i32, i32)> {
    let end = |r: Rect, p: Port| {
        let f = p.pos as i32;
        match p.side {
            Side::Top => (r.x + r.w * f / 100, r.y - p.offset),
            Side::Bottom => (r.x + r.w * f / 100, r.y + r.h + p.offset),
            Side::Left => (r.x - p.offset, r.y + r.h * f / 100),
            Side::Right => (r.x + r.w + p.offset, r.y + r.h * f / 100),
        }
    };
    let mut raw = vec![end(layout.rect(&w.from), w.exit)];
    raw.extend(w.points.iter().copied());
    raw.push(end(layout.rect(&w.to), w.entry));
    let mut out = vec![raw[0]];
    for &p in &raw[1..] {
        let a = *out.last().expect("not empty");
        if a.0 != p.0 && a.1 != p.1 {
            out.push((p.0, a.1));
        }
        out.push(p);
    }
    out
}

/// The box a line's label takes, from where draw.io anchors it on the drawn line.
pub(crate) fn audit_wire_label(w: &Wire, path: &[(i32, i32)]) -> Option<Rect> {
    if w.label.is_empty() {
        return None;
    }
    let len: i32 = path.windows(2).map(|s| (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs()).sum();
    let mut d = match (w.label_at, w.label_near_source) {
        (Some(at), _) => ((at as i64 * len as i64 + 500) / 1000) as i32,
        (None, true) => len / 4,
        (None, false) => len / 2,
    };
    let mut anchor = path[path.len() - 1];
    for s in path.windows(2) {
        let l = (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs();
        if d <= l && l > 0 {
            anchor = (s[0].0 + (s[1].0 - s[0].0).signum() * d, s[0].1 + (s[1].1 - s[0].1).signum() * d);
            break;
        }
        d -= l;
    }
    let (w_px, h_px) = (audit_wire_text_width(&w.label), 13 * w.label.lines().count().max(1) as i32);
    let (x, y) = anchor;
    Some(match (w.horizontal_label, w.label_flip) {
        (true, false) => Rect { x: x - w_px / 2 - 2, y: y - h_px - 4, w: w_px + 4, h: h_px + 4 },
        (true, true) => Rect { x: x - w_px / 2 - 2, y, w: w_px + 4, h: h_px + 12 },
        (false, false) => Rect { x: x + 6, y: y - h_px - 1, w: w_px + 6, h: h_px + 3 },
        (false, true) => Rect { x: x - 12 - w_px, y: y - h_px - 1, w: w_px + 6, h: h_px + 3 },
    })
}

/// The least distance in pixels between a line and another line's label, as the audit reads the
/// drawn diagram, with the line's id and the id of the line whose label it is.
#[cfg(test)]
pub(crate) fn closest_line_to_label(layout: &Layout, wires: &[Wire]) -> Option<(i32, String, String)> {
    let paths: Vec<Vec<(i32, i32)>> = wires.iter().map(|w| audit_path(w, layout)).collect();
    let mut best: Option<(i32, String, String)> = None;
    for (m, (w, p)) in wires.iter().zip(&paths).enumerate() {
        let Some(r) = audit_wire_label(w, p) else { continue };
        for (n, q) in paths.iter().enumerate().filter(|&(n, _)| n != m) {
            for s in q.windows(2) {
                let (x1, x2) = (s[0].0.min(s[1].0), s[0].0.max(s[1].0));
                let (y1, y2) = (s[0].1.min(s[1].1), s[0].1.max(s[1].1));
                let dx = (r.x - x2).max(x1 - (r.x + r.w)).max(0);
                let dy = (r.y - y2).max(y1 - (r.y + r.h)).max(0);
                let d = dx.max(dy);
                if best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, wires[n].id.clone(), w.id.clone()));
                }
            }
        }
    }
    best
}

/// Lines that run over text (an icon's label, a frame's name, another line's label), over an
/// icon they do not attach to, or on top of another line. Ids in line order, each once.
/// The VPC layout is checked with its own measure of names, written apart from the router's; the
/// layout without a VPC with the measure it is laid out by, which is never narrower than draw.io draws.
pub(crate) fn audit(_model: &Model, layout: &Layout, wires: &[Wire], measure: Measure) -> Vec<String> {
    let icon_labels: Vec<(&str, Rect)> = layout
        .icons
        .iter()
        .map(|i| {
            if measure == Measure::Flow {
                return (i.id.as_str(), name_area(i));
            }
            let half = (audit_text_width(&i.label) + 1) / 2;
            (i.id.as_str(), Rect { x: i.rect.x + i.rect.w / 2 - half, y: i.rect.y + i.rect.h + 4, w: 2 * half, h: 14 * i.lines as i32 + 6 })
        })
        .collect();
    let frame_names: Vec<Rect> = layout
        .frames
        .iter()
        .map(|f| {
            if measure == Measure::Flow {
                return frame_name_box(f);
            }
            let (indent, h) = if f.kind == FrameKind::Az { (10, 20) } else { (30, 25) };
            Rect { x: f.rect.x, y: f.rect.y, w: indent + audit_text_width(&f.label), h }
        })
        .collect();
    let paths: Vec<Vec<(i32, i32)>> = wires.iter().map(|w| audit_path(w, layout)).collect();
    let room = measure.label_margin(0);
    let labels: Vec<Option<Rect>> = wires.iter().zip(&paths).map(|(w, p)| audit_wire_label(w, p).map(|r| grow(r, room))).collect();
    let mut bad = vec![false; wires.len()];
    for (n, (w, p)) in wires.iter().zip(&paths).enumerate() {
        let last = p.len() - 2;
        for (k, s) in p.windows(2).map(|s| (s[0], s[1])).enumerate() {
            let own = |id: &str| (k == 0 && id == w.from) || (k == last && id == w.to);
            let over_icon = layout.icons.iter().any(|i| {
                i.id != w.from && i.id != w.to && audit_enters(s, Rect { x: i.rect.x + AUDIT_ICON_MARGIN, y: i.rect.y + AUDIT_ICON_MARGIN, w: i.rect.w - 2 * AUDIT_ICON_MARGIN, h: i.rect.h - 2 * AUDIT_ICON_MARGIN })
            });
            let over_text = icon_labels.iter().any(|&(id, r)| !own(id) && audit_enters(s, r))
                || frame_names.iter().any(|&r| audit_enters(s, r))
                || labels.iter().enumerate().any(|(m, r)| m != n && r.is_some_and(|r| audit_enters(s, r)));
            if over_icon || over_text {
                bad[n] = true;
            }
            for (m, q) in paths.iter().enumerate().skip(n + 1) {
                if q.windows(2).any(|t| audit_overlap(s, (t[0], t[1])) > 1) {
                    bad[n] = true;
                    bad[m] = true;
                }
            }
        }
    }
    wires.iter().zip(bad).filter(|(_, b)| *b).map(|(w, _)| w.id.clone()).collect()
}
