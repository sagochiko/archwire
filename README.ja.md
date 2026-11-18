# archwire

[English](README.md)

archwire は、サービスどうしのつながりを短く書いた定義から、AWS の構成図を draw.io の形式で組み立てる。AI ではなく規則で動く。同じ定義を同じ版の archwire と同じ規則のデータで変換すれば、macOS でも Linux でも1バイトも違わない同じ図が出る。

archwire が自分で足した部品や設定には、レポートと図の両方に理由が付く。定義に書いていない設定は archwire の既定値で補い、そのことを書き残す。つながりだけでは構成が決まらないところは archwire が選ばず、穴としてレポートに返す。答えは定義に書く。これは 0.1.0 の版。

## 例

次の内容を `web.yaml` として保存する。

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

コマンドは `web.drawio` を書き出し、JSON のレポートを表示し、終了コード0で終わる。図はこうなる。

![archwire が web.yaml から書き出す図。上から順に、利用者、CloudFront があり、東京リージョンの枠の中に VPC がある。VPC の上端に Internet Gateway があり、2つの Availability Zone が横に並び、それぞれのパブリックサブネットに NAT gateway がある。ゾーンの境目に Application Load Balancer、プライベートサブネットに ECS と Aurora があり、VPC の右に S3 と CloudWatch がある](assets/web-2az.png)

VPC、Internet Gateway、サブネット、2つ目の Availability Zone、NAT gateway、Aurora のライターとリーダーは、どれも定義に書いていない。archwire がそれらを足し、それぞれの理由をレポートと `.drawio` ファイルの中の部品に入れる。ファイルは draw.io で開いて見たり直したりできる。VPC の無い API、VPC に入る関数などの例は [Examples](docs/examples.md) にある（英語）。

## インストール

archwire は Rust のプログラムで、新しめの安定版の Rust ツールチェーンが要る。リポジトリを取ってきて、その直下からインストールする。

```sh
git clone https://github.com/sagochiko/archwire.git
cd archwire
cargo install --path crates/archwire
```

`archwire` は Cargo の bin フォルダに入る。ふつうは `~/.cargo/bin` になる。

## 使い方

```sh
archwire compile <definition> -o <diagram.drawio>
archwire set <definition> <place> <value>
```

`compile` は図を書き出し、レポートを JSON で標準出力に出す。レポートを残すなら、出力を振り向ける。

<!-- archwire-test: run exit=0 -->
```sh
archwire compile web.yaml -o web.drawio > web.json
```

| 終了コード | 意味 |
|---|---|
| 0 | 図を書き出し、決まっていないところは無い。 |
| 1 | 定義かファイルに誤りがある。図は書き出さず、前の変換で `-o` の場所に残ったファイルを消す。消せないときは `CANNOT_WRITE` として報告する。レポートは出す。 |
| 2 | 図は書き出したが、穴が1つ以上残っている。 |

穴とは、つながりだけでは決められないところのこと。`web.yaml` に `- ecs -> s3` を足して変換し直すと、ECS から S3 への通信は VPC エンドポイントからも NAT gateway からも VPC の外へ出られるので、レポートに穴 `route.ecs.s3` が選択肢 `endpoint` と `nat` 付きで載る。図は書き出され、その線は灰色の破線になり、終了コードは2になる。もう1種類の穴は、`(http)` も `(rest)` も付けずに書いた API Gateway の種類。

<!-- archwire-test: append file=web.yaml line="  - ecs -> s3" -->
<!-- archwire-test: compile file=web.yaml exit=2 holes=route.ecs.s3 -->

`set` は定義の1か所を書きかえ、ほかの行はそのまま残す。これで穴に答えて変換し直すと、終了コードは0になる。

<!-- archwire-test: run exit=0 -->
```sh
archwire set web.yaml choose.route.ecs.s3 endpoint
archwire compile web.yaml -o web.drawio
```

定義の書き方と穴ごとの答え方は [Writing a definition](docs/definition.md) にある。レポートの項目と、図に入る理由は [Reading the report](docs/report.md) にある（どちらも英語）。

## 描ける構成

- Application Load Balancer、ECS か EC2、Aurora を使う Web 構成。VPC は1つで、Availability Zone は2つか3つ
- VPC の無い API。CloudFront、S3、API Gateway、Lambda、DynamoDB、Cognito を使う
- その2つを1つの定義に並べた形。Aurora につなぐ Lambda 関数は VPC の中に描く
- 検出から通知への流れ。GuardDuty、Inspector、Security Hub、Config、CloudTrail から、EventBridge、SNS、Step Functions、Systems Manager へつなぐ
- Security Lake、Athena、Glue Data Catalog を使うログ分析
- S3 のバケットや DynamoDB のテーブルに付ける KMS のキー
- EventBridge、SNS、SQS、Lambda、S3 のあいだのイベント。2つの関数のあいだにキューを挟む形、トピックから複数のキューに分ける形、バケットのイベント通知など
- 自分から通信を始める、プライベートサブネットの EC2 インスタンス

1つの図に描ける VPC は1つまで。リージョンは `ap-northeast-1` で、この版ではほかを選べない。ラベルは英語で、`set` の下に `lang: ja` と書けば日本語になる。図は上から下へ流れ、`direction: right` と書けば左から右へ流れる。サービスと設定の一覧は [Writing a definition](docs/definition.md#services) にある。

## 制限

archwire は、この版が対応するサービスとつながりの範囲に入っているかを検査する。受け付けるのは [Which connections are accepted](docs/definition.md#which-connections-are-accepted) に並べたつながりだけで、それ以外は `UNSUPPORTED` で止まる。

込み合った図では、アイコン、ラベル、ほかの線を避けきれない線が出ることがある。図は書き出し、そうした線をレポートの注記で1本ずつ名指しする。ある数を超えると、配置が `LAYOUT_FAILED` で止まる。1つのロードバランサーの先に置けるサービスの数のような測った数と、この版で描かないものは [Limits](docs/limits.md) にある。

変換が通ったというのは、この規則の範囲で矛盾が無いという意味にとどまる。その構成が AWS で動くことは保証しない。

## 文書と開発への参加

- [Writing a definition](docs/definition.md)、[Reading the report](docs/report.md)、[Examples](docs/examples.md)、[Limits](docs/limits.md)、[Rules](docs/rules.md)
- テスト、図の検査、問題の報告の仕方は [Contributing](CONTRIBUTING.md) にある
- [Issues](https://github.com/sagochiko/archwire/issues)

## ライセンス

Apache License 2.0。[LICENSE](LICENSE) を参照。スタイルの文字列、色、図形の名前の一部の出どころは [NOTICE](NOTICE) にある。

archwire は AWS の公式ツールではなく、Amazon Web Services とは提携していない。
