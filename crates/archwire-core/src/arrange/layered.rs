//! The layout of the parts outside every frame of their own arrangement, in ranks along a main
//! axis: columns from left to right, or rows from top to bottom.
//!
//! Three steps: the rank a part takes, the order of the parts in a rank, and the distances that
//! give every part its place. The Region's contents are laid out first, in the Region frame's own
//! coordinates and from the Region's parts and the lines between them alone; the global rank and
//! the users are then set against them. Which frame a part belongs to is `Node.area`, decided by
//! inference; a part with no main line (`->`) is a side part. Positions are integers, and ties go
//! to the order the definition first wrote the parts and lines in.
//!
//! Every distance is worked out along the main axis `u` (the way main lines run) and the cross
//! axis `v`; `(x, y)` is `(u, v)` when lines run right and `(v, u)` when they run down. Names stay
//! below their icons and frame names at their frames' top-left corners either way, so a part's
//! size is kept in x and y and read as a length along each axis, and every frame keeps its own
//! margin on each side.

use super::Placement;
use crate::def::Rel;
use crate::layout::{icon, Frame, FrameKind, Icon, Layout, Rect};
use crate::measure::{frame_name_box, name_area, name_text_width, wire_label_area, wire_text_size};
use crate::model::{Area, Axis, Model};
use crate::route::{Bend, Keep, LinePlan, Side};
use crate::rules::Rules;

/// The least distance between the centres of two neighbouring columns.
pub(crate) const COL_STEP: i32 = 220;
/// The least distance between the tops of two rows.
pub(crate) const ROW_STEP: i32 = 160;
/// The least distance between the centres of two side parts set side by side in the global rank, along the main axis.
pub(crate) const SIDE_STEP: i32 = 160;

const ICON: i32 = 60;
/// Room between two neighbouring parts' icons and names across the main axis.
const ACROSS_ROOM: i32 = 16;
/// Two names side by side stay at least this far apart along the main axis.
const NAME_ROOM: i32 = 20;
/// Room for a line's label between two ranks, on top of the parts on the line and the label.
const WIRE_ROOM: i32 = 40;
/// More room between two ranks for each further main line that bends between them.
const LANE_ROOM: i32 = 60;
/// From the Region frame's left side to the first icons, and from its top to the first icons.
const REGION_LEFT: i32 = 110;
const REGION_TOP: i32 = 60;
/// From the rightmost and the lowest icon or name to the Region frame.
const REGION_RIGHT: i32 = 50;
const REGION_BOTTOM: i32 = 40;
/// A frame's name starts this far from the frame's left side, as the router reads it.
const FRAME_NAME_INDENT: i32 = 30;
/// A name stays at least this far inside its frame's sides; a frame's name this far from its right side.
const FRAME_ROOM: i32 = 20;
/// From the AWS Cloud frame's left side to the global rank's icons, and from its top to them.
const CLOUD_LEFT: i32 = 80;
const CLOUD_TOP: i32 = 50;
/// From the end of the global rank's icons and names to the Region frame, along the main axis.
const GLOBAL_TO_REGION: i32 = 80;
/// The Region inside the AWS Cloud frame on the right, at the bottom, and on the left when it comes first along the main axis.
const CLOUD_INSET: i32 = 30;
/// From the diagram's top to the AWS Cloud frame, or to the users when they come first.
const CLOUD_Y: i32 = 60;
/// From the AWS Cloud frame's top to the Region frame's top.
const REGION_BELOW_CLOUD: i32 = 50;
/// From the diagram's left to the users' icons and names, or to the AWS Cloud frame when it comes
/// first; from the end of the users to the AWS Cloud frame along the main axis.
const USER_LEFT: i32 = 40;
const USER_TO_CLOUD: i32 = 60;

pub(crate) const CLOUD_FRAME: &str = "aws-cloud";
pub(crate) const REGION_FRAME: &str = "region";
const CLOUD_NAME: &str = "AWS Cloud";

/// From and to, along one axis, relative to an icon's left or top side.
type Span = (i32, i32);

/// One frame's contents laid out in the frame's own coordinates.
struct Block {
    w: i32,
    h: i32,
    icons: Vec<Icon>,
    /// The centre of each main rank along the main axis: the main ranks, then the side parts' own past the last one.
    ranks: Vec<i32>,
    /// How far into the frame across the main axis the parts' cross coordinate 0 lies.
    across: i32,
}

/// The Region's parts on a main flow and the main lines between them.
pub(super) struct Flow {
    /// Indexes into `Model.nodes`, in model order.
    pub(super) parts: Vec<usize>,
    /// The rank of each part, from 1.
    pub(super) col: Vec<usize>,
    /// Indexes into `parts` of the Region parts each part's main lines come from and go to, in model order.
    preds: Vec<Vec<usize>>,
    succs: Vec<Vec<usize>>,
}

/// `(x, y)` read along the main and the cross axis, `(u, v)`. The same swap turns `(u, v)` back into `(x, y)`.
fn uv<T>(axis: Axis, (x, y): (T, T)) -> (T, T) {
    match axis {
        Axis::Right => (x, y),
        Axis::Down => (y, x),
    }
}

/// The layout of a model without parts inside a VPC, with main lines running along `axis`.
pub(super) fn layered(model: &Model, rules: &Rules, axis: Axis) -> Placement {
    let r = cross_step(model, rules, axis);
    let block = lay_region(model, rules, axis);
    let nodes = &model.nodes;
    let index = |id: &str| nodes.iter().position(|n| n.id == id);
    let area = |i: usize| nodes[i].area;
    let targets = |i: usize| -> Vec<usize> {
        model.edges.iter().filter(|e| e.rel == Rel::Main && e.from == nodes[i].id).filter_map(|e| index(&e.to)).collect()
    };

    // One scale across the main axis: 0 is the Region's first part across.
    let mut v: Vec<Option<i32>> = vec![None; nodes.len()];
    for i in &block.icons {
        v[index(&i.id).expect("the Region block holds model parts")] = Some(uv(axis, (i.rect.x, i.rect.y)).1 - block.across);
    }

    // The global rank, after the Region. Its main parts go level with the middle of their
    // targets in the Region, two steps apart, so the step after each stays free for its side parts.
    let global: Vec<usize> = (0..nodes.len()).filter(|&i| area(i) == Area::Cloud).collect();
    let (g_main, g_side): (Vec<usize>, Vec<usize>) = global.iter().partition(|&&i| on_main(model, &nodes[i].id));
    let mut before: Option<i32> = None;
    for &i in &g_main {
        let want = mid(targets(i).into_iter().filter(|&t| area(t) == Area::Region).filter_map(|t| v[t]));
        let at = after(before, want, 2 * r);
        v[i] = Some(at);
        before = Some(at);
    }
    // Side parts with one partner sit one step across from it; ones with more, past everything
    // else in the rank. Each such line of side parts runs along the main axis, in model order.
    let mut side_rows: Vec<(i32, Vec<usize>)> = Vec::new();
    let put = |rows: &mut Vec<(i32, Vec<usize>)>, at: i32, i: usize| match rows.iter_mut().find(|(rv, _)| *rv == at) {
        Some(row) => row.1.push(i),
        None => rows.push((at, vec![i])),
    };
    let mut shared: Vec<usize> = Vec::new();
    for &i in &g_side {
        match partners(model, i).as_slice() {
            [one] if v[*one].is_some() => put(&mut side_rows, v[*one].expect("checked") + r, i),
            _ => shared.push(i),
        }
    }
    if !shared.is_empty() {
        let last = g_main.iter().filter_map(|&i| v[i]).map(|at| at + r).chain(side_rows.iter().map(|row| row.0)).max();
        let at = last.map_or(0, |l| l + r);
        for i in shared {
            put(&mut side_rows, at, i);
        }
    }
    for (at, parts) in &side_rows {
        for &i in parts {
            v[i] = Some(*at);
        }
    }

    // The users, after the global rank: level with the middle of their main targets, a step apart.
    let users: Vec<usize> = (0..nodes.len()).filter(|&i| area(i) == Area::Outside).collect();
    let mut before: Option<i32> = None;
    for &i in &users {
        let at = after(before, mid(targets(i).into_iter().filter_map(|t| v[t])), r);
        v[i] = Some(at);
        before = Some(at);
    }

    // Along the main axis: the users, the AWS Cloud frame, the global rank, the Region frame.
    let (diagram_main, diagram_cross) = uv(axis, (USER_LEFT, CLOUD_Y));
    let (region_main, region_cross) = uv(axis, (CLOUD_INSET, REGION_BELOW_CLOUD));
    let named = |i: usize| icon_of(model, rules, i, icon_at(0, 0), None);
    let main = |i: usize| uv(axis, spans(&named(i))).0;
    let users_span = users.iter().map(|&i| main(i)).fold((0, ICON), widen);
    let user_u = diagram_main - users_span.0;
    let cloud_u = if users.is_empty() { diagram_main } else { user_u + users_span.1 + USER_TO_CLOUD };
    let first_from = g_main.iter().copied().chain(side_rows.iter().map(|row| row.1[0])).map(|i| main(i).0).min().unwrap_or(0);
    let global_u = cloud_u + uv(axis, (CLOUD_LEFT, CLOUD_TOP)).0.max(FRAME_ROOM - first_from);
    let mut u: Vec<Option<i32>> = vec![None; nodes.len()];
    for &i in &g_main {
        u[i] = Some(global_u);
    }
    for (_, parts) in &side_rows {
        let mut at = global_u;
        for (k, &i) in parts.iter().enumerate() {
            if k > 0 {
                at += gap(SIDE_STEP, main(parts[k - 1]), main(i));
            }
            u[i] = Some(at);
        }
    }
    for &i in &users {
        u[i] = Some(user_u);
    }

    let base = diagram_cross + region_cross + block.across;
    let at = |i: usize| match (u[i], v[i]) {
        (Some(a), Some(b)) => {
            let (x, y) = uv(axis, (a, base + b));
            Some(icon_at(x, y))
        }
        _ => None,
    };
    let global_icons: Vec<Icon> =
        global.iter().filter_map(|&i| at(i).map(|r| icon_of(model, rules, i, r, Some(CLOUD_FRAME.into())))).collect();
    let region_u = match global_icons.iter().map(|i| uv(axis, extent(i)).0).max() {
        Some(end) => end + GLOBAL_TO_REGION,
        None => cloud_u + region_main,
    };
    let (region_x, region_y) = uv(axis, (region_u, diagram_cross + region_cross));
    let region = Rect { x: region_x, y: region_y, w: block.w, h: block.h };
    let (cloud_x, cloud_y) = uv(axis, (cloud_u, diagram_cross));
    // The Region frame is drawn only when a part is in it; else the AWS Cloud frame closes
    // around the global rank and its own name.
    let has_region = nodes.iter().any(|n| n.area == Area::Region);
    let (inner_right, inner_bottom) = if has_region {
        (global_icons.iter().map(|i| extent(i).0 + REGION_RIGHT).fold(right(region), i32::max), bottom(region))
    } else {
        let cloud_name = Frame { id: CLOUD_FRAME.into(), kind: FrameKind::Cloud, label: CLOUD_NAME.into(), rect: icon_at(cloud_x, cloud_y), parent: None };
        let name_right = right(frame_name_box(&cloud_name)) + FRAME_ROOM - CLOUD_INSET;
        let icons_right = global_icons.iter().map(|i| right(i.rect) + CLOUD_LEFT - CLOUD_INSET).chain(global_icons.iter().map(|i| extent(i).0 + FRAME_ROOM - CLOUD_INSET));
        (icons_right.fold(name_right, i32::max), cloud_y + REGION_BELOW_CLOUD)
    };
    let cloud_bottom = global_icons.iter().map(|i| extent(i).1 + REGION_BOTTOM).fold(inner_bottom, i32::max) + CLOUD_INSET;
    let cloud = Rect { x: cloud_x, y: cloud_y, w: inner_right + CLOUD_INSET - cloud_x, h: cloud_bottom - cloud_y };

    let mut icons: Vec<Icon> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let placed = match area(i) {
            Area::Region => block.icons.iter().find(|b| b.id == n.id).map(|b| {
                let mut b = b.clone();
                b.rect.x += region.x;
                b.rect.y += region.y;
                b
            }),
            Area::Cloud => global_icons.iter().find(|g| g.id == n.id).cloned(),
            Area::Outside => at(i).map(|r| icon_of(model, rules, i, r, None)),
            // Inside a VPC, which this layout does not draw.
            Area::VpcEdge | Area::Subnet(_) => None,
        };
        icons.extend(placed);
    }
    let frame = |id: &str, kind: FrameKind, label: String, rect: Rect, parent: Option<&str>| Frame {
        id: id.into(),
        kind,
        label,
        rect,
        parent: parent.map(String::from),
    };
    let mut frames = vec![frame(CLOUD_FRAME, FrameKind::Cloud, CLOUD_NAME.into(), cloud, None)];
    if has_region {
        frames.push(frame(REGION_FRAME, FrameKind::Region, region_name(model, rules), region, Some(CLOUD_FRAME)));
    }
    let layout = Layout { frames, icons };

    let col_x: Vec<i32> = block.ranks.iter().map(|c| region_u + c).collect();
    let lines = plans(model, &layout, &col_x, axis);
    Placement { layout, lines, row_step: r, col_x, crossings: Vec::new(), lanes: Vec::new() }
}

/// The Region's contents, from the Region's parts and the lines between them alone.
fn lay_region(model: &Model, rules: &Rules, axis: Axis) -> Block {
    let r = cross_step(model, rules, axis);
    let (least_main, _) = uv(axis, (COL_STEP, ROW_STEP));
    let (start_main, start_cross) = uv(axis, (REGION_LEFT, REGION_TOP));
    let f = flow(model);
    let vs = rows(&f, r, true);

    // Main ranks: their lengths come from the main parts alone, so side parts never move them.
    let named = |i: usize| icon_of(model, rules, i, icon_at(0, 0), None);
    let main = |i: usize| uv(axis, spans(&named(i))).0;
    let on_line = |i: usize| uv(axis, on_lines(&named(i))).0;
    let ranks = f.col.iter().copied().max().unwrap_or(0);
    let mut span = vec![(0, ICON); ranks + 1];
    let mut line = vec![(0, ICON); ranks + 1];
    for (k, &i) in f.parts.iter().enumerate() {
        span[f.col[k]] = widen(span[f.col[k]], main(i));
        line[f.col[k]] = widen(line[f.col[k]], on_line(i));
    }
    let local = |id: &str| f.parts.iter().position(|&i| model.nodes[i].id == id);
    let mut u = vec![0; ranks + 1];
    if ranks > 0 {
        u[1] = start_main.max(FRAME_ROOM - span[1].0);
    }
    for c in 1..ranks {
        // The main lines from this rank into the next, and how many of them bend on the way.
        let lines: Vec<(usize, usize, &str)> = model
            .edges
            .iter()
            .filter(|e| e.rel == Rel::Main)
            .filter_map(|e| Some((local(&e.from)?, local(&e.to)?, e.label.as_str())))
            .filter(|&(a, b, _)| f.col[a] == c && f.col[b] == c + 1)
            .collect();
        let label = lines.iter().map(|l| uv(axis, wire_text_size(&rules.text(l.2, model.lang))).0).max().unwrap_or(0);
        let bending = lines.iter().filter(|&&(a, b, _)| vs[a] != vs[b]).count() as i32;
        u[c + 1] = u[c] + rank_gap(least_main, (span[c].1, span[c + 1].0), (line[c].1, line[c + 1].0), label, bending);
    }
    // (model index, u, v) of every Region part.
    let mut spots: Vec<(usize, i32, i32)> = f.parts.iter().enumerate().map(|(k, &i)| (i, u[f.col[k]], vs[k])).collect();

    // Side parts take one step across past the main parts, at the rank of the part their first
    // side line joins, or the next rank on where their names keep clear of the side parts placed
    // before them, or past the last rank.
    let sides: Vec<usize> =
        (0..model.nodes.len()).filter(|&i| model.nodes[i].area == Area::Region && !f.parts.contains(&i)).collect();
    let side_v = vs.iter().max().map_or(0, |m| m + r);
    let mut placed: Vec<(i32, Span)> = Vec::new();
    let mut extra: Vec<i32> = Vec::new();
    for &i in &sides {
        let si = main(i);
        let partner = partners(model, i).into_iter().next();
        let want = partner.and_then(|p| f.parts.iter().position(|&j| j == p)).map_or(1, |k| f.col[k]);
        let apart = |pu: i32, ps: Span, cu: i32| if cu >= pu { cu - pu >= gap(least_main, ps, si) } else { pu - cu >= gap(least_main, si, ps) };
        let clear = |cu: i32| cu + si.0 >= FRAME_ROOM && placed.iter().all(|&(pu, ps)| apart(pu, ps, cu));
        let cu = match (want..=ranks).map(|c| u[c]).find(|&cu| clear(cu)) {
            Some(cu) => cu,
            None => {
                let base = if ranks > 0 { u[ranks] + least_main } else { start_main };
                placed.iter().map(|&(pu, ps)| pu + gap(least_main, ps, si)).chain([base, FRAME_ROOM - si.0]).max().expect("not empty")
            }
        };
        if ranks == 0 || cu > u[ranks] {
            extra.push(cu);
        }
        placed.push((cu, si));
        spots.push((i, cu, side_v));
    }
    spots.sort_by_key(|s| s.0);

    // The parts' cross coordinate 0 lies past the frame's margin, far enough in for every name,
    // and, for lines coming in across the frame's first side, clear of the frame's own name.
    let probe = Frame { id: REGION_FRAME.into(), kind: FrameKind::Region, label: region_name(model, rules), rect: icon_at(0, 0), parent: None };
    let name = frame_name_box(&probe);
    let name_across = uv(axis, (name.x + name.w, name.y + name.h)).1;
    let across = spots
        .iter()
        .map(|&(i, _, at)| FRAME_ROOM - uv(axis, spans(&named(i))).1 .0 - at)
        .fold(start_cross.max(name_across + FRAME_ROOM - ICON / 2), i32::max);

    let icons: Vec<Icon> = spots
        .iter()
        .map(|&(i, cu, cv)| {
            let (x, y) = uv(axis, (cu, across + cv));
            icon_of(model, rules, i, icon_at(x, y), Some(REGION_FRAME.into()))
        })
        .collect();
    let (mut bw, mut bh) = (REGION_LEFT + ICON + REGION_RIGHT, REGION_TOP + ICON + REGION_BOTTOM);
    if !icons.is_empty() {
        bw = icons.iter().map(|i| extent(i).0).max().expect("not empty") + REGION_RIGHT;
        bh = icons.iter().map(|i| extent(i).1).max().expect("not empty") + REGION_BOTTOM;
    }
    // Wide enough for the frame's own name.
    bw = bw.max(FRAME_NAME_INDENT + name_text_width(&region_name(model, rules)) + FRAME_ROOM);
    let mut centres: Vec<i32> = u[1..].iter().copied().chain(extra).map(|c| c + ICON / 2).collect();
    centres.sort_unstable();
    centres.dedup();
    Block { w: bw, h: bh, icons, ranks: centres, across }
}

/// The step across the main axis: from the start of one part to the start of the next, past the
/// longest part across and the label a main line carries beside it, with some room, and at least
/// the least step in that direction.
pub(super) fn cross_step(model: &Model, rules: &Rules, axis: Axis) -> i32 {
    let (_, least) = uv(axis, (COL_STEP, ROW_STEP));
    let named = |i: usize| icon_of(model, rules, i, icon_at(0, 0), None);
    let parts: Vec<Span> = (0..model.nodes.len()).map(|i| uv(axis, spans(&named(i))).1).collect();
    let mut from = parts.iter().map(|s| s.0).min().unwrap_or(0);
    let mut to = parts.iter().map(|s| s.1).max().unwrap_or(0);
    // A main line leaves from the middle of its part, and its label sits by the line where it
    // runs along the main axis: above a horizontal stretch, beside a vertical one.
    let above = uv(axis, (true, false)).0;
    for e in model.edges.iter().filter(|e| e.rel == Rel::Main && !e.label.is_empty()) {
        let l = wire_label_area(&rules.text(&e.label, model.lang), (ICON / 2, ICON / 2), above, false);
        let (lf, lt) = uv(axis, ((l.x, l.x + l.w), (l.y, l.y + l.h))).1;
        from = from.min(lf);
        to = to.max(lt);
    }
    least.max(to - from + ACROSS_ROOM)
}

/// The Region's parts on a main flow, each in the rank one past the furthest of its
/// predecessors in the Region. Lines from outside the Region do not count.
pub(super) fn flow(model: &Model) -> Flow {
    let parts: Vec<usize> = (0..model.nodes.len())
        .filter(|&i| model.nodes[i].area == Area::Region && on_main(model, &model.nodes[i].id))
        .collect();
    let local = |id: &str| parts.iter().position(|&i| model.nodes[i].id == id);
    let n = parts.len();
    let (mut preds, mut succs) = (vec![Vec::new(); n], vec![Vec::new(); n]);
    for e in model.edges.iter().filter(|e| e.rel == Rel::Main) {
        if let (Some(a), Some(b)) = (local(&e.from), local(&e.to)) {
            if !succs[a].contains(&b) {
                succs[a].push(b);
                preds[b].push(a);
            }
        }
    }
    // The accepted main pairs have no cycle, so n rounds settle the longest paths in any order.
    let mut col = vec![1usize; n];
    for _ in 0..n {
        for i in 0..n {
            col[i] = preds[i].iter().map(|&p| col[p] + 1).max().unwrap_or(1);
        }
    }
    Flow { parts, col, preds, succs }
}

/// The cross coordinate of every part of `f`, a step `r` apart in each rank, with the smallest at 0.
pub(super) fn rows(f: &Flow, r: i32, sweep_back: bool) -> Vec<i32> {
    let last = f.col.iter().copied().max().unwrap_or(0);
    let in_rank = |c: usize| -> Vec<usize> { (0..f.parts.len()).filter(|&i| f.col[i] == c).collect() };
    let mut v = vec![0; f.parts.len()];
    // From the first rank: its parts one after another, every other part level with the middle
    // of its predecessors, and at least a step past the part before it.
    for c in 1..=last {
        let mut before: Option<i32> = None;
        for i in in_rank(c) {
            let at = after(before, mid(f.preds[i].iter().map(|&p| v[p])), r);
            v[i] = at;
            before = Some(at);
        }
    }
    // From the last rank back: a part with two or more targets moves toward their middle, as far
    // as the part before it (already moved) and the part after it (not yet moved) leave a step.
    if sweep_back {
        for c in (1..last).rev() {
            let rank = in_rank(c);
            for (k, &i) in rank.iter().enumerate() {
                if f.succs[i].len() < 2 {
                    continue;
                }
                let mut at = mid(f.succs[i].iter().map(|&s| v[s])).expect("two or more targets");
                if k > 0 {
                    at = at.max(v[rank[k - 1]] + r);
                }
                if k + 1 < rank.len() {
                    at = at.min(v[rank[k + 1]] - r);
                }
                v[i] = at;
            }
        }
    }
    if let Some(first) = v.iter().copied().min() {
        v.iter_mut().for_each(|at| *at -= first);
    }
    v
}

/// The line plans, in model order, one per semantic edge. Main lines leave at the end of their
/// part along the main axis and enter at the start of the next, bending halfway to whatever comes
/// next along the main axis; side lines join the part before across to the part after, bending
/// halfway between what lies on the line of the first and the second. Every plan is a first try
/// the router may change, on no band, along `axis`.
fn plans(model: &Model, layout: &Layout, col_u: &[i32], axis: Axis) -> Vec<LinePlan> {
    let find = |id: &str| layout.icons.iter().find(|i| i.id == id);
    let ((main_end, cross_end), (main_start, cross_start)) = (uv(axis, (Side::Right, Side::Bottom)), uv(axis, (Side::Left, Side::Top)));
    let at = |i: &Icon| uv(axis, (i.rect.x, i.rect.y));
    let mut out = Vec::new();
    for (edge, e) in model.edges.iter().enumerate() {
        let (Some(a), Some(b)) = (find(&e.from), find(&e.to)) else { continue };
        let ((au, av), (bu, bv)) = (at(a), at(b));
        let (exit, entry, bend) = if e.rel == Rel::Main {
            let bend = if av == bv {
                Bend::Straight
            } else {
                let from = au + uv(axis, on_lines(a)).0 .1;
                let next = layout
                    .frames
                    .iter()
                    .map(|f| uv(axis, (f.rect.x, f.rect.y)).0)
                    .chain(col_u.iter().map(|c| c - ICON / 2))
                    .filter(|&s| s > from)
                    .min()
                    .unwrap_or(from + 2 * NAME_ROOM);
                bend_at(axis, true, (from + next).div_euclid(2))
            };
            (main_end, main_start, bend)
        } else {
            let first = av <= bv;
            let (one, one_at, two_at) = if first { (a, (au, av), (bu, bv)) } else { (b, (bu, bv), (au, av)) };
            let bend = if one_at.0 == two_at.0 {
                Bend::Straight
            } else {
                bend_at(axis, false, (one_at.1 + uv(axis, on_lines(one)).1 .1 + two_at.1).div_euclid(2))
            };
            if first {
                (cross_end, cross_start, bend)
            } else {
                (cross_start, cross_end, bend)
            }
        };
        out.push(LinePlan {
            edge,
            from: e.from.clone(),
            to: e.to.clone(),
            exit,
            entry,
            bend,
            exit_pos: None,
            entry_pos: None,
            keep: Keep::Free,
            tracks: Vec::new(),
            axis,
        });
    }
    out
}

/// A bend at `at`, a coordinate along the main axis (`main`) or across it.
fn bend_at(axis: Axis, main: bool, at: i32) -> Bend {
    if uv(axis, (main, !main)).0 {
        Bend::AtX(at)
    } else {
        Bend::AtY(at)
    }
}

/// The distance between the starts of two neighbouring Region ranks along the main axis: past the
/// end of the first rank's parts and the start of the second's, with room for the widest label of
/// the main lines from one into the other past what lies on their lines, and more for each
/// further line that bends between them.
pub(super) fn rank_gap(least: i32, parts: (i32, i32), on_line: (i32, i32), label: i32, bending: i32) -> i32 {
    least.max(parts.0 - parts.1 + NAME_ROOM).max(on_line.0 - on_line.1 + label + WIRE_ROOM) + (bending - 1).max(0) * LANE_ROOM
}

/// The distance between the starts of two parts one after the other along the main axis: their
/// names apart, and at least `least`.
fn gap(least: i32, first: Span, second: Span) -> i32 {
    least.max(first.1 - second.0 + NAME_ROOM)
}

/// Along x and along y, from the icon's left and top sides: what the part's icon and name take.
fn spans(i: &Icon) -> (Span, Span) {
    let (r, n) = (i.rect, name_area(i));
    ((0.min(n.x - r.x), r.w.max(n.x + n.w - r.x)), (0.min(n.y - r.y), r.h.max(n.y + n.h - r.y)))
}

/// Like `spans`, but only what lies on the lines through the icon's centre: along x on the
/// horizontal one, along y on the vertical one. A name lies on the vertical line only.
fn on_lines(i: &Icon) -> (Span, Span) {
    let (r, n) = (i.rect, name_area(i));
    let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
    let (mut x, mut y) = ((0, r.w), (0, r.h));
    if n.y <= cy && cy < n.y + n.h {
        x = widen(x, (n.x - r.x, n.x + n.w - r.x));
    }
    if n.x <= cx && cx < n.x + n.w {
        y = widen(y, (n.y - r.y, n.y + n.h - r.y));
    }
    (x, y)
}

fn widen(a: Span, b: Span) -> Span {
    (a.0.min(b.0), a.1.max(b.1))
}

fn on_main(model: &Model, id: &str) -> bool {
    model.edges.iter().any(|e| e.rel == Rel::Main && (e.from == id || e.to == id))
}

/// The parts a side part's lines join, in model order, each once.
fn partners(model: &Model, i: usize) -> Vec<usize> {
    let id = &model.nodes[i].id;
    let mut out: Vec<usize> = Vec::new();
    for e in model.edges.iter().filter(|e| e.rel != Rel::Main) {
        let other = if &e.from == id {
            &e.to
        } else if &e.to == id {
            &e.from
        } else {
            continue;
        };
        if let Some(j) = model.nodes.iter().position(|n| &n.id == other) {
            if !out.contains(&j) {
                out.push(j);
            }
        }
    }
    out
}

fn icon_of(model: &Model, rules: &Rules, i: usize, rect: Rect, parent: Option<String>) -> Icon {
    let n = &model.nodes[i];
    let s = rules.service(&n.service).unwrap_or_else(|| panic!("model has unknown service `{}`", n.service));
    icon(n, s, rules, model.lang, rect, parent)
}

fn region_name(model: &Model, rules: &Rules) -> String {
    rules.text(&format!("region.{}", model.region), model.lang)
}

/// The cross coordinate of the next part in a rank: `want` if there is room, else a `step` past the part before.
fn after(before: Option<i32>, want: Option<i32>, step: i32) -> i32 {
    match before {
        None => want.unwrap_or(0),
        Some(a) => want.map_or(a + step, |w| w.max(a + step)),
    }
}

/// Halfway between the smallest and the largest, rounded down.
fn mid(values: impl IntoIterator<Item = i32>) -> Option<i32> {
    let v: Vec<i32> = values.into_iter().collect();
    Some((v.iter().min()? + v.iter().max()?).div_euclid(2))
}

/// The right and the bottom end of a part's icon and name.
fn extent(i: &Icon) -> (i32, i32) {
    let l = name_area(i);
    (right(i.rect).max(right(l)), bottom(i.rect).max(bottom(l)))
}

fn icon_at(x: i32, y: i32) -> Rect {
    Rect { x, y, w: ICON, h: ICON }
}

fn right(r: Rect) -> i32 {
    r.x + r.w
}

fn bottom(r: Rect) -> i32 {
    r.y + r.h
}
#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use super::super::check::{check, overlaps};
    use super::super::{arrange, BlockOrder, Choices, SpanRow};
    use crate::measure::{NAME_CHARS, NAME_DRAWN, NAME_WIDE_CHARS, NAME_WIDE_DRAWN};
    use crate::route::{port_point, route_planned, Wire};

    pub(in crate::arrange) const CHOICES: Choices = Choices { span_row: SpanRow::AboveZones, block_order: BlockOrder::First };

    /// The model of a definition without a VPC, and its layout with lines running along `axis`.
    pub(in crate::arrange) fn arranged(src: &str, axis: Axis) -> (Model, Placement) {
        let r = Rules::builtin();
        let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        let p = arrange(&m, &r, axis, CHOICES).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
        (m, p)
    }
    pub(in crate::arrange) fn placed(src: &str) -> (Model, Placement) {
        arranged(src, Axis::Right)
    }
    fn at(p: &Placement, id: &str) -> Rect {
        p.layout.rect(id)
    }
    /// y relative to the smallest y of the Region's main parts, in rows.
    fn rel_y(p: &Placement, id: &str, top: &str) -> i32 {
        at(p, id).y - at(p, top).y
    }
    fn cx(p: &Placement, id: &str) -> i32 {
        at(p, id).x + 30
    }
    fn right(r: Rect) -> i32 {
        r.x + r.w
    }
    /// Where a part sits inside the Region frame.
    fn in_region(p: &Placement, id: &str) -> (i32, i32) {
        let (a, f) = (at(p, id), at(p, "region"));
        (a.x - f.x, a.y - f.y)
    }
    fn label_of(p: &Placement, id: &str) -> Rect {
        name_area(p.layout.icons.iter().find(|i| i.id == id).unwrap())
    }
    fn plan<'a>(p: &'a Placement, from: &str, to: &str) -> &'a LinePlan {
        p.lines.iter().find(|l| l.from == from && l.to == to).unwrap_or_else(|| panic!("no plan {from} -> {to}"))
    }

    pub(in crate::arrange) const TARGET: &str = "flows:
  - user -> cloudfront -> s3
  - user -> apigateway(http) -> lambda -> dynamodb
  - cognito -- apigateway
  - lambda ~> cloudwatch
set:
  lang: ja
";

    #[test]
    fn the_target_definition_reads_like_the_mock() {
        let (_, p) = placed(TARGET);
        let r = p.row_step;
        let c = |k: usize| p.col_x[k - 1];
        assert_eq!((cx(&p, "s3"), cx(&p, "apigateway"), cx(&p, "lambda"), cx(&p, "dynamodb")), (c(1), c(1), c(2), c(3)));
        let rows: Vec<i32> = ["s3", "apigateway", "lambda", "dynamodb"].iter().map(|id| rel_y(&p, id, "s3")).collect();
        assert_eq!(rows, [0, r, r, r]);
        assert_eq!(at(&p, "cloudfront").y, at(&p, "s3").y);
        assert_eq!((cx(&p, "cognito"), rel_y(&p, "cognito", "s3")), (c(1), 2 * r));
        assert_eq!((cx(&p, "cloudwatch"), rel_y(&p, "cloudwatch", "s3")), (c(2), 2 * r));
        assert_eq!(at(&p, "user").y, (at(&p, "cloudfront").y + at(&p, "apigateway").y).div_euclid(2));
        let (cloud, region, cf) = (at(&p, "aws-cloud"), at(&p, "region"), at(&p, "cloudfront"));
        assert!(right(at(&p, "user")) < cloud.x && cloud.x < cf.x, "{cloud:?} {cf:?}");
        assert!(right(cf) < region.x);
        let frames: Vec<&str> = p.layout.frames.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(frames, ["aws-cloud", "region"]);
    }

    const SHARED: &str = "flows:
  - user -> apigateway(http) -> lambda.a -> dynamodb.x
  - lambda.a -> dynamodb.y
  - apigateway -> lambda.b -> dynamodb.x
  - lambda.b -> dynamodb.y
";

    #[test]
    fn two_functions_sharing_two_tables_follow_the_hand_trace() {
        let r = Rules::builtin();
        let (m, _) = crate::flow_infer::infer(&crate::def::parse(SHARED).unwrap(), &r);
        let m = m.unwrap();
        let rs = cross_step(&m, &r, Axis::Right);
        let f = flow(&m);
        let ys = |sweep_right: bool| {
            let y = rows(&f, rs, sweep_right);
            ["apigateway", "lambda.a", "lambda.b", "dynamodb.x", "dynamodb.y"]
                .map(|id| y[f.parts.iter().position(|&i| m.nodes[i].id == id).unwrap()])
        };
        assert_eq!(ys(false), [0, 0, rs, rs / 2, 3 * rs / 2]);
        // lambda.a wants R, between its two tables, but stays at lambda.b - R = 0.
        assert_eq!(ys(true), [rs / 2, 0, rs, rs / 2, 3 * rs / 2]);

        let (_, p) = placed(SHARED);
        let rr = p.row_step;
        let got = ["apigateway", "lambda.a", "lambda.b", "dynamodb.x", "dynamodb.y"].map(|id| rel_y(&p, id, "lambda.a"));
        assert_eq!(got, [rr / 2, 0, rr, rr / 2, 3 * rr / 2]);
        assert_eq!(at(&p, "lambda.a").x, at(&p, "lambda.b").x);
    }

    #[test]
    fn the_same_with_the_shared_lines_written_in_another_order() {
        let src = "flows:
  - apigateway -> lambda.b -> dynamodb.x
  - lambda.b -> dynamodb.y
  - user -> apigateway(http) -> lambda.a -> dynamodb.x
  - lambda.a -> dynamodb.y
";
        let (m, p) = placed(src);
        assert!(check(&m, &p).is_empty());
        assert!(at(&p, "lambda.b").y < at(&p, "lambda.a").y);
    }

    #[test]
    fn a_branching_function_sits_between_its_targets() {
        let (_, p) = placed("flows:
  - user -> cloudfront -> s3.web
  - user -> apigateway(http) -> lambda -> dynamodb
  - lambda -> s3.uploads
");
        assert_eq!(at(&p, "lambda").y, (at(&p, "dynamodb").y + at(&p, "s3.uploads").y).div_euclid(2));
        assert_ne!(at(&p, "lambda").y, at(&p, "apigateway").y);
    }

    #[test]
    fn a_crossing_into_the_region_does_not_decide_the_region() {
        let (_, p) = placed("flows:
  - user -> cloudfront -> s3
  - user -> apigateway(http) -> lambda -> s3
");
        let c = |k: usize| p.col_x[k - 1];
        assert_eq!((cx(&p, "apigateway"), cx(&p, "lambda"), cx(&p, "s3")), (c(1), c(2), c(3)));
        let rows = ["apigateway", "lambda", "s3", "cloudfront", "user"].map(|id| rel_y(&p, id, "apigateway"));
        assert_eq!(rows, [0; 5]);
    }

    /// A name's box from the widths draw.io was measured to draw, apart from the layout's own measure.
    fn drawn_name(i: &Icon) -> Rect {
        let ten = |c: char| {
            NAME_CHARS
                .chars()
                .position(|x| x == c)
                .map(|k| NAME_DRAWN[k])
                .or_else(|| NAME_WIDE_CHARS.chars().position(|x| x == c).map(|k| NAME_WIDE_DRAWN[k]))
                .unwrap_or_else(|| panic!("{c:?} in `{}` was not measured", i.label))
        };
        let tens = i.label.lines().map(|l| l.chars().map(ten).sum::<i32>()).max().unwrap_or(0);
        let half = (tens + 19) / 20;
        Rect { x: i.rect.x + i.rect.w / 2 - half, y: i.rect.y + i.rect.h + 4, w: 2 * half, h: 14 * i.lines as i32 + 6 }
    }
    /// No two names as drawn overlap, and no name as drawn lies over another icon.
    fn assert_names_apart(p: &Placement) {
        let icons = &p.layout.icons;
        for (k, a) in icons.iter().enumerate() {
            for b in &icons[k + 1..] {
                let (na, nb) = (drawn_name(a), drawn_name(b));
                assert!(!overlaps(na, nb), "the names of {} and {} overlap: {na:?} {nb:?}", a.id, b.id);
                assert!(!overlaps(na, b.rect) && !overlaps(a.rect, nb), "{} and {} overlap", a.id, b.id);
            }
        }
    }

    #[test]
    fn names_at_the_longest_allowed_do_not_overlap() {
        let long = "a".repeat(crate::def::MAX_TAG);
        let (m, p) = placed(&format!("flows:\n  - user -> apigateway(http) -> lambda.{long} -> dynamodb.{long}\n  - lambda.{long} -> dynamodb.b{}\n", &long[1..]));
        assert!(check(&m, &p).is_empty());
        assert_names_apart(&p);
        assert!(p.col_x.windows(2).all(|w| w[1] - w[0] >= COL_STEP), "{:?}", p.col_x);
    }

    #[test]
    fn the_widest_letters_at_the_longest_allowed_do_not_overlap_anywhere() {
        // A name after the dot holds lower-case letters only, so `m` and `w` are its widest letters.
        for c in ["m", "w"] {
            let t = |k: &str| format!("{k}{}", c.repeat(crate::def::MAX_TAG - k.len()));
            let src = format!(
                "flows:
  - user -> cloudfront.{cf} -> s3.{s3}
  - user -> apigateway(http) -> lambda.{la} -> dynamodb.{da}
  - apigateway -> lambda.{lb} -> dynamodb.{db}
  - waf.{waf} -- cloudfront.{cf}
  - acm.{acm} -- cloudfront.{cf}
  - cognito.{cog} -- apigateway
  - lambda.{la} ~> cloudwatch.{cw}
  - apigateway ~> cloudwatch.{cw2}
",
                cf = t("c"), s3 = t("s"), la = t("a"), lb = t("b"), da = t("d"), db = t("e"),
                waf = t("w"), acm = t("k"), cog = t("g"), cw = t("x"), cw2 = t("y"),
            );
            let (m, p) = placed(&src);
            assert!(check(&m, &p).is_empty(), "{c}");
            assert_names_apart(&p);
        }
    }

    /// The gap between two columns of parts `a` and `b` wide, by their widths as the layout read
    /// them before it read lengths along an axis.
    fn col_gap(a: i32, b: i32, wire: i32, bending: i32) -> i32 {
        rank_gap(COL_STEP, (30 + (a + 1) / 2, 30 - b / 2), (ICON, 0), wire, bending)
    }

    #[test]
    fn names_wider_than_the_column_step_widen_the_columns() {
        assert_eq!(col_gap(60, 60, 0, 0), COL_STEP);
        assert_eq!(col_gap(300, 300, 0, 0), 300 + 20);
        assert_eq!(col_gap(301, 300, 0, 0), 301 + 20);
        assert_eq!(col_gap(60, 60, 200, 1), 60 + 200 + 40);
    }

    #[test]
    fn each_further_bending_main_line_widens_its_gap() {
        assert_eq!(col_gap(60, 60, 0, 0), COL_STEP);
        assert_eq!(col_gap(60, 60, 0, 1), COL_STEP);
        assert_eq!(col_gap(60, 60, 0, 2), COL_STEP + 60);
        assert_eq!(col_gap(60, 60, 0, 3), COL_STEP + 120);
        assert_eq!(col_gap(300, 300, 0, 2), 320 + 60);

        // One function: one straight line. Two: API Gateway between them, two bending lines.
        let (_, one) = placed("flows:\n  - user -> apigateway(http) -> lambda\n");
        assert_eq!(one.col_x[1] - one.col_x[0], COL_STEP);
        let two = "flows:\n  - user -> apigateway(http) -> lambda.a\n  - apigateway -> lambda.b\n";
        let (_, p) = placed(two);
        assert_ne!(at(&p, "apigateway").y, at(&p, "lambda.a").y);
        assert_ne!(at(&p, "apigateway").y, at(&p, "lambda.b").y);
        assert_eq!(p.col_x[1] - p.col_x[0], COL_STEP + 60);
        // Side lines add no width.
        let (_, q) = placed(&format!("{two}  - cognito -- apigateway\n  - apigateway ~> cloudwatch\n  - lambda.a ~> cloudwatch\n  - lambda.b ~> cloudwatch\n"));
        assert_eq!(q.col_x[..2], p.col_x[..2]);
        for id in ["apigateway", "lambda.a", "lambda.b"] {
            assert_eq!(in_region(&p, id), in_region(&q, id), "{id}");
        }
    }

    #[test]
    fn a_side_part_gathering_lines_moves_no_part() {
        let base = "flows:
  - user -> apigateway(http) -> lambda.f1
  - apigateway -> lambda.f2
  - apigateway -> lambda.f3
";
        let one = format!("{base}  - lambda.f1 ~> cloudwatch\n");
        let four = format!("{base}  - lambda.f1 ~> cloudwatch\n  - lambda.f2 ~> cloudwatch\n  - lambda.f3 ~> cloudwatch\n  - apigateway ~> cloudwatch\n");
        let (_, a) = placed(base);
        let (_, b) = placed(&one);
        let (_, c) = placed(&four);
        let main = ["user", "apigateway", "lambda.f1", "lambda.f2", "lambda.f3"];
        for id in main {
            assert_eq!(at(&a, id).y, at(&b, id).y, "{id}");
            assert_eq!(at(&a, id).y, at(&c, id).y, "{id}");
        }
        let bottom = main.map(|id| at(&a, id).y).into_iter().max().unwrap();
        assert_eq!(at(&b, "cloudwatch").y, bottom + b.row_step);
        // However many lines a side part gathers, it stays one row below the main rows.
        assert_eq!(at(&c, "cloudwatch").y, bottom + c.row_step);
    }

    #[test]
    fn side_lines_from_one_service_into_one_part_are_drawn_as_one_from_the_nearest() {
        let src = "flows:
  - user -> apigateway(http) -> lambda.f1
  - apigateway -> lambda.f2
  - apigateway -> lambda.f3
  - lambda.f1 ~> cloudwatch
  - lambda.f2 ~> cloudwatch
  - lambda.f3 ~> cloudwatch
  - apigateway ~> cloudwatch
";
        for (lang, sources, list) in [("", "(3 sources)", "lambda.f1, lambda.f2, lambda.f3"), ("set:\n  lang: ja\n", "（3か所から）", "lambda.f1、lambda.f2、lambda.f3")] {
            let (m, p, wires, notes) = drawn(&format!("{src}{lang}"));
            let into: Vec<&Wire> = wires.iter().filter(|w| w.to == "cloudwatch").collect();
            assert_eq!(into.len(), 2, "{into:?}");
            let lambdas = into.iter().find(|w| w.from.starts_with("lambda")).unwrap();
            assert_eq!(lambdas.id, "edge-lambda.f3-to-cloudwatch");
            assert!(lambdas.label.ends_with(sources), "{:?}", lambdas.label);
            let stands: Vec<&str> = lambdas.represents.iter().map(|(f, _)| f.as_str()).collect();
            assert_eq!(stands, ["lambda.f1", "lambda.f2", "lambda.f3"]);
            assert!(into.iter().any(|w| w.from == "apigateway" && w.represents.len() == 1));
            let said: Vec<&String> = notes.iter().filter(|n| n.contains(list)).collect();
            assert_eq!(said.len(), 1, "{notes:?}");
            assert!(!said[0].contains("edge-"), "{said:?}");
            assert_lines_clear(&m, &p, &wires, &notes);
        }
    }

    fn drawn(src: &str) -> (Model, Placement, Vec<Wire>, Vec<String>) {
        let (m, p) = placed(src);
        let (w, notes) = route_planned(&m, &p.layout, &Rules::builtin(), &p.lines);
        (m, p, w, notes)
    }
    /// No note names a line that could not be drawn clear, and every line as finally drawn keeps
    /// clear of every icon but its own two ends and of every name.
    fn assert_lines_clear(m: &Model, p: &Placement, wires: &[Wire], notes: &[String]) {
        assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?}");
        let l = &p.layout;
        for w in wires {
            let mut path = vec![port_point(l.rect(&w.from), w.exit)];
            path.extend(w.points.iter().copied());
            path.push(port_point(l.rect(&w.to), w.entry));
            for s in path.windows(2) {
                let (x0, x1, y0, y1) = (s[0].0.min(s[1].0), s[0].0.max(s[1].0), s[0].1.min(s[1].1), s[0].1.max(s[1].1));
                let enters = |r: Rect| x1 > r.x && x0 < r.x + r.w && y1 > r.y && y0 < r.y + r.h;
                for i in &l.icons {
                    let own = i.id == w.from || i.id == w.to;
                    assert!(own || !enters(i.rect), "{} runs through {}", w.id, i.id);
                    assert!(!enters(name_area(i)), "{} runs through the name of {}", w.id, i.id);
                }
            }
        }
        assert!(check(m, p).is_empty());
    }
    fn assert_clean(src: &str) {
        let (m, p, w, notes) = drawn(src);
        assert_names_apart(&p);
        assert_lines_clear(&m, &p, &w, &notes);
    }

    #[test]
    fn lines_whose_first_try_runs_through_parts_end_clear() {
        // CloudFront's line to S3 is first tried straight, through API Gateway and Lambda on its row.
        assert_clean("flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(http) -> lambda -> s3\n");
        // lambda.a's line to CloudWatch is first tried straight down, through lambda.b and lambda.c.
        assert_clean("flows:\n  - user -> apigateway(http) -> lambda.a\n  - apigateway -> lambda.b\n  - apigateway -> lambda.c\n  - lambda.a ~> cloudwatch\n");
    }

    #[test]
    fn two_apis_and_four_functions_lay_out_cleanly() {
        assert_clean("flows:
  - user -> apigateway.a(http) -> lambda.a1 -> dynamodb.a
  - apigateway.a -> lambda.a2 -> dynamodb.a
  - user -> apigateway.b(rest) -> lambda.b1 -> dynamodb.b
  - apigateway.b -> lambda.b2 -> dynamodb.b
  - lambda.a1 ~> cloudwatch
  - lambda.a2 ~> cloudwatch
  - lambda.b1 ~> cloudwatch
  - lambda.b2 ~> cloudwatch
");
    }

    const TWO_LOGS: &str = "flows:
  - user -> cloudfront -> apigateway.a(rest) -> lambda.a
  - user -> apigateway.b(http) -> lambda.b
  - lambda.a ~> cloudwatch.x
  - lambda.b ~> cloudwatch.x
  - apigateway.a ~> cloudwatch.y
  - lambda.a ~> cloudwatch.y
  - cognito -- apigateway.a
  - cognito -- apigateway.b
";

    #[test]
    fn two_log_groups_with_partly_shared_senders_and_a_distribution_in_front_of_an_api() {
        assert_clean(TWO_LOGS);
        assert_clean(&TWO_LOGS.replace("  - user -> cloudfront", "  - user.a -> cloudfront").replace("  - user -> apigateway.b", "  - user.b -> apigateway.b"));
    }

    #[test]
    fn the_order_of_side_lines_moves_no_main_part() {
        let main = "flows:\n  - user -> cloudfront -> apigateway.a(rest) -> lambda.a\n  - user -> apigateway.b(http) -> lambda.b\n";
        let sides = ["lambda.a ~> cloudwatch.x", "lambda.b ~> cloudwatch.x", "apigateway.a ~> cloudwatch.y", "lambda.a ~> cloudwatch.y", "cognito -- apigateway.a", "cognito -- apigateway.b"];
        let write = |order: &[&str]| format!("{main}{}", order.iter().map(|l| format!("  - {l}\n")).collect::<String>());
        let mut reversed = sides;
        reversed.reverse();
        let (_, a) = placed(&write(&sides));
        let (_, b) = placed(&write(&reversed));
        for id in ["user", "cloudfront", "apigateway.a", "apigateway.b", "lambda.a", "lambda.b"] {
            assert_eq!(at(&a, id), at(&b, id), "{id}");
        }
        assert_clean(&write(&sides));
        assert_clean(&write(&reversed));
    }

    #[test]
    fn global_parts_and_side_parts_with_the_longest_widest_names_lay_out_cleanly() {
        let t = |k: &str| format!("{k}{}", "m".repeat(crate::def::MAX_TAG - k.len()));
        assert_clean(&format!(
            "flows:
  - user -> cloudfront.{a} -> s3
  - user -> cloudfront.{b} -> apigateway(http) -> lambda
  - waf.{w} -- cloudfront.{a}
  - acm.{k} -- cloudfront.{a}
  - waf.{v} -- cloudfront.{b}
  - cognito.{g} -- apigateway
  - lambda ~> cloudwatch.{x}
  - apigateway ~> cloudwatch.{x}
",
            a = t("a"), b = t("b"), w = t("w"), k = t("k"), v = t("v"), g = t("g"), x = t("x"),
        ));
    }

    #[test]
    fn side_parts_move_neither_main_rows_nor_main_places_in_the_region() {
        let base = "flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n";
        let (_, a) = placed(base);
        let (_, b) = placed(&format!("{base}  - cognito -- apigateway\n  - apigateway ~> cloudwatch\n  - lambda ~> cloudwatch\n"));
        for id in ["user", "apigateway", "lambda", "dynamodb"] {
            assert_eq!(at(&a, id).y, at(&b, id).y, "{id}");
        }
        for id in ["apigateway", "lambda", "dynamodb"] {
            assert_eq!(in_region(&a, id), in_region(&b, id), "{id}");
        }
        let bottom = ["apigateway", "lambda", "dynamodb"].map(|id| at(&b, id).y).into_iter().max().unwrap();
        let side = bottom + b.row_step;
        assert_eq!((cx(&b, "cognito"), at(&b, "cognito").y), (cx(&b, "apigateway"), side));
        assert_eq!((cx(&b, "cloudwatch"), at(&b, "cloudwatch").y), (b.col_x[1], side));
    }

    #[test]
    fn attachments_sit_below_their_distribution_and_move_no_main_row() {
        let base = "flows:
  - user -> cloudfront -> s3
  - user -> apigateway(rest) -> lambda -> dynamodb
  - cognito -- apigateway
";
        let (_, a) = placed(&format!("{base}  - waf -- cloudfront\n  - acm -- cloudfront\n"));
        let (_, b) = placed(base);
        let cf = at(&a, "cloudfront");
        assert_eq!((at(&a, "waf").y, at(&a, "acm").y), (cf.y + a.row_step, cf.y + a.row_step));
        assert_eq!(at(&a, "waf").x, cf.x);
        assert!(at(&a, "acm").x > cf.x);
        assert!(right(label_of(&a, "acm")) < at(&a, "region").x);
        for id in ["user", "cloudfront", "s3", "apigateway", "lambda", "dynamodb"] {
            assert_eq!(at(&a, id).y, at(&b, id).y, "{id}");
        }
        for id in ["s3", "apigateway", "lambda", "dynamodb"] {
            assert_eq!(in_region(&a, id), in_region(&b, id), "{id}");
        }
    }

    #[test]
    fn a_shared_web_acl_sits_at_the_bottom_of_the_global_column() {
        let (_, p) = placed("flows:
  - user -> cloudfront.a -> s3.a
  - user -> cloudfront.b -> s3.b
  - waf -- cloudfront.a
  - waf -- cloudfront.b
");
        let r = p.row_step;
        assert_eq!(at(&p, "cloudfront.b").y, at(&p, "cloudfront.a").y + 2 * r);
        assert_eq!(at(&p, "waf").y, at(&p, "cloudfront.b").y + 2 * r);
        assert!(right(at(&p, "waf")) < at(&p, "region").x);
    }

    #[test]
    fn region_contents_do_not_depend_on_the_global_column() {
        let base = "flows:\n  - user -> apigateway(http) -> lambda\n";
        let (_, a) = placed(base);
        let (_, b) = placed(&format!("{base}  - user -> cloudfront -> apigateway\n"));
        for id in ["apigateway", "lambda"] {
            assert_eq!(in_region(&a, id), in_region(&b, id), "{id}");
        }
    }

    #[test]
    fn a_region_frame_is_drawn_only_with_a_part_in_it() {
        for (src, region) in [
            ("flows:\n  - user -> cloudfront\n", false),
            ("flows:\n  - user -> cloudfront\n  - waf -- cloudfront\n", false),
            ("flows:\n  - user -> apigateway(http)\n", true),
            ("flows:\n  - user -> cloudfront -> s3\n", true),
        ] {
            let (m, p) = placed(src);
            assert_eq!(p.layout.frames.iter().any(|f| f.id == "region"), region, "{src}");
            assert!(check(&m, &p).is_empty(), "{src}");
        }
        // Without a Region frame, the AWS Cloud frame holds the global column and its own name.
        let (_, p) = placed("flows:\n  - user -> cloudfront\n");
        let cloud = p.layout.frames.iter().find(|f| f.id == "aws-cloud").unwrap();
        let cf = p.layout.icons.iter().find(|i| i.id == "cloudfront").unwrap();
        assert!(right(name_area(cf)) + FRAME_ROOM <= right(cloud.rect));
        assert!(right(frame_name_box(cloud)) + FRAME_ROOM <= right(cloud.rect));
    }

    #[test]
    fn line_plans_leave_right_and_side_lines_leave_from_the_upper_part() {
        let (_, p) = placed(TARGET);
        assert_eq!(p.lines.len(), 7);
        let gw = plan(&p, "apigateway", "lambda");
        assert_eq!((gw.exit, gw.entry, gw.bend.clone()), (Side::Right, Side::Left, Bend::Straight));
        let uc = plan(&p, "user", "cloudfront");
        assert_eq!((uc.exit, uc.entry), (Side::Right, Side::Left));
        match &uc.bend {
            &Bend::AtX(x) => assert!(right(at(&p, "user")) < x && x < at(&p, "aws-cloud").x, "{x}"),
            other => panic!("{other:?}"),
        }
        let cw = plan(&p, "lambda", "cloudwatch");
        assert_eq!((cw.exit, cw.entry), (Side::Bottom, Side::Top));
        let co = plan(&p, "cognito", "apigateway");
        assert_eq!((co.exit, co.entry), (Side::Top, Side::Bottom));
    }

    #[test]
    fn a_finished_layout_passes_its_own_check_and_comes_out_the_same_twice() {
        let (m, p) = placed(TARGET);
        assert!(check(&m, &p).is_empty());
        assert_eq!(placed(TARGET).1, p);
    }

    /// Every definition the generator makes.
    pub(in crate::arrange) fn generated() -> Vec<String> {
        let mut out = Vec::new();
        let apis = ["apigateway(http)", "apigateway(rest)", "apigateway.public(http)"];
        // (CloudFront shape, attachments): 0 none; 1 web; 2 a and b; 3 one CloudFront into a Lambda's S3.
        let mut fronts: Vec<(usize, usize)> = vec![(0, 0)];
        for cf in 1..=3 {
            for att in 0..=3 {
                if att == 3 && cf != 2 {
                    continue;
                }
                fronts.push((cf, att));
            }
        }
        let mut i = 0usize;
        for lambdas in 1..=3usize {
            for dest in 0..3 {
                for aux in [false, true] {
                    for api in apis {
                        for cognito in [false, true] {
                            for &(cf, att) in &fronts {
                                out.push(definition(i, lambdas, dest, aux, api, cognito, cf, att));
                                i += 1;
                            }
                        }
                    }
                }
            }
        }
        out.extend(through_cloudfront());
        out
    }

    /// Definitions where the API is reached through CloudFront (`user -> cloudfront ->
    /// apigateway -> lambda -> s3`), with names as written or every name made of `m` alone at the
    /// longest allowed.
    fn through_cloudfront() -> Vec<String> {
        let mut out = Vec::new();
        for wide in [false, true] {
            let n = |svc: &str| if wide { format!("{svc}.{}", "m".repeat(crate::def::MAX_TAG)) } else { svc.to_string() };
            for api in ["http", "rest"] {
                for att in 0..3 {
                    for cognito in [false, true] {
                        let mut src = format!("flows:\n  - user -> {} -> {}({api}) -> {} -> {}\n", n("cloudfront"), n("apigateway"), n("lambda"), n("s3"));
                        if att > 0 {
                            src.push_str(&format!("  - {} -- {}\n", n("waf"), n("cloudfront")));
                        }
                        if att > 1 {
                            src.push_str(&format!("  - {} -- {}\n", n("acm"), n("cloudfront")));
                        }
                        if cognito {
                            src.push_str(&format!("  - {} -- {}\n", n("cognito"), n("apigateway")));
                        }
                        src.push_str(&format!("  - {} ~> {}\n", n("lambda"), n("cloudwatch")));
                        if wide {
                            src.push_str("set:\n  lang: ja\n");
                        }
                        out.push(src);
                    }
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn definition(i: usize, lambdas: usize, dest: usize, aux: bool, api: &str, cognito: bool, cf: usize, att: usize) -> String {
        // Names: as written, or every name after the dot at the longest allowed, filled with `x`
        // or with `m`, the widest letter a name may hold. Side parts and global parts are named too
        // then. Taken from the number apart from the settings, which go by `i % 3`.
        let fill = match (i / 3) % 3 {
            0 => None,
            1 => Some("x"),
            _ => Some("m"),
        };
        let tag = |t: &str| match fill {
            Some(c) => format!("{t}{}", c.repeat(crate::def::MAX_TAG - t.len())),
            None => t.to_string(),
        };
        // A part named only when names are long: `cloudwatch`, or `cloudwatch.l…` filled.
        let side = |svc: &str, t: &str| if fill.is_some() { format!("{svc}.{}", tag(t)) } else { svc.to_string() };
        let lambda = |k: usize| if lambdas == 1 { "lambda".to_string() } else { format!("lambda.{}", tag(&format!("f{k}"))) };
        let table = |t: &str| format!("dynamodb.{}", tag(t));
        let api_name = api.split('(').next().unwrap();
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("user -> {api} -> {}", lambda(1)));
        for k in 2..=lambdas {
            lines.push(format!("{api_name} -> {}", lambda(k)));
        }
        match dest {
            0 => (1..=lambdas).for_each(|k| lines.push(format!("{} -> {}", lambda(k), table(&format!("t{k}"))))),
            1 => (1..=lambdas).for_each(|k| {
                lines.push(format!("{} -> {}", lambda(k), table("x")));
                lines.push(format!("{} -> {}", lambda(k), table("y")));
            }),
            _ => (1..=lambdas).rev().for_each(|k| {
                lines.push(format!("{} -> {}", lambda(k), table("x")));
                lines.push(format!("{} -> {}", lambda(k), table("y")));
            }),
        }
        let cloudwatch = side("cloudwatch", "l");
        if aux {
            (1..=lambdas).for_each(|k| lines.push(format!("{} ~> {cloudwatch}", lambda(k))));
            lines.push(format!("{api_name} ~> {cloudwatch}"));
        }
        if cognito {
            lines.push(format!("{} -- {api_name}", side("cognito", "g")));
        }
        // Each distribution's own letter: "" for the only one, "a" and "b" for two.
        let fronts: Vec<&str> = if cf == 2 { vec!["a", "b"] } else if cf > 0 { vec![""] } else { vec![] };
        // `svc` for the only one with short names, else `svc.<letter>` with the letter filled out.
        let named = |svc: &str, f: &str, alone: &str| match (f.is_empty(), fill.is_some()) {
            (true, false) => svc.to_string(),
            (true, true) => format!("{svc}.{}", tag(alone)),
            (false, _) => format!("{svc}.{}", tag(f)),
        };
        let front = |f: &str| named("cloudfront", f, "c");
        match cf {
            1 => lines.push(format!("user -> {} -> s3.web", front(""))),
            2 => {
                lines.push(format!("user -> {} -> s3.a", front("a")));
                lines.push(format!("user -> {} -> s3.b", front("b")));
            }
            3 => {
                lines.push(format!("user -> {} -> s3.web", front("")));
                lines.push(format!("{} -> s3.web", lambda(1)));
            }
            _ => {}
        }
        match att {
            1 | 2 => {
                for f in &fronts {
                    lines.push(format!("{} -- {}", named("waf", f, "w"), front(f)));
                    if att == 2 {
                        lines.push(format!("{} -- {}", named("acm", f, "k"), front(f)));
                    }
                }
            }
            3 => fronts.iter().for_each(|f| lines.push(format!("{} -- {}", side("waf", "w"), front(f)))),
            _ => {}
        }
        let mut src = String::from("flows:\n");
        for l in lines {
            src.push_str(&format!("  - {l}\n"));
        }
        match i % 3 {
            1 => src.push_str("set:\n  lang: ja\n"),
            2 => src.push_str("set:\n  lang: ja\n  region: ap-northeast-1\n"),
            _ => {}
        }
        src
    }

    #[test]
    fn generated_definitions_lay_out_cleanly() {
        let r = Rules::builtin();
        let defs = generated();
        assert!(defs.len() > 1000, "{}", defs.len());
        let mut failed: Vec<String> = Vec::new();
        for src in &defs {
            let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
            let Some(m) = m.filter(|_| d.errors.is_empty() && d.holes.is_empty()) else {
                failed.push(format!("inference: {:?} {:?}\n{src}", d.errors, d.holes));
                continue;
            };
            let p = match arrange(&m, &r, Axis::Right, CHOICES) {
                Ok(p) => p,
                Err(e) => {
                    failed.push(format!("place: {e:?}\n{src}"));
                    continue;
                }
            };
            // A note that names a line by its id says the line or its label could not be drawn
            // clear. The note on lines drawn as one names parts, not a line id, and does not count.
            let (wires, notes) = crate::route::route_planned(&m, &p.layout, &r, &p.lines);
            if notes.iter().any(|n| n.contains("edge-")) {
                failed.push(format!("route: {notes:?}\n{src}"));
            }
            // A line keeps at least 3 pixels from every other line's label.
            if let Some((d, line, label)) = crate::audit::closest_line_to_label(&p.layout, &wires).filter(|c| c.0 < 3) {
                failed.push(format!("label: {line} runs {d}px from the label of {label}\n{src}"));
            }
        }
        assert!(failed.is_empty(), "{} of {} definitions failed:\n{}", failed.len(), defs.len(), failed.join("\n"));
    }

    /// How many diagrams `tools/check_generated.sh` expects.
    const CHECKED: usize = 40;

    /// The definitions handed to the diagram checker, by their number in `generated()`: two
    /// taken from the ones through CloudFront (one with short names, the last with every name
    /// made of `m` alone), and the rest taken evenly from the others.
    fn generated_for_the_checker() -> Vec<(usize, String)> {
        let defs = generated();
        let extra = through_cloudfront().len();
        let base = defs.len() - extra;
        let mut picked: Vec<usize> = (0..CHECKED - 2).map(|k| k * base / (CHECKED - 2)).collect();
        // The short-named HTTP API with WAF, ACM and Cognito, and the last one.
        picked.extend([base + extra / 4 - 1, defs.len() - 1]);
        picked.into_iter().map(|i| (i, defs[i].clone())).collect()
    }

    /// With `ARCHWIRE_GEN_OUT=<dir>`, removes the numbered files a run left there, then writes
    /// each picked definition as `<dir>/<number>.yaml` and what `compile` makes of it as
    /// `<dir>/<number>.drawio` and `<dir>/<number>.json`, for `tools/check_generated.sh`.
    /// Without it, only compiles them. Every one must give a diagram.
    #[test]
    fn generated_diagrams_compile_for_the_checker() {
        let picked = generated_for_the_checker();
        assert_eq!(picked.len(), CHECKED);
        let all_m = "m".repeat(crate::def::MAX_TAG);
        let has = |what: &str, f: &dyn Fn(&str) -> bool| assert!(picked.iter().any(|(_, s)| f(s)), "no {what} in the pick");
        has("three Lambda functions sending to one CloudWatch", &|s| s.matches(" ~> cloudwatch").count() >= 4);
        has("name filled with `m` after a letter", &|s| s.contains(&"m".repeat(crate::def::MAX_TAG - 2)) && !s.contains(&all_m));
        has("name made of `m` alone", &|s| s.contains(&format!(".{all_m}")));
        has("API reached through CloudFront", &|s| s.contains("user -> cloudfront -> apigateway(http) -> lambda -> s3\n"));
        // A relative folder is taken from the workspace root, where the command is typed, not
        // from this crate's folder, where the test runs.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::var_os("ARCHWIRE_GEN_OUT").map(|d| root.join(d));
        if let Some(d) = &dir {
            std::fs::create_dir_all(d).unwrap();
            for entry in std::fs::read_dir(d).unwrap() {
                let path = entry.unwrap().path();
                let numbered = path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
                let ours = path.extension().and_then(|e| e.to_str()).is_some_and(|e| ["yaml", "drawio", "json"].contains(&e));
                if numbered && ours {
                    std::fs::remove_file(&path).unwrap();
                }
            }
        }
        let mut failed: Vec<String> = Vec::new();
        for (i, src) in &picked {
            let out = crate::compile(src);
            let d = &out.diagnostics;
            if out.drawio.is_none() || d.exit_code() != 0 || d.notes.iter().any(|n| n.contains("edge-")) {
                failed.push(format!("{i}: diagram {} {:?} {:?}\n{src}", out.drawio.is_some(), d.errors, d.notes));
            }
            if let Some(dir) = &dir {
                let name = format!("{i:04}");
                std::fs::write(dir.join(format!("{name}.yaml")), src).unwrap();
                std::fs::write(dir.join(format!("{name}.json")), d.to_json()).unwrap();
                if let Some(x) = &out.drawio {
                    std::fs::write(dir.join(format!("{name}.drawio")), x).unwrap();
                }
            }
        }
        assert!(failed.is_empty(), "{} of {} definitions failed:\n{}", failed.len(), picked.len(), failed.join("\n"));
    }

    /// The definitions of the fixed examples of the API shape that give a diagram.
    pub(in crate::arrange) const API_CASES: [(&str, &str); 7] = [
        ("api-http-ja", include_str!("../../../../tests/cases/api-http-ja/def.yaml")),
        ("api-merged-ja", include_str!("../../../../tests/cases/api-merged-ja/def.yaml")),
        ("api-min-en", include_str!("../../../../tests/cases/api-min-en/def.yaml")),
        ("api-names-ja", include_str!("../../../../tests/cases/api-names-ja/def.yaml")),
        ("api-rest-en", include_str!("../../../../tests/cases/api-rest-en/def.yaml")),
        ("edge-waf-en", include_str!("../../../../tests/cases/edge-waf-en/def.yaml")),
        ("hole-apigateway", include_str!("../../../../tests/cases/hole-apigateway/def.yaml")),
    ];

    /// Every model the down layout is tried on: the fixed API examples and every generated
    /// definition, with the definition each came from.
    fn down_inputs() -> Vec<(String, Model)> {
        let r = Rules::builtin();
        let defs = API_CASES.iter().map(|(_, s)| s.to_string()).chain(generated());
        defs.map(|src| {
            let (m, d) = crate::flow_infer::infer(&crate::def::parse(&src).unwrap(), &r);
            let m = m.unwrap_or_else(|| panic!("{:?}\n{src}", d.errors));
            (src, m)
        })
        .collect()
    }

    /// The down layout of every input that lays out, and the definitions that do not.
    fn down_layouts() -> (Vec<(String, Model, Placement)>, Vec<String>) {
        let r = Rules::builtin();
        let (mut ok, mut failed) = (Vec::new(), Vec::new());
        for (src, m) in down_inputs() {
            match arrange(&m, &r, Axis::Down, CHOICES) {
                Ok(p) => ok.push((src, m, p)),
                Err(e) => failed.push(format!("{e:?}\n{src}")),
            }
        }
        (ok, failed)
    }

    // Fails if turning the axis makes parts or names overlap, leave their frames or lie over a frame's name.
    #[test]
    fn down_layouts_pass_their_check() {
        let (ok, failed) = down_layouts();
        assert!(ok.len() > 1000, "{}", ok.len());
        assert!(failed.is_empty(), "{} definitions failed:\n{}", failed.len(), failed.join("\n"));
    }

    // Fails if the main axis is left running right when lines run down.
    #[test]
    fn down_runs_from_top_to_bottom() {
        for (src, m, p) in down_layouts().0 {
            let l = &p.layout;
            for e in m.edges.iter().filter(|e| e.rel == Rel::Main) {
                let (a, b) = (l.rect(&e.from), l.rect(&e.to));
                assert!(a.y < b.y, "{} -> {}: {a:?} {b:?}\n{src}", e.from, e.to);
            }
            let cloud = l.rect(CLOUD_FRAME);
            for n in m.nodes.iter().filter(|n| n.area == Area::Outside) {
                assert!(bottom(l.rect(&n.id)) < cloud.y, "{}: {:?} {cloud:?}\n{src}", n.id, l.rect(&n.id));
            }
            for pl in &p.lines {
                assert_eq!(pl.axis, Axis::Down, "{pl:?}");
            }
        }
    }

    // Fails if side parts stay below the main parts when lines run down.
    #[test]
    fn down_puts_side_parts_to_the_right() {
        for (src, m, p) in down_layouts().0 {
            let icons = &p.layout.icons;
            let icon = |id: &str| icons.iter().find(|i| i.id == id).unwrap();
            for (k, n) in m.nodes.iter().enumerate().filter(|(_, n)| !on_main(&m, &n.id)) {
                let side = icon(&n.id);
                let against: Vec<&Icon> = if n.area == Area::Cloud {
                    partners(&m, k).into_iter().map(|j| icon(&m.nodes[j].id)).collect()
                } else {
                    icons.iter().filter(|i| i.parent == side.parent && on_main(&m, &i.id)).collect()
                };
                assert!(!against.is_empty(), "{}\n{src}", n.id);
                for o in against {
                    assert!(side.rect.x > right(o.rect), "{} at {:?} is not right of {} at {:?}\n{src}", n.id, side.rect, o.id, o.rect);
                }
            }
        }
    }

    // Fails if names are turned with the axis, beside or above their icons instead of below them.
    #[test]
    fn names_do_not_turn() {
        for (src, _, p) in down_layouts().0 {
            for i in &p.layout.icons {
                assert!(name_area(i).y >= bottom(i.rect), "{}: {:?} {:?}\n{src}", i.id, name_area(i), i.rect);
            }
        }
        let r = Rules::builtin();
        for (name, src) in API_CASES {
            let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
            let (x, _) = crate::arranged_drawio(&m.unwrap(), &d, &r, Axis::Down, CHOICES).unwrap();
            let shapes: Vec<&str> = x.lines().filter(|l| l.contains("mxgraph.aws4.") && l.contains("vertex=\"1\"") && !l.contains("mxgraph.aws4.group")).collect();
            assert!(!shapes.is_empty(), "{name}");
            for s in shapes {
                assert!(s.contains("verticalLabelPosition=bottom;verticalAlign=top"), "{name}: {s}");
            }
        }
    }
}
