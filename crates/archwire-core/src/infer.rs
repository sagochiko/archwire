//! Inference: derives the semantic model from a definition and the rules data.
//!
//! It decides which concrete parts exist, in which Availability Zone, and how they
//! connect. Every part or default archwire adds gets one `Derived` entry, and every
//! route the connections alone cannot decide becomes a hole.

use crate::def::{Definition, Link, Rel};
use crate::diag::{Derived, Diagnostics, Hole, Problem, Versions};
use crate::model::{Edge, Model, Node};
use crate::rules::{Lang, Place, Rules, Scope, Service, Subnet};

const SETTINGS: [&str; 5] = ["azs", "lang", "nat", "readers", "region"];
const AZ_COUNTS: [&str; 2] = ["2", "3"];
const AZ_LETTERS: [&str; 3] = ["a", "c", "d"];
const REGIONS: [&str; 1] = ["ap-northeast-1"];
const LANGS: [&str; 2] = ["en", "ja"];
const NAT_MODES: [&str; 2] = ["per-az", "single"];
const ROUTE_CHOICES: [&str; 2] = ["endpoint", "nat"];
/// Parts a main flow into the load balancer may start from.
const ENTRIES: [&str; 2] = ["user", "cloudfront"];
const INTERNET: &str = "internet";
/// A certificate CloudFront uses lives in this Region, whatever the diagram's Region is.
const CERTIFICATE: &str = "acm";
const CLOUDFRONT: &str = "cloudfront";
const CLOUDFRONT_CERTIFICATE_REGION: &str = "us-east-1";

struct Settings {
    azs: usize,
    region: String,
    readers: usize,
    nat_single: bool,
    lang: Lang,
}

/// A main flow from a private part to a Regional service: its route is a hole.
struct Route {
    id: String,
    at: String,
    from: String,
    to: String,
}

fn problem(code: &str, at: &str, got: &str, allowed: Vec<String>, message: String) -> Problem {
    Problem { code: code.into(), at: Some(at.into()), got: Some(got.into()), allowed, rule: None, message }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| s.to_string()).collect()
}

fn is_private(s: &Service) -> bool {
    s.scope == Scope::Vpc && s.subnet == Some(Subnet::Private)
}

pub fn infer(def: &Definition, rules: &Rules) -> (Option<Model>, Diagnostics) {
    let mut diag = Diagnostics {
        versions: Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() },
        ..Default::default()
    };

    let names_ok = check_names(def, rules, &mut diag.errors);
    let untagged = check_no_tags(def, &mut diag.errors);
    let settings = check_settings(def, &mut diag.errors);
    let routes = find_routes(def, rules);
    check_choices(def, &routes, &mut diag.errors);
    if names_ok && untagged {
        check_shape(def, rules, &mut diag.errors);
    }
    if !diag.errors.is_empty() {
        return (None, diag);
    }

    let model = build(def, rules, &settings, &routes, &mut diag);
    (Some(model), diag)
}

/// Every part must be a writable service, and its kind one the service has.
/// Returns false if any name is unknown.
pub(crate) fn check_names(def: &Definition, rules: &Rules, errors: &mut Vec<Problem>) -> bool {
    let writable: Vec<String> =
        rules.service_keys().into_iter().filter(|k| rules.service(k).is_some_and(|s| !s.derived_only)).collect();
    let mut seen_at: Vec<&str> = Vec::new();
    let mut seen_option: Vec<&str> = Vec::new();
    let mut ok = true;
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]) {
        if seen_at.contains(&r.at.as_str()) {
            continue;
        }
        seen_at.push(&r.at);
        match rules.service(&r.service).filter(|s| !s.derived_only) {
            None => {
                ok = false;
                errors.push(problem(
                    "UNKNOWN_SERVICE",
                    &r.at,
                    &r.service,
                    writable.clone(),
                    format!("unknown service `{}`", r.service),
                ));
            }
            Some(s) => {
                let Some(opt) = &r.option else { continue };
                if !s.options.contains_key(opt) && !seen_option.contains(&r.name.as_str()) {
                    seen_option.push(&r.name);
                    errors.push(problem(
                        "UNKNOWN_OPTION",
                        &r.at,
                        opt,
                        s.options.keys().cloned().collect(),
                        format!("`{}` has no kind `{opt}`", r.name),
                    ));
                }
            }
        }
    }
    ok
}

/// The VPC layout draws one part per service, so a part with a name after the dot stops here,
/// once per part. Returns false if any part has one.
fn check_no_tags(def: &Definition, errors: &mut Vec<Problem>) -> bool {
    let mut seen_at: Vec<&str> = Vec::new();
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]).filter(|r| r.tag.is_some()) {
        if seen_at.contains(&r.at.as_str()) {
            continue;
        }
        seen_at.push(&r.at);
        errors.push(problem(
            "UNSUPPORTED",
            &r.at,
            &r.name,
            Vec::new(),
            "named parts such as `s3.web` are not supported together with VPC services yet".into(),
        ));
    }
    seen_at.is_empty()
}

fn bad_setting(errors: &mut Vec<Problem>, key: &str, got: &str, allowed: Vec<String>) {
    errors.push(problem("BAD_SETTING", &format!("set.{key}"), got, allowed, format!("`{got}` is not a valid value for `{key}`")));
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

fn check_settings(def: &Definition, errors: &mut Vec<Problem>) -> Settings {
    let get = |key: &str| def.set.get(key).map(String::as_str);

    let azs = match get("azs") {
        None => 2,
        Some(v) if AZ_COUNTS.contains(&v) => v.parse().expect("listed value"),
        Some(v) => {
            bad_setting(errors, "azs", v, strings(&AZ_COUNTS));
            2
        }
    };
    let region = check_region(def, errors);
    let lang = check_lang(def, errors);
    let nat_single = match get("nat") {
        None | Some("per-az") => false,
        Some("single") => true,
        Some(v) => {
            bad_setting(errors, "nat", v, strings(&NAT_MODES));
            false
        }
    };
    let readers = match get("readers") {
        None => azs - 1,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if (1..azs).contains(&n) => n,
            _ => {
                bad_setting(errors, "readers", v, (1..azs).map(|n| n.to_string()).collect());
                azs - 1
            }
        },
    };
    for key in def.set.keys().filter(|k| !SETTINGS.contains(&k.as_str())) {
        errors.push(problem("UNKNOWN_SETTING", &format!("set.{key}"), key, strings(&SETTINGS), format!("unknown setting `{key}`")));
    }
    Settings { azs, region, readers, nat_single, lang }
}

fn find_routes(def: &Definition, rules: &Rules) -> Vec<Route> {
    def.links
        .iter()
        .filter(|l| l.rel == Rel::Main)
        .filter(|l| {
            let from = rules.service(&l.from.name).is_some_and(is_private);
            let to = rules.service(&l.to.name).is_some_and(|s| s.scope == Scope::Regional);
            from && to
        })
        .map(|l| Route {
            id: format!("route.{}.{}", l.from.name, l.to.name),
            at: l.at.clone(),
            from: l.from.name.clone(),
            to: l.to.name.clone(),
        })
        .collect()
}

fn check_choices(def: &Definition, routes: &[Route], errors: &mut Vec<Problem>) {
    for (key, value) in &def.choose {
        let at = format!("choose.{key}");
        if !routes.iter().any(|r| &r.id == key) {
            let allowed = routes.iter().map(|r| r.id.clone()).collect();
            errors.push(problem("UNKNOWN_HOLE", &at, key, allowed, format!("there is no hole `{key}`")));
        } else if !ROUTE_CHOICES.contains(&value.as_str()) {
            errors.push(problem("BAD_CHOICE", &at, value, strings(&ROUTE_CHOICES), format!("`{value}` is not a choice for `{key}`")));
        }
    }
}

/// v0.1 draws one shape: a main flow from outside into the load balancer and from it into ECS.
/// Every link must also be an association an attachment allows, or one of the class
/// combinations in `allowed_link`.
fn check_shape(def: &Definition, rules: &Rules, errors: &mut Vec<Problem>) {
    let main = |from: &dyn Fn(&str) -> bool, to: &str| {
        def.links.iter().any(|l| l.rel == Rel::Main && from(&l.from.name) && l.to.name == to)
    };
    if !main(&|n| ENTRIES.contains(&n), "alb") || !main(&|n| n == "alb", "ecs") {
        errors.push(Problem {
            code: "UNSUPPORTED".into(),
            at: Some("flows".into()),
            message: "v0.1 needs a main flow from user or cloudfront into alb, and from alb into ecs".into(),
            ..Default::default()
        });
    }
    for l in &def.links {
        let (Some(from), Some(to)) = (class(&l.from.name, rules), class(&l.to.name, rules)) else { continue };
        let ok = match l.rel {
            Rel::Assoc => rules.service(&l.from.name).is_some_and(|s| s.attach_to.contains(&l.to.name)),
            rel => allowed_link(from, to, rel),
        };
        if !ok {
            errors.push(Problem {
                code: "UNSUPPORTED".into(),
                at: Some(l.at.clone()),
                got: Some(format!("{} {} {}", l.from.name, arrow(l.rel), l.to.name)),
                message: format!("v0.1 does not support `{} {} {}`", l.from.name, arrow(l.rel), l.to.name),
                ..Default::default()
            });
        }
    }
}

/// Where a part sits, as far as which flows make sense.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Outside,
    Internet,
    /// A global service that carries flows (`flow_source` in the rules data).
    Global,
    /// A service that is only associated with others (`attach_to`), never in a flow.
    Attachment,
    /// A global service that neither carries flows nor attaches: no flow accepts it.
    OtherGlobal,
    Public,
    Private,
    Regional,
}

fn class(name: &str, rules: &Rules) -> Option<Class> {
    let s = rules.service(name)?;
    Some(match s.scope {
        _ if !s.attach_to.is_empty() => Class::Attachment,
        Scope::Outside if name == INTERNET => Class::Internet,
        Scope::Outside => Class::Outside,
        Scope::Global if s.flow_source => Class::Global,
        Scope::Global => Class::OtherGlobal,
        Scope::Regional => Class::Regional,
        Scope::Vpc if s.subnet == Some(Subnet::Private) => Class::Private,
        Scope::Vpc => Class::Public,
    })
}

/// The flows v0.1 can place without claiming reachability that does not exist,
/// such as the internet reaching a private subnet directly.
fn allowed_link(from: Class, to: Class, rel: Rel) -> bool {
    use Class::*;
    matches!(
        (from, to, rel),
        (Outside, Global, Rel::Main)
            | (Outside, Public, Rel::Main)
            | (Global, Public, Rel::Main)
            | (Global, Regional, Rel::Main | Rel::Aux)
            | (Public, Private, Rel::Main)
            | (Public, Regional, Rel::Aux)
            | (Private, Private, Rel::Main)
            | (Private, Regional, Rel::Main | Rel::Aux)
            | (Private, Internet, Rel::Main)
    )
}

fn arrow(rel: Rel) -> &'static str {
    match rel {
        Rel::Main => "->",
        Rel::Aux => "~>",
        Rel::Assoc => "--",
    }
}

pub(crate) fn derived(rules: &Rules, lang: Lang, target: &str, id: &str, added: bool) -> Derived {
    let r = rules.rule(id);
    Derived {
        target: target.into(),
        added,
        kind: r.kind.clone(),
        rule: id.into(),
        reason: match lang {
            Lang::En => r.reason_en.clone(),
            Lang::Ja => r.reason_ja.clone(),
        },
        source: r.source.clone(),
    }
}

/// The id of the copy of a part in one Availability Zone, e.g. "ecs-1a" for "ecs" in ap-northeast-1a.
pub(crate) fn copy_id(part: &str, suffix: &str) -> String {
    format!("{part}-{suffix}")
}

/// `service` is both the service key and the written part: the VPC layout has no named parts.
fn node(id: String, service: &str, option: Option<String>, az: Option<usize>, role: Option<&str>, added: bool) -> Node {
    Node {
        id,
        service: service.into(),
        option,
        az,
        role: role.map(String::from),
        added,
        region: None,
        part: service.into(),
        tag: None,
        area: None,
    }
}

fn build(def: &Definition, rules: &Rules, s: &Settings, routes: &[Route], diag: &mut Diagnostics) -> Model {
    let svc = |name: &str| rules.service(name).expect("names were checked");
    let azs: Vec<String> = AZ_LETTERS[..s.azs].iter().map(|l| format!("{}{l}", s.region)).collect();
    let suffix = |i: usize| azs[i][azs[i].len() - 2..].to_string();
    let choice = |r: &Route| def.choose.get(&r.id).map(String::as_str);

    // Part names in the order they first appear, with their resolved kind.
    let mut parts: Vec<(&str, Option<String>)> = Vec::new();
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]) {
        if r.name != INTERNET && !parts.iter().any(|(n, _)| *n == r.name) {
            parts.push((&r.name, r.option.clone()));
        }
    }
    let in_group = |pred: &dyn Fn(&Service) -> bool| -> Vec<(&str, Option<String>)> {
        parts.iter().filter(|(n, _)| pred(svc(n))).cloned().collect()
    };

    // Private parts whose traffic leaves through NAT gateways.
    let mut egress: Vec<&str> = Vec::new();
    for l in def.links.iter().filter(|l| l.to.name == INTERNET) {
        egress.push(&l.from.name);
    }
    for r in routes.iter().filter(|r| choice(r) == Some("nat")) {
        if !egress.contains(&r.from.as_str()) {
            egress.push(&r.from);
        }
    }
    let nat_azs: Vec<usize> = match (egress.is_empty(), s.nat_single) {
        (true, _) => Vec::new(),
        (false, true) => vec![0],
        (false, false) => (0..s.azs).collect(),
    };
    let entry = def.links.iter().any(|l| l.rel == Rel::Main && ENTRIES.contains(&l.from.name.as_str()) && l.to.name == "alb");
    let cloudfront_certificate =
        def.links.iter().any(|l| l.rel == Rel::Assoc && l.from.name == CERTIFICATE && l.to.name == CLOUDFRONT);
    let igw = entry || !nat_azs.is_empty();

    // Nodes, in the order the diagram reads: outside, gateway, public, private rows, Regional.
    let mut nodes: Vec<Node> = Vec::new();
    for (name, opt) in in_group(&|v| matches!(v.scope, Scope::Outside | Scope::Global)) {
        let mut n = node(name.into(), name, opt, None, None, false);
        if name == CERTIFICATE && cloudfront_certificate {
            n.region = Some(CLOUDFRONT_CERTIFICATE_REGION.into());
        }
        nodes.push(n);
    }
    if igw {
        nodes.push(node("igw".into(), "igw", None, None, None, true));
    }
    for (name, opt) in in_group(&|v| v.scope == Scope::Vpc && !is_private(v)) {
        nodes.push(node(name.into(), name, opt, None, None, false));
    }
    for &i in &nat_azs {
        nodes.push(node(copy_id("nat", &suffix(i)), "nat", None, Some(i), None, true));
    }
    for (name, opt) in in_group(&is_private) {
        match svc(name).place {
            Place::PerAz => {
                for i in 0..s.azs {
                    nodes.push(node(copy_id(name, &suffix(i)), name, opt.clone(), Some(i), None, false));
                }
            }
            Place::WriterReader => {
                for i in 0..=s.readers {
                    let role = if i == 0 { "writer" } else { "reader" };
                    nodes.push(node(copy_id(name, &suffix(i)), name, opt.clone(), Some(i), Some(role), false));
                }
            }
            Place::Single | Place::Gateway | Place::Span => nodes.push(node(name.into(), name, opt, None, None, false)),
        }
    }
    for (name, opt) in in_group(&|v| v.scope == Scope::Regional) {
        nodes.push(node(name.into(), name, opt, None, None, false));
    }

    // Edges.
    let mut edges: Vec<Edge> = Vec::new();
    let find = |id: &str| nodes.iter().find(|n| n.id == id).expect("edge ends are nodes");
    let add = |edges: &mut Vec<Edge>, from: &Node, to: &Node, rel: Rel, hole: Option<String>, endpoint: bool| {
        if !edges.iter().any(|e| e.from == from.id && e.to == to.id && e.rel == rel) {
            let label = edge_label(from, to, rel, rules);
            edges.push(Edge { from: from.id.clone(), to: to.id.clone(), rel, label, hole, endpoint });
        }
    };
    let instances = |name: &str| nodes.iter().filter(|n| n.service == name && !n.added).collect::<Vec<_>>();
    for l in def.links.iter().filter(|l| l.to.name != INTERNET) {
        let hole = routes
            .iter()
            .find(|r| l.rel == Rel::Main && r.from == l.from.name && r.to == l.to.name && choice(r).is_none())
            .map(|r| r.id.clone());
        let endpoint = routes
            .iter()
            .any(|r| l.rel == Rel::Main && r.from == l.from.name && r.to == l.to.name && choice(r) == Some("endpoint"));
        if is_entry_into_vpc(l, rules) {
            for from in instances(&l.from.name) {
                add(&mut edges, from, find("igw"), Rel::Main, None, false);
            }
            for to in instances(&l.to.name) {
                add(&mut edges, find("igw"), to, Rel::Main, None, false);
            }
            continue;
        }
        for from in instances(&l.from.name) {
            for to in instances(&l.to.name) {
                add(&mut edges, from, to, l.rel, hole.clone(), endpoint);
            }
        }
    }
    for w in nodes.iter().filter(|n| n.role.as_deref() == Some("writer")) {
        for r in nodes.iter().filter(|n| n.service == w.service && n.role.as_deref() == Some("reader")) {
            add(&mut edges, w, r, Rel::Aux, None, false);
        }
    }
    for name in &egress {
        for from in instances(name) {
            let i = if s.nat_single { 0 } else { from.az.unwrap_or(0) };
            add(&mut edges, from, find(&copy_id("nat", &suffix(i))), Rel::Main, None, false);
        }
    }
    for &i in &nat_azs {
        add(&mut edges, find(&copy_id("nat", &suffix(i))), find("igw"), Rel::Main, None, false);
    }

    // Reasons for everything added, holes and notes.
    let lang = s.lang;
    // A target the definition did not write is something archwire added.
    let written: Vec<&str> = parts.iter().map(|(n, _)| *n).collect();
    let mut why = |target: &str, id: &str| diag.derived.push(derived(rules, lang, target, id, !written.contains(&target)));
    why("aws-cloud", "resources-inside-aws-cloud-frame");
    if !def.set.contains_key("region") {
        why("region", "default-region");
    }
    why("vpc", "load-balancer-in-vpc");
    why("az", "alb-spans-two-azs");
    if !def.set.contains_key("azs") {
        why("az", "default-two-azs");
    }
    why("subnet", "public-and-private-subnet-per-az");
    for (name, _) in &parts {
        let v = svc(name);
        match (v.scope, v.place) {
            (Scope::Global, _) if *name == CERTIFICATE && cloudfront_certificate => {
                why(name, "cloudfront-certificate-in-us-east-1")
            }
            (Scope::Global, _) => why(name, "global-service-outside-region"),
            (Scope::Regional, _) => why(name, "regional-service-outside-vpc"),
            (Scope::Vpc, Place::PerAz) if is_private(v) => why(name, "compute-in-private"),
            (Scope::Vpc, Place::WriterReader) => {
                why(name, "database-in-private");
                why(name, "aurora-writer-reader");
            }
            _ => {}
        }
    }
    if igw {
        why("igw", if entry { "internet-facing-alb-in-vpc-with-igw" } else { "private-egress-through-nat" });
    }
    for &i in &nat_azs {
        why(&copy_id("nat", &suffix(i)), "private-egress-through-nat");
    }
    if !nat_azs.is_empty() && !def.set.contains_key("nat") {
        why("nat", "nat-per-az");
    }

    for r in routes.iter().filter(|r| choice(r).is_none()) {
        diag.holes.push(Hole {
            hole: r.id.clone(),
            at: r.at.clone(),
            need: rules.text("need.route", lang).replace("{from}", &r.from).replace("{to}", &r.to),
            choices: strings(&ROUTE_CHOICES),
        });
    }
    if def.links.iter().any(|l| l.rel == Rel::Aux) {
        diag.notes.push(rules.text("note.aux_not_routed", lang));
    }
    if routes.iter().any(|r| choice(r) == Some("endpoint")) {
        diag.notes.push(rules.text("note.endpoint_not_drawn", lang));
    }

    Model { region: s.region.clone(), azs, lang: s.lang, nodes, edges }
}

/// A main flow from outside the VPC into a public part enters through the Internet Gateway.
fn is_entry_into_vpc(l: &Link, rules: &Rules) -> bool {
    let (Some(from), Some(to)) = (rules.service(&l.from.name), rules.service(&l.to.name)) else { return false };
    l.rel == Rel::Main
        && matches!(from.scope, Scope::Outside | Scope::Global)
        && to.scope == Scope::Vpc
        && to.subnet == Some(Subnet::Public)
}

/// A labels.toml key: the target's `in_label`, except where the pair says more.
fn edge_label(from: &Node, to: &Node, rel: Rel, rules: &Rules) -> String {
    let (f, t) = (from.service.as_str(), to.service.as_str());
    let key = match (from.role.as_deref(), to.role.as_deref()) {
        _ if rel == Rel::Assoc && (f == "waf" || t == "waf") => "web_acl",
        _ if rel == Rel::Assoc && (f == "acm" || t == "acm") => "certificate",
        (Some("writer"), Some("reader")) => "replication",
        // An auxiliary flow carries logs and metrics, not the target's main traffic.
        _ if rel == Rel::Aux => "logs_metrics",
        _ if f == "cloudfront" && t == "s3" => "static_files",
        (_, Some("writer")) => "read_write",
        (_, Some("reader")) => "read",
        _ if t == "nat" || (f == "nat" && t == "igw") => "outbound",
        _ => return rules.service(t).and_then(|s| s.in_label.clone()).unwrap_or_default(),
    };
    key.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::DerivedKind;

    fn run(src: &str) -> (Option<Model>, Diagnostics) { infer(&crate::def::parse(src).unwrap(), &Rules::builtin()) }
    fn ids(m: &Model) -> Vec<&str> { m.nodes.iter().map(|n| n.id.as_str()).collect() }
    fn has(m: &Model, a: &str, b: &str) -> bool { m.edges.iter().any(|e| e.from == a && e.to == b) }

    #[test]
    fn two_az_web_gets_igw_and_per_az_parts() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n");
        let m = m.unwrap();
        assert_eq!(m.azs, ["ap-northeast-1a", "ap-northeast-1c"]);
        assert_eq!(ids(&m), ["user","cloudfront","igw","alb","ecs-1a","ecs-1c","aurora-1a","aurora-1c"]);
        assert!(has(&m,"cloudfront","igw") && has(&m,"igw","alb") && !has(&m,"cloudfront","alb"));
        assert!(has(&m,"ecs-1c","aurora-1a") && has(&m,"aurora-1a","aurora-1c"));
        let igw = d.derived.iter().find(|x| x.target == "igw").unwrap();
        assert_eq!((igw.kind.clone(), igw.rule.as_str()), (DerivedKind::Constraint, "internet-facing-alb-in-vpc-with-igw"));
        assert!(igw.source.is_some());
        assert!(d.derived.iter().any(|x| x.rule == "default-two-azs" && x.kind == DerivedKind::Default));
        assert_eq!(d.exit_code(), 0);
    }

    #[test]
    fn three_azs_have_one_writer_and_two_readers() {
        let (m, _) = run("flows:\n  - user -> alb -> ecs -> aurora\nset:\n  azs: 3\n");
        let m = m.unwrap();
        assert_eq!(m.azs.len(), 3);
        let roles: Vec<_> = m.nodes.iter().filter(|n| n.service == "aurora").map(|n| n.role.clone().unwrap()).collect();
        assert_eq!(roles, ["writer", "reader", "reader"]);
        let n = |f: &str, t: &str| m.edges.iter().filter(|e| e.from.starts_with(f) && e.to.starts_with(t)).count();
        assert_eq!(n("ecs-", "aurora-"), 9);
        assert_eq!(n("aurora-", "aurora-"), 2);
    }

    #[test]
    fn private_to_regional_main_flow_is_a_hole_until_chosen() {
        let src = "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n";
        let (m, d) = run(src);
        assert_eq!(d.holes[0].hole, "route.ecs.s3");
        assert_eq!(d.holes[0].choices, ["endpoint", "nat"]);
        assert_eq!(d.exit_code(), 2);
        assert!(m.unwrap().edges.iter().any(|e| e.to == "s3" && e.hole.as_deref() == Some("route.ecs.s3")));
        let (m, d) = run(&format!("{src}choose:\n  route.ecs.s3: nat\n"));
        assert!(d.holes.is_empty());
        assert!(ids(&m.unwrap()).contains(&"nat-1a"));
    }

    #[test]
    fn hole_id_does_not_shift_when_lines_are_added() {
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - cloudfront -> s3\n  - user -> cloudfront\n  - ecs -> s3\n");
        assert_eq!(d.holes[0].hole, "route.ecs.s3");
    }

    #[test]
    fn outbound_adds_nat_per_az_or_single() {
        let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n");
        let m = m.unwrap();
        assert!(has(&m,"ecs-1c","nat-1c") && has(&m,"nat-1c","igw"));
        assert!(d.derived.iter().any(|x| x.rule == "nat-per-az"));
        let (m, _) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\nset:\n  nat: single\n");
        let m = m.unwrap();
        assert!(has(&m,"ecs-1c","nat-1a") && !ids(&m).contains(&"nat-1c"));
    }

    #[test]
    fn aux_flows_are_not_holes_and_leave_a_note() {
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs ~> cloudwatch\n");
        assert!(d.holes.is_empty());
        assert_eq!(d.notes.len(), 1);
    }

    #[test]
    fn out_of_scope_shapes_stop_with_unsupported() {
        for src in ["flows:\n  - user -> s3\n", "flows:\n  - user -> cloudfront -> s3\n", "flows:\n  - user -> alb -> aurora\n"] {
            let (m, d) = run(src);
            assert!(m.is_none());
            assert_eq!(d.errors[0].code, "UNSUPPORTED");
        }
    }

    #[test]
    fn unknown_names_and_bad_settings_list_allowed_values() {
        let (_, d) = run("flows:\n  - user -> cloudfrnt -> alb -> ecs\n");
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNKNOWN_SERVICE", Some("flows[0][1]"), Some("cloudfrnt")));
        assert!(d.errors[0].allowed.contains(&"cloudfront".to_string()) && !d.errors[0].allowed.contains(&"igw".to_string()));
        for (k, v) in [("azs","4"), ("azs","two"), ("lang","fr"), ("region","us-east-1"), ("nat","none")] {
            let (_, d) = run(&format!("flows:\n  - user -> alb -> ecs\nset:\n  {k}: {v}\n"));
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("BAD_SETTING", Some(&*format!("set.{k}"))));
            assert!(!d.errors[0].allowed.is_empty());
        }
        let (_, d) = run("flows:\n  - user -> alb -> ecs\nset:\n  colour: red\n");
        assert_eq!(d.errors[0].code, "UNKNOWN_SETTING");
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: tunnel\n");
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].allowed.clone()), ("BAD_CHOICE", vec!["endpoint".to_string(), "nat".to_string()]));
        let (_, d) = run("flows:\n  - user -> alb -> ecs\nchoose:\n  route.ecs.rds: nat\n");
        assert_eq!(d.errors[0].code, "UNKNOWN_HOLE");
        let (_, d) = run("flows:\n  - user -> alb -> ecs(lambda)\n");
        assert_eq!(d.errors[0].code, "UNKNOWN_OPTION");
    }
    fn label(m: &Model, a: &str, b: &str) -> String { m.edges.iter().find(|e| e.from == a && e.to == b).unwrap().label.clone() }

    #[test]
    fn edge_labels_follow_the_pair() {
        let src = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\n  - acm -- cloudfront\n";
        let m = run(src).0.unwrap();
        assert_eq!(label(&m, "user", "cloudfront"), "https");
        assert_eq!(label(&m, "cloudfront", "igw"), "https");
        assert_eq!(label(&m, "alb", "ecs-1a"), "http");
        assert_eq!(label(&m, "ecs-1a", "aurora-1a"), "read_write");
        assert_eq!(label(&m, "ecs-1a", "aurora-1c"), "read");
        assert_eq!(label(&m, "aurora-1a", "aurora-1c"), "replication");
        assert_eq!(label(&m, "cloudfront", "s3"), "static_files");
        assert_eq!(label(&m, "ecs-1a", "nat-1a"), "outbound");
        assert_eq!(label(&m, "nat-1a", "igw"), "outbound");
        assert_eq!(label(&m, "ecs-1c", "cloudwatch"), "logs_metrics");
        assert_eq!(label(&m, "waf", "cloudfront"), "web_acl");
        assert_eq!(label(&m, "acm", "cloudfront"), "certificate");
        assert!(!ids(&m).contains(&"internet"));
        assert_eq!(ids(&m), ["user","cloudfront","waf","acm","igw","alb","nat-1a","nat-1c","ecs-1a","ecs-1c","aurora-1a","aurora-1c","s3","cloudwatch"]);
    }

    #[test]
    fn an_auxiliary_flow_is_labelled_logs_and_metrics_whatever_its_target() {
        let m = run("flows:\n  - user -> cloudfront -> alb -> ecs\n  - alb ~> s3\n  - cloudfront ~> s3\n").0.unwrap();
        assert_eq!(label(&m, "alb", "s3"), "logs_metrics");
        assert_eq!(label(&m, "cloudfront", "s3"), "logs_metrics");
    }

    #[test]
    fn explicit_settings_suppress_their_defaults() {
        let (m, d) = run("flows:\n  - user -> alb -> ecs -> aurora\n  - ecs -> internet\nset:\n  azs: 3\n  region: ap-northeast-1\n  nat: per-az\n  readers: 1\n  lang: ja\n");
        let m = m.unwrap();
        for rule in ["default-two-azs", "default-region", "nat-per-az"] {
            assert!(!d.derived.iter().any(|x| x.rule == rule), "{rule}");
        }
        assert_eq!(ids(&m).iter().filter(|i| i.starts_with("aurora")).count(), 2);
        assert_eq!(m.lang, Lang::Ja);
        let igw = d.derived.iter().find(|x| x.target == "igw").unwrap();
        assert_eq!(igw.reason, Rules::builtin().rule("internet-facing-alb-in-vpc-with-igw").reason_ja);
        assert_eq!(d.versions.rules, Rules::builtin().version);
        assert!(!d.versions.archwire.is_empty());
    }

    #[test]
    fn every_added_part_has_one_derived() {
        let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n");
        for n in m.unwrap().nodes.iter().filter(|n| n.added) {
            assert_eq!(d.derived.iter().filter(|x| x.target == n.id).count(), 1, "{}", n.id);
        }
    }

    #[test]
    fn endpoint_choice_adds_no_part_and_leaves_a_note() {
        let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\n");
        let m = m.unwrap();
        assert!(d.holes.is_empty() && d.notes.len() == 1);
        assert!(!ids(&m).iter().any(|i| i.starts_with("nat")));
        assert!(m.edges.iter().all(|e| e.hole.is_none()));
    }

    #[test]
    fn readers_and_internet_misuse_and_several_errors_at_once() {
        let (_, d) = run("flows:\n  - user -> alb -> ecs\nset:\n  readers: 2\n");
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].allowed.clone()), ("BAD_SETTING", vec!["1".to_string()]));
        let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - internet -> alb\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[1]")));
        let (_, d) = run("flows:\n  - user -> alb -> ecs -> nat\nset:\n  azs: 5\n  lang: fr\n");
        let codes: Vec<_> = d.errors.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(codes, ["UNKNOWN_SERVICE", "BAD_SETTING", "BAD_SETTING"]);
    }
    #[test]
    fn links_outside_the_allowlist_stop_with_unsupported() {
        let base = "flows:\n  - user -> alb -> ecs -> aurora\n";
        for line in ["cloudfront -> ecs", "user -> aurora", "user ~> cloudfront", "ecs -> user", "ecs -> cloudfront", "alb -> s3", "user -> s3"] {
            let (m, d) = run(&format!("{base}  - {line}\n"));
            assert!(m.is_none(), "{line}");
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[1]")), "{line}");
        }
        assert!(run(&format!("{base}  - aurora -> ecs\n")).0.is_some());
    }

    #[test]
    fn attachments_attach_only_to_cloudfront_and_carry_no_flows() {
        let base = "flows:\n  - user -> cloudfront -> alb -> ecs\n";
        for line in [
            "waf -- alb", "acm -- alb", "waf -- acm", "acm -- waf", "cloudfront -- waf", "cloudfront -- acm", "alb -- acm",
            "user -- alb", "alb -- cloudfront", "waf -> alb", "acm -> s3", "waf ~> cloudwatch", "user -> waf", "cloudfront -> acm",
        ] {
            let (m, d) = run(&format!("{base}  - {line}\n"));
            assert!(m.is_none(), "{line}");
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[1]")), "{line}");
        }
    }

    #[test]
    fn waf_and_acm_on_cloudfront_compile_and_the_certificate_is_in_us_east_1() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> alb -> ecs\n  - waf -- cloudfront\n  - acm -- cloudfront\n");
        let m = m.unwrap();
        assert_eq!(d.exit_code(), 0);
        let acm = m.nodes.iter().find(|n| n.id == "acm").unwrap();
        assert_eq!(acm.region.as_deref(), Some("us-east-1"));
        let why: Vec<_> = d.derived.iter().filter(|x| x.target == "acm").map(|x| x.rule.as_str()).collect();
        assert_eq!(why, ["cloudfront-certificate-in-us-east-1"]);
        let rule = Rules::builtin().rule("cloudfront-certificate-in-us-east-1").clone();
        assert_eq!(rule.kind, DerivedKind::Constraint);
        let why: Vec<_> = d.derived.iter().filter(|x| x.target == "waf").map(|x| x.rule.as_str()).collect();
        assert_eq!(why, ["global-service-outside-region"]);
        assert_eq!(m.nodes.iter().find(|n| n.id == "waf").unwrap().region, None);
    }

    #[test]
    fn an_auxiliary_flow_from_outside_into_the_load_balancer_is_unsupported() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> alb -> ecs\n  - user ~> alb\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[1]")));
    }

    #[test]
    fn every_allowed_combination_compiles() {
        let src = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> alb\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - ecs -> internet\n  - waf -- cloudfront\n  - acm -- cloudfront\nchoose:\n  route.ecs.s3: endpoint\n";
        let (m, d) = run(src);
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert!(m.is_some());
        assert_eq!(d.exit_code(), 0);
    }

    #[test]
    fn named_parts_are_not_supported_with_vpc_services_yet() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> alb -> ecs\n  - cloudfront -> s3.web\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNSUPPORTED", Some("flows[1][1]"), Some("s3.web")));
    }

    #[test]
    fn every_node_knows_the_part_it_stands_for() {
        let m = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n").0.unwrap();
        let part = |id: &str| m.nodes.iter().find(|n| n.id == id).unwrap().part.clone();
        assert_eq!([part("ecs-1a"), part("nat-1c"), part("alb"), part("igw")], ["ecs", "nat", "alb", "igw"]);
        assert!(m.nodes.iter().all(|n| n.tag.is_none() && n.area.is_none()));
    }

    #[test]
    fn named_parts_never_share_an_id_with_a_copy_or_make_two_lines_alike() {
        let src = "flows:\n  - user -> lambda.1a -> lambda\n  - lambda.a-to-b -> s3\n  - lambda.a -> b\n  - lambda.a -> b.to-s3\n  - s3.x-to-y -> lambda.a-to-b\n  - lambda.a-to-s3 -> s3.x\n";
        let d = crate::def::parse(src).unwrap();
        let links: Vec<_> = d.links.iter().map(|l| (l.from.name.as_str(), l.to.name.as_str())).collect();
        assert_eq!(links, [
            ("user", "lambda.1a"), ("lambda.1a", "lambda"), ("lambda.a-to-b", "s3"), ("lambda.a", "b"),
            ("lambda.a", "b.to-s3"), ("s3.x-to-y", "lambda.a-to-b"), ("lambda.a-to-s3", "s3.x"),
        ]);
        // Every written part, and a copy per Availability Zone of every part without a name after the dot.
        let mut ids: Vec<String> = Vec::new();
        for r in d.links.iter().flat_map(|l| [&l.from, &l.to]) {
            if !ids.contains(&r.name) {
                ids.push(r.name.clone());
            }
            if r.tag.is_none() {
                for suffix in ["1a", "1c", "1d"] {
                    let copy = copy_id(&r.name, suffix);
                    if !ids.contains(&copy) {
                        ids.push(copy);
                    }
                }
            }
        }
        assert!(ids.contains(&"lambda.1a".to_string()) && ids.contains(&"lambda-1a".to_string()));
        let written = ids.iter().filter(|i| d.links.iter().any(|l| &l.from.name == *i || &l.to.name == *i)).count();
        let copies = ids.len() - written;
        assert_eq!((written, copies), (11, 12));
        // A line id names exactly one ordered pair of nodes, so it points at the parts it joins.
        for (from, to) in &links {
            let id = crate::route::edge_id(from, to);
            let pairs: Vec<_> = ids.iter().flat_map(|a| ids.iter().map(move |b| (a, b)))
                .filter(|(a, b)| a != b && crate::route::edge_id(a, b) == id).collect();
            assert_eq!(pairs.len(), 1, "{id}");
            assert_eq!((pairs[0].0.as_str(), pairs[0].1.as_str()), (*from, *to), "{id}");
        }
        let mut lines: Vec<String> = ids.iter().flat_map(|a| ids.iter().filter(move |b| *b != a).map(move |b| crate::route::edge_id(a, b))).collect();
        let n = lines.len();
        lines.sort();
        lines.dedup();
        assert_eq!(lines.len(), n);
    }

    #[test]
    fn notes_and_needs_follow_lang() {
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n  - ecs ~> cloudwatch\nset:\n  lang: ja\n");
        assert_eq!(d.holes[0].need, "ecs から s3 への経路");
        assert_eq!(d.notes, ["補助の流れ（~>）は経路を扱わず、線だけを描く。"]);
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\nset:\n  lang: ja\n");
        assert_eq!(d.notes, ["VPC エンドポイントのアイコンは描かない。"]);
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n");
        assert_eq!(d.holes[0].need, "Route from ecs to s3");
    }
}
