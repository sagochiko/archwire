//! What a definition can use: the services, the accepted connections and the settings, taken
//! from the rules data and from the checks of the settings.

use crate::def::Rel;
use crate::diag::Versions;
use crate::rules::{Lang, Rules, Scope};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListedService {
    pub name: String,
    pub label: String,
    pub kinds: Vec<String>,
    pub in_vpc: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListedLink {
    pub from: String,
    pub to: String,
    pub symbol: String,
    pub label: String,
    pub kind_labels: BTreeMap<String, String>,
    pub role_labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListedSetting {
    pub key: String,
    pub values: Vec<String>,
    pub vpc_only: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct List {
    pub versions: Versions,
    pub services: Vec<ListedService>,
    pub links: Vec<ListedLink>,
    pub settings: Vec<ListedSetting>,
}

impl List {
    /// Pretty JSON (2-space indent) with a trailing LF, as the report is written.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("list serialize");
        s.push('\n');
        s
    }
}

/// The list for the built-in rules.
pub fn list() -> List {
    list_with(&Rules::builtin())
}

/// Services a definition can write in the order of their names, connections in the order of
/// links.toml, settings in the order of their keys. Labels are in English.
pub(crate) fn list_with(rules: &Rules) -> List {
    let services = rules
        .services()
        .filter(|s| !s.derived_only)
        .map(|s| ListedService {
            name: s.key.clone(),
            label: rules.text(&s.label, Lang::En),
            kinds: s.options.keys().cloned().collect(),
            in_vpc: s.scope == Scope::Vpc,
        })
        .collect();
    let links = rules
        .links()
        .iter()
        .map(|l| ListedLink {
            from: l.from.clone(),
            to: l.to.clone(),
            symbol: match l.rel {
                Rel::Main => "->",
                Rel::Aux => "~>",
                Rel::Assoc => "--",
            }
            .to_string(),
            label: rules.text(&l.label, Lang::En),
            kind_labels: l.to_kind.iter().map(|(kind, note)| (kind.clone(), rules.text(&note.label, Lang::En))).collect(),
            role_labels: l.to_role.iter().map(|(role, label)| (role.clone(), rules.text(label, Lang::En))).collect(),
        })
        .collect();
    let settings = crate::inference::settings()
        .into_iter()
        .map(|(key, values, vpc_only)| ListedSetting { key: key.to_string(), values, vpc_only })
        .collect();
    List {
        versions: Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() },
        services,
        links,
        settings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fails if a setting the checks accept is left out of the list, or a listed value is refused.
    #[test]
    fn every_listed_setting_value_is_accepted() {
        let list = list();
        for s in &list.settings {
            for v in &s.values {
                let src = format!("flows:\n  - user -> alb -> ecs\nset:\n  azs: 3\n  {}: {v}\n", s.key);
                let src = if s.key == "azs" { format!("flows:\n  - user -> alb -> ecs\nset:\n  azs: {v}\n") } else { src };
                let out = crate::compile(&src);
                assert!(out.diagnostics.errors.is_empty(), "{src}: {:?}", out.diagnostics.errors);
            }
        }
        let flow = crate::compile("flows:\n  - user -> cloudfront\nset:\n  colour: red\n");
        let allowed: Vec<String> = list.settings.iter().filter(|s| !s.vpc_only).map(|s| s.key.clone()).collect();
        assert_eq!(flow.diagnostics.errors[0].allowed, allowed);
        let vpc = crate::compile("flows:\n  - user -> alb -> ecs\nset:\n  colour: red\n");
        let allowed: Vec<String> = list.settings.iter().map(|s| s.key.clone()).collect();
        assert_eq!(vpc.diagnostics.errors[0].allowed, allowed);
    }
}
