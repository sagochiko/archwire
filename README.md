# archwire

[日本語](README.ja.md)

archwire turns a short definition of how services connect into a draw.io diagram of an AWS architecture. It works by rules, not by an AI. The same definition, compiled by the same version of archwire with the same rules data, gives the same diagram, byte for byte, on macOS and Linux.

Every part or setting archwire adds on its own carries a reason, in the report and in the diagram. Where a setting is left out, archwire uses its default and says so. Where the connections alone cannot decide the architecture, archwire does not pick: it returns a hole in the report for you to answer in the definition. This is version 0.1.0.

## Example

Save this as `web.yaml`:

<!-- archwire-test: compile exit=0 file=web.yaml -->
```yaml
flows:
  - user -> cloudfront -> alb -> ecs(fargate) -> aurora
  - cloudfront -> s3
  - ecs -> internet
  - ecs ~> cloudwatch
```

<!-- archwire-test: run exit=0 -->
```sh
archwire compile web.yaml -o web.drawio
```

The command writes `web.drawio`, prints a JSON report, and exits with 0. This is the diagram:

![The diagram archwire writes for web.yaml, from top to bottom: users, CloudFront, and inside the Tokyo Region frame a VPC with the Internet Gateway at its top, two Availability Zones side by side with a NAT gateway in each public subnet, the Application Load Balancer on the border between the zones, ECS and Aurora in the private subnets, and S3 and CloudWatch to the right of the VPC](assets/web-2az.png)

The VPC, the Internet Gateway, the subnets, the second Availability Zone, the NAT gateways and the Aurora writer and reader are not in the definition. archwire adds them, and each has a reason in the report and on the part in the `.drawio` file. Open the file in draw.io to look at it or edit it. An API without a VPC, a function in a VPC, and more are in [Examples](docs/examples.md).

## Install

archwire is a Rust program and needs a recent stable Rust toolchain. Get the repository and install from its top folder:

```sh
git clone https://github.com/sagochiko/archwire.git
cd archwire
cargo install --path crates/archwire
```

This puts `archwire` in Cargo's bin folder, usually `~/.cargo/bin`.

## Use

```sh
archwire compile <definition> -o <diagram.drawio>
archwire set <definition> <place> <value>
```

`compile` writes the diagram and prints the report as JSON on standard output. To keep the report, redirect it:

<!-- archwire-test: run exit=0 -->
```sh
archwire compile web.yaml -o web.drawio > web.json
```

| Exit code | Meaning |
|---|---|
| 0 | The diagram was written and nothing is left undecided. |
| 1 | The definition or a file has an error. No diagram is written, and a file left at the `-o` path by an earlier compile is removed, or reported as `CANNOT_WRITE` when it cannot be. The report is still printed. |
| 2 | The diagram was written, but at least one hole is left. |

A hole is a decision the connections alone cannot make. Add `- ecs -> s3` to `web.yaml` and compile again: traffic from ECS to S3 can leave the VPC through a VPC endpoint or a NAT gateway, so the report has the hole `route.ecs.s3` with the choices `endpoint` and `nat`. The diagram is still written, with that line grey and dashed, and the exit code is 2. The other kind of hole is the kind of an API Gateway written without `(http)` or `(rest)`.

<!-- archwire-test: append file=web.yaml line="  - ecs -> s3" -->
<!-- archwire-test: compile file=web.yaml exit=2 holes=route.ecs.s3 -->

`set` rewrites one place in the definition and leaves every other line as it was. Answer the hole with it and compile again; the exit code is 0:

<!-- archwire-test: run exit=0 -->
```sh
archwire set web.yaml choose.route.ecs.s3 endpoint
archwire compile web.yaml -o web.drawio
```

How to write a definition and answer each hole is in [Writing a definition](docs/definition.md). The report's fields and the reasons stored in the diagram are in [Reading the report](docs/report.md).

## Supported architectures

- A Web architecture with an Application Load Balancer, ECS or EC2, and Aurora, in one VPC with two or three Availability Zones
- An API without a VPC, with CloudFront, S3, API Gateway, Lambda, DynamoDB and Cognito
- Both in one definition, with a Lambda function that reaches Aurora drawn in the VPC
- A flow from detection to notification, from GuardDuty, Inspector, Security Hub, Config or CloudTrail to EventBridge, SNS, Step Functions or Systems Manager
- Log analysis with Security Lake, Athena and the Glue Data Catalog
- A KMS key attached to an S3 bucket or a DynamoDB table
- Route 53 attached to a CloudFront distribution, drawn as an association because DNS only answers the name and the users' traffic does not pass through it
- Events between EventBridge, SNS, SQS, Lambda and S3, such as a queue between two functions, a topic fanned out to queues, event notifications from a bucket, the stream of a DynamoDB table, or events a function sends to an event bus
- EC2 instances in private subnets that start flows of their own

A diagram has at most one VPC. The Region is `ap-northeast-1`, the only one in this version. Labels are in English, or in Japanese with `lang: ja` under `set`. Diagrams run from top to bottom, or from left to right with `direction: right`. Every service and setting is listed in [Writing a definition](docs/definition.md#services).

## Limitations

archwire checks the services and connections against what this version supports. Only the connections listed in [Which connections are accepted](docs/definition.md#which-connections-are-accepted) are accepted, and anything else stops with `UNSUPPORTED`.

A crowded diagram can have lines that cannot avoid every icon, label and other line. The diagram is still written, and the report has a note naming each such line. Past a point, the layout stops with `LAYOUT_FAILED`. The measured counts, such as how many services one load balancer takes, and what this version does not draw are in [Limits](docs/limits.md).

A diagram that compiles is consistent under these rules. It does not mean the architecture is guaranteed to work in AWS.

## Documentation and contributing

- [Writing a definition](docs/definition.md), [Reading the report](docs/report.md), [Examples](docs/examples.md), [Limits](docs/limits.md), [Rules](docs/rules.md)
- [Contributing](CONTRIBUTING.md), for the tests, the diagram checks, and how to report a problem
- [Issues](https://github.com/sagochiko/archwire/issues)

## License

Apache License 2.0, see [LICENSE](LICENSE). Where some of the style strings, colors and shape names come from is in [NOTICE](NOTICE).

archwire is not an official AWS tool and is not affiliated with Amazon Web Services.
