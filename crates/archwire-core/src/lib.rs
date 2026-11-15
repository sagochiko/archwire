pub mod arrange;
pub mod def;
pub mod diag;
pub mod drawio;
pub mod edit;
pub mod engine;
pub mod flow_infer;
pub mod infer;
pub mod layout;
pub mod model;
pub mod place;
pub mod route;
pub mod rules;

mod audit;
mod grid;
mod labels;
mod lines;
mod measure;
mod path;

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

/// parse, infer, layout, route, write. Errors give no diagram; holes still do.
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
    // Which layout the definition goes through is decided here, once: the VPC layout (Grid) or
    // the layout without a VPC (Flow). Nothing after this point decides it again; the writer is
    // told through its arguments. A definition mixing VPC parts with services kept apart from
    // them stops here.
    match engine::pick(&def, &rules) {
        Err(errors) => stop(errors),
        Ok(Engine::Grid) => {
            let (model, mut diagnostics) = infer::infer(&def, &rules);
            let Some(model) = model else {
                return Output { drawio: None, diagnostics };
            };
            let layout = layout::layout(&model, &rules);
            let (wires, notes) = route::route(&model, &layout, &rules);
            diagnostics.notes.extend(notes);
            let drawio = drawio::write(&model, &layout, &wires, &diagnostics, &rules, true);
            Output { drawio: Some(drawio), diagnostics }
        }
        Ok(Engine::Flow) => {
            let (model, diagnostics) = flow_infer::infer(&def, &rules);
            let Some(model) = model else {
                return Output { drawio: None, diagnostics };
            };
            let placed = place::place(&model, &rules);
            flow_output(&model, placed, diagnostics, &rules)
        }
    }
}

/// The rest of the layout without a VPC once the parts are placed: a layout that failed its
/// checks gives its errors and no diagram; otherwise the lines are routed from the placement's
/// plans and associations are written without arrowheads.
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
    choices: arrange::Choices,
) -> Result<(String, Vec<String>), Vec<Problem>> {
    let p = arrange::arrange(model, rules, axis, choices)?;
    let (wires, notes) = route::route_planned(model, &p.layout, rules, &p.lines);
    Ok((drawio::write(model, &p.layout, &wires, diagnostics, rules, false), notes))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn the_web_layout_keeps_its_association_arrows() {
        let x = compile("flows:\n  - user -> cloudfront -> alb -> ecs\n  - waf -- cloudfront\n").drawio.unwrap();
        assert!(line(&x, "edge-waf-to-cloudfront").contains("endArrow=block"));
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
}
