//! Every id the model and the diagram are read by is made here, so two kinds of id never collide.
//!
//! A part with a dot in its name is a named part ("lambda.api"). `@`, `[` and `]` can never be
//! part of a written name, so the forms below cannot be taken by another part.

fn is_named(part: &str) -> bool {
    part.contains('.')
}

/// The copy of a part in one zone: "ecs-1a" for a part without a name after the dot, "lambda.api@1a" for a named one.
pub(crate) fn copy_id(part: &str, zone_suffix: &str) -> String {
    if is_named(part) {
        format!("{part}@{zone_suffix}")
    } else {
        format!("{part}-{zone_suffix}")
    }
}

/// The cell id of a line: "edge-<from>-to-<to>", unchanged.
pub(crate) fn edge_id(from: &str, to: &str) -> String {
    format!("edge-{from}-to-{to}")
}

fn hole_part(part: &str) -> String {
    if is_named(part) {
        format!("[{part}]")
    } else {
        part.to_string()
    }
}

/// The hole for the route between two parts: "route.ecs.s3"; a part with a dot in its name is put in brackets, "route.[lambda.api].s3".
pub(crate) fn route_hole(from_part: &str, to_part: &str) -> String {
    format!("route.{}.{}", hole_part(from_part), hole_part(to_part))
}

/// The hole for a part's kind: "kind.apigateway.public", unchanged.
pub(crate) fn kind_hole(part: &str) -> String {
    format!("kind.{part}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fails if the ids of copies of parts without a name change.
    #[test]
    fn copies_of_unnamed_parts_keep_their_ids() {
        assert_eq!(copy_id("ecs", "1a"), "ecs-1a");
        assert_eq!(copy_id("nat", "1c"), "nat-1c");
    }

    // Fails if the copy of a named part can be taken by another part such as "lambda.api-1a".
    #[test]
    fn copies_of_named_parts_use_an_at_sign() {
        assert_eq!(copy_id("lambda.api", "1a"), "lambda.api@1a");
    }

    // Fails if the copy id is made in a form a written name can also take.
    #[test]
    fn a_copy_never_takes_another_parts_name() {
        let parts = ["user", "apigateway", "lambda.api", "aurora", "lambda.api-1a", "dynamodb"];
        let mut ids: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
        for p in parts {
            for zone in ["1a", "1c", "1d"] {
                ids.push(copy_id(p, zone));
            }
        }
        let n = ids.len();
        assert_eq!(n, 24);
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "{ids:?}");
    }

    // Fails if two different pairs of parts get the same route hole.
    #[test]
    fn route_holes_bracket_named_parts() {
        assert_eq!(route_hole("ecs", "s3"), "route.ecs.s3");
        assert_eq!(route_hole("lambda.api", "s3"), "route.[lambda.api].s3");
        assert_eq!(route_hole("lambda", "s3.uploads"), "route.lambda.[s3.uploads]");
        assert_eq!(route_hole("lambda.s3", "dynamodb"), "route.[lambda.s3].dynamodb");
        assert_eq!(route_hole("lambda", "s3.dynamodb"), "route.lambda.[s3.dynamodb]");
        assert_ne!(route_hole("lambda.s3", "dynamodb"), route_hole("lambda", "s3.dynamodb"));
    }

    // Fails if the kind holes the answers are written with, or the line ids in the diagram, change.
    #[test]
    fn kind_holes_and_line_ids_are_unchanged() {
        assert_eq!(kind_hole("apigateway.public"), "kind.apigateway.public");
        assert_eq!(edge_id("ecs-1a", "s3"), "edge-ecs-1a-to-s3");
    }
}
