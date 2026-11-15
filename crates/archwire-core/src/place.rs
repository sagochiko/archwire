//! Layout for definitions without a VPC part, from left to right.
//!
//! Three steps: the column a part takes, the order of the parts in a column, and the distances
//! that give every part its place. The Region's contents are laid out first, in the Region
//! frame's own coordinates and from the Region's parts and the lines between them alone; the
//! global column and the users are then set against them. Which frame a part belongs to is
//! `Node.area`, decided by inference; a part with no main line (`->`) is a side part. Positions
//! are integers, and ties go to the order the definition first wrote the parts and lines in.
//!
//! The must-hold conditions are checked on the finished layout; a layout that fails them is
//! not given out.

use crate::def::Rel;
use crate::diag::Problem;
use crate::layout::{icon, Frame, FrameKind, Icon, Layout, Rect};
use crate::measure::{frame_name_box, name_area, name_text_width, wire_text_width};
use crate::model::{Area, Model};
use crate::route::{Bend, LinePlan, Side};
use crate::rules::Rules;

/// The least distance between the centres of two neighbouring columns.
pub(crate) const COL_STEP: i32 = 220;
/// The least distance between the tops of two rows.
pub(crate) const ROW_STEP: i32 = 160;
/// The least distance between the centres of two side parts set side by side in the global column.
pub(crate) const SIDE_STEP: i32 = 160;

const ICON: i32 = 60;
/// A row holds an icon, the tallest name and this much room below it.
const ROW_ROOM: i32 = 20;
/// Two names side by side stay at least this far apart.
const NAME_ROOM: i32 = 20;
/// Room for a line's label between two columns, on top of an icon's width and the label.
const WIRE_ROOM: i32 = 40;
/// More room between two columns for each further main line that bends between them.
const LANE_ROOM: i32 = 60;
/// From the Region frame's left side to the first column's icons, and from its top to the first row.
const REGION_LEFT: i32 = 110;
const REGION_TOP: i32 = 60;
/// From the rightmost and the lowest icon or name to the Region frame.
const REGION_RIGHT: i32 = 50;
const REGION_BOTTOM: i32 = 40;
/// A frame's name starts this far from the frame's left side, as the router reads it.
const FRAME_NAME_INDENT: i32 = 30;
/// A name stays at least this far inside its frame's left side; a frame's name this far from its right side.
const FRAME_ROOM: i32 = 20;
/// From the AWS Cloud frame's left side to the global column's icons.
const CLOUD_LEFT: i32 = 80;
/// From the right end of the global column's icons and names to the Region frame.
const GLOBAL_TO_REGION: i32 = 80;
/// The Region inside the AWS Cloud frame on the right, at the bottom, and on the left when there is no global column.
const CLOUD_INSET: i32 = 30;
const CLOUD_Y: i32 = 60;
/// From the AWS Cloud frame's top to the Region frame's top.
const REGION_BELOW_CLOUD: i32 = 50;
/// From the diagram's left to the users' icons, and from the right end of the users to the AWS Cloud frame.
const USER_LEFT: i32 = 40;
const USER_TO_CLOUD: i32 = 60;

const CLOUD_FRAME: &str = "aws-cloud";
const REGION_FRAME: &str = "region";
const CLOUD_NAME: &str = "AWS Cloud";

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub layout: Layout,
    pub lines: Vec<LinePlan>,
    /// The distance between rows this diagram uses.
    pub row_step: i32,
    /// The centre x of each column in the Region, from the first.
    pub col_x: Vec<i32>,
}

/// Which way a frame's contents run. Only the Region is laid out through it, only from left to
/// right, and nothing reads it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    LeftToRight,
}

/// One frame's contents laid out in the frame's own coordinates.
struct Block {
    w: i32,
    h: i32,
    icons: Vec<Icon>,
    /// The centre x of each column: the main columns, then the side row's own past the last one.
    cols: Vec<i32>,
}

/// The Region's parts on a main flow and the main lines between them.
struct Flow {
    /// Indexes into `Model.nodes`, in model order.
    parts: Vec<usize>,
    /// The column of each part, from 1.
    col: Vec<usize>,
    /// Indexes into `parts` of the Region parts each part's main lines come from and go to, in model order.
    preds: Vec<Vec<usize>>,
    succs: Vec<Vec<usize>>,
}

/// The layout, or LAYOUT_FAILED problems when a must-hold condition fails after laying out.
pub fn place(model: &Model, rules: &Rules) -> Result<Placement, Vec<Problem>> {
    let r = row_step(model, rules);
    let block = lay_region(model, rules, Direction::LeftToRight);
    let nodes = &model.nodes;
    let index = |id: &str| nodes.iter().position(|n| n.id == id);
    let area = |i: usize| nodes[i].area;
    let targets = |i: usize| -> Vec<usize> {
        model.edges.iter().filter(|e| e.rel == Rel::Main && e.from == nodes[i].id).filter_map(|e| index(&e.to)).collect()
    };

    // One scale for every y: 0 is the Region's first row.
    let mut y: Vec<Option<i32>> = vec![None; nodes.len()];
    for i in &block.icons {
        y[index(&i.id).expect("the Region block holds model parts")] = Some(i.rect.y - REGION_TOP);
    }

    // The global column, after the Region. Its main parts go level with the middle of their
    // targets in the Region, two rows apart, so the row below each stays free for its side parts.
    let global: Vec<usize> = (0..nodes.len()).filter(|&i| area(i) == Area::Cloud).collect();
    let (g_main, g_side): (Vec<usize>, Vec<usize>) = global.iter().partition(|&&i| on_main(model, &nodes[i].id));
    let mut above: Option<i32> = None;
    for &i in &g_main {
        let want = mid(targets(i).into_iter().filter(|&t| area(t) == Area::Region).filter_map(|t| y[t]));
        let v = below(above, want, 2 * r);
        y[i] = Some(v);
        above = Some(v);
    }
    // Side parts with one partner sit in the row below it; ones with more, below everything else
    // in the column. Each row runs from the column's x to the right, in model order.
    let mut side_rows: Vec<(i32, Vec<usize>)> = Vec::new();
    let put = |rows: &mut Vec<(i32, Vec<usize>)>, at: i32, i: usize| match rows.iter_mut().find(|(ry, _)| *ry == at) {
        Some(row) => row.1.push(i),
        None => rows.push((at, vec![i])),
    };
    let mut shared: Vec<usize> = Vec::new();
    for &i in &g_side {
        match partners(model, i).as_slice() {
            [one] if y[*one].is_some() => put(&mut side_rows, y[*one].expect("checked") + r, i),
            _ => shared.push(i),
        }
    }
    if !shared.is_empty() {
        let lowest = g_main.iter().filter_map(|&i| y[i]).map(|v| v + r).chain(side_rows.iter().map(|row| row.0)).max();
        let at = lowest.map_or(0, |l| l + r);
        for i in shared {
            put(&mut side_rows, at, i);
        }
    }
    for (at, parts) in &side_rows {
        for &i in parts {
            y[i] = Some(*at);
        }
    }

    // The users, after the global column: level with the middle of their main targets, a row apart.
    let users: Vec<usize> = (0..nodes.len()).filter(|&i| area(i) == Area::Outside).collect();
    let mut above: Option<i32> = None;
    for &i in &users {
        let v = below(above, mid(targets(i).into_iter().filter_map(|t| y[t])), r);
        y[i] = Some(v);
        above = Some(v);
    }

    // Left to right: the users, the AWS Cloud frame, the global column, the Region frame.
    let named = |i: usize| icon_of(model, rules, i, icon_at(0, 0), None);
    let users_w = users.iter().map(|&i| width(&named(i))).max().unwrap_or(ICON);
    let user_x = USER_LEFT + half_up(users_w);
    let cloud_x = if users.is_empty() { USER_LEFT } else { user_x + half_up(users_w) + USER_TO_CLOUD };
    let first_w = g_main.iter().copied().chain(side_rows.iter().map(|row| row.1[0])).map(|i| width(&named(i))).max().unwrap_or(ICON);
    let global_x = cloud_x + (CLOUD_LEFT + ICON / 2).max(half_up(first_w) + FRAME_ROOM);
    let mut x: Vec<Option<i32>> = vec![None; nodes.len()];
    for &i in &g_main {
        x[i] = Some(global_x);
    }
    for (_, parts) in &side_rows {
        let mut at = global_x;
        for (k, &i) in parts.iter().enumerate() {
            if k > 0 {
                at += step(SIDE_STEP, width(&named(parts[k - 1])), width(&named(i)));
            }
            x[i] = Some(at);
        }
    }
    for &i in &users {
        x[i] = Some(user_x);
    }

    let top = CLOUD_Y + REGION_BELOW_CLOUD + REGION_TOP;
    let at = |i: usize| match (x[i], y[i]) {
        (Some(cx), Some(cy)) => Some(icon_at(cx - ICON / 2, top + cy)),
        _ => None,
    };
    let global_icons: Vec<Icon> =
        global.iter().filter_map(|&i| at(i).map(|r| icon_of(model, rules, i, r, Some(CLOUD_FRAME.into())))).collect();
    let region_x = match global_icons.iter().map(|i| extent(i).0).max() {
        Some(end) => end + GLOBAL_TO_REGION,
        None => cloud_x + CLOUD_INSET,
    };
    let region_y = CLOUD_Y + REGION_BELOW_CLOUD;
    let region = Rect { x: region_x, y: region_y, w: block.w, h: block.h };
    // The Region frame is drawn only when a part is in it; else the AWS Cloud frame closes
    // around the global column and its own name.
    let has_region = nodes.iter().any(|n| n.area == Area::Region);
    let (inner_right, inner_bottom) = if has_region {
        (right(region), bottom(region))
    } else {
        let cloud_name = Frame { id: CLOUD_FRAME.into(), kind: FrameKind::Cloud, label: CLOUD_NAME.into(), rect: icon_at(cloud_x, CLOUD_Y), parent: None };
        let name_right = right(frame_name_box(&cloud_name)) + FRAME_ROOM - CLOUD_INSET;
        let icons_right = global_icons.iter().map(|i| right(i.rect) + CLOUD_LEFT - CLOUD_INSET).chain(global_icons.iter().map(|i| extent(i).0 + FRAME_ROOM - CLOUD_INSET));
        (icons_right.fold(name_right, i32::max), region_y)
    };
    let cloud_bottom = global_icons.iter().map(|i| extent(i).1 + REGION_BOTTOM).fold(inner_bottom, i32::max) + CLOUD_INSET;
    let cloud = Rect { x: cloud_x, y: CLOUD_Y, w: inner_right + CLOUD_INSET - cloud_x, h: cloud_bottom - CLOUD_Y };

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
            // Inside a VPC, which this layout does not draw: left unplaced, and the check names it.
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

    let col_x: Vec<i32> = block.cols.iter().map(|c| region.x + c).collect();
    let lines = plans(model, &layout, &col_x);

    let problems = check(model, &layout);
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Placement { layout, lines, row_step: r, col_x })
}

/// The must-hold conditions of the spec, checked on a finished layout; empty when all hold.
pub(crate) fn check(model: &Model, layout: &Layout) -> Vec<Problem> {
    let mut out = Vec::new();
    let ids: Vec<&str> = layout.frames.iter().map(|f| f.id.as_str()).chain(layout.icons.iter().map(|i| i.id.as_str())).collect();
    for (k, id) in ids.iter().enumerate() {
        if ids[..k].iter().filter(|x| *x == id).count() == 1 {
            out.push(failed(id, id.to_string(), "duplicate ids"));
        }
    }
    for n in &model.nodes {
        if !layout.icons.iter().any(|i| i.id == n.id) {
            out.push(failed(&n.id, n.id.clone(), "a place"));
        }
    }
    let icons = &layout.icons;
    let labels: Vec<Rect> = icons.iter().map(name_area).collect();
    for a in 0..icons.len() {
        for b in a + 1..icons.len() {
            let (ia, ib, la, lb) = (icons[a].rect, icons[b].rect, labels[a], labels[b]);
            if overlaps(ia, ib) || overlaps(la, lb) || overlaps(la, ib) || overlaps(ia, lb) {
                out.push(failed(&icons[a].id, format!("{} and {}", icons[a].id, icons[b].id), "overlapping"));
            }
        }
    }
    let frame = |id: &str| layout.frames.iter().find(|f| f.id == id).map(|f| f.rect);
    for (i, label) in icons.iter().zip(&labels) {
        let Some(parent) = &i.parent else { continue };
        if !frame(parent).is_some_and(|f| inside(f, i.rect) && inside(f, *label)) {
            out.push(failed(&i.id, i.id.clone(), "leaving its frame"));
        }
    }
    for f in &layout.frames {
        let Some(parent) = &f.parent else { continue };
        if !frame(parent).is_some_and(|p| inside(p, f.rect)) {
            out.push(failed(&f.id, f.id.clone(), "leaving its frame"));
        }
    }
    // The frame a part sits in is the one the model gives it.
    for n in &model.nodes {
        let want = match n.area {
            Area::Outside => None,
            Area::Cloud => Some(CLOUD_FRAME),
            Area::Region => Some(REGION_FRAME),
            // Parts inside a VPC never reach this layout.
            Area::VpcEdge | Area::Subnet(_) => continue,
        };
        if let Some(i) = icons.iter().find(|i| i.id == n.id) {
            if i.parent.as_deref() != want {
                out.push(failed(&i.id, i.id.clone(), "leaving its frame"));
            }
        }
    }
    // Parts in one column, in one frame, stay at least a row apart.
    for a in 0..icons.len() {
        for b in a + 1..icons.len() {
            let (ia, ib) = (&icons[a], &icons[b]);
            if ia.rect.x == ib.rect.x && ia.parent == ib.parent && (ia.rect.y - ib.rect.y).abs() < ROW_STEP {
                out.push(failed(&ia.id, format!("{} and {}", ia.id, ib.id), "keeping rows apart"));
            }
        }
    }
    // No part or name lies over a frame's name.
    for f in &layout.frames {
        let name = frame_name_box(f);
        for (i, label) in icons.iter().zip(&labels) {
            if overlaps(i.rect, name) || overlaps(*label, name) {
                out.push(failed(&i.id, format!("{} and {}", i.id, f.id), "overlapping"));
            }
        }
    }
    out
}

fn failed(at: &str, got: String, what: &str) -> Problem {
    let message = format!("archwire could not lay out {got} without {what}; this is a bug in archwire, please report the definition");
    Problem { code: "LAYOUT_FAILED".into(), at: Some(at.into()), got: Some(got), allowed: Vec::new(), rule: None, message }
}

/// The Region's contents, from the Region's parts and the lines between them alone.
fn lay_region(model: &Model, rules: &Rules, dir: Direction) -> Block {
    match dir {
        Direction::LeftToRight => {}
    }
    let r = row_step(model, rules);
    let f = flow(model);
    let ys = rows(&f, r, true);

    // Main columns: their widths come from the main parts' names alone, so side parts never move them.
    let named = |i: usize| icon_of(model, rules, i, icon_at(0, 0), None);
    let cols = f.col.iter().copied().max().unwrap_or(0);
    let mut w = vec![ICON; cols + 1];
    for (k, &i) in f.parts.iter().enumerate() {
        w[f.col[k]] = w[f.col[k]].max(width(&named(i)));
    }
    let local = |id: &str| f.parts.iter().position(|&i| model.nodes[i].id == id);
    let mut x = vec![0; cols + 1];
    if cols > 0 {
        x[1] = (REGION_LEFT + ICON / 2).max(half_up(w[1]) + FRAME_ROOM);
    }
    for c in 1..cols {
        // The main lines from this column into the next, and how many of them bend on the way.
        let lines: Vec<(usize, usize, &str)> = model
            .edges
            .iter()
            .filter(|e| e.rel == Rel::Main)
            .filter_map(|e| Some((local(&e.from)?, local(&e.to)?, e.label.as_str())))
            .filter(|&(a, b, _)| f.col[a] == c && f.col[b] == c + 1)
            .collect();
        let wire = lines.iter().map(|l| wire_text_width(&rules.text(l.2, model.lang))).max().unwrap_or(0);
        let bending = lines.iter().filter(|&&(a, b, _)| ys[a] != ys[b]).count() as i32;
        x[c + 1] = x[c] + col_gap(w[c], w[c + 1], wire, bending);
    }
    // (model index, centre x, y) of every Region part.
    let mut spots: Vec<(usize, i32, i32)> = f.parts.iter().enumerate().map(|(k, &i)| (i, x[f.col[k]], ys[k])).collect();

    // Side parts take one row below the main rows, under the column of the part their first side
    // line joins, or the next column to the right where their names keep clear of the side parts
    // placed before them, or past the last column.
    let sides: Vec<usize> =
        (0..model.nodes.len()).filter(|&i| model.nodes[i].area == Area::Region && !f.parts.contains(&i)).collect();
    let side_y = ys.iter().max().map_or(0, |m| m + r);
    let mut placed: Vec<(i32, i32)> = Vec::new();
    let mut extra: Vec<i32> = Vec::new();
    for &i in &sides {
        let wi = width(&named(i));
        let partner = partners(model, i).into_iter().next();
        let want = partner.and_then(|p| f.parts.iter().position(|&j| j == p)).map_or(1, |k| f.col[k]);
        let clear = |cx: i32| cx - half_up(wi) >= FRAME_ROOM && placed.iter().all(|&(px, pw)| (cx - px).abs() >= step(COL_STEP, pw, wi));
        let cx = match (want..=cols).map(|c| x[c]).find(|&cx| clear(cx)) {
            Some(cx) => cx,
            None => {
                let base = if cols > 0 { x[cols] + COL_STEP } else { REGION_LEFT + ICON / 2 };
                placed.iter().map(|&(px, pw)| px + step(COL_STEP, pw, wi)).chain([base, half_up(wi) + FRAME_ROOM]).max().expect("not empty")
            }
        };
        if cols == 0 || cx > x[cols] {
            extra.push(cx);
        }
        placed.push((cx, wi));
        spots.push((i, cx, side_y));
    }
    spots.sort_by_key(|s| s.0);

    let icons: Vec<Icon> = spots
        .iter()
        .map(|&(i, cx, y)| icon_of(model, rules, i, icon_at(cx - ICON / 2, REGION_TOP + y), Some(REGION_FRAME.into())))
        .collect();
    let (mut bw, mut bh) = (REGION_LEFT + ICON + REGION_RIGHT, REGION_TOP + ICON + REGION_BOTTOM);
    if !icons.is_empty() {
        bw = icons.iter().map(|i| extent(i).0).max().expect("not empty") + REGION_RIGHT;
        bh = icons.iter().map(|i| extent(i).1).max().expect("not empty") + REGION_BOTTOM;
    }
    // Wide enough for the frame's own name.
    bw = bw.max(FRAME_NAME_INDENT + name_text_width(&region_name(model, rules)) + FRAME_ROOM);
    let mut centres: Vec<i32> = x[1..].iter().copied().chain(extra).collect();
    centres.sort_unstable();
    centres.dedup();
    Block { w: bw, h: bh, icons, cols: centres }
}

/// R: an icon, the tallest name in the diagram and some room, and at least `ROW_STEP`.
fn row_step(model: &Model, rules: &Rules) -> i32 {
    let tallest = (0..model.nodes.len()).map(|i| name_area(&icon_of(model, rules, i, icon_at(0, 0), None)).h).max().unwrap_or(0);
    ROW_STEP.max(ICON + tallest + ROW_ROOM)
}

/// The Region's parts on a main flow, each in the column one past the furthest of its
/// predecessors in the Region. Lines from outside the Region do not count.
fn flow(model: &Model) -> Flow {
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

/// The y of every part of `f`, rows apart in each column, with the smallest at 0.
fn rows(f: &Flow, r: i32, sweep_right: bool) -> Vec<i32> {
    let last = f.col.iter().copied().max().unwrap_or(0);
    let in_col = |c: usize| -> Vec<usize> { (0..f.parts.len()).filter(|&i| f.col[i] == c).collect() };
    let mut y = vec![0; f.parts.len()];
    // From the left: the first column from the top down, every other part level with the middle
    // of its predecessors, and at least a row below the part above it.
    for c in 1..=last {
        let mut above: Option<i32> = None;
        for i in in_col(c) {
            let v = below(above, mid(f.preds[i].iter().map(|&p| y[p])), r);
            y[i] = v;
            above = Some(v);
        }
    }
    // From the right: a part with two or more targets moves toward their middle, as far as the
    // part above (already moved) and the part below (not yet moved) leave a row.
    if sweep_right {
        for c in (1..last).rev() {
            let col = in_col(c);
            for (k, &i) in col.iter().enumerate() {
                if f.succs[i].len() < 2 {
                    continue;
                }
                let mut v = mid(f.succs[i].iter().map(|&s| y[s])).expect("two or more targets");
                if k > 0 {
                    v = v.max(y[col[k - 1]] + r);
                }
                if k + 1 < col.len() {
                    v = v.min(y[col[k + 1]] - r);
                }
                y[i] = v;
            }
        }
    }
    if let Some(top) = y.iter().copied().min() {
        y.iter_mut().for_each(|v| *v -= top);
    }
    y
}

/// The line plans, in model order. Main lines leave on the right and enter on the left, bending
/// halfway to whatever is next on the right; side lines join the upper part's bottom to the
/// lower part's top, bending halfway between the upper part's name and the lower part.
fn plans(model: &Model, layout: &Layout, col_x: &[i32]) -> Vec<LinePlan> {
    let find = |id: &str| layout.icons.iter().find(|i| i.id == id);
    let mut out = Vec::new();
    for e in &model.edges {
        let (Some(a), Some(b)) = (find(&e.from), find(&e.to)) else { continue };
        let (exit, entry, bend) = if e.rel == Rel::Main {
            let bend = if a.rect.y == b.rect.y {
                Bend::Straight
            } else {
                let from = right(a.rect);
                let next = layout
                    .frames
                    .iter()
                    .map(|f| f.rect.x)
                    .chain(col_x.iter().map(|c| c - ICON / 2))
                    .filter(|&x| x > from)
                    .min()
                    .unwrap_or(from + 2 * NAME_ROOM);
                Bend::AtX((from + next).div_euclid(2))
            };
            (Side::Right, Side::Left, bend)
        } else {
            let from_upper = a.rect.y <= b.rect.y;
            let (upper, lower) = if from_upper { (a, b) } else { (b, a) };
            let bend = if upper.rect.x == lower.rect.x {
                Bend::Straight
            } else {
                Bend::AtY((bottom(name_area(upper)) + lower.rect.y).div_euclid(2))
            };
            if from_upper {
                (Side::Bottom, Side::Top, bend)
            } else {
                (Side::Top, Side::Bottom, bend)
            }
        };
        out.push(LinePlan { from: e.from.clone(), to: e.to.clone(), exit, entry, bend });
    }
    out
}

/// The distance between the centres of two neighbouring Region columns of widths `a` and `b`,
/// with the widest label of the main lines from one into the other and how many of them bend.
fn col_gap(a: i32, b: i32, wire: i32, bending: i32) -> i32 {
    step(COL_STEP, a, b).max(ICON + wire + WIRE_ROOM) + (bending - 1).max(0) * LANE_ROOM
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

/// The y of the next part down a column: `want` if there is room, else a `step` below the part above.
fn below(above: Option<i32>, want: Option<i32>, step: i32) -> i32 {
    match above {
        None => want.unwrap_or(0),
        Some(a) => want.map_or(a + step, |w| w.max(a + step)),
    }
}

/// Halfway between the smallest and the largest, rounded down.
fn mid(values: impl IntoIterator<Item = i32>) -> Option<i32> {
    let v: Vec<i32> = values.into_iter().collect();
    Some((v.iter().min()? + v.iter().max()?).div_euclid(2))
}

/// Half, rounded up, of a width that is not negative.
fn half_up(v: i32) -> i32 {
    (v + 1).div_euclid(2)
}

/// The width a part takes: its icon or its name, whichever is wider.
fn width(i: &Icon) -> i32 {
    ICON.max(name_area(i).w)
}

/// The distance between the centres of two neighbours of these widths: their names apart, and at least `least`.
fn step(least: i32, a: i32, b: i32) -> i32 {
    least.max(half_up(a + b) + NAME_ROOM)
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

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn inside(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && right(inner) <= right(outer) && bottom(inner) <= bottom(outer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;
    use crate::layout::Icon;
    use crate::measure::{name_area, NAME_CHARS, NAME_DRAWN, NAME_WIDE_CHARS, NAME_WIDE_DRAWN};
    use crate::route::{port_point, route_planned, Bend, Side, Wire};

    fn placed(src: &str) -> (Model, Placement) {
        let r = Rules::builtin();
        let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        let p = place(&m, &r).unwrap_or_else(|e| panic!("{e:?}"));
        (m, p)
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

    const TARGET: &str = "flows:
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
        let rs = row_step(&m, &r);
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
        assert!(check(&m, &p.layout).is_empty());
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
        assert!(check(&m, &p.layout).is_empty());
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
            assert!(check(&m, &p.layout).is_empty(), "{c}");
            assert_names_apart(&p);
        }
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
        assert!(check(m, l).is_empty());
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
            assert!(check(&m, &p.layout).is_empty(), "{src}");
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
        assert_eq!((gw.exit, gw.entry, gw.bend), (Side::Right, Side::Left, Bend::Straight));
        let uc = plan(&p, "user", "cloudfront");
        assert_eq!((uc.exit, uc.entry), (Side::Right, Side::Left));
        match uc.bend {
            Bend::AtX(x) => assert!(right(at(&p, "user")) < x && x < at(&p, "aws-cloud").x, "{x}"),
            other => panic!("{other:?}"),
        }
        let cw = plan(&p, "lambda", "cloudwatch");
        assert_eq!((cw.exit, cw.entry), (Side::Bottom, Side::Top));
        let co = plan(&p, "cognito", "apigateway");
        assert_eq!((co.exit, co.entry), (Side::Top, Side::Bottom));
    }

    /// The target layout, with `change` applied to it by hand.
    fn broken(change: impl Fn(&mut Layout)) -> Vec<Problem> {
        let (m, p) = placed(TARGET);
        let mut l = p.layout.clone();
        change(&mut l);
        check(&m, &l)
    }
    fn icon_mut<'a>(l: &'a mut Layout, id: &str) -> &'a mut crate::layout::Icon {
        l.icons.iter_mut().find(|i| i.id == id).unwrap()
    }

    #[test]
    fn check_finds_overlapping_icons() {
        let got = broken(|l| {
            let to = l.rect("apigateway");
            icon_mut(l, "lambda").rect = to;
        });
        assert!(got.iter().all(|e| e.code == "LAYOUT_FAILED"), "{got:?}");
        assert!(got.iter().any(|e| e.got.as_deref() == Some("apigateway and lambda")), "{got:?}");
    }

    #[test]
    fn check_finds_a_name_over_another() {
        let got = broken(|l| {
            let gw = l.rect("apigateway");
            icon_mut(l, "lambda").rect.x = gw.x + 70;
            icon_mut(l, "lambda").rect.y = gw.y;
        });
        assert!(!got.is_empty() && got.iter().all(|e| e.code == "LAYOUT_FAILED"), "{got:?}");
        assert!(got.iter().any(|e| e.message.contains("without overlapping")), "{got:?}");
    }

    #[test]
    fn check_finds_a_part_outside_its_frame() {
        let got = broken(|l| {
            let f = l.rect("region");
            icon_mut(l, "dynamodb").rect.x = f.x + f.w + 10;
        });
        assert!(got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.message.contains("leaving its frame") && e.at.as_deref() == Some("dynamodb")), "{got:?}");
    }

    #[test]
    fn check_finds_a_missing_part() {
        let got = broken(|l| l.icons.retain(|i| i.id != "cloudwatch"));
        assert_eq!(got.len(), 1, "{got:?}");
        let e = &got[0];
        assert_eq!((e.code.as_str(), e.at.as_deref(), e.got.as_deref()), ("LAYOUT_FAILED", Some("cloudwatch"), Some("cloudwatch")));
        assert_eq!(e.message, "archwire could not lay out cloudwatch without a place; this is a bug in archwire, please report the definition");
    }

    #[test]
    fn check_finds_parts_too_close_in_a_column() {
        let (m, p) = placed("flows:\n  - user -> apigateway(http) -> lambda.a\n  - apigateway -> lambda.b\n");
        let mut l = p.layout.clone();
        let a = l.rect("lambda.a");
        icon_mut(&mut l, "lambda.b").rect.y = a.y + 110;
        let got = check(&m, &l);
        assert!(got.iter().any(|e| e.got.as_deref() == Some("lambda.a and lambda.b") && e.message.contains("without keeping rows apart")), "{got:?}");
    }

    #[test]
    fn check_finds_a_part_over_a_frame_name() {
        let (m, p) = placed("flows:\n  - user -> apigateway(http)\n");
        let mut l = p.layout.clone();
        let f = l.rect("region");
        icon_mut(&mut l, "apigateway").rect = Rect { x: f.x + 5, y: f.y + 2, w: 60, h: 60 };
        let got = check(&m, &l);
        assert!(got.iter().any(|e| e.got.as_deref() == Some("apigateway and region") && e.message.contains("without overlapping")), "{got:?}");
    }

    #[test]
    fn check_finds_a_part_in_a_frame_the_model_does_not_give_it() {
        let got = broken(|l| icon_mut(l, "dynamodb").parent = Some("aws-cloud".into()));
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!((got[0].at.as_deref(), got[0].message.contains("leaving its frame")), (Some("dynamodb"), true));
    }

    #[test]
    fn check_finds_duplicate_ids() {
        let got = broken(|l| {
            let mut copy = l.icons[0].clone();
            copy.rect.y += 1000;
            copy.id = "region".into();
            l.icons.push(copy);
        });
        assert!(got.iter().any(|e| e.message.contains("without duplicate ids") && e.got.as_deref() == Some("region")), "{got:?}");
    }

    #[test]
    fn a_finished_layout_passes_its_own_check_and_comes_out_the_same_twice() {
        let (m, p) = placed(TARGET);
        assert!(check(&m, &p.layout).is_empty());
        assert_eq!(placed(TARGET).1, p);
    }

    /// Every definition the generator makes.
    fn generated() -> Vec<String> {
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
            let p = match place(&m, &r) {
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
}
