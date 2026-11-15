//! Inference for definitions without a VPC part.
//!
//! Every written part becomes one node and every connection one edge. Which connections are
//! accepted, their labels, limits and the reasons they give come from links.toml; which frame a
//! part belongs to comes from its scope in services.toml.

use crate::def::{Definition, Link, Ref, Rel};
use crate::diag::{Diagnostics, Hole, Problem, Versions};
use crate::infer::{check_lang, check_names, check_region, derived};
use crate::model::{Area, Edge, Model, Node};
use crate::rules::{LinkRule, Rules, Scope};

/// The only settings a definition without a VPC may write.
const SETTINGS: [&str; 2] = ["lang", "region"];

fn problem(code: &str, at: &str, got: &str, allowed: Vec<String>, message: String) -> Problem {
    Problem { code: code.into(), at: Some(at.into()), got: Some(got.into()), allowed, rule: None, message }
}

fn arrow(rel: Rel) -> &'static str {
    match rel {
        Rel::Main => "->",
        Rel::Aux => "~>",
        Rel::Assoc => "--",
    }
}

fn written(l: &Link) -> String {
    format!("{} {} {}", l.from.name, arrow(l.rel), l.to.name)
}

fn hole_id(part: &str) -> String {
    format!("kind.{part}")
}

pub fn infer(def: &Definition, rules: &Rules) -> (Option<Model>, Diagnostics) {
    let mut diag = Diagnostics {
        versions: Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() },
        ..Default::default()
    };
    let errors = &mut diag.errors;

    let names_ok = check_names(def, rules, errors);
    for key in def.set.keys().filter(|k| !SETTINGS.contains(&k.as_str())) {
        let allowed = SETTINGS.iter().map(|s| s.to_string()).collect();
        errors.push(problem("UNKNOWN_SETTING", &format!("set.{key}"), key, allowed, format!("unknown setting `{key}`")));
    }
    let region = check_region(def, errors);
    let lang = check_lang(def, errors);

    // Each part once, at its first mention.
    let mut parts: Vec<&Ref> = Vec::new();
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]) {
        if !parts.iter().any(|p| p.name == r.name) {
            parts.push(r);
        }
    }

    let mut accepted: Vec<(&Link, &LinkRule)> = Vec::new();
    if names_ok {
        for l in &def.links {
            match rules.link(&l.from.service, &l.to.service, l.rel) {
                Some(rule) => accepted.push((l, rule)),
                None => {
                    let got = written(l);
                    errors.push(problem("UNSUPPORTED", &l.at, &got, Vec::new(), format!("`{got}` is not a supported connection")));
                }
            }
        }
        check_main_flows(def, rules, &parts, &accepted, errors);
        check_limits(&accepted, errors);
    }

    // A part of a service that needs a kind and is written without one is a hole.
    let open: Vec<(String, &Ref)> = parts
        .iter()
        .filter(|r| r.option.is_none() && rules.service(&r.service).is_some_and(|s| s.kind_hole))
        .map(|r| (hole_id(&r.name), *r))
        .collect();
    for (key, value) in &def.choose {
        let at = format!("choose.{key}");
        match open.iter().find(|(id, _)| id == key) {
            Some((_, r)) => {
                let options = &rules.service(&r.service).expect("open holes have known services").options;
                if !options.contains_key(value) {
                    let allowed = options.keys().cloned().collect();
                    errors.push(problem("BAD_CHOICE", &at, value, allowed, format!("`{value}` is not a choice for `{key}`")));
                }
            }
            None => {
                let allowed = open.iter().map(|(id, _)| id.clone()).collect();
                errors.push(problem("UNKNOWN_HOLE", &at, key, allowed, format!("there is no hole `{key}`")));
            }
        }
    }
    if !diag.errors.is_empty() {
        return (None, diag);
    }

    let model = build(def, rules, &parts, &accepted, &open, region, lang, &mut diag);
    (Some(model), diag)
}

/// At least one main flow; every main flow reachable from an `origin` part; every auxiliary
/// flow leaving, and every association reaching, a part on those main flows.
fn check_main_flows(def: &Definition, rules: &Rules, parts: &[&Ref], accepted: &[(&Link, &LinkRule)], errors: &mut Vec<Problem>) {
    if !def.links.iter().any(|l| l.rel == Rel::Main) {
        errors.push(Problem {
            code: "UNSUPPORTED".into(),
            at: Some("flows".into()),
            message: "a definition needs at least one main flow (->)".into(),
            ..Default::default()
        });
    }
    let origins: Vec<String> = rules.service_keys().into_iter().filter(|k| rules.service(k).is_some_and(|s| s.origin)).collect();
    let origin_list = origins.iter().map(|o| format!("`{o}`")).collect::<Vec<_>>().join(", ");
    let main: Vec<&Link> = accepted.iter().filter(|(l, _)| l.rel == Rel::Main).map(|(l, _)| *l).collect();

    let mut reached: Vec<&str> =
        parts.iter().filter(|r| origins.contains(&r.service)).map(|r| r.name.as_str()).collect();
    loop {
        let next = main.iter().find(|l| reached.contains(&l.from.name.as_str()) && !reached.contains(&l.to.name.as_str()));
        match next {
            Some(l) => reached.push(&l.to.name),
            None => break,
        }
    }

    let mut lines: Vec<&str> = Vec::new();
    for l in main.iter().filter(|l| !reached.contains(&l.from.name.as_str())) {
        if lines.contains(&l.at.as_str()) {
            continue;
        }
        lines.push(&l.at);
        let got = written(l);
        let message = format!("`{got}` is not reached by main flows (->) from {origin_list}");
        errors.push(problem("UNSUPPORTED", &l.at, &got, Vec::new(), message));
    }

    let on_main = |name: &str| reached.contains(&name) && main.iter().any(|l| l.from.name == name || l.to.name == name);
    for (l, _) in accepted {
        let hang = match l.rel {
            Rel::Main => continue,
            Rel::Aux => &l.from,
            Rel::Assoc => &l.to,
        };
        if !on_main(&hang.name) {
            let got = written(l);
            let message = format!("`{got}` needs `{}` on a main flow (->) from {origin_list}", hang.name);
            errors.push(problem("UNSUPPORTED", &l.at, &got, Vec::new(), message));
        }
    }
}

/// `max_from`: one `to` part may have at most that many `from` parts of the same connection.
fn check_limits(accepted: &[(&Link, &LinkRule)], errors: &mut Vec<Problem>) {
    for (i, (l, rule)) in accepted.iter().enumerate() {
        let Some(max) = rule.max_from else { continue };
        let count = accepted[..=i].iter().filter(|(k, r)| std::ptr::eq(*r, *rule) && k.to.name == l.to.name).count();
        if count > max {
            let mut p = problem(
                "TOO_MANY_ATTACHED",
                &l.at,
                &l.from.name,
                Vec::new(),
                format!("`{}` can have at most {max} `{}` attached", l.to.name, rule.from),
            );
            p.rule = rule.max_rule.clone();
            errors.push(p);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build(
    def: &Definition,
    rules: &Rules,
    parts: &[&Ref],
    accepted: &[(&Link, &LinkRule)],
    open: &[(String, &Ref)],
    region: String,
    lang: crate::rules::Lang,
    diag: &mut Diagnostics,
) -> Model {
    let svc = |name: &str| rules.service(name).expect("names were checked");
    let option = |r: &Ref| r.option.clone().or_else(|| def.choose.get(&hole_id(&r.name)).cloned());
    // The connection that moves a part to another Region, if any.
    let moved = |name: &str| accepted.iter().map(|(l, rule)| (*l, *rule)).find(|(l, rule)| l.from.name == name && rule.from_rule.is_some());

    let mut nodes: Vec<Node> = Vec::new();
    for r in parts {
        let s = svc(&r.service);
        let area = match s.scope {
            Scope::Outside => Area::Outside,
            Scope::Global => Area::Cloud,
            // No accepted connection has a VPC service, so `Vpc` never reaches here.
            Scope::Regional | Scope::Vpc => Area::Region,
        };
        nodes.push(Node {
            id: r.name.clone(),
            service: r.service.clone(),
            option: option(r),
            az: None,
            role: None,
            added: false,
            region: moved(&r.name).and_then(|(_, rule)| rule.from_region.clone()),
            part: r.name.clone(),
            tag: r.tag.clone(),
            area: Some(area),
        });
    }
    let kind_of = |name: &str| nodes.iter().find(|n| n.id == name).and_then(|n| n.option.clone());

    let mut edges: Vec<Edge> = Vec::new();
    for (l, rule) in accepted {
        let note = kind_of(&l.to.name).and_then(|k| rule.to_kind.get(&k).cloned());
        edges.push(Edge {
            from: l.from.name.clone(),
            to: l.to.name.clone(),
            rel: l.rel,
            label: note.map_or_else(|| rule.label.clone(), |n| n.label),
            hole: None,
            endpoint: false,
        });
    }

    let mut why = |target: &str, id: &str, added: bool| diag.derived.push(derived(rules, lang, target, id, added));
    why("aws-cloud", "resources-inside-aws-cloud-frame", true);
    why("region", "resources-inside-region-frame", true);
    if !def.set.contains_key("region") {
        why("region", "default-region", true);
    }
    for r in parts {
        let s = svc(&r.service);
        let place = match (moved(&r.name), &s.place_rule, s.scope) {
            (Some((_, rule)), _, _) => rule.from_rule.clone(),
            (None, Some(id), _) => Some(id.clone()),
            (None, None, Scope::Global) => Some("global-service-outside-region".into()),
            (None, None, Scope::Regional) => Some("regional-service-outside-vpc".into()),
            (None, None, Scope::Outside | Scope::Vpc) => None,
        };
        if let Some(id) = place {
            why(&r.name, &id, false);
        }
        let Some(kind) = kind_of(&r.name) else { continue };
        let mut given: Vec<&str> = Vec::new();
        for (_, rule) in accepted.iter().filter(|(l, _)| l.to.name == r.name) {
            let Some(note) = rule.to_kind.get(&kind) else { continue };
            if !given.contains(&note.rule.as_str()) {
                given.push(&note.rule);
                why(&r.name, &note.rule, false);
            }
        }
    }

    for (id, r) in open.iter().filter(|(id, _)| !def.choose.contains_key(id)) {
        diag.holes.push(Hole {
            hole: id.clone(),
            at: r.at.clone(),
            need: rules.text("need.kind", lang).replace("{part}", &r.name),
            choices: svc(&r.service).options.keys().cloned().collect(),
        });
    }

    Model { region, azs: Vec::new(), lang, nodes, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> (Option<Model>, Diagnostics) { infer(&crate::def::parse(src).unwrap(), &Rules::builtin()) }
    fn label(m: &Model, a: &str, b: &str) -> String { m.edges.iter().find(|e| e.from == a && e.to == b).unwrap().label.clone() }
    fn rules_of(d: &Diagnostics, t: &str) -> Vec<String> { d.derived.iter().filter(|x| x.target == t).map(|x| x.rule.clone()).collect() }
    fn first(d: &Diagnostics) -> (&str, Option<&str>, Option<&str>) {
        let e = &d.errors[0];
        (e.code.as_str(), e.at.as_deref(), e.got.as_deref())
    }
    const API: &str = "flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(http) -> lambda -> dynamodb\n  - cognito -- apigateway\n  - lambda ~> cloudwatch\nset:\n  lang: ja\n";
    const B: &str = "flows:\n  - user -> apigateway(http) -> lambda\n";

    #[test]
    fn the_target_definition_infers_parts_labels_and_reasons() {
        let (m, d) = run(API);
        let m = m.unwrap();
        assert_eq!(d.exit_code(), 0);
        let ids: Vec<_> = m.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["user","cloudfront","s3","apigateway","lambda","dynamodb","cognito","cloudwatch"]);
        let area = |id: &str| m.nodes.iter().find(|n| n.id == id).unwrap().area;
        assert_eq!((area("user"), area("cloudfront"), area("cognito")), (Some(Area::Outside), Some(Area::Cloud), Some(Area::Region)));
        assert!(m.azs.is_empty());
        assert_eq!([label(&m,"user","apigateway"), label(&m,"apigateway","lambda"), label(&m,"lambda","dynamodb"), label(&m,"cloudfront","s3"), label(&m,"cognito","apigateway"), label(&m,"lambda","cloudwatch")],
                   ["https", "invoke", "read_write", "static_files", "jwt_authorizer", "logs_metrics"]);
        assert_eq!(rules_of(&d, "apigateway"), ["regional-service-outside-vpc", "http-api-jwt-authorizer"]);
        assert_eq!(rules_of(&d, "cognito"), ["cognito-user-pool-in-one-region"]);
        assert_eq!(rules_of(&d, "lambda"), ["regional-service-outside-vpc"]);
        assert_eq!(rules_of(&d, "cloudfront"), ["global-service-outside-region"]);
        assert_eq!(rules_of(&d, "aws-cloud"), ["resources-inside-aws-cloud-frame"]);
        assert_eq!(rules_of(&d, "region"), ["resources-inside-region-frame", "default-region"]);
        assert!(["vpc", "az", "subnet"].iter().all(|t| rules_of(&d, t).is_empty()));
        assert!(d.notes.is_empty());
    }

    #[test]
    fn a_rest_api_gets_the_cognito_authorizer() {
        let (m, d) = run("flows:\n  - user -> apigateway(rest) -> lambda\n  - cognito -- apigateway\n");
        assert_eq!(label(&m.unwrap(), "cognito", "apigateway"), "cognito_authorizer");
        assert_eq!(rules_of(&d, "apigateway"), ["regional-service-outside-vpc", "rest-api-cognito-authorizer"]);
    }

    #[test]
    fn a_written_region_still_gives_the_frame_a_reason() {
        let (_, d) = run(&format!("{B}set:\n  region: ap-northeast-1\n"));
        assert_eq!(rules_of(&d, "region"), ["resources-inside-region-frame"]);
    }

    #[test]
    fn pairs_outside_the_list_stop() {
        for line in ["user -> s3", "cloudfront -> lambda", "cloudfront ~> cloudwatch", "dynamodb -> lambda", "lambda -> internet", "apigateway -- cognito", "waf -- apigateway"] {
            let (m, d) = run(&format!("{B}  - {line}\n"));
            assert!(m.is_none(), "{line}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{line}");
            assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"));
        }
    }

    #[test]
    fn every_main_flow_starts_from_the_user() {
        let (m, d) = run("flows:\n  - lambda -> dynamodb\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[0]")));
        let (m, d) = run("flows:\n  - cognito -- apigateway(http)\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows")));
    }

    #[test]
    fn side_lines_hang_on_the_main_flow() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> s3\n  - cognito -- apigateway(http)\n  - apigateway ~> cloudwatch\n");
        assert!(m.is_none());
        let errors: Vec<_> = d.errors.iter().map(|e| (e.code.as_str(), e.at.as_deref())).collect();
        assert_eq!(errors, [("UNSUPPORTED", Some("flows[1]")), ("UNSUPPORTED", Some("flows[2]"))]);
    }

    #[test]
    fn two_web_acls_on_one_distribution_stop() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> s3\n  - waf.front -- cloudfront\n  - waf.extra -- cloudfront\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].rule.as_deref()), ("TOO_MANY_ATTACHED", Some("flows[2]"), Some("one-web-acl-per-resource")));
        assert_eq!(d.errors.len(), 1);
        let (m, d) = run("flows:\n  - user -> cloudfront.a -> s3.a\n  - user -> cloudfront.b -> s3.b\n  - waf -- cloudfront.a\n  - waf -- cloudfront.b\n");
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert!(m.is_some());
        assert_eq!(d.exit_code(), 0);
    }

    #[test]
    fn an_api_without_a_kind_is_a_hole() {
        let src = "flows:\n  - user -> apigateway -> lambda\n  - cognito -- apigateway\n";
        let (m, d) = run(src);
        let m = m.unwrap();
        assert_eq!(d.exit_code(), 2);
        assert_eq!(d.holes.len(), 1);
        let h = &d.holes[0];
        assert_eq!((h.hole.as_str(), h.at.as_str(), h.need.as_str()), ("kind.apigateway", "flows[0][1]", "Kind of apigateway"));
        assert_eq!(h.choices, ["http", "rest"]);
        assert_eq!(label(&m, "cognito", "apigateway"), "verify_token");
        assert_eq!(rules_of(&d, "apigateway"), ["regional-service-outside-vpc"]);

        let (m, d) = run(&format!("{src}choose:\n  kind.apigateway: rest\n"));
        let m = m.unwrap();
        assert!(d.holes.is_empty() && d.errors.is_empty());
        assert_eq!(m.nodes.iter().find(|n| n.id == "apigateway").unwrap().option.as_deref(), Some("rest"));
        assert_eq!(label(&m, "cognito", "apigateway"), "cognito_authorizer");

        let (m, d) = run(&format!("{src}choose:\n  kind.apigateway: soap\n"));
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].allowed.clone()), ("BAD_CHOICE", vec!["http".to_string(), "rest".to_string()]));

        let (m, d) = run(&format!("{B}choose:\n  kind.apigateway: http\n"));
        assert!(m.is_none());
        assert_eq!(d.errors[0].code, "UNKNOWN_HOLE");
    }

    #[test]
    fn only_the_part_without_a_kind_is_a_hole() {
        let src = "flows:\n  - user -> apigateway(http) -> lambda\n  - user -> apigateway.admin -> lambda.admin\n";
        let (_, d) = run(src);
        let holes: Vec<_> = d.holes.iter().map(|h| h.hole.as_str()).collect();
        assert_eq!(holes, ["kind.apigateway.admin"]);
        let (_, d) = run(&format!("{src}choose:\n  kind.apigateway.adm: http\n"));
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].allowed.clone()), ("UNKNOWN_HOLE", vec!["kind.apigateway.admin".to_string()]));
    }

    #[test]
    fn named_parts_have_their_own_reasons() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> s3.web\n  - user -> apigateway(http) -> lambda -> s3.uploads\n");
        let m = m.unwrap();
        let web = m.nodes.iter().find(|n| n.id == "s3.web").unwrap();
        assert_eq!((web.service.as_str(), web.part.as_str(), web.tag.as_deref()), ("s3", "s3.web", Some("web")));
        assert_eq!(rules_of(&d, "s3.web"), ["regional-service-outside-vpc"]);
        assert!(rules_of(&d, "s3").is_empty());
    }

    #[test]
    fn only_lang_and_region_may_be_set() {
        for (k, v) in [("azs", "2"), ("nat", "2"), ("readers", "2"), ("colour", "2")] {
            let (m, d) = run(&format!("{B}set:\n  {k}: {v}\n"));
            assert!(m.is_none(), "{k}");
            assert_eq!(d.errors.len(), 1, "{k}: {:?}", d.errors);
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].allowed.clone()), ("UNKNOWN_SETTING", vec!["lang".to_string(), "region".to_string()]), "{k}");
            assert_eq!(d.exit_code(), 1);
        }
        let (_, d) = run(&format!("{B}set:\n  region: us-east-1\n"));
        assert_eq!(d.errors[0].code, "BAD_SETTING");
    }

    #[test]
    fn a_certificate_on_cloudfront_lives_in_us_east_1() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> s3\n  - acm -- cloudfront\n");
        let m = m.unwrap();
        assert_eq!(m.nodes.iter().find(|n| n.id == "acm").unwrap().region.as_deref(), Some("us-east-1"));
        assert_eq!(rules_of(&d, "acm"), ["cloudfront-certificate-in-us-east-1"]);
    }

    #[test]
    fn an_unknown_service_lists_the_new_services() {
        let (m, d) = run("flows:\n  - user -> apigatway -> lambda\n");
        assert!(m.is_none());
        assert_eq!(d.errors[0].code, "UNKNOWN_SERVICE");
        for s in ["apigateway", "cognito", "dynamodb", "lambda"] {
            assert!(d.errors[0].allowed.contains(&s.to_string()), "{s}");
        }
    }

    #[test]
    fn named_parts_in_the_model_never_share_a_node_or_line_id() {
        let src = "flows:\n  - user -> apigateway(http) -> lambda -> s3\n  - apigateway -> lambda.1a -> s3.lambda-to-s3\n  - apigateway -> lambda.a-to-b -> dynamodb\n  - apigateway -> lambda.a -> s3.to-dynamodb\n  - lambda.a -> dynamodb.b-to-s3\n  - lambda.a-to-b ~> cloudwatch\n  - lambda.1a ~> cloudwatch.to-x\n";
        let (m, d) = run(src);
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        let m = m.unwrap();
        let nodes: Vec<String> = m.nodes.iter().map(|n| n.id.clone()).collect();
        assert!(nodes.contains(&"lambda.1a".to_string()) && nodes.contains(&"lambda".to_string()));
        assert!(nodes.contains(&"lambda.a-to-b".to_string()) && nodes.contains(&"s3.lambda-to-s3".to_string()));
        let mut unique = nodes.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), nodes.len());
        let lines: Vec<String> = m.edges.iter().map(|e| crate::route::edge_id(&e.from, &e.to)).collect();
        assert_eq!(lines.len(), 12);
        let mut unique = lines.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), lines.len());
        assert!(lines.iter().all(|l| !nodes.contains(l)));
        // Each line id names exactly one ordered pair of nodes in the model: the pair it joins.
        for e in &m.edges {
            let id = crate::route::edge_id(&e.from, &e.to);
            let pairs: Vec<_> = nodes.iter().flat_map(|a| nodes.iter().map(move |b| (a, b)))
                .filter(|(a, b)| a != b && crate::route::edge_id(a, b) == id).collect();
            assert_eq!(pairs, [(&e.from, &e.to)], "{id}");
        }
    }
}
