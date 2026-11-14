//! Semantic model: which concrete parts exist, in which Availability Zone, and how they connect.
//! It knows nothing about drawing; layout and routing read it and never write back.

use crate::def::Rel;
use crate::rules::Lang;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// e.g. "cloudfront", "ecs-1a", "aurora-1c", "igw", "nat-1a".
    pub id: String,
    /// Service key in the rules data.
    pub service: String,
    pub option: Option<String>,
    /// Index into `Model.azs`, for per-AZ parts only.
    pub az: Option<usize>,
    /// "writer" or "reader".
    pub role: Option<String>,
    /// Added by inference, not written in the definition.
    pub added: bool,
    /// The Region the part lives in when it is not the diagram's Region, e.g. "us-east-1"
    /// for a certificate used by CloudFront.
    pub region: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub rel: Rel,
    /// A key of labels.toml, or empty for no label.
    pub label: String,
    /// The hole this edge belongs to while it is not filled.
    pub hole: Option<String>,
    /// The traffic goes through a VPC endpoint (a route answered with `endpoint`).
    pub endpoint: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub region: String,
    /// Full names, e.g. ["ap-northeast-1a", "ap-northeast-1c"].
    pub azs: Vec<String>,
    pub lang: Lang,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}
