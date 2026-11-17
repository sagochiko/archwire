pub mod arrange;
pub mod def;
pub mod diag;
pub mod drawio;
pub mod edit;
pub mod engine;
pub mod flow_infer;
pub mod infer;
pub mod inference;
pub mod layout;
pub mod model;
pub mod place;
pub mod route;
pub mod rules;

mod audit;
mod grid;
mod ids;
mod labels;
mod lines;
mod measure;
mod path;
#[cfg(test)]
pub(crate) mod test_inputs;

use diag::{Diagnostics, Problem, Versions};
use engine::Engine;
use model::Model;
use place::Placement;
use rules::Rules;

pub struct Output {
    pub drawio: Option<String>,
    pub diagnostics: Diagnostics,
}

/// The versions every diagnostics report carries: this program's and the built-in rules'.
pub fn versions() -> Versions {
    Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: Rules::builtin().version }
}

/// parse, infer, arrange, route, write. Errors give no diagram; holes still do.
pub fn compile(def_text: &str) -> Output {
    let rules = Rules::builtin();
    let stop = |errors| {
        let versions = Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() };
        Output { drawio: None, diagnostics: Diagnostics { versions, errors, ..Default::default() } }
    };
    let def = match def::parse(def_text) {
        Ok(d) => d,
        Err(errors) => return stop(errors),
    };
    // Two shapes are not drawn yet and stop before the inference: a definition mixing VPC parts
    // with services kept apart from them, and one with a VPC that writes two or more parts
    // spanning its zones. Every other definition, with a VPC or without, goes through the one
    // inference.
    match engine::pick(&def, &rules) {
        Err(errors) => return stop(errors),
        Ok(Engine::Grid) => {
            if let Err(errors) = engine::spanning_parts(&def, &rules) {
                return stop(errors);
            }
        }
        Ok(Engine::Flow) => {}
    }
    let (model, diagnostics) = inference::infer(&def, &rules);
    let Some(model) = model else {
        return Output { drawio: None, diagnostics };
    };
    let axis = model.direction.unwrap_or_else(|| arrange::default_axis(&model));
    let placed = arrange::arrange(&model, &rules, axis);
    flow_output(&model, placed, diagnostics, &rules)
}

/// The rest of the layout once the parts are placed: a layout that failed its checks gives its
/// errors and no diagram; otherwise the lines are routed from the placement's drafts and
/// associations are written without arrowheads.
fn flow_output(model: &Model, placed: Result<Placement, Vec<Problem>>, mut diagnostics: Diagnostics, rules: &Rules) -> Output {
    let p = match placed {
        Ok(p) => p,
        Err(errors) => {
            diagnostics.errors.extend(errors);
            return Output { drawio: None, diagnostics };
        }
    };
    let (wires, notes) = route::route_planned(model, &p.layout, rules, &p.lines);
    diagnostics.notes.extend(notes);
    let drawio = drawio::write(model, &p.layout, &wires, &diagnostics, rules, false);
    Output { drawio: Some(drawio), diagnostics }
}

/// The diagram the new layout gives for a model: arrange, route_planned with the placement's drafts,
/// and the writer without association arrowheads. Also the router's notes.
#[cfg(test)]
pub(crate) fn arranged_drawio(
    model: &Model,
    diagnostics: &Diagnostics,
    rules: &Rules,
    axis: model::Axis,
) -> Result<(String, Vec<String>), Vec<Problem>> {
    let p = arrange::arrange(model, rules, axis)?;
    let (wires, notes) = route::route_planned(model, &p.layout, rules, &p.lines);
    Ok((drawio::write(model, &p.layout, &wires, diagnostics, rules, false), notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_inputs::{
        heavy_variants, CROWDED_SIDE_EN, CROWDED_S3_JA, ID_CLASH, LOAD_BALANCER_ALONE, ONE_FUNCTION_THREE_WAYS, TWIN_APPS, TWO_WEBS, VPC_TEST_DEFINITIONS,
        WEB_EXAMPLES,
    };

    #[test]
    fn a_definition_mixing_vpc_parts_with_separate_services_stops_before_inference() {
        let web = "flows:\n  - user -> cloudfront -> alb -> ecs\n";
        let cases = [
            ("flows:\n  - user -> alb -> ecs -> lambda\n".to_string(), "flows[0][3]", "lambda"),
            (format!("{web}  - cloudfront -> lambda\n"), "flows[1][1]", "lambda"),
            (format!("{web}  - ecs ~> cognito\n"), "flows[1][1]", "cognito"),
            (format!("{web}  - cognito -- alb\n"), "flows[1][0]", "cognito"),
            (format!("{web}  - ecs -> dynamodb.orders\n"), "flows[1][1]", "dynamodb"),
            (format!("{web}  - cloudfront -> apigateway.public(rest)\n"), "flows[1][1]", "apigateway"),
        ];
        for (src, at, got) in &cases {
            let out = compile(src);
            assert!(out.drawio.is_none(), "{src}");
            let d = &out.diagnostics;
            assert_eq!(d.exit_code(), 1, "{src}");
            assert_eq!(d.errors.len(), 1, "{src}: {:?}", d.errors);
            let e = &d.errors[0];
            assert_eq!((e.code.as_str(), e.at.as_deref(), e.got.as_deref()), ("UNSUPPORTED", Some(*at), Some(*got)), "{src}");
            assert!(e.message.contains("cannot be drawn together with VPC services"), "{src}: {}", e.message);
            assert_eq!(d.versions.rules, Rules::builtin().version);
            assert!(d.derived.is_empty() && d.holes.is_empty() && d.notes.is_empty());
        }
    }

    /// The tag that opens the object of `id`, up to its first `>`.
    fn object(x: &str, id: &str) -> String {
        let tag = x.split("<object ").find(|s| s.starts_with(&format!("id=\"{id}\""))).unwrap_or_else(|| panic!("no object `{id}`"));
        tag.split('>').next().unwrap().to_string()
    }

    /// The line of the diagram that writes the cell `id`.
    fn line<'a>(x: &'a str, id: &str) -> &'a str {
        x.lines().find(|l| l.contains(&format!("id=\"{id}\""))).unwrap_or_else(|| panic!("no cell `{id}`"))
    }

    const API: &str = "flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(http) -> lambda -> dynamodb\n  - cognito -- apigateway\n  - lambda ~> cloudwatch\nset:\n  lang: ja\n";

    #[test]
    fn an_api_definition_compiles_with_associations_without_arrows() {
        let out = compile(API);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        assert!(out.diagnostics.notes.is_empty(), "{:?}", out.diagnostics.notes);
        let x = out.drawio.unwrap();
        assert!(line(&x, "edge-cognito-to-apigateway").contains("endArrow=none"));
        assert!(!line(&x, "edge-cognito-to-apigateway").contains("endArrow=block"));
        assert!(line(&x, "edge-lambda-to-cloudwatch").contains("endArrow=block"));
    }

    // Fails if an association in a definition with a VPC gets an arrowhead.
    #[test]
    fn web_associations_are_drawn_without_arrows() {
        let x = compile("flows:\n  - user -> cloudfront -> alb -> ecs\n  - waf -- cloudfront\n").drawio.unwrap();
        let waf = line(&x, "edge-waf-to-cloudfront");
        assert!(waf.contains("endArrow=none") && !waf.contains("endArrow=block"), "{waf}");
    }

    #[test]
    fn every_added_frame_carries_a_reason() {
        let x = compile("flows:\n  - user -> apigateway(http) -> lambda\nset:\n  region: ap-northeast-1\n").drawio.unwrap();
        let region = object(&x, "region");
        assert!(region.contains(" added=\"1\"") && region.contains("rule=\"resources-inside-region-frame\""), "{region}");
        for tag in x.split("<object ").skip(1).map(|s| s.split('>').next().unwrap()) {
            assert!(!tag.contains(" added=\"1\"") || tag.contains(" rule=\""), "{tag}");
        }
    }

    #[test]
    fn a_bare_part_reason_does_not_reach_a_named_part() {
        let src = "flows:\n  - user -> apigateway(http) -> lambda.a\n  - user -> apigateway.other(rest) -> lambda.b\n  - cognito -- apigateway\n";
        let out = compile(src);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        assert!(object(&x, "apigateway").contains("http-api-jwt-authorizer"));
        assert!(!object(&x, "apigateway.other").contains("authorizer"));
    }

    #[test]
    fn ids_with_digits_and_to_in_names_stay_unique() {
        let out = compile("flows:\n  - user -> apigateway(http) -> lambda.1a -> s3.a-to-b\n  - lambda.1a -> s3\n");
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        let ids: Vec<&str> = x.split(" id=\"").skip(1).map(|s| s.split('"').next().unwrap()).collect();
        let mut seen = std::collections::BTreeSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "`{id}` twice in {ids:?}");
        }
    }

    #[test]
    fn splitting_lines_differently_gives_the_same_bytes() {
        let one = compile("flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n");
        let three = compile("flows:\n  - user -> apigateway(http)\n  - apigateway -> lambda\n  - lambda -> dynamodb\n");
        assert!(one.drawio.is_some());
        assert_eq!(one.drawio, three.drawio);
    }

    #[test]
    fn a_region_without_parts_is_not_drawn() {
        let out = compile("flows:\n  - user -> cloudfront\n");
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        assert!(!x.contains("id=\"region\""), "{x}");
        assert!(x.contains("id=\"aws-cloud\"") && x.contains("id=\"cloudfront\""));
        let rules: Vec<&str> = out.diagnostics.derived.iter().map(|d| d.rule.as_str()).collect();
        assert!(!rules.contains(&"resources-inside-region-frame") && !rules.contains(&"default-region"), "{rules:?}");
        assert!(out.diagnostics.derived.iter().all(|d| d.target != "region"));
    }

    #[test]
    fn a_region_with_one_part_is_drawn() {
        let out = compile("flows:\n  - user -> cloudfront -> s3\n");
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let region = object(out.drawio.as_ref().unwrap(), "region");
        assert!(region.contains("resources-inside-region-frame") && region.contains("default-region"), "{region}");
    }

    #[test]
    fn a_layout_failure_is_an_error_without_a_diagram() {
        let rules = Rules::builtin();
        let (model, diagnostics) = flow_infer::infer(&def::parse(HOLE).unwrap(), &rules);
        let (derived, holes) = (diagnostics.derived.clone(), diagnostics.holes.clone());
        assert!(!derived.is_empty() && !holes.is_empty());
        let failed = diag::Problem {
            code: "LAYOUT_FAILED".into(),
            at: Some("lambda".into()),
            got: Some("lambda and dynamodb".into()),
            message: "archwire could not lay out lambda and dynamodb without overlapping; this is a bug in archwire, please report the definition".into(),
            ..Default::default()
        };
        let out = flow_output(&model.unwrap(), Err(vec![failed]), diagnostics, &rules);
        assert!(out.drawio.is_none());
        assert_eq!(out.diagnostics.exit_code(), 1);
        assert_eq!(out.diagnostics.errors.iter().map(|e| e.code.as_str()).collect::<Vec<_>>(), ["LAYOUT_FAILED"]);
        assert_eq!((out.diagnostics.derived, out.diagnostics.holes), (derived, holes));
    }

    #[test]
    fn mixed_definitions_stop_before_either_layout() {
        let out = compile("flows:\n  - user -> apigateway(http) -> lambda -> aurora\n");
        assert!(out.drawio.is_none());
        assert_eq!(out.diagnostics.exit_code(), 1);
        assert!(out.diagnostics.errors.iter().all(|e| e.code == "UNSUPPORTED"), "{:?}", out.diagnostics.errors);
    }

    const HOLE: &str = "flows:\n  - user -> apigateway -> lambda -> dynamodb\n  - cognito -- apigateway\n";

    #[test]
    fn a_hole_in_a_definition_without_a_vpc_still_gives_a_diagram() {
        let out = compile(HOLE);
        let d = &out.diagnostics;
        assert_eq!(d.exit_code(), 2, "{:?}", d.errors);
        assert_eq!(d.holes.len(), 1, "{:?}", d.holes);
        assert_eq!((d.holes[0].hole.as_str(), d.holes[0].choices.clone()), ("kind.apigateway", vec!["http".to_string(), "rest".to_string()]));
        let x = out.drawio.unwrap();
        // The part with the open hole is named without a kind and says the kind is not decided.
        assert!(object(&x, "apigateway").contains("label=\"Amazon API Gateway&#xa;type undecided\""), "{}", object(&x, "apigateway"));
        assert!(!object(&x, "lambda").contains("undecided"));
        let line = line(&x, "edge-cognito-to-apigateway");
        assert!(line.contains("value=\"Verify tokens\"") && line.contains("endArrow=none"), "{line}");
        assert!(!line.contains("#879196"), "{line}");
        let ja = compile(&format!("{HOLE}set:\n  lang: ja\n")).drawio.unwrap();
        assert!(object(&ja, "apigateway").contains("label=\"Amazon API Gateway&#xa;種類未定\""), "{}", object(&ja, "apigateway"));
        let answered = compile(&format!("{HOLE}choose:\n  kind.apigateway: http\n")).drawio.unwrap();
        assert!(!object(&answered, "apigateway").contains("undecided"));
    }

    /// Three functions sending logs to one CloudWatch: the lines are drawn as one.
    const MERGED: &str = "flows:\n  - user -> apigateway(http) -> lambda.a -> dynamodb\n  - apigateway -> lambda.b -> dynamodb\n  - apigateway -> lambda.c -> dynamodb\n  - lambda.a ~> cloudwatch\n  - lambda.b ~> cloudwatch\n  - lambda.c ~> cloudwatch\n";

    #[test]
    fn the_router_notes_reach_the_diagnostics() {
        let out = compile(MERGED);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let notes = &out.diagnostics.notes;
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("lambda.a") && notes[0].contains("lambda.b") && notes[0].contains("lambda.c") && notes[0].contains("cloudwatch"), "{notes:?}");
        assert!(out.diagnostics.to_json().contains(&notes[0]));
    }

    #[test]
    fn a_line_keeps_three_pixels_from_another_lines_label() {
        // Wide names, shared targets and side parts in both frames put lines close to labels.
        let m = "m".repeat(22);
        let src = format!(
            "flows:\n  - user -> apigateway(http) -> lambda.f1{m}\n  - apigateway -> lambda.f2{m}\n  - lambda.f1{m} -> dynamodb.t1{m}\n  - lambda.f2{m} -> dynamodb.t2{m}\n  - lambda.f1{m} ~> cloudwatch.l{m}m\n  - lambda.f2{m} ~> cloudwatch.l{m}m\n  - apigateway ~> cloudwatch.l{m}m\n  - cognito.g{m}m -- apigateway\n  - user -> cloudfront.c{m}m -> s3.web\n"
        );
        for src in [src.as_str(), API, MERGED, HOLE] {
            let rules = Rules::builtin();
            let (model, _) = flow_infer::infer(&def::parse(src).unwrap(), &rules);
            let model = model.unwrap();
            let p = place::place(&model, &rules).unwrap();
            let (wires, notes) = route::route_planned(&model, &p.layout, &rules, &p.lines);
            assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?}");
            if let Some((d, line, label)) = audit::closest_line_to_label(&p.layout, &wires) {
                assert!(d >= 3, "{line} runs {d}px from the label of {label}\n{src}");
            }
        }
    }

    /// The drawn path of a line: exit, waypoints, entry, with diagonal steps folded into right angles.
    fn drawn(w: &route::Wire, layout: &layout::Layout) -> Vec<(i32, i32)> {
        let mut raw = vec![route::port_point(layout.rect(&w.from), w.exit)];
        raw.extend(w.points.iter().copied());
        raw.push(route::port_point(layout.rect(&w.to), w.entry));
        let mut out = vec![raw[0]];
        for &p in &raw[1..] {
            let a = *out.last().unwrap();
            if a.0 != p.0 && a.1 != p.1 {
                out.push((p.0, a.1));
            }
            out.push(p);
        }
        out
    }

    /// Main lines that cross a side line they share no part with, as "main x side".
    fn main_lines_across_side_lines(src: &str) -> Vec<String> {
        let rules = Rules::builtin();
        let (model, _) = flow_infer::infer(&def::parse(src).unwrap(), &rules);
        let model = model.unwrap();
        let p = place::place(&model, &rules).unwrap();
        let (wires, notes) = route::route_planned(&model, &p.layout, &rules, &p.lines);
        assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?}");
        let cross = |a: (i32, i32), b: (i32, i32), c: (i32, i32), d: (i32, i32)| {
            let (h, v) = if a.1 == b.1 { ((a, b), (c, d)) } else { ((c, d), (a, b)) };
            let ((hx1, hx2), hy) = ((h.0 .0.min(h.1 .0), h.0 .0.max(h.1 .0)), h.0 .1);
            let (vx, (vy1, vy2)) = (v.0 .0, (v.0 .1.min(v.1 .1), v.0 .1.max(v.1 .1)));
            h.0 .1 == h.1 .1 && v.0 .0 == v.1 .0 && hx1 < vx && vx < hx2 && vy1 < hy && hy < vy2
        };
        let mut out = Vec::new();
        for m in wires.iter().filter(|w| w.style == route::WireStyle::Main) {
            for s in wires.iter().filter(|w| w.style != route::WireStyle::Main) {
                if [&s.from, &s.to].iter().any(|x| **x == m.from || **x == m.to) {
                    continue;
                }
                let (pm, ps) = (drawn(m, &p.layout), drawn(s, &p.layout));
                if pm.windows(2).any(|a| ps.windows(2).any(|b| cross(a[0], a[1], b[0], b[1]))) {
                    out.push(format!("{} x {}", m.id, s.id));
                }
            }
        }
        out
    }

    #[test]
    fn a_main_line_into_the_region_goes_round_the_global_columns_side_parts() {
        let rest = "flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(rest) -> lambda -> dynamodb\n  - cognito -- apigateway\n  - waf -- cloudfront\n  - acm -- cloudfront\n";
        let m = "m".repeat(24);
        let wide = format!(
            "flows:\n  - user -> cloudfront.{m} -> s3.{m}\n  - user -> apigateway.{m}(http) -> lambda.{m} -> dynamodb.{m}\n  - cognito.{m} -- apigateway.{m}\n  - waf.{m} -- cloudfront.{m}\n  - lambda.{m} ~> cloudwatch.{m}\nset:\n  lang: ja\n"
        );
        for src in [rest, wide.as_str(), API] {
            assert_eq!(main_lines_across_side_lines(src), Vec::<String>::new(), "{src}");
        }
    }

    /// How many definitions the old and the new layout are compared on: the web examples, the test
    /// definitions and the heavy variants.
    const KEPT: usize = 47;

    /// The definitions whose diagnostics must not change with the layout, with their models: the
    /// fixed lists above, none of which writes a direction or has three zones with fewer Readers.
    /// Each must go to the grid layout and give a model; one that no longer does fails here by name.
    fn kept_definitions() -> Vec<(String, Model, Diagnostics)> {
        let rules = Rules::builtin();
        let all: Vec<String> = WEB_EXAMPLES.iter().chain(VPC_TEST_DEFINITIONS.iter()).map(|s| s.to_string()).chain(heavy_variants()).collect();
        assert_eq!(all.len(), KEPT);
        let mut out: Vec<(String, Model, Diagnostics)> = Vec::new();
        for src in all {
            assert!(!out.iter().any(|(s, _, _)| *s == src), "listed twice:\n{src}");
            let def = def::parse(&src).unwrap();
            assert!(matches!(engine::pick(&def, &rules), Ok(Engine::Grid)), "not taken to the grid layout:\n{src}");
            assert!(!def.set.contains_key("direction"), "writes a direction:\n{src}");
            assert!(!(def.set.get("azs").map(String::as_str) == Some("3") && def.set.contains_key("readers")), "fewer Readers:\n{src}");
            let (model, d) = infer::infer(&def, &rules);
            let m = model.unwrap_or_else(|| panic!("no model: {:?}\n{src}", d.errors));
            out.push((src, m, d));
        }
        out
    }

    // Fails if the new layout drops the note on lines merged across zones, or breaks a line the old layout drew whole.
    // CROWDED_S3_JA and CROWDED_SIDE_EN are left out: the old layout cannot draw them without faults either.
    #[test]
    fn old_and_new_layouts_give_the_same_diagnostics() {
        let rules = Rules::builtin();
        let kept = kept_definitions();
        assert!(kept.iter().all(|(s, _, _)| s != CROWDED_S3_JA && s != CROWDED_SIDE_EN));
        let mut differ = Vec::new();
        for (src, model, d) in &kept {
            let mut old = d.clone();
            let layout = layout::layout(model, &rules);
            old.notes.extend(route::route(model, &layout, &rules).1);
            for axis in [model::Axis::Down, model::Axis::Right] {
                let mut new = d.clone();
                match arranged_drawio(model, d, &rules, axis) {
                    Ok((_, notes)) => new.notes.extend(notes),
                    Err(e) => new.errors.extend(e),
                }
                if old.to_json() != new.to_json() {
                    differ.push(format!("{axis:?}\n{src}old notes: {:?}\nnew notes: {:?}\nnew errors: {:?}", old.notes, new.notes, new.errors));
                }
            }
        }
        assert!(differ.is_empty(), "{} differ:\n{}", differ.len(), differ.join("\n----\n"));
    }

    /// The lines the new layout cannot draw clear in the crowded definitions, by axis. Down:
    /// CROWDED_S3_JA's side line from the load balancer to S3. Right: CROWDED_SIDE_EN's line from
    /// ECS in the second zone to S3. No more than these may break, and each is named in a note.
    const CROWDED_BROKEN: [(&str, model::Axis, &[&str]); 4] = [
        ("s3-ja", model::Axis::Down, &["edge-alb-to-s3"]),
        ("side-en", model::Axis::Down, &[]),
        ("s3-ja", model::Axis::Right, &[]),
        ("side-en", model::Axis::Right, &["edge-ecs-1c-to-s3"]),
    ];

    // Fails if a crowded definition cannot be laid out, or a line it cannot draw is not named in a note, or more lines break.
    #[test]
    fn crowded_definitions_name_every_line_they_cannot_draw() {
        let rules = Rules::builtin();
        for (name, axis, want) in CROWDED_BROKEN {
            let src = if name == "s3-ja" { CROWDED_S3_JA } else { CROWDED_SIDE_EN };
            let (model, d) = infer::infer(&def::parse(src).unwrap(), &rules);
            let model = model.unwrap();
            let (_, notes) = arranged_drawio(&model, &d, &rules, axis).unwrap_or_else(|e| panic!("{name} {axis:?}: {e:?}"));
            let mut named: Vec<String> = notes
                .iter()
                .filter_map(|n| n.split_whitespace().find(|w| w.starts_with("edge-")).map(String::from))
                .collect();
            named.sort();
            named.dedup();
            assert_eq!(named, want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{name} {axis:?}: {notes:?}");
        }
    }

    // Fails if a line from a zone without a Reader stops naming the zones it comes from, or the fallback note is missing or doubled.
    #[test]
    fn fewer_readers_name_their_zones() {
        let src = "flows:\n  - user -> alb -> ecs -> aurora\n  - aurora -> ecs\n  - aurora -> s3\nset:\n  azs: 3\n  readers: 1\nchoose:\n  route.aurora.s3: endpoint\n";
        let out = compile(src);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        let to_s3: Vec<&str> = x.lines().filter(|l| l.contains("<mxCell") && l.contains("edge=\"1\"") && l.contains("target=\"s3\"")).collect();
        assert_eq!(to_s3.len(), 1, "{to_s3:?}");
        assert!(to_s3[0].contains("(from 1a, 1c)") && !to_s3[0].contains("(all AZs)"), "{}", to_s3[0]);
        let fallback = |n: &str| {
            let mut rest = n.strip_prefix("In ").unwrap_or("");
            for piece in [", ", " has no ", " in its own zone, so the line from ", " in another zone is drawn."] {
                match rest.find(piece) {
                    Some(at) => rest = &rest[at + piece.len()..],
                    None => return false,
                }
            }
            rest.is_empty()
        };
        let notes = &out.diagnostics.notes;
        assert_eq!(notes.iter().filter(|n| fallback(n)).count(), 1, "{notes:?}");
    }

    // Fails if writing a direction changes anything but the picture, or does not change the picture.
    #[test]
    fn direction_changes_the_picture_only() {
        let web = WEB_EXAMPLES[6];
        assert!(web.contains("set:\n  lang: ja\n"));
        let [right, down] = ["right", "down"].map(|way| compile(&web.replace("set:\n", &format!("set:\n  direction: {way}\n"))));
        assert_eq!(right.diagnostics.to_json(), down.diagnostics.to_json());
        assert!(right.drawio.is_some() && down.drawio.is_some());
        assert_ne!(right.drawio, down.drawio);
    }

    #[test]
    fn a_bent_main_line_carries_its_label_where_it_leaves_its_source() {
        let src = "flows:\n  - user -> apigateway(http) -> lambda.orders -> dynamodb.orders\n  - apigateway -> lambda.users -> dynamodb.users\n  - apigateway -> lambda.search -> dynamodb.orders\n  - lambda.orders ~> cloudwatch\n  - lambda.users ~> cloudwatch\n  - lambda.search ~> cloudwatch\nset:\n  lang: ja\n";
        let rules = Rules::builtin();
        let (model, _) = flow_infer::infer(&def::parse(src).unwrap(), &rules);
        let model = model.unwrap();
        let p = place::place(&model, &rules).unwrap();
        let (wires, notes) = route::route_planned(&model, &p.layout, &rules, &p.lines);
        assert!(notes.iter().all(|n| !n.contains("edge-")), "{notes:?}");
        let mut bent = 0;
        for w in wires.iter().filter(|w| w.style == route::WireStyle::Main) {
            let path = drawn(w, &p.layout);
            if path.len() < 3 || path[0].1 != path[1].1 {
                continue;
            }
            bent += 1;
            let len: i32 = path.windows(2).map(|s| (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs()).sum();
            let at = w.label_at.map_or(len / 2, |a| (a as i32 * len + 500) / 1000);
            let first = (path[1].0 - path[0].0).abs();
            assert!(at > 0 && at < first && w.horizontal_label, "{} puts its label {at} along, past its first stretch of {first}", w.id);
        }
        assert!(bent >= 4, "{bent}");
    }

    /// The errors of a definition that stops: no diagram, exit code 1, and each error's code, `at` and `got`.
    fn stops(src: &str) -> Vec<(String, Option<String>, Option<String>)> {
        let out = compile(src);
        assert!(out.drawio.is_none(), "{src}");
        assert_eq!(out.diagnostics.exit_code(), 1, "{src}");
        out.diagnostics.errors.iter().map(|e| (e.code.clone(), e.at.clone(), e.got.clone())).collect()
    }

    fn unsupported(at: &str, got: Option<&str>) -> (String, Option<String>, Option<String>) {
        ("UNSUPPORTED".to_string(), Some(at.to_string()), got.map(str::to_string))
    }

    // Today a named function next to a part named like its zone copy stops, because the definition mixes VPC parts with separate services.
    // Fails if a mixed definition is let through without an error.
    #[test]
    fn a_named_function_and_a_part_named_like_its_copy_stop() {
        assert_eq!(stops(ID_CLASH), [unsupported("flows[0][1]", Some("apigateway"))]);
    }

    // Fails if a second part spanning the zones is let through, so a broken diagram or LAYOUT_FAILED comes out.
    #[test]
    fn two_spanning_parts_stop() {
        assert_eq!(stops(TWO_WEBS), [unsupported("flows[1][1]", Some("alb.admin"))]);
        let e = &compile(TWO_WEBS).diagnostics.errors[0];
        assert_eq!(e.message, "`alb.admin` is a second part spanning the Availability Zones, after `alb.web`; this version draws one such part in a VPC");
    }

    // Fails if named VPC parts are still stopped, a database loses its replication line, or a line
    // joins the writer of one named database to the other database in either direction.
    #[test]
    fn named_parts_under_one_balancer_are_drawn() {
        for set in ["", "set:\n  direction: right\n"] {
            let src = format!("{TWIN_APPS}{set}");
            let out = compile(&src);
            assert_eq!(out.diagnostics.exit_code(), 0, "{src}: {:?}", out.diagnostics.errors);
            assert!(out.diagnostics.notes.is_empty(), "{src}: {:?}", out.diagnostics.notes);
            let x = out.drawio.unwrap_or_else(|| panic!("no diagram for {src}"));
            object(&x, "aurora.web@1a");
            object(&x, "aurora.admin@1c");
            line(&x, "edge-aurora.web@1a-to-aurora.web@1c");
            line(&x, "edge-aurora.admin@1a-to-aurora.admin@1c");
            for (from, to) in [("web", "admin"), ("admin", "web")] {
                let wrong = format!("id=\"edge-aurora.{from}@");
                let joined = x.lines().filter(|l| l.contains(&wrong) && l.contains(&format!("-to-aurora.{to}@"))).count();
                assert_eq!(joined, 0, "{src}: a line joins aurora.{from} to aurora.{to}");
            }
        }
    }

    // Fails if named services sharing one named database, a named service without a database next
    // to one with it, or three named services each with its own database stop, or come out with a
    // line the router could not place, in either direction with two or three zones.
    #[test]
    fn named_services_behind_one_balancer_are_drawn_in_every_shape() {
        let shapes = [
            "flows:\n  - user -> cloudfront -> alb -> ecs.web -> aurora.main\n  - alb -> ecs.admin -> aurora.main\n",
            "flows:\n  - user -> cloudfront -> alb -> ecs.web -> aurora.web\n  - alb -> ecs.admin\n",
            "flows:\n  - user -> cloudfront -> alb -> ecs.a -> aurora.a\n  - alb -> ecs.b -> aurora.b\n  - alb -> ecs.c -> aurora.c\n",
        ];
        for shape in shapes {
            for direction in ["down", "right"] {
                for azs in [2, 3] {
                    let src = format!("{shape}set:\n  direction: {direction}\n  azs: {azs}\n");
                    let out = compile(&src);
                    assert_eq!(out.diagnostics.exit_code(), 0, "{src}: {:?}", out.diagnostics.errors);
                    assert!(out.drawio.is_some(), "{src}");
                    assert!(out.diagnostics.notes.iter().all(|n| !n.contains("edge-")), "{src}: {:?}", out.diagnostics.notes);
                }
            }
        }
    }

    /// `n` named services behind one load balancer, the first one reaching a database.
    fn services_behind_one_balancer(n: usize, direction: &str, azs: usize) -> String {
        let mut src = "flows:\n  - user -> cloudfront -> alb -> ecs.s01 -> aurora\n".to_string();
        for i in 2..=n {
            src.push_str(&format!("  - alb -> ecs.s{i:02}\n"));
        }
        src + &format!("set:\n  direction: {direction}\n  azs: {azs}\n")
    }

    // Fails if the load balancer's side gives out at another count of services, or a definition
    // past that count comes out as a diagram instead of stopping with LAYOUT_FAILED.
    #[test]
    fn services_behind_one_balancer_stop_once_its_sides_run_out_of_spots() {
        for (azs, last) in [(2, 50), (3, 33)] {
            for direction in ["down", "right"] {
                let src = services_behind_one_balancer(last, direction, azs);
                let out = compile(&src);
                assert_eq!(out.diagnostics.exit_code(), 0, "{src}: {:?}", out.diagnostics.errors);
                assert!(out.drawio.is_some(), "{src}");
                let src = services_behind_one_balancer(last + 1, direction, azs);
                let out = compile(&src);
                assert!(out.drawio.is_none(), "{src}");
                assert_eq!(out.diagnostics.exit_code(), 1, "{src}");
                let errors = &out.diagnostics.errors;
                assert!(!errors.is_empty() && errors.iter().all(|e| e.code == "LAYOUT_FAILED"), "{src}: {errors:?}");
            }
        }
    }

    // Today a function reaching a database in a VPC stops in every line order, at the first separate service written.
    // Fails if a mixed definition is let through for some order of its lines.
    #[test]
    fn a_function_reaching_a_database_stops_whatever_the_line_order() {
        let want = [("flows[0][1]", "apigateway"), ("flows[0][1]", "apigateway"), ("flows[0][0]", "lambda")];
        for (src, (at, got)) in ONE_FUNCTION_THREE_WAYS.iter().zip(want) {
            assert_eq!(stops(src), [unsupported(at, Some(got))], "{src}");
        }
    }

    // Fails if a definition whose VPC holds only public parts is stopped, or is drawn with a private subnet.
    #[test]
    fn a_load_balancer_alone_is_drawn() {
        let out = compile(LOAD_BALANCER_ALONE);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        assert!(x.contains("id=\"pub-1a\"") && x.contains("id=\"pub-1c\""), "{x}");
        assert!(!x.contains("id=\"pri-"), "{x}");
    }

    // Fails if a connection left out of the list is let through.
    #[test]
    fn pairs_left_out_stop() {
        let cases = [
            ("flows:\n  - user -> alb -> ecs -> aurora\n  - alb -> aurora\n", "flows[1]", "alb -> aurora"),
            ("flows:\n  - user -> cloudfront -> alb -> ecs\n  - cloudfront -> cloudwatch\n", "flows[1]", "cloudfront -> cloudwatch"),
            (VPC_TEST_DEFINITIONS[25], "flows[1]", "alb -> aurora"),
        ];
        for (src, at, got) in cases {
            assert_eq!(stops(src), [unsupported(at, Some(got))], "{src}");
        }
    }

    // Fails if a line is drawn from a part the main flows from the user do not reach, or to such a part.
    #[test]
    fn parts_off_the_main_flow_stop() {
        let cases = [
            ("flows:\n  - user -> alb -> ecs\n  - aurora -> s3\n", "flows[1]", "aurora -> s3"),
            ("flows:\n  - user -> alb -> ecs\n  - aurora ~> cloudwatch\n", "flows[1]", "aurora ~> cloudwatch"),
            ("flows:\n  - user -> alb -> ecs\n  - waf -- cloudfront\n", "flows[1]", "waf -- cloudfront"),
            ("flows:\n  - cloudfront -> alb -> ecs\n", "flows[0]", "cloudfront -> alb"),
        ];
        for (src, at, got) in cases {
            assert_eq!(stops(src), [unsupported(at, Some(got))], "{src}");
            assert!(compile(src).diagnostics.holes.is_empty(), "{src}");
        }
    }

    // Fails if the check on named parts is left in, so a named part in a VPC definition is reported as well.
    #[test]
    fn named_parts_in_a_vpc_definition_report_only_their_own_errors() {
        let out = compile("flows:\n  - user -> cloudfrnt -> alb -> ecs.web\n");
        assert!(out.drawio.is_none());
        let got: Vec<_> = out.diagnostics.errors.iter().map(|e| (e.code.as_str(), e.at.as_deref())).collect();
        assert_eq!(got, [("UNKNOWN_SERVICE", Some("flows[0][1]"))]);
    }

    // Fails if a named part outside the VPC is still stopped in a definition with a VPC part.
    #[test]
    fn a_named_part_outside_the_vpc_is_drawn() {
        let src = "flows:\n  - user -> cloudfront -> alb -> ecs\n  - cloudfront -> s3.web\n";
        let out = compile(src);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        object(&out.drawio.unwrap(), "s3.web");
    }
}
