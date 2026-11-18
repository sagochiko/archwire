//! Inference: checks a definition against the rules data and derives the semantic model.
//!
//! The same checks run for every definition, in this order: names, settings, connections,
//! whether the main flows are reached, limits, and the answers given to holes. Whether a
//! definition has a VPC is decided before them, from the scope of the services it writes.
//! A definition without a VPC part is then built with every written part as one node and every
//! connection as one edge, with the labels, limits and reasons of links.toml. A definition with
//! one also gets per-zone copies of its private parts, the gateways its traffic needs, and the
//! reasons for the VPC, its zones and subnets and every part added, from the same checked result.

mod build;
mod links;
mod reasons;

pub(crate) use links::check_names;

use crate::def::{Definition, Link, Ref};
use crate::diag::{Diagnostics, Problem, Versions};
use crate::model::{Area, Axis, Model, Tier};
use crate::rules::{Lang, LinkRule, Place, Rules, Scope, Service, Subnet};

/// The rules this module's code names itself; the rest come from the data, which loading checks.
pub(crate) const RULES: [&str; 15] = [
    "resources-inside-aws-cloud-frame",
    "resources-inside-region-frame",
    "default-region",
    "global-service-outside-region",
    "regional-service-outside-vpc",
    "public-and-private-subnet-per-az",
    "public-subnet-per-az",
    "private-subnet-per-az",
    "compute-in-private",
    "database-in-private",
    "aurora-writer-reader",
    "internet-facing-alb-in-vpc-with-igw",
    "private-egress-through-nat",
    "nat-device-is-nat-gateway",
    "nat-per-az",
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
    /// Kind holes, answered or not, in the order the parts first appear.
    kinds: Vec<(String, &'a Ref)>,
    /// Route holes, answered or not, with the line each is on, in the order written.
    routes: Vec<(String, &'a Link)>,
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
    let model = if checked.vpc {
        // What the rules data lacks for this definition stops it, as a rule the code names does.
        let mut errors = Vec::new();
        if reasons::vpc_reasons(rules, &checked.parts).is_none() {
            errors.push(Problem {
                code: "RULE_MISSING".into(),
                at: Some("vpc_reasons".into()),
                message: "no service of a part in the definition has `vpc_reasons` in services.toml".into(),
                ..Default::default()
            });
        }
        let model = build::vpc_model(def, rules, &checked).map_err(|e| errors.extend(e)).ok();
        match model {
            Some(m) if errors.is_empty() => m,
            _ => {
                diag.errors = errors;
                return (None, diag);
            }
        }
    } else {
        build::build(def, rules, &checked.parts, &checked.accepted, &checked.kinds, &checked.settings)
    };
    reasons::explain(def, rules, &model, &checked, &mut diag);
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
    let routes = links::route_holes(def, rules, &accepted);
    links::check_choices(def, rules, &kinds, &routes, errors);
    if !diag.errors.is_empty() {
        return None;
    }
    Some(Checked { vpc, settings, parts, accepted, kinds, routes })
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
    use super::reasons::derived;
    use super::*;
    use crate::def::Rel::{self, Aux, Main};

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
        let runs: [(&[&str], &Definition, Infer); 2] = [(&RULES, &api, infer), (&RULES, &web, infer)];
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

    /// Rust source read once: the code without its line comments, and each ordinary string in
    /// it, with the place in that code where the string opens.
    struct Code {
        text: String,
        strings: Vec<(usize, String)>,
    }

    /// Reads Rust source in one pass. Line comments (`//` to the end of the line, outside a string
    /// or a character literal) are left out, and each ordinary string is kept as written between
    /// its quotes. Only line comments, ordinary strings closed on their own line and character
    /// literals without a `"` are read; any other way of writing that could hide or split a string
    /// (`/*`, a raw string `r"` or `r#"`, a string left open at the end of its line, a character
    /// literal holding `"`) is an error naming the file and the line. A `'` opens a character
    /// literal when one character, or one escape, and a `'` follow it; otherwise it is a lifetime
    /// or a label.
    fn read_code(file: &str, src: &str) -> Result<Code, String> {
        let ident = |c: Option<&u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        let mut code = Code { text: String::new(), strings: Vec::new() };
        for (n, line) in src.lines().enumerate() {
            let at = |what: &str| format!("{file}:{}: {what} is not read by this test; write it with `//` and strings on one line", n + 1);
            let bytes = line.as_bytes();
            let base = code.text.len();
            let (mut open, mut escaped, mut end) = (None::<usize>, false, line.len());
            let mut i = 0;
            while i < bytes.len() {
                let c = bytes[i];
                if let Some(from) = open {
                    match c {
                        _ if escaped => escaped = false,
                        b'\\' => escaped = true,
                        b'"' => {
                            code.strings.push((base + from, line[from + 1..i].to_string()));
                            open = None;
                        }
                        _ => {}
                    }
                    i += 1;
                    continue;
                }
                let before = |k: usize| if i >= k { bytes.get(i - k) } else { None };
                match c {
                    b'"' => open = Some(i),
                    b'\'' => {
                        let close = if bytes.get(i + 1) == Some(&b'\\') {
                            bytes.get(i + 3..).and_then(|rest| rest.iter().position(|&b| b == b'\'')).map(|p| i + 3 + p)
                        } else {
                            let len = line[i + 1..].chars().next().map_or(0, char::len_utf8);
                            (len > 0 && bytes.get(i + 1 + len) == Some(&b'\'')).then_some(i + 1 + len)
                        };
                        if let Some(close) = close {
                            if line[i + 1..close].contains('"') {
                                return Err(at("a character literal holding `\"`"));
                            }
                            i = close + 1;
                            continue;
                        }
                    }
                    b'/' if bytes.get(i + 1) == Some(&b'/') => {
                        end = i;
                        break;
                    }
                    b'/' if bytes.get(i + 1) == Some(&b'*') => return Err(at("a block comment `/*`")),
                    b'*' if bytes.get(i + 1) == Some(&b'/') => return Err(at("the end of a block comment `*/`")),
                    b'r' if matches!(bytes.get(i + 1), Some(b'"') | Some(b'#'))
                        && (!ident(before(1)) || (before(1) == Some(&b'b') && !ident(before(2)))) =>
                    {
                        return Err(at("a raw string"))
                    }
                    _ => {}
                }
                i += 1;
            }
            if open.is_some() {
                return Err(at("a string that goes on past its line"));
            }
            code.text.push_str(&line[..end]);
            code.text.push('\n');
        }
        Ok(code)
    }

    // Fails if a rule id written only in a line comment is counted as one the code names, a string
    // is read apart from the comments around it, or a way of writing the reading does not follow
    // (`/*`, raw strings, strings over lines, a `"` in a character literal) passes.
    #[test]
    fn a_rule_named_only_in_a_comment_is_not_used() {
        let strings = |src: &str| read_code("x.rs", src).unwrap().strings.into_iter().map(|(_, s)| s).collect::<Vec<_>>();
        let src = "let a = \"nat-per-az\"; // \"default-region\"\n/// \"compute-in-private\"\nlet b = \"a // b \\\" c\";\n";
        let code = read_code("x.rs", src).unwrap();
        assert_eq!(code.text, "let a = \"nat-per-az\"; \n\nlet b = \"a // b \\\" c\";\n");
        assert_eq!(code.strings, [(8, "nat-per-az".to_string()), (32, "a // b \\\" c".to_string())]);
        let fails = [
            ("let a = 1;\nlet b = 2; /* \"default-region\" */\n", "x.rs:2: a block comment `/*`"),
            ("let a = r\"default-region\";\n", "x.rs:1: a raw string"),
            ("let a = r#\"de\"fault\"#;\n", "x.rs:1: a raw string"),
            ("let a = br\"x\";\n", "x.rs:1: a raw string"),
            ("let a = 1;\n\nlet b = \"one\ntwo\";\n", "x.rs:3: a string that goes on past its line"),
            ("let a = '\"'; /* comment */ let b = '\"';\n", "x.rs:1: a character literal holding `\"`"),
            ("let a = 1;\nlet b = b'\"';\n", "x.rs:2: a character literal holding `\"`"),
            ("let a = '\\\"';\n", "x.rs:1: a character literal holding `\"`"),
        ];
        for (src, want) in fails {
            let e = read_code("x.rs", src).err().unwrap_or_else(|| panic!("{src:?} was read"));
            assert!(e.starts_with(want), "{src:?}: {e}");
        }
        // Names that only end in `r` before a string are not raw strings.
        assert!(read_code("x.rs", "let a = for_r(\"x\"); let b = matches!(c, br);\n").is_ok());
        // Lifetimes, labels and character literals without `"` are read past, and a `/` or a `'`
        // in a character literal neither starts a comment nor hides a string.
        assert_eq!(strings("fn f<'a>(x: &'a str) -> &'a str { 'outer: loop { break 'outer; } x }\n"), Vec::<String>::new());
        assert_eq!(strings("let c = '/'; let d = '\\''; let s = \"nat-per-az\"; // \"default-region\"\n"), ["nat-per-az"]);
        assert_eq!(strings("let c = 'é'; let s = \"nat-per-az\"; let u = '\\u{1F600}';\n"), ["nat-per-az"]);
    }

    // Fails if the code names a rule missing from RULES, or a file of this module is not read for it,
    // or a rule id in a comment is counted as named.
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
        // The rules the code uses: every string the rules data knows as a rule, outside the tests
        // and outside the declaration of RULES itself.
        let mut used: Vec<String> = Vec::new();
        for (name, src) in SOURCES {
            let code = read_code(name, src.find("#[cfg(test)]").map_or(src, |i| &src[..i])).unwrap_or_else(|e| panic!("{e}"));
            let declared = code.text.find("pub(crate) const RULES").map(|start| {
                let end = start + code.text[start..].find("];").expect("the list ends") + 2;
                assert!(name == "mod.rs" && code.text[end..].find("const RULES").is_none(), "RULES is declared once, in mod.rs");
                start..end
            });
            for (_, lit) in code.strings.iter().filter(|(at, _)| !declared.as_ref().is_some_and(|d| d.contains(at))) {
                let lit = lit.as_str();
                if r.require(&[lit]).is_empty() && !used.iter().any(|u| u == lit) {
                    used.push(lit.to_string());
                }
            }
        }
        let mut listed: Vec<String> = RULES.iter().map(|s| s.to_string()).collect();
        listed.sort();
        let n = listed.len();
        listed.dedup();
        assert_eq!(listed.len(), n, "a rule is listed twice in RULES");
        used.sort();
        assert_eq!(used, listed, "RULES must hold exactly the rules the code names");
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

    // Fails if an association to a part off the main flows is accepted.
    #[test]
    fn a_web_acl_on_an_unreached_distribution_stops() {
        let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - waf -- cloudfront\n");
        assert!(m.is_none());
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some("waf -- cloudfront")));
        assert_eq!(d.errors[0].message, "`waf -- cloudfront` needs `cloudfront` on a main flow (->) from `cloudtrail`, `cloudwatch`, `config`, `ec2`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`");
    }

    #[test]
    fn pairs_outside_the_list_stop() {
        for line in [
            "user -> dynamodb",
            "cloudfront -> lambda",
            "dynamodb -> lambda",
            "apigateway -- cognito",
            "waf -- apigateway",
            "sns -> eventbridge",
            "securityhub -> guardduty",
            "guardduty ~> eventbridge",
            "s3 -> athena",
            "athena -- glue",
            "athena -> glue",
            "s3 -> securitylake",
            "sqs -> securitylake",
            "securitylake -> athena",
            "guardduty -> securitylake",
            "waf -> securitylake",
            "cognito -> user",
            "user -> sqs",
            "user -> securitylake",
        ] {
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
        assert_eq!(d.errors[0].message, "`cloudfront ~> cloudwatch` needs `cloudfront` on a main flow (->) from `cloudtrail`, `cloudwatch`, `config`, `ec2`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`");
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

    // Fails if a detection service stops being a starting point, or the list of starting points grows by accident.
    #[test]
    fn the_detection_flows_start_from_their_services() {
        let defs = [
            "flows:\n  - guardduty -> securityhub -> eventbridge -> sns\n  - inspector -> securityhub\n  - config -> securityhub\nset:\n  lang: ja\n",
            "flows:\n  - guardduty -> securityhub -> eventbridge -> stepfunctions\n  - stepfunctions -> ssm\n  - stepfunctions -> sns\n",
            "flows:\n  - cloudtrail -> cloudwatch -> sns\n  - cloudtrail -> s3\n",
            "flows:\n  - config -> ssm\n",
        ];
        for src in defs {
            let (m, d) = run(src);
            assert!(d.errors.is_empty(), "{src}: {:?}", d.errors);
            assert!(m.is_some(), "{src}");
            assert_eq!(d.exit_code(), 0, "{src}");
        }
    }

    // Fails if a service that only receives starts a flow.
    #[test]
    fn a_flow_from_a_receiving_service_still_stops() {
        let (m, d) = run("flows:\n  - stepfunctions -> sns\n");
        assert!(m.is_none());
        assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[0]"), Some("stepfunctions -> sns")));
        assert_eq!(
            d.errors[0].message,
            "`stepfunctions -> sns` is not reached by main flows (->) from `cloudtrail`, `cloudwatch`, `config`, `ec2`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`"
        );
        let (m, d) = run("flows:\n  - lambda -> dynamodb\n");
        assert!(m.is_none());
        assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[0]"), Some("lambda -> dynamodb")));
    }

    // Fails if an alarm reached only by a side line cannot start the notification.
    // Fails if one of the pairs of the detection flows stops, gets another label, or is drawn
    // without its label in English or Japanese. A pair from Step Functions, which starts no flow,
    // is reached through EventBridge.
    #[test]
    fn every_detection_pair_is_accepted_with_its_label() {
        let r = Rules::builtin();
        let pairs = [
            ("guardduty", "eventbridge", "findings", "Findings", "検出結果"),
            ("guardduty", "securityhub", "findings", "Findings", "検出結果"),
            ("inspector", "eventbridge", "findings", "Findings", "検出結果"),
            ("inspector", "securityhub", "findings", "Findings", "検出結果"),
            ("securityhub", "eventbridge", "findings", "Findings", "検出結果"),
            ("config", "eventbridge", "compliance_changes", "Compliance changes", "準拠状態の変化"),
            ("config", "securityhub", "findings", "Findings", "検出結果"),
            ("config", "sns", "notifications", "Notifications", "通知"),
            ("config", "ssm", "remediation", "Remediation", "自動修復"),
            ("cloudtrail", "s3", "log_files", "Log files", "ログファイル"),
            ("cloudtrail", "cloudwatch", "logs", "Logs", "ログ"),
            ("cloudtrail", "eventbridge", "api_calls", "API calls", "API 呼び出し"),
            ("cloudwatch", "sns", "alarms", "Alarms", "アラーム"),
            ("eventbridge", "sns", "by_rule", "Rule", "ルール"),
            ("eventbridge", "stepfunctions", "by_rule", "Rule", "ルール"),
            ("eventbridge", "ssm", "by_rule", "Rule", "ルール"),
            ("stepfunctions", "sns", "publish", "Publish", "発行"),
            ("stepfunctions", "lambda", "invoke", "Invoke", "呼び出し"),
            ("stepfunctions", "ssm", "call", "Call", "呼び出し"),
        ];
        assert_eq!(pairs.len(), 19);
        for (from, to, key, en, ja) in pairs {
            let flows = if from == "stepfunctions" {
                format!("flows:\n  - eventbridge -> stepfunctions\n  - stepfunctions -> {to}\n")
            } else {
                format!("flows:\n  - {from} -> {to}\n")
            };
            let (m, d) = run(&flows);
            assert!(d.errors.is_empty(), "{from} -> {to}: {:?}", d.errors);
            let m = m.unwrap_or_else(|| panic!("{from} -> {to}: no model"));
            assert_eq!(label(&m, from, to), key, "{from} -> {to}");
            assert_eq!((r.text(key, Lang::En).as_str(), r.text(key, Lang::Ja).as_str()), (en, ja), "{from} -> {to}");
            for (lang, text) in [("en", en), ("ja", ja)] {
                let out = crate::compile(&format!("{flows}set:\n  lang: {lang}\n"));
                assert_eq!(out.diagnostics.exit_code(), 0, "{from} -> {to} {lang}: {:?}", out.diagnostics.errors);
                let drawio = out.drawio.unwrap_or_else(|| panic!("{from} -> {to} {lang}: no diagram"));
                assert!(drawio.contains(&format!("value=\"{text}\"")), "{from} -> {to} {lang}: no label {text}");
            }
        }
    }

    // Fails if one of the pairs of the sign-in and log analysis flows stops, gets another label,
    // or is drawn without its label in English or Japanese. A pair whose sender starts no flow is
    // reached from the user or from CloudTrail; the Glue Data Catalog hangs on an Athena that the
    // user reaches.
    #[test]
    fn every_log_analysis_pair_is_accepted_with_its_label() {
        let r = Rules::builtin();
        let pairs = [
            ("user", "cognito", "sign_in", "Sign-in", "サインイン"),
            ("user", "s3", "upload_download", "Upload/Download", "アップロード・ダウンロード"),
            ("user", "athena", "query", "Query", "クエリ"),
            ("athena", "s3", "query", "Query", "クエリ"),
            ("glue", "athena", "table_definitions", "Table definitions", "テーブル定義"),
            ("cloudtrail", "securitylake", "logs", "Logs", "ログ"),
            ("securityhub", "securitylake", "findings", "Findings", "検出結果"),
            ("securitylake", "s3", "data_lake", "Data lake", "データレイク"),
            ("securitylake", "sqs", "new_objects", "New objects", "新しいオブジェクトの通知"),
            ("athena", "securitylake", "query", "Query", "クエリ"),
        ];
        assert_eq!(pairs.len(), 10);
        for (from, to, key, en, ja) in pairs {
            let flows = match from {
                "athena" => format!("flows:\n  - user -> athena -> {to}\n"),
                "glue" => "flows:\n  - user -> athena -> s3\n  - glue -- athena\n".to_string(),
                "securitylake" => format!("flows:\n  - cloudtrail -> securitylake -> {to}\n"),
                _ => format!("flows:\n  - {from} -> {to}\n"),
            };
            let (m, d) = run(&flows);
            assert!(d.errors.is_empty(), "{from} -> {to}: {:?}", d.errors);
            let m = m.unwrap_or_else(|| panic!("{from} -> {to}: no model"));
            assert_eq!(label(&m, from, to), key, "{from} -> {to}");
            assert_eq!((r.text(key, Lang::En).as_str(), r.text(key, Lang::Ja).as_str()), (en, ja), "{from} -> {to}");
            for (lang, text) in [("en", en), ("ja", ja)] {
                let out = crate::compile(&format!("{flows}set:\n  lang: {lang}\n"));
                assert_eq!(out.diagnostics.exit_code(), 0, "{from} -> {to} {lang}: {:?}", out.diagnostics.errors);
                let drawio = out.drawio.unwrap_or_else(|| panic!("{from} -> {to} {lang}: no diagram"));
                assert!(drawio.contains(&format!("value=\"{text}\"")), "{from} -> {to} {lang}: no label {text}");
            }
        }
    }

    // Fails if a definition of the sign-in or log analysis shapes stops or leaves a hole.
    #[test]
    fn the_sign_in_and_log_analysis_shapes_are_drawn() {
        let defs = [
            "flows:\n  - user -> cloudfront -> apigateway(rest) -> lambda -> dynamodb\n  - user -> cognito\n  - cognito -- apigateway\nset:\n  lang: ja\n",
            "flows:\n  - user -> cognito\n  - user -> s3\n",
            "flows:\n  - cloudtrail -> s3\n  - user -> athena -> s3\n  - glue -- athena\nset:\n  lang: ja\n",
            "flows:\n  - cloudtrail -> securitylake\n  - securityhub -> securitylake\n  - securitylake -> s3\n  - securitylake -> sqs\n  - user -> athena -> securitylake\n",
        ];
        for src in defs {
            let (m, d) = run(src);
            assert!(d.errors.is_empty(), "{src}: {:?}", d.errors);
            assert!(m.is_some(), "{src}");
            assert_eq!(d.exit_code(), 0, "{src}");
        }
    }

    // Fails if a service added for log analysis starts a flow, or a pair left out on purpose is accepted.
    #[test]
    fn log_analysis_services_only_receive() {
        for src in ["flows:\n  - athena -> s3\n", "flows:\n  - securitylake -> sqs\n"] {
            let (m, d) = run(src);
            assert!(m.is_none(), "{src}");
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[0]")), "{src}");
        }
        let (m, d) = run("flows:\n  - glue -- athena\n");
        assert!(m.is_none());
        assert_eq!(d.errors[0].code, "UNSUPPORTED");
    }

    // Fails if one of the pairs with a KMS key stops, gets another label, or is drawn without its
    // label in English or Japanese, with or without a VPC. The key hangs on a part that a main flow
    // reaches.
    #[test]
    fn every_kms_pair_is_accepted_with_its_label() {
        let r = Rules::builtin();
        let pairs = [
            ("s3", "flows:\n  - user -> cloudfront -> s3\n  - kms -- s3\n"),
            ("dynamodb", "flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n  - kms -- dynamodb\n"),
            ("s3", "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - kms -- s3\n"),
            ("dynamodb", "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> dynamodb\n  - kms -- dynamodb\nchoose:\n  route.lambda.dynamodb: endpoint\n"),
        ];
        assert_eq!(r.link("kms", "s3", Rel::Assoc).map(|l| l.label.as_str()), Some("encryption_key"));
        assert_eq!(r.link("kms", "dynamodb", Rel::Assoc).map(|l| l.label.as_str()), Some("encryption_key"));
        let (en, ja) = ("Encryption key", "暗号化キー");
        assert_eq!((r.text("encryption_key", Lang::En).as_str(), r.text("encryption_key", Lang::Ja).as_str()), (en, ja));
        for (to, flows) in pairs {
            let (m, d) = run(flows);
            assert!(d.errors.is_empty(), "{flows}: {:?}", d.errors);
            let m = m.unwrap_or_else(|| panic!("{flows}: no model"));
            let e = m.edges.iter().find(|e| e.from == "kms" && e.to == to).unwrap_or_else(|| panic!("{flows}: no edge kms -- {to}"));
            assert_eq!((e.rel, e.label.as_str()), (Rel::Assoc, "encryption_key"), "{flows}");
            for (lang, text) in [("en", en), ("ja", ja)] {
                let out = crate::compile(&format!("{flows}set:\n  lang: {lang}\n"));
                assert_eq!(out.diagnostics.exit_code(), 0, "{flows} {lang}: {:?}", out.diagnostics.errors);
                let drawio = out.drawio.unwrap_or_else(|| panic!("{flows} {lang}: no diagram"));
                assert!(drawio.contains(&format!("value=\"{text}\"")), "{flows} {lang}: no label {text}");
            }
        }
    }

    // Fails if one key cannot serve both S3 and DynamoDB, or a named key stops.
    #[test]
    fn one_kms_key_serves_s3_and_dynamodb() {
        let defs = [
            "flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n  - user -> cloudfront -> s3\n  - kms -- dynamodb\n  - kms -- s3\nset:\n  lang: ja\n",
            "flows:\n  - user -> cloudfront -> s3\n  - kms.assets -- s3\n",
        ];
        for src in defs {
            let (m, d) = run(src);
            assert!(d.errors.is_empty(), "{src}: {:?}", d.errors);
            assert!(m.is_some(), "{src}");
            assert_eq!(d.exit_code(), 0, "{src}");
        }
    }

    // Fails if a KMS key is accepted on a part or in a direction left out on purpose. The list of
    // connections is asked directly as well, because a reversed or flowing line would also stop
    // later, for not being on a main flow, with the same code, place and text.
    #[test]
    fn kms_attaches_only_to_s3_and_dynamodb() {
        let r = Rules::builtin();
        let web = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n";
        let api = "flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n";
        let edge = "flows:\n  - user -> cloudfront -> s3\n";
        let defs = [
            (web, "kms -- aurora", "kms", "aurora", Rel::Assoc),
            (web, "kms -- ecs", "kms", "ecs", Rel::Assoc),
            (api, "kms -- lambda", "kms", "lambda", Rel::Assoc),
            (edge, "s3 -- kms", "s3", "kms", Rel::Assoc),
            (edge, "kms -> s3", "kms", "s3", Rel::Main),
            (edge, "kms ~> s3", "kms", "s3", Rel::Aux),
        ];
        for (base, line, from, to, rel) in defs {
            assert!(r.link(from, to, rel).is_none(), "links.toml accepts `{line}`");
            let src = format!("{base}  - {line}\n");
            let (m, d) = run(&src);
            assert!(m.is_none(), "{src}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{src}: {:?}", d.errors);
            assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"), "{src}");
        }
    }

    // Fails if one of the event pairs between EventBridge, SNS, SQS, Lambda and S3 stops, gets
    // another label, or is drawn without its label in English or Japanese. Each definition reaches
    // the pair from a starting point, and the pair's label is the only one with that text.
    #[test]
    fn every_event_pair_is_accepted_with_its_label() {
        let r = Rules::builtin();
        let pairs = [
            ("eventbridge", "lambda", "by_rule", "flows:\n  - eventbridge -> lambda -> dynamodb\n"),
            ("eventbridge", "sqs", "by_rule", "flows:\n  - eventbridge -> sqs -> lambda\n"),
            ("sns", "lambda", "notifications", "flows:\n  - user -> apigateway(http) -> lambda.api -> sns -> lambda\n"),
            ("sns", "sqs", "notifications", "flows:\n  - user -> apigateway(http) -> lambda.api -> sns -> sqs -> lambda\n"),
            ("sqs", "lambda", "messages", "flows:\n  - eventbridge -> sqs -> lambda\n"),
            ("lambda", "sqs", "send_messages", "flows:\n  - user -> apigateway(http) -> lambda -> sqs -> lambda.worker\n"),
            ("lambda", "sns", "publish", "flows:\n  - user -> apigateway(http) -> lambda -> sns\n"),
            ("s3", "lambda", "event_notifications", "flows:\n  - user -> s3 -> lambda -> dynamodb\n"),
            ("s3", "sqs", "event_notifications", "flows:\n  - user -> s3 -> sqs -> lambda\n"),
            ("s3", "sns", "event_notifications", "flows:\n  - user -> s3 -> sns -> lambda\n"),
        ];
        for (from, to, key, flows) in pairs {
            assert_eq!(r.link(from, to, Rel::Main).map(|l| l.label.as_str()), Some(key), "{from} -> {to}");
            let (m, d) = run(flows);
            assert!(d.errors.is_empty(), "{flows}: {:?}", d.errors);
            let m = m.unwrap_or_else(|| panic!("{flows}: no model"));
            let e = m.edges.iter().find(|e| e.from == from && e.to == to).unwrap_or_else(|| panic!("{flows}: no edge {from} -> {to}"));
            assert_eq!((e.rel, e.label.as_str()), (Rel::Main, key), "{flows}");
            for lang in [Lang::En, Lang::Ja] {
                let text = r.text(key, lang);
                let code = if lang == Lang::En { "en" } else { "ja" };
                let out = crate::compile(&format!("{flows}set:\n  lang: {code}\n"));
                assert_eq!(out.diagnostics.exit_code(), 0, "{flows} {code}: {:?}", out.diagnostics.errors);
                let drawio = out.drawio.unwrap_or_else(|| panic!("{flows} {code}: no diagram"));
                assert!(drawio.contains(&format!("value=\"{text}\"")), "{flows} {code}: no label {text}");
            }
        }
        let texts = |key: &str| (r.text(key, Lang::En), r.text(key, Lang::Ja));
        assert_eq!(texts("messages"), ("Messages".to_string(), "メッセージ".to_string()));
        assert_eq!(texts("send_messages"), ("Send messages".to_string(), "メッセージ送信".to_string()));
        assert_eq!(texts("event_notifications"), ("Event notifications".to_string(), "イベント通知".to_string()));
    }

    // Fails if a reversed or unlisted event pair is accepted. The list of connections is asked
    // directly as well, because a line whose start is not reached would also stop later with the
    // same code, place and text.
    #[test]
    fn event_pairs_left_out_stop() {
        let r = Rules::builtin();
        let base = "flows:\n  - user -> apigateway(http) -> lambda -> sqs\n  - lambda -> sns\n  - user -> s3 -> lambda.files\n  - eventbridge -> lambda.timer\n";
        let defs = [
            ("lambda -> eventbridge", "lambda", "eventbridge", Main),
            ("sqs -> sns", "sqs", "sns", Main),
            ("sqs -> s3", "sqs", "s3", Main),
            ("sns -> s3", "sns", "s3", Main),
            ("s3 -> eventbridge", "s3", "eventbridge", Main),
            ("sns -> eventbridge", "sns", "eventbridge", Main),
            ("sqs -> sqs.next", "sqs", "sqs", Main),
            ("sqs ~> lambda", "sqs", "lambda", Aux),
            ("sqs -- lambda", "sqs", "lambda", Rel::Assoc),
        ];
        for (line, from, to, rel) in defs {
            assert!(r.link(from, to, rel).is_none(), "links.toml accepts `{line}`");
            let src = format!("{base}  - {line}\n");
            let (m, d) = run(&src);
            assert!(m.is_none(), "{src}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[4]"), Some(line)), "{src}: {:?}", d.errors);
            assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"), "{src}");
        }
    }

    // Fails if a queue becomes a starting point: nothing sends to it, so the flow still stops.
    #[test]
    fn a_queue_alone_does_not_start_a_flow() {
        let (m, d) = run("flows:\n  - sqs -> lambda -> dynamodb\n");
        assert!(m.is_none());
        assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref()), ("UNSUPPORTED", Some("flows[0]")));
    }

    // Fails if a definition whose flow comes back to the same function stops with a panic, loops, or
    // is drawn with a broken diagram.
    #[test]
    fn a_flow_back_to_the_same_function_is_handled() {
        // A line back to a part already on the flow runs against the direction of the drawing, which
        // the layout refuses. The definition stops with LAYOUT_FAILED and no diagram, in either
        // direction, with or without a VPC.
        let defs = [
            "flows:\n  - user -> apigateway(http) -> lambda.a -> sqs -> lambda.a\n",
            "flows:\n  - user -> apigateway(http) -> lambda -> sqs\n  - sqs -> lambda\n",
            "flows:\n  - user -> s3 -> lambda -> sns -> sqs -> lambda\n",
            "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> sqs -> lambda\nchoose:\n  route.lambda.sqs: endpoint\n",
        ];
        for src in defs {
            let (m, d) = run(src);
            assert!(d.errors.is_empty() && m.is_some(), "{src}: {:?}", d.errors);
            for direction in ["down", "right"] {
                let out = crate::compile(&format!("{src}set:\n  direction: {direction}\n"));
                let codes: Vec<_> = out.diagnostics.errors.iter().map(|e| e.code.as_str()).collect();
                assert!(!codes.is_empty() && codes.iter().all(|c| *c == "LAYOUT_FAILED"), "{src} {direction}: {codes:?}");
                assert!(out.drawio.is_none(), "{src} {direction}");
                assert_eq!(out.diagnostics.exit_code(), 1, "{src} {direction}");
            }
        }
    }

    // Fails if a function in the VPC sends to a queue or a topic without the choice of a route, or
    // a queue that invokes a function in the VPC is given a route to choose.
    #[test]
    fn a_function_in_the_vpc_needs_a_route_to_send_but_not_to_be_invoked() {
        let (m, d) = run("flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> sqs -> lambda.worker -> dynamodb\n  - lambda -> sns\n");
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert_eq!(spot(&m, "lambda-1a"), (PRIVATE, Some(0), None));
        let mut holes: Vec<_> = d.holes.iter().map(|h| (h.hole.as_str(), h.choices.clone())).collect();
        holes.sort();
        assert_eq!(holes, [("route.lambda.sns", strings(&["endpoint", "nat"])), ("route.lambda.sqs", strings(&["endpoint", "nat"]))]);
        assert_eq!(d.exit_code(), 2);
        let (m, d) = passes("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - sqs -> lambda -> aurora\n  - eventbridge -> sqs\n");
        assert_eq!(spot(&m, "lambda-1a"), (PRIVATE, Some(0), None));
        assert!(d.holes.is_empty(), "{:?}", hole_ids(&d));
        assert_eq!(d.exit_code(), 0);
        assert!(m.edges.iter().any(|e| e.from == "sqs" && e.to == "lambda-1a" && e.label == "messages"));
    }

    // Fails if a line from a queue, a topic, an EventBridge rule or a bucket to a function in the
    // VPC leaves a route to choose, or adds an Internet Gateway or a NAT gateway: the services
    // invoke the function, it sends nothing out of the VPC.
    #[test]
    fn invoking_a_function_in_the_vpc_needs_no_route() {
        let cases = [
            ("sqs", "messages", "flows:\n  - eventbridge -> sqs -> lambda -> aurora\n"),
            ("sns", "notifications", "flows:\n  - eventbridge -> sns -> lambda -> aurora\n"),
            ("eventbridge", "by_rule", "flows:\n  - eventbridge -> lambda -> aurora\n"),
            ("s3", "event_notifications", "flows:\n  - user -> s3 -> lambda -> aurora\n"),
        ];
        for (from, label, src) in cases {
            let (m, d) = passes(src);
            assert_eq!(spot(&m, "lambda-1a"), (PRIVATE, Some(0), None), "{src}");
            assert_eq!(spot(&m, "lambda-1c"), (PRIVATE, Some(1), None), "{src}");
            assert!(d.holes.is_empty(), "{src}: {:?}", hole_ids(&d));
            assert_eq!(d.exit_code(), 0, "{src}");
            for to in ["lambda-1a", "lambda-1c"] {
                assert!(m.edges.iter().any(|e| e.from == from && e.to == to && e.label == label), "{src}: no {from} -> {to}");
            }
            let added: Vec<_> = m.nodes.iter().filter(|n| n.added).map(|n| n.id.as_str()).collect();
            assert!(added.iter().all(|id| !id.starts_with("igw") && !id.starts_with("nat")), "{src}: {added:?}");
            assert!(m.nodes.iter().all(|n| !matches!(n.area, Area::Subnet(Tier::Public) | Area::VpcEdge)), "{src}: {:?}", node_ids(&m));
        }
    }

    // Fails if a service in the VPC other than Lambda is accepted to send to a queue or a topic.
    #[test]
    fn only_lambda_in_the_vpc_sends_to_queues_and_topics() {
        let web = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n";
        let ec2 = "flows:\n  - user -> alb -> ec2 -> aurora\n";
        for (base, line, from, to) in [(web, "ecs -> sqs", "ecs", "sqs"), (web, "ecs -> sns", "ecs", "sns"), (ec2, "ec2 -> sqs", "ec2", "sqs"), (ec2, "ec2 -> sns", "ec2", "sns")] {
            assert!(Rules::builtin().link(from, to, Main).is_none(), "links.toml accepts `{line}`");
            let src = format!("{base}  - {line}\n");
            let (m, d) = run(&src);
            assert!(m.is_none(), "{src}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{src}: {:?}", d.errors);
        }
    }

    // Fails if one of the pairs with EC2 stops, gets another label, or is drawn without its label
    // in English or Japanese. EC2 starts flows of its own; the load balancer reaches it from the user.
    #[test]
    fn every_ec2_pair_is_accepted_with_its_label() {
        let r = Rules::builtin();
        let (m, a) = (Rel::Main, Rel::Aux);
        let pairs = [
            ("alb", "ec2", m, "http", "flows:\n  - user -> alb -> ec2\n", "alb", "ec2-1a"),
            ("ec2", "aurora", m, "read_write", "flows:\n  - ec2 -> aurora\n", "ec2-1a", "aurora-1a"),
            ("aurora", "ec2", m, "http", "flows:\n  - ec2 -> aurora\n  - aurora -> ec2\n", "aurora-1a", "ec2-1a"),
            ("ec2", "s3", m, "read_write", "flows:\n  - ec2 -> s3\nchoose:\n  route.ec2.s3: endpoint\n", "ec2-1c", "s3"),
            ("ec2", "dynamodb", m, "read_write", "flows:\n  - ec2 -> dynamodb\nchoose:\n  route.ec2.dynamodb: endpoint\n", "ec2-1c", "dynamodb"),
            ("ec2", "cloudwatch", m, "logs_metrics", "flows:\n  - ec2 -> cloudwatch\nchoose:\n  route.ec2.cloudwatch: endpoint\n", "ec2-1c", "cloudwatch"),
            ("ec2", "internet", m, "outbound", "flows:\n  - user -> alb -> ec2 -> internet\n", "ec2-1a", "nat-1a"),
            ("ec2", "s3", a, "logs_metrics", "flows:\n  - user -> alb -> ec2\n  - ec2 ~> s3\n", "ec2-1c", "s3"),
            ("ec2", "cloudwatch", a, "logs_metrics", "flows:\n  - user -> alb -> ec2\n  - ec2 ~> cloudwatch\n", "ec2-1c", "cloudwatch"),
            ("ec2", "ssm", m, "https", "flows:\n  - ec2 -> ssm\nchoose:\n  route.ec2.ssm: endpoint\n", "ec2-1c", "ssm"),
        ];
        assert_eq!(pairs.len(), 10);
        for (from, to, rel, key, flows, a_id, b_id) in pairs {
            assert_eq!(r.link(from, to, rel).map(|l| l.label.as_str()), Some(key), "{from} {rel:?} {to}");
            let (m, d) = run(flows);
            assert!(d.errors.is_empty(), "{from} -> {to}: {:?}", d.errors);
            let m = m.unwrap_or_else(|| panic!("{from} -> {to}: no model"));
            let edge = m.edges.iter().find(|e| e.from == a_id && e.to == b_id).unwrap_or_else(|| panic!("{from} -> {to}: no edge {a_id} -> {b_id}"));
            assert_eq!(edge.label, key, "{from} -> {to}");
            for (lang, text) in [("en", r.text(key, Lang::En)), ("ja", r.text(key, Lang::Ja))] {
                let src = match flows.find("choose:") {
                    Some(at) => format!("{}set:\n  lang: {lang}\n{}", &flows[..at], &flows[at..]),
                    None => format!("{flows}set:\n  lang: {lang}\n"),
                };
                let out = crate::compile(&src);
                assert_eq!(out.diagnostics.exit_code(), 0, "{from} -> {to} {lang}: {:?}", out.diagnostics.errors);
                let drawio = out.drawio.unwrap_or_else(|| panic!("{from} -> {to} {lang}: no diagram"));
                assert!(drawio.contains(&format!("value=\"{text}")), "{from} -> {to} {lang}: no label {text}");
            }
        }
    }

    // Fails if EC2 is not drawn once in the private subnet of each zone, or its parts and the VPC
    // lose their reasons: those of EC2 when it is the first VPC part, those of the load balancer
    // when that comes first.
    #[test]
    fn ec2_is_drawn_per_zone_in_private_subnets_with_its_reasons() {
        use crate::diag::DerivedKind;
        let (m, d) = run("flows:\n  - ec2 -> ssm\nchoose:\n  route.ec2.ssm: endpoint\n");
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        let m = m.unwrap();
        let copies: Vec<_> = m.nodes.iter().filter(|n| n.part == "ec2").map(|n| (n.id.as_str(), n.area)).collect();
        assert_eq!(copies, [("ec2-1a", Area::Subnet(Tier::Private)), ("ec2-1c", Area::Subnet(Tier::Private))]);
        assert_eq!(rules_of(&d, "vpc"), ["ec2-instance-in-subnet"]);
        assert_eq!(rules_of(&d, "az"), ["default-two-azs-for-instances"]);
        assert_eq!(rules_of(&d, "subnet"), ["private-subnet-per-az"]);
        assert_eq!(rules_of(&d, "ec2"), ["instances-in-private"]);
        let r = Rules::builtin();
        let rule = r.rule("ec2-instance-in-subnet");
        assert_eq!(rule.kind, DerivedKind::Constraint);
        assert_eq!(rule.source.as_deref(), Some("https://aws.amazon.com/vpc/faqs/"));
        assert_eq!(r.rule("instances-in-private").kind, DerivedKind::Default);
        assert_eq!(r.rule("default-two-azs-for-instances").kind, DerivedKind::Default);

        let (_, d) = run("flows:\n  - ec2 -> ssm\nset:\n  azs: 3\nchoose:\n  route.ec2.ssm: endpoint\n");
        assert!(rules_of(&d, "az").is_empty(), "{:?}", rules_of(&d, "az"));

        let (m, d) = run("flows:\n  - user -> alb -> ec2 -> aurora\n");
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(m.unwrap().nodes.iter().filter(|n| n.part == "ec2").count(), 2);
        assert_eq!(rules_of(&d, "vpc"), ["load-balancer-in-vpc"]);
        assert_eq!(rules_of(&d, "az"), ["alb-spans-two-azs", "default-two-azs"]);
        assert_eq!(rules_of(&d, "ec2"), ["instances-in-private"]);
    }

    // Fails if a main flow from a private EC2 to a regional service leaves no route hole, or the
    // shapes of the problems EC2 is added for stop or leave a hole once answered.
    #[test]
    fn the_ec2_shapes_are_drawn() {
        for (src, holes) in [
            ("flows:\n  - ec2 -> ssm\n", vec!["route.ec2.ssm"]),
            ("flows:\n  - ec2 -> s3\n  - ec2 -> ssm\n  - ec2 ~> cloudwatch\n", vec!["route.ec2.s3", "route.ec2.ssm"]),
            ("flows:\n  - user -> alb -> ec2\n  - ec2 -> internet\n", vec![]),
        ] {
            let (m, d) = run(src);
            assert!(d.errors.is_empty(), "{src}: {:?}", d.errors);
            assert!(m.is_some(), "{src}");
            assert_eq!(d.holes.iter().map(|h| h.hole.as_str()).collect::<Vec<_>>(), holes, "{src}");
        }
        for src in [
            "flows:\n  - user -> alb -> ec2 -> aurora\n  - ec2 -> internet\nset:\n  lang: ja\n",
            "flows:\n  - ec2 -> s3\nset:\n  lang: ja\nchoose:\n  route.ec2.s3: endpoint\n",
            "flows:\n  - ec2 -> ssm\n  - ec2 ~> cloudwatch\nchoose:\n  route.ec2.ssm: endpoint\n",
            "flows:\n  - user -> cloudfront -> alb -> ec2\n  - waf -- cloudfront\nset:\n  azs: 3\n",
        ] {
            let out = crate::compile(src);
            assert_eq!(out.diagnostics.exit_code(), 0, "{src}: {:?}", out.diagnostics.errors);
            assert!(out.drawio.is_some(), "{src}");
        }
    }

    // Fails if a pair with EC2 left out of the list is accepted.
    #[test]
    fn ec2_pairs_outside_the_list_stop() {
        for line in ["user -> ec2", "cloudfront -> ec2", "ssm -> ec2", "ec2 -> user", "ec2 -> ecs", "ecs -> ec2", "ec2 ~> ssm", "ec2 -- alb", "apigateway -> ec2", "lambda -> ec2"] {
            let (m, d) = run(&format!("flows:\n  - user -> alb -> ec2\n  - {line}\n"));
            assert!(m.is_none(), "{line}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{line}");
        }
    }

    #[test]
    fn an_alarm_reached_by_a_side_line_notifies() {
        let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs ~> cloudwatch\n  - cloudwatch -> sns\n");
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert!(m.is_some());
        assert_eq!(label(&m.unwrap(), "cloudwatch", "sns"), "alarms");
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

    /// The web stack in Japanese with these lines added.
    fn web_with(lines: &str) -> String {
        let web = include_str!("../../../../tests/cases/web-2az-ja/def.yaml");
        let src = web.replace("set:\n", &format!("{lines}set:\n"));
        assert_ne!(web, src);
        src
    }

    fn node_of<'m>(m: &'m Model, id: &str) -> (Area, Option<&'m str>) {
        let n = m.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}"));
        (n.area, n.region.as_deref())
    }

    // Fails if a load balancer's certificate is drawn in us-east-1 or outside the Region, or keeps
    // the reasons of a global part or of CloudFront's certificate.
    #[test]
    fn a_load_balancer_certificate_stays_in_the_region() {
        let (m, d) = run(&web_with("  - acm -- alb\n"));
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert_eq!(d.exit_code(), 0, "{:?}", d.errors);
        assert_eq!(node_of(&m, "acm"), (Area::Region, None));
        assert_eq!(rules_of(&d, "acm"), ["alb-certificate-in-load-balancer-region"]);
        assert_eq!(label(&m, "acm", "alb"), "certificate");
    }

    // Fails if a load balancer's web ACL is drawn in the AWS Cloud frame or keeps the reason of a
    // global part.
    #[test]
    fn a_load_balancer_web_acl_stays_in_the_region() {
        let (m, d) = run(&web_with("  - waf -- alb\n"));
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert_eq!(d.exit_code(), 0, "{:?}", d.errors);
        assert_eq!(node_of(&m, "waf"), (Area::Region, None));
        assert_eq!(rules_of(&d, "waf"), ["regional-web-acl-in-resource-region"]);
        assert_eq!(label(&m, "waf", "alb"), "web_acl");
    }

    // Fails if the override of one attached part leaks to another.
    #[test]
    fn four_attachments_keep_their_own_places() {
        let src = web_with("  - acm.cdn -- cloudfront\n  - acm.alb -- alb\n  - waf.cdn -- cloudfront\n  - waf.alb -- alb\n");
        let (m, d) = run(&src);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert_eq!(d.exit_code(), 0, "{:?}", d.errors);
        assert_eq!(node_of(&m, "acm.cdn"), (Area::Cloud, Some("us-east-1")));
        assert_eq!(node_of(&m, "acm.alb"), (Area::Region, None));
        assert_eq!(node_of(&m, "waf.cdn"), (Area::Cloud, None));
        assert_eq!(node_of(&m, "waf.alb"), (Area::Region, None));
        assert_eq!(rules_of(&d, "acm.cdn"), ["cloudfront-certificate-in-us-east-1"]);
        assert_eq!(rules_of(&d, "acm.alb"), ["alb-certificate-in-load-balancer-region"]);
        assert_eq!(rules_of(&d, "waf.cdn"), ["global-service-outside-region"]);
        assert_eq!(rules_of(&d, "waf.alb"), ["regional-web-acl-in-resource-region"]);
    }

    /// The error of a part attached to both CloudFront and a load balancer, at line `at`.
    fn stopped_on_both(src: &str, part: &str, at: &str, one: &str, other: &str) {
        let (m, d) = run(src);
        assert!(m.is_none(), "{src}");
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        assert_eq!(first(&d), ("UNSUPPORTED", Some(at), Some(part)), "{src}");
        let message = &d.errors[0].message;
        assert!(message.contains(&format!("`{part}` is attached to both `{one}` and `{other}`")), "{message}");
        assert!(message.contains(&format!("`{part}.cloudfront -- cloudfront`")) && message.contains(&format!("`{part}.alb -- alb`")), "{message}");
        // The definition written with the suggested names passes.
        let split = src.replacen(&format!("  - {part} -- {one}\n"), &format!("  - {part}.{one} -- {one}\n"), 1).replacen(&format!("  - {part} -- {other}\n"), &format!("  - {part}.{other} -- {other}\n"), 1);
        assert_ne!(split, src);
        let (m, d) = run(&split);
        assert!(m.is_some() && d.exit_code() == 0, "{split}: {:?}", d.errors);
    }

    // Fails if one certificate attached to CloudFront and to a load balancer is silently drawn as one.
    #[test]
    fn one_certificate_on_both_stops() {
        stopped_on_both(&web_with("  - acm -- cloudfront\n  - acm -- alb\n"), "acm", "flows[5]", "cloudfront", "alb");
    }

    // Fails if a web ACL attached to CloudFront and to a load balancer is accepted: CloudFront's
    // line has no Region of its own, so comparing the raw overrides misses it.
    #[test]
    fn one_web_acl_on_both_stops() {
        stopped_on_both(&web_with("  - waf -- cloudfront\n  - waf -- alb\n"), "waf", "flows[5]", "cloudfront", "alb");
    }

    // Fails if the clash is looked for only on CloudFront's line.
    #[test]
    fn the_stop_comes_at_the_second_line_whatever_the_order() {
        stopped_on_both(&web_with("  - acm -- alb\n  - acm -- cloudfront\n"), "acm", "flows[5]", "alb", "cloudfront");
        stopped_on_both(&web_with("  - waf -- alb\n  - waf -- cloudfront\n"), "waf", "flows[5]", "alb", "cloudfront");
    }

    // Fails if the two names the error suggests for parts attached to named parts are the same
    // name, or a definition written with them does not pass.
    #[test]
    fn the_names_suggested_for_a_split_differ_and_pass() {
        let base = "flows:\n  - user -> cloudfront.web -> alb.web -> ecs\n";
        for part in ["acm", "waf"] {
            let src = format!("{base}  - {part} -- cloudfront.web\n  - {part} -- alb.web\n");
            let (m, d) = run(&src);
            assert!(m.is_none(), "{src}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[2]"), Some(part)), "{src}");
            let message = &d.errors[0].message;
            let (one, other) = (format!("{part}.cloudfront-web"), format!("{part}.alb-web"));
            assert!(message.contains(&format!("`{one} -- cloudfront.web`")) && message.contains(&format!("`{other} -- alb.web`")), "{message}");
            let split = format!("{base}  - {one} -- cloudfront.web\n  - {other} -- alb.web\n");
            let (m, d) = run(&split);
            assert!(m.is_some() && d.exit_code() == 0, "{split}: {:?}", d.errors);
        }
    }

    // Fails if a load balancer may have two web ACLs.
    #[test]
    fn one_web_acl_per_load_balancer() {
        let (m, d) = run(&web_with("  - waf.a -- alb\n  - waf.b -- alb\n"));
        assert!(m.is_none());
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        assert_eq!((first(&d), d.errors[0].rule.as_deref()), (("TOO_MANY_ATTACHED", Some("flows[5]"), Some("waf.b")), Some("one-web-acl-per-resource")));
    }

    // Fails if one web ACL on two load balancers in the Region is stopped, or is copied per load balancer.
    #[test]
    fn one_web_acl_on_two_load_balancers() {
        let src = format!("{}  - waf -- alb.web\n  - waf -- alb.admin\n", crate::test_inputs::TWO_WEBS);
        let (m, d) = run(&src);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert_eq!(d.exit_code(), 0, "{:?}", d.errors);
        assert_eq!(m.nodes.iter().filter(|n| n.service == "waf").count(), 1);
        assert_eq!(node_of(&m, "waf"), (Area::Region, None));
        let lines: Vec<(&str, &str)> = m.edges.iter().filter(|e| e.from == "waf").map(|e| (e.to.as_str(), e.label.as_str())).collect();
        assert_eq!(lines, [("alb.web", "web_acl"), ("alb.admin", "web_acl")]);
        assert!(m.edges.iter().filter(|e| e.from == "waf").all(|e| e.rel == Rel::Assoc));
        assert_eq!(rules_of(&d, "waf"), ["regional-web-acl-in-resource-region"]);
    }

    // Fails if an attachment written the other way round is accepted.
    #[test]
    fn reversed_attachments_stay_unsupported() {
        for line in ["alb -- acm", "alb -- waf"] {
            let (m, d) = run(&web_with(&format!("  - {line}\n")));
            assert!(m.is_none(), "{line}");
            assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[4]"), Some(line)), "{line}");
            assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"));
        }
    }

    // Fails if a definition without a VPC reads only its services' scopes and not a connection's.
    #[test]
    fn a_scope_override_reaches_a_definition_without_a_vpc() {
        let links = include_str!("../../../../rules/aws/links.toml");
        let region = "from_region = \"us-east-1\"\n";
        assert_eq!(links.matches(region).count(), 1);
        let rules = Rules::load(
            include_str!("../../../../rules/aws/services.toml"),
            include_str!("../../../../rules/aws/rules.toml"),
            include_str!("../../../../rules/aws/labels.toml"),
            &links.replacen(region, &format!("from_scope = \"regional\"\n{region}"), 1),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let src = "flows:\n  - user -> cloudfront -> s3\n  - acm -- cloudfront\n";
        let (m, d) = infer(&crate::def::parse(src).unwrap(), &rules);
        let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
        assert!(m.azs.is_empty());
        assert_eq!(node_of(&m, "acm"), (Area::Region, Some("us-east-1")));
        // The built-in data leaves it in the AWS Cloud frame.
        assert_eq!(node_of(&run(src).0.unwrap(), "acm"), (Area::Cloud, Some("us-east-1")));
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

    /// The model of a definition with a VPC that passes the checks, built with these rules.
    fn vpc_with(src: &str, rules: &Rules) -> Model {
        let def = crate::def::parse(src).unwrap();
        let mut diag = Diagnostics::default();
        let checked = check(&def, rules, &mut diag).unwrap_or_else(|| panic!("{:?}", diag.errors));
        assert!(checked.vpc);
        build::vpc_model(&def, rules, &checked).unwrap_or_else(|e| panic!("{e:?}"))
    }
    fn vpc(src: &str) -> Model { vpc_with(src, &Rules::builtin()) }
    fn node_ids(m: &Model) -> Vec<&str> { m.nodes.iter().map(|n| n.id.as_str()).collect() }
    fn lines(m: &Model) -> Vec<(&str, &str, Rel, &str)> {
        m.edges.iter().map(|e| (e.from.as_str(), e.to.as_str(), e.rel, e.label.as_str())).collect()
    }
    /// Each node's id, the name after the dot, zone and area.
    fn places(m: &Model) -> Vec<(&str, Option<&str>, Option<usize>, Area)> {
        m.nodes.iter().map(|n| (n.id.as_str(), n.tag.as_deref(), n.az, n.area)).collect()
    }
    fn role<'m>(m: &'m Model, id: &str) -> Option<&'m str> {
        m.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("{id}")).role.as_deref()
    }

    // Fails if copies are gathered by service rather than by part, replication joins the Writer of
    // one part to the Reader of another, the entry is looked up by the written name, or a line's
    // relation, a copy's zone or area, or the name after the dot changes.
    #[test]
    fn named_vpc_parts_build_by_part() {
        twin_apps_built(&vpc(crate::test_inputs::TWIN_APPS));
    }

    /// The parts, copies and lines of TWIN_APPS.
    fn twin_apps_built(m: &Model) {
        assert_eq!(node_ids(&m), [
            "user", "cloudfront", "igw", "alb", "ecs.web@1a", "ecs.web@1c", "aurora.web@1a", "aurora.web@1c",
            "ecs.admin@1a", "ecs.admin@1c", "aurora.admin@1a", "aurora.admin@1c",
        ]);
        let (pv, pb) = (Area::Subnet(Tier::Private), Area::Subnet(Tier::Public));
        assert_eq!(places(&m), [
            ("user", None, None, Area::Outside),
            ("cloudfront", None, None, Area::Cloud),
            ("igw", None, None, Area::VpcEdge),
            ("alb", None, None, pb),
            ("ecs.web@1a", Some("web"), Some(0), pv),
            ("ecs.web@1c", Some("web"), Some(1), pv),
            ("aurora.web@1a", Some("web"), Some(0), pv),
            ("aurora.web@1c", Some("web"), Some(1), pv),
            ("ecs.admin@1a", Some("admin"), Some(0), pv),
            ("ecs.admin@1c", Some("admin"), Some(1), pv),
            ("aurora.admin@1a", Some("admin"), Some(0), pv),
            ("aurora.admin@1c", Some("admin"), Some(1), pv),
        ]);
        for id in ["aurora.web@1a", "aurora.admin@1a"] {
            assert_eq!(role(&m, id), Some("writer"), "{id}");
        }
        for id in ["aurora.web@1c", "aurora.admin@1c"] {
            assert_eq!(role(&m, id), Some("reader"), "{id}");
        }
        assert_eq!(lines(&m), [
            ("user", "cloudfront", Main, "https"),
            ("cloudfront", "igw", Main, "https"),
            ("igw", "alb", Main, "https"),
            ("alb", "ecs.web@1a", Main, "http"),
            ("alb", "ecs.web@1c", Main, "http"),
            ("ecs.web@1a", "aurora.web@1a", Main, "read_write"),
            ("ecs.web@1a", "aurora.web@1c", Main, "read"),
            ("ecs.web@1c", "aurora.web@1a", Main, "read_write"),
            ("ecs.web@1c", "aurora.web@1c", Main, "read"),
            ("alb", "ecs.admin@1a", Main, "http"),
            ("alb", "ecs.admin@1c", Main, "http"),
            ("ecs.admin@1a", "aurora.admin@1a", Main, "read_write"),
            ("ecs.admin@1a", "aurora.admin@1c", Main, "read"),
            ("ecs.admin@1c", "aurora.admin@1a", Main, "read_write"),
            ("ecs.admin@1c", "aurora.admin@1c", Main, "read"),
            ("aurora.web@1a", "aurora.web@1c", Aux, "replication"),
            ("aurora.admin@1a", "aurora.admin@1c", Aux, "replication"),
        ]);
        assert!(!m.edges.iter().any(|e| e.from == "aurora.web@1a" && e.to == "aurora.admin@1c"));
    }

    // Fails if the two load balancers become one, the entry gateway is made twice, one of the web
    // stacks is dropped, copies are gathered by service, lines cross from one part to another, or a
    // line's relation, a part's zone or area, or the name after the dot changes.
    #[test]
    fn two_load_balancers_build_by_part() {
        two_webs_built(&vpc(crate::test_inputs::TWO_WEBS));
    }

    /// The parts, copies and lines of TWO_WEBS.
    fn two_webs_built(m: &Model) {
        assert_eq!(node_ids(&m), [
            "user", "cloudfront", "igw", "alb.web", "alb.admin", "ecs.web@1a", "ecs.web@1c", "aurora.web@1a",
            "aurora.web@1c", "ecs.admin@1a", "ecs.admin@1c", "aurora.admin@1a", "aurora.admin@1c",
        ]);
        let (pv, pb) = (Area::Subnet(Tier::Private), Area::Subnet(Tier::Public));
        assert_eq!(places(&m), [
            ("user", None, None, Area::Outside),
            ("cloudfront", None, None, Area::Cloud),
            ("igw", None, None, Area::VpcEdge),
            ("alb.web", Some("web"), None, pb),
            ("alb.admin", Some("admin"), None, pb),
            ("ecs.web@1a", Some("web"), Some(0), pv),
            ("ecs.web@1c", Some("web"), Some(1), pv),
            ("aurora.web@1a", Some("web"), Some(0), pv),
            ("aurora.web@1c", Some("web"), Some(1), pv),
            ("ecs.admin@1a", Some("admin"), Some(0), pv),
            ("ecs.admin@1c", Some("admin"), Some(1), pv),
            ("aurora.admin@1a", Some("admin"), Some(0), pv),
            ("aurora.admin@1c", Some("admin"), Some(1), pv),
        ]);
        for id in ["alb.web", "alb.admin"] {
            let n = m.nodes.iter().find(|n| n.id == id).unwrap();
            assert_eq!((n.area, n.az), (Area::Subnet(Tier::Public), None), "{id}");
        }
        assert_eq!(m.nodes.iter().filter(|n| n.area == Area::VpcEdge).count(), 1);
        assert_eq!(lines(&m), [
            ("user", "cloudfront", Main, "https"),
            ("cloudfront", "igw", Main, "https"),
            ("igw", "alb.web", Main, "https"),
            ("alb.web", "ecs.web@1a", Main, "http"),
            ("alb.web", "ecs.web@1c", Main, "http"),
            ("ecs.web@1a", "aurora.web@1a", Main, "read_write"),
            ("ecs.web@1a", "aurora.web@1c", Main, "read"),
            ("ecs.web@1c", "aurora.web@1a", Main, "read_write"),
            ("ecs.web@1c", "aurora.web@1c", Main, "read"),
            ("user", "igw", Main, "https"),
            ("igw", "alb.admin", Main, "https"),
            ("alb.admin", "ecs.admin@1a", Main, "http"),
            ("alb.admin", "ecs.admin@1c", Main, "http"),
            ("ecs.admin@1a", "aurora.admin@1a", Main, "read_write"),
            ("ecs.admin@1a", "aurora.admin@1c", Main, "read"),
            ("ecs.admin@1c", "aurora.admin@1a", Main, "read_write"),
            ("ecs.admin@1c", "aurora.admin@1c", Main, "read"),
            ("aurora.web@1a", "aurora.web@1c", Aux, "replication"),
            ("aurora.admin@1a", "aurora.admin@1c", Aux, "replication"),
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
        // Their reasons are found by the same roles.
        let (m, d) = infer(&crate::def::parse(src).unwrap(), &rules);
        assert!(m.is_some(), "{:?}", d.errors);
        assert_eq!(rules_of(&d, "door"), ["internet-facing-alb-in-vpc-with-igw"]);
        assert_eq!(rules_of(&d, "exit-1a"), ["private-egress-through-nat", "nat-device-is-nat-gateway"]);
        assert_eq!(rules_of(&d, "exit-1c"), ["private-egress-through-nat", "nat-device-is-nat-gateway"]);
        assert_eq!(rules_of(&d, "exit"), ["nat-per-az"]);
        assert!(["igw", "nat-1a", "nat-1c", "nat"].iter().all(|t| rules_of(&d, t).is_empty()));
    }

    // Fails if a definition with only a public tier loses a part or a line.
    #[test]
    fn a_public_tier_alone_builds_its_gateway() {
        load_balancer_alone_built(&vpc(crate::test_inputs::LOAD_BALANCER_ALONE));
    }

    /// The parts and lines of LOAD_BALANCER_ALONE.
    fn load_balancer_alone_built(m: &Model) {
        assert_eq!(node_ids(m), ["user", "igw", "alb"]);
        assert_eq!(lines(m), [("user", "igw", Main, "https"), ("igw", "alb", Main, "https")]);
    }

    /// Each reason's target and rule, in order.
    fn reasons(d: &Diagnostics) -> Vec<(&str, &str)> {
        d.derived.iter().map(|x| (x.target.as_str(), x.rule.as_str())).collect()
    }

    // Fails if the reasons of the VPC and its zones are not taken from the first part whose service
    // has `vpc_reasons` in the rules data.
    #[test]
    fn vpc_reasons_come_from_the_first_part_that_has_them() {
        let src = crate::test_inputs::WEB_CASES.iter().find(|(n, _)| *n == "web-2az-ja").unwrap().1;
        let (_, d) = run(src);
        assert_eq!(rules_of(&d, "vpc"), ["load-balancer-in-vpc"]);
        assert_eq!(rules_of(&d, "az"), ["alb-spans-two-azs", "default-two-azs"]);
        // Other ids in the data: ECS, written after the load balancer, has them too.
        let services = include_str!("../../../../rules/aws/services.toml");
        let ecs_too = services.replace(
            "key = \"ecs\"\n",
            "key = \"ecs\"\nvpc_reasons = { vpc = \"compute-in-private\", az = [\"database-in-private\"], default_azs = \"nat-per-az\" }\n",
        );
        let alb_line = "vpc_reasons = { vpc = \"load-balancer-in-vpc\", az = [\"alb-spans-two-azs\"], default_azs = \"default-two-azs\" }\n";
        let ecs_only = ecs_too.replacen(alb_line, "", 1);
        assert!(ecs_too != services && ecs_only != ecs_too);
        let load = |services: &str| {
            Rules::load(
                services,
                include_str!("../../../../rules/aws/rules.toml"),
                include_str!("../../../../rules/aws/labels.toml"),
                include_str!("../../../../rules/aws/links.toml"),
            )
            .unwrap()
        };
        let def = crate::def::parse(src).unwrap();
        let (_, d) = infer(&def, &load(&ecs_too));
        assert_eq!(rules_of(&d, "vpc"), ["load-balancer-in-vpc"]);
        assert_eq!(rules_of(&d, "az"), ["alb-spans-two-azs", "default-two-azs"]);
        let (_, d) = infer(&def, &load(&ecs_only));
        assert_eq!(rules_of(&d, "vpc"), ["compute-in-private"]);
        assert_eq!(rules_of(&d, "az"), ["database-in-private", "nat-per-az"]);
        // Without any part that has them, the definition stops as a rule the data lacks does.
        let aurora_line = "vpc_reasons = { vpc = \"aurora-cluster-in-vpc\", az = [\"aurora-subnet-group-two-azs\"], default_azs = \"default-two-azs-for-database\" }\n";
        assert_eq!(ecs_only.matches(aurora_line).count(), 1);
        let none = ecs_only
            .replacen(
                "vpc_reasons = { vpc = \"compute-in-private\", az = [\"database-in-private\"], default_azs = \"nat-per-az\" }\n",
                "",
                1,
            )
            .replacen(aurora_line, "", 1);
        let (m, d) = infer(&def, &load(&none));
        assert!(m.is_none());
        assert!(d.derived.is_empty());
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        let e = &d.errors[0];
        assert_eq!((e.code.as_str(), e.at.as_deref(), e.got.as_deref()), ("RULE_MISSING", Some("vpc_reasons"), None));
        assert_eq!(e.message, "no service of a part in the definition has `vpc_reasons` in services.toml");
        assert_eq!(d.exit_code(), 1);
    }

    // Fails if the note that auxiliary flows are not routed is given for a line from outside the
    // VPC, or not given for one from inside it.
    #[test]
    fn the_aux_note_is_for_lines_from_inside_the_vpc() {
        let outside = "flows:\n  - user -> cloudfront -> alb -> ecs\n  - cloudfront ~> cloudwatch\n";
        let (m, d) = run(outside);
        assert!(m.is_some(), "{:?}", d.errors);
        assert!(d.notes.is_empty(), "{:?}", d.notes);
        let (m, d) = run(&format!("{outside}  - ecs ~> cloudwatch\n"));
        assert!(m.is_some(), "{:?}", d.errors);
        assert_eq!(d.notes, ["Auxiliary flows (~>) are drawn as lines only; their network route is not handled."]);
    }

    /// The reasons, holes and notes of LOAD_BALANCER_ALONE.
    fn load_balancer_alone_explained(d: &Diagnostics) {
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(d.exit_code(), 0);
        assert_eq!(rules_of(d, "subnet"), ["public-subnet-per-az"]);
        assert_eq!(rules_of(d, "igw"), ["internet-facing-alb-in-vpc-with-igw"]);
        assert!(d.holes.is_empty() && d.notes.is_empty());
    }

    // Fails if a definition with only a public tier is stopped, or its subnets or entry gateway get
    // the wrong reason.
    #[test]
    fn a_public_tier_alone_gets_its_subnet_reason() {
        let (m, d) = run(crate::test_inputs::LOAD_BALANCER_ALONE);
        load_balancer_alone_built(&m.unwrap());
        load_balancer_alone_explained(&d);
    }

    /// The reasons, holes and notes of TWO_WEBS.
    fn two_webs_explained(d: &Diagnostics) {
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(d.exit_code(), 0);
        assert_eq!(reasons(d), [
            ("aws-cloud", "resources-inside-aws-cloud-frame"),
            ("region", "resources-inside-region-frame"),
            ("region", "default-region"),
            ("vpc", "load-balancer-in-vpc"),
            ("az", "alb-spans-two-azs"),
            ("az", "default-two-azs"),
            ("subnet", "public-and-private-subnet-per-az"),
            ("cloudfront", "global-service-outside-region"),
            ("ecs.web", "compute-in-private"),
            ("aurora.web", "database-in-private"),
            ("aurora.web", "aurora-writer-reader"),
            ("ecs.admin", "compute-in-private"),
            ("aurora.admin", "database-in-private"),
            ("aurora.admin", "aurora-writer-reader"),
            ("igw", "internet-facing-alb-in-vpc-with-igw"),
        ]);
        assert!(["alb.web", "alb.admin", "user"].iter().all(|t| rules_of(d, t).is_empty()));
        assert!(d.holes.is_empty() && d.notes.is_empty());
    }

    // Fails if the reasons of one of two named load balancers' parts are dropped, reasons are given
    // per copy, or the entry gateway gets the wrong reason.
    #[test]
    fn two_load_balancers_get_reasons_per_part() {
        let (m, d) = run(crate::test_inputs::TWO_WEBS);
        two_webs_built(&m.unwrap());
        two_webs_explained(&d);
    }

    /// The reasons of the named parts of TWIN_APPS.
    fn twin_apps_explained(d: &Diagnostics) {
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(d.exit_code(), 0);
        for part in ["ecs.web", "ecs.admin"] {
            assert_eq!(rules_of(d, part), ["compute-in-private"], "{part}");
        }
        for part in ["aurora.web", "aurora.admin"] {
            assert_eq!(rules_of(d, part), ["database-in-private", "aurora-writer-reader"], "{part}");
        }
        assert!(d.derived.iter().all(|x| !x.target.contains('@')), "{:?}", reasons(d));
        assert!(d.holes.is_empty() && d.notes.is_empty());
    }

    // Fails if reasons are given per copy, or a named part's reasons are dropped.
    #[test]
    fn named_parts_under_one_balancer_get_reasons_per_part() {
        let (m, d) = run(crate::test_inputs::TWIN_APPS);
        twin_apps_built(&m.unwrap());
        twin_apps_explained(&d);
    }

    /// The whole answer for a named bucket behind CloudFront next to a web stack.
    fn named_bucket_inferred(m: &Model, d: &Diagnostics) {
        assert_eq!(node_ids(m), ["user", "cloudfront", "igw", "alb", "ecs-1a", "ecs-1c", "s3.web"]);
        assert_eq!(lines(m), [
            ("user", "cloudfront", Main, "https"),
            ("cloudfront", "igw", Main, "https"),
            ("igw", "alb", Main, "https"),
            ("alb", "ecs-1a", Main, "http"),
            ("alb", "ecs-1c", Main, "http"),
            ("cloudfront", "s3.web", Main, "static_files"),
        ]);
        let web = m.nodes.iter().find(|n| n.id == "s3.web").unwrap();
        assert_eq!((web.service.as_str(), web.part.as_str(), web.tag.as_deref(), web.area), ("s3", "s3.web", Some("web"), Area::Region));
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(d.exit_code(), 0);
        assert_eq!(reasons(d), [
            ("aws-cloud", "resources-inside-aws-cloud-frame"),
            ("region", "resources-inside-region-frame"),
            ("region", "default-region"),
            ("vpc", "load-balancer-in-vpc"),
            ("az", "alb-spans-two-azs"),
            ("az", "default-two-azs"),
            ("subnet", "public-and-private-subnet-per-az"),
            ("cloudfront", "global-service-outside-region"),
            ("ecs", "compute-in-private"),
            ("s3.web", "regional-service-outside-vpc"),
            ("igw", "internet-facing-alb-in-vpc-with-igw"),
        ]);
        assert!(d.holes.is_empty() && d.notes.is_empty());
    }

    // Fails if a reason's kind, rule, text in either language or source is not the one the rules
    // data writes, or the reason given in a diagram is not made this way.
    #[test]
    fn a_reason_carries_its_rule_text_and_source() {
        use crate::diag::{Derived, DerivedKind};
        let rules = Rules::builtin();
        let igw = Derived {
            target: "igw".into(),
            added: true,
            kind: DerivedKind::Constraint,
            rule: "internet-facing-alb-in-vpc-with-igw".into(),
            reason: "With an internet-facing Application Load Balancer, only VPCs that have an Internet Gateway can be selected, so the VPC gets an Internet Gateway.".into(),
            source: Some("https://docs.aws.amazon.com/elasticloadbalancing/latest/application/create-application-load-balancer.html".into()),
        };
        assert_eq!(derived(&rules, Lang::En, "igw", "internet-facing-alb-in-vpc-with-igw", true), igw);
        let region = Derived {
            target: "region".into(),
            added: true,
            kind: DerivedKind::Default,
            rule: "default-region".into(),
            reason: "リージョンの指定が無いので、ap-northeast-1（東京）にする。".into(),
            source: None,
        };
        assert_eq!(derived(&rules, Lang::Ja, "region", "default-region", true), region);
        let (_, d) = run("flows:\n  - user -> alb -> ecs\n");
        assert!(d.derived.contains(&igw), "{:?}", d.derived);
        let (_, d) = run("flows:\n  - user -> alb -> ecs\nset:\n  lang: ja\n");
        assert!(d.derived.contains(&region), "{:?}", d.derived);
    }

    fn bad(code: &str, at: &str, got: &str, allowed: &[&str], message: &str) -> Problem {
        Problem { code: code.into(), at: Some(at.into()), got: Some(got.into()), allowed: strings(allowed), rule: None, message: message.into() }
    }

    // Fails if a setting's error changes its code, place, value, allowed values, text or order,
    // with a VPC or without one.
    #[test]
    fn every_bad_setting_is_reported_in_full() {
        let src = "flows:\n  - user -> alb -> ecs\nset:\n  azs: 4\n  colour: red\n  direction: up\n  lang: fr\n  nat: none\n  readers: 5\n  region: us-east-1\n";
        let (m, d) = run(src);
        assert!(m.is_none());
        assert_eq!(d.errors, [
            bad("BAD_SETTING", "set.azs", "4", &["2", "3"], "`4` is not a valid value for `azs`"),
            bad("BAD_SETTING", "set.region", "us-east-1", &["ap-northeast-1"], "`us-east-1` is not a valid value for `region`"),
            bad("BAD_SETTING", "set.lang", "fr", &["en", "ja"], "`fr` is not a valid value for `lang`"),
            bad("BAD_SETTING", "set.direction", "up", &["down", "right"], "`up` is not a valid value for `direction`"),
            bad("BAD_SETTING", "set.nat", "none", &["per-az", "single"], "`none` is not a valid value for `nat`"),
            bad("BAD_SETTING", "set.readers", "5", &["1"], "`5` is not a valid value for `readers`"),
            bad("UNKNOWN_SETTING", "set.colour", "colour", &["azs", "direction", "lang", "nat", "readers", "region"], "unknown setting `colour`"),
        ]);
        let (m, d) = run(&format!("{B}set:\n  azs: 3\n  nat: single\n"));
        assert!(m.is_none());
        let flow = ["direction", "lang", "region"];
        assert_eq!(d.errors, [
            bad("UNKNOWN_SETTING", "set.azs", "azs", &flow, "unknown setting `azs`"),
            bad("UNKNOWN_SETTING", "set.nat", "nat", &flow, "unknown setting `nat`"),
        ]);
    }

    // Fails if an unknown service or kind is reported with another place, value, list or text.
    #[test]
    fn unknown_names_are_reported_in_full() {
        let (_, d) = run("flows:\n  - user -> cloudfrnt -> alb -> ecs(lambda)\n");
        let services = [
            "acm", "alb", "apigateway", "athena", "aurora", "cloudfront", "cloudtrail", "cloudwatch", "cognito", "config", "dynamodb",
            "ec2", "ecs", "eventbridge", "glue", "guardduty", "inspector", "internet", "kms", "lambda", "s3", "securityhub", "securitylake", "sns",
            "sqs", "ssm", "stepfunctions", "user", "waf",
        ];
        assert_eq!(d.errors, [
            bad("UNKNOWN_SERVICE", "flows[0][1]", "cloudfrnt", &services, "unknown service `cloudfrnt`"),
            bad("UNKNOWN_OPTION", "flows[0][3]", "lambda", &["fargate"], "`ecs` has no kind `lambda`"),
        ]);
    }

    // Fails if a named part outside the VPC stops a definition with a VPC, or loses its reason or its line.
    #[test]
    fn a_named_bucket_next_to_a_web_stack_infers() {
        let (m, d) = run("flows:\n  - user -> cloudfront -> alb -> ecs\n  - cloudfront -> s3.web\n");
        named_bucket_inferred(&m.unwrap(), &d);
    }

    /// A web stack written with Aurora before the load balancer: the VPC's reasons come from Aurora.
    const AURORA_BEFORE_ALB: &str = "flows:\n  - ecs -> aurora\n  - user -> alb -> ecs\n";
    /// The same lines the other way round: the load balancer comes first.
    const ALB_BEFORE_AURORA: &str = "flows:\n  - user -> alb -> ecs\n  - ecs -> aurora\n";

    /// The built-in rules data with one piece of text taken out of services.toml or links.toml.
    fn rules_cut(services_cut: &str, links_cut: &str) -> Rules {
        let services = include_str!("../../../../rules/aws/services.toml");
        let links = include_str!("../../../../rules/aws/links.toml");
        for (text, cut) in [(services, services_cut), (links, links_cut)] {
            assert!(cut.is_empty() || text.matches(cut).count() == 1, "{cut}");
        }
        Rules::load(
            &services.replacen(services_cut, "", 1),
            include_str!("../../../../rules/aws/rules.toml"),
            include_str!("../../../../rules/aws/labels.toml"),
            &links.replacen(links_cut, "", 1),
        )
        .unwrap_or_else(|e| panic!("{e:?}"))
    }

    // Fails if a line or a role the inference needs and the rules data lacks makes it panic instead
    // of returning an error, or the lack changes the answer for a definition that does not need it.
    #[test]
    fn a_line_or_role_the_data_lacks_is_an_error_not_a_panic() {
        let line = |from: &str, to: &str, rel: &str, label: &str| {
            format!("[[derived_link]]\nfrom = \"{from}\"\nto = \"{to}\"\nrel = \"{rel}\"\nlabel = \"{label}\"\n")
        };
        const WEB: &str = "flows:\n  - user -> alb -> ecs\n";
        const OUT: &str = "flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n";
        const DB: &str = "flows:\n  - user -> alb -> ecs -> aurora\n";
        const API: &str = "flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n  - cognito -- apigateway\n";
        // What is cut, the code and `at` of the error, a definition that needs it, and ones that do not.
        let cases: [(Rules, &str, &str, &str, &[&str]); 5] = [
            (rules_cut("", &line("private", "egress_nat", "main", "outbound")), "DERIVED_LINK_MISSING", "derived_link private egress_nat", OUT, &[WEB, DB]),
            (rules_cut("", &line("egress_nat", "entry_gateway", "main", "outbound")), "DERIVED_LINK_MISSING", "derived_link egress_nat entry_gateway", OUT, &[WEB, DB]),
            (rules_cut("", &line("writer", "reader", "aux", "replication")), "DERIVED_LINK_MISSING", "derived_link writer reader", DB, &[WEB, OUT]),
            (rules_cut("derived_role = \"entry_gateway\"\n", ""), "DERIVED_ROLE_MISSING", "entry_gateway", WEB, &[]),
            (rules_cut("derived_role = \"egress_nat\"\n", ""), "DERIVED_ROLE_MISSING", "egress_nat", OUT, &[WEB, DB]),
        ];
        let built = |src: &str, rules: &Rules| {
            let def = crate::def::parse(src).unwrap();
            let mut diag = Diagnostics::default();
            let checked = check(&def, rules, &mut diag).unwrap_or_else(|| panic!("{:?}", diag.errors));
            build::vpc_model(&def, rules, &checked)
        };
        for (rules, code, at, needs, others) in &cases {
            let e = built(needs, rules).expect_err(code);
            let got: Vec<_> = e.iter().map(|p| (p.code.as_str(), p.at.as_deref())).collect();
            assert_eq!(got, [(*code, Some(*at))], "{at}");
            // The inference stops with the same errors and nothing else.
            let (m, d) = infer(&crate::def::parse(needs).unwrap(), rules);
            assert!(m.is_none(), "{at}");
            assert_eq!(d.errors, e, "{at}");
            assert!(d.derived.is_empty() && d.holes.is_empty() && d.notes.is_empty(), "{at}");
            assert_eq!(d.exit_code(), 1, "{at}");
            for src in *others {
                assert_eq!(built(src, rules).unwrap(), vpc(src), "{at}: {src}");
                let (m, d) = infer(&crate::def::parse(src).unwrap(), rules);
                assert_eq!((m, d.to_json()), (Some(vpc(src)), run(src).1.to_json()), "{at}: {src}");
            }
            // Without a VPC no gateway or added line is needed.
            let (m, d) = infer(&crate::def::parse(API).unwrap(), rules);
            let (want_m, want_d) = run(API);
            assert_eq!((m, d.to_json()), (want_m, want_d.to_json()), "{at}");
        }
    }

    // Fails if the label a connection gives per kind of its target is dropped in a definition with a VPC.
    #[test]
    fn a_kind_gives_its_label_with_a_vpc_too() {
        let base = "flows:\n  - user -> alb -> ecs\n  - user -> apigateway(http) -> lambda\n  - cognito -- apigateway\n";
        for (src, want) in [(base.to_string(), "jwt_authorizer"), (base.replace("(http)", "(rest)"), "cognito_authorizer")] {
            let m = vpc(&src);
            let e = m.edges.iter().find(|e| e.from == "cognito" && e.to == "apigateway").unwrap();
            assert_eq!((e.rel, e.label.as_str()), (Rel::Assoc, want), "{src}");
        }
    }

    // Fails if the two lines through the entry gateway do not take the label a connection gives
    // for the kind of the part it goes to, or lose the connection's label without a kind.
    #[test]
    fn lines_through_the_entry_gateway_take_the_label_for_the_kind() {
        let services = include_str!("../../../../rules/aws/services.toml");
        let links = include_str!("../../../../rules/aws/links.toml");
        let alb = "default_azs = \"default-two-azs\" }\n";
        let user_to_alb = "from = \"user\"\nto = \"alb\"\nrel = \"main\"\nlabel = \"https\"\n";
        assert_eq!((services.matches(alb).count(), links.matches(user_to_alb).count()), (1, 1));
        let rules = Rules::load(
            &services.replace(alb, &format!("{alb}\n[service.options.variant]\nshape = \"elastic_load_balancing\"\nlabel = \"Application Load Balancer\"\n")),
            include_str!("../../../../rules/aws/rules.toml"),
            include_str!("../../../../rules/aws/labels.toml"),
            &links.replace(user_to_alb, &format!("{user_to_alb}to_kind.variant = {{ label = \"http\", rule = \"load-balancer-in-vpc\" }}\n")),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let entry = |src: &str| {
            let (m, d) = infer(&crate::def::parse(src).unwrap(), &rules);
            let m = m.unwrap_or_else(|| panic!("{:?}", d.errors));
            lines(&m).into_iter().filter(|(f, t, _, _)| *t == "igw" || *f == "igw").map(|(f, t, r, l)| (f.to_string(), t.to_string(), r, l.to_string())).collect::<Vec<_>>()
        };
        let want = |label: &str| vec![("user".to_string(), "igw".to_string(), Main, label.to_string()), ("igw".into(), "alb".into(), Main, label.to_string())];
        assert_eq!(entry("flows:\n  - user -> alb(variant) -> ecs\n"), want("http"));
        assert_eq!(entry("flows:\n  - user -> alb -> ecs\n"), want("https"));
    }

    // Fails if a route hole is named by service instead of by part, or an answer for one part's
    // route reaches the lines of another part.
    #[test]
    fn named_parts_have_their_own_routes() {
        const SRC: &str = "flows:\n  - user -> alb -> ecs.web\n  - alb -> ecs.admin\n  - ecs.web -> s3.web\n  - ecs.admin -> s3.admin\n";
        const WEB: &str = "route.[ecs.web].[s3.web]";
        const ADMIN: &str = "route.[ecs.admin].[s3.admin]";
        let front = [
            ("user", "igw", Main, "https"),
            ("igw", "alb", Main, "https"),
            ("alb", "ecs.web@1a", Main, "http"),
            ("alb", "ecs.web@1c", Main, "http"),
            ("alb", "ecs.admin@1a", Main, "http"),
            ("alb", "ecs.admin@1c", Main, "http"),
        ];
        let to_s3 = [
            ("ecs.web@1a", "s3.web", Main, "read_write"),
            ("ecs.web@1c", "s3.web", Main, "read_write"),
            ("ecs.admin@1a", "s3.admin", Main, "read_write"),
            ("ecs.admin@1c", "s3.admin", Main, "read_write"),
        ];
        let parts = ["user", "igw", "alb", "ecs.web@1a", "ecs.web@1c", "ecs.admin@1a", "ecs.admin@1c", "s3.web", "s3.admin"];
        // The hole and the endpoint mark of the lines to S3, in the order above.
        let marks = |m: &Model| m.edges.iter().filter(|e| e.to.starts_with("s3.")).map(|e| (e.hole.clone(), e.endpoint)).collect::<Vec<_>>();
        let (web, admin) = (Some(WEB.to_string()), Some(ADMIN.to_string()));

        let m = vpc(SRC);
        assert_eq!(node_ids(&m), parts);
        assert_eq!(lines(&m), [&front[..], &to_s3[..]].concat());
        assert_eq!(marks(&m), [(web.clone(), false), (web.clone(), false), (admin.clone(), false), (admin.clone(), false)]);

        let m = vpc(&format!("{SRC}choose:\n  {WEB}: endpoint\n"));
        assert_eq!(node_ids(&m), parts);
        assert_eq!(lines(&m), [&front[..], &to_s3[..]].concat());
        assert_eq!(marks(&m), [(None, true), (None, true), (admin.clone(), false), (admin.clone(), false)]);

        let m = vpc(&format!("{SRC}choose:\n  {ADMIN}: nat\n"));
        assert_eq!(node_ids(&m), ["user", "igw", "alb", "nat-1a", "nat-1c", "ecs.web@1a", "ecs.web@1c", "ecs.admin@1a", "ecs.admin@1c", "s3.web", "s3.admin"]);
        let out = [
            ("ecs.admin@1a", "nat-1a", Main, "outbound"),
            ("ecs.admin@1c", "nat-1c", Main, "outbound"),
            ("nat-1a", "igw", Main, "outbound"),
            ("nat-1c", "igw", Main, "outbound"),
        ];
        assert_eq!(lines(&m), [&front[..], &to_s3[..], &out[..]].concat());
        assert_eq!(marks(&m), [(web.clone(), false), (web.clone(), false), (None, false), (None, false)]);

        // The holes the diagnostics give: each at the line it is on, in the order written.
        let hole = |id: &str, at: &str, from: &str, to: &str| crate::diag::Hole {
            hole: id.into(),
            at: at.into(),
            need: format!("Route from {from} to {to}"),
            choices: strings(&["endpoint", "nat"]),
        };
        let web_hole = hole(WEB, "flows[2]", "ecs.web", "s3.web");
        let admin_hole = hole(ADMIN, "flows[3]", "ecs.admin", "s3.admin");
        let endpoint_note = Rules::builtin().text("note.endpoint_not_drawn", Lang::En);
        for (src, holes, notes) in [
            (SRC.to_string(), vec![web_hole.clone(), admin_hole.clone()], vec![]),
            (format!("{SRC}choose:\n  {WEB}: endpoint\n"), vec![admin_hole.clone()], vec![endpoint_note]),
            (format!("{SRC}choose:\n  {ADMIN}: nat\n"), vec![web_hole.clone()], vec![]),
        ] {
            let (m, d) = run(&src);
            assert_eq!(m, Some(vpc(&src)), "{src}");
            assert!(d.errors.is_empty(), "{src}: {:?}", d.errors);
            assert_eq!(d.holes, holes, "{src}");
            assert_eq!(d.notes, notes, "{src}");
            assert_eq!(d.exit_code(), 2, "{src}");
        }
    }

    /// A node of a model by its id.
    fn node<'m>(m: &'m Model, id: &str) -> &'m crate::model::Node {
        m.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}: {:?}", node_ids(m)))
    }
    /// A node's area, zone and role.
    fn spot<'m>(m: &'m Model, id: &str) -> (Area, Option<usize>, Option<&'m str>) {
        let n = node(m, id);
        (n.area, n.az, n.role.as_deref())
    }
    fn hole_ids(d: &Diagnostics) -> Vec<&str> { d.holes.iter().map(|h| h.hole.as_str()).collect() }
    /// A definition that passes the checks: its model and diagnostics.
    fn passes(src: &str) -> (Model, Diagnostics) {
        let (m, d) = run(src);
        assert!(d.errors.is_empty(), "{src}: {:?}", d.errors);
        (m.unwrap_or_else(|| panic!("{src}")), d)
    }
    const PRIVATE: Area = Area::Subnet(Tier::Private);
    const FUNCTION_IN_VPC: [&str; 2] = ["function-attaches-to-resource-vpc", "attached-function-per-az"];

    // Fails if the function's place is not taken from its connection to the database, or the
    // reasons of the function, the VPC, its zones or subnets are dropped.
    #[test]
    fn a_function_reaching_a_database_is_attached_to_the_vpc() {
        let (m, d) = passes(crate::test_inputs::MIX_SMALLEST);
        assert_eq!(spot(&m, "lambda-1a"), (PRIVATE, Some(0), None));
        assert_eq!(spot(&m, "lambda-1c"), (PRIVATE, Some(1), None));
        assert_eq!(spot(&m, "aurora-1a"), (PRIVATE, Some(0), Some("writer")));
        assert_eq!(spot(&m, "aurora-1c"), (PRIVATE, Some(1), Some("reader")));
        assert!(!node_ids(&m).contains(&"lambda"), "{:?}", node_ids(&m));
        // No public part, NAT gateway or entry gateway.
        assert!(m.nodes.iter().all(|n| !matches!(n.area, Area::Subnet(Tier::Public) | Area::VpcEdge)), "{:?}", node_ids(&m));
        assert!(m.nodes.iter().all(|n| !n.added), "{:?}", node_ids(&m));
        let holes: Vec<_> = d.holes.iter().map(|h| (h.hole.as_str(), h.at.as_str(), h.choices.clone())).collect();
        assert_eq!(holes, [("route.lambda.s3", "flows[1]", strings(&["endpoint", "nat"]))]);
        assert_eq!(rules_of(&d, "lambda"), FUNCTION_IN_VPC);
        assert_eq!(rules_of(&d, "vpc"), ["aurora-cluster-in-vpc"]);
        assert_eq!(rules_of(&d, "az"), ["aurora-subnet-group-two-azs", "default-two-azs-for-database"]);
        assert_eq!(rules_of(&d, "subnet"), ["private-subnet-per-az"]);
        assert_eq!(d.notes, [Rules::builtin().text("note.aux_not_routed", Lang::Ja)]);
        assert_eq!(d.exit_code(), 2);
    }

    // Fails if answering `nat` for an attached function does not add the NAT gateways, the entry
    // gateway, the lines out through them, or their reasons.
    #[test]
    fn answering_nat_adds_the_public_tier() {
        let (m, d) = passes(crate::test_inputs::MIX_NAT);
        let public = Area::Subnet(Tier::Public);
        assert_eq!(spot(&m, "nat-1a"), (public, Some(0), None));
        assert_eq!(spot(&m, "nat-1c"), (public, Some(1), None));
        assert_eq!(spot(&m, "igw"), (Area::VpcEdge, None, None));
        let out = lines(&m).into_iter().filter(|(_, to, _, _)| to.starts_with("nat") || *to == "igw").collect::<Vec<_>>();
        assert_eq!(
            out,
            [
                ("lambda-1a", "nat-1a", Main, "outbound"),
                ("lambda-1c", "nat-1c", Main, "outbound"),
                ("nat-1a", "igw", Main, "outbound"),
                ("nat-1c", "igw", Main, "outbound"),
            ]
        );
        assert_eq!(rules_of(&d, "lambda"), [FUNCTION_IN_VPC[0], FUNCTION_IN_VPC[1], "attached-function-internet-through-nat"]);
        assert_eq!(rules_of(&d, "igw"), ["private-egress-through-nat"]);
        assert_eq!(rules_of(&d, "subnet"), ["public-and-private-subnet-per-az"]);
        assert!(d.holes.is_empty(), "{:?}", d.holes);
        assert_eq!(d.exit_code(), 0);
    }

    // Fails if where a part belongs is decided line by line, so a line written before the one to
    // the database leaves the function outside the VPC or opens no route hole for it.
    #[test]
    fn the_line_order_does_not_move_a_function() {
        let answers: Vec<(Model, Diagnostics)> = crate::test_inputs::ONE_FUNCTION_THREE_WAYS.iter().map(|src| passes(src)).collect();
        let placed = |m: &Model| {
            let mut v: Vec<_> = m.nodes.iter().map(|n| (n.id.clone(), format!("{:?}", n.area), n.az, n.role.clone())).collect();
            v.sort();
            v
        };
        let sorted_holes = |d: &Diagnostics| {
            let mut v = hole_ids(d).into_iter().map(String::from).collect::<Vec<_>>();
            v.sort();
            v
        };
        let reasons = |d: &Diagnostics| {
            let mut v: Vec<_> = d.derived.iter().map(|x| (x.target.clone(), x.rule.clone())).collect();
            v.sort();
            v
        };
        let (first_m, first_d) = &answers[0];
        assert_eq!(spot(first_m, "lambda-1a"), (PRIVATE, Some(0), None));
        assert_eq!(spot(first_m, "lambda-1c"), (PRIVATE, Some(1), None));
        for (i, (m, d)) in answers.iter().enumerate() {
            assert_eq!(placed(m), placed(first_m), "order {i}");
            assert_eq!(sorted_holes(d), ["route.lambda.dynamodb", "route.lambda.s3"], "order {i}");
            assert_eq!(reasons(d), reasons(first_d), "order {i}");
            assert!(!rules_of(d, "lambda").contains(&"regional-service-outside-vpc".to_string()), "order {i}");
        }
        let at = |d: &Diagnostics, id: &str| d.holes.iter().find(|h| h.hole == id).unwrap().at.clone();
        let want = [("flows[1]", "flows[2]"), ("flows[2]", "flows[1]"), ("flows[1]", "flows[0]")];
        for ((_, d), (dynamodb, s3)) in answers.iter().zip(want) {
            assert_eq!((at(d, "route.lambda.dynamodb").as_str(), at(d, "route.lambda.s3").as_str()), (dynamodb, s3));
        }
    }

    // Fails if where a function belongs is decided by its service rather than by the part, so a
    // named function reaching the database and one reaching only a table share a place.
    #[test]
    fn functions_inside_and_outside_the_vpc() {
        let (m, d) = passes(crate::test_inputs::MIX_WEB);
        assert_eq!(spot(&m, "lambda.api@1a"), (PRIVATE, Some(0), None));
        assert_eq!(spot(&m, "lambda.api@1c"), (PRIVATE, Some(1), None));
        assert_eq!(spot(&m, "lambda.report"), (Area::Region, None, None));
        assert_eq!(hole_ids(&d), ["route.ecs.dynamodb"]);
        assert_eq!(rules_of(&d, "lambda.api"), FUNCTION_IN_VPC);
        assert_eq!(rules_of(&d, "lambda.report"), ["regional-service-outside-vpc"]);
    }

    // Fails if the zone copies of a named function take the id of a part named like them.
    #[test]
    fn a_named_function_and_a_part_named_like_its_copy_differ() {
        let (m, _) = passes(crate::test_inputs::ID_CLASH);
        assert_eq!(node(&m, "lambda.api@1a").part, "lambda.api");
        assert_eq!(node(&m, "lambda.api@1c").part, "lambda.api");
        assert_eq!(spot(&m, "lambda.api@1a"), (PRIVATE, Some(0), None));
        let clash = node(&m, "lambda.api-1a");
        assert_eq!((clash.part.as_str(), clash.area, clash.az), ("lambda.api-1a", Area::Region, None));
        let ids: Vec<&str> = node_ids(&m);
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "{ids:?}");
        let edges: Vec<String> = m.edges.iter().map(|e| crate::ids::edge_id(&e.from, &e.to)).collect();
        let mut unique = edges.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), edges.len(), "{edges:?}");
        assert!(edges.iter().all(|e| !ids.contains(&e.as_str())));
    }

    // Fails if a main flow to the internet is accepted from a function outside the VPC, where
    // there is no NAT gateway to draw it through.
    #[test]
    fn a_function_outside_the_vpc_cannot_reach_the_internet() {
        let (m, d) = run("flows:\n  - user -> apigateway(http) -> lambda -> internet\n");
        assert!(m.is_none());
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        assert_eq!(first(&d), ("UNSUPPORTED", Some("flows[0]"), Some("lambda -> internet")));
        assert_eq!(d.errors[0].message, "`lambda -> internet` needs `lambda` in a VPC; connect it to a part in a private subnet");
    }

    // Fails if the VPC's reasons are not taken from the first part written that has them.
    #[test]
    fn the_first_part_with_vpc_reasons_wins() {
        let (_, d) = passes(crate::test_inputs::MIX_WEB);
        assert_eq!(rules_of(&d, "vpc"), ["load-balancer-in-vpc"]);
        assert_eq!(rules_of(&d, "az"), ["alb-spans-two-azs", "default-two-azs"]);
        let (_, d) = passes(crate::test_inputs::MIX_SMALLEST);
        assert_eq!(rules_of(&d, "vpc"), ["aurora-cluster-in-vpc"]);
    }

    // Fails if the route hole of a named attached function cannot be answered by its bracketed id.
    #[test]
    fn a_bracketed_hole_is_answered() {
        let base = crate::test_inputs::MIX_WEB.replace("set:\n", "  - lambda.api -> s3\nset:\n");
        let (m, d) = passes(&format!("{base}choose:\n  route.[lambda.api].s3: endpoint\n"));
        assert_eq!(hole_ids(&d), ["route.ecs.dynamodb"]);
        let to_s3: Vec<_> = m.edges.iter().filter(|e| e.from.starts_with("lambda.api@") && e.to == "s3").map(|e| (e.hole.clone(), e.endpoint)).collect();
        assert_eq!(to_s3, [(None, true), (None, true)]);
        let (m, d) = run(&format!("{base}choose:\n  route.lambda.api.s3: endpoint\n"));
        assert!(m.is_none());
        assert_eq!(d.errors.len(), 1, "{:?}", d.errors);
        assert_eq!(first(&d), ("UNKNOWN_HOLE", Some("choose.route.lambda.api.s3"), Some("route.lambda.api.s3")));
        assert_eq!(d.errors[0].allowed, ["route.ecs.dynamodb", "route.[lambda.api].s3"]);
    }

    // Fails if the VPC's reasons are not taken from the first part written that has them when
    // Aurora comes before the load balancer, or if the order of the lines changes the parts and lines.
    #[test]
    fn aurora_written_first_gives_the_vpc_its_reasons() {
        let vpc_az = |d: &Diagnostics| d.derived.iter().filter(|x| x.target == "vpc" || x.target == "az").map(|x| x.rule.clone()).collect::<Vec<_>>();
        let (first, d) = passes(AURORA_BEFORE_ALB);
        assert_eq!(vpc_az(&d), ["aurora-cluster-in-vpc", "aurora-subnet-group-two-azs", "default-two-azs-for-database"]);
        let (second, d) = passes(ALB_BEFORE_AURORA);
        assert_eq!(vpc_az(&d), ["load-balancer-in-vpc", "alb-spans-two-azs", "default-two-azs"]);
        // Each node and line by its id: where it is, its role and its label.
        let nodes = |m: &Model| {
            let mut v: Vec<_> = m.nodes.iter().map(|n| (n.id.clone(), format!("{:?}", n.area), n.az, n.role.clone(), n.added)).collect();
            v.sort();
            v
        };
        let edges = |m: &Model| {
            let mut v: Vec<_> = m.edges.iter().map(|e| (crate::ids::edge_id(&e.from, &e.to), format!("{:?}", e.rel), e.label.clone(), e.hole.clone())).collect();
            v.sort();
            v
        };
        assert_eq!(nodes(&first), nodes(&second));
        assert_eq!(edges(&first), edges(&second));
        assert!(!first.nodes.is_empty() && !first.edges.is_empty());
    }

    // Fails if the parts or lines of a model of the mixed shape come in another order: the
    // arrangement gives columns in this order.
    #[test]
    fn a_mixed_model_keeps_its_order() {
        use crate::def::Rel::Aux;
        let (m, _) = passes(crate::test_inputs::MIX_SMALLEST);
        let parts = ["user", "lambda-1a", "lambda-1c", "aurora-1a", "aurora-1c", "apigateway", "s3", "cloudwatch"];
        let written = [
            ("user", "apigateway", Main, "https"),
            ("apigateway", "lambda-1a", Main, "invoke"),
            ("apigateway", "lambda-1c", Main, "invoke"),
            ("lambda-1a", "aurora-1a", Main, "read_write"),
            ("lambda-1a", "aurora-1c", Main, "read"),
            ("lambda-1c", "aurora-1a", Main, "read_write"),
            ("lambda-1c", "aurora-1c", Main, "read"),
            ("lambda-1a", "s3", Main, "read_write"),
            ("lambda-1c", "s3", Main, "read_write"),
            ("lambda-1a", "cloudwatch", Aux, "logs_metrics"),
            ("lambda-1c", "cloudwatch", Aux, "logs_metrics"),
            ("aurora-1a", "aurora-1c", Aux, "replication"),
        ];
        assert_eq!(node_ids(&m), parts);
        assert_eq!(lines(&m), written);

        let (m, _) = passes(crate::test_inputs::MIX_NAT);
        let parts = ["user", "igw", "nat-1a", "nat-1c", "lambda-1a", "lambda-1c", "aurora-1a", "aurora-1c", "apigateway", "s3", "cloudwatch"];
        let out = [
            ("lambda-1a", "nat-1a", Main, "outbound"),
            ("lambda-1c", "nat-1c", Main, "outbound"),
            ("nat-1a", "igw", Main, "outbound"),
            ("nat-1c", "igw", Main, "outbound"),
        ];
        assert_eq!(node_ids(&m), parts);
        assert_eq!(lines(&m), [&written[..], &out[..]].concat());
    }

    /// A function attached to the VPC through its database, with traffic out to the internet.
    const ATTACHED_TO_INTERNET: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> internet\n";

    // Fails if a function in the VPC cannot send traffic to the internet through NAT gateways, or
    // the gateways, the lines out or the reasons are not added.
    #[test]
    fn an_attached_function_reaches_the_internet_through_nat() {
        let (m, d) = passes(ATTACHED_TO_INTERNET);
        assert!(d.holes.is_empty(), "{:?}", d.holes);
        assert_eq!(d.exit_code(), 0);
        assert_eq!(node_ids(&m), ["user", "igw", "nat-1a", "nat-1c", "lambda-1a", "lambda-1c", "aurora-1a", "aurora-1c", "apigateway"]);
        let out: Vec<_> = lines(&m).into_iter().filter(|(_, _, _, label)| *label == "outbound").collect();
        assert_eq!(
            out,
            [
                ("lambda-1a", "nat-1a", Main, "outbound"),
                ("lambda-1c", "nat-1c", Main, "outbound"),
                ("nat-1a", "igw", Main, "outbound"),
                ("nat-1c", "igw", Main, "outbound"),
            ]
        );
        assert!(m.nodes.iter().all(|n| n.service != "internet"));
        assert_eq!(rules_of(&d, "lambda"), [FUNCTION_IN_VPC[0], FUNCTION_IN_VPC[1], "attached-function-internet-through-nat"]);
        assert_eq!(rules_of(&d, "igw"), ["private-egress-through-nat"]);
        assert_eq!(rules_of(&d, "nat"), ["nat-per-az"]);

        let (m, d) = passes(&format!("{ATTACHED_TO_INTERNET}set:\n  azs: 3\n  nat: single\n"));
        assert!(d.holes.is_empty(), "{:?}", d.holes);
        assert_eq!(d.exit_code(), 0);
        assert_eq!(
            node_ids(&m),
            ["user", "igw", "nat-1a", "lambda-1a", "lambda-1c", "lambda-1d", "aurora-1a", "aurora-1c", "aurora-1d", "apigateway"]
        );
        assert_eq!(spot(&m, "nat-1a"), (Area::Subnet(Tier::Public), Some(0), None));
        let out: Vec<_> = lines(&m).into_iter().filter(|(_, _, _, label)| *label == "outbound").collect();
        assert_eq!(
            out,
            [
                ("lambda-1a", "nat-1a", Main, "outbound"),
                ("lambda-1c", "nat-1a", Main, "outbound"),
                ("lambda-1d", "nat-1a", Main, "outbound"),
                ("nat-1a", "igw", Main, "outbound"),
            ]
        );
        assert!(m.nodes.iter().all(|n| n.service != "internet"));
        assert_eq!(rules_of(&d, "lambda"), [FUNCTION_IN_VPC[0], FUNCTION_IN_VPC[1], "attached-function-internet-through-nat"]);
        assert_eq!(rules_of(&d, "igw"), ["private-egress-through-nat"]);
        assert!(rules_of(&d, "nat").is_empty());
    }

    /// The tests of the inference with a VPC, run on this one.
    mod with_a_vpc {
        use crate::diag::{DerivedKind, Diagnostics};
        use crate::ids::copy_id;
        use crate::model::Model;
        use crate::rules::{Lang, Rules};

        fn run(src: &str) -> (Option<Model>, Diagnostics) { crate::inference::infer(&crate::def::parse(src).unwrap(), &Rules::builtin()) }
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

        // Fails if a shape without a VPC is stopped as one with a VPC, or a pair outside the list is
        // accepted or reported at another line.
        #[test]
        fn shapes_outside_the_pairs_stop_with_unsupported() {
            let (m, d) = run("flows:\n  - user -> cloudfront -> s3\n");
            assert!(m.is_some(), "{:?}", d.errors);
            assert_eq!(d.exit_code(), 0);
            for (src, at, got) in [("flows:\n  - user -> dynamodb\n", "flows[0]", "user -> dynamodb"), ("flows:\n  - user -> alb -> aurora\n", "flows[0]", "alb -> aurora")] {
                let (m, d) = run(src);
                assert!(m.is_none(), "{src}");
                assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNSUPPORTED", Some(at), Some(got)), "{src}");
                assert_eq!(d.errors[0].message, format!("`{got}` is not a supported connection"), "{src}");
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

        // Breaks when the Region frame of a VPC definition loses its reason, gets it twice, or moves it away from just after the AWS Cloud frame's.
        #[test]
        fn vpc_region_frames_have_their_reasons_once() {
            let web = include_str!("../../../../tests/cases/web-2az-ja/def.yaml");
            let written = web.replace("set:\n", "set:\n  region: ap-northeast-1\n");
            assert_ne!(web, written);
            for (src, want) in [(web.to_string(), vec!["resources-inside-region-frame", "default-region"]), (written, vec!["resources-inside-region-frame"])] {
                let (_, d) = run(&src);
                let region: Vec<&str> = d.derived.iter().filter(|x| x.target == "region").map(|x| x.rule.as_str()).collect();
                assert_eq!(region, want, "{src}");
                assert!(d.derived.iter().filter(|x| x.target == "region").all(|x| x.added), "{src}");
                let cloud = d.derived.iter().position(|x| x.target == "aws-cloud").unwrap();
                assert_eq!(d.derived[cloud + 1].target, "region", "{src}");
                assert_eq!(d.derived[cloud + 1].rule, "resources-inside-region-frame", "{src}");
            }
        }

        // Fails if an added part loses a reason or gets one too many.
        #[test]
        fn every_added_part_has_its_reasons() {
            let (m, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n");
            let m = m.unwrap();
            let added: Vec<&str> = m.nodes.iter().filter(|n| n.added).map(|n| n.id.as_str()).collect();
            assert_eq!(added, ["igw", "nat-1a", "nat-1c"]);
            assert_eq!(super::rules_of(&d, "igw"), ["internet-facing-alb-in-vpc-with-igw"]);
            for id in ["nat-1a", "nat-1c"] {
                assert_eq!(super::rules_of(&d, id), ["private-egress-through-nat", "nat-device-is-nat-gateway"], "{id}");
            }
        }

        // Fails if a NAT gateway is drawn without the reason that archwire chose that kind of NAT device.
        #[test]
        fn every_nat_gateway_says_why_it_is_a_nat_gateway() {
            let web = crate::test_inputs::WEB_CASES.iter().find(|(n, _)| *n == "web-2az-ja").unwrap().1;
            let src = web.replace("set:\n", "set:\n  nat: single\n");
            assert_ne!(web, src);
            let (m, d) = run(&src);
            let rules = Rules::builtin();
            let egress = |s: &str| rules.service(s).and_then(|s| s.derived_role) == Some(crate::rules::DerivedRole::EgressNat);
            let nats: Vec<String> = m.unwrap().nodes.iter().filter(|n| n.added && egress(&n.service)).map(|n| n.id.clone()).collect();
            assert_eq!(nats, ["nat-1a"]);
            let choice: Vec<_> = d.derived.iter().filter(|x| x.rule == "nat-device-is-nat-gateway").collect();
            assert_eq!(choice.len(), 1, "{:?}", d.derived);
            assert_eq!((choice[0].target.as_str(), choice[0].kind.clone()), ("nat-1a", crate::diag::DerivedKind::Default));
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
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNSUPPORTED", Some("flows[1]"), Some("internet -> alb")));
            assert_eq!(d.errors[0].message, "`internet -> alb` is not a supported connection");
            let (_, d) = run("flows:\n  - user -> alb -> ecs -> nat\nset:\n  azs: 5\n  lang: fr\n");
            let codes: Vec<_> = d.errors.iter().map(|e| e.code.as_str()).collect();
            assert_eq!(codes, ["UNKNOWN_SERVICE", "BAD_SETTING", "BAD_SETTING"]);
        }
        #[test]
        fn links_outside_the_allowlist_stop_with_unsupported() {
            let base = "flows:\n  - user -> alb -> ecs -> aurora\n";
            for line in ["cloudfront -> ecs", "user -> aurora", "user ~> cloudfront", "ecs -> user", "ecs -> cloudfront", "alb -> s3", "user -> dynamodb"] {
                let (m, d) = run(&format!("{base}  - {line}\n"));
                assert!(m.is_none(), "{line}");
                assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{line}");
                assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"), "{line}");
            }
            assert!(run(&format!("{base}  - aurora -> ecs\n")).0.is_some());
        }

        #[test]
        fn attachments_carry_no_flows_and_attach_only_as_listed() {
            let base = "flows:\n  - user -> cloudfront -> alb -> ecs\n";
            for line in [
                "waf -- acm", "acm -- waf", "cloudfront -- waf", "cloudfront -- acm", "alb -- acm",
                "user -- alb", "alb -- cloudfront", "waf -> alb", "acm -> s3", "waf ~> cloudwatch", "user -> waf", "cloudfront -> acm",
            ] {
                let (m, d) = run(&format!("{base}  - {line}\n"));
                assert!(m.is_none(), "{line}");
                assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNSUPPORTED", Some("flows[1]"), Some(line)), "{line}");
                assert_eq!(d.errors[0].message, format!("`{line}` is not a supported connection"), "{line}");
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
            assert_eq!((d.errors[0].code.as_str(), d.errors[0].at.as_deref(), d.errors[0].got.as_deref()), ("UNSUPPORTED", Some("flows[1]"), Some("user ~> alb")));
            assert_eq!(d.errors[0].message, "`user ~> alb` is not a supported connection");
        }

        #[test]
        fn every_allowed_combination_compiles() {
            let src = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> alb\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - ecs -> internet\n  - waf -- cloudfront\n  - acm -- cloudfront\nchoose:\n  route.ecs.s3: endpoint\n";
            let (m, d) = run(src);
            assert!(d.errors.is_empty(), "{:?}", d.errors);
            assert!(m.is_some());
            assert_eq!(d.exit_code(), 0);
        }

        // Fails if a named part outside the VPC stops a definition with a VPC, or loses its reason.
        #[test]
        fn a_named_part_outside_the_vpc_infers() {
            let (m, d) = run("flows:\n  - user -> cloudfront -> alb -> ecs\n  - cloudfront -> s3.web\n");
            assert!(d.errors.is_empty(), "{:?}", d.errors);
            assert!(ids(&m.unwrap()).contains(&"s3.web"));
            let why: Vec<_> = d.derived.iter().filter(|x| x.target == "s3.web").map(|x| x.rule.as_str()).collect();
            assert_eq!(why, ["regional-service-outside-vpc"]);
        }

        #[test]
        fn every_node_knows_the_part_it_stands_for() {
            let m = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\n").0.unwrap();
            let part = |id: &str| m.nodes.iter().find(|n| n.id == id).unwrap().part.clone();
            assert_eq!([part("ecs-1a"), part("nat-1c"), part("alb"), part("igw")], ["ecs", "nat", "alb", "igw"]);
            assert!(m.nodes.iter().all(|n| n.tag.is_none()));
        }

        // Breaks when a node's area is not taken from its service, or a per-AZ copy is mixed up with a part spanning the AZs.
        #[test]
        fn every_node_has_the_area_its_service_gives() {
            use crate::model::{Area, Tier};
            let src = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\nset:\n  lang: ja\n";
            let m = run(src).0.unwrap();
            let at = |id: &str| {
                let n = m.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("{id}"));
                (n.area, n.az)
            };
            assert_eq!(at("user"), (Area::Outside, None));
            assert_eq!(at("cloudfront"), (Area::Cloud, None));
            assert_eq!(at("igw"), (Area::VpcEdge, None));
            assert_eq!(at("alb"), (Area::Subnet(Tier::Public), None));
            assert_eq!(at("nat-1a"), (Area::Subnet(Tier::Public), Some(0)));
            assert_eq!(at("ecs-1c"), (Area::Subnet(Tier::Private), Some(1)));
            assert_eq!(at("aurora-1a"), (Area::Subnet(Tier::Private), Some(0)));
            assert_eq!(at("s3"), (Area::Region, None));
            assert_eq!(at("cloudwatch"), (Area::Region, None));
        }

        // Breaks when the VPC layout does not read `set.direction`, or takes an unknown value as a default.
        #[test]
        fn direction_is_read_in_both_layouts() {
            use crate::model::Axis;
            const B: &str = "flows:\n  - user -> alb -> ecs\n";
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
            assert_eq!(d.errors[0].allowed, ["azs", "direction", "lang", "nat", "readers", "region"]);
        }

        // Breaks when `direction` is checked out of its place among the other settings.
        #[test]
        fn a_bad_direction_is_reported_after_lang_and_before_nat() {
            let (_, d) = run("flows:\n  - user -> alb -> ecs\n  - ecs -> internet\nset:\n  nat: none\n  direction: up\n  lang: fr\n");
            let at: Vec<_> = d.errors.iter().map(|e| e.at.as_deref().unwrap()).collect();
            assert_eq!(at, ["set.lang", "set.direction", "set.nat"]);
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
            // Every written part, and a copy per Availability Zone of every part, named or not.
            let mut ids: Vec<String> = Vec::new();
            for r in d.links.iter().flat_map(|l| [&l.from, &l.to]) {
                if !ids.contains(&r.name) {
                    ids.push(r.name.clone());
                }
                for suffix in ["1a", "1c", "1d"] {
                    let copy = copy_id(&r.name, suffix);
                    if !ids.contains(&copy) {
                        ids.push(copy);
                    }
                }
            }
            assert!(ids.contains(&"lambda.1a".to_string()) && ids.contains(&"lambda-1a".to_string()));
            assert!(ids.contains(&"lambda.1a@1a".to_string()) && ids.contains(&"lambda.a-to-b@1d".to_string()));
            let written = ids.iter().filter(|i| d.links.iter().any(|l| &l.from.name == *i || &l.to.name == *i)).count();
            let copies = ids.len() - written;
            assert_eq!((written, copies), (11, 33));
            // A line id names exactly one ordered pair of nodes, so it points at the parts it joins.
            for (from, to) in &links {
                let id = crate::ids::edge_id(from, to);
                let pairs: Vec<_> = ids.iter().flat_map(|a| ids.iter().map(move |b| (a, b)))
                    .filter(|(a, b)| a != b && crate::ids::edge_id(a, b) == id).collect();
                assert_eq!(pairs.len(), 1, "{id}");
                assert_eq!((pairs[0].0.as_str(), pairs[0].1.as_str()), (*from, *to), "{id}");
            }
            let mut lines: Vec<String> = ids.iter().flat_map(|a| ids.iter().filter(move |b| *b != a).map(move |b| crate::ids::edge_id(a, b))).collect();
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
}
