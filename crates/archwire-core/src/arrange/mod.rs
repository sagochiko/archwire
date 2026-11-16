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
        problems.extend(check::bands_kept(model, &placement, &bands, &reps));
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
    use super::azgrid::tests::inputs;
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
