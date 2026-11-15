pub mod def;
pub mod diag;
pub mod drawio;
pub mod edit;
pub mod engine;
pub mod flow_infer;
pub mod infer;
pub mod layout;
pub mod model;
pub mod place;
pub mod route;
pub mod rules;

use diag::{Diagnostics, Versions};
use rules::Rules;

pub struct Output {
    pub drawio: Option<String>,
    pub diagnostics: Diagnostics,
}

/// The versions every diagnostics report carries: this program's and the built-in rules'.
pub fn versions() -> Versions {
    Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: Rules::builtin().version }
}

/// parse, infer, layout, route, write. Errors give no diagram; holes still do.
pub fn compile(def_text: &str) -> Output {
    let rules = Rules::builtin();
    let stop = |errors| {
        let versions = Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() };
        Output { drawio: None, diagnostics: Diagnostics { versions, errors, ..Default::default() } }
    };
    let def = match def::parse(def_text) {
        Ok(d) => d,
        Err(errors) => return stop(errors),
    };
    // Which layout the definition goes through is decided here, once. Both still take the VPC
    // layout below; a definition mixing VPC parts with services kept apart from them stops here.
    let _engine = match engine::pick(&def, &rules) {
        Ok(e) => e,
        Err(errors) => return stop(errors),
    };
    let (model, mut diagnostics) = infer::infer(&def, &rules);
    let Some(model) = model else {
        return Output { drawio: None, diagnostics };
    };
    let layout = layout::layout(&model, &rules);
    let (wires, notes) = route::route(&model, &layout, &rules);
    diagnostics.notes.extend(notes);
    let drawio = drawio::write(&model, &layout, &wires, &diagnostics, &rules);
    Output { drawio: Some(drawio), diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_definition_mixing_vpc_parts_with_separate_services_stops_before_inference() {
        let web = "flows:\n  - user -> cloudfront -> alb -> ecs\n";
        let cases = [
            ("flows:\n  - user -> alb -> ecs -> lambda\n".to_string(), "flows[0][3]", "lambda"),
            (format!("{web}  - cloudfront -> lambda\n"), "flows[1][1]", "lambda"),
            (format!("{web}  - ecs ~> cognito\n"), "flows[1][1]", "cognito"),
            (format!("{web}  - cognito -- alb\n"), "flows[1][0]", "cognito"),
            (format!("{web}  - ecs -> dynamodb.orders\n"), "flows[1][1]", "dynamodb"),
            (format!("{web}  - cloudfront -> apigateway.public(rest)\n"), "flows[1][1]", "apigateway"),
        ];
        for (src, at, got) in &cases {
            let out = compile(src);
            assert!(out.drawio.is_none(), "{src}");
            let d = &out.diagnostics;
            assert_eq!(d.exit_code(), 1, "{src}");
            assert_eq!(d.errors.len(), 1, "{src}: {:?}", d.errors);
            let e = &d.errors[0];
            assert_eq!((e.code.as_str(), e.at.as_deref(), e.got.as_deref()), ("UNSUPPORTED", Some(*at), Some(*got)), "{src}");
            assert!(e.message.contains("cannot be drawn together with VPC services"), "{src}: {}", e.message);
            assert_eq!(d.versions.rules, Rules::builtin().version);
            assert!(d.derived.is_empty() && d.holes.is_empty() && d.notes.is_empty());
        }
    }

    #[test]
    fn a_definition_without_a_vpc_still_stops_in_the_vpc_inference() {
        let out = compile("flows:\n  - user -> apigateway(http) -> lambda -> dynamodb\n");
        assert!(out.drawio.is_none());
        assert_eq!(out.diagnostics.exit_code(), 1);
        assert!(out.diagnostics.errors.iter().all(|e| e.code == "UNSUPPORTED"));
    }
}
