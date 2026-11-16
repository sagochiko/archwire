//! The reasons for every frame, place and kind archwire decides, and the holes left open.

use super::build::moved;
use crate::def::{Definition, Link, Ref};
use crate::diag::{Derived, Diagnostics, Hole};
use crate::model::{Area, Model};
use crate::rules::{Lang, LinkRule, Rules, Scope};

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

/// The reasons of a model without a VPC, then its open holes.
pub(super) fn explain(
    def: &Definition,
    rules: &Rules,
    model: &Model,
    parts: &[&Ref],
    accepted: &[(&Link, &LinkRule)],
    open: &[(String, &Ref)],
    diag: &mut Diagnostics,
) {
    let lang = model.lang;
    let svc = |name: &str| rules.service(name).expect("names were checked");
    let kind_of = |name: &str| model.nodes.iter().find(|n| n.id == name).and_then(|n| n.option.clone());

    let mut why = |target: &str, id: &str, added: bool| diag.derived.push(derived(rules, lang, target, id, added));
    why("aws-cloud", "resources-inside-aws-cloud-frame", true);
    // The Region frame, and the Region chosen for it, only when a part is drawn inside it.
    if model.nodes.iter().any(|n| n.area == Area::Region) {
        why("region", "resources-inside-region-frame", true);
        if !def.set.contains_key("region") {
            why("region", "default-region", true);
        }
    }
    for r in parts {
        let s = svc(&r.service);
        let place = match (moved(accepted, &r.name), &s.place_rule, s.scope) {
            (Some(rule), _, _) => rule.from_rule.clone(),
            (None, Some(id), _) => Some(id.clone()),
            (None, None, Scope::Global) => Some("global-service-outside-region".into()),
            (None, None, Scope::Regional) => Some("regional-service-outside-vpc".into()),
            (None, None, Scope::Outside | Scope::Vpc) => None,
        };
        if let Some(id) = place {
            why(&r.name, &id, false);
        }
        let Some(kind) = kind_of(&r.name) else { continue };
        let mut given: Vec<&str> = Vec::new();
        for (_, rule) in accepted.iter().filter(|(l, _)| l.to.name == r.name) {
            let Some(note) = rule.to_kind.get(&kind) else { continue };
            if !given.contains(&note.rule.as_str()) {
                given.push(&note.rule);
                why(&r.name, &note.rule, false);
            }
        }
    }

    for (id, r) in open.iter().filter(|(id, _)| !def.choose.contains_key(id)) {
        diag.holes.push(Hole {
            hole: id.clone(),
            at: r.at.clone(),
            need: rules.text("need.kind", lang).replace("{part}", &r.name),
            choices: svc(&r.service).options.keys().cloned().collect(),
        });
    }
}
