//! Knowledge data: services, rules and labels, loaded from TOML.
//! Every rule of kind `constraint` must carry the URL of the AWS documentation that states it.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::def::Rel;
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

/// The part a derived service plays where traffic enters or leaves the VPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedRole { EntryGateway, EgressNat }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IconKind { Service, Resource }

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceOption {
    pub shape: String,
    pub label: String,
}

/// `label` is either a proper name ("Amazon S3") or a key of labels.toml; resolve it with `Rules::text`.
/// `derived_only` services are added by inference only and may not be written in a definition.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// A part of this service written without a kind is a hole.
    #[serde(default)]
    pub kind_hole: bool,
    /// Cannot be written in one definition together with parts placed in a VPC yet.
    #[serde(default)]
    pub separate_from_vpc: bool,
    /// Main flows start here: every main flow must be reachable from a part of such a service.
    #[serde(default)]
    pub origin: bool,
    /// The rule that gives the reason for where the part is drawn; without it the scope decides.
    #[serde(default)]
    pub place_rule: Option<String>,
    /// For a service inference adds: the part it plays at the VPC's edge.
    #[serde(default)]
    pub derived_role: Option<DerivedRole>,
    /// A main flow to this service is traffic leaving the VPC to the internet; it is not drawn as a part.
    #[serde(default)]
    pub egress: bool,
    /// The reasons the VPC and its zones get when this part is the first in the definition that has them.
    #[serde(default)]
    pub vpc_reasons: Option<VpcReasons>,
    /// The reason a part of this service gets when its traffic leaves the VPC through a NAT gateway.
    #[serde(default)]
    pub nat_rule: Option<String>,
}

/// Rule ids: why a VPC is drawn, why it has its zones, and why it has the default number of them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpcReasons {
    pub vpc: String,
    pub az: Vec<String>,
    pub default_azs: String,
}

/// The roles a line inference adds may join.
const DERIVED_ROLES: [&str; 5] = ["private", "egress_nat", "entry_gateway", "writer", "reader"];

/// The roles a connection may give a label for, per role of its `to` part.
const TO_ROLES: [&str; 2] = ["writer", "reader"];

/// A line between parts inference adds: `from` and `to` are roles ("private", "egress_nat", "entry_gateway", "writer", "reader").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedLink {
    pub from: String,
    pub to: String,
    pub rel: Rel,
    /// A key of labels.toml.
    pub label: String,
}

/// The label and the reason a connection gets when its target has a given kind.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindNote {
    pub label: String,
    pub rule: String,
}

/// One accepted connection between two services, from links.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRule {
    pub from: String,
    pub to: String,
    pub rel: Rel,
    /// A key of labels.toml, also used while the kind of `to` is not decided.
    pub label: String,
    /// Per kind of `to`: the label of the connection and the reason given to `to`.
    pub to_kind: BTreeMap<String, KindNote>,
    /// How many `from` parts one `to` part may have, and the rule that says so.
    pub max_from: Option<usize>,
    pub max_rule: Option<String>,
    /// The Region the `from` part lives in, and the rule that places it there.
    pub from_region: Option<String>,
    pub from_rule: Option<String>,
    /// Per role of `to` ("writer", "reader"): the label key of the connection.
    pub to_role: BTreeMap<String, String>,
    /// With this connection the `from` part is placed in this subnet tier of the VPC, one copy per zone,
    /// with `from_subnet_rule` giving why it is in the VPC and `from_copies_rule` why there is one per zone.
    pub from_subnet: Option<Subnet>,
    pub from_subnet_rule: Option<String>,
    pub from_copies_rule: Option<String>,
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
#[serde(deny_unknown_fields)]
struct ServicesFile {
    #[serde(default)]
    service: Vec<Service>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleRaw {
    id: String,
    kind: String,
    #[serde(default)]
    source: Option<String>,
    reason_en: String,
    reason_ja: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    version: String,
    #[serde(default)]
    rule: Vec<RuleRaw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkRaw {
    from: String,
    to: String,
    rel: String,
    label: String,
    #[serde(default)]
    to_kind: BTreeMap<String, KindNote>,
    #[serde(default)]
    max_from: Option<usize>,
    #[serde(default)]
    max_rule: Option<String>,
    #[serde(default)]
    from_region: Option<String>,
    #[serde(default)]
    from_rule: Option<String>,
    #[serde(default)]
    to_role: BTreeMap<String, String>,
    #[serde(default)]
    from_subnet: Option<Subnet>,
    #[serde(default)]
    from_subnet_rule: Option<String>,
    #[serde(default)]
    from_copies_rule: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DerivedLinkRaw {
    from: String,
    to: String,
    rel: String,
    label: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LinksFile {
    #[serde(default)]
    link: Vec<LinkRaw>,
    #[serde(default)]
    derived_link: Vec<DerivedLinkRaw>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelRaw {
    en: String,
    ja: String,
}

pub struct Rules {
    pub version: String,
    services: BTreeMap<String, Service>,
    rules: BTreeMap<String, Rule>,
    labels: BTreeMap<String, LabelRaw>,
    links: Vec<LinkRule>,
    derived_links: Vec<DerivedLink>,
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
            include_str!("../../../rules/aws/links.toml"),
        )
        .unwrap_or_else(|e| panic!("built-in rules are invalid: {e:?}"))
    }

    /// Empty `services`, `labels` or `links` are accepted; `rules` is always validated.
    pub fn load(services: &str, rules: &str, labels: &str, links: &str) -> Result<Rules, Vec<Problem>> {
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

        for s in service_map.values() {
            let named = s
                .place_rule
                .iter()
                .map(|id| ("place_rule", id))
                .chain(s.nat_rule.iter().map(|id| ("nat_rule", id)))
                .chain(s.vpc_reasons.iter().flat_map(|v| {
                    std::iter::once(&v.vpc).chain(v.az.iter()).chain(std::iter::once(&v.default_azs)).map(|id| ("vpc_reasons", id))
                }));
            for (field, id) in named.filter(|(_, id)| !rule_map.contains_key(*id)) {
                problems.push(problem("SERVICE_UNKNOWN_RULE", &s.key, format!("{field} `{id}` is not in rules.toml")));
            }
            // A part in the VPC is drawn in a subnet tier unless it is a gateway on the VPC's edge.
            let in_vpc = s.scope == Scope::Vpc;
            if in_vpc && s.place != Place::Gateway && s.subnet.is_none() {
                problems.push(problem("SERVICE_NO_SUBNET", &s.key, "a VPC service that is not a gateway needs `subnet`".into()));
            }
            if !in_vpc && s.subnet.is_some() {
                problems.push(problem("SERVICE_SUBNET_OUTSIDE_VPC", &s.key, "only a VPC service may have `subnet`".into()));
            }
            if !in_vpc && s.vpc_reasons.is_some() {
                problems.push(problem("SERVICE_VPC_REASONS_OUTSIDE_VPC", &s.key, "only a VPC service may have `vpc_reasons`".into()));
            }
        }

        let (raw_links, raw_derived) = match toml::from_str::<LinksFile>(links) {
            Ok(f) => (f.link, f.derived_link),
            Err(e) => {
                problems.push(problem("LINKS_PARSE", "links.toml", e.to_string()));
                (Vec::new(), Vec::new())
            }
        };
        let link_list = check_links(raw_links, &service_map, &rule_map, &labels, &mut problems);
        let derived_links = check_derived_links(raw_derived, &labels, &mut problems);

        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(Rules { version, services: service_map, rules: rule_map, labels, links: link_list, derived_links })
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

    /// RULE_MISSING for every id that rules.toml does not have. Inference calls this with the ids
    /// its code names, before it uses any of them, so missing data is an error instead of a panic.
    pub fn require(&self, ids: &[&str]) -> Vec<Problem> {
        ids.iter()
            .filter(|id| !self.rules.contains_key(**id))
            .map(|id| problem("RULE_MISSING", id, format!("rule `{id}` is not in rules.toml")))
            .collect()
    }

    /// The accepted connection from one service to another, if there is one.
    pub fn link(&self, from_service: &str, to_service: &str, rel: Rel) -> Option<&LinkRule> {
        self.links.iter().find(|l| l.from == from_service && l.to == to_service && l.rel == rel)
    }

    /// The line inference adds from a part of one role to a part of another, if there is one.
    pub fn derived_link(&self, from_role: &str, to_role: &str) -> Option<&DerivedLink> {
        self.derived_links.iter().find(|l| l.from == from_role && l.to == to_role)
    }

    /// Whether labels.toml has this key.
    pub fn has_text(&self, key: &str) -> bool {
        self.labels.contains_key(key)
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

/// Every name a link uses must exist in the other files, and each pair and relation is listed once.
fn parse_rel(rel: &str, at: &str, problems: &mut Vec<Problem>) -> Option<Rel> {
    match rel {
        "main" => Some(Rel::Main),
        "aux" => Some(Rel::Aux),
        "assoc" => Some(Rel::Assoc),
        other => {
            problems.push(problem("LINK_BAD_REL", at, format!("unknown relation `{other}`; use main, aux or assoc")));
            None
        }
    }
}

fn check_links(
    raw: Vec<LinkRaw>,
    services: &BTreeMap<String, Service>,
    rules: &BTreeMap<String, Rule>,
    labels: &BTreeMap<String, LabelRaw>,
    problems: &mut Vec<Problem>,
) -> Vec<LinkRule> {
    let mut links: Vec<LinkRule> = Vec::new();
    for l in raw {
        let at = format!("{} {} {}", l.from, l.rel, l.to);
        let before = problems.len();
        for key in [&l.from, &l.to] {
            if !services.contains_key(key) {
                problems.push(problem("LINK_UNKNOWN_SERVICE", &at, format!("unknown service `{key}`")));
            }
        }
        let rel = parse_rel(&l.rel, &at, problems);
        for label in std::iter::once(&l.label).chain(l.to_kind.values().map(|k| &k.label)).chain(l.to_role.values()) {
            if !labels.contains_key(label) {
                problems.push(problem("LINK_UNKNOWN_LABEL", &at, format!("label `{label}` is not in labels.toml")));
            }
        }
        if let Some(to) = services.get(&l.to) {
            for kind in l.to_kind.keys().filter(|k| !to.options.contains_key(*k)) {
                problems.push(problem("LINK_UNKNOWN_KIND", &at, format!("`{}` has no kind `{kind}`", l.to)));
            }
        }
        for role in l.to_role.keys().filter(|r| !TO_ROLES.contains(&r.as_str())) {
            problems.push(problem("LINK_BAD_ROLE", &at, format!("unknown role `{role}` in `to_role`; use writer or reader")));
        }
        let named = l
            .to_kind
            .values()
            .map(|k| &k.rule)
            .chain(l.max_rule.iter())
            .chain(l.from_rule.iter())
            .chain(l.from_subnet_rule.iter())
            .chain(l.from_copies_rule.iter());
        for rule in named {
            if !rules.contains_key(rule) {
                problems.push(problem("LINK_UNKNOWN_RULE", &at, format!("rule `{rule}` is not in rules.toml")));
            }
        }
        if let Some(rel) = rel {
            if links.iter().any(|k| k.from == l.from && k.to == l.to && k.rel == rel) {
                problems.push(problem("LINK_DUPLICATE", &at, "the same pair and relation is listed twice".into()));
            }
        }
        if l.max_from.is_some() != l.max_rule.is_some() || l.from_region.is_some() != l.from_rule.is_some() {
            problems.push(problem(
                "LINK_BAD_LIMIT",
                &at,
                "`max_from` needs `max_rule` and `from_region` needs `from_rule`, and the other way round".into(),
            ));
        }
        let tier = [l.from_subnet.is_some(), l.from_subnet_rule.is_some(), l.from_copies_rule.is_some()];
        if tier.contains(&true) && tier.contains(&false) {
            problems.push(problem(
                "LINK_BAD_LIMIT",
                &at,
                "`from_subnet`, `from_subnet_rule` and `from_copies_rule` are written together or not at all".into(),
            ));
        }
        if l.from_subnet.is_some() && services.get(&l.to).is_some_and(|to| to.scope != Scope::Vpc) {
            problems.push(problem("LINK_SUBNET_WITHOUT_VPC", &at, format!("`from_subnet` needs `{}` to be a VPC service", l.to)));
        }
        if let (Some(rel), true) = (rel, problems.len() == before) {
            links.push(LinkRule {
                from: l.from,
                to: l.to,
                rel,
                label: l.label,
                to_kind: l.to_kind,
                max_from: l.max_from,
                max_rule: l.max_rule,
                from_region: l.from_region,
                from_rule: l.from_rule,
                to_role: l.to_role,
                from_subnet: l.from_subnet,
                from_subnet_rule: l.from_subnet_rule,
                from_copies_rule: l.from_copies_rule,
            });
        }
    }
    links
}

/// Every line inference adds joins two known roles, is listed once per pair and has a known label.
fn check_derived_links(raw: Vec<DerivedLinkRaw>, labels: &BTreeMap<String, LabelRaw>, problems: &mut Vec<Problem>) -> Vec<DerivedLink> {
    let mut links: Vec<DerivedLink> = Vec::new();
    for l in raw {
        let at = format!("derived_link {} {} {}", l.from, l.rel, l.to);
        let before = problems.len();
        for role in [&l.from, &l.to].into_iter().filter(|r| !DERIVED_ROLES.contains(&r.as_str())) {
            let allowed = DERIVED_ROLES.join(", ");
            problems.push(problem("DERIVED_LINK_BAD_ROLE", &at, format!("unknown role `{role}`; use one of {allowed}")));
        }
        let rel = parse_rel(&l.rel, &at, problems);
        if !labels.contains_key(&l.label) {
            problems.push(problem("LINK_UNKNOWN_LABEL", &at, format!("label `{}` is not in labels.toml", l.label)));
        }
        if links.iter().any(|k| k.from == l.from && k.to == l.to) {
            problems.push(problem("DERIVED_LINK_DUPLICATE", &at, "the same pair of roles is listed twice".into()));
        }
        if let (Some(rel), true) = (rel, problems.len() == before) {
            links.push(DerivedLink { from: l.from, to: l.to, rel, label: l.label });
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_the_v01_services() {
        let r = Rules::builtin();
        assert_eq!(r.service_keys(), ["acm","alb","apigateway","aurora","cloudfront","cloudwatch","cognito","dynamodb","ecs","igw","internet","lambda","nat","s3","user","waf"]);
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
        for id in ["cloudfront-certificate-in-us-east-1","load-balancer-in-vpc","alb-spans-two-azs","internet-facing-alb-in-vpc-with-igw","private-egress-through-nat","cognito-user-pool-in-one-region","one-web-acl-per-resource"] {
            let rule = r.rule(id);
            assert_eq!(rule.kind, DerivedKind::Constraint);
            assert!(rule.source.as_deref().unwrap().starts_with("https://docs.aws.amazon.com/"), "{id}");
        }
    }

    // Fails if the global placement is made a constraint again or loses its source.
    #[test]
    fn global_placement_is_a_default_with_its_source() {
        let r = Rules::builtin();
        let rule = r.rule("global-service-outside-region");
        assert_eq!(rule.kind, DerivedKind::Default);
        assert_eq!(rule.source.as_deref(), Some("https://docs.aws.amazon.com/whitepapers/latest/aws-fault-isolation-boundaries/global-services.html"));
    }

    #[test]
    fn constraint_without_source_is_rejected_at_load() {
        let rules = "version = \"t\"\n[[rule]]\nid = \"x\"\nkind = \"constraint\"\nreason_en = \"a\"\nreason_ja = \"b\"\n";
        let e = Rules::load("", rules, "", "").err().unwrap();
        assert_eq!(e[0].code, "RULE_WITHOUT_SOURCE");
    }

    #[test]
    fn duplicate_rules_bad_kinds_and_duplicate_services_are_rejected_at_load() {
        let rule = |id: &str, kind: &str| format!("[[rule]]\nid = \"{id}\"\nkind = \"{kind}\"\nreason_en = \"a\"\nreason_ja = \"b\"\n");
        let codes = |e: Vec<Problem>| e.into_iter().map(|p| (p.code, p.at.unwrap_or_default())).collect::<Vec<_>>();
        let dup = format!("version = \"t\"\n{}{}", rule("x", "default"), rule("x", "default"));
        assert_eq!(codes(Rules::load("", &dup, "", "").err().unwrap()), [("RULE_DUPLICATE".to_string(), "x".to_string())]);
        let bad = format!("version = \"t\"\n{}", rule("y", "guess"));
        assert_eq!(codes(Rules::load("", &bad, "", "").err().unwrap()), [("RULE_BAD_KIND".to_string(), "y".to_string())]);
        let service = "[[service]]\nkey = \"s3\"\nscope = \"regional\"\nplace = \"single\"\nicon = \"service\"\nshape = \"s3\"\ncolor = \"#7AA116\"\nlabel = \"Amazon S3\"\n";
        let services = format!("{service}{service}");
        assert_eq!(codes(Rules::load(&services, "version = \"t\"\n", "", "").err().unwrap()), [("SERVICE_DUPLICATE".to_string(), "s3".to_string())]);
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

    #[test]
    fn links_are_checked_at_load() {
        let services = include_str!("../../../rules/aws/services.toml");
        let rules = include_str!("../../../rules/aws/rules.toml");
        let labels = include_str!("../../../rules/aws/labels.toml");
        let links = include_str!("../../../rules/aws/links.toml");
        assert!(Rules::load(services, rules, labels, links).is_ok());
        let waf = "from = \"waf\"\nto = \"cloudfront\"\nrel = \"assoc\"\nlabel = \"web_acl\"\n";
        let acm = "from = \"acm\"\nto = \"cloudfront\"\nrel = \"assoc\"\nlabel = \"certificate\"\n";
        let broken = |from: &str, to: &str| {
            assert!(links.contains(from), "{from}");
            links.replacen(from, to, 1)
        };
        let cases = [
            (broken("from = \"cognito\"", "from = \"cognitoo\""), "LINK_UNKNOWN_SERVICE"),
            (broken("rel = \"assoc\"", "rel = \"near\""), "LINK_BAD_REL"),
            (broken("label = \"verify_token\"", "label = \"verify\""), "LINK_UNKNOWN_LABEL"),
            (broken("to_kind.http =", "to_kind.websocket ="), "LINK_UNKNOWN_KIND"),
            (broken("rule = \"http-api-jwt-authorizer\"", "rule = \"http-api-jwt\""), "LINK_UNKNOWN_RULE"),
            (format!("{links}\n[[link]]\n{waf}"), "LINK_DUPLICATE"),
            (broken(&format!("{acm}from_region = \"us-east-1\"\n"), acm), "LINK_BAD_LIMIT"),
            (broken("label = \"jwt_authorizer\"", "label = \"jwt_auth\""), "LINK_UNKNOWN_LABEL"),
            (broken("max_rule = \"one-web-acl-per-resource\"", "max_rule = \"one-web-acl\""), "LINK_UNKNOWN_RULE"),
            (broken("from_rule = \"cloudfront-certificate-in-us-east-1\"", "from_rule = \"cloudfront-certificate\""), "LINK_UNKNOWN_RULE"),
            (broken("from_rule = \"cloudfront-certificate-in-us-east-1\"\n", ""), "LINK_BAD_LIMIT"),
            (broken("max_from = 1\n", ""), "LINK_BAD_LIMIT"),
            (broken("max_rule = \"one-web-acl-per-resource\"\n", ""), "LINK_BAD_LIMIT"),
        ];
        assert_eq!(cases.len(), 13);
        for (text, code) in &cases {
            let e = Rules::load(services, rules, labels, text).err().unwrap_or_else(|| panic!("{code} not reported"));
            assert_eq!(e[0].code, *code);
        }
        let r = Rules::builtin();
        assert_eq!(r.link("cognito", "apigateway", Rel::Assoc).unwrap().to_kind["http"].rule, "http-api-jwt-authorizer");
        assert_eq!(r.link("cloudfront", "lambda", Rel::Main), None);
        assert!(r.has_text("invoke") && !r.has_text("Amazon S3"));
    }

    #[test]
    fn the_api_services_carry_their_properties() {
        let r = Rules::builtin();
        let api = r.service("apigateway").unwrap();
        assert!(api.kind_hole && api.separate_from_vpc && !api.origin);
        assert_eq!(api.options.keys().collect::<Vec<_>>(), ["http", "rest"]);
        assert_eq!(api.options["http"].label, "Amazon API Gateway\nHTTP API");
        assert_eq!(r.service("cognito").unwrap().place_rule.as_deref(), Some("cognito-user-pool-in-one-region"));
        assert!(r.service("user").unwrap().origin);
        let origins: Vec<_> = r.service_keys().into_iter().filter(|k| r.service(k).unwrap().origin).collect();
        assert_eq!(origins, ["user"]);
        let separate: Vec<_> = r.service_keys().into_iter().filter(|k| r.service(k).unwrap().separate_from_vpc).collect();
        assert_eq!(separate, ["apigateway", "cognito", "dynamodb", "lambda"]);
        assert!(!r.service("ecs").unwrap().kind_hole);
    }

    // Breaks when a gateway loses its derived role in the data, or a misspelt `derived_role` is silently dropped.
    #[test]
    fn derived_roles_mark_the_gateways() {
        let r = Rules::builtin();
        let marked: Vec<_> = r
            .service_keys()
            .into_iter()
            .filter_map(|k| r.service(&k).unwrap().derived_role.map(|role| (k, role)))
            .collect();
        assert_eq!(marked, [("igw".to_string(), DerivedRole::EntryGateway), ("nat".to_string(), DerivedRole::EgressNat)]);
        assert!(marked.iter().all(|(k, _)| r.service(k).unwrap().derived_only));

        let [services, rules, labels, links] = builtin_files();
        let from = "\nderived_role = \"entry_gateway\"";
        assert!(services.contains(from));
        let broken = services.replacen(from, "\nderived_roles = \"entry_gateway\"", 1);
        let e = Rules::load(&broken, rules, labels, links).err().expect("a misspelt field was accepted");
        assert_eq!(e[0].code, "SERVICES_PARSE");
    }

    fn builtin_files() -> [&'static str; 4] {
        [
            include_str!("../../../rules/aws/services.toml"),
            include_str!("../../../rules/aws/rules.toml"),
            include_str!("../../../rules/aws/labels.toml"),
            include_str!("../../../rules/aws/links.toml"),
        ]
    }

    #[test]
    fn a_place_rule_must_be_in_the_rules() {
        let [services, rules, labels, links] = builtin_files();
        let from = "place_rule = \"cognito-user-pool-in-one-region\"";
        assert!(services.contains(from));
        let broken = services.replacen(from, "place_rule = \"cognito-user-pool\"", 1);
        let e = Rules::load(&broken, rules, labels, links).err().unwrap();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref()), ("SERVICE_UNKNOWN_RULE", Some("cognito")));
    }

    // Breaks when a VPC part without a subnet tier is loaded, or a gateway is made to need one.
    #[test]
    fn a_vpc_service_outside_a_gateway_must_name_its_subnet() {
        let [services, rules, labels, links] = builtin_files();
        let r = Rules::load(services, rules, labels, links).expect("the builtin data loads");
        let igw = r.service("igw").unwrap();
        assert_eq!((igw.scope, igw.place, igw.subnet), (Scope::Vpc, Place::Gateway, None));
        let from = "key = \"ecs\"\nscope = \"vpc\"\nsubnet = \"private\"\n";
        assert!(services.contains(from));
        let broken = services.replacen(from, "key = \"ecs\"\nscope = \"vpc\"\n", 1);
        let e = Rules::load(&broken, rules, labels, links).err().expect("a VPC service without a subnet was accepted");
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref()), ("SERVICE_NO_SUBNET", Some("ecs")));
    }

    // Breaks when a subnet tier written on a service outside the VPC is silently ignored.
    #[test]
    fn a_service_outside_the_vpc_may_not_name_a_subnet() {
        let [services, rules, labels, links] = builtin_files();
        let from = "key = \"s3\"\nscope = \"regional\"\n";
        assert!(services.contains(from));
        let broken = services.replacen(from, "key = \"s3\"\nscope = \"regional\"\nsubnet = \"private\"\n", 1);
        let e = Rules::load(&broken, rules, labels, links).err().expect("a subnet outside the VPC was accepted");
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref()), ("SERVICE_SUBNET_OUTSIDE_VPC", Some("s3")));
    }

    #[test]
    fn a_misspelt_field_is_a_load_error() {
        let [services, rules, labels, links] = builtin_files();
        let cases = [
            (0, "\nto_kind.http =", "\nto_kinds.http =", "LINKS_PARSE"),
            (0, "\nmax_from = 1", "\nmax_form = 1", "LINKS_PARSE"),
            (1, "\nkind_hole = true", "\nkind_holes = true", "SERVICES_PARSE"),
            (1, "\nplace_rule =", "\nplace_rules =", "SERVICES_PARSE"),
            (1, "\nseparate_from_vpc = true", "\nseparate_from_vpcs = true", "SERVICES_PARSE"),
            (1, "\norigin = true", "\norigins = true", "SERVICES_PARSE"),
            (2, "\nreason_en =", "\nreason_eng =", "RULES_PARSE"),
            (3, "ja = \"利用者\"", "jp = \"利用者\"", "LABELS_PARSE"),
        ];
        for (file, from, to, code) in cases {
            let mut files = [links.to_string(), services.to_string(), rules.to_string(), labels.to_string()];
            assert!(files[file].contains(from), "{from}");
            files[file] = files[file].replacen(from, to, 1);
            let [l, sv, r, lb] = &files;
            let e = Rules::load(sv, r, lb, l).err().unwrap_or_else(|| panic!("{to} was accepted"));
            assert_eq!(e[0].code, code, "{to}");
        }
    }

    // Fails if a connection with a VPC part, or its label, is left out of links.toml.
    #[test]
    fn vpc_links_are_listed_once_with_their_labels() {
        let r = Rules::builtin();
        let (m, a) = (Rel::Main, Rel::Aux);
        let want = [
            ("user", "alb", m, "https"),
            ("cloudfront", "alb", m, "https"),
            ("alb", "ecs", m, "http"),
            ("ecs", "aurora", m, "read_write"),
            ("aurora", "ecs", m, "http"),
            ("ecs", "s3", m, "read_write"),
            ("ecs", "cloudwatch", m, "logs_metrics"),
            ("aurora", "s3", m, "read_write"),
            ("aurora", "cloudwatch", m, "logs_metrics"),
            ("ecs", "internet", m, "outbound"),
            ("aurora", "internet", m, "outbound"),
            ("cloudfront", "s3", a, "logs_metrics"),
            ("cloudfront", "cloudwatch", a, "logs_metrics"),
            ("alb", "s3", a, "logs_metrics"),
            ("alb", "cloudwatch", a, "logs_metrics"),
            ("ecs", "s3", a, "logs_metrics"),
            ("ecs", "cloudwatch", a, "logs_metrics"),
            ("aurora", "s3", a, "logs_metrics"),
            ("aurora", "cloudwatch", a, "logs_metrics"),
        ];
        assert_eq!(want.len(), 19);
        for (from, to, rel, label) in want {
            let listed = r.links.iter().filter(|l| l.from == from && l.to == to && l.rel == rel).count();
            assert_eq!(listed, 1, "{from} {rel:?} {to}");
            assert_eq!(r.link(from, to, rel).unwrap_or_else(|| panic!("{from} {rel:?} {to}")).label, label, "{from} {rel:?} {to}");
        }
        let roles = &r.link("ecs", "aurora", Rel::Main).unwrap().to_role;
        let roles: Vec<_> = roles.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(roles, [("reader", "read"), ("writer", "read_write")]);
    }

    // Fails if an auxiliary line from CloudFront is stopped, or drawn with a note, in a definition without a VPC.
    #[test]
    fn a_cloudfront_side_line_needs_no_vpc() {
        let defs = [
            "flows:\n  - user -> cloudfront -> s3\n  - cloudfront ~> cloudwatch\n",
            "flows:\n  - user -> cloudfront -> s3\n  - cloudfront ~> s3.logs\n",
        ];
        for def in defs {
            for direction in ["down", "right"] {
                let src = format!("{def}set:\n  direction: {direction}\n");
                let out = crate::compile(&src);
                assert_eq!(out.diagnostics.exit_code(), 0, "{src}{:?}", out.diagnostics.errors);
                assert!(out.drawio.is_some(), "{src}");
                assert!(out.diagnostics.notes.is_empty(), "{src}{:?}", out.diagnostics.notes);
            }
        }
    }

    // Fails if a line inference adds between roles loses its relation or label.
    #[test]
    fn derived_links_join_roles() {
        let r = Rules::builtin();
        let pairs = [("private", "egress_nat"), ("egress_nat", "entry_gateway"), ("writer", "reader")];
        let got: Vec<_> = pairs
            .iter()
            .map(|(from, to)| {
                let d = r.derived_link(from, to).unwrap_or_else(|| panic!("{from} {to}"));
                (d.from.as_str(), d.to.as_str(), d.rel, d.label.as_str())
            })
            .collect();
        assert_eq!(
            got,
            [
                ("private", "egress_nat", Rel::Main, "outbound"),
                ("egress_nat", "entry_gateway", Rel::Main, "outbound"),
                ("writer", "reader", Rel::Aux, "replication"),
            ]
        );
        assert!(r.derived_link("reader", "writer").is_none());
    }

    // Fails if a misspelt role, a subnet tier on a connection outside the VPC, a half-written subnet
    // tier, a derived line with a bad role, relation or label or listed twice, VPC reasons on a service
    // outside the VPC, or a rule missing from rules.toml is loaded without an error.
    #[test]
    fn bad_link_fields_stop_loading() {
        let [services, rules, labels, links] = builtin_files();
        let edit = |text: &str, from: &str, to: &str| {
            assert!(text.contains(from), "{from}");
            text.replacen(from, to, 1)
        };
        let lambda = "from = \"lambda\"\nto = \"dynamodb\"\nrel = \"main\"\nlabel = \"read_write\"\n";
        let alb = "from = \"alb\"\nto = \"ecs\"\nrel = \"main\"\nlabel = \"http\"\n";
        let tier = "from_subnet = \"private\"\nfrom_subnet_rule = \"private-subnet-per-az\"\nfrom_copies_rule = \"private-subnet-per-az\"\n";
        let derived = |from: &str, to: &str, rel: &str, label: &str| {
            format!("{links}\n[[derived_link]]\nfrom = \"{from}\"\nto = \"{to}\"\nrel = \"{rel}\"\nlabel = \"{label}\"\n")
        };
        let s3 = "label = \"Amazon S3\"\nin_label = \"read_write\"\n";
        let ecs = "label = \"Amazon ECS\"\nin_label = \"http\"\n";
        let reasons = "vpc_reasons = { vpc = \"load-balancer-in-vpc\", az = [\"alb-spans-two-azs\"], default_azs = \"default-two-azs\" }\n";
        let cases = [
            (services.to_string(), edit(links, "to_role.writer =", "to_role.primary ="), "LINK_BAD_ROLE"),
            (services.to_string(), edit(links, lambda, &format!("{lambda}{tier}")), "LINK_SUBNET_WITHOUT_VPC"),
            (services.to_string(), edit(links, alb, &format!("{alb}from_subnet = \"public\"\n")), "LINK_BAD_LIMIT"),
            (services.to_string(), derived("public", "reader", "aux", "replication"), "DERIVED_LINK_BAD_ROLE"),
            (services.to_string(), derived("writer", "reader", "aux", "replication"), "DERIVED_LINK_DUPLICATE"),
            (services.to_string(), derived("private", "reader", "near", "replication"), "LINK_BAD_REL"),
            (services.to_string(), derived("private", "reader", "aux", "replicate"), "LINK_UNKNOWN_LABEL"),
            (
                services.to_string(),
                edit(links, alb, &format!("{alb}{}", tier.replacen("from_subnet_rule = \"private-subnet-per-az\"", "from_subnet_rule = \"no-such-rule\"", 1))),
                "LINK_UNKNOWN_RULE",
            ),
            (edit(services, "vpc = \"load-balancer-in-vpc\"", "vpc = \"load-balancer\""), links.to_string(), "SERVICE_UNKNOWN_RULE"),
            (edit(services, ecs, &format!("{ecs}nat_rule = \"no-such-rule\"\n")), links.to_string(), "SERVICE_UNKNOWN_RULE"),
            (edit(services, s3, &format!("{s3}{reasons}")), links.to_string(), "SERVICE_VPC_REASONS_OUTSIDE_VPC"),
        ];
        assert_eq!(cases.len(), 11);
        for (services, links, code) in &cases {
            let e = Rules::load(services, rules, labels, links).err().unwrap_or_else(|| panic!("{code} not reported"));
            let codes: Vec<_> = e.iter().map(|p| p.code.as_str()).collect();
            assert_eq!(codes, [*code], "{e:?}");
        }
    }

    // Fails if the place traffic leaves the VPC for cannot be told from the data.
    #[test]
    fn the_internet_is_where_traffic_leaves() {
        let r = Rules::builtin();
        let egress: Vec<_> = r.service_keys().into_iter().filter(|k| r.service(k).unwrap().egress).collect();
        assert_eq!(egress, ["internet"]);
    }

    // Fails if the reasons the VPC and its zones get cannot be read from the load balancer's data.
    #[test]
    fn the_load_balancer_gives_the_vpc_its_reasons() {
        let r = Rules::builtin();
        let want = VpcReasons { vpc: "load-balancer-in-vpc".into(), az: vec!["alb-spans-two-azs".into()], default_azs: "default-two-azs".into() };
        assert_eq!(r.service("alb").unwrap().vpc_reasons.as_ref(), Some(&want));
        let with: Vec<_> = r.service_keys().into_iter().filter(|k| r.service(k).unwrap().vpc_reasons.is_some()).collect();
        assert_eq!(with, ["alb"]);
    }

    #[test]
    fn require_lists_the_missing_rules() {
        let r = Rules::builtin();
        assert!(r.require(&["default-region"]).is_empty());
        let e = r.require(&["default-region", "no-such-rule"]);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref()), ("RULE_MISSING", Some("no-such-rule")));
    }
}
