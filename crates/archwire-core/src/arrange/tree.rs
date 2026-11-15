//! The membership tree the layout arranges.

use super::{Arrange, FrameItem, Item};
use crate::layout::FrameKind;
use crate::model::{Area, Model};

const VPC_FRAME: &str = "vpc";

/// The membership tree: the parts outside the VPC, and one VPC frame item holding every part whose area is VpcEdge or Subnet.
pub(crate) fn tree(model: &Model) -> Vec<Item> {
    let inside = |i: &usize| matches!(model.nodes[*i].area, Area::VpcEdge | Area::Subnet(_));
    let all = 0..model.nodes.len();
    let mut out: Vec<Item> = all.clone().filter(|i| !inside(i)).map(Item::Part).collect();
    let vpc: Vec<Item> = all.filter(inside).map(Item::Part).collect();
    if !vpc.is_empty() {
        out.push(Item::Frame(FrameItem { id: VPC_FRAME.into(), kind: FrameKind::Vpc, arrange: Arrange::AzGrid, items: vpc }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rules;

    fn model(src: &str, grid: bool) -> Model {
        let r = Rules::builtin();
        let def = crate::def::parse(src).unwrap();
        let m = if grid { crate::infer::infer(&def, &r).0 } else { crate::flow_infer::infer(&def, &r).0 };
        m.unwrap()
    }

    // Fails if a part outside the VPC is left out of the tree, or put in a frame item.
    #[test]
    fn without_a_vpc_the_tree_is_every_part_in_model_order() {
        let m = model("flows:\n  - user -> cloudfront -> s3\n  - user -> apigateway(http) -> lambda\n  - cognito -- apigateway\n", false);
        let want: Vec<Item> = (0..m.nodes.len()).map(Item::Part).collect();
        assert_eq!(tree(&m), want);
    }

    // Fails if a part inside the VPC is left outside the VPC frame item, or one outside is put in it.
    #[test]
    fn parts_inside_the_vpc_go_into_one_vpc_frame_item() {
        let m = model("flows:\n  - user -> cloudfront -> alb -> ecs -> aurora\n  - ecs ~> cloudwatch\n", true);
        let got = tree(&m);
        let outside: Vec<&str> = got.iter().filter_map(|it| match it { Item::Part(i) => Some(m.nodes[*i].id.as_str()), _ => None }).collect();
        let frames: Vec<&FrameItem> = got.iter().filter_map(|it| match it { Item::Frame(f) => Some(f), _ => None }).collect();
        assert_eq!(frames.len(), 1, "{got:?}");
        let f = frames[0];
        assert_eq!((f.id.as_str(), f.kind, f.arrange), ("vpc", FrameKind::Vpc, Arrange::AzGrid));
        assert!(matches!(got.last(), Some(Item::Frame(_))), "{got:?}");
        for n in &m.nodes {
            let inner = matches!(n.area, Area::VpcEdge | Area::Subnet(_));
            assert_eq!(outside.contains(&n.id.as_str()), !inner, "{}", n.id);
            let held = f.items.iter().any(|it| matches!(it, Item::Part(i) if m.nodes[*i].id == n.id));
            assert_eq!(held, inner, "{}", n.id);
        }
        assert!(outside.contains(&"cloudfront") && outside.contains(&"cloudwatch"), "{outside:?}");
        assert!(f.items.len() >= 4, "{:?}", f.items);
    }
}
