//! The checks on what a definition writes: its names, its connections, whether every main flow
//! is reached from an origin, the limits of a connection, and the answers given to its holes.

use super::{area_of, problem, strings, ROUTE_CHOICES};
use crate::def::{Definition, Link, Ref, Rel};
use crate::diag::Problem;
use crate::ids::{kind_hole, route_hole};
use crate::model::{Area, Tier};
use crate::rules::{LinkRule, Rules, Scope, Service, Subnet};

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

/// The model's tier for a subnet tier of the rules data.
fn tier(subnet: Subnet) -> Tier {
    match subnet {
        Subnet::Public => Tier::Public,
        Subnet::Private => Tier::Private,
    }
}

/// The accepted connection that places a part in a subnet tier of the VPC (`from_subnet`), if
/// any: the first such connection from the part among all of them, so where a part belongs is
/// decided after every line is read, whatever their order.
pub(super) fn attached_by<'r>(accepted: &[(&Link, &'r LinkRule)], name: &str) -> Option<&'r LinkRule> {
    accepted.iter().find(|(l, rule)| l.from.name == name && rule.from_subnet.is_some()).map(|(_, rule)| *rule)
}

/// The accepted connection that gives a part another scope (`from_scope`), if any: the first such
/// connection from the part among all of them. The checks stop a part two connections place apart.
fn scoped_by<'r>(accepted: &[(&Link, &'r LinkRule)], name: &str) -> Option<&'r LinkRule> {
    accepted.iter().find(|(l, rule)| l.from.name == name && rule.from_scope.is_some()).map(|(_, rule)| *rule)
}

/// The area a part of this service has with this scope, mapped as `area_of` maps the service's own.
fn area_with(service: &Service, scope: Option<Scope>) -> Area {
    match scope {
        Some(scope) => area_of(&Service { scope, ..service.clone() }),
        None => area_of(service),
    }
}

/// Where a written part belongs: the subnet tier an accepted connection places it in, else the
/// scope an accepted connection gives it, else the area its service gives. The checks, the model
/// and the reasons all read this one answer. None for a part of an unknown service.
pub(super) fn area(rules: &Rules, accepted: &[(&Link, &LinkRule)], r: &Ref) -> Option<Area> {
    match attached_by(accepted, &r.name).and_then(|rule| rule.from_subnet) {
        Some(subnet) => Some(Area::Subnet(tier(subnet))),
        None => {
            let scope = scoped_by(accepted, &r.name).and_then(|rule| rule.from_scope);
            rules.service(&r.service).map(|s| area_with(s, scope))
        }
    }
}

/// The connections the rules data lists, in the order written; any other stops with UNSUPPORTED.
/// Then whether traffic leaving the VPC leaves from inside it, whether the main flows are
/// reached, and the limits of each connection.
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
    check_egress(rules, &accepted, errors);
    check_main_flows(def, rules, parts, &accepted, errors);
    let closing = check_loops(rules, &accepted, errors);
    check_reentry(rules, &accepted, &closing, errors);
    check_limits(&accepted, errors);
    check_places(rules, &accepted, errors);
    accepted
}

/// One part, two places: a part whose connections give it a different area or Region than an
/// earlier connection from it stops at the later line. Each connection's place is its service's
/// with the connection's own `from_scope` and `from_region` applied, so a connection without an
/// override still counts. A connection that places the part in a subnet tier is left out: the
/// part follows that one, whatever its other connections are.
fn check_places(rules: &Rules, accepted: &[(&Link, &LinkRule)], errors: &mut Vec<Problem>) {
    let mut first: Vec<(&str, (Area, Option<&str>), &Link)> = Vec::new();
    for (l, rule) in accepted.iter().filter(|(_, rule)| rule.from_subnet.is_none()) {
        let Some(service) = rules.service(&l.from.service) else { continue };
        let place = (area_with(service, rule.from_scope), rule.from_region.as_deref());
        match first.iter().find(|(name, _, _)| *name == l.from.name) {
            None => first.push((&l.from.name, place, l)),
            Some((_, seen, earlier)) if *seen != place => {
                let part = &l.from.name;
                // The part's service with the whole name of the part it is attached to, which differ
                // between the two connections.
                let split = |to: &Ref| format!("`{}.{} -- {}`", l.from.service, to.name.replace(['.', '_'], "-"), to.name);
                let message = format!(
                    "`{part}` is attached to both `{}` and `{}`, which need it in different places; write them as two parts, such as {} and {}",
                    earlier.to.name,
                    l.to.name,
                    split(&earlier.to),
                    split(&l.to)
                );
                errors.push(problem("UNSUPPORTED", &l.at, part, Vec::new(), message));
            }
            Some(_) => {}
        }
    }
}

/// A main flow to where traffic leaves the VPC (`egress`) needs its `from` part in a private
/// subnet, as all the accepted connections place it; anywhere else there is nothing to draw it through.
fn check_egress(rules: &Rules, accepted: &[(&Link, &LinkRule)], errors: &mut Vec<Problem>) {
    let leaving = |l: &Link| l.rel == Rel::Main && rules.service(&l.to.service).is_some_and(|s| s.egress);
    for (l, _) in accepted.iter().filter(|(l, _)| leaving(l)) {
        if area(rules, accepted, &l.from) != Some(Area::Subnet(Tier::Private)) {
            let got = written(l);
            let message = format!("`{got}` needs `{}` in a VPC; connect it to a part in a private subnet", l.from.name);
            errors.push(problem("UNSUPPORTED", &l.at, &got, Vec::new(), message));
        }
    }
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

/// A loop of main flows: the line that closes it, the parts it goes through from that line's
/// `to` part on, and whether every one of them is in a subnet of the VPC.
pub(super) struct Loop<'a> {
    pub(super) line: &'a Link,
    pub(super) through: Vec<&'a str>,
    pub(super) in_subnets: bool,
}

/// The loops of main flows between parts, each found at the line that closes it: the lines are
/// read in the order written, and a line closes a loop when the parts before it already lead from
/// its `to` part back to its `from` part. A loop through a part outside the subnets of the VPC is
/// preferred over one that stays inside them. A line that closes a loop through such a part is
/// not followed by the lines after it, so one loop is found once.
pub(super) fn main_flow_loops<'a>(rules: &Rules, accepted: &[(&'a Link, &LinkRule)]) -> Vec<Loop<'a>> {
    let in_subnet = |name: &str| {
        let r = accepted.iter().find_map(|(l, _)| [&l.from, &l.to].into_iter().find(|r| r.name == name));
        r.is_some_and(|r| matches!(area(rules, accepted, r), Some(Area::Subnet(_))))
    };
    // The shortest path along `kept` from `from` to `to`, each step taken in the order written.
    fn path<'a>(kept: &[&'a Link], from: &'a str, to: &str) -> Option<Vec<&'a str>> {
        let mut seen: Vec<(&'a str, Option<usize>)> = vec![(from, None)];
        let mut k = 0;
        while k < seen.len() {
            let here = seen[k].0;
            if here == to {
                let mut out = vec![here];
                let mut at = k;
                while let Some(back) = seen[at].1 {
                    out.push(seen[back].0);
                    at = back;
                }
                out.reverse();
                return Some(out);
            }
            for l in kept.iter().filter(|l| l.from.name == here) {
                if !seen.iter().any(|(n, _)| *n == l.to.name) {
                    seen.push((&l.to.name, Some(k)));
                }
            }
            k += 1;
        }
        None
    }
    let mut kept: Vec<&'a Link> = Vec::new();
    let mut out = Vec::new();
    for (l, _) in accepted.iter().filter(|(l, _)| l.rel == Rel::Main) {
        let (from, to) = (l.from.name.as_str(), l.to.name.as_str());
        let Some(any) = path(&kept, to, from) else {
            kept.push(l);
            continue;
        };
        // A loop through a part outside the subnets, by way of the first such part reached.
        let outside = if !in_subnet(from) || !in_subnet(to) {
            Some(any.clone())
        } else {
            any.iter().copied().chain(kept.iter().map(|k| k.to.name.as_str())).filter(|n| !in_subnet(n)).find_map(|x| {
                let there = path(&kept, to, x)?;
                let back = path(&kept, x, from)?;
                Some(there.into_iter().chain(back.into_iter().skip(1)).collect())
            })
        };
        match outside {
            Some(through) => out.push(Loop { line: l, through, in_subnets: false }),
            None => {
                out.push(Loop { line: l, through: any, in_subnets: true });
                kept.push(l);
            }
        }
    }
    out
}

/// A loop of main flows through a part outside the subnets of the VPC stops at the line that
/// closes it: the parts of the diagram outside the VPC are laid out along the flows, so a line back
/// to a part it left has nowhere to go. A loop that stays inside the subnets is drawn.
/// Returns the lines it stopped.
fn check_loops<'a>(rules: &Rules, accepted: &[(&'a Link, &LinkRule)], errors: &mut Vec<Problem>) -> Vec<&'a Link> {
    let mut closing = Vec::new();
    for lp in main_flow_loops(rules, accepted).into_iter().filter(|lp| !lp.in_subnets) {
        let got = written(lp.line);
        let through = lp.through.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ");
        let message = format!("`{got}` closes a loop of main flows (->) through {through}; this version does not draw main flows that come back to a part");
        errors.push(problem("UNSUPPORTED", &lp.line.at, &got, Vec::new(), message));
        closing.push(lp.line);
    }
    closing
}

/// A main flow that leaves the subnets of the VPC and comes back into them stops at the line that
/// comes back: a main flow from a part outside the subnets to a part in them, when that outside
/// part is reached from a part in the subnets along main flows through parts outside them. The
/// parts outside the subnets are laid out apart from the zones, after the parts that flow into
/// them, so a line from them back into a zone has nowhere to go. The lines are read in the order
/// written, and the part named is the first part in the subnets found going back from the line,
/// each step taken in the order written. A line that closes a loop is left to the check on loops.
fn check_reentry(rules: &Rules, accepted: &[(&Link, &LinkRule)], closing: &[&Link], errors: &mut Vec<Problem>) {
    let main: Vec<&Link> = accepted.iter().filter(|(l, _)| l.rel == Rel::Main).map(|(l, _)| *l).collect();
    let in_subnet = |r: &Ref| matches!(area(rules, accepted, r), Some(Area::Subnet(_)));
    for l in main.iter().filter(|l| !in_subnet(&l.from) && in_subnet(&l.to)) {
        if closing.iter().any(|c| std::ptr::eq(*c, *l)) {
            continue;
        }
        // Back from the line's `from` part through parts outside the subnets, to a part in them.
        let mut outside: Vec<&str> = vec![&l.from.name];
        let mut left: Option<&str> = None;
        let mut k = 0;
        while k < outside.len() && left.is_none() {
            let here = outside[k];
            for m in main.iter().filter(|m| m.to.name == here) {
                if in_subnet(&m.from) {
                    left = Some(&m.from.name);
                    break;
                }
                if !outside.contains(&m.from.name.as_str()) {
                    outside.push(&m.from.name);
                }
            }
            k += 1;
        }
        let Some(left) = left else { continue };
        let got = written(l);
        let message = format!(
            "`{got}` brings a main flow (->) that left the subnets of the VPC at `{left}` back into them; this version does not draw main flows that leave the subnets and come back"
        );
        errors.push(problem("UNSUPPORTED", &l.at, &got, Vec::new(), message));
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

/// A main flow from a part in a private subnet to a Regional part outside the VPC, where each
/// part belongs as `area` decides from all the accepted connections: its route is a hole, in the
/// order the lines are written, with the line it is on.
pub(super) fn route_holes<'a>(def: &'a Definition, rules: &Rules, accepted: &[(&Link, &LinkRule)]) -> Vec<(String, &'a Link)> {
    let private = |r: &Ref| area(rules, accepted, r) == Some(Area::Subnet(Tier::Private));
    let regional = |r: &Ref| area(rules, accepted, r) == Some(Area::Region);
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
