O salvamento falhou; reparando {{ attempt }}/{{ max_attempts }}

<hidden data-gold-band-hidden="true" title="Reparo do Forge Spec">
O incremento do Forge Spec que você gerou na última rodada falhou na validação anterior ao salvamento, após a mesclagem. Corrija-o e gere o incremento novamente; não altere conclusões que o usuário já confirmou.

Erros estruturados (código → detalhe, todos listados de uma vez):

{{ errors }}

Requisitos de reparo:

1. Corrija apenas os estágios e campos apontados pelos erros; deixe todo o resto como está. Localize o estágio com falha na "Linha de base atual do Spec" e leve o `id` dele ao modificá-lo;
2. Gere o incremento reparado na mesma estrutura JSON de antes (um único bloco de código, ainda um incremento — não o Spec completo); corrija todos os problemas corrigíveis nesta mesma rodada em vez de um por rodada;
3. Corrija diretamente os erros estruturais:
   - `forge/unknown-failure-target`: o destino da falha aponta para um estágio que não existe. Mude para `"retry"` e coloque em `openQuestions`, com sua recomendação, a questão "para onde este estágio deve ir quando falha?";
   - `forge/role-id-conflict`, `forge/role-definition-conflict`: Use uma única definição da linha de base para todos os estágios com o mesmo papel. Omita `profileId`; o sistema resolve o vínculo.
   - `forge/readonly-role`: Referências à biblioteca sem definição editável são somente leitura: mantenha o nome e omita `roleDefinition`.
4. Nunca invente a resposta para algo que cabe ao usuário decidir: coloque em `openQuestions` com sua recomendação e diga isso na resposta. Isso cobre:
   - `forge/missing-goal-element`, `forge/unresolved-questions`: falta um elemento no objetivo do estágio, ou uma questão aberta ainda não foi resolvida;
   - `forge/duplicate-workflow-name`: o nome do workflow já está em uso. Recomende um nome que os distinga e peça confirmação ao usuário; não o renomeie por conta própria.
</hidden>
