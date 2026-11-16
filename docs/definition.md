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

`->` is a main flow, the path traffic takes. `~>` is an auxiliary flow, such as logs going to CloudWatch. It is drawn as a dashed line, and archwire does not work out a network route for it. `--` is an association, for something attached to something else. Write the thing being attached first. An association is drawn as a dashed line without an arrowhead, because nothing flows along it. Which associations are accepted depends on the layout, see below. `waf` and `acm` are attachments: they may not be the source or the target of `->` or `~>`.

One pair of parts can be connected with one relation only. `ecs -> s3` together with `ecs ~> s3` is the error `CONFLICTING_RELATION`. Direction matters. `ecs -> aurora` and `aurora -> ecs` are two different pairs.

### Two layouts

archwire picks the layout from the services the definition writes. A definition with any service that sits in a VPC (`alb`, `ecs` or `aurora`) is drawn with a VPC, as described under [Definitions with a VPC](#definitions-with-a-vpc). A definition without them is drawn without a VPC, as described under [Definitions without a VPC](#definitions-without-a-vpc).

`apigateway`, `lambda`, `dynamodb` and `cognito` cannot be written together with VPC services yet. Such a definition stops with `UNSUPPORTED`, naming the first of those parts. For example, `- user -> apigateway(http) -> lambda -> aurora` gives `at` `flows[0][1]` and the message ``"`apigateway` cannot be drawn together with VPC services such as `aurora`; this is not supported yet"``.

### Services

| Name | Service | Kinds | Layout |
|---|---|---|---|
| `user` | Users | | both |
| `internet` | The internet. Only as the target of `->`, see below | | with a VPC |
| `cloudfront` | Amazon CloudFront | | both |
| `waf` | AWS WAF. Only as `waf -- cloudfront` | | both |
| `acm` | AWS Certificate Manager. Only as `acm -- cloudfront` | | both |
| `alb` | Application Load Balancer | | with a VPC |
| `ecs` | Amazon ECS | `fargate` | with a VPC |
| `aurora` | Amazon Aurora | | with a VPC |
| `s3` | Amazon S3 | | both |
| `cloudwatch` | Amazon CloudWatch | | both |
| `apigateway` | Amazon API Gateway | `http`, `rest` | without a VPC |
| `lambda` | AWS Lambda | | without a VPC |
| `dynamodb` | Amazon DynamoDB | | without a VPC |
| `cognito` | Amazon Cognito. Only as `cognito -- apigateway` | | without a VPC |

The Internet Gateway and the NAT gateways are added by archwire and cannot be written. Writing `nat` is `UNKNOWN_SERVICE`.

A kind is written once or on every mention of the part. `ecs(fargate)` on one line and plain `ecs` on another are the same part, and both get the kind. Two different kinds for one part are the error `CONFLICTING_OPTION`. `fargate` changes the icon's label to Amazon ECS (Fargate). `http` and `rest` change the label of API Gateway to Amazon API Gateway with HTTP API or REST API on the next line. Leaving out the kind of `apigateway` leaves a hole, see [The kind of API Gateway](#the-kind-of-api-gateway).

A certificate used with CloudFront lives in US East (N. Virginia), whatever Region the diagram shows, so archwire draws `acm` outside the Region frame with `us-east-1` on the line under its name, and records the reason with its AWS source.

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

The name after the dot is made of `a-z`, `0-9` and `-`. It may not start or end with `-`, and a part has at most one dot. It is at most 24 characters long; a longer one is the error `NAME_TOO_LONG`, for example `lambda.resize-images-for-the-web` in `- user -> apigateway(http) -> lambda.resize-images-for-the-web` gives `at` `flows[0][2]` and `got` `resize-images-for-the-web`. `--` is always read as the association symbol, even inside a name, so `s3.foo--bar` is read as `s3.foo -- bar`; in `- user -> cloudfront -> s3.foo--bar` that gives `UNKNOWN_SERVICE` for `bar` at `flows[0][3]`. Write the kind after the name: `apigateway.public(http)`.

Named parts are accepted only in definitions without a VPC. `- user -> alb -> ecs -> s3.web` stops with `UNSUPPORTED` at `flows[0][3]` and the message ``"named parts such as `s3.web` are not supported together with VPC services yet"``.

## Definitions with a VPC

archwire adds what the path needs, such as the Internet Gateway between CloudFront and the load balancer, and lays the parts out as described under [How the parts are laid out with a VPC](#how-the-parts-are-laid-out-with-a-vpc). A note in the report says that auxiliary flows are not given a network route.

In this layout exactly two associations are accepted, `waf -- cloudfront` (a web ACL on the distribution) and `acm -- cloudfront` (its certificate). Every other `--` is `UNSUPPORTED`, including `waf -- alb`, `acm -- alb`, `waf -- acm` and the reversed `cloudfront -- waf` and `cloudfront -- acm`. Which service may attach to which is data, the `attach_to` field in `rules/aws/services.toml`.

`internet` is never drawn. `ecs -> internet` says the ECS tasks make outbound requests. archwire adds a NAT gateway in each public subnet, one per Availability Zone unless `nat` is `single`, and the lines from each ECS task to its NAT gateway and on to the Internet Gateway.

### Which connections are accepted with a VPC

A definition needs a main flow from `user` or `cloudfront` into `alb`, and a main flow from `alb` into `ecs`. Every other line must be one of the combinations below. Anything else is `UNSUPPORTED`, with the line named in `at`.

For flows the services fall into five groups. Outside is `user`. Global is `cloudfront`. The load balancer `alb` is public. `ecs` and `aurora` are private. Regional is `s3` and `cloudwatch`. `waf` and `acm` are in no group: they only attach to CloudFront, as described above.

| From and to | Symbol | Example |
|---|---|---|
| outside into a global service | `->` | `user -> cloudfront` |
| outside into the load balancer | `->` | `user -> alb` |
| a global service into the load balancer | `->` | `cloudfront -> alb` |
| a global service into a Regional one | `->` or `~>` | `cloudfront -> s3`, `cloudfront ~> cloudwatch` |
| WAF or Certificate Manager attached to CloudFront | `--` | `waf -- cloudfront`, `acm -- cloudfront` |
| the load balancer into a private service | `->` | `alb -> ecs` |
| the load balancer to a Regional service | `~>` | `alb ~> cloudwatch` |
| a private service into a private service | `->` | `ecs -> aurora` |
| a private service into a Regional one | `->` or `~>` | `ecs -> s3`, `ecs ~> cloudwatch` |
| a private service out to the internet | `->` | `ecs -> internet` |

For example, these are all `UNSUPPORTED` in a definition with a VPC: `cloudfront -> ecs` (skipping the load balancer), `user -> aurora`, `alb -> s3`, `user ~> alb`, `internet -> alb`, `ecs -> user`, `waf -> alb` and `acm -- alb`.

An auxiliary flow is labelled "Logs and metrics" whatever its target, because it does not carry the target's main traffic.

`ecs -> s3` is accepted but leaves a hole, see `choose`.

### How the parts are laid out with a VPC

Outside the VPC, the parts are laid out as in a definition without a VPC (see [How the parts are laid out](#how-the-parts-are-laid-out)), in the direction `set.direction` gives: from top to bottom, or from left to right with `direction: right`. Users and CloudFront come first, then the Region frame, which holds the VPC and the Regional services. The VPC takes a place in its rank like one large part, and comes first in that rank. So S3 in `cloudfront -> s3` goes to the right of the VPC in a top-to-bottom diagram, and below it in a left-to-right one. A part that only parts inside the VPC connect to, such as CloudWatch in `ecs ~> cloudwatch`, goes to the right of the VPC in both directions.

The inside of the VPC is the same in both directions. The Availability Zones stand side by side, and in each zone the public subnet is above the private one. Inside the subnets the main flows run from top to bottom, so in `alb -> ecs -> aurora` ECS is above Aurora. ECS gets a copy in each zone. Aurora's writer is in the first zone and its readers in the zones after it.

The Internet Gateway sits inside the VPC on the side where the traffic comes in: the top side in a top-to-bottom diagram, the left side in a left-to-right one. A part that sends a main flow to the Internet Gateway, such as CloudFront, is lined up with it, so CloudFront, the Internet Gateway and the load balancer stand on one straight line, and a line from CloudFront to S3 bends on its way.

The load balancer spans the zones. With two zones it sits in the public tier on the border between them, level with the NAT gateways in a top-to-bottom diagram. With three zones it sits above the zone frames.

A line's label says which zones it stands for. With two zones, archwire draws the lines between parts in different zones. With three zones it draws no line between zones: each line inside a zone stands for the lines from every zone into the same part, and a note in the report says so. A label ends with "(all AZs)" when the sender has a copy in every zone, and names the zones otherwise. For example, with `azs: 3` and `readers: 1`, Aurora has a copy in two zones only, and `aurora ~> cloudwatch` is labelled "Logs and metrics (from 1a, 1c)". When a target in one zone has no sender in the same zone, the line comes from a sender in another zone, and a note names it. With the same settings, `aurora -> ecs` gives the note "In ap-northeast-1d, ecs-1d has no aurora in its own zone, so the line from aurora-1a in another zone is drawn."

Some diagrams are correct but harder to read than they could be. In a top-to-bottom diagram, WAF and Certificate Manager attached to CloudFront stand one under the other to its right, which makes the row of global services tall. An auxiliary line from the load balancer to a part right of the VPC, such as `alb ~> s3`, can go round the far side of that part and enter it from the right. In a left-to-right diagram, S3 fed by CloudFront sits below the VPC, so the line to it is long.

## Definitions without a VPC

### Which connections are accepted without a VPC

The accepted connections are listed pair by pair in `rules/aws/links.toml`. Anything not in the list is `UNSUPPORTED`, with the line in `at` and the connection in `got`.

| From | Symbol | To | Line label |
|---|---|---|---|
| `user` | `->` | `cloudfront` | HTTPS |
| `user` | `->` | `apigateway` | HTTPS |
| `cloudfront` | `->` | `s3` | Static files |
| `cloudfront` | `->` | `apigateway` | HTTPS |
| `apigateway` | `->` | `lambda` | Invoke |
| `lambda` | `->` | `dynamodb` | Read/Write |
| `lambda` | `->` | `s3` | Read/Write |
| `lambda` | `~>` | `cloudwatch` | Logs and metrics |
| `apigateway` | `~>` | `cloudwatch` | Logs and metrics |
| `cognito` | `--` | `apigateway` | JWT authorizer for `http`, Cognito authorizer for `rest`, Verify tokens while the kind is undecided |
| `waf` | `--` | `cloudfront` | Web ACL |
| `acm` | `--` | `cloudfront` | Certificate |

A pair means the services, whatever the parts are named: `lambda.resize -> s3.uploads` is the pair `lambda -> s3`.

For example, each of these stops with `UNSUPPORTED`:

| Definition | `at` | Message |
|---|---|---|
| `- user -> s3` | `flows[0]` | ``"`user -> s3` is not a supported connection"`` |
| `- user -> cloudfront -> lambda` | `flows[0]` | ``"`cloudfront -> lambda` is not a supported connection"`` |
| `- user -> cloudfront -> s3` and `- cloudfront ~> cloudwatch` | `flows[1]` | ``"`cloudfront ~> cloudwatch` is not a supported connection"`` |
| `- lambda -> dynamodb` alone | `flows[0]` | ``"`lambda -> dynamodb` is not reached by main flows (->) from `user`"`` |

Besides the list, archwire checks the following, and stops with `UNSUPPORTED` when one does not hold:

- There is at least one main flow (`at` `flows`), and every main flow can be reached from `user` along main flows (`at` the line that cannot be reached).
- The source of an auxiliary flow and the target of an association are on a main flow reached from `user`. With `- user -> cloudfront -> s3` and `- cognito -- apigateway`, `apigateway` is on no main flow, so the second line stops with ``"`cognito -- apigateway` needs `apigateway` on a main flow (->) from `user`"``.

One CloudFront distribution takes one WAF. With `- user -> cloudfront -> s3`, `- waf.front -- cloudfront` and `- waf.extra -- cloudfront`, the third line stops with `TOO_MANY_ATTACHED`, `got` `waf.extra`, and `rule` `one-web-acl-per-resource`; the reason is in [docs/rules.md](rules.md). One WAF attached to two distributions is accepted.

### The kind of API Gateway

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

### Settings without a VPC

Only `direction`, `lang` and `region` can be set. Any other key, including `azs`, `nat` and `readers`, is `UNKNOWN_SETTING` with `allowed` `direction`, `lang` and `region`.

### How the parts are laid out

The main flows run from top to bottom, or from left to right with `direction: right` (see [set](#set)). Below, a rank is a row in a top-to-bottom diagram and a column in a left-to-right one, and across means along the rank: from left to right along a row, or from top to bottom down a column. Users come first, then a rank for global services (CloudFront, WAF and Certificate Manager) inside the AWS Cloud frame, then the Region frame. archwire works out the layout from the definition alone; the definition cannot move a part.

Inside the Region, each part on a main flow goes in the rank one past the furthest part that flows into it within the Region; a part with nothing flowing into it within the Region goes in the first rank. Within a rank, parts are placed across in the order they first appear in the definition. A part is centred on the parts that flow into it: its place across is halfway between the first and the last of them, rounded down to a whole pixel, unless that would bring it too close to the part before it. A part that sends main flows to two or more parts is then moved, where there is room, to halfway between the first and the last of them. Ranks move apart when the names or line labels between them are wide.

Parts that carry no main flow, such as Cognito and CloudWatch, go one step across past the parts on main flows (to their right in a top-to-bottom diagram, below them in a left-to-right one), in the rank of the part they are attached to (for an association) or of the first part that sends to them (for an auxiliary flow). That rank is tried first; when another such part or its name is already there, the part moves to the next free rank. WAF and Certificate Manager go one step across from the CloudFront they are attached to, one after another in the direction of the main flows; one attached to two distributions goes past everything else in the global rank. Adding or removing such a part does not move the parts on main flows within the Region.

When several parts of the same service send the same auxiliary flow (`~>`) or association (`--`), with the same label, to the same part, the diagram draws one line for them. Main flows (`->`) are never merged; each is drawn. The merged line is labelled with the count, such as "Logs and metrics (3 sources)" or 「ログ・メトリクス（3か所から）」. The report's `notes` names the parts, such as `"The line from lambda.search to cloudwatch stands for the lines from lambda.orders, lambda.users, lambda.search."`. The acceptance case `tests/cases/api-merged-ja` shows this with three Lambda functions sending logs to one CloudWatch.

When no part belongs to a Region, as in `- user -> cloudfront` with `- waf -- cloudfront` (the acceptance case `tests/cases/edge-waf-en`, drawn with `direction: right`), no Region frame is drawn, and the report gives no reason for one.

A diagram without a VPC gets these reasons: the AWS Cloud frame, the Region frame and the Region chosen when `set.region` is left out, and the place of each part the definition wrote (global services outside the Region frame, Regional services inside it, Cognito inside it because a user pool is created in one Region, and Certificate Manager outside it because a certificate used with CloudFront is in us-east-1). With `apigateway(http)` or `apigateway(rest)` and a `cognito -- apigateway` line, the API Gateway also gets the reason why the association is drawn as that authorizer.

If archwire cannot lay out a definition, with or without a VPC, without parts or names overlapping or leaving their frame, it writes no diagram and reports `LAYOUT_FAILED`. That is a bug in archwire, not in the definition.

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

When `azs`, `region`, `readers` or `nat` is left out, archwire uses the default and records it in the report as a `default` reason. `direction` and `lang` change only how the diagram is drawn and get no reason. A value outside the list is `BAD_SETTING`; `direction: left` gives `allowed` `down` and `right`. A key that is not in the table is `UNKNOWN_SETTING`.

## choose

```yaml
choose:
  route.ecs.s3: endpoint
```

`choose` answers holes. There are two kinds of hole: the route of `ecs -> s3` in a definition with a VPC, described here, and [the kind of API Gateway](#the-kind-of-api-gateway) in a definition without one.

For the route, the key is the hole id from the report, `route.<from>.<to>`, and the value is `endpoint` (the traffic goes through a VPC endpoint; archwire draws no endpoint icon, and the line's label adds "(via VPC endpoint)") or `nat` (it goes out through NAT gateways, which archwire adds). The id depends on the part names only, so adding or moving lines does not change it. `archwire set <definition> choose.route.ecs.s3 endpoint` writes the entry for you.

### Answering with nat

```yaml
flows:
  - user -> alb -> ecs -> s3
choose:
  route.ecs.s3: nat
```

`compile` exits with 0. archwire adds a NAT gateway in each public subnet, with a line from each ECS task to its NAT gateway and on to the Internet Gateway, all labelled "Outbound". The line to S3 is labelled "Read/Write (all AZs)" on one line. The report gives the reasons for the NAT gateways:

```json
{
  "target": "nat-1a",
  "added": true,
  "kind": "constraint",
  "rule": "private-egress-through-nat",
  "reason": "Instances in a private subnet connect to the internet through a public NAT gateway, which is created in a public subnet.",
  "source": "https://docs.aws.amazon.com/vpc/latest/userguide/vpc-nat-gateway.html"
},
```

The same entry follows for `nat-1c`, and then `nat-per-az`, the default that puts one NAT gateway in each zone. `notes` is empty.

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

The place is `flows[i][j]`, `set.<key>` or `choose.<hole id>`. For `set` and `choose`, the section or the key is added if it is missing, and an existing value is replaced in place. An added line copies the indentation of the other lines in its section, or of the file's first indented line when the section is new, so a file keeps its own indentation. For `flows[i][j]`, the j-th part of the i-th flow line (both counting from 0, and only flow lines are counted) is replaced; the value may carry a name and a kind, as in `ecs(fargate)` or `apigateway.public(http)`, and must read as one part, so that the edit cannot change the shape of the line; `s3.foo--bar` is `BAD_VALUE`. The length of a name after the dot is not checked by `set`; `compile` reports `NAME_TOO_LONG`. The value must be one non-empty line without `#`. `set` does not check that the result is a valid definition. `compile` does.

## Errors

When a definition has errors, `compile` writes no diagram, removes a diagram left at the `-o` path by an earlier compile, prints the report with an `errors` list, and exits with 1. Several errors can be reported at once. Each example below was run, and the line shows the report's `code`, `at` and `got`. The last column says whether `archwire set <at> <value>` can fix the error at the place the report gives, and what to do otherwise. The examples for `BAD_CHOICE` and `UNKNOWN_HOLE` assume the definition contains `ecs -> s3`, so that the hole `route.ecs.s3` exists.

| Code | Meaning | Example and result | Fixed by `archwire set <at>`? |
|---|---|---|---|
| `EMPTY_DEFINITION` | there are no flows | an empty file: `at` is `flows` | No. Write a `flows:` section with at least one line in the file |
| `BAD_SYNTAX` | a line is not in the accepted form | `- user => alb` under `flows:` gives `at` `flows[0]` | No. `at` is a whole line (`flows[i]`, `set[i]`, `choose[i]` or `line n`); edit that line in the file |
| `UNKNOWN_SECTION` | a heading other than `flows`, `set`, `choose` | `extra:` as a heading gives `at` `line 3`, `allowed` lists the three sections | No. Rename or remove the heading in the file |
| `NAME_TOO_LONG` | a name after the dot is longer than 24 characters | `- user -> apigateway(http) -> lambda.resize-images-for-the-web` gives `at` `flows[0][2]`, `got` `resize-images-for-the-web` | Yes, with a shorter name, such as `archwire set api.yaml 'flows[0][2]' lambda.resize` |
| `SELF_LINK` | a part connected to itself | `- ecs -> ecs` gives `at` `flows[0]` | No. `at` is the whole flow line; change one of its parts with `flows[i][j]` or edit the line |
| `CONFLICTING_OPTION` | one part written with two kinds | `- alb -> ecs(fargate)` and `- ecs(ec2) -> aurora` give `at` `flows[1][0]`, `allowed` is `fargate` | Yes, with a value from `allowed`, such as `archwire set web.yaml 'flows[1][0]' 'ecs(fargate)'` |
| `CONFLICTING_RELATION` | one pair of parts with two relations | `- ecs -> s3` and then `- ecs ~> s3` give `at` `flows[2]` | No. `set` changes parts, not symbols; remove one of the two lines in the file |
| `DUPLICATE_KEY` | a key written twice in `set` or `choose` | `azs: 2` and `azs: 3` give `at` `set.azs` | No. `set` would change only the first of the two; delete one line in the file |
| `UNKNOWN_SERVICE` | a name that is not a writable service | `- user -> cloudfrnt -> alb -> ecs` gives `at` `flows[0][1]`, `allowed` lists the writable names | Yes, with a name from `allowed` |
| `UNKNOWN_OPTION` | a kind the service does not have | `ecs(lambda)` gives `allowed` `fargate` | Yes, with the part and a kind from `allowed`, or the bare name |
| `BAD_SETTING` | a value outside the list | `azs: 4` gives `at` `set.azs`, `allowed` is `2` and `3` | Yes, with a value from `allowed` |
| `UNKNOWN_SETTING` | a key that is not a setting | `colour: red` gives `at` `set.colour`, `allowed` lists the six keys; without a VPC, `azs: 3` gives `at` `set.azs`, `allowed` `direction`, `lang` and `region` | No. `set` can change only the value; delete the line in the file |
| `BAD_CHOICE` | an answer that is not one of the hole's choices | `route.ecs.s3: tunnel` gives `at` `choose.route.ecs.s3`, `allowed` `endpoint` and `nat`; `kind.apigateway: websocket` gives `allowed` `http` and `rest` | Yes, with a value from `allowed` |
| `UNKNOWN_HOLE` | an answer for a hole that does not exist | `route.ecs.rds: nat` gives `at` `choose.route.ecs.rds`; so does `kind.apigateway: http` when the definition already says `apigateway(http)` | No. `set` can change only the value; delete the line in the file, and answer a hole from `allowed` with `archwire set <definition> choose.<hole> <value>` |
| `UNSUPPORTED` | a connection or shape this version does not draw | `- user -> cloudfront -> ecs` gives `at` `flows` and `at` `flows[0]`; `- user -> s3` gives `at` `flows[0]`; `- user -> apigateway(http) -> lambda -> aurora` gives `at` `flows[0][1]` | Sometimes. `at` is a flow line (`flows[i]`), a part (`flows[i][j]`) or `flows`; change a part with `flows[i][j]`, or edit or remove the line in the file |
| `TOO_MANY_ATTACHED` | more parts attached to one part than AWS allows | `- user -> cloudfront -> s3`, `- waf.front -- cloudfront` and `- waf.extra -- cloudfront` give `at` `flows[2]`, `got` `waf.extra`, `rule` `one-web-acl-per-resource` | No. Remove the extra line in the file |
| `LAYOUT_FAILED` | archwire could not lay out a definition without overlaps; a bug in archwire | no known definition; the message names the parts involved | No. Please report the definition |
| `BAD_PATH` | the place given to `set` does not exist | `archwire set web.yaml 'flows[9][0]' x`, on a file with fewer flow lines | Run `set` again with an existing place |
| `BAD_VALUE` | the value given to `set` is empty or has a line break or `#`, or a value for `flows[i][j]` does not read as one part | `archwire set web.yaml set.azs ""`; `archwire set api.yaml 'flows[0][2]' 's3.foo--bar'` | Run `set` again with one non-empty line, or one part |
| `CANNOT_READ` | `compile` or `set` could not read the definition file | `archwire compile missing.yaml -o out.drawio` | No. Check the path and the file's permissions |
| `CANNOT_WRITE` | `compile` could not write the diagram or remove an old one, or `set` could not write the definition | `archwire compile web.yaml -o no/such/dir/web.drawio`. The report is still printed, and the exit code is 1 | No. Check the path and the permissions |

The reports that `set` prints (for `BAD_PATH`, `BAD_VALUE`, `CANNOT_READ` and `CANNOT_WRITE`) have both versions filled in and empty `derived`, `holes` and `notes`. When `set` succeeds it prints nothing.

Here is a full report for a misspelled name, from `- user -> cloudfrnt -> alb -> ecs` (exit code 1):

```json
{
  "versions": {
    "archwire": "0.1.0",
    "rules": "2026.11.3"
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
        "aurora",
        "cloudfront",
        "cloudwatch",
        "cognito",
        "dynamodb",
        "ecs",
        "internet",
        "lambda",
        "s3",
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
