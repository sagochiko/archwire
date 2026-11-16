//! Layout for definitions without a VPC part, from left to right: the layered layout with main
//! lines running right.

use crate::arrange::arrange;
use crate::diag::Problem;
use crate::model::{Axis, Model};
use crate::rules::Rules;

pub use crate::arrange::Placement;

/// The layout, or LAYOUT_FAILED problems when a must-hold condition fails after laying out.
pub fn place(model: &Model, rules: &Rules) -> Result<Placement, Vec<Problem>> {
    arrange(model, rules, Axis::Right)
}
