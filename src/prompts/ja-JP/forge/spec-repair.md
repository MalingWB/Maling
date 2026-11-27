保存未通過のため修正中 {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Forge Spec 修復">
直近のラウンドで出力した Forge Spec 増分は、マージ後の保存前検証を通過しませんでした。修復して増分を再出力してください。ユーザーが既に確認した結論は変更しないでください。

構造化エラー（エラーコード → 明細、全件を一括掲載）:

{{ errors }}

修復要件:

1. エラーが指すステージとフィールドのみを修復し、それ以外は元のまま。「現在の基線 Spec」で該当ステージを特定し、修正時はその `id` を付ける；
2. 以前と同じ増分 JSON 構造で修復後の増分を出力（単一のコードフェンス、全量ではなく増分のまま）。修復可能な問題はこのラウンドで全て直し、1 ラウンドに 1 件にしない；
3. 構造系のエラーは直接修復する：
   - `forge/unknown-failure-target`：失敗時の行き先が存在しないステージを指しています。`"retry"` に変え、「このステージが失敗したらどこへ進むべきか」を推奨案とともに `openQuestions` に書く；
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: 同じロールを使う全ステージの職務定義を現在の基準に合わせて統一します。`profileId` は省略し、対応付けはシステムに任せます。
   - `forge/readonly-role`: 編集可能な職務定義がないライブラリ参照は読み取り専用です。名前はそのままにし、`roleDefinition` は省略します。
4. ユーザーが決めるべきことを勝手に答えを作らないでください。推奨案とともに `openQuestions` に書き、本文でユーザーに伝えます。該当するのは次です：
   - `forge/missing-goal-element`、`forge/unresolved-questions`：ステージ目標に要素が足りない、または未確認事項がまだ反映されていない；
   - `forge/duplicate-workflow-name`：ワークフロー名がすでに使われています。区別できる新しい名前を推奨してユーザーに確認を求め、自分で改名しない。
</hidden>
