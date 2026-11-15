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
    let placed = (!inside.is_empty()).then(|| azgrid::lay_vpc(model, rules, &inside, axis, choices));
    let piece = placed.as_ref().map(|p| layered::Piece { placed: p, first: matches!(choices.block_order, BlockOrder::First) });
    let mut placement = layered::layered(model, rules, axis, piece.as_ref());
    if placed.is_some() {
        let reps: Vec<usize> = crate::lines::gather(model, &placement.layout, rules).0.iter().map(|l| l.rep).collect();
        for lane in &mut placement.lanes {
            lane.tracks.retain(|t| reps.contains(&t.0));
        }
        placement.lanes.retain(|l| !l.tracks.is_empty());
        for pl in placement.lines.iter_mut().filter(|pl| !reps.contains(&pl.edge)) {
            pl.tracks.clear();
        }
    }
    let problems = check::check(model, &placement);
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
    use super::layered::tests::{API_CASES, CHOICES};
    use super::*;

    /// With `ARCHWIRE_PROTO_OUT=<dir>`, writes each fixed API example laid out with lines running
    /// down as `<dir>/<example>-down.drawio`, for the diagram checker. Without it, does nothing.
    #[test]
    fn prototype_samples() {
        let Some(dir) = std::env::var_os("ARCHWIRE_PROTO_OUT") else { return };
        // A relative folder is taken from the workspace root, where the command is typed.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Rules::builtin();
        for (name, src) in API_CASES {
            let (m, d) = crate::flow_infer::infer(&crate::def::parse(src).unwrap(), &r);
            let (x, notes) = crate::arranged_drawio(&m.unwrap(), &d, &r, Axis::Down, CHOICES).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(notes.iter().all(|n| !n.contains("edge-")), "{name}: {notes:?}");
            std::fs::write(dir.join(format!("{name}-down.drawio")), x).unwrap();
        }
    }
}
