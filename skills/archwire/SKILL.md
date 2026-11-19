---
name: archwire
description: Use when the user asks for an AWS architecture diagram, a draw.io file of an AWS setup, or to turn a described AWS architecture into a diagram. Writes a short definition of how the services connect and compiles it with the archwire command, which lays the diagram out by rules and adds VPCs, subnets, zones and gateways with a reason for each.
---

# Drawing an AWS architecture diagram with archwire

You write a few lines saying which services connect to which. The `archwire` command lays out the diagram, adds the parts the lines imply (VPC, subnets, Availability Zones, gateways) and records why it added each. You do not write coordinates or XML. The same definition always gives the same diagram.

archwire is not an official AWS tool. A definition that compiles is consistent under archwire's rules; that does not mean the architecture works in AWS. Do not tell the user it does.

## Three rules that come first

1. Never change what the user asked for to make the definition compile. When archwire stops on a service or a connection (`UNKNOWN_SERVICE`, `UNSUPPORTED`, `LAYOUT_FAILED`), do not drop the part, swap it for another service, or reroute the connection on your own. Tell the user what could not be drawn and what the report said, and ask how to go on.
2. Never answer a hole yourself. A hole is a decision the connections alone cannot make, such as whether traffic from ECS to S3 leaves the VPC through a VPC endpoint or a NAT gateway. Show the user the choices from the report and write the answer they give.
3. Exit code 0 is not the end. When the report's `notes` name a line (a note with an id such as `edge-alb-to-s3`), that line could not be drawn clear of everything. Say which line, and do not call the diagram finished without the user seeing it.

## Steps

### 1. Find the command

```sh
archwire --version
```

When it is not installed, run it through `uvx` instead, with the same arguments: `uvx archwire --version`. Use whichever works for every command below. It runs on macOS and Linux.

### 2. Ask what can be written

```sh
archwire list
```

This prints JSON with `services` (the names to write, their `kinds`, and whether they sit in a VPC), `links` (every accepted connection, as `from`, `symbol`, `to` and the label the line gets) and `settings`. Read it before writing: a connection that is not in `links` stops with `UNSUPPORTED`. Do not guess names from memory; the list changes between versions.

### 3. Write the definition

A definition is a small text file with up to three sections. Only `flows` is required.

```yaml
flows:
  - user -> cloudfront -> alb -> ecs(fargate) -> aurora
  - cloudfront -> s3
  - ecs ~> cloudwatch
  - waf -- cloudfront
set:
  lang: ja
```

- Each line under `flows` chains parts with a symbol. `->` is a main flow, the path traffic or data takes. `~>` is an auxiliary flow such as logs and metrics. `--` attaches one thing to another, with the thing being attached written first.
- A part is `service`, `service.name` or `service(kind)`, such as `s3.logs` or `apigateway(http)`. Use a name after the dot when there are two of one service.
- Do not write VPCs, subnets, zones, Internet Gateways or NAT gateways. archwire adds them.
- `set` holds settings from the `settings` list, such as `azs: 3`, `direction: right` or `lang: ja`. Use `lang: ja` when the user writes in Japanese.
- Only a small part of YAML is accepted: no quotes, no nested lists, no multi-line values.

Write every part and connection the user named. When something they named is not in the list from step 2, stop here and tell them, as in rule 1.

### 4. Compile

```sh
archwire compile web.yaml -o web.drawio > web.json
echo $?
```

| Exit code | Meaning |
|---|---|
| 0 | The diagram was written and nothing is undecided |
| 1 | The definition has an error. No diagram was written |
| 2 | The diagram was written, with at least one hole |

### 5. Read the report

The report is JSON on standard output.

- `errors`: each has a `code`, a place `at`, and often `allowed` values. A typing mistake of yours (a misspelt name, a kind that does not exist) can be fixed and compiled again. An error about what the user asked for falls under rule 1.
- `holes`: each has the `hole` id, what it `need`s and its `choices`. Ask the user, then write the answer and compile again:

  ```sh
  archwire set web.yaml choose.route.ecs.s3 endpoint
  ```

- `notes`: read every one. A note naming a line falls under rule 3. The other notes explain how the diagram is drawn.
- `derived`: the parts and settings archwire added or chose, each with a `reason`. A `kind` of `constraint` means AWS requires it and `source` links the AWS documentation; `default` means archwire chose it. Use these to tell the user what was added and why.

### 6. Show the result

When the draw.io desktop CLI is installed, write a picture and look at it yourself before showing it:

```sh
drawio -x -f png -b 10 -o web.png web.drawio
```

Tell the user:

- where the `.drawio` file is (it opens in draw.io and can be edited there)
- what archwire added that they did not write, with the reasons from `derived`
- any answer they gave to a hole, and any note naming a line
- anything they asked for that is not in the diagram

Compare the finished definition with what the user first asked for, part by part, before saying it is done. If they differ, say how.
