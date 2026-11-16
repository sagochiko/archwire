//! Inference: checks a definition against the rules data and derives the semantic model.
//!
//! The same checks run for every definition, in this order: names, settings, connections,
//! whether the main flows are reached, limits, and the answers given to holes. Whether a
//! definition has a VPC is decided before them, from the scope of the services it writes.
//! A definition without a VPC part is then built here: every written part becomes one node and
//! every connection one edge, with the labels, limits and reasons of links.toml.

mod build;
mod links;
mod reasons;

pub(crate) use links::check_names;
pub(crate) use reasons::derived;

use crate::def::{Definition, Link, Ref};
use crate::diag::{Diagnostics, Problem, Versions};
use crate::model::{Area, Axis, Model, Tier};
use crate::rules::{Lang, LinkRule, Place, Rules, Scope, Service, Subnet};

/// The rules this module's code names itself; the rest come from the data, which loading checks.
pub(crate) const RULES: [&str; 5] = [
    "resources-inside-aws-cloud-frame",
    "resources-inside-region-frame",
    "default-region",
    "global-service-outside-region",
    "regional-service-outside-vpc",
];

/// The settings a definition with a VPC may write.
const VPC_SETTINGS: [&str; 6] = ["azs", "direction", "lang", "nat", "readers", "region"];
/// The settings a definition without a VPC may write.
const FLOW_SETTINGS: [&str; 3] = ["direction", "lang", "region"];
const AZ_COUNTS: [&str; 2] = ["2", "3"];
const REGIONS: [&str; 1] = ["ap-northeast-1"];
const LANGS: [&str; 2] = ["en", "ja"];
const DIRECTIONS: [&str; 2] = ["down", "right"];
const NAT_MODES: [&str; 2] = ["per-az", "single"];
const ROUTE_CHOICES: [&str; 2] = ["endpoint", "nat"];

/// The settings a model is built with.
pub(crate) struct Settings {
    region: String,
    lang: Lang,
    direction: Option<Axis>,
    /// The number of Availability Zones: 2 unless a definition with a VPC writes `azs`.
    azs: usize,
    /// The number of Readers of a Writer: one fewer than the zones unless `readers` is written.
    readers: usize,
    /// `nat: single`: one NAT gateway in the first zone instead of one in every zone.
    nat_single: bool,
}

/// What the checks found in a definition that passed them.
struct Checked<'a> {
    /// The definition writes a part of a service whose scope is the VPC.
    vpc: bool,
    settings: Settings,
    parts: Vec<&'a Ref>,
    accepted: Vec<(&'a Link, &'a LinkRule)>,
    kinds: Vec<(String, &'a Ref)>,
}

fn problem(code: &str, at: &str, got: &str, allowed: Vec<String>, message: String) -> Problem {
    Problem { code: code.into(), at: Some(at.into()), got: Some(got.into()), allowed, rule: None, message }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

fn bad_setting(errors: &mut Vec<Problem>, key: &str, got: &str, allowed: Vec<String>) {
    errors.push(problem("BAD_SETTING", &format!("set.{key}"), got, allowed, format!("`{got}` is not a valid value for `{key}`")));
}

pub fn infer(def: &Definition, rules: &Rules) -> (Option<Model>, Diagnostics) {
    let mut diag = Diagnostics {
        versions: Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() },
        ..Default::default()
    };
    let Some(checked) = check(def, rules, &mut diag) else { return (None, diag) };
    if checked.vpc {
        return crate::infer::infer(def, rules);
    }
    let Checked { parts, accepted, kinds, settings, .. } = checked;
    let model = build::build(def, rules, &parts, &accepted, &kinds, settings);
    reasons::explain(def, rules, &model, &parts, &accepted, &kinds, &mut diag);
    (Some(model), diag)
}

/// Every check, in order, adding its errors to `diag`; None if any check stopped the definition.
fn check<'a>(def: &'a Definition, rules: &'a Rules, diag: &mut Diagnostics) -> Option<Checked<'a>> {
    diag.errors = rules.require(&RULES);
    if !diag.errors.is_empty() {
        return None;
    }
    let vpc = def
        .links
        .iter()
        .flat_map(|l| [&l.from, &l.to])
        .any(|r| rules.service(&r.service).is_some_and(|s| s.scope == Scope::Vpc));
    let errors = &mut diag.errors;

    let names_ok = check_names(def, rules, errors);
    let settings = check_settings(def, vpc, errors);
    let parts = links::parts(def);
    let accepted = if names_ok { links::check_links(def, rules, &parts, errors) } else { Vec::new() };
    let kinds = links::kind_holes(rules, &parts);
    let routes = links::route_holes(def, rules);
    links::check_choices(def, rules, &kinds, &routes, errors);
    if !diag.errors.is_empty() {
        return None;
    }
    Some(Checked { vpc, settings, parts, accepted, kinds })
}

/// The settings in this order: `azs`, `region`, `lang`, `direction`, `nat`, `readers`, then
/// the keys the definition may not write. Without a VPC only `direction`, `lang` and `region`
/// may be written.
fn check_settings(def: &Definition, vpc: bool, errors: &mut Vec<Problem>) -> Settings {
    let get = |key: &str| def.set.get(key).map(String::as_str);
    let allowed: &[&str] = if vpc { &VPC_SETTINGS } else { &FLOW_SETTINGS };

    let mut azs = 2;
    if vpc {
        match get("azs") {
            None => {}
            Some(v) if AZ_COUNTS.contains(&v) => azs = v.parse().expect("listed value"),
            Some(v) => bad_setting(errors, "azs", v, strings(&AZ_COUNTS)),
        }
    }
    let region = check_region(def, errors);
    let lang = check_lang(def, errors);
    let direction = check_direction(def, errors);
    let mut readers = azs - 1;
    let mut nat_single = false;
    if vpc {
        match get("nat") {
            None | Some("per-az") => {}
            Some("single") => nat_single = true,
            Some(v) => bad_setting(errors, "nat", v, strings(&NAT_MODES)),
        }
        if let Some(v) = get("readers") {
            match v.parse::<usize>() {
                Ok(n) if (1..azs).contains(&n) => readers = n,
                _ => bad_setting(errors, "readers", v, (1..azs).map(|n| n.to_string()).collect()),
            }
        }
    }
    for key in def.set.keys().filter(|k| !allowed.contains(&k.as_str())) {
        errors.push(problem("UNKNOWN_SETTING", &format!("set.{key}"), key, strings(allowed), format!("unknown setting `{key}`")));
    }
    Settings { region, lang, direction, azs, readers, nat_single }
}

/// The value of `set.region`, ap-northeast-1 when it is not written. A value outside the list
/// adds BAD_SETTING.
pub(crate) fn check_region(def: &Definition, errors: &mut Vec<Problem>) -> String {
    match def.set.get("region").map(String::as_str) {
        Some(v) if !REGIONS.contains(&v) => {
            bad_setting(errors, "region", v, strings(&REGIONS));
            REGIONS[0].to_string()
        }
        v => v.unwrap_or(REGIONS[0]).to_string(),
    }
}

/// The value of `set.lang`, English when it is not written. A value outside the list adds BAD_SETTING.
pub(crate) fn check_lang(def: &Definition, errors: &mut Vec<Problem>) -> Lang {
    match def.set.get("lang").map(String::as_str) {
        None | Some("en") => Lang::En,
        Some("ja") => Lang::Ja,
        Some(v) => {
            bad_setting(errors, "lang", v, strings(&LANGS));
            Lang::En
        }
    }
}

/// `set.direction`: None when it is not written; a value other than `down` and `right` adds BAD_SETTING.
pub(crate) fn check_direction(def: &Definition, errors: &mut Vec<Problem>) -> Option<Axis> {
    match def.set.get("direction").map(String::as_str) {
        None => None,
        Some("down") => Some(Axis::Down),
        Some("right") => Some(Axis::Right),
        Some(v) => {
            bad_setting(errors, "direction", v, strings(&DIRECTIONS));
            None
        }
    }
}

/// The area a part of this service belongs to, from its scope, subnet and place in the rules data.
pub(crate) fn area_of(service: &Service) -> Area {
    match (service.scope, service.place, service.subnet) {
        (Scope::Outside, _, _) => Area::Outside,
        (Scope::Global, _, _) => Area::Cloud,
        (Scope::Regional, _, _) => Area::Region,
        (Scope::Vpc, Place::Gateway, _) => Area::VpcEdge,
        (Scope::Vpc, _, Some(Subnet::Private)) => Area::Subnet(Tier::Private),
        // Loading the rules data stops a VPC service that is neither a gateway nor in a subnet,
        // so `None` never reaches here.
        (Scope::Vpc, _, Some(Subnet::Public) | None) => Area::Subnet(Tier::Public),
    }
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
        let areas: Vec<_> = m.nodes.iter().map(|n| (n.id.as_str(), n.area)).collect();
        let (o, c, r) = (Area::Outside, Area::Cloud, Area::Region);
        assert_eq!(areas, [("user", o), ("cloudfront", c), ("s3", r), ("apigateway", r), ("lambda", r), ("dynamodb", r), ("cognito", r), ("cloudwatch", r)]);
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
        // The frames are added by archwire; the parts were written in the definition.
        for x in &d.derived {
            assert_eq!(x.added, ["aws-cloud", "region"].contains(&x.target.as_str()), "{} {}", x.target, x.rule);
        }
        assert!(d.derived.iter().any(|x| x.added) && d.derived.iter().any(|x| !x.added));
    }

    #[test]
    fn an_authorizer_reason_goes_only_to_the_api_it_is_on() {
        let (m, d) = run("flows:\n  - user -> apigateway(http) -> lambda\n  - user -> apigateway.admin(rest) -> lambda.admin\n  - cognito -- apigateway.admin\n");
        let m = m.unwrap();
        assert_eq!(rules_of(&d, "apigateway"), ["regional-service-outside-vpc"]);
        assert_eq!(rules_of(&d, "apigateway.admin"), ["regional-service-outside-vpc", "rest-api-cognito-authorizer"]);
        assert_eq!(label(&m, "cognito", "apigateway.admin"), "cognito_authorizer");
        assert!(d.derived.iter().filter(|x| x.target.starts_with("apigateway")).all(|x| !x.added));
    }

    /// The built-in rules.toml without one rule.
    fn rules_without(id: &str) -> Result<Rules, Vec<crate::diag::Problem>> {
        let text = include_str!("../../../../rules/aws/rules.toml");
        let start = text.find(&format!("[[rule]]\nid = \"{id}\"")).unwrap();
        let end = text[start + 1..].find("[[rule]]").map_or(text.len(), |e| start + 1 + e);
        let rules = format!("{}{}", &text[..start], &text[end..]);
        Rules::load(
            include_str!("../../../../rules/aws/services.toml"),
            &rules,
            include_str!("../../../../rules/aws/labels.toml"),
            include_str!("../../../../rules/aws/links.toml"),
        )
    }

    #[test]
    fn a_rule_the_code_names_but_the_data_lacks_is_an_error_not_a_panic() {
        type Infer = fn(&Definition, &Rules) -> (Option<Model>, Diagnostics);
        let api = crate::def::parse(API).unwrap();
        let web = crate::def::parse("flows:\n  - user -> alb -> ecs -> aurora\n  - ecs -> internet\n").unwrap();
        // A definition with a VPC is checked here and then built by the inference with a VPC,
        // so the rules both name are needed for it.
        let runs: [(&[&str], &Definition, Infer); 3] = [(&RULES, &api, infer), (&RULES, &web, infer), (&crate::infer::RULES, &web, infer)];
        // The rules the data names too, so loading already stops: by where in the data they are named.
        let named_by_links = ["cloudfront-certificate-in-us-east-1"];
        let named_by_services = ["load-balancer-in-vpc", "alb-spans-two-azs", "default-two-azs"];
        for (ids, def, run) in runs {
            for id in ids {
                let stops_loading = if named_by_links.contains(id) {
                    Some("LINK_UNKNOWN_RULE")
                } else if named_by_services.contains(id) {
                    Some("SERVICE_UNKNOWN_RULE")
                } else {
                    None
                };
                match (rules_without(id), stops_loading) {
                    (Err(e), Some(code)) => assert_eq!(e[0].code, code, "{id}"),
                    (Ok(rules), None) => {
                        let (m, d) = run(def, &rules);
                        assert!(m.is_none(), "{id}");
                        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("RULE_MISSING", Some(*id)));
                    }
                    (Err(e), None) => panic!("loading stopped without {id}: {e:?}"),
                    (Ok(_), Some(code)) => panic!("loading did not stop with {code} without {id}"),
                }
            }
        }
    }

    /// Every file of this module, so the rules its code names are all looked at.
    const SOURCES: [(&str, &str); 4] = [
        ("build.rs", include_str!("build.rs")),
        ("links.rs", include_str!("links.rs")),
        ("mod.rs", include_str!("mod.rs")),
        ("reasons.rs", include_str!("reasons.rs")),
    ];

    // Fails if the code names a rule missing from RULES, or a file of this module is not read for it.
    #[test]
    fn the_rule_lists_hold_every_rule_the_code_names() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("inference");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".rs"))
            .collect();
        files.sort();
        assert_eq!(files, SOURCES.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>());
        let r = Rules::builtin();
        for (name, src) in SOURCES {
            let code = src.find("#[cfg(test)]").map_or(src, |i| &src[..i]);
            for lit in code.split('"').skip(1).step_by(2) {
                if r.require(&[lit]).is_empty() {
                    assert!(RULES.contains(&lit), "`{lit}` is named in {name} but missing from RULES");
                }
            }
        }
        assert!(r.require(&RULES).is_empty());
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
        for line in ["user -> s3", "cloudfront -> lambda", "dynamodb -> lambda", "lambda -> internet", "apigateway -- cognito", "waf -- apigateway"] {
            let (m, d) = run(&format!("{B}  - {line}\n"));
            assert!(m.is_none(), "{line}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{line}");
            assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"));
        }
    }

    // Fails if an auxiliary line from CloudFront is accepted while CloudFront is not on a main flow,
    // or stopped while it is.
    #[test]
    fn a_cloudfront_side_line_needs_cloudfront_on_a_main_flow() {
        let (m, d) = run(&format!("{B}  - cloudfront ~> cloudwatch\n"));
        assert!(m.is_none());
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some("cloudfront ~> cloudwatch")));
        assert_eq!(d.errors[0].message, "`cloudfront ~> cloudwatch` needs `cloudfront` on a main flow (->) from `user`");
        let (m, d) = run("flows:\n  - user -> cloudfront -> s3\n  - cloudfront ~> cloudwatch\n");
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert!(m.is_some());
        assert_eq!(d.exit_code(), 0);
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
    fn only_direction_lang_and_region_may_be_set() {
        for (k, v) in [("azs", "2"), ("nat", "2"), ("readers", "2"), ("colour", "2")] {
            let (m, d) = run(&format!("{B}set:\n  {k}: {v}\n"));
            assert!(m.is_none(), "{k}");
            assert_eq!(d.errors.len(), 1, "{k}: {:?}", d.errors);
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].allowed.clone()), ("UNKNOWN_SETTING", vec!["direction".to_string(), "lang".to_string(), "region".to_string()]), "{k}");
            assert_eq!(d.exit_code(), 1);
        }
        let (_, d) = run(&format!("{B}set:\n  region: us-east-1\n"));
        assert_eq!(d.errors[0].code, "BAD_SETTING");
    }

    // Breaks when the layout without a VPC does not read `set.direction`, or takes an unknown value as a default.
    #[test]
    fn direction_is_read_in_both_layouts() {
        use crate::model::Axis;
        let (m, d) = run(B);
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(m.unwrap().direction, None);
        for (v, want) in [("right", Axis::Right), ("down", Axis::Down)] {
            let (m, d) = run(&format!("{B}set:\n  direction: {v}\n"));
            assert!(d.errors.is_empty(), "{v}: {:?}", d.errors);
            assert_eq!(m.unwrap().direction, Some(want), "{v}");
        }
        for v in ["left", "Right", "up"] {
            let (m, d) = run(&format!("{B}set:\n  direction: {v}\n"));
            assert!(m.is_none(), "{v}");
            assert_eq!(d.errors.len(), 1, "{v}: {:?}", d.errors);
            let e = &d.errors[0];
            assert_eq!((e.code.as_str(), e.at.as_deref(), e.got.as_deref()), ("BAD_SETTING", Some("set.direction"), Some(v)));
            assert_eq!(e.allowed, ["down", "right"]);
            assert_eq!(d.exit_code(), 1);
        }
        let (m, d) = run(&format!("{B}set:\n  colour: red\n"));
        assert!(m.is_none());
        assert_eq!(d.errors[0].code, "UNKNOWN_SETTING");
        assert_eq!(d.errors[0].allowed, ["direction", "lang", "region"]);
    }

    // Breaks when `direction` is checked out of its place among the other settings.
    #[test]
    fn a_bad_direction_is_reported_after_region_and_lang_and_before_unknown_keys() {
        let (_, d) = run(&format!("{B}set:\n  direction: up\n  lang: fr\n  region: us-east-1\n  azs: 2\n"));
        let at: Vec<_> = d.errors.iter().map(|e| e.at.as_deref().unwrap()).collect();
        assert_eq!(at, ["set.region", "set.lang", "set.direction", "set.azs"]);
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
        let lines: Vec<String> = m.edges.iter().map(|e| crate::ids::edge_id(&e.from, &e.to)).collect();
        assert_eq!(lines.len(), 12);
        let mut unique = lines.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), lines.len());
        assert!(lines.iter().all(|l| !nodes.contains(l)));
        // Each line id names exactly one ordered pair of nodes in the model: the pair it joins.
        for e in &m.edges {
            let id = crate::ids::edge_id(&e.from, &e.to);
            let pairs: Vec<_> = nodes.iter().flat_map(|a| nodes.iter().map(move |b| (a, b)))
                .filter(|(a, b)| a != b && crate::ids::edge_id(a, b) == id).collect();
            assert_eq!(pairs, [(&e.from, &e.to)], "{id}");
        }
    }

    /// How the answers of the inference without a VPC and of this one may differ for an input.
    enum Expect {
        /// The same model and the same diagnostics.
        Same,
        /// Both stop with the same errors; these are the `at` of the new order.
        ErrorOrder(&'static [&'static str]),
    }

    /// The inputs whose answer changes on purpose, by their name in the lists.
    const CHANGED: [(&str, Expect); 1] = [
        // Unknown keys are reported after the values of the known settings.
        ("FLOW_INFER_TEST_DEFINITIONS#36", Expect::ErrorOrder(&["set.region", "set.lang", "set.direction", "set.azs"])),
    ];
    const SAME_COUNT: usize = 1264;
    const ERROR_ORDER_COUNT: usize = 1;
    const INPUT_COUNT: usize = 1265;

    // Fails if the model, labels, reasons, holes or errors of a definition without a VPC change
    // from the inference without a VPC outside the listed inputs, or an input drops out of the lists.
    #[test]
    fn definitions_without_a_vpc_infer_as_before() {
        use crate::test_inputs::{generated, API_CASES, FLOW_INFER_TEST_DEFINITIONS};
        let mut inputs: Vec<(String, String)> = Vec::new();
        for (i, (name, src)) in API_CASES.iter().enumerate() {
            inputs.push((format!("API_CASES#{i} {name}"), src.to_string()));
        }
        for (name, src) in [
            ("api-http-ja-down", include_str!("../../../../tests/cases/api-http-ja-down/def.yaml")),
            ("api-merged-ja-down", include_str!("../../../../tests/cases/api-merged-ja-down/def.yaml")),
        ] {
            inputs.push((format!("tests/cases/{name}"), src.to_string()));
        }
        for (i, src) in generated().into_iter().enumerate() {
            inputs.push((format!("generated#{i}"), src));
        }
        for (i, src) in FLOW_INFER_TEST_DEFINITIONS.iter().enumerate() {
            inputs.push((format!("FLOW_INFER_TEST_DEFINITIONS#{i}"), src.to_string()));
        }
        inputs.push(("tests/cases/err-unsupported".into(), include_str!("../../../../tests/cases/err-unsupported/def.yaml").into()));

        let rules = Rules::builtin();
        let (mut same, mut error_order) = (0, 0);
        for (id, src) in &inputs {
            let def = crate::def::parse(src).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            let (old_model, old) = crate::flow_infer::infer(&def, &rules);
            let (new_model, new) = infer(&def, &rules);
            match CHANGED.iter().find(|(name, _)| name == id).map(|(_, e)| e).unwrap_or(&Expect::Same) {
                Expect::Same => {
                    assert_eq!(old_model, new_model, "{id}:\n{src}");
                    assert_eq!(old.to_json(), new.to_json(), "{id}:\n{src}");
                    same += 1;
                }
                Expect::ErrorOrder(want) => {
                    assert!(old_model.is_none() && new_model.is_none(), "{id}:\n{src}");
                    let at: Vec<&str> = new.errors.iter().map(|e| e.at.as_deref().unwrap_or("")).collect();
                    assert_eq!(at, *want, "{id}:\n{src}");
                    assert_ne!(old.errors, new.errors, "{id}: the order did not change");
                    // The old errors put in the new order are the new errors, one for one.
                    let mut left = old.errors.clone();
                    let mut reordered = old.clone();
                    reordered.errors = new
                        .errors
                        .iter()
                        .map(|e| {
                            let i = left.iter().position(|o| o == e).unwrap_or_else(|| panic!("{id}: {e:?} is new"));
                            left.remove(i)
                        })
                        .collect();
                    assert!(left.is_empty(), "{id}: {left:?} are gone");
                    assert_eq!(reordered.to_json(), new.to_json(), "{id}:\n{src}");
                    error_order += 1;
                }
            }
        }
        assert_eq!((same, error_order), (SAME_COUNT, ERROR_ORDER_COUNT));
        assert_eq!(inputs.len(), INPUT_COUNT);
        assert_eq!(SAME_COUNT + ERROR_ORDER_COUNT, INPUT_COUNT);
    }

    /// The model of a definition with a VPC that passes the checks, built with these rules.
    fn vpc_with(src: &str, rules: &Rules) -> Model {
        let def = crate::def::parse(src).unwrap();
        let mut diag = Diagnostics::default();
        let checked = check(&def, rules, &mut diag).unwrap_or_else(|| panic!("{:?}", diag.errors));
        assert!(checked.vpc);
        build::vpc_model(&def, rules, &checked.settings)
    }
    fn vpc(src: &str) -> Model { vpc_with(src, &Rules::builtin()) }
    fn node_ids(m: &Model) -> Vec<&str> { m.nodes.iter().map(|n| n.id.as_str()).collect() }
    fn lines(m: &Model) -> Vec<(&str, &str, &str)> {
        m.edges.iter().map(|e| (e.from.as_str(), e.to.as_str(), e.label.as_str())).collect()
    }
    fn role<'m>(m: &'m Model, id: &str) -> Option<&'m str> {
        m.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("{id}")).role.as_deref()
    }

    // Fails if copies are gathered by service rather than by part, replication joins the Writer of
    // one part to the Reader of another, or the entry is looked up by the written name.
    #[test]
    fn named_vpc_parts_build_by_part() {
        use crate::test_inputs::TWIN_APPS;
        let m = vpc(TWIN_APPS);
        assert_eq!(node_ids(&m), [
            "user", "cloudfront", "igw", "alb", "ecs.web@1a", "ecs.web@1c", "aurora.web@1a", "aurora.web@1c",
            "ecs.admin@1a", "ecs.admin@1c", "aurora.admin@1a", "aurora.admin@1c",
        ]);
        for id in ["aurora.web@1a", "aurora.admin@1a"] {
            assert_eq!(role(&m, id), Some("writer"), "{id}");
        }
        for id in ["aurora.web@1c", "aurora.admin@1c"] {
            assert_eq!(role(&m, id), Some("reader"), "{id}");
        }
        assert_eq!(lines(&m), [
            ("user", "cloudfront", "https"),
            ("cloudfront", "igw", "https"),
            ("igw", "alb", "https"),
            ("alb", "ecs.web@1a", "http"),
            ("alb", "ecs.web@1c", "http"),
            ("ecs.web@1a", "aurora.web@1a", "read_write"),
            ("ecs.web@1a", "aurora.web@1c", "read"),
            ("ecs.web@1c", "aurora.web@1a", "read_write"),
            ("ecs.web@1c", "aurora.web@1c", "read"),
            ("alb", "ecs.admin@1a", "http"),
            ("alb", "ecs.admin@1c", "http"),
            ("ecs.admin@1a", "aurora.admin@1a", "read_write"),
            ("ecs.admin@1a", "aurora.admin@1c", "read"),
            ("ecs.admin@1c", "aurora.admin@1a", "read_write"),
            ("ecs.admin@1c", "aurora.admin@1c", "read"),
            ("aurora.web@1a", "aurora.web@1c", "replication"),
            ("aurora.admin@1a", "aurora.admin@1c", "replication"),
        ]);
        assert!(!m.edges.iter().any(|e| e.from == "aurora.web@1a" && e.to == "aurora.admin@1c"));
    }

    // Fails if the two load balancers become one, the entry gateway is made twice, one of the web
    // stacks is dropped, copies are gathered by service, or lines cross from one part to another.
    #[test]
    fn two_load_balancers_build_by_part() {
        use crate::model::Tier;
        use crate::test_inputs::TWO_WEBS;
        let m = vpc(TWO_WEBS);
        assert_eq!(node_ids(&m), [
            "user", "cloudfront", "igw", "alb.web", "alb.admin", "ecs.web@1a", "ecs.web@1c", "aurora.web@1a",
            "aurora.web@1c", "ecs.admin@1a", "ecs.admin@1c", "aurora.admin@1a", "aurora.admin@1c",
        ]);
        for id in ["alb.web", "alb.admin"] {
            let n = m.nodes.iter().find(|n| n.id == id).unwrap();
            assert_eq!((n.area, n.az), (Area::Subnet(Tier::Public), None), "{id}");
        }
        assert_eq!(m.nodes.iter().filter(|n| n.area == Area::VpcEdge).count(), 1);
        assert_eq!(lines(&m), [
            ("user", "cloudfront", "https"),
            ("cloudfront", "igw", "https"),
            ("igw", "alb.web", "https"),
            ("alb.web", "ecs.web@1a", "http"),
            ("alb.web", "ecs.web@1c", "http"),
            ("ecs.web@1a", "aurora.web@1a", "read_write"),
            ("ecs.web@1a", "aurora.web@1c", "read"),
            ("ecs.web@1c", "aurora.web@1a", "read_write"),
            ("ecs.web@1c", "aurora.web@1c", "read"),
            ("user", "igw", "https"),
            ("igw", "alb.admin", "https"),
            ("alb.admin", "ecs.admin@1a", "http"),
            ("alb.admin", "ecs.admin@1c", "http"),
            ("ecs.admin@1a", "aurora.admin@1a", "read_write"),
            ("ecs.admin@1a", "aurora.admin@1c", "read"),
            ("ecs.admin@1c", "aurora.admin@1a", "read_write"),
            ("ecs.admin@1c", "aurora.admin@1c", "read"),
            ("aurora.web@1a", "aurora.web@1c", "replication"),
            ("aurora.admin@1a", "aurora.admin@1c", "replication"),
        ]);
        let part = |id: &str| m.nodes.iter().find(|n| n.id == id).unwrap().part.clone();
        assert!(!m.edges.iter().any(|e| part(&e.from).starts_with("aurora.") && part(&e.to).starts_with("aurora.") && part(&e.from) != part(&e.to)));
        assert!(!m.edges.iter().any(|e| (part(&e.from), part(&e.to)) == ("alb.web".into(), "ecs.admin".into())));
        assert!(m.edges.iter().all(|e| e.hole.is_none()));
    }

    // Fails if the parts inference adds are looked up by a service name instead of their role in the rules data.
    #[test]
    fn the_entry_gateway_and_nat_gateways_are_found_by_their_role() {
        let src = crate::test_inputs::WEB_CASES.iter().find(|(n, _)| *n == "web-2az-ja").unwrap().1;
        let services = include_str!("../../../../rules/aws/services.toml");
        let renamed = services.replace("key = \"igw\"", "key = \"door\"").replace("key = \"nat\"", "key = \"exit\"");
        assert_eq!(renamed.matches("key = \"door\"").count() + renamed.matches("key = \"exit\"").count(), 2);
        let rules = Rules::load(
            &renamed,
            include_str!("../../../../rules/aws/rules.toml"),
            include_str!("../../../../rules/aws/labels.toml"),
            include_str!("../../../../rules/aws/links.toml"),
        )
        .unwrap();
        let (before, after) = (vpc(src), vpc_with(src, &rules));
        let added = |m: &Model| m.nodes.iter().filter(|n| n.added).map(|n| (n.id.clone(), n.service.clone())).collect::<Vec<_>>();
        assert_eq!(added(&before), [("igw", "igw"), ("nat-1a", "nat"), ("nat-1c", "nat")].map(|(a, b)| (a.to_string(), b.to_string())));
        assert_eq!(added(&after), [("door", "door"), ("exit-1a", "exit"), ("exit-1c", "exit")].map(|(a, b)| (a.to_string(), b.to_string())));
        assert_eq!((after.nodes.len(), after.edges.len()), (before.nodes.len(), before.edges.len()));
    }

    // Fails if a definition with only a public tier loses a part or a line.
    #[test]
    fn a_public_tier_alone_builds_its_gateway() {
        let m = vpc(crate::test_inputs::LOAD_BALANCER_ALONE);
        assert_eq!(node_ids(&m), ["user", "igw", "alb"]);
        assert_eq!(lines(&m), [("user", "igw", "https"), ("igw", "alb", "https")]);
    }

    /// How the inference with a VPC and this one may differ for an input.
    #[derive(Debug, PartialEq)]
    enum Built {
        /// Both give a model, and the models are equal.
        Same,
        /// The checks here pass where the inference with a VPC stops.
        NewlyBuilt,
        /// The checks here stop where the inference with a VPC gives a model.
        NewlyStopped,
        /// Both stop.
        BothStop,
    }

    /// The inputs whose answer is not the same model, by their name in the lists.
    const NOT_SAME: [(&str, Built); 46] = [
        // Named VPC parts, and a load balancer with nothing behind it.
        ("GRID_INFER_TEST_DEFINITIONS#53", Built::NewlyBuilt),
        ("TWIN_APPS", Built::NewlyBuilt),
        ("TWO_WEBS", Built::NewlyBuilt),
        ("LOAD_BALANCER_ALONE", Built::NewlyBuilt),
        // `alb -> aurora` is not a listed connection.
        ("VPC_TEST_DEFINITIONS#25", Built::NewlyStopped),
        ("tests/cases/err-unknown-service", Built::BothStop),
        // A name, a setting, an answer or a connection outside the lists.
        ("GRID_INFER_TEST_DEFINITIONS#8", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#9", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#10", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#11", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#12", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#13", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#14", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#15", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#16", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#17", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#18", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#25", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#26", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#27", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#28", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#29", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#30", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#31", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#32", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#33", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#34", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#36", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#37", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#38", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#39", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#40", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#41", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#42", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#43", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#44", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#45", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#46", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#47", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#48", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#49", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#51", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#57", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#58", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#59", Built::BothStop),
        ("GRID_INFER_TEST_DEFINITIONS#60", Built::BothStop),
    ];
    const VPC_SAME_COUNT: usize = 1080;
    const VPC_NEWLY_BUILT_COUNT: usize = 4;
    const VPC_NEWLY_STOPPED_COUNT: usize = 1;
    const VPC_BOTH_STOP_COUNT: usize = 41;
    const VPC_INPUT_COUNT: usize = 1126;

    // Fails if a part, copy, area, line, label, hole id or the order of the model of a definition
    // with a VPC differs from the inference with a VPC outside the listed inputs, or an input drops out of the lists.
    #[test]
    fn definitions_with_a_vpc_build_the_same_model() {
        use crate::test_inputs::*;
        let mut inputs: Vec<(String, String)> = Vec::new();
        let mut list = |name: &str, srcs: Vec<String>| {
            for (i, src) in srcs.into_iter().enumerate() {
                inputs.push((format!("{name}#{i}"), src));
            }
        };
        let owned = |srcs: &[&str]| srcs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        list("WEB_EXAMPLES", owned(&WEB_EXAMPLES));
        list("VPC_TEST_DEFINITIONS", owned(&VPC_TEST_DEFINITIONS));
        list("heavy_variants", heavy_variants());
        list("generated_with_a_vpc", generated_with_a_vpc());
        list("GRID_INFER_TEST_DEFINITIONS", owned(&GRID_INFER_TEST_DEFINITIONS));
        for (name, src) in [
            ("CROWDED_S3_JA", CROWDED_S3_JA),
            ("CROWDED_SIDE_EN", CROWDED_SIDE_EN),
            ("FEWER_READERS", FEWER_READERS),
            ("TWIN_APPS", TWIN_APPS),
            ("TWO_WEBS", TWO_WEBS),
            ("LOAD_BALANCER_ALONE", LOAD_BALANCER_ALONE),
            ("tests/cases/err-unknown-service", include_str!("../../../../tests/cases/err-unknown-service/def.yaml")),
        ] {
            inputs.push((name.to_string(), src.to_string()));
        }

        let rules = Rules::builtin();
        let mut counts = [0usize; 4];
        for (id, src) in &inputs {
            let def = crate::def::parse(src).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            let (old, old_diag) = crate::infer::infer(&def, &rules);
            let mut diag = Diagnostics::default();
            let checked = check(&def, &rules, &mut diag);
            let want = NOT_SAME.iter().find(|(name, _)| name == id).map(|(_, b)| b).unwrap_or(&Built::Same);
            let got = match (&old, &checked) {
                (Some(_), Some(_)) => Built::Same,
                (None, Some(_)) => Built::NewlyBuilt,
                (Some(_), None) => Built::NewlyStopped,
                (None, None) => Built::BothStop,
            };
            assert_eq!(&got, want, "{id}:\n{src}\nold errors {:?}\nnew errors {:?}", old_diag.errors, diag.errors);
            if let Some(c) = &checked {
                assert!(c.vpc, "{id}");
                let model = build::vpc_model(&def, &rules, &c.settings);
                if let Some(old) = &old {
                    assert_eq!(old, &model, "{id}:\n{src}");
                }
            }
            counts[[Built::Same, Built::NewlyBuilt, Built::NewlyStopped, Built::BothStop].iter().position(|b| b == want).unwrap()] += 1;
        }
        assert_eq!(counts, [VPC_SAME_COUNT, VPC_NEWLY_BUILT_COUNT, VPC_NEWLY_STOPPED_COUNT, VPC_BOTH_STOP_COUNT]);
        assert_eq!(inputs.len(), VPC_INPUT_COUNT);
        assert_eq!(VPC_SAME_COUNT + VPC_NEWLY_BUILT_COUNT + VPC_NEWLY_STOPPED_COUNT + VPC_BOTH_STOP_COUNT, VPC_INPUT_COUNT);
        assert_eq!(NOT_SAME.len(), VPC_INPUT_COUNT - VPC_SAME_COUNT);
    }
}
