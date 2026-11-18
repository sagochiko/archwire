# Writing a definition

A definition is a text file with up to three sections: `flows`, `set` and `choose`. Only `flows` is required. Only a small part of YAML is accepted, so that `archwire set` can change one place by editing a single line. There are no lists inside lists, no quoting, no multi-line values and no anchors. A `#` starts a comment that runs to the end of the line. Blank lines and Windows line endings are fine. Section headings start at the left edge, and the lines under them are indented with spaces. archwire also reads a definition indented with tabs, but YAML does not allow tabs for indentation, so other YAML tools reject such a file.

```yaml
flows:
  - user -> cloudfront -> alb -> ecs(fargate) -> aurora
  - cloudfront -> s3
  - ecs -> s3
  - ecs -> internet
  - ecs ~> cloudwatch
  - waf -- cloudfront
set:
  azs: 3
  lang: ja
choose:
  route.ecs.s3: nat
```

## flows

Each line is `- ` followed by parts joined by connection symbols. A line may chain several parts, and `a -> b -> c` means `a -> b` and `b -> c`. The same line written twice counts once.

A part is a service name made of `a-z`, `0-9` and `_`, optionally followed by a name after a dot and a kind in parentheses, in that order: `service[.name][(kind)]`, such as `ecs(fargate)`, `s3.web` or `apigateway.public(http)`. Names after a dot are described under [Naming parts](#naming-parts).

There are three connection symbols.

`->` is a main flow, the path traffic takes. `~>` is an auxiliary flow, such as logs going to CloudWatch. It is drawn as a dashed line, and archwire does not work out a network route for it. `--` is an association, for something attached to something else. Write the thing being attached first. An association is drawn as a dashed line without an arrowhead, because nothing flows along it. Which connections are accepted is listed under [Which connections are accepted](#which-connections-are-accepted). `waf`, `acm` and `cognito` are attachments: they may not be the source or the target of `->` or `~>`.

One pair of parts can be connected with one relation only. `ecs -> s3` together with `ecs ~> s3` is the error `CONFLICTING_RELATION`. Direction matters. `ecs -> aurora` and `aurora -> ecs` are two different pairs.

### With a VPC or without

A definition that writes a service that sits in a VPC (`alb`, `ecs` or `aurora`) is drawn with one VPC, as described under [Definitions with a VPC](#definitions-with-a-vpc). A definition without them is drawn without a VPC, as described under [Definitions without a VPC](#definitions-without-a-vpc). Which settings may be written depends on this, see [set](#set).

The other services can be written in either. In a definition with a VPC, a Lambda function that sends a main flow to Aurora is attached to the VPC, as described under [A function attached to the VPC](#a-function-attached-to-the-vpc). Every other part of those services is drawn outside the VPC, as it would be without one.

### Services

| Name | Service | Kinds | Where it is drawn |
|---|---|---|---|
| `user` | Users | | outside the AWS Cloud frame |
| `internet` | The internet. Only as the target of `->`, see below | | not drawn |
| `cloudfront` | Amazon CloudFront | | in the AWS Cloud frame, outside the Region |
| `waf` | AWS WAF. Only as `waf -- cloudfront` or `waf -- alb` | | in the AWS Cloud frame, outside the Region, when attached to `cloudfront`; in the Region, right of the VPC, when attached to `alb` |
| `acm` | AWS Certificate Manager. Only as `acm -- cloudfront` or `acm -- alb` | | in the AWS Cloud frame, outside the Region, when attached to `cloudfront`; in the Region, right of the VPC, when attached to `alb` |
| `alb` | Application Load Balancer | | in the VPC, spanning the public subnets |
| `ecs` | Amazon ECS | `fargate` | in the VPC, in each private subnet |
| `aurora` | Amazon Aurora | | in the VPC, the writer and readers in the private subnets |
| `s3` | Amazon S3 | | in the Region |
| `cloudwatch` | Amazon CloudWatch | | in the Region |
| `apigateway` | Amazon API Gateway | `http`, `rest` | in the Region |
| `lambda` | AWS Lambda | | in the Region, or in each private subnet of the VPC when it sends a main flow to `aurora` |
| `dynamodb` | Amazon DynamoDB | | in the Region |
| `cognito` | Amazon Cognito user pool. Only as `user -> cognito` or `cognito -- apigateway` | | in the Region |
| `guardduty` | Amazon GuardDuty | | in the Region |
| `inspector` | Amazon Inspector | | in the Region |
| `securityhub` | AWS Security Hub | | in the Region |
| `config` | AWS Config | | in the Region |
| `cloudtrail` | AWS CloudTrail | | in the Region |
| `eventbridge` | Amazon EventBridge | | in the Region |
| `sns` | Amazon SNS | | in the Region |
| `stepfunctions` | AWS Step Functions | | in the Region |
| `ssm` | AWS Systems Manager | | in the Region |
| `athena` | Amazon Athena | | in the Region |
| `glue` | AWS Glue Data Catalog. Only as `glue -- athena` | | in the Region |
| `securitylake` | Amazon Security Lake | | in the Region |
| `sqs` | Amazon SQS. Only as the target of `securitylake -> sqs` | | in the Region |

The Internet Gateway and the NAT gateways are added by archwire and cannot be written. Writing `nat` is `UNKNOWN_SERVICE`.

A kind is written once or on every mention of the part. `ecs(fargate)` on one line and plain `ecs` on another are the same part, and both get the kind. Two different kinds for one part are the error `CONFLICTING_OPTION`. `fargate` changes the icon's label to Amazon ECS (Fargate). `http` and `rest` change the label of API Gateway to Amazon API Gateway with HTTP API or REST API on the next line. Leaving out the kind of `apigateway` leaves a hole, see [The kind of API Gateway](#the-kind-of-api-gateway).

A certificate used with CloudFront lives in US East (N. Virginia), whatever Region the diagram shows, so archwire draws `acm` outside the Region frame with `us-east-1` on the line under its name, and records the reason with its AWS source.

A certificate attached to a load balancer, `acm -- alb`, is drawn inside the Region frame, because certificates in Certificate Manager are Regional resources; its reason is `alb-certificate-in-load-balancer-region`. A web ACL attached to a load balancer, `waf -- alb`, is drawn inside the Region frame too, because a web ACL is in the same Region as the resources it protects; its reason is `regional-web-acl-in-resource-region`. One part is drawn in one place, so one `acm` or `waf` cannot be attached to both CloudFront and a load balancer, see [Which connections are accepted](#which-connections-are-accepted).

### Naming parts

The same name is the same part. To draw two parts of one service, give each a name after a dot:

```yaml
flows:
  - user -> cloudfront -> s3.web
  - user -> apigateway(http) -> lambda -> dynamodb
  - lambda -> s3.uploads
set:
  lang: ja
```

`s3.web` and `s3.uploads` are two buckets. The diagram shows the name on the line under the service's name, so the first bucket reads Amazon S3 and then web. The part's id in the report and in the `.drawio` file is the name as written, such as `s3.web`. With `direction: right` added under `set`, this definition and its expected output are the acceptance case `tests/cases/api-names-ja`.

Names work the same way in a definition with a VPC:

```yaml
flows:
  - user -> cloudfront -> alb -> ecs.web -> aurora.web
  - alb -> ecs.admin -> aurora.admin
set:
  lang: ja
```

This draws two services and two databases behind one load balancer, and is the acceptance case `tests/cases/web-named-ja`. A part drawn once in each zone gets one id per zone. For a part without a name the id is the name, a hyphen and the zone, such as `ecs-1a`; for a named part it is the name, `@` and the zone, such as `ecs.web@1a` and `aurora.admin@1c`. `@` cannot be written in a name, so these ids cannot collide with another part's id. A route hole puts a named part in square brackets, such as `route.ecs.[s3.web]` or `route.[lambda.api].s3`; see [choose](#choose). The hole for the kind of a named API Gateway has no brackets: `apigateway.public` gives `kind.apigateway.public`.

A VPC can hold more than one load balancer, each with a name:

```yaml
flows:
  - user -> cloudfront -> alb.web -> ecs.web -> aurora.web
  - user -> alb.admin -> ecs.admin -> aurora.admin
set:
  lang: ja
```

This draws two load balancers, each with its own service and database, and is the acceptance case `tests/cases/web-two-albs-ja`. Where the load balancers go is described under [How the parts are laid out with a VPC](#how-the-parts-are-laid-out-with-a-vpc).

The name after the dot is made of `a-z`, `0-9` and `-`. It may not start or end with `-`, and a part has at most one dot. It is at most 24 characters long; a longer one is the error `NAME_TOO_LONG`, for example `lambda.resize-images-for-the-web` in `- user -> apigateway(http) -> lambda.resize-images-for-the-web` gives `at` `flows[0][2]` and `got` `resize-images-for-the-web`. `--` is always read as the association symbol, even inside a name, so `s3.foo--bar` is read as `s3.foo -- bar`; in `- user -> cloudfront -> s3.foo--bar` that gives `UNKNOWN_SERVICE` for `bar` at `flows[0][3]`. Write the kind after the name: `apigateway.public(http)`.

## Which connections are accepted

The accepted connections are listed pair by pair in `rules/aws/links.toml`, and the same list holds for every definition. Anything not in the list is `UNSUPPORTED`, with the line in `at` and the connection in `got`.

| From | Symbol | To | Line label |
|---|---|---|---|
| `user` | `->` | `cloudfront` | HTTPS |
| `user` | `->` | `apigateway` | HTTPS |
| `user` | `->` | `alb` | HTTPS |
| `user` | `->` | `cognito` | Sign-in |
| `user` | `->` | `s3` | Upload/Download |
| `user` | `->` | `athena` | Query |
| `cloudfront` | `->` | `s3` | Static files |
| `cloudfront` | `->` | `apigateway` | HTTPS |
| `cloudfront` | `->` | `alb` | HTTPS |
| `cloudfront` | `~>` | `s3` | Logs and metrics |
| `cloudfront` | `~>` | `cloudwatch` | Logs and metrics |
| `waf` | `--` | `cloudfront` | Web ACL |
| `acm` | `--` | `cloudfront` | Certificate |
| `acm` | `--` | `alb` | Certificate |
| `waf` | `--` | `alb` | Web ACL |
| `apigateway` | `->` | `lambda` | Invoke |
| `apigateway` | `~>` | `cloudwatch` | Logs and metrics |
| `cognito` | `--` | `apigateway` | JWT authorizer for `http`, Cognito authorizer for `rest`, Verify tokens while the kind is undecided |
| `lambda` | `->` | `dynamodb` | Read/Write |
| `lambda` | `->` | `s3` | Read/Write |
| `lambda` | `->` | `aurora` | Read/Write to the writer, Read to a reader |
| `lambda` | `->` | `internet` | Outbound |
| `lambda` | `~>` | `cloudwatch` | Logs and metrics |
| `alb` | `->` | `ecs` | HTTP |
| `alb` | `~>` | `s3` | Logs and metrics |
| `alb` | `~>` | `cloudwatch` | Logs and metrics |
| `ecs` | `->` | `aurora` | Read/Write to the writer, Read to a reader |
| `ecs` | `->` | `s3` | Read/Write |
| `ecs` | `->` | `dynamodb` | Read/Write |
| `ecs` | `->` | `cloudwatch` | Logs and metrics |
| `ecs` | `->` | `internet` | Outbound |
| `ecs` | `~>` | `s3` | Logs and metrics |
| `ecs` | `~>` | `cloudwatch` | Logs and metrics |
| `aurora` | `->` | `ecs` | HTTP |
| `aurora` | `->` | `s3` | Read/Write |
| `aurora` | `->` | `cloudwatch` | Logs and metrics |
| `aurora` | `->` | `internet` | Outbound |
| `aurora` | `~>` | `s3` | Logs and metrics |
| `aurora` | `~>` | `cloudwatch` | Logs and metrics |
| `guardduty` | `->` | `eventbridge` | Findings |
| `guardduty` | `->` | `securityhub` | Findings |
| `inspector` | `->` | `eventbridge` | Findings |
| `inspector` | `->` | `securityhub` | Findings |
| `securityhub` | `->` | `eventbridge` | Findings |
| `config` | `->` | `eventbridge` | Compliance changes |
| `config` | `->` | `securityhub` | Findings |
| `config` | `->` | `sns` | Notifications |
| `config` | `->` | `ssm` | Remediation |
| `cloudtrail` | `->` | `s3` | Log files |
| `cloudtrail` | `->` | `cloudwatch` | Logs |
| `cloudtrail` | `->` | `eventbridge` | API calls |
| `cloudwatch` | `->` | `sns` | Alarms |
| `eventbridge` | `->` | `sns` | Rule |
| `eventbridge` | `->` | `stepfunctions` | Rule |
| `eventbridge` | `->` | `ssm` | Rule |
| `stepfunctions` | `->` | `sns` | Publish |
| `stepfunctions` | `->` | `lambda` | Invoke |
| `stepfunctions` | `->` | `ssm` | Call |
| `athena` | `->` | `s3` | Query |
| `athena` | `->` | `securitylake` | Query |
| `glue` | `--` | `athena` | Table definitions |
| `cloudtrail` | `->` | `securitylake` | Logs |
| `securityhub` | `->` | `securitylake` | Findings |
| `securitylake` | `->` | `s3` | Data lake |
| `securitylake` | `->` | `sqs` | New objects |

A pair means the services, whatever the parts are named: `lambda.resize -> s3.uploads` is the pair `lambda -> s3`.

`config -> securityhub` is drawn as one line. AWS Config sends its rule evaluations to Security Hub through EventBridge, but that is a link between the two services, not an EventBridge rule the definition writes.

`user -> cognito` is the sign-in to a user pool, drawn from the users although the tokens go to their app. `user -> s3` is an upload or download straight to a bucket, with a presigned URL or with temporary credentials from a Cognito identity pool; the identity pool is not drawn. The users of `user -> athena` are the people who run the queries. Security Lake keeps its data lake in S3 buckets in the account, so `securitylake -> s3` is drawn as a main flow. GuardDuty, Inspector and Config findings reach Security Lake through Security Hub, so there is no line from them to `securitylake`; write `securityhub -> securitylake`. The data access subscribers are drawn up to the SQS queue Security Lake creates for them; the subscriber that polls it is not drawn.

Besides the list, archwire checks the following, and stops with `UNSUPPORTED` when one does not hold:

- There is at least one main flow (`at` `flows`), and every main flow can be reached along main flows from a starting point (`at` the line that cannot be reached). The starting points are `user` and the services that start a flow by themselves: `guardduty`, `inspector`, `securityhub`, `config`, `cloudtrail`, `eventbridge` and `cloudwatch`. `sns`, `stepfunctions`, `ssm`, `athena`, `securitylake` and the other services only receive, so a definition cannot start with them.
- The source of an auxiliary flow and the target of an association are on a main flow reached from a starting point.
- A main flow to `internet` comes from a part in a private subnet of the VPC: ECS, Aurora, or a function attached to the VPC.

One CloudFront distribution or one load balancer takes one WAF. With `- user -> cloudfront -> s3`, `- waf.front -- cloudfront` and `- waf.extra -- cloudfront`, the third line stops with `TOO_MANY_ATTACHED`, `got` `waf.extra`, and `rule` `one-web-acl-per-resource`; the reason is in [docs/rules.md](rules.md). `- user -> alb -> ecs`, `- waf.a -- alb` and `- waf.b -- alb` stop the same way, at `flows[2]` with `got` `waf.b`. One WAF attached to two distributions is accepted, and so is one WAF attached to two load balancers, such as `- waf -- alb.web` and `- waf -- alb.admin`.

A certificate or web ACL attached to CloudFront is drawn outside the Region frame, and one attached to a load balancer inside it, so one part cannot be attached to both. With `- user -> cloudfront -> alb -> ecs`, `- acm -- cloudfront` and `- acm -- alb`, the third line stops with `UNSUPPORTED`, `at` `flows[2]`, `got` `acm`, and the message ``"`acm` is attached to both `cloudfront` and `alb`, which need it in different places; write them as two parts, such as `acm.cloudfront -- cloudfront` and `acm.alb -- alb`"``. `waf` attached to both stops the same way. The message names the two parts after what they are attached to; for a named part it uses the whole name with the dot written as `-`, so `acm -- cloudfront.web` with `acm -- alb.web` suggests `acm.cloudfront-web -- cloudfront.web` and `acm.alb-web -- alb.web`. Any two names will do.

For example, each of these stops with `UNSUPPORTED`:

| Definition | `at` | Message |
|---|---|---|
| `- user -> dynamodb` | `flows[0]` | ``"`user -> dynamodb` is not a supported connection"`` |
| `- user -> cloudfront -> lambda` | `flows[0]` | ``"`cloudfront -> lambda` is not a supported connection"`` |
| `- user -> cloudfront -> ecs` | `flows[0]` | ``"`cloudfront -> ecs` is not a supported connection"`` |
| `- user -> alb -> ecs -> aurora` and `- alb -> aurora` | `flows[1]` | ``"`alb -> aurora` is not a supported connection"`` |
| `- user -> cloudfront -> alb -> ecs` and `- cloudfront -> cloudwatch` | `flows[1]` | ``"`cloudfront -> cloudwatch` is not a supported connection"`` |
| `- user -> cloudfront -> alb -> ecs` and `- alb -- acm` | `flows[1]` | ``"`alb -- acm` is not a supported connection"`` |
| `- stepfunctions -> sns` alone | `flows[0]` | ``"`stepfunctions -> sns` is not reached by main flows (->) from `cloudtrail`, `cloudwatch`, `config`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`"`` |
| `- lambda -> dynamodb` alone | `flows[0]` | ``"`lambda -> dynamodb` is not reached by main flows (->) from `cloudtrail`, `cloudwatch`, `config`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`"`` |
| `- user -> alb -> ecs` and `- aurora -> s3` | `flows[1]` | ``"`aurora -> s3` is not reached by main flows (->) from `cloudtrail`, `cloudwatch`, `config`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`"`` |
| `- user -> cloudfront -> s3` and `- cognito -- apigateway` | `flows[1]` | ``"`cognito -- apigateway` needs `apigateway` on a main flow (->) from `cloudtrail`, `cloudwatch`, `config`, `eventbridge`, `guardduty`, `inspector`, `securityhub`, `user`"`` |
| `- user -> apigateway(http) -> lambda -> internet` | `flows[0]` | ``"`lambda -> internet` needs `lambda` in a VPC; connect it to a part in a private subnet"`` |

`alb -- waf` stops the same way as `alb -- acm`: write the attached part first. So do `user -> aurora`, `alb -> s3`, `user ~> alb`, `internet -> alb`, `ecs -> user` and `waf -> alb` in a definition with a VPC. The auxiliary flows `cloudfront ~> s3` and `cloudfront ~> cloudwatch` are accepted in every definition, and so is `- user -> alb` alone.

A main flow from a part in a private subnet to a part in the Region outside the VPC leaves a hole for its route, see [choose](#choose). With the list above that is `ecs` to `s3`, `dynamodb` or `cloudwatch`, `aurora` to `s3` or `cloudwatch`, and a function attached to the VPC to `s3` or `dynamodb`.

## Definitions with a VPC

archwire adds what the path needs and lays the parts out as described under [How the parts are laid out with a VPC](#how-the-parts-are-laid-out-with-a-vpc). Public subnets are drawn when a part goes in one: the load balancer, or the NAT gateways. Private subnets are drawn when a part goes in one: ECS, Aurora, or a function attached to the VPC. The Internet Gateway is added for an internet-facing load balancer, and for traffic going out to the internet. A note in the report says that auxiliary flows are not given a network route.

`internet` is never drawn. `ecs -> internet` says the ECS tasks make outbound requests; `aurora -> internet` and `lambda -> internet` for a function attached to the VPC say the same. archwire adds a NAT gateway in each public subnet, one per Availability Zone unless `nat` is `single`, and the lines from each copy of the part to the NAT gateway in its zone and on to the Internet Gateway, all labelled "Outbound". Parts that go out share the same NAT gateways.

### A function attached to the VPC

A Lambda function that sends a main flow to Aurora is attached to the VPC and drawn in the private subnet of each Availability Zone, one copy per zone, like ECS. Functions are never drawn in a public subnet. A function without a main flow to Aurora stays outside the VPC, even in a definition with one. The README's `mix.yaml` shows the first:

```yaml
flows:
  - user -> apigateway(http) -> lambda -> aurora
  - lambda -> s3
  - lambda ~> cloudwatch
set:
  lang: ja
```

`compile` exits with 2, with the hole `route.lambda.s3`. API Gateway, S3 and CloudWatch are in the Region outside the VPC. Each copy of the function has a line to the writer, labelled Read/Write, and a line to the reader, labelled Read. The function gets two reasons: `function-attaches-to-resource-vpc`, a constraint, for being in the private subnets, and `attached-function-per-az`, a default, for being drawn once per zone. The VPC and its zones get their reasons from the first part in the definition that has them, the load balancer or Aurora. Here it is Aurora, so the VPC gets `aurora-cluster-in-vpc`, the zones `aurora-subnet-group-two-azs`, and their number `default-two-azs-for-database`. When the load balancer comes first, they get `load-balancer-in-vpc`, `alb-spans-two-azs` and `default-two-azs`. With `set: direction: right` this definition is the acceptance case `tests/cases/mix-vpc-ja-right`, and without the direction `tests/cases/mix-vpc-ja`.

With `lambda -> internet` added, the function gets `attached-function-internet-through-nat` too, and its traffic goes out through NAT gateways. `lambda -> internet` for a function outside the VPC stops, as in the table above.

A web architecture and an API can stand side by side:

```yaml
flows:
  - user -> alb -> ecs
  - user -> apigateway(http) -> lambda
```

`compile` exits with 0. The function is not connected to Aurora, so it is drawn outside the VPC. In the acceptance case `tests/cases/mix-web-ja`, `lambda.api` reaches Aurora and is drawn in the VPC next to ECS, while `lambda.report` reaches only DynamoDB and is drawn outside the VPC. `tests/cases/mix-beside-ja` has the same two kinds of function without a load balancer.

archwire does not draw RDS Proxy: the line from a function to Aurora goes to Aurora directly.

### How the parts are laid out with a VPC

Outside the VPC, the parts are laid out as in a definition without a VPC (see [How the parts are laid out](#how-the-parts-are-laid-out)), in the direction `set.direction` gives: from top to bottom, or from left to right with `direction: right`. Users and CloudFront come first, then the Region frame, which holds the VPC and the Regional services. The VPC takes a place in its rank like one large part, and comes first in that rank. So S3 in `cloudfront -> s3` goes to the right of the VPC in a top-to-bottom diagram, and below it in a left-to-right one. A Regional part that flows into the VPC, such as API Gateway in `apigateway -> lambda` with the function attached, comes in a rank before the VPC. A part that only parts inside the VPC connect to, such as CloudWatch in `ecs ~> cloudwatch`, goes to the right of the VPC in both directions.

The inside of the VPC is the same in both directions. The Availability Zones stand side by side, and in each zone the public subnet is above the private one. Inside the subnets the main flows run from top to bottom, so in `alb -> ecs -> aurora` ECS is above Aurora. ECS and a function attached to the VPC get a copy in each zone. Aurora's writer is in the first zone and its readers in the zones after it.

The Internet Gateway sits inside the VPC on the side where the traffic comes in: the top side in a top-to-bottom diagram, the left side in a left-to-right one. A part that sends a main flow to the Internet Gateway, such as CloudFront, is lined up with it, so CloudFront, the Internet Gateway and the load balancer stand on one straight line, and a line from CloudFront to S3 bends on its way.

The load balancer spans the zones. With two zones it sits in the public tier on the border between them, level with the NAT gateways in a top-to-bottom diagram. With three zones it sits above the zone frames. Two or more load balancers stand in one row above the zone frames, with two zones or three and in both directions, in the order the definition first mentions them. The line from the Internet Gateway goes straight to the first one; the lines to the others leave the Internet Gateway at separate points on the side facing the flow (the bottom side in a top-to-bottom diagram, the right side in a left-to-right one) and enter each load balancer from the top.

Certificate Manager and WAF attached to a load balancer stand one under the other to the right of the VPC, inside the Region frame, in both directions. Their dashed lines leave the load balancer from its right side, or from its bottom side when another part stands to its right in the same row. Their labels sit on the stretch of the line outside the VPC, where there is room, so they are not read as labels of the lines inside it. In a VPC without private subnets, the line of a load balancer with another load balancer to its right still leaves from the right side and crosses that load balancer, and a note names the line (see the limits in the [README](../README.md#what-this-version-does-not-do)). This definition and its expected output are the acceptance case `tests/cases/web-alb-attach-en`:

```yaml
flows:
  - user -> cloudfront -> alb -> ecs(fargate) -> aurora
  - cloudfront -> s3
  - ecs -> internet
  - ecs ~> cloudwatch
  - acm.cdn -- cloudfront
  - acm.alb -- alb
  - waf -- alb
```

A line's label says which zones it stands for. With two zones, archwire draws the lines between parts in different zones. With three zones it draws no line between copies of two different services in different zones: the line inside the target's zone stands for the lines from every zone into the same part, and a note in the report says so. Two kinds of line still cross zones: the replication line from Aurora's writer to each reader, and a line from another zone when the target's zone has no sender, described below. A label ends with "(all AZs)" when the sender has a copy in every zone, and names the zones otherwise. For example, with `azs: 3` and `readers: 1`, Aurora has a copy in two zones only, and `aurora ~> cloudwatch` is labelled "Logs and metrics (from 1a, 1c)". When a target in one zone has no sender in the same zone, the line comes from a sender in another zone, and a note names it. With the same settings, `aurora -> ecs` gives the note "In ap-northeast-1d, ecs-1d has no aurora in its own zone, so the line from aurora-1a in another zone is drawn."

Some diagrams are correct but harder to read than they could be. In a top-to-bottom diagram, WAF and Certificate Manager attached to CloudFront stand one under the other to its right, which makes the row of global services tall. An auxiliary line from the load balancer to a part right of the VPC, such as `alb ~> s3`, can go round the far side of that part and enter it from the right. With two zones, each NAT gateway sits to the right of the ECS copy in its zone, so in the first zone the line from the load balancer to ECS crosses the line from ECS to the NAT gateway. In a left-to-right diagram, S3 fed by CloudFront sits below the VPC, so the line to it is long.

### Services behind one load balancer

Every service behind the load balancer is copied into each zone, and the load balancer sends a line to each copy. The counts below are for this shape, with more `- alb -> ecs.sNN` lines added one at a time:

```yaml
flows:
  - user -> cloudfront -> alb -> ecs.s01 -> aurora
  - alb -> ecs.s02
  - alb -> ecs.s03
```

With two zones up to six services, and with three zones up to three, are drawn without a note naming a line; notes that explain the drawing, such as the one on auxiliary flows or on three zones, may still be there. With seven services in two zones, or four in three, the diagram is still drawn, with notes naming the lines that could not avoid every icon, label and other line. From 51 services with two zones, and from 34 with three, the layout stops with `LAYOUT_FAILED`. The counts are the same in both directions. Writing fewer services avoids this; it is a limit of this layout, not of AWS.

## Definitions without a VPC

### Settings without a VPC

Only `direction`, `lang` and `region` can be set. Any other key, including `azs`, `nat` and `readers`, is `UNKNOWN_SETTING` with `allowed` `direction`, `lang` and `region`.

### How the parts are laid out

The main flows run from top to bottom, or from left to right with `direction: right` (see [set](#set)). Below, a rank is a row in a top-to-bottom diagram and a column in a left-to-right one, and across means along the rank: from left to right along a row, or from top to bottom down a column. Users come first, then a rank for global services (CloudFront, WAF and Certificate Manager) inside the AWS Cloud frame, then the Region frame. archwire works out the layout from the definition alone; the definition cannot move a part.

Inside the Region, each part on a main flow goes in the rank one past the furthest part that flows into it within the Region; a part with nothing flowing into it within the Region goes in the first rank. Within a rank, parts are placed across in the order they first appear in the definition. A rank after the first is instead put in the order of the parts in the rank before that flow into its parts (by the middle of the first and the last of them, keeping the definition's order between parts level so), when that leaves fewer pairs of crossing lines from the rank before and no part of the rank has a main flow from outside the Region. So in `eventbridge.a -> stepfunctions.a1`, `eventbridge.a -> stepfunctions.a2`, `eventbridge.b -> stepfunctions.b`, `stepfunctions.b -> sns.z`, `stepfunctions.a1 -> sns.x` and `stepfunctions.a2 -> sns.y`, the topics come in the order x, y, z, each across from the state machine that publishes to it. A part is centred on the parts that flow into it: its place across is halfway between the first and the last of them, rounded down to a whole pixel, unless that would bring it too close to the part before it. A part whose main flows all go to parts that only it flows into is moved past the parts that the parts before it in its rank alone flow into, when all of those parts are in the next rank and, in the next rank, the parts that only one part of this rank flows into come in the order of those parts. So in `cloudtrail -> cloudwatch`, `cloudtrail -> s3` and `config -> ssm`, Config comes past S3, above Systems Manager, instead of above S3. When these conditions do not hold, or the moves do not settle, no part is moved this way, and lines from two parts can still meet. A part that sends main flows to two or more parts is then moved, where there is room, to halfway between the first and the last of them. Ranks move apart when the names or line labels between them are wide.

Parts that carry no main flow, such as Cognito and CloudWatch, go one step across past the parts on main flows (to their right in a top-to-bottom diagram, below them in a left-to-right one), in the rank of the part they are attached to (for an association) or of the first part that sends to them (for an auxiliary flow). That rank is tried first; when another such part or its name is already there, the part moves to the next free rank. WAF and Certificate Manager go one step across from the CloudFront they are attached to, one after another in the direction of the main flows; one attached to two distributions goes past everything else in the global rank. Adding or removing such a part does not move the parts on main flows within the Region.

When several parts of the same service send the same auxiliary flow (`~>`) or association (`--`), with the same label, to the same part, the diagram draws one line for them. Main flows (`->`) are never merged; each is drawn. The merged line is labelled with the count, such as "Logs and metrics (3 sources)" or 「ログ・メトリクス（3か所から）」. The report's `notes` names the parts, such as `"The line from lambda.search to cloudwatch stands for the lines from lambda.orders, lambda.users, lambda.search."`. The acceptance case `tests/cases/api-merged-ja` shows this with three Lambda functions sending logs to one CloudWatch.

When no part belongs to a Region, as in `- user -> cloudfront` with `- waf -- cloudfront` (the acceptance case `tests/cases/edge-waf-en`, drawn with `direction: right`), no Region frame is drawn, and the report gives no reason for one.

A diagram without a VPC gets these reasons: the AWS Cloud frame, the Region frame and the Region chosen when `set.region` is left out, and the place of each part the definition wrote (global services outside the Region frame, Regional services inside it, Cognito inside it because a user pool is created in one Region, and Certificate Manager outside it because a certificate used with CloudFront is in us-east-1). With `apigateway(http)` or `apigateway(rest)` and a `cognito -- apigateway` line, the API Gateway also gets the reason why the association is drawn as that authorizer.

If archwire cannot lay out a definition, with or without a VPC, without parts or names overlapping or leaving their frame, or without a spot of its own for each line on a side of a part, it writes no diagram and reports `LAYOUT_FAILED`.

## The kind of API Gateway

When `apigateway` is written without a kind, archwire does not pick one. It reports a hole and still writes the diagram, and the exit code is 2. Save this as `hole.yaml`:

```yaml
flows:
  - user -> apigateway -> lambda
  - cognito -- apigateway
```

The report contains:

```json
"holes": [
  {
    "hole": "kind.apigateway",
    "at": "flows[0][1]",
    "need": "Kind of apigateway",
    "choices": [
      "http",
      "rest"
    ]
  }
]
```

In the diagram the API Gateway reads Amazon API Gateway with "type undecided" (「種類未定」 in Japanese) on the next line, and the line from Cognito is labelled "Verify tokens". With `set:` and `direction: right` added, this definition and its expected output are the acceptance case `tests/cases/hole-apigateway`. A named part has its own hole: `apigateway.public` gives `kind.apigateway.public`.

Answer the hole in one of two ways. Write the kind in the definition, as in `apigateway(http)`, or answer in `choose`:

```sh
archwire set hole.yaml choose.kind.apigateway http
```

Compile again and the exit code is 0. Do not do both: when the definition already says `apigateway(http)`, there is no hole, and `choose` with `kind.apigateway` is the error `UNKNOWN_HOLE`, even with the same value. An answer that is not `http` or `rest` is `BAD_CHOICE`, with `allowed` `http` and `rest`.

## set

```yaml
set:
  azs: 3
```

The table is for definitions with a VPC. A definition without a VPC accepts only `direction`, `lang` and `region`, see [Settings without a VPC](#settings-without-a-vpc).

| Key | Values | Default |
|---|---|---|
| `azs` | `2` or `3`, the number of Availability Zones | `2` |
| `region` | `ap-northeast-1`. No other Region in this version | `ap-northeast-1` |
| `readers` | the number of Aurora readers, from `1` up to `azs` minus 1 | `azs` minus 1 |
| `nat` | `per-az` (a NAT gateway in each zone) or `single` (one, in the first zone) | `per-az` |
| `lang` | `en` or `ja`, the language of labels, reasons and notes. Service names stay in English | `en` |
| `direction` | `down` (the main flows run from top to bottom) or `right` (from left to right) | `down` |

When `azs`, `region`, `readers` or `nat` is left out, archwire uses the default and records it in the report as a `default` reason where it applies. `direction` and `lang` change only how the diagram is drawn and get no reason. A value outside the list is `BAD_SETTING`; `direction: left` gives `allowed` `down` and `right`. A key that is not in the table is `UNKNOWN_SETTING`.

## choose

```yaml
choose:
  route.ecs.s3: endpoint
```

`choose` answers holes. There are two kinds of hole: the route of a main flow out of the VPC's private subnets, described here, and [the kind of API Gateway](#the-kind-of-api-gateway).

For the route, the key is the hole id from the report, `route.<from>.<to>`, and the value is `endpoint` (the traffic goes through a VPC endpoint; archwire draws no endpoint icon, and the line's label adds "(via VPC endpoint)") or `nat` (it goes out through NAT gateways, which archwire adds). A named part is put in square brackets: `lambda.api -> s3` gives `route.[lambda.api].s3`, and `ecs -> s3.web` gives `route.ecs.[s3.web]`. The id depends on the part names only, so adding or moving lines does not change it. `archwire set <definition> choose.route.ecs.s3 endpoint` writes the entry for you. Quote a place with brackets, as in `archwire set mix.yaml 'choose.route.[lambda.api].s3' endpoint`: shells such as zsh read unquoted brackets as a file pattern and stop with `no matches found` before archwire runs, leaving the file as it was.

### Answering with nat

```yaml
flows:
  - user -> alb -> ecs -> s3
choose:
  route.ecs.s3: nat
```

`compile` exits with 0. archwire adds a NAT gateway in each public subnet, with a line from each ECS task to its NAT gateway and on to the Internet Gateway, all labelled "Outbound". The line to S3 is labelled "Read/Write (all AZs)" on one line. The report gives each NAT gateway two reasons:

```json
{
  "target": "nat-1a",
  "added": true,
  "kind": "constraint",
  "rule": "private-egress-through-nat",
  "reason": "Resources in a private subnet can connect to the internet through a NAT device in a public subnet, which replaces their source IPv4 addresses with its own.",
  "source": "https://docs.aws.amazon.com/vpc/latest/userguide/vpc-nat.html"
},
{
  "target": "nat-1a",
  "added": true,
  "kind": "default",
  "rule": "nat-device-is-nat-gateway",
  "reason": "You can use the managed NAT device AWS offers, the NAT gateway, or build your own NAT device on EC2, a NAT instance; AWS recommends NAT gateways for better availability and bandwidth and less administration. archwire draws a NAT gateway.",
  "source": "https://docs.aws.amazon.com/vpc/latest/userguide/vpc-nat.html"
},
```

The same two entries follow for `nat-1c`, and then `nat-per-az`, the default that puts one NAT gateway in each zone. `notes` is empty.

### Answering with endpoint

```yaml
flows:
  - user -> alb -> ecs -> s3
set:
  lang: ja
choose:
  route.ecs.s3: endpoint
```

`compile` exits with 0. archwire adds no NAT gateway and no endpoint icon. The line to S3 is labelled with one part per line: 「読み書き」, 「（VPC エンドポイント経由）」 and 「（全 AZ から）」. The report's `notes` says that endpoint icons are not drawn:

```json
"notes": [
  "VPC エンドポイントのアイコンは描かない。"
]
```

This definition and its expected output are the acceptance case `tests/cases/endpoint-ja`.

## Editing with set

```sh
archwire set <definition> <place> <value>
```

The place is `flows[i][j]`, `set.<key>` or `choose.<hole id>`. For `set` and `choose`, the section or the key is added if it is missing, and an existing value is replaced in place. An added line copies the indentation of the other lines in its section, or of the file's first indented line when the section is new, so a file keeps its own indentation. For `flows[i][j]`, the j-th part of the i-th flow line (both counting from 0, and only flow lines are counted) is replaced; the value may carry a name and a kind, as in `ecs(fargate)` or `apigateway.public(http)`, and must read as one part, so that the edit cannot change the shape of the line; `s3.foo--bar` is `BAD_VALUE`. The length of a name after the dot is not checked by `set`; `compile` reports `NAME_TOO_LONG`. The value must be one non-empty line without `#`. `set` does not check that the result is a valid definition. `compile` does. Quote a place that holds square brackets, such as `'choose.route.[lambda.api].s3'`.

## Errors

When a definition has errors, `compile` writes no diagram, removes a diagram left at the `-o` path by an earlier compile, prints the report with an `errors` list, and exits with 1. Several errors can be reported at once. Each example below shows the report's `code`, `at` and `got`. The last column says whether `archwire set <definition> <at> <value>` can fix the error at the place the report gives, and what to do otherwise. The examples for `BAD_CHOICE` and `UNKNOWN_HOLE` assume the definition contains `ecs -> s3`, so that the hole `route.ecs.s3` exists.

| Code | Meaning | Example and result | Fixed with `set` at `at`? |
|---|---|---|---|
| `EMPTY_DEFINITION` | there are no flows | an empty file: `at` is `flows` | No. Write a `flows:` section with at least one line in the file |
| `BAD_SYNTAX` | a line is not in the accepted form | `- user => alb` under `flows:` gives `at` `flows[0]` | No. `at` is a whole line (`flows[i]`, `set[i]`, `choose[i]` or `line n`); edit that line in the file |
| `UNKNOWN_SECTION` | a heading other than `flows`, `set`, `choose` | `extra:` as a heading gives `at` `line 3`, `allowed` lists the three sections | No. Rename or remove the heading in the file |
| `NAME_TOO_LONG` | a name after the dot is longer than 24 characters | `- user -> apigateway(http) -> lambda.resize-images-for-the-web` gives `at` `flows[0][2]`, `got` `resize-images-for-the-web` | Yes, with a shorter name, such as `archwire set api.yaml 'flows[0][2]' lambda.resize` |
| `SELF_LINK` | a part connected to itself | `- ecs -> ecs` gives `at` `flows[0]` | No. `at` is the whole flow line; change one of its parts with `flows[i][j]` or edit the line |
| `CONFLICTING_OPTION` | one part written with two kinds | `- alb -> ecs(fargate)` and `- ecs(ec2) -> aurora` give `at` `flows[1][0]`, `allowed` is `fargate` | Yes, with a value from `allowed`, such as `archwire set web.yaml 'flows[1][0]' 'ecs(fargate)'` |
| `CONFLICTING_RELATION` | one pair of parts with two relations | `- ecs -> s3` and then `- ecs ~> s3` give `at` `flows[1]` | No. `set` changes parts, not symbols; remove one of the two lines in the file |
| `DUPLICATE_KEY` | a key written twice in `set` or `choose` | `azs: 2` and `azs: 3` give `at` `set.azs` | No. `set` would change only the first of the two; delete one line in the file |
| `UNKNOWN_SERVICE` | a name that is not a writable service | `- user -> cloudfrnt -> alb -> ecs` gives `at` `flows[0][1]`, `allowed` lists the writable names | Yes, with a name from `allowed` |
| `UNKNOWN_OPTION` | a kind the service does not have | `ecs(lambda)` gives `allowed` `fargate` | Yes, with the part and a kind from `allowed`, or the bare name |
| `BAD_SETTING` | a value outside the list | `azs: 4` gives `at` `set.azs`, `allowed` is `2` and `3` | Yes, with a value from `allowed` |
| `UNKNOWN_SETTING` | a key that is not a setting | `colour: red` gives `at` `set.colour`, `allowed` lists the six keys; without a VPC, `azs: 3` gives `at` `set.azs`, `allowed` `direction`, `lang` and `region` | No. `set` can change only the value; delete the line in the file |
| `BAD_CHOICE` | an answer that is not one of the hole's choices | `route.ecs.s3: tunnel` gives `at` `choose.route.ecs.s3`, `allowed` `endpoint` and `nat`; `kind.apigateway: websocket` gives `allowed` `http` and `rest` | Yes, with a value from `allowed` |
| `UNKNOWN_HOLE` | an answer for a hole that does not exist | `route.ecs.rds: nat` gives `at` `choose.route.ecs.rds`; so does `kind.apigateway: http` when the definition already says `apigateway(http)` | No. `set` can change only the value; delete the line in the file, and answer a hole from `allowed` with `archwire set <definition> choose.<hole> <value>` |
| `UNSUPPORTED` | a connection or shape this version does not draw | `- user -> dynamodb` gives `at` `flows[0]`; `- acm -- cloudfront` and `- acm -- alb` after `- user -> cloudfront -> alb -> ecs` give `at` `flows[2]`, `got` `acm`; more under [Which connections are accepted](#which-connections-are-accepted) | Sometimes. `at` is a flow line (`flows[i]`) or `flows`; change a part with `flows[i][j]`, or edit or remove the line in the file |
| `TOO_MANY_ATTACHED` | more parts attached to one part than AWS allows | `- user -> cloudfront -> s3`, `- waf.front -- cloudfront` and `- waf.extra -- cloudfront` give `at` `flows[2]`, `got` `waf.extra`, `rule` `one-web-acl-per-resource` | No. Remove the extra line in the file |
| `LAYOUT_FAILED` | archwire could not lay out the definition; the message names the parts or lines involved | 34 named services behind one load balancer with `azs: 3`, see [Services behind one load balancer](#services-behind-one-load-balancer) | No. Write fewer parts, or report the definition. Fewer parts is a way around a limit of the layout, not a limit of AWS |
| `BAD_PATH` | the place given to `set` does not exist | `archwire set web.yaml 'flows[9][0]' x`, on a file with fewer flow lines | Run `set` again with an existing place |
| `BAD_VALUE` | the value given to `set` is empty or has a line break or `#`, or a value for `flows[i][j]` does not read as one part | `archwire set web.yaml set.azs ""`; `archwire set api.yaml 'flows[0][2]' 's3.foo--bar'` | Run `set` again with one non-empty line, or one part |
| `CANNOT_READ` | `compile` or `set` could not read the definition file | `archwire compile missing.yaml -o out.drawio` | No. Check the path and the file's permissions |
| `CANNOT_WRITE` | `compile` could not write the diagram or remove an old one, or `set` could not write the definition | `archwire compile web.yaml -o no/such/dir/web.drawio`. The report is still printed, and the exit code is 1 | No. Check the path and the permissions |

The reports that `set` prints (for `BAD_PATH`, `BAD_VALUE`, `CANNOT_READ` and `CANNOT_WRITE`) have both versions filled in and empty `derived`, `holes` and `notes`. When `set` succeeds it prints nothing.

### The order of errors

The errors come in the order of the checks. Within a check on the flows, they come in the order of the lines. Keys that are not settings, and the answers in `choose`, are checked in the alphabetical order of their keys, whatever order they are written in: `choose` with `z.hole` and then `a.hole` gives `UNKNOWN_HOLE` at `choose.a.hole` first. A file that cannot be read as a definition gives only its reading errors (`EMPTY_DEFINITION`, `BAD_SYNTAX`, `UNKNOWN_SECTION`, `NAME_TOO_LONG`, `SELF_LINK`, `CONFLICTING_OPTION`, `CONFLICTING_RELATION`, `DUPLICATE_KEY`). Otherwise the checks run in this order:

1. names: `UNKNOWN_SERVICE`, `UNKNOWN_OPTION`
2. settings, by key in the order `azs`, `region`, `lang`, `direction`, `nat`, `readers`, then the keys that are not settings
3. connections: those not in the list, then a main flow to `internet` from outside a private subnet, then the main flows not reached from a starting point and the auxiliary flows and associations hanging off them, then `TOO_MANY_ATTACHED`, then a part attached to both CloudFront and a load balancer
4. the answers in `choose`: `BAD_CHOICE`, `UNKNOWN_HOLE`

When a name is unknown, the connections are not checked. For example, this definition gives five errors:

```yaml
flows:
  - user -> alb -> ecs
  - alb -> aurora
  - ecs -> s3
set:
  colour: red
  lang: fr
  azs: 4
choose:
  route.ecs.rds: nat
```

`BAD_SETTING` at `set.azs`, `BAD_SETTING` at `set.lang`, `UNKNOWN_SETTING` at `set.colour`, `UNSUPPORTED` at `flows[1]`, and `UNKNOWN_HOLE` at `choose.route.ecs.rds`, in that order.

Here is a full report for a misspelled name, from `- user -> cloudfrnt -> alb -> ecs` (exit code 1):

```json
{
  "versions": {
    "archwire": "0.1.0",
    "rules": "2026.11.7"
  },
  "derived": [],
  "holes": [],
  "errors": [
    {
      "code": "UNKNOWN_SERVICE",
      "at": "flows[0][1]",
      "got": "cloudfrnt",
      "allowed": [
        "acm",
        "alb",
        "apigateway",
        "athena",
        "aurora",
        "cloudfront",
        "cloudtrail",
        "cloudwatch",
        "cognito",
        "config",
        "dynamodb",
        "ecs",
        "eventbridge",
        "glue",
        "guardduty",
        "inspector",
        "internet",
        "lambda",
        "s3",
        "securityhub",
        "securitylake",
        "sns",
        "sqs",
        "ssm",
        "stepfunctions",
        "user",
        "waf"
      ],
      "rule": null,
      "message": "unknown service `cloudfrnt`"
    }
  ],
  "warnings": [],
  "notes": []
}
```

Fix it with `archwire set web.yaml 'flows[0][1]' cloudfront`.
