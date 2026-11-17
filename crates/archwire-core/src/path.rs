//! Laying a line: the sides and shape it is planned with, the check against everything it
//! must not pass over, and the search over the channels between them.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

use crate::layout::{FrameKind, Icon, Layout, Rect};
use crate::lines::Line;
use crate::measure::{frame_name_box, name_area};
use crate::model::{Area, Axis, Model};
use crate::route::{port_point, Bend, Ctx, Keep, LinePlan, Port, Side, Through, LEVEL, RANK_AUTO, RANK_MAIN};

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

pub(crate) type Pt = (i32, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir { Up, Down, Left, Right }

const DIRS: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];
const ALL_SIDES: [Side; 4] = [Side::Top, Side::Bottom, Side::Left, Side::Right];

pub(crate) fn outward(s: Side) -> Dir {
    match s {
        Side::Top => Dir::Up,
        Side::Bottom => Dir::Down,
        Side::Left => Dir::Left,
        Side::Right => Dir::Right,
    }
}

pub(crate) fn reverse(d: Dir) -> Dir {
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

pub(crate) fn opposite(s: Side) -> Side {
    match s {
        Side::Top => Side::Bottom,
        Side::Bottom => Side::Top,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

pub(crate) fn dir_of(a: Pt, b: Pt) -> Option<Dir> {
    match (a.0 == b.0, a.1 == b.1) {
        (true, false) => Some(if b.1 > a.1 { Dir::Down } else { Dir::Up }),
        (false, true) => Some(if b.0 > a.0 { Dir::Right } else { Dir::Left }),
        _ => None,
    }
}

pub(crate) fn center(r: Rect) -> Pt {
    (r.x + r.w / 2, r.y + r.h / 2)
}

/// The rectangle of no width or no height an axis-aligned segment covers.
fn span(a: Pt, b: Pt) -> Rect {
    Rect { x: a.0.min(b.0), y: a.1.min(b.1), w: (a.0 - b.0).abs(), h: (a.1 - b.1).abs() }
}

pub(crate) fn grow(r: Rect, by: i32) -> Rect {
    Rect { x: r.x - by, y: r.y - by, w: r.w + 2 * by, h: r.h + 2 * by }
}

/// True if the axis-aligned segment passes through the inside of the rectangle; touching its edge is not a hit.
pub(crate) fn hits(a: Pt, b: Pt, r: Rect) -> bool {
    let (x1, x2) = (a.0.min(b.0), a.0.max(b.0));
    let (y1, y2) = (a.1.min(b.1), a.1.max(b.1));
    x1 < r.x + r.w && x2 > r.x && y1 < r.y + r.h && y2 > r.y
}

/// How long two collinear segments run together, or 0.
pub(crate) fn shared(a: Pt, b: Pt, c: Pt, d: Pt) -> i32 {
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
pub(crate) fn alongside(a: Pt, b: Pt, c: Pt, d: Pt) -> i32 {
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

/// Drops a point that repeats the one before it; keeps every other point, even in a straight run.
fn dedup(raw: Vec<Pt>) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::new();
    for p in raw {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

pub(crate) fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
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
pub(crate) struct Scene<'a> {
    icons: &'a [Icon],
    labels: Vec<(&'a str, Rect)>,
    frame_names: Vec<Rect>,
    /// The four sides of every frame.
    pub(crate) borders: Vec<(Pt, Pt)>,
    xs: Vec<i32>,
    ys: Vec<i32>,
    /// The planned paths of side lines, with the two parts each joins: other lines may not cross
    /// them, so a line goes round a side part and its partner instead of between them.
    pub(crate) barriers: Vec<(String, String, Vec<Pt>)>,
    /// The VPC frames, and the parts outside every VPC: a line between two such parts may not
    /// pass through the inside of a VPC frame, where it would read as a line to the parts in it.
    /// Lines with an end in a VPC are not held by it.
    pub(crate) fence: Fence,
}

/// The VPC frames, and the ids of the parts outside them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Fence {
    pub(crate) frames: Vec<Rect>,
    pub(crate) outside: Vec<String>,
}

impl Fence {
    /// The VPC frames of the layout, and the parts the model places outside a VPC (neither in a
    /// subnet nor on the VPC's edge).
    pub(crate) fn of(model: &Model, layout: &Layout) -> Fence {
        Fence {
            frames: layout.frames.iter().filter(|f| f.kind == FrameKind::Vpc).map(|f| f.rect).collect(),
            outside: model.nodes.iter().filter(|n| !matches!(n.area, Area::Subnet(_) | Area::VpcEdge)).map(|n| n.id.clone()).collect(),
        }
    }

    /// Whether a line between these two parts is held out of the VPC frames.
    pub(crate) fn holds(&self, from: &str, to: &str) -> bool {
        !self.frames.is_empty() && [from, to].iter().all(|id| self.outside.iter().any(|o| o == id))
    }

    /// Whether the segment of a line between these parts keeps out of the VPC frames, or the line is not held.
    fn lets(&self, a: Pt, b: Pt, (from, to): (&str, &str)) -> bool {
        !self.holds(from, to) || self.frames.iter().all(|&r| !hits(a, b, r))
    }
}

impl<'a> Scene<'a> {
    pub(crate) fn new(layout: &'a Layout) -> Scene<'a> {
        let labels: Vec<(&str, Rect)> = layout.icons.iter().map(|i| (i.id.as_str(), name_area(i))).collect();
        let frame_names: Vec<Rect> = layout.frames.iter().map(frame_name_box).collect();
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
        Scene { icons: &layout.icons, labels, frame_names, borders, xs: channels(ex), ys: channels(ey), barriers: Vec::new(), fence: Fence::default() }
    }

    /// Whether one segment of the line from `from` to `to` stays clear of icons, labels and frame
    /// names, by at least `margin`. The ends' own labels are skipped where the line leaves or enters
    /// below them, and so is the margin around the ends' own icons.
    fn clear(&self, a: Pt, b: Pt, (from, to): (&str, &str), (own_from, own_to): (bool, bool), margin: i32) -> bool {
        self.clear_of(a, b, (from, to), (own_from, own_to), margin, true)
    }

    /// Like `clear`; `barriers` says whether the side lines' planned paths count.
    fn clear_of(&self, a: Pt, b: Pt, (from, to): (&str, &str), (own_from, own_to): (bool, bool), margin: i32, barriers: bool) -> bool {
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
        let barriers = !barriers
            || self
            .barriers
            .iter()
            .filter(|(p, q, _)| ![p, q].iter().any(|x| *x == from || *x == to))
            .all(|(_, _, path)| path.windows(2).all(|s| !hits(a, b, grow(span(s[0], s[1]), margin.max(1)))));
        icons && labels && barriers && self.frame_names.iter().all(|&r| !hits(a, b, grow(r, margin))) && self.fence.lets(a, b, (from, to))
    }

    /// Whether the label of line `me` taking `area` stays `margin` off every frame side, icon, icon
    /// label, frame name, label already placed and other line. Its own line only touches its edge.
    pub(crate) fn label_clear(&self, area: Rect, margin: i32, paths: &[Vec<Pt>], me: usize, placed: &[Rect]) -> bool {
        self.borders_under(area, margin) == 0 && self.label_clear_but_frames(area, margin, paths, me, placed)
    }

    /// How many frame sides come within `margin` of a label taking `area`.
    pub(crate) fn borders_under(&self, area: Rect, margin: i32) -> usize {
        let wide = grow(area, margin);
        self.borders.iter().filter(|&&(c, d)| hits(c, d, wide)).count()
    }

    /// Like `label_clear`, but frame sides may run under the label: a frame side is drawn behind
    /// the text and leaves it readable, where a line, an icon or other text does not.
    pub(crate) fn label_clear_but_frames(&self, area: Rect, margin: i32, paths: &[Vec<Pt>], me: usize, placed: &[Rect]) -> bool {
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
    pub(crate) fn check(&self, path: &[Pt], from: &str, to: &str, out: Dir, inward: Dir, placed: &[Vec<Pt>]) -> Option<usize> {
        self.check_part(path, (from, to), (out, inward), (true, true), placed)
    }

    /// Like `check`, for a part of a line: `own` says which of the part's ends are the line's own
    /// ends at an icon, whose label and margin the part may pass where it leaves or enters.
    fn check_part(&self, path: &[Pt], ends: (&str, &str), dirs: (Dir, Dir), own: (bool, bool), placed: &[Vec<Pt>]) -> Option<usize> {
        self.check_part_of(path, ends, dirs, own, placed, true)
    }

    /// Like `check`, for a line kept whole. It is drawn as planned and never searched, so it cannot
    /// go round the side lines' planned paths, which only send searched lines round them; it may
    /// cross them as it may cross any other line.
    pub(crate) fn check_kept(&self, path: &[Pt], from: &str, to: &str, out: Dir, inward: Dir, placed: &[Vec<Pt>]) -> Option<usize> {
        self.check_part_of(path, (from, to), (out, inward), (true, true), placed, false)
    }

    fn check_part_of(&self, path: &[Pt], (from, to): (&str, &str), (out, inward): (Dir, Dir), own: (bool, bool), placed: &[Vec<Pt>], barriers: bool) -> Option<usize> {
        if path.len() < 2 {
            return None;
        }
        let last = path.len() - 2;
        let mut crossings = 0;
        for (k, s) in path.windows(2).enumerate() {
            let d = dir_of(s[0], s[1])?;
            if (k == 0 && d != out) || (k == last && d != inward) || !self.clear_of(s[0], s[1], (from, to), (k == 0 && own.0, k == last && own.1), 0, barriers) {
                return None;
            }
            crossings += self.against(s[0], s[1], placed)?;
        }
        Some(crossings)
    }

    /// Shortest acceptable path over the channels, counting bends, crossings and frame sides crossed, or None.
    /// Unlike `check`, it keeps a margin around everything and never runs along a frame's side.
    /// `own` says which of the start and the goal are the line's own ends at an icon, as in `check_part`.
    fn search(&self, (start, out): (Pt, Dir), (goal, inward): (Pt, Dir), (from, to): (&str, &str), own: (bool, bool), placed: &[Vec<Pt>]) -> Option<Vec<Pt>> {
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
            // The start is never a finished path, even where the exit and the entry meet: a line
            // of no length has nothing to draw.
            if (xi, yi) == (gx, gy) && DIRS[d] == inward && s != s0 {
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
                if !self.clear(a, b, (from, to), (nfirst && own.0, on_entry && own.1), CLEARANCE) {
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

/// The path a shape gives between two ports, or None for a shape left to the search. Waypoints
/// are kept as written, even in a straight run; only a point repeating the one before is dropped.
/// The corners of the other shapes are dropped where they lie on a straight run.
pub(crate) fn shape_path(shape: &Shape, ep: Pt, np: Pt, out: Dir) -> Option<Vec<Pt>> {
    let raw = shape_points(shape, ep, np, out)?;
    Some(if matches!(shape, Shape::Points(_)) { raw } else { simplify(raw) })
}

/// Every corner a shape gives between two ports, in order, or None for a shape left to the
/// search; only a point repeating the one before is dropped, so a shape that turns back on
/// itself still shows it.
pub(crate) fn shape_points(shape: &Shape, ep: Pt, np: Pt, out: Dir) -> Option<Vec<Pt>> {
    let raw = match shape {
        Shape::Straight => vec![ep, np],
        Shape::ChannelY(y) => vec![ep, (ep.0, *y), (np.0, *y), np],
        Shape::ChannelX(x) => vec![ep, (*x, ep.1), (*x, np.1), np],
        Shape::Elbow if vertical(out) => vec![ep, (ep.0, np.1), np],
        Shape::Elbow => vec![ep, (np.0, ep.1), np],
        Shape::Points(v) => {
            let mut raw = vec![ep];
            raw.extend(v.iter().copied());
            raw.push(np);
            raw
        }
        Shape::Search => return None,
    };
    Some(dedup(raw))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Exit and entry face each other on one line.
    Straight,
    /// Out, along a horizontal channel at this y, then in.
    ChannelY(i32),
    /// Out, along a vertical channel at this x, then in.
    ChannelX(i32),
    /// One bend where the exit's line meets the entry's line.
    Elbow,
    /// Through these waypoints, in order.
    Points(Vec<Pt>),
    Search,
}

/// The sides and shape a line is planned with, before positions on the sides are shared out.
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    pub(crate) exit: Side,
    pub(crate) entry: Side,
    /// Positions fixed by the shape instead of shared out with the side's other lines.
    pub(crate) exit_pin: Option<u8>,
    pub(crate) entry_pin: Option<u8>,
    /// Positions the draft keeps: taken first, and the side's other ends shared out around them.
    pub(crate) exit_fixed: Option<u8>,
    pub(crate) entry_fixed: Option<u8>,
    /// Positions the draft writes for an end it does not fix: taken next, after the fixed ends.
    pub(crate) exit_want: Option<u8>,
    pub(crate) entry_want: Option<u8>,
    pub(crate) shape: Shape,
    pub(crate) rank: u8,
    pub(crate) keep: Keep,
    pub(crate) axis: Axis,
}

pub(crate) fn draft(exit: Side, entry: Side, shape: Shape, rank: u8) -> Draft {
    Draft {
        exit,
        entry,
        exit_pin: None,
        entry_pin: None,
        exit_fixed: None,
        entry_fixed: None,
        exit_want: None,
        entry_want: None,
        shape,
        rank,
        keep: Keep::Free,
        axis: Axis::Right,
    }
}

impl Draft {
    /// Where the exit sits before the sharing out: fixed, written, pinned, else the middle.
    pub(crate) fn exit_start(&self) -> u8 {
        self.exit_fixed.or(self.exit_want).or(self.exit_pin).unwrap_or(50)
    }

    pub(crate) fn entry_start(&self) -> u8 {
        self.entry_fixed.or(self.entry_want).or(self.entry_pin).unwrap_or(50)
    }

    /// The draft a line plan gives. A position the plan writes for an end it does not fix is
    /// taken where it is written unless a fixed end has it (see `share_out`).
    pub(crate) fn from_plan(p: &LinePlan, rank: u8) -> Draft {
        let shape = match &p.bend {
            Bend::Straight => Shape::Straight,
            Bend::AtX(x) => Shape::ChannelX(*x),
            Bend::AtY(y) => Shape::ChannelY(*y),
            Bend::Elbow => Shape::Elbow,
            Bend::Points(v) => Shape::Points(v.clone()),
        };
        let (exit_fixed, entry_fixed) = p.fixed();
        Draft {
            exit: p.exit,
            entry: p.entry,
            exit_pin: None,
            entry_pin: None,
            exit_fixed,
            entry_fixed,
            exit_want: if exit_fixed.is_none() { p.exit_pos } else { None },
            entry_want: if entry_fixed.is_none() { p.entry_pos } else { None },
            shape,
            rank,
            keep: p.keep.clone(),
            axis: p.axis,
        }
    }
}

/// A line through a gate, split there: the kept inner part, and the ends of the outer part with
/// the way the line runs at each. `own` says which of the outer part's ends is at an icon.
struct Split {
    inner: Vec<Pt>,
    start: (Pt, Dir),
    goal: (Pt, Dir),
    own: (bool, bool),
    inner_first: bool,
}

/// Splits the line from `ep` to `np` at the gate. The line runs through the gate the way the
/// inner part runs from it (or to it), so it crosses the frame's side at a right angle; where
/// the gate is the inner end's own port, the way the line leaves or enters there.
fn split(t: &Through, ep: Pt, np: Pt, out: Dir, inward: Dir) -> Split {
    if t.inner_is_start {
        let mut raw = vec![ep];
        raw.extend(t.inner.iter().copied());
        raw.push(t.gate);
        let inner = dedup(raw);
        let n = inner.len();
        let at_gate = if n >= 2 { dir_of(inner[n - 2], inner[n - 1]) } else { None }.unwrap_or(out);
        Split { inner, start: (t.gate, at_gate), goal: (np, inward), own: (false, true), inner_first: true }
    } else {
        let mut raw = vec![t.gate];
        raw.extend(t.inner.iter().copied());
        raw.push(np);
        let inner = dedup(raw);
        let at_gate = if inner.len() >= 2 { dir_of(inner[0], inner[1]) } else { None }.unwrap_or(inward);
        Split { inner, start: (ep, out), goal: (t.gate, at_gate), own: (true, false), inner_first: false }
    }
}

impl Split {
    /// The whole line: the inner part and the outer part, joined at the gate.
    fn join(&self, outer: &[Pt]) -> Vec<Pt> {
        let mut raw = Vec::new();
        if self.inner_first {
            raw.extend(self.inner.iter().copied());
            raw.extend(outer.iter().copied());
        } else {
            raw.extend(outer.iter().copied());
            raw.extend(self.inner.iter().copied());
        }
        dedup(raw)
    }
}

impl<'a> Ctx<'a> {
    /// A line between two icons at the same height that leaves and enters sideways runs straight:
    /// the end alone on its side moves level with the other end. An end the draft fixes never
    /// moves: the other end moves level with it, if that one is free and alone on its side.
    pub(crate) fn straighten_level_lines(&self, specs: &[Line], drafts: &[Draft], ports: &mut [(Port, Port)]) {
        let alone = |ports: &[(Port, Port)], me: usize, id: &str, side: Side| {
            !specs.iter().zip(ports).enumerate().any(|(j, (o, (e, n)))| {
                j != me && ((o.from == id && e.side == side) || (o.to == id && n.side == side))
            })
        };
        for (i, (s, d)) in specs.iter().zip(drafts).enumerate() {
            let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
            let (exit, entry) = ports[i];
            let sideways = matches!(exit.side, Side::Left | Side::Right) && entry.side == opposite(exit.side);
            if fr.y != tr.y || fr.h != tr.h || !sideways || exit.pos == entry.pos {
                continue;
            }
            let (to_free, from_free) = (alone(ports, i, &s.to, entry.side), alone(ports, i, &s.from, exit.side));
            match (d.exit_fixed.is_some(), d.entry_fixed.is_some()) {
                (true, true) => {}
                (true, false) if to_free => ports[i].1.pos = exit.pos,
                (false, true) if from_free => ports[i].0.pos = entry.pos,
                (false, false) if to_free => ports[i].1.pos = exit.pos,
                (false, false) if from_free => ports[i].0.pos = entry.pos,
                _ => {}
            }
        }
    }

    /// A path for the line with these ports: its shape if that is clear, else a search.
    fn try_route(&self, s: &Line, d: &Draft, (exit, entry): (Port, Port), placed: &[Vec<Pt>], search: bool) -> Option<Vec<Pt>> {
        let ep = port_point(self.layout.rect(&s.from), exit);
        let np = port_point(self.layout.rect(&s.to), entry);
        let (out, inward) = (outward(exit.side), reverse(outward(entry.side)));
        if let Some(p) = shape_path(&d.shape, ep, np, out) {
            if self.scene.check(&p, &s.from, &s.to, out, inward, placed).is_some() {
                return Some(p);
            }
            if !search {
                return None;
            }
        }
        self.scene.search((ep, out), (np, inward), (&s.from, &s.to), (true, true), placed)
    }

    /// The path the draft gives with these ports, unchecked, or None for a draft left to the
    /// search. A line through a gate keeps its inner part and takes the shape for its outer part.
    pub(crate) fn draft_path(&self, s: &Line, d: &Draft, (exit, entry): (Port, Port)) -> Option<Vec<Pt>> {
        let ep = port_point(self.layout.rect(&s.from), exit);
        let np = port_point(self.layout.rect(&s.to), entry);
        let (out, inward) = (outward(exit.side), reverse(outward(entry.side)));
        match &d.keep {
            Keep::Inner(t) => {
                let sp = split(t, ep, np, out, inward);
                let outer = shape_path(&d.shape, sp.start.0, sp.goal.0, sp.start.1)?;
                Some(sp.join(&outer))
            }
            // Drawn as planned, corner for corner: a draft that turns back stays as it is and
            // fails the check, instead of being straightened into a line that passes.
            Keep::Whole => shape_points(&d.shape, ep, np, out),
            Keep::Free => shape_path(&d.shape, ep, np, out),
        }
    }

    /// The final path, or Err with the best path there is when none is acceptable. A line kept
    /// whole is drawn as planned and never searched; a line kept inside searches its outer part only.
    pub(crate) fn lay(&self, s: &Line, d: &Draft, ports: (Port, Port), placed: &[Vec<Pt>]) -> Result<Vec<Pt>, Vec<Pt>> {
        let ep = port_point(self.layout.rect(&s.from), ports.0);
        let np = port_point(self.layout.rect(&s.to), ports.1);
        let (out, inward) = (outward(ports.0.side), reverse(outward(ports.1.side)));
        match &d.keep {
            Keep::Free => self.try_route(s, d, ports, placed, true).ok_or_else(|| {
                let shape = if d.shape == Shape::Search { Shape::Elbow } else { d.shape.clone() };
                let path = shape_path(&shape, ep, np, out).expect("not a search");
                // Where the exit and the entry meet, the line still keeps both ends.
                if path.len() < 2 { vec![ep, np] } else { path }
            }),
            Keep::Whole => {
                let path = self.draft_path(s, d, ports).filter(|p| p.len() >= 2).unwrap_or_else(|| vec![ep, np]);
                match self.scene.check_kept(&path, &s.from, &s.to, out, inward, placed) {
                    Some(_) => Ok(path),
                    None => Err(path),
                }
            }
            Keep::Inner(t) => {
                let sp = split(t, ep, np, out, inward);
                let ((a, leave), (b, arrive)) = (sp.start, sp.goal);
                // Where the exit, the gate and the entry meet, the line still keeps both ends,
                // and a line of no length is named.
                let two = |p: Vec<Pt>| if p.len() < 2 { vec![ep, np] } else { p };
                if a == b {
                    let p = sp.join(&[a]);
                    return if p.len() < 2 { Err(two(p)) } else { Ok(p) };
                }
                let ids = (s.from.as_str(), s.to.as_str());
                let planned = shape_path(&d.shape, a, b, leave).filter(|p| p.len() >= 2);
                if let Some(p) = planned.as_ref().filter(|p| self.scene.check_part(p, ids, (leave, arrive), sp.own, placed).is_some()) {
                    return Ok(sp.join(p));
                }
                if let Some(p) = self.scene.search((a, leave), (b, arrive), ids, sp.own, placed) {
                    return Ok(sp.join(&p));
                }
                Err(two(sp.join(&planned.unwrap_or_else(|| vec![a, b]))))
            }
        }
    }

    /// For a line its planned sides cannot carry: a search between the middles of sides no other
    /// line uses, nearest the straight line first. Returns the path and the new ports.
    pub(crate) fn on_free_sides(&self, specs: &[Line], me: usize, ports: &[(Port, Port)], placed: &[Vec<Pt>]) -> Option<(Vec<Pt>, (Port, Port))> {
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

    /// Whether a draft can be drawn past the icons, labels and frame names, with its ports in the
    /// middle of their sides or where the draft fixes or pins them.
    pub(crate) fn fits(&self, s: &Line, d: &Draft) -> bool {
        let exit = self.port(&s.from, d.exit, d.exit_start());
        let entry = self.port(&s.to, d.entry, d.entry_start());
        self.try_route(s, d, (exit, entry), &[], false).is_some()
    }

    pub(crate) fn first_fit(&self, s: &Line, candidates: Vec<Draft>) -> Draft {
        candidates.iter().find(|d| self.fits(s, d)).cloned().unwrap_or_else(|| candidates[0].clone())
    }

    /// Sides for the lines in `rest`, which no shape covers: the first of `candidates`, then of
    /// any two sides, that fits off the sides the main lines in `drafts` use, else the first
    /// candidate that fits at all. The other lines keep off the main lines' sides, where they can,
    /// so those stay centred and straight.
    pub(crate) fn plan_rest(&self, specs: &[Line], rest: &[usize], drafts: &mut [Option<Draft>]) {
        let used: Vec<(&str, Side)> = specs
            .iter()
            .zip(drafts.iter())
            .filter_map(|(s, d)| d.as_ref().filter(|d| d.rank == RANK_MAIN).map(|d| [(s.from.as_str(), d.exit), (s.to.as_str(), d.entry)]))
            .flatten()
            .collect();
        for &i in rest {
            let s = &specs[i];
            let (fr, tr) = (self.layout.rect(&s.from), self.layout.rect(&s.to));
            let candidates = self.candidates(fr, tr);
            let free = |d: &Draft| !used.contains(&(s.from.as_str(), d.exit)) && !used.contains(&(s.to.as_str(), d.entry));
            let any_sides = ALL_SIDES.iter().flat_map(|&e| ALL_SIDES.iter().map(move |&n| draft(e, n, Shape::Search, RANK_AUTO)));
            drafts[i] = Some(
                candidates
                    .iter()
                    .cloned()
                    .chain(any_sides)
                    .find(|d| free(d) && self.fits(s, d))
                    .unwrap_or_else(|| self.first_fit(s, candidates)),
            );
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
    /// A side with ends the drafts fix takes `slots` for all its ends, takes out for each fixed
    /// end the spot nearest its position (the smaller at a tie), and gives the rest to the free
    /// ends in that order, so no free end lands on a fixed one's spot. An end with a position
    /// written by its draft comes after the fixed ends: it keeps that position unless an end
    /// before it on the side has it, and then takes the spot left nearest it (the smaller at a
    /// tie); either way it takes out the spot nearest where it sits. Pinned ends are left out.
    pub(crate) fn share_out(&self, specs: &[Line], drafts: &[Draft]) -> Vec<(Port, Port)> {
        type Key = (i32, i32, usize);
        /// (order, line, is the exit, fixed position, written position)
        type Ends = Vec<(Key, usize, bool, Option<u8>, Option<u8>)>;
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
            if d.exit_fixed.is_some() || d.exit_pin.is_none() {
                sides.entry((s.from.as_str(), side_index(d.exit))).or_default().push((key(d.exit, fc, tc, i), i, true, d.exit_fixed, d.exit_want));
            }
            if d.entry_fixed.is_some() || d.entry_pin.is_none() {
                sides.entry((s.to.as_str(), side_index(d.entry))).or_default().push((key(d.entry, tc, fc, i), i, false, d.entry_fixed, d.entry_want));
            }
        }
        let mut pos: Vec<(u8, u8)> = drafts.iter().map(|d| (d.exit_start(), d.entry_start())).collect();
        let nearest = |spots: &[u8], to: u8| {
            (0..spots.len()).min_by_key(|&k| ((spots[k] as i32 - to as i32).abs(), spots[k])).expect("slots gives a spot for every end")
        };
        for (_, mut ends) in sides {
            ends.sort();
            let mut spots = slots(ends.len());
            let mut taken: Vec<u8> = Vec::new();
            for fixed in ends.iter().filter_map(|e| e.3) {
                spots.remove(nearest(&spots, fixed));
                taken.push(fixed);
            }
            for &(_, i, is_exit, _, want) in ends.iter().filter(|e| e.3.is_none()) {
                let Some(want) = want else { continue };
                let at = if taken.contains(&want) { spots[nearest(&spots, want)] } else { want };
                spots.remove(nearest(&spots, at));
                taken.push(at);
                if is_exit { pos[i].0 = at } else { pos[i].1 = at }
            }
            for (k, &(_, i, is_exit, _, _)) in ends.iter().filter(|e| e.3.is_none() && e.4.is_none()).enumerate() {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
