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

use super::azgrid::Placed;
use super::{Crossing, Lane, Placement};
use crate::def::Rel;
use crate::layout::{icon, Frame, FrameKind, Icon, Layout, Rect};
use crate::measure::{frame_name_box, name_area, name_text_width, wire_label_area, wire_text_size, wire_text_width};
use crate::model::{Area, Axis, Model};
use crate::path::opposite;
use crate::route::{Bend, Keep, LinePlan, Side, Track};
use crate::rules::Rules;

/// The least distance between the centres of two neighbouring columns.
pub(crate) const COL_STEP: i32 = 220;
/// The least distance between the tops of two rows.
pub(crate) const ROW_STEP: i32 = 160;
/// The least distance between the centres of two side parts set side by side in the global rank, along the main axis.
pub(crate) const SIDE_STEP: i32 = 160;
/// The least room between the VPC's right side and the parts only the VPC's parts send to, for the
/// labels of the lines into them.
pub(super) const OUT_GAP: i32 = 140;

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
/// From the AWS Cloud frame's left side to the global rank's icons.
const CLOUD_LEFT: i32 = 80;
/// From the end of the global rank's icons and names to the Region frame, along the main axis
/// across the page; down the page, the gap holds the labels of the lines across it.
const GLOBAL_TO_REGION: i32 = 80;
/// Top to bottom, the room above the global rank for each line run over a row of attached parts.
const OVER_ROOM: i32 = 20;
/// The Region inside the AWS Cloud frame on the right, at the bottom, and on the left when it comes first along the main axis.
const CLOUD_INSET: i32 = 30;
/// From the diagram's top to the AWS Cloud frame.
const CLOUD_Y: i32 = 60;
/// From the AWS Cloud frame's top to the Region frame's top.
const REGION_BELOW_CLOUD: i32 = 50;
/// From the diagram's edge to the users' icons and names, or from its left to the AWS Cloud frame
/// when it comes first; from the end of the users to the AWS Cloud frame along the main axis
/// across the page (down the page, the gap holds the labels of the lines across it).
const USER_LEFT: i32 = 40;
const USER_TO_CLOUD: i32 = 60;

pub(crate) const CLOUD_FRAME: &str = "aws-cloud";
pub(crate) const REGION_FRAME: &str = "region";
const CLOUD_NAME: &str = "AWS Cloud";

/// From and to, along one axis, relative to an icon's left or top side.
type Span = (i32, i32);
/// A part's index in the flow and the model node a line ends at in it (itself, or a part inside the VPC).
type Unit = (usize, usize);
/// The other part of a main line, and how far across this part lies from it when the line's ends are level.
type Link = (usize, i32);

/// One frame's contents laid out in the frame's own coordinates.
struct Block {
    w: i32,
    h: i32,
    icons: Vec<Icon>,
    /// The centre of each main rank along the main axis: the main ranks, then the side parts' own past the last one.
    ranks: Vec<i32>,
    /// How far into the frame across the main axis the parts' cross coordinate 0 lies.
    across: i32,
    /// Where the VPC's top-left corner lies in the frame.
    vpc: Option<(i32, i32)>,
}

/// The inside of a VPC laid out as one piece. It comes first among the parts of its rank.
pub(super) struct Piece<'a> {
    pub(super) placed: &'a Placed,
}

/// The Region's parts on a main flow and the main lines between them.
pub(super) struct Flow {
    /// Indexes into `Model.nodes`, rank by rank, each rank in the order its parts are placed across.
    pub(super) parts: Vec<usize>,
    /// The rank of each part, from 1.
    pub(super) col: Vec<usize>,
    /// Indexes into `parts` of the Region parts each part's main lines come from and go to, in
    /// model order, with how far across the part's own coordinate lies from the other's when the
    /// two ends of the line are level: 0 between two parts, the end's place in the VPC for the VPC.
    preds: Vec<Vec<Link>>,
    succs: Vec<Vec<Link>>,
    /// The VPC: its place in `parts`, its size across, and the room after it across.
    block: Option<(usize, i32, i32)>,
}

impl Flow {
    /// The least distance across from the start of part `a` to the start of part `b` after it.
    fn step(&self, a: usize, r: i32) -> i32 {
        match self.block {
            Some((k, size, room)) if k == a => size + room,
            _ => r,
        }
    }
}

/// `(x, y)` read along the main and the cross axis, `(u, v)`. The same swap turns `(u, v)` back into `(x, y)`.
pub(super) fn uv<T>(axis: Axis, (x, y): (T, T)) -> (T, T) {
    match axis {
        Axis::Right => (x, y),
        Axis::Down => (y, x),
    }
}

/// The layout of a model with main lines running along `axis`, with the inside of its VPC as one
/// piece if it has one.
pub(super) fn layered(model: &Model, rules: &Rules, axis: Axis, piece: Option<&Piece>) -> Placement {
    let r = cross_step(model, rules, axis);
    let block = lay_region(model, rules, axis, piece);
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
        // A part sending into the VPC's entry gateway lines up with it, before the middle of its targets.
        let ts: Vec<usize> = targets(i).into_iter().filter(|&t| in_region(area(t))).collect();
        let entry: Vec<usize> = ts.iter().copied().filter(|&t| area(t) == Area::VpcEdge).collect();
        let want = mid((if entry.is_empty() { ts } else { entry }).into_iter().filter_map(|t| v[t]));
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
    // The users come first from the diagram's edge the same distance whichever side they are on.
    let user_u = USER_LEFT - users_span.0;
    // Down the page, a gap holds the labels of the lines across it, as between two ranks; across
    // the page, it keeps its least width.
    let across_gap = |least: i32, from: &[usize]| {
        let label = model
            .edges
            .iter()
            .filter(|e| e.rel == Rel::Main && from.iter().any(|&i| nodes[i].id == e.from))
            .map(|e| wire_text_size(&rules.text(&e.label, model.lang)).1)
            .max()
            .unwrap_or(0);
        uv(axis, (least, label + WIRE_ROOM)).0
    };
    let cloud_u = if users.is_empty() { diagram_main } else { user_u + users_span.1 + across_gap(USER_TO_CLOUD, &users) };
    let first_from = g_main.iter().copied().chain(side_rows.iter().map(|row| row.1[0])).map(|i| main(i).0).min().unwrap_or(0);
    // Down the page, the global rank starts past the AWS Cloud frame's name, with room.
    let cloud_name = frame_name_box(&Frame { id: CLOUD_FRAME.into(), kind: FrameKind::Cloud, label: CLOUD_NAME.into(), rect: icon_at(0, 0), parent: None }).h;
    let global_u = cloud_u + uv(axis, (CLOUD_LEFT, cloud_name + FRAME_ROOM)).0.max(FRAME_ROOM - first_from);
    let mut u: Vec<Option<i32>> = vec![None; nodes.len()];
    for &i in &g_main {
        u[i] = Some(global_u);
    }
    for (_, parts) in &side_rows {
        let mut at = global_u;
        for (k, &i) in parts.iter().enumerate() {
            if k > 0 {
                at += gap_of(SIDE_STEP, main(parts[k - 1]), main(i));
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
        Some(end) => end + across_gap(GLOBAL_TO_REGION, &g_main),
        None => cloud_u + region_main,
    };
    let (region_x, region_y) = uv(axis, (region_u, diagram_cross + region_cross));
    let region = Rect { x: region_x, y: region_y, w: block.w, h: block.h };
    let (cloud_x, cloud_y) = uv(axis, (cloud_u, diagram_cross));
    // The Region frame is drawn only when a part is in it; else the AWS Cloud frame closes
    // around the global rank and its own name.
    let has_region = nodes.iter().any(|n| n.area == Area::Region) || piece.is_some();
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
            Area::Region | Area::VpcEdge | Area::Subnet(_) => block.icons.iter().find(|b| b.id == n.id).map(|b| {
                let mut b = b.clone();
                b.rect.x += region.x;
                b.rect.y += region.y;
                b
            }),
            Area::Cloud => global_icons.iter().find(|g| g.id == n.id).cloned(),
            Area::Outside => at(i).map(|r| icon_of(model, rules, i, r, None)),
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
    // The VPC's frames, lines, gates and bands, moved from its own coordinates into the diagram's.
    let origin = block.vpc.map(|(x, y)| (region.x + x, region.y + y));
    if let (Some(p), Some(o)) = (piece, origin) {
        frames.extend(p.placed.frames.iter().map(|f| Frame { rect: Rect { x: f.rect.x + o.0, y: f.rect.y + o.1, ..f.rect }, ..f.clone() }));
    }
    let layout = Layout { frames, icons };

    let col_x: Vec<i32> = block.ranks.iter().map(|c| region_u + c).collect();
    let mut lines = plans(model, &layout, &col_x, axis);
    let (mut crossings, mut lanes) = (Vec::new(), Vec::new());
    if let (Some(p), Some(o)) = (piece, origin) {
        let at = |q: (i32, i32)| (q.0 + o.0, q.1 + o.1);
        for pl in &p.placed.inside {
            let mut pl = pl.clone();
            if let Bend::Points(ps) = &mut pl.bend {
                ps.iter_mut().for_each(|q| *q = at(*q));
            }
            for t in &mut pl.tracks {
                (t.from, t.to, t.at) = if t.horizontal { (t.from + o.0, t.to + o.0, t.at + o.1) } else { (t.from + o.1, t.to + o.1, t.at + o.0) };
            }
            lines.push(pl);
        }
        lanes = p
            .placed
            .lanes
            .iter()
            .map(|l| {
                let (along, across) = if l.horizontal { o } else { (o.1, o.0) };
                Lane { from: l.from + along, to: l.to + along, lo: l.lo + across, hi: l.hi + across, tracks: l.tracks.iter().map(|&(e, t)| (e, t + across)).collect(), ..l.clone() }
            })
            .collect();
        let vpc = layout.rect(super::azgrid::VPC_FRAME);
        // Lines run as side lines out of the part they attach to turn down toward their parts in
        // the gap right of the VPC, the one from the lowest gate nearest the VPC, so none crosses another.
        let beside = stagger(model, &layout, p, at);
        for c in &p.placed.crossing {
            let mut c = c.clone();
            c.through.gate = at(c.through.gate);
            c.through.inner.iter_mut().for_each(|q| *q = at(*q));
            let mut plan = crossing_plan(model, &layout, &c, vpc, &lanes, axis);
            if let (Some(&(_, step)), Bend::AtX(_)) = (beside.iter().find(|b| b.0 == c.edge), &plan.bend) {
                plan.bend = Bend::AtX(c.through.gate.0 + step);
            }
            lines.push(plan);
            crossings.push(c);
        }
        lines.sort_by_key(|l| l.edge);
    }
    Placement { layout, lines, row_step: r, col_x, crossings, lanes }
}

/// The draft of a line through a gate in the VPC's side: its inner part as the VPC laid it out,
/// and its outer part out of the side of the part outside that faces the gate, bending halfway.
fn crossing_plan(model: &Model, layout: &Layout, c: &Crossing, vpc: Rect, lanes: &[Lane], axis: Axis) -> LinePlan {
    let e = &model.edges[c.edge];
    let t = &c.through;
    let gate_side = if t.gate.0 == vpc.x {
        Side::Left
    } else if t.gate.0 == vpc.x + vpc.w {
        Side::Right
    } else if t.gate.1 == vpc.y {
        Side::Top
    } else {
        Side::Bottom
    };
    let outer_id = if t.inner_is_start { &e.to } else { &e.from };
    let outer = layout.icons.iter().find(|i| &i.id == outer_id).expect("the outer end is placed");
    let mut side = opposite(gate_side);
    let (cx, cy) = (outer.rect.x + ICON / 2, outer.rect.y + ICON / 2);
    // A line out of the VPC's right side to a part with another part of its column between them
    // passes right of that column and comes into the part's far side, so it does not cut the
    // line into the part between.
    let outer_index = model.nodes.iter().position(|n| &n.id == outer_id).expect("edge ends are nodes");
    let (lo, hi) = (t.gate.1.min(cy), t.gate.1.max(cy));
    let column: Vec<&Icon> = layout.icons.iter().filter(|i| i.rect.x == outer.rect.x && i.id != outer.id).collect();
    let between = column.iter().any(|i| i.rect.y < hi && extent(i).1 > lo);
    if t.inner_is_start && gate_side == Side::Right && only_inside(model, outer_index) && between {
        let past = column.iter().chain([&outer]).map(|i| extent(i).0).max().expect("not empty") + NAME_ROOM;
        side = Side::Right;
        return LinePlan {
            edge: c.edge,
            from: e.from.clone(),
            to: e.to.clone(),
            exit: c.inner_side,
            entry: side,
            bend: Bend::AtX(past),
            exit_pos: Some(c.inner_pos),
            entry_pos: None,
            keep: Keep::Inner(t.clone()),
            tracks: inner_tracks(c, lanes),
            axis,
        };
    }
    let bend = match side {
        Side::Top | Side::Bottom if cx == t.gate.0 => Bend::Straight,
        Side::Left | Side::Right if cy == t.gate.1 => Bend::Straight,
        Side::Bottom => Bend::AtY((extent(outer).1 + t.gate.1).div_euclid(2)),
        Side::Top => Bend::AtY((outer.rect.y + t.gate.1).div_euclid(2)),
        Side::Right => Bend::AtX((right(outer.rect) + t.gate.0).div_euclid(2)),
        Side::Left => Bend::AtX((outer.rect.x + t.gate.0).div_euclid(2)),
    };
    let tracks = inner_tracks(c, lanes);
    // A straight outer part asks for the middle of the outer side, so it stays straight.
    let outer_pos = (bend == Bend::Straight).then_some(50);
    let (exit, entry, exit_pos, entry_pos) =
        if t.inner_is_start { (c.inner_side, side, Some(c.inner_pos), outer_pos) } else { (side, c.inner_side, outer_pos, Some(c.inner_pos)) };
    LinePlan { edge: c.edge, from: e.from.clone(), to: e.to.clone(), exit, entry, bend, exit_pos, entry_pos, keep: Keep::Inner(t.clone()), tracks, axis }
}

/// The stretch of a crossing line's inner part along the track the VPC gave it, if any.
fn inner_tracks(c: &Crossing, lanes: &[Lane]) -> Vec<Track> {
    let t = &c.through;
    let mut pts = t.inner.clone();
    if t.inner_is_start {
        pts.push(t.gate);
    } else {
        pts.insert(0, t.gate);
    }
    let mut tracks = Vec::new();
    for l in lanes.iter().filter(|l| l.horizontal) {
        for &(_, at) in l.tracks.iter().filter(|tr| tr.0 == c.edge) {
            if let Some(s) = pts.windows(2).find(|s| s[0].1 == at && s[1].1 == at && s[0].0 != s[1].0) {
                tracks.push(Track { horizontal: true, from: s[0].0.min(s[1].0), to: s[0].0.max(s[1].0), at });
            }
        }
    }
    tracks
}

/// The Region frame holds the parts inside the VPC too.
fn in_region(a: Area) -> bool {
    matches!(a, Area::Region | Area::VpcEdge | Area::Subnet(_))
}

fn inside(model: &Model, i: usize) -> bool {
    matches!(model.nodes[i].area, Area::VpcEdge | Area::Subnet(_))
}

/// The Region's parts that a line joins to a part outside the Region, as a line from the users or
/// the global rank does: such a line comes in across the Region frame's side, at the part.
fn entered_parts(model: &Model) -> Vec<usize> {
    let index = |id: &str| model.nodes.iter().position(|n| n.id == id);
    let mut out = Vec::new();
    for e in &model.edges {
        if let (Some(a), Some(b)) = (index(&e.from), index(&e.to)) {
            match (in_region(model.nodes[a].area), in_region(model.nodes[b].area)) {
                (true, false) => out.push(a),
                (false, true) => out.push(b),
                _ => {}
            }
        }
    }
    out
}

/// A Region part whose every line comes from or goes to a part inside the VPC.
pub(super) fn only_inside(model: &Model, i: usize) -> bool {
    let id = &model.nodes[i].id;
    let others: Vec<usize> = model
        .edges
        .iter()
        .filter_map(|e| match (&e.from == id, &e.to == id) {
            (true, _) => model.nodes.iter().position(|n| n.id == e.to),
            (_, true) => model.nodes.iter().position(|n| n.id == e.from),
            _ => None,
        })
        .collect();
    !others.is_empty() && others.iter().all(|&j| inside(model, j))
}

/// The across coordinates, in the VPC's own, of the gates of the lines of part `i` attached to
/// a part inside the VPC.
fn attached_gates(model: &Model, p: &Piece, i: usize) -> Vec<i32> {
    p.placed.crossing.iter().filter(|c| model.edges[c.edge].rel == Rel::Assoc && model.edges[c.edge].from == model.nodes[i].id).map(|c| c.through.gate.1).collect()
}

/// Where the attached parts `attached` (in the order of their gates) go in the column at `x` right
/// of the VPC, clear of `placed`: from the lowest gate up, each level with its gate or as near above
/// it as is free and above the one below it; or, if that would rise past the VPC's top `top`, from
/// the highest gate down the same way.
fn attached_places(model: &Model, rules: &Rules, attached: &[usize], placed: &[Icon], x: i32, top: i32, gate_y: &dyn Fn(usize) -> Option<i32>) -> Vec<i32> {
    let try_way = |up: bool| -> Option<Vec<i32>> {
        let mut icons: Vec<Icon> = placed.to_vec();
        let mut ys = vec![0; attached.len()];
        let mut last: Option<i32> = None;
        let ks: Vec<usize> = if up { (0..attached.len()).rev().collect() } else { (0..attached.len()).collect() };
        for k in ks {
            let i = attached[k];
            let g = gate_y(i).expect("attached") - ICON / 2;
            let want = match (last, up) {
                (Some(l), true) => g.min(l - ROW_STEP),
                (Some(l), false) => g.max(l + ROW_STEP),
                (None, _) => g,
            };
            let free = |y: i32| {
                let ic = icon_of(model, rules, i, icon_at(x, y), Some(REGION_FRAME.into()));
                !icons.iter().any(|o| rows_too_close(o, &ic) || boxes_meet(o, &ic))
            };
            let step = if up { -ROW_STEP } else { ROW_STEP };
            let mut tries: Vec<i32> = icons.iter().filter(|o| o.rect.x == x).map(|o| o.rect.y + step).chain([want]).filter(|&y| if up { y <= want } else { y >= want }).collect();
            tries.sort_by_key(|&y| (y - want).abs());
            let y = tries.into_iter().find(|&y| free(y))?;
            if up && y < top {
                return None;
            }
            ys[k] = y;
            last = Some(y);
            icons.push(icon_of(model, rules, i, icon_at(x, y), Some(REGION_FRAME.into())));
        }
        Some(ys)
    };
    try_way(true).or_else(|| try_way(false)).unwrap_or_else(|| attached.iter().map(|&i| gate_y(i).expect("attached") - ICON / 2).collect())
}

/// How far apart the lines of attached parts turn toward their parts right of the VPC.
const ATTACHED_STEP: i32 = super::azgrid::TRACK_STEP;

/// For the lines run as side lines out of the part they attach to: each one's edge and how far
/// right of its gate it turns toward its part, in steps of `ATTACHED_STEP`, by the first order of
/// steps (1 nearest the VPC) that keeps the lines' outer parts from crossing one another.
fn stagger(model: &Model, layout: &Layout, p: &Piece, at: impl Fn((i32, i32)) -> (i32, i32)) -> Vec<(usize, i32)> {
    let step = ATTACHED_STEP;
    // (edge, gate, the part's centre across)
    let lines: Vec<(usize, (i32, i32), i32)> = p
        .placed
        .crossing
        .iter()
        .filter(|c| model.edges[c.edge].rel == Rel::Assoc)
        .map(|c| {
            let part = layout.icons.iter().find(|i| i.id == model.edges[c.edge].from).expect("placed");
            (c.edge, at(c.through.gate), part.rect.y + ICON / 2)
        })
        .collect();
    let n = lines.len();
    let crosses = |steps: &[i32]| {
        (0..n).any(|a| {
            (0..n).any(|b| {
                let (xa, (_, ya), ea) = (lines[a].1 .0 + steps[a] * step, lines[a].1, lines[a].2);
                let (xb, yb, eb) = (lines[b].1 .0 + steps[b] * step, lines[b].1 .1, lines[b].2);
                let (lo, hi) = (ya.min(ea), ya.max(ea));
                // b's run at its gate reaches past a's turn, or b's run into its part starts short of it.
                a != b && ((lo < yb && yb < hi && xb > xa) || (lo < eb && eb < hi && xb < xa))
            })
        })
    };
    let best = first_clear_order(n, crosses);
    lines.iter().zip(best).map(|(l, s)| (l.0, s * step)).collect()
}

/// How many orders of the attached lines are tried: every order of up to seven lines.
const STAGGER_TRIES: usize = 5040;

/// The first order of the steps 1 to `n`, in sequence, for which `crosses` is false; if none is
/// found within `STAGGER_TRIES` orders, the steps in turn. More lines than seven are left to
/// the router's last check, which names a line that crosses another.
fn first_clear_order(n: usize, crosses: impl Fn(&[i32]) -> bool) -> Vec<i32> {
    let mut order: Vec<i32> = (1..=n as i32).collect();
    let mut best = order.clone();
    for _ in 0..STAGGER_TRIES {
        if !crosses(&order) {
            best = order.clone();
            break;
        }
        // Next permutation.
        let Some(k) = (1..order.len()).rev().find(|&k| order[k - 1] < order[k]) else { break };
        let j = (k..order.len()).rev().find(|&j| order[j] > order[k - 1]).expect("exists");
        order.swap(k - 1, j);
        order[k..].reverse();
    }
    best
}

/// The Region's contents, from the Region's parts and the lines between them alone. The VPC, if
/// there is one, takes a place in its rank like a part, with its own size.
fn lay_region(model: &Model, rules: &Rules, axis: Axis, piece: Option<&Piece>) -> Block {
    let r = cross_step(model, rules, axis);
    let (least_main, _) = uv(axis, (COL_STEP, ROW_STEP));
    let (start_main, start_cross) = uv(axis, (REGION_LEFT, REGION_TOP));
    let vpc_id = model.nodes.len();
    // The parts only the VPC's parts are joined to sit right of the VPC, past room for the labels.
    let lone: Vec<usize> = match piece {
        Some(_) => (0..model.nodes.len()).filter(|&i| model.nodes[i].area == Area::Region && only_inside(model, i)).collect(),
        None => Vec::new(),
    };
    let gap = out_gap(model, rules, &lone).max(r - ICON);
    let named = |i: usize| icon_of(model, rules, i, icon_at(0, 0), None);
    // The parts only the VPC's parts are joined to sit right of it: they widen its block, never heighten it.
    let lone_right = lone.iter().map(|&i| spans(&named(i)).0 .1).max().map_or(0, |end| gap + end);
    let f = flow_of(model, piece, axis, lone_right, gap);
    let vs = rows(&f, r, true);

    // Main ranks: their lengths come from the main parts alone, so side parts never move them.
    let block_main = piece.map_or((0, 0), |p| (0, uv(axis, (p.placed.w + lone_right, p.placed.h)).0));
    let main = |i: usize| if i == vpc_id { block_main } else { uv(axis, spans(&named(i))).0 };
    let on_line = |i: usize| if i == vpc_id { block_main } else { uv(axis, on_lines(&named(i))).0 };
    let off = |i: usize| inner_offset(model, piece, axis, i);
    let ranks = f.col.iter().copied().max().unwrap_or(0);
    let mut span = vec![(0, ICON); ranks + 1];
    let mut line = vec![(0, ICON); ranks + 1];
    for (k, &i) in f.parts.iter().enumerate() {
        span[f.col[k]] = widen(span[f.col[k]], main(i));
        line[f.col[k]] = widen(line[f.col[k]], on_line(i));
    }
    let unit = |id: &str| {
        let i = model.nodes.iter().position(|n| n.id == id)?;
        let u = if piece.is_some() && inside(model, i) { vpc_id } else { i };
        f.parts.iter().position(|&p| p == u).map(|k| (k, i))
    };
    // A frame inside the Region keeps the inset of a frame in a frame from the Region's sides,
    // and clears the Region's name below it; parts keep the Region's own margins. The first rank
    // starts where all of its members can.
    let region_probe = Frame { id: REGION_FRAME.into(), kind: FrameKind::Region, label: region_name(model, rules), rect: icon_at(0, 0), parent: None };
    let (frame_main, frame_cross) = uv(axis, (CLOUD_INSET, frame_name_box(&region_probe).h + FRAME_ROOM));
    let mut u = vec![0; ranks + 1];
    if ranks > 0 {
        let first: Vec<usize> = f.parts.iter().enumerate().filter(|&(k, _)| f.col[k] == 1).map(|(_, &i)| i).collect();
        // Across the page with no line coming in, the first parts and their names keep the room
        // the frame keeps on its far side.
        let lead = if axis == Axis::Right && entered_parts(model).is_empty() { REGION_RIGHT - span[1].0 } else { start_main };
        let parts_start = if first.iter().any(|&i| i != vpc_id) { lead } else { frame_main };
        let block_start = if first.contains(&vpc_id) { frame_main } else { parts_start };
        u[1] = parts_start.max(block_start).max(FRAME_ROOM - span[1].0);
    }
    for c in 1..ranks {
        // The main lines from this rank into the next, and how many of them bend on the way.
        let lines: Vec<(Unit, Unit, &str)> = model
            .edges
            .iter()
            .filter(|e| e.rel == Rel::Main)
            .filter_map(|e| Some((unit(&e.from)?, unit(&e.to)?, e.label.as_str())))
            .filter(|&(a, b, _)| f.col[a.0] == c && f.col[b.0] == c + 1)
            .collect();
        let label = lines.iter().map(|l| uv(axis, wire_text_size(&rules.text(l.2, model.lang))).0).max().unwrap_or(0);
        let bending = lines.iter().filter(|&&(a, b, _)| vs[a.0] + off(a.1) != vs[b.0] + off(b.1)).count() as i32;
        u[c + 1] = u[c] + rank_gap(least_main, (span[c].1, span[c + 1].0), (line[c].1, line[c + 1].0), label, bending);
    }
    // The parts past the VPC's rank whose main lines all come from the other parts of that rank,
    // or from such parts in turn, follow them at the distances their own ranks take without the
    // VPC, rather than past the VPC's end.
    let vpc_rank = f.parts.iter().position(|&i| i == vpc_id).map(|k| f.col[k]);
    let mut near = vec![false; f.parts.len()];
    let mut near_u = u.clone();
    if let Some(cv) = vpc_rank {
        let beside = |k: usize, near: &[bool]| f.parts[k] != vpc_id && (f.col[k] == cv || near[k]);
        for c in cv + 1..=ranks {
            for k in (0..f.parts.len()).filter(|&k| f.col[k] == c) {
                near[k] = !f.preds[k].is_empty() && f.preds[k].iter().all(|&(p, _)| beside(p, &near));
            }
        }
        for c in cv..ranks {
            let (from, into): (Vec<usize>, Vec<usize>) = (
                (0..f.parts.len()).filter(|&k| f.col[k] == c && beside(k, &near)).collect(),
                (0..f.parts.len()).filter(|&k| f.col[k] == c + 1 && near[k]).collect(),
            );
            if into.is_empty() {
                break;
            }
            let (mut s, mut l) = (((0, ICON), (0, ICON)), ((0, ICON), (0, ICON)));
            for &k in &from {
                (s.0, l.0) = (widen(s.0, main(f.parts[k])), widen(l.0, on_line(f.parts[k])));
            }
            for &k in &into {
                (s.1, l.1) = (widen(s.1, main(f.parts[k])), widen(l.1, on_line(f.parts[k])));
            }
            let lines: Vec<(Unit, Unit, &str)> = model
                .edges
                .iter()
                .filter(|e| e.rel == Rel::Main)
                .filter_map(|e| Some((unit(&e.from)?, unit(&e.to)?, e.label.as_str())))
                .filter(|&(a, b, _)| from.contains(&a.0) && into.contains(&b.0))
                .collect();
            let label = lines.iter().map(|l| uv(axis, wire_text_size(&rules.text(l.2, model.lang))).0).max().unwrap_or(0);
            let bending = lines.iter().filter(|&&(a, b, _)| vs[a.0] + off(a.1) != vs[b.0] + off(b.1)).count() as i32;
            near_u[c + 1] = (near_u[c] + rank_gap(least_main, (s.0 .1, s.1 .0), (l.0 .1, l.1 .0), label, bending)).min(u[c + 1]);
        }
    }
    // (model index, u, v) of every Region part.
    let block_at = f.parts.iter().position(|&i| i == vpc_id).map(|k| (u[f.col[k]], vs[k]));
    let mut spots: Vec<(usize, i32, i32)> = f.parts.iter().enumerate().filter(|&(_, &i)| i != vpc_id).map(|(k, &i)| (i, if near[k] { near_u[f.col[k]] } else { u[f.col[k]] }, vs[k])).collect();

    // Side parts take one step across past the main parts, at the rank of the part their first
    // side line joins, or the next rank on where their names keep clear of the side parts placed
    // before them, or past the last rank.
    let sides: Vec<usize> = (0..model.nodes.len())
        .filter(|&i| model.nodes[i].area == Area::Region && !f.parts.contains(&i) && !lone.contains(&i))
        .collect();
    let side_v = f.parts.iter().enumerate().map(|(k, _)| vs[k] + f.step(k, r)).max().unwrap_or(0);
    let mut placed: Vec<(i32, Span)> = Vec::new();
    let mut extra: Vec<i32> = Vec::new();
    for &i in &sides {
        let si = main(i);
        let partner = partners(model, i).into_iter().next();
        let want = partner.and_then(|p| unit(&model.nodes[p].id)).map_or(1, |(k, _)| f.col[k]);
        let apart = |pu: i32, ps: Span, cu: i32| if cu >= pu { cu - pu >= gap_of(least_main, ps, si) } else { pu - cu >= gap_of(least_main, si, ps) };
        let clear = |cu: i32| cu + si.0 >= FRAME_ROOM && placed.iter().all(|&(pu, ps)| apart(pu, ps, cu));
        let cu = match (want..=ranks).map(|c| u[c]).find(|&cu| clear(cu)) {
            Some(cu) => cu,
            None => {
                let base = if ranks > 0 { u[ranks] + least_main } else { start_main };
                placed.iter().map(|&(pu, ps)| pu + gap_of(least_main, ps, si)).chain([base, FRAME_ROOM - si.0]).max().expect("not empty")
            }
        };
        if ranks == 0 || cu > u[ranks] {
            extra.push(cu);
        }
        placed.push((cu, si));
        spots.push((i, cu, side_v));
    }
    spots.sort_by_key(|s| s.0);

    // The parts' cross coordinate 0 lies past the frame's margin, far enough in for every name.
    // A part a line comes into from outside the Region, across the frame's first side, keeps
    // clear of the frame's own name, so the line does not run through it. With no line coming
    // in, down the page, the parts and their names keep the room the frame keeps on its far side,
    // so the frame sits evenly around them.
    let probe = Frame { id: REGION_FRAME.into(), kind: FrameKind::Region, label: region_name(model, rules), rect: icon_at(0, 0), parent: None };
    let name = frame_name_box(&probe);
    let name_across = uv(axis, (name.x + name.w, name.y + name.h)).1;
    let named_rule = name_across + FRAME_ROOM - ICON / 2;
    let entered = entered_parts(model);
    let across_with = |rule: i32| match block_at {
        None => spots.iter().map(|&(i, _, at)| FRAME_ROOM - uv(axis, spans(&named(i))).1 .0 - at).fold(start_cross.max(rule), i32::max),
        // The VPC is entered through its gates, deep inside it, so only the parts keep clear of the name.
        Some((_, bv)) => spots
            .iter()
            .map(|&(i, _, at)| (FRAME_ROOM - uv(axis, spans(&named(i))).1 .0).max(rule).max(start_cross) - at)
            .chain([frame_cross - bv])
            .max()
            .expect("not empty"),
    };
    let mut across = if entered.is_empty() && axis == Axis::Down {
        let even = |i: usize, at: i32| REGION_RIGHT - uv(axis, spans(&named(i))).1 .0 - at;
        spots.iter().map(|&(i, _, at)| even(i, at)).chain(block_at.map(|(_, bv)| frame_cross - bv)).max().unwrap_or(start_cross)
    } else {
        let loose = across_with(i32::MIN);
        if spots.iter().any(|&(i, _, at)| entered.contains(&i) && loose + at < named_rule) { across_with(named_rule) } else { loose }
    };

    let mut icons: Vec<Icon> = spots
        .iter()
        .map(|&(i, cu, cv)| {
            let (x, y) = uv(axis, (cu, across + cv));
            icon_of(model, rules, i, icon_at(x, y), Some(REGION_FRAME.into()))
        })
        .collect();
    let (mut bw, mut bh) = (REGION_LEFT + ICON + REGION_RIGHT, REGION_TOP + ICON + REGION_BOTTOM);
    let mut vpc = None;
    if let (Some(p), Some((bu, bv))) = (piece, block_at) {
        let (ox, oy) = uv(axis, (bu, across + bv));
        vpc = Some((ox, oy));
        icons.extend(p.placed.icons.iter().map(|i| Icon { rect: Rect { x: i.rect.x + ox, y: i.rect.y + oy, ..i.rect }, ..i.clone() }));
        // Right of the VPC, level with the first private part sending to it, else the first part
        // inside it joined to it; a row lower each time the place is taken.
        let x = ox + p.placed.w + gap;
        // Run as side lines out of the part they attach to, the attached parts come before the
        // others, in a column in the order of their gates.
        let gate_y = |i: usize| attached_gates(model, p, i).into_iter().min().map(|g| g + oy);
        let mut attached: Vec<usize> = lone.iter().copied().filter(|&i| gate_y(i).is_some()).collect();
        attached.sort_by_key(|&i| gate_y(i));
        let at_gates = attached_places(model, rules, &attached, &icons, x, oy, &gate_y);
        let order: Vec<usize> = attached.iter().copied().chain(lone.iter().copied().filter(|i| !attached.contains(i))).collect();
        for &i in &order {
            let joined: Vec<usize> = partners_all(model, i).into_iter().filter(|&j| inside(model, j)).collect();
            let sender = joined.iter().copied().find(|&j| matches!(model.nodes[j].area, Area::Subnet(crate::model::Tier::Private))).or(joined.first().copied());
            let mut y = sender.and_then(|j| icons.iter().find(|ic| ic.id == model.nodes[j].id)).map_or(oy, |ic| ic.rect.y);
            if let Some(k) = attached.iter().position(|&a| a == i) {
                y = at_gates[k];
            }
            let mut ic = icon_of(model, rules, i, icon_at(x, y), Some(REGION_FRAME.into()));
            // Lower each time the place is taken: past a part of its column a row's step apart,
            // else a row lower.
            loop {
                if let Some(o) = icons.iter().find(|o| rows_too_close(o, &ic)) {
                    y = o.rect.y + ROW_STEP;
                } else if icons.iter().any(|o| boxes_meet(o, &ic)) {
                    y += ROW_STEP;
                } else {
                    break;
                }
                ic = icon_of(model, rules, i, icon_at(x, y), Some(REGION_FRAME.into()));
            }
            icons.push(ic);
        }
        bw = bw.max(ox + p.placed.w + REGION_RIGHT);
        bh = bh.max(oy + p.placed.h + REGION_BOTTOM);
    }
    if !icons.is_empty() {
        bw = bw.max(icons.iter().map(|i| extent(i).0).max().expect("not empty") + REGION_RIGHT);
        bh = bh.max(icons.iter().map(|i| extent(i).1).max().expect("not empty") + REGION_BOTTOM);
        if vpc.is_none() {
            bw = icons.iter().map(|i| extent(i).0).max().expect("not empty") + REGION_RIGHT;
            bh = icons.iter().map(|i| extent(i).1).max().expect("not empty") + REGION_BOTTOM;
        }
    }
    // Wide enough for the frame's own name. With no line coming in and no VPC, the parts move
    // to the middle of the room the name adds, so the frame stays even around them.
    let content_w = bw;
    bw = bw.max(FRAME_NAME_INDENT + name_text_width(&region_name(model, rules)) + FRAME_ROOM);
    let shift = if entered.is_empty() && vpc.is_none() { (bw - content_w) / 2 } else { 0 };
    for i in &mut icons {
        i.rect.x += shift;
    }
    let (shift_main, shift_cross) = uv(axis, (shift, 0));
    across += shift_cross;
    let near_ranks = (0..f.parts.len()).filter(|&k| near[k]).map(|k| near_u[f.col[k]]);
    let mut centres: Vec<i32> = u[1..].iter().copied().chain(extra).chain(near_ranks).map(|c| c + ICON / 2 + shift_main).collect();
    centres.sort_unstable();
    centres.dedup();
    Block { w: bw, h: bh, icons, ranks: centres, across, vpc }
}

/// Whether two parts of one column in one frame have their tops less than a row's step apart.
/// The layout lowers a part past such a neighbour, and the layout's check stops on a pair left so.
pub(super) fn rows_too_close(a: &Icon, b: &Icon) -> bool {
    a.rect.x == b.rect.x && a.parent == b.parent && (a.rect.y - b.rect.y).abs() < ROW_STEP
}

/// Whether two parts' icons and names come within room of each other.
fn boxes_meet(a: &Icon, b: &Icon) -> bool {
    let grown = |r: Rect| Rect { x: r.x - ACROSS_ROOM, y: r.y - ACROSS_ROOM, w: r.w + 2 * ACROSS_ROOM, h: r.h + 2 * ACROSS_ROOM };
    let meet = |p: Rect, q: Rect| p.x < q.x + q.w && q.x < p.x + p.w && p.y < q.y + q.h && q.y < p.y + p.h;
    [a.rect, name_area(a)].iter().any(|&p| [b.rect, name_area(b)].iter().any(|&q| meet(grown(p), q)))
}

/// The room right of the VPC for the parts only its parts send to: the widest label of the lines
/// into them, one part of it to a row, and room on both sides, and at least `OUT_GAP`.
fn out_gap(model: &Model, rules: &Rules, lone: &[usize]) -> i32 {
    let note = wire_text_width(&rules.text("from_all_azs", model.lang));
    let widest = model
        .edges
        .iter()
        .filter(|e| lone.iter().any(|&i| model.nodes[i].id == e.to || model.nodes[i].id == e.from))
        .map(|e| wire_text_width(&rules.text(&e.label, model.lang)).max(note))
        .max()
        .unwrap_or(0);
    OUT_GAP.max(widest + WIRE_ROOM)
}

/// How far across the VPC's own coordinate lies from part `i`'s inside it, for lines to be level;
/// 0 for a part outside it.
fn inner_offset(model: &Model, piece: Option<&Piece>, axis: Axis, i: usize) -> i32 {
    match piece {
        Some(p) if i < model.nodes.len() && inside(model, i) => {
            p.placed.icons.iter().find(|ic| ic.id == model.nodes[i].id).map_or(0, |ic| uv(axis, (ic.rect.x, ic.rect.y)).1)
        }
        _ => 0,
    }
}

/// Every part a part's lines join, main lines too, in model order, each once.
fn partners_all(model: &Model, i: usize) -> Vec<usize> {
    let id = &model.nodes[i].id;
    let mut out: Vec<usize> = Vec::new();
    for e in &model.edges {
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
#[cfg(test)]
pub(super) fn flow(model: &Model) -> Flow {
    flow_of(model, None, Axis::Right, 0, 0)
}

/// Like `flow`, with the VPC as one part, standing at the index one past the model's nodes, for
/// every part inside it, and `room` across after it. Parts only the VPC's parts are joined to are
/// left out: they sit right of the VPC, taking `lone_right` past its right side.
fn flow_of(model: &Model, piece: Option<&Piece>, axis: Axis, lone_right: i32, room: i32) -> Flow {
    let vpc_id = model.nodes.len();
    let mut parts: Vec<usize> = (0..model.nodes.len())
        .filter(|&i| model.nodes[i].area == Area::Region && on_main(model, &model.nodes[i].id) && !(piece.is_some() && only_inside(model, i)))
        .collect();
    let index = |id: &str| model.nodes.iter().position(|n| n.id == id);
    // The VPC stands in the flow whenever it is drawn, also when no main line enters or leaves
    // it: its parts then start the flows and send only to each other or out through its gateways.
    if piece.is_some() {
        parts.insert(0, vpc_id);
    }
    let unit = |i: usize| if piece.is_some() && inside(model, i) { vpc_id } else { i };
    let local = |i: usize| parts.iter().position(|&p| p == unit(i));
    let off = |i: usize| inner_offset(model, piece, axis, i);
    let n = parts.len();
    let (mut preds, mut succs): (Vec<Vec<Link>>, Vec<Vec<Link>>) = (vec![Vec::new(); n], vec![Vec::new(); n]);
    for e in model.edges.iter().filter(|e| e.rel == Rel::Main) {
        let (Some(ia), Some(ib)) = (index(&e.from), index(&e.to)) else { continue };
        if let (Some(a), Some(b)) = (local(ia), local(ib)) {
            let d = off(ia) - off(ib);
            if a != b && !succs[a].contains(&(b, -d)) {
                succs[a].push((b, -d));
                preds[b].push((a, d));
            }
        }
    }
    // The accepted main pairs have no cycle, so n rounds settle the longest paths in any order.
    let mut col = vec![1usize; n];
    for _ in 0..n {
        for i in 0..n {
            col[i] = preds[i].iter().map(|&(p, _)| col[p] + 1).max().unwrap_or(1);
        }
    }
    // A rank after the first is put in the order of its parts' predecessors in the rank before,
    // by the middle of the first and the last of them, in model order between parts level so,
    // with the VPC first; but only when that leaves fewer pairs of lines from the rank before
    // that cross, and no part of the rank is reached by a main line from outside these parts,
    // whose line comes in from where the order does not see. In model order, a part could come
    // before the parts its predecessor's neighbours before it lead to, and its line crossed theirs.
    let from_outside: Vec<bool> = (0..n)
        .map(|k| {
            model.edges.iter().any(|e| {
                e.rel == Rel::Main && index(&e.to).and_then(local) == Some(k) && index(&e.from).and_then(local).is_none()
            })
        })
        .collect();
    let last = col.iter().copied().max().unwrap_or(0);
    let mut order: Vec<usize> = Vec::with_capacity(n);
    let mut pos = vec![0usize; n];
    for c in 1..=last {
        let mut rank: Vec<usize> = (0..n).filter(|&i| col[i] == c).collect();
        if c > 1 && !rank.iter().any(|&i| from_outside[i]) {
            let before: Vec<Vec<usize>> = (0..n).map(|i| preds[i].iter().filter(|&&(p, _)| col[p] == c - 1).map(|&(p, _)| pos[p]).collect()).collect();
            let mut sorted = rank.clone();
            sorted.sort_by_key(|&i| {
                let (lo, hi) = (before[i].iter().min().copied().unwrap_or(0), before[i].iter().max().copied().unwrap_or(0));
                (parts[i] != vpc_id, lo + hi)
            });
            // Pairs of lines from the rank before, sharing no end, whose ends come in opposite orders.
            let crossings = |r: &[usize]| {
                let ls: Vec<(usize, usize)> = r.iter().enumerate().flat_map(|(k, &i)| before[i].iter().map(move |&p| (p, k))).collect();
                let mut count = 0;
                for (a, x) in ls.iter().enumerate() {
                    count += ls[a + 1..].iter().filter(|y| x.0 != y.0 && x.1 != y.1 && (x.0 < y.0) != (x.1 < y.1)).count();
                }
                count
            };
            if crossings(&sorted) < crossings(&rank) {
                rank = sorted;
            }
        }
        for (k, &i) in rank.iter().enumerate() {
            pos[i] = k;
        }
        order.extend(rank);
    }
    let mut new_of = vec![0usize; n];
    for (k, &i) in order.iter().enumerate() {
        new_of[i] = k;
    }
    let relink = |ls: &Vec<Link>| -> Vec<Link> { ls.iter().map(|&(p, d)| (new_of[p], d)).collect() };
    let parts: Vec<usize> = order.iter().map(|&i| parts[i]).collect();
    let col: Vec<usize> = order.iter().map(|&i| col[i]).collect();
    let preds: Vec<Vec<Link>> = order.iter().map(|&i| relink(&preds[i])).collect();
    let succs: Vec<Vec<Link>> = order.iter().map(|&i| relink(&succs[i])).collect();
    // A part after the VPC across in its rank whose main lines go on along the main axis would run
    // them through the parts right of the VPC if it shared their place across, so the VPC's size
    // across then takes them in. Across the main axis running right, that size is the VPC's height
    // and they never change it.
    let block = parts.iter().position(|&i| i == vpc_id).zip(piece).map(|(k, p)| {
        let onward = (0..n).any(|i| i != k && col[i] == col[k] && !succs[i].is_empty());
        (k, uv(axis, (p.placed.w + if onward { lone_right } else { 0 }, p.placed.h)).1, room)
    });
    Flow { parts, col, preds, succs, block }
}

/// From the first rank: its parts one after another, every other part level with the middle of
/// its predecessors, at least a step past the part before it and at least at its `least`.
fn forward_rows(f: &Flow, r: i32, least: &[Option<i32>]) -> Vec<i32> {
    let last = f.col.iter().copied().max().unwrap_or(0);
    let mut v = vec![0; f.parts.len()];
    for c in 1..=last {
        let mut before: Option<usize> = None;
        for i in (0..f.parts.len()).filter(|&i| f.col[i] == c) {
            let at = after(before.map(|b| v[b]), mid(f.preds[i].iter().map(|&(p, d)| v[p] + d)), before.map_or(r, |b| f.step(b, r)));
            v[i] = least[i].map_or(at, |l| at.max(l));
            before = Some(i);
        }
    }
    v
}

/// The least cross coordinate of each part, so that a part whose main lines all go to its own
/// targets (those it alone flows into) comes past the own targets of the parts before it in its
/// rank, and their lines do not meet where they turn. None when the moves do not settle within
/// `rounds`.
///
/// A rank is looked at only when, in the next rank, every own target of its parts comes in the
/// order of its part: a part moved there then moves only parts after the targets it is moved
/// past. A part is moved only when all its targets and those it is moved past are in the next
/// rank. A part sharing a target with others is left where it is: its line crosses theirs
/// wherever it lies, and moving it only spreads the drawing.
fn least_rows(f: &Flow, r: i32, rounds: usize) -> Option<Vec<Option<i32>>> {
    let last = f.col.iter().copied().max().unwrap_or(0);
    let in_rank = |c: usize| -> Vec<usize> { (0..f.parts.len()).filter(|&i| f.col[i] == c).collect() };
    let own = |i: usize| -> Vec<usize> { f.succs[i].iter().map(|s| s.0).filter(|&s| f.preds[s].iter().all(|&(p, _)| p == i)).collect() };
    let ordered: Vec<bool> = (0..=last)
        .map(|c| {
            let rank = in_rank(c);
            let parents: Vec<usize> = in_rank(c + 1)
                .into_iter()
                .filter_map(|t| match f.preds[t].as_slice() {
                    [(p, _), rest @ ..] if rest.iter().all(|q| q.0 == *p) => rank.iter().position(|&i| i == *p),
                    _ => None,
                })
                .collect();
            parents.windows(2).all(|w| w[0] <= w[1])
        })
        .collect();
    let mut least: Vec<Option<i32>> = vec![None; f.parts.len()];
    for _ in 0..rounds {
        let v = forward_rows(f, r, &least);
        let mut raised = false;
        'ranks: for c in (1..=last).filter(|&c| ordered[c]) {
            let rank = in_rank(c);
            for (k, &j) in rank.iter().enumerate() {
                if f.succs[j].is_empty() || own(j).len() < f.succs[j].len() {
                    continue;
                }
                let earlier: Vec<usize> = rank[..k].iter().flat_map(|&i| own(i)).collect();
                let next = |t: &usize| f.col[*t] == c + 1;
                if !earlier.iter().all(next) || !f.succs[j].iter().all(|s| next(&s.0)) {
                    continue;
                }
                let far = earlier.iter().map(|&t| v[t] + f.step(t, r)).max();
                if let Some(far) = far.filter(|&far| v[j] < far) {
                    least[j] = Some(far);
                    raised = true;
                    break 'ranks;
                }
            }
        }
        if !raised {
            return Some(least);
        }
    }
    None
}

/// The cross coordinate of every part of `f`, a step `r` apart in each rank (more after the VPC),
/// with the smallest at 0.
pub(super) fn rows(f: &Flow, r: i32, sweep_back: bool) -> Vec<i32> {
    let last = f.col.iter().copied().max().unwrap_or(0);
    let in_rank = |c: usize| -> Vec<usize> { (0..f.parts.len()).filter(|&i| f.col[i] == c).collect() };
    let n = f.parts.len();
    // Moves that do not settle within the bound are dropped, and the parts are placed as if none
    // were made: a drawing that kept growing is worse than lines that meet.
    let least = least_rows(f, r, n * n + 1).unwrap_or_else(|| vec![None; n]);
    let mut v = forward_rows(f, r, &least);
    // From the last rank back: a part with two or more targets moves toward their middle, as far
    // as the part before it (already moved) and the part after it (not yet moved) leave a step.
    if sweep_back {
        for c in (1..last).rev() {
            let rank = in_rank(c);
            for (k, &i) in rank.iter().enumerate() {
                let mut targets: Vec<usize> = f.succs[i].iter().map(|s| s.0).collect();
                targets.sort_unstable();
                targets.dedup();
                if targets.len() < 2 {
                    continue;
                }
                let mut at = mid(f.succs[i].iter().map(|&(s, d)| v[s] + d)).expect("two or more targets");
                if k > 0 {
                    at = at.max(v[rank[k - 1]] + f.step(rank[k - 1], r));
                }
                if k + 1 < rank.len() {
                    at = at.min(v[rank[k + 1]] - f.step(i, r));
                }
                if let Some(l) = least[i] {
                    at = at.max(l);
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
    let within = |id: &str| model.nodes.iter().position(|n| n.id == id).is_some_and(|i| inside(model, i));
    for (edge, e) in model.edges.iter().enumerate() {
        // The VPC drafts the lines with an end inside it.
        if within(&e.from) || within(&e.to) {
            continue;
        }
        let (Some(a), Some(b)) = (find(&e.from), find(&e.to)) else { continue };
        let ((au, av), (bu, bv)) = (at(a), at(b));
        // Main lines bend halfway from the end of `a` to whatever comes next along the main axis.
        let from = au + uv(axis, on_lines(a)).0 .1;
        let next = layout
            .frames
            .iter()
            .map(|f| uv(axis, (f.rect.x, f.rect.y)).0)
            .chain(col_u.iter().map(|c| c - ICON / 2))
            .filter(|&s| s > from)
            .min()
            .unwrap_or(from + 2 * NAME_ROOM);
        let turn = (from + next).div_euclid(2);
        let way_around = (e.rel == Rel::Main && av != bv).then(|| around_the_vpc(layout, a, b, axis, turn)).flatten();
        let around = way_around.is_some();
        let (exit, entry, bend) = if let Some(points) = way_around {
            (cross_start, cross_start, Bend::Points(points))
        } else if e.rel == Rel::Main {
            let bend = if av == bv { Bend::Straight } else { bend_at(axis, true, past_the_vpc(layout, a, b, axis, turn)) };
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
            // A way around the Region leaves and enters at the middle of its sides, where its points meet them.
            exit_pos: around.then_some(50),
            entry_pos: around.then_some(50),
            keep: Keep::Free,
            tracks: Vec::new(),
            axis,
        });
    }
    out
}

/// For a main line from a part outside the Region into a part in it whose plain draft would run
/// through the VPC: the points of a way around the Region on its low side across the main axis
/// (above it when lines run right, left of it when they run down), midway between the Region's
/// side and its parent frame's, clear of the Region's and the VPC's names. The line leaves and
/// enters by the low side of its parts across the main axis.
fn around_the_vpc(layout: &Layout, a: &Icon, b: &Icon, axis: Axis, turn: i32) -> Option<Vec<(i32, i32)>> {
    let frame = |id: &str| layout.frames.iter().find(|f| f.id == id);
    let (vpc, region) = (frame(super::azgrid::VPC_FRAME)?.rect, frame(REGION_FRAME)?);
    if a.parent.as_deref() == Some(REGION_FRAME) || b.parent.as_deref() != Some(REGION_FRAME) {
        return None;
    }
    let cloud = frame(region.parent.as_deref()?)?.rect;
    let region = region.rect;
    let centre = |r: Rect| uv(axis, (r.x + r.w / 2, r.y + r.h / 2));
    let ((au, av), (bu, bv)) = (centre(a.rect), centre(b.rect));
    let end = |r: Rect| uv(axis, (right(r), bottom(r))).0;
    let start = |r: Rect| uv(axis, (r.x, r.y)).0;
    // The plain draft: out of the end of `a`, across at `turn`, along into the start of `b`.
    let plain = [(end(a.rect), av), (turn, av), (turn, bv), (start(b.rect), bv)].map(|p| uv(axis, p));
    if !plain.windows(2).any(|s| crate::path::hits(s[0], s[1], vpc)) {
        return None;
    }
    let low = |r: Rect| uv(axis, (r.x, r.y)).1;
    let w = (low(cloud) + low(region)).div_euclid(2);
    Some(vec![uv(axis, (au, w)), uv(axis, (bu, w))])
}

/// Where a main line between two parts of the Region outside the VPC turns across: at `turn`,
/// unless its draft would then run through the VPC and turning halfway between the VPC's end
/// along the main axis and the start of `b` keeps it clear.
fn past_the_vpc(layout: &Layout, a: &Icon, b: &Icon, axis: Axis, turn: i32) -> i32 {
    let Some(vpc) = layout.frames.iter().find(|f| f.id == super::azgrid::VPC_FRAME).map(|f| f.rect) else { return turn };
    let centre = |r: Rect| uv(axis, (r.x + r.w / 2, r.y + r.h / 2));
    let ((_, av), (_, bv)) = (centre(a.rect), centre(b.rect));
    let end = |r: Rect| uv(axis, (right(r), bottom(r))).0;
    let start = |r: Rect| uv(axis, (r.x, r.y)).0;
    let draft = |t: i32| [(end(a.rect), av), (t, av), (t, bv), (start(b.rect), bv)].map(|p| uv(axis, p));
    let through = |t: i32| draft(t).windows(2).any(|s| crate::path::hits(s[0], s[1], vpc));
    if !through(turn) || start(b.rect) <= end(vpc) {
        return turn;
    }
    let past = (end(vpc) + start(b.rect)).div_euclid(2);
    if through(past) {
        turn
    } else {
        past
    }
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
fn gap_of(least: i32, first: Span, second: Span) -> i32 {
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
    use super::super::arrange;
    use crate::measure::{NAME_CHARS, NAME_DRAWN, NAME_WIDE_CHARS, NAME_WIDE_DRAWN};
    use crate::route::{port_point, route_planned, Wire};
    use crate::test_inputs::{generated, through_cloudfront, API_CASES};

    // Fails if the search for the order in which attached lines turn tries more orders than its
    // bound however many lines there are, stops trying every order of up to seven lines, or stops
    // taking the first clear order in sequence.
    #[test]
    fn the_order_of_attached_lines_is_searched_within_a_bound() {
        for n in [7, 8, 12] {
            let tried = std::cell::Cell::new(0usize);
            let order = first_clear_order(n, |_| {
                tried.set(tried.get() + 1);
                true
            });
            assert_eq!(order, (1..=n as i32).collect::<Vec<_>>(), "{n}");
            let all: usize = (1..=n).product();
            assert_eq!(tried.get(), all.min(STAGGER_TRIES), "{n}");
        }
        assert_eq!(STAGGER_TRIES, (1..=7).product::<usize>());
        let seen = std::cell::RefCell::new(Vec::new());
        let order = first_clear_order(3, |o| {
            seen.borrow_mut().push(o.to_vec());
            o != [3, 1, 2]
        });
        assert_eq!(order, [3, 1, 2]);
        assert_eq!(seen.into_inner(), [[1, 2, 3], [1, 3, 2], [2, 1, 3], [2, 3, 1], [3, 1, 2]]);
    }

    /// The model of a definition without a VPC, and its layout with lines running along `axis`.
    pub(in crate::arrange) fn arranged(src: &str, axis: Axis) -> (Model, Placement) {
        let r = Rules::builtin();
        let (m, d) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        let p = arrange(&m, &r, axis).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
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
        let (m, _) = crate::inference::infer(&crate::def::parse(SHARED).unwrap(), &r);
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

    #[test]
    fn names_wider_than_the_column_step_widen_the_columns() {
        // Spans are from the icon's left side: a part 300 wide runs from 30 - 150 to 30 + 150.
        // Icons alone: max(220, 60 - 0 + 20, 60 - 0 + 0 + 40) = 220.
        assert_eq!(rank_gap(COL_STEP, (60, 0), (ICON, 0), 0, 0), 220);
        // Two parts 300 wide: 180 - (-120) + 20 = 320.
        assert_eq!(rank_gap(COL_STEP, (180, -120), (ICON, 0), 0, 0), 320);
        // An odd end on the first part: 181 - (-120) + 20 = 321.
        assert_eq!(rank_gap(COL_STEP, (181, -120), (ICON, 0), 0, 0), 321);
        // An odd start on the second part: 180 - (-121) + 20 = 321.
        assert_eq!(rank_gap(COL_STEP, (180, -121), (ICON, 0), 0, 0), 321);
        // A label 200 wide on a straight line: 60 - 0 + 200 + 40 = 300.
        assert_eq!(rank_gap(COL_STEP, (60, 0), (ICON, 0), 200, 1), 300);
    }

    #[test]
    fn each_further_bending_main_line_widens_its_gap() {
        // Each bending line past the first adds 60: 220, 220, 220 + 60, 220 + 120.
        assert_eq!(rank_gap(COL_STEP, (60, 0), (ICON, 0), 0, 0), 220);
        assert_eq!(rank_gap(COL_STEP, (60, 0), (ICON, 0), 0, 1), 220);
        assert_eq!(rank_gap(COL_STEP, (60, 0), (ICON, 0), 0, 2), 280);
        assert_eq!(rank_gap(COL_STEP, (60, 0), (ICON, 0), 0, 3), 340);
        // Two parts 300 wide and two bending lines: 180 - (-120) + 20 + 60 = 380.
        assert_eq!(rank_gap(COL_STEP, (180, -120), (ICON, 0), 0, 2), 380);

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

    // Fails if, top to bottom with a VPC, a part attached to CloudFront leaves CloudFront's row,
    // the Region moves down by more than the room above that row and its names, a dotted line to
    // an attached part crosses an icon, a name, a label or another line or runs along one, or a
    // line is named in a note: one, two and three parts attached, with and without a line from
    // CloudFront to S3, in English and Japanese.
    #[test]
    fn top_to_bottom_attachments_share_the_row_of_their_distribution() {
        let shapes = [
            "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n",
            "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront -> s3\n  - ecs -> s3\n  - alb ~> cloudwatch\n",
            "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - acm.alb -- alb\n  - waf.alb -- alb\n",
        ];
        let attached = ["  - waf -- cloudfront\n", "  - acm -- cloudfront\n", "  - route53 -- cloudfront\n"];
        let r = Rules::builtin();
        for shape in shapes {
            for lang in ["en", "ja"] {
                let (_, bare) = arranged(&format!("{shape}set:\n  lang: {lang}\n"), Axis::Down);
                for n in 1..=attached.len() {
                    let src = format!("{shape}{}set:\n  lang: {lang}\n", attached[..n].concat());
                    let (m, p) = arranged(&src, Axis::Down);
                    let cf = at(&p, "cloudfront");
                    for id in ["waf", "acm", "route53"].iter().take(n) {
                        assert_eq!(at(&p, id).y, cf.y, "{id} leaves CloudFront's row\n{src}");
                    }
                    // The Region moves down only by the room for the lines run over the row and
                    // as far as a taller name in the row reaches past CloudFront's, never by a row.
                    let name_end = |id: &str| { let l = label_of(&p, id); l.y + l.h };
                    let deeper = ["waf", "acm", "route53"].iter().take(n).map(|id| name_end(id)).max().unwrap() - name_end("cloudfront");
                    assert_eq!(at(&p, "region").y, at(&bare, "region").y + OVER_ROOM * (n as i32 - 1) + deeper.max(0), "the Region moves down\n{src}");
                    assert!(OVER_ROOM * (n as i32 - 1) + deeper.max(0) < p.row_step / 2, "{src}");
                    let (w, notes) = crate::route::route_planned(&m, &p.layout, &r, &p.lines);
                    let (faults, named) = crate::route::tests::wire_faults(&p.layout, &w, &notes);
                    assert!(named.is_empty(), "{named:?}\n{src}");
                    let dotted: Vec<&str> = w.iter().filter(|x| x.style == crate::route::WireStyle::Assoc && (x.from == "cloudfront" || x.to == "cloudfront")).map(|x| x.id.as_str()).collect();
                    assert_eq!(dotted.len(), n, "{src}");
                    let bad: Vec<&(String, String)> = faults.iter().filter(|(id, what)| dotted.contains(&id.as_str()) || dotted.iter().any(|d| what.ends_with(d))).collect();
                    assert!(bad.is_empty(), "{bad:?}\n{src}");
                    let paths: Vec<(&str, Vec<(i32, i32)>)> = w.iter().map(|x| (x.id.as_str(), crate::route::tests::full_path(x, &p.layout))).collect();
                    for (id, a) in paths.iter().filter(|(id, _)| dotted.contains(id)) {
                        for (other, b) in paths.iter().filter(|(o, _)| o != id) {
                            let crossed = a.windows(2).any(|s| b.windows(2).any(|t| crate::path::crosses(s[0], s[1], t[0], t[1])));
                            assert!(!crossed, "{id} crosses {other}\n{src}");
                        }
                    }
                }
            }
        }
    }

    // Fails if, top to bottom, the parts attached to CloudFront leave the column one step across
    // from it when a line from the users comes down past them or into it from past them, or when
    // the diagram has no VPC; or if they leave it left to right.
    #[test]
    fn attachments_stay_in_a_column_where_a_row_would_cross_lines() {
        let two = "  - waf -- cloudfront\n  - acm -- cloudfront\n";
        let column = [
            ("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> apigateway(http) -> lambda -> aurora\n", Axis::Down),
            ("flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(rest) -> lambda -> dynamodb\n", Axis::Down),
            ("flows:\n  - user -> cloudfront -> s3\n", Axis::Down),
            ("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n", Axis::Right),
        ];
        for (shape, axis) in column {
            let src = format!("{shape}{two}");
            let (_, p) = arranged(&src, axis);
            let (cf, waf, acm) = (at(&p, "cloudfront"), at(&p, "waf"), at(&p, "acm"));
            let across = |r: Rect| uv(axis, (r.x, r.y)).1;
            let along = |r: Rect| uv(axis, (r.x, r.y)).0;
            assert_eq!((across(waf), across(acm)), (across(cf) + p.row_step, across(cf) + p.row_step), "{src}");
            assert_eq!(along(waf), along(cf), "{src}");
            assert!(along(acm) > along(waf), "{src}");
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

    #[test]
    fn generated_definitions_lay_out_cleanly() {
        let r = Rules::builtin();
        let defs = generated();
        assert!(defs.len() > 1000, "{}", defs.len());
        let mut failed: Vec<String> = Vec::new();
        for src in &defs {
            let (m, d) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
            let Some(m) = m.filter(|_| d.errors.is_empty() && d.holes.is_empty()) else {
                failed.push(format!("inference: {:?} {:?}\n{src}", d.errors, d.holes));
                continue;
            };
            let p = match arrange(&m, &r, Axis::Right) {
                Ok(p) => p,
                Err(e) => {
                    failed.push(format!("place: {e:?}\n{src}"));
                    continue;
                }
            };
            if let Some(spread) = super::super::spread_too_far(&p.layout) {
                failed.push(format!("size: {spread}\n{src}"));
            }
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
    /// each picked definition, with `direction: right` written, as `<dir>/<number>.yaml` and what `compile` makes of it as
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
            // Written to run right, as these diagrams have always been drawn; down is checked by the other tests.
            let src = &match src.find("set:\n") {
                Some(at) => format!("{}  direction: right\n{}", &src[..at + 5], &src[at + 5..]),
                None => format!("{src}set:\n  direction: right\n"),
            };
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

    /// Every model the down layout is tried on: the fixed API examples and every generated
    /// definition, with the definition each came from.
    fn down_inputs() -> Vec<(String, Model)> {
        let r = Rules::builtin();
        let defs = API_CASES.iter().map(|(_, s)| s.to_string()).chain(generated());
        defs.map(|src| {
            let (m, d) = crate::inference::infer(&crate::def::parse(&src).unwrap(), &r);
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
            match arrange(&m, &r, Axis::Down) {
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
            let (m, d) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
            let (x, _) = crate::arranged_drawio(&m.unwrap(), &d, &r, Axis::Down).unwrap();
            let shapes: Vec<&str> = x.lines().filter(|l| l.contains("mxgraph.aws4.") && l.contains("vertex=\"1\"") && !l.contains("mxgraph.aws4.group")).collect();
            assert!(!shapes.is_empty(), "{name}");
            for s in shapes {
                assert!(s.contains("verticalLabelPosition=bottom;verticalAlign=top"), "{name}: {s}");
            }
        }
    }

    /// The drawing of a mixed definition along `axis`, failing on anything but a clean diagram.
    fn drawn_mixed(src: &str, axis: Axis) -> String {
        use super::super::generated::{draw_counted, Drawn};
        match draw_counted(src, axis) {
            Drawn::Drawn { xml, notes, lines_broken, .. } => {
                let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                assert!(broken.is_empty() && lines_broken.is_empty(), "{axis:?} {broken:?} {lines_broken:?}\n{src}");
                xml
            }
            Drawn::LayoutFailed(e) => panic!("{axis:?} layout failed {e:?}\n{src}"),
            Drawn::Errors(e) => panic!("{axis:?} errors {e:?}\n{src}"),
            Drawn::Panicked(p) => panic!("{axis:?} panicked {p}\n{src}"),
        }
    }

    /// The layout of a definition with a VPC along `axis`.
    fn arranged_mixed(src: &str, axis: Axis) -> (Model, Placement) {
        let r = Rules::builtin();
        let (m, d) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        let p = arrange(&m, &r, axis).unwrap_or_else(|e| panic!("{e:?}\n{src}"));
        (m, p)
    }

    const ONWARD_BESIDE_THE_VPC: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\n  - apigateway -> lambda.report -> dynamodb\nset:\n  azs: 2\n  lang: en\n";
    const ONWARD_BESIDE_THE_VPC_WEB: &str = "flows:\n  - user -> alb -> ecs\n  - ecs -> dynamodb\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\n  - apigateway -> lambda.report -> dynamodb\n  - lambda ~> cloudwatch\nset:\n  azs: 2\n  lang: ja\nchoose:\n  route.ecs.dynamodb: endpoint\n  route.lambda.s3: endpoint\n";
    const LEAF_BESIDE_THE_VPC: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\n  - apigateway -> lambda.report\nset:\n  azs: 2\n  lang: en\n";
    const LEAF_BESIDE_THE_VPC_LOGS: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - apigateway -> lambda.report\n  - lambda ~> cloudwatch\n";

    // Fails if a part beside the VPC in its rank whose main lines go on shares its place across
    // with the parts only the VPC's parts are joined to, running its lines through them.
    #[test]
    fn a_part_beside_the_vpc_with_lines_going_on_clears_the_parts_right_of_it() {
        for src in [ONWARD_BESIDE_THE_VPC, ONWARD_BESIDE_THE_VPC_WEB] {
            for axis in [Axis::Right, Axis::Down] {
                drawn_mixed(src, axis);
            }
            let (m, p) = arranged_mixed(src, Axis::Down);
            let report = at(&p, "lambda.report");
            let lone: Vec<&Icon> = p.layout.icons.iter().filter(|i| m.nodes.iter().position(|n| n.id == i.id).is_some_and(|k| m.nodes[k].area == Area::Region && only_inside(&m, k))).collect();
            assert!(!lone.is_empty(), "{src}");
            for i in lone {
                assert!(report.x >= extent(i).0, "{} at {:?} reaches past lambda.report at {report:?}\n{src}", i.id, i.rect);
            }
        }
    }

    // Fails if a part only the VPC's parts are joined to is moved down only until its icon and
    // name clear the part above it, closer than a row's step in their column.
    #[test]
    fn parts_right_of_the_vpc_keep_a_row_apart_in_their_column() {
        for src in [LEAF_BESIDE_THE_VPC, LEAF_BESIDE_THE_VPC_LOGS] {
            for axis in [Axis::Right, Axis::Down] {
                drawn_mixed(src, axis);
            }
            let (_, p) = arranged_mixed(src, Axis::Down);
            let column: Vec<&Icon> = p.layout.icons.iter().filter(|i| i.rect.x == at(&p, "lambda.report").x && i.parent == at_icon(&p, "lambda.report").parent).collect();
            assert!(column.len() >= 2, "{column:?}\n{src}");
            for (k, a) in column.iter().enumerate() {
                for b in &column[k + 1..] {
                    assert!((a.rect.y - b.rect.y).abs() >= ROW_STEP, "{} and {}\n{src}", a.id, b.id);
                }
            }
        }
    }

    /// Where a part starts and ends along the main axis, and where it starts across.
    fn along(axis: Axis, r: Rect) -> (i32, i32, i32) {
        let (u, v) = uv(axis, (r.x, r.y));
        (u, u + uv(axis, (r.w, r.h)).0, v)
    }

    // Fails if a part reached only from a part beside the VPC in its rank is put past the VPC's
    // end instead of a usual rank's distance after the part before it.
    #[test]
    fn a_part_after_one_beside_the_vpc_follows_it_at_the_usual_distance() {
        for lang in ["en", "ja"] {
            let src = ONWARD_BESIDE_THE_VPC.replace("  lang: en\n", &format!("  lang: {lang}\n"));
            let alone = format!("flows:\n  - user -> apigateway(http) -> lambda.report -> dynamodb\nset:\n  lang: {lang}\n");
            for axis in [Axis::Right, Axis::Down] {
                drawn_mixed(&src, axis);
                let (_, p) = arranged_mixed(&src, axis);
                let vpc = along(axis, p.layout.rect(super::super::azgrid::VPC_FRAME));
                let (report, table) = (along(axis, at(&p, "lambda.report")), along(axis, at(&p, "dynamodb")));
                assert!(table.0 < vpc.1, "{axis:?} dynamodb starts at {} past the VPC's end {}\n{src}", table.0, vpc.1);
                assert_eq!(table.2, report.2, "{axis:?} dynamodb is not level with lambda.report\n{src}");
                let (_, q) = arranged(&alone, axis);
                let usual = along(axis, at(&q, "dynamodb")).0 - along(axis, at(&q, "lambda.report")).0;
                assert_eq!(table.0 - report.0, usual, "{axis:?}\n{src}");
            }
        }
    }

    const ONWARD_BESIDE_THE_VPC_TWO_WAYS: &str = "flows:\n  - user -> alb -> ecs\n  - ecs -> dynamodb\n  - user -> apigateway(http) -> lambda -> aurora\n  - apigateway -> lambda.report -> dynamodb\n  - lambda.report -> s3\nset:\n  azs: 2\n  lang: ja\nchoose:\n  route.ecs.dynamodb: endpoint\n";

    // Fails if a part a part inside the VPC sends a main line to is moved before the VPC's end
    // along with the parts reached only from beside the VPC, or keeps those parts past it too.
    #[test]
    fn a_part_reached_from_inside_the_vpc_stays_past_its_end() {
        for axis in [Axis::Right, Axis::Down] {
            for src in [ONWARD_BESIDE_THE_VPC_WEB, ONWARD_BESIDE_THE_VPC_TWO_WAYS] {
                drawn_mixed(src, axis);
                let (_, p) = arranged_mixed(src, axis);
                let vpc = along(axis, p.layout.rect(super::super::azgrid::VPC_FRAME));
                let table = along(axis, at(&p, "dynamodb"));
                assert!(table.0 > vpc.1, "{axis:?} dynamodb starts at {} before the VPC's end {}\n{src}", table.0, vpc.1);
            }
            let (_, p) = arranged_mixed(ONWARD_BESIDE_THE_VPC_TWO_WAYS, axis);
            let vpc = along(axis, p.layout.rect(super::super::azgrid::VPC_FRAME));
            let (report, bucket) = (along(axis, at(&p, "lambda.report")), along(axis, at(&p, "s3")));
            assert!(bucket.0 < vpc.1 && bucket.2 == report.2, "{axis:?} s3 at {bucket:?}, lambda.report at {report:?}, the VPC ends at {}", vpc.1);
        }
    }

    fn at_icon<'a>(p: &'a Placement, id: &str) -> &'a Icon {
        p.layout.icons.iter().find(|i| i.id == id).unwrap_or_else(|| panic!("{id} is placed"))
    }

    /// Generated mixed definitions with a function outside the VPC in the VPC's rank whose main
    /// line goes on to another part outside the VPC, and a part right of the VPC only the VPC's
    /// parts are joined to.
    const BESIDE_AND_RIGHT_OF_THE_VPC: [usize; 60] = [
        6, 7, 14, 15, 16, 17, 18, 19, 24, 25, 26, 27, 28, 29, 56, 57, 64, 65, 66, 67, 68, 69, 74, 75, 76, 77, 78, 79, 227, 277, 426, 427, 434, 435, 436, 437, 438, 439,
        444, 445, 446, 447, 448, 449, 476, 477, 484, 485, 486, 487, 488, 489, 494, 495, 496, 497, 498, 499, 647, 697,
    ];

    // Fails if a picked definition lacks the shape it is picked for (a part outside the VPC in the
    // VPC's rank, reached by a main line from outside and sending one on outside, and a part right of the VPC
    // joined only to the VPC's parts), or fails to lay out or breaks a line along either axis.
    #[test]
    fn mixed_definitions_with_parts_beside_and_right_of_the_vpc_draw_cleanly() {
        let defs = super::super::generated::generated_mixed();
        let out = |a: &Area| !matches!(a, Area::Subnet(_) | Area::VpcEdge);
        for k in BESIDE_AND_RIGHT_OF_THE_VPC {
            let src = &defs[k];
            let (m, p) = arranged_mixed(src, Axis::Down);
            let area = |id: &str| &m.nodes.iter().find(|n| n.id == id).expect("an edge's end is a part").area;
            let main = |e: &&crate::model::Edge| e.rel == Rel::Main && out(area(&e.from)) && out(area(&e.to));
            let vpc = p.layout.rect(super::super::azgrid::VPC_FRAME);
            // In the VPC's rank: running down, its icon shares some height with the VPC's frame.
            let beside = |id: &str| {
                let r = at(&p, id);
                r.y < vpc.y + vpc.h && vpc.y < r.y + r.h
            };
            let onward = m.nodes.iter().filter(|n| n.area == Area::Region).any(|n| {
                m.edges.iter().filter(main).any(|e| e.to == n.id) && m.edges.iter().filter(main).any(|e| e.from == n.id) && beside(&n.id)
            });
            assert!(onward, "#{k} has no part outside the VPC in the VPC's rank whose main line goes on\n{src}");
            let right_of_the_vpc = m.nodes.iter().filter(|n| n.area == Area::Region).any(|n| {
                let ends: Vec<&crate::model::Edge> = m.edges.iter().filter(|e| e.from == n.id || e.to == n.id).collect();
                !ends.is_empty()
                    && ends.iter().all(|e| !out(area(if e.from == n.id { &e.to } else { &e.from })))
                    && at(&p, &n.id).x >= vpc.x + vpc.w
            });
            assert!(right_of_the_vpc, "#{k} has no part right of the VPC joined only to the VPC's parts\n{src}");
            for axis in [Axis::Right, Axis::Down] {
                drawn_mixed(src, axis);
            }
        }
    }

    fn icon_at_y(id: &str, x: i32, y: i32, parent: &str) -> Icon {
        Icon { id: id.to_string(), label: id.to_string(), lines: 1, rect: Rect { x, y, w: 60, h: 60 }, parent: Some(parent.to_string()) }
    }

    // Fails if parts of one column in one frame count as a row apart when their tops are closer
    // than 160, or as too close when 160 or more apart, in another column or in another frame.
    #[test]
    fn parts_closer_than_a_row_in_one_column_and_frame_are_too_close() {
        let a = icon_at_y("a", 700, 400, "region");
        for (b, close) in [
            (icon_at_y("b", 700, 559, "region"), true),
            (icon_at_y("b", 700, 560, "region"), false),
            (icon_at_y("b", 700, 561, "region"), false),
            (icon_at_y("b", 700, 241, "region"), true),
            (icon_at_y("b", 700, 240, "region"), false),
            (icon_at_y("b", 701, 459, "region"), false),
            (icon_at_y("b", 700, 459, "cloud"), false),
        ] {
            assert_eq!(rows_too_close(&a, &b), close, "{b:?}");
            assert_eq!(rows_too_close(&b, &a), close, "{b:?}");
        }
    }

    // Fails if a main line between two parts of the Region outside the VPC, whose plain draft
    // turns across before the VPC's end, is drafted or drawn through the VPC.
    #[test]
    fn a_line_between_parts_outside_the_vpc_turns_past_it() {
        let r = Rules::builtin();
        for axis in [Axis::Right, Axis::Down] {
            let (m, p) = arranged_mixed(ONWARD_BESIDE_THE_VPC_WEB, axis);
            let vpc = p.layout.rect(super::super::azgrid::VPC_FRAME);
            let k = m.edges.iter().position(|e| e.from == "lambda.report" && e.to == "dynamodb").expect("the line is in the model");
            let draft = p.lines.iter().find(|l| l.edge == k).expect("the line has a draft");
            let (a, b) = (at(&p, "lambda.report"), at(&p, "dynamodb"));
            let ((_, av), (_, bv)) = (uv(axis, (a.x + ICON / 2, a.y + ICON / 2)), uv(axis, (b.x + ICON / 2, b.y + ICON / 2)));
            let t = match (axis, &draft.bend) {
                (Axis::Right, Bend::AtX(t)) | (Axis::Down, Bend::AtY(t)) => *t,
                other => panic!("{other:?}"),
            };
            let pts = [(uv(axis, (right(a), bottom(a))).0, av), (t, av), (t, bv), (uv(axis, (b.x, b.y)).0, bv)].map(|q| uv(axis, q));
            assert!(!pts.windows(2).any(|s| crate::path::hits(s[0], s[1], vpc)), "{axis:?} draft {pts:?} runs through the VPC {vpc:?}");
            let (wires, _) = route_planned(&m, &p.layout, &r, &p.lines);
            let w = wires.iter().find(|w| w.id == "edge-lambda.report-to-dynamodb").expect("drawn");
            let mut path = vec![port_point(p.layout.rect(&w.from), w.exit)];
            path.extend(w.points.iter().copied());
            path.push(port_point(p.layout.rect(&w.to), w.entry));
            assert!(!path.windows(2).any(|s| crate::path::hits(s[0], s[1], vpc)), "{axis:?} drawn {path:?} runs through the VPC {vpc:?}");
        }
    }

    const TWO_ORIGINS: &str = "flows:
  - cloudtrail -> cloudwatch -> sns
  - cloudtrail -> s3
  - config -> ssm
set:
  lang: en
";

    // Fails if, for two flows that start side by side and share no part, a part lies level across
    // with a part of the rank before that is not one of its predecessors while that part has
    // targets of its own, or two drawn main lines that share no end meet or cross anywhere, along
    // either axis. Laid out one rank at a time, S3 (CloudTrail's) was put under Config, and the
    // line to it and Config's line to Systems Manager turned at one point.
    #[test]
    fn flows_from_two_origins_keep_their_lines_apart() {
        for axis in [Axis::Down, Axis::Right] {
            let (m, p) = arranged(TWO_ORIGINS, axis);
            let across = |id: &str| {
                let a = at(&p, id);
                uv(axis, (a.x + ICON / 2, a.y + ICON / 2))
            };
            let mains: Vec<&crate::model::Edge> = m.edges.iter().filter(|e| e.rel == Rel::Main).collect();
            for e in &mains {
                let (bu, bv) = across(&e.to);
                for other in &mains {
                    let (ou, ov) = across(&other.from);
                    let feeds = mains.iter().any(|x| x.from == other.from && x.to == e.to);
                    if !feeds && ov == bv && ou < bu && ou > across(&e.from).0 - 1 {
                        panic!("{axis:?}: {} lies level with {}, which does not feed it", e.to, other.from);
                    }
                }
            }
            assert_unshared_main_lines_apart(&m, &p, axis);
        }
    }

    /// No two drawn main lines that share no end meet or cross anywhere.
    fn assert_unshared_main_lines_apart(m: &Model, p: &Placement, axis: Axis) {
        let (wires, _) = route_planned(m, &p.layout, &Rules::builtin(), &p.lines);
        let path = |w: &Wire| {
            let mut v = vec![port_point(p.layout.rect(&w.from), w.exit)];
            v.extend(w.points.iter().copied());
            v.push(port_point(p.layout.rect(&w.to), w.entry));
            v
        };
        let main: Vec<&Wire> = wires.iter().filter(|w| w.style == crate::route::WireStyle::Main).collect();
        for (k, a) in main.iter().enumerate() {
            for b in &main[k + 1..] {
                if [&a.from, &a.to].iter().any(|x| **x == b.from || **x == b.to) {
                    continue;
                }
                let (pa, pb) = (path(a), path(b));
                for s in pa.windows(2) {
                    for t in pb.windows(2) {
                        let box_of = |q: &[(i32, i32)]| (q[0].0.min(q[1].0), q[0].0.max(q[1].0), q[0].1.min(q[1].1), q[0].1.max(q[1].1));
                        let ((sx0, sx1, sy0, sy1), (tx0, tx1, ty0, ty1)) = (box_of(s), box_of(t));
                        let meet = sx0 <= tx1 && tx0 <= sx1 && sy0 <= ty1 && ty0 <= sy1;
                        assert!(!meet, "{axis:?}: {} and {} meet: {s:?} {t:?}", a.id, b.id);
                    }
                }
            }
        }
    }

    /// The room left and right of the Region's parts, icons and names, inside the Region frame.
    fn region_margins(p: &Placement) -> (i32, i32) {
        let region = at(p, "region");
        let inner: Vec<&Icon> = p.layout.icons.iter().filter(|i| inside_rect(region, i.rect)).collect();
        let left = inner.iter().map(|i| i.rect.x.min(name_area(i).x)).min().expect("parts in the Region");
        let end = inner.iter().map(|i| extent(i).0).max().expect("parts in the Region");
        (left - region.x, right(region) - end)
    }
    fn inside_rect(outer: Rect, r: Rect) -> bool {
        r.x >= outer.x && r.y >= outer.y && r.x + r.w <= outer.x + outer.w && r.y + r.h <= outer.y + outer.h
    }

    // Fails if the Region's parts keep the room for lines coming in though no line comes into the
    // Region: down the page, the parts of these flows sat clear of the Region's name, 210 pixels
    // in from the left, and the frame closed 50 pixels past them on the right; across the page,
    // they sat 110 pixels in.
    #[test]
    fn a_region_no_line_comes_into_keeps_even_room_on_both_sides() {
        let flows = [
            "flows:\n  - cloudtrail -> cloudwatch -> sns\n",
            "flows:\n  - cloudtrail -> cloudwatch -> sns\n  - cloudtrail -> s3\n  - config -> ssm\n",
            "flows:\n  - guardduty -> securityhub -> eventbridge -> sns\n  - inspector -> securityhub\n  - config -> securityhub\n",
            "flows:\n  - guardduty -> securityhub -> eventbridge -> stepfunctions\n  - stepfunctions -> ssm\n  - stepfunctions -> sns\n",
        ];
        for flow in flows {
            for lang in ["ja", "en"] {
                for axis in [Axis::Down, Axis::Right] {
                    let src = format!("{flow}set:\n  lang: {lang}\n");
                    let (_, p) = arranged(&src, axis);
                    let (left, right) = region_margins(&p);
                    assert!((left - right).abs() <= 10, "{axis:?} {lang}: left {left}, right {right}\n{src}");
                }
            }
        }
    }

    // Fails if a line coming into the Region from above runs through the Region's name: the
    // Region's first parts keep clear of the name when a line comes in.
    #[test]
    fn a_region_a_line_comes_into_keeps_its_parts_clear_of_its_name() {
        let (_, p) = arranged(TARGET, Axis::Down);
        let region = p.layout.frames.iter().find(|f| f.id == "region").unwrap();
        let name = frame_name_box(region);
        let first = p.layout.icons.iter().filter(|i| inside_rect(region.rect, i.rect)).map(|i| i.rect.x + ICON / 2).min().unwrap();
        assert!(first >= right(name) + FRAME_ROOM, "first centre {first}, name ends at {}", right(name));
    }

    // Fails if the Region's parts keep clear of the Region's name when the lines coming in reach
    // only parts past it: here the users' line comes into API Gateway in the middle of the
    // Region, and the parts sat 210 pixels in from the left rather than the usual 110.
    #[test]
    fn a_region_whose_lines_come_in_past_its_name_keeps_its_usual_room() {
        let src = "flows:
  - user -> apigateway(http) -> lambda.orders -> dynamodb.orders
  - apigateway -> lambda.users -> dynamodb.users
  - apigateway -> lambda.search -> dynamodb.orders
  - lambda.orders ~> cloudwatch
  - lambda.users ~> cloudwatch
  - lambda.search ~> cloudwatch
set:
  lang: ja
";
        let (_, p) = arranged(src, Axis::Down);
        let region = p.layout.frames.iter().find(|f| f.id == "region").unwrap();
        let left = p.layout.icons.iter().filter(|i| inside_rect(region.rect, i.rect)).map(|i| i.rect.x - region.rect.x).min().unwrap();
        assert_eq!(left, REGION_LEFT);
        let api = at(&p, "apigateway").x + ICON / 2;
        assert!(api >= right(frame_name_box(region)) + FRAME_ROOM, "API Gateway centre {api}, name ends at {}", right(frame_name_box(region)));
    }

    const PARENTS_IN_ANOTHER_ORDER: &str = "flows:
  - eventbridge.a -> stepfunctions.a1
  - eventbridge.a -> stepfunctions.a2
  - eventbridge.b -> stepfunctions.b
  - stepfunctions.b -> sns.z
  - stepfunctions.a1 -> sns.x
  - stepfunctions.a2 -> sns.y
";

    // Fails if the parts of a rank keep the order the definition wrote them in when their
    // predecessors come in another order: here sns.z came first, under stepfunctions.a1, and the
    // lines from a1 and a2 crossed the line from b.
    #[test]
    fn parts_in_a_rank_follow_the_order_of_their_predecessors() {
        for axis in [Axis::Down, Axis::Right] {
            let (m, p) = arranged(PARENTS_IN_ANOTHER_ORDER, axis);
            let across = |id: &str| {
                let a = at(&p, id);
                uv(axis, (a.x, a.y)).1
            };
            let order = ["sns.x", "sns.y", "sns.z"].map(across);
            assert!(order[0] < order[1] && order[1] < order[2], "{axis:?}: sns.x, sns.y, sns.z across at {order:?}");
            assert_unshared_main_lines_apart(&m, &p, axis);
        }
    }

    // Fails if a part whose target comes before another part's own target in the next rank is
    // moved past it: each move then moves the other target past its own again, and the
    // drawing grew to tens of thousands of pixels.
    #[test]
    fn a_part_whose_target_comes_first_is_not_moved_past_the_others() {
        let src = "flows:
  - user -> apigateway(http) -> lambda.f1
  - apigateway -> lambda.f2
  - lambda.f1 -> dynamodb.t1
  - lambda.f2 -> dynamodb.t2
  - user -> cloudfront -> s3.web
  - lambda.f1 -> s3.web
set:
  lang: ja
";
        for axis in [Axis::Down, Axis::Right] {
            let (_, p) = arranged(src, axis);
            let region = at(&p, "region");
            assert!(region.w < 2000 && region.h < 2000, "{axis:?}: {region:?}");
        }
    }

    // Fails if moving a part past the targets before it moves a later part of its rank whose
    // target comes before those targets in the next rank, which moves them on and the first part
    // again: the Region frame of these eight parts grew to 22,325 pixels.
    #[test]
    fn moves_that_push_each_other_through_the_ranks_do_not_spread_the_drawing() {
        let src = "flows:
  - eventbridge.a -> stepfunctions.a1
  - eventbridge.a -> stepfunctions.a2
  - eventbridge.b -> stepfunctions.b
  - stepfunctions.b -> sns.z
  - stepfunctions.a1 -> sns.x
  - stepfunctions.a2 -> sns.y
";
        for axis in [Axis::Down, Axis::Right] {
            let (_, p) = arranged(src, axis);
            let region = at(&p, "region");
            assert!(region.w < 2000 && region.h < 2000, "{axis:?}: {region:?}");
        }
    }

    // Fails if moves that have not settled within their bound are kept, or if those of the two
    // flows from CloudTrail and Config do not settle once one move is allowed and checked.
    #[test]
    fn moves_that_do_not_settle_are_dropped() {
        let r = Rules::builtin();
        let (m, _) = crate::inference::infer(&crate::def::parse(TWO_ORIGINS).unwrap(), &r);
        let m = m.unwrap();
        let f = flow(&m);
        let rs = cross_step(&m, &r, Axis::Right);
        assert_eq!(least_rows(&f, rs, 1), None);
        let settled = least_rows(&f, rs, 2).expect("one move, then a round that moves nothing");
        assert_eq!(settled.iter().filter(|l| l.is_some()).count(), 1);
    }

    const USERS_TO_FOUR_PARTS: &str = "flows:
  - user -> cloudfront -> s3
  - user -> cognito
  - user -> s3.uploads
  - user -> athena -> s3.logs
  - cloudtrail -> s3.logs
set:
  lang: ja
";

    // The web stack with the users signing in, uploading and querying logs beside it.
    const USERS_TO_THE_WEB_AND_THREE_PARTS: &str = "flows:
  - user -> cloudfront -> alb -> ecs -> aurora
  - user -> cognito
  - user -> s3.uploads
  - user -> athena -> s3.logs
set:
  lang: ja
";

    // The same beside an API, with a distribution in front of a bucket.
    const USERS_TO_THE_API_AND_FOUR_PARTS: &str = "flows:
  - user -> apigateway(http) -> lambda -> dynamodb
  - user -> cognito
  - user -> s3.uploads
  - user -> athena -> s3.logs
  - cloudtrail -> s3.logs
  - user -> cloudfront -> s3
set:
  lang: ja
";

    /// The drawn points of a line, from its exit to its entry.
    fn drawn_path(w: &Wire, l: &crate::layout::Layout) -> Vec<(i32, i32)> {
        let mut v = vec![port_point(l.rect(&w.from), w.exit)];
        v.extend(w.points.iter().copied());
        v.push(port_point(l.rect(&w.to), w.entry));
        v
    }

    // A distribution in front of the web stack, two buckets and an API: four lines leave the
    // bottom of CloudFront, or its right side left to right.
    const CLOUDFRONT_TO_FOUR_PARTS: &str = "flows:
  - user -> cloudfront -> alb -> ecs -> aurora
  - cloudfront -> s3
  - cloudfront -> s3.media
  - cloudfront -> apigateway(http) -> lambda -> dynamodb
set:
  lang: ja
";

    /// The crossings of the lines leaving `from` with each other, each named with the segments
    /// that cross; with `turning`, only of lines that turn on their way. Also the notes that name
    /// a line, and how many lines leave `from`.
    fn crossings_from(src: &str, axis: Axis, from: &str, turning: bool) -> (Vec<String>, Vec<String>, usize) {
        let (m, p) = arranged(src, axis);
        let (wires, notes) = route_planned(&m, &p.layout, &Rules::builtin(), &p.lines);
        let leaving: Vec<&Wire> = wires.iter().filter(|w| w.from == from).collect();
        let turns = |w: &Wire| !turning || !crate::path::corners(&drawn_path(w, &p.layout)).is_empty();
        let mut found = Vec::new();
        for (k, a) in leaving.iter().enumerate() {
            for b in leaving[k + 1..].iter().filter(|b| turns(a) && turns(b)) {
                for s in drawn_path(a, &p.layout).windows(2) {
                    for t in drawn_path(b, &p.layout).windows(2) {
                        if segments_cross(s, t) {
                            found.push(format!("{axis:?}: {} crosses {}: {s:?} {t:?}", a.id, b.id));
                        }
                    }
                }
            }
        }
        let named: Vec<String> = notes.into_iter().filter(|n| n.contains("edge-")).collect();
        (found, named, leaving.len())
    }

    /// Asserts that no two lines leaving `from` cross (with `turning`, no two that turn) and that
    /// no note names a line, top to bottom and left to right, after looking at both.
    fn assert_no_crossing_from(src: &str, from: &str, lines: usize, turning: bool) {
        let faults = crossing_faults_from(src, from, lines, turning);
        assert!(faults.is_empty(), "{faults:#?}");
    }

    /// The crossings and notes `assert_no_crossing_from` looks for, top to bottom and left to right.
    fn crossing_faults_from(src: &str, from: &str, lines: usize, turning: bool) -> Vec<String> {
        let mut faults = Vec::new();
        for axis in [Axis::Down, Axis::Right] {
            let (found, named, n) = crossings_from(src, axis, from, turning);
            assert_eq!(n, lines, "{axis:?}: lines leaving {from}");
            faults.extend(found);
            faults.extend(named.into_iter().map(|n| format!("{axis:?}: {n}")));
        }
        faults
    }

    // Fails if two lines leaving one side of the users cross: down the page, the line to S3
    // uploads turned right first, above the Region, and the line to Athena, leaving right of it,
    // came down through it to turn lower. Laying the outer lines first is enough here, so this
    // holds whether or not the turns are shared out again.
    #[test]
    fn lines_leaving_one_side_of_the_users_do_not_cross() {
        assert_no_crossing_from(USERS_TO_FOUR_PARTS, "user", 4, false);
    }

    // Fails if, of the lines leaving the users beside the web stack or an API, an inner line
    // turns farther from the users than an outer line turning the same way and crosses it: down
    // the page, the line to Cognito turned above the line to S3 uploads leaving right of it, and
    // in English the line to S3 uploads turned above the line to Athena. Left to right, the lines
    // to the web stack's neighbours crossed when laid in the order they are planned.
    // These tests look only at the finished lines. Top to bottom, all three definitions need the
    // turns shared out again (`nest_turns`): laying the outer lines first leaves the crossings
    // above. Left to right, laying the outer lines first is enough, so those diagrams hold
    // whether or not the turns are shared out again. Sharing out alone is held, on every side and
    // both ways, by `of_two_lines_leaving_a_part_outside_the_region_the_outer_turns_nearer_it` in
    // the router's tests.
    #[test]
    fn lines_from_the_users_beside_a_web_stack_or_an_api_do_not_cross() {
        let english = USERS_TO_THE_WEB_AND_THREE_PARTS.replace("lang: ja", "lang: en");
        let defs = [("web, ja", USERS_TO_THE_WEB_AND_THREE_PARTS, 4), ("web, en", english.as_str(), 4), ("api", USERS_TO_THE_API_AND_FOUR_PARTS, 5)];
        let faults: Vec<String> = defs.iter().flat_map(|&(name, src, lines)| crossing_faults_from(src, "user", lines, false).into_iter().map(move |f| format!("{name}: {f}"))).collect();
        assert!(faults.is_empty(), "{faults:#?}");
    }

    // Fails if two lines leaving CloudFront that both turn cross: the lines to the buckets and
    // the API turned in the order they were laid. The line straight on to the web stack is left
    // out: it leaves the middle of the side, and lines leaving left of it to parts on its right
    // cross it whatever their turns. Laying the outer lines first leaves a crossing in both
    // directions, so this holds the turns shared out again for a real layout.
    #[test]
    fn turning_lines_from_cloudfront_do_not_cross() {
        assert_no_crossing_from(CLOUDFRONT_TO_FOUR_PARTS, "cloudfront", 4, true);
    }

    /// Whether a level and an upright segment cross inside both.
    fn segments_cross(s: &[(i32, i32)], t: &[(i32, i32)]) -> bool {
        let level = |q: &[(i32, i32)]| q[0].1 == q[1].1 && q[0].0 != q[1].0;
        let (h, v) = if level(s) { (s, t) } else if level(t) { (t, s) } else { return false };
        if v[0].0 != v[1].0 || level(v) {
            return false;
        }
        let (x0, x1) = (h[0].0.min(h[1].0), h[0].0.max(h[1].0));
        let (y0, y1) = (v[0].1.min(v[1].1), v[0].1.max(v[1].1));
        x0 < v[0].0 && v[0].0 < x1 && y0 < h[0].1 && h[0].1 < y1
    }
}
