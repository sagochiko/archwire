//! Layout: where every frame and icon goes, as absolute integer coordinates, and the
//! label each icon shows.

use crate::model::Node;
use crate::rules::{Lang, Rules, Service};

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

/// The service (or kind) label, then the name after the dot, the role word and a Region other
/// than the diagram's, each on its own line.
pub(crate) fn icon(n: &Node, s: &Service, rules: &Rules, lang: Lang, rect: Rect, parent: Option<String>) -> Icon {
    let key = n.option.as_ref().and_then(|o| s.options.get(o)).map_or(&s.label, |o| &o.label);
    let mut label = rules.text(key, lang);
    if let Some(tag) = &n.tag {
        label = format!("{label}\n{tag}");
    }
    if let Some(role) = &n.role {
        label = format!("{label}\n{}", rules.text(role, lang));
    }
    if let Some(region) = &n.region {
        label = format!("{label}\n{region}");
    }
    if n.undecided {
        label = format!("{label}\n{}", rules.text("undecided", lang));
    }
    let lines = label.lines().count() as u8;
    Icon { id: n.id.clone(), label, lines, rect, parent }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_shows_the_name_after_the_dot_below_the_service() {
        let r = Rules::builtin();
        let n = |service: &str, role: Option<&str>| Node {
            id: format!("{service}.web"),
            service: service.into(),
            option: None,
            az: None,
            role: role.map(String::from),
            added: false,
            region: None,
            part: format!("{service}.web"),
            tag: Some("web".into()),
            area: crate::inference::area_of(r.service(service).unwrap()),
            undecided: false,
        };
        let at = Rect { x: 0, y: 0, w: 60, h: 60 };
        let s3 = icon(&n("s3", None), r.service("s3").unwrap(), &r, Lang::En, at, None);
        assert_eq!((s3.label.as_str(), s3.lines), ("Amazon S3\nweb", 2));
        let aurora = icon(&n("aurora", Some("writer")), r.service("aurora").unwrap(), &r, Lang::En, at, None);
        assert_eq!((aurora.label.as_str(), aurora.lines), ("Amazon Aurora\nweb\nWriter", 3));
    }
}
