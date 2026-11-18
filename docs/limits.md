# Limits

This page is for archwire 0.1.0 with rules data 2026.11.9. The counts below were measured on that version by compiling each definition and counting the notes that name a line (see [Notes](report.md#notes)). Notes that explain the drawing, such as the one on auxiliary flows or on three zones, may be there at any count and are not counted.

A note naming a line means the diagram is still written, but that line could not avoid every icon, label and other line; look at it in the diagram. `LAYOUT_FAILED` means no diagram is written. Writing fewer parts avoids both. They are limits of this layout, not of AWS.

## Services behind one load balancer

Every service behind the load balancer is copied into each zone, and the load balancer sends a line to each copy. Start from this definition and add `- alb -> ecs.sNN` lines one at a time:

<!-- archwire-test: compile exit=0 -->
```yaml
flows:
  - user -> cloudfront -> alb -> ecs.s01 -> aurora
  - alb -> ecs.s02
  - alb -> ecs.s03
```

| Zones | No note naming a line | Notes naming lines | `LAYOUT_FAILED` |
|---|---|---|---|
| 2 | up to 6 services | from 7 | from 51 |
| 3 (`azs: 3`) | up to 3 services | from 4 | from 34 |

The counts are the same from top to bottom and from left to right. See [issue #11](https://github.com/sagochiko/archwire/issues/11).

## Several load balancers

Start from `- user -> alb.b01 -> ecs.b01` and add lines `- user -> alb.b02 -> ecs.b02` and so on, with two or three zones and in both directions. Up to seven load balancers are drawn without a note naming a line. From eight, the diagram is drawn with such notes, and from 51 the layout stops with `LAYOUT_FAILED` ([issue #16](https://github.com/sagochiko/archwire/issues/16)).

With a certificate and a web ACL attached to each load balancer (`- acm.b01 -- alb.b01` and `- waf.b01 -- alb.b01`, and so on), up to three load balancers are drawn without a note naming a line, and from four the diagram is drawn with such notes. The compile also gets slower with each load balancer added ([issue #17](https://github.com/sagochiko/archwire/issues/17)).

With two zones in a top-to-bottom diagram, the row of load balancers and the Internet Gateway sit over the second zone, and the space over the first zone stays empty ([issue #13](https://github.com/sagochiko/archwire/issues/13)).

In a VPC without private subnets, a line from a load balancer to a part outside the VPC, such as `alb.a ~> cloudwatch` with `alb.b` to its right, leaves from the right side of the load balancer and crosses the load balancer to its right, and a note names the line. The dashed line of a certificate or web ACL attached to that load balancer does the same ([issue #14](https://github.com/sagochiko/archwire/issues/14)).

## Diagrams that are harder to read than they could be

Each of these diagrams is correct.

- In a top-to-bottom diagram, WAF and Certificate Manager attached to CloudFront stand one under the other to its right, which makes the row of global services tall ([issue #4](https://github.com/sagochiko/archwire/issues/4)).
- An auxiliary line from the load balancer to a part right of the VPC, such as `alb ~> s3`, can go round the far side of that part and enter it from the right ([issue #5](https://github.com/sagochiko/archwire/issues/5)).
- With two zones, each NAT gateway sits to the right of the ECS copy in its zone, so in the first zone the line from the load balancer to ECS crosses the line from ECS to the NAT gateway ([issue #6](https://github.com/sagochiko/archwire/issues/6)).
- In a left-to-right diagram, S3 fed by CloudFront sits below the VPC, so the line to it is long ([issue #15](https://github.com/sagochiko/archwire/issues/15)).

## What this version does not draw

A connection or shape not listed in [Which connections are accepted](definition.md#which-connections-are-accepted) stops with `UNSUPPORTED`. These are not drawn in this version:

- A Lambda function is drawn in a VPC only in its private subnets, and only when it sends a main flow to Aurora. The line from a function to Aurora is drawn directly; RDS Proxy is not drawn.
- A route answered with `endpoint` adds no VPC endpoint icon.
- The endpoint types of API Gateway (edge-optimized, Regional, private) and WebSocket APIs are not drawn. WAF does not attach to API Gateway; `waf` attaches only to `cloudfront` and `alb`. Which API types support AWS WAF is in the AWS comparison [Choose between REST APIs and HTTP APIs](https://docs.aws.amazon.com/apigateway/latest/developerguide/http-api-vs-rest.html).
- Cognito identity pools are not drawn; a user's upload straight to S3 is one line, `user -> s3`.
- A KMS key attaches only to S3 and DynamoDB (`kms -- s3`, `kms -- dynamodb`). It cannot be attached to Aurora, ECS, Lambda or other services, and AWS Secrets Manager and multi-Region keys are not drawn.
- Queues other than the one Security Lake notifies through, the subscribers of Security Lake, Lake Formation, rollup Regions, Glue crawlers or jobs, the kinds of SNS topics (standard or FIFO), Lambda functions invoked by EventBridge or SNS, and AI services are not drawn.
- More than one VPC, Transit Gateway, on-premises networks, several accounts and other clouds are not drawn.
- archwire does not simulate failures, show migration stages, compare a diagram with Terraform, or read a `.drawio` file back into a definition.
