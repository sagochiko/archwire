//! Checks that the documents agree with the rules data and with what archwire does, so that a
//! change to one without the other fails `cargo test`.
//!
//! - The tables of services and of accepted connections in `docs/definition.md` match
//!   `rules/aws/services.toml`, `links.toml` and `labels.toml`, and every TOML excerpt in the
//!   documents is found in the rules data.
//! - Every `yaml` and `json` code block in the documents carries a marker that says how it is
//!   checked, and the definitions are compiled and compared with what the marker states. The
//!   marker format is described in `CONTRIBUTING.md`.
//! - The table of `UNSUPPORTED` examples in `docs/definition.md` is compiled row by row.
//! - `README.md` and `README.ja.md` have the same code blocks, markers and heading levels.
//! - Relative links and their `#anchor`s lead somewhere.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The documents the tests read, as paths relative to the repository root.
fn documents() -> Vec<String> {
    let mut docs: Vec<String> = std::fs::read_dir(root().join("docs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".md"))
        .map(|n| format!("docs/{n}"))
        .collect();
    docs.sort();
    let mut all = vec!["README.md".to_string(), "README.ja.md".to_string(), "CONTRIBUTING.md".to_string()];
    all.extend(docs);
    all
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

fn finish(problems: Vec<String>) {
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// A small reader for the Markdown these documents use.

const MARKER_START: &str = "<!-- archwire-test:";

#[derive(Debug)]
enum Item {
    Heading { level: usize, text: String },
    Code { lang: String, body: String },
    Marker { words: String },
    Text { text: String },
}

/// The document as items with their 1-based line numbers. A code block starts and ends with a
/// line of three backticks at the left edge; a marker is a whole line `<!-- archwire-test: ... -->`.
fn items(src: &str) -> Vec<(usize, Item)> {
    let mut out = Vec::new();
    let mut lines = src.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        if let Some(lang) = line.strip_prefix("```") {
            let mut body = String::new();
            for (_, l) in lines.by_ref() {
                if l == "```" {
                    break;
                }
                body.push_str(l);
                body.push('\n');
            }
            out.push((i + 1, Item::Code { lang: lang.trim().to_string(), body }));
        } else if let Some(rest) = line.trim().strip_prefix(MARKER_START) {
            let words = rest.strip_suffix("-->").unwrap_or_else(|| panic!("line {}: a marker must end with `-->` on the same line", i + 1));
            out.push((i + 1, Item::Marker { words: words.trim().to_string() }));
        } else if line.starts_with('#') {
            let level = line.chars().take_while(|&c| c == '#').count();
            let text = line[level..].trim().to_string();
            if line[level..].starts_with(' ') {
                out.push((i + 1, Item::Heading { level, text }));
            } else {
                out.push((i + 1, Item::Text { text: line.to_string() }));
            }
        } else {
            out.push((i + 1, Item::Text { text: line.to_string() }));
        }
    }
    out
}

/// The lines that open or close a code block in a way `items` does not read: a fence with `~`,
/// with more than three backticks or indented, or a language that is not lower case or is `yml`.
/// Such a block could hold a definition that no check sees, so it is named instead.
fn fence_problems(doc: &str, src: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let t = line.trim_start();
        if !t.starts_with("```") && !t.starts_with("~~~") {
            continue;
        }
        let lang = line.strip_prefix("```").filter(|rest| !rest.starts_with('`'));
        let readable = lang.is_some_and(|l| l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && l != "yml");
        if !readable {
            problems.push(format!(
                "{doc}:{}: the code fence `{line}` is not read by the document checks; write three backticks at the left edge, with a lower-case language such as `yaml`, `json` or `sh`",
                i + 1
            ));
        }
    }
    problems
}

/// The code spans of a piece of inline Markdown, without their backticks. A span opened by N
/// backticks closes at the next run of exactly N, and one space on each side is dropped when
/// both are there, as CommonMark does.
fn code_spans(s: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('`') {
        let run = rest[start..].chars().take_while(|&c| c == '`').count();
        let after = &rest[start + run..];
        let fence = "`".repeat(run);
        let mut search = 0;
        let mut close = None;
        while let Some(p) = after[search..].find(&fence) {
            let p = search + p;
            let len = after[p..].chars().take_while(|&c| c == '`').count();
            if len == run {
                close = Some(p);
                break;
            }
            search = p + len;
        }
        match close {
            Some(p) => {
                let inner = &after[..p];
                let inner = if inner.len() >= 2 && inner.starts_with(' ') && inner.ends_with(' ') { &inner[1..inner.len() - 1] } else { inner };
                spans.push(inner.to_string());
                rest = &after[p + run..];
            }
            None => rest = after,
        }
    }
    spans
}

/// The text with its code spans removed, so that brackets inside them are not read as links.
fn without_code_spans(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let run = rest[start..].chars().take_while(|&c| c == '`').count();
        let after = &rest[start + run..];
        match after.find(&"`".repeat(run)) {
            Some(p) => rest = &after[p + run..],
            None => {
                out.push_str(&rest[start..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The cells of a table row, split at `|` outside code spans.
fn cells(row: &str) -> Vec<String> {
    let row = row.trim().trim_start_matches('|').trim_end_matches('|');
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut open: Option<usize> = None;
    let chars: Vec<char> = row.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            let run = chars[i..].iter().take_while(|&&c| c == '`').count();
            match open {
                None => open = Some(run),
                Some(n) if n == run => open = None,
                _ => {}
            }
            cell.extend(std::iter::repeat_n('`', run));
            i += run;
            continue;
        }
        if c == '|' && open.is_none() {
            out.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(c);
        }
        i += 1;
    }
    out.push(cell.trim().to_string());
    out
}

/// The rows of the first table whose header row is `header`, each as its cells.
fn table(src: &str, header: &[&str]) -> Option<(usize, Vec<(usize, Vec<String>)>)> {
    let lines: Vec<&str> = src.lines().collect();
    let start = lines.iter().position(|l| l.trim_start().starts_with('|') && cells(l) == header)?;
    let rows = lines[start + 2..]
        .iter()
        .enumerate()
        .take_while(|(_, l)| l.trim_start().starts_with('|'))
        .map(|(i, l)| (start + 3 + i, cells(l)))
        .collect();
    Some((start + 1, rows))
}

// ---------------------------------------------------------------------------------------------
// 1. The tables in docs/definition.md and the rules data.

fn rules_toml(name: &str) -> toml::Table {
    toml::from_str(&read(&format!("rules/aws/{name}"))).unwrap_or_else(|e| panic!("rules/aws/{name}: {e}"))
}

/// The entries of an array of tables, such as `[[service]]`, in a rules file.
fn entries(file: &toml::Table, key: &str) -> Vec<toml::Value> {
    file.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default()
}

fn english_label(labels: &toml::Table, key: &str) -> String {
    labels.get(key).and_then(|l| l.get("en")).and_then(|v| v.as_str()).unwrap_or_else(|| panic!("labels.toml has no English text for `{key}`")).to_string()
}

#[test]
fn the_service_table_lists_every_writable_service_and_its_kinds() {
    let src = read("docs/definition.md");
    let (at, rows) = table(&src, &["Name", "Service", "Kinds", "Where it is drawn"]).expect("docs/definition.md has no table with the header `Name | Service | Kinds | Where it is drawn`");
    let mut problems = Vec::new();

    let mut documented: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (line, row) in &rows {
        let name = code_spans(&row[0]);
        let [name] = name.as_slice() else {
            problems.push(format!("docs/definition.md:{line}: the Name cell must hold one service in backticks, got `{}`", row[0]));
            continue;
        };
        let kinds = code_spans(row.get(2).map(String::as_str).unwrap_or("")).into_iter().collect();
        if documented.insert(name.clone(), kinds).is_some() {
            problems.push(format!("docs/definition.md:{line}: the service `{name}` is listed twice"));
        }
    }

    let services = entries(&rules_toml("services.toml"), "service");
    let mut writable: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for s in &services {
        if s.get("derived_only").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let key = s["key"].as_str().unwrap().to_string();
        let kinds = s.get("options").and_then(|o| o.as_table()).map(|t| t.keys().cloned().collect()).unwrap_or_default();
        writable.insert(key, kinds);
    }

    for (name, kinds) in &documented {
        match writable.get(name) {
            None => problems.push(format!("docs/definition.md:{at}: the service table lists `{name}`, which rules/aws/services.toml does not have as a writable service")),
            Some(want) if want != kinds => problems.push(format!(
                "docs/definition.md:{at}: the service table gives `{name}` the kinds {kinds:?}, but rules/aws/services.toml gives {want:?}"
            )),
            Some(_) => {}
        }
    }
    for name in writable.keys().filter(|n| !documented.contains_key(*n)) {
        problems.push(format!("docs/definition.md:{at}: rules/aws/services.toml has the writable service `{name}`, which the service table does not list"));
    }
    finish(problems);
}

/// The text the connection table gives as the line label of a link: the English label, with the
/// labels per kind of the target or per role of the target spelled out.
fn documented_label(link: &toml::Value, labels: &toml::Table) -> String {
    let base = english_label(labels, link["label"].as_str().unwrap());
    if let Some(kinds) = link.get("to_kind").and_then(|v| v.as_table()) {
        let mut parts: Vec<String> = kinds.iter().map(|(kind, note)| format!("{} for `{kind}`", english_label(labels, note["label"].as_str().unwrap()))).collect();
        parts.push(format!("{base} while the kind is undecided"));
        return parts.join(", ");
    }
    if let Some(roles) = link.get("to_role").and_then(|v| v.as_table()) {
        let writer = roles.get("writer").map(|l| format!("{} to the writer", english_label(labels, l.as_str().unwrap())));
        let reader = roles.get("reader").map(|l| format!("{} to a reader", english_label(labels, l.as_str().unwrap())));
        return writer.into_iter().chain(reader).collect::<Vec<_>>().join(", ");
    }
    base
}

#[test]
fn the_connection_table_lists_every_accepted_connection_with_its_label() {
    let src = read("docs/definition.md");
    let (at, rows) = table(&src, &["From", "Symbol", "To", "Line label"]).expect("docs/definition.md has no table with the header `From | Symbol | To | Line label`");
    let mut problems = Vec::new();

    let mut documented: BTreeMap<(String, String, String), (usize, String)> = BTreeMap::new();
    for (line, row) in &rows {
        let one = |cell: &str| code_spans(cell).into_iter().next().unwrap_or_default();
        let key = (one(&row[0]), one(&row[1]), one(&row[2]));
        if documented.insert(key.clone(), (*line, row.get(3).cloned().unwrap_or_default())).is_some() {
            problems.push(format!("docs/definition.md:{line}: `{} {} {}` is listed twice", key.0, key.1, key.2));
        }
    }

    let links = entries(&rules_toml("links.toml"), "link");
    let labels = rules_toml("labels.toml");
    let mut accepted: BTreeMap<(String, String, String), String> = BTreeMap::new();
    for link in &links {
        let symbol = match link["rel"].as_str().unwrap() {
            "main" => "->",
            "aux" => "~>",
            "assoc" => "--",
            other => panic!("links.toml: unknown rel `{other}`"),
        };
        let key = (link["from"].as_str().unwrap().to_string(), symbol.to_string(), link["to"].as_str().unwrap().to_string());
        accepted.insert(key, documented_label(link, &labels));
    }

    for ((from, symbol, to), (line, label)) in &documented {
        match accepted.get(&(from.clone(), symbol.clone(), to.clone())) {
            None => problems.push(format!("docs/definition.md:{line}: the connection table lists `{from} {symbol} {to}`, which rules/aws/links.toml does not accept")),
            Some(want) if want != label => problems.push(format!(
                "docs/definition.md:{line}: the label of `{from} {symbol} {to}` is written `{label}`, but rules/aws/links.toml and labels.toml give `{want}`"
            )),
            Some(_) => {}
        }
    }
    for (from, symbol, to) in accepted.keys().filter(|k| !documented.contains_key(*k)) {
        problems.push(format!("docs/definition.md:{at}: rules/aws/links.toml accepts `{from} {symbol} {to}`, which the connection table does not list"));
    }
    finish(problems);
}

/// What `archwire list` prints, read as JSON.
fn listed() -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_archwire")).arg("list").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "archwire list failed");
    serde_json::from_slice(&out.stdout).expect("archwire list prints JSON")
}

/// The text of a JSON array of strings.
fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

/// The label of a connection in `archwire list`, spelt out as the connection table writes it.
fn listed_label(link: &serde_json::Value) -> String {
    let base = link["label"].as_str().unwrap().to_string();
    let kinds = link["kind_labels"].as_object().unwrap();
    if !kinds.is_empty() {
        let mut parts: Vec<String> = kinds.iter().map(|(kind, label)| format!("{} for `{kind}`", label.as_str().unwrap())).collect();
        parts.push(format!("{base} while the kind is undecided"));
        return parts.join(", ");
    }
    let roles = link["role_labels"].as_object().unwrap();
    if !roles.is_empty() {
        let writer = roles.get("writer").map(|l| format!("{} to the writer", l.as_str().unwrap()));
        let reader = roles.get("reader").map(|l| format!("{} to a reader", l.as_str().unwrap()));
        return writer.into_iter().chain(reader).collect::<Vec<_>>().join(", ");
    }
    base
}

#[test]
fn archwire_list_gives_the_services_and_connections_of_the_tables() {
    let src = read("docs/definition.md");
    let list = listed();
    let mut problems = Vec::new();

    let (at, rows) = table(&src, &["Name", "Service", "Kinds", "Where it is drawn"]).expect("docs/definition.md has no service table");
    let documented: BTreeMap<String, BTreeSet<String>> = rows
        .iter()
        .filter_map(|(_, row)| code_spans(&row[0]).into_iter().next().map(|n| (n, code_spans(row.get(2).map(String::as_str).unwrap_or("")).into_iter().collect())))
        .collect();
    let services = list["services"].as_array().unwrap();
    let mut seen = BTreeSet::new();
    for s in services {
        let name = s["name"].as_str().unwrap().to_string();
        if !seen.insert(name.clone()) {
            problems.push(format!("archwire list gives the service `{name}` twice"));
        }
        let kinds: BTreeSet<String> = strings(&s["kinds"]).into_iter().collect();
        match documented.get(&name) {
            None => problems.push(format!("docs/definition.md:{at}: archwire list gives the service `{name}`, which the service table does not list")),
            Some(want) if *want != kinds => problems.push(format!("docs/definition.md:{at}: archwire list gives `{name}` the kinds {kinds:?}, the service table {want:?}")),
            Some(_) => {}
        }
    }
    for name in documented.keys().filter(|n| !seen.contains(*n)) {
        problems.push(format!("docs/definition.md:{at}: the service table lists `{name}`, which archwire list does not give"));
    }

    let (at, rows) = table(&src, &["From", "Symbol", "To", "Line label"]).expect("docs/definition.md has no connection table");
    let one = |cell: &str| code_spans(cell).into_iter().next().unwrap_or_default();
    let documented: BTreeMap<(String, String, String), String> = rows.iter().map(|(_, row)| ((one(&row[0]), one(&row[1]), one(&row[2])), row.get(3).cloned().unwrap_or_default())).collect();
    let links = list["links"].as_array().unwrap();
    let mut seen = BTreeSet::new();
    for l in links {
        let key = (l["from"].as_str().unwrap().to_string(), l["symbol"].as_str().unwrap().to_string(), l["to"].as_str().unwrap().to_string());
        let (from, symbol, to) = &key;
        if !seen.insert(key.clone()) {
            problems.push(format!("archwire list gives `{from} {symbol} {to}` twice"));
        }
        let label = listed_label(l);
        match documented.get(&key) {
            None => problems.push(format!("docs/definition.md:{at}: archwire list gives `{from} {symbol} {to}`, which the connection table does not list")),
            Some(want) if *want != label => problems.push(format!("docs/definition.md:{at}: archwire list labels `{from} {symbol} {to}` `{label}`, the connection table `{want}`")),
            Some(_) => {}
        }
    }
    for (from, symbol, to) in documented.keys().filter(|k| !seen.contains(*k)) {
        problems.push(format!("docs/definition.md:{at}: the connection table lists `{from} {symbol} {to}`, which archwire list does not give"));
    }
    if links.len() != rows.len() {
        problems.push(format!("archwire list gives {} connections, the connection table has {} rows", links.len(), rows.len()));
    }

    let (at, rows) = table(&src, &["Key", "Values", "Default"]).expect("docs/definition.md has no table of settings");
    let mut documented: Vec<String> = rows.iter().map(|(_, row)| one(&row[0])).collect();
    documented.sort();
    let settings: Vec<String> = list["settings"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap().to_string()).collect();
    if settings != documented {
        problems.push(format!("docs/definition.md:{at}: archwire list gives the settings {settings:?}, the table of `set` {documented:?}"));
    }
    finish(problems);
}

// Fails if `archwire list` leaves out a writable service of the rules data or a connection, or
// gives one the rules data does not have, so the list cannot drift even where a table does.
#[test]
fn archwire_list_follows_the_rules_data() {
    let list = listed();
    let mut problems = Vec::new();
    let services = entries(&rules_toml("services.toml"), "service");
    let labels = rules_toml("labels.toml");
    let mut want = Vec::new();
    for s in services.iter().filter(|s| s.get("derived_only").and_then(|v| v.as_bool()) != Some(true)) {
        let kinds: Vec<String> = s.get("options").and_then(|o| o.as_table()).map(|t| t.keys().cloned().collect()).unwrap_or_default();
        let label = s["label"].as_str().unwrap();
        let label = labels.get(label).and_then(|l| l.get("en")).and_then(|v| v.as_str()).unwrap_or(label);
        want.push(serde_json::json!({"name": s["key"], "label": label, "kinds": kinds, "in_vpc": s["scope"].as_str() == Some("vpc")}));
    }
    want.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    if list["services"].as_array().unwrap() != &want {
        problems.push(format!("archwire list gives the services\n{:#}\nbut rules/aws/services.toml gives\n{:#}", list["services"], serde_json::Value::Array(want)));
    }
    let links = entries(&rules_toml("links.toml"), "link");
    let listed: Vec<String> = list["links"].as_array().unwrap().iter().map(|l| format!("{} {} {}: {}", l["from"].as_str().unwrap(), l["symbol"].as_str().unwrap(), l["to"].as_str().unwrap(), listed_label(l))).collect();
    let want: Vec<String> = links
        .iter()
        .map(|l| {
            let symbol = match l["rel"].as_str().unwrap() {
                "main" => "->",
                "aux" => "~>",
                _ => "--",
            };
            format!("{} {symbol} {}: {}", l["from"].as_str().unwrap(), l["to"].as_str().unwrap(), documented_label(l, &labels))
        })
        .collect();
    if listed != want {
        problems.push(format!("archwire list gives the connections, in order,\n{}\nbut rules/aws/links.toml gives\n{}", listed.join("\n"), want.join("\n")));
    }
    finish(problems);
}

#[test]
fn every_toml_excerpt_in_the_documents_is_in_the_rules_data() {
    let data: Vec<String> = ["services.toml", "rules.toml", "labels.toml", "links.toml"].iter().map(|n| read(&format!("rules/aws/{n}"))).collect();
    let mut problems = Vec::new();
    for doc in documents() {
        for (line, item) in items(&read(&doc)) {
            let Item::Code { lang, body } = item else { continue };
            if lang != "toml" {
                continue;
            }
            for chunk in body.split("\n\n").map(str::trim).filter(|c| !c.is_empty()) {
                if !data.iter().any(|d| d.contains(chunk)) {
                    problems.push(format!("{doc}:{line}: this part of the TOML block is not in rules/aws/*.toml:\n{chunk}"));
                }
            }
        }
    }
    finish(problems);
}

// ---------------------------------------------------------------------------------------------
// 2. The examples, compiled.

/// The words of a marker: `key=value` pairs and bare words, a value in double quotes may hold spaces.
fn marker_words(words: &str) -> (Vec<String>, BTreeMap<String, String>) {
    let mut bare = Vec::new();
    let mut pairs = BTreeMap::new();
    for word in shell_words(words) {
        match word.split_once('=') {
            Some((k, v)) => {
                pairs.insert(k.to_string(), v.to_string());
            }
            None => bare.push(word),
        }
    }
    (bare, pairs)
}

/// Splits a command line into words, with single and double quotes as a shell reads them.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            None => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

struct Run {
    code: Option<i32>,
    stdout: String,
}

fn archwire(work: &Path, args: &[String]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_archwire")).args(args).current_dir(work).output().unwrap();
    Run { code: out.status.code(), stdout: String::from_utf8(out.stdout).unwrap() }
}

/// The lines of a report, trimmed and without a trailing comma, for finding a quoted part in it.
fn report_lines(s: &str) -> Vec<String> {
    s.lines().map(|l| l.trim().trim_end_matches(',').to_string()).filter(|l| !l.is_empty()).collect()
}

/// The holes and errors of a report, short enough for a failure message.
fn summary(stdout: &str) -> String {
    let Ok(report) = serde_json::from_str::<serde_json::Value>(stdout) else { return format!("output: {stdout}") };
    let holes: Vec<&str> = report["holes"].as_array().map(|a| a.iter().filter_map(|h| h["hole"].as_str()).collect()).unwrap_or_default();
    let errors: Vec<String> = report["errors"]
        .as_array()
        .map(|a| a.iter().map(|e| format!("{} at {}: {}", e["code"].as_str().unwrap_or(""), e["at"].as_str().unwrap_or("-"), e["message"].as_str().unwrap_or(""))).collect())
        .unwrap_or_default();
    format!("holes {holes:?}, errors {errors:?}")
}

/// Where a quoted text and the printed one first differ, line by line.
fn first_difference(want: &str, got: &str) -> String {
    let (mut w, mut g) = (want.lines(), got.lines());
    for n in 1.. {
        match (w.next(), g.next()) {
            (Some(a), Some(b)) if a == b => continue,
            (None, None) => return "they differ only in blank lines at the end".into(),
            (a, b) => return format!("first difference at line {n} of the block:\n  written: {}\n  printed: {}", a.unwrap_or("(end)"), b.unwrap_or("(end)")),
        }
    }
    unreachable!()
}

/// Checks the exit code and, when the marker names them, the holes and the errors of a report.
fn check_report(place: &str, run: &Run, pairs: &BTreeMap<String, String>, problems: &mut Vec<String>) {
    if let Some(want) = pairs.get("exit") {
        let want: i32 = want.parse().unwrap_or_else(|_| panic!("{place}: `exit` must be a number"));
        if run.code != Some(want) {
            problems.push(format!("{place}: the document says exit code {want}, but archwire exited with {:?} ({})", run.code, summary(&run.stdout)));
        }
    }
    if run.stdout.is_empty() && !pairs.contains_key("holes") && !pairs.contains_key("errors") {
        // A `set` that succeeds prints nothing.
        return;
    }
    let report: serde_json::Value = match serde_json::from_str(&run.stdout) {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("{place}: the report is not JSON ({e}):\n{}", run.stdout));
            return;
        }
    };
    if let Some(want) = pairs.get("holes") {
        let want: Vec<&str> = want.split(',').filter(|s| !s.is_empty()).collect();
        let got: Vec<&str> = report["holes"].as_array().unwrap().iter().map(|h| h["hole"].as_str().unwrap()).collect();
        if got != want {
            problems.push(format!("{place}: the document gives the holes {want:?}, but the report has {got:?}"));
        }
    }
    if let Some(want) = pairs.get("errors") {
        let want: Vec<&str> = want.split(',').filter(|s| !s.is_empty()).collect();
        let got: Vec<String> = report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("{}@{}", e["code"].as_str().unwrap(), e["at"].as_str().unwrap_or("")))
            .collect();
        if got != want {
            problems.push(format!("{place}: the document gives the errors {want:?}, but the report has {got:?}"));
        }
    }
}

/// `compile` and `run` must say which exit code the document expects.
fn require_exit(place: &str, words: &str, pairs: &BTreeMap<String, String>, problems: &mut Vec<String>) {
    if !pairs.contains_key("exit") {
        problems.push(format!("{place}: the marker `{words}` has no `exit=`; `compile` and `run` must say which exit code the document expects"));
    }
}

/// Compiles `file` in `work` and checks the result against the marker.
fn compile_file(work: &Path, file: &str, place: &str, pairs: &BTreeMap<String, String>, problems: &mut Vec<String>) -> Run {
    let out = format!("{}.drawio", file.trim_end_matches(".yaml"));
    let run = archwire(work, &["compile".into(), file.into(), "-o".into(), out]);
    check_report(place, &run, pairs, problems);
    run
}

const KNOWN_KEYS: [&str; 6] = ["exit", "file", "case", "holes", "errors", "line"];

fn unknown_keys(place: &str, pairs: &BTreeMap<String, String>, problems: &mut Vec<String>) {
    for key in pairs.keys().filter(|k| !KNOWN_KEYS.contains(&k.as_str())) {
        problems.push(format!("{place}: the marker has the unknown key `{key}`"));
    }
}

/// Runs the examples of one document. Every `yaml` and `json` block must have a marker on the
/// line just above it; the markers are described in CONTRIBUTING.md.
fn check_examples(doc: &str, src: &str, problems: &mut Vec<String>) -> usize {
    // Tests run side by side, some on the same document name, so each call gets its own folder.
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let work = std::env::temp_dir().join(format!("archwire-docs-{}-{call}-{}", std::process::id(), doc.replace(['/', '.'], "-")));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let list = items(src);
    let mut last_report: Option<String> = None;
    let mut checked = 0;
    let mut scratch = 0;
    problems.extend(fence_problems(doc, src));

    for (index, (line, item)) in list.iter().enumerate() {
        let place = format!("{doc}:{line}");
        let marker_above = match index.checked_sub(1).map(|i| &list[i]) {
            Some((l, Item::Marker { words })) if l + 1 == *line => Some(words.as_str()),
            _ => None,
        };
        let next_is_code = matches!(list.get(index + 1), Some((l, Item::Code { .. })) if *l == line + 1);
        match item {
            Item::Code { lang, body } if lang == "yaml" || lang == "json" => {
                let Some(words) = marker_above else {
                    problems.push(format!(
                        "{place}: this `{lang}` block has no marker on the line above it; add `<!-- archwire-test: ... -->` (see CONTRIBUTING.md)"
                    ));
                    continue;
                };
                let (bare, pairs) = marker_words(words);
                unknown_keys(&place, &pairs, problems);
                match (lang.as_str(), bare.first().map(String::as_str)) {
                    (_, Some("skip")) => {}
                    ("yaml", Some("compile")) => {
                        require_exit(&place, words, &pairs, problems);
                        let file = match pairs.get("file") {
                            Some(f) => f.clone(),
                            None => {
                                scratch += 1;
                                format!("example-{scratch}.yaml")
                            }
                        };
                        std::fs::write(work.join(&file), body).unwrap();
                        if let Some(case) = pairs.get("case") {
                            let def = format!("tests/cases/{case}/def.yaml");
                            match std::fs::read_to_string(root().join(&def)) {
                                Ok(want) if want == *body => {}
                                Ok(_) => problems.push(format!("{place}: the document says this is the acceptance case `{case}`, but the block differs from {def}")),
                                Err(_) => problems.push(format!("{place}: the acceptance case `{case}` named by the marker does not exist")),
                            }
                        }
                        last_report = Some(compile_file(&work, &file, &place, &pairs, problems).stdout);
                        checked += 1;
                    }
                    ("json", Some("report")) => {
                        match &last_report {
                            Some(r) if r.trim_end() == body.trim_end() => {}
                            Some(r) => problems.push(format!("{place}: the report written here differs from the one archwire prints; {}", first_difference(body, r))),
                            None => problems.push(format!("{place}: `report` needs a compile above it in the document")),
                        }
                        checked += 1;
                    }
                    ("json", Some("report-part")) => {
                        let part = report_lines(body);
                        match &last_report {
                            Some(r) => {
                                let lines = report_lines(r);
                                if !lines.windows(part.len()).any(|w| w == part.as_slice()) {
                                    let note = match part.iter().find(|l| !lines.contains(l)) {
                                        Some(l) => format!("the report has no line `{l}`"),
                                        None => "the report has every line, but not together in this order".into(),
                                    };
                                    problems.push(format!("{place}: this part of a report is not in the one archwire prints: {note}"));
                                }
                            }
                            None => problems.push(format!("{place}: `report-part` needs a compile above it in the document")),
                        }
                        checked += 1;
                    }
                    _ => problems.push(format!("{place}: the marker `{words}` does not fit a `{lang}` block; use `compile`, `report`, `report-part` or `skip`")),
                }
            }
            Item::Code { lang, body } if lang == "sh" => {
                let Some(words) = marker_above else { continue };
                let (bare, pairs) = marker_words(words);
                unknown_keys(&place, &pairs, problems);
                if bare.first().map(String::as_str) != Some("run") {
                    problems.push(format!("{place}: a `sh` block takes only the marker `run`"));
                    continue;
                }
                require_exit(&place, words, &pairs, problems);
                let commands: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).collect();
                for (n, command) in commands.iter().enumerate() {
                    let mut words = shell_words(command);
                    if words.first().map(String::as_str) != Some("archwire") {
                        problems.push(format!("{place}: `run` can run only `archwire` commands, not `{command}`"));
                        break;
                    }
                    words.remove(0);
                    let redirect = words.iter().position(|w| w.starts_with('>')).map(|p| {
                        let to = match words[p].strip_prefix('>').unwrap() {
                            "" => words.get(p + 1).cloned().unwrap_or_default(),
                            file => file.to_string(),
                        };
                        words.truncate(p);
                        to
                    });
                    let run = archwire(&work, &words);
                    if let Some(to) = redirect {
                        std::fs::write(work.join(to), &run.stdout).unwrap();
                    }
                    let what = format!("{place}: `{command}`");
                    if n + 1 == commands.len() {
                        check_report(&what, &run, &pairs, problems);
                    } else if run.code != Some(0) {
                        problems.push(format!("{what} exited with {:?}, but only the last command of a `run` block may fail ({})", run.code, summary(&run.stdout)));
                    }
                    if words.first().map(String::as_str) == Some("set") {
                        if run.code == Some(0) && !run.stdout.is_empty() {
                            problems.push(format!("{what} succeeded but printed something, which `set` does not do"));
                        }
                    } else {
                        last_report = Some(run.stdout);
                    }
                }
                checked += 1;
            }
            Item::Marker { words } if !next_is_code => {
                let (bare, pairs) = marker_words(words);
                unknown_keys(&place, &pairs, problems);
                match (bare.first().map(String::as_str), pairs.get("file")) {
                    (Some("append"), Some(file)) => {
                        let Some(text) = pairs.get("line") else {
                            problems.push(format!("{place}: `append` needs `line=\"...\"`"));
                            continue;
                        };
                        let path = work.join(file);
                        match std::fs::read_to_string(&path) {
                            Ok(mut s) => {
                                s.push_str(text);
                                s.push('\n');
                                std::fs::write(&path, s).unwrap();
                            }
                            Err(_) => problems.push(format!("{place}: `append` to `{file}`, which no block above saved")),
                        }
                    }
                    (Some("compile"), Some(file)) => {
                        require_exit(&place, words, &pairs, problems);
                        if !work.join(file).exists() {
                            problems.push(format!("{place}: `compile` of `{file}`, which no block above saved"));
                            continue;
                        }
                        last_report = Some(compile_file(&work, file, &place, &pairs, problems).stdout);
                        checked += 1;
                    }
                    _ => problems.push(format!("{place}: a marker that is not just above a code block must be `append file=... line=\"...\"` or `compile file=...`")),
                }
            }
            _ => {}
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    checked
}

#[test]
fn every_example_in_the_documents_gives_what_the_document_says() {
    let mut problems = Vec::new();
    let mut checked = 0;
    for doc in documents() {
        checked += check_examples(&doc, &read(&doc), &mut problems);
    }
    finish(problems);
    assert!(checked >= 20, "only {checked} examples were checked; are the markers missing?");
}

#[test]
fn every_row_of_the_unsupported_table_stops_as_written() {
    let src = read("docs/definition.md");
    let (_, rows) = table(&src, &["Definition", "`at`", "Message"]).expect("docs/definition.md has no table with the header ``Definition | `at` | Message``");
    assert!(!rows.is_empty());
    let work = std::env::temp_dir().join(format!("archwire-docs-unsupported-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let mut problems = Vec::new();
    for (line, row) in &rows {
        let flows: Vec<String> = code_spans(&row[0]).into_iter().filter(|s| s.starts_with("- ")).collect();
        let at = code_spans(&row[1]).into_iter().next().unwrap_or_default();
        let message = code_spans(&row[2]).into_iter().next().unwrap_or_default();
        let message = message.trim_matches('"');
        let def = format!("flows:\n{}", flows.iter().map(|f| format!("  {f}\n")).collect::<String>());
        std::fs::write(work.join("a.yaml"), &def).unwrap();
        let run = archwire(&work, &["compile".into(), "a.yaml".into(), "-o".into(), "a.drawio".into()]);
        let report: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
        let errors: Vec<(String, String, String)> = report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["code"].as_str().unwrap().into(), e["at"].as_str().unwrap_or("").into(), e["message"].as_str().unwrap().into()))
            .collect();
        let want = ("UNSUPPORTED".to_string(), at.clone(), message.to_string());
        if run.code != Some(1) || errors != [want.clone()] {
            problems.push(format!("docs/definition.md:{line}: the table says\n{def}stops with {want:?}, but archwire exited with {:?} and reported {errors:?}", run.code));
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    finish(problems);
}

// ---------------------------------------------------------------------------------------------
// 3. README.md and README.ja.md.

#[test]
fn the_two_readmes_have_the_same_code_blocks_markers_and_headings() {
    let shape = |doc: &str| {
        let mut blocks = Vec::new();
        let mut headings = Vec::new();
        for (line, item) in items(&read(doc)) {
            match item {
                Item::Code { lang, body } => blocks.push((line, format!("```{lang}\n{body}```"))),
                Item::Marker { words } => blocks.push((line, format!("<!-- archwire-test: {words} -->"))),
                Item::Heading { level, text } => headings.push((line, level, text)),
                Item::Text { .. } => {}
            }
        }
        (blocks, headings)
    };
    let (en_blocks, en_headings) = shape("README.md");
    let (ja_blocks, ja_headings) = shape("README.ja.md");
    let mut problems = Vec::new();

    for i in 0..en_blocks.len().max(ja_blocks.len()) {
        match (en_blocks.get(i), ja_blocks.get(i)) {
            (Some((_, a)), Some((_, b))) if a == b => {}
            (Some((la, a)), Some((lb, b))) => problems.push(format!("code block or marker {} differs: README.md:{la}\n{a}\nREADME.ja.md:{lb}\n{b}", i + 1)),
            (Some((la, a)), None) => problems.push(format!("README.md:{la} has this code block or marker, which README.ja.md lacks:\n{a}")),
            (None, Some((lb, b))) => problems.push(format!("README.ja.md:{lb} has this code block or marker, which README.md lacks:\n{b}")),
            (None, None) => {}
        }
    }
    let levels = |h: &[(usize, usize, String)]| h.iter().map(|(_, l, _)| *l).collect::<Vec<_>>();
    if levels(&en_headings) != levels(&ja_headings) {
        let list = |h: &[(usize, usize, String)]| h.iter().map(|(n, l, t)| format!("  {n}: {} {t}", "#".repeat(*l))).collect::<Vec<_>>().join("\n");
        problems.push(format!(
            "the headings differ in number or level:\nREADME.md\n{}\nREADME.ja.md\n{}",
            list(&en_headings),
            list(&ja_headings)
        ));
    }
    finish(problems);
}

// ---------------------------------------------------------------------------------------------
// 4. Links.

/// The text of inline links, `[text](target)`, without the targets.
fn link_texts(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else { break };
        let Some(end) = after[close + 2..].find(')') else { break };
        out.push_str(&rest[..open]);
        out.push_str(&after[..close]);
        rest = &after[close + 2 + end + 1..];
    }
    out.push_str(rest);
    out
}

/// A heading's text as GitHub renders it: link targets, emphasis marks (`*`, and `_` at the
/// edge of a word) and the backticks of code spans dropped. Inside a code span nothing is dropped.
fn rendered_heading(heading: &str) -> String {
    let text = link_texts(heading);
    let mut out = String::new();
    for (i, piece) in text.split('`').enumerate() {
        if i % 2 == 1 {
            out.push_str(piece);
            continue;
        }
        let chars: Vec<char> = piece.chars().collect();
        for (j, &c) in chars.iter().enumerate() {
            if c == '*' {
                continue;
            }
            if c == '_' {
                let before = chars[..j].iter().rev().find(|&&b| b != '_');
                let after = chars[j + 1..].iter().find(|&&a| a != '_');
                let inside_word = before.is_some_and(|b| b.is_alphanumeric()) && after.is_some_and(|a| a.is_alphanumeric());
                if !inside_word {
                    continue;
                }
            }
            out.push(c);
        }
    }
    out
}

/// The anchor GitHub gives a heading before repeats are numbered: the rendered text in lower
/// case, punctuation dropped, spaces turned into hyphens.
fn slug(heading: &str) -> String {
    rendered_heading(heading)
        .to_lowercase()
        .chars()
        .filter(|&c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The anchors of every heading of a document. A repeated anchor gets `-1`, `-2` and so on,
/// skipping any that an earlier heading already has, as GitHub does.
fn anchors(src: &str) -> BTreeSet<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = BTreeSet::new();
    for (_, item) in items(src) {
        if let Item::Heading { text, .. } = item {
            let base = slug(&text);
            let mut anchor = base.clone();
            while out.contains(&anchor) {
                let n = counts.entry(base.clone()).or_insert(0);
                *n += 1;
                anchor = format!("{base}-{n}");
            }
            out.insert(anchor);
        }
    }
    out
}

/// A link fragment with its `%XX` escapes decoded, so that `#%E4%BE%8B` reads as `#例`.
fn decoded(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = fragment.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The targets of the inline links and images on one line of text, outside code spans.
fn link_targets(line: &str) -> Vec<String> {
    let text = without_code_spans(line);
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(p) = rest.find("](") {
        let after = &rest[p + 2..];
        match after.find(')') {
            Some(end) => {
                let target = after[..end].split_whitespace().next().unwrap_or("").to_string();
                out.push(target);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// The reference definitions of a document, `[label]: target`, by label in lower case.
fn reference_definitions(src: &str) -> BTreeMap<String, (usize, String)> {
    let mut defs = BTreeMap::new();
    for (line, item) in items(src) {
        let Item::Text { text } = item else { continue };
        let t = text.trim_start();
        if text.len() - t.len() > 3 || !t.starts_with('[') {
            continue;
        }
        let Some((label, rest)) = t[1..].split_once("]:") else { continue };
        if let Some(target) = rest.split_whitespace().next() {
            defs.insert(label.to_lowercase(), (line, target.trim_matches(['<', '>']).to_string()));
        }
    }
    defs
}

/// The labels used by reference links on one line, `[text][label]` and `[text][]`.
fn reference_labels(line: &str) -> Vec<String> {
    let text = without_code_spans(line);
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(p) = rest.find("][") {
        let after = &rest[p + 2..];
        let Some(end) = after.find(']') else { break };
        let label = match &after[..end] {
            "" => rest[..p].rsplit_once('[').map(|(_, t)| t.to_string()).unwrap_or_default(),
            l => l.to_string(),
        };
        out.push(label);
        rest = &after[end + 1..];
    }
    out
}

/// The problems with the relative links of one document; returns them and how many links were seen.
/// Links are read from text and headings, inline and in reference form. A target starting with
/// `/` is read from the top folder of the repository, as GitHub does.
fn link_problems(doc: &str, src: &str) -> (Vec<String>, usize) {
    let mut problems = Vec::new();
    let defs = reference_definitions(src);
    let mut targets: Vec<(usize, String)> = Vec::new();
    for (line, item) in items(src) {
        let text = match item {
            Item::Text { text } => text,
            Item::Heading { text, .. } => text,
            _ => continue,
        };
        targets.extend(link_targets(&text).into_iter().map(|t| (line, t)));
        for label in reference_labels(&text) {
            if !defs.contains_key(&label.to_lowercase()) {
                problems.push(format!("{doc}:{line}: the reference link `[{label}]` has no definition `[{label}]: ...` in the document"));
            }
        }
    }
    targets.extend(defs.values().cloned());
    targets.sort();

    let mut checked = 0;
    let dir = Path::new(doc).parent().unwrap().to_path_buf();
    for (line, target) in targets {
        if target.contains("://") || target.starts_with("mailto:") {
            continue;
        }
        checked += 1;
        let (path, anchor) = match target.split_once('#') {
            Some((p, a)) => (p, Some(decoded(a))),
            None => (target.as_str(), None),
        };
        let file = match path.strip_prefix('/') {
            _ if path.is_empty() => PathBuf::from(doc),
            Some(from_root) => PathBuf::from(from_root),
            None => dir.join(path),
        };
        let target_src = if path.is_empty() {
            Some(src.to_string())
        } else {
            let full = root().join(&file);
            if !full.exists() {
                problems.push(format!("{doc}:{line}: the link `{target}` leads to `{}`, which does not exist", file.display()));
                continue;
            }
            std::fs::read_to_string(&full).ok()
        };
        if let Some(anchor) = anchor {
            let Some(target_src) = target_src else {
                problems.push(format!("{doc}:{line}: the link `{target}` has an anchor, but `{}` is not a text file", file.display()));
                continue;
            };
            let found = anchors(&target_src);
            if !found.contains(&anchor) {
                problems.push(format!(
                    "{doc}:{line}: the link `{target}` names the heading `#{anchor}`, which `{}` does not have; it has {found:?}",
                    file.display()
                ));
            }
        }
    }
    (problems, checked)
}

#[test]
fn every_relative_link_leads_to_a_file_and_heading_that_exist() {
    let mut problems = Vec::new();
    let mut checked = 0;
    for doc in documents() {
        let (p, n) = link_problems(&doc, &read(&doc));
        problems.extend(p);
        checked += n;
    }
    assert!(checked >= 20, "only {checked} links were checked");
    finish(problems);
}

#[test]
fn the_heading_anchors_follow_github() {
    assert_eq!(slug("Which connections are accepted"), "which-connections-are-accepted");
    assert_eq!(slug("The kind of API Gateway"), "the-kind-of-api-gateway");
    assert_eq!(slug("`set` and `choose`"), "set-and-choose");
    assert_eq!(slug("Answering with nat"), "answering-with-nat");
    assert_eq!(slug("文書と開発への参加"), "文書と開発への参加");
    assert_eq!(slug("例（VPC あり）"), "例vpc-あり");
    assert_eq!(anchors("# A\n## A\n## A\n"), ["a", "a-1", "a-2"].iter().map(|s| s.to_string()).collect());
    assert_eq!(code_spans("``\"`a -> b` is\"`` and `c`"), ["\"`a -> b` is\"", "c"]);
    assert_eq!(cells("| `a` | ``b | c`` |"), ["`a`", "``b | c``"]);
    assert_eq!(shell_words("set 'flows[0][1]' \"a b\" c"), ["set", "flows[0][1]", "a b", "c"]);
}

// ---------------------------------------------------------------------------------------------
// The readers above, tried on small documents.

fn example_problems(src: &str) -> Vec<String> {
    let mut problems = Vec::new();
    check_examples("try.md", src, &mut problems);
    problems
}

#[test]
fn a_code_fence_the_checks_cannot_read_is_named() {
    for fence in ["~~~yaml\nflows:\n  - user -> alb -> ecs\n~~~\n", "````yaml\nflows:\n  - user -> alb -> ecs\n````\n", "  ```yaml\nflows:\n  - user -> alb -> ecs\n  ```\n", "```yml\nflows:\n  - user -> alb -> ecs\n```\n", "```YAML\nflows:\n  - user -> alb -> ecs\n```\n"] {
        let problems = example_problems(fence);
        assert!(problems.iter().any(|p| p.contains("try.md:1:")), "not named: {fence:?} gave {problems:?}");
    }
    assert!(example_problems("```sh\nls\n```\n\n```text\nx\n```\n").is_empty());
}

#[test]
fn a_compile_or_run_marker_without_exit_is_named() {
    let compile = example_problems("<!-- archwire-test: compile -->\n```yaml\nflows:\n  - user -> alb -> ecs\n```\n");
    assert!(compile.iter().any(|p| p.contains("`exit=`")), "{compile:?}");
    let run = example_problems("<!-- archwire-test: compile exit=0 file=a.yaml -->\n```yaml\nflows:\n  - user -> alb -> ecs\n```\n\n<!-- archwire-test: run -->\n```sh\narchwire compile a.yaml -o a.drawio\n```\n");
    assert!(run.iter().any(|p| p.contains("`exit=`")), "{run:?}");
    let standalone = example_problems("<!-- archwire-test: compile exit=0 file=a.yaml -->\n```yaml\nflows:\n  - user -> alb -> ecs\n```\n\n<!-- archwire-test: compile file=a.yaml -->\n");
    assert!(standalone.iter().any(|p| p.contains("`exit=`")), "{standalone:?}");
}

#[test]
fn a_table_row_indented_by_a_space_is_still_read() {
    let (_, rows) = table("| H |\n|---|\n| a |\n | b |\n| c |\n", &["H"]).unwrap();
    assert_eq!(rows.iter().map(|(_, r)| r[0].as_str()).collect::<Vec<_>>(), ["a", "b", "c"]);
    let (_, rows) = table(" | H |\n |---|\n | a |\n", &["H"]).unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn anchors_drop_emphasis_and_do_not_collide() {
    let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<String>>();
    assert_eq!(anchors("# _Setup_\n# **Bold** and *it*\n# snake_case\n# `from_rule`\n"), set(&["setup", "bold-and-it", "snake_case", "from_rule"]));
    assert_eq!(anchors("# A\n# A\n# A-1\n"), set(&["a", "a-1", "a-1-1"]));
    assert_eq!(anchors("# See [the rules](rules.md)\n"), set(&["see-the-rules"]));
}

#[test]
fn links_are_read_in_headings_reference_form_and_from_the_root() {
    let (p, _) = link_problems("docs/try.md", "# 例\n\n[x](#%E4%BE%8B)\n");
    assert!(p.is_empty(), "{p:?}");
    let (p, _) = link_problems("docs/try.md", "## See [x](no-such-file.md)\n");
    assert!(p.iter().any(|p| p.contains("no-such-file.md")), "heading link not read: {p:?}");
    let (p, _) = link_problems("docs/try.md", "[x][r]\n\n[r]: no-such-file.md\n");
    assert!(p.iter().any(|p| p.contains("no-such-file.md")), "reference definition not read: {p:?}");
    let (p, _) = link_problems("docs/try.md", "[x][missing]\n");
    assert!(p.iter().any(|p| p.contains("missing")), "undefined reference not named: {p:?}");
    let (p, n) = link_problems("docs/try.md", "[x](/README.md) and [y](/docs/definition.md#services)\n");
    assert!(p.is_empty() && n == 2, "root links: {p:?}");
}
