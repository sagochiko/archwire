//! How wide text is drawn, and the areas icon names, frame names and line labels take.

use crate::layout::{Frame, FrameKind, Icon, Rect};
use crate::path::Pt;
use crate::route::LABEL_LINE;

/// An icon's label area starts this far below the icon and is this much taller than its lines.
const LABEL_GAP: i32 = 4;
const LABEL_PAD: i32 = 6;
/// Text width per character at font size 12.
const CHAR_ASCII: i32 = 7;
const CHAR_WIDE: i32 = 12;
/// Text width of a full-width character, and of a full-width parenthesis, at font size 11.
const WIRE_CHAR_WIDE: i32 = 11;
const WIRE_CHAR_PAREN: i32 = 6;
/// Frame names sit at the frame's top-left corner, after the group icon or a small indent.
const FRAME_NAME_INDENT: i32 = 30;
const FRAME_NAME_H: i32 = 25;
const AZ_NAME_INDENT: i32 = 10;
const AZ_NAME_H: i32 = 20;

/// A line's label: the height of each text line, and how far beside the line its area starts.
const WIRE_LABEL_LINE: i32 = 13;
const WIRE_LABEL_BESIDE: i32 = 6;

/// The least room between a line and another line's label in the layout without a VPC.
const FLOW_LABEL_ROOM: i32 = 3;

fn text_width(text: &str) -> i32 {
    text.lines()
        .map(|l| l.chars().map(|c| if c.is_ascii() { CHAR_ASCII } else { CHAR_WIDE }).sum::<i32>())
        .max()
        .unwrap_or(0)
}

/// Width of a Latin character in a line's label at font size 11, a pixel or so over what draw.io
/// draws: wide capitals and m, w; other capitals; most small letters, digits and symbols; the
/// narrower small letters; and the narrow letters, punctuation and space. A character not listed
/// counts at the widest width, so it is never counted narrower than it is drawn.
fn wire_latin_width(c: char) -> i32 {
    match c {
        '@' => 12,
        'W' => 11,
        'M' | 'm' | '%' => 10,
        'G' | 'O' | 'Q' | 'w' => 9,
        'A'..='Z' | '&' if !matches!(c, 'I' | 'J') => 8,
        'a' | 'b' | 'd' | 'e' | 'g' | 'h' | 'n' | 'o' | 'p' | 'q' | 'u' | '0'..='9' => 7,
        '#' | '$' | '?' | '_' | '+' | '=' | '<' | '>' | '~' | '^' | '"' | '*' | '`' | '{' | '}' | '\\' => 7,
        'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' | 'J' => 6,
        'f' | 'i' | 'j' | 'l' | 'r' | 't' | 'I' | '(' | ')' | '[' | ']' | '/' | '.' | ',' | ':' | ';' | '-' | '\'' | '!' | '|' | ' ' => 4,
        _ => 12,
    }
}

/// Text width of a line's label, drawn at font size 11: Latin text letter by letter, other text at
/// the width of a full-width character. A full-width parenthesis is drawn at full width inside a
/// line; an opening one at the start of a line and a closing one at its end show only half, and
/// count at half plus a pixel.
pub(crate) fn wire_text_width(text: &str) -> i32 {
    let line_width = |l: &str| {
        let n = l.chars().count();
        l.chars()
            .enumerate()
            .map(|(i, c)| match c {
                c if c.is_ascii() => wire_latin_width(c),
                '（' if i == 0 => WIRE_CHAR_PAREN,
                '）' if i + 1 == n => WIRE_CHAR_PAREN,
                _ => WIRE_CHAR_WIDE,
            })
            .sum::<i32>()
    };
    text.lines().map(line_width).max().unwrap_or(0)
}

/// The area an icon's label takes: centred under the icon.
pub(crate) fn label_area(i: &Icon) -> Rect {
    let half = (text_width(&i.label) + 1) / 2;
    Rect {
        x: i.rect.x + i.rect.w / 2 - half,
        y: i.rect.y + i.rect.h + LABEL_GAP,
        w: 2 * half,
        h: LABEL_LINE * i.lines as i32 + LABEL_PAD,
    }
}

/// Width of a Latin character in an icon's or a frame's name at font size 12, rounded up from
/// what draw.io draws, so a name is never counted narrower than it is drawn. A character not
/// listed counts at the widest width.
fn name_latin_width(c: char) -> i32 {
    match c {
        '@' => 13,
        'W' => 12,
        '%' => 11,
        'm' | 'M' | 'G' | 'O' | 'Q' => 10,
        'w' | 'C' | 'D' | 'H' | 'N' | 'R' | 'U' => 9,
        'A' | 'B' | 'E' | 'F' | 'K' | 'P' | 'S' | 'T' | 'V' | 'X' | 'Y' | 'Z' | '&' => 8,
        'a' | 'b' | 'd' | 'e' | 'g' | 'h' | 'n' | 'o' | 'p' | 'q' | 'u' | 'L' | '0'..='9' => 7,
        '#' | '$' | '?' | '_' | '+' | '<' | '=' | '>' | '~' => 7,
        'c' | 'k' | 's' | 'v' | 'x' | 'y' | 'z' | 'J' | '^' => 6,
        '*' | '"' | '{' | '}' => 5,
        'r' | 't' | 'f' | 'I' | '(' | ')' | '[' | ']' | '-' | '`' | '\\' | '/' | '!' | ',' | '.' | ':' | ';' | '|' | ' ' => 4,
        'i' | 'j' | 'l' | '\'' => 3,
        _ => 13,
    }
}

/// Text width of an icon's or a frame's name in the layout without a VPC: Latin text letter by
/// letter, other text at the width of a full-width character.
pub(crate) fn name_text_width(text: &str) -> i32 {
    text.lines()
        .map(|l| l.chars().map(|c| if c.is_ascii() { name_latin_width(c) } else { CHAR_WIDE }).sum::<i32>())
        .max()
        .unwrap_or(0)
}

/// The area an icon's name takes in the layout without a VPC: like `label_area`, measured with `name_text_width`.
pub(crate) fn name_area(i: &Icon) -> Rect {
    let half = (name_text_width(&i.label) + 1) / 2;
    Rect { w: 2 * half, x: i.rect.x + i.rect.w / 2 - half, ..label_area(i) }
}

/// The area a frame's name takes at its top-left corner in the layout without a VPC.
pub(crate) fn frame_name_box(f: &Frame) -> Rect {
    Rect { w: FRAME_NAME_INDENT + name_text_width(&f.label), ..frame_name_area(f) }
}

/// Ten times the width of each printable Latin character and of some full-width ones, read from
/// draw.io's rendering at font size 12: the larger of ten in a row and ten more between two
/// others. Spaces in a row collapse into one when drawn, so a space is read as ten single spaces
/// between eleven letters, less the eleven letters alone.
#[cfg(test)]
pub(crate) const NAME_CHARS: &str = r##"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~ "##;
#[cfg(test)]
pub(crate) const NAME_DRAWN: [i32; 95] = [
    67, 59, 67, 67, 67, 67, 67, 67, 67, 67, 67, 67, 60, 67, 67, 32, 67, 67, 26, 27, 60, 27, 100, 67, 67, 67, 67, 40, 60, 34, 67, 60,
    87, 60, 60, 60, 80, 80, 87, 87, 80, 74, 94, 87, 34, 60, 80, 67, 100, 87, 94, 80, 94, 87, 80, 74, 87, 80, 114, 80, 80, 74, 34, 43,
    67, 67, 107, 80, 23, 40, 40, 47, 70, 34, 40, 34, 34, 34, 34, 70, 70, 70, 67, 122, 33, 35, 34, 57, 67, 38, 41, 31, 41, 70, 34,
];
#[cfg(test)]
pub(crate) const NAME_WIDE_CHARS: &str = "東京リージョン（）・ログメトクス読み書き呼び出し利用者静的ファイル全関数か所ら";
#[cfg(test)]
pub(crate) const NAME_WIDE_DRAWN: [i32; 39] = [
    120, 120, 120, 120, 120, 120, 120, 66, 66, 120, 120, 112, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120, 120,
    120, 120, 120, 120, 120, 111, 116, 120, 120, 120, 120, 120, 120,
];

/// How names are measured: the VPC layout keeps its own measure; the layout without a VPC uses
/// `name_area` and `frame_name_box`, never narrower than draw.io draws a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Measure {
    Grid,
    Flow,
}

impl Measure {
    pub(crate) fn icon_name(self, i: &Icon) -> Rect {
        match self {
            Measure::Grid => label_area(i),
            Measure::Flow => name_area(i),
        }
    }
    pub(crate) fn frame_name(self, f: &Frame) -> Rect {
        match self {
            Measure::Grid => frame_name_area(f),
            Measure::Flow => frame_name_box(f),
        }
    }
    /// The least room kept between a line's label and anything else. In the layout without a VPC
    /// a line may not run closer than `FLOW_LABEL_ROOM` to another line's label: draw.io draws the
    /// tops of wide letters a pixel or two from the box `wire_label_area` gives, so a line along
    /// the box's edge touches the text.
    pub(crate) fn label_margin(self, margin: i32) -> i32 {
        match self {
            Measure::Grid => margin,
            Measure::Flow => margin.max(FLOW_LABEL_ROOM),
        }
    }
}

/// The area a line's label takes when anchored at `p`, as draw.io draws it at font size 11:
/// centred above the anchor if `above` (below it if `flip`), else beside it on the right (on the
/// left if `flip`) with its last line ending at the anchor. Measured from rendered diagrams, with
/// a pixel or two to spare.
pub(crate) fn wire_label_area(text: &str, p: Pt, above: bool, flip: bool) -> Rect {
    let w = wire_text_width(text);
    let lines = WIRE_LABEL_LINE * text.lines().count().max(1) as i32;
    match (above, flip) {
        (true, false) => Rect { x: p.0 - w / 2 - 2, y: p.1 - lines - 4, w: w + 4, h: lines + 4 },
        (true, true) => Rect { x: p.0 - w / 2 - 2, y: p.1, w: w + 4, h: lines + 12 },
        (false, false) => Rect { x: p.0 + WIRE_LABEL_BESIDE, y: p.1 - lines - 1, w: w + WIRE_LABEL_BESIDE, h: lines + 3 },
        (false, true) => Rect { x: p.0 - 2 * WIRE_LABEL_BESIDE - w, y: p.1 - lines - 1, w: w + WIRE_LABEL_BESIDE, h: lines + 3 },
    }
}

/// The area a frame's name takes at its top-left corner.
pub(crate) fn frame_name_area(f: &Frame) -> Rect {
    let (indent, h) = if f.kind == FrameKind::Az { (AZ_NAME_INDENT, AZ_NAME_H) } else { (FRAME_NAME_INDENT, FRAME_NAME_H) };
    Rect { x: f.rect.x, y: f.rect.y, w: indent + text_width(&f.label), h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_character_is_estimated_no_narrower_than_draw_io_draws_it_at_twelve() {
        assert_eq!(NAME_CHARS.chars().count(), NAME_DRAWN.len());
        assert_eq!(NAME_WIDE_CHARS.chars().count(), NAME_WIDE_DRAWN.len());
        let all = NAME_CHARS.chars().zip(NAME_DRAWN).chain(NAME_WIDE_CHARS.chars().zip(NAME_WIDE_DRAWN));
        for (c, d) in all {
            let w = name_text_width(&c.to_string().repeat(10));
            assert!(w >= d, "{c:?}: estimated {w} for ten, drawn {d}");
            // Rounded up a character at a time; a digit counts at the one width every digit is set in.
            assert!(w <= d + 12 || !c.is_ascii(), "{c:?}: estimated {w} for ten, drawn {d}");
        }
        // Characters not measured count at the widest width, never narrower.
        assert!(name_text_width("\t") >= name_text_width("@"));
        // The VPC layout's measure is left as it was.
        assert_eq!(text_width("mmmm"), 28);
    }
}
