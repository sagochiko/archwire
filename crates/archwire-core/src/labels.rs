//! Placing a line's label along the line.

use crate::layout::Rect;
use crate::measure::{wire_label_area, Measure};
use crate::model::Axis;
use crate::path::{alongside, Pt};
use crate::route::Ctx;

/// Spots tried for a moved label are this far apart along the line; the spots in between are tried after them.
const LABEL_STEP: i32 = 2;
/// A line's label keeps at least this far from frame sides, icons and other text; a moved label
/// looks for a spot with this much room first.
const LABEL_MARGIN: i32 = 1;
const LABEL_ROOM: i32 = 4;

/// One coordinate of a point.
type Coord = fn(Pt) -> i32;

/// The point `d` along a path from its start, and whether the segment it lies on is horizontal.
pub(crate) fn point_along(path: &[Pt], mut d: i32) -> (Pt, bool) {
    for s in path.windows(2) {
        let len = (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs();
        if d <= len && len > 0 {
            let p = (s[0].0 + (s[1].0 - s[0].0).signum() * d, s[0].1 + (s[1].1 - s[0].1).signum() * d);
            return (p, s[0].1 == s[1].1);
        }
        d -= len;
    }
    let n = path.len();
    (path[n - 1], path[n - 2].1 == path[n - 1].1)
}

/// How far along a line of length `len` draw.io puts a label written at `at` thousandths.
pub(crate) fn label_distance(at: u16, len: i32) -> i32 {
    ((at as i64 * len as i64 + 500) / 1000) as i32
}

/// True if `d` along the path lies within a few pixels of a bend.
fn near_bend(path: &[Pt], d: i32) -> bool {
    let mut at = 0;
    for s in path.windows(2).take(path.len().saturating_sub(2)) {
        at += (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs();
        if (d - at).abs() < 3 {
            return true;
        }
    }
    false
}

pub(crate) fn path_length(path: &[Pt]) -> i32 {
    path.windows(2).map(|s| (s[1].0 - s[0].0).abs() + (s[1].1 - s[0].1).abs()).sum()
}

/// Where a line's label goes and how it reads.
pub(crate) struct LabelSpot {
    pub(crate) text: String,
    pub(crate) above: bool,
    pub(crate) flip: bool,
    pub(crate) at: Option<u16>,
    pub(crate) area: Option<Rect>,
    /// False when no spot was clear and the label stays at its usual spot regardless.
    pub(crate) clear: bool,
}

impl<'a> Ctx<'a> {
    /// The label at its usual spot (the middle, or a quarter along for `near_source`) if nothing
    /// cuts or covers it there. Otherwise the clear spot nearest the usual one along the line, on
    /// one line if that fits anywhere, else one part per line; spots with room around them come
    /// before tight ones, and spots on a 2-pixel grid before those between. A horizontal stretch
    /// carries the label centred above it, or below; a vertical one beside it on the right, or on
    /// the left. If no spot is clear, frame sides may run under the label: the spot clear of lines,
    /// icons and text with the fewest frame sides under it, in the same order, on one line if that
    /// fits anywhere, else one part per line. If there is none either, the usual spot stays.
    /// `level` puts the label at the usual spot above the line instead of beside it; `axis` is the
    /// way the line's arrangement runs, which picks the stretch `label_where_it_leaves` looks at.
    pub(crate) fn place_label(&self, parts: &[String], (paths, me): (&[Vec<Pt>], usize), level: bool, near_source: bool, axis: Axis, placed: &[Rect]) -> LabelSpot {
        let path = &paths[me];
        let one = parts.join(" ");
        if one.is_empty() {
            return LabelSpot { text: one, above: level, flip: false, at: None, area: None, clear: true };
        }
        let len = path_length(path);
        let usual = if near_source { len / 4 } else { len / 2 };
        if let Some(spot) = self.label_where_it_leaves(&one, (paths, me), axis, placed) {
            return spot;
        }
        let area = wire_label_area(&one, point_along(path, usual).0, level, false);
        if self.scene.label_clear(area, self.measure.label_margin(LABEL_MARGIN), paths, me, placed) {
            return LabelSpot { text: one, above: level, flip: false, at: None, area: Some(area), clear: true };
        }
        let mut texts = vec![one.clone()];
        if parts.len() > 1 {
            texts.push(parts.join("\n"));
        }
        // The spots between the steps come last, for a label that fits only there.
        let mut tries = Vec::new();
        for between in [0, 1] {
            for margin in [LABEL_ROOM, LABEL_MARGIN] {
                let margin = self.measure.label_margin(margin);
                tries.extend(texts.iter().map(|t| (between, margin, t)));
            }
        }
        let mut spots = Vec::new();
        for &(between, margin, text) in &tries {
            for (at, p, horizontal) in self.label_spots(path, usual, between) {
                for flip in [false, true] {
                    spots.push((margin, text, at, horizontal, flip, wire_label_area(text, p, horizontal, flip)));
                }
            }
        }
        let spot = |&(_, text, at, horizontal, flip, area): &(i32, &String, u16, bool, bool, Rect)| LabelSpot {
            text: text.clone(),
            above: horizontal,
            flip,
            at: Some(at),
            area: Some(area),
            clear: true,
        };
        if let Some(s) = spots.iter().find(|s| self.scene.label_clear(s.5, s.0, paths, me, placed)) {
            return spot(s);
        }
        // Over frame sides: the fewest of them, the first in the order above at a tie; one line
        // before one part per line, since a label that reads on one line is the easier to read.
        for text in &texts {
            let best = spots
                .iter()
                .filter(|s| s.1 == text && self.scene.label_clear_but_frames(s.5, s.0, paths, me, placed))
                .min_by_key(|s| self.scene.borders_under(s.5, s.0));
            if let Some(s) = best {
                return spot(s);
            }
        }
        LabelSpot { text: one, above: level, flip: false, at: None, area: Some(area), clear: false }
    }

    /// In the layout without a VPC, a bent line's label goes first on one stretch, when no other
    /// line runs along it: there it reads as this line's alone, where a label beside a channel
    /// shared by bends can read as a neighbour's. Running right, the stretch the line leaves its
    /// source by, when that is level, with the label centred above it, or below; running down,
    /// the first upright stretch, with the label beside it on the right, or on the left. Within
    /// the stretch and on one line; None if no spot there is clear.
    fn label_where_it_leaves(&self, text: &str, (paths, me): (&[Vec<Pt>], usize), axis: Axis, placed: &[Rect]) -> Option<LabelSpot> {
        let path = &paths[me];
        if self.measure != Measure::Flow || path.len() < 3 {
            return None;
        }
        let k = match axis {
            Axis::Right => (path[0].1 == path[1].1).then_some(0)?,
            Axis::Down => path.windows(2).position(|s| s[0].0 == s[1].0 && s[0].1 != s[1].1)?,
        };
        let horizontal = axis == Axis::Right;
        let (a, b) = (path[k], path[k + 1]);
        let alone = paths.iter().enumerate().all(|(i, q)| i == me || q.windows(2).all(|s| alongside(a, b, s[0], s[1]) == 0));
        if !alone {
            return None;
        }
        let before = path_length(&path[..=k]);
        let (along, across): (Coord, Coord) = if horizontal { (|p: Pt| p.0, |p: Pt| p.1) } else { (|p: Pt| p.1, |p: Pt| p.0) };
        let first = (along(b) - along(a)).abs();
        let on_first = |p: Pt| {
            across(p) == across(a)
                && along(p) != along(a)
                && along(p) != along(b)
                && (along(p) - along(a)).signum() == (along(b) - along(a)).signum()
                && (along(p) - along(a)).abs() < first
        };
        for margin in [LABEL_ROOM, LABEL_MARGIN] {
            let margin = self.measure.label_margin(margin);
            for (at, p, on_level) in self.label_spots(path, before + first / 2, 0) {
                if on_level != horizontal || !on_first(p) {
                    continue;
                }
                for flip in [false, true] {
                    let area = wire_label_area(text, p, horizontal, flip);
                    // The label stays within the stretch, so it does not reach past the bend.
                    let within = if horizontal {
                        area.x >= a.0.min(b.0) && area.x + area.w <= a.0.max(b.0)
                    } else {
                        area.y >= a.1.min(b.1) && area.y + area.h <= a.1.max(b.1)
                    };
                    if within && self.scene.label_clear(area, margin, paths, me, placed) {
                        return Some(LabelSpot { text: text.to_string(), above: horizontal, flip, at: Some(at), area: Some(area), clear: true });
                    }
                }
            }
        }
        None
    }

    /// The spots along a path a label may take, nearest `usual` first (`between` shifts them off
    /// the 2-pixel grid), as thousandths of the length, the point and whether the stretch is
    /// horizontal.
    fn label_spots(&self, path: &[Pt], usual: i32, between: i32) -> Vec<(u16, Pt, bool)> {
        let len = path_length(path);
        let mut out = Vec::new();
        for k in 0..=len / LABEL_STEP {
            for d in [usual + k * LABEL_STEP + between, usual - k * LABEL_STEP - between] {
                if d < 0 || d > len {
                    continue;
                }
                // The spot draw.io draws: it rounds the position written in thousandths of the length.
                let at = ((d as i64 * 1000 + len as i64 / 2) / len as i64) as u16;
                let d = label_distance(at, len);
                // Keep off the bends, where a label just past the bend would turn with the line.
                if near_bend(path, d) {
                    continue;
                }
                let (p, horizontal) = point_along(path, d);
                out.push((at, p, horizontal));
            }
        }
        out
    }
}
