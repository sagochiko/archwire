# Rules

archwire's knowledge of AWS is data, not code. It lives in four TOML files under `rules/aws/`, which are compiled into the program. Changing them means rebuilding.

## services.toml

`services.toml` describes each service. `key` is the name used in a definition. `scope` says where it sits: `outside`, `global`, `regional` or `vpc`, and for VPC services `subnet` is `public` or `private`. `place` says how many copies are drawn and where: `single` for one, `gateway` for one on the VPC's edge, `per_az` for one per Availability Zone, `span` for one spanning the zones, or `writer_reader` for Aurora's writer and readers. `icon`, `shape` and `color` give the icon, `label` the name under it, and `options` the kinds a service can have.

`derived_only` marks the services archwire adds by itself and that cannot be written in a definition (`igw`, `nat`), and `derived_role` names the part such a service plays at the VPC's edge: `entry_gateway` for the Internet Gateway, where traffic from outside comes in, and `egress_nat` for the NAT gateway, where private traffic goes out.

These fields decide more of what happens to a part:

| Field | Meaning | Set on |
|---|---|---|
| `origin = true` | main flows start here; every main flow must be reachable from such a part. A service gets it when the sender of its flow is outside the diagram or when it starts a flow by detecting or recording something itself, as `ec2`, whose instances run code that sends requests. ECS and Lambda are drawn only behind a sender, so they do not have it. Services that only receive, such as `sns`, `stepfunctions`, `ssm`, `athena` and `securitylake`, do not. This is archwire's choice, not an AWS rule | `user`, `cloudwatch`, `guardduty`, `inspector`, `securityhub`, `config`, `cloudtrail`, `eventbridge`, `ec2` |
| `egress = true` | where traffic leaving the VPC goes. A main flow to it is not drawn as a part: archwire adds NAT gateways and the lines through them instead. The part the flow comes from must be in a private subnet | `internet` |
| `kind_hole = true` | a part written without a kind is a hole | `apigateway`, whose kinds are `http` and `rest` |
| `place_rule` | the rule that explains where the part is drawn, instead of the rule its `scope` gives | `cognito` uses `cognito-user-pool-in-one-region`, `ec2` uses `instances-in-private` |
| `vpc_reasons` | for a VPC service, the rules that give the VPC (`vpc`), its zones (`az`) and their default number (`default_azs`) their reasons, taken from the first part in the definition that has them. `az` may be empty | `alb`, `ec2` and `aurora` |
| `nat_rule` | the rule a part gets when its traffic leaves the VPC through a NAT gateway | `lambda` uses `attached-function-internet-through-nat` |

## rules.toml and labels.toml

`rules.toml` holds the reasons. Each rule has an `id`, a `kind`, an optional `source`, and a reason in English (`reason_en`) and Japanese (`reason_ja`). The file also carries `version`, which is printed in every report as `versions.rules`.

`labels.toml` holds the display words, each in English and Japanese: subnet names, line labels such as HTTPS, and the sentences used in notes and holes.

## links.toml

`links.toml` lists the connections a definition may write, one `[[link]]` entry per pair of services and relation. The same list holds for every definition, with a VPC or without. Anything not listed stops with `UNSUPPORTED`. The fields are:

| Field | Meaning |
|---|---|
| `from`, `to` | the service keys at the two ends |
| `rel` | `main` for `->`, `aux` for `~>`, `assoc` for `--` |
| `label` | the key in `labels.toml` of the line's label |
| `to_kind.<kind>` | for each kind of `to`, the `label` used instead of `label`, and the `rule` whose reason is given to `to`. While the kind of `to` is a hole, the line keeps `label` |
| `to_role.<role>` | for each role of the `to` part (`writer`, `reader`), the label used instead of `label` |
| `max_from`, `max_rule` | how many `from` parts one `to` part may have, and the rule that says why. More is `TOO_MANY_ATTACHED` |
| `from_region`, `from_scope`, `from_rule` | the Region the `from` part is drawn in, or the scope (`regional` or `global`) it takes instead of its service's `scope`, and the rule that says why. `from_rule` is written with one of the other two |
| `from_subnet`, `from_subnet_rule`, `from_copies_rule` | the subnet tier of the VPC the `from` part is drawn in, one copy per Availability Zone; the rule that says why it is in the VPC, and the rule that says why there is one copy per zone. The three are written together |

For example, the entry for Cognito and API Gateway, and the one that puts a Lambda function connected to Aurora into the VPC:

```toml
[[link]]
from = "cognito"
to = "apigateway"
rel = "assoc"
label = "verify_token"
to_kind.http = { label = "jwt_authorizer", rule = "http-api-jwt-authorizer" }
to_kind.rest = { label = "cognito_authorizer", rule = "rest-api-cognito-authorizer" }

[[link]]
from = "lambda"
to = "aurora"
rel = "main"
label = "read_write"
to_role.writer = "read_write"
to_role.reader = "read"
from_subnet = "private"
from_subnet_rule = "function-attaches-to-resource-vpc"
from_copies_rule = "attached-function-per-az"
```

A part is placed by `from_subnet` when any of its accepted connections has it, whatever the order of the lines. Otherwise `from_scope` places it, and then its service's `scope`. `acm -- alb` and `waf -- alb` carry `from_scope = "regional"`, so a certificate or web ACL attached to a load balancer is drawn in the Region, while `acm` and `waf` are `global` services. When the connections of one part would place it in two different scopes or Regions, such as `acm -- cloudfront` and `acm -- alb`, `compile` stops with `UNSUPPORTED` at the second of the two lines.

An association that needs no field beyond `label` places the attached part by its service's `scope` alone. `kms -- s3` and `kms -- dynamodb` are such entries: `kms` is a `regional` service, so a key is drawn in the Region and gets the reason of `regional-service-outside-vpc`, as the Glue Data Catalog of `glue -- athena` does. No rule of its own is written for it.

The same file lists, as `[[derived_link]]`, the lines archwire adds between parts by their roles. A role is `private` (a part in a private subnet), `egress_nat` (the NAT gateway), `entry_gateway` (the Internet Gateway), `writer` or `reader` (Aurora's writer and readers). Each entry has `from`, `to`, `rel` and `label`, one per pair of roles: today `private` to `egress_nat` and `egress_nat` to `entry_gateway` as main flows labelled Outbound, and `writer` to `reader` as an auxiliary flow labelled Replication.

## Constraints and defaults

A rule's `kind` is `constraint` or `default`, and the difference is the point of the file.

A constraint states something AWS requires or does, and it must have a `source`, the URL of the AWS documentation page that says so. A constraint without a source is rejected when the rules are loaded. A reason claims no more than its source says; where the reason also says how archwire draws the fact, that part is written as archwire's choice. The fourteen constraints today are the following.

| Rule | What it says |
|---|---|
| `cloudfront-certificate-in-us-east-1` | a Certificate Manager certificate used with CloudFront is requested or imported in us-east-1; archwire draws it outside the Region frame with `us-east-1` under its name |
| `load-balancer-in-vpc` | an Application Load Balancer is created in subnets of a VPC |
| `alb-spans-two-azs` | an Application Load Balancer requires subnets in at least two Availability Zones |
| `internet-facing-alb-in-vpc-with-igw` | an internet-facing Application Load Balancer can only use a VPC that has an Internet Gateway |
| `private-egress-through-nat` | resources in a private subnet can connect to the internet through a NAT device in a public subnet, which replaces their source IPv4 addresses with its own |
| `cognito-user-pool-in-one-region` | an Amazon Cognito user pool is created in one AWS Region; archwire draws it inside the Region frame |
| `alb-certificate-in-load-balancer-region` | certificates in Certificate Manager are Regional resources: to use a certificate with Elastic Load Balancing in more than one Region, one is requested or imported for each Region, and a certificate cannot be copied between Regions; archwire draws the certificate attached to an Application Load Balancer inside the Region frame |
| `regional-web-acl-in-resource-region` | a web ACL and the AWS WAF resources it uses are in the same Region as the resources it protects, and an Application Load Balancer is one of the Regional resources AWS WAF can protect; archwire draws the web ACL attached to an Application Load Balancer inside the Region frame |
| `one-web-acl-per-resource` | one web ACL can be associated with one or more resources, but each resource can be associated with only one web ACL; archwire stops a definition that attaches two WAFs to one CloudFront distribution or one load balancer, and accepts one WAF attached to several |
| `function-attaches-to-resource-vpc` | a Lambda function can be given access to resources in a VPC by attaching it to the VPC through the private subnets that contain the resources; archwire draws a function that connects to a part in a VPC in that VPC's private subnets |
| `attached-function-internet-through-nat` | a function attached to a VPC can reach only resources in that VPC unless the VPC has internet access, and a private subnet can reach the internet for IPv4 through a NAT gateway; archwire adds a NAT gateway in a public subnet for the attached function's internet access over IPv4 |
| `aurora-cluster-in-vpc` | an Amazon Aurora DB cluster can be created only in a VPC |
| `aurora-subnet-group-two-azs` | the DB subnet group chosen for an Aurora DB cluster must cover at least two Availability Zones; archwire draws at least two |
| `ec2-instance-in-subnet` | an Amazon EC2 instance is launched in a subnet, in the Availability Zone associated with that subnet |

A default is archwire's own choice for something the definition did not say. It is not an AWS requirement, and the report marks it `default` so nobody mistakes it for one. A source is optional. The twenty defaults today are `resources-inside-aws-cloud-frame`, `resources-inside-region-frame`, `global-service-outside-region`, `regional-service-outside-vpc`, `nat-device-is-nat-gateway`, `nat-per-az`, `default-two-azs`, `default-two-azs-for-database`, `default-two-azs-for-instances`, `public-and-private-subnet-per-az`, `public-subnet-per-az`, `private-subnet-per-az`, `compute-in-private`, `instances-in-private`, `database-in-private`, `aurora-writer-reader`, `default-region`, `rest-api-cognito-authorizer`, `http-api-jwt-authorizer` and `attached-function-per-az`.

Six defaults cite an AWS page for the facts they rest on. `global-service-outside-region` cites the page where AWS lists CloudFront and AWS WAF for CloudFront among the global services, whose resources are not Region-specific; drawing global services outside the Region frame is archwire's choice, so the rule is a default. `nat-device-is-nat-gateway` cites the page where AWS offers a managed NAT device, the NAT gateway, notes that you can build your own on EC2, a NAT instance, and recommends NAT gateways; drawing a NAT gateway is archwire's choice. `nat-per-az` cites an AWS page for its reasoning. It is still a default, because a single NAT gateway is also valid and `set.nat: single` asks for it. The two authorizer rules cite AWS pages for the facts they state (a REST API uses a Cognito user pool through a `COGNITO_USER_POOLS` authorizer; the JWT authorizer type is supported only for HTTP APIs), and are defaults because drawing `cognito -- apigateway` as that authorizer is archwire's choice. `attached-function-per-az` cites the page that says a function attached to a VPC uses network interfaces Lambda creates for the chosen subnets; drawing the function once in each zone is archwire's choice.

Every part archwire adds gets its reasons from these rules, in the report under `derived` and inside the `.drawio` file as the cell attributes `derived`, `rule`, `kind` and `source`. A NAT gateway gets two in the report: `private-egress-through-nat`, and `nat-device-is-nat-gateway` for drawing that NAT device. Its cell in the `.drawio` file also carries `nat-per-az`, which the report lists against the NAT gateways as a whole (target `nat`). The Internet Gateway gets `internet-facing-alb-in-vpc-with-igw` when the definition has a load balancer, and `private-egress-through-nat` when it is there only for traffic going out. A reason can also be about a part the definition wrote, such as `compute-in-private` for ECS; the report tells the two apart with `added`, and the `.drawio` file marks only the cells archwire added with `added="1"`.

## Loader errors

The rules are checked when archwire starts. The built-in rules are valid, so a user of the released program does not see these. If you edit the files and make a mistake, the program stops with a panic message that lists the problems, each with one of these codes and the rule or file it is about.

| Code | Meaning |
|---|---|
| `SERVICES_PARSE`, `LABELS_PARSE`, `RULES_PARSE`, `LINKS_PARSE` | the file is not valid TOML or does not have the expected fields |
| `RULE_BAD_KIND` | a rule's `kind` is neither `constraint` nor `default` |
| `RULE_WITHOUT_SOURCE` | a `constraint` has no `source` |
| `RULE_DUPLICATE` | two rules have the same `id` |
| `SERVICE_DUPLICATE` | two services have the same `key` |
| `SERVICE_UNKNOWN_RULE` | a service's `place_rule`, `nat_rule` or one of its `vpc_reasons` is not in `rules.toml` |
| `SERVICE_NO_SUBNET` | a VPC service that is not a gateway has no `subnet` |
| `SERVICE_SUBNET_OUTSIDE_VPC` | a service outside the VPC has `subnet` |
| `SERVICE_VPC_REASONS_OUTSIDE_VPC` | a service outside the VPC has `vpc_reasons` |
| `SERVICE_DUPLICATE_ROLE` | two services have the same `derived_role` |
| `LINK_UNKNOWN_SERVICE` | a link's `from` or `to` is not a service in `services.toml` |
| `LINK_BAD_REL` | a link's `rel` is not `main`, `aux` or `assoc` |
| `LINK_UNKNOWN_LABEL` | the `label` of a link, of one of its `to_kind` or `to_role` entries, or of a `[[derived_link]]`, is not in `labels.toml` |
| `LINK_UNKNOWN_KIND` | a `to_kind` key is not one of the kinds (`options`) of the `to` service |
| `LINK_BAD_ROLE` | a `to_role` key is not `writer` or `reader` |
| `LINK_KIND_AND_ROLE` | a link has both `to_kind` and `to_role` |
| `LINK_UNKNOWN_RULE` | a `to_kind` `rule`, `max_rule`, `from_rule`, `from_subnet_rule` or `from_copies_rule` is not in `rules.toml` |
| `LINK_DUPLICATE` | two links have the same `from`, `to` and `rel` |
| `LINK_BAD_LIMIT` | a link has only one of `max_from` and `max_rule`, or `from_region` or `from_scope` without `from_rule`, or `from_rule` without either, or some but not all of `from_subnet`, `from_subnet_rule` and `from_copies_rule` |
| `LINK_BAD_SCOPE` | a link's `from_scope` is `vpc` or `outside` (only `regional` and `global` are accepted; any other word is a `LINKS_PARSE` error), or is written together with `from_subnet` |
| `LINK_SUBNET_WITHOUT_VPC` | a link has `from_subnet`, but its `to` is not a VPC service |
| `DERIVED_LINK_BAD_ROLE` | a `[[derived_link]]` names a role that is not one of the five above |
| `DERIVED_LINK_DUPLICATE` | two `[[derived_link]]` entries have the same pair of roles |

For example, deleting the `source` line of `private-egress-through-nat` and running any `compile` stops with `RULE_WITHOUT_SOURCE` at `private-egress-through-nat`. Changing `rel = "main"` to `rel = "flow"` in the `apigateway` to `lambda` entry of `links.toml` stops with `LINK_BAD_REL` at `apigateway flow lambda`.

A few mistakes are found later, by `compile`, and reported as an error with exit code 1, without a panic. `RULE_MISSING` names a rule the program itself names and no data file refers to. Every such rule is checked at the start of every compile, so a missing one stops every definition, even one that would not use it: without `nat-device-is-nat-gateway`, `- user -> apigateway(http) -> lambda` stops too. `DERIVED_ROLE_MISSING` names a role no service in `services.toml` plays, and `DERIVED_LINK_MISSING` a pair of roles with no `[[derived_link]]`. These two are found only when a definition needs that role or that line: without the `egress_nat` role, `- user -> alb -> ecs` still compiles, and `- user -> alb -> ecs` with `- ecs -> internet` stops.

## Adding or changing a rule

A rule in `rules.toml` is the text and the source. Where it applies, and which part it is recorded against, is decided by the data or by the code. Most rules are named in the data, which the loader checks: a link's `to_kind` `rule`, `max_rule`, `from_rule`, `from_subnet_rule` and `from_copies_rule`, and a service's `place_rule`, `nat_rule` and `vpc_reasons`. The others, such as the frame rules, the rules a service's `scope` gives, the subnet rules and the gateway rules, are named in `crates/archwire-core/src/inference/`, whose list `RULES` in `mod.rs` holds every rule the code names; a definition stops with `RULE_MISSING` if one of them is not in `rules.toml`. So a new rule needs both: an entry in `rules.toml`, and the code or the data that adds it to the report.

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

`tools/check_cases.sh` needs the draw.io desktop command line (`drawio`) and Python with Pillow. It runs `tools/geocheck.py` on every expected diagram, which fails on a line over text, a line over an icon, two lines on top of each other, two icons or two texts overlapping, or an icon or its text outside its frame, and writes a picture of each case to `out/` for you to look at.
