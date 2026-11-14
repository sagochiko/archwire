//! Single-place edits of a definition, done as line operations so that every
//! other line, comment and line ending stays byte for byte the same.

use crate::def::split_flow;
use crate::diag::Problem;

fn bad(code: &str, at: &str, message: String) -> Problem {
    Problem { code: code.into(), at: Some(at.into()), message, ..Default::default() }
}

fn bad_path(at: &str) -> Problem {
    bad(
        "BAD_PATH",
        at,
        "path is `flows[i][j]`, `set.<key>` or `choose.<hole id>` and must point at an existing flow part".into(),
    )
}

enum Target<'a> {
    Part(usize, usize),
    Entry(&'static str, &'a str),
}

fn parse_at(at: &str) -> Option<Target<'_>> {
    if let Some(rest) = at.strip_prefix("flows[") {
        let (i, rest) = rest.split_once("][")?;
        let j = rest.strip_suffix(']')?;
        return Some(Target::Part(i.parse().ok()?, j.parse().ok()?));
    }
    for (prefix, section) in [("set.", "set"), ("choose.", "choose")] {
        if let Some(key) = at.strip_prefix(prefix) {
            if !key.is_empty() && !key.contains(char::is_whitespace) && !key.contains('#') {
                return Some(Target::Entry(section, key));
            }
        }
    }
    None
}

/// Splits a line into (content without line ending, line ending).
fn split_eol(line: &str) -> (&str, &str) {
    let body = line.strip_suffix('\n').unwrap_or(line);
    let body = body.strip_suffix('\r').unwrap_or(body);
    (body, &line[body.len()..])
}

fn code_part(body: &str) -> &str {
    body.split('#').next().unwrap_or("")
}

/// Replaces the trimmed text of `part` (a slice of `whole`) with `value`, keeping the whitespace around it.
fn replace_trimmed(whole: &str, part: &str, value: &str) -> String {
    let offset = part.as_ptr() as usize - whole.as_ptr() as usize;
    let start = offset + (part.len() - part.trim_start().len());
    let end = offset + part.trim_end().len();
    format!("{}{}{}", &whole[..start], value, &whole[end..])
}

/// `at` is "flows[i][j]" (replace that part), "set.<key>" or "choose.<hole id>".
/// For set and choose, the section or the entry is added when missing.
/// Other lines, comments and line endings are left as they are.
pub fn set_value(text: &str, at: &str, value: &str) -> Result<String, Problem> {
    let target = parse_at(at).ok_or_else(|| bad_path(at))?;
    if value.trim().is_empty() || value.contains(['\n', '\r', '#']) {
        return Err(bad("BAD_VALUE", at, "a value must be one non-empty line without `#`".into()));
    }
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };

    let mut current = "";
    let mut n_flows = 0usize;
    // The last heading or entry line of the target section.
    let mut section_end: Option<usize> = None;
    let mut found: Option<usize> = None;
    // Indentation an appended entry copies: the target section's, else the file's first indented line.
    let mut section_indent: Option<&str> = None;
    let mut file_indent: Option<&str> = None;
    for (idx, line) in lines.iter().enumerate() {
        let (body, ending) = split_eol(line);
        let code = code_part(body);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !code.starts_with(char::is_whitespace) {
            current = match trimmed.strip_suffix(':').map(str::trim) {
                Some("flows") => "flows",
                Some("set") => "set",
                Some("choose") => "choose",
                _ => "",
            };
            if matches!(target, Target::Entry(section, _) if section == current) {
                section_end = Some(idx);
            }
            continue;
        }
        let indent = &code[..code.len() - code.trim_start().len()];
        file_indent.get_or_insert(indent);
        match target {
            Target::Part(i, j) if current == "flows" => {
                let this = n_flows;
                n_flows += 1;
                if this != i {
                    continue;
                }
                let rest = code.trim_start().strip_prefix('-').ok_or_else(|| bad_path(at))?;
                let (parts, _) = split_flow(rest);
                let part = parts.get(j).filter(|_| parts.len() > 1).ok_or_else(|| bad_path(at))?;
                let new_body = replace_trimmed(body, part, value);
                lines[idx] = format!("{new_body}{ending}");
                return Ok(lines.concat());
            }
            Target::Entry(section, key) if current == section => {
                section_end = Some(idx);
                section_indent = Some(indent);
                if code.split_once(':').is_some_and(|(k, _)| k.trim() == key) {
                    found = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }

    let Target::Entry(section, key) = target else { return Err(bad_path(at)) };
    if let Some(idx) = found {
        let (body, ending) = split_eol(&lines[idx]);
        let code = code_part(body);
        let colon = code.find(':').expect("a found entry has a colon");
        let new_body = replace_trimmed(body, &code[colon + 1..], value);
        lines[idx] = format!("{new_body}{ending}");
        return Ok(lines.concat());
    }
    let indent = section_indent.or(file_indent).unwrap_or("  ");
    let entry = format!("{indent}{key}: {value}");
    match section_end {
        Some(idx) if split_eol(&lines[idx]).1.is_empty() => {
            lines[idx].push_str(eol);
            lines.insert(idx + 1, entry);
        }
        Some(idx) => lines.insert(idx + 1, format!("{entry}{eol}")),
        None => {
            if let Some(last) = lines.last_mut().filter(|l| !l.ends_with('\n')) {
                last.push_str(eol);
            }
            lines.push(format!("{section}:{eol}"));
            lines.push(format!("{entry}{eol}"));
        }
    }
    Ok(lines.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_one_ref_and_keeps_everything_else() {
        let src = "# my web\nflows:\n  - user -> cloudfrnt -> alb   # entry\n";
        let out = set_value(src, "flows[0][1]", "cloudfront").unwrap();
        assert_eq!(out, "# my web\nflows:\n  - user -> cloudfront -> alb   # entry\n");
    }

    #[test]
    fn appends_choose_section_when_missing() {
        let out = set_value("flows:\n  - ecs -> s3\n", "choose.route.ecs.s3", "endpoint").unwrap();
        assert_eq!(out, "flows:\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\n");
    }

    #[test]
    fn updates_existing_key_in_place() {
        let out = set_value("flows:\n  - a -> b\nset:\n  azs: 2  # two\n", "set.azs", "3").unwrap();
        assert_eq!(out, "flows:\n  - a -> b\nset:\n  azs: 3  # two\n");
    }

    #[test]
    fn keeps_crlf() {
        let out = set_value("flows:\r\n  - a -> b\r\n", "set.lang", "ja").unwrap();
        assert_eq!(out, "flows:\r\n  - a -> b\r\nset:\r\n  lang: ja\r\n");
    }

    #[test]
    fn bad_path_is_an_error() {
        assert_eq!(set_value("flows:\n  - a -> b\n", "flows[3][0]", "x").unwrap_err().code, "BAD_PATH");
    }

    #[test]
    fn bad_path_forms_and_part_index() {
        for at in ["flows[0][2]", "flows[0]", "flows[x][0]", "set.", "other.x", "set.a b"] {
            assert_eq!(set_value("flows:\n  - a -> b\n", at, "x").unwrap_err().code, "BAD_PATH", "{at}");
        }
    }

    #[test]
    fn replacement_may_carry_an_option_and_crlf_comment_survive() {
        let src = "flows:\r\n  - alb -> ecs # c\r\n  - ecs -> s3\r\n";
        let out = set_value(src, "flows[0][1]", "ecs(fargate)").unwrap();
        assert_eq!(out, "flows:\r\n  - alb -> ecs(fargate) # c\r\n  - ecs -> s3\r\n");
    }

    #[test]
    fn adds_key_to_existing_section_and_handles_missing_final_newline() {
        let out = set_value("flows:\n  - a -> b\nset:\n  azs: 2", "set.lang", "ja").unwrap();
        assert_eq!(out, "flows:\n  - a -> b\nset:\n  azs: 2\n  lang: ja");
        let out = set_value("set:\n  azs: 2\nflows:\n  - a -> b\n", "set.lang", "ja").unwrap();
        assert_eq!(out, "set:\n  azs: 2\n  lang: ja\nflows:\n  - a -> b\n");
    }

    #[test]
    fn updates_an_existing_choose_key() {
        let out = set_value("flows:\n  - ecs -> s3\nchoose:\n  route.ecs.s3: nat\n", "choose.route.ecs.s3", "endpoint").unwrap();
        assert_eq!(out, "flows:\n  - ecs -> s3\nchoose:\n  route.ecs.s3: endpoint\n");
    }

    #[test]
    fn appends_to_an_existing_section() {
        let out = set_value("flows:\n  - ecs -> s3\nchoose:\n  route.ecs.s3: nat\nset:\n  azs: 2\n", "set.lang", "ja").unwrap();
        assert_eq!(out, "flows:\n  - ecs -> s3\nchoose:\n  route.ecs.s3: nat\nset:\n  azs: 2\n  lang: ja\n");
    }

    #[test]
    fn an_appended_line_copies_the_indentation_of_its_section_or_the_file() {
        let out = set_value("flows:\n    - a -> b\nset:\n    azs: 2\n", "set.lang", "ja").unwrap();
        assert_eq!(out, "flows:\n    - a -> b\nset:\n    azs: 2\n    lang: ja\n");
        let out = set_value("flows:\n    - a -> b\n", "choose.route.a.b", "nat").unwrap();
        assert_eq!(out, "flows:\n    - a -> b\nchoose:\n    route.a.b: nat\n");
        let out = set_value("flows:\n\t- a -> b\nset:\n\tazs: 2\n", "set.lang", "ja").unwrap();
        assert_eq!(out, "flows:\n\t- a -> b\nset:\n\tazs: 2\n\tlang: ja\n");
        let out = set_value("flows:\n\t- a -> b\n", "set.lang", "ja").unwrap();
        assert_eq!(out, "flows:\n\t- a -> b\nset:\n\tlang: ja\n");
    }

    #[test]
    fn a_flow_line_without_its_leading_dash_is_a_bad_path() {
        let src = "flows:\n  a -> b -> c\n";
        assert_eq!(set_value(src, "flows[0][0]", "x").unwrap_err().code, "BAD_PATH");
    }

    #[test]
    fn rejects_values_that_would_break_the_line() {
        assert_eq!(set_value("flows:\n  - a -> b\n", "set.x", "a\nb").unwrap_err().code, "BAD_VALUE");
        assert_eq!(set_value("flows:\n  - a -> b\n", "set.x", "").unwrap_err().code, "BAD_VALUE");
    }
}
