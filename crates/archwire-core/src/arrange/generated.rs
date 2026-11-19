//! Definitions mixing serverless parts with VPC parts: made by a generator along fixed axes, and
//! named by hand for shapes the generator does not make. Every one is drawn along both axes, and
//! each drawing is kept apart at the stage where it stops, so no failure hides another.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::diag::{Diagnostics, Problem};
use crate::layout::Layout;
use crate::lines::Line;
use crate::model::{Area, Axis, Model};
use crate::route::{port_point, LinePlan, Wire};
use crate::rules::Rules;
use crate::test_inputs::{ID_CLASH, MIX_LOGS, MIX_NAT, MIX_SMALLEST, MIX_WEB, ONE_FUNCTION_THREE_WAYS, TWIN_APPS};

/// How many definitions `generated_mixed` makes.
pub(in crate::arrange) const MIXED_DEFINITIONS: usize = 840;

/// How many named inputs `mixed_representatives` gives.
const REPRESENTATIVES: usize = 25;

/// The named inputs drawn for the diagram checker, by name, in this order: the first twenty
/// named ones but `partial-senders`, with `both-to-the-internet` in its place, a web stack and a
/// function sending traffic out through the same NAT gateways.
/// Left out: `partial-senders`, whose top-to-bottom drawing has a line over an icon that the
/// router does not avoid; the run over every mixed definition counts it.
const REPRESENTATIVES_FOR_THE_CHECKER: [&str; 20] = [
    "web-and-api",
    "mix-web",
    "same-row-3az",
    "twin-apps",
    "twin-apps-3az",
    "id-clash",
    "mix-web-attach",
    "mix-web-aux",
    "four-functions",
    "both-to-the-internet",
    "aux-order",
    "fewer-readers-mix",
    "mix-logs",
    "two-attached-to-one-log",
    "function-before-web-3az",
    "one-function-1",
    "one-function-2",
    "one-function-3",
    "mix-smallest",
    "mix-nat",
];

/// Every definition mixing serverless parts with VPC parts the generator makes, in order: two or
/// three zones; one NAT gateway per zone or one in all, only where a NAT gateway is drawn; no web
/// stack, one behind CloudFront sharing Aurora with the function, or one behind a load balancer
/// reaching DynamoDB through an endpoint; the function in the VPC unnamed or named; its traffic
/// out: none, to S3 with the route left open, answered with an endpoint or a NAT gateway, or to
/// the internet; a function outside the VPC or not; side lines to CloudWatch from none, from the
/// function in the VPC, or from both functions; English or Japanese. Each definition keeps the
/// main line from the user through an HTTP API and the function in the VPC to Aurora.
pub(in crate::arrange) fn generated_mixed() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for azs in [2, 3] {
        for nat in ["per-az", "single"] {
            for web in [0, 1, 2] {
                for f in ["lambda", "lambda.api"] {
                    for outward in ["none", "open", "endpoint", "nat", "internet"] {
                        for outer in [false, true] {
                            for aux in ["none", "attached", "both"].into_iter().filter(|a| outer || *a != "both") {
                                for lang in ["en", "ja"] {
                                    let mut src = String::from("flows:\n");
                                    match web {
                                        1 => src.push_str("  - user -> cloudfront -> alb -> ecs -> aurora\n"),
                                        2 => src.push_str("  - user -> alb -> ecs\n  - ecs -> dynamodb\n"),
                                        _ => {}
                                    }
                                    src.push_str(&format!("  - user -> apigateway(http) -> {f} -> aurora\n"));
                                    match outward {
                                        "open" | "endpoint" | "nat" => src.push_str(&format!("  - {f} -> s3\n")),
                                        "internet" => src.push_str(&format!("  - {f} -> internet\n")),
                                        _ => {}
                                    }
                                    if outer {
                                        src.push_str("  - apigateway -> lambda.report -> dynamodb\n");
                                    }
                                    if aux != "none" {
                                        src.push_str(&format!("  - {f} ~> cloudwatch\n"));
                                    }
                                    if aux == "both" {
                                        src.push_str("  - lambda.report ~> cloudwatch\n");
                                    }
                                    src.push_str(&format!("set:\n  azs: {azs}\n"));
                                    if outward == "nat" || outward == "internet" {
                                        src.push_str(&format!("  nat: {nat}\n"));
                                    }
                                    src.push_str(&format!("  lang: {lang}\n"));
                                    let mut choose = String::new();
                                    if web == 2 {
                                        choose.push_str("  route.ecs.dynamodb: endpoint\n");
                                    }
                                    if outward == "endpoint" || outward == "nat" {
                                        choose.push_str(&format!("  {}: {outward}\n", crate::ids::route_hole(f, "s3")));
                                    }
                                    if !choose.is_empty() {
                                        src.push_str("choose:\n");
                                        src.push_str(&choose);
                                    }
                                    if !out.contains(&src) {
                                        out.push(src);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

const WEB_AND_API: &str = "flows:\n  - user -> alb -> ecs\n  - user -> apigateway(http) -> lambda -> dynamodb\n";
const SAME_ROW_3AZ: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> dynamodb\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - user -> apigateway -> lambda.report -> dynamodb\n  - ecs ~> cloudwatch\n  - lambda.api ~> cloudwatch\nset:\n  lang: ja\n  azs: 3\n";
const TWIN_APPS_3AZ: &str = "flows:\n  - user -> cloudfront -> alb -> ecs.web -> aurora.web\n  - alb -> ecs.admin -> aurora.admin\nset:\n  azs: 3\n";
const MIX_WEB_ATTACH: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> dynamodb\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - user -> apigateway -> lambda.report -> dynamodb\n  - ecs ~> cloudwatch\n  - lambda.api ~> cloudwatch\n  - waf -- cloudfront\n  - acm -- cloudfront\nset:\n  lang: ja\n";
const MIX_WEB_AUX: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> dynamodb\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - user -> apigateway -> lambda.report -> dynamodb\n  - ecs ~> cloudwatch\n  - lambda.api ~> cloudwatch\n  - alb ~> cloudwatch\n  - aurora ~> cloudwatch\nset:\n  lang: ja\n";
const FOUR_FUNCTIONS: &str = "flows:\n  - user -> apigateway(http) -> lambda.a -> aurora\n  - apigateway -> lambda.b -> aurora\n  - apigateway -> lambda.c -> dynamodb\n  - apigateway -> lambda.d -> dynamodb\n  - lambda.a ~> cloudwatch\n  - lambda.c ~> cloudwatch\n";
const PARTIAL_SENDERS: &str = "flows:\n  - user -> apigateway(http) -> lambda.resize-images-fast -> dynamodb.images\n  - apigateway -> lambda.thumbnails-generator -> dynamodb.images\n  - apigateway -> lambda.metadata-extractor -> dynamodb.meta\n  - apigateway -> lambda.notification-sender -> dynamodb.meta\n  - lambda.resize-images-fast ~> cloudwatch.app\n  - lambda.thumbnails-generator ~> cloudwatch.app\n  - lambda.metadata-extractor ~> cloudwatch.app\n  - lambda.metadata-extractor ~> cloudwatch.audit\n  - lambda.notification-sender ~> cloudwatch.audit\n  - lambda.resize-images-fast -> aurora\n  - lambda.metadata-extractor -> aurora\n";
const AUX_ORDER: &str = "flows:\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - apigateway -> lambda.report -> dynamodb\n  - lambda.report ~> cloudwatch\n  - lambda.api ~> cloudwatch\n";
const FEWER_READERS_MIX: &str = "flows:\n  - user -> alb -> ecs -> aurora\n  - aurora -> ecs\n  - aurora -> s3\n  - user -> apigateway(http) -> lambda -> aurora\nset:\n  azs: 3\n  readers: 1\nchoose:\n  route.aurora.s3: endpoint\n";
const TWO_ATTACHED_TO_ONE_LOG: &str = "flows:\n  - user -> apigateway(http) -> lambda.a -> aurora\n  - apigateway -> lambda.b -> aurora\n  - lambda.a ~> cloudwatch\n  - lambda.b ~> cloudwatch\n";
const FUNCTION_BEFORE_WEB_3AZ: &str = "flows:\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - user -> cloudfront -> alb -> ecs -> aurora\nset:\n  azs: 3\n";
const BOTH_TO_THE_INTERNET: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> apigateway(http) -> lambda -> aurora\n  - ecs -> internet\n  - lambda -> internet\n";
const SPLIT_ROUTES: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - user -> apigateway(http) -> lambda -> aurora\n  - ecs -> s3\n  - lambda -> s3\nchoose:\n  route.ecs.s3: endpoint\n  route.lambda.s3: nat\n";
const MIX_COGNITO: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - cognito -- apigateway\n  - lambda ~> cloudwatch\n";
const ONE_FRONT_TWO_WAYS: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront -> apigateway(http) -> lambda -> aurora\n";
const SHARED_BUCKET: &str = "flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\nchoose:\n  route.lambda.s3: endpoint\n";

/// Named definitions picked by hand for shapes the generator does not make.
pub(in crate::arrange) fn mixed_representatives() -> Vec<(&'static str, String)> {
    let list: Vec<(&'static str, &str)> = vec![
        ("web-and-api", WEB_AND_API),
        ("mix-web", MIX_WEB),
        ("same-row-3az", SAME_ROW_3AZ),
        ("twin-apps", TWIN_APPS),
        ("twin-apps-3az", TWIN_APPS_3AZ),
        ("id-clash", ID_CLASH),
        ("mix-web-attach", MIX_WEB_ATTACH),
        ("mix-web-aux", MIX_WEB_AUX),
        ("four-functions", FOUR_FUNCTIONS),
        ("partial-senders", PARTIAL_SENDERS),
        ("aux-order", AUX_ORDER),
        ("fewer-readers-mix", FEWER_READERS_MIX),
        ("mix-logs", MIX_LOGS),
        ("two-attached-to-one-log", TWO_ATTACHED_TO_ONE_LOG),
        ("function-before-web-3az", FUNCTION_BEFORE_WEB_3AZ),
        ("one-function-1", ONE_FUNCTION_THREE_WAYS[0]),
        ("one-function-2", ONE_FUNCTION_THREE_WAYS[1]),
        ("one-function-3", ONE_FUNCTION_THREE_WAYS[2]),
        ("mix-smallest", MIX_SMALLEST),
        ("mix-nat", MIX_NAT),
        ("both-to-the-internet", BOTH_TO_THE_INTERNET),
        ("split-routes", SPLIT_ROUTES),
        ("mix-cognito", MIX_COGNITO),
        ("one-front-two-ways", ONE_FRONT_TWO_WAYS),
        ("shared-bucket", SHARED_BUCKET),
    ];
    list.into_iter().map(|(n, s)| (n, s.to_string())).collect()
}

/// One drawing of a model, kept apart at every stage so a failure is counted where it happens.
#[derive(Debug)]
pub(in crate::arrange) enum Drawn {
    /// The definition did not parse, or the inference gave no model.
    Errors(Vec<Problem>),
    /// The layout gave LAYOUT_FAILED problems.
    LayoutFailed(Vec<Problem>),
    /// A stage panicked: its name and the panic message.
    Panicked(String),
    /// A diagram, the router's notes, the diagnostics with those notes as JSON, and what the
    /// checks on the gathered and drawn lines found.
    Drawn { xml: String, notes: Vec<String>, json: String, lines_broken: Vec<String> },
}

/// Runs `f`, turning a panic into the stage's name and the panic message.
fn guarded<T>(stage: &str, f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
        format!("{stage}: {msg}")
    })
}

/// Parses, infers with the one inference, lays out along `axis`, gathers the lines, routes them
/// from the placement's drafts and writes the diagram, keeping what each stage gave. The checks on
/// the gathered and drawn lines run only when a diagram was drawn.
pub(in crate::arrange) fn draw_counted(src: &str, axis: Axis) -> Drawn {
    draw_stages(src, axis).0
}

/// Like `draw_counted`, with what `through_the_vpc` found on the drawn lines; empty when no
/// diagram was drawn.
fn draw_stages(src: &str, axis: Axis) -> (Drawn, Vec<String>) {
    let stop = |d: Drawn| (d, Vec::new());
    let r = Rules::builtin();
    let def = match crate::def::parse(src) {
        Ok(d) => d,
        Err(e) => return stop(Drawn::Errors(e)),
    };
    // The test rules accept `fewer-readers-mix`; the generated definitions use none of their extra pairs.
    let (model, mut diag) = match guarded("infer", || crate::inference::infer(&def, &crate::test_inputs::rules_with_test_links())) {
        Ok(x) => x,
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    let Some(model) = model else { return stop(Drawn::Errors(diag.errors)) };
    if !diag.errors.is_empty() {
        return stop(Drawn::Errors(diag.errors));
    }
    let placement = match guarded("arrange", || super::arrange(&model, &r, axis)) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return stop(Drawn::LayoutFailed(e)),
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    let lines = match guarded("gather", || crate::lines::gather(&model, &placement.layout, &r).0) {
        Ok(l) => l,
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    let (wires, notes) = match guarded("route", || crate::route::route_planned(&model, &placement.layout, &r, &placement.lines)) {
        Ok(x) => x,
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    diag.notes.extend(notes.iter().cloned());
    let xml = match guarded("write", || crate::drawio::write(&model, &placement.layout, &wires, &diag, &r)) {
        Ok(x) => x,
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    let lines_broken = match guarded("line_checks", || line_checks(&model, &lines, &placement.lines, &wires)) {
        Ok(l) => l,
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    let through = match guarded("through_the_vpc", || through_the_vpc(&model, &placement.layout, &wires)) {
        Ok(t) => t,
        Err(p) => return stop(Drawn::Panicked(p)),
    };
    (Drawn::Drawn { xml, notes, json: diag.to_json(), lines_broken }, through)
}

/// Check 8, apart from the checks on gathered lines: a drawn line whose two ends are both outside
/// the VPC frame (not in a subnet, not on the VPC's edge) never passes through the inside of the
/// VPC's rectangle, its path taken from the exit port through every waypoint to the entry port.
/// Empty when the diagram has no VPC.
fn through_the_vpc(m: &Model, layout: &Layout, wires: &[Wire]) -> Vec<String> {
    let Some(vpc) = layout.frames.iter().find(|f| f.id == super::azgrid::VPC_FRAME).map(|f| f.rect) else { return Vec::new() };
    let outside = |id: &str| m.nodes.iter().find(|n| n.id == id).is_some_and(|n| !matches!(n.area, Area::Subnet(_) | Area::VpcEdge));
    let mut found = Vec::new();
    for w in wires.iter().filter(|w| outside(&w.from) && outside(&w.to)) {
        let mut path = vec![port_point(layout.rect(&w.from), w.exit)];
        path.extend(w.points.iter().copied());
        path.push(port_point(layout.rect(&w.to), w.entry));
        if path.windows(2).any(|s| crate::path::hits(s[0], s[1], vpc)) {
            found.push(format!("8: {} runs through the VPC {vpc:?} along {path:?}", w.id));
        }
    }
    found
}

/// The checks on gathered and drawn lines no diagram shows, each finding prefixed with its
/// number: 1 every semantic edge in exactly one gathered line; 2 the representative among the
/// line's edges; 3 one label key, hole and endpoint route in a line; 4 each part's sources only
/// the zones of its copies whose edges are in the line; 5 a draft for each representative;
/// 6 no wire id twice in a diagram; 7 the drawn lines standing for every semantic edge exactly once.
fn line_checks(m: &Model, lines: &[Line], plans: &[LinePlan], wires: &[Wire]) -> Vec<String> {
    let mut found = Vec::new();
    for (i, e) in m.edges.iter().enumerate() {
        let n = lines.iter().map(|l| l.edges.iter().filter(|&&x| x == i).count()).sum::<usize>();
        if n != 1 {
            found.push(format!("1: edge {i} {}->{} is in {n} lines", e.from, e.to));
        }
    }
    let node = |id: &str| m.nodes.iter().find(|n| n.id == id);
    for l in lines {
        let name = crate::ids::edge_id(&l.from, &l.to);
        if !l.edges.contains(&l.rep) {
            found.push(format!("2: {name} has representative {} outside {:?}", l.rep, l.edges));
        }
        // A number past the model's edges is found here, before any check reads the edge.
        for &k in l.edges.iter().filter(|&&k| k >= m.edges.len()) {
            found.push(format!("1: {name} holds edge {k}, past the model's {} edges", m.edges.len()));
        }
        for &k in l.edges.iter().filter(|&&k| k < m.edges.len()) {
            let e = &m.edges[k];
            if e.label != l.label_key || e.hole != l.hole || e.endpoint != l.endpoint {
                found.push(format!("3: {name} holds edge {k} {}->{} with label {:?}, hole {:?}, endpoint {}", e.from, e.to, e.label, e.hole, e.endpoint));
            }
        }
        let mut want: Vec<(String, Vec<usize>)> = Vec::new();
        for &k in l.edges.iter().filter(|&&k| k < m.edges.len()) {
            let Some(f) = node(&m.edges[k].from) else {
                found.push(format!("4: {name} holds edge {k} from an unknown part"));
                continue;
            };
            let at = match want.iter().position(|(p, _)| *p == f.part) {
                Some(at) => at,
                None => {
                    want.push((f.part.clone(), Vec::new()));
                    want.len() - 1
                }
            };
            want[at].1.extend(f.az);
        }
        for w in &mut want {
            w.1.sort_unstable();
            w.1.dedup();
        }
        let got: Vec<(String, Vec<usize>)> = l.sources.iter().map(|s| (s.part.clone(), s.azs.clone())).collect();
        if got != want {
            found.push(format!("4: {name} has sources {got:?}, its edges send from {want:?}"));
        }
        if !plans.iter().any(|p| p.edge == l.rep) {
            found.push(format!("5: {name} has no draft for its representative {}", l.rep));
        }
    }
    let mut ids: Vec<&str> = wires.iter().map(|w| w.id.as_str()).collect();
    ids.sort_unstable();
    for pair in ids.windows(2).filter(|p| p[0] == p[1]) {
        found.push(format!("6: wire id {} twice", pair[0]));
    }
    let mut semantic: Vec<(String, String)> = m.edges.iter().map(|e| (e.from.clone(), e.to.clone())).collect();
    let mut drawn: Vec<(String, String)> = wires.iter().flat_map(|w| w.represents.clone()).collect();
    semantic.sort();
    drawn.sort();
    if semantic != drawn {
        let missing: Vec<&(String, String)> = semantic.iter().filter(|e| drawn.iter().filter(|d| d == e).count() < semantic.iter().filter(|s| s == e).count()).collect();
        let extra: Vec<&(String, String)> = drawn.iter().filter(|d| semantic.iter().filter(|s| s == d).count() < drawn.iter().filter(|x| x == d).count()).collect();
        found.push(format!("7: drawn lines miss {missing:?} and repeat or add {extra:?}"));
    }
    found
}

/// A definition with its direction written in its settings.
fn with_direction(src: &str, way: &str) -> String {
    match src.find("set:\n") {
        Some(at) => format!("{}  direction: {way}\n{}", &src[..at + 5], &src[at + 5..]),
        None => match src.find("choose:\n") {
            Some(at) => format!("{}set:\n  direction: {way}\n{}", &src[..at], &src[at..]),
            None => format!("{src}set:\n  direction: {way}\n"),
        },
    }
}

/// Every input of the full count: the generated definitions by number, then the named ones.
fn all_inputs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = generated_mixed().into_iter().enumerate().map(|(i, s)| (format!("generated_mixed#{i}"), s)).collect();
    out.extend(mixed_representatives().into_iter().map(|(n, s)| (n.to_string(), s)));
    out
}

const AXES: [(Axis, &str); 2] = [(Axis::Right, "right"), (Axis::Down, "down")];

/// The kinds a drawing is counted as, in the order they are reported. A drawing counts as the
/// first stage it stops at. A drawn diagram is `lines_broken` when a check on its gathered and
/// drawn lines (1 to 7) finds anything, else `broken` when the router names a line it could not
/// draw clear, else `through_vpc` when a line between two parts outside the VPC passes through
/// it (check 8), else `pass`.
const KINDS: [&str; 7] = ["errors", "layout_failed", "panicked", "lines_broken", "broken", "through_vpc", "pass"];

/// One cell of a report: no tab or line break.
fn cell(s: &str) -> String {
    s.replace('\t', " ").replace('\n', "\\n")
}

fn problems(p: &[Problem]) -> String {
    p.iter().map(|p| format!("{} {}: {}", p.code, p.at.as_deref().unwrap_or("-"), p.message)).collect::<Vec<_>>().join(" | ")
}

/// The file stem a drawing is written under: the input's name with `#` made `-`, then the axis,
/// so each row of the report names one file.
fn stem(name: &str, way: &str) -> String {
    format!("{}-{way}", name.replace('#', "-"))
}

/// What drawing a list of inputs gave.
struct Counted {
    /// One per drawing: name, axis, kind, what was found, the definition.
    rows: Vec<[String; 5]>,
    /// The drawings of each of `KINDS`.
    kinds: Vec<usize>,
    /// The findings of each check, 1 to 8, over every drawn diagram.
    checks: [usize; 8],
    /// One per drawn diagram check 8 found anything in, whatever its kind: name, axis, kind, findings.
    through: Vec<String>,
}

/// Draws every input along both axes. With `out`, writes each drawn diagram there as
/// `<stem>.drawio` beside its definition with its direction written, `<stem>.yaml`, and its
/// diagnostics with the router's notes, `<stem>.json`, after removing the files of these stems a
/// run left.
fn count(inputs: &[(String, String)], out: Option<&std::path::Path>) -> Counted {
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).unwrap();
        for (name, _) in inputs {
            for (_, way) in AXES {
                for ext in ["drawio", "yaml", "json"] {
                    let f = dir.join(format!("{}.{ext}", stem(name, way)));
                    if f.exists() {
                        std::fs::remove_file(f).unwrap();
                    }
                }
            }
        }
    }
    let mut c = Counted { rows: Vec::new(), kinds: vec![0; KINDS.len()], checks: [0; 8], through: Vec::new() };
    for (name, src) in inputs {
        for (axis, way) in AXES {
            let (drawn, through) = draw_stages(src, axis);
            let (kind, found) = match drawn {
                Drawn::Errors(e) => ("errors", problems(&e)),
                Drawn::LayoutFailed(e) => ("layout_failed", problems(&e)),
                Drawn::Panicked(p) => ("panicked", p),
                Drawn::Drawn { xml, notes, json, lines_broken } => {
                    if let Some(dir) = out {
                        std::fs::write(dir.join(format!("{}.drawio", stem(name, way))), &xml).unwrap();
                        std::fs::write(dir.join(format!("{}.yaml", stem(name, way))), with_direction(src, way)).unwrap();
                        std::fs::write(dir.join(format!("{}.json", stem(name, way))), json).unwrap();
                    }
                    let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                    for f in lines_broken.iter().chain(&through) {
                        if let Some(k) = f.split(':').next().and_then(|k| k.parse::<usize>().ok()).filter(|k| (1..=8).contains(k)) {
                            c.checks[k - 1] += 1;
                        }
                    }
                    let mut all: Vec<String> = lines_broken.clone();
                    all.extend(broken.iter().map(|n| n.to_string()));
                    match (lines_broken.is_empty(), broken.is_empty(), through.is_empty()) {
                        (false, _, _) => ("lines_broken", all.join(" | ")),
                        (true, false, _) => ("broken", all.join(" | ")),
                        (true, true, false) => ("through_vpc", through.join(" | ")),
                        (true, true, true) => ("pass", notes.join(" | ")),
                    }
                }
            };
            c.kinds[KINDS.iter().position(|k| *k == kind).expect("a listed kind")] += 1;
            if !through.is_empty() {
                c.through.push(format!("{name}\t{way}\t{kind}\t{}", cell(&through.join(" | "))));
            }
            c.rows.push([name.clone(), way.to_string(), kind.to_string(), cell(&found), cell(src)]);
        }
    }
    c
}

/// Fails with the count of each kind when any drawing is not a pass.
fn assert_all_pass(c: &Counted) {
    let failed: Vec<String> = c.rows.iter().filter(|r| r[2] != "pass").map(|r| format!("{} {} {}: {}", r[0], r[1], r[2], r[3])).collect();
    let counts: Vec<String> = KINDS.iter().zip(&c.kinds).map(|(k, n)| format!("{k} {n}")).collect();
    assert!(
        failed.is_empty(),
        "{} of {} drawings failed; {}; line checks {:?}\n{}",
        failed.len(),
        c.rows.len(),
        counts.join(", "),
        c.checks,
        failed.join("\n")
    );
}

/// A folder named by an environment variable, taken from the workspace root when relative.
fn folder_from(var: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(var).map(|d| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(d))
}

// Fails if the generator makes a different number of definitions or makes one twice, or a named
// input is listed twice, is also generated, or stops before it is drawn.
#[test]
fn the_mixed_generator_keeps_its_count() {
    let defs = generated_mixed();
    assert_eq!(defs.len(), MIXED_DEFINITIONS);
    for (i, s) in defs.iter().enumerate() {
        assert!(!defs[..i].contains(s), "#{i} twice:\n{s}");
    }
    let reps = mixed_representatives();
    assert_eq!(reps.len(), REPRESENTATIVES);
    let r = crate::test_inputs::rules_with_test_links();
    for (i, (n, s)) in reps.iter().enumerate() {
        assert!(!reps[..i].iter().any(|(m, t)| m == n || t == s), "{n} twice");
        assert!(!defs.contains(s), "{n} is also generated");
        let (m, d) = crate::inference::infer(&crate::def::parse(s).unwrap(), &r);
        assert!(m.is_some() && d.errors.is_empty(), "{n}: {:?}", d.errors);
    }
}

// Fails if any generated or named definition mixing serverless parts with VPC parts gives no
// model, fails to lay out, panics, loses or repeats a line while gathering or drawing, breaks a
// line, or draws a line between two parts outside the VPC through it, along either axis.
// A pass is judged by the layout's own checks, the checks on gathered and drawn lines (which run
// only on the inputs a diagram was drawn for) and the router's notes. The diagram checker is run
// apart: it starts draw.io for each diagram, so it is run only on the eighty diagrams
// `mixed_diagrams_for_the_checker` writes, not on every diagram. With ARCHWIRE_MIXED_OUT=<dir>,
// writes every drawn diagram there as `<stem>.drawio` beside `<stem>.yaml`, `<stem>` being the
// input's name with `#` made `-`, then `-right` or `-down`, to look at one or check a few with
// `python3 tools/geocheck.py <dir>/<stem>.drawio`, one at a time. With ARCHWIRE_MIXED_REPORT=<file>, writes one
// tab-separated row per drawing there, and the drawings check 8 found anything in, whatever their
// kind, to `through-vpc.tsv` beside it. Both are taken from the workspace root when relative.
// It takes minutes, so it runs only when named:
//     cargo test -p archwire-core generated_mixed_definitions_all_draw_cleanly -- --ignored
#[test]
#[ignore]
fn generated_mixed_definitions_all_draw_cleanly() {
    let inputs = all_inputs();
    assert_eq!(inputs.len(), MIXED_DEFINITIONS + REPRESENTATIVES);
    let out = folder_from("ARCHWIRE_MIXED_OUT");
    let c = count(&inputs, out.as_deref());
    assert_eq!(c.rows.len(), 2 * inputs.len());
    if let Some(path) = folder_from("ARCHWIRE_MIXED_REPORT") {
        let mut text = String::from("input\taxis\tkind\tfound\tdefinition\n");
        for r in &c.rows {
            text.push_str(&r.join("\t"));
            text.push('\n');
        }
        std::fs::write(&path, text).unwrap();
        let mut text = String::from("input\taxis\tkind\tfound\n");
        for t in &c.through {
            text.push_str(t);
            text.push('\n');
        }
        std::fs::write(path.with_file_name("through-vpc.tsv"), text).unwrap();
    }
    assert_all_pass(&c);
}

// Fails if any generated or named mixed definition with two zones gives no model, fails to lay
// out, panics, loses or repeats a line, breaks a line or draws a line outside the VPC through it,
// along either axis. A pass is judged as in `generated_mixed_definitions_all_draw_cleanly`,
// without the diagram checker. Some still break, so it runs only when named:
//     cargo test -p archwire-core generated_mixed_two_zone_definitions_draw_cleanly -- --ignored
#[test]
#[ignore]
fn generated_mixed_two_zone_definitions_draw_cleanly() {
    let inputs: Vec<(String, String)> = all_inputs().into_iter().filter(|(_, s)| !s.contains("  azs: 3\n")).collect();
    assert_all_pass(&count(&inputs, None));
}

/// How many inputs `mixed_diagrams_for_the_checker` draws: the twenty named ones of
/// `REPRESENTATIVES_FOR_THE_CHECKER` and the twenty generated ones of `GENERATED_FOR_THE_CHECKER`,
/// each along both axes.
const MIXED_FOR_THE_CHECKER: usize = 40;

/// The generated definitions drawn for the diagram checker, by number: 14, 6 and 227 by name (an
/// S3 bucket, a CloudWatch beside a function outside the VPC, and a long Japanese label), then
/// seventeen spread over the list, taking the Japanese one of a pair at every other place.
/// Ten are in English and ten in Japanese.
const GENERATED_FOR_THE_CHECKER: [usize; 20] = [14, 6, 227, 1, 48, 99, 148, 197, 246, 297, 344, 395, 444, 495, 542, 593, 642, 691, 740, 791];

// Fails if an input handed to the checker gives no model, a named one picked is missing, picked
// twice or is `partial-senders`, the generated ones picked change or lose their mix of
// languages, or fewer definitions or diagrams are written than picked.
/// With `ARCHWIRE_GEN_OUT=<dir>`, removes the `m<number>-<axis>` files a run left there, then
/// writes each picked input drawn along both axes as `<dir>/m<number>-<axis>.yaml` (the
/// definition with its direction written) and, when a diagram was drawn,
/// `<dir>/m<number>-<axis>.drawio` and `<dir>/m<number>-<axis>.json` (the diagnostics with the
/// router's notes). Without it, only infers them.
#[test]
fn mixed_diagrams_for_the_checker() {
    let named = mixed_representatives();
    assert!(!REPRESENTATIVES_FOR_THE_CHECKER.contains(&"partial-senders"));
    for (i, n) in REPRESENTATIVES_FOR_THE_CHECKER.iter().enumerate() {
        assert!(!REPRESENTATIVES_FOR_THE_CHECKER[..i].contains(n), "{n} twice");
    }
    let mut picked: Vec<String> = REPRESENTATIVES_FOR_THE_CHECKER
        .iter()
        .map(|n| named.iter().find(|(m, _)| m == n).unwrap_or_else(|| panic!("no named input {n}")).1.clone())
        .collect();
    let defs = generated_mixed();
    let generated: Vec<&String> = GENERATED_FOR_THE_CHECKER.iter().map(|&k| &defs[k]).collect();
    let ja = generated.iter().filter(|s| s.contains("  lang: ja\n")).count();
    let en = generated.iter().filter(|s| s.contains("  lang: en\n")).count();
    assert_eq!((ja, en), (10, 10));
    for (i, k) in GENERATED_FOR_THE_CHECKER.iter().enumerate() {
        assert!(!GENERATED_FOR_THE_CHECKER[..i].contains(k), "#{k} twice");
    }
    picked.extend(generated.into_iter().cloned());
    assert_eq!(picked.len(), MIXED_FOR_THE_CHECKER);
    let r = crate::test_inputs::rules_with_test_links();
    for src in &picked {
        let (m, d): (Option<Model>, Diagnostics) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
        assert!(m.is_some() && d.errors.is_empty(), "{:?}\n{src}", d.errors);
    }
    let Some(dir) = folder_from("ARCHWIRE_GEN_OUT") else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let ours = |stem: &str| {
        let b = stem.as_bytes();
        b.len() > 5 && b[0] == b'm' && b[1..4].iter().all(u8::is_ascii_digit) && (stem.ends_with("-right") || stem.ends_with("-down"))
    };
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ours(stem) && ["yaml", "drawio", "json"].contains(&ext) {
            std::fs::remove_file(&path).unwrap();
        }
    }
    let (mut yamls, mut drawn) = (0, 0);
    for (k, src) in picked.iter().enumerate() {
        for (axis, way) in AXES {
            let name = format!("m{:03}-{way}", k + 1);
            let s = with_direction(src, way);
            std::fs::write(dir.join(format!("{name}.yaml")), &s).unwrap();
            yamls += 1;
            if let Drawn::Drawn { xml, json, .. } = draw_counted(&s, axis) {
                std::fs::write(dir.join(format!("{name}.drawio")), xml).unwrap();
                std::fs::write(dir.join(format!("{name}.json")), json).unwrap();
                drawn += 1;
            }
        }
    }
    assert_eq!((yamls, drawn), (2 * MIXED_FOR_THE_CHECKER, 2 * MIXED_FOR_THE_CHECKER));
}

/// The corners of a drawn path: its inner points where the way turns, after dropping repeated
/// points and points a straight run passes through.
fn corners(path: &[(i32, i32)]) -> Vec<((i32, i32), [(i32, i32); 2])> {
    let mut p: Vec<(i32, i32)> = Vec::new();
    for &q in path {
        if p.last() != Some(&q) {
            p.push(q);
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for k in 1..p.len().saturating_sub(1) {
            let (a, b, c) = (p[k - 1], p[k], p[k + 1]);
            if (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1) {
                p.remove(k);
                changed = true;
                break;
            }
        }
    }
    let dir = |from: (i32, i32), to: (i32, i32)| ((to.0 - from.0).signum(), (to.1 - from.1).signum());
    (1..p.len().saturating_sub(1)).map(|k| (p[k], [dir(p[k], p[k - 1]), dir(p[k], p[k + 1])])).collect()
}

/// The straight runs of a drawn path, without repeated points.
fn runs_of(path: &[(i32, i32)]) -> Vec<((i32, i32), (i32, i32))> {
    path.windows(2).filter(|s| s[0] != s[1]).map(|s| (s[0], s[1])).collect()
}

/// Two kinds of meeting between two drawn lines, each as rows "a | b | point | look | shared ends":
/// both lines turning at one point, and a line turning on the inside of another's straight run.
fn meetings(m: &Model, layout: &Layout, wires: &[Wire]) -> (Vec<String>, Vec<String>) {
    let paths: Vec<Vec<(i32, i32)>> = wires.iter().map(|w| crate::audit::drawn_path(w, layout)).collect();
    let cs: Vec<_> = paths.iter().map(|p| corners(p)).collect();
    let _ = m;
    let (mut same, mut tee) = (Vec::new(), Vec::new());
    for a in 0..wires.len() {
        for b in 0..wires.len() {
            if a == b {
                continue;
            }
            let (wa, wb) = (&wires[a], &wires[b]);
            let shared = [&wa.from, &wa.to].iter().any(|e| *e == &wb.from || *e == &wb.to);
            if a < b {
                for (pa, arms_a) in &cs[a] {
                    for (pb, arms_b) in &cs[b] {
                        if pa != pb {
                            continue;
                        }
                        let same_arm = arms_a.iter().filter(|x| arms_b.contains(x)).count();
                        let opposite = arms_a.iter().filter(|x| arms_b.contains(&(-x.0, -x.1))).count();
                        let look = match (same_arm, opposite) {
                            (0, 2) => "cross",
                            (1, 1) => "one_arm_along",
                            (2, 0) => "same_corner",
                            _ => "other",
                        };
                        same.push(format!("{}\t{}\t{:?}\t{look}\t{shared}", wa.id, wb.id, pa));
                    }
                }
            }
            if shared {
                continue;
            }
            let bends_b: Vec<(i32, i32)> = cs[b].iter().map(|c| c.0).collect();
            for (pa, arms_a) in &cs[a] {
                for (s, e) in runs_of(&paths[b]) {
                    let inside = if s.0 == e.0 && pa.0 == s.0 {
                        pa.1 > s.1.min(e.1) && pa.1 < s.1.max(e.1)
                    } else if s.1 == e.1 && pa.1 == s.1 {
                        pa.0 > s.0.min(e.0) && pa.0 < s.0.max(e.0)
                    } else {
                        false
                    };
                    if inside && !bends_b.contains(pa) {
                        let along = arms_a.iter().any(|x| if s.0 == e.0 { x.0 == 0 } else { x.1 == 0 });
                        tee.push(format!("{}\t{}\t{:?}\t{}\tfalse", wa.id, wb.id, pa, if along { "arm_along_run" } else { "other" }));
                    }
                }
            }
        }
    }
    (same, tee)
}

/// Lays out and routes a model the way `compile` does along `axis`.
fn routed(m: &Model, axis: Axis) -> Option<(Layout, Vec<Wire>)> {
    let r = Rules::builtin();
    let p = super::arrange(m, &r, axis).ok()?;
    let (wires, _) = crate::route::route_planned(m, &p.layout, &r, &p.lines);
    Some((p.layout, wires))
}

fn model_of(src: &str) -> Option<Model> {
    let r = Rules::builtin();
    let (m, d) = crate::inference::infer(&crate::def::parse(src).ok()?, &r);
    if d.errors.is_empty() { m } else { None }
}

// Counts two meetings of drawn lines over every set of inputs and writes them to the file named
// by ARCHWIRE_MEET_OUT. It runs only when named.
#[test]
#[ignore]
fn count_lines_meeting_at_corners() {
    let Some(path) = folder_from("ARCHWIRE_MEET_OUT") else { return };
    let mut sets: Vec<(&str, String, Model, Axis)> = Vec::new();
    for (name, src) in all_inputs() {
        for (axis, way) in AXES {
            if let Some(m) = model_of(&src) {
                sets.push(("mixed", format!("{name} {way} azs={}", m.azs.len()), m, axis));
            }
        }
    }
    for (k, src) in crate::test_inputs::generated_with_a_vpc().into_iter().enumerate() {
        for (axis, way) in AXES {
            if let Some(m) = model_of(&src) {
                sets.push(("vpc2016", format!("generated_with_a_vpc#{k} {way} azs={}", m.azs.len()), m, axis));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let hand: Vec<(String, Model)> = super::azgrid::tests::inputs().into_iter().filter(|(n, _, _)| n.starts_with("mix-")).map(|(n, m, _)| (n, m)).collect();
    let mut eighty: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("out/view/m2b/gen-before")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "yaml")).collect();
    eighty.sort();
    for f in eighty {
        let src = std::fs::read_to_string(&f).unwrap();
        let stem = f.file_stem().unwrap().to_str().unwrap().to_string();
        let axis = if stem.ends_with("-down") { Axis::Down } else { Axis::Right };
        let m = match src.strip_prefix("# hand-built model: ") {
            Some(n) => hand.iter().find(|(h, _)| h == n.trim()).expect("a hand-built model").1.clone(),
            None => model_of(&src).expect("a model"),
        };
        sets.push(("eighty", format!("{stem} azs={}", m.azs.len()), m, axis));
    }
    let mut cases: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("tests/cases")).unwrap().map(|e| e.unwrap().path()).filter(|p| !p.file_name().unwrap().to_str().unwrap().starts_with("err-")).collect();
    cases.sort();
    for c in cases {
        let src = std::fs::read_to_string(c.join("def.yaml")).unwrap();
        let m = model_of(&src).expect("a case gives a model");
        let axis = m.direction.unwrap_or_else(|| super::default_axis(&m));
        sets.push(("cases", format!("{} azs={}", c.file_name().unwrap().to_str().unwrap(), m.azs.len()), m, axis));
    }
    let mut out = String::from("set\tdrawing\tkind\ta\tb\tpoint\tlook\tshared_ends\n");
    let mut drawn: Vec<(&str, usize)> = Vec::new();
    for (set, name, m, axis) in &sets {
        let Some((layout, wires)) = routed(m, *axis) else { continue };
        match drawn.iter_mut().find(|d| d.0 == *set) {
            Some(d) => d.1 += 1,
            None => drawn.push((set, 1)),
        }
        let (same, tee) = meetings(m, &layout, &wires);
        for r in same {
            out.push_str(&format!("{set}\t{name}\tsame_corner_point\t{r}\n"));
        }
        for r in tee {
            out.push_str(&format!("{set}\t{name}\tcorner_on_run\t{r}\n"));
        }
    }
    std::fs::write(&path, out).unwrap();
    std::fs::write(path.with_file_name("meet-drawn.tsv"), drawn.iter().map(|(s, n)| format!("{s}\t{n}\n")).collect::<String>()).unwrap();
}


/// Every line's label lying just below or above a part's name, the two sharing some width, as
/// rows "line | label | part | own | side | gap | label box | name box": `own` when the part is an
/// end of the line, `side` below or above, `gap` the height between the two boxes, up to `most`.
fn labels_by_names(layout: &Layout, wires: &[Wire], most: i32) -> Vec<String> {
    let mut out = Vec::new();
    for w in wires {
        let path = crate::audit::drawn_path(w, layout);
        let Some(l) = crate::audit::audit_wire_label(w, &path) else { continue };
        for i in &layout.icons {
            let n = crate::measure::name_area(i);
            if !(l.x < n.x + n.w && n.x < l.x + l.w) {
                continue;
            }
            let (side, gap) = if l.y >= n.y + n.h {
                ("below", l.y - (n.y + n.h))
            } else if l.y + l.h <= n.y {
                ("above", n.y - (l.y + l.h))
            } else {
                continue;
            };
            if gap > most {
                continue;
            }
            let own = [&w.from, &w.to].iter().any(|e| **e == i.id) || w.represents.iter().any(|(a, b)| *a == i.id || *b == i.id);
            let label = w.label.replace('\n', " ");
            out.push(format!("{}\t{label}\t{}\t{own}\t{side}\t{gap}\t{:?}\t{:?}", w.id, i.id, (l.x, l.y, l.w, l.h), (n.x, n.y, n.w, n.h)));
        }
    }
    out
}

// Writes, for every set of inputs, each line's label lying close below or above a part's name
// to the file named by ARCHWIRE_LABEL_NAME_OUT, and the drawings counted per set beside it. It
// runs only when named.
#[test]
#[ignore]
fn count_labels_by_names() {
    let Some(path) = folder_from("ARCHWIRE_LABEL_NAME_OUT") else { return };
    let mut sets: Vec<(&str, String, Model, Axis)> = Vec::new();
    for (name, src) in all_inputs() {
        for (axis, way) in AXES {
            if let Some(m) = model_of(&src) {
                sets.push(("mixed", format!("{name} {way}"), m, axis));
            }
        }
    }
    for (k, src) in crate::test_inputs::generated_with_a_vpc().into_iter().enumerate() {
        for (axis, way) in AXES {
            if let Some(m) = model_of(&src) {
                sets.push(("vpc2016", format!("generated_with_a_vpc#{k} {way}"), m, axis));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let hand: Vec<(String, Model)> = super::azgrid::tests::inputs().into_iter().filter(|(n, _, _)| n.starts_with("mix-")).map(|(n, m, _)| (n, m)).collect();
    let mut eighty: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("out/view/m2b/gen-before")).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "yaml")).collect();
    eighty.sort();
    for f in eighty {
        let src = std::fs::read_to_string(&f).unwrap();
        let stem = f.file_stem().unwrap().to_str().unwrap().to_string();
        let axis = if stem.ends_with("-down") { Axis::Down } else { Axis::Right };
        let m = match src.strip_prefix("# hand-built model: ") {
            Some(n) => hand.iter().find(|(h, _)| h == n.trim()).expect("a hand-built model").1.clone(),
            None => model_of(&src).expect("a model"),
        };
        sets.push(("eighty", stem, m, axis));
    }
    let mut cases: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("tests/cases")).unwrap().map(|e| e.unwrap().path()).filter(|p| !p.file_name().unwrap().to_str().unwrap().starts_with("err-")).collect();
    cases.sort();
    for c in cases {
        let src = std::fs::read_to_string(c.join("def.yaml")).unwrap();
        let m = model_of(&src).expect("a case gives a model");
        let axis = m.direction.unwrap_or_else(|| super::default_axis(&m));
        sets.push(("cases", c.file_name().unwrap().to_str().unwrap().to_string(), m, axis));
    }
    let mut out = String::from("set\tdrawing\tline\tlabel\tpart\town\tside\tgap\tlabel_box\tname_box\n");
    let mut drawn: Vec<(&str, usize)> = Vec::new();
    for (set, name, m, axis) in &sets {
        let Some((layout, wires)) = routed(m, *axis) else { continue };
        match drawn.iter_mut().find(|d| d.0 == *set) {
            Some(d) => d.1 += 1,
            None => drawn.push((set, 1)),
        }
        for r in labels_by_names(&layout, &wires, 40) {
            out.push_str(&format!("{set}\t{name}\t{r}\n"));
        }
    }
    std::fs::write(&path, out).unwrap();
    std::fs::write(path.with_file_name("label-name-drawn.tsv"), drawn.iter().map(|(s, n)| format!("{s}\t{n}\n")).collect::<String>()).unwrap();
}

// Fails if two drawn lines that share no end turn at one point: drawn, the first half of one and
// the second half of the other read as one line. The generated definitions 969 and 1031 (left to
// right) and the named mixed definition (top to bottom) each had two such lines.
#[test]
fn lines_sharing_no_end_never_turn_at_one_point() {
    let gen = crate::test_inputs::generated();
    let named = mixed_representatives().into_iter().find(|(n, _)| *n == "function-before-web-3az").expect("a named definition").1;
    let inputs = [(with_direction(&gen[969], "right"), Axis::Right), (with_direction(&gen[1031], "right"), Axis::Right), (named, Axis::Down)];
    for (src, axis) in inputs {
        let m = model_of(&src).expect("a model");
        let (layout, wires) = routed(&m, axis).expect("laid out");
        let (same, _) = meetings(&m, &layout, &wires);
        let apart: Vec<&String> = same.iter().filter(|r| r.ends_with("\tfalse")).collect();
        assert!(apart.is_empty(), "{axis:?}: {apart:?}\n{src}");
    }
}

// Fails if the check on loops of main flows, or on main flows back into the subnets of the VPC,
// stops a definition that is drawn: a fixed case, a
// generated definition with a VPC, mixed with one or without one. Each set is counted, so a set
// that shrinks by accident fails too. Loops inside the subnets of the VPC, such as `ec2 -> aurora`
// with `aurora -> ec2`, are among them; no accepted pair closes one, so they are read with the
// test rules, and the built-in rules stop them as connections not in the list.
#[test]
fn the_loop_check_stops_no_drawn_definition() {
    let r = crate::test_inputs::rules_with_test_links();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases");
    let mut cases: Vec<String> = Vec::new();
    let mut dirs: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.sort();
    for dir in dirs {
        if dir.join("expected.drawio").exists() {
            cases.push(std::fs::read_to_string(dir.join("def.yaml")).unwrap());
        }
    }
    let sets = [
        ("fixed cases", cases),
        ("with a VPC", crate::test_inputs::generated_with_a_vpc()),
        ("mixed", all_inputs().into_iter().map(|(_, src)| src).collect()),
        ("without a VPC", crate::test_inputs::generated()),
        (
            "loops inside the subnets",
            vec![
                crate::test_inputs::FEWER_READERS.to_string(),
                FEWER_READERS_MIX.to_string(),
                "flows:\n  - ec2 -> aurora\n  - aurora -> ec2\nset:\n  azs: 3\n".to_string(),
                "flows:\n  - user -> alb -> ecs -> aurora\n  - aurora -> ecs\n".to_string(),
            ],
        ),
    ];
    let mut counts: Vec<(&str, usize, usize)> = Vec::new();
    for (name, defs) in &sets {
        let mut looped = 0;
        let mut stopped = Vec::new();
        for src in defs {
            let def = crate::def::parse(src).unwrap();
            if crate::inference::main_flow_loop_inside_subnets(&def, &r) {
                looped += 1;
            }
            let (_, d) = crate::inference::infer(&def, &r);
            if d.errors.iter().any(|e| e.message.contains("closes a loop of main flows") || e.message.contains("back into them")) {
                stopped.push(src.clone());
            }
        }
        assert!(stopped.is_empty(), "{name}: {} stopped as loops or as flows back into the subnets:\n{}", stopped.len(), stopped.join("\n"));
        counts.push((name, defs.len(), looped));
    }
    // The fixed cases grow with every shape that is added, so their number is not held here.
    let (name, fixed, looped) = counts.remove(0);
    assert!(name == "fixed cases" && fixed > 0 && looped == 0, "{name}: {fixed} definitions, {looped} with a loop inside the subnets");
    assert_eq!(counts, [("with a VPC", 1008, 0), ("mixed", 865, 1), ("without a VPC", 1212, 0), ("loops inside the subnets", 4, 4)]);
    let builtin = Rules::builtin();
    for src in &sets[4].1 {
        let def = crate::def::parse(src).unwrap();
        assert!(!crate::inference::main_flow_loop_inside_subnets(&def, &builtin), "{src}");
        let (m, d) = crate::inference::infer(&def, &builtin);
        assert!(m.is_none() && d.errors.iter().all(|e| e.code == "UNSUPPORTED" && e.message.ends_with("is not a supported connection")), "{src}{:?}", d.errors);
    }
}
