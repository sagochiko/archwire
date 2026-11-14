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

Each line is `- ` followed by parts joined by connection symbols. A part is a lowercase name made of `a-z`, `0-9` and `_`, optionally with a kind in parentheses, such as `ecs(fargate)`. A line may chain several parts, and `a -> b -> c` means `a -> b` and `b -> c`. The same line written twice counts once.

There are three connection symbols.

`->` is a main flow, the path traffic takes. archwire places the parts so the flow reads from top to bottom and adds what the path needs, such as the Internet Gateway between CloudFront and the load balancer. `~>` is an auxiliary flow, such as logs going to CloudWatch. It is drawn as a dashed line, and archwire does not work out a network route for it. A note in the report says so. `--` is an association, for something attached to something else. Write the thing being attached first. In this version exactly two associations are accepted, `waf -- cloudfront` (a web ACL on the distribution) and `acm -- cloudfront` (its certificate). Every other `--` is `UNSUPPORTED`, including `waf -- alb`, `acm -- alb`, `waf -- acm` and the reversed `cloudfront -- waf` and `cloudfront -- acm`. `waf` and `acm` are attachments: they may not be the source or the target of `->` or `~>`. Which service may attach to which is data, the `attach_to` field in `rules/aws/services.toml`.

A certificate used with CloudFront lives in US East (N. Virginia), whatever Region the diagram shows, so archwire draws `acm` outside the Region frame with `us-east-1` on the second line of its label, and records the reason with its AWS source.

One pair of parts can be connected with one relation only. `ecs -> s3` together with `ecs ~> s3` is the error `CONFLICTING_RELATION`. Direction matters. `ecs -> aurora` and `aurora -> ecs` are two different pairs.

### Services

| Name | Service | Kinds |
|---|---|---|
| `user` | Users | |
| `internet` | The internet. Only as the target of `->`, see below | |
| `cloudfront` | Amazon CloudFront | |
| `waf` | AWS WAF. Only as `waf -- cloudfront` | |
| `acm` | AWS Certificate Manager. Only as `acm -- cloudfront` | |
| `alb` | Application Load Balancer | |
| `ecs` | Amazon ECS | `fargate` |
| `aurora` | Amazon Aurora | |
| `s3` | Amazon S3 | |
| `cloudwatch` | Amazon CloudWatch | |

The Internet Gateway and the NAT gateways are added by archwire and cannot be written. Writing `nat` is `UNKNOWN_SERVICE`.

A kind is written once or on every mention of the part. `ecs(fargate)` on one line and plain `ecs` on another are the same part, and both get the kind. Two different kinds for one part are the error `CONFLICTING_OPTION`. Today `fargate` is the only kind, and it changes the icon and label to Amazon ECS (Fargate).

`internet` is never drawn. `ecs -> internet` says the ECS tasks make outbound requests. archwire adds a NAT gateway in each public subnet, one per Availability Zone unless `nat` is `single`, and the lines from each ECS task to its NAT gateway and on to the Internet Gateway.

### Which connections are accepted

A definition needs a main flow from `user` or `cloudfront` into `alb`, and a main flow from `alb` into `ecs`. Every other line must be one of the combinations below. Anything else is `UNSUPPORTED`, with the line named in `at`.

For flows the services fall into five groups. Outside is `user`. Global is `cloudfront`. The load balancer `alb` is public. `ecs` and `aurora` are private. Regional is `s3` and `cloudwatch`. `waf` and `acm` are in no group: they only attach to CloudFront, as described under `--` above.

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

For example, these are all `UNSUPPORTED`: `user -> s3` (a user reaching S3 directly), `cloudfront -> ecs` (skipping the load balancer), `user -> aurora`, `alb -> s3`, `user ~> alb`, `internet -> alb`, `ecs -> user`, `waf -> alb` and `acm -- alb`.

An auxiliary flow is labelled "Logs and metrics" whatever its target, because it does not carry the target's main traffic.

`ecs -> s3` is accepted but leaves a hole, see `choose`.

## set

```yaml
set:
  azs: 3
```

| Key | Values | Default |
|---|---|---|
| `azs` | `2` or `3`, the number of Availability Zones | `2` |
| `region` | `ap-northeast-1`. No other Region in this version | `ap-northeast-1` |
| `readers` | the number of Aurora readers, from `1` up to `azs` minus 1 | `azs` minus 1 |
| `nat` | `per-az` (a NAT gateway in each zone) or `single` (one, in the first zone) | `per-az` |
| `lang` | `en` or `ja`, the language of labels, reasons and notes. Service names stay in English | `en` |

When a key is left out, archwire uses the default and records it in the report as a `default` reason. A value outside the list is `BAD_SETTING`, and a key that is not in the table is `UNKNOWN_SETTING`.

## choose

```yaml
choose:
  route.ecs.s3: endpoint
```

`choose` answers holes. The key is the hole id from the report, `route.<from>.<to>`, and the value is `endpoint` (the traffic goes through a VPC endpoint; archwire draws no endpoint icon, and the line's label adds "(via VPC endpoint)") or `nat` (it goes out through NAT gateways, which archwire adds). The id depends on the part names only, so adding or moving lines does not change it. `archwire set <definition> choose.route.ecs.s3 endpoint` writes the entry for you.

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

The place is `flows[i][j]`, `set.<key>` or `choose.<hole id>`. For `set` and `choose`, the section or the key is added if it is missing, and an existing value is replaced in place. An added line copies the indentation of the other lines in its section, or of the file's first indented line when the section is new, so a file keeps its own indentation. For `flows[i][j]`, the j-th part of the i-th flow line (both counting from 0, and only flow lines are counted) is replaced; the value may carry a kind, as in `ecs(fargate)`. The value must be one non-empty line without `#`. `set` does not check that the result is a valid definition. `compile` does.

## Errors

When a definition has errors, `compile` writes no diagram, removes a diagram left at the `-o` path by an earlier compile, prints the report with an `errors` list, and exits with 1. Several errors can be reported at once. Each example below was run, and the line shows the report's `code`, `at` and `got`. The last column says whether `archwire set <at> <value>` can fix the error at the place the report gives, and what to do otherwise. The examples for `BAD_CHOICE` and `UNKNOWN_HOLE` assume the definition contains `ecs -> s3`, so that the hole `route.ecs.s3` exists.

| Code | Meaning | Example and result | Fixed by `archwire set <at>`? |
|---|---|---|---|
| `EMPTY_DEFINITION` | there are no flows | an empty file: `at` is `flows` | No. Write a `flows:` section with at least one line in the file |
| `BAD_SYNTAX` | a line is not in the accepted form | `- user => alb` under `flows:` gives `at` `flows[0]` | No. `at` is a whole line (`flows[i]`, `set[i]`, `choose[i]` or `line n`); edit that line in the file |
| `UNKNOWN_SECTION` | a heading other than `flows`, `set`, `choose` | `extra:` as a heading gives `at` `line 3`, `allowed` lists the three sections | No. Rename or remove the heading in the file |
| `SELF_LINK` | a part connected to itself | `- ecs -> ecs` gives `at` `flows[0]` | No. `at` is the whole flow line; change one of its parts with `flows[i][j]` or edit the line |
| `CONFLICTING_OPTION` | one part written with two kinds | `- alb -> ecs(fargate)` and `- ecs(ec2) -> aurora` give `at` `flows[1][0]`, `allowed` is `fargate` | Yes, with a value from `allowed`, such as `archwire set web.yaml 'flows[1][0]' 'ecs(fargate)'` |
| `CONFLICTING_RELATION` | one pair of parts with two relations | `- ecs -> s3` and then `- ecs ~> s3` give `at` `flows[2]` | No. `set` changes parts, not symbols; remove one of the two lines in the file |
| `DUPLICATE_KEY` | a key written twice in `set` or `choose` | `azs: 2` and `azs: 3` give `at` `set.azs` | No. `set` would change only the first of the two; delete one line in the file |
| `UNKNOWN_SERVICE` | a name that is not a writable service | `- user -> cloudfrnt -> alb -> ecs` gives `at` `flows[0][1]`, `allowed` lists the writable names | Yes, with a name from `allowed` |
| `UNKNOWN_OPTION` | a kind the service does not have | `ecs(lambda)` gives `allowed` `fargate` | Yes, with the part and a kind from `allowed`, or the bare name |
| `BAD_SETTING` | a value outside the list | `azs: 4` gives `at` `set.azs`, `allowed` is `2` and `3` | Yes, with a value from `allowed` |
| `UNKNOWN_SETTING` | a key that is not a setting | `colour: red` gives `at` `set.colour`, `allowed` lists the five keys | No. `set` can change only the value; delete the line in the file |
| `BAD_CHOICE` | an answer that is not `endpoint` or `nat` | `route.ecs.s3: tunnel` gives `at` `choose.route.ecs.s3` | Yes, with `endpoint` or `nat` |
| `UNKNOWN_HOLE` | an answer for a hole that does not exist | `route.ecs.rds: nat` gives `at` `choose.route.ecs.rds` | No. `set` can change only the value; delete the line in the file, and answer a hole from `allowed` with `archwire set <definition> choose.<hole> <value>` |
| `UNSUPPORTED` | a connection or shape this version does not draw | `- user -> s3` gives `at` `flows` and `at` `flows[0]` | Sometimes. `at` is a flow line (`flows[i]`) or `flows`; change a part with `flows[i][j]`, or edit or remove the line in the file |
| `BAD_PATH` | the place given to `set` does not exist | `archwire set web.yaml 'flows[9][0]' x`, on a file with fewer flow lines | Run `set` again with an existing place |
| `BAD_VALUE` | the value given to `set` is empty or has a line break or `#` | `archwire set web.yaml set.azs ""` | Run `set` again with one non-empty line |
| `CANNOT_READ` | `compile` or `set` could not read the definition file | `archwire compile missing.yaml -o out.drawio` | No. Check the path and the file's permissions |
| `CANNOT_WRITE` | `compile` could not write the diagram or remove an old one, or `set` could not write the definition | `archwire compile web.yaml -o no/such/dir/web.drawio`. The report is still printed, and the exit code is 1 | No. Check the path and the permissions |

The reports that `set` prints (for `BAD_PATH`, `BAD_VALUE`, `CANNOT_READ` and `CANNOT_WRITE`) have both versions filled in and empty `derived`, `holes` and `notes`. When `set` succeeds it prints nothing.

Here is a full report for a misspelled name, from `- user -> cloudfrnt -> alb -> ecs` (exit code 1):

```json
{
  "versions": {
    "archwire": "0.1.0",
    "rules": "2026.11.1"
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
        "aurora",
        "cloudfront",
        "cloudwatch",
        "ecs",
        "internet",
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
