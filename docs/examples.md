# Examples

Each example is a definition, the command that compiles it, the diagram, and the exit code to expect. Save the definition under the name given and run the command in the same folder.

## A Web architecture

This is the example in the [README](../README.md#example). Save this as `web.yaml`:

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

The exit code is 0.

![The diagram archwire writes for web.yaml, from top to bottom: users, CloudFront, and inside the Tokyo Region frame a VPC with the Internet Gateway at its top, two Availability Zones side by side with a NAT gateway in each public subnet, the Application Load Balancer on the border between the zones, ECS and Aurora in the private subnets, and S3 and CloudWatch to the right of the VPC](../assets/web-2az.png)

Four lines of definition produced the VPC, the Internet Gateway, the public and private subnets, the second Availability Zone, the NAT gateways, and the Aurora writer and reader. None of them were written in the definition. Each has a reason in the report, and the same reason is stored on the part inside the `.drawio` file (see [Reading the report](report.md)).

In this example these elements carry a reason: the AWS Cloud frame, the Region frame, the VPC frame, both Availability Zone frames, all four subnet frames, the Internet Gateway, both NAT gateways, and the parts whose place archwire chose (CloudFront, ECS, Aurora, S3 and CloudWatch). The users and the load balancer carry none.

To use three Availability Zones, change one setting and compile again:

<!-- archwire-test: run exit=0 -->
```sh
archwire set web.yaml set.azs 3
archwire compile web.yaml -o web.drawio
```

`set` prints nothing when it succeeds. It adds a `set:` section with `azs: 3` and leaves every other line as it was. The exit code of the compile is 0.

## An API without a VPC

A definition that writes no VPC service is drawn without a VPC. Save this as `api.yaml`:

<!-- archwire-test: compile exit=0 file=api.yaml -->
```yaml
flows:
  - user -> cloudfront -> s3
  - user -> apigateway(rest) -> lambda -> dynamodb
  - cognito -- apigateway
  - lambda ~> cloudwatch
  - waf -- cloudfront
  - acm -- cloudfront
set:
  lang: en
```

<!-- archwire-test: run exit=0 -->
```sh
archwire compile api.yaml -o api.drawio
```

The exit code is 0.

![The diagram archwire writes for api.yaml, from top to bottom: users, CloudFront with AWS WAF and Certificate Manager to its right, and inside the Tokyo Region frame S3 under CloudFront, and API Gateway (REST API) with Cognito to its right, Lambda with CloudWatch to its right, and DynamoDB](../assets/api.png)

The line from Cognito is labelled "Cognito authorizer" because the API is a REST API; with `apigateway(http)` it reads "JWT authorizer". Certificate Manager carries `us-east-1` under its name. The report gives the reasons for the AWS Cloud and Region frames, the Region, the place of each part, and the authorizer.

## A function in a VPC

A Lambda function that sends a main flow to Aurora is attached to the VPC Aurora is in. Save this as `mix.yaml`:

<!-- archwire-test: compile exit=2 file=mix.yaml case=mix-vpc-ja holes=route.lambda.s3 -->
```yaml
flows:
  - user -> apigateway(http) -> lambda -> aurora
  - lambda -> s3
  - lambda ~> cloudwatch
set:
  lang: ja
```

<!-- archwire-test: run exit=2 holes=route.lambda.s3 -->
```sh
archwire compile mix.yaml -o mix.drawio
```

The exit code is 2, because the route from the function to S3 is left open as the hole `route.lambda.s3` (see [choose](definition.md#choose)). The diagram is still written, with labels in Japanese:

![The diagram archwire writes for mix.yaml, from top to bottom: users, and inside the Tokyo Region frame API Gateway (HTTP API), then a VPC with two Availability Zones side by side, each with a private subnet holding a copy of Lambda above Aurora, the writer in the first zone and the reader in the second, and S3 and CloudWatch to the right of the VPC. The line to S3 is grey and dashed and labelled as an undecided route](../assets/mix-vpc.png)

The function is drawn once in the private subnet of each zone, and each copy reads and writes the writer and reads the reader. API Gateway, S3 and CloudWatch stay outside the VPC. The VPC has only private subnets, because no part here is placed in a public subnet. The report gives the reasons: the VPC because an Aurora DB cluster can be created only in a VPC, two zones because its DB subnet group covers at least two, and the function in the private subnets because it is attached to the VPC to reach Aurora. This definition and its expected output are the acceptance case `tests/cases/mix-vpc-ja`.

Answer the hole and compile again:

<!-- archwire-test: run exit=0 holes= -->
```sh
archwire set mix.yaml choose.route.lambda.s3 endpoint
archwire compile mix.yaml -o mix.drawio
```

`set` adds a `choose:` section with `route.lambda.s3: endpoint`. The exit code of the compile is 0. archwire adds no VPC endpoint icon; the line to S3 is drawn solid like the other main flows, its label says the traffic goes through a VPC endpoint, and the report's `notes` says that endpoint icons are not drawn. With `nat` instead of `endpoint`, archwire adds a public subnet and a NAT gateway in each zone, and the Internet Gateway.

## Direction

The diagrams above run from top to bottom, which is what you get when the definition says nothing about direction. To draw the main flows from left to right instead, add `direction: right` under `set`. The inside of a VPC stays the same either way: the Availability Zones side by side, the public subnets above the private ones. `api.yaml` with `direction: right` is the acceptance case `tests/cases/api-rest-en`, and `tests/cases/web-2az-ja-right` and `tests/cases/mix-vpc-ja-right` show a VPC drawn from left to right. Where each part goes is described in [How the parts are laid out with a VPC](definition.md#how-the-parts-are-laid-out-with-a-vpc).

## More shapes

Every folder under `tests/cases` holds a definition (`def.yaml`) and the output archwire must give for it, byte for byte. These cover the other shapes:

| Case | Shape |
|---|---|
| `detect-hub-ja`, `detect-respond-en`, `detect-trail-en` | a flow from detection to notification, with no users |
| `security-lake-en`, `trail-athena-ja` | log analysis with Security Lake, Athena and the Glue Data Catalog |
| `ec2-ssm-en`, `ec2-internet-en`, `web-ec2-ja` | EC2 instances in private subnets |
| `web-two-albs-ja`, `web-alb-attach-en` | two load balancers, and a certificate and web ACL attached to a load balancer |
| `signin-api-ja`, `direct-upload-en` | users signing in to Cognito, and uploading straight to S3 |
| `api-kms-ja`, `web-kms-en` | a KMS key attached to S3 and DynamoDB without a VPC, and to S3 in a Web architecture with a VPC |
| `err-unknown-service`, `err-unsupported` | definitions that stop with an error |
