//! Diagnostics: everything the compiler reports besides the diagram itself.

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DerivedKind {
    Constraint,
    Default,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Derived {
    pub target: String,
    /// True when the target is something archwire added (a frame, the Internet Gateway, NAT
    /// gateways, the Availability Zones); false when the reason places a part the definition wrote.
    pub added: bool,
    pub kind: DerivedKind,
    pub rule: String,
    pub reason: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Hole {
    pub hole: String,
    pub at: String,
    pub need: String,
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct Problem {
    pub code: String,
    pub at: Option<String>,
    pub got: Option<String>,
    pub allowed: Vec<String>,
    pub rule: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct Versions {
    pub archwire: String,
    pub rules: String,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
pub struct Diagnostics {
    pub versions: Versions,
    pub derived: Vec<Derived>,
    pub holes: Vec<Hole>,
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
    pub notes: Vec<String>,
}

impl Diagnostics {
    /// Pretty JSON (2-space indent) with a trailing LF. Field order follows declaration order.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("diagnostics serialize");
        s.push('\n');
        s
    }

    /// 1 if any error, else 2 if any hole is left, else 0.
    pub fn exit_code(&self) -> i32 {
        if !self.errors.is_empty() {
            1
        } else if !self.holes.is_empty() {
            2
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_orders_error_over_hole() {
        let mut d = Diagnostics::default();
        assert_eq!(d.exit_code(), 0);
        d.holes.push(Hole { hole: "route.ecs.s3".into(), at: "flows[1]".into(), need: "ecs to s3".into(), choices: vec!["endpoint".into(), "nat".into()] });
        assert_eq!(d.exit_code(), 2);
        d.errors.push(Problem { code: "UNKNOWN_SERVICE".into(), ..Default::default() });
        assert_eq!(d.exit_code(), 1);
    }

    #[test]
    fn json_is_stable_and_ends_with_lf() {
        let d = Diagnostics::default();
        let a = d.to_json();
        assert_eq!(a, d.to_json());
        assert!(a.ends_with("}\n"));
        assert!(a.find("\"versions\"").unwrap() < a.find("\"derived\"").unwrap());
    }
}
