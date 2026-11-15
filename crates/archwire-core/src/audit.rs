//! The final check of the finished lines.

use crate::layout::{FrameKind, Layout, Rect};
use crate::measure::{frame_name_box, name_area, Measure};
use crate::model::Model;
use crate::path::grow;
use crate::route::{Bend, Keep, LinePlan, Port, Side, Track, Wire};

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

/// Where draw.io puts a line's end on an icon.
fn audit_end(r: Rect, p: Port) -> (i32, i32) {
    let f = p.pos as i32;
    match p.side {
        Side::Top => (r.x + r.w * f / 100, r.y - p.offset),
        Side::Bottom => (r.x + r.w * f / 100, r.y + r.h + p.offset),
        Side::Left => (r.x - p.offset, r.y + r.h * f / 100),
        Side::Right => (r.x + r.w + p.offset, r.y + r.h * f / 100),
    }
}

/// The line as written: its exit, waypoints and entry.
fn audit_written(w: &Wire, layout: &Layout) -> Vec<(i32, i32)> {
    let mut raw = vec![audit_end(layout.rect(&w.from), w.exit)];
    raw.extend(w.points.iter().copied());
    raw.push(audit_end(layout.rect(&w.to), w.entry));
    raw
}

/// The drawn line: its exit, waypoints and entry, a diagonal step drawn as draw.io draws it.
fn audit_path(w: &Wire, layout: &Layout) -> Vec<(i32, i32)> {
    let raw = audit_written(w, layout);
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

/// Drops a point repeating the one before; keeps every other point, a straight run's middle included.
fn audit_points(raw: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out: Vec<(i32, i32)> = Vec::new();
    for &p in raw {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

/// True if the path runs straight on and then back over itself somewhere: three points on one
/// line, the middle one not between the others.
fn audit_turns_back(path: &[(i32, i32)]) -> bool {
    path.windows(3).any(|w| {
        let (a, b, c) = (w[0], w[1], w[2]);
        let between = |p: i32, q: i32, r: i32| p.min(r) <= q && q <= p.max(r);
        (a.0 == b.0 && b.0 == c.0 && !between(a.1, b.1, c.1)) || (a.1 == b.1 && b.1 == c.1 && !between(a.0, b.0, c.0))
    })
}

/// True if `want` appears in `path` in order, other points allowed between.
fn audit_in_order(path: &[(i32, i32)], want: &[(i32, i32)]) -> bool {
    let mut rest = path.iter();
    want.iter().all(|p| rest.any(|q| q == p))
}

/// True if the drawn line runs along the track's whole stretch, at its coordinate.
fn audit_on_track(path: &[(i32, i32)], t: &Track) -> bool {
    let mut spans: Vec<(i32, i32)> = path
        .windows(2)
        .filter_map(|s| {
            let ((x1, y1), (x2, y2)) = (s[0], s[1]);
            match t.horizontal {
                true if y1 == t.at && y2 == t.at && x1 != x2 => Some((x1.min(x2), x1.max(x2))),
                false if x1 == t.at && x2 == t.at && y1 != y2 => Some((y1.min(y2), y1.max(y2))),
                _ => None,
            }
        })
        .collect();
    spans.sort_unstable();
    let (lo, hi) = (t.from.min(t.to), t.from.max(t.to));
    let mut reached = lo;
    for (a, b) in spans {
        if a <= reached {
            reached = reached.max(b);
        }
    }
    reached >= hi
}

/// True if the line keeps its draft: a line kept whole has the draft's ends and every corner of
/// its draft, compared as written; a line kept inside has its inner end where the draft puts it
/// and passes the gate and the inner points in order; a kept line never turns back over itself;
/// every track's stretch lies on its band.
fn audit_keeps_draft(w: &Wire, p: &LinePlan, layout: &Layout) -> bool {
    let at = |port: Port, side: Side, pos: Option<u8>| port.side == side && port.pos == pos.unwrap_or(50);
    let written = audit_written(w, layout);
    let kept = match &p.keep {
        Keep::Free => true,
        Keep::Whole => {
            let (ep, np) = (written[0], written[written.len() - 1]);
            let bend: Vec<(i32, i32)> = match &p.bend {
                Bend::Straight => Vec::new(),
                Bend::AtX(x) => vec![(*x, ep.1), (*x, np.1)],
                Bend::AtY(y) => vec![(ep.0, *y), (np.0, *y)],
                Bend::Elbow if matches!(p.exit, Side::Top | Side::Bottom) => vec![(ep.0, np.1)],
                Bend::Elbow => vec![(np.0, ep.1)],
                Bend::Points(v) => v.clone(),
            };
            let mut planned = vec![ep];
            planned.extend(bend);
            planned.push(np);
            at(w.exit, p.exit, p.exit_pos) && at(w.entry, p.entry, p.entry_pos) && audit_points(&written) == audit_points(&planned)
        }
        Keep::Inner(t) => {
            let mut want: Vec<(i32, i32)> = Vec::new();
            if t.inner_is_start {
                want.extend(t.inner.iter().copied());
                want.push(t.gate);
            } else {
                want.push(t.gate);
                want.extend(t.inner.iter().copied());
            }
            let end = if t.inner_is_start { at(w.exit, p.exit, p.exit_pos) } else { at(w.entry, p.entry, p.entry_pos) };
            end && audit_in_order(&audit_points(&written), &audit_points(&want))
        }
    };
    let path = audit_path(w, layout);
    let turns_back = p.keep != Keep::Free && audit_turns_back(&audit_points(&path));
    kept && !turns_back && p.tracks.iter().all(|t| audit_on_track(&path, t))
}

/// Lines that run over text (an icon's label, a frame's name, another line's label), over an
/// icon they do not attach to, or on top of another line, or that break their drafts in `plans`
/// (a kept part moved, a gate missed, a track left); a line is matched to the draft of the
/// semantic edge it stands for. Ids in line order, each once.
/// The VPC layout is checked with its own measure of names, written apart from the router's; the
/// layout without a VPC with the measure it is laid out by, which is never narrower than draw.io draws.
pub(crate) fn audit(_model: &Model, layout: &Layout, wires: &[Wire], plans: &[LinePlan], measure: Measure) -> Vec<String> {
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
    for (n, w) in wires.iter().enumerate() {
        if plans.iter().find(|p| p.edge == w.rep).is_some_and(|p| !audit_keeps_draft(w, p, layout)) {
            bad[n] = true;
        }
    }
    wires.iter().zip(bad).filter(|(_, b)| *b).map(|(w, _)| w.id.clone()).collect()
}
