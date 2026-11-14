pub mod def;
pub mod diag;
pub mod drawio;
pub mod edit;
pub mod infer;
pub mod layout;
pub mod model;
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
    let def = match def::parse(def_text) {
        Ok(d) => d,
        Err(errors) => {
            let versions = Versions { archwire: env!("CARGO_PKG_VERSION").to_string(), rules: rules.version.clone() };
            let diagnostics = Diagnostics { versions, errors, ..Default::default() };
            return Output { drawio: None, diagnostics };
        }
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
