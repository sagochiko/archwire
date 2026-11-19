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

fn list() -> (Option<i32>, String) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_archwire")).arg("list").output().unwrap();
    (o.status.code(), String::from_utf8(o.stdout).unwrap())
}

#[test]
fn list_prints_the_services_connections_and_settings_as_json() {
    let (code, stdout) = list();
    assert_eq!(code, Some(0));
    assert!(stdout.ends_with("}\n"), "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["links", "services", "settings", "versions"]);
    // The fields come in this order in the output.
    assert!(stdout.find("\"versions\"").unwrap() < stdout.find("\"services\"").unwrap());
    assert!(stdout.find("\"services\"").unwrap() < stdout.find("\"links\"").unwrap());
    assert!(stdout.find("\"links\"").unwrap() < stdout.find("\"settings\"").unwrap());

    // The same versions as the report of a compile.
    let dir = std::env::temp_dir().join(format!("archwire-cli-list-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let def = dir.join("a.yaml");
    std::fs::write(&def, "flows:\n  - user -> alb -> ecs\n").unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_archwire")).args(["compile", def.to_str().unwrap(), "-o", dir.join("a.drawio").to_str().unwrap()]).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["versions"], report["versions"]);

    let services = v["services"].as_array().unwrap();
    let service = |name: &str| services.iter().find(|s| s["name"] == name).cloned();
    let ecs = service("ecs").expect("ecs");
    assert_eq!(ecs, serde_json::json!({"name": "ecs", "label": "Amazon ECS", "kinds": ["fargate"], "in_vpc": true}));
    assert_eq!(service("apigateway").unwrap()["kinds"], serde_json::json!(["http", "rest"]));
    assert_eq!(service("s3").unwrap(), serde_json::json!({"name": "s3", "label": "Amazon S3", "kinds": [], "in_vpc": false}));
    assert_eq!(service("user").unwrap()["label"], "Users");
    assert!(service("internet").is_some());
    assert!(service("nat").is_none() && service("igw").is_none());
    let names: Vec<&str> = services.iter().map(|s| s["name"].as_str().unwrap()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names, sorted, "services must be in name order, each once");

    let links = v["links"].as_array().unwrap();
    let link = |from: &str, symbol: &str, to: &str| links.iter().find(|l| l["from"] == from && l["symbol"] == symbol && l["to"] == to).cloned();
    assert_eq!(
        link("user", "->", "cloudfront").unwrap(),
        serde_json::json!({"from": "user", "to": "cloudfront", "symbol": "->", "label": "HTTPS", "kind_labels": {}, "role_labels": {}})
    );
    let cognito = link("cognito", "--", "apigateway").unwrap();
    let kinds: Vec<&String> = cognito["kind_labels"].as_object().unwrap().keys().collect();
    assert_eq!(kinds, ["http", "rest"]);
    assert!(links.iter().any(|l| l["symbol"] == "~>"));
    assert!(links.iter().any(|l| l["role_labels"].as_object().unwrap().contains_key("writer")));
    assert!(link("user", "->", "dynamodb").is_none());

    let settings = v["settings"].as_array().unwrap();
    let keys: Vec<&str> = settings.iter().map(|s| s["key"].as_str().unwrap()).collect();
    assert_eq!(keys, ["azs", "direction", "lang", "nat", "readers", "region"]);
    let setting = |key: &str| settings.iter().find(|s| s["key"] == key).unwrap().clone();
    assert_eq!(setting("azs"), serde_json::json!({"key": "azs", "values": ["2", "3"], "vpc_only": true}));
    assert_eq!(setting("direction"), serde_json::json!({"key": "direction", "values": ["down", "right"], "vpc_only": false}));
    assert_eq!(setting("readers")["values"], serde_json::json!(["1", "2"]));
}

#[test]
fn list_prints_the_same_bytes_every_time() {
    let (a, b) = (list(), list());
    assert_eq!(a.0, Some(0));
    assert_eq!(a, b);
}
