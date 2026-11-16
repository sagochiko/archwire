//! The must-hold conditions on a finished placement.

use super::layered::{CLOUD_FRAME, REGION_FRAME, ROW_STEP};
use super::{Lane, Placement};
use crate::route::{edge_id, Bend, Keep, Side, Track};
use crate::diag::Problem;
use crate::layout::Rect;
use crate::measure::{frame_name_box, name_area};
use crate::model::{Area, Model};

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
        let r = crate::rules::Rules::builtin();
        let src = include_str!("../../../../tests/cases/web-2az-ja/def.yaml");
        let m = crate::infer::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap();
        let p = super::super::arrange(&m, &r, crate::model::Axis::Down).unwrap();
        let reps = crate::lines::gather(&m, &p.layout, &r).0.iter().map(|l| l.rep).collect();
        (m, p, reps)
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
