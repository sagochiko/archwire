//! Definitions read by more than one module's tests, kept in one place so that every test reading
//! a list reads the same definitions in the same order.

/// Two zones in Japanese with lines from ECS and Aurora to S3 and the side column: crowded, as in the routing tests.
pub(crate) const CROWDED_S3_JA: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront ~> cloudwatch\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - aurora ~> cloudwatch\n  - waf -- cloudfront\n  - acm -- cloudfront\n  - aurora -> s3\n  - alb ~> s3\nset:\n  azs: 2\n  lang: ja\n  nat: per-az\nchoose:\n  route.ecs.s3: endpoint\n  route.aurora.s3: nat\n";

/// Two zones in English with lines from every part to S3 and CloudWatch: crowded, as in the routing tests.
pub(crate) const CROWDED_SIDE_EN: &str = "flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - cloudfront -> s3\n  - cloudfront ~> cloudwatch\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n  - aurora ~> cloudwatch\n  - ecs -> internet\n  - waf -- cloudfront\n  - acm -- cloudfront\n  - user -> alb\n  - aurora -> s3\n  - alb ~> s3\n  - ecs ~> s3\nset:\n  azs: 2\n  lang: en\n  nat: per-az\nchoose:\n  route.aurora.s3: nat\n";

pub(crate) const WEB_CASES: [(&str, &str); 10] = [
    ("web-2az-ja", include_str!("../../../tests/cases/web-2az-ja/def.yaml")),
    ("web-2az-en", include_str!("../../../tests/cases/web-2az-en/def.yaml")),
    ("web-3az-ja", include_str!("../../../tests/cases/web-3az-ja/def.yaml")),
    ("web-3az-en", include_str!("../../../tests/cases/web-3az-en/def.yaml")),
    ("endpoint-3az-ja", include_str!("../../../tests/cases/endpoint-3az-ja/def.yaml")),
    ("endpoint-3az-en", include_str!("../../../tests/cases/endpoint-3az-en/def.yaml")),
    ("endpoint-ja", include_str!("../../../tests/cases/endpoint-ja/def.yaml")),
    ("hole-s3", include_str!("../../../tests/cases/hole-s3/def.yaml")),
    ("aux-to-side", include_str!("../../../tests/cases/aux-to-side/def.yaml")),
    ("web-attach-en", include_str!("../../../tests/cases/web-attach-en/def.yaml")),
];

/// Three zones and one Reader: Aurora has copies in the first two zones only, and lines run
/// back up from Aurora to ECS.
pub(crate) const FEWER_READERS: &str = "flows:\n  - user -> alb -> ecs -> aurora\n  - aurora -> ecs\n  - aurora -> s3\nset:\n  azs: 3\n  readers: 1\nchoose:\n  route.aurora.s3: endpoint\n";

/// Every web definition with a VPC the generator makes, in order: two or three zones, one NAT
/// gateway per zone or one in all, one Reader or as many as the zones allow, traffic out to
/// the internet or not, ECS's route to S3 left open or answered, the side line to CloudWatch
/// from the load balancer, ECS or Aurora, WAF and ACM on CloudFront or not, S3 reached from
/// CloudFront, from ECS or from both, and English or Japanese.
pub(crate) fn generated_with_a_vpc() -> Vec<String> {
    let mut out = Vec::new();
    for azs in [2, 3] {
        for nat in ["per-az", "single"] {
            for readers in [None, Some(1)].into_iter().filter(|r| azs == 3 || r.is_none()) {
                for outbound in [false, true] {
                    for route in [None, Some("endpoint"), Some("nat")] {
                        for aux in ["alb", "ecs", "aurora"] {
                            for attach in [false, true] {
                                for s3 in ["cloudfront", "ecs", "both"] {
                                    for ja in [false, true] {
                                        let mut src = String::from("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n");
                                        if s3 != "ecs" {
                                            src.push_str("  - cloudfront -> s3\n");
                                        }
                                        if s3 != "cloudfront" {
                                            src.push_str("  - ecs -> s3\n");
                                        }
                                        if outbound {
                                            src.push_str("  - ecs -> internet\n");
                                        }
                                        src.push_str(&format!("  - {aux} ~> cloudwatch\n"));
                                        if attach {
                                            src.push_str("  - waf -- cloudfront\n  - acm -- cloudfront\n");
                                        }
                                        src.push_str(&format!("set:\n  azs: {azs}\n  nat: {nat}\n"));
                                        if let Some(r) = readers {
                                            src.push_str(&format!("  readers: {r}\n"));
                                        }
                                        if ja {
                                            src.push_str("  lang: ja\n");
                                        }
                                        if let (Some(r), true) = (route, s3 != "cloudfront") {
                                            src.push_str(&format!("choose:\n  route.ecs.s3: {r}\n"));
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
    }
    out
}

/// Every definition the generator makes.
pub(crate) fn generated() -> Vec<String> {
    let mut out = Vec::new();
    let apis = ["apigateway(http)", "apigateway(rest)", "apigateway.public(http)"];
    // (CloudFront shape, attachments): 0 none; 1 web; 2 a and b; 3 one CloudFront into a Lambda's S3.
    let mut fronts: Vec<(usize, usize)> = vec![(0, 0)];
    for cf in 1..=3 {
        for att in 0..=3 {
            if att == 3 && cf != 2 {
                continue;
            }
            fronts.push((cf, att));
        }
    }
    let mut i = 0usize;
    for lambdas in 1..=3usize {
        for dest in 0..3 {
            for aux in [false, true] {
                for api in apis {
                    for cognito in [false, true] {
                        for &(cf, att) in &fronts {
                            out.push(definition(i, lambdas, dest, aux, api, cognito, cf, att));
                            i += 1;
                        }
                    }
                }
            }
        }
    }
    out.extend(through_cloudfront());
    out
}

/// Definitions where the API is reached through CloudFront (`user -> cloudfront ->
/// apigateway -> lambda -> s3`), with names as written or every name made of `m` alone at the
/// longest allowed.
pub(crate) fn through_cloudfront() -> Vec<String> {
    let mut out = Vec::new();
    for wide in [false, true] {
        let n = |svc: &str| if wide { format!("{svc}.{}", "m".repeat(crate::def::MAX_TAG)) } else { svc.to_string() };
        for api in ["http", "rest"] {
            for att in 0..3 {
                for cognito in [false, true] {
                    let mut src = format!("flows:\n  - user -> {} -> {}({api}) -> {} -> {}\n", n("cloudfront"), n("apigateway"), n("lambda"), n("s3"));
                    if att > 0 {
                        src.push_str(&format!("  - {} -- {}\n", n("waf"), n("cloudfront")));
                    }
                    if att > 1 {
                        src.push_str(&format!("  - {} -- {}\n", n("acm"), n("cloudfront")));
                    }
                    if cognito {
                        src.push_str(&format!("  - {} -- {}\n", n("cognito"), n("apigateway")));
                    }
                    src.push_str(&format!("  - {} ~> {}\n", n("lambda"), n("cloudwatch")));
                    if wide {
                        src.push_str("set:\n  lang: ja\n");
                    }
                    out.push(src);
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn definition(i: usize, lambdas: usize, dest: usize, aux: bool, api: &str, cognito: bool, cf: usize, att: usize) -> String {
    // Names: as written, or every name after the dot at the longest allowed, filled with `x`
    // or with `m`, the widest letter a name may hold. Side parts and global parts are named too
    // then. Taken from the number apart from the settings, which go by `i % 3`.
    let fill = match (i / 3) % 3 {
        0 => None,
        1 => Some("x"),
        _ => Some("m"),
    };
    let tag = |t: &str| match fill {
        Some(c) => format!("{t}{}", c.repeat(crate::def::MAX_TAG - t.len())),
        None => t.to_string(),
    };
    // A part named only when names are long: `cloudwatch`, or `cloudwatch.l…` filled.
    let side = |svc: &str, t: &str| if fill.is_some() { format!("{svc}.{}", tag(t)) } else { svc.to_string() };
    let lambda = |k: usize| if lambdas == 1 { "lambda".to_string() } else { format!("lambda.{}", tag(&format!("f{k}"))) };
    let table = |t: &str| format!("dynamodb.{}", tag(t));
    let api_name = api.split('(').next().unwrap();
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("user -> {api} -> {}", lambda(1)));
    for k in 2..=lambdas {
        lines.push(format!("{api_name} -> {}", lambda(k)));
    }
    match dest {
        0 => (1..=lambdas).for_each(|k| lines.push(format!("{} -> {}", lambda(k), table(&format!("t{k}"))))),
        1 => (1..=lambdas).for_each(|k| {
            lines.push(format!("{} -> {}", lambda(k), table("x")));
            lines.push(format!("{} -> {}", lambda(k), table("y")));
        }),
        _ => (1..=lambdas).rev().for_each(|k| {
            lines.push(format!("{} -> {}", lambda(k), table("x")));
            lines.push(format!("{} -> {}", lambda(k), table("y")));
        }),
    }
    let cloudwatch = side("cloudwatch", "l");
    if aux {
        (1..=lambdas).for_each(|k| lines.push(format!("{} ~> {cloudwatch}", lambda(k))));
        lines.push(format!("{api_name} ~> {cloudwatch}"));
    }
    if cognito {
        lines.push(format!("{} -- {api_name}", side("cognito", "g")));
    }
    // Each distribution's own letter: "" for the only one, "a" and "b" for two.
    let fronts: Vec<&str> = if cf == 2 { vec!["a", "b"] } else if cf > 0 { vec![""] } else { vec![] };
    // `svc` for the only one with short names, else `svc.<letter>` with the letter filled out.
    let named = |svc: &str, f: &str, alone: &str| match (f.is_empty(), fill.is_some()) {
        (true, false) => svc.to_string(),
        (true, true) => format!("{svc}.{}", tag(alone)),
        (false, _) => format!("{svc}.{}", tag(f)),
    };
    let front = |f: &str| named("cloudfront", f, "c");
    match cf {
        1 => lines.push(format!("user -> {} -> s3.web", front(""))),
        2 => {
            lines.push(format!("user -> {} -> s3.a", front("a")));
            lines.push(format!("user -> {} -> s3.b", front("b")));
        }
        3 => {
            lines.push(format!("user -> {} -> s3.web", front("")));
            lines.push(format!("{} -> s3.web", lambda(1)));
        }
        _ => {}
    }
    match att {
        1 | 2 => {
            for f in &fronts {
                lines.push(format!("{} -- {}", named("waf", f, "w"), front(f)));
                if att == 2 {
                    lines.push(format!("{} -- {}", named("acm", f, "k"), front(f)));
                }
            }
        }
        3 => fronts.iter().for_each(|f| lines.push(format!("{} -- {}", side("waf", "w"), front(f)))),
        _ => {}
    }
    let mut src = String::from("flows:\n");
    for l in lines {
        src.push_str(&format!("  - {l}\n"));
    }
    match i % 3 {
        1 => src.push_str("set:\n  lang: ja\n"),
        2 => src.push_str("set:\n  lang: ja\n  region: ap-northeast-1\n"),
        _ => {}
    }
    src
}

/// The definitions of the fixed examples of the API shape that give a diagram.
pub(crate) const API_CASES: [(&str, &str); 7] = [
    ("api-http-ja", include_str!("../../../tests/cases/api-http-ja/def.yaml")),
    ("api-merged-ja", include_str!("../../../tests/cases/api-merged-ja/def.yaml")),
    ("api-min-en", include_str!("../../../tests/cases/api-min-en/def.yaml")),
    ("api-names-ja", include_str!("../../../tests/cases/api-names-ja/def.yaml")),
    ("api-rest-en", include_str!("../../../tests/cases/api-rest-en/def.yaml")),
    ("edge-waf-en", include_str!("../../../tests/cases/edge-waf-en/def.yaml")),
    ("hole-apigateway", include_str!("../../../tests/cases/hole-apigateway/def.yaml")),
];

/// A named function next to a part named like the zone copy of another function.
pub(crate) const ID_CLASH: &str = "flows:\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - apigateway -> lambda.api-1a -> dynamodb\n";
/// Two web stacks of named VPC parts, each behind a load balancer of its own.
pub(crate) const TWO_WEBS: &str = "flows:\n  - user -> cloudfront -> alb.web -> ecs.web -> aurora.web\n  - user -> alb.admin -> ecs.admin -> aurora.admin\n";
/// Two named services behind one unnamed load balancer.
pub(crate) const TWIN_APPS: &str = "flows:\n  - user -> cloudfront -> alb -> ecs.web -> aurora.web\n  - alb -> ecs.admin -> aurora.admin\n";
/// One function reaching a database in a VPC, a table and a bucket, in three line orders.
pub(crate) const ONE_FUNCTION_THREE_WAYS: [&str; 3] = [
    "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> dynamodb\n  - lambda -> s3\n",
    "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\n  - lambda -> dynamodb\n",
    "flows:\n  - lambda -> s3\n  - lambda -> dynamodb\n  - user -> apigateway(http) -> lambda -> aurora\n",
];
/// A load balancer with nothing behind it.
pub(crate) const LOAD_BALANCER_ALONE: &str = "flows:\n  - user -> alb\n";

/// The definitions of the mixed shapes the diagrams aim at. The smallest: one function behind an
/// HTTP API reaching a database in a VPC, a bucket and logs.
pub(crate) const MIX_SMALLEST: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\n  - lambda ~> cloudwatch\nset:\n  lang: ja\n";
/// The smallest with the function's route to the bucket going through a NAT gateway.
pub(crate) const MIX_NAT: &str = "flows:\n  - user -> apigateway(http) -> lambda -> aurora\n  - lambda -> s3\n  - lambda ~> cloudwatch\nset:\n  lang: ja\nchoose:\n  route.lambda.s3: nat\n";
/// A web stack behind CloudFront next to two functions behind API Gateway.
pub(crate) const MIX_WEB: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> dynamodb\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - user -> apigateway -> lambda.report -> dynamodb\n  - ecs ~> cloudwatch\n  - lambda.api ~> cloudwatch\nset:\n  lang: ja\n";
/// Two functions behind one API, one reaching a database in a VPC, both sending logs.
pub(crate) const MIX_LOGS: &str = "flows:\n  - user -> apigateway(http) -> lambda.api -> aurora\n  - apigateway -> lambda.report -> dynamodb\n  - lambda.api ~> cloudwatch\n  - lambda.report ~> cloudwatch\n";

#[cfg(test)]
mod tests {
    use super::*;

    // Fails if an input silently drops out of a shared list.
    #[test]
    fn the_shared_inputs_keep_their_counts() {
        assert_eq!(WEB_CASES.len(), 10);
        assert_eq!(generated_with_a_vpc().len(), 1008);
        assert_eq!(API_CASES.len(), 7);
        assert_eq!(generated().len(), GENERATED_COUNT);
        assert_eq!(ONE_FUNCTION_THREE_WAYS.len(), 3);
    }

    // Fails if a definition no longer parses or is listed twice in one list.
    #[test]
    fn the_shared_inputs_parse_and_appear_once() {
        let lists: Vec<(&str, Vec<String>)> = vec![
            ("CROWDED", vec![CROWDED_S3_JA.to_string(), CROWDED_SIDE_EN.to_string()]),
            ("WEB_CASES", WEB_CASES.iter().map(|(_, s)| s.to_string()).collect()),
            ("FEWER_READERS", vec![FEWER_READERS.to_string()]),
            ("generated_with_a_vpc", generated_with_a_vpc()),
            ("API_CASES", API_CASES.iter().map(|(_, s)| s.to_string()).collect()),
            ("generated", generated()),
            ("named and small shapes", vec![ID_CLASH.to_string(), TWO_WEBS.to_string(), TWIN_APPS.to_string(), LOAD_BALANCER_ALONE.to_string()]),
            ("ONE_FUNCTION_THREE_WAYS", ONE_FUNCTION_THREE_WAYS.iter().map(|s| s.to_string()).collect()),
            ("MIX", vec![MIX_SMALLEST.to_string(), MIX_NAT.to_string(), MIX_WEB.to_string(), MIX_LOGS.to_string()]),
        ];
        for (name, list) in &lists {
            for (i, src) in list.iter().enumerate() {
                assert!(crate::def::parse(src).is_ok(), "{name}[{i}] does not parse:\n{src}");
                assert!(!list[..i].contains(src), "{name}[{i}] is listed twice:\n{src}");
            }
        }
    }

    /// How many definitions `generated()` makes.
    const GENERATED_COUNT: usize = 1212;
}
