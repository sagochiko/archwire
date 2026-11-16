//! Picks which layout a definition goes through: the VPC layout (Grid) or the layout without
//! a VPC (Flow). It is decided once, right after the definition is read, from the services data.

use crate::def::{Definition, Ref};
use crate::diag::Problem;
use crate::rules::{Rules, Scope};

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

/// Stops a definition with a VPC part that writes a name after the dot on any part: one error
/// per position (`Ref.at`) of such a part, as the inference with a VPC used to.
pub fn named_vpc_parts(def: &Definition, rules: &Rules) -> Result<(), Vec<Problem>> {
    let refs = || def.links.iter().flat_map(|l| [&l.from, &l.to]);
    if !refs().any(|r| rules.service(&r.service).is_some_and(|s| s.scope == Scope::Vpc)) {
        return Ok(());
    }
    let mut seen_at: Vec<&str> = Vec::new();
    let mut errors = Vec::new();
    for r in refs().filter(|r| r.tag.is_some()) {
        if seen_at.contains(&r.at.as_str()) {
            continue;
        }
        seen_at.push(&r.at);
        errors.push(Problem {
            code: "UNSUPPORTED".into(),
            at: Some(r.at.clone()),
            got: Some(r.name.clone()),
            message: "named parts such as `s3.web` are not supported together with VPC services yet".into(),
            ..Default::default()
        });
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

    // Fails if a named part outside the VPC is let through, a position is reported twice, or a definition without a VPC part is stopped.
    #[test]
    fn named_parts_stop_only_with_a_vpc_part() {
        let e = named_vpc_parts(&crate::def::parse("flows:\n  - user -> alb -> ecs -> s3.web\n  - alb ~> s3.web\n").unwrap(), &Rules::builtin()).unwrap_err();
        let got: Vec<_> = e.iter().map(|p| (p.code.as_str(), p.at.as_deref(), p.got.as_deref())).collect();
        assert_eq!(got, [("UNSUPPORTED", Some("flows[0][3]"), Some("s3.web")), ("UNSUPPORTED", Some("flows[1][1]"), Some("s3.web"))]);
        assert_eq!(e[0].message, "named parts such as `s3.web` are not supported together with VPC services yet");
        assert_eq!(named_vpc_parts(&crate::def::parse("flows:\n  - user -> apigateway(http) -> lambda.a\n").unwrap(), &Rules::builtin()), Ok(()));
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
