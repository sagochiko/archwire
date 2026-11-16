//! Layout from the membership tree: the parts outside every frame of their own arrangement,
//! laid out in columns or rows along an axis, and frame items holding their own arrangement.
//!
//! The outside is one procedure written along a main axis `u` and a cross axis `v`; the axis
//! only decides at the end whether `(x, y)` is `(u, v)` or `(v, u)`. Names stay below their icons
//! and frame names at their frames' top-left corners whichever way the lines run.

mod azgrid;
mod check;
mod layered;
mod tree;

use crate::diag::Problem;
use crate::layout::{FrameKind, Layout};
use crate::model::{Axis, Model};
use crate::route::{LinePlan, Side, Through};
use crate::rules::Rules;

/// Membership: what the layout arranges. A part, or a frame holding its own arrangement.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Index into `Model.nodes`.
    Part(usize),
    Frame(FrameItem),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameItem {
    pub id: String,
    pub kind: FrameKind,
    pub arrange: Arrange,
    pub items: Vec<Item>,
}

/// How a frame's contents are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrange {
    /// Columns or rows by the longest main path, along `axis`.
    Layered { axis: Axis },
    /// Availability Zones side by side, tiers top to bottom: the inside of a VPC. Always runs down.
    AzGrid,
}

/// The inner part of a crossing line, in diagram coordinates. The LinePlan of `edge` carries the same Through.
#[derive(Debug, Clone, PartialEq)]
pub struct Crossing {
    pub edge: usize,
    pub frame: String,
    pub through: Through,
    pub inner_side: Side,
    pub inner_pos: u8,
}

/// A band kept for lines, in diagram coordinates: usable from `from` to `to` along it, `lo` to `hi` across,
/// and each drawn line's own track (the representative semantic edge and its coordinate across the band).
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    pub frame: String,
    pub horizontal: bool,
    pub from: i32,
    pub to: i32,
    pub lo: i32,
    pub hi: i32,
    pub tracks: Vec<(usize, i32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub layout: Layout,
    /// One draft per semantic edge, in model order.
    pub lines: Vec<LinePlan>,
    /// The distance between two neighbouring parts across the main axis: between the tops of
    /// rows when lines run right, between the left sides of columns when they run down.
    pub row_step: i32,
    /// The centre of each main column (lines running right) or row (lines running down) in the
    /// Region, along the main axis, from the first.
    pub col_x: Vec<i32>,
    /// Empty without a VPC.
    pub crossings: Vec<Crossing>,
    /// Empty without a VPC.
    pub lanes: Vec<Lane>,
}

/// The axis a model is drawn along when its definition writes no direction: down, for every model.
pub fn default_axis(_model: &Model) -> Axis {
    Axis::Down
}

/// The layout, or LAYOUT_FAILED problems when a must-hold condition fails after laying out.
///
/// The inside of a VPC is laid out on its own and then placed as one piece among the parts
/// outside it. Once every part has its place, the lines are gathered as the router gathers them,
/// and each band keeps the tracks of the lines that are drawn; a line that is not drawn keeps no track.
///
/// A part spanning the zones sits on the boundary between them when there are two, else in a row
/// of its own above them; the VPC comes first among the parts of its rank.
pub fn arrange(model: &Model, rules: &Rules, axis: Axis) -> Result<Placement, Vec<Problem>> {
    let mut inside: Vec<usize> = Vec::new();
    for item in tree::tree(model) {
        if let Item::Frame(f) = item {
            if f.arrange == Arrange::AzGrid {
                inside.extend(parts(&f.items));
            }
        }
    }
    let placed = match inside.is_empty() {
        true => None,
        false => Some(azgrid::lay_vpc(model, rules, &inside, axis)?),
    };
    let piece = placed.as_ref().map(|p| layered::Piece { placed: p });
    let mut placement = layered::layered(model, rules, axis, piece.as_ref());
    let mut problems = Vec::new();
    if placed.is_some() {
        let reps: Vec<usize> = crate::lines::gather(model, &placement.layout, rules).0.iter().map(|l| l.rep).collect();
        let bands: Vec<Lane> = placement.lanes.clone();
        for lane in &mut placement.lanes {
            lane.tracks.retain(|t| reps.contains(&t.0));
        }
        placement.lanes.retain(|l| !l.tracks.is_empty());
        for pl in placement.lines.iter_mut().filter(|pl| !reps.contains(&pl.edge)) {
            pl.tracks.clear();
        }
        problems.extend(check::fixed_ends_apart(model, &placement, &reps));
        let needs: Vec<usize> = placed.as_ref().map(|p| p.needs.clone()).unwrap_or_default();
        problems.extend(check::bands_kept(model, &placement, &bands, &reps, &needs));
    }
    problems.extend(check::check(model, &placement));
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(placement)
}

/// The parts among `items` and in their frames, in tree order.
fn parts(items: &[Item]) -> Vec<usize> {
    items
        .iter()
        .flat_map(|it| match it {
            Item::Part(i) => vec![*i],
            Item::Frame(f) => parts(&f.items),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::azgrid::tests::{generated_with_a_vpc, grid, inputs, FEWER_READERS, WEB_CASES};
    use super::layered::tests::API_CASES;
    use super::*;
    use crate::diag::Diagnostics;

    /// The diagrams written for viewing: (file name stem, input name). Each is written with lines
    /// running right and with lines running down.
    const SAMPLES: [(&str, &str); 17] = [
        ("web-2az-ja", "web-2az-ja"),
        ("web-3az-ja", "web-3az-ja"),
        ("mix", "mix-min"),
        ("api-http-ja", "api-http-ja"),
        ("aux-to-side", "aux-to-side"),
        ("endpoint-3az-en", "endpoint-3az-en"),
        ("endpoint-3az-ja", "endpoint-3az-ja"),
        ("endpoint-ja", "endpoint-ja"),
        ("hole-s3", "hole-s3"),
        ("web-2az-en", "web-2az-en"),
        ("web-3az-en", "web-3az-en"),
        ("web-attach-en", "web-attach-en"),
        ("readers", "fewer-readers"),
        ("natsingle", "web-3az-ja-single"),
        ("samerow", "mix-same-row"),
        ("shared", "mix-shared"),
        ("longnames", "mix-long"),
    ];

    // Fails if a model that writes no direction is drawn along any axis but down, with a VPC or without.
    #[test]
    fn models_run_down_unless_written_otherwise() {
        let r = Rules::builtin();
        let mut models: Vec<Model> = inputs().into_iter().map(|(_, m, _)| m).collect();
        for (_, src) in API_CASES {
            models.push(crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap());
        }
        for m in &models {
            assert_eq!(default_axis(m), Axis::Down);
        }
    }

    /// A definition with its direction written in its settings.
    fn with_direction(src: &str, way: &str) -> String {
        match src.find("set:\n") {
            Some(at) => format!("{}  direction: {way}\n{}", &src[..at + 5], &src[at + 5..]),
            None => format!("{src}set:\n  direction: {way}\n"),
        }
    }

    /// How many diagrams of the new layout `arranged_diagrams_for_the_checker` writes: twenty
    /// inputs, each along both axes.
    const ARRANGED: usize = 40;

    /// The inputs drawn with the new layout for the diagram checker: the definition, or the name
    /// of a model built by hand, and the model. Nine fixed shapes first: the web examples
    /// with two and three zones, three zones with one Reader, three zones with one NAT gateway,
    /// WAF and ACM, and the four models mixing Lambda in the VPC with parts outside it. Then
    /// eleven taken evenly from the generated definitions whose models are none of those.
    fn arranged_for_the_checker() -> Vec<(Result<String, String>, Model)> {
        let single = WEB_CASES[2].1.replace("  azs: 3\n", "  azs: 3\n  nat: single\n");
        let fixed: Vec<String> = vec![WEB_CASES[0].1.into(), WEB_CASES[2].1.into(), FEWER_READERS.into(), single, WEB_CASES[9].1.into()];
        let mut out: Vec<(Result<String, String>, Model)> = fixed.iter().map(|s| (Ok(s.clone()), grid(s).0)).collect();
        for (name, m, _) in inputs().into_iter().filter(|(n, _, _)| n.starts_with("mix-")) {
            out.push((Err(name), m));
        }
        assert_eq!(out.len(), 9);
        let r = Rules::builtin();
        let defs: Vec<(String, Model)> = generated_with_a_vpc()
            .into_iter()
            .filter_map(|src| {
                let (m, d) = crate::infer::infer(&crate::def::parse(&src).unwrap(), &r);
                assert!(d.errors.is_empty(), "{:?}\n{src}", d.errors);
                m.map(|m| (src, m))
            })
            .filter(|(_, m)| !out.iter().any(|(_, o)| o == m))
            .collect();
        assert!(defs.len() > 100, "{}", defs.len());
        let n = ARRANGED / 2 - out.len();
        for k in 0..n {
            let (src, m) = &defs[k * defs.len() / n];
            out.push((Ok(src.clone()), m.clone()));
        }
        out
    }

    // Fails if a diagram of the new layout handed to the checker cannot be laid out or breaks a line.
    /// With `ARCHWIRE_GEN_OUT=<dir>`, removes the `v<number>-<axis>` files a run left there, then
    /// writes each input drawn along both axes (a definition by `compile`, a model built by hand by the same layout) as `<dir>/v<number>-<axis>.drawio`
    /// and `<dir>/v<number>-<axis>.yaml`: the definition with its direction written, or one comment
    /// line naming a model built by hand. Without it, only draws them.
    #[test]
    fn arranged_diagrams_for_the_checker() {
        let picked = arranged_for_the_checker();
        let r = Rules::builtin();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::var_os("ARCHWIRE_GEN_OUT").map(|d| root.join(d));
        let ours = |stem: &str| {
            let b = stem.as_bytes();
            b.len() > 4 && b[0] == b'v' && b[1].is_ascii_digit() && b[2].is_ascii_digit() && (stem.ends_with("-right") || stem.ends_with("-down"))
        };
        if let Some(d) = &dir {
            std::fs::create_dir_all(d).unwrap();
            for entry in std::fs::read_dir(d).unwrap() {
                let path = entry.unwrap().path();
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ours(stem) && ["yaml", "drawio"].contains(&ext) {
                    std::fs::remove_file(&path).unwrap();
                }
            }
        }
        let mut failed = Vec::new();
        let mut written = 0;
        for (k, (src, model)) in picked.iter().enumerate() {
            for (axis, way) in [(Axis::Right, "right"), (Axis::Down, "down")] {
                let name = format!("v{:02}-{way}", k + 1);
                // A definition goes through compile, with its direction written; a model built
                // by hand is laid out, routed and written the way compile does.
                let (drawn, yaml) = match src {
                    Ok(s) => {
                        let s = with_direction(s, way);
                        assert_eq!(grid(&s).0.direction, Some(axis), "{name}");
                        let out = crate::compile(&s);
                        let drawn = match out.drawio {
                            Some(x) if out.diagnostics.errors.is_empty() => Ok((x, out.diagnostics.notes)),
                            _ => Err(out.diagnostics.errors),
                        };
                        (drawn, s)
                    }
                    Err(hand) => (crate::arranged_drawio(model, &Diagnostics::default(), &r, axis), format!("# hand-built model: {hand}\n")),
                };
                match drawn {
                    Ok((x, notes)) => {
                        let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                        if !broken.is_empty() {
                            failed.push(format!("{name}: {broken:?}\n{yaml}"));
                        }
                        if let Some(d) = &dir {
                            std::fs::write(d.join(format!("{name}.yaml")), &yaml).unwrap();
                            std::fs::write(d.join(format!("{name}.drawio")), x).unwrap();
                        }
                        written += 1;
                    }
                    Err(e) => failed.push(format!("{name}: {e:?}\n{yaml}")),
                }
            }
        }
        assert!(failed.is_empty(), "{} of {ARRANGED} diagrams failed:\n{}", failed.len(), failed.join("\n"));
        assert_eq!(written, ARRANGED);
    }

    // With the folder given, fails if a sample cannot be laid out or a line breaks or leaves its draft.
    /// With `ARCHWIRE_SAMPLE_OUT=<dir>`, writes each sample as `<dir>/<name>-right.drawio` and
    /// `<dir>/<name>-down.drawio`, for the diagram checker, and `<dir>/notes.tsv` with the router's
    /// notes of each diagram. Without it, does nothing.
    #[test]
    fn sample_diagrams() {
        let Some(dir) = std::env::var_os("ARCHWIRE_SAMPLE_OUT") else { return };
        // A relative folder is taken from the workspace root, where the command is typed.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Rules::builtin();
        let mut models: Vec<(String, Model, Diagnostics)> = inputs();
        for (name, src) in API_CASES {
            let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
            models.push((name.to_string(), m.unwrap_or_else(|| panic!("{name}: {:?}", d.errors)), d));
        }
        let mut report = String::from("diagram\tnotes\tbroken\tall notes\n");
        let mut broken_files = Vec::new();
        for (stem, input) in SAMPLES {
            let (_, m, d) = models.iter().find(|(n, _, _)| n == input).unwrap_or_else(|| panic!("no input {input}"));
            for (axis, way) in [(Axis::Right, "right"), (Axis::Down, "down")] {
                let file = format!("{stem}-{way}");
                let (x, notes) = crate::arranged_drawio(m, d, &r, axis).unwrap_or_else(|e| panic!("{file}: {e:?}"));
                let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                report.push_str(&format!("{file}\t{}\t{}\t{notes:?}\n", notes.len(), broken.len()));
                if !broken.is_empty() {
                    broken_files.push(file.clone());
                }
                std::fs::write(dir.join(format!("{file}.drawio")), x).unwrap();
            }
        }
        std::fs::write(dir.join("notes.tsv"), report).unwrap();
        assert!(broken_files.is_empty(), "lines broken or off their drafts in {broken_files:?}");
    }
}
