//! Gathering semantic edges into the lines to draw, and the parts of each line's label.
//!
//! Edges are gathered in two steps, and every edge ends in exactly one line. The first step
//! merges the lines from the per-AZ copies of one part. The second merges side lines into one
//! part from parts of the same service; it only joins the groups of the first step, so no edge
//! is counted twice.

use crate::def::Rel;
use crate::layout::Layout;
use crate::model::{Area, Model, Node};
use crate::rules::{Rules, Scope};

/// One definition part a line comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Source {
    pub part: String,
    /// The AZs its sending copies are in, in order; empty for a part with no copies.
    pub azs: Vec<usize>,
}

/// One line to draw and the semantic edges it stands for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line {
    /// The representative semantic edge: one of `edges`; its draft is used.
    pub rep: usize,
    /// The source of `rep`.
    pub from: String,
    pub to: String,
    pub rel: Rel,
    /// The hole id; lines merge only with the same id.
    pub hole: Option<String>,
    /// Through a VPC endpoint: the label says so.
    pub endpoint: bool,
    pub label_key: String,
    /// Indexes into `Model.edges`, in model order.
    pub edges: Vec<usize>,
    /// Per definition part, in model order.
    pub sources: Vec<Source>,
}

/// Edges merged by the first step, in model order.
struct Group {
    edges: Vec<usize>,
    /// The copies of one part merged into a per-AZ part: drawn by the line inside the target's AZ.
    across: bool,
}

/// Every edge of the model in exactly one line; notes for merged side lines and for cross-AZ merges.
///
/// First step: lines from the per-AZ copies of one part merge when they go to a Regional service,
/// or, with three AZs, to the per-AZ copies of another service. They merge only with the same
/// part, target, relation, hole and endpoint route.
///
/// Second step: auxiliary and association lines into one part merge when they come from the same
/// service with the same label, are not holes, and their senders all sit in subnets or all outside them.
///
/// The line is drawn from the sender nearest the target (centre to centre, across plus down, then
/// the first in the model). Lines merged into a per-AZ part are drawn from the copy in the target's
/// AZ, or, when that AZ has none, from the first edge's sender, with a note saying so.
pub(crate) fn gather(model: &Model, layout: &Layout, rules: &Rules) -> (Vec<Line>, Vec<String>) {
    let lang = model.lang;
    let node = |id: &str| -> &Node { model.nodes.iter().find(|n| n.id == id).expect("edge ends are nodes") };
    let scope = |n: &Node| rules.service(&n.service).map(|s| s.scope).expect("model services are in the rules");

    let mut groups: Vec<Group> = Vec::new();
    let mut keys: Vec<Option<usize>> = Vec::new(); // the edge whose key a mergeable group has
    for (i, e) in model.edges.iter().enumerate() {
        let (f, t) = (node(&e.from), node(&e.to));
        let merge = f.az.is_some()
            && (scope(t) == Scope::Regional || (model.azs.len() > 2 && t.az.is_some() && f.service != t.service));
        let same = |k: usize| {
            let g = &model.edges[k];
            node(&g.from).part == f.part && g.to == e.to && g.rel == e.rel && g.hole == e.hole && g.endpoint == e.endpoint
        };
        match keys.iter().position(|k| merge && k.is_some_and(|k| same(k))) {
            Some(at) => groups[at].edges.push(i),
            None => {
                groups.push(Group { edges: vec![i], across: false });
                keys.push(merge.then_some(i));
            }
        }
    }
    let mut merged_across_azs = false;
    for g in groups.iter_mut().filter(|g| g.edges.len() > 1) {
        if scope(node(&model.edges[g.edges[0]].to)) != Scope::Regional {
            g.across = true;
            merged_across_azs = true;
        }
    }

    // Second step, over the groups of the first.
    let in_subnet = |n: &Node| matches!(n.area, Area::Subnet(_));
    let side_key = |g: &Group| {
        let e = &model.edges[g.edges[0]];
        (e.rel != Rel::Main && e.hole.is_none()).then(|| {
            let f = node(&e.from);
            (e.to.as_str(), e.rel, e.label.as_str(), e.endpoint, f.service.as_str(), in_subnet(f))
        })
    };
    let mut bundles: Vec<Vec<usize>> = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let key = side_key(g);
        match bundles.iter_mut().find(|b| key.is_some() && side_key(&groups[b[0]]) == key) {
            Some(b) => b.push(gi),
            None => bundles.push(vec![gi]),
        }
    }

    let center = |id: &str| layout.icons.iter().find(|i| i.id == id).map(|i| (i.rect.x + i.rect.w / 2, i.rect.y + i.rect.h / 2));
    let distance = |from: &str, to: &str| match (center(from), center(to)) {
        (Some(a), Some(b)) => (a.0 - b.0).abs() + (a.1 - b.1).abs(),
        _ => i32::MAX,
    };
    let mut lines = Vec::new();
    let mut line_notes = Vec::new();
    let sep = rules.text("parts_separator", lang);
    for b in bundles {
        let mut edges: Vec<usize> = b.iter().flat_map(|&gi| groups[gi].edges.iter().copied()).collect();
        edges.sort_unstable();
        let first = &model.edges[edges[0]];
        let to = node(&first.to);
        let across = b.iter().any(|&gi| groups[gi].across);
        let rep = if across {
            match edges.iter().find(|&&e| node(&model.edges[e].from).az == to.az) {
                Some(&e) => e,
                None => {
                    let (zone, service) = (to.az.map(|a| model.azs[a].as_str()).unwrap_or_default(), &node(&first.from).service);
                    line_notes.push(
                        rules
                            .text("note.cross_az_fallback", lang)
                            .replace("{zone}", zone)
                            .replace("{to}", &to.id)
                            .replace("{service}", service)
                            .replace("{from}", &first.from),
                    );
                    edges[0]
                }
            }
        } else {
            *edges.iter().min_by_key(|&&e| (distance(&model.edges[e].from, &first.to), e)).expect("a line has an edge")
        };
        let mut sources: Vec<Source> = Vec::new();
        for &e in &edges {
            let f = node(&model.edges[e].from);
            let at = match sources.iter().position(|s| s.part == f.part) {
                Some(at) => at,
                None => {
                    sources.push(Source { part: f.part.clone(), azs: Vec::new() });
                    sources.len() - 1
                }
            };
            sources[at].azs.extend(f.az);
        }
        for s in &mut sources {
            s.azs.sort_unstable();
            s.azs.dedup();
        }
        let r = &model.edges[rep];
        if sources.len() > 1 {
            let parts: Vec<&str> = sources.iter().map(|s| s.part.as_str()).collect();
            line_notes.push(
                rules
                    .text("note.merged_side_lines", lang)
                    .replace("{from}", &r.from)
                    .replace("{to}", &r.to)
                    .replace("{parts}", &parts.join(&sep)),
            );
        }
        lines.push(Line {
            rep,
            from: r.from.clone(),
            to: r.to.clone(),
            rel: r.rel,
            hole: r.hole.clone(),
            endpoint: r.endpoint,
            label_key: r.label.clone(),
            edges,
            sources,
        });
    }
    let mut notes = Vec::new();
    if merged_across_azs {
        notes.push(rules.text("note.cross_az_merged", lang));
    }
    notes.extend(line_notes);
    (lines, notes)
}

/// The label's parts: the label, "(via VPC endpoint)", the AZ note, the sources note. They read
/// on one line joined by a space, or one per line where one line does not fit.
///
/// The AZ note names the AZs the senders' copies are in when there are two or more of them:
/// "(all AZs)" when they are every AZ of the model, the AZs' last two letters otherwise.
pub(crate) fn label_parts(line: &Line, model: &Model, rules: &Rules) -> Vec<String> {
    let lang = model.lang;
    if line.hole.is_some() {
        return vec![rules.text("undecided_route", lang)];
    }
    let mut parts = vec![rules.text(&line.label_key, lang)];
    if line.endpoint {
        parts.push(rules.text("via_endpoint", lang));
    }
    let mut azs: Vec<usize> = line.sources.iter().flat_map(|s| s.azs.iter().copied()).collect();
    azs.sort_unstable();
    azs.dedup();
    if azs.len() > 1 {
        if azs.len() == model.azs.len() {
            parts.push(rules.text("from_all_azs", lang));
        } else {
            let names: Vec<&str> = azs.iter().map(|&a| last_two(&model.azs[a])).collect();
            parts.push(rules.text("from_azs", lang).replace("{azs}", &names.join(&rules.text("parts_separator", lang))));
        }
    }
    if line.sources.len() > 1 {
        parts.push(rules.text("from_sources", lang).replace("{n}", &line.sources.len().to_string()));
    }
    parts.retain(|p| !p.is_empty());
    parts
}

/// The last two characters of an AZ name: "1a" for "ap-northeast-1a".
fn last_two(name: &str) -> &str {
    let at = name.char_indices().rev().nth(1).map_or(0, |(i, _)| i);
    &name[at..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Icon, Layout, Rect};
    use crate::model::{Area, Edge, Node, Tier};
    use crate::rules::{Lang, Rules};
    use crate::test_inputs::FEWER_READERS;

    /// The fixed examples that give a diagram.
    const CASES: [(&str, &str); 17] = [
        ("api-http-ja", include_str!("../../../tests/cases/api-http-ja/def.yaml")),
        ("api-merged-ja", include_str!("../../../tests/cases/api-merged-ja/def.yaml")),
        ("api-min-en", include_str!("../../../tests/cases/api-min-en/def.yaml")),
        ("api-names-ja", include_str!("../../../tests/cases/api-names-ja/def.yaml")),
        ("api-rest-en", include_str!("../../../tests/cases/api-rest-en/def.yaml")),
        ("aux-to-side", include_str!("../../../tests/cases/aux-to-side/def.yaml")),
        ("edge-waf-en", include_str!("../../../tests/cases/edge-waf-en/def.yaml")),
        ("endpoint-3az-en", include_str!("../../../tests/cases/endpoint-3az-en/def.yaml")),
        ("endpoint-3az-ja", include_str!("../../../tests/cases/endpoint-3az-ja/def.yaml")),
        ("endpoint-ja", include_str!("../../../tests/cases/endpoint-ja/def.yaml")),
        ("hole-apigateway", include_str!("../../../tests/cases/hole-apigateway/def.yaml")),
        ("hole-s3", include_str!("../../../tests/cases/hole-s3/def.yaml")),
        ("web-2az-en", include_str!("../../../tests/cases/web-2az-en/def.yaml")),
        ("web-2az-ja", include_str!("../../../tests/cases/web-2az-ja/def.yaml")),
        ("web-3az-en", include_str!("../../../tests/cases/web-3az-en/def.yaml")),
        ("web-3az-ja", include_str!("../../../tests/cases/web-3az-ja/def.yaml")),
        ("web-attach-en", include_str!("../../../tests/cases/web-attach-en/def.yaml")),
    ];

    fn fewer_readers_en() -> String {
        FEWER_READERS.replace("  readers: 1\n", "  readers: 1\n  lang: en\n")
    }

    /// The model a definition infers and the layout it goes through: the VPC layout when the model
    /// has a part in a subnet or on the VPC's edge, the layout without a VPC otherwise.
    fn model_and_layout(src: &str, r: &Rules) -> (Model, Layout) {
        let m = crate::inference::infer(&crate::def::parse(src).unwrap(), r).0.unwrap();
        let l = if m.nodes.iter().any(|n| matches!(n.area, Area::VpcEdge | Area::Subnet(_))) {
            crate::layout::layout(&m, r)
        } else {
            crate::place::place(&m, r).unwrap().layout
        };
        (m, l)
    }

    fn gathered(src: &str) -> (Model, Vec<Line>, Vec<String>) {
        let r = Rules::builtin();
        let (m, l) = model_and_layout(src, &r);
        let (lines, notes) = gather(&m, &l, &r);
        (m, lines, notes)
    }

    fn sources_and_fewer_readers() -> Vec<(String, String)> {
        let mut all: Vec<(String, String)> = CASES.iter().map(|(n, s)| (n.to_string(), s.to_string())).collect();
        all.push(("fewer-readers".to_string(), FEWER_READERS.to_string()));
        all
    }

    fn parts_of(line: &Line, m: &Model) -> Vec<String> {
        label_parts(line, m, &Rules::builtin())
    }

    // Fails if a semantic edge is dropped from every line, or counted in two lines.
    #[test]
    fn every_edge_is_in_exactly_one_line() {
        for (name, src) in sources_and_fewer_readers() {
            let (m, lines, _) = gathered(&src);
            let mut all: Vec<usize> = lines.iter().flat_map(|l| l.edges.clone()).collect();
            all.sort_unstable();
            assert_eq!(all, (0..m.edges.len()).collect::<Vec<_>>(), "{name}");
        }
    }

    // Fails if a merged line is drawn from a sender none of its edges has.
    #[test]
    fn the_representative_is_one_of_the_merged_sources() {
        for (name, src) in sources_and_fewer_readers() {
            let (m, lines, _) = gathered(&src);
            for l in &lines {
                assert!(l.edges.contains(&l.rep), "{name}: {} -> {}", l.from, l.to);
                assert!(l.edges.iter().any(|&e| m.edges[e].from == l.from), "{name}: {} -> {}", l.from, l.to);
                assert_eq!(m.edges[l.rep].from, l.from, "{name}");
                assert_eq!(m.edges[l.rep].to, l.to, "{name}");
            }
        }
    }

    /// The hole example with the line from ecs-1c given a hole id of its own: two hole ids into s3.
    fn two_holes(r: &Rules) -> (Model, Layout) {
        let (mut m, l) = model_and_layout(CASES[11].1, r);
        let e = m.edges.iter_mut().find(|e| e.from == "ecs-1c" && e.to == "s3").unwrap();
        assert_eq!(e.hole.as_deref(), Some("route.ecs.s3"));
        e.hole = Some("route.x.s3".to_string());
        (m, l)
    }

    // Fails if edges with different labels, hole ids or endpoint routes are merged into one line.
    #[test]
    fn merged_lines_agree_on_label_hole_and_endpoint() {
        let r = Rules::builtin();
        let mut inputs: Vec<(String, Model, Vec<Line>)> = sources_and_fewer_readers()
            .into_iter()
            .map(|(name, src)| {
                let (m, lines, _) = gathered(&src);
                (name, m, lines)
            })
            .collect();
        let (m, l) = two_holes(&r);
        let (lines, _) = gather(&m, &l, &r);
        inputs.push(("two-holes".to_string(), m, lines));
        for (name, m, lines) in &inputs {
            for l in lines {
                for &e in &l.edges {
                    let edge = &m.edges[e];
                    assert_eq!(edge.label, l.label_key, "{name}: {} -> {}", edge.from, edge.to);
                    assert_eq!(edge.hole, l.hole, "{name}: {} -> {}", edge.from, edge.to);
                    assert_eq!(edge.endpoint, l.endpoint, "{name}: {} -> {}", edge.from, edge.to);
                    assert_eq!((edge.to.as_str(), edge.rel), (l.to.as_str(), l.rel), "{name}");
                }
            }
        }
    }

    // Fails if an AZ with no copy of the sender is counted as one the line comes from.
    #[test]
    fn sources_list_only_the_zones_that_send() {
        let (_, lines, _) = gathered(FEWER_READERS);
        let s3: Vec<&Line> = lines.iter().filter(|l| l.to == "s3").collect();
        assert_eq!(s3.len(), 1);
        assert_eq!(s3[0].sources, [Source { part: "aurora".to_string(), azs: vec![0, 1] }]);
    }

    // Fails if a line merged across AZs is dropped by the merging of side lines.
    #[test]
    fn web_logs_from_both_zones_are_one_line() {
        let (m, lines, _) = gathered(CASES[13].1);
        let cw: Vec<&Line> = lines.iter().filter(|l| l.to == "cloudwatch").collect();
        assert_eq!(cw.len(), 1);
        let senders: Vec<&str> = cw[0].edges.iter().map(|&e| m.edges[e].from.as_str()).collect();
        assert_eq!(senders, ["ecs-1a", "ecs-1c"]);
        assert_eq!(cw[0].from, "ecs-1c");
        assert!(parts_of(cw[0], &m).contains(&"（全 AZ から）".to_string()), "{:?}", parts_of(cw[0], &m));
    }

    // Fails if a line whose senders are not in every AZ says it comes from all AZs.
    #[test]
    fn zones_without_a_source_are_named() {
        let (m, lines, _) = gathered(&fewer_readers_en());
        let s3 = lines.iter().find(|l| l.to == "s3").unwrap();
        let parts = parts_of(s3, &m);
        assert_eq!(parts, ["Read/Write", "(via VPC endpoint)", "(from 1a, 1c)"]);
        assert!(!parts.contains(&"(all AZs)".to_string()));
    }

    // Fails if a line into an AZ with no copy of the sender is dropped, or if it is drawn with only the note that says such lines are not drawn.
    #[test]
    fn a_line_into_a_zone_without_a_source_is_drawn_across() {
        let (m, lines, notes) = gathered(&fewer_readers_en());
        let into: Vec<&Line> = lines.iter().filter(|l| l.to == "ecs-1d" && m.edges[l.rep].from.starts_with("aurora-")).collect();
        assert_eq!(into.len(), 1);
        assert_eq!(into[0].from, "aurora-1a");
        assert!(parts_of(into[0], &m).contains(&"(from 1a, 1c)".to_string()), "{:?}", parts_of(into[0], &m));
        let fallback = "In ap-northeast-1d, ecs-1d has no aurora in its own zone, so the line from aurora-1a in another zone is drawn.";
        assert_eq!(notes.iter().filter(|n| *n == fallback).count(), 1, "{notes:?}");
        let merged = Rules::builtin().text("note.cross_az_merged", Lang::En);
        assert_eq!(notes.iter().filter(|n| **n == merged).count(), 1, "{notes:?}");
    }

    // Fails if lines with different hole ids are merged into one line.
    #[test]
    fn merged_lines_share_one_hole() {
        let r = Rules::builtin();
        let (m, l) = two_holes(&r);
        let (lines, _) = gather(&m, &l, &r);
        let edge = |from: &str| m.edges.iter().position(|e| e.from == from && e.to == "s3").unwrap();
        let s3: Vec<(Option<&str>, &[usize])> = lines.iter().filter(|l| l.to == "s3").map(|l| (l.hole.as_deref(), l.edges.as_slice())).collect();
        assert_eq!(s3, [(Some("route.ecs.s3"), &[edge("ecs-1a")][..]), (Some("route.x.s3"), &[edge("ecs-1c")][..])]);
    }

    // Fails if two drawn lines get the same id (edge-<from>-to-<to>).
    #[test]
    fn line_ids_are_unique() {
        for (name, src) in CASES {
            let (_, lines, _) = gathered(src);
            let mut ends: Vec<(&str, &str)> = lines.iter().map(|l| (l.from.as_str(), l.to.as_str())).collect();
            ends.sort_unstable();
            let n = ends.len();
            ends.dedup();
            assert_eq!(ends.len(), n, "{name}");
        }
    }

    fn node(id: &str, part: &str, service: &str, az: Option<usize>, area: Area) -> Node {
        Node {
            id: id.to_string(),
            service: service.to_string(),
            option: None,
            az,
            role: None,
            added: false,
            region: None,
            part: part.to_string(),
            tag: part.split_once('.').map(|(_, t)| t.to_string()),
            area,
            undecided: false,
        }
    }

    // Fails if per-AZ copies inside the VPC and a single part outside it are merged, which would read as both being in every AZ.
    #[test]
    fn copies_and_a_single_part_are_not_merged() {
        let nodes = vec![
            node("lambda.api-1a", "lambda.api", "lambda", Some(0), Area::Subnet(Tier::Private)),
            node("lambda.api-1c", "lambda.api", "lambda", Some(1), Area::Subnet(Tier::Private)),
            node("lambda.report", "lambda.report", "lambda", None, Area::Region),
            node("cloudwatch", "cloudwatch", "cloudwatch", None, Area::Region),
        ];
        let edges = ["lambda.api-1a", "lambda.api-1c", "lambda.report"]
            .iter()
            .map(|f| Edge { from: f.to_string(), to: "cloudwatch".to_string(), rel: Rel::Aux, label: "logs_metrics".to_string(), hole: None, endpoint: false })
            .collect();
        let m = Model {
            region: "ap-northeast-1".to_string(),
            azs: vec!["ap-northeast-1a".to_string(), "ap-northeast-1c".to_string()],
            lang: Lang::En,
            nodes,
            edges,
            direction: None,
        };
        let icons = [("lambda.api-1a", 0, 0), ("lambda.api-1c", 0, 200), ("lambda.report", 0, 400), ("cloudwatch", 300, 200)]
            .iter()
            .map(|&(id, x, y)| Icon { id: id.to_string(), label: String::new(), lines: 1, rect: Rect { x, y, w: 60, h: 60 }, parent: None })
            .collect();
        let l = Layout { frames: Vec::new(), icons };
        let r = Rules::builtin();
        let (lines, notes) = gather(&m, &l, &r);
        let cw: Vec<Vec<&str>> = lines
            .iter()
            .filter(|l| l.to == "cloudwatch")
            .map(|l| l.edges.iter().map(|&e| m.edges[e].from.as_str()).collect())
            .collect();
        assert_eq!(cw, [vec!["lambda.api-1a", "lambda.api-1c"], vec!["lambda.report"]]);
        let template = r.text("note.merged_side_lines", Lang::En);
        let head = template.split('{').next().unwrap();
        assert!(notes.iter().all(|n| !n.starts_with(head)), "{notes:?}");
    }

    /// Two Lambda parts with logs into CloudWatch, in model order `lambda.a` then `lambda.b`, at
    /// the given rectangles (x, y, w, h); one line comes out of them and this is its sender.
    fn drawn_from(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32), target: (i32, i32, i32, i32)) -> String {
        let nodes = vec![
            node("lambda.a", "lambda.a", "lambda", None, Area::Region),
            node("lambda.b", "lambda.b", "lambda", None, Area::Region),
            node("cloudwatch", "cloudwatch", "cloudwatch", None, Area::Region),
        ];
        let edges = ["lambda.a", "lambda.b"]
            .iter()
            .map(|f| Edge { from: f.to_string(), to: "cloudwatch".to_string(), rel: Rel::Aux, label: "logs_metrics".to_string(), hole: None, endpoint: false })
            .collect();
        let m = Model { region: "ap-northeast-1".to_string(), azs: Vec::new(), lang: Lang::En, nodes, edges, direction: None };
        let icons = [("lambda.a", a), ("lambda.b", b), ("cloudwatch", target)]
            .iter()
            .map(|&(id, (x, y, w, h))| Icon { id: id.to_string(), label: String::new(), lines: 1, rect: Rect { x, y, w, h }, parent: None })
            .collect();
        let r = Rules::builtin();
        let (lines, _) = gather(&m, &Layout { frames: Vec::new(), icons }, &r);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].edges, [0, 1]);
        lines[0].from.clone()
    }

    // Fails if the sender is chosen by the vertical distance alone instead of the across-plus-down distance.
    #[test]
    fn the_sender_nearest_across_plus_down_draws_the_line() {
        // Target centre (330, 30). lambda.a centre (30, 30): 300 across + 0 down = 300.
        // lambda.b centre (330, 130): 0 across + 100 down = 100. By the vertical distance alone a (0) would win.
        assert_eq!(drawn_from((0, 0, 60, 60), (300, 100, 60, 60), (300, 0, 60, 60)), "lambda.b");
    }

    // Fails if the distance is measured between the icons' top-left corners instead of their centres.
    #[test]
    fn the_distance_is_measured_between_icon_centres() {
        // Target centre (330, 30), corner (300, 0). lambda.a is 200 square: centre (400, 200) gives
        // 70 + 170 = 240, corner (300, 100) gives 0 + 100 = 100. lambda.b centre (130, 30) gives
        // 200 + 0 = 200, corner (100, 0) gives 200 + 0 = 200. By centres b wins, by corners a would.
        assert_eq!(drawn_from((300, 100, 200, 200), (100, 0, 60, 60), (300, 0, 60, 60)), "lambda.b");
    }

    // Fails if a tie in distance goes to the later sender in the model instead of the first.
    #[test]
    fn a_tie_in_distance_goes_to_the_first_sender_in_the_model() {
        // Target centre (330, 30). lambda.a centre (330, 230): 0 + 200 = 200.
        // lambda.b centre (130, 30): 200 + 0 = 200. Equal, so the first in the model, lambda.a.
        assert_eq!(drawn_from((300, 200, 60, 60), (100, 0, 60, 60), (300, 0, 60, 60)), "lambda.a");
    }

    // Fails if a line with no per-AZ copies reads its empty set of AZs as matching every AZ.
    #[test]
    fn lines_without_copies_have_no_zone_note() {
        for (name, src) in [CASES[0], CASES[1]] {
            let (m, lines, _) = gathered(src);
            for l in &lines {
                for p in parts_of(l, &m) {
                    assert!(!p.contains("AZ") && !p.contains("1a") && !p.contains("1c"), "{name}: {p}");
                }
            }
        }
    }

    // Fails if the merging of side lines into one service is broken.
    #[test]
    fn api_merged_lines_keep_their_note() {
        let (_, _, notes) = gathered(CASES[1].1);
        assert_eq!(notes, ["lambda.search から cloudwatch への線は、lambda.orders、lambda.users、lambda.search からの線をまとめて表す。"]);
    }
}
