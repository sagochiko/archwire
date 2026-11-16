//! The checks on what a definition writes: its names, its connections, whether every main flow
//! is reached from an origin, the limits of a connection, and the answers given to its holes.

use super::{problem, strings, ROUTE_CHOICES};
use crate::def::{Definition, Link, Ref, Rel};
use crate::diag::Problem;
use crate::ids::{kind_hole, route_hole};
use crate::rules::{LinkRule, Rules, Scope, Subnet};

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

/// Each written part once, at its first mention.
pub(super) fn parts(def: &Definition) -> Vec<&Ref> {
    let mut parts: Vec<&Ref> = Vec::new();
    for r in def.links.iter().flat_map(|l| [&l.from, &l.to]) {
        if !parts.iter().any(|p| p.name == r.name) {
            parts.push(r);
        }
    }
    parts
}

/// The connections the rules data lists, in the order written; any other stops with UNSUPPORTED.
/// Then whether the main flows are reached, and the limits of each connection.
pub(super) fn check_links<'a>(
    def: &'a Definition,
    rules: &'a Rules,
    parts: &[&Ref],
    errors: &mut Vec<Problem>,
) -> Vec<(&'a Link, &'a LinkRule)> {
    let mut accepted: Vec<(&Link, &LinkRule)> = Vec::new();
    for l in &def.links {
        match rules.link(&l.from.service, &l.to.service, l.rel) {
            Some(rule) => accepted.push((l, rule)),
            None => {
                let got = written(l);
                errors.push(problem("UNSUPPORTED", &l.at, &got, Vec::new(), format!("`{got}` is not a supported connection")));
            }
        }
    }
    check_main_flows(def, rules, parts, &accepted, errors);
    check_limits(&accepted, errors);
    accepted
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

    let mut reached: Vec<&str> = parts.iter().filter(|r| origins.contains(&r.service)).map(|r| r.name.as_str()).collect();
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

/// A part of a service that needs a kind and is written without one is a hole, in the order
/// the parts first appear.
pub(super) fn kind_holes<'a>(rules: &Rules, parts: &[&'a Ref]) -> Vec<(String, &'a Ref)> {
    parts
        .iter()
        .filter(|r| r.option.is_none() && rules.service(&r.service).is_some_and(|s| s.kind_hole))
        .map(|r| (kind_hole(&r.name), *r))
        .collect()
}

/// A main flow from a part in a private subnet to a Regional part outside the VPC: its route is
/// a hole, in the order the lines are written, with the line it is on.
pub(super) fn route_holes<'a>(def: &'a Definition, rules: &Rules) -> Vec<(String, &'a Link)> {
    let private = |r: &Ref| rules.service(&r.service).is_some_and(|s| s.scope == Scope::Vpc && s.subnet == Some(Subnet::Private));
    let regional = |r: &Ref| rules.service(&r.service).is_some_and(|s| s.scope == Scope::Regional);
    def.links
        .iter()
        .filter(|l| l.rel == Rel::Main && private(&l.from) && regional(&l.to))
        .map(|l| (route_hole(&l.from.name, &l.to.name), l))
        .collect()
}

/// Every answer in `choose` names a hole, and is one of its choices.
pub(super) fn check_choices(
    def: &Definition,
    rules: &Rules,
    kinds: &[(String, &Ref)],
    routes: &[(String, &Link)],
    errors: &mut Vec<Problem>,
) {
    for (key, value) in &def.choose {
        let at = format!("choose.{key}");
        if let Some((_, r)) = kinds.iter().find(|(id, _)| id == key) {
            let options = &rules.service(&r.service).expect("open holes have known services").options;
            if !options.contains_key(value) {
                let allowed = options.keys().cloned().collect();
                errors.push(problem("BAD_CHOICE", &at, value, allowed, format!("`{value}` is not a choice for `{key}`")));
            }
        } else if routes.iter().any(|(id, _)| id == key) {
            if !ROUTE_CHOICES.contains(&value.as_str()) {
                let allowed = strings(&ROUTE_CHOICES);
                errors.push(problem("BAD_CHOICE", &at, value, allowed, format!("`{value}` is not a choice for `{key}`")));
            }
        } else {
            let allowed = kinds.iter().map(|(id, _)| id.clone()).chain(routes.iter().map(|(id, _)| id.clone())).collect();
            errors.push(problem("UNKNOWN_HOLE", &at, key, allowed, format!("there is no hole `{key}`")));
        }
    }
}
