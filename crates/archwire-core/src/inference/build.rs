//! The parts and lines of a definition without a VPC part: every written part becomes one node
//! and every accepted connection one edge.

use super::{area_of, Settings};
use crate::def::{Definition, Link, Ref};
use crate::ids::kind_hole;
use crate::model::{Edge, Model, Node};
use crate::rules::{LinkRule, Rules};

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
    settings: Settings,
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
    let kind_of = |name: &str| nodes.iter().find(|n| n.id == name).and_then(|n| n.option.clone());

    let mut edges: Vec<Edge> = Vec::new();
    for (l, rule) in accepted {
        let note = kind_of(&l.to.name).and_then(|k| rule.to_kind.get(&k).cloned());
        edges.push(Edge {
            from: l.from.name.clone(),
            to: l.to.name.clone(),
            rel: l.rel,
            label: note.map_or_else(|| rule.label.clone(), |n| n.label),
            hole: None,
            endpoint: false,
        });
    }

    let Settings { region, lang, direction } = settings;
    Model { region, azs: Vec::new(), lang, nodes, edges, direction }
}
