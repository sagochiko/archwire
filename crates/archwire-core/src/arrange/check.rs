//! The must-hold conditions on a finished placement.

use super::layered::{CLOUD_FRAME, REGION_FRAME, ROW_STEP};
use super::Placement;
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
