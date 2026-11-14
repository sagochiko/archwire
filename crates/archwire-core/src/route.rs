//! Routing: how every line is drawn. For each line it decides where it leaves and
//! enters an icon, its waypoints, its label, and which semantic edges it stands for.
//!
//! Routes come out right by construction. The usual shapes of the diagram (the main
//! column, the load balancer fanning out into the zones, the lines between per-zone
//! parts, the NAT gateway lines, the lines to Regional services) run through the gaps
//! between frames. Every route is checked against the icons, their labels, the frame
//! names and the lines already placed. A route that fails the check is searched for
//! again over the same gaps, and a line no search can place is named in a note.
//!
//! A line's label stays at the line's middle unless a frame side cuts it there or something
//! covers it; then it moves along the line to the nearest stretch where nothing does.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use crate::def::Rel;
use crate::layout::{Frame, FrameKind, Icon, Layout, Rect};
use crate::model::{Model, Node};
use crate::rules::{Rules, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side { Top, Bottom, Left, Right }

/// `pos` is 25, 50 or 75 percent along the side; `offset` moves the point outward from the side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port { pub side: Side, pub pos: u8, pub offset: i32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireStyle { Main, Aux, Assoc, Hole }

#[derive(Debug, Clone, PartialEq)]
pub struct Wire {
    /// "edge-<from>-to-<to>" of the representative edge.
    pub id: String,
    pub from: String,
    pub to: String,
    pub style: WireStyle,
    /// Display text in the model's language.
    pub label: String,
    pub exit: Port,
    pub entry: Port,
    /// Waypoints between the exit and the entry, absolute coordinates.
    pub points: Vec<(i32, i32)>,
    pub horizontal_label: bool,
    /// The label sits toward the source end instead of the middle: the line from the Internet
    /// Gateway down to the load balancer, whose middle is level with the AZ frame names.
    pub label_near_source: bool,
    /// Where the label sits along the line, in thousandths of the line's length from the source,
    /// when the usual spot is cut by a frame side or covered by something else; None keeps the usual spot.
    pub label_at: Option<u16>,
    /// The label sits on the other side of the line: below a horizontal label, left of one beside the line.
    pub label_flip: bool,
    /// The semantic edges this line stands for, in model order; just its own edge if it merges none.
    pub represents: Vec<(String, String)>,
}

/// Where a port sits on an icon's rectangle.
pub fn port_point(r: Rect, p: Port) -> (i32, i32) {
    let along = |len: i32| len * p.pos as i32 / 100;
    match p.side {
        Side::Top => (r.x + along(r.w), r.y - p.offset),
        Side::Bottom => (r.x + along(r.w), r.y + r.h + p.offset),
        Side::Left => (r.x - p.offset, r.y + along(r.h)),
        Side::Right => (r.x + r.w + p.offset, r.y + along(r.h)),
    }
}

/// How far a port on the bottom side moves down to clear the label: the first line, then each further line.
const LABEL_FIRST: i32 = 24;
const LABEL_LINE: i32 = 14;
/// An icon's label area starts this far below the icon and is this much taller than its lines.
const LABEL_GAP: i32 = 4;
const LABEL_PAD: i32 = 6;
/// Text width per character at font size 12.
const CHAR_ASCII: i32 = 7;
const CHAR_WIDE: i32 = 12;
/// Text width of a full-width character, and of a full-width parenthesis, at font size 11.
const WIRE_CHAR_WIDE: i32 = 11;
const WIRE_CHAR_PAREN: i32 = 6;
/// Frame names sit at the frame's top-left corner, after the group icon or a small indent.
const FRAME_NAME_INDENT: i32 = 30;
const FRAME_NAME_H: i32 = 25;
const AZ_NAME_INDENT: i32 = 10;
const AZ_NAME_H: i32 = 20;
/// Ends closer in height than this count as level: the label goes above the line, centred.
const LEVEL: i32 = 30;
/// With two AZs, a line to the other AZ's part turns this far below the source's top, by source AZ.
const MESH_CHANNELS: [i32; 2] = [110, 155];
/// Two lines may run together for at most this long.
const MAX_SHARED: i32 = 1;
/// Parallel lines closer than this many pixels count as running together.
const STROKES_APART: i32 = 3;
/// Search costs on top of length.
const BEND_COST: i64 = 100;
const CROSS_COST: i64 = 30;
/// Crossing a frame's side costs this much, so searched lines keep out of frames they do not need;
/// little enough that a line between parts in one row runs inside the frames, not around below them.
const BORDER_COST: i64 = 15;
/// Searched lines keep at least this far from icons, labels and frame names.
const CLEARANCE: i32 = 8;
/// Search channels: the outermost lie this far beyond everything; wide gaps get two more lanes this far from the middle.
const OUTER: i32 = 20;
const WIDE_GAP: i32 = 32;
const LANE: i32 = 8;
/// A line's label: the height of each text line, and how far beside the line its area starts.
const WIRE_LABEL_LINE: i32 = 13;
const WIRE_LABEL_BESIDE: i32 = 6;
/// Spots tried for a moved label are this far apart along the line; the spots in between are tried after them.
const LABEL_STEP: i32 = 2;
/// A line's label keeps at least this far from frame sides, icons and other text; a moved label
/// looks for a spot with this much room first.
const LABEL_MARGIN: i32 = 1;
const LABEL_ROOM: i32 = 4;

const NAT: &str = "nat";
const IGW: &str = "igw";

/// Lines are placed in this order, so the shapes the diagram is read by get their channels first.
const RANK_MAIN: u8 = 0;
const RANK_NAT: u8 = 1;
const RANK_SIDE: u8 = 2;
const RANK_AUTO: u8 = 3;

type Pt = (i32, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir { Up, Down, Left, Right }

const DIRS: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];
const ALL_SIDES: [Side; 4] = [Side::Top, Side::Bottom, Side::Left, Side::Right];

fn outward(s: Side) -> Dir {
    match s {
        Side::Top => Dir::Up,
        Side::Bottom => Dir::Down,
        Side::Left => Dir::Left,
        Side::Right => Dir::Right,
    }
}

fn reverse(d: Dir) -> Dir {
    match d {
        Dir::Up => Dir::Down,
        Dir::Down => Dir::Up,
        Dir::Left => Dir::Right,
        Dir::Right => Dir::Left,
    }
}

fn vertical(d: Dir) -> bool {
    matches!(d, Dir::Up | Dir::Down)
}

fn opposite(s: Side) -> Side {
    match s {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

fn dir_of(a: Pt, b: Pt) -> Option<Dir> {
    match (a.0 == b.0, a.1 == b.1) {
        (true, false) => Some(if b.1 > a.1 { Dir::Down } else { Dir::Up }),
        (false, true) => Some(if b.0 > a.0 { Dir::Right } else { Dir::Left }),
        _ => None,
    }
}

fn center(r: Rect) -> Pt {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// The left or right side of `a` that faces `b`.
fn facing(a: Rect, b: Rect) -> Side {
    if center(b).0 >= center(a).0 { Side::Right } else { Side::Left }
}

fn grow(r: Rect, by: i32) -> Rect {
    Rect { x: r.x - by, y: r.y - by, w: r.w + 2 * by, h: r.h + 2 * by }
}

/// True if the axis-aligned segment passes through the inside of the rectangle; touching its edge is not a hit.
fn hits(a: Pt, b: Pt, r: Rect) -> bool {
    let (x1, x2) = (a.0.min(b.0), a.0.max(b.0));
    let (y1, y2) = (a.1.min(b.1), a.1.max(b.1));
    x1 < r.x + r.w && x2 > r.x && y1 < r.y + r.h && y2 > r.y
}

/// How long two collinear segments run together, or 0.
fn shared(a: Pt, b: Pt, c: Pt, d: Pt) -> i32 {
    let span = |p: i32, q: i32, r: i32, s: i32| (p.max(q).min(r.max(s)) - p.min(q).max(r.min(s))).max(0);
    if a.0 == b.0 && c.0 == d.0 && a.0 == c.0 {
        span(a.1, b.1, c.1, d.1)
    } else if a.1 == b.1 && c.1 == d.1 && a.1 == c.1 {
        span(a.0, b.0, c.0, d.0)
    } else {
        0
    }
}

/// Like `shared`, for parallel segments up to `STROKES_APART` apart.
fn alongside(a: Pt, b: Pt, c: Pt, d: Pt) -> i32 {
    (-STROKES_APART..=STROKES_APART)
        .map(|o| {
            let shift = |p: Pt| if a.0 == b.0 { (p.0 + o, p.1) } else { (p.0, p.1 + o) };
            shared(shift(a), shift(b), c, d)
        })
        .max()
        .unwrap_or(0)
}

/// True if a vertical and a horizontal segment cross inside both.
fn crosses(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let cross = |v: (Pt, Pt), h: (Pt, Pt)| {
        let (x, y) = (v.0 .0, h.0 .1);
        v.0 .0 == v.1 .0 && h.0 .1 == h.1 .1
            && h.0 .0.min(h.1 .0) < x && x < h.0 .0.max(h.1 .0)
            && v.0 .1.min(v.1 .1) < y && y < v.0 .1.max(v.1 .1)
    };
    cross((a, b), (c, d)) || cross((c, d), (a, b))
}

/// Drops repeated points and points in the middle of a straight run.
fn simplify(raw: Vec<Pt>) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::new();
    for p in raw {
        if out.last() == Some(&p) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            if (a.0 == b.0 && b.0 == p.0) || (a.1 == b.1 && b.1 == p.1) {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

fn text_width(text: &str) -> i32 {
    text.lines()
        .map(|l| l.chars().map(|c| if c.is_ascii() { CHAR_ASCII } else { CHAR_WIDE }).sum::<i32>())
        .max()
        .unwrap_or(0)
}

/// Width of a Latin character in a line's label at font size 11, a pixel or so over what draw.io
/// draws: wide capitals and m, w; other capitals; most small letters, digits and symbols; the
/// narrower small letters; and the narrow letters, punctuation and space. A character not listed
/// counts at the widest width, so it is never counted narrower than it is drawn.
fn wire_latin_width(c: char) -> i32 {
    match c {
        '@' => 12,
        'W' => 11,
        'M' | 'm' | '%' => 10,
        'G' | 'O' | 'Q' | 'w' => 9,
        'A'..='Z' | '&' if !matches!(c, 'I' | 'J') => 8,
        'a' | 'b' | 'd' | 'e' | 'g' | 'h' | 'n' | 'o' | 'p' | 'q' | 'u' | '0'..='9' => 7,
        '#' | '$' | '?' | '_' | '+' | '=' | '<' | '>' | '~' | '^' | '"' | '*' | '`' | '{' | '}' | '\\' => 7,
        'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' | 'J' => 6,
        'f' | 'i' | 'j' | 'l' | 'r' | 't' | 'I' | '(' | ')' | '[' | ']' | '/' | '.' | ',' | ':' | ';' | '-' | '\'' | '!' | '|' | ' ' => 4,
        _ => 12,
    }
}

/// Text width of a line's label, drawn at font size 11: Latin text letter by letter, other text at
/// the width of a full-width character. A full-width parenthesis is drawn at full width inside a
/// line; an opening one at the start of a line and a closing one at its end show only half, and
/// count at half plus a pixel.
fn wire_text_width(text: &str) -> i32 {
    let line_width = |l: &str| {
        let n = l.chars().count();
        l.chars()
            .enumerate()
            .map(|(i, c)| match c {
                c if c.is_ascii() => wire_latin_width(c),
                '（' if i == 0 => WIRE_CHAR_PAREN,
                '）' if i + 1 == n => WIRE_CHAR_PAREN,
                _ => WIRE_CHAR_WIDE,
            })
            .sum::<i32>()
    };
    text.lines().map(line_width).max().unwrap_or(0)
}

/// The area an icon's label takes: centred under the icon.
fn label_area(i: &Icon) -> Rect {
    let half = (text_width(&i.label) + 1) / 2;
    Rect {
        x: i.rect.x + i.rect.w / 2 - half,
        y: i.rect.y + i.rect.h + LABEL_GAP,
        w: 2 * half,
        h: LABEL_LINE * i.lines as i32 + LABEL_PAD,
    }
}

/// The area a line's label takes when anchored at `p`, as draw.io draws it at font size 11:
/// centred above the anchor if `above` (below it if `flip`), else beside it on the right (on the
/// left if `flip`) with its last line ending at the anchor. Measured from rendered diagrams, with
/// a pixel or two to spare.
fn wire_label_area(text: &str, p: Pt, above: bool, flip: bool) -> Rect {
    let w = wire_text_width(text);
    let lines = WIRE_LABEL_LINE * text.lines().count().max(1) as i32;
    match (above, flip) {
        (true, false) => Rect { x: p.0 - w / 2 - 2, y: p.1 - lines - 4, w: w + 4, h: lines + 4 },
        (true, true) => Rect { x: p.0 - w / 2 - 2, y: p.1, w: w + 4, h: lines + 12 },
        (false, false) => Rect { x: p.0 + WIRE_LABEL_BESIDE, y: p.1 - lines - 1, w: w + WIRE_LABEL_BESIDE, h: lines + 3 },
        (false, true) => Rect { x: p.0 - 2 * WIRE_LABEL_BESIDE - w, y: p.1 - lines - 1, w: w + WIRE_LABEL_BESIDE, h: lines + 3 },
    }
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// The point `d` along a path from its start, and whether the segment it lies on is horizontal.
fn point_along(path: &[Pt], mut d: i32) -> (Pt, bool) {
    for s in path.windows(2) {
        let len = (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs();
        if d <= len && len > 0 {
            let p = (s[0].0 + (s[1].0 - s[0].0).signum() * d, s[0].1 + (s[1].1 - s[0].1).signum() * d);
            return (p, s[0].1 == s[1].1);
        }
        d -= len;
    }
    let n = path.len();
    (path[n - 1], path[n - 2].1 == path[n - 1].1)
}

/// How far along a line of length `len` draw.io puts a label written at `at` thousandths.
fn label_distance(at: u16, len: i32) -> i32 {
    ((at as i64 * len as i64 + 500) / 1000) as i32
}

/// True if `d` along the path lies within a few pixels of a bend.
fn near_bend(path: &[Pt], d: i32) -> bool {
    let mut at = 0;
    for s in path.windows(2).take(path.len().saturating_sub(2)) {
        at += (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs();
        if (d - at).abs() < 3 {
            return true;
        }
    }
    false
}

fn path_length(path: &[Pt]) -> i32 {
    path.windows(2).map(|s| (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs()).sum()
}

/// The area a frame's name takes at its top-left corner.
fn frame_name_area(f: &Frame) -> Rect {
    let (indent, h) = if f.kind == FrameKind::Az { (AZ_NAME_INDENT, AZ_NAME_H) } else { (FRAME_NAME_INDENT, FRAME_NAME_H) };
    Rect { x: f.rect.x, y: f.rect.y, w: indent + text_width(&f.label), h }
}

/// Lines a search may run along: the middle of every gap between edges of icons, labels and frames.
fn channels(mut edges: Vec<i32>) -> Vec<i32> {
    edges.sort_unstable();
    edges.dedup();
    let (Some(&lo), Some(&hi)) = (edges.first(), edges.last()) else { return Vec::new() };
    let mut out = vec![lo - OUTER, hi + OUTER];
    for w in edges.windows(2) {
        let gap = w[1] - w[0];
        if gap >= 2 {
            let mid = w[0] + gap / 2;
            out.push(mid);
            if gap >= WIDE_GAP {
                out.extend([mid - LANE, mid + LANE]);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Everything a line must not pass over, and the channels between them.
struct Scene<'a> {
    icons: &'a [Icon],
    labels: Vec<(&'a str, Rect)>,
    frame_names: Vec<Rect>,
    /// The four sides of every frame.
    borders: Vec<(Pt, Pt)>,
    xs: Vec<i32>,
    ys: Vec<i32>,
}

impl<'a> Scene<'a> {
    fn new(layout: &'a Layout) -> Scene<'a> {
        let labels: Vec<(&str, Rect)> = layout.icons.iter().map(|i| (i.id.as_str(), label_area(i))).collect();
        let frame_names: Vec<Rect> = layout.frames.iter().map(frame_name_area).collect();
        // An icon and its label form one block, so no channel runs between them.
        let blocks = layout.icons.iter().zip(&labels).map(|(i, l)| {
            let (x, y) = (i.rect.x.min(l.1.x), i.rect.y);
            Rect { x, y, w: (i.rect.x + i.rect.w).max(l.1.x + l.1.w) - x, h: l.1.y + l.1.h - y }
        });
        let rects = blocks.chain(layout.frames.iter().map(|f| f.rect)).chain(frame_names.iter().copied());
        let (mut ex, mut ey) = (Vec::new(), Vec::new());
        for r in rects {
            ex.extend([r.x, r.x + r.w]);
            ey.extend([r.y, r.y + r.h]);
        }
        let borders = layout
            .frames
            .iter()
            .flat_map(|f| {
                let r = f.rect;
                let (l, t, rt, b) = (r.x, r.y, r.x + r.w, r.y + r.h);
                [((l, t), (rt, t)), ((l, b), (rt, b)), ((l, t), (l, b)), ((rt, t), (rt, b))]
            })
            .collect();
        Scene { icons: &layout.icons, labels, frame_names, borders, xs: channels(ex), ys: channels(ey) }
    }

    /// Whether one segment of the line from `from` to `to` stays clear of icons, labels and frame
    /// names, by at least `margin`. The ends' own labels are skipped where the line leaves or enters
    /// below them, and so is the margin around the ends' own icons.
    fn clear(&self, a: Pt, b: Pt, (from, to): (&str, &str), (own_from, own_to): (bool, bool), margin: i32) -> bool {
        let exempt = |id: &str| (own_from && id == from) || (own_to && id == to);
        let icons = self.icons.iter().all(|i| {
            let by = match (i.id == from || i.id == to, exempt(&i.id)) {
                (true, true) => 0,
                (true, false) => margin,
                (false, _) => margin.max(1),
            };
            !hits(a, b, grow(i.rect, by))
        });
        let labels = self.labels.iter().all(|&(id, r)| exempt(id) || !hits(a, b, grow(r, margin)));
        icons && labels && self.frame_names.iter().all(|&r| !hits(a, b, grow(r, margin)))
    }

    /// Whether the label of line `me` taking `area` stays `margin` off every frame side, icon, icon
    /// label, frame name, label already placed and other line. Its own line only touches its edge.
    fn label_clear(&self, area: Rect, margin: i32, paths: &[Vec<Pt>], me: usize, placed: &[Rect]) -> bool {
        self.borders_under(area, margin) == 0 && self.label_clear_but_frames(area, margin, paths, me, placed)
    }

    /// How many frame sides come within `margin` of a label taking `area`.
    fn borders_under(&self, area: Rect, margin: i32) -> usize {
        let wide = grow(area, margin);
        self.borders.iter().filter(|&&(c, d)| hits(c, d, wide)).count()
    }

    /// Like `label_clear`, but frame sides may run under the label: a frame side is drawn behind
    /// the text and leaves it readable, where a line, an icon or other text does not.
    fn label_clear_but_frames(&self, area: Rect, margin: i32, paths: &[Vec<Pt>], me: usize, placed: &[Rect]) -> bool {
        let wide = grow(area, margin);
        self.icons.iter().all(|i| !overlaps(i.rect, wide))
            && self.labels.iter().all(|&(_, r)| !overlaps(r, wide))
            && self.frame_names.iter().all(|&r| !overlaps(r, wide))
            && paths.iter().enumerate().all(|(i, p)| p.windows(2).all(|s| !hits(s[0], s[1], if i == me { area } else { wide })))
            && placed.iter().all(|&r| !overlaps(r, wide))
    }

    /// How many frame sides a segment crosses, or None if it runs along one.
    fn borders_crossed(&self, a: Pt, b: Pt) -> Option<usize> {
        let mut n = 0;
        for &(c, d) in &self.borders {
            if shared(a, b, c, d) > MAX_SHARED {
                return None;
            }
            n += crosses(a, b, c, d) as usize;
        }
        Some(n)
    }

    /// Crossings with the placed lines, or None if the segment runs along one of them or so close
    /// beside one that the two strokes would read as one.
    fn against(&self, a: Pt, b: Pt, placed: &[Vec<Pt>]) -> Option<usize> {
        let mut n = 0;
        for s in placed.iter().flat_map(|p| p.windows(2)) {
            if alongside(a, b, s[0], s[1]) > MAX_SHARED {
                return None;
            }
            n += crosses(a, b, s[0], s[1]) as usize;
        }
        Some(n)
    }

    /// Checks a whole path from exit to entry. Returns its crossings if it is acceptable.
    fn check(&self, path: &[Pt], from: &str, to: &str, out: Dir, inward: Dir, placed: &[Vec<Pt>]) -> Option<usize> {
        if path.len() < 2 {
            return None;
        }
        let last = path.len() - 2;
        let mut crossings = 0;
        for (k, s) in path.windows(2).enumerate() {
            let d = dir_of(s[0], s[1])?;
            if (k == 0 && d != out) || (k == last && d != inward) || !self.clear(s[0], s[1], (from, to), (k == 0, k == last), 0) {
                return None;
            }
            crossings += self.against(s[0], s[1], placed)?;
        }
        Some(crossings)
    }

    /// Shortest acceptable path over the channels, counting bends, crossings and frame sides crossed, or None.
    /// Unlike `check`, it keeps a margin around everything and never runs along a frame's side.
    fn search(&self, start: Pt, out: Dir, goal: Pt, inward: Dir, (from, to): (&str, &str), placed: &[Vec<Pt>]) -> Option<Vec<Pt>> {
        let with = |base: &[i32], a: i32, b: i32| {
            let mut v = base.to_vec();
            v.extend([a, b]);
            v.sort_unstable();
            v.dedup();
            v
        };
        let (xs, ys) = (with(&self.xs, start.0, goal.0), with(&self.ys, start.1, goal.1));
        let (nx, ny) = (xs.len(), ys.len());
        let at = |p: Pt| (xs.binary_search(&p.0).expect("added"), ys.binary_search(&p.1).expect("added"));
        let dir_index = |d: Dir| DIRS.iter().position(|&x| x == d).expect("listed");
        // A state is a grid point, the heading, and whether the line is still on its first segment.
        let state = |xi: usize, yi: usize, d: usize, first: bool| ((yi * nx + xi) * 4 + d) * 2 + first as usize;
        let total = nx * ny * 8;
        let mut best = vec![i64::MAX; total];
        let mut prev = vec![usize::MAX; total];
        let ((sx, sy), (gx, gy)) = (at(start), at(goal));
        let h = |xi: usize, yi: usize| ((xs[xi] - goal.0).abs() + (ys[yi] - goal.1).abs()) as i64;
        let s0 = state(sx, sy, dir_index(out), true);
        best[s0] = 0;
        let mut heap = BinaryHeap::new();
        heap.push(Reverse((h(sx, sy), 0i64, s0)));
        while let Some(Reverse((_, g, s))) = heap.pop() {
            if g > best[s] {
                continue;
            }
            let (first, d, cell) = (s % 2 == 1, (s / 2) % 4, s / 8);
            let (xi, yi) = (cell % nx, cell / nx);
            if (xi, yi) == (gx, gy) && DIRS[d] == inward {
                let mut path = vec![goal];
                let mut cur = s;
                while prev[cur] != usize::MAX {
                    cur = prev[cur];
                    let c = cur / 8;
                    path.push((xs[c % nx], ys[c / nx]));
                }
                path.reverse();
                return Some(simplify(path));
            }
            for (nd, &dir) in DIRS.iter().enumerate() {
                if dir == reverse(DIRS[d]) || ((xi, yi) == (sx, sy) && nd != d) {
                    continue;
                }
                let next = match dir {
                    Dir::Up => yi.checked_sub(1).map(|y| (xi, y)),
                    Dir::Down => (yi + 1 < ny).then_some((xi, yi + 1)),
                    Dir::Left => xi.checked_sub(1).map(|x| (x, yi)),
                    Dir::Right => (xi + 1 < nx).then_some((xi + 1, yi)),
                };
                let Some((xj, yj)) = next else { continue };
                let (a, b) = ((xs[xi], ys[yi]), (xs[xj], ys[yj]));
                let nfirst = first && nd == d;
                let on_entry = dir == inward
                    && if vertical(dir) { a.0 == goal.0 } else { a.1 == goal.1 }
                    && match dir {
                        Dir::Down => b.1 <= goal.1,
                        Dir::Up => b.1 >= goal.1,
                        Dir::Right => b.0 <= goal.0,
                        Dir::Left => b.0 >= goal.0,
                    };
                if !self.clear(a, b, (from, to), (nfirst, on_entry), CLEARANCE) {
                    continue;
                }
                let Some(crossings) = self.against(a, b, placed) else { continue };
                let Some(borders) = self.borders_crossed(a, b) else { continue };
                let step = ((b.0 - a.0).abs() + (b.1 - a.1).abs()) as i64;
                let cost = g + step
                    + if nd != d { BEND_COST } else { 0 }
                    + CROSS_COST * crossings as i64
                    + BORDER_COST * borders as i64;
                let t = state(xj, yj, nd, nfirst);
                if cost < best[t] {
                    best[t] = cost;
                    prev[t] = s;
                    heap.push(Reverse((cost + h(xj, yj), cost, t)));
                }
            }
        }
        None
    }
}

/// The path a shape gives between two ports, or None for a shape left to the search.
fn shape_path(shape: Shape, ep: Pt, np: Pt, out: Dir) -> Option<Vec<Pt>> {
    let raw = match shape {
        Shape::Straight => vec![ep, np],
        Shape::ChannelY(y) => vec![ep, (ep.0, y), (np.0, y), np],
        Shape::ChannelX(x) => vec![ep, (x, ep.1), (x, np.1), np],
        Shape::Elbow if vertical(out) => vec![ep, (ep.0, np.1), np],
        Shape::Elbow => vec![ep, (np.0, ep.1), np],
        Shape::Search => return None,
    };
    Some(simplify(raw))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Exit and entry face each other on one line.
    Straight,
    /// Out, along a horizontal channel at this y, then in.
    ChannelY(i32),
    /// Out, along a vertical channel at this x, then in.
    ChannelX(i32),
    /// One bend where the exit's line meets the entry's line.
    Elbow,
    Search,
}

/// The sides and shape a line is planned with, before positions on the sides are shared out.
#[derive(Debug, Clone, Copy)]
struct Draft {
    exit: Side,
    entry: Side,
    /// Positions fixed by the shape instead of shared out with the side's other lines.
    exit_pin: Option<u8>,
    entry_pin: Option<u8>,
    shape: Shape,
    rank: u8,
}

fn draft(exit: Side, entry: Side, shape: Shape, rank: u8) -> Draft {
    Draft { exit, entry, exit_pin: None, entry_pin: None, shape, rank }
}

/// One line to draw and the semantic edges it stands for.
struct Spec {
    from: String,
    to: String,
    rel: Rel,
    hole: bool,
    /// Through a VPC endpoint: the label says so.
    endpoint: bool,
    label_key: String,
    /// Service of the merged sources, for lines that merge edges.
    merge: Option<String>,
    /// Indexes into `Model.edges`, in model order.
    edges: Vec<usize>,
    several_azs: bool,
}

struct Ctx<'a> {
    model: &'a Model,
    layout: &'a Layout,
    rules: &'a Rules,
    scene: Scene<'a>,
}

pub fn route(model: &Model, layout: &Layout, rules: &Rules) -> (Vec<Wire>, Vec<String>) {
    let ctx = Ctx { model, layout, rules, scene: Scene::new(layout) };
    let lang = model.lang;
    let (specs, merged_across_azs) = ctx.group();
    let mut notes = Vec::new();
    if merged_across_azs {
        notes.push(rules.text("note.cross_az_merged", lang));
    }
    let drafts = ctx.plan(&specs);
    let mut ports = ctx.share_out(&specs, &drafts);
    ctx.straighten_level_lines(&specs, &mut ports);

    let mut order: Vec<usize> = (0..specs.len()).collect();
    order.sort_by_key(|&i| (drafts[i].rank, i));
    let mut paths: Vec<Vec<Pt>> = vec![Vec::new(); specs.len()];
    let mut placed: Vec<Vec<Pt>> = Vec::new();
    for i in order {
        let (s, d) = (&specs[i], &drafts[i]);
        let path = match ctx.lay(s, d, ports[i], &placed) {
            Ok(p) => p,
            Err(best) => match ctx.on_free_sides(&specs, i, &ports, &placed) {
                Some((p, moved)) => {
                    ports[i] = moved;
                    p
                }
                None => {
                    notes.push(rules.text("note.unrouted_line", lang).replace("{line}", &format!("edge-{}-to-{}", s.from, s.to)));
                    best
                }
            },
        };
        placed.push(path.clone());
        paths[i] = path;
    }

    // Labels on the shortest lines first, since they have the fewest spots to choose from; each
    // keeps clear of the labels placed before it.
    let mut by_length: Vec<usize> = (0..specs.len()).collect();
    by_length.sort_by_key(|&i| (path_length(&paths[i]), i));
    let mut areas: Vec<Rect> = Vec::new();
    let mut spots: Vec<Option<LabelSpot>> = (0..specs.len()).map(|_| None).collect();
    for i in by_length {
        let s = &specs[i];
        let (fr, tr) = (layout.rect(&s.from), layout.rect(&s.to));
        let near_source = ctx.node(&s.from).service == IGW;
        let spot = ctx.place_label(&ctx.label_parts(s), (&paths, i), (fr.y - tr.y).abs() <= LEVEL, near_source, &areas);
        areas.extend(spot.area);
        spots[i] = Some(spot);
    }
    let mut wires = Vec::new();
    for (((s, (exit, entry)), path), spot) in specs.iter().zip(ports).zip(&paths).zip(spots) {
        let spot = spot.expect("every line has a label spot");
        let near_source = ctx.node(&s.from).service == IGW;
        if !spot.clear {
            notes.push(rules.text("note.unplaced_label", lang).replace("{line}", &format!("edge-{}-to-{}", s.from, s.to)));
        }
        wires.push(Wire {
            id: format!("edge-{}-to-{}", s.from, s.to),
            from: s.from.clone(),
            to: s.to.clone(),
            style: match (s.hole, s.rel) {
                (true, _) => WireStyle::Hole,
                (_, Rel::Main) => WireStyle::Main,
                (_, Rel::Aux) => WireStyle::Aux,
                (_, Rel::Assoc) => WireStyle::Assoc,
            },
            label: spot.text,
            exit,
            entry,
            points: path[1..path.len() - 1].to_vec(),
            horizontal_label: spot.above,
            label_near_source: near_source,
            label_at: spot.at,
            label_flip: spot.flip,
            represents: s.edges.iter().map(|&e| (model.edges[e].from.clone(), model.edges[e].to.clone())).collect(),
        });
    }
    // Whatever the router believed, the finished lines are checked once more on their own terms.
    for id in audit(model, layout, &wires) {
        let note = rules.text("note.unrouted_line", lang).replace("{line}", &id);
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
    (wires, notes)
}

/// Where a line's label goes and how it reads.
struct LabelSpot {
    text: String,
    above: bool,
    flip: bool,
    at: Option<u16>,
    area: Option<Rect>,
    /// False when no spot was clear and the label stays at its usual spot regardless.
    clear: bool,
}

impl<'a> Ctx<'a> {
    fn node(&self, id: &str) -> &'a Node {
        self.model.nodes.iter().find(|n| n.id == id).expect("edge ends are nodes")
    }

    fn scope(&self, n: &Node) -> Scope {
        self.rules.service(&n.service).map(|s| s.scope).expect("model services are in the rules")
    }

    fn icon(&self, id: &str) -> &'a Icon {
        self.layout.icons.iter().find(|i| i.id == id).expect("every node has an icon")
    }

    /// The AZ suffix of the subnet an icon sits in, e.g. "1a" for "pri-1a".
    fn subnet_suffix(&self, id: &str) -> Option<&'a str> {
        let p = self.icon(id).parent.as_deref()?;
        p.strip_prefix("pri-").or_else(|| p.strip_prefix("pub-"))
    }

    /// Which lines to draw. Lines into a Regional service from every AZ merge into one from the
    /// last AZ, nearest the side column. With three AZs, lines between per-AZ parts merge into the
    /// one inside the target's AZ. Everything else is one line per edge.
    fn group(&self) -> (Vec<Spec>, bool) {
        let mut specs: Vec<Spec> = Vec::new();
        for (i, e) in self.model.edges.iter().enumerate() {
            let (f, t) = (self.node(&e.from), self.node(&e.to));
            let merge = f.az.is_some()
                && (self.scope(t) == Scope::Regional
                    || (self.model.azs.len() > 2 && t.az.is_some() && f.service != t.service));
            let hole = e.hole.is_some();
            let same = |s: &Spec| {
                s.merge.as_deref() == Some(f.service.as_str()) && s.to == e.to && s.rel == e.rel && s.hole == hole && s.endpoint == e.endpoint
            };
            match specs.iter_mut().find(|s| merge && same(s)) {
                Some(s) => s.edges.push(i),
                None => specs.push(Spec {
                    from: e.from.clone(),
                    to: e.to.clone(),
                    rel: e.rel,
                    hole,
                    endpoint: e.endpoint,
                    label_key: e.label.clone(),
                    merge: merge.then(|| f.service.clone()),
                    edges: vec![i],
                    several_azs: false,
                }),
            }
        }
        let mut merged_across_azs = false;
        for s in specs.iter_mut().filter(|s| s.edges.len() > 1) {
            let from = |e: usize| self.node(&self.model.edges[e].from);
            let to = self.node(&s.to);
            let rep = if self.scope(to) == Scope::Regional {
                *s.edges.iter().max_by_key(|&&e| (from(e).az, Reverse(e))).expect("not empty")
            } else {
                merged_across_azs = true;
                *s.edges.iter().find(|&&e| from(e).az == to.az).unwrap_or(&s.edges[0])
            };
            s.from = self.model.edges[rep].from.clone();
            let mut azs: Vec<Option<usize>> = s.edges.iter().map(|&e| from(e).az).collect();
            azs.sort_unstable();
            azs.dedup();
            s.several_azs = azs.len() > 1;
        }
        (specs, merged_across_azs)
    }

    /// The label's parts: they read on one line joined by a space, or one per line where one line does not fit.
    fn label_parts(&self, s: &Spec) -> Vec<String> {
        let lang = self.model.lang;
        if s.hole {
            return vec![self.rules.text("undecided_route", lang)];
        }
        let mut parts = vec![self.rules.text(&s.label_key, lang)];
        if s.endpoint {
            parts.push(self.rules.text("via_endpoint", lang));
        }
        if s.several_azs {
            parts.push(self.rules.text("from_all_azs", lang));
        }
        parts.retain(|p| !p.is_empty());
        parts
    }

    /// The label at its usual spot (the middle, or a quarter along for `near_source`) if nothing
    /// cuts or covers it there. Otherwise the clear spot nearest the usual one along the line, on
    /// one line if that fits anywhere, else one part per line; spots with room around them come
    /// before tight ones, and spots on a 2-pixel grid before those between. A horizontal stretch
    /// carries the label centred above it, or below; a vertical one beside it on the right, or on
    /// the left. If no spot is clear, frame sides may run under the label: the spot clear of lines,
    /// icons and text with the fewest frame sides under it, in the same order, on one line if that
    /// fits anywhere, else one part per line. If there is none either, the usual spot stays.
    fn place_label(&self, parts: &[String], (paths, me): (&[Vec<Pt>], usize), level: bool, near_source: bool, placed: &[Rect]) -> LabelSpot {
        let path = &paths[me];
        let one = parts.join(" ");
        if one.is_empty() {
            return LabelSpot { text: one, above: level, flip: false, at: None, area: None, clear: true };
        }
        let len = path_length(path);
        let usual = if near_source { len / 4 } else { len / 2 };
        let area = wire_label_area(&one, point_along(path, usual).0, level, false);
        if self.scene.label_clear(area, LABEL_MARGIN, paths, me, placed) {
            return LabelSpot { text: one, above: level, flip: false, at: None, area: Some(area), clear: true };
        }
        let mut texts = vec![one.clone()];
        if parts.len() > 1 {
            texts.push(parts.join("\n"));
        }
        // The spots between the steps come last, for a label that fits only there.
        let mut tries = Vec::new();
        for between in [0, 1] {
            for margin in [LABEL_ROOM, LABEL_MARGIN] {
                tries.extend(texts.iter().map(|t| (between, margin, t)));
            }
        }
        let mut spots = Vec::new();
        for &(between, margin, text) in &tries {
            for (at, p, horizontal) in self.label_spots(path, usual, between) {
                for flip in [false, true] {
                    spots.push((margin, text, at, horizontal, flip, wire_label_area(text, p, horizontal, flip)));
                }
            }
        }
        let spot = |&(_, text, at, horizontal, flip, area): &(i32, &String, u16, bool, bool, Rect)| LabelSpot {
            text: text.clone(),
            above: horizontal,
            flip,
            at: Some(at),
            area: Some(area),
            clear: true,
        };
        if let Some(s) = spots.iter().find(|s| self.scene.label_clear(s.5, s.0, paths, me, placed)) {
            return spot(s);
        }
        // Over frame sides: the fewest of them, the first in the order above at a tie; one line
        // before one part per line, since a label that reads on one line is the easier to read.
        for text in &texts {
            let best = spots
                .iter()
                .filter(|s| s.1 == text && self.scene.label_clear_but_frames(s.5, s.0, paths, me, placed))
                .min_by_key(|s| self.scene.borders_under(s.5, s.0));
            if let Some(s) = best {
                return spot(s);
            }
        }
        LabelSpot { text: one, above: level, flip: false, at: None, area: Some(area), clear: false }
    }

    /// The spots along a path a label may take, nearest `usual` first (`between` shifts them off
    /// the 2-pixel grid), as thousandths of the length, the point and whether the stretch is
    /// horizontal.
    fn label_spots(&self, path: &[Pt], usual: i32, between: i32) -> Vec<(u16, Pt, bool)> {
        let len = path_length(path);
        let mut out = Vec::new();
        for k in 0..=len / LABEL_STEP {
            for d in [usual + k * LABEL_STEP + between, usual - k * LABEL_STEP - between] {
                if d < 0 || d > len {
                    continue;
                }
                // The spot draw.io draws: it rounds the position written in thousandths of the length.
                let at = ((d as i64 * 1000 + len as i64 / 2) / len as i64) as u16;
                let d = label_distance(at, len);
                // Keep off the bends, where a label just past the bend would turn with the line.
                if near_bend(path, d) {
                    continue;
                }
                let (p, horizontal) = point_along(path, d);
                out.push((at, p, horizontal));
            }
        }
        out
    }

    /// A line between two icons at the same height that leaves and enters sideways runs straight:
    /// the end alone on its side moves level with the other end.
    fn straighten_level_lines(&self, specs: &[Spec], ports: &mut [(Port, Port)]) {
        let alone = |ports: &[(Port, Port)], me: usize, id: &str, side: Side| {
            !specs.iter().zip(ports).enumerate().any(|(j, (o, (e, n)))| {
                j != me && ((o.from == id && e.side == side) || (o.to == id && n.side == side))
            })
        };
        for (i, s) in specs.iter().enumerate() {
            let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
            let (exit, entry) = ports[i];
            let sideways = matches!(exit.side, Side::Left | Side::Right) && entry.side == opposite(exit.side);
            if fr.y != tr.y || fr.h != tr.h || !sideways || exit.pos == entry.pos {
                continue;
            }
            if alone(ports, i, &s.to, entry.side) {
                ports[i].1.pos = exit.pos;
            } else if alone(ports, i, &s.from, exit.side) {
                ports[i].0.pos = entry.pos;
            }
        }
    }

    fn offset(&self, id: &str, side: Side) -> i32 {
        match side {
            Side::Bottom => LABEL_FIRST + LABEL_LINE * (self.icon(id).lines as i32 - 1),
            _ => 0,
        }
    }

    fn port(&self, id: &str, side: Side, pos: u8) -> Port {
        Port { side, pos, offset: self.offset(id, side) }
    }

    /// A path for the line with these ports: its shape if that is clear, else a search.
    fn try_route(&self, s: &Spec, d: &Draft, (exit, entry): (Port, Port), placed: &[Vec<Pt>], search: bool) -> Option<Vec<Pt>> {
        let ep = port_point(self.layout.rect(&s.from), exit);
        let np = port_point(self.layout.rect(&s.to), entry);
        let (out, inward) = (outward(exit.side), reverse(outward(entry.side)));
        if let Some(p) = shape_path(d.shape, ep, np, out) {
            if self.scene.check(&p, &s.from, &s.to, out, inward, placed).is_some() {
                return Some(p);
            }
            if !search {
                return None;
            }
        }
        self.scene.search(ep, out, np, inward, (&s.from, &s.to), placed)
    }

    /// The final path, or Err with the best path there is when none is acceptable.
    fn lay(&self, s: &Spec, d: &Draft, ports: (Port, Port), placed: &[Vec<Pt>]) -> Result<Vec<Pt>, Vec<Pt>> {
        self.try_route(s, d, ports, placed, true).ok_or_else(|| {
            let ep = port_point(self.layout.rect(&s.from), ports.0);
            let np = port_point(self.layout.rect(&s.to), ports.1);
            let out = outward(ports.0.side);
            let shape = if d.shape == Shape::Search { Shape::Elbow } else { d.shape };
            shape_path(shape, ep, np, out).expect("not a search")
        })
    }

    /// For a line its planned sides cannot carry: a search between the middles of sides no other
    /// line uses, nearest the straight line first. Returns the path and the new ports.
    fn on_free_sides(&self, specs: &[Spec], me: usize, ports: &[(Port, Port)], placed: &[Vec<Pt>]) -> Option<(Vec<Pt>, (Port, Port))> {
        let s = &specs[me];
        let taken = |id: &str, side: Side| {
            specs.iter().zip(ports).enumerate().any(|(j, (o, (e, n)))| {
                j != me && ((o.from == id && e.side == side) || (o.to == id && n.side == side))
            })
        };
        let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
        let mut pairs: Vec<(Side, Side)> = self.candidates(fr, tr).iter().map(|d| (d.exit, d.entry)).collect();
        pairs.extend(ALL_SIDES.iter().flat_map(|&e| ALL_SIDES.iter().map(move |&n| (e, n))));
        pairs.into_iter().filter(|&(e, n)| !taken(&s.from, e) && !taken(&s.to, n)).find_map(|(e, n)| {
            let moved = (self.port(&s.from, e, 50), self.port(&s.to, n, 50));
            let d = draft(e, n, Shape::Search, RANK_AUTO);
            self.try_route(s, &d, moved, placed, true).map(|p| (p, moved))
        })
    }

    /// Whether a draft can be drawn past the icons, labels and frame names, with its ports in the middle of their sides.
    fn fits(&self, s: &Spec, d: &Draft) -> bool {
        let exit = self.port(&s.from, d.exit, d.exit_pin.unwrap_or(50));
        let entry = self.port(&s.to, d.entry, d.entry_pin.unwrap_or(50));
        self.try_route(s, d, (exit, entry), &[], false).is_some()
    }

    fn first_fit(&self, s: &Spec, candidates: Vec<Draft>) -> Draft {
        candidates.iter().find(|d| self.fits(s, d)).copied().unwrap_or(candidates[0])
    }

    /// Sides and shape for every line.
    fn plan(&self, specs: &[Spec]) -> Vec<Draft> {
        let mut drafts: Vec<Option<Draft>> = vec![None; specs.len()];
        let (mut nat_in, mut nat_out, mut side, mut rest) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (i, s) in specs.iter().enumerate() {
            let (f, t) = (self.node(&s.from), self.node(&s.to));
            let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
            let per_az = f.az.is_some() && t.az.is_some();
            drafts[i] = if self.scope(t) == Scope::Regional {
                if self.scope(f) == Scope::Vpc {
                    side.push(i);
                    None
                } else {
                    Some(self.above_to_side(t, tr))
                }
            } else if per_az && f.service == t.service {
                let h = facing(fr, tr);
                Some(self.first_fit(s, vec![
                    draft(h, opposite(h), Shape::Straight, RANK_SIDE),
                    draft(Side::Bottom, Side::Bottom, Shape::Search, RANK_SIDE),
                ]))
            } else if t.service == NAT && f.az.is_some() && f.az == t.az {
                nat_in.push(i);
                None
            } else if f.service == NAT && t.service == IGW {
                nat_out.push(i);
                None
            } else if let Some(d) = self
                .fan_out_channel(f, t, fr, tr)
                .map(|y| draft(Side::Bottom, Side::Top, Shape::ChannelY(y), RANK_MAIN))
                .filter(|d| self.fits(s, d))
            {
                Some(d)
            } else if per_az && self.model.azs.len() == 2 && tr.y > fr.y + fr.h {
                Some(self.mesh(f, t, fr))
            } else if let Some(d) = Some(draft(Side::Bottom, Side::Top, Shape::Straight, RANK_MAIN))
                .filter(|d| center(fr).0 == center(tr).0 && self.fits(s, d))
            {
                Some(d)
            } else {
                rest.push(i);
                None
            };
        }
        self.choose_together(specs, &nat_in, &mut drafts, |i| self.nat_in_variants(&specs[i]));
        self.choose_together(specs, &nat_out, &mut drafts, |i| self.nat_out_variants(&specs[i]));

        // Lines from the VPC to the side column jog in the gap between them, each in its own lane,
        // spread evenly between the VPC's side and the nearest icon or label in the column.
        side.sort_by_key(|&i| (self.layout.rect(&specs[i].to).y, i));
        let vpc = self.layout.rect("vpc");
        let n = side.len() as i32;
        let column = side
            .iter()
            .map(|&i| {
                let icon = self.icon(&specs[i].to);
                icon.rect.x.min(label_area(icon).x)
            })
            .min()
            .unwrap_or(vpc.x + vpc.w);
        for (k, &i) in side.iter().enumerate() {
            let x = vpc.x + vpc.w + (column - vpc.x - vpc.w) * (k as i32 + 1) / (n + 1);
            drafts[i] = Some(draft(Side::Right, Side::Left, Shape::ChannelX(x), RANK_SIDE));
        }

        // The other lines keep off the sides the main lines use, where they can, so those stay centred and straight.
        let used: Vec<(&str, Side)> = specs
            .iter()
            .zip(&drafts)
            .filter_map(|(s, d)| d.filter(|d| d.rank == RANK_MAIN).map(|d| [(s.from.as_str(), d.exit), (s.to.as_str(), d.entry)]))
            .flatten()
            .collect();
        for i in rest {
            let s = &specs[i];
            let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
            let candidates = self.candidates(fr, tr);
            let free = |d: &Draft| !used.contains(&(s.from.as_str(), d.exit)) && !used.contains(&(s.to.as_str(), d.entry));
            let any_sides = ALL_SIDES.iter().flat_map(|&e| ALL_SIDES.iter().map(move |&n| draft(e, n, Shape::Search, RANK_AUTO)));
            drafts[i] = Some(
                candidates
                    .iter()
                    .copied()
                    .chain(any_sides)
                    .find(|d| free(d) && self.fits(s, d))
                    .unwrap_or_else(|| self.first_fit(s, candidates)),
            );
        }
        drafts.into_iter().map(|d| d.expect("every line planned")).collect()
    }

    /// From above the Region (CloudFront) into a Regional service: down onto its top if nothing
    /// sits above it in the side column, else around the column's right and in from that side.
    fn above_to_side(&self, t: &Node, tr: Rect) -> Draft {
        let blocked = self.layout.icons.iter().any(|i| {
            i.id != t.id && i.rect.x < tr.x + tr.w && i.rect.x + i.rect.w > tr.x && i.rect.y < tr.y
        });
        if blocked {
            let region = self.layout.rect("region");
            let x = (tr.x + tr.w + region.x + region.w) / 2;
            draft(Side::Right, Side::Right, Shape::ChannelX(x), RANK_SIDE)
        } else {
            draft(Side::Right, Side::Top, Shape::Elbow, RANK_SIDE)
        }
    }

    /// From one part into each AZ's instance in a private subnet: along the gap between the
    /// target AZ's public and private subnets.
    fn fan_out_channel(&self, f: &Node, t: &Node, fr: Rect, tr: Rect) -> Option<i32> {
        if f.az.is_some() || t.az.is_none() || tr.y <= fr.y + fr.h {
            return None;
        }
        let p = self.icon(&t.id).parent.as_deref()?;
        let sfx = p.strip_prefix("pri-")?;
        let (public, private) = (self.layout.rect(&format!("pub-{sfx}")), self.layout.rect(p));
        Some((public.y + public.h + private.y) / 2)
    }

    /// With two AZs, every per-AZ part reaches every per-AZ part below it. The line inside an AZ
    /// drops straight on the outer quarter; the line to the other AZ turns in its own channel and
    /// enters the middle, so the two crossing lines never share a column.
    fn mesh(&self, f: &Node, t: &Node, fr: Rect) -> Draft {
        let az = f.az.expect("per-AZ").min(1);
        let mut d = draft(Side::Bottom, Side::Top, Shape::Straight, RANK_MAIN);
        if f.az == t.az {
            d.entry_pin = Some(if az == 0 { 25 } else { 75 });
        } else {
            d.entry_pin = Some(50);
            d.shape = Shape::ChannelY(fr.y + MESH_CHANNELS[az]);
        }
        d
    }

    /// Into the NAT gateway of the same AZ: sideways toward it and up into its bottom, or, if
    /// that passes over a frame name, up the gap between the AZ and subnet frames into its far side.
    fn nat_in_variants(&self, s: &Spec) -> Vec<Draft> {
        let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
        let h = facing(fr, tr);
        let mut v = vec![draft(h, Side::Bottom, Shape::Elbow, RANK_NAT)];
        if let (Some(sfx), Some(sub)) = (self.subnet_suffix(&s.from), self.icon(&s.from).parent.as_deref()) {
            let (az, sub) = (self.layout.rect(&format!("az-{sfx}")), self.layout.rect(sub));
            let x = match h {
                Side::Left => (az.x + sub.x) / 2,
                _ => (az.x + az.w + sub.x + sub.w) / 2,
            };
            v.push(draft(h, h, Shape::ChannelX(x), RANK_NAT));
        }
        v
    }

    /// From a NAT gateway to the Internet Gateway: up from its top and into the gateway's side,
    /// or, if that passes over a frame name, out its far side and up the gap between the VPC and AZ frames.
    fn nat_out_variants(&self, s: &Spec) -> Vec<Draft> {
        let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
        let h = facing(tr, fr);
        let mut v = vec![draft(Side::Top, h, Shape::Elbow, RANK_NAT)];
        if let Some(sfx) = self.subnet_suffix(&s.from) {
            let (az, vpc) = (self.layout.rect(&format!("az-{sfx}")), self.layout.rect("vpc"));
            let x = match h {
                Side::Left => (vpc.x + az.x) / 2,
                _ => (vpc.x + vpc.w + az.x + az.w) / 2,
            };
            v.push(draft(h, h, Shape::ChannelX(x), RANK_NAT));
        }
        v
    }

    /// Gives every member the first variant that fits for all of them, so AZs that mirror each
    /// other get mirrored lines; members no common variant fits get their own first fit.
    fn choose_together(&self, specs: &[Spec], members: &[usize], drafts: &mut [Option<Draft>], variants: impl Fn(usize) -> Vec<Draft>) {
        let all: Vec<Vec<Draft>> = members.iter().map(|&i| variants(i)).collect();
        let common = (0..all.iter().map(Vec::len).min().unwrap_or(0))
            .find(|&v| members.iter().zip(&all).all(|(&i, vs)| self.fits(&specs[i], &vs[v])));
        for (&i, vs) in members.iter().zip(all) {
            drafts[i] = Some(match common {
                Some(v) => vs[v],
                None => self.first_fit(&specs[i], vs),
            });
        }
    }

    /// Sides to try for a line no rule above covers, nearest the straight line first.
    fn candidates(&self, fr: Rect, tr: Rect) -> Vec<Draft> {
        let (fc, tc) = (center(fr), center(tr));
        let (dx, dy) = (tc.0 - fc.0, tc.1 - fc.1);
        let h = if dx >= 0 { Side::Right } else { Side::Left };
        let (b, t) = (Side::Bottom, Side::Top);
        let pairs: Vec<(Side, Side)> = if dy.abs() <= LEVEL {
            vec![(h, opposite(h)), (b, b), (t, t)]
        } else if dy > 0 && dx.abs() <= LEVEL {
            // Straight down was blocked, so go around.
            vec![(h, h), (opposite(h), opposite(h)), (b, t)]
        } else if dy > 0 {
            vec![(b, opposite(h)), (h, t), (b, t)]
        } else if dx.abs() <= LEVEL {
            vec![(t, b), (h, h), (opposite(h), opposite(h))]
        } else {
            vec![(t, opposite(h)), (h, b), (t, b)]
        };
        pairs.into_iter().map(|(e, n)| draft(e, n, Shape::Search, RANK_AUTO)).collect()
    }

    /// Positions on each side, from `slots`.
    /// Lines on a top or bottom side are ordered by where their other end lies left to right; on a
    /// left or right side, top to bottom, and the farther first among ends at the same height.
    fn share_out(&self, specs: &[Spec], drafts: &[Draft]) -> Vec<(Port, Port)> {
        type Key = (i32, i32, usize);
        type Ends = Vec<(Key, usize, bool)>;
        let mut sides: BTreeMap<(&str, u8), Ends> = BTreeMap::new();
        let side_index = |s: Side| s as u8;
        let key = |side: Side, this: Pt, other: Pt, i: usize| -> Key {
            match side {
                Side::Top | Side::Bottom => (other.0, other.1, i),
                Side::Left | Side::Right => (other.1, -(other.0 - this.0).abs(), i),
            }
        };
        for (i, (s, d)) in specs.iter().zip(drafts).enumerate() {
            let (fc, tc) = (center(self.layout.rect(&s.from)), center(self.layout.rect(&s.to)));
            if d.exit_pin.is_none() {
                sides.entry((s.from.as_str(), side_index(d.exit))).or_default().push((key(d.exit, fc, tc, i), i, true));
            }
            if d.entry_pin.is_none() {
                sides.entry((s.to.as_str(), side_index(d.entry))).or_default().push((key(d.entry, tc, fc, i), i, false));
            }
        }
        let mut pos: Vec<(u8, u8)> = drafts.iter().map(|d| (d.exit_pin.unwrap_or(50), d.entry_pin.unwrap_or(50))).collect();
        for (_, mut ends) in sides {
            ends.sort();
            let spots = slots(ends.len());
            for (k, &(_, i, is_exit)) in ends.iter().enumerate() {
                let p = spots[k];
                if is_exit { pos[i].0 = p } else { pos[i].1 = p }
            }
        }
        specs
            .iter()
            .zip(drafts)
            .zip(pos)
            .map(|((s, d), (e, n))| (self.port(&s.from, d.exit, e), self.port(&s.to, d.entry, n)))
            .collect()
    }
}

/// Positions along one side for `n` lines: the middle for one, quarters for two and three, and
/// for more, evenly spread so that no two lines share a spot.
fn slots(n: usize) -> Vec<u8> {
    match n {
        1 => vec![50],
        2 => vec![25, 75],
        3 => vec![25, 50, 75],
        _ => (1..=n).map(|k| (k * 100 / (n + 1)) as u8).collect(),
    }
}

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
fn audit_wire_text_width(text: &str) -> i32 {
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
fn audit_wire_label(w: &Wire, path: &[(i32, i32)]) -> Option<Rect> {
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

/// Lines that run over text (an icon's label, a frame's name, another line's label), over an
/// icon they do not attach to, or on top of another line. Ids in line order, each once.
fn audit(_model: &Model, layout: &Layout, wires: &[Wire]) -> Vec<String> {
    let icon_labels: Vec<(&str, Rect)> = layout
        .icons
        .iter()
        .map(|i| {
            let half = (audit_text_width(&i.label) + 1) / 2;
            (i.id.as_str(), Rect { x: i.rect.x + i.rect.w / 2 - half, y: i.rect.y + i.rect.h + 4, w: 2 * half, h: 14 * i.lines as i32 + 6 })
        })
        .collect();
    let frame_names: Vec<Rect> = layout
        .frames
        .iter()
        .map(|f| {
            let (indent, h) = if f.kind == FrameKind::Az { (10, 20) } else { (30, 25) };
            Rect { x: f.rect.x, y: f.rect.y, w: indent + audit_text_width(&f.label), h }
        })
        .collect();
    let paths: Vec<Vec<(i32, i32)>> = wires.iter().map(|w| audit_path(w, layout)).collect();
    let labels: Vec<Option<Rect>> = wires.iter().zip(&paths).map(|(w, p)| audit_wire_label(w, p)).collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wires(src: &str) -> (Model, Layout, Vec<Wire>, Vec<String>) {
        let r = Rules::builtin();
        let (m, _) = crate::infer::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap(); let l = crate::layout::layout(&m, &r); let (w, n) = route(&m, &l, &r);
        (m, l, w, n)
    }
    const WEB2: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n";

    /// Exit, waypoints and entry in order, with any diagonal step folded into a right angle.
    fn full_path(x: &Wire, l: &Layout) -> Vec<(i32, i32)> {
        let mut raw = vec![port_point(l.rect(&x.from), x.exit)];
        raw.extend(x.points.iter().copied());
        raw.push(port_point(l.rect(&x.to), x.entry));
        let mut out = vec![raw[0]];
        for &p in &raw[1..] {
            let a = *out.last().unwrap();
            if a.0 != p.0 && a.1 != p.1 { out.push((p.0, a.1)); }
            out.push(p);
        }
        out
    }

    /// True if the segment passes through the inside of the rectangle; touching its edge is not a hit.
    fn segment_hits_rect(a: (i32, i32), b: (i32, i32), r: Rect) -> bool {
        let (x1, x2) = (a.0.min(b.0), a.0.max(b.0));
        let (y1, y2) = (a.1.min(b.1), a.1.max(b.1));
        x1 < r.x + r.w && x2 > r.x && y1 < r.y + r.h && y2 > r.y
    }

    /// The rectangle an icon's label takes, by the measure the diagram checker uses.
    fn label_rect(i: &crate::layout::Icon) -> Rect {
        let w = i.label.lines().map(|l| l.chars().map(|c| if c.is_ascii() { 7 } else { 12 }).sum::<i32>()).max().unwrap_or(0);
        let half = (w + 1) / 2;
        Rect { x: i.rect.x + i.rect.w / 2 - half, y: i.rect.y + i.rect.h + 4, w: 2 * half, h: 14 * i.lines as i32 + 6 }
    }

    fn both() -> [String; 2] { [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n")] }

    #[test]
    fn every_semantic_edge_is_represented_exactly_once() {
        for src in [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n")] {
            let (m, _, w, _) = wires(&src);
            let mut drawn: Vec<(String, String)> = w.iter().flat_map(|x| x.represents.clone()).collect();
            let mut want: Vec<(String, String)> = m.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
            drawn.sort(); want.sort();
            assert_eq!(drawn, want);
        }
    }

    #[test]
    fn bottom_ports_clear_the_label() {
        let (_, _, w, _) = wires(WEB2);
        let x = w.iter().find(|x| x.id == "edge-alb-to-ecs-1a").unwrap();
        assert_eq!(x.exit, Port { side: Side::Bottom, pos: 25, offset: 24 });
        assert_eq!(x.points, vec![(515, 670), (335, 670)]);
        let y = w.iter().find(|x| x.id == "edge-alb-to-ecs-1c").unwrap();
        assert_eq!((y.exit.pos, y.points.clone()), (75, vec![(545, 670), (725, 670)]));
    }

    #[test]
    fn cross_az_wires_use_separate_channels() {
        let (_, _, w, _) = wires(WEB2);
        let a = w.iter().find(|x| x.id == "edge-ecs-1a-to-aurora-1c").unwrap();
        let c = w.iter().find(|x| x.id == "edge-ecs-1c-to-aurora-1a").unwrap();
        assert_eq!((a.points[0].1, c.points[0].1), (840, 885));
    }

    #[test]
    fn styles_follow_the_relation_and_holes() {
        let (_, _, w, _) = wires("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\n");
        let style = |to: &str| w.iter().find(|x| x.to == to).unwrap().style;
        assert_eq!(style("s3"), WireStyle::Hole);
        assert_eq!(style("cloudwatch"), WireStyle::Aux);
        assert_eq!(w.iter().find(|x| x.from == "waf").unwrap().style, WireStyle::Assoc);
        assert_eq!(w.iter().find(|x| x.to == "s3").unwrap().label, "Route undecided");
        assert_eq!(w.iter().find(|x| x.id == "edge-aurora-1a-to-aurora-1c").unwrap().style, WireStyle::Aux);
    }

    #[test]
    fn three_azs_merge_cross_az_wires_and_say_so() {
        let (_, _, w, notes) = wires(&format!("{WEB2}set:\n  azs: 3\n"));
        assert!(w.iter().all(|x| !(x.from.starts_with("ecs-") && x.to.starts_with("aurora-") && x.from[4..] != x.to[7..])));
        assert!(w.iter().find(|x| x.id == "edge-ecs-1a-to-aurora-1a").unwrap().represents.len() >= 3);
        assert!(!notes.is_empty());
    }

    #[test]
    fn no_wire_segment_crosses_an_icon_it_is_not_attached_to() {
        for src in [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n")] {
            let (_, l, w, _) = wires(&src);
            for x in &w {
                let pts = full_path(x, &l);
                for seg in pts.windows(2) { for i in &l.icons {
                    if i.id == x.from || i.id == x.to { continue; }
                    assert!(!segment_hits_rect(seg[0], seg[1], i.rect), "{} crosses {}", x.id, i.id);
                } }
            }
        }
    }

    #[test]
    fn no_wire_segment_crosses_a_label_except_its_own_ends() {
        for src in both() {
            let (_, l, w, _) = wires(&src);
            for x in &w {
                let pts = full_path(x, &l);
                let last = pts.len() - 2;
                for (k, seg) in pts.windows(2).enumerate() { for i in &l.icons {
                    if (k == 0 && i.id == x.from) || (k == last && i.id == x.to) { continue; }
                    assert!(!segment_hits_rect(seg[0], seg[1], label_rect(i)), "{} crosses the label of {}", x.id, i.id);
                } }
            }
        }
    }

    #[test]
    fn no_two_wires_share_a_segment() {
        for src in both() {
            let (_, l, w, _) = wires(&src);
            // Wires are told apart by index, so two wires are always compared even if their ids matched.
            type Seg = (usize, (i32, i32), (i32, i32));
            let segs: Vec<Seg> = w.iter().enumerate()
                .flat_map(|(n, x)| { let p = full_path(x, &l); p.windows(2).map(|s| (n, s[0], s[1])).collect::<Vec<_>>() })
                .collect();
            for (i, a) in segs.iter().enumerate() { for b in &segs[i + 1..] {
                if a.0 == b.0 { continue; }
                let shared = if a.1.0 == a.2.0 && b.1.0 == b.2.0 && a.1.0 == b.1.0 {
                    a.1.1.max(a.2.1).min(b.1.1.max(b.2.1)) - a.1.1.min(a.2.1).max(b.1.1.min(b.2.1))
                } else if a.1.1 == a.2.1 && b.1.1 == b.2.1 && a.1.1 == b.1.1 {
                    a.1.0.max(a.2.0).min(b.1.0.max(b.2.0)) - a.1.0.min(a.2.0).max(b.1.0.min(b.2.0))
                } else { 0 };
                assert!(shared <= 1, "{} and {} share {shared} px", w[a.0].id, w[b.0].id);
            } }
        }
    }

    #[test]
    fn every_wire_is_orthogonal_and_leaves_and_enters_outward() {
        for src in both() {
            let (_, l, w, notes) = wires(&src);
            assert!(notes.iter().all(|n| !n.contains("could not")), "{notes:?}");
            for x in &w {
                let mut raw = vec![port_point(l.rect(&x.from), x.exit)];
                raw.extend(x.points.iter().copied());
                raw.push(port_point(l.rect(&x.to), x.entry));
                for s in raw.windows(2) { assert!(s[0].0 == s[1].0 || s[0].1 == s[1].1, "{} has a diagonal step", x.id); }
                let (a, b) = (raw[0], raw[1]);
                let out = match x.exit.side { Side::Top => b.1 < a.1, Side::Bottom => b.1 > a.1, Side::Left => b.0 < a.0, Side::Right => b.0 > a.0 };
                let (c, d) = (raw[raw.len() - 2], raw[raw.len() - 1]);
                let inward = match x.entry.side { Side::Top => d.1 > c.1, Side::Bottom => d.1 < c.1, Side::Left => d.0 > c.0, Side::Right => d.0 < c.0 };
                assert!(out && inward, "{} runs along its icon's side", x.id);
            }
        }
    }

    #[test]
    fn lines_to_regional_services_merge_and_say_all_azs() {
        let (_, _, w, _) = wires(WEB2);
        let cw = w.iter().find(|x| x.to == "cloudwatch").unwrap();
        assert_eq!((cw.id.as_str(), cw.label.as_str(), cw.style), ("edge-ecs-1c-to-cloudwatch", "Logs and metrics\n(all AZs)", WireStyle::Aux));
        assert_eq!(cw.represents, [("ecs-1a".to_string(), "cloudwatch".to_string()), ("ecs-1c".to_string(), "cloudwatch".to_string())]);
        assert!(cw.horizontal_label);
        assert_eq!(w.iter().filter(|x| x.to == "cloudwatch").count(), 1);
        let (_, _, ja, _) = wires(&format!("{WEB2}set:\n  lang: ja\n"));
        assert_eq!(ja.iter().find(|x| x.to == "cloudwatch").unwrap().label, "ログ・メトリクス\n（全 AZ から）");
        let (_, _, w3, _) = wires(&format!("{WEB2}set:\n  azs: 3\n"));
        let writer = w3.iter().find(|x| x.id == "edge-ecs-1a-to-aurora-1a").unwrap();
        assert_eq!(writer.label, "Read/Write (all AZs)");
    }

    #[test]
    fn main_column_is_straight_and_nat_lines_mirror() {
        let (_, l, w, _) = wires(WEB2);
        let get = |id: &str| w.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("{id}"));
        for id in ["edge-user-to-cloudfront", "edge-cloudfront-to-igw", "edge-igw-to-alb"] {
            assert!(get(id).points.is_empty(), "{id}");
        }
        let mirror = |p: Vec<(i32, i32)>| p.into_iter().map(|(x, y)| (1060 - x, y)).collect::<Vec<_>>();
        assert_eq!(mirror(full_path(get("edge-nat-1a-to-igw"), &l)), full_path(get("edge-nat-1c-to-igw"), &l));
        assert_eq!(get("edge-ecs-1a-to-nat-1a").points.len(), get("edge-ecs-1c-to-nat-1c").points.len());
        let near: Vec<&str> = w.iter().filter(|x| x.label_near_source).map(|x| x.id.as_str()).collect();
        assert_eq!(near, ["edge-igw-to-alb"]);
    }

    #[test]
    fn an_endpoint_route_says_so_in_its_label() {
        let src = "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\n";
        let (_, _, w, _) = wires(src);
        let s3 = w.iter().find(|x| x.to == "s3").unwrap();
        assert_eq!((s3.label.replace('\n', " ").as_str(), s3.style), ("Read/Write (via VPC endpoint) (all AZs)", WireStyle::Main));
        let (_, _, ja, _) = wires(&format!("{src}set:\n  lang: ja\n"));
        assert_eq!(ja.iter().find(|x| x.to == "s3").unwrap().label.replace('\n', " "), "読み書き （VPC エンドポイント経由） （全 AZ から）");
        let (_, _, nat, _) = wires(&src.replace("endpoint", "nat"));
        assert!(!nat.iter().find(|x| x.to == "s3").unwrap().label.contains("VPC"));
    }

    /// A route answered with `endpoint`, in Japanese: the label has three parts and is too long for one line.
    const ENDPOINT_JA: &str = "flows:\n  - user -> alb -> ecs -> s3\nset:\n  lang: ja\nchoose:\n  route.ecs.s3: endpoint\n";

    fn endpoint_ja_definitions() -> Vec<String> {
        let with_cw = ENDPOINT_JA.replace("ecs -> s3\n", "ecs -> s3\n  - ecs ~> cloudwatch\n");
        vec![
            ENDPOINT_JA.to_string(),
            ENDPOINT_JA.replace("lang: ja\n", "lang: ja\n  azs: 3\n"),
            with_cw.clone(),
            with_cw.replace("lang: ja\n", "lang: ja\n  azs: 3\n"),
        ]
    }

    #[test]
    fn a_japanese_label_of_several_parts_stacks_them_clear_of_frame_sides() {
        for src in endpoint_ja_definitions() {
            let (_, l, w, notes) = wires(&src);
            assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?} in\n{src}");
            let s3 = w.iter().find(|x| x.to == "s3").unwrap();
            assert_eq!(s3.label, "読み書き\n（VPC エンドポイント経由）\n（全 AZ から）", "{src}");
            let borders = Scene::new(&l).borders;
            for x in w.iter().filter(|x| !x.label.is_empty()) {
                let area = label_area_of(x, &l);
                for &(c, d) in &borders {
                    assert!(!hits(c, d, area), "a frame side {c:?}-{d:?} cuts the label of {} in\n{src}", x.id);
                }
            }
        }
    }

    #[test]
    fn a_line_label_is_estimated_no_narrower_than_draw_io_draws_it_and_not_much_wider() {
        // Widths of the drawn text, read from draw.io's rendering at font size 11.
        for (text, drawn) in [
            ("読み書き", 42), ("（VPC エンドポイント経由）", 132), ("（全 AZ から）", 61), ("ログ・メトリクス", 86), ("ああああああああああ", 108),
            // Full-width parentheses inside a line are drawn at full width, at its ends at half.
            ("読み書き （VPC エンドポイント経由） （全 AZ から）", 264), ("ログ・メトリクス （全 AZ から）", 158), ("あ（あ", 31), ("あ（あ）あ", 53),
        ] {
            let w = wire_text_width(text);
            assert!(w >= drawn && w <= drawn + 16, "{text}: estimated {w}, drawn {drawn}");
        }
        // Latin text: wide capitals, narrow letters and punctuation, never under the drawn width
        // and not much over it.
        for (text, drawn) in [
            ("Read/Write", 54), ("(via VPC endpoint)", 92), ("HTTPS", 35), ("HTTP", 28), ("(all AZs)", 40),
            ("Logs and metrics", 83), ("Outbound", 48), ("Replication", 53), ("Route undecided", 82),
            ("WWWWWWWWWW", 104), ("MMMMMMMMMM", 91), ("mmmmmmmmmm", 91), ("AAAAAAAAAA", 73), ("aaaaaaaaaa", 61),
        ] {
            let w = wire_text_width(text);
            assert!(w >= drawn && w <= drawn + 16, "{text}: estimated {w}, drawn {drawn}");
            assert_eq!(audit_wire_text_width(text), w, "{text}: the final check measures it the same way");
        }
    }

    #[test]
    fn every_printable_latin_character_is_estimated_no_narrower_than_draw_io_draws_it() {
        // Ten times the width of each character, read from draw.io's rendering at font size 11:
        // the larger of ten in a row and ten more between two others.
        let chars = r##"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~"##;
        let drawn = [61, 54, 61, 61, 61, 61, 61, 61, 61, 61, 61, 61, 55, 61, 61, 29, 61, 61, 24, 24, 55, 24, 92, 61, 61, 61, 61, 37, 55, 31, 61, 55, 79, 55, 55, 55, 73, 73, 79, 79, 73, 67, 86, 79, 31, 55, 73, 61, 92, 79, 86, 73, 86, 79, 73, 67, 79, 73, 104, 73, 73, 67, 31, 39, 61, 61, 98, 73, 21, 37, 37, 43, 64, 31, 37, 31, 31, 31, 31, 64, 64, 64, 61, 112, 31, 31, 31, 52, 61, 37, 37, 29, 37, 64];
        assert_eq!(chars.chars().count(), drawn.len());
        for (c, d) in chars.chars().zip(drawn) {
            let ten = c.to_string().repeat(10);
            let w = wire_text_width(&ten);
            assert!(w >= d, "{c:?}: estimated {w} for ten, drawn {d}");
            assert_eq!(audit_wire_text_width(&ten), w, "{c:?}: the final check measures it the same way");
        }
        // Characters not measured count at the widest width, never narrower.
        assert!(wire_text_width("\t") >= wire_text_width("@"));
    }

    /// Three AZs in English, with the S3 line answered with `endpoint` and lines to the side column
    /// on both sides of it: no stretch of the S3 line holds its label clear of every frame side.
    const ENDPOINT_3AZ_EN: &str = "flows:\n  - user -> alb -> ecs -> aurora\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - aurora ~> cloudwatch\n  - ecs -> internet\nset:\n  azs: 3\n  lang: en\n  nat: per-az\nchoose:\n  route.ecs.s3: endpoint\n";

    #[test]
    fn a_label_with_no_spot_clear_of_frame_sides_lies_on_them_clear_of_lines_and_text() {
        assert_eq!(violations(ENDPOINT_3AZ_EN), Vec::<String>::new());
        let (_, l, w, notes) = wires(ENDPOINT_3AZ_EN);
        assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?}");
        let s3 = w.iter().find(|x| x.to == "s3").unwrap();
        // Whole parts: on one line, or one part per line.
        assert!(
            ["Read/Write (via VPC endpoint) (all AZs)", "Read/Write\n(via VPC endpoint)\n(all AZs)"].contains(&s3.label.as_str()),
            "{:?}",
            s3.label
        );
        let area = label_area_of(s3, &l);
        assert!(Scene::new(&l).borders.iter().any(|&(c, d)| hits(c, d, area)), "{area:?} is clear of frame sides, so the first search should have found it");
        for y in &w {
            for s in full_path(y, &l).windows(2) {
                assert!(!hits(s[0], s[1], area), "{} runs through the label of {}", y.id, s3.id);
            }
        }
        for i in &l.icons {
            assert!(!overlaps(i.rect, area) && !overlaps(label_area(i), area), "{} or its name is under the label", i.id);
        }
        // The same in Japanese.
        let ja = ENDPOINT_3AZ_EN.replace("lang: en", "lang: ja");
        assert_eq!(violations(&ja), Vec::<String>::new());
        let (_, l, w, _) = wires(&ja);
        let s3 = w.iter().find(|x| x.to == "s3").unwrap();
        assert!(s3.label.starts_with("読み書き"), "{:?}", s3.label);
        assert!(!s3.label.contains("（VPC\n"), "{:?}", s3.label);
        let area = label_area_of(s3, &l);
        for y in &w {
            for s in full_path(y, &l).windows(2) {
                assert!(!hits(s[0], s[1], area), "{} runs through the label of {}", y.id, s3.id);
            }
        }
        // A label that fits one part per line clear of frame sides keeps that spot and shape.
        let (_, l, w, _) = wires(&ENDPOINT_JA.replace("lang: ja", "lang: en"));
        let s3 = w.iter().find(|x| x.to == "s3").unwrap();
        assert_eq!(s3.label, "Read/Write\n(via VPC endpoint)\n(all AZs)");
        let area = label_area_of(s3, &l);
        assert!(Scene::new(&l).borders.iter().all(|&(c, d)| !hits(c, d, area)));
    }

    #[test]
    fn a_label_with_no_spot_clear_of_lines_and_text_stays_at_its_usual_spot_and_is_not_clear() {
        let (m, l, w, _) = wires(WEB2);
        let r = Rules::builtin();
        let ctx = Ctx { model: &m, layout: &l, rules: &r, scene: Scene::new(&l) };
        let paths: Vec<Vec<Pt>> = w.iter().map(|x| full_path(x, &l)).collect();
        let me = w.iter().position(|x| x.id == "edge-cloudfront-to-s3").unwrap();
        // Labels already placed everywhere: no spot is clear even over frame sides.
        let everywhere = Rect { x: -10_000, y: -10_000, w: 20_000, h: 20_000 };
        let spot = ctx.place_label(&["Static files".to_string()], (&paths, me), false, false, &[everywhere]);
        assert!(!spot.clear);
        assert_eq!((spot.text.as_str(), spot.at), ("Static files", None));
    }

    /// Lines from the load balancer and the private parts all head for the side column.
    const AUX_TO_SIDE: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - alb ~> s3\nchoose:\n  route.aurora.s3: endpoint\n";

    #[test]
    fn lines_to_the_side_column_get_their_own_lanes() {
        assert_eq!(violations(AUX_TO_SIDE), Vec::<String>::new());
        let (_, l, w, notes) = wires(AUX_TO_SIDE);
        assert!(notes.iter().all(|n| !n.contains("could not")), "{notes:?}");
        // Every jog toward the side column is a vertical run at its own x, clear of the column's labels.
        let side_labels: Vec<Rect> = l.icons.iter().filter(|i| i.parent.as_deref() == Some("region")).map(label_rect).collect();
        let mut xs: Vec<i32> = Vec::new();
        for x in w.iter().filter(|x| l.icons.iter().any(|i| i.id == x.to && i.parent.as_deref() == Some("region"))) {
            let p = full_path(x, &l);
            for s in p.windows(2).filter(|s| s[0].0 == s[1].0) {
                assert!(side_labels.iter().all(|r| !segment_hits_rect(s[0], s[1], *r)), "{} runs over a label", x.id);
                xs.push(s[0].0);
            }
        }
        let n = xs.len();
        xs.sort_unstable();
        xs.dedup();
        assert_eq!(xs.len(), n, "two jogs share an x");
    }

    #[test]
    fn four_or_more_lines_on_one_side_each_get_their_own_spot() {
        for n in 1..=7 {
            let s = slots(n);
            assert_eq!(s.len(), n);
            assert!(s.windows(2).all(|w| w[0] < w[1]) && s[0] > 0 && s[n - 1] < 100, "{n}: {s:?}");
        }
        assert_eq!(slots(3), [25, 50, 75]);
        assert_eq!(slots(4), [20, 40, 60, 80]);
    }

    #[test]
    fn the_final_check_names_lines_that_overlap_or_cross_text_whatever_the_router_thought() {
        let (m, l, mut w, _) = wires(WEB2);
        assert_eq!(audit(&m, &l, &w), Vec::<String>::new());
        // Lay one line on top of another, one pixel apart, and another across an icon's label.
        let a = w.iter().position(|x| x.id == "edge-ecs-1a-to-aurora-1a").unwrap();
        let b = w.iter().position(|x| x.id == "edge-ecs-1c-to-aurora-1c").unwrap();
        let pa = full_path(&w[a], &l);
        w[b].points = vec![(pa[0].0 + 1, pa[0].1 + 5), (pa[0].0 + 1, pa[pa.len() - 1].1 - 5)];
        w[b].exit = w[a].exit;
        let named = audit(&m, &l, &w);
        assert!(named.contains(&"edge-ecs-1a-to-aurora-1a".to_string()) && named.contains(&"edge-ecs-1c-to-aurora-1c".to_string()), "{named:?}");
    }

    #[test]
    fn routing_is_deterministic() {
        for src in both() {
            assert_eq!(wires(&src).2, wires(&src).2);
        }
    }

    /// Every way a line breaks the diagram checker's rules in one definition, as (line id, what),
    /// measured by this test's own arithmetic; and the notes that name a line.
    fn faults(src: &str) -> (Vec<(String, String)>, Vec<String>) {
        let (_, l, w, notes) = wires(src);
        let named: Vec<String> = notes.iter().filter(|n| n.contains("edge-")).cloned().collect();
        let mut out: Vec<(String, String)> = Vec::new();
        let paths: Vec<(&Wire, Vec<(i32, i32)>)> = w.iter().map(|x| (x, full_path(x, &l))).collect();
        let labels: Vec<Option<Rect>> = w.iter().map(|x| (!x.label.is_empty()).then(|| label_area_of(x, &l))).collect();
        for (n, (x, p)) in paths.iter().enumerate() {
            let last = p.len() - 2;
            let mut fault = |what: String| out.push((x.id.clone(), what));
            for (k, s) in p.windows(2).enumerate() {
                for i in &l.icons {
                    if i.id != x.from && i.id != x.to && segment_hits_rect(s[0], s[1], i.rect) { fault(format!("crosses {}", i.id)); }
                    let own = (k == 0 && i.id == x.from) || (k == last && i.id == x.to);
                    if !own && segment_hits_rect(s[0], s[1], label_rect(i)) { fault(format!("crosses the label of {}", i.id)); }
                }
                for f in &l.frames {
                    if segment_hits_rect(s[0], s[1], frame_name_area(f)) { fault(format!("crosses the name of {}", f.id)); }
                }
                for (m, r) in labels.iter().enumerate() {
                    if m != n && r.is_some_and(|r| hits(s[0], s[1], r)) { fault(format!("runs through the label of {}", w[m].id)); }
                }
                for (y, q) in &paths[n + 1..] {
                    for t in q.windows(2) {
                        // Collinear, or parallel so close that draw.io's strokes merge.
                        let near = (0..=2).any(|o| {
                            let shift = |p: (i32, i32)| if s[0].0 == s[1].0 { (p.0 + o, p.1) } else { (p.0, p.1 + o) };
                            shared(shift(s[0]), shift(s[1]), t[0], t[1]) > 1 || shared(s[0], s[1], shift(t[0]), shift(t[1])) > 1
                        });
                        if near { fault(format!("runs along {}", y.id)); }
                    }
                }
            }
        }
        (out, named)
    }

    /// Every fault and every note naming a line, for a definition that should have none.
    fn violations(src: &str) -> Vec<String> {
        let (f, named) = faults(src);
        named.into_iter().chain(f.into_iter().map(|(id, what)| format!("{id} {what}"))).collect()
    }

    /// The example at the top of the definition guide.
    const SHOWCASE: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\nset:\n  azs: 3\n  lang: ja\nchoose:\n  route.ecs.s3: nat\n";

    const HEAVY: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> alb\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - ecs -> internet\n  - waf -- cloudfront\n  - acm -- cloudfront\nchoose:\n  route.ecs.s3: endpoint\n";

    /// Two AZs in Japanese with lines from ECS and Aurora to S3 and the side column: the label of
    /// the line from ECS to S3 has no spot clear of lines and text even over frame sides.
    const CROWDED_S3_JA: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront ~> cloudwatch\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - aurora ~> cloudwatch\n  - waf -- cloudfront\n  - acm -- cloudfront\n  - aurora -> s3\n  - alb ~> s3\nset:\n  azs: 2\n  lang: ja\n  nat: per-az\nchoose:\n  route.ecs.s3: endpoint\n  route.aurora.s3: nat\n";

    /// Two AZs in English with lines from every part to S3 and CloudWatch: the labels of the lines
    /// to CloudWatch have no spot clear of lines and text even over frame sides.
    const CROWDED_SIDE_EN: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - aurora ~> cloudwatch\n  - ecs -> internet\n  - waf -- cloudfront\n  - acm -- cloudfront\n  - user -> alb\n  - aurora -> s3\n  - alb ~> s3\n  - ecs ~> s3\nset:\n  azs: 2\n  lang: en\n  nat: per-az\nchoose:\n  route.aurora.s3: nat\n";

    /// Accepted definitions this version cannot draw without a fault: too many lines and labels
    /// meet in one gap. They are still drawn, and every line at fault must be named in a note.
    fn crowded_definitions() -> Vec<String> {
        vec![CROWDED_S3_JA.to_string(), CROWDED_SIDE_EN.to_string()]
    }

    /// Accepted definitions beyond the acceptance cases: other settings, every allowed link, reversed and extra flows.
    fn more_definitions() -> Vec<String> {
        let heavy = HEAVY;
        let single = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\n  - acm -- cloudfront\n  - ecs -> internet\n";
        let mut srcs = vec![format!("{WEB2}set:\n  lang: ja\n"), format!("{WEB2}set:\n  azs: 3\n  lang: ja\n  readers: 1\n")];
        for extra in ["", "set:\n  azs: 3\n", "set:\n  nat: single\n", "set:\n  azs: 3\n  nat: single\n", "set:\n  lang: ja\n"] {
            srcs.push(format!("{heavy}{extra}"));
            srcs.push(format!("{single}{extra}"));
        }
        srcs.extend([
            "flows:\n  - user -> alb -> ecs\n",
            "flows:\n  - user -> alb -> ecs -> aurora\n  - aurora -> ecs\n",
            "flows:\n  - user -> alb -> ecs -> aurora\n  - ecs -> s3\nchoose:\n  route.ecs.s3: nat\n",
            "flows:\n  - user -> alb -> ecs -> aurora\n  - alb -> aurora\n  - aurora ~> cloudwatch\n  - alb ~> s3\nset:\n  azs: 3\n",
            "flows:\n  - user -> cloudfront -> alb -> ecs\n  - acm -- cloudfront\n  - waf -- cloudfront\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n",
        ].map(String::from));
        srcs.extend(endpoint_ja_definitions());
        srcs.extend([SHOWCASE.to_string(), SHOWCASE.replace("route.ecs.s3: nat", "route.ecs.s3: endpoint")]);
        srcs.extend([ENDPOINT_3AZ_EN.to_string(), ENDPOINT_3AZ_EN.replace("lang: en", "lang: ja")]);
        srcs.extend(crowded_definitions());
        srcs
    }

    #[test]
    fn other_accepted_definitions_route_cleanly() {
        let crowded = crowded_definitions();
        let mut bad = Vec::new();
        for src in more_definitions().iter().chain(&both()).filter(|s| !crowded.contains(s)) {
            let v = violations(src);
            if !v.is_empty() { bad.push(format!("{src}{v:#?}")); }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n----\n"));
    }

    #[test]
    fn no_line_at_fault_comes_out_without_a_note() {
        let crowded = crowded_definitions();
        for src in more_definitions().iter().chain(&both()).chain(&crowded) {
            let (f, named) = faults(src);
            for (id, what) in f {
                assert!(named.iter().any(|n| n.contains(&format!("{id} "))), "{id} {what}, but no note names it, in\n{src}");
            }
        }
        for src in &crowded {
            assert!(more_definitions().contains(src), "{src}");
            assert!(!faults(src).1.is_empty(), "{src}");
        }
    }

    #[test]
    fn a_line_between_icons_at_the_same_height_is_straight() {
        for src in [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n"), format!("{WEB2}set:\n  lang: ja\n")] {
            let (_, l, w, _) = wires(&src);
            let cw = w.iter().find(|x| x.to == "cloudwatch").unwrap();
            assert!(cw.points.is_empty() && cw.exit.pos == cw.entry.pos, "{} bends: {:?} {:?} {:?}", cw.id, cw.exit, cw.entry, cw.points);
            for x in &w {
                let (fr, tr) = (l.rect(&x.from), l.rect(&x.to));
                let sideways = matches!(x.exit.side, Side::Left | Side::Right) && x.entry.side == opposite(x.exit.side);
                if fr.y == tr.y && fr.h == tr.h && sideways {
                    assert!(x.points.is_empty(), "{} is not straight: {:?}", x.id, x.points);
                }
            }
        }
    }

    /// Where a line's label sits, by the same measure the router uses.
    fn label_area_of(x: &Wire, l: &Layout) -> Rect {
        let p = full_path(x, l);
        let len = path_length(&p);
        let d = match (x.label_at, x.label_near_source) {
            (Some(at), _) => label_distance(at, len),
            (None, true) => len / 4,
            (None, false) => len / 2,
        };
        wire_label_area(&x.label, point_along(&p, d).0, x.horizontal_label, x.label_flip)
    }

    /// The definitions the acceptance cases fix: two and three AZs, in English and Japanese.
    fn acceptance_definitions() -> Vec<String> {
        both().into_iter().flat_map(|s| {
            let ja = if s.contains("set:") { format!("{s}  lang: ja\n") } else { format!("{s}set:\n  lang: ja\n") };
            [s, ja]
        }).collect()
    }

    #[test]
    fn no_frame_side_cuts_a_line_label_in_the_acceptance_definitions() {
        for src in acceptance_definitions() {
            let (_, l, w, _) = wires(&src);
            let borders = Scene::new(&l).borders;
            for x in w.iter().filter(|x| !x.label.is_empty()) {
                let area = label_area_of(x, &l);
                for &(c, d) in &borders {
                    assert!(!hits(c, d, area), "a frame side {c:?}-{d:?} cuts the label of {} in\n{src}", x.id);
                }
            }
        }
    }

    #[test]
    fn no_other_line_runs_through_a_line_label() {
        let crowded = crowded_definitions();
        for src in more_definitions().iter().chain(&acceptance_definitions()).filter(|s| !crowded.contains(s)) {
            let (_, l, w, _) = wires(src);
            for x in w.iter().filter(|x| !x.label.is_empty()) {
                let area = label_area_of(x, &l);
                for y in &w {
                    for s in full_path(y, &l).windows(2) {
                        assert!(!hits(s[0], s[1], area), "{} runs through the label of {} in\n{src}", y.id, x.id);
                    }
                }
            }
        }
    }

    #[test]
    fn a_label_moves_off_frame_sides_to_a_clear_stretch() {
        let (_, l, w, _) = wires(&format!("{WEB2}set:\n  lang: ja\n"));
        let get = |id: &str| w.iter().find(|x| x.id == id).unwrap();
        // Its middle is on the AZ and private subnet sides; the stretch inside the private subnet is clear.
        let nat = get("edge-ecs-1c-to-nat-1c");
        assert!(nat.label_at.is_some());
        let area = label_area_of(nat, &l);
        let sub = l.rect("pri-1c");
        assert!(area.x > sub.x && area.x + area.w < sub.x + sub.w && area.y > sub.y, "{area:?} outside {sub:?}");
        // Too long for any stretch on one line, so it goes on two, between the icon and the private subnet's side.
        let cw = get("edge-ecs-1c-to-cloudwatch");
        assert_eq!(cw.label, "ログ・メトリクス\n（全 AZ から）");
        let (area, ecs) = (label_area_of(cw, &l), l.rect("ecs-1c"));
        assert!(area.x >= ecs.x + ecs.w && area.x + area.w < sub.x + sub.w && area.y > sub.y, "{area:?}");
        // A label nothing cuts stays where it was.
        assert_eq!(get("edge-ecs-1a-to-aurora-1a").label_at, None);
        assert_eq!(get("edge-igw-to-alb").label_at, None);
    }

    #[test]
    fn the_replication_line_to_the_far_reader_stays_inside_the_zones() {
        for src in acceptance_definitions().into_iter().filter(|s| s.contains("azs: 3")) {
            let (_, l, w, _) = wires(&src);
            let x = w.iter().find(|x| x.id == "edge-aurora-1a-to-aurora-1d").unwrap();
            let bottom = ["az-1a", "az-1c", "az-1d"].map(|id| { let r = l.rect(id); r.y + r.h }).into_iter().min().unwrap();
            assert!(full_path(x, &l).iter().all(|p| p.1 < bottom), "{:?} goes below the zones at {bottom} in\n{src}", x.points);
        }
    }

    #[test]
    fn wire_ids_are_unique() {
        for src in more_definitions().iter().chain(&both()) {
            let (_, _, w, _) = wires(src);
            let mut ids: Vec<&str> = w.iter().map(|x| x.id.as_str()).collect();
            ids.sort_unstable();
            let n = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), n, "{src}");
        }
    }

    #[test]
    fn a_line_no_route_can_place_is_kept_and_named_in_a_note() {
        let r = Rules::builtin();
        let (m, _) = crate::infer::infer(&crate::def::parse(WEB2).unwrap(), &r);
        let m = m.unwrap();
        let mut l = crate::layout::layout(&m, &r);
        let s3 = l.rect("s3");
        for (k, (dx, dy)) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)].into_iter().enumerate() {
            let rect = Rect { x: s3.x + dx * 62, y: s3.y + dy * 62, w: 60, h: 60 };
            l.icons.push(crate::layout::Icon { id: format!("wall-{k}"), label: String::new(), lines: 1, rect, parent: None });
        }
        let (w, notes) = route(&m, &l, &r);
        assert!(w.iter().any(|x| x.id == "edge-cloudfront-to-s3"));
        assert_eq!(notes, ["The line edge-cloudfront-to-s3 could not avoid every icon, label and other line; check it in the diagram."]);
    }
}
