儲存未通過，正在修正 {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Forge Spec 修正">
你上一輪輸出的 Forge Spec 增量合併後沒有通過儲存前的校驗。請修復後重新輸出增量，不要改變使用者已經確認的結論。

結構化錯誤（錯誤碼 → 明細，已一次列全）：

{{ errors }}

修復要求：

1. 只修復錯誤指向的 stage 與欄位，其餘內容保持原樣；對照「目前基線 Spec」定位出錯的 stage，修改時帶上它的 `id`；
2. 以與之前相同的增量 JSON 結構輸出修復後的增量（單一程式碼圍欄，仍是增量、不是全量）；能修復的問題在這一輪全部修完，不要一輪只修一處；
3. 結構類錯誤直接修復：
   - `forge/unknown-failure-target`：失敗去向指向了不存在的階段。改成 `"retry"`，並把「這個階段失敗後應該去哪裡」連同你的推薦答案寫入 `openQuestions`；
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: 同名角色的所有階段使用基線中同一份職責定義；省略 `profileId`，由系統綁定角色。
   - `forge/readonly-role`: 角色庫參照若無可編輯職責即為唯讀：保留名稱，省略 `roleDefinition`。
4. 需要使用者決定的問題不要編造答案：把它連同你的推薦答案寫入 `openQuestions`，並在正文中向使用者說明。這類問題包括：
   - `forge/missing-goal-element`、`forge/unresolved-questions`：階段目標缺要素，或還有待釐清項沒有落實；
   - `forge/duplicate-workflow-name`：工作流名稱已被占用。推薦一個能區分的新名稱請使用者確認，不要自行改名。
</hidden>
