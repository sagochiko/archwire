# Reading the report

`archwire compile` prints a report as one JSON object on standard output, whether or not it writes a diagram. Save it by redirecting standard output:

```sh
archwire compile web.yaml -o web.drawio > web.json
```

`archwire set` prints nothing when it succeeds. When it fails, it prints a report in the same format with the error.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | The diagram was written and nothing is left undecided. For `set`, the definition was rewritten. |
| 1 | The definition or a file has an error, listed in `errors`. The report is still printed. |
| 2 | The diagram was written, but at least one hole is left in `holes`. |

An error comes before a hole: when there are both, the exit code is 1.

When `compile` writes no diagram, because the definition has errors or cannot be read, it removes a file left at the `-o` path by an earlier compile, so that file cannot pass for the result of this one. When that file cannot be removed, the report has the error `CANNOT_WRITE` naming it. When the diagram cannot be written, the report has `CANNOT_WRITE` too. Either way the exit code is 1.

## Fields

The fields come in this order: `versions`, `derived`, `holes`, `errors`, `warnings`, `notes`.

`versions` holds the archwire version (`archwire`) and the version of the rules data (`rules`). The same definition gives the same report and diagram, byte for byte, as long as both versions are the same.

`derived` lists everything archwire added or decided by itself. Each entry has:

- `target`, the part it applies to
- `added`, true when the target is something archwire added, such as a frame, the Internet Gateway or a NAT gateway; false when the reason places a part the definition wrote, such as ECS in a private subnet
- `kind`, `constraint` or `default`
- `rule`, the rule id, listed in `rules/aws/rules.toml` (see [Rules](rules.md))
- `reason`, the reason in the language chosen by `set.lang`
- `source`, for constraints, the AWS documentation page that states it

A constraint comes from AWS documentation. A default is archwire's own choice for something the definition did not say, such as using two Availability Zones or placing ECS in private subnets. A default is not an AWS requirement. The report and the diagram both say which is which.

`holes` lists the decisions the connections alone cannot make. Each has the hole id (`hole`), the place in the definition (`at`), what is needed (`need`) and the `choices`. How to answer them is in [choose](definition.md#choose) and [The kind of API Gateway](definition.md#the-kind-of-api-gateway).

`errors` lists what stopped the compile, each with a `code`, the place `at`, the value `got`, the values `allowed` where there is a fixed list, the `rule`, and a `message`. The `rule` field is `null`, except for `TOO_MANY_ATTACHED`, where it names the rule the definition breaks. The codes are listed in [Errors](definition.md#errors).

`warnings` is part of the format and is not used in this version: it is always empty.

`notes` carries plain remarks about the drawing.

## Notes

When a line could not be drawn clear of every icon, text and other line, or its label could not be placed clear of every icon, text and other line, a note names the line (such as `edge-alb-to-s3`) so you can look at it. The diagram is still written.

Two lines that share no end are kept from turning at one point, where the first half of one and the second half of the other would read as one line; when they still do, a note names both.

A line's label may lie over the side of a frame when no spot clear of frame sides is left; the text stays readable, so that gets no note.

In a diagram without a VPC, when several parts of one service send the same auxiliary flow (`~>`) or association (`--`) to one part, the diagram draws one line labelled with the count, such as "Logs and metrics (3 sources)", and a note names the parts it stands for.

Other notes explain how the diagram is drawn, such as that auxiliary flows are not given a network route, or that VPC endpoint icons are not drawn. They do not point at a fault.

How often notes naming a line appear in crowded diagrams is in [Limits](limits.md).

## Reasons in the diagram

The reasons are not only in the report. Every element with a reason carries it inside the `.drawio` file, as the attributes `derived` (the reason text), `rule`, `kind` and `source` on the cell, so they travel with the diagram. When an element has more than one reason, each attribute holds them in the same order, separated by ` | `. Every cell archwire added, that is every frame and the Internet Gateway and NAT gateways, also has the attribute `added="1"`; cells for parts the definition wrote do not, even when they carry a reason for where they were placed.

## Listing what a definition can use

`archwire list` takes no arguments, prints one JSON object on standard output and exits with 0. It is made from the built-in rules data, so a program that writes definitions, such as an AI agent, can learn what it may write without reading these documents.

<!-- archwire-test: run exit=0 -->
```sh
archwire list > list.json
```

The fields come in this order: `versions`, `services`, `links`, `settings`. `versions` is the same as in the report of `compile`.

`services` lists the services a definition can write, in the order of their names. The parts archwire only adds, the Internet Gateway and the NAT gateways, are not in it. Each has the name to write (`name`), the service's name in English (`label`), the kinds that can be written after it in parentheses (`kinds`, empty when there are none), and `in_vpc`, true for a service that is always drawn in the VPC. `lambda` has `in_vpc` false, although a function that sends a main flow to `aurora` is drawn in the VPC.

<!-- archwire-test: report-part -->
```json
{
  "name": "ecs",
  "label": "Amazon ECS",
  "kinds": [
    "fargate"
  ],
  "in_vpc": true
}
```

`links` lists the accepted connections, in the order of `rules/aws/links.toml`, the same as the table in [Which connections are accepted](definition.md#which-connections-are-accepted). Each has `from`, `to`, the `symbol` to write between them (`->`, `~>` or `--`) and the line label in English (`label`). `kind_labels` gives the label for each kind of the `to` part, when it depends on the kind; `label` is then the one used while the kind is undecided. `role_labels` gives the label for the Aurora writer and for a reader. Both are empty objects for the other connections.

<!-- archwire-test: report-part -->
```json
{
  "from": "cognito",
  "to": "apigateway",
  "symbol": "--",
  "label": "Verify tokens",
  "kind_labels": {
    "http": "JWT authorizer",
    "rest": "Cognito authorizer"
  },
  "role_labels": {}
}
```

`settings` lists the keys of `set`, in the order of their names, each with the `values` it accepts and `vpc_only`, true for a key only a definition with a VPC may write. `readers` lists every value some definition accepts; the values a definition accepts stop at its `azs` minus 1, see [set](definition.md#set).

## Compatibility

This page describes the report of archwire 0.1.0. A report says which archwire and which rules data produced it in `versions`, so compare those first when two reports for one definition differ. A new version of the rules data can change the reasons, the labels and the diagram for the same definition.
