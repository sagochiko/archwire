//! The must-hold conditions on a finished placement.

use super::azgrid::{ROOM, VPC_FRAME};
use super::layered::{CLOUD_FRAME, REGION_FRAME, ROW_STEP};
use super::{Lane, Placement};
use crate::def::Rel;
use crate::route::{edge_id, port_point, Bend, Keep, Port, Side, Track, LABEL_FIRST, LABEL_LINE};
use crate::diag::Problem;
use crate::layout::{FrameKind, Icon, Layout, Rect};
use crate::measure::{frame_name_box, name_area};
use crate::model::{Area, Axis, Model, Tier};

/// The must-hold conditions on a finished placement; empty when all hold.
pub(crate) fn check(model: &Model, placement: &Placement) -> Vec<Problem> {
    let layout = &placement.layout;
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
    // The frame a part sits in is the one the model gives it: inside the VPC, the subnet of its
    // tier in its zone, and the VPC itself for the entry gateway and for parts spanning the zones.
    for n in &model.nodes {
        let want = match (n.area, n.az) {
            (Area::Outside, _) => None,
            (Area::Cloud, _) => Some(CLOUD_FRAME.to_string()),
            (Area::Region, _) => Some(REGION_FRAME.to_string()),
            (Area::VpcEdge, _) | (Area::Subnet(_), None) => Some(VPC_FRAME.to_string()),
            (Area::Subnet(tier), Some(k)) => Some(subnet_id(model, tier, k)),
        };
        if let Some(i) = icons.iter().find(|i| i.id == n.id) {
            if i.parent != want {
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
    // No part or name lies over a frame's name, a zone's and a subnet's too.
    for f in &layout.frames {
        let name = frame_name_box(f);
        for (i, label) in icons.iter().zip(&labels) {
            if overlaps(i.rect, name) || overlaps(*label, name) {
                out.push(failed(&i.id, format!("{} and {}", i.id, f.id), "overlapping"));
            }
        }
    }
    out.extend(zones_kept(model, layout));
    out.extend(lanes_kept(model, placement));
    out.extend(gates_kept(model, placement));
    out.extend(along_the_axis(model, placement));
    out
}

/// The id of the subnet frame of a tier in zone `k`: `pub-1a`, `pri-1c`.
fn subnet_id(model: &Model, tier: Tier, k: usize) -> String {
    let zone = &model.azs[k];
    let prefix = match tier {
        Tier::Public => "pub",
        Tier::Private => "pri",
    };
    format!("{prefix}-{}", &zone[zone.len() - 2..])
}

fn zone_id(model: &Model, k: usize) -> String {
    let zone = &model.azs[k];
    format!("az-{}", &zone[zone.len() - 2..])
}

/// Inside a VPC: a part's copies at one height and one place in their zones, the zones of one
/// size, and a subnet tier drawn in every zone when it has parts, in none when it has none.
fn zones_kept(model: &Model, layout: &Layout) -> Vec<Problem> {
    let mut out = Vec::new();
    let frame = |id: &str| layout.frames.iter().find(|f| f.id == id).map(|f| f.rect);
    let icon = |id: &str| layout.icons.iter().find(|i| i.id == id);
    let mut parts: Vec<&str> = Vec::new();
    for n in model.nodes.iter().filter(|n| n.az.is_some()) {
        if !parts.contains(&n.part.as_str()) {
            parts.push(&n.part);
        }
    }
    for part in parts {
        let mut first: Option<(&str, (i32, i32))> = None;
        for n in model.nodes.iter().filter(|n| n.part == part) {
            let (Some(k), Some(i)) = (n.az, icon(&n.id)) else { continue };
            let Some(z) = frame(&zone_id(model, k)) else { continue };
            let at = (i.rect.y, i.rect.x - z.x);
            match first {
                None => first = Some((&n.id, at)),
                Some((one, there)) if there != at => out.push(failed(&n.id, format!("{one} and {}", n.id), "keeping its copies level")),
                Some(_) => {}
            }
        }
    }
    let zones: Vec<&crate::layout::Frame> = layout.frames.iter().filter(|f| f.kind == FrameKind::Az).collect();
    if let Some(one) = zones.first() {
        for z in zones.iter().filter(|z| (z.rect.w, z.rect.h) != (one.rect.w, one.rect.h)) {
            out.push(failed(&z.id, format!("{} and {}", one.id, z.id), "zones of one size"));
        }
    }
    if !zones.is_empty() {
        for (tier, kind) in [(Tier::Public, FrameKind::PublicSubnet), (Tier::Private, FrameKind::PrivateSubnet)] {
            let has_parts = model.nodes.iter().any(|n| n.area == Area::Subnet(tier));
            for k in 0..model.azs.len() {
                let id = subnet_id(model, tier, k);
                if has_parts && frame(&id).is_none() {
                    out.push(failed(&id, id.clone(), "a frame for the parts in its tier"));
                }
            }
            if !has_parts {
                for f in layout.frames.iter().filter(|f| f.kind == kind) {
                    out.push(failed(&f.id, f.id.clone(), "parts in its tier"));
                }
            }
        }
    }
    out
}

/// Each drawn line's track in a band lies in the band and keeps room from the others.
fn lanes_kept(model: &Model, placement: &Placement) -> Vec<Problem> {
    let mut out = Vec::new();
    let line = |e: usize| edge_id(&model.edges[e].from, &model.edges[e].to);
    for lane in &placement.lanes {
        for (k, &(e, at)) in lane.tracks.iter().enumerate() {
            if at < lane.lo || lane.hi < at {
                out.push(failed(&line(e), line(e), "its track inside its band"));
            }
            for &(o, other) in &lane.tracks[k + 1..] {
                if (at - other).abs() < ROOM {
                    out.push(failed(&line(e), format!("{} and {}", line(e), line(o)), "room in its band"));
                }
            }
        }
    }
    out
}

/// Where a line meets the side of its icon, below the name on the bottom side, as the router puts it.
fn port_of(i: &Icon, side: Side, pos: u8) -> (i32, i32) {
    let offset = if side == Side::Bottom { LABEL_FIRST + LABEL_LINE * (i.lines as i32 - 1) } else { 0 };
    port_point(i.rect, Port { side, pos, offset })
}

/// A crossing line's gate lies on a side of its frame, and the line crosses the side there at a
/// right angle: the stretch of its inner part next to the gate runs straight away from the side.
fn gates_kept(model: &Model, placement: &Placement) -> Vec<Problem> {
    let layout = &placement.layout;
    let mut out = Vec::new();
    for c in &placement.crossings {
        let e = &model.edges[c.edge];
        let line = edge_id(&e.from, &e.to);
        let Some(f) = layout.frames.iter().find(|f| f.id == c.frame).map(|f| f.rect) else {
            out.push(failed(&line, line.clone(), "its gate on the side of a frame"));
            continue;
        };
        let (gx, gy) = c.through.gate;
        let on_x = (gx == f.x || gx == f.x + f.w) && f.y < gy && gy < f.y + f.h;
        let on_y = (gy == f.y || gy == f.y + f.h) && f.x < gx && gx < f.x + f.w;
        if on_x == on_y {
            out.push(failed(&line, line.clone(), &format!("its gate on the side of {}", c.frame)));
            continue;
        }
        let t = &c.through;
        let end = if t.inner_is_start { &e.from } else { &e.to };
        let next = if t.inner_is_start { t.inner.last() } else { t.inner.first() };
        let next = match (next, layout.icons.iter().find(|i| i.id == *end)) {
            (Some(&p), _) => p,
            (None, Some(i)) => port_of(i, c.inner_side, c.inner_pos),
            (None, None) => continue,
        };
        let square = if on_x { next.1 == gy && next.0 != gx } else { next.0 == gx && next.1 != gy };
        if !square {
            out.push(failed(&line, line.clone(), &format!("crossing the side at its gate on {}", c.frame)));
        }
    }
    out
}

/// A main line between two parts outside every VPC runs along its axis: from left to right when
/// lines run right, from top to bottom when they run down. Lines with an end inside a VPC are
/// laid out by the VPC's own arrangement.
fn along_the_axis(model: &Model, placement: &Placement) -> Vec<Problem> {
    let layout = &placement.layout;
    let inside = |id: &str| model.nodes.iter().find(|n| n.id == id).is_none_or(|n| matches!(n.area, Area::VpcEdge | Area::Subnet(_)));
    let mut out = Vec::new();
    for pl in &placement.lines {
        let e = &model.edges[pl.edge];
        if e.rel != Rel::Main || inside(&e.from) || inside(&e.to) {
            continue;
        }
        let (Some(a), Some(b)) = (layout.icons.iter().find(|i| i.id == e.from), layout.icons.iter().find(|i| i.id == e.to)) else { continue };
        let ahead = match pl.axis {
            Axis::Right => a.rect.x < b.rect.x,
            Axis::Down => a.rect.y < b.rect.y,
        };
        if !ahead {
            let line = edge_id(&e.from, &e.to);
            out.push(failed(&line, line.clone(), "running along the axis"));
        }
    }
    out
}

/// The ends a drawn line's draft fixes keep a spot of their own: no two of them sit at one
/// position on one side of one part. Only the drafts of the lines drawn (`drawn`, the
/// representatives of the gathered lines) count; the router never draws the others.
pub(crate) fn fixed_ends_apart(model: &Model, placement: &Placement, drawn: &[usize]) -> Vec<Problem> {
    let mut spots: Vec<((String, Side, u8), Vec<String>)> = Vec::new();
    for pl in placement.lines.iter().filter(|pl| drawn.contains(&pl.edge)) {
        let (exit, entry) = pl.fixed();
        let ends = [exit.map(|p| (pl.from.clone(), pl.exit, p)), entry.map(|p| (pl.to.clone(), pl.entry, p))];
        let e = &model.edges[pl.edge];
        for spot in ends.into_iter().flatten() {
            match spots.iter_mut().find(|(s, _)| *s == spot) {
                Some((_, lines)) => lines.push(edge_id(&e.from, &e.to)),
                None => spots.push((spot, vec![edge_id(&e.from, &e.to)])),
            }
        }
    }
    spots
        .into_iter()
        .filter(|(_, lines)| lines.len() > 1)
        .map(|((part, _, _), lines)| failed(&lines[0], lines.join(" and "), &format!("a spot of their own on a side of {part}")))
        .collect()
}

/// A drawn line whose draft runs along a band keeps a track there: the stretch is in its
/// `tracks` and its band holds it. `bands` are the bands as laid out, before the tracks of the
/// lines not drawn are taken off. A line drawn that was not expected to be shows up here.
pub(crate) fn bands_kept(model: &Model, placement: &Placement, bands: &[Lane], drawn: &[usize]) -> Vec<Problem> {
    let mut out = Vec::new();
    for pl in placement.lines.iter().filter(|pl| drawn.contains(&pl.edge)) {
        let points: Vec<(i32, i32)> = match &pl.keep {
            Keep::Whole => match &pl.bend {
                Bend::Points(v) => v.clone(),
                _ => Vec::new(),
            },
            Keep::Inner(t) if t.inner_is_start => t.inner.iter().copied().chain([t.gate]).collect(),
            Keep::Inner(t) => [t.gate].into_iter().chain(t.inner.iter().copied()).collect(),
            Keep::Free => Vec::new(),
        };
        for s in points.windows(2).filter(|s| s[0].1 == s[1].1 && s[0].0 != s[1].0) {
            let (from, to, at) = (s[0].0.min(s[1].0), s[0].0.max(s[1].0), s[0].1);
            if !bands.iter().any(|b| b.horizontal && b.lo <= at && at <= b.hi && from < b.to && b.from < to) {
                continue;
            }
            let in_plan = pl.tracks.contains(&Track { horizontal: true, from, to, at });
            let in_band = placement.lanes.iter().any(|l| l.tracks.contains(&(pl.edge, at)));
            if !(in_plan && in_band) {
                let e = &model.edges[pl.edge];
                let line = edge_id(&e.from, &e.to);
                out.push(failed(&line, line.clone(), "a track in its band"));
            }
        }
    }
    out
}

pub(super) fn failed(at: &str, got: String, what: &str) -> Problem {
    let message = format!("archwire could not lay out {got} without {what}; this is a bug in archwire, please report the definition");
    Problem { code: "LAYOUT_FAILED".into(), at: Some(at.into()), got: Some(got), allowed: Vec::new(), rule: None, message }
}

pub(super) fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

fn inside(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::layered::tests::{placed, TARGET};
    use crate::layout::{Icon, Layout};
    use crate::model::Axis;

    /// The target layout, with `change` applied to it by hand.
    fn broken(change: impl Fn(&mut Layout)) -> Vec<Problem> {
        let (m, p) = placed(TARGET);
        let mut l = p.layout.clone();
        change(&mut l);
        check(&m, &Placement { layout: l, ..p })
    }
    fn icon_mut<'a>(l: &'a mut Layout, id: &str) -> &'a mut Icon {
        l.icons.iter_mut().find(|i| i.id == id).unwrap()
    }
    fn check_layout(m: &Model, p: &Placement, l: Layout) -> Vec<Problem> {
        check(m, &Placement { layout: l, ..p.clone() })
    }

    /// A layout with a VPC: the web example running down, and its representative lines.
    fn with_a_vpc() -> (Model, Placement, Vec<usize>) {
        let (m, p) = web_along(Axis::Down);
        let reps = crate::lines::gather(&m, &p.layout, &crate::rules::Rules::builtin()).0.iter().map(|l| l.rep).collect();
        (m, p, reps)
    }
    /// The web example with two zones, laid out along `axis`.
    fn web_along(axis: Axis) -> (Model, Placement) {
        let r = crate::rules::Rules::builtin();
        let src = include_str!("../../../../tests/cases/web-2az-ja/def.yaml");
        let m = crate::infer::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap();
        let p = super::super::arrange(&m, &r, axis).unwrap();
        (m, p)
    }
    /// The smallest model mixing Lambda in the VPC with parts outside it: no public tier.
    fn mix() -> (Model, Placement) {
        let m = super::super::azgrid::tests::smallest_mix("lambda");
        let p = super::super::arrange(&m, &crate::rules::Rules::builtin(), Axis::Down).unwrap();
        (m, p)
    }
    fn frame_rect(p: &Placement, id: &str) -> Rect {
        p.layout.frames.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("no frame {id}")).rect
    }
    /// The web example running down, with `change` applied to its placement by hand.
    fn web_broken(change: impl Fn(&mut Placement)) -> Vec<Problem> {
        let (m, mut p) = web_along(Axis::Down);
        assert_eq!(check(&m, &p), Vec::new());
        change(&mut p);
        check(&m, &p)
    }
    fn says(got: &[Problem], at: &str, what: &str) -> bool {
        got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.at.as_deref() == Some(at) && e.message.contains(what))
    }

    // Fails if the check does not see one copy of a part at another height than the others.
    #[test]
    fn check_finds_a_copy_out_of_its_zone_row() {
        let got = web_broken(|p| icon_mut(&mut p.layout, "ecs-1c").rect.y += 10);
        assert!(got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.got.as_deref().is_some_and(|g| g.contains("ecs-1a") && g.contains("ecs-1c"))), "{got:?}");
        assert!(got.iter().any(|e| e.message.contains("its copies level")), "{got:?}");
    }

    // Fails if the check does not see zones of different sizes.
    #[test]
    fn check_finds_zones_of_different_sizes() {
        let got = web_broken(|p| p.layout.frames.iter_mut().find(|f| f.id == "az-1c").unwrap().rect.w -= 10);
        assert!(says(&got, "az-1c", "zones of one size"), "{got:?}");
    }

    // Fails if the check does not see a subnet frame with nothing in its tier.
    #[test]
    fn check_finds_an_empty_subnet_frame() {
        let (m, mut p) = mix();
        assert_eq!(check(&m, &p), Vec::new());
        let (az, pri) = (frame_rect(&p, "az-1a"), frame_rect(&p, "pri-1a"));
        // Below the private subnet, inside the zone: clear of every icon and name.
        let rect = Rect { x: pri.x, y: pri.y + pri.h + 2, w: pri.w, h: az.y + az.h - (pri.y + pri.h) - 4 };
        p.layout.frames.push(crate::layout::Frame { id: "pub-1a".into(), kind: crate::layout::FrameKind::PublicSubnet, label: "Public".into(), rect, parent: Some("az-1a".into()) });
        let got = check(&m, &p);
        assert!(says(&got, "pub-1a", "parts in its tier"), "{got:?}");
    }

    // Fails if the check does not compare the subnet a part sits in with its tier and zone inside the VPC.
    #[test]
    fn check_finds_a_part_in_the_wrong_subnet() {
        let got = web_broken(|p| {
            let sub = frame_rect(p, "pub-1a");
            let i = icon_mut(&mut p.layout, "ecs-1a");
            i.parent = Some("pub-1a".into());
            i.rect.x = sub.x + 80;
            i.rect.y = sub.y + 40;
            assert!(inside(sub, i.rect) && inside(sub, name_area(i)), "{sub:?} {i:?}");
        });
        assert!(says(&got, "ecs-1a", "leaving its frame"), "{got:?}");
    }

    // Fails if the check does not see a part over the name of a zone's frame.
    #[test]
    fn check_finds_a_name_over_a_zone_name() {
        let got = web_broken(|p| {
            let az = frame_rect(p, "az-1a");
            let i = icon_mut(&mut p.layout, "ecs-1a");
            i.rect.x = az.x + 2;
            i.rect.y = az.y + 2;
        });
        assert!(got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.got.as_deref().is_some_and(|g| g.contains("az-1a"))), "{got:?}");
    }

    // Fails if the check does not see two lines on one height in a band.
    #[test]
    fn check_finds_two_tracks_too_close() {
        let got = web_broken(|p| {
            let lane = p.lanes.iter_mut().find(|l| l.tracks.len() >= 2).expect("a band with two tracks");
            lane.tracks[1].1 = lane.tracks[0].1;
        });
        assert!(got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.message.contains("room in its band")), "{got:?}");
    }

    // Fails if the check does not see a gate off the side of its frame.
    #[test]
    fn check_finds_a_gate_off_its_side() {
        let got = web_broken(|p| {
            let v = frame_rect(p, "vpc");
            let gate = &mut p.crossings[0].through.gate;
            if gate.0 == v.x || gate.0 == v.x + v.w {
                gate.0 += if gate.0 == v.x { 10 } else { -10 };
            } else {
                gate.1 += if gate.1 == v.y { 10 } else { -10 };
            }
        });
        assert!(got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.message.contains("its gate on the side")), "{got:?}");
    }

    // Fails if the check does not see a line running along the side of its frame at its gate.
    #[test]
    fn check_finds_a_line_running_along_the_side_at_its_gate() {
        let got = web_broken(|p| {
            let v = frame_rect(p, "vpc");
            let t = &mut p.crossings[0].through;
            let (gx, gy) = t.gate;
            let along = if gx == v.x || gx == v.x + v.w { (gx, gy + 20) } else { (gx + 20, gy) };
            if t.inner_is_start {
                t.inner.push(along);
            } else {
                t.inner.insert(0, along);
            }
        });
        assert!(got.iter().any(|e| e.code == "LAYOUT_FAILED" && e.message.contains("crossing the side at its gate")), "{got:?}");
    }

    // Fails if the check does not see a main line outside the VPC running against the axis.
    #[test]
    fn check_finds_a_main_line_against_the_axis() {
        let (m, mut p) = web_along(Axis::Right);
        assert_eq!(check(&m, &p), Vec::new());
        let (u, c) = (p.layout.rect("user").x, p.layout.rect("cloudfront").x);
        icon_mut(&mut p.layout, "user").rect.x = c;
        icon_mut(&mut p.layout, "cloudfront").rect.x = u;
        let got = check(&m, &p);
        assert!(says(&got, "edge-user-to-cloudfront", "running along the axis"), "{got:?}");
    }

    // Fails if a drawn line that runs along a band is not found when its draft or its band has lost its track.
    #[test]
    fn check_finds_a_drawn_line_without_its_track() {
        let (m, p, reps) = with_a_vpc();
        assert_eq!(bands_kept(&m, &p, &p.lanes, &reps), Vec::new());
        let k = p.lines.iter().position(|l| !l.tracks.is_empty() && reps.contains(&l.edge)).expect("a drawn line on a track");
        let e = &m.edges[p.lines[k].edge];
        let id = edge_id(&e.from, &e.to);
        let mut draft = p.clone();
        draft.lines[k].tracks.clear();
        let got = bands_kept(&m, &draft, &p.lanes, &reps);
        assert!(got.iter().any(|x| x.at.as_deref() == Some(id.as_str()) && x.message.contains("a track in its band")), "{got:?}");
        let mut band = p.clone();
        band.lanes.iter_mut().for_each(|l| l.tracks.retain(|t| t.0 != p.lines[k].edge));
        let got = bands_kept(&m, &band, &p.lanes, &reps);
        assert!(got.iter().any(|x| x.at.as_deref() == Some(id.as_str())), "{got:?}");
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
        let got = check_layout(&m, &p, l);
        assert!(got.iter().any(|e| e.got.as_deref() == Some("lambda.a and lambda.b") && e.message.contains("without keeping rows apart")), "{got:?}");
    }

    #[test]
    fn check_finds_a_part_over_a_frame_name() {
        let (m, p) = placed("flows:\n  - user -> apigateway(http)\n");
        let mut l = p.layout.clone();
        let f = l.rect("region");
        icon_mut(&mut l, "apigateway").rect = Rect { x: f.x + 5, y: f.y + 2, w: 60, h: 60 };
        let got = check_layout(&m, &p, l);
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
}
