//! Definition parser: reads the short line-based definition into a `Definition`.
//!
//! Only a small subset of YAML-like syntax is accepted, so that edits can be
//! done as line operations. No YAML crate is used.

use crate::diag::Problem;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rel {
    Main,
    Aux,
    Assoc,
}

/// The longest name a part may carry after the dot.
pub const MAX_TAG: usize = 24;

#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    /// As written, e.g. "s3.web" or "apigateway". A part is identified by this.
    pub name: String,
    /// The part before the dot, e.g. "s3".
    pub service: String,
    /// The part after the dot, e.g. "web".
    pub tag: Option<String>,
    pub option: Option<String>,
    /// Position of this part, e.g. `flows[0][1]`.
    pub at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub from: Ref,
    pub to: Ref,
    pub rel: Rel,
    /// Position of the flow line, e.g. `flows[0]`.
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Definition {
    /// In written order, duplicates removed.
    pub links: Vec<Link>,
    pub set: BTreeMap<String, String>,
    pub choose: BTreeMap<String, String>,
}

const SECTIONS: [&str; 3] = ["choose", "flows", "set"];

#[derive(Clone, Copy, PartialEq)]
enum Section {
    None,
    Flows,
    Set,
    Choose,
}

fn problem(code: &str, at: &str, got: &str, allowed: &[&str], message: String) -> Problem {
    Problem {
        code: code.into(),
        at: Some(at.into()),
        got: Some(got.into()),
        allowed: allowed.iter().map(|s| s.to_string()).collect(),
        rule: None,
        message,
    }
}

fn is_word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `[a-z0-9-]+`, not starting or ending with a hyphen. `--` is never part of a name: in a flow
/// line it is always the association operator.
fn is_tag(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("--")
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `service[.tag][(kind)]`. The length of the tag is not checked here.
pub(crate) fn parse_part(text: &str, at: String) -> Option<Ref> {
    let t = text.trim();
    let (base, option) = match t.find('(') {
        Some(open) => (&t[..open], Some(t[open + 1..].strip_suffix(')')?)),
        None => (t, None),
    };
    if option.is_some_and(|o| !is_word(o)) {
        return None;
    }
    let (service, tag) = match base.split_once('.') {
        Some((service, tag)) => (service, Some(tag)),
        None => (base, None),
    };
    if !is_word(service) || tag.is_some_and(|g| !is_tag(g)) {
        return None;
    }
    Some(Ref {
        name: base.into(),
        service: service.into(),
        tag: tag.map(String::from),
        option: option.map(String::from),
        at,
    })
}

/// Splits `a -> b ~> c` into parts and the operators between them.
pub(crate) fn split_flow(body: &str) -> (Vec<&str>, Vec<Rel>) {
    let bytes = body.as_bytes();
    let (mut parts, mut rels) = (Vec::new(), Vec::new());
    let (mut start, mut i) = (0, 0);
    while i + 1 < bytes.len() {
        let rel = match &bytes[i..i + 2] {
            b"->" => Some(Rel::Main),
            b"~>" => Some(Rel::Aux),
            b"--" => Some(Rel::Assoc),
            _ => None,
        };
        if let Some(rel) = rel {
            parts.push(&body[start..i]);
            rels.push(rel);
            i += 2;
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&body[start..]);
    (parts, rels)
}

fn parse_flow_line(body: &str, index: usize, errors: &mut Vec<Problem>) -> Vec<Link> {
    let at = format!("flows[{index}]");
    let bad = |errors: &mut Vec<Problem>| {
        errors.push(problem(
            "BAD_SYNTAX",
            &at,
            body,
            &[],
            "a flow line is `- a -> b ~> c -- d`; a part is service[.name][(kind)], with service and kind in [a-z0-9_] and name in [a-z0-9-], not starting or ending with a hyphen".into(),
        ));
        Vec::new()
    };
    let (texts, rels) = split_flow(body);
    if texts.len() < 2 {
        return bad(errors);
    }
    let mut refs = Vec::new();
    for (j, t) in texts.iter().enumerate() {
        match parse_part(t, format!("{at}[{j}]")) {
            Some(r) => refs.push(r),
            None => return bad(errors),
        }
    }
    let mut too_long = false;
    for r in &refs {
        if let Some(tag) = r.tag.as_deref().filter(|g| g.chars().count() > MAX_TAG) {
            too_long = true;
            errors.push(problem(
                "NAME_TOO_LONG",
                &r.at,
                tag,
                &[],
                format!("a part name after the dot is at most {MAX_TAG} characters"),
            ));
        }
    }
    if too_long {
        return Vec::new();
    }
    let mut links = Vec::new();
    for (k, rel) in rels.into_iter().enumerate() {
        links.push(Link { from: refs[k].clone(), to: refs[k + 1].clone(), rel, at: at.clone() });
    }
    links
}

fn parse_entry(
    body: &str,
    at: String,
    section: &str,
    map: &mut BTreeMap<String, String>,
    errors: &mut Vec<Problem>,
) {
    let entry = body.split_once(':').and_then(|(k, v)| {
        let (k, v) = (k.trim(), v.trim());
        (!k.is_empty() && !v.is_empty() && !k.contains(char::is_whitespace)).then_some((k, v))
    });
    match entry {
        Some((k, _)) if map.contains_key(k) => errors.push(problem(
            "DUPLICATE_KEY",
            &format!("{section}.{k}"),
            k,
            &[],
            format!("`{k}` is written twice in `{section}`"),
        )),
        Some((k, v)) => {
            map.insert(k.into(), v.into());
        }
        None => errors.push(problem("BAD_SYNTAX", &at, body, &[], "expected `key: value`".into())),
    }
}

pub fn parse(text: &str) -> Result<Definition, Vec<Problem>> {
    let mut def = Definition::default();
    let mut errors: Vec<Problem> = Vec::new();
    let mut section = Section::None;
    let (mut n_flows, mut n_set, mut n_choose) = (0usize, 0usize, 0usize);
    let mut options: BTreeMap<String, String> = BTreeMap::new();

    for (lineno, raw) in text.lines().enumerate() {
        let code = raw.split('#').next().unwrap_or("");
        let line = code.trim();
        if line.is_empty() {
            continue;
        }
        let indented = code.starts_with(char::is_whitespace);
        if !indented {
            let name = line.strip_suffix(':').map(str::trim);
            section = match name {
                Some("flows") => Section::Flows,
                Some("set") => Section::Set,
                Some("choose") => Section::Choose,
                Some(other) if is_word(other) => {
                    errors.push(problem(
                        "UNKNOWN_SECTION",
                        &format!("line {}", lineno + 1),
                        other,
                        &SECTIONS,
                        format!("unknown section `{other}`"),
                    ));
                    Section::None
                }
                _ => {
                    errors.push(problem(
                        "BAD_SYNTAX",
                        &format!("line {}", lineno + 1),
                        line,
                        &SECTIONS,
                        "expected a section heading such as `flows:`".into(),
                    ));
                    Section::None
                }
            };
            continue;
        }
        match section {
            Section::None => {} // inside a section already reported
            Section::Flows => {
                let index = n_flows;
                n_flows += 1;
                match line.strip_prefix('-') {
                    Some(body) => def.links.extend(parse_flow_line(body.trim(), index, &mut errors)),
                    None => errors.push(problem(
                        "BAD_SYNTAX",
                        &format!("flows[{index}]"),
                        line,
                        &[],
                        "a flow line starts with `-`".into(),
                    )),
                }
            }
            Section::Set => {
                parse_entry(line, format!("set[{n_set}]"), "set", &mut def.set, &mut errors);
                n_set += 1;
            }
            Section::Choose => {
                parse_entry(line, format!("choose[{n_choose}]"), "choose", &mut def.choose, &mut errors);
                n_choose += 1;
            }
        }
    }

    // Per-link checks, in written order. A part in the middle of a chain ends one link and
    // starts the next; its kind is checked once.
    let mut checked: Vec<&str> = Vec::new();
    for l in &def.links {
        if l.from.name == l.to.name {
            errors.push(problem("SELF_LINK", &l.at, &l.from.name, &[], format!("`{}` is linked to itself", l.from.name)));
        }
        for r in [&l.from, &l.to] {
            let Some(opt) = &r.option else { continue };
            if checked.contains(&r.at.as_str()) {
                continue;
            }
            checked.push(&r.at);
            match options.get(&r.name) {
                Some(prev) if prev != opt => errors.push(problem(
                    "CONFLICTING_OPTION",
                    &r.at,
                    opt,
                    &[prev.as_str()],
                    format!("`{}` is written with kind `{prev}` and `{opt}`", r.name),
                )),
                Some(_) => {}
                None => {
                    options.insert(r.name.clone(), opt.clone());
                }
            }
        }
    }

    // Resolve the option per part name, so every mention carries it.
    for l in &mut def.links {
        for r in [&mut l.from, &mut l.to] {
            if let Some(opt) = options.get(&r.name) {
                r.option = Some(opt.clone());
            }
        }
    }

    let mut seen: Vec<(String, String, Rel)> = Vec::new();
    def.links.retain(|l| {
        let key = (l.from.name.clone(), l.to.name.clone(), l.rel);
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });

    // One ordered pair of parts is one connection: two different relations between them would
    // become two lines with the same id.
    for (i, l) in def.links.iter().enumerate() {
        let earlier = def.links[..i].iter().find(|e| e.from.name == l.from.name && e.to.name == l.to.name);
        if let Some(e) = earlier {
            let symbol = |r: Rel| match r {
                Rel::Main => "->",
                Rel::Aux => "~>",
                Rel::Assoc => "--",
            };
            errors.push(problem(
                "CONFLICTING_RELATION",
                &l.at,
                &format!("{} {} {}", l.from.name, symbol(l.rel), l.to.name),
                &[],
                format!(
                    "`{}` and `{}` are already connected with `{}` in {}; write one relation per pair",
                    l.from.name,
                    l.to.name,
                    symbol(e.rel),
                    e.at
                ),
            ));
        }
    }

    if errors.is_empty() && def.links.is_empty() {
        errors.push(Problem {
            code: "EMPTY_DEFINITION".into(),
            at: Some("flows".into()),
            message: "the definition has no flows".into(),
            ..Default::default()
        });
    }
    if errors.is_empty() {
        Ok(def)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEB: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n  - waf -- cloudfront\nset:\n  azs: 3\n  lang: ja\nchoose:\n  route.ecs.s3: endpoint\n";

    #[test]
    fn parses_the_spec_example() {
        let d = parse(WEB).unwrap();
        assert_eq!(d.links.len(), 8);
        assert_eq!((d.links[0].from.name.as_str(), d.links[0].to.name.as_str(), d.links[0].rel), ("user", "cloudfront", Rel::Main));
        assert_eq!(d.links[2].to.option.as_deref(), Some("fargate"));
        assert_eq!(d.links[2].to.at, "flows[0][3]");
        assert_eq!(d.links[6].rel, Rel::Aux);
        assert_eq!(d.links[7].rel, Rel::Assoc);
        assert_eq!(d.links[7].at, "flows[4]");
        assert_eq!(d.set["azs"], "3");
        assert_eq!(d.choose["route.ecs.s3"], "endpoint");
    }

    #[test]
    fn tolerates_crlf_tabs_trailing_space_and_comments() {
        let d = parse("# top\r\nflows:\r\n  - user -> alb   # entry\r\n\r\n").unwrap();
        assert_eq!(d.links.len(), 1);
        let d = parse("flows:\t\n\t- user -> alb \t\n").unwrap();
        assert_eq!(d.links.len(), 1);
    }

    #[test]
    fn empty_inputs_are_errors_not_panics() {
        for t in ["", "# only a comment\n", "set:\n  azs: 2\n"] {
            assert_eq!(parse(t).unwrap_err()[0].code, "EMPTY_DEFINITION");
        }
    }

    #[test]
    fn reports_where_the_syntax_broke() {
        let e = parse("flows:\n  - user => alb\n").unwrap_err();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref(), e[0].got.as_deref()), ("BAD_SYNTAX", Some("flows[0]"), Some("user => alb")));
        assert_eq!(
            e[0].message,
            "a flow line is `- a -> b ~> c -- d`; a part is service[.name][(kind)], with service and kind in [a-z0-9_] and name in [a-z0-9-], not starting or ending with a hyphen"
        );
        let e = parse("flows:\n  - user -> alb\nextra:\n  x: 1\n").unwrap_err();
        assert_eq!(e[0].code, "UNKNOWN_SECTION");
        assert_eq!(e[0].allowed, vec!["choose", "flows", "set"]);
    }

    #[test]
    fn rejects_self_links_and_conflicting_options_and_dedupes() {
        assert_eq!(parse("flows:\n  - ecs -> ecs\n").unwrap_err()[0].code, "SELF_LINK");
        assert_eq!(parse("flows:\n  - alb -> ecs(fargate)\n  - ecs(ec2) -> aurora\n").unwrap_err()[0].code, "CONFLICTING_OPTION");
        assert_eq!(parse("flows:\n  - alb -> ecs\n  - alb -> ecs\n").unwrap().links.len(), 1);
    }

    #[test]
    fn one_pair_with_two_relations_is_an_error() {
        let e = parse("flows:\n  - user -> alb -> ecs -> s3\n  - ecs ~> s3\n").unwrap_err();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref(), e[0].got.as_deref()), ("CONFLICTING_RELATION", Some("flows[1]"), Some("ecs ~> s3")));
        assert_eq!(parse("flows:\n  - ecs -> s3\n  - ecs -> s3\n").unwrap().links.len(), 1);
        assert_eq!(parse("flows:\n  - ecs -> s3\n  - s3 ~> ecs\n").unwrap().links.len(), 2);
    }

    #[test]
    fn a_conflicting_kind_in_the_middle_of_a_chain_is_reported_once() {
        let e = parse("flows:\n  - alb -> ecs(fargate)\n  - user -> ecs(ec2) -> aurora\n").unwrap_err();
        let at: Vec<_> = e.iter().map(|p| (p.code.as_str(), p.at.as_deref())).collect();
        assert_eq!(at, [("CONFLICTING_OPTION", Some("flows[1][1]"))]);
    }

    #[test]
    fn same_option_and_bare_mention_of_one_part_is_allowed() {
        let d = parse("flows:\n  - alb -> ecs(fargate)\n  - ecs -> aurora\n  - ecs(fargate) -> s3\n").unwrap();
        assert_eq!(d.links.len(), 3);
        assert_eq!(d.links[1].from.option.as_deref(), Some("fargate"));
    }

    #[test]
    fn option_written_only_on_a_duplicate_line_survives() {
        let d = parse("flows:\n  - alb -> ecs\n  - alb -> ecs(fargate)\n").unwrap();
        assert_eq!(d.links.len(), 1);
        assert_eq!(d.links[0].to.option.as_deref(), Some("fargate"));
    }

    #[test]
    fn option_reaches_earlier_and_later_bare_mentions() {
        let d = parse("flows:\n  - alb -> ecs(fargate)\n  - ecs -> aurora\n").unwrap();
        assert_eq!(d.links[1].from.option.as_deref(), Some("fargate"));
    }

    #[test]
    fn named_parts_follow_the_grammar() {
        let one = |part: &str| parse(&format!("flows:\n  - user -> {part}\n"));
        let r = one("s3.web").unwrap().links[0].to.clone();
        assert_eq!((r.name.as_str(), r.service.as_str(), r.tag.as_deref()), ("s3.web", "s3", Some("web")));
        let r = one("apigateway.public(http)").unwrap().links[0].to.clone();
        assert_eq!((r.service.as_str(), r.tag.as_deref(), r.option.as_deref()), ("apigateway", Some("public"), Some("http")));
        assert_eq!(one("lambda.resize-1").unwrap().links[0].to.tag.as_deref(), Some("resize-1"));
        let user = one("s3").unwrap().links[0].from.clone();
        assert_eq!((user.service.as_str(), user.tag.as_deref()), ("user", None));
        assert!(one("s3.abcdefghijklmnopqrstuvwx").is_ok());

        for bad in ["s3.", "s3.-web", "s3.web-", "s3.Web", "s3.my_web", "s3.a.b", ".web", "S3.web", "s3.web-->x"] {
            let e = one(bad).unwrap_err();
            assert_eq!((e[0].code.as_str(), e[0].at.as_deref()), ("BAD_SYNTAX", Some("flows[0]")), "{bad}");
            assert!(e[0].message.contains("[a-z0-9-]"), "{bad}");
        }

        let e = one("s3.abcdefghijklmnopqrstuvwxy").unwrap_err();
        assert_eq!(
            (e[0].code.as_str(), e[0].at.as_deref(), e[0].got.as_deref(), e[0].allowed.len()),
            ("NAME_TOO_LONG", Some("flows[0][1]"), Some("abcdefghijklmnopqrstuvwxy"), 0)
        );
        assert_eq!(e[0].message, format!("a part name after the dot is at most {MAX_TAG} characters"));

        let d = one("s3.foo--bar").unwrap();
        assert_eq!(d.links.len(), 2);
        let l = &d.links[1];
        assert_eq!((l.from.name.as_str(), l.to.name.as_str(), l.rel), ("s3.foo", "bar", Rel::Assoc));

        let d = parse("flows:\n  - user -> apigateway(http)\n  - user -> apigateway.admin(rest)\n").unwrap();
        assert_eq!(d.links.len(), 2);
    }

    #[test]
    fn duplicate_keys_are_errors() {
        let e = parse("flows:\n  - a -> b\nset:\n  azs: 2\n  azs: 3\n").unwrap_err();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref(), e[0].got.as_deref()), ("DUPLICATE_KEY", Some("set.azs"), Some("azs")));
        let e = parse("flows:\n  - a -> b\nchoose:\n  r.a.b: x\n  r.a.b: y\n").unwrap_err();
        assert_eq!((e[0].code.as_str(), e[0].at.as_deref()), ("DUPLICATE_KEY", Some("choose.r.a.b")));
    }
}
