#[test]
fn compile_writes_the_file_and_exit_codes_follow_the_diagnostics() {
    let dir = std::env::temp_dir().join("archwire-cli-8");
    std::fs::create_dir_all(&dir).unwrap();
    let run = |name: &str, src: &str| {
        let def = dir.join(format!("{name}.yaml")); std::fs::write(&def, src).unwrap();
        let out = dir.join(format!("{name}.drawio")); let _ = std::fs::remove_file(&out);
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_archwire")).args(["compile", def.to_str().unwrap(), "-o", out.to_str().unwrap()]).output().unwrap();
        (o.status.code(), out.exists())
    };
    assert_eq!(run("ok", "flows:\n  - user -> alb -> ecs\n"), (Some(0), true));
    assert_eq!(run("hole", "flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n"), (Some(2), true));
    assert_eq!(run("bad", "flows:\n  - user -> dynamodb\n"), (Some(1), false));
}

#[test]
fn compile_that_cannot_read_the_definition_prints_diagnostics_and_exits_1() {
    let dir = std::env::temp_dir().join("archwire-cli-cannot-read");
    std::fs::create_dir_all(&dir).unwrap();
    let def = dir.join("missing.yaml");
    let _ = std::fs::remove_file(&def);
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archwire"))
        .args(["compile", def.to_str().unwrap(), "-o", dir.join("a.drawio").to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"CANNOT_READ\"") && stdout.contains("missing.yaml"));
}

#[test]
fn compile_that_cannot_write_the_output_prints_diagnostics_and_exits_1() {
    let dir = std::env::temp_dir().join("archwire-cli-cannot-write");
    std::fs::create_dir_all(&dir).unwrap();
    let def = dir.join("a.yaml");
    std::fs::write(&def, "flows:\n  - user -> alb -> ecs\n").unwrap();
    let target = dir.join("no-such-dir").join("a.drawio");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archwire"))
        .args(["compile", def.to_str().unwrap(), "-o", target.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"CANNOT_WRITE\"") && stdout.contains("no-such-dir"));
    assert_eq!(stdout.matches("\"versions\"").count(), 1);
}

#[test]
fn set_rewrites_the_file_in_place_and_prints_nothing() {
    let dir = std::env::temp_dir().join("archwire-cli-set-ok");
    std::fs::create_dir_all(&dir).unwrap();
    let def = dir.join("a.yaml");
    std::fs::write(&def, "flows:\n  - user -> cloudfrnt\n").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archwire"))
        .args(["set", def.to_str().unwrap(), "flows[0][1]", "cloudfront"]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert_eq!(std::fs::read_to_string(&def).unwrap(), "flows:\n  - user -> cloudfront\n");
}

#[test]
fn set_with_a_bad_path_prints_diagnostics_and_exits_1() {
    let dir = std::env::temp_dir().join("archwire-cli-set-bad");
    std::fs::create_dir_all(&dir).unwrap();
    let def = dir.join("a.yaml");
    std::fs::write(&def, "flows:\n  - user -> cloudfront\n").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_archwire"))
        .args(["set", def.to_str().unwrap(), "flows[9][0]", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"errors\"") && stdout.contains("BAD_PATH"));
    assert_eq!(std::fs::read_to_string(&def).unwrap(), "flows:\n  - user -> cloudfront\n");
}

#[test]
fn a_failed_compile_removes_the_previous_output_file() {
    let dir = std::env::temp_dir().join(format!("archwire-cli-stale-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (def, out) = (dir.join("a.yaml"), dir.join("a.drawio"));
    let compile = || std::process::Command::new(env!("CARGO_BIN_EXE_archwire")).args(["compile", def.to_str().unwrap(), "-o", out.to_str().unwrap()]).output().unwrap();
    std::fs::write(&def, "flows:\n  - user -> alb -> ecs\n").unwrap();
    assert_eq!(compile().status.code(), Some(0));
    assert!(out.exists());
    std::fs::write(&def, "flows:\n  - user -> dynamodb\n").unwrap();
    assert_eq!(compile().status.code(), Some(1));
    assert!(!out.exists());
    std::fs::write(&out, "old").unwrap();
    std::fs::remove_file(&def).unwrap();
    let o = compile();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8(o.stdout).unwrap().contains("CANNOT_READ"));
    assert!(!out.exists());
}

#[test]
#[allow(clippy::permissions_set_readonly_false)]
fn set_that_cannot_read_or_write_prints_diagnostics_with_both_versions() {
    let dir = std::env::temp_dir().join(format!("archwire-cli-set-io-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let set = |def: &std::path::Path, at: &str| {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_archwire")).args(["set", def.to_str().unwrap(), at, "3"]).output().unwrap();
        (o.status.code(), String::from_utf8(o.stdout).unwrap())
    };
    let rules_version = "\"rules\": \"2026.";
    let missing = dir.join("missing.yaml");
    let _ = std::fs::remove_file(&missing);
    let (code, json) = set(&missing, "set.azs");
    assert_eq!(code, Some(1));
    assert!(json.contains("\"CANNOT_READ\"") && json.contains(rules_version), "{json}");
    let locked = dir.join("locked.yaml");
    let _ = std::fs::remove_file(&locked);
    std::fs::write(&locked, "flows:\n  - user -> alb -> ecs\n").unwrap();
    let mut perm = std::fs::metadata(&locked).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&locked, perm.clone()).unwrap();
    let (code, json) = set(&locked, "set.azs");
    perm.set_readonly(false);
    std::fs::set_permissions(&locked, perm).unwrap();
    assert_eq!(code, Some(1));
    assert!(json.contains("\"CANNOT_WRITE\"") && json.contains(rules_version), "{json}");
    let (code, json) = set(&locked, "flows[9][0]");
    assert_eq!(code, Some(1));
    assert!(json.contains("BAD_PATH") && json.contains(rules_version), "{json}");
}

#[test]
fn answering_the_kind_hole_with_set_turns_exit_2_into_0() {
    let dir = std::env::temp_dir().join(format!("archwire-cli-kind-hole-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (def, out) = (dir.join("a.yaml"), dir.join("a.drawio"));
    std::fs::write(&def, "flows:\n  - user -> apigateway -> lambda\n").unwrap();
    let compile = || std::process::Command::new(env!("CARGO_BIN_EXE_archwire")).args(["compile", def.to_str().unwrap(), "-o", out.to_str().unwrap()]).output().unwrap();
    let first = compile();
    assert_eq!(first.status.code(), Some(2));
    assert!(String::from_utf8(first.stdout).unwrap().contains("\"kind.apigateway\""));
    let set = std::process::Command::new(env!("CARGO_BIN_EXE_archwire"))
        .args(["set", def.to_str().unwrap(), "choose.kind.apigateway", "http"]).output().unwrap();
    assert_eq!(set.status.code(), Some(0));
    let second = compile();
    assert_eq!(second.status.code(), Some(0), "{}", String::from_utf8_lossy(&second.stdout));
    assert!(std::fs::read_to_string(&out).unwrap().contains("label=\"Amazon API Gateway&#xa;HTTP API\""));
}
