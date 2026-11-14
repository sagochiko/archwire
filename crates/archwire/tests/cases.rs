//! Acceptance cases: each folder under `tests/cases` holds a definition (`def.yaml`), the
//! diagnostics it must print (`expected.json`) and, unless it is an error case, the diagram it
//! must write (`expected.drawio`). Outputs must match byte for byte.
//!
//! `ARCHWIRE_BLESS=1 cargo test -p archwire --test cases` rewrites the expected files from the
//! current output instead of comparing. Run `sh tools/check_cases.sh` and look at the rendered
//! images before committing blessed files.

use std::path::{Path, PathBuf};
use std::process::Command;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases")
}

/// Where two texts first differ, as a short note for the failure message.
fn first_difference(want: &[u8], got: &[u8]) -> String {
    let (want, got) = (String::from_utf8_lossy(want), String::from_utf8_lossy(got));
    let (mut w, mut g) = (want.lines(), got.lines());
    for n in 1.. {
        match (w.next(), g.next()) {
            (Some(a), Some(b)) if a == b => continue,
            (None, None) => return "the texts differ only in line endings or a final newline".into(),
            (a, b) => {
                return format!(
                    "first difference at line {n}:\n  expected: {}\n  actual:   {}",
                    a.unwrap_or("(end of file)"),
                    b.unwrap_or("(end of file)")
                )
            }
        }
    }
    unreachable!()
}

/// The exit code the diagnostics call for: 1 with errors, 2 with holes, else 0.
fn exit_code_for(json: &str) -> i32 {
    if !json.contains("\"errors\": []") {
        1
    } else if !json.contains("\"holes\": []") {
        2
    } else {
        0
    }
}

#[test]
fn every_case_matches_its_expected_output() {
    let bless = std::env::var("ARCHWIRE_BLESS").as_deref() == Ok("1");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(cases_dir()).unwrap().map(|e| e.unwrap().path()).collect();
    dirs.sort();
    let mut failures = Vec::new();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let work = std::env::temp_dir().join(format!("archwire-case-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&work).unwrap();
        let out = work.join("out.drawio");
        let _ = std::fs::remove_file(&out);
        let run = Command::new(env!("CARGO_BIN_EXE_archwire"))
            .args(["compile", dir.join("def.yaml").to_str().unwrap(), "-o", out.to_str().unwrap()])
            .output()
            .unwrap();
        let json = run.stdout;
        let drawio = std::fs::read(&out).ok();
        let _ = std::fs::remove_dir_all(&work);
        let (want_json, want_drawio) = (dir.join("expected.json"), dir.join("expected.drawio"));

        if bless {
            std::fs::write(&want_json, &json).unwrap();
            match &drawio {
                Some(d) => std::fs::write(&want_drawio, d).unwrap(),
                None => {
                    if std::fs::remove_file(&want_drawio).is_ok() {
                        println!("case {name}: no diagram was written, so expected.drawio was removed");
                    }
                }
            }
            continue;
        }

        let fix = "if the change is intended, run `ARCHWIRE_BLESS=1 cargo test -p archwire --test cases`, then `sh tools/check_cases.sh`, and look at the images";
        match std::fs::read(&want_json) {
            Ok(want) if want == json => {}
            Ok(want) => failures.push(format!("case {name}: expected.json differs from the printed diagnostics; {}\n{fix}", first_difference(&want, &json))),
            Err(e) => failures.push(format!("case {name}: cannot read expected.json: {e}")),
        }
        let code = run.status.code();
        let want_code = exit_code_for(&String::from_utf8_lossy(&json));
        if code != Some(want_code) {
            failures.push(format!("case {name}: exit code {code:?}, but the diagnostics call for {want_code}"));
        }
        match (std::fs::read(&want_drawio).ok(), drawio) {
            (Some(want), Some(got)) if want == got => {}
            (Some(want), Some(got)) => failures.push(format!("case {name}: expected.drawio differs from the written diagram; {}\n{fix}", first_difference(&want, &got))),
            (Some(_), None) => failures.push(format!("case {name}: expected.drawio exists but no diagram was written")),
            (None, Some(_)) => failures.push(format!("case {name}: a diagram was written but the case has no expected.drawio")),
            (None, None) => {}
        }
    }
    assert!(failures.is_empty(), "\n{}\n", failures.join("\n\n"));
}

#[test]
fn case_list_is_what_the_plan_says() {
    let mut names: Vec<String> = std::fs::read_dir(cases_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["aux-to-side", "endpoint-ja", "err-unknown-service", "err-unsupported", "hole-s3", "web-2az-en", "web-2az-ja", "web-3az-en", "web-3az-ja"]);
}

#[test]
fn error_cases_have_no_expected_diagram() {
    for name in ["err-unknown-service", "err-unsupported"] {
        assert!(!cases_dir().join(name).join("expected.drawio").exists(), "{name}");
    }
}
