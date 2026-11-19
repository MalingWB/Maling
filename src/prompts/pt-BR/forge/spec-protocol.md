A partir desta rodada, anexe o incremento do Forge Spec à sua resposta. O incremento é o único conteúdo que o sistema analisa; todo o resto é conversa.

Regras de saída:

1. Na resposta, o incremento desta rodada vai em um único bloco de código JSON; nenhum outro JSON fora dele;
2. Inclua apenas os campos alterados nesta rodada; nunca despeje o Spec completo;
3. O incremento segue esta estrutura (campos desconhecidos são ignorados; campos ausentes significam "inalterado nesta rodada"):

```json
{
  "removedStageIds": ["ids dos estágios a remover"],
  "name": "nome do workflow após a convergência (apenas em rodadas em que muda)",
  "intent": "intenção do processo em uma linha (apenas em rodadas em que muda)",
  "workflowName": "nome do workflow de destino do incremento ao dividir em vários",
  "stages": [
    {
      "id": "id de um estágio existente (omita para novo)",
      "role": "plan | dev | test | review | accept | custom",
      "goal": "objetivo do estágio cobrindo os quatro elementos",
      "roleDefinition": "carta completa do papel para papéis custom (apenas quando role é custom; veja as regras de forja abaixo)",
      "profileId": "id do papel já forjado atribuído pelo sistema — carregue-o literalmente; nunca invente, reescreva ou omita",
      "artifact": "nome do artefato (quando outputKind não é none)",
      "outputKind": "json | markdown | none",
      "schemaHint": { "campo": "descrição do tipo" },
      "successCondition": "$.field == value",
      "onFailure": "retry | manual-check | new-round | end",
      "manualCheck": false
    }
  ],
  "openQuestions": ["perguntas ainda não resolvidas que exigem mais interrogatório"]
}
```

4. Cada estágio em `stages` é o **estado novo completo** do estágio (não um patch de campos): os estágios se alinham por `id` e substituem por inteiro; estágio sem `id` é novo e anexado;
5. **Regras de forja de papéis custom** (quando role é custom):
   - escreva `role` como `{"custom": {"profileName": "Nome do papel"}}` — uma frase nominal concisa nomeando a profissão, não a etapa, e nunca duplicando os papéis internos (dev/test/review/plan/accept);
   - `roleDefinition` é a carta completa do papel, independente de qualquer estágio: responsabilidades, método, limites, convenções de saída — uma persona profissional estável reutilizável entre workflows; não copie o goal do estágio para dentro dela;
   - quando o estágio da linha de base já carrega um `profileId`, esse papel já foi forjado: ao ajustar nome ou carta na conversa, altere apenas `profileName`/`roleDefinition` **carregando esse `profileId` literalmente** — o sistema atualiza o papel no lugar por id e nunca o duplica;
6. `openQuestions` substitui a lista completa de pendências a cada rodada; array vazio quando tudo convergir;
7. Omita `artifact`/`schemaHint`/`successCondition` quando `outputKind` for `none`; nunca leve contrato de artefato quando `onFailure` for `manual-check` (revisão manual e validação de saída são mutuamente exclusivas);
8. Um workflow comporta no máximo 10 estágios; ao dividir, carregue os estágios do incremento sob um novo `workflowName`.

Restrições completas dos campos do Spec (JSON Schema):

{{ spec_schema }}

Linha de base atual do Spec (os incrementos se mesclam nela; vazia para uma nova sessão):

{{ current_spec }}
