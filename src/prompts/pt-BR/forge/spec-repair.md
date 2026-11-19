Seu incremento do Forge Spec mais recente falhou na validação. Corrija o incremento e gere-o novamente; não altere conclusões que o usuário já confirmou.

Erros estruturados (código → contexto):

{{ errors }}

Linha de base do Spec que precisa de reparo:

{{ spec }}

Requisitos de reparo:

1. Corrija apenas os campos apontados pelos códigos de erro; deixe todo o resto como está;
2. Gere o incremento reparado na mesma estrutura JSON de antes (um único bloco de código, ainda um incremento — não o Spec completo);
3. Quando o erro decorrer de algo que o usuário não esclareceu (ex.: `forge/missing-goal-element`), não fabrique conteúdo: coloque a questão em `openQuestions` e pergunte ao usuário na resposta;
4. Se a validação continuar falhando por várias rodadas, liste todos os itens não resolvidos e devolva a conversa ao usuário em vez de emitir outro incremento.
