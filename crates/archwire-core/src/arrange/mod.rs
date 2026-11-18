//! Layout from the membership tree: the parts outside every frame of their own arrangement,
//! laid out in columns or rows along an axis, and frame items holding their own arrangement.
//!
//! The outside is one procedure written along a main axis `u` and a cross axis `v`; the axis
//! only decides at the end whether `(x, y)` is `(u, v)` or `(v, u)`. Names stay below their icons
//! and frame names at their frames' top-left corners whichever way the lines run.

mod azgrid;
mod check;
#[cfg(test)]
mod generated;
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
///
/// A part attached to a part spanning the zones (a certificate or a web ACL on a load balancer)
/// sits right of the VPC, and its line runs as a side line out of that part would: out of the
/// part's right side, or down to the band above the private tier when a part of its row stands
/// right of it, and out of the VPC's right side.
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

/// How wide or tall a drawing may be per part before a test takes it for one that kept growing.
/// Measured on 2026-11-18, the generated definitions with a VPC took at most 188 pixels across
/// and 144 down per part, and those without one at most 232 and 185.
#[cfg(test)]
const MOST_PER_PART: i32 = 300;

/// The size of a laid-out drawing, when it is wider or taller than `MOST_PER_PART` per part.
#[cfg(test)]
pub(crate) fn spread_too_far(layout: &Layout) -> Option<String> {
    let rects: Vec<crate::layout::Rect> = layout.icons.iter().map(|i| i.rect).chain(layout.frames.iter().map(|f| f.rect)).collect();
    let (x0, y0) = (rects.iter().map(|r| r.x).min()?, rects.iter().map(|r| r.y).min()?);
    let (x1, y1) = (rects.iter().map(|r| r.x + r.w).max()?, rects.iter().map(|r| r.y + r.h).max()?);
    let most = MOST_PER_PART * layout.icons.len() as i32;
    (x1 - x0 > most || y1 - y0 > most).then(|| format!("{} by {} pixels for {} parts, more than {most}", x1 - x0, y1 - y0, layout.icons.len()))
}

#[cfg(test)]
mod tests {
    use super::azgrid::tests::{grid, inputs};
    use crate::test_inputs::{generated_with_a_vpc, API_CASES, FEWER_READERS, WEB_CASES};
    use super::*;
    use crate::diag::Diagnostics;

    /// The diagrams written for viewing: (file name stem, input name). Each is written with lines
    /// running right and with lines running down.
    const SAMPLES: [(&str, &str); 25] = [
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
        ("mix-vpc-ja", "mix-vpc-ja"),
        ("mix-nat-ja", "mix-nat-ja"),
        ("mix-web-ja", "mix-web-ja"),
        ("mix-logs-ja", "mix-logs-ja"),
        ("web-named-ja", "web-named-ja"),
        ("mix-3az-ja", "mix-3az-ja"),
        ("fn-before-web-3az-ja", "fn-before-web-3az-ja"),
        ("mix-beside-ja", "mix-beside-ja"),
    ];

    /// A definition with one more line in its settings.
    fn with_setting(src: &str, line: &str) -> String {
        match src.find("set:\n") {
            Some(at) => format!("{}  {line}\n{}", &src[..at + 5], &src[at + 5..]),
            None => format!("{src}set:\n  {line}\n"),
        }
    }

    /// The sample definitions mixing serverless parts with VPC parts, and naming VPC parts, each
    /// written in Japanese: (input name, definition). They go through the inference `compile`
    /// uses, so each carries its real reasons and holes.
    fn vpc_samples() -> Vec<(&'static str, String)> {
        use crate::test_inputs::{MIX_LOGS, MIX_NAT, MIX_SMALLEST, MIX_WEB, TWIN_APPS};
        let named = super::generated::mixed_representatives();
        let before = &named.iter().find(|(n, _)| *n == "function-before-web-3az").expect("a named input").1;
        // One generated shape with a function outside the VPC beside it and a part right of it.
        let beside = super::generated::generated_mixed()[14].replace("  lang: en\n", "  lang: ja\n");
        let samples = vec![
            ("mix-vpc-ja", MIX_SMALLEST.to_string()),
            ("mix-nat-ja", MIX_NAT.to_string()),
            ("mix-web-ja", MIX_WEB.to_string()),
            ("mix-logs-ja", with_setting(MIX_LOGS, "lang: ja")),
            ("web-named-ja", with_setting(TWIN_APPS, "lang: ja")),
            ("mix-3az-ja", with_setting(MIX_SMALLEST, "azs: 3")),
            ("fn-before-web-3az-ja", with_setting(before, "lang: ja")),
            ("mix-beside-ja", beside),
        ];
        for (name, src) in &samples {
            assert_eq!(src.matches("lang: ja").count(), 1, "{name}\n{src}");
        }
        samples
    }

    // Fails if a model that writes no direction is drawn along any axis but down, with a VPC or without.
    #[test]
    fn models_run_down_unless_written_otherwise() {
        let r = Rules::builtin();
        let mut models: Vec<Model> = inputs().into_iter().map(|(_, m, _)| m).collect();
        for (_, src) in API_CASES {
            models.push(crate::inference::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap());
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

    /// Lays out every generated definition among `defs` along each of `axes` and draws its
    /// lines, without writing a diagram; returns, for each that fails, the definition, the axis
    /// and the problems or the notes on lines that broke.
    fn generated_failures(defs: &[String], axes: &[Axis]) -> Vec<String> {
        let r = Rules::builtin();
        let mut failed = Vec::new();
        for src in defs {
            for &axis in axes {
                let (m, d) = grid(src);
                if let Some(spread) = super::arrange(&m, &r, axis).ok().and_then(|p| super::spread_too_far(&p.layout)) {
                    failed.push(format!("{axis:?}: {spread}\n{src}"));
                }
                match crate::arranged_drawio(&m, &d, &r, axis) {
                    Ok((_, notes)) => {
                        let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                        if !broken.is_empty() {
                            failed.push(format!("{axis:?}: {broken:?}\n{src}"));
                        }
                    }
                    Err(e) => failed.push(format!("{axis:?}: {e:?}\n{src}")),
                }
            }
        }
        failed
    }

    // Fails if a generated definition with two zones fails to lay out, breaks a line, or is drawn
    // wider or taller than its parts can take up, along either axis.
    #[test]
    fn generated_two_zone_definitions_draw_cleanly() {
        let defs: Vec<String> = generated_with_a_vpc().into_iter().filter(|s| s.contains("  azs: 2\n")).collect();
        assert_eq!(defs.len(), TWO_ZONE_DEFINITIONS);
        let failed = generated_failures(&defs, &[Axis::Right, Axis::Down]);
        assert!(failed.is_empty(), "{} of {} diagrams failed:\n{}", failed.len(), 2 * defs.len(), failed.join("\n"));
    }

    /// How many generated definitions have two zones.
    const TWO_ZONE_DEFINITIONS: usize = 336;

    // Fails if any generated definition fails to lay out, breaks a line, or is drawn wider or
    // taller than its parts can take up, along either axis. It takes minutes, so it runs only when named:
    //     cargo test -p archwire-core generated_definitions_all_draw_cleanly -- --ignored
    #[test]
    #[ignore]
    fn generated_definitions_all_draw_cleanly() {
        let defs = generated_with_a_vpc();
        let failed = generated_failures(&defs, &[Axis::Right, Axis::Down]);
        assert!(failed.is_empty(), "{} of {} diagrams failed:\n{}", failed.len(), 2 * defs.len(), failed.join("\n"));
    }

    /// True for the stem of a file `generated_definitions_through_compile` writes: four digits, a
    /// dash and a direction.
    fn through_compile_stem(stem: &str) -> bool {
        let Some((number, way)) = stem.split_once('-') else { return false };
        number.len() == 4 && number.bytes().all(|b| b.is_ascii_digit()) && ["down", "right"].contains(&way)
    }

    // With the folder given, fails if a generated definition with a VPC gives no diagram or an
    // error along either direction, or cannot be written there.
    /// With `ARCHWIRE_VPC_OUT=<dir>`, writes every generated definition with a VPC, with each
    /// direction written (`down` and `right`), as `compile` draws it: `<dir>/<number>-<way>.yaml`
    /// (the definition with its direction written), `<dir>/<number>-<way>.drawio` when a diagram
    /// was drawn, and `<dir>/<number>-<way>.json` (the diagnostics). The number has four digits,
    /// in the order of `generated_with_a_vpc`. Files of that name already in the folder are
    /// removed first, so none is left from an earlier run. A relative folder is taken from the
    /// workspace root. Without it, does nothing. It writes thousands of files, so it runs only when named:
    ///     cargo test -p archwire-core generated_definitions_through_compile -- --ignored
    #[test]
    #[ignore]
    fn generated_definitions_through_compile() {
        let Some(dir) = std::env::var_os("ARCHWIRE_VPC_OUT") else { return };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if through_compile_stem(stem) && ["yaml", "drawio", "json"].contains(&ext) {
                std::fs::remove_file(&path).unwrap();
            }
        }
        let mut failed = Vec::new();
        for (k, src) in generated_with_a_vpc().iter().enumerate() {
            for way in ["down", "right"] {
                let name = format!("{k:04}-{way}");
                let s = with_direction(src, way);
                let out = crate::compile(&s);
                std::fs::write(dir.join(format!("{name}.yaml")), &s).unwrap();
                if let Some(x) = &out.drawio {
                    std::fs::write(dir.join(format!("{name}.drawio")), x).unwrap();
                }
                std::fs::write(dir.join(format!("{name}.json")), out.diagnostics.to_json()).unwrap();
                if out.drawio.is_none() || !out.diagnostics.errors.is_empty() {
                    failed.push(format!("{name}: {:?}", out.diagnostics.errors));
                }
            }
        }
        assert!(failed.is_empty(), "{} of {} gave no diagram or an error:\n{}", failed.len(), 2 * generated_with_a_vpc().len(), failed.join("\n"));
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
                let (m, d) = crate::inference::infer(&crate::def::parse(&src).unwrap(), &r);
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
    /// writes each input drawn along both axes (a definition by `compile`, a model built by hand by the same layout) as `<dir>/v<number>-<axis>.drawio`,
    /// `<dir>/v<number>-<axis>.yaml`: the definition with its direction written, or one comment
    /// line naming a model built by hand, and `<dir>/v<number>-<axis>.json`: the diagnostics of
    /// `compile`, or, for a model built by hand, empty diagnostics with the router's notes.
    /// Without it, only draws them.
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
                if ours(stem) && ["yaml", "drawio", "json"].contains(&ext) {
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
                            Some(x) if out.diagnostics.errors.is_empty() => Ok((x, out.diagnostics)),
                            _ => Err(out.diagnostics.errors),
                        };
                        (drawn, s)
                    }
                    Err(hand) => {
                        let drawn = crate::arranged_drawio(model, &Diagnostics::default(), &r, axis)
                            .map(|(x, notes)| (x, Diagnostics { notes, ..Default::default() }));
                        (drawn, format!("# hand-built model: {hand}\n"))
                    }
                };
                match drawn {
                    Ok((x, diag)) => {
                        let notes = &diag.notes;
                        let broken: Vec<&String> = notes.iter().filter(|n| n.contains("edge-")).collect();
                        if !broken.is_empty() {
                            failed.push(format!("{name}: {broken:?}\n{yaml}"));
                        }
                        if let Some(d) = &dir {
                            std::fs::write(d.join(format!("{name}.yaml")), &yaml).unwrap();
                            std::fs::write(d.join(format!("{name}.drawio")), x).unwrap();
                            std::fs::write(d.join(format!("{name}.json")), diag.to_json()).unwrap();
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
    /// `<dir>/<name>-down.drawio`, for the diagram checker, `<dir>/<name>.yaml` with the definition
    /// of each sample taken from a definition, and `<dir>/notes.tsv` with the router's notes of
    /// each diagram. Without it, does nothing.
    #[test]
    fn sample_diagrams() {
        let Some(dir) = std::env::var_os("ARCHWIRE_SAMPLE_OUT") else { return };
        // A relative folder is taken from the workspace root, where the command is typed.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Rules::builtin();
        let mut models: Vec<(String, Model, Diagnostics)> = inputs();
        for (name, src) in API_CASES {
            let (m, d) = crate::inference::infer(&crate::def::parse(src).unwrap(), &r);
            models.push((name.to_string(), m.unwrap_or_else(|| panic!("{name}: {:?}", d.errors)), d));
        }
        for (name, src) in vpc_samples() {
            let (m, d) = crate::inference::infer(&crate::def::parse(&src).unwrap(), &r);
            assert!(d.errors.is_empty(), "{name}: {:?}\n{src}", d.errors);
            models.push((name.to_string(), m.unwrap_or_else(|| panic!("{name}\n{src}")), d));
            std::fs::write(dir.join(format!("{name}.yaml")), &src).unwrap();
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


    const WEB2: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n";

    fn laid(src: &str, axis: Axis) -> Layout {
        let r = Rules::builtin();
        let m = crate::inference::infer(&crate::def::parse(src).unwrap(), &r).0.unwrap();
        arrange(&m, &r, axis).unwrap_or_else(|e| panic!("{axis:?}: {e:?}\n{src}")).layout
    }

    // Fails if a frame loses its name or parent, or a part its name, its line count or its frame.
    #[test]
    fn frames_labels_and_parents_follow_the_skill_diagram() {
        for axis in [Axis::Down, Axis::Right] {
            let l = laid(&format!("{WEB2}  - waf -- cloudfront\n  - acm -- cloudfront\n"), axis);
            let frames: Vec<_> = l.frames.iter().map(|f| (f.id.as_str(), f.label.as_str(), f.parent.as_deref())).collect();
            assert_eq!(frames, [
                ("aws-cloud", "AWS Cloud", None), ("region", "Tokyo (ap-northeast-1)", Some("aws-cloud")), ("vpc", "VPC", Some("region")),
                ("az-1a", "ap-northeast-1a", Some("vpc")), ("pub-1a", "Public subnet", Some("az-1a")), ("pri-1a", "Private subnet", Some("az-1a")),
                ("az-1c", "ap-northeast-1c", Some("vpc")), ("pub-1c", "Public subnet", Some("az-1c")), ("pri-1c", "Private subnet", Some("az-1c")),
            ], "{axis:?}");
            let icon = |id: &str| l.icons.iter().find(|i| i.id == id).unwrap();
            assert_eq!((icon("aurora-1a").label.as_str(), icon("aurora-1a").lines), ("Amazon Aurora\nWriter", 2));
            assert_eq!((icon("ecs-1c").label.as_str(), icon("ecs-1c").lines), ("Amazon ECS (Fargate)", 1));
            assert_eq!(icon("user").label, "Users");
            assert_eq!((icon("acm").label.as_str(), icon("acm").lines), ("AWS Certificate Manager\nus-east-1", 2));
            assert_eq!((icon("waf").label.as_str(), icon("waf").lines), ("AWS WAF", 1));
            let parents: Vec<_> = l.icons.iter().map(|i| (i.id.as_str(), i.parent.as_deref())).collect();
            assert_eq!(parents, [
                ("user", None), ("cloudfront", Some("aws-cloud")), ("waf", Some("aws-cloud")), ("acm", Some("aws-cloud")),
                ("igw", Some("vpc")), ("alb", Some("vpc")), ("nat-1a", Some("pub-1a")), ("nat-1c", Some("pub-1c")),
                ("ecs-1a", Some("pri-1a")), ("ecs-1c", Some("pri-1c")), ("aurora-1a", Some("pri-1a")), ("aurora-1c", Some("pri-1c")),
                ("s3", Some("region")), ("cloudwatch", Some("region")),
            ], "{axis:?}");
            let ja = laid(&format!("{WEB2}set:\n  lang: ja\n"), axis);
            assert_eq!(ja.frames[1].label, "東京リージョン（ap-northeast-1）");
            assert_eq!(ja.frames[4].label, "パブリックサブネット");
        }
    }

    // Fails if a part outside the VPC moves the VPC or anything in it.
    #[test]
    fn adding_a_side_service_does_not_move_the_vpc_contents() {
        for axis in [Axis::Down, Axis::Right] {
            let (a, b) = (laid("flows:\n  - user -> alb -> ecs -> aurora\n", axis), laid("flows:\n  - user -> alb -> ecs -> aurora\n  - ecs ~> cloudwatch\n", axis));
            for id in ["alb", "igw", "ecs-1a", "ecs-1c", "aurora-1a", "aurora-1c", "vpc", "az-1c"] {
                assert_eq!(a.rect(id), b.rect(id), "{id} {axis:?}");
            }
        }
    }
}
