//! Knowledge data: services, rules and labels, loaded from TOML.
//! Every rule of kind `constraint` must carry the URL of the AWS documentation that states it.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diag::{DerivedKind, Problem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope { Outside, Global, Regional, Vpc }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Subnet { Public, Private }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place { Single, Gateway, Span, PerAz, WriterReader }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang { En, Ja }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconKind { Service, Resource }

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServiceOption {
    pub shape: String,
    pub label: String,
}

/// `label` is either a proper name ("Amazon S3") or a key of labels.toml; resolve it with `Rules::text`.
/// `derived_only` services are added by inference only and may not be written in a definition.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Service {
    pub key: String,
    pub scope: Scope,
    #[serde(default)]
    pub subnet: Option<Subnet>,
    pub place: Place,
    pub icon: IconKind,
    pub shape: String,
    pub color: String,
    pub label: String,
    #[serde(default)]
    pub options: BTreeMap<String, ServiceOption>,
    #[serde(default)]
    pub in_label: Option<String>,
    #[serde(default)]
    pub derived_only: bool,
    /// A global service that may start a main or auxiliary flow.
    #[serde(default)]
    pub flow_source: bool,
    /// For an attachment, the services it may be associated with; empty for every other service.
    #[serde(default)]
    pub attach_to: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub kind: DerivedKind,
    pub source: Option<String>,
    pub reason_en: String,
    pub reason_ja: String,
}

#[derive(Deserialize)]
struct ServicesFile {
    #[serde(default)]
    service: Vec<Service>,
}

#[derive(Deserialize)]
struct RuleRaw {
    id: String,
    kind: String,
    #[serde(default)]
    source: Option<String>,
    reason_en: String,
    reason_ja: String,
}

#[derive(Deserialize)]
struct RulesFile {
    version: String,
    #[serde(default)]
    rule: Vec<RuleRaw>,
}

#[derive(Deserialize)]
struct LabelRaw {
    en: String,
    ja: String,
}

pub struct Rules {
    pub version: String,
    services: BTreeMap<String, Service>,
    rules: BTreeMap<String, Rule>,
    labels: BTreeMap<String, LabelRaw>,
}

fn problem(code: &str, at: &str, message: String) -> Problem {
    Problem { code: code.into(), at: Some(at.into()), message, ..Default::default() }
}

impl Rules {
    pub fn builtin() -> Rules {
        Rules::load(
            include_str!("../../../rules/aws/services.toml"),
            include_str!("../../../rules/aws/rules.toml"),
            include_str!("../../../rules/aws/labels.toml"),
        )
        .unwrap_or_else(|e| panic!("built-in rules are invalid: {e:?}"))
    }

    /// Empty `services` or `labels` are accepted; `rules` is always validated.
    pub fn load(services: &str, rules: &str, labels: &str) -> Result<Rules, Vec<Problem>> {
        let mut problems = Vec::new();

        let services = match toml::from_str::<ServicesFile>(services) {
            Ok(f) => f.service,
            Err(e) => {
                problems.push(problem("SERVICES_PARSE", "services.toml", e.to_string()));
                Vec::new()
            }
        };
        let labels = match toml::from_str::<BTreeMap<String, LabelRaw>>(labels) {
            Ok(m) => m,
            Err(e) => {
                problems.push(problem("LABELS_PARSE", "labels.toml", e.to_string()));
                BTreeMap::new()
            }
        };
        let (version, raw_rules) = match toml::from_str::<RulesFile>(rules) {
            Ok(f) => (f.version, f.rule),
            Err(e) => {
                problems.push(problem("RULES_PARSE", "rules.toml", e.to_string()));
                (String::new(), Vec::new())
            }
        };

        let mut rule_map = BTreeMap::new();
        for r in raw_rules {
            let kind = match r.kind.as_str() {
                "constraint" => DerivedKind::Constraint,
                "default" => DerivedKind::Default,
                other => {
                    problems.push(problem("RULE_BAD_KIND", &r.id, format!("unknown rule kind `{other}`")));
                    continue;
                }
            };
            if kind == DerivedKind::Constraint && r.source.as_deref().is_none_or(|s| s.trim().is_empty()) {
                problems.push(problem(
                    "RULE_WITHOUT_SOURCE",
                    &r.id,
                    "a constraint rule must have the URL of the AWS documentation that states it".into(),
                ));
            }
            let id = r.id.clone();
            let rule = Rule { id: r.id, kind, source: r.source, reason_en: r.reason_en, reason_ja: r.reason_ja };
            if rule_map.insert(id.clone(), rule).is_some() {
                problems.push(problem("RULE_DUPLICATE", &id, "duplicate rule id".into()));
            }
        }

        let mut service_map = BTreeMap::new();
        for s in services {
            let key = s.key.clone();
            if service_map.insert(key.clone(), s).is_some() {
                problems.push(problem("SERVICE_DUPLICATE", &key, "duplicate service key".into()));
            }
        }

        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(Rules { version, services: service_map, rules: rule_map, labels })
    }

    pub fn service(&self, key: &str) -> Option<&Service> {
        self.services.get(key)
    }

    /// Keys in lexicographic order.
    pub fn service_keys(&self) -> Vec<String> {
        self.services.keys().cloned().collect()
    }

    /// Panics on an unknown id: that is a bug in the program, not in the input.
    pub fn rule(&self, id: &str) -> &Rule {
        self.rules.get(id).unwrap_or_else(|| panic!("unknown rule id `{id}`"))
    }

    /// A word from labels.toml; an unknown key comes back unchanged.
    pub fn text(&self, key: &str, lang: Lang) -> String {
        match self.labels.get(key) {
            Some(l) => match lang {
                Lang::En => l.en.clone(),
                Lang::Ja => l.ja.clone(),
            },
            None => key.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_the_v01_services() {
        let r = Rules::builtin();
        assert_eq!(r.service_keys(), ["acm","alb","aurora","cloudfront","cloudwatch","ecs","igw","internet","nat","s3","user","waf"]);
        let alb = r.service("alb").unwrap();
        assert_eq!((alb.scope, alb.subnet, alb.place), (Scope::Vpc, Some(Subnet::Public), Place::Span));
        assert_eq!(r.service("ecs").unwrap().options["fargate"].shape, "fargate");
        assert_eq!(r.text("public_subnet", Lang::Ja), "パブリックサブネット");
        assert_eq!(r.text("public_subnet", Lang::En), "Public subnet");
        assert!(!r.version.is_empty());
    }

    #[test]
    fn every_constraint_rule_has_an_https_source() {
        let r = Rules::builtin();
        for id in ["global-service-outside-region","cloudfront-certificate-in-us-east-1","load-balancer-in-vpc","alb-spans-two-azs","internet-facing-alb-in-vpc-with-igw","private-egress-through-nat"] {
            let rule = r.rule(id);
            assert_eq!(rule.kind, DerivedKind::Constraint);
            assert!(rule.source.as_deref().unwrap().starts_with("https://docs.aws.amazon.com/"), "{id}");
        }
    }

    #[test]
    fn constraint_without_source_is_rejected_at_load() {
        let rules = "version = \"t\"\n[[rule]]\nid = \"x\"\nkind = \"constraint\"\nreason_en = \"a\"\nreason_ja = \"b\"\n";
        let e = Rules::load("", rules, "").err().unwrap();
        assert_eq!(e[0].code, "RULE_WITHOUT_SOURCE");
    }

    #[test]
    fn duplicate_rules_bad_kinds_and_duplicate_services_are_rejected_at_load() {
        let rule = |id: &str, kind: &str| format!("[[rule]]\nid = \"{id}\"\nkind = \"{kind}\"\nreason_en = \"a\"\nreason_ja = \"b\"\n");
        let codes = |e: Vec<Problem>| e.into_iter().map(|p| (p.code, p.at.unwrap_or_default())).collect::<Vec<_>>();
        let dup = format!("version = \"t\"\n{}{}", rule("x", "default"), rule("x", "default"));
        assert_eq!(codes(Rules::load("", &dup, "").err().unwrap()), [("RULE_DUPLICATE".to_string(), "x".to_string())]);
        let bad = format!("version = \"t\"\n{}", rule("y", "guess"));
        assert_eq!(codes(Rules::load("", &bad, "").err().unwrap()), [("RULE_BAD_KIND".to_string(), "y".to_string())]);
        let service = "[[service]]\nkey = \"s3\"\nscope = \"regional\"\nplace = \"single\"\nicon = \"service\"\nshape = \"s3\"\ncolor = \"#7AA116\"\nlabel = \"Amazon S3\"\n";
        let services = format!("{service}{service}");
        assert_eq!(codes(Rules::load(&services, "version = \"t\"\n", "").err().unwrap()), [("SERVICE_DUPLICATE".to_string(), "s3".to_string())]);
    }

    #[test]
    fn regional_placement_is_a_default_without_source() {
        let rule = Rules::builtin().rule("regional-service-outside-vpc").clone();
        assert_eq!(rule.kind, DerivedKind::Default);
        assert_eq!(rule.source, None);
    }

    #[test]
    fn in_label_is_set_for_connected_services_only() {
        let r = Rules::builtin();
        assert_eq!(r.service("s3").unwrap().in_label.as_deref(), Some("read_write"));
        assert_eq!(r.service("aurora").unwrap().in_label, None);
    }

    #[test]
    fn derived_only_marks_igw_and_nat() {
        let r = Rules::builtin();
        let only: Vec<_> = r.service_keys().into_iter().filter(|k| r.service(k).unwrap().derived_only).collect();
        assert_eq!(only, ["igw", "nat"]);
    }

    #[test]
    fn unknown_label_key_comes_back_unchanged() {
        assert_eq!(Rules::builtin().text("Amazon S3", Lang::Ja), "Amazon S3");
        assert_eq!(Rules::builtin().text("Amazon ECS (Fargate)", Lang::Ja), "Amazon ECS（Fargate）");
    }
}
