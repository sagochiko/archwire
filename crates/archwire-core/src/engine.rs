//! The check at the entrance of `compile` for a definition with a VPC: the shape this version
//! does not draw yet, two or more parts spanning the zones, stops here before the inference.

use crate::def::Definition;
use crate::diag::Problem;
use crate::rules::{Place, Rules, Scope};

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
}
