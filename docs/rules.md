# Rules

archwire's knowledge of AWS is data, not code. It lives in three TOML files under `rules/aws/`, which are compiled into the program. Changing them means rebuilding.

`services.toml` describes each service: its `key` (the name used in a definition), where it sits (`scope` is `outside`, `global`, `regional` or `vpc`, and for VPC services `subnet` is `public` or `private`), how many copies are drawn and where (`place`: `single` for one, `gateway` for one on the VPC's edge, `per_az` for one per Availability Zone, `span` for one spanning the zones, or `writer_reader` for Aurora's writer and readers), the icon (`icon`, `shape` and `color`), the `label`, the kinds a service can have (`options`), the label on lines into it (`in_label`), and `derived_only` for the services archwire adds by itself and that cannot be written in a definition (`igw`, `nat`). Two fields decide which connections a global service takes part in: `flow_source = true` marks one that may carry main and auxiliary flows (`cloudfront`), and `attach_to` makes a service an attachment that carries no flows and may only be associated with the services it lists (`waf` and `acm`, both with `["cloudfront"]`).

`rules.toml` holds the reasons. Each rule has an `id`, a `kind`, an optional `source`, and a reason in English (`reason_en`) and Japanese (`reason_ja`). The file also carries `version`, which is printed in every report as `versions.rules`.

`labels.toml` holds the display words, each in English and Japanese: subnet names, line labels such as HTTPS, and the sentences used in notes and holes.

## Constraints and defaults

A rule's `kind` is `constraint` or `default`, and the difference is the point of the file.

A constraint states something AWS requires or does, and it must have a `source`, the URL of the AWS documentation page that says so. A constraint without a source is rejected when the rules are loaded. A reason claims no more than its source says; where the reason also says how archwire draws the fact, that part is written as archwire's choice. The six constraints today are the following.

| Rule | What it says |
|---|---|
| `global-service-outside-region` | AWS lists CloudFront and AWS WAF for CloudFront as global services; archwire draws them outside the Region frame |
| `cloudfront-certificate-in-us-east-1` | a Certificate Manager certificate used with CloudFront is requested or imported in us-east-1; archwire draws it outside the Region frame with `us-east-1` under its name |
| `load-balancer-in-vpc` | an Application Load Balancer is created in subnets of a VPC |
| `alb-spans-two-azs` | an Application Load Balancer needs subnets in at least two Availability Zones |
| `internet-facing-alb-in-vpc-with-igw` | an internet-facing Application Load Balancer can only use a VPC with an Internet Gateway |
| `private-egress-through-nat` | a private subnet reaches the internet through a NAT gateway in a public subnet |

A default is archwire's own choice for something the definition did not say. It is not an AWS requirement, and the report marks it `default` so nobody mistakes it for one. A source is optional. The nine defaults today are `resources-inside-aws-cloud-frame`, `regional-service-outside-vpc`, `nat-per-az`, `default-two-azs`, `public-and-private-subnet-per-az`, `compute-in-private`, `database-in-private`, `aurora-writer-reader` and `default-region`. `nat-per-az` cites an AWS page for its reasoning. It is still a default, because a single NAT gateway is also valid and `set.nat: single` asks for it.

Every part archwire adds gets its reasons from these rules, in the report under `derived` and inside the `.drawio` file as the cell attributes `derived`, `rule`, `kind` and `source`. A reason can also be about a part the definition wrote, such as `compute-in-private` for ECS; the report tells the two apart with `added`, and the `.drawio` file marks only the cells archwire added with `added="1"`.

## Loader errors

The rules are checked when archwire starts. The built-in rules are valid, so a user of the released program does not see these. If you edit the files and make a mistake, the program stops with a panic message that lists the problems, each with one of these codes and the rule or file it is about.

| Code | Meaning |
|---|---|
| `SERVICES_PARSE`, `LABELS_PARSE`, `RULES_PARSE` | the file is not valid TOML or does not have the expected fields |
| `RULE_BAD_KIND` | a rule's `kind` is neither `constraint` nor `default` |
| `RULE_WITHOUT_SOURCE` | a `constraint` has no `source` |
| `RULE_DUPLICATE` | two rules have the same `id` |
| `SERVICE_DUPLICATE` | two services have the same `key` |

For example, deleting the `source` line of `private-egress-through-nat` and running any `compile` stops with `RULE_WITHOUT_SOURCE` at `private-egress-through-nat`.

## Adding or changing a rule

A rule in `rules.toml` is the text and the source. Where it applies, and which part it is recorded against, is decided in `crates/archwire-core/src/infer.rs`, which looks a rule up by its `id` and stops if the id does not exist. So a new rule needs both: an entry in `rules.toml`, and the place in `infer.rs` that adds it to the report.

Then check that nothing else moved.

```sh
cargo test --workspace
```

Bump `version` in `rules.toml` whenever the rules change, so a report says which rules produced it.

The test `every_case_matches_its_expected_output` compiles every definition under `tests/cases` and compares the report and the diagram with the stored `expected.json` and `expected.drawio`, byte for byte. The rules version is written into every report and every diagram, so after the bump every case differs in that one value. What you have to check is that nothing else changed: a new rule that applies to a shape no case uses changes only the version, while any other difference means the change altered an existing example. That can be right, but it is a decision to make on purpose.

Rewrite the expected files, look at every difference besides the version, and look at the rendered diagrams.

```sh
ARCHWIRE_BLESS=1 cargo test -p archwire --test cases
git diff tests/cases
sh tools/check_cases.sh
```

`tools/check_cases.sh` needs the draw.io desktop command line (`drawio`) and Python with Pillow. It runs `tools/geocheck.py` on every expected diagram, which fails on a line over text, a line over an icon, or two lines on top of each other, and writes a picture of each case to `out/` for you to look at.
