//! Writer: turns the laid-out, routed diagram into draw.io XML.
//! The output depends only on its inputs: elements and attributes come in a fixed order,
//! coordinates are integers and lines end with LF.

use crate::diag::{Derived, DerivedKind, Diagnostics};
use crate::layout::{FrameKind, Layout, Rect};
use crate::model::Model;
use crate::route::{Port, Side, Wire, WireStyle};
use crate::rules::{IconKind, Rules};

const CLOUD_STYLE: &str = "html=1;whiteSpace=wrap;fontSize=12;shape=mxgraph.aws4.group;grIcon=mxgraph.aws4.group_aws_cloud_alt;strokeColor=#000000;fillColor=none;verticalAlign=top;align=left;spacingLeft=30;fontColor=#000000;container=1;collapsible=0;recursiveResize=0";
const REGION_STYLE: &str = "html=1;whiteSpace=wrap;fontSize=12;shape=mxgraph.aws4.group;grIcon=mxgraph.aws4.group_region;strokeColor=#00A4A6;fillColor=none;verticalAlign=top;align=left;spacingLeft=30;fontColor=#000000;dashed=1;container=1;collapsible=0;recursiveResize=0";
const VPC_STYLE: &str = "html=1;whiteSpace=wrap;fontSize=12;shape=mxgraph.aws4.group;grIcon=mxgraph.aws4.group_vpc2;strokeColor=#8C4FFF;fillColor=none;verticalAlign=top;align=left;spacingLeft=30;fontColor=#000000;container=1;collapsible=0;recursiveResize=0";
const AZ_STYLE: &str = "fillColor=none;strokeColor=#00A4A6;dashed=1;verticalAlign=top;align=left;spacingLeft=10;fontSize=12;fontColor=#000000;whiteSpace=wrap;html=1;container=1;collapsible=0;recursiveResize=0";
const PUBLIC_SUBNET_STYLE: &str = "html=1;whiteSpace=wrap;fontSize=12;shape=mxgraph.aws4.group;grIcon=mxgraph.aws4.group_security_group;grStroke=0;strokeColor=#7AA116;fillColor=#F2F6E8;verticalAlign=top;align=left;spacingLeft=30;fontColor=#248814;container=1;collapsible=0;recursiveResize=0";
const PRIVATE_SUBNET_STYLE: &str = "html=1;whiteSpace=wrap;fontSize=12;shape=mxgraph.aws4.group;grIcon=mxgraph.aws4.group_security_group;grStroke=0;strokeColor=#00A4A6;fillColor=#E6F6F7;verticalAlign=top;align=left;spacingLeft=30;fontColor=#147EBA;container=1;collapsible=0;recursiveResize=0";

const MAIN_EDGE_STYLE: &str = "edgeStyle=orthogonalEdgeStyle;html=1;endArrow=block;endFill=1;strokeColor=#545B64;rounded=0;labelBackgroundColor=none;verticalAlign=bottom;align=left;spacingLeft=6";
const AUX_EDGE_STYLE: &str = "edgeStyle=orthogonalEdgeStyle;html=1;endArrow=block;endFill=1;strokeColor=#E7157B;rounded=0;dashed=1;labelBackgroundColor=none;verticalAlign=bottom;align=left;spacingLeft=6";
const AUX_COLOR: &str = "strokeColor=#E7157B";
const ARROW: &str = "endArrow=block;endFill=1";
/// An association line has no arrowhead.
const NO_ARROW: &str = "endArrow=none";
/// A line whose route is still a hole is drawn in the auxiliary style, in grey.
const HOLE_COLOR: &str = "strokeColor=#879196";
const LABEL_BESIDE: &str = "align=left;spacingLeft=6";
const LABEL_ABOVE: &str = "align=center";
const LABEL_LEFT: &str = "align=right;spacingRight=6";
const LABEL_BOTTOM: &str = "verticalAlign=bottom";
const LABEL_TOP: &str = "verticalAlign=top";

/// Association lines (`--`) end without an arrowhead.
pub fn write(model: &Model, layout: &Layout, wires: &[Wire], diag: &Diagnostics, rules: &Rules) -> String {
    let origin = |parent: &Option<String>| parent.as_deref().map_or((0, 0), |p| {
        let r = layout.rect(p);
        (r.x, r.y)
    });
    let mut out = format!(
        "<mxfile archwire=\"{}\" rules=\"{}\"><diagram id=\"p1\" name=\"Architecture\"><mxGraphModel adaptiveColors=\"auto\"><root>\n",
        escape(&diag.versions.archwire),
        escape(&rules.version)
    );
    out.push_str("<mxCell id=\"0\" />\n<mxCell id=\"1\" parent=\"0\" />\n");

    for f in &layout.frames {
        let (style, target) = match f.kind {
            FrameKind::Cloud => (CLOUD_STYLE, Some("aws-cloud")),
            FrameKind::Region => (REGION_STYLE, Some("region")),
            FrameKind::Vpc => (VPC_STYLE, Some("vpc")),
            FrameKind::Az => (AZ_STYLE, Some("az")),
            FrameKind::PublicSubnet => (PUBLIC_SUBNET_STYLE, Some("subnet")),
            FrameKind::PrivateSubnet => (PRIVATE_SUBNET_STYLE, Some("subnet")),
        };
        let reasons: Vec<&Derived> = diag.derived.iter().filter(|d| Some(d.target.as_str()) == target).collect();
        // Every frame is something archwire added: a definition names services only.
        vertex(&mut out, &f.id, &f.label, true, &reasons, style, &f.parent, f.rect, origin(&f.parent));
    }

    for i in &layout.icons {
        let node = model.nodes.iter().find(|n| n.id == i.id).unwrap_or_else(|| panic!("icon `{}` has no node", i.id));
        let s = rules.service(&node.service).unwrap_or_else(|| panic!("model has unknown service `{}`", node.service));
        let shape = node.option.as_ref().and_then(|o| s.options.get(o)).map_or(&s.shape, |o| &o.shape);
        let style = match s.icon {
            IconKind::Service => format!("fontColor=#16191F;fillColor={};strokeColor=#ffffff;verticalLabelPosition=bottom;verticalAlign=top;align=center;html=1;fontSize=12;aspect=fixed;shape=mxgraph.aws4.resourceIcon;resIcon=mxgraph.aws4.{shape}", s.color),
            IconKind::Resource => format!("fontColor=#16191F;fillColor={};strokeColor=none;verticalLabelPosition=bottom;verticalAlign=top;align=center;html=1;fontSize=12;aspect=fixed;shape=mxgraph.aws4.{shape}", s.color),
        };
        let reasons: Vec<&Derived> = diag.derived.iter().filter(|d| d.target == node.id || d.target == node.part).collect();
        vertex(&mut out, &i.id, &i.label, node.added, &reasons, &style, &i.parent, i.rect, origin(&i.parent));
    }

    for w in wires {
        let mut style = match w.style {
            WireStyle::Main => MAIN_EDGE_STYLE.to_string(),
            WireStyle::Aux => AUX_EDGE_STYLE.to_string(),
            WireStyle::Assoc => AUX_EDGE_STYLE.replace(ARROW, NO_ARROW),
            WireStyle::Hole => AUX_EDGE_STYLE.replace(AUX_COLOR, HOLE_COLOR),
        };
        style = match (w.horizontal_label, w.label_flip) {
            (true, false) => style.replace(LABEL_BESIDE, LABEL_ABOVE),
            (true, true) => style.replace(LABEL_BESIDE, LABEL_ABOVE).replace(LABEL_BOTTOM, LABEL_TOP),
            (false, false) => style,
            (false, true) => style.replace(LABEL_BESIDE, LABEL_LEFT),
        };
        style.push_str(&port("exit", w.exit));
        style.push_str(&port("entry", w.entry));
        let x = match (w.label_at, w.label_near_source) {
            (Some(at), _) => format!(" x=\"{}\"", relative(at)),
            (None, true) => " x=\"-0.5\"".to_string(),
            (None, false) => String::new(),
        };
        out.push_str(&format!(
            "<mxCell id=\"{}\" value=\"{}\" style=\"{}\" edge=\"1\" parent=\"1\" source=\"{}\" target=\"{}\">",
            escape(&w.id),
            escape(&w.label),
            escape(&style),
            escape(&w.from),
            escape(&w.to)
        ));
        if w.points.is_empty() {
            out.push_str(&format!("<mxGeometry{x} relative=\"1\" as=\"geometry\" />"));
        } else {
            out.push_str(&format!("<mxGeometry{x} relative=\"1\" as=\"geometry\"><Array as=\"points\">"));
            for (px, py) in &w.points {
                out.push_str(&format!("<mxPoint x=\"{px}\" y=\"{py}\" />"));
            }
            out.push_str("</Array></mxGeometry>");
        }
        out.push_str("</mxCell>\n");
    }

    out.push_str("</root></mxGraphModel></diagram></mxfile>\n");
    out
}

/// One frame or icon: an `object` carrying the label, `added="1"` if archwire added it, and the
/// reasons it was added or placed, around its cell.
#[allow(clippy::too_many_arguments)]
fn vertex(out: &mut String, id: &str, label: &str, added: bool, reasons: &[&Derived], style: &str, parent: &Option<String>, r: Rect, origin: (i32, i32)) {
    out.push_str(&format!("<object id=\"{}\" label=\"{}\"", escape(id), escape(label)));
    if added {
        out.push_str(" added=\"1\"");
    }
    if !reasons.is_empty() {
        let joined = |f: &dyn Fn(&Derived) -> String| reasons.iter().map(|d| f(d)).collect::<Vec<_>>().join(" | ");
        out.push_str(&format!(" derived=\"{}\"", escape(&joined(&|d| d.reason.clone()))));
        out.push_str(&format!(" rule=\"{}\"", escape(&joined(&|d| d.rule.clone()))));
        out.push_str(&format!(" kind=\"{}\"", joined(&|d| match d.kind {
            DerivedKind::Constraint => "constraint".into(),
            DerivedKind::Default => "default".into(),
        })));
        // A reason without a source keeps its place as an empty value, so the lists stay aligned.
        if reasons.iter().any(|d| d.source.is_some()) {
            out.push_str(&format!(" source=\"{}\"", escape(&joined(&|d| d.source.clone().unwrap_or_default()))));
        }
    }
    out.push_str(&format!(
        "><mxCell style=\"{}\" vertex=\"1\" parent=\"{}\"><mxGeometry x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" as=\"geometry\" /></mxCell></object>\n",
        escape(style),
        escape(parent.as_deref().unwrap_or("1")),
        r.x - origin.0,
        r.y - origin.1,
        r.w,
        r.h
    ));
}

/// `;exitX=..;exitY=..`, plus the outward shift and `exitPerimeter=0` when the port moves off the side.
fn port(prefix: &str, p: Port) -> String {
    let along = match p.pos {
        0 => "0".to_string(),
        100 => "1".to_string(),
        n if n % 10 == 0 => format!("0.{}", n / 10),
        n => format!("0.{n:02}"),
    };
    let (x, y, dx, dy) = match p.side {
        Side::Top => (along, "0".to_string(), 0, -p.offset),
        Side::Bottom => (along, "1".to_string(), 0, p.offset),
        Side::Left => ("0".to_string(), along, -p.offset, 0),
        Side::Right => ("1".to_string(), along, p.offset, 0),
    };
    let mut s = format!(";{prefix}X={x};{prefix}Y={y}");
    if p.offset != 0 {
        s.push_str(&format!(";{prefix}Dx={dx};{prefix}Dy={dy};{prefix}Perimeter=0"));
    }
    s
}

/// draw.io's relative label position, -1 at the source to 1 at the target, from thousandths of the length.
fn relative(at: u16) -> String {
    let v = 2 * at as i32 - 1000;
    let (sign, a) = (if v < 0 { "-" } else { "" }, v.abs());
    let frac = format!("{:03}", a % 1000);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() { format!("{sign}{}", a / 1000) } else { format!("{sign}{}.{frac}", a / 1000) }
}

/// Attribute value text: XML's special characters as entities, a newline as `&#xa;`.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#xa;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    const WEB2: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n";

    #[test]
    fn output_is_byte_identical_across_runs() {
        let (a, b) = (crate::compile(WEB2), crate::compile(WEB2));
        assert_eq!(a.drawio, b.drawio);
        assert_eq!(a.diagnostics.to_json(), b.diagnostics.to_json());
        let x = a.drawio.unwrap();
        assert!(x.ends_with(">\n") && !x.contains('\r') && !x.contains("<!--"));
    }

    #[test]
    fn derived_parts_carry_their_reason_in_the_file() {
        let x = crate::compile(WEB2).drawio.unwrap();
        let igw = x.split("<object ").find(|s| s.starts_with("id=\"igw\"")).unwrap();
        assert!(igw.contains("rule=\"internet-facing-alb-in-vpc-with-igw\"") && igw.contains("kind=\"constraint\"") && igw.contains("source=\"https://docs.aws.amazon.com/"));
        let user = x.split("<object ").find(|s| s.starts_with("id=\"user\"")).unwrap();
        assert!(!user.contains("rule="));
        assert!(x.starts_with("<mxfile archwire=\""));
        assert!(x.contains("rules=\""));
    }

    #[test]
    fn every_subnet_frame_carries_its_reason() {
        let out = crate::compile(WEB2);
        let x = out.drawio.unwrap();
        for id in ["pub-1a", "pub-1c", "pri-1a", "pri-1c"] {
            let cell = x.split("<object ").find(|s| s.starts_with(&format!("id=\"{id}\""))).unwrap();
            assert!(cell.contains("rule=\"public-and-private-subnet-per-az\"") && cell.contains("kind=\"default\""), "{id}");
        }
        let d: Vec<_> = out.diagnostics.derived.iter().filter(|d| d.target == "subnet").map(|d| d.rule.as_str()).collect();
        assert_eq!(d, ["public-and-private-subnet-per-az"]);
    }

    #[test]
    fn added_cells_are_marked_and_written_parts_are_not() {
        let src = format!("{WEB2}  - waf -- cloudfront\n");
        let out = crate::compile(&src);
        let x = out.drawio.unwrap();
        let cell = |id: &str| x.split("<object ").find(|s| s.starts_with(&format!("id=\"{id}\""))).unwrap().split('>').next().unwrap().to_string();
        for id in ["aws-cloud", "region", "vpc", "az-1a", "pub-1c", "pri-1a", "igw", "nat-1a", "nat-1c"] {
            assert!(cell(id).contains(" added=\"1\""), "{id}");
        }
        for id in ["user", "cloudfront", "waf", "alb", "ecs-1a", "aurora-1c", "s3", "cloudwatch"] {
            assert!(!cell(id).contains("added="), "{id}");
        }
        let added = |t: &str| out.diagnostics.derived.iter().filter(|d| d.target == t).all(|d| d.added);
        let written = |t: &str| out.diagnostics.derived.iter().filter(|d| d.target == t).all(|d| !d.added);
        for t in ["region", "vpc", "az", "subnet", "igw", "nat-1a", "nat"] {
            assert!(out.diagnostics.derived.iter().any(|d| d.target == t) && added(t), "{t}");
        }
        for t in ["cloudfront", "waf", "ecs", "aurora", "s3", "cloudwatch"] {
            assert!(out.diagnostics.derived.iter().any(|d| d.target == t) && written(t), "{t}");
        }
        assert!(out.diagnostics.to_json().contains("\"added\": true"));
    }

    /// A Region frame at (100, 50) holding two parts in a row, at (250, 130) and (450, 130), with no lines.
    fn two_in_a_region() -> (crate::model::Model, crate::layout::Layout) {
        use crate::layout::{Frame, FrameKind, Icon, Layout, Rect};
        use crate::model::{Area, Model, Node};
        let node = |id: &str| Node {
            id: id.into(),
            service: id.into(),
            option: None,
            az: None,
            role: None,
            added: false,
            region: None,
            part: id.into(),
            tag: None,
            area: Area::Region,
            undecided: false,
        };
        let model = Model {
            region: "ap-northeast-1".into(),
            azs: Vec::new(),
            lang: crate::rules::Lang::En,
            nodes: vec![node("s3"), node("cloudwatch")],
            edges: Vec::new(),
            direction: None,
        };
        let region = Frame { id: "region".into(), kind: FrameKind::Region, label: "Region".into(), rect: Rect { x: 100, y: 50, w: 600, h: 300 }, parent: None };
        let icon = |id: &str, x: i32| Icon { id: id.into(), label: id.into(), lines: 1, rect: Rect { x, y: 130, w: 60, h: 60 }, parent: Some("region".into()) };
        (model, Layout { frames: vec![region], icons: vec![icon("s3", 250), icon("cloudwatch", 450)] })
    }

    /// A line from s3 to cloudwatch, labelled `label`, with its label spot as given.
    fn line(id: &str, label: &str, horizontal: bool, flip: bool, near_source: bool, at: Option<u16>) -> crate::route::Wire {
        use crate::route::{Port, Side, Wire, WireStyle};
        Wire {
            id: id.into(),
            from: "s3".into(),
            to: "cloudwatch".into(),
            style: WireStyle::Main,
            label: label.into(),
            exit: Port { side: Side::Right, pos: 50, offset: 0 },
            entry: Port { side: Side::Left, pos: 50, offset: 0 },
            points: Vec::new(),
            horizontal_label: horizontal,
            label_near_source: near_source,
            label_at: at,
            label_flip: flip,
            represents: vec![("s3".into(), "cloudwatch".into())],
            rep: 0,
        }
    }

    #[test]
    fn coordinates_are_relative_to_the_parent_frame() {
        let rules = crate::rules::Rules::builtin();
        let (model, layout) = two_in_a_region();
        let x = super::write(&model, &layout, &[], &crate::diag::Diagnostics::default(), &rules);
        let cell = |id: &str| x.split("<object ").find(|s| s.starts_with(&format!("id=\"{id}\""))).unwrap().to_string();
        // The Region frame has no parent: its own coordinates. The parts: 250 - 100 and 130 - 50 from the Region's corner.
        assert!(cell("region").contains("parent=\"1\"") && cell("region").contains("x=\"100\" y=\"50\" width=\"600\" height=\"300\""), "{x}");
        assert!(cell("s3").contains("parent=\"region\"") && cell("s3").contains("x=\"150\" y=\"80\" width=\"60\" height=\"60\""), "{x}");
        assert!(cell("cloudwatch").contains("x=\"350\" y=\"80\""), "{x}");
    }

    #[test]
    fn labels_switch_language_and_escape_xml() {
        let en = crate::compile(WEB2).drawio.unwrap();
        assert!(en.contains("label=\"Public subnet\"") && en.contains("value=\"Read/Write\""));
        let ja = crate::compile(&format!("{WEB2}set:\n  lang: ja\n")).drawio.unwrap();
        assert!(ja.contains("label=\"パブリックサブネット\"") && ja.contains("label=\"Amazon Aurora&#xa;Writer\""));
        assert_eq!(super::escape("a&b<c>\"d\ne"), "a&amp;b&lt;c&gt;&quot;d&#xa;e");
    }

    #[test]
    fn hole_still_writes_a_diagram_and_errors_do_not() {
        let h = crate::compile("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n");
        assert!(h.drawio.is_some());
        assert_eq!(h.diagnostics.exit_code(), 2);
        assert!(h.drawio.unwrap().contains("strokeColor=#879196"));
        let e = crate::compile("flows:\n  - user -> s3\n");
        assert!(e.drawio.is_none());
        assert_eq!(e.diagnostics.exit_code(), 1);
    }

    #[test]
    fn a_moved_label_is_written_with_its_position_and_side() {
        assert_eq!([0, 250, 500, 661, 1000].map(super::relative), ["-1", "-0.5", "0", "0.322", "1"]);
        let rules = crate::rules::Rules::builtin();
        let (model, layout) = two_in_a_region();
        let wires = [
            line("moved-above", "Read/Write", true, false, false, Some(661)),
            line("moved-two-lines", "ログ・メトリクス\n（全 AZ から）", false, false, false, Some(250)),
            line("below", "HTTPS", true, true, false, None),
            line("left", "HTTPS", false, true, false, None),
            line("near-source", "HTTPS", false, false, true, None),
            line("left-alone", "HTTPS", false, false, false, None),
        ];
        let x = super::write(&model, &layout, &wires, &crate::diag::Diagnostics::default(), &rules);
        let line = |id: &str| x.lines().find(|l| l.contains(&format!("id=\"{id}\""))).unwrap().to_string();
        // Moved along the line, above it and centred.
        let above = line("moved-above");
        assert!(above.contains("<mxGeometry x=\"0.322\" relative=\"1\"") && above.contains("align=center") && !above.contains("spacingLeft"), "{above}");
        // Two lines, moved, beside the line.
        let two = line("moved-two-lines");
        assert!(two.contains("value=\"ログ・メトリクス&#xa;（全 AZ から）\"") && two.contains("<mxGeometry x=\"-0.5\" relative=\"1\"") && two.contains("align=left;spacingLeft=6"), "{two}");
        // Flipped: below a label above the line, left of one beside it.
        let below = line("below");
        assert!(below.contains("align=center") && below.contains("verticalAlign=top") && !below.contains("verticalAlign=bottom"), "{below}");
        let left = line("left");
        assert!(left.contains("align=right;spacingRight=6") && !left.contains("spacingLeft"), "{left}");
        // Toward the source, or left where it was.
        assert!(line("near-source").contains("<mxGeometry x=\"-0.5\" relative=\"1\""));
        assert!(line("left-alone").contains("<mxGeometry relative=\"1\""));
    }

    #[test]
    fn a_reason_reaches_only_the_part_it_names() {
        use crate::diag::{Derived, Diagnostics};
        use crate::layout::{Icon, Layout, Rect};
        use crate::model::{Model, Node};
        use crate::rules::{Lang, Rules};
        let rules = Rules::builtin();
        let node = |id: &str, tag: Option<&str>| Node {
            id: id.into(),
            service: "s3".into(),
            option: None,
            az: None,
            role: None,
            added: false,
            region: None,
            part: id.into(),
            tag: tag.map(String::from),
            area: crate::model::Area::Region,
            undecided: false,
        };
        let model = Model {
            region: "ap-northeast-1".into(),
            azs: Vec::new(),
            lang: Lang::En,
            nodes: vec![node("s3", None), node("s3.web", Some("web"))],
            edges: Vec::new(),
            direction: None,
        };
        let icon = |id: &str, x: i32| Icon { id: id.into(), label: id.into(), lines: 1, rect: Rect { x, y: 0, w: 60, h: 60 }, parent: None };
        let layout = Layout { frames: Vec::new(), icons: vec![icon("s3", 0), icon("s3.web", 200)] };
        let r = rules.rule("regional-service-outside-vpc");
        let why = Derived {
            target: "s3".into(),
            added: false,
            kind: r.kind.clone(),
            rule: "regional-service-outside-vpc".into(),
            reason: r.reason_en.clone(),
            source: r.source.clone(),
        };
        let diag = Diagnostics { derived: vec![why], ..Default::default() };
        let x = super::write(&model, &layout, &[], &diag, &rules);
        let cell = |id: &str| x.split("<object ").find(|s| s.starts_with(&format!("id=\"{id}\""))).unwrap().split('>').next().unwrap().to_string();
        assert!(cell("s3").contains("rule=\"regional-service-outside-vpc\""));
        assert!(!cell("s3.web").contains("rule="));
    }

    // Fails if the writer leaves out a part that has no lines.
    #[test]
    fn a_part_with_no_wires_is_still_drawn() {
        use crate::diag::Diagnostics;
        use crate::layout::{Icon, Layout, Rect};
        use crate::model::{Model, Node};
        use crate::rules::{Lang, Rules};
        let rules = Rules::builtin();
        let node = |id: &str| Node {
            id: id.into(),
            service: id.into(),
            option: None,
            az: None,
            role: None,
            added: false,
            region: None,
            part: id.into(),
            tag: None,
            area: crate::model::Area::Cloud,
            undecided: false,
        };
        let model = Model {
            region: "ap-northeast-1".into(),
            azs: Vec::new(),
            lang: Lang::En,
            nodes: vec![node("waf"), node("cloudfront")],
            edges: Vec::new(),
            direction: None,
        };
        let icon = |id: &str, x: i32| Icon { id: id.into(), label: id.into(), lines: 1, rect: Rect { x, y: 0, w: 60, h: 60 }, parent: None };
        let layout = Layout { frames: Vec::new(), icons: vec![icon("waf", 0), icon("cloudfront", 200)] };
        let x = super::write(&model, &layout, &[], &Diagnostics::default(), &rules);
        let has = |id: &str| x.split("<object ").any(|s| s.starts_with(&format!("id=\"{id}\"")));
        assert!(has("waf") && has("cloudfront"), "{x}");
    }
}
