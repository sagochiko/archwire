//! The parts and lines of a definition. Without a VPC part every written part becomes one node
//! and every accepted connection one edge. With one, parts in a private subnet are copied per
//! zone, and the gateways the traffic needs are added with the lines that join them.

use super::links::{area as area_in, attached_by};
use super::{area_of, Checked, Settings};
use crate::def::{Definition, Link, Ref, Rel};
use crate::diag::Problem;
use crate::ids::{copy_id, kind_hole};
use crate::model::{Area, Edge, Model, Node, Tier};
use crate::rules::{DerivedLink, DerivedRole, LinkRule, Place, Rules, Scope, Service};

/// The letters after the Region of the zones a definition with a VPC uses, in order.
const AZ_LETTERS: [&str; 3] = ["a", "c", "d"];

/// The kind a part has: written after its service, or answered in `choose`.
pub(super) fn kind(def: &Definition, r: &Ref) -> Option<String> {
    r.option.clone().or_else(|| def.choose.get(&kind_hole(&r.name)).cloned())
}

/// The connection that moves a part to another Region, if any.
pub(super) fn moved<'a>(accepted: &[(&'a Link, &'a LinkRule)], name: &str) -> Option<&'a LinkRule> {
    accepted.iter().find(|(l, rule)| l.from.name == name && rule.from_rule.is_some()).map(|(_, rule)| *rule)
}

pub(super) fn build(
    def: &Definition,
    rules: &Rules,
    parts: &[&Ref],
    accepted: &[(&Link, &LinkRule)],
    open: &[(String, &Ref)],
    settings: &Settings,
) -> Model {
    let svc = |name: &str| rules.service(name).expect("names were checked");

    let mut nodes: Vec<Node> = Vec::new();
    for r in parts {
        let s = svc(&r.service);
        // No accepted connection has a VPC service, so every part is outside the VPC.
        let area = area_of(s);
        nodes.push(Node {
            id: r.name.clone(),
            service: r.service.clone(),
            option: kind(def, r),
            az: None,
            role: None,
            added: false,
            region: moved(accepted, &r.name).and_then(|rule| rule.from_region.clone()),
            part: r.name.clone(),
            tag: r.tag.clone(),
            area,
            undecided: open.iter().any(|(id, o)| o.name == r.name && !def.choose.contains_key(id)),
        });
    }
    let find = |name: &str| nodes.iter().find(|n| n.id == name).expect("every written part is a node");

    let mut edges: Vec<Edge> = Vec::new();
    for (l, rule) in accepted {
        edges.push(Edge {
            from: l.from.name.clone(),
            to: l.to.name.clone(),
            rel: l.rel,
            label: line_label(rule, find(&l.to.name)),
            hole: None,
            endpoint: false,
        });
    }

    Model { region: settings.region.clone(), azs: Vec::new(), lang: settings.lang, nodes, edges, direction: settings.direction }
}

/// The label of a written line into `to`: the label for the kind of `to` (`to_kind`) if it has
/// one, else the label for its role (`to_role`), else the connection's `label`. Loading stops a
/// connection that writes both `to_kind` and `to_role`, so at most one of the first two applies.
fn line_label(rule: &LinkRule, to: &Node) -> String {
    let by_kind = to.option.as_ref().and_then(|k| rule.to_kind.get(k)).map(|note| &note.label);
    let by_role = to.role.as_ref().and_then(|r| rule.to_role.get(r));
    by_kind.or(by_role).unwrap_or(&rule.label).clone()
}

/// The service that plays this role at the VPC's edge. Loading allows at most one; a missing one
/// adds DERIVED_ROLE_MISSING.
fn by_role<'r>(rules: &'r Rules, role: DerivedRole, missing: &mut Vec<Problem>) -> Option<&'r Service> {
    let found = rules.service_keys().iter().filter_map(|k| rules.service(k)).find(|s| s.derived_role == Some(role));
    if found.is_none() {
        let message = format!("no service in services.toml has `derived_role = \"{}\"`", role.name());
        missing.push(Problem { code: "DERIVED_ROLE_MISSING".into(), at: Some(role.name().into()), message, ..Default::default() });
    }
    found
}

/// The line inference adds from a part of one role to a part of another; a missing one adds
/// DERIVED_LINK_MISSING.
fn derived_line<'r>(rules: &'r Rules, from: &str, to: &str, missing: &mut Vec<Problem>) -> Option<&'r DerivedLink> {
    let found = rules.derived_link(from, to);
    if found.is_none() {
        let message = format!("links.toml has no `[[derived_link]]` from `{from}` to `{to}`");
        let at = format!("derived_link {from} {to}");
        missing.push(Problem { code: "DERIVED_LINK_MISSING".into(), at: Some(at), message, ..Default::default() });
    }
    found
}

/// A node inference adds: one per service, or one per zone.
fn added(id: String, s: &Service, az: Option<usize>) -> Node {
    Node {
        id,
        service: s.key.clone(),
        option: None,
        az,
        role: None,
        added: true,
        region: None,
        part: s.key.clone(),
        tag: None,
        area: area_of(s),
        undecided: false,
    }
}

/// The semantic model of a definition with a VPC part that passed the checks: parts, copies, added parts and lines.
/// A part or line the definition needs and the rules data lacks (DERIVED_ROLE_MISSING, DERIVED_LINK_MISSING)
/// is an error; one it does not need may be missing.
pub(super) fn vpc_model(def: &Definition, rules: &Rules, checked: &Checked) -> Result<Model, Vec<Problem>> {
    let svc = |name: &str| rules.service(name).expect("names were checked");
    let Checked { settings, parts, accepted, kinds: open, routes, .. } = checked;
    let azs: Vec<String> = AZ_LETTERS[..settings.azs].iter().map(|l| format!("{}{l}", settings.region)).collect();
    let suffix = |i: usize| azs[i][azs[i].len() - 2..].to_string();

    // Where every written part belongs, as the checks decided it from all the connections: a
    // connection with `from_subnet` puts its `from` part in that tier, one copy per zone.
    let into_subnet = |r: &Ref| attached_by(accepted, &r.name).is_some();
    let area = |r: &Ref| area_in(rules, accepted, r).expect("names were checked");
    let egress = |r: &Ref| svc(&r.service).egress;
    let private = |r: &Ref| area(r) == Area::Subnet(Tier::Private);

    // The route hole on a line, found by the checks; it stays open until answered.
    let route = |l: &Link| routes.iter().find(|(_, r)| std::ptr::eq(*r, l)).map(|(id, _)| id.clone());
    let answer = |l: &Link| route(l).and_then(|id| def.choose.get(&id).map(String::as_str));

    // Private parts whose traffic leaves the VPC: main flows to where traffic leaves, then routes answered `nat`.
    let mut outbound: Vec<&str> = Vec::new();
    let leaving = accepted.iter().filter(|(l, _)| l.rel == Rel::Main && egress(&l.to)).map(|(l, _)| l);
    let through_nat = accepted.iter().filter(|(l, _)| answer(l) == Some("nat")).map(|(l, _)| l);
    for l in leaving.chain(through_nat) {
        if !outbound.contains(&l.from.name.as_str()) {
            outbound.push(&l.from.name);
        }
    }
    let nat_azs: Vec<usize> = match (outbound.is_empty(), settings.nat_single) {
        (true, _) => Vec::new(),
        (false, true) => vec![0],
        (false, false) => (0..settings.azs).collect(),
    };
    // A main flow from outside the VPC into a public part enters through the entry gateway.
    let entry_line = |l: &Link| {
        l.rel == Rel::Main
            && matches!(svc(&l.from.service).scope, Scope::Outside | Scope::Global)
            && area(&l.to) == Area::Subnet(Tier::Public)
    };
    let entry = accepted.iter().any(|(l, _)| entry_line(l));
    // The rules data is looked up only for what this definition needs.
    let mut missing: Vec<Problem> = Vec::new();
    let gateway = if entry || !nat_azs.is_empty() { by_role(rules, DerivedRole::EntryGateway, &mut missing) } else { None };
    let nat = if nat_azs.is_empty() { None } else { by_role(rules, DerivedRole::EgressNat, &mut missing) };
    let egress_lines = if nat_azs.is_empty() {
        None
    } else {
        let out = derived_line(rules, "private", "egress_nat", &mut missing);
        let up = derived_line(rules, "egress_nat", "entry_gateway", &mut missing);
        out.zip(up)
    };
    let nat_id = |nat: &Service, i: usize| copy_id(&nat.key, &suffix(i));

    let written = |r: &Ref, id: String, az: Option<usize>, role: Option<&str>| Node {
        id,
        service: r.service.clone(),
        option: kind(def, r),
        az,
        role: role.map(String::from),
        added: false,
        region: moved(accepted, &r.name).and_then(|rule| rule.from_region.clone()),
        part: r.name.clone(),
        tag: r.tag.clone(),
        area: area(r),
        undecided: open.iter().any(|(id, o)| o.name == r.name && !def.choose.contains_key(id)),
    };
    // A part in a subnet: one per zone, a Writer and its Readers, or one spanning the zones.
    let in_subnet = |r: &Ref, nodes: &mut Vec<Node>| {
        let place = svc(&r.service).place;
        if into_subnet(r) || (private(r) && place == Place::PerAz) {
            for i in 0..settings.azs {
                nodes.push(written(r, copy_id(&r.name, &suffix(i)), Some(i), None));
            }
        } else if private(r) && place == Place::WriterReader {
            for i in 0..=settings.readers {
                let role = if i == 0 { "writer" } else { "reader" };
                nodes.push(written(r, copy_id(&r.name, &suffix(i)), Some(i), Some(role)));
            }
        } else {
            nodes.push(written(r, r.name.clone(), None, None));
        }
    };

    // Nodes, in the order the diagram reads: outside and global parts, the entry gateway, public
    // parts, NAT gateways, private parts with their copies, Regional parts.
    let drawn: Vec<&Ref> = parts.iter().copied().filter(|r| !egress(r)).collect();
    let mut nodes: Vec<Node> = Vec::new();
    for r in drawn.iter().filter(|r| matches!(area(r), Area::Outside | Area::Cloud)) {
        nodes.push(written(r, r.name.clone(), None, None));
    }
    if let Some(g) = gateway {
        nodes.push(added(g.key.clone(), g, None));
    }
    for r in drawn.iter().filter(|r| area(r) == Area::Subnet(Tier::Public)) {
        in_subnet(r, &mut nodes);
    }
    if let Some(n) = nat {
        for &i in &nat_azs {
            nodes.push(added(nat_id(n, i), n, Some(i)));
        }
    }
    for r in drawn.iter().filter(|r| private(r)) {
        in_subnet(r, &mut nodes);
    }
    for r in drawn.iter().filter(|r| area(r) == Area::Region) {
        nodes.push(written(r, r.name.clone(), None, None));
    }

    let writers = nodes.iter().any(|n| n.role.as_deref() == Some("writer"));
    let replication = if writers { derived_line(rules, "writer", "reader", &mut missing) } else { None };
    if !missing.is_empty() {
        return Err(missing);
    }

    // Edges, one per pair of ends and relation.
    let mut edges: Vec<Edge> = Vec::new();
    let find = |id: &str| nodes.iter().find(|n| n.id == id).expect("edge ends are nodes");
    let copies = |part: &str| nodes.iter().filter(|n| n.part == part && !n.added).collect::<Vec<_>>();
    let add = |edges: &mut Vec<Edge>, from: &Node, to: &Node, rel: Rel, label: &str, hole: Option<String>, endpoint: bool| {
        if !edges.iter().any(|e| e.from == from.id && e.to == to.id && e.rel == rel) {
            edges.push(Edge { from: from.id.clone(), to: to.id.clone(), rel, label: label.to_string(), hole, endpoint });
        }
    };
    for (l, rule) in accepted.iter().filter(|(l, _)| !egress(&l.to)) {
        if entry_line(l) {
            // An entry line asks for the gateway above, and a missing one has returned.
            let g = find(&gateway.expect("an entry adds the gateway").key);
            // Both halves carry the label of the written line, read for the part it goes to. An
            // entry goes to a public part, whose copies share its kind and never have a role.
            let tos = copies(&l.to.name);
            let label = line_label(rule, tos[0]);
            for from in copies(&l.from.name) {
                add(&mut edges, from, g, Rel::Main, &label, None, false);
            }
            for to in tos {
                add(&mut edges, g, to, Rel::Main, &label, None, false);
            }
            continue;
        }
        let hole = route(l).filter(|_| answer(l).is_none());
        let endpoint = answer(l) == Some("endpoint");
        for from in copies(&l.from.name) {
            for to in copies(&l.to.name) {
                add(&mut edges, from, to, l.rel, &line_label(rule, to), hole.clone(), endpoint);
            }
        }
    }
    if let Some(rep) = replication {
        for w in nodes.iter().filter(|n| n.role.as_deref() == Some("writer")) {
            for r in nodes.iter().filter(|n| n.part == w.part && n.role.as_deref() == Some("reader")) {
                add(&mut edges, w, r, rep.rel, &rep.label, None, false);
            }
        }
    }
    if let (Some(g), Some(n), Some((out, up))) = (gateway, nat, egress_lines) {
        for name in &outbound {
            for from in copies(name) {
                let i = if settings.nat_single { 0 } else { from.az.unwrap_or(0) };
                add(&mut edges, from, find(&nat_id(n, i)), out.rel, &out.label, None, false);
            }
        }
        for &i in &nat_azs {
            add(&mut edges, find(&nat_id(n, i)), find(&g.key), up.rel, &up.label, None, false);
        }
    }

    Ok(Model { region: settings.region.clone(), azs, lang: settings.lang, nodes, edges, direction: settings.direction })
}
