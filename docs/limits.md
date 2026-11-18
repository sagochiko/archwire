# Limits

This page is for archwire 0.1.0 with rules data 2026.11.11. The counts below were measured on that version by compiling each definition and counting the notes that name a line (see [Notes](report.md#notes)). Notes that explain the drawing, such as the one on auxiliary flows or on three zones, may be there at any count and are not counted.

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

- In a top-to-bottom diagram, WAF, Certificate Manager and Route 53 attached to CloudFront stand one under the other to its right, which makes the row of global services tall ([issue #4](https://github.com/sagochiko/archwire/issues/4)).
- An auxiliary line from the load balancer to a part right of the VPC, such as `alb ~> s3`, can go round the far side of that part and enter it from the right ([issue #5](https://github.com/sagochiko/archwire/issues/5)).
- With two zones, each NAT gateway sits to the right of the ECS copy in its zone, so in the first zone the line from the load balancer to ECS crosses the line from ECS to the NAT gateway ([issue #6](https://github.com/sagochiko/archwire/issues/6)).
- In a left-to-right diagram, S3 fed by CloudFront sits below the VPC, so the line to it is long ([issue #15](https://github.com/sagochiko/archwire/issues/15)).
- In a left-to-right Japanese diagram with two zones, EC2 above Aurora and no public subnet, the label of a line from EC2 to S3 or Systems Manager through a VPC endpoint is written on one line and crosses the sides of the private subnet, the zone and the VPC. The gap between the VPC and the part right of it is too narrow for the label stacked in three lines.

## What this version does not draw

A connection or shape not listed in [Which connections are accepted](definition.md#which-connections-are-accepted) stops with `UNSUPPORTED`. These are not drawn in this version:

- A Lambda function is drawn in a VPC only in its private subnets, and only when it sends a main flow to Aurora. The line from a function to Aurora is drawn directly; RDS Proxy is not drawn.
- A route answered with `endpoint` adds no VPC endpoint icon.
- The endpoint types of API Gateway (edge-optimized, Regional, private) and WebSocket APIs are not drawn. WAF does not attach to API Gateway; `waf` attaches only to `cloudfront` and `alb`. Which API types support AWS WAF is in the AWS comparison [Choose between REST APIs and HTTP APIs](https://docs.aws.amazon.com/apigateway/latest/developerguide/http-api-vs-rest.html).
- Cognito identity pools are not drawn; a user's upload straight to S3 is one line, `user -> s3`.
- Route 53 attaches only to CloudFront (`route53 -- cloudfront`). It cannot be attached to a load balancer or API Gateway, and it cannot be written on a main or auxiliary flow, such as `user -> route53` or `route53 -> cloudfront`. Hosted zone types, health checks and routing policies are not drawn.
- A KMS key attaches only to S3 and DynamoDB (`kms -- s3`, `kms -- dynamodb`). It cannot be attached to Aurora, ECS, Lambda or other services, and AWS Secrets Manager and multi-Region keys are not drawn.
- Events between EventBridge, SNS, SQS, Lambda and S3 are drawn only along the pairs in the list. The kinds of SNS topics and SQS queues (standard or FIFO) and dead-letter queues are not drawn. `s3 -> eventbridge` is not accepted, and ECS and EC2 do not send to queues or topics; only a Lambda function does (`lambda -> sqs`, `lambda -> sns`).
- A main flow back to a part it left through a part outside the subnets of the VPC, such as `lambda.a -> sqs -> lambda.a`, stops with `UNSUPPORTED`.
- The settings of Security Lake's subscribers are not drawn; a function that reads the queue Security Lake notifies through can be drawn after it, as `securitylake -> sqs -> lambda`. Lake Formation, rollup Regions, Glue crawlers or jobs, and AI services are not drawn.
- More than one VPC, Transit Gateway, on-premises networks, several accounts and other clouds are not drawn.
- archwire does not simulate failures, show migration stages, compare a diagram with Terraform, or read a `.drawio` file back into a definition.
