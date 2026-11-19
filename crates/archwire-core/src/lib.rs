pub mod arrange;
pub mod def;
pub mod diag;
pub mod drawio;
pub mod edit;
pub mod inference;
pub mod layout;
pub mod model;
pub mod route;
pub mod rules;

mod audit;
mod ids;
mod labels;
mod lines;
mod measure;
mod path;
#[cfg(test)]
pub(crate) mod test_inputs;

use diag::{Diagnostics, Problem, Versions};
use arrange::Placement;
use model::Model;
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
    compile_with(def_text, &Rules::builtin())
}

/// `compile` with the given rules.
pub(crate) fn compile_with(def_text: &str, rules: &Rules) -> Output {
    let stop = |errors| {
        let versions = Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() };
        Output { drawio: None, diagnostics: Diagnostics { versions, errors, ..Default::default() } }
    };
    let def = match def::parse(def_text) {
        Ok(d) => d,
        Err(errors) => return stop(errors),
    };
    // Every definition, with a VPC or without, goes through the one inference.
    let (model, diagnostics) = inference::infer(&def, rules);
    let Some(model) = model else {
        return Output { drawio: None, diagnostics };
    };
    let axis = model.direction.unwrap_or_else(|| arrange::default_axis(&model));
    let placed = arrange::arrange(&model, rules, axis);
    flow_output(&model, placed, diagnostics, rules)
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
    let drawio = drawio::write(model, &p.layout, &wires, &diagnostics, rules);
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
    Ok((drawio::write(model, &p.layout, &wires, diagnostics, rules), notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_inputs::{
        CROWDED_SIDE_EN, CROWDED_S3_JA, ID_CLASH, LOAD_BALANCER_ALONE, MIX_LOGS, MIX_NAT, MIX_SMALLEST, MIX_WEB, ONE_FUNCTION_THREE_WAYS, TWIN_APPS,
        TWO_WEBS, WEB_CASES,
    };

    // Fails if a definition once stopped for mixing VPC parts with separate services stops for
    // another reason than a connection missing from the pairs, or one the pairs accept stops.
    #[test]
    fn definitions_once_stopped_for_mixing() {
        let web = "flows:\n  - user -> cloudfront -> alb -> ecs\n";
        let stopped = [
            ("flows:\n  - user -> alb -> ecs -> lambda\n".to_string(), "flows[0]", "ecs -> lambda"),
            (format!("{web}  - cloudfront -> lambda\n"), "flows[1]", "cloudfront -> lambda"),
            (format!("{web}  - ecs ~> cognito\n"), "flows[1]", "ecs ~> cognito"),
            (format!("{web}  - cognito -- alb\n"), "flows[1]", "cognito -- alb"),
        ];
        for (src, at, got) in &stopped {
            let out = compile(src);
            assert!(out.drawio.is_none(), "{src}");
            let d = &out.diagnostics;
            assert_eq!(d.exit_code(), 1, "{src}");
            assert_eq!(d.errors.len(), 1, "{src}: {:?}", d.errors);
            let e = &d.errors[0];
            assert_eq!((e.code.as_str(), e.at.as_deref(), e.got.as_deref()), ("UNSUPPORTED", Some(*at), Some(*got)), "{src}");
            assert_eq!(e.message, format!("`{got}` is not a supported connection"), "{src}");
            assert_eq!(d.versions.rules, Rules::builtin().version);
            assert!(d.derived.is_empty() && d.holes.is_empty() && d.notes.is_empty());
        }
        let hole = compile(&format!("{web}  - ecs -> dynamodb.orders\n"));
        assert_eq!(hole.diagnostics.exit_code(), 2, "{:?}", hole.diagnostics.errors);
        let holes: Vec<&str> = hole.diagnostics.holes.iter().map(|h| h.hole.as_str()).collect();
        assert_eq!(holes, ["route.ecs.[dynamodb.orders]"]);
        assert!(hole.drawio.is_some());
        let api = compile(&format!("{web}  - cloudfront -> apigateway.public(rest)\n"));
        assert_eq!(api.diagnostics.exit_code(), 0, "{:?}", api.diagnostics.errors);
        assert!(api.drawio.is_some());
    }

    // Fails if a definition mixing VPC parts with separate services is still stopped, gets the
    // wrong exit code, or comes out with a line the router could not place, in either direction.
    #[test]
    fn mixed_definitions_are_drawn() {
        for (src, code) in [(MIX_SMALLEST, 2), (MIX_NAT, 0), (MIX_WEB, 2), (MIX_LOGS, 0)] {
            for direction in ["down", "right"] {
                let src = if src.contains("set:\n") {
                    src.replace("set:\n", &format!("set:\n  direction: {direction}\n"))
                } else {
                    format!("{src}set:\n  direction: {direction}\n")
                };
                let out = compile(&src);
                let d = &out.diagnostics;
                assert_eq!(d.exit_code(), code, "{src}: {:?} {:?}", d.errors, d.holes);
                assert!(out.drawio.is_some(), "{src}");
                assert!(d.notes.iter().all(|n| !n.contains("edge-")), "{src}: {:?}", d.notes);
            }
        }
        let holes = |src: &str| compile(src).diagnostics.holes.iter().map(|h| h.hole.clone()).collect::<Vec<_>>();
        assert_eq!(holes(MIX_SMALLEST), ["route.lambda.s3"]);
        assert_eq!(holes(MIX_WEB), ["route.ecs.dynamodb"]);
    }

    // Fails if a web stack and an API that share no part are stopped, or the function behind the
    // API is put in the VPC.
    #[test]
    fn web_and_api_side_by_side_are_drawn() {
        let src = "flows:\n  - user -> alb -> ecs\n  - user -> apigateway(http) -> lambda\n";
        let out = compile(src);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        object(&x, "lambda");
        let (model, _) = inference::infer(&def::parse(src).unwrap(), &Rules::builtin());
        let lambda: Vec<_> = model.unwrap().nodes.into_iter().filter(|n| n.part == "lambda").collect();
        assert_eq!(lambda.len(), 1);
        assert_eq!((lambda[0].id.as_str(), lambda[0].area, lambda[0].az), ("lambda", model::Area::Region, None));
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
        let (model, diagnostics) = inference::infer(&def::parse(HOLE).unwrap(), &rules);
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
            let (model, _) = inference::infer(&def::parse(src).unwrap(), &rules);
            let model = model.unwrap();
            let p = arrange::arrange(&model, &rules, model::Axis::Right).unwrap();
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
        let (model, _) = inference::infer(&def::parse(src).unwrap(), &rules);
        let model = model.unwrap();
        let p = arrange::arrange(&model, &rules, model::Axis::Right).unwrap();
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

    /// The lines the new layout cannot draw clear in the crowded definitions, by axis. Down:
    /// CROWDED_S3_JA's side line from the load balancer to S3. Right: CROWDED_SIDE_EN's line from
    /// ECS in the second zone to S3. No more than these may break, and each is named in a note.
    pub(crate) const CROWDED_BROKEN: [(&str, model::Axis, &[&str]); 4] = [
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
            let (model, d) = inference::infer(&def::parse(src).unwrap(), &rules);
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

    // Fails if a line from a zone without a Reader stops naming the zones it comes from, or the
    // fallback note is missing or doubled. The note needs `aurora -> ecs`, which only the test
    // rules accept; the label is checked with the built-in rules too, without that line.
    #[test]
    fn fewer_readers_name_their_zones() {
        let src = crate::test_inputs::FEWER_READERS;
        let accepted = src.replace("  - aurora -> ecs\n", "");
        let to_s3_label = |out: &Output| {
            assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
            let x = out.drawio.clone().unwrap();
            let to_s3: Vec<String> = x.lines().filter(|l| l.contains("<mxCell") && l.contains("edge=\"1\"") && l.contains("target=\"s3\"")).map(String::from).collect();
            assert_eq!(to_s3.len(), 1, "{to_s3:?}");
            assert!(to_s3[0].contains("(from 1a, 1c)") && !to_s3[0].contains("(all AZs)"), "{}", to_s3[0]);
        };
        to_s3_label(&compile(&accepted));
        let out = compile_with(src, &crate::test_inputs::rules_with_test_links());
        to_s3_label(&out);
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
        let notes = compile(&accepted).diagnostics.notes;
        assert_eq!(notes.iter().filter(|n| fallback(n)).count(), 0, "{notes:?}");
    }

    // Fails if writing a direction changes anything but the picture, or does not change the picture.
    #[test]
    fn direction_changes_the_picture_only() {
        let web = WEB_CASES[0].1;
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
        let (model, _) = inference::infer(&def::parse(src).unwrap(), &rules);
        let model = model.unwrap();
        let p = arrange::arrange(&model, &rules, model::Axis::Right).unwrap();
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

    // Fails if the zone copy of a named function and a part named like that copy get the same id.
    #[test]
    fn a_named_function_and_a_part_named_like_its_copy_are_drawn() {
        let out = compile(ID_CLASH);
        assert_eq!(out.diagnostics.exit_code(), 0, "{:?}", out.diagnostics.errors);
        let x = out.drawio.unwrap();
        object(&x, "lambda.api@1a");
        object(&x, "lambda.api-1a");
        assert_eq!(x.matches("<object id=\"lambda.api@1a\"").count(), 1);
        assert_eq!(x.matches("<object id=\"lambda.api-1a\"").count(), 1);
    }

    /// A definition drawn in full: exit code 0, a diagram, and no note naming a line.
    fn drawn_cleanly(src: &str) -> String {
        let out = compile(src);
        assert_eq!(out.diagnostics.exit_code(), 0, "{src}: {:?}", out.diagnostics.errors);
        assert!(out.diagnostics.notes.iter().all(|n| !n.contains("edge-")), "{src}: {:?}", out.diagnostics.notes);
        out.drawio.unwrap_or_else(|| panic!("no diagram for {src}"))
    }

    /// `src` with its direction and zones set, in every combination drawn.
    fn every_shape(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        for direction in ["down", "right"] {
            for azs in [2, 3] {
                out.push(format!("{src}set:\n  direction: {direction}\n  azs: {azs}\n"));
            }
        }
        out
    }

    // Fails if a definition with two named load balancers is stopped, or a line joins a part behind
    // one load balancer to a part behind the other, in either direction with two or three zones.
    #[test]
    fn balancers_are_drawn() {
        for src in every_shape(TWO_WEBS) {
            let x = drawn_cleanly(&src);
            object(&x, "alb.web");
            object(&x, "alb.admin");
            for (from, to) in [("web", "admin"), ("admin", "web")] {
                let wrong = format!("id=\"edge-aurora.{from}@");
                let joined = x.lines().filter(|l| l.contains(&wrong) && l.contains(&format!("-to-aurora.{to}@"))).count();
                assert_eq!(joined, 0, "{src}: a line joins aurora.{from} to aurora.{to}");
            }
        }
    }

    // Fails if a third load balancer runs out of places beside the others, in either direction
    // with two or three zones.
    #[test]
    fn three_balancers_are_drawn() {
        let three = "flows:\n  - user -> cloudfront -> alb.a -> ecs.a\n  - user -> alb.b -> ecs.b\n  - user -> alb.c -> ecs.c\n";
        for src in every_shape(three) {
            let x = drawn_cleanly(&src);
            for alb in ["alb.a", "alb.b", "alb.c"] {
                object(&x, alb);
            }
        }
    }

    // Fails if the number of load balancers is still checked, or the way the lines are written
    // decides whether a definition with several of them is drawn.
    #[test]
    fn balancers_written_in_any_way_are_drawn() {
        let drawn = [
            "flows:\n  - user -> alb.a -> ecs\n  - user -> alb.b -> ecs\n  - user -> alb.c -> ecs\n",
            "flows:\n  - user -> alb -> ecs\n  - user -> alb.admin -> ecs.admin\n",
            "flows:\n  - user -> alb.a -> ecs.a\n  - user -> alb.b -> ecs.b\n  - alb.b -> ecs.c\n",
            "flows:\n  - user -> alb.a -> ecs\n  - alb.b ~> cloudwatch\n  - user -> alb.b -> ecs\n",
            "flows:\n  - user -> alb.a -> ecs.a\n  - alb.a -> ecs.b\n",
        ];
        for src in drawn {
            drawn_cleanly(src);
        }
        drawn_cleanly("flows:\n  - user -> alb.a -> ecs\n  - waf -- alb.b\n  - user -> alb.b -> ecs\n");
        let misspelt = "flows:\n  - user -> alb.a -> ecs\n  - user -> albb.b -> ecs\n";
        assert_eq!(stops(misspelt), [("UNKNOWN_SERVICE".to_string(), Some("flows[1][1]".to_string()), Some("albb".to_string()))]);
    }

    // Fails if a certificate or web ACL attached to a load balancer is stopped, comes out with a line
    // the router could not place, leaves the Region, stops sitting right of the VPC, or its line
    // stops leaving the load balancer by its right side (or its bottom when a part of its row stands
    // right of it), with one or two load balancers, in either direction, with two or three zones.
    #[test]
    fn attachments_on_the_load_balancer_are_drawn() {
        let web = WEB_CASES.iter().find(|(n, _)| *n == "web-2az-ja").unwrap().1.replace("set:\n  lang: ja\n", "");
        let shapes = [
            format!("{web}  - acm -- alb\n  - waf -- alb\n"),
            format!("{web}  - acm.cdn -- cloudfront\n  - waf.cdn -- cloudfront\n  - acm -- alb\n  - waf -- alb\n"),
            format!("{TWO_WEBS}  - acm.web -- alb.web\n  - waf.web -- alb.web\n  - acm.admin -- alb.admin\n  - waf.admin -- alb.admin\n"),
            format!("{TWO_WEBS}  - waf -- alb.web\n  - waf -- alb.admin\n"),
        ];
        assert!(shapes.iter().all(|s| !s.contains("set:")));
        let rules = Rules::builtin();
        for shape in &shapes {
            for src in every_shape(shape) {
                drawn_cleanly(&src);
                let (m, _) = inference::infer(&def::parse(&src).unwrap(), &rules);
                let m = m.unwrap();
                let p = arrange::arrange(&m, &rules, m.direction.unwrap()).unwrap_or_else(|e| panic!("{src}: {e:?}"));
                let (wires, _) = route::route_planned(&m, &p.layout, &rules, &p.lines);
                let (vpc, region) = (p.layout.rect("vpc"), p.layout.rect("region"));
                let on_alb: Vec<&model::Edge> = m.edges.iter().filter(|e| e.rel == def::Rel::Assoc && m.nodes.iter().any(|n| n.id == e.to && n.service == "alb")).collect();
                assert!(on_alb.len() >= 2, "{src}");
                for e in on_alb {
                    let part = m.nodes.iter().find(|n| n.id == e.from).unwrap();
                    assert_eq!(part.area, model::Area::Region, "{src}: {}", e.from);
                    let r = p.layout.rect(&e.from);
                    assert!(r.x >= vpc.x + vpc.w, "{src}: {} is not right of the VPC", e.from);
                    assert!(r.x >= region.x && r.x + r.w <= region.x + region.w && r.y >= region.y && r.y + r.h <= region.y + region.h, "{src}: {} leaves the Region", e.from);
                    let alb = p.layout.rect(&e.to);
                    let blocked = p.layout.icons.iter().any(|o| o.rect.y == alb.y && o.rect.x > alb.x && o.rect.x < vpc.x + vpc.w);
                    let side = if blocked { route::Side::Bottom } else { route::Side::Right };
                    let w = wires.iter().find(|w| w.from == e.from && w.to == e.to).unwrap_or_else(|| panic!("{src}: no line {} to {}", e.from, e.to));
                    assert_eq!(w.entry.side, side, "{src}: {} to {}", e.from, e.to);
                }
            }
        }
    }

    // Fails if a definition with eight to twelve lines attached to load balancers stops by a panic
    // or never ends, or ends otherwise than with a diagram (perhaps with notes naming lines) or with
    // LAYOUT_FAILED, in either direction.
    #[test]
    fn many_attachments_on_load_balancers_end() {
        let albs = |n: usize, own_waf: bool| {
            let mut src = String::from("flows:\n");
            for k in 0..n {
                src.push_str(&format!("  - user -> alb.a{k} -> ecs.a{k}\n  - acm.a{k} -- alb.a{k}\n"));
                src.push_str(&if own_waf { format!("  - waf.a{k} -- alb.a{k}\n") } else { format!("  - waf -- alb.a{k}\n") });
            }
            src
        };
        // Eight and twelve lines: more than the seven whose every order the layout tries.
        for shape in [albs(4, true), albs(6, false)] {
            for direction in ["down", "right"] {
                let src = format!("{shape}set:\n  direction: {direction}\n");
                let out = compile(&src);
                let d = &out.diagnostics;
                match &out.drawio {
                    Some(_) => assert!(d.errors.is_empty(), "{src}: {:?}", d.errors),
                    None => assert!(!d.errors.is_empty() && d.errors.iter().all(|e| e.code == "LAYOUT_FAILED"), "{src}: {:?}", d.errors),
                }
            }
        }
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

    /// Each part's frame in the diagram, keyed by its id: the `parent` of the cell of each object.
    fn frames(x: &str) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        for chunk in x.split("<object ").skip(1) {
            let id = chunk.strip_prefix("id=\"").and_then(|r| r.split('"').next()).unwrap().to_string();
            let cell = chunk.split("<mxCell").nth(1).unwrap_or_else(|| panic!("no cell for {id}"));
            let parent = cell.split(" parent=\"").nth(1).map(|r| r.split('"').next().unwrap().to_string()).unwrap_or_default();
            out.insert(id, parent);
        }
        out
    }

    // Fails if the order of the lines changes which frame a part lands in, its holes, or its reasons.
    #[test]
    fn a_function_reaching_a_database_lands_the_same_whatever_the_line_order() {
        let at = [
            [("route.lambda.dynamodb", "flows[1]"), ("route.lambda.s3", "flows[2]")],
            [("route.lambda.dynamodb", "flows[2]"), ("route.lambda.s3", "flows[1]")],
            [("route.lambda.dynamodb", "flows[1]"), ("route.lambda.s3", "flows[0]")],
        ];
        let mut seen = Vec::new();
        for (src, want) in ONE_FUNCTION_THREE_WAYS.iter().zip(at) {
            let out = compile(src);
            let d = &out.diagnostics;
            assert_eq!(d.exit_code(), 2, "{src}: {:?}", d.errors);
            let x = out.drawio.clone().unwrap_or_else(|| panic!("no diagram for {src}"));
            assert_eq!(compile(src).drawio, out.drawio, "{src}");
            let mut holes: Vec<(String, Vec<String>)> = d.holes.iter().map(|h| (h.hole.clone(), h.choices.clone())).collect();
            holes.sort();
            let mut placed: Vec<(&str, &str)> = d.holes.iter().map(|h| (h.hole.as_str(), h.at.as_str())).collect();
            placed.sort();
            assert_eq!(placed, want, "{src}");
            let mut reasons: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
            for r in &d.derived {
                reasons.entry(r.target.clone()).or_default().push(r.rule.clone());
            }
            for rules in reasons.values_mut() {
                rules.sort();
            }
            let (model, _) = inference::infer(&def::parse(src).unwrap(), &Rules::builtin());
            let roles: std::collections::BTreeMap<String, (model::Area, Option<usize>, Option<String>)> =
                model.unwrap().nodes.into_iter().map(|n| (n.id, (n.area, n.az, n.role))).collect();
            seen.push((frames(&x), roles, holes, reasons));
        }
        assert_eq!(seen[0], seen[1]);
        assert_eq!(seen[0], seen[2]);
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
            ("flows:\n  - user -> alb -> ecs -> aurora\n  - alb -> aurora\n  - aurora ~> cloudwatch\n  - alb ~> s3\nset:\n  azs: 3\n", "flows[1]", "alb -> aurora"),
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
