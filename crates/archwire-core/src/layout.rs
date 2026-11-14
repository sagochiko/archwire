//! Layout: where every frame and icon goes, as absolute integer coordinates.
//! Every position is a formula over the named constants below, so the same model
//! always gives the same picture.

use crate::def::Rel;
use crate::model::{Model, Node};
use crate::rules::{Lang, Place, Rules, Scope, Service, Subnet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind { Cloud, Region, Vpc, Az, PublicSubnet, PrivateSubnet }

#[derive(Debug, Clone, PartialEq)]
pub struct Frame { pub id: String, pub kind: FrameKind, pub label: String, pub rect: Rect, pub parent: Option<String> }

/// `id` is the `Node::id` it draws.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon { pub id: String, pub label: String, pub lines: u8, pub rect: Rect, pub parent: Option<String> }

#[derive(Debug, Clone, PartialEq)]
pub struct Layout { pub frames: Vec<Frame>, pub icons: Vec<Icon> }

impl Layout {
    /// Panics on an unknown id: callers only ask for parts the layout placed.
    pub fn rect(&self, id: &str) -> Rect {
        self.frames.iter().find(|f| f.id == id).map(|f| f.rect)
            .or_else(|| self.icons.iter().find(|i| i.id == id).map(|i| i.rect))
            .unwrap_or_else(|| panic!("no frame or icon `{id}`"))
    }
}

const ICON: i32 = 60;

// Horizontal.
const VPC_X: i32 = 100;
/// From the VPC edge to the outer AZ frames, on both sides.
const AZ_INSET: i32 = 30;
const AZ_W: i32 = 390;
const AZ_GAP: i32 = 20;
const SUB_W: i32 = 350;
/// With two AZs the subnets hug the boundary between them, this far from it.
const SUB_TO_BOUNDARY: i32 = 10;
/// The main column sits in the middle of its subnet.
const COL_IN_SUB: i32 = SUB_W / 2 - ICON / 2;
/// With two AZs a NAT gateway sits this far from the subnet's outer edge.
const NAT_INSET: i32 = 20;
/// With three AZs a NAT gateway sits this far right of the main column.
const NAT_RIGHT_OF_COL: i32 = 120;
/// WAF and ACM sit this far left and right of CloudFront.
const GLOBAL_SIDE: i32 = 200;
/// From the VPC's right edge to the column of Regional services. Wider than the Skill's 70, so
/// the label of a line from the VPC fits, on two lines, between the VPC's side and the icon.
const SIDE_GAP: i32 = 140;
/// From the Regional column's right edge to the Region frame's right edge.
const SIDE_TO_REGION: i32 = 50;
/// The Region frame inside the cloud frame, and the VPC inside the Region, on the left and right.
const FRAME_INSET: i32 = 30;

// Vertical.
const USER_Y: i32 = 20;
const CLOUD_Y: i32 = 130;
const GLOBAL_Y: i32 = 170;
const REGION_Y: i32 = 290;
const VPC_Y: i32 = 330;
const IGW_Y: i32 = 360;
const AZ_Y: i32 = 480;
const PUB_Y: i32 = AZ_Y + 30;
const PUB_H: i32 = 150;
/// An icon in the public subnet row sits this far below the subnet's top.
const ICON_IN_PUB: i32 = 40;
const PRI_Y: i32 = PUB_Y + PUB_H + 20;
const PRI_HEAD: i32 = 50;
const PRI_TAIL: i32 = 120;
const ROW_STEP: i32 = 210;
const AZ_BOTTOM_PAD: i32 = 15;
const VPC_BOTTOM_PAD: i32 = 20;
const REGION_BOTTOM_PAD: i32 = 25;
const CLOUD_BOTTOM_PAD: i32 = 25;
/// With three AZs the load balancer gets its own row above the AZ frames; everything below moves this far.
const ABOVE_AZ_ROW: i32 = 160;
/// Regional services that would share a y move down by this much each.
const SIDE_STEP: i32 = 160;
/// Room for an icon's label under it: the first line, then each further line.
const LABEL_FIRST: i32 = 24;
const LABEL_LINE: i32 = 14;

/// Global services that sit left of the main column; the other Global ones not in a main flow sit right.
const GLOBAL_LEFT: [&str; 1] = ["waf"];

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

fn icon_at(x: i32, y: i32) -> Rect {
    rect(x, y, ICON, ICON)
}

/// The fixed geometry for a number of AZs and private rows.
struct Grid {
    az_x: Vec<i32>,
    sub_x: Vec<i32>,
    /// Main column x of the main request path.
    spine: i32,
    vpc_w: i32,
    /// How far everything from the AZ frames down moves.
    dy: i32,
    pri_h: i32,
}

impl Grid {
    fn new(azs: usize, rows: i32) -> Grid {
        let n = azs as i32;
        let az_x: Vec<i32> = (0..n).map(|i| VPC_X + AZ_INSET + i * (AZ_W + AZ_GAP)).collect();
        let sub_x = az_x
            .iter()
            .enumerate()
            .map(|(i, x)| {
                x + match (azs, i) {
                    (2, 0) => AZ_W - SUB_W - SUB_TO_BOUNDARY,
                    (2, _) => SUB_TO_BOUNDARY,
                    _ => (AZ_W - SUB_W) / 2,
                }
            })
            .collect();
        let row_w = n * AZ_W + (n - 1) * AZ_GAP;
        Grid {
            spine: VPC_X + AZ_INSET + row_w / 2 - ICON / 2,
            vpc_w: 2 * AZ_INSET + row_w,
            dy: if azs > 2 { ABOVE_AZ_ROW } else { 0 },
            pri_h: PRI_HEAD + (rows - 1) * ROW_STEP + PRI_TAIL,
            az_x,
            sub_x,
        }
    }
    fn col(&self, az: usize) -> i32 {
        self.sub_x[az] + COL_IN_SUB
    }
    fn pub_y(&self) -> i32 {
        PUB_Y + self.dy
    }
    fn pri_y(&self) -> i32 {
        PRI_Y + self.dy
    }
    fn row_y(&self, row: usize) -> i32 {
        self.pri_y() + PRI_HEAD + row as i32 * ROW_STEP
    }
    fn az_y(&self) -> i32 {
        AZ_Y + self.dy
    }
    fn az_h(&self) -> i32 {
        self.pri_y() + self.pri_h + AZ_BOTTOM_PAD - self.az_y()
    }
    fn vpc_h(&self) -> i32 {
        self.az_y() + self.az_h() + VPC_BOTTOM_PAD - VPC_Y
    }
    fn side_x(&self) -> i32 {
        VPC_X + self.vpc_w + SIDE_GAP
    }
    fn nat_x(&self, az: usize, azs: usize) -> i32 {
        match (azs, az) {
            (2, 0) => self.sub_x[0] + NAT_INSET,
            (2, _) => self.sub_x[az] + SUB_W - NAT_INSET - ICON,
            _ => self.col(az) + NAT_RIGHT_OF_COL,
        }
    }
}

pub fn layout(model: &Model, rules: &Rules) -> Layout {
    let svc = |key: &str| rules.service(key).unwrap_or_else(|| panic!("model has unknown service `{key}`"));
    let lang = model.lang;
    let suffix = |i: usize| model.azs[i][model.azs[i].len() - 2..].to_string();
    let is_private = |key: &str| {
        let s = svc(key);
        s.scope == Scope::Vpc && s.subnet == Some(Subnet::Private)
    };

    // Private rows: one per private service, in the order the model lists them.
    let mut rows: Vec<&str> = Vec::new();
    for n in model.nodes.iter().filter(|n| is_private(&n.service)) {
        if !rows.contains(&n.service.as_str()) {
            rows.push(&n.service);
        }
    }
    let g = Grid::new(model.azs.len(), rows.len().max(1) as i32);

    // Icons, except Regional ones, which take their y from what they connect to.
    let mut icons: Vec<Icon> = Vec::new();
    let mut global_left = g.spine - GLOBAL_SIDE;
    let mut global_right = g.spine + GLOBAL_SIDE;
    for n in &model.nodes {
        let s = svc(&n.service);
        let az_parent = |prefix: &str| n.az.map(|i| format!("{prefix}-{}", suffix(i)));
        let (r, parent) = match s.scope {
            Scope::Outside => (icon_at(g.spine, USER_Y), None),
            Scope::Global => {
                let in_main = model.edges.iter().any(|e| e.rel == Rel::Main && (e.from == n.id || e.to == n.id));
                let x = if in_main {
                    g.spine
                } else if GLOBAL_LEFT.contains(&n.service.as_str()) {
                    global_left -= GLOBAL_SIDE;
                    global_left + GLOBAL_SIDE
                } else {
                    global_right += GLOBAL_SIDE;
                    global_right - GLOBAL_SIDE
                };
                (icon_at(x, GLOBAL_Y), Some("aws-cloud".to_string()))
            }
            Scope::Regional => continue,
            Scope::Vpc if s.place == Place::Gateway => (icon_at(g.spine, IGW_Y), Some("vpc".to_string())),
            Scope::Vpc if is_private(&n.service) => {
                let row = rows.iter().position(|k| *k == n.service).expect("rows hold every private service");
                match n.az {
                    Some(i) => (icon_at(g.col(i), g.row_y(row)), az_parent("pri")),
                    None => (icon_at(g.spine, g.row_y(row)), Some("vpc".to_string())),
                }
            }
            Scope::Vpc => match n.az {
                Some(i) => (icon_at(g.nat_x(i, model.azs.len()), g.pub_y() + ICON_IN_PUB), az_parent("pub")),
                // A load balancer spans the AZs: on the boundary with two, in its own row above them with three.
                None if g.dy > 0 => (icon_at(g.spine, IGW_Y + ABOVE_AZ_ROW), Some("vpc".to_string())),
                None => (icon_at(g.spine, g.pub_y() + ICON_IN_PUB), Some("vpc".to_string())),
            },
        };
        icons.push(icon(n, s, rules, lang, r, parent));
    }

    // Regional services: level with a private part they connect to, else with a VPC part
    // sending to them, else at the VPC's top. Ones that would share a y move down.
    let side_x = g.side_x();
    let mut used: Vec<i32> = Vec::new();
    for n in model.nodes.iter().filter(|n| svc(&n.service).scope == Scope::Regional) {
        let others: Vec<&str> = model
            .edges
            .iter()
            .filter_map(|e| match (e.from == n.id, e.to == n.id) {
                (true, _) => Some(e.to.as_str()),
                (_, true) => Some(e.from.as_str()),
                _ => None,
            })
            .collect();
        let node_of = |id: &str| model.nodes.iter().find(|m| m.id == id).expect("edge ends are nodes");
        let y_of = |id: &str| icons.iter().find(|i| i.id == id).map(|i| i.rect.y);
        let private = others.iter().find(|o| is_private(&node_of(o).service));
        let in_vpc = others.iter().find(|o| svc(&node_of(o).service).scope == Scope::Vpc);
        let mut y = private.or(in_vpc).and_then(|o| y_of(o)).unwrap_or(VPC_Y);
        while used.contains(&y) {
            y += SIDE_STEP;
        }
        used.push(y);
        icons.push(icon(n, svc(&n.service), rules, lang, icon_at(side_x, y), Some("region".to_string())));
    }
    // Keep icons in model order.
    icons.sort_by_key(|i| model.nodes.iter().position(|n| n.id == i.id));

    // Frames, outermost first. The Region grows downward if Regional services stack below the VPC.
    let vpc = rect(VPC_X, VPC_Y, g.vpc_w, g.vpc_h());
    let side_bottom = icons
        .iter()
        .filter(|i| i.parent.as_deref() == Some("region"))
        .map(|i| i.rect.y + ICON + LABEL_FIRST + (i.lines as i32 - 1) * LABEL_LINE)
        .max()
        .unwrap_or(0);
    let region_bottom = (vpc.y + vpc.h).max(side_bottom) + REGION_BOTTOM_PAD;
    let region_x = VPC_X - FRAME_INSET;
    let region_right = side_x + ICON + SIDE_TO_REGION;
    let region = rect(region_x, REGION_Y, region_right - region_x, region_bottom - REGION_Y);
    let cloud_x = region_x - FRAME_INSET;
    let cloud = rect(cloud_x, CLOUD_Y, region_right + FRAME_INSET - cloud_x, region_bottom + CLOUD_BOTTOM_PAD - CLOUD_Y);

    let frame = |id: &str, kind: FrameKind, label: String, rect: Rect, parent: Option<&str>| Frame {
        id: id.into(),
        kind,
        label,
        rect,
        parent: parent.map(String::from),
    };
    let mut frames = vec![
        frame("aws-cloud", FrameKind::Cloud, "AWS Cloud".into(), cloud, None),
        frame("region", FrameKind::Region, rules.text(&format!("region.{}", model.region), lang), region, Some("aws-cloud")),
        frame("vpc", FrameKind::Vpc, "VPC".into(), vpc, Some("region")),
    ];
    for (i, name) in model.azs.iter().enumerate() {
        let (s, az) = (suffix(i), format!("az-{}", suffix(i)));
        frames.push(frame(&az, FrameKind::Az, name.clone(), rect(g.az_x[i], g.az_y(), AZ_W, g.az_h()), Some("vpc")));
        let public = rect(g.sub_x[i], g.pub_y(), SUB_W, PUB_H);
        frames.push(frame(&format!("pub-{s}"), FrameKind::PublicSubnet, rules.text("public_subnet", lang), public, Some(&az)));
        let private = rect(g.sub_x[i], g.pri_y(), SUB_W, g.pri_h);
        frames.push(frame(&format!("pri-{s}"), FrameKind::PrivateSubnet, rules.text("private_subnet", lang), private, Some(&az)));
    }

    Layout { frames, icons }
}

/// The service (or kind) label, then the role word and a Region other than the diagram's, each on its own line.
fn icon(n: &Node, s: &Service, rules: &Rules, lang: Lang, rect: Rect, parent: Option<String>) -> Icon {
    let key = n.option.as_ref().and_then(|o| s.options.get(o)).map_or(&s.label, |o| &o.label);
    let mut label = rules.text(key, lang);
    if let Some(role) = &n.role {
        label = format!("{label}\n{}", rules.text(role, lang));
    }
    if let Some(region) = &n.region {
        label = format!("{label}\n{region}");
    }
    let lines = label.lines().count() as u8;
    Icon { id: n.id.clone(), label, lines, rect, parent }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay(src: &str) -> Layout {
        let r = Rules::builtin();
        let (m, _) = crate::infer::infer(&crate::def::parse(src).unwrap(), &r);
        layout(&m.unwrap(), &r)
    }
    const WEB2: &str = "flows:\n  - user -> cloudfront -> alb -> ecs(fargate) -> aurora\n  - cloudfront -> s3\n  - ecs -> internet\n  - ecs ~> cloudwatch\n";

    #[test]
    fn two_az_matches_the_skill_coordinates() {
        let l = lay(WEB2);
        let r = |id| { let q = l.rect(id); (q.x, q.y, q.w, q.h) };
        assert_eq!(r("user"), (500, 20, 60, 60));
        assert_eq!(r("cloudfront"), (500, 170, 60, 60));
        assert_eq!(r("igw"), (500, 360, 60, 60));
        assert_eq!(r("alb"), (500, 550, 60, 60));
        assert_eq!(r("az-1a"), (130, 480, 390, 595));
        assert_eq!(r("pub-1c"), (550, 510, 350, 150));
        assert_eq!(r("pri-1a"), (160, 680, 350, 380));
        assert_eq!(r("ecs-1a"), (305, 730, 60, 60));
        assert_eq!(r("aurora-1c"), (695, 940, 60, 60));
        assert_eq!(r("nat-1a"), (180, 550, 60, 60));
        assert_eq!(r("nat-1c"), (820, 550, 60, 60));
        assert_eq!(r("vpc"), (100, 330, 860, 765));
        // The Regional column sits 70 further right than in the Skill, to leave room for line labels.
        assert_eq!(r("s3"), (1100, 330, 60, 60));
        assert_eq!(r("cloudwatch"), (1100, 730, 60, 60));
    }

    #[test]
    fn every_icon_sits_inside_its_parent_and_icons_do_not_overlap() {
        for src in [WEB2.to_string(), format!("{WEB2}set:\n  azs: 3\n")] {
            let l = lay(&src);
            for i in &l.icons {
                if let Some(p) = &i.parent { if p.starts_with("pub-") || p.starts_with("pri-") {
                    let f = l.rect(p);
                    assert!(i.rect.x >= f.x && i.rect.x + 60 <= f.x + f.w && i.rect.y >= f.y && i.rect.y + 60 <= f.y + f.h, "{} in {}", i.id, p);
                } }
            }
            for a in &l.icons { for b in &l.icons { if a.id < b.id {
                let apart = a.rect.x + 60 <= b.rect.x || b.rect.x + 60 <= a.rect.x || a.rect.y + 60 <= b.rect.y || b.rect.y + 60 <= a.rect.y;
                assert!(apart, "{} overlaps {}", a.id, b.id);
            } } }
        }
    }

    #[test]
    fn three_az_puts_one_alb_above_the_middle_az() {
        let l = lay(&format!("{WEB2}set:\n  azs: 3\n"));
        let (alb, mid) = (l.rect("alb"), l.rect("az-1c"));
        assert_eq!(alb.x + 30, mid.x + mid.w / 2);
        assert!(alb.y + 60 < mid.y);
        assert_eq!(l.rect("igw").x, alb.x);
        assert_eq!(l.icons.iter().filter(|i| i.id == "alb").count(), 1);
    }

    #[test]
    fn frames_labels_and_parents_follow_the_skill_diagram() {
        let l = lay(&format!("{WEB2}  - waf -- cloudfront\n  - acm -- cloudfront\n"));
        let r = |id| { let q = l.rect(id); (q.x, q.y, q.w, q.h) };
        assert_eq!(r("aws-cloud"), (40, 130, 1200, 1015));
        assert_eq!(r("region"), (70, 290, 1140, 830));
        assert_eq!((r("waf"), r("acm")), ((300, 170, 60, 60), (700, 170, 60, 60)));
        let frames: Vec<_> = l.frames.iter().map(|f| (f.id.as_str(), f.label.as_str(), f.parent.as_deref())).collect();
        assert_eq!(frames, [
            ("aws-cloud", "AWS Cloud", None), ("region", "Tokyo (ap-northeast-1)", Some("aws-cloud")), ("vpc", "VPC", Some("region")),
            ("az-1a", "ap-northeast-1a", Some("vpc")), ("pub-1a", "Public subnet", Some("az-1a")), ("pri-1a", "Private subnet", Some("az-1a")),
            ("az-1c", "ap-northeast-1c", Some("vpc")), ("pub-1c", "Public subnet", Some("az-1c")), ("pri-1c", "Private subnet", Some("az-1c")),
        ]);
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
        ]);
        let ja = lay(&format!("{WEB2}set:\n  lang: ja\n"));
        assert_eq!(ja.frames[1].label, "東京リージョン（ap-northeast-1）");
        assert_eq!(ja.frames[4].label, "パブリックサブネット");
    }

    #[test]
    fn side_services_on_the_same_y_stack_down_and_stay_in_the_region() {
        let l = lay("flows:\n  - user -> alb -> ecs\n  - ecs -> s3\n  - ecs ~> cloudwatch\n  - alb ~> cloudwatch\n");
        assert_eq!((l.rect("s3").y, l.rect("cloudwatch").y), (730, 890));
        let (cw, region, vpc) = (l.rect("cloudwatch"), l.rect("region"), l.rect("vpc"));
        assert!(cw.y > vpc.y + vpc.h - 60);
        assert!(cw.y + 60 < region.y + region.h && region.y + region.h < l.rect("aws-cloud").y + l.rect("aws-cloud").h);
        let aux = lay("flows:\n  - user -> alb -> ecs\n  - alb ~> cloudwatch\n");
        assert_eq!(aux.rect("cloudwatch").y, aux.rect("alb").y);
    }

    #[test]
    fn adding_a_side_service_does_not_move_the_vpc_contents() {
        let (a, b) = (lay("flows:\n  - user -> alb -> ecs -> aurora\n"), lay("flows:\n  - user -> alb -> ecs -> aurora\n  - ecs ~> cloudwatch\n"));
        for id in ["alb", "igw", "ecs-1a", "ecs-1c", "aurora-1a", "aurora-1c", "vpc", "az-1c"] { assert_eq!(a.rect(id), b.rect(id), "{id}"); }
    }
}
