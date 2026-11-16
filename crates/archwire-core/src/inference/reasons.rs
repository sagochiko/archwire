//! The reasons for every frame, place and kind archwire decides, the holes left open, and the notes.

use super::build::moved;
use super::{Checked, ROUTE_CHOICES};
use crate::def::{Definition, Ref, Rel};
use crate::diag::{Derived, Diagnostics, Hole};
use crate::model::{Area, Model, Tier};
use crate::rules::{DerivedRole, Lang, Place, Rules, Scope, VpcReasons};

pub(crate) fn derived(rules: &Rules, lang: Lang, target: &str, id: &str, added: bool) -> Derived {
    let r = rules.rule(id);
    Derived {
        target: target.into(),
        added,
        kind: r.kind.clone(),
        rule: id.into(),
        reason: match lang {
            Lang::En => r.reason_en.clone(),
            Lang::Ja => r.reason_ja.clone(),
        },
        source: r.source.clone(),
    }
}

/// The reasons the VPC and its zones get: those of the first part whose service has them.
pub(super) fn vpc_reasons<'r>(rules: &'r Rules, parts: &[&Ref]) -> Option<&'r VpcReasons> {
    parts.iter().find_map(|r| rules.service(&r.service).and_then(|s| s.vpc_reasons.as_ref()))
}

/// Whether a node is inside the VPC: on its edge or in a subnet.
fn in_vpc(area: Area) -> bool {
    matches!(area, Area::VpcEdge | Area::Subnet(_))
}

/// The reasons of a model, in the order the diagram reads: the frames, the VPC, its zones and
/// subnets, each written part, then the gateways added. Then its open holes and its notes.
pub(super) fn explain(def: &Definition, rules: &Rules, model: &Model, checked: &Checked, diag: &mut Diagnostics) {
    let Checked { vpc, parts, accepted, kinds, routes, .. } = checked;
    let lang = model.lang;
    let svc = |name: &str| rules.service(name).expect("names were checked");
    let kind_of = |name: &str| model.nodes.iter().find(|n| n.part == name).and_then(|n| n.option.clone());
    // The parts drawn: a main flow to where traffic leaves the VPC is not a part.
    let drawn: Vec<&Ref> = parts.iter().copied().filter(|r| !svc(&r.service).egress).collect();
    // A target the definition did not write is something archwire added.
    let written = |target: &str| drawn.iter().any(|r| r.name == target);
    let by_role = |role: DerivedRole| {
        model.nodes.iter().filter(move |n| n.added && rules.service(&n.service).and_then(|s| s.derived_role) == Some(role))
    };

    let mut why = |target: &str, id: &str| diag.derived.push(derived(rules, lang, target, id, !written(target)));
    why("aws-cloud", "resources-inside-aws-cloud-frame");
    // The Region frame, and the Region chosen for it, when the VPC or a part is drawn inside it.
    if *vpc || model.nodes.iter().any(|n| n.area == Area::Region) {
        why("region", "resources-inside-region-frame");
        if !def.set.contains_key("region") {
            why("region", "default-region");
        }
    }
    if let Some(v) = vpc_reasons(rules, parts).filter(|_| *vpc) {
        why("vpc", &v.vpc);
        for id in &v.az {
            why("az", id);
        }
        if !def.set.contains_key("azs") {
            why("az", &v.default_azs);
        }
        let tier = |t: Tier| model.nodes.iter().any(|n| n.area == Area::Subnet(t));
        match (tier(Tier::Public), tier(Tier::Private)) {
            (true, true) => why("subnet", "public-and-private-subnet-per-az"),
            (true, false) => why("subnet", "public-subnet-per-az"),
            (false, true) => why("subnet", "private-subnet-per-az"),
            (false, false) => {}
        }
    }
    for r in &drawn {
        let s = svc(&r.service);
        let private = area_of_part(model, &r.name) == Some(Area::Subnet(Tier::Private));
        let place: Vec<String> = match (moved(accepted, &r.name), &s.place_rule, s.scope) {
            (Some(rule), _, _) => rule.from_rule.iter().cloned().collect(),
            (None, Some(id), _) => vec![id.clone()],
            (None, None, Scope::Global) => vec!["global-service-outside-region".into()],
            (None, None, Scope::Regional) => vec!["regional-service-outside-vpc".into()],
            (None, None, Scope::Vpc) if private && s.place == Place::PerAz => vec!["compute-in-private".into()],
            (None, None, Scope::Vpc) if private && s.place == Place::WriterReader => {
                vec!["database-in-private".into(), "aurora-writer-reader".into()]
            }
            (None, None, Scope::Outside | Scope::Vpc) => Vec::new(),
        };
        for id in &place {
            why(&r.name, id);
        }
        let Some(kind) = kind_of(&r.name) else { continue };
        let mut given: Vec<&str> = Vec::new();
        for (_, rule) in accepted.iter().filter(|(l, _)| l.to.name == r.name) {
            let Some(note) = rule.to_kind.get(&kind) else { continue };
            if !given.contains(&note.rule.as_str()) {
                given.push(&note.rule);
                why(&r.name, &note.rule);
            }
        }
    }
    // The entry gateway: for traffic coming in from outside the VPC, else for traffic leaving it.
    for g in by_role(DerivedRole::EntryGateway) {
        let from_outside = model.edges.iter().any(|e| {
            e.to == g.id && model.nodes.iter().any(|n| n.id == e.from && matches!(n.area, Area::Outside | Area::Cloud))
        });
        why(&g.id, if from_outside { "internet-facing-alb-in-vpc-with-igw" } else { "private-egress-through-nat" });
    }
    let nats: Vec<_> = by_role(DerivedRole::EgressNat).collect();
    for n in &nats {
        why(&n.id, "private-egress-through-nat");
    }
    if let Some(n) = nats.first().filter(|_| !def.set.contains_key("nat")) {
        why(&n.part, "nat-per-az");
    }

    let answered = |id: &String| def.choose.contains_key(id);
    for (id, r) in kinds.iter().filter(|(id, _)| !answered(id)) {
        diag.holes.push(Hole {
            hole: id.clone(),
            at: r.at.clone(),
            need: rules.text("need.kind", lang).replace("{part}", &r.name),
            choices: svc(&r.service).options.keys().cloned().collect(),
        });
    }
    for (id, l) in routes.iter().filter(|(id, _)| !answered(id)) {
        diag.holes.push(Hole {
            hole: id.clone(),
            at: l.at.clone(),
            need: rules.text("need.route", lang).replace("{from}", &l.from.name).replace("{to}", &l.to.name),
            choices: ROUTE_CHOICES.iter().map(|c| c.to_string()).collect(),
        });
    }

    let aux_from_vpc = accepted.iter().any(|(l, _)| l.rel == Rel::Aux && area_of_part(model, &l.from.name).is_some_and(in_vpc));
    if aux_from_vpc {
        diag.notes.push(rules.text("note.aux_not_routed", lang));
    }
    if routes.iter().any(|(id, _)| def.choose.get(id).map(String::as_str) == Some("endpoint")) {
        diag.notes.push(rules.text("note.endpoint_not_drawn", lang));
    }
}

/// The area of a written part: that of its nodes, which its copies share.
fn area_of_part(model: &Model, part: &str) -> Option<Area> {
    model.nodes.iter().find(|n| n.part == part && !n.added).map(|n| n.area)
}
