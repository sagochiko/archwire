//! Definitions mixing serverless parts with VPC parts: made by a generator along fixed axes, and
//! named by hand for shapes the generator does not make. Every one is drawn along both axes, and
//! each drawing is kept apart at the stage where it stops, so no failure hides another.

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::diag::{Diagnostics, Problem};
use crate::lines::Line;
use crate::model::{Axis, Model};
use crate::route::{LinePlan, Wire};
use crate::rules::Rules;
use crate::test_inputs::{ID_CLASH, MIX_LOGS, MIX_NAT, MIX_SMALLEST, MIX_WEB, ONE_FUNCTION_THREE_WAYS, TWIN_APPS};

/// How many definitions `generated_mixed` makes.
pub(in crate::arrange) const MIXED_DEFINITIONS: usize = 840;

/// How many named inputs `mixed_representatives` gives.
const REPRESENTATIVES: usize = 20;

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
    /// A diagram, the router's notes, and what the checks on the gathered and drawn lines found.
    Drawn { xml: String, notes: Vec<String>, lines_broken: Vec<String> },
}

/// Runs `f`, turning a panic into the stage's name and the panic message.
fn guarded<T>(stage: &str, f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
        format!("{stage}: {msg}")
    })
}

/// Parses, infers with the one inference, lays out along `axis`, gathers the lines, routes them
/// from the placement's drafts and writes the diagram, keeping what each stage gave.
pub(in crate::arrange) fn draw_counted(src: &str, axis: Axis) -> Drawn {
    let r = Rules::builtin();
    let def = match crate::def::parse(src) {
        Ok(d) => d,
        Err(e) => return Drawn::Errors(e),
    };
    let (model, mut diag) = match guarded("infer", || crate::inference::infer(&def, &r)) {
        Ok(x) => x,
        Err(p) => return Drawn::Panicked(p),
    };
    let Some(model) = model else { return Drawn::Errors(diag.errors) };
    if !diag.errors.is_empty() {
        return Drawn::Errors(diag.errors);
    }
    let placement = match guarded("arrange", || super::arrange(&model, &r, axis)) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return Drawn::LayoutFailed(e),
        Err(p) => return Drawn::Panicked(p),
    };
    let lines = match guarded("gather", || crate::lines::gather(&model, &placement.layout, &r).0) {
        Ok(l) => l,
        Err(p) => return Drawn::Panicked(p),
    };
    let (wires, notes) = match guarded("route", || crate::route::route_planned(&model, &placement.layout, &r, &placement.lines)) {
        Ok(x) => x,
        Err(p) => return Drawn::Panicked(p),
    };
    diag.notes.extend(notes.iter().cloned());
    let xml = match guarded("write", || crate::drawio::write(&model, &placement.layout, &wires, &diag, &r, false)) {
        Ok(x) => x,
        Err(p) => return Drawn::Panicked(p),
    };
    let lines_broken = line_checks(&model, &lines, &placement.lines, &wires);
    Drawn::Drawn { xml, notes, lines_broken }
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
        for &k in &l.edges {
            let e = &m.edges[k];
            if e.label != l.label_key || e.hole != l.hole || e.endpoint != l.endpoint {
                found.push(format!("3: {name} holds edge {k} {}->{} with label {:?}, hole {:?}, endpoint {}", e.from, e.to, e.label, e.hole, e.endpoint));
            }
        }
        let mut want: Vec<(String, Vec<usize>)> = Vec::new();
        for &k in &l.edges {
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

/// The kinds a drawing is counted as, in the order they are reported.
const KINDS: [&str; 6] = ["errors", "layout_failed", "panicked", "lines_broken", "broken", "pass"];

/// One cell of a report: no tab or line break.
fn cell(s: &str) -> String {
    s.replace('\t', " ").replace('\n', "\\n")
}

fn problems(p: &[Problem]) -> String {
    p.iter().map(|p| format!("{} {}: {}", p.code, p.at.as_deref().unwrap_or("-"), p.message)).collect::<Vec<_>>().join(" | ")
}

/// Draws every input along both axes and returns one row per drawing (name, axis, kind, what
/// was found, the definition) and the count of each kind and of each line check.
fn count(inputs: &[(String, String)]) -> (Vec<[String; 5]>, Vec<usize>, [usize; 7]) {
    let mut rows = Vec::new();
    let mut kinds = vec![0; KINDS.len()];
    let mut checks = [0; 7];
    for (name, src) in inputs {
        for (axis, way) in AXES {
            let (kind, found) = match draw_counted(src, axis) {
                Drawn::Errors(e) => ("errors", problems(&e)),
                Drawn::LayoutFailed(e) => ("layout_failed", problems(&e)),
                Drawn::Panicked(p) => ("panicked", p),
                Drawn::Drawn { notes, lines_broken, .. } => {
                    let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                    for f in &lines_broken {
                        if let Some(k) = f.split(':').next().and_then(|k| k.parse::<usize>().ok()).filter(|k| (1..=7).contains(k)) {
                            checks[k - 1] += 1;
                        }
                    }
                    let mut all: Vec<String> = lines_broken.clone();
                    all.extend(broken.iter().map(|n| n.to_string()));
                    match (lines_broken.is_empty(), broken.is_empty()) {
                        (false, _) => ("lines_broken", all.join(" | ")),
                        (true, false) => ("broken", all.join(" | ")),
                        (true, true) => ("pass", notes.join(" | ")),
                    }
                }
            };
            kinds[KINDS.iter().position(|k| *k == kind).expect("a listed kind")] += 1;
            rows.push([name.clone(), way.to_string(), kind.to_string(), cell(&found), cell(src)]);
        }
    }
    (rows, kinds, checks)
}

/// Fails with the count of each kind when any drawing is not a pass.
fn assert_all_pass(rows: &[[String; 5]], kinds: &[usize], checks: &[usize; 7]) {
    let failed: Vec<String> = rows.iter().filter(|r| r[2] != "pass").map(|r| format!("{} {} {}: {}", r[0], r[1], r[2], r[3])).collect();
    let counts: Vec<String> = KINDS.iter().zip(kinds).map(|(k, n)| format!("{k} {n}")).collect();
    assert!(
        failed.is_empty(),
        "{} of {} drawings failed; {}; line checks {checks:?}\n{}",
        failed.len(),
        rows.len(),
        counts.join(", "),
        failed.join("\n")
    );
}

// Fails if the generator makes a different number of definitions or makes one twice.
#[test]
fn the_mixed_generator_keeps_its_count() {
    let defs = generated_mixed();
    assert_eq!(defs.len(), MIXED_DEFINITIONS);
    for (i, s) in defs.iter().enumerate() {
        assert!(!defs[..i].contains(s), "#{i} twice:\n{s}");
    }
    let reps = mixed_representatives();
    assert_eq!(reps.len(), REPRESENTATIVES);
    for (i, (n, s)) in reps.iter().enumerate() {
        assert!(!reps[..i].iter().any(|(m, t)| m == n || t == s), "{n} twice");
        assert!(!defs.contains(s), "{n} is also generated");
    }
}

// Fails if any generated or named definition mixing serverless parts with VPC parts gives no
// model, fails to lay out, panics, loses or repeats a line while gathering or drawing, or breaks
// a line, along either axis. With ARCHWIRE_MIXED_REPORT=<file> (relative to the workspace root),
// writes one tab-separated row per drawing there. It takes minutes, so it runs only when named:
//     cargo test -p archwire-core generated_mixed_definitions_all_draw_cleanly -- --ignored
#[test]
#[ignore]
fn generated_mixed_definitions_all_draw_cleanly() {
    let inputs = all_inputs();
    assert_eq!(inputs.len(), MIXED_DEFINITIONS + REPRESENTATIVES);
    let (rows, kinds, checks) = count(&inputs);
    assert_eq!(rows.len(), 2 * inputs.len());
    if let Some(file) = std::env::var_os("ARCHWIRE_MIXED_REPORT") {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(file);
        let mut text = String::from("input\taxis\tkind\tfound\tdefinition\n");
        for r in &rows {
            text.push_str(&r.join("\t"));
            text.push('\n');
        }
        std::fs::write(path, text).unwrap();
    }
    assert_all_pass(&rows, &kinds, &checks);
}

// Fails if any generated or named mixed definition with two zones gives no model, fails to lay
// out, panics, loses or repeats a line, or breaks a line, along either axis. Some still break,
// so it runs only when named:
//     cargo test -p archwire-core generated_mixed_two_zone_definitions_draw_cleanly -- --ignored
#[test]
#[ignore]
fn generated_mixed_two_zone_definitions_draw_cleanly() {
    let inputs: Vec<(String, String)> = all_inputs().into_iter().filter(|(_, s)| !s.contains("  azs: 3\n")).collect();
    let (rows, kinds, checks) = count(&inputs);
    assert_all_pass(&rows, &kinds, &checks);
}

/// How many inputs `mixed_diagrams_for_the_checker` draws: every named one and twenty taken
/// evenly from the generated ones, each along both axes.
const MIXED_FOR_THE_CHECKER: usize = 40;

// Fails if an input handed to the checker gives no model, or if fewer definitions are written than picked.
/// With `ARCHWIRE_GEN_OUT=<dir>`, removes the `m<number>-<axis>` files a run left there, then
/// writes each named input and twenty generated ones drawn along both axes as
/// `<dir>/m<number>-<axis>.yaml` (the definition with its direction written) and, when a diagram
/// was drawn, `<dir>/m<number>-<axis>.drawio`. Without it, only infers them.
#[test]
fn mixed_diagrams_for_the_checker() {
    let mut picked: Vec<String> = mixed_representatives().into_iter().map(|(_, s)| s).collect();
    let defs = generated_mixed();
    let n = MIXED_FOR_THE_CHECKER - picked.len();
    for k in 0..n {
        picked.push(defs[k * defs.len() / n].clone());
    }
    assert_eq!(picked.len(), MIXED_FOR_THE_CHECKER);
    let r = Rules::builtin();
    for src in &picked {
        let (m, d): (Option<Model>, Diagnostics) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
        assert!(m.is_some() && d.errors.is_empty(), "{:?}\n{src}", d.errors);
    }
    let Some(dir) = std::env::var_os("ARCHWIRE_GEN_OUT") else { return };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ours = |stem: &str| {
        let b = stem.as_bytes();
        b.len() > 5 && b[0] == b'm' && b[1..4].iter().all(u8::is_ascii_digit) && (stem.ends_with("-right") || stem.ends_with("-down"))
    };
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ours(stem) && ["yaml", "drawio"].contains(&ext) {
            std::fs::remove_file(&path).unwrap();
        }
    }
    let mut yamls = 0;
    for (k, src) in picked.iter().enumerate() {
        for (axis, way) in AXES {
            let name = format!("m{:03}-{way}", k + 1);
            let s = with_direction(src, way);
            std::fs::write(dir.join(format!("{name}.yaml")), &s).unwrap();
            yamls += 1;
            if let Drawn::Drawn { xml, .. } = draw_counted(&s, axis) {
                std::fs::write(dir.join(format!("{name}.drawio")), xml).unwrap();
            }
        }
    }
    assert_eq!(yamls, 2 * MIXED_FOR_THE_CHECKER);
}
