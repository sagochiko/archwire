//! Picks which layout a definition goes through: the VPC layout (Grid) or the layout without
//! a VPC (Flow). It is decided once, right after the definition is read, from the services data.

use crate::def::{Definition, Ref};
use crate::diag::Problem;
use crate::rules::{Place, Rules, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Grid,
    Flow,
}

/// Grid if any part is placed in a VPC (scope = vpc), Flow otherwise.
/// A definition that mixes VPC parts with `separate_from_vpc` parts stops with UNSUPPORTED.
/// Unknown names are not counted.
pub fn pick(def: &Definition, rules: &Rules) -> Result<Engine, Vec<Problem>> {
    let mut vpc: Option<&Ref> = None;
    let mut separate: Option<&Ref> = None;
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]) {
        let Some(s) = rules.service(&r.service) else { continue };
        if s.scope == Scope::Vpc && vpc.is_none() {
            vpc = Some(r);
        }
        if s.separate_from_vpc && separate.is_none() {
            separate = Some(r);
        }
    }
    match (vpc, separate) {
        (None, _) => Ok(Engine::Flow),
        (Some(_), None) => Ok(Engine::Grid),
        (Some(v), Some(s)) => Err(vec![Problem {
            code: "UNSUPPORTED".into(),
            at: Some(s.at.clone()),
            got: Some(s.service.clone()),
            message: format!(
                "`{}` cannot be drawn together with VPC services such as `{}`; this is not supported yet",
                s.service, v.service
            ),
            ..Default::default()
        }]),
    }
}

/// Stops a definition that writes two or more parts spanning the zones of a VPC (`scope = "vpc"`
/// and `place = "span"` in services.toml): one error per part after the first, at its first position.
pub fn spanning_parts(def: &Definition, rules: &Rules) -> Result<(), Vec<Problem>> {
    let mut seen: Vec<&str> = Vec::new();
    let mut errors = Vec::new();
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]) {
        let Some(s) = rules.service(&r.service) else { continue };
        if s.scope != Scope::Vpc || s.place != Place::Span || seen.contains(&r.name.as_str()) {
            continue;
        }
        if let Some(first) = seen.first() {
            errors.push(Problem {
                code: "UNSUPPORTED".into(),
                at: Some(r.at.clone()),
                got: Some(r.name.clone()),
                message: format!(
                    "`{}` is a second part spanning the Availability Zones, after `{first}`; this version draws one such part in a VPC",
                    r.name
                ),
                ..Default::default()
            });
        }
        seen.push(&r.name);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_inputs::TWO_WEBS;

    // Fails if the parts that span the zones are told apart by the service's name instead of the services data.
    #[test]
    fn the_spanning_check_reads_the_data() {
        let services = include_str!("../../../rules/aws/services.toml");
        let rules = include_str!("../../../rules/aws/rules.toml");
        let labels = include_str!("../../../rules/aws/labels.toml");
        let links = include_str!("../../../rules/aws/links.toml");
        assert!(services.contains("key = \"alb\"") && links.contains("= \"alb\""));
        let renamed = Rules::load(
            &services.replace("key = \"alb\"", "key = \"balancer\""),
            rules,
            labels,
            &links.replace("from = \"alb\"", "from = \"balancer\"").replace("to = \"alb\"", "to = \"balancer\""),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        let def = crate::def::parse(&TWO_WEBS.replace("alb.", "balancer.")).unwrap();
        let e = spanning_parts(&def, &renamed).unwrap_err();
        let got: Vec<_> = e.iter().map(|p| (p.code.as_str(), p.at.as_deref(), p.got.as_deref())).collect();
        assert_eq!(got, [("UNSUPPORTED", Some("flows[1][1]"), Some("balancer.admin"))]);
        assert_eq!(e[0].message, "`balancer.admin` is a second part spanning the Availability Zones, after `balancer.web`; this version draws one such part in a VPC");
    }

    // Fails if a third spanning part goes unreported, an unnamed part is not counted, a part written
    // again is reported twice, a part first written on a `--` or `~>` line is reported elsewhere,
    // or a single spanning part is stopped.
    #[test]
    fn spanning_parts_are_reported_once_each_at_their_first_position() {
        let cases: [(&str, &[(&str, &str, &str)]); 7] = [
            (
                "flows:\n  - user -> alb.a -> ecs\n  - user -> alb.b -> ecs\n  - user -> alb.c -> ecs\n",
                &[("flows[1][1]", "alb.b", "alb.a"), ("flows[2][1]", "alb.c", "alb.a")],
            ),
            ("flows:\n  - user -> alb -> ecs\n  - user -> alb.admin -> ecs.admin\n", &[("flows[1][1]", "alb.admin", "alb")]),
            ("flows:\n  - user -> alb.a -> ecs.a\n  - user -> alb.b -> ecs.b\n  - alb.b -> ecs.c\n", &[("flows[1][1]", "alb.b", "alb.a")]),
            ("flows:\n  - user -> alb.a -> ecs\n  - waf -- alb.b\n  - user -> alb.b -> ecs\n", &[("flows[1][1]", "alb.b", "alb.a")]),
            ("flows:\n  - user -> alb.a -> ecs\n  - alb.b ~> cloudwatch\n  - user -> alb.b -> ecs\n", &[("flows[1][0]", "alb.b", "alb.a")]),
            ("flows:\n  - user -> alb.a -> ecs.a\n  - alb.a -> ecs.b\n", &[]),
            ("flows:\n  - user -> alb.a -> ecs\n  - user -> albb.b -> ecs\n", &[]),
        ];
        for (src, want) in cases {
            let got = spanning_parts(&crate::def::parse(src).unwrap(), &Rules::builtin()).err().unwrap_or_default();
            let got: Vec<_> = got.iter().map(|p| (p.code.as_str(), p.at.clone().unwrap(), p.got.clone().unwrap(), p.message.clone())).collect();
            let want: Vec<_> = want
                .iter()
                .map(|(at, name, first)| {
                    let message = format!("`{name}` is a second part spanning the Availability Zones, after `{first}`; this version draws one such part in a VPC");
                    ("UNSUPPORTED", at.to_string(), name.to_string(), message)
                })
                .collect();
            assert_eq!(got, want, "{src}");
        }
    }

    fn pick_src(src: &str) -> Result<Engine, Vec<Problem>> {
        pick(&crate::def::parse(src).unwrap(), &Rules::builtin())
    }

    #[test]
    fn vpc_parts_pick_the_grid_and_the_rest_the_flow() {
        assert_eq!(pick_src("flows:\n  - user -> cloudfront -> alb -> ecs\n"), Ok(Engine::Grid));
        assert_eq!(pick_src("flows:\n  - user -> apigateway(http) -> lambda\n"), Ok(Engine::Flow));
        assert_eq!(pick_src("flows:\n  - user -> cloudfront -> s3\n"), Ok(Engine::Flow));
        assert_eq!(pick_src("flows:\n  - user -> cloudfrnt -> s3\n"), Ok(Engine::Flow));
    }

    #[test]
    fn mixing_vpc_parts_with_separate_services_stops() {
        let e = pick_src("flows:\n  - user -> apigateway(http) -> lambda -> aurora\n").unwrap_err();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref(), e[0].got.as_deref()), ("UNSUPPORTED", Some("flows[0][1]"), Some("apigateway")));
        assert!(e[0].message.contains("not supported yet"), "{}", e[0].message);
        let e = pick_src("flows:\n  - user -> alb -> ecs -> lambda\n").unwrap_err();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref(), e[0].got.as_deref()), ("UNSUPPORTED", Some("flows[0][3]"), Some("lambda")));
    }
}
