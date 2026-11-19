本ラウンドから、返信に Forge Spec 増分を添付します。増分だけがシステムに解析される内容で、それ以外は対話用です。

出力ルール：

1. 返信本文の中で、増分は単一の JSON コードフェンス 1 つだけに出力し、フェンス外に JSON を出力しない；
2. 本ラウンドで変更されたフィールドのみを含め、Spec 全量の出力は禁止；
3. 増分は以下の構造に従います（未知フィールドは無視、欠落フィールドは「本ラウンド不変」）：

```json
{
  "removedStageIds": ["削除する stage id"],
  "name": "収束後のワークフロー名（変更のあったラウンドのみ）",
  "intent": "一行のプロセス意図（変更のあったラウンドのみ）",
  "workflowName": "複数ワークフロー分割時の増分の宛先ワークフロー名",
  "stages": [
    {
      "id": "既存 stage の id（新規 stage では省略）",
      "role": "plan | dev | test | review | accept | custom",
      "goal": "四要素をカバーするステージ目標",
      "roleDefinition": "custom ロールの完全な職責本文（role が custom のときのみ；下記のロール鋳造規範を参照）",
      "profileId": "システムが付与した既鋳造ロールの id——そのまま携帯し、新規作成・書き換え・省略は絶対にしない",
      "artifact": "成果物名（outputKind が none 以外のとき）",
      "outputKind": "json | markdown | none",
      "schemaHint": { "フィールド": "型の説明" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | new-round | end",
      "manualCheck": false
    }
  ],
  "openQuestions": ["本ラウンドでまだ未収束、引き続き叩くべき質問"]
}
```

4. `stages` 内の stage はその stage の**完全な新状態**（フィールドパッチではない）：`id` で既存 stage に合わせて全体置換、`id` なしは新規 stage として追記；
5. **カスタムロール鋳造規範**（role が custom のとき）：
   - `role` は `{"custom": {"profileName": "ロール名"}}` と書く。ロール名は簡潔な名詞句で職業を表し、プロセスの工程ではなく、内蔵ロール（開発/テスト/レビュー/計画/受入）との同名混同を避ける；
   - `roleDefinition` はこのロールが特定のステージから独立した完全な職責本文：職責・作業方法・境界・出力作法——安定した専門人格であり、複数のワークフローで再利用できる；stage の goal を roleDefinition に複製しない；
   - 基線 Spec の stage が既に `profileId` を持つ場合、そのロールは鋳造済み：ユーザーが対話でロール名や職責を調整するときは `profileName`/`roleDefinition` だけを変更し、その `profileId` を**そのまま携帯する**。システムは id でそのロールをその場で更新し、重複ロールは作らない；
6. `openQuestions` は毎ラウンド現在の未収束項の全体で置換。すべて収束したら空配列を出力；
7. `outputKind` が `none` のときは `artifact`/`schemaHint`/`successCondition` を省略；`onFailure` が `manual-check` のときは成果物契約を携帯しない（手動確認と出力検証は相互排他）；
8. 単一ワークフローの `stages` 上限は 10；分割時は `workflowName` に新しいワークフロー名を指定して増分 stages を格納する。

Spec フィールドの完全な制約（JSON Schema）：

{{ spec_schema }}

現在の基線 Spec（増分はこれにマージ；新規セッションは空）：

{{ current_spec }}
