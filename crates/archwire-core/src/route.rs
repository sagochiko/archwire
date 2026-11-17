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

use crate::audit::audit;
use crate::def::Rel;
use crate::ids::edge_id;
use crate::labels::{path_length, LabelSpot};
use crate::layout::{Icon, Layout, Rect};
use crate::lines::{gather, label_parts, Line};
use crate::measure::Measure;
use crate::model::{Axis, Model, Node};
use crate::path::{outward, reverse, Draft, Fence, Pt, Scene};
use crate::rules::{DerivedRole, Rules, Scope};

pub use crate::grid::route;

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
    /// The representative semantic edge (an index into `Model.edges`): the draft it was drawn from.
    pub rep: usize,
}

/// How a planned line bends between its exit and its entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bend {
    /// Exit and entry face each other on one line.
    Straight,
    /// Out, along a vertical channel at this x, then in.
    AtX(i32),
    /// Out, along a horizontal channel at this y, then in.
    AtY(i32),
    /// One bend where the exit's line meets the entry's line.
    Elbow,
    /// Through these waypoints, in order, between the exit and the entry.
    Points(Vec<(i32, i32)>),
}

/// The inner part of a line that crosses a frame's side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Through {
    /// On the frame's side; the line crosses the side here, at a right angle.
    pub gate: (i32, i32),
    /// The points between the gate and the inner end's port, in the line's direction.
    pub inner: Vec<(i32, i32)>,
    /// The inner end is the source: the line leaves the frame.
    pub inner_is_start: bool,
}

/// How much of the draft the router keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keep {
    /// Drawn exactly as planned: both ends' sides and positions and every waypoint. Never searched again.
    Whole,
    /// The inner part (the inner end's side and position, `inner`, the gate) is kept; only the outer part may be searched.
    Inner(Through),
    /// The draft is a first try; the router may search, and move to other sides.
    Free,
}

/// A stretch of a line that must lie on a band: horizontal at y = `at` from x = `from` to x = `to`,
/// or vertical at x = `at` from y = `from` to y = `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Track {
    pub horizontal: bool,
    pub from: i32,
    pub to: i32,
    pub at: i32,
}

/// The draft of the line for one semantic edge: the sides, bend and positions it is tried with,
/// and how much of it the router keeps.
#[derive(Debug, Clone, PartialEq)]
pub struct LinePlan {
    /// The semantic edge (an index into `Model.edges`) the draft is for.
    pub edge: usize,
    pub from: String,
    pub to: String,
    pub exit: Side,
    pub entry: Side,
    pub bend: Bend,
    /// Along the side, 0 to 100; None lets the router share the side out.
    pub exit_pos: Option<u8>,
    pub entry_pos: Option<u8>,
    pub keep: Keep,
    /// The stretches that must lie on their bands; empty for none.
    pub tracks: Vec<Track>,
    /// The axis of the arrangement the line belongs to.
    pub axis: Axis,
}

impl LinePlan {
    /// The positions of the ends the draft fixes: both for a line kept whole, the inner end for a
    /// line kept inside, none for a free line. A fixed end without a position sits in the middle.
    pub(crate) fn fixed(&self) -> (Option<u8>, Option<u8>) {
        let (exit, entry) = (self.exit_pos.unwrap_or(50), self.entry_pos.unwrap_or(50));
        match &self.keep {
            Keep::Whole => (Some(exit), Some(entry)),
            Keep::Inner(t) if t.inner_is_start => (Some(exit), None),
            Keep::Inner(_) => (None, Some(entry)),
            Keep::Free => (None, None),
        }
    }
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
pub(crate) const LABEL_FIRST: i32 = 24;
pub(crate) const LABEL_LINE: i32 = 14;

/// Ends closer in height than this count as level: the label goes above the line, centred.
pub(crate) const LEVEL: i32 = 30;

/// Lines are placed in this order, so the shapes the diagram is read by get their channels first.
pub(crate) const RANK_MAIN: u8 = 0;

pub(crate) const RANK_SIDE: u8 = 2;
pub(crate) const RANK_AUTO: u8 = 3;

pub(crate) struct Ctx<'a> {
    pub(crate) model: &'a Model,
    pub(crate) layout: &'a Layout,
    pub(crate) rules: &'a Rules,
    pub(crate) scene: Scene<'a>,
    pub(crate) measure: Measure,
}

impl Ctx<'_> {
    /// True if the part is the gateway traffic enters the VPC by, as the rules data marks it.
    fn enters_the_vpc(&self, id: &str) -> bool {
        self.rules.service(&self.node(id).service).and_then(|s| s.derived_role) == Some(DerivedRole::EntryGateway)
    }
}

/// Like `route`, with every line's first try taken from `plans`. A line with no plan gets the
/// usual `candidates`; `plan()` is never called. Reads only frames and icons the layout has.
pub fn route_planned(model: &Model, layout: &Layout, rules: &Rules, plans: &[LinePlan]) -> (Vec<Wire>, Vec<String>) {
    let mut scene = Scene::new(layout, Measure::Flow);
    scene.fence = Fence::of(model, layout);
    let ctx = Ctx { model, layout, rules, scene, measure: Measure::Flow };
    let (specs, notes) = gather(model, layout, rules);
    // A line takes the draft of the semantic edge it stands for.
    let mut drafts: Vec<Option<Draft>> = specs
        .iter()
        .map(|s| {
            let p = plans.iter().find(|p| p.edge == s.rep)?;
            Some(Draft::from_plan(p, if s.rel == Rel::Main { RANK_MAIN } else { RANK_SIDE }))
        })
        .collect();
    let rest: Vec<usize> = (0..specs.len()).filter(|&i| drafts[i].is_none()).collect();
    ctx.plan_rest(&specs, &rest, &mut drafts);
    let drafts: Vec<Draft> = drafts.into_iter().map(|d| d.expect("every line planned")).collect();
    // A side line drawn as planned between the middles of its sides (or the spots its draft
    // fixes or writes), past
    // every icon and name, is a barrier the other lines go round.
    let barriers: Vec<(String, String, Vec<Pt>)> = specs
        .iter()
        .zip(&drafts)
        .filter(|(s, _)| s.rel != Rel::Main)
        .filter_map(|(s, d)| {
            let exit = ctx.port(&s.from, d.exit, d.exit_fixed.or(d.exit_want).unwrap_or(50));
            let entry = ctx.port(&s.to, d.entry, d.entry_fixed.or(d.entry_want).unwrap_or(50));
            let path = ctx.draft_path(s, d, (exit, entry))?;
            let (out, inward) = (outward(d.exit), reverse(outward(d.entry)));
            ctx.scene.check(&path, &s.from, &s.to, out, inward, &[])?;
            Some((s.from.clone(), s.to.clone(), path))
        })
        .collect();
    let mut ctx = ctx;
    ctx.scene.barriers = barriers;
    draw(&ctx, &specs, &drafts, plans, notes)
}

/// Shares out the ports, lays every line, places the labels and runs the final check, which
/// also looks at how each line keeps its draft in `plans` (none on the path without drafts).
pub(crate) fn draw(ctx: &Ctx, specs: &[Line], drafts: &[Draft], plans: &[LinePlan], mut notes: Vec<String>) -> (Vec<Wire>, Vec<String>) {
    let (model, layout, rules, lang) = (ctx.model, ctx.layout, ctx.rules, ctx.model.lang);
    let mut ports = ctx.share_out(specs, drafts);
    ctx.straighten_level_lines(specs, drafts, &mut ports);

    let mut order: Vec<usize> = (0..specs.len()).collect();
    order.sort_by_key(|&i| (drafts[i].rank, i));
    let mut paths: Vec<Vec<Pt>> = vec![Vec::new(); specs.len()];
    let mut placed: Vec<Vec<Pt>> = Vec::new();
    for i in order {
        let (s, d) = (&specs[i], &drafts[i]);
        let unrouted = || rules.text("note.unrouted_line", lang).replace("{line}", &edge_id(&s.from, &s.to));
        let path = match ctx.lay(s, d, ports[i], &placed) {
            Ok(p) => p,
            // A kept draft is never moved to other sides: it is drawn as planned and named.
            Err(best) if d.keep != Keep::Free => {
                notes.push(unrouted());
                best
            }
            Err(best) => match ctx.on_free_sides(specs, i, &ports, &placed) {
                Some((p, moved)) => {
                    ports[i] = moved;
                    p
                }
                None => {
                    notes.push(unrouted());
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
        let near_source = ctx.enters_the_vpc(&s.from);
        let axis = drafts[i].axis;
        // Running right, ends level in height put the label above the line, else beside it;
        // running down, ends level left to right put it beside the line, else above it.
        let level = match axis {
            Axis::Right => (fr.y - tr.y).abs() <= LEVEL,
            Axis::Down => (fr.x - tr.x).abs() > LEVEL,
        };
        // A line of no length was never laid, and its note already names it: there is no spot along it.
        if path_length(&paths[i]) == 0 {
            let text = label_parts(s, model, rules).join(" ");
            spots[i] = Some(LabelSpot { text, above: level, flip: false, at: None, area: None, clear: true });
            continue;
        }
        let spot = ctx.place_label(&label_parts(s, model, rules), (&paths, i), level, near_source, axis, &areas);
        areas.extend(spot.area);
        spots[i] = Some(spot);
    }
    let mut wires = Vec::new();
    for (((s, (exit, entry)), path), spot) in specs.iter().zip(ports).zip(&paths).zip(spots) {
        let spot = spot.expect("every line has a label spot");
        let near_source = ctx.enters_the_vpc(&s.from);
        if !spot.clear {
            notes.push(rules.text("note.unplaced_label", lang).replace("{line}", &edge_id(&s.from, &s.to)));
        }
        wires.push(Wire {
            id: edge_id(&s.from, &s.to),
            from: s.from.clone(),
            to: s.to.clone(),
            style: match (s.hole.is_some(), s.rel) {
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
            rep: s.rep,
        });
    }
    // Whatever the router believed, the finished lines are checked once more on their own terms.
    for id in audit(model, layout, &wires, plans, ctx.measure) {
        let note = rules.text("note.unrouted_line", lang).replace("{line}", &id);
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
    (wires, notes)
}

impl<'a> Ctx<'a> {
    pub(crate) fn node(&self, id: &str) -> &'a Node {
        self.model.nodes.iter().find(|n| n.id == id).expect("edge ends are nodes")
    }

    pub(crate) fn scope(&self, n: &Node) -> Scope {
        self.rules.service(&n.service).map(|s| s.scope).expect("model services are in the rules")
    }

    pub(crate) fn icon(&self, id: &str) -> &'a Icon {
        self.layout.icons.iter().find(|i| i.id == id).expect("every node has an icon")
    }

    fn offset(&self, id: &str, side: Side) -> i32 {
        match side {
            Side::Bottom => LABEL_FIRST + LABEL_LINE * (self.icon(id).lines as i32 - 1),
            _ => 0,
        }
    }

    pub(crate) fn port(&self, id: &str, side: Side, pos: u8) -> Port {
        Port { side, pos, offset: self.offset(id, side) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{audit_wire_label, audit_wire_text_width};
    use crate::labels::{label_distance, point_along};
    use crate::layout::{Frame, FrameKind};
    use crate::measure::{frame_name_box, label_area, name_area, wire_label_area, wire_text_width};
    use crate::path::{hits, opposite, overlaps, shared};
    use crate::test_inputs::{CROWDED_SIDE_EN, CROWDED_S3_JA};

    /// The model, layout, wires and notes the drawing path gives a definition along `axis`.
    fn planned(src: &str, axis: Axis) -> (Model, Layout, Vec<Wire>, Vec<String>) {
        let r = Rules::builtin();
        let m = crate::inference::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap();
        let p = crate::arrange::arrange(&m, &r, axis).unwrap_or_else(|e| panic!("{axis:?}: {e:?}\n{src}"));
        let (w, n) = route_planned(&m, &p.layout, &r, &p.lines);
        (m, p.layout, w, n)
    }

    /// Like `planned`, along the direction the definition writes, else the default one.
    fn compiled(src: &str) -> (Model, Layout, Vec<Wire>, Vec<String>) {
        let r = Rules::builtin();
        let m = crate::inference::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap();
        planned(src, m.direction.unwrap_or_else(|| crate::arrange::default_axis(&m)))
    }

    const AXES: [Axis; 2] = [Axis::Down, Axis::Right];

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

    /// True if the drawn line runs along one straight line, waypoints and all.
    fn straight(x: &Wire, l: &Layout) -> bool {
        let p = full_path(x, l);
        p.iter().all(|q| q.0 == p[0].0) || p.iter().all(|q| q.1 == p[0].1)
    }

    fn both() -> [String; 2] { [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n")] }

    #[test]
    fn every_semantic_edge_is_represented_exactly_once() {
        for src in both() { for axis in AXES {
            let (m, _, w, _) = planned(&src, axis);
            let mut drawn: Vec<(String, String)> = w.iter().flat_map(|x| x.represents.clone()).collect();
            let mut want: Vec<(String, String)> = m.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
            drawn.sort(); want.sort();
            assert_eq!(drawn, want, "{axis:?}");
        } }
    }

    #[test]
    fn bottom_ports_clear_the_label() {
        let mut from_alb = 0;
        for src in both() { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
            let lines = |id: &str| l.icons.iter().find(|i| i.id == id).unwrap().lines as i32;
            for x in &w {
                for (id, p) in [(&x.from, x.exit), (&x.to, x.entry)] {
                    if p.side == Side::Bottom {
                        assert_eq!(p.offset, LABEL_FIRST + LABEL_LINE * (lines(id) - 1), "{} at {id} {axis:?}", x.id);
                    }
                }
                from_alb += usize::from(x.from == "alb" && x.exit.side == Side::Bottom);
            }
        } }
        assert!(from_alb > 0, "no line leaves the load balancer by its bottom side");
    }

    #[test]
    fn styles_follow_the_relation_and_holes() {
        for axis in AXES {
            let (_, _, w, _) = planned("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\n", axis);
            let style = |to: &str| w.iter().find(|x| x.to == to).unwrap().style;
            assert_eq!(style("s3"), WireStyle::Hole);
            assert_eq!(style("cloudwatch"), WireStyle::Aux);
            assert_eq!(w.iter().find(|x| x.from == "waf").unwrap().style, WireStyle::Assoc);
            assert_eq!(w.iter().find(|x| x.to == "s3").unwrap().label, "Route undecided");
            assert_eq!(w.iter().find(|x| x.id == "edge-aurora-1a-to-aurora-1c").unwrap().style, WireStyle::Aux);
        }
    }

    #[test]
    fn three_azs_merge_cross_az_wires_and_say_so() {
        for axis in AXES {
            let (_, _, w, notes) = planned(&format!("{WEB2}set:\n  azs: 3\n"), axis);
            assert!(w.iter().all(|x| !(x.from.starts_with("ecs-") && x.to.starts_with("aurora-") && x.from[4..] != x.to[7..])));
            assert!(w.iter().find(|x| x.id == "edge-ecs-1a-to-aurora-1a").unwrap().represents.len() >= 3);
            assert!(!notes.is_empty());
        }
    }

    #[test]
    fn no_wire_segment_crosses_an_icon_it_is_not_attached_to() {
        for src in both() { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
            for x in &w {
                let pts = full_path(x, &l);
                for seg in pts.windows(2) { for i in &l.icons {
                    if i.id == x.from || i.id == x.to { continue; }
                    assert!(!segment_hits_rect(seg[0], seg[1], i.rect), "{} crosses {} {axis:?}", x.id, i.id);
                } }
            }
        } }
    }

    #[test]
    fn no_wire_segment_crosses_a_label_except_its_own_ends() {
        for src in both() { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
            for x in &w {
                let pts = full_path(x, &l);
                let last = pts.len() - 2;
                for (k, seg) in pts.windows(2).enumerate() { for i in &l.icons {
                    if (k == 0 && i.id == x.from) || (k == last && i.id == x.to) { continue; }
                    assert!(!segment_hits_rect(seg[0], seg[1], label_rect(i)), "{} crosses the label of {} {axis:?}", x.id, i.id);
                } }
            }
        } }
    }

    #[test]
    fn no_two_wires_share_a_segment() {
        for src in both() { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
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
                assert!(shared <= 1, "{} and {} share {shared} px {axis:?}", w[a.0].id, w[b.0].id);
            } }
        } }
    }

    #[test]
    fn every_wire_is_orthogonal_and_leaves_and_enters_outward() {
        for src in both() { for axis in AXES {
            let (_, l, w, notes) = planned(&src, axis);
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
                assert!(out && inward, "{} runs along its icon's side {axis:?}", x.id);
            }
        } }
    }

    #[test]
    fn lines_to_regional_services_merge_and_say_all_azs() {
        for axis in AXES {
            let (_, _, w, _) = planned(WEB2, axis);
            let cw: Vec<&Wire> = w.iter().filter(|x| x.to == "cloudwatch").collect();
            assert_eq!(cw.len(), 1, "{axis:?}");
            assert_eq!((cw[0].label.replace('\n', " ").as_str(), cw[0].style), ("Logs and metrics (all AZs)", WireStyle::Aux), "{axis:?}");
            assert_eq!(cw[0].represents, [("ecs-1a".to_string(), "cloudwatch".to_string()), ("ecs-1c".to_string(), "cloudwatch".to_string())]);
            let (_, _, ja, _) = planned(&format!("{WEB2}set:\n  lang: ja\n"), axis);
            assert!(ja.iter().find(|x| x.to == "cloudwatch").unwrap().label.contains("（全 AZ から）"), "{axis:?}");
            let (_, _, w3, _) = planned(&format!("{WEB2}set:\n  azs: 3\n"), axis);
            let writer = w3.iter().find(|x| x.id == "edge-ecs-1a-to-aurora-1a").unwrap();
            assert_eq!(writer.label.replace('\n', " "), "Read/Write (all AZs)", "{axis:?}");
        }
    }

    #[test]
    fn main_column_is_straight_and_only_the_entry_gateway_line_has_its_label_near_its_source() {
        for axis in AXES {
            let (_, l, w, _) = planned(WEB2, axis);
            let get = |id: &str| w.iter().find(|x| x.id == id).unwrap_or_else(|| panic!("{id}"));
            for id in ["edge-user-to-cloudfront", "edge-cloudfront-to-igw", "edge-igw-to-alb"] {
                assert!(straight(get(id), &l), "{id} {axis:?}: {:?}", get(id).points);
            }
            let near: Vec<&str> = w.iter().filter(|x| x.label_near_source).map(|x| x.id.as_str()).collect();
            assert_eq!(near, ["edge-igw-to-alb"], "{axis:?}");
        }
    }

    #[test]
    fn an_endpoint_route_says_so_in_its_label() {
        let src = "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\n";
        for axis in AXES {
            let (_, _, w, _) = planned(src, axis);
            let s3 = w.iter().find(|x| x.to == "s3").unwrap();
            assert_eq!((s3.label.replace('\n', " ").as_str(), s3.style), ("Read/Write (via VPC endpoint) (all AZs)", WireStyle::Main));
            let (_, _, ja, _) = planned(&format!("{src}set:\n  lang: ja\n"), axis);
            assert_eq!(ja.iter().find(|x| x.to == "s3").unwrap().label.replace('\n', " "), "読み書き （VPC エンドポイント経由） （全 AZ から）");
            let (_, _, nat, _) = planned(&src.replace("endpoint", "nat"), axis);
            assert!(!nat.iter().find(|x| x.to == "s3").unwrap().label.contains("VPC"));
        }
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
        let parts = ["読み書き", "（VPC エンドポイント経由）", "（全 AZ から）"];
        for src in endpoint_ja_definitions() { for axis in AXES {
            let (_, l, w, notes) = planned(&src, axis);
            assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?} {axis:?} in\n{src}");
            let s3 = w.iter().find(|x| x.to == "s3").unwrap();
            // The three parts in order, none broken across lines.
            assert_eq!(s3.label.replace('\n', " "), parts.join(" "), "{axis:?} in\n{src}");
            let whole = |line: &str| (0..parts.len()).any(|i| (i + 1..=parts.len()).any(|j| parts[i..j].join(" ") == line));
            assert!(s3.label.lines().all(whole), "{:?} {axis:?} in\n{src}", s3.label);
            // A frame side cuts a label only where no stretch is clear of them, and then the label
            // lies clear of every line, icon and name.
            let borders = Scene::new(&l, Measure::Flow).borders;
            for x in w.iter().filter(|x| !x.label.is_empty()) {
                let area = label_area_of(x, &l);
                if !borders.iter().any(|&(c, d)| hits(c, d, area)) { continue; }
                for y in &w {
                    assert!(full_path(y, &l).windows(2).all(|s| !hits(s[0], s[1], area)), "{} runs through the label of {} on a frame side {axis:?} in\n{src}", y.id, x.id);
                }
                for i in &l.icons {
                    assert!(!overlaps(i.rect, area) && !overlaps(name_area(i), area), "{} or its name is under the label of {} on a frame side {axis:?} in\n{src}", i.id, x.id);
                }
            }
        } }
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
        // Running right, the S3 line of the Japanese endpoint example leaves the last zone and
        // crosses the VPC's side and the Region's: no stretch of it holds its label clear of every frame side.
        let three = ENDPOINT_JA.replace("lang: ja\n", "lang: ja\n  azs: 3\n");
        for (src, id) in [(ENDPOINT_JA, "edge-ecs-1c-to-s3"), (three.as_str(), "edge-ecs-1d-to-s3")] {
            assert_eq!(violations(src, Axis::Right), Vec::<String>::new(), "{src}");
            let (_, l, w, notes) = planned(src, Axis::Right);
            assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?}");
            let s3 = w.iter().find(|x| x.to == "s3").unwrap();
            assert_eq!(s3.id, id);
            // Whole parts: on one line, or one part per line.
            assert!(["読み書き （VPC エンドポイント経由） （全 AZ から）", "読み書き\n（VPC エンドポイント経由）\n（全 AZ から）"].contains(&s3.label.as_str()), "{:?}", s3.label);
            let area = label_area_of(s3, &l);
            assert!(Scene::new(&l, Measure::Flow).borders.iter().any(|&(c, d)| hits(c, d, area)), "{area:?} is clear of frame sides, so the first search should have found it");
            for y in &w {
                for s in full_path(y, &l).windows(2) {
                    assert!(!hits(s[0], s[1], area), "{} runs through the label of {}", y.id, s3.id);
                }
            }
            for i in &l.icons {
                assert!(!overlaps(i.rect, area) && !overlaps(name_area(i), area), "{} or its name is under the label", i.id);
            }
        }
        // A label that fits one part per line clear of frame sides keeps that spot and shape.
        for axis in AXES {
            let (_, l, w, _) = planned(&ENDPOINT_JA.replace("lang: ja", "lang: en"), axis);
            let s3 = w.iter().find(|x| x.to == "s3").unwrap();
            assert_eq!(s3.label, "Read/Write\n(via VPC endpoint)\n(all AZs)", "{axis:?}");
            let area = label_area_of(s3, &l);
            assert!(Scene::new(&l, Measure::Flow).borders.iter().all(|&(c, d)| !hits(c, d, area)), "{axis:?}");
        }
    }

    #[test]
    fn a_label_with_no_spot_clear_of_lines_and_text_stays_at_its_usual_spot_and_is_not_clear() {
        let (m, l, w, _) = compiled(WEB2);
        let r = Rules::builtin();
        let ctx = Ctx { model: &m, layout: &l, rules: &r, scene: Scene::new(&l, Measure::Flow), measure: Measure::Flow };
        let paths: Vec<Vec<Pt>> = w.iter().map(|x| full_path(x, &l)).collect();
        let me = w.iter().position(|x| x.id == "edge-cloudfront-to-s3").unwrap();
        // Labels already placed everywhere: no spot is clear even over frame sides.
        let everywhere = Rect { x: -10_000, y: -10_000, w: 20_000, h: 20_000 };
        let spot = ctx.place_label(&["Static files".to_string()], (&paths, me), false, false, Axis::Right, &[everywhere]);
        assert!(!spot.clear);
        assert_eq!((spot.text.as_str(), spot.at), ("Static files", None));
    }

    /// Lines from the load balancer and the private parts all head for the side column.
    const AUX_TO_SIDE: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - alb ~> s3\nchoose:\n  route.aurora.s3: endpoint\n";

    #[test]
    fn lines_to_the_side_column_get_their_own_lanes() {
        for axis in AXES {
        assert_eq!(violations(AUX_TO_SIDE, axis), Vec::<String>::new());
        let (_, l, w, notes) = planned(AUX_TO_SIDE, axis);
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
        assert_eq!(xs.len(), n, "two jogs share an x {axis:?}");
        }
    }

    #[test]
    fn the_final_check_names_lines_that_overlap_or_cross_text_whatever_the_router_thought() {
        let (m, l, mut w, _) = compiled(WEB2);
        assert_eq!(audit(&m, &l, &w, &[], Measure::Flow), Vec::<String>::new());
        // Lay one line on top of another, one pixel apart, and another across an icon's label.
        let a = w.iter().position(|x| x.id == "edge-ecs-1a-to-aurora-1a").unwrap();
        let b = w.iter().position(|x| x.id == "edge-ecs-1c-to-aurora-1c").unwrap();
        let pa = full_path(&w[a], &l);
        w[b].points = vec![(pa[0].0 + 1, pa[0].1 + 5), (pa[0].0 + 1, pa[pa.len() - 1].1 - 5)];
        w[b].exit = w[a].exit;
        let named = audit(&m, &l, &w, &[], Measure::Flow);
        assert!(named.contains(&"edge-ecs-1a-to-aurora-1a".to_string()) && named.contains(&"edge-ecs-1c-to-aurora-1c".to_string()), "{named:?}");
    }

    #[test]
    fn routing_is_deterministic() {
        for src in both() { for axis in AXES {
            assert_eq!(planned(&src, axis).2, planned(&src, axis).2);
        } }
    }

    /// Every way a line breaks the diagram checker's rules in one definition along `axis`, as
    /// (line id, what), measured by this test's own arithmetic; and the notes that name a line.
    fn faults(src: &str, axis: Axis) -> (Vec<(String, String)>, Vec<String>) {
        let (_, l, w, notes) = planned(src, axis);
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
                    if segment_hits_rect(s[0], s[1], frame_name_box(f)) { fault(format!("crosses the name of {}", f.id)); }
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
    fn violations(src: &str, axis: Axis) -> Vec<String> {
        let (f, named) = faults(src, axis);
        named.into_iter().chain(f.into_iter().map(|(id, what)| format!("{id} {what}"))).collect()
    }

    /// The example at the top of the definition guide.
    const SHOWCASE: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\nset:\n  azs: 3\n  lang: ja\nchoose:\n  route.ecs.s3: nat\n";

    const HEAVY: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> alb\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - ecs -> internet\n  - waf -- cloudfront\n  - acm -- cloudfront\nchoose:\n  route.ecs.s3: endpoint\n";

    /// Accepted definitions this version cannot draw without a fault: too many lines and labels
    /// meet in one gap. They are still drawn, and every line at fault must be named in a note.
    fn crowded_definitions() -> Vec<String> {
        vec![CROWDED_S3_JA.to_string(), CROWDED_SIDE_EN.to_string()]
    }

    /// True if the crowded definition breaks a line along this axis, by the table of the lines it cannot draw.
    fn breaks(src: &str, axis: Axis) -> bool {
        let name = match src {
            CROWDED_S3_JA => "s3-ja",
            CROWDED_SIDE_EN => "side-en",
            _ => return false,
        };
        crate::tests::CROWDED_BROKEN.iter().any(|(n, a, lines)| *n == name && *a == axis && !lines.is_empty())
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
            "flows:\n  - user -> cloudfront -> alb -> ecs\n  - acm -- cloudfront\n  - waf -- cloudfront\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n",
        ].map(String::from));
        srcs.extend(endpoint_ja_definitions());
        srcs.extend([SHOWCASE.to_string(), SHOWCASE.replace("route.ecs.s3: nat", "route.ecs.s3: endpoint")]);
        srcs.extend([ENDPOINT_3AZ_EN.to_string(), ENDPOINT_3AZ_EN.replace("lang: en", "lang: ja")]);
        srcs.extend(SMALL_DEFINITIONS.map(String::from));
        srcs.extend(crowded_definitions());
        srcs
    }

    /// Small definitions with a VPC, each writing a few of the allowed links on its own.
    const SMALL_DEFINITIONS: [&str; 17] = [
        "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n",
        "flows:\n  - user -> alb -> ecs -> aurora\nset:\n  azs: 3\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n",
        "flows:\n  - user -> alb -> ecs\n  - cloudfront -> s3\n  - user -> cloudfront\n  - ecs -> s3\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> internet\nset:\n  nat: single\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs ~> cloudwatch\n",
        "flows:\n  - user -> cloudfront -> alb -> ecs\n  - alb ~> s3\n  - cloudfront ~> s3\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\n",
        "flows:\n  - user -> alb -> ecs -> aurora\n",
        "flows:\n  - user -> cloudfront -> alb -> ecs\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n  - ecs ~> cloudwatch\nset:\n  lang: ja\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\nset:\n  lang: ja\n",
        "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n",
        "flows:\n  - user -> alb -> ecs\n  - alb ~> cloudwatch\n",
        "flows:\n  - user -> alb -> ecs -> aurora\n  - ecs ~> cloudwatch\n",
        "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\n",
    ];

    #[test]
    fn other_accepted_definitions_route_cleanly() {
        let mut bad = Vec::new();
        for src in more_definitions().iter().chain(&both()) { for axis in AXES {
            if breaks(src, axis) { continue; }
            let v = violations(src, axis);
            if !v.is_empty() { bad.push(format!("{axis:?}\n{src}{v:#?}")); }
        } }
        assert!(bad.is_empty(), "{}", bad.join("\n----\n"));
    }

    #[test]
    fn no_line_at_fault_comes_out_without_a_note() {
        let crowded = crowded_definitions();
        for src in more_definitions().iter().chain(&both()).chain(&crowded) { for axis in AXES {
            let (f, named) = faults(src, axis);
            for (id, what) in f {
                assert!(named.iter().any(|n| n.contains(&format!("{id} "))), "{id} {what}, but no note names it, {axis:?} in\n{src}");
            }
        } }
        for src in &crowded { for axis in AXES {
            assert!(more_definitions().contains(src), "{src}");
            assert_eq!(!faults(src, axis).1.is_empty(), breaks(src, axis), "{axis:?} {src}");
        } }
    }

    #[test]
    fn a_line_between_icons_at_the_same_height_is_straight() {
        for src in [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n"), format!("{WEB2}set:\n  lang: ja\n")] { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
            let cw = w.iter().find(|x| x.to == "cloudwatch").unwrap();
            assert!(straight(cw, &l) && cw.exit.pos == cw.entry.pos, "{} bends: {:?} {:?} {:?} {axis:?}", cw.id, cw.exit, cw.entry, cw.points);
            for x in &w {
                let (fr, tr) = (l.rect(&x.from), l.rect(&x.to));
                let sideways = matches!(x.exit.side, Side::Left | Side::Right) && x.entry.side == opposite(x.exit.side);
                if fr.y == tr.y && fr.h == tr.h && sideways {
                    assert!(straight(x, &l), "{} is not straight: {:?} {axis:?}", x.id, x.points);
                }
            }
        } }
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
        for src in acceptance_definitions() { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
            let borders = Scene::new(&l, Measure::Flow).borders;
            for x in w.iter().filter(|x| !x.label.is_empty()) {
                let area = label_area_of(x, &l);
                for &(c, d) in &borders {
                    assert!(!hits(c, d, area), "a frame side {c:?}-{d:?} cuts the label of {} {axis:?} in\n{src}", x.id);
                }
            }
        } }
    }

    #[test]
    fn no_other_line_runs_through_a_line_label() {
        for src in more_definitions().iter().chain(&acceptance_definitions()) { for axis in AXES {
            if breaks(src, axis) { continue; }
            let (_, l, w, _) = planned(src, axis);
            for x in w.iter().filter(|x| !x.label.is_empty()) {
                let area = label_area_of(x, &l);
                for y in &w {
                    for s in full_path(y, &l).windows(2) {
                        assert!(!hits(s[0], s[1], area), "{} runs through the label of {} {axis:?} in\n{src}", y.id, x.id);
                    }
                }
            }
        } }
    }

    #[test]
    fn a_label_moves_off_frame_sides_to_a_clear_stretch() {
        for axis in AXES {
            let mut moved = 0;
            for src in acceptance_definitions() {
                let (_, l, w, _) = planned(&src, axis);
                let borders = Scene::new(&l, Measure::Flow).borders;
                let cut = |area: Rect| borders.iter().any(|&(c, d)| hits(c, d, area));
                for x in w.iter().filter(|x| !x.label.is_empty()) {
                    let usual = Wire { label_at: None, ..x.clone() };
                    if !cut(label_area_of(&usual, &l)) { continue; }
                    // Its usual spot is on a frame side; it sits on a stretch clear of every frame side.
                    assert!(x.label_at.is_some(), "{} {axis:?} in\n{src}", x.id);
                    assert!(!cut(label_area_of(x, &l)), "{} {axis:?} in\n{src}", x.id);
                    moved += 1;
                }
            }
            assert!(moved > 0, "no label in the acceptance definitions starts on a frame side, {axis:?}");
        }
    }

    #[test]
    fn the_replication_line_to_the_far_reader_stays_inside_the_zones() {
        for src in acceptance_definitions().into_iter().filter(|s| s.contains("azs: 3")) { for axis in AXES {
            let (_, l, w, _) = planned(&src, axis);
            let x = w.iter().find(|x| x.id == "edge-aurora-1a-to-aurora-1d").unwrap();
            let bottom = ["az-1a", "az-1c", "az-1d"].map(|id| { let r = l.rect(id); r.y + r.h }).into_iter().min().unwrap();
            assert!(full_path(x, &l).iter().all(|p| p.1 < bottom), "{:?} goes below the zones at {bottom} {axis:?} in\n{src}", x.points);
        } }
    }

    #[test]
    fn wire_ids_are_unique() {
        for src in more_definitions().iter().chain(&both()) { for axis in AXES {
            let (_, _, w, _) = planned(src, axis);
            let mut ids: Vec<&str> = w.iter().map(|x| x.id.as_str()).collect();
            ids.sort_unstable();
            let n = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), n, "{axis:?} {src}");
        } }
    }

    #[test]
    fn a_line_no_route_can_place_is_kept_and_named_in_a_note() {
        let r = Rules::builtin();
        let (m, _) = crate::inference::infer(&crate::def::parse(WEB2).unwrap(), &r);
        let m = m.unwrap();
        let p = crate::arrange::arrange(&m, &r, crate::arrange::default_axis(&m)).unwrap();
        let mut l = p.layout;
        let s3 = l.rect("s3");
        for (k, (dx, dy)) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)].into_iter().enumerate() {
            let rect = Rect { x: s3.x + dx * 62, y: s3.y + dy * 62, w: 60, h: 60 };
            l.icons.push(crate::layout::Icon { id: format!("wall-{k}"), label: String::new(), lines: 1, rect, parent: None });
        }
        let (w, notes) = route_planned(&m, &l, &r, &p.lines);
        assert!(w.iter().any(|x| x.id == "edge-cloudfront-to-s3"));
        assert_eq!(notes, ["The line edge-cloudfront-to-s3 could not avoid every icon, label and other line; check it in the diagram."]);
    }

    // Planned routes: a hand-built layout with only a Region frame, no VPC.

    type IconSpec<'a> = (&'a str, i32, i32, &'a str);
    type EdgeSpec<'a> = (&'a str, &'a str, Rel, &'a str);

    fn scene(icons: &[IconSpec], edges: &[EdgeSpec]) -> (Model, Layout) {
        scene_in(FrameKind::Region, "region", icons, edges)
    }

    /// Like `scene`, with the one frame of this kind and id instead of the Region frame.
    fn scene_in(kind: FrameKind, frame: &str, icons: &[IconSpec], edges: &[EdgeSpec]) -> (Model, Layout) {
        let nodes = icons
            .iter()
            .map(|&(id, _, _, _)| Node {
                id: id.to_string(),
                service: "s3".to_string(),
                option: None,
                az: None,
                role: None,
                added: false,
                region: None,
                part: id.to_string(),
                tag: id.split_once('.').map(|(_, t)| t.to_string()),
                area: crate::model::Area::Region,
                undecided: false,
            })
            .collect();
        let edges = edges
            .iter()
            .map(|&(from, to, rel, label)| crate::model::Edge {
                from: from.to_string(),
                to: to.to_string(),
                rel,
                label: label.to_string(),
                hole: None,
                endpoint: false,
            })
            .collect();
        let model = Model { region: "ap-northeast-1".to_string(), azs: Vec::new(), lang: crate::rules::Lang::En, nodes, edges, direction: None };
        let frames = vec![Frame {
            id: frame.to_string(),
            kind,
            label: "Asia Pacific (Tokyo)".to_string(),
            rect: Rect { x: -100, y: -100, w: 1000, h: 600 },
            parent: None,
        }];
        let icons = icons
            .iter()
            .map(|&(id, x, y, label)| Icon {
                id: id.to_string(),
                label: label.to_string(),
                lines: label.lines().count() as u8,
                rect: Rect { x, y, w: 60, h: 60 },
                parent: Some(frame.to_string()),
            })
            .collect();
        (model, Layout { frames, icons })
    }

    /// A free draft with no edge yet; `route_by_ends` gives it the edge with the same ends.
    fn plan(from: &str, to: &str, exit: Side, entry: Side, bend: Bend) -> LinePlan {
        draft_for(usize::MAX, from, to, exit, entry, bend, Keep::Free)
    }

    fn draft_for(edge: usize, from: &str, to: &str, exit: Side, entry: Side, bend: Bend, keep: Keep) -> LinePlan {
        LinePlan {
            edge,
            from: from.to_string(),
            to: to.to_string(),
            exit,
            entry,
            bend,
            exit_pos: None,
            entry_pos: None,
            keep,
            tracks: Vec::new(),
            axis: Axis::Right,
        }
    }

    /// `route_planned` with every draft made by `plan` given the model edge with the same ends;
    /// a draft whose ends are no edge keeps an edge number no line has.
    fn route_by_ends(m: &Model, l: &Layout, r: &Rules, plans: &[LinePlan]) -> (Vec<Wire>, Vec<String>) {
        let plans: Vec<LinePlan> = plans
            .iter()
            .map(|p| {
                let mut p = p.clone();
                if p.edge == usize::MAX {
                    p.edge = m.edges.iter().position(|e| e.from == p.from && e.to == p.to).unwrap_or(usize::MAX);
                }
                p
            })
            .collect();
        route_planned(m, l, r, &plans)
    }

    fn wire<'w>(w: &'w [Wire], from: &str, to: &str) -> &'w Wire {
        w.iter().find(|x| x.from == from && x.to == to).unwrap_or_else(|| panic!("no line {from} -> {to}"))
    }

    const S3: &str = "Amazon S3";

    fn three_in_a_row() -> (Model, Layout, Vec<LinePlan>) {
        let (m, l) = scene(
            &[("a", 0, 0, S3), ("b", 220, 0, S3), ("c", 440, 160, S3)],
            &[("a", "b", Rel::Main, "https"), ("b", "c", Rel::Main, "read_write")],
        );
        let plans = vec![
            plan("a", "b", Side::Right, Side::Left, Bend::Straight),
            plan("b", "c", Side::Right, Side::Left, Bend::AtX(390)),
        ];
        (m, l, plans)
    }

    fn gateway_and_cognito() -> (Model, Layout, Vec<LinePlan>) {
        let (mut m, l) = scene(
            &[("g", 0, 0, "Amazon API Gateway\nHTTP API"), ("c", 0, 160, "Amazon Cognito")],
            &[("c", "g", Rel::Assoc, "jwt_authorizer")],
        );
        m.lang = crate::rules::Lang::Ja;
        (m, l, vec![plan("c", "g", Side::Top, Side::Bottom, Bend::Straight)])
    }

    fn two_into_one() -> (Model, Layout, Vec<LinePlan>) {
        let (m, l) = scene(
            &[("a", 0, 0, S3), ("b", 0, 160, S3), ("c", 220, 80, S3)],
            &[("a", "c", Rel::Main, "https"), ("b", "c", Rel::Main, "https")],
        );
        let plans = vec![
            plan("a", "c", Side::Right, Side::Left, Bend::AtX(150)),
            plan("b", "c", Side::Right, Side::Left, Bend::AtX(150)),
        ];
        (m, l, plans)
    }

    #[test]
    fn planned_lines_take_their_sides_and_bend_without_a_vpc() {
        let (m, l, plans) = three_in_a_row();
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!(ab.exit, Port { side: Side::Right, pos: 50, offset: 0 });
        assert_eq!(ab.entry, Port { side: Side::Left, pos: 50, offset: 0 });
        assert!(ab.points.is_empty(), "{:?}", ab.points);
        assert_eq!(wire(&w, "b", "c").points, [(390, 30), (390, 190)]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_bottom_port_clears_a_multi_line_name_and_a_japanese_label_is_placed() {
        let (m, l, plans) = gateway_and_cognito();
        let r = Rules::builtin();
        let (w, notes) = route_by_ends(&m, &l, &r, &plans);
        let cg = wire(&w, "c", "g");
        assert_eq!(cg.entry, Port { side: Side::Bottom, pos: 50, offset: 38 });
        assert_eq!(cg.label, "JWT オーソライザー");
        let unplaced = r.text("note.unplaced_label", crate::rules::Lang::Ja).replace("{line}", &cg.id);
        assert!(!notes.contains(&unplaced), "{notes:?}");
        let path = full_path(cg, &l);
        let area = audit_wire_label(cg, &path).expect("the line has a label");
        for icon in &l.icons {
            assert!(!overlaps(area, label_area(icon)), "the line's label {area:?} covers the name of {}", icon.id);
        }
    }

    #[test]
    fn two_lines_into_one_side_share_it() {
        let (m, l, plans) = two_into_one();
        let (w, _) = route_by_ends(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(wire(&w, "a", "c").entry.pos, 25);
        assert_eq!(wire(&w, "b", "c").entry.pos, 75);
    }

    #[test]
    fn a_planned_line_that_hits_an_icon_goes_around_it_on_the_same_side() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("x", 110, 0, S3), ("b", 220, 0, S3)], &[("a", "b", Rel::Main, "https")]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "b", Side::Right, Side::Left, Bend::Straight)]);
        let ab = wire(&w, "a", "b");
        let x = l.rect("x");
        let path = full_path(ab, &l);
        assert!(path.windows(2).all(|s| !segment_hits_rect(s[0], s[1], x)), "{path:?}");
        assert_eq!(ab.exit.side, Side::Right);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_planned_side_that_is_closed_moves_to_another_side() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("w", 62, 0, S3), ("b", 300, 200, S3)], &[("a", "b", Rel::Main, "https")]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "b", Side::Right, Side::Left, Bend::AtX(200))]);
        let ab = wire(&w, "a", "b");
        assert_ne!(ab.exit.side, Side::Right);
        let wall = l.rect("w");
        let path = full_path(ab, &l);
        assert!(path.windows(2).all(|s| !segment_hits_rect(s[0], s[1], wall)), "{path:?}");
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn every_semantic_edge_is_represented_once_in_a_planned_route() {
        for (m, l, plans) in [three_in_a_row(), gateway_and_cognito(), two_into_one()] {
            let (w, _) = route_by_ends(&m, &l, &Rules::builtin(), &plans);
            let mut semantic: Vec<(String, String)> = m.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
            let mut drawn: Vec<(String, String)> = w.iter().flat_map(|x| x.represents.clone()).collect();
            semantic.sort();
            drawn.sort();
            assert_eq!(semantic, drawn);
        }
    }

    #[test]
    fn an_empty_plan_list_draws_every_line_without_a_vpc() {
        let (m, l) = scene(
            &[("a", 0, 0, S3), ("b", 220, 0, S3), ("c", 440, 0, S3)],
            &[("a", "b", Rel::Main, "https"), ("b", "c", Rel::Main, "https")],
        );
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[]);
        let ends: Vec<(&str, &str)> = w.iter().map(|x| (x.from.as_str(), x.to.as_str())).collect();
        assert_eq!(ends, [("a", "b"), ("b", "c")]);
        assert_represented_once(&m, &w);
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// Every semantic edge is in exactly one line's `represents`, and nothing else is.
    fn assert_represented_once(m: &Model, w: &[Wire]) {
        let mut semantic: Vec<(String, String)> = m.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
        let mut drawn: Vec<(String, String)> = w.iter().flat_map(|x| x.represents.clone()).collect();
        semantic.sort();
        drawn.sort();
        assert_eq!(semantic, drawn);
    }

    fn row_of_three() -> (Model, Layout) {
        scene(
            &[("a", 0, 0, S3), ("b", 220, 0, S3), ("c", 440, 0, S3)],
            &[("a", "b", Rel::Main, "https"), ("b", "c", Rel::Main, "https")],
        )
    }

    #[test]
    fn a_plan_for_some_lines_only_leaves_the_others_to_the_candidates() {
        let (m, l) = row_of_three();
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "b", Side::Bottom, Side::Bottom, Bend::AtY(150))]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side), (Side::Bottom, Side::Bottom));
        assert_eq!(ab.points, [(30, 150), (250, 150)]);
        let bc = wire(&w, "b", "c");
        assert_eq!((bc.exit.side, bc.entry.side), (Side::Right, Side::Left));
        assert_represented_once(&m, &w);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_bend_at_a_y_turns_on_that_y() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 220, 160, S3)], &[("a", "b", Rel::Aux, "logs_metrics")]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "b", Side::Bottom, Side::Top, Bend::AtY(130))]);
        let ab = wire(&w, "a", "b");
        assert_eq!(ab.exit, Port { side: Side::Bottom, pos: 50, offset: 24 });
        assert_eq!(ab.entry, Port { side: Side::Top, pos: 50, offset: 0 });
        assert_eq!(ab.points, [(30, 130), (250, 130)]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn an_elbow_turns_where_the_exit_line_meets_the_entry_line() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 220, 160, S3)], &[("a", "b", Rel::Main, "https")]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "b", Side::Right, Side::Top, Bend::Elbow)]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side), (Side::Right, Side::Top));
        assert_eq!(ab.points, [(250, 30)]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_layout_with_only_a_cloud_frame_is_routed() {
        let (m, l) = scene_in(
            FrameKind::Cloud,
            "aws-cloud",
            &[("a", 0, 0, S3), ("b", 220, 0, S3), ("c", 440, 160, S3)],
            &[("a", "b", Rel::Main, "https"), ("b", "c", Rel::Main, "read_write")],
        );
        let plans = [
            plan("a", "b", Side::Right, Side::Left, Bend::Straight),
            plan("b", "c", Side::Right, Side::Left, Bend::AtX(390)),
        ];
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &plans);
        assert!(wire(&w, "a", "b").points.is_empty());
        assert_eq!(wire(&w, "b", "c").points, [(390, 30), (390, 190)]);
        assert_represented_once(&m, &w);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn no_lines_give_no_wires_and_no_notes() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 220, 0, S3)], &[]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "b", Side::Right, Side::Left, Bend::Straight)]);
        assert!(w.is_empty());
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_plan_names_a_part_with_a_dot_by_its_full_id() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("s3.web", 220, 160, S3)], &[("a", "s3.web", Rel::Main, "https")]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("a", "s3.web", Side::Right, Side::Top, Bend::Elbow)]);
        let x = wire(&w, "a", "s3.web");
        assert_eq!(x.id, "edge-a-to-s3.web");
        assert_eq!((x.exit.side, x.entry.side), (Side::Right, Side::Top));
        assert_eq!(x.points, [(250, 30)]);
        assert_eq!(x.represents, [("a".to_string(), "s3.web".to_string())]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_plan_for_a_line_the_model_does_not_have_is_ignored() {
        let (m, l) = row_of_three();
        let plans = [
            plan("a", "nowhere", Side::Bottom, Side::Top, Bend::Straight),
            plan("nowhere", "c", Side::Bottom, Side::Top, Bend::Straight),
            plan("a", "c", Side::Bottom, Side::Bottom, Bend::AtY(150)),
        ];
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &plans);
        let ends: Vec<(&str, &str)> = w.iter().map(|x| (x.from.as_str(), x.to.as_str())).collect();
        assert_eq!(ends, [("a", "b"), ("b", "c")]);
        for x in &w {
            assert_eq!((x.exit.side, x.entry.side), (Side::Right, Side::Left), "{}", x.id);
        }
        assert_represented_once(&m, &w);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_plan_in_the_other_direction_is_not_taken_for_the_line() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 220, 0, S3)], &[("a", "b", Rel::Main, "https")]);
        let (w, notes) = route_by_ends(&m, &l, &Rules::builtin(), &[plan("b", "a", Side::Bottom, Side::Bottom, Bend::AtY(150))]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side), (Side::Right, Side::Left));
        assert!(ab.points.is_empty(), "{:?}", ab.points);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn of_two_plans_for_one_line_the_first_is_taken() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 220, 0, S3)], &[("a", "b", Rel::Main, "https")]);
        let straight = plan("a", "b", Side::Right, Side::Left, Bend::Straight);
        let below = plan("a", "b", Side::Bottom, Side::Bottom, Bend::AtY(150));
        let (w, _) = route_by_ends(&m, &l, &Rules::builtin(), &[straight.clone(), below.clone()]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side), (Side::Right, Side::Left));
        assert!(ab.points.is_empty(), "{:?}", ab.points);
        let (w, _) = route_by_ends(&m, &l, &Rules::builtin(), &[below, straight]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side), (Side::Bottom, Side::Bottom));
        assert_eq!(ab.points, [(30, 150), (250, 150)]);
    }

    #[test]
    fn a_plan_whose_exit_and_entry_meet_does_not_panic_and_the_line_is_kept() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 60, 0, S3)], &[("a", "b", Rel::Main, "https")]);
        let r = Rules::builtin();
        let (w, notes) = route_by_ends(&m, &l, &r, &[plan("a", "b", Side::Right, Side::Left, Bend::Straight)]);
        let ab = wire(&w, "a", "b");
        assert_eq!(ab.represents, [("a".to_string(), "b".to_string())]);
        let unrouted = r.text("note.unrouted_line", crate::rules::Lang::En).replace("{line}", &ab.id);
        let moved = (ab.exit.side, ab.entry.side) != (Side::Right, Side::Left);
        assert!(moved || notes.contains(&unrouted), "{ab:?} {notes:?}");
    }

    #[test]
    fn a_line_whose_ends_meet_and_no_side_is_free_is_named_in_a_note() {
        let mut icons = vec![("a", 0, 0, S3), ("b", 60, 0, S3)];
        let walls = [
            ("w0", -62, -62), ("w1", 0, -62), ("w2", 60, -62), ("w3", 122, -62),
            ("w4", -62, 0), ("w5", 122, 0),
            ("w6", -62, 62), ("w7", 0, 62), ("w8", 60, 62), ("w9", 122, 62),
        ];
        icons.extend(walls.iter().map(|&(id, x, y)| (id, x, y, "")));
        let (m, l) = scene(&icons, &[("a", "b", Rel::Main, "https")]);
        let r = Rules::builtin();
        let (w, notes) = route_by_ends(&m, &l, &r, &[plan("a", "b", Side::Right, Side::Left, Bend::Straight)]);
        let ab = wire(&w, "a", "b");
        assert_eq!(ab.represents, [("a".to_string(), "b".to_string())]);
        let unrouted = r.text("note.unrouted_line", crate::rules::Lang::En).replace("{line}", &ab.id);
        assert!(notes.contains(&unrouted), "{ab:?} {notes:?}");
    }

    // Fails if a line already merged across AZs is counted again by the step that bundles side lines into one service, and dropped.
    #[test]
    fn web_lines_merged_across_azs_survive_bundling() {
        let src = format!("{WEB2}set:\n  lang: ja\n");
        let r = Rules::builtin();
        let (m, _) = crate::inference::infer(&crate::def::parse(&src).unwrap(), &r);
        let m = m.unwrap();
        let p = crate::arrange::arrange(&m, &r, crate::arrange::default_axis(&m)).unwrap();
        let (w, _) = route_planned(&m, &p.layout, &r, &[]);
        let cw: Vec<&Wire> = w.iter().filter(|x| x.to == "cloudwatch").collect();
        assert_eq!(cw.len(), 1, "{:?}", w.iter().map(|x| &x.id).collect::<Vec<_>>());
        assert_eq!(cw[0].represents, [("ecs-1a".to_string(), "cloudwatch".to_string()), ("ecs-1c".to_string(), "cloudwatch".to_string())]);
    }

    // Fails if a line whose target AZ has no copy of the sender is dropped instead of drawn from another AZ.
    #[test]
    fn three_zones_with_fewer_readers_keep_every_edge() {
        let src = "flows:\n  - user -> alb -> ecs -> aurora\n  - aurora -> ecs\n  - aurora -> s3\nset:\n  azs: 3\n  readers: 1\nchoose:\n  route.aurora.s3: endpoint\n";
        let (m, _, w, _) = compiled(src);
        let mut drawn: Vec<(String, String)> = w.iter().flat_map(|x| x.represents.clone()).collect();
        let mut want: Vec<(String, String)> = m.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
        drawn.sort();
        want.sort();
        assert_eq!(drawn, want);
        let into_1d: Vec<&Wire> = w.iter().filter(|x| x.to == "ecs-1d" && x.from.starts_with("aurora-")).collect();
        assert_eq!(into_1d.len(), 1, "{:?}", w.iter().map(|x| &x.id).collect::<Vec<_>>());
        assert_eq!(into_1d[0].from, "aurora-1a");
    }

    // Drafts that keep their ends, waypoints, gates and bands.

    const HTTPS: &str = "https";

    /// A draft for the model's edge `edge`, read from the model.
    fn keep_draft(m: &Model, edge: usize, exit: Side, entry: Side, bend: Bend, keep: Keep) -> LinePlan {
        let e = &m.edges[edge];
        draft_for(edge, &e.from, &e.to, exit, entry, bend, keep)
    }

    fn fixed_25() -> (Model, Layout, Vec<LinePlan>) {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let mut p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Whole);
        (p.exit_pos, p.entry_pos) = (Some(25), Some(25));
        (m, l, vec![p])
    }

    fn waypoints() -> (Model, Layout, Vec<LinePlan>) {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 200, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Points(vec![(150, 30), (150, 230)]), Keep::Whole);
        (m, l, vec![p])
    }

    /// `b` inside the frame `f`, whose left side is at x = 200; `a` outside it.
    fn crossing() -> (Model, Layout, Vec<LinePlan>) {
        let (mut m, mut l) = scene(&[("a", 0, 100, S3), ("b", 400, 100, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        m.nodes[1].area = crate::model::Area::Subnet(crate::model::Tier::Private);
        l.frames.push(Frame { id: "f".to_string(), kind: FrameKind::Vpc, label: "F".to_string(), rect: Rect { x: 200, y: -100, w: 400, h: 400 }, parent: Some("region".to_string()) });
        l.icons[1].parent = Some("f".to_string());
        let through = Through { gate: (200, 130), inner: vec![(300, 130)], inner_is_start: false };
        let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Inner(through));
        (m, l, vec![p])
    }

    /// The exit, the waypoints and the entry, as written: no step folded, no point dropped.
    fn drawn(x: &Wire, l: &Layout) -> Vec<(i32, i32)> {
        let mut p = vec![port_point(l.rect(&x.from), x.exit)];
        p.extend(x.points.iter().copied());
        p.push(port_point(l.rect(&x.to), x.entry));
        p
    }

    // Fails if a fixed position is not read and the end is shared out with the side's other ends instead.
    #[test]
    fn a_fixed_position_is_kept() {
        let (m, l, plans) = fixed_25();
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.pos, ab.entry.pos), (25, 25));
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if a free end is shared out without leaving the fixed end's spot, and lands on it.
    #[test]
    fn a_free_end_does_not_take_a_fixed_spot() {
        let (m, l) = scene(
            &[("a", 0, 100, S3), ("c", 0, 300, S3), ("b", 300, 100, S3)],
            &[("a", "b", Rel::Main, HTTPS), ("c", "b", Rel::Main, HTTPS)],
        );
        let mut fixed = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Whole);
        fixed.entry_pos = Some(50);
        let free = keep_draft(&m, 1, Side::Right, Side::Left, Bend::AtX(180), Keep::Free);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[fixed, free]);
        assert_eq!(wire(&w, "a", "b").entry.pos, 50);
        // b's left side has two ends, slots(2) = [25, 75]: 25 and 75 are as near the fixed 50, so
        // the smaller, 25, is taken out and the free end gets 75.
        assert_eq!(wire(&w, "c", "b").entry.pos, 75);
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if straightening a line moves its fixed exit.
    #[test]
    fn straightening_keeps_a_fixed_exit() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let through = Through { gate: (60, 15), inner: Vec::new(), inner_is_start: true };
        let mut p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Inner(through));
        p.exit_pos = Some(25);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.pos, ab.entry.pos), (25, 25));
        assert!(ab.points.is_empty(), "{:?}", ab.points);
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if straightening moves a fixed entry level with the exit instead of the free exit level with the entry.
    #[test]
    fn straightening_keeps_a_fixed_entry() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let through = Through { gate: (300, 15), inner: Vec::new(), inner_is_start: false };
        let mut p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Inner(through));
        p.entry_pos = Some(25);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.entry.pos, ab.exit.pos), (25, 25));
        assert!(ab.points.is_empty(), "{:?}", ab.points);
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if the waypoints of a draft are not read.
    #[test]
    fn waypoints_are_drawn_as_planned() {
        let (m, l, plans) = waypoints();
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(wire(&w, "a", "b").points, [(150, 30), (150, 230)]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// `c` touches `b`'s left side, so every line into the middle of that side passes over `c`.
    fn blocked_left(keep: Keep) -> (Model, Layout, Vec<LinePlan>) {
        let (m, l) = scene(&[("a", 0, 100, S3), ("b", 300, 100, S3), ("c", 240, 100, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, keep);
        (m, l, vec![p])
    }

    // Fails if a line kept whole whose entry is closed is moved to another side, or drawn over an icon without a note.
    #[test]
    fn a_whole_line_is_never_searched() {
        let (m, l, plans) = blocked_left(Keep::Whole);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!(ab.entry.side, Side::Left);
        assert!(ab.points.is_empty(), "{:?}", ab.points);
        assert!(notes.iter().any(|n| n.contains("edge-a-to-b")), "{notes:?}");
    }

    // Fails if the way a draft is kept is applied to every line, free ones included.
    #[test]
    fn a_free_line_may_move() {
        let (m, l, plans) = blocked_left(Keep::Free);
        let (w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_ne!(wire(&w, "a", "b").entry.side, Side::Left);
    }

    /// `a` and `b` outside the VPC frame `f`, which lies between them across their straight line.
    fn vpc_between(keep: Keep) -> (Model, Layout, Vec<LinePlan>) {
        let (m, mut l) = scene(&[("a", 0, 100, S3), ("b", 600, 100, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        l.frames.push(Frame { id: "f".to_string(), kind: FrameKind::Vpc, label: "F".to_string(), rect: Rect { x: 200, y: 0, w: 200, h: 300 }, parent: Some("region".to_string()) });
        let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, keep);
        (m, l, vec![p])
    }

    // Fails if a free line between two parts outside the VPC is drawn through the VPC's inside, or named.
    #[test]
    fn a_line_between_parts_outside_the_vpc_goes_round_it() {
        let (m, l, plans) = vpc_between(Keep::Free);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let path = drawn(wire(&w, "a", "b"), &l);
        let vpc = Rect { x: 200, y: 0, w: 200, h: 300 };
        assert!(!path.windows(2).any(|s| hits(s[0], s[1], vpc)), "{path:?}");
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if the final check lets a line between two parts outside the VPC pass through its
    // inside unnamed, or names a line with an end inside the VPC that crosses the frame.
    #[test]
    fn the_final_check_names_a_line_between_parts_outside_the_vpc_through_it() {
        let (m, l, plans) = vpc_between(Keep::Whole);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert!(drawn(wire(&w, "a", "b"), &l).windows(2).any(|s| hits(s[0], s[1], Rect { x: 200, y: 0, w: 200, h: 300 })));
        assert!(notes.iter().any(|n| n.contains("edge-a-to-b")), "{notes:?}");
        assert_eq!(audit(&m, &l, &w, &[], Measure::Flow), vec!["edge-a-to-b".to_string()]);
        let mut inside = m.clone();
        inside.nodes[1].area = crate::model::Area::Subnet(crate::model::Tier::Private);
        assert_eq!(audit(&inside, &l, &w, &[], Measure::Flow), Vec::<String>::new());
    }

    // Fails if a line crosses the frame's side away from its gate, or the inner points are searched again.
    #[test]
    fn a_crossing_line_passes_its_gate() {
        let (m, l, plans) = crossing();
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        let path = drawn(ab, &l);
        let gate = path.iter().position(|&p| p == (200, 130)).unwrap_or_else(|| panic!("no gate in {path:?}"));
        assert_eq!(path.get(gate + 1), Some(&(300, 130)), "{path:?}");
        assert_eq!((ab.entry.side, ab.entry.pos), (Side::Left, 50));
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if the label ignores the line's axis and always takes the horizontal stretch first.
    #[test]
    fn labels_follow_the_line_axis() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 400, 300, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let mut horizontal = Vec::new();
        for axis in [Axis::Right, Axis::Down] {
            let mut p = keep_draft(&m, 0, Side::Right, Side::Top, Bend::Elbow, Keep::Free);
            p.axis = axis;
            let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
            let ab = wire(&w, "a", "b");
            // Out of (60, 30), 370 right to (430, 30), then 270 down into (430, 300).
            assert_eq!(drawn(ab, &l), [(60, 30), (430, 30), (430, 300)]);
            assert!(notes.is_empty(), "{notes:?}");
            horizontal.push(ab.horizontal_label);
        }
        assert_eq!(horizontal, [true, false]);
    }

    // Fails if the draft is chosen by the pair of ends, first found, instead of by the edge.
    #[test]
    fn drafts_are_chosen_by_edge() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let mut other = draft_for(1, "a", "b", Side::Right, Side::Left, Bend::Straight, Keep::Whole);
        (other.exit_pos, other.entry_pos) = (Some(25), Some(25));
        let mut own = draft_for(0, "a", "b", Side::Right, Side::Left, Bend::Straight, Keep::Whole);
        (own.exit_pos, own.entry_pos) = (Some(75), Some(75));
        for plans in [[other.clone(), own.clone()], [own, other]] {
            let (w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
            let ab = wire(&w, "a", "b");
            assert_eq!((ab.entry.pos, ab.rep), (75, 0));
        }
    }

    // Fails if a line kept whole may run another way inside, as long as it overlaps nothing.
    #[test]
    fn audit_finds_a_changed_whole_line() {
        let (m, l, plans) = waypoints();
        let (mut w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
        w[0].points = vec![(160, 30), (160, 230)];
        assert_eq!(audit(&m, &l, &w, &[], Measure::Flow), Vec::<String>::new(), "the changed line overlaps nothing");
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    // Fails if the final check does not look for the gate.
    #[test]
    fn audit_finds_a_line_missing_its_gate() {
        let (m, l, plans) = crossing();
        let (mut w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
        w[0].points = vec![(150, 130), (150, 160), (250, 160), (250, 130), (300, 130)];
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    // Fails if the final check does not look at the fixed ends.
    #[test]
    fn audit_finds_a_moved_fixed_end() {
        let (m, l, plans) = fixed_25();
        let (mut w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
        w[0].entry.pos = 50;
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    // Fails if the final check does not look at where a line runs on its band.
    #[test]
    fn audit_finds_a_line_off_its_track() {
        let (m, l, mut plans) = crossing();
        plans[0].tracks = vec![Track { horizontal: true, from: 220, to: 380, at: 130 }];
        let (mut w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
        // Through the gate and the inner point still, but at 140 between them.
        w[0].points = vec![(200, 130), (200, 140), (300, 140), (300, 130)];
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    /// The note that a line could not be drawn clear, for this line id.
    fn unrouted(id: &str) -> String {
        Rules::builtin().text("note.unrouted_line", crate::rules::Lang::En).replace("{line}", id)
    }

    // Fails if a line kept inside whose exit, entry and gate are one point is drawn as a single point (and panics) or without a note.
    #[test]
    fn a_line_through_a_gate_of_no_length_is_kept_and_named() {
        for inner_is_start in [false, true] {
            let (m, l) = scene(&[("a", 0, 0, S3), ("b", 60, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
            let through = Through { gate: (60, 30), inner: Vec::new(), inner_is_start };
            let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Inner(through));
            let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
            let ab = wire(&w, "a", "b");
            assert_eq!((ab.exit.side, ab.entry.side), (Side::Right, Side::Left), "{inner_is_start}");
            assert!(ab.points.is_empty(), "{inner_is_start}: {:?}", ab.points);
            assert!(notes.contains(&unrouted("edge-a-to-b")), "{inner_is_start}: {notes:?}");
        }
    }

    // Fails if a line kept whole whose draft turns back on itself is straightened, or passes without a note.
    #[test]
    fn a_whole_draft_that_turns_back_is_kept_and_named() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let plans = vec![keep_draft(&m, 0, Side::Right, Side::Left, Bend::AtX(450), Keep::Whole)];
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        // Out of (60, 30), right to x = 450, then back left into (300, 30).
        assert_eq!(drawn(wire(&w, "a", "b"), &l), [(60, 30), (450, 30), (300, 30)]);
        assert!(notes.contains(&unrouted("edge-a-to-b")), "{notes:?}");
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    /// Out of (60, 30), 100 right, 10 down, 340 right, then 260 down into b's top at (500, 300).
    fn short_drop_then_long() -> (Model, Layout, LinePlan) {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 470, 300, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let p = keep_draft(&m, 0, Side::Right, Side::Top, Bend::Points(vec![(160, 30), (160, 40), (500, 40)]), Keep::Whole);
        (m, l, p)
    }

    // Fails if a line running down, whose first upright stretch is too short for the label, puts it above a level stretch before trying the later upright one.
    #[test]
    fn running_down_a_label_tries_every_upright_stretch_first() {
        let (m, l, mut p) = short_drop_then_long();
        p.axis = Axis::Down;
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
        assert!(notes.is_empty(), "{notes:?}");
        let ab = wire(&w, "a", "b");
        assert_eq!(drawn(ab, &l), [(60, 30), (160, 30), (160, 40), (500, 40), (500, 300)]);
        let path = full_path(ab, &l);
        let at = label_distance(ab.label_at.expect("moved off the usual spot"), path_length(&path));
        let (anchor, level) = point_along(&path, at);
        assert!(!ab.horizontal_label && !level, "{ab:?}");
        assert!(anchor.0 == 500 && anchor.1 > 40 && anchor.1 < 300, "{anchor:?}");
    }

    // Fails if the order of the label spots for a line running right changes with the new order for lines running down.
    #[test]
    fn running_right_a_label_still_takes_the_level_stretch_it_leaves_by() {
        let (m, l, p) = short_drop_then_long();
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
        assert!(notes.is_empty(), "{notes:?}");
        let ab = wire(&w, "a", "b");
        let path = full_path(ab, &l);
        let at = label_distance(ab.label_at.expect("on the stretch it leaves by"), path_length(&path));
        let (anchor, _) = point_along(&path, at);
        assert!(ab.horizontal_label, "{ab:?}");
        assert!(anchor.1 == 30 && anchor.0 > 60 && anchor.0 < 160, "{anchor:?}");
    }

    // Fails if a position written for a free end is set aside without taking its spot, so it lands on a fixed end's spot.
    #[test]
    fn a_written_position_of_a_free_end_does_not_take_a_fixed_spot() {
        let (m, l) = scene(
            &[("a", 0, 100, S3), ("c", 0, 300, S3), ("b", 300, 100, S3)],
            &[("a", "b", Rel::Main, HTTPS), ("c", "b", Rel::Main, HTTPS)],
        );
        let mut fixed = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Whole);
        fixed.entry_pos = Some(50);
        let mut free = keep_draft(&m, 1, Side::Right, Side::Left, Bend::AtX(180), Keep::Free);
        free.entry_pos = Some(50);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[fixed, free.clone()]);
        assert_eq!(wire(&w, "a", "b").entry.pos, 50);
        // slots(2) = [25, 75]; the fixed 50 takes 25 (the smaller at a tie); 50 is the fixed end's,
        // so the free end takes the spot left nearest 50, which is 75.
        assert_eq!(wire(&w, "c", "b").entry.pos, 75);
        assert!(notes.is_empty(), "{notes:?}");
        // Alone on its side, a free end with a written position takes it.
        free.entry_pos = Some(30);
        let (w, _) = route_planned(&m, &l, &Rules::builtin(), &[keep_draft(&m, 0, Side::Right, Side::Top, Bend::Elbow, Keep::Free), free]);
        assert_eq!(wire(&w, "c", "b").entry.pos, 30);
    }

    /// `x` sits in the middle of the line from a to b, with room to go round it on the same sides.
    fn icon_in_the_way(keep: Keep) -> (Model, Layout, Vec<LinePlan>) {
        let (m, l) = scene(&[("a", 0, 100, S3), ("x", 150, 100, S3), ("b", 300, 100, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, keep);
        (m, l, vec![p])
    }

    // Fails if a line kept whole is searched round an icon in its way, or is drawn over it without the unrouted note.
    #[test]
    fn a_whole_line_is_not_searched_round_an_icon_in_its_way() {
        let (m, l, plans) = icon_in_the_way(Keep::Free);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side), (Side::Right, Side::Left));
        assert!(!ab.points.is_empty(), "a free line goes round x");
        assert!(!notes.contains(&unrouted("edge-a-to-b")), "{notes:?}");
        let (m, l, plans) = icon_in_the_way(Keep::Whole);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!(drawn(ab, &l), [(60, 130), (300, 130)]);
        assert!(notes.contains(&unrouted("edge-a-to-b")), "{notes:?}");
    }

    // Fails if a line kept whole is stopped by a side line's planned path, which only sends searched lines round it.
    #[test]
    fn a_whole_line_may_cross_a_side_line() {
        let (m, l) = scene(
            &[("a", 0, 100, S3), ("b", 300, 100, S3), ("c", 150, 0, S3), ("d", 150, 220, S3)],
            &[("a", "b", Rel::Main, HTTPS), ("c", "d", Rel::Aux, "logs_metrics")],
        );
        // The side line runs straight down at x = 180, across the main line's height 130.
        let side = keep_draft(&m, 1, Side::Bottom, Side::Top, Bend::Straight, Keep::Free);
        let main = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Whole);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[main, side]);
        assert_eq!(drawn(wire(&w, "a", "b"), &l), [(60, 130), (300, 130)]);
        assert!(!notes.contains(&unrouted("edge-a-to-b")), "{notes:?}");
        assert_eq!(drawn(wire(&w, "c", "d"), &l).iter().map(|p| p.0).collect::<Vec<_>>(), [180, 180]);
    }

    /// `a` inside the frame `f`, whose right side is at x = 250; `b` outside it. The line leaves
    /// through the gate (250, 130) after the inner point (150, 130).
    fn leaving(icons: &[IconSpec]) -> (Model, Layout, Vec<LinePlan>) {
        let mut all = vec![("a", 0, 100, S3), ("b", 450, 100, S3)];
        all.extend_from_slice(icons);
        let (mut m, mut l) = scene(&all, &[("a", "b", Rel::Main, HTTPS)]);
        m.nodes[0].area = crate::model::Area::Subnet(crate::model::Tier::Private);
        l.frames.push(Frame { id: "f".to_string(), kind: FrameKind::Vpc, label: "F".to_string(), rect: Rect { x: -50, y: -50, w: 300, h: 300 }, parent: Some("region".to_string()) });
        l.icons[0].parent = Some("f".to_string());
        let through = Through { gate: (250, 130), inner: vec![(150, 130)], inner_is_start: true };
        let p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Inner(through));
        (m, l, vec![p])
    }

    // Fails if a line leaving a frame does not run from its exit through the inner points and the gate, in that order.
    #[test]
    fn a_line_leaving_a_frame_runs_inner_points_then_gate() {
        let (m, l, plans) = leaving(&[]);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(drawn(wire(&w, "a", "b"), &l), [(60, 130), (150, 130), (250, 130), (450, 130)]);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
    }

    /// The drawn path keeps the gate and the inner points in order, and stays clear of the icon `x`.
    fn assert_kept_inner_round(w: &Wire, l: &Layout, kept: &[(i32, i32)]) {
        let path = drawn(w, l);
        let at = path.windows(kept.len()).position(|s| s == kept).unwrap_or_else(|| panic!("{kept:?} not in {path:?}"));
        assert!(at > 0 || kept[0] == path[0], "{path:?}");
        let x = l.rect("x");
        let full = full_path(w, l);
        assert!(full.windows(2).all(|s| !segment_hits_rect(s[0], s[1], x)), "{full:?}");
    }

    // Fails if the outer part of a line kept inside is not searched when its draft meets an icon, in either direction.
    #[test]
    fn the_outer_part_of_a_line_kept_inside_is_searched_round_an_icon() {
        // Entering: x between a and the gate (200, 130).
        let (m, mut l, plans) = crossing();
        l.icons.push(Icon { id: "x".to_string(), label: String::new(), lines: 1, rect: Rect { x: 100, y: 100, w: 60, h: 60 }, parent: Some("region".to_string()) });
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.entry.side, ab.entry.pos), (Side::Right, Side::Left, 50));
        assert_kept_inner_round(ab, &l, &[(200, 130), (300, 130), (400, 130)]);
        assert!(notes.is_empty(), "{notes:?}");
        // Leaving: x between the gate (250, 130) and b.
        let (m, l, plans) = leaving(&[("x", 330, 100, "")]);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.side, ab.exit.pos, ab.entry.side), (Side::Right, 50, Side::Left));
        assert_kept_inner_round(ab, &l, &[(60, 130), (150, 130), (250, 130)]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if the outer part of a line kept inside, which no search can draw, moves its outer end to another side or goes without a note.
    #[test]
    fn the_outer_part_no_search_can_draw_is_kept_as_planned_and_named() {
        // Entering: w touches a's right side, where the outer part starts.
        let (m, mut l, plans) = crossing();
        l.icons.push(Icon { id: "w".to_string(), label: String::new(), lines: 1, rect: Rect { x: 60, y: 100, w: 60, h: 60 }, parent: Some("region".to_string()) });
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(drawn(wire(&w, "a", "b"), &l), [(60, 130), (200, 130), (300, 130), (400, 130)]);
        assert!(notes.contains(&unrouted("edge-a-to-b")), "{notes:?}");
        // Leaving: w touches b's left side, where the outer part ends.
        let (m, l, plans) = leaving(&[("w", 390, 100, "")]);
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(drawn(wire(&w, "a", "b"), &l), [(60, 130), (150, 130), (250, 130), (450, 130)]);
        assert!(notes.contains(&unrouted("edge-a-to-b")), "{notes:?}");
    }

    // Fails if straightening moves an end of a line whose ends are both fixed at different positions.
    #[test]
    fn straightening_leaves_a_line_fixed_at_both_ends() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let mut p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Points(vec![(180, 15), (180, 45)]), Keep::Whole);
        (p.exit_pos, p.entry_pos) = (Some(25), Some(75));
        let (w, notes) = route_planned(&m, &l, &Rules::builtin(), &[p]);
        let ab = wire(&w, "a", "b");
        assert_eq!((ab.exit.pos, ab.entry.pos), (25, 75));
        assert!(notes.is_empty(), "{notes:?}");
    }

    // Fails if the final check does not look at an upright track.
    #[test]
    fn audit_finds_a_line_off_an_upright_track() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 200, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let mut p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Points(vec![(150, 30), (150, 230)]), Keep::Free);
        p.tracks = vec![Track { horizontal: false, from: 50, to: 210, at: 150 }];
        let plans = vec![p];
        let (mut w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(drawn(&w[0], &l), [(60, 30), (150, 30), (150, 230), (300, 230)]);
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
        w[0].points = vec![(160, 30), (160, 230)];
        assert_eq!(audit(&m, &l, &w, &[], Measure::Flow), Vec::<String>::new(), "the moved line overlaps nothing");
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    // Fails if the final check takes a track as kept when the line leaves it and comes back, leaving a gap.
    #[test]
    fn audit_finds_a_gap_in_a_track() {
        let (m, l) = scene(&[("a", 0, 0, S3), ("b", 300, 0, S3)], &[("a", "b", Rel::Main, HTTPS)]);
        let mut p = keep_draft(&m, 0, Side::Right, Side::Left, Bend::Straight, Keep::Free);
        p.tracks = vec![Track { horizontal: true, from: 80, to: 280, at: 30 }];
        let plans = vec![p];
        let (mut w, _) = route_planned(&m, &l, &Rules::builtin(), &plans);
        assert_eq!(audit(&m, &l, &w, &plans, Measure::Flow), Vec::<String>::new());
        // At 30 from 60 to 100 and from 200 to 300: the band's 100 to 200 is left.
        w[0].points = vec![(100, 30), (100, 80), (200, 80), (200, 30)];
        assert_eq!(audit(&m, &l, &w, &[], Measure::Flow), Vec::<String>::new(), "the line overlaps nothing");
        assert!(audit(&m, &l, &w, &plans, Measure::Flow).contains(&"edge-a-to-b".to_string()));
    }

    /// The built-in rules with the services `from` renamed `to`, and `model` with its parts of
    /// those services renamed the same way.
    fn renamed(model: &Model, names: &[(&str, &str)]) -> (Model, Rules) {
        let mut services = include_str!("../../../rules/aws/services.toml").to_string();
        let mut m = model.clone();
        for &(from, to) in names {
            let key = format!("key = \"{from}\"");
            assert!(services.contains(&key), "no service {from}");
            services = services.replacen(&key, &format!("key = \"{to}\""), 1);
            for n in m.nodes.iter_mut().filter(|n| n.service == from) {
                n.service = to.to_string();
            }
        }
        let r = Rules::load(
            &services,
            include_str!("../../../rules/aws/rules.toml"),
            include_str!("../../../rules/aws/labels.toml"),
            include_str!("../../../rules/aws/links.toml"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        (m, r)
    }

    // Fails if where a label sits on a line out of the entry gateway is decided by the gateway's service name.
    #[test]
    fn a_label_leaving_the_entry_gateway_sits_near_it_whatever_its_key() {
        let src = crate::test_inputs::WEB_CASES.iter().find(|c| c.0 == "web-2az-ja").unwrap().1;
        let r = Rules::builtin();
        let (m, d) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        let gate: Vec<String> = m.nodes.iter().filter(|n| r.service(&n.service).and_then(|s| s.derived_role) == Some(crate::rules::DerivedRole::EntryGateway)).map(|n| n.id.clone()).collect();
        assert_eq!(gate.len(), 1);
        let leaving = |m: &Model, r: &Rules| {
            let axis = m.direction.unwrap_or_else(|| crate::arrange::default_axis(m));
            let p = crate::arrange::arrange(m, r, axis).unwrap_or_else(|e| panic!("{e:?}"));
            let (w, _) = route_planned(m, &p.layout, r, &p.lines);
            w.into_iter().filter(|x| x.from == gate[0]).map(|x| (x.id, x.label_at, x.label_near_source)).collect::<Vec<_>>()
        };
        let before = leaving(&m, &r);
        assert!(!before.is_empty());
        assert!(before.iter().all(|x| x.2), "{before:?}");
        let (m2, r2) = renamed(&m, &[("igw", "gate")]);
        assert_eq!(leaving(&m2, &r2), before);
    }


}
