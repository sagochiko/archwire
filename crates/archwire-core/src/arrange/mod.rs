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

/// Layout choices that still have two forms each; a choice goes away once one form is settled on.
#[derive(Debug, Clone, Copy)]
pub struct Choices {
    pub span_row: SpanRow,
    pub block_order: BlockOrder,
}

/// Where a part spanning the Availability Zones goes.
#[derive(Debug, Clone, Copy)]
pub enum SpanRow {
    /// In a row of its own above the zones.
    AboveZones,
    /// On the boundary between the zones when there are two.
    OnBoundaryWithTwoZones,
}

/// Where the VPC goes among the outside parts of its rank.
#[derive(Debug, Clone, Copy)]
pub enum BlockOrder {
    First,
    Last,
}

/// The layout, or LAYOUT_FAILED problems when a must-hold condition fails after laying out.
///
/// The inside of a VPC is laid out on its own and then placed as one piece among the parts
/// outside it. Once every part has its place, the lines are gathered as the router gathers them,
/// and each band keeps the tracks of the lines that are drawn; a line that is not drawn keeps no track.
pub fn arrange(model: &Model, rules: &Rules, axis: Axis, choices: Choices) -> Result<Placement, Vec<Problem>> {
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
        false => Some(azgrid::lay_vpc(model, rules, &inside, axis, choices)?),
    };
    let piece = placed.as_ref().map(|p| layered::Piece { placed: p, first: matches!(choices.block_order, BlockOrder::First) });
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

    /// The forms the diagrams are drawn in: spanning parts on the boundary between two zones, the
    /// VPC first in its rank. The other two forms are each written once, to compare.
    const DRAWN: Choices = Choices { span_row: SpanRow::OnBoundaryWithTwoZones, block_order: BlockOrder::First };
    const SPAN_ROW_ABOVE: Choices = Choices { span_row: SpanRow::AboveZones, ..DRAWN };
    const BLOCK_LAST: Choices = Choices { block_order: BlockOrder::Last, ..DRAWN };

    /// The diagrams written for viewing: (file name stem, input name, choices). Each is written
    /// with lines running right and with lines running down.
    const SAMPLES: [(&str, &str, Choices); 21] = [
        ("web-2az-ja", "web-2az-ja", DRAWN),
        ("web-3az-ja", "web-3az-ja", DRAWN),
        ("mix", "mix-min", DRAWN),
        ("api-http-ja", "api-http-ja", DRAWN),
        ("alb-above", "web-2az-ja", SPAN_ROW_ABOVE),
        ("alb-boundary", "web-2az-ja", DRAWN),
        ("s3-after", "web-2az-ja", DRAWN),
        ("s3-before", "web-2az-ja", BLOCK_LAST),
        ("aux-to-side", "aux-to-side", DRAWN),
        ("endpoint-3az-en", "endpoint-3az-en", DRAWN),
        ("endpoint-3az-ja", "endpoint-3az-ja", DRAWN),
        ("endpoint-ja", "endpoint-ja", DRAWN),
        ("hole-s3", "hole-s3", DRAWN),
        ("web-2az-en", "web-2az-en", DRAWN),
        ("web-3az-en", "web-3az-en", DRAWN),
        ("web-attach-en", "web-attach-en", DRAWN),
        ("readers", "fewer-readers", DRAWN),
        ("natsingle", "web-3az-ja-single", DRAWN),
        ("samerow", "mix-same-row", DRAWN),
        ("shared", "mix-shared", DRAWN),
        ("longnames", "mix-long", DRAWN),
    ];

    // With the folder given, fails if a sample cannot be laid out or a line breaks or leaves its draft.
    /// With `ARCHWIRE_PROTO_OUT=<dir>`, writes each sample as `<dir>/<name>-right.drawio` and
    /// `<dir>/<name>-down.drawio`, for the diagram checker, and `<dir>/notes.tsv` with the router's
    /// notes of each diagram. Without it, does nothing.
    #[test]
    fn prototype_samples() {
        let Some(dir) = std::env::var_os("ARCHWIRE_PROTO_OUT") else { return };
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
        for (stem, input, choices) in SAMPLES {
            let (_, m, d) = models.iter().find(|(n, _, _)| n == input).unwrap_or_else(|| panic!("no input {input}"));
            for (axis, way) in [(Axis::Right, "right"), (Axis::Down, "down")] {
                let file = format!("{stem}-{way}");
                let (x, notes) = crate::arranged_drawio(m, d, &r, axis, choices).unwrap_or_else(|e| panic!("{file}: {e:?}"));
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
