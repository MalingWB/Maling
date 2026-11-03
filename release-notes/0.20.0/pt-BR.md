## Novidades e ajustes

### 1. Intervenção remota e notificações pelo Feishu

Agora você pode aprovar permissões, responder a perguntas do Agent, realizar verificações manuais e receber notificações de resultados pelo Feishu, mesmo longe do computador.

Para começar:

1. Abra “Configurações → Geral → Intervenção remota e notificações por IM”, selecione Feishu, confirme as permissões e escaneie o código QR para conectar.
2. Envie uma mensagem privada ao bot usando a conta que autorizou a conexão pelo código QR para concluir o vínculo.
3. Ao receber um cartão, você pode aprovar ou negar permissões, responder com uma ou várias opções, texto ou uma resposta personalizada, ou realizar uma verificação manual.

Depois que uma solicitação for resolvida no aplicativo de desktop, no WeCom ou no Feishu, o resultado será sincronizado com os outros canais conectados que tenham a notificação correspondente ativada. Os cartões do Feishu mantêm as respostas efetivamente enviadas e desativam os controles já utilizados.

No momento, apenas conversas privadas individuais com a conta vinculada são aceitas, e cada bot pode se conectar a um único dispositivo. Não há suporte a grupos nem a iniciar, parar ou executar novamente tarefas por IM. O recebimento e o envio em tempo real não são garantidos quando o cliente está offline, o computador está suspenso ou a conexão de rede está indisponível. A versão internacional, Lark, ainda não foi validada.

> [!WARNING]
> **Antes de atualizar, confira as limitações de cartões antigos e de retorno a versões anteriores**
> Esta atualização remove os registros de entrega dos cartões interativos de IM antigos e os vínculos de mensagens associados. Alguns cartões recebidos antes da atualização podem deixar de funcionar; resolva as solicitações ainda pendentes no aplicativo de desktop. Os dados de IM atualizados não podem ser lidos diretamente por clientes antigos. Se precisar manter a possibilidade de voltar a uma versão anterior, faça um backup dos dados do aplicativo antes de atualizar.

### 2. Melhorias na exibição de sub-Agents nativos do Claude e de tarefas em segundo plano

Quando o Claude inicia um sub-Agent, você pode acompanhar seu estado na linha Agent da conversa e abrir detalhes somente para leitura no espaço de trabalho à direita. As subtarefas executadas de forma síncrona ou em segundo plano exibem suas próprias mensagens, solicitações de permissão e estados. O fim do turno principal não faz mais com que subtarefas ainda em execução sejam marcadas incorretamente como interrompidas.

A atividade em segundo plano tem uma contagem de tempo separada, reduzindo a inclusão de longos períodos ociosos no tempo de execução. Também é possível copiar e citar normalmente as respostas depois que a atividade em segundo plano termina.

> [!NOTE]
> É necessário Claude ACP `0.90.0` ou posterior; versões antigas exibem uma solicitação de atualização. A continuação de sub-Agents existentes depois de recarregar a sessão ainda depende das capacidades do adaptador. O total de Token não inclui o consumo das reativações em segundo plano; o painel exibe um aviso sobre essa limitação.

### 3. Melhorias na execução de requisitos remotos e na transferência de resultados

A página “Requisitos” da edição interna (WB) aprimora o tratamento de tarefas de desenvolvimento, tarefas de teste e anexos remotos:

1. Visualize e execute os requisitos por tarefas de desenvolvimento ou de teste. Se a tarefa de desenvolvimento correspondente ainda não tiver terminado, a tarefa de teste exibirá uma recomendação para aguardar, mas continuará permitindo a execução manual.
2. Ao clicar em “Executar”, o conteúdo do requisito, as instruções de entrega das tarefas anteriores e outros contextos são incluídos na conversa. A tarefa só é assumida oficialmente quando você clica em “Enviar”.
3. Antes de enviar, você pode consultar a lista de anexos remotos, somente para leitura. Eles são baixados ao enviar e usados como entradas da tarefa. A falha no download de um anexo não bloqueia a tarefa, e o Agent é informado sobre o conteúdo ausente.
4. O modo Direct aguarda uma saída de conclusão explícita antes de informar que a tarefa foi concluída, evitando encerrar o requisito logo após a primeira resposta. A etapa final do fluxo de trabalho resume as entregas de todo o item de trabalho para uso nas tarefas seguintes.

### 4. Sincronização bidirecional de Skills remotos com vários arquivos

Agora é possível sincronizar Skills completos entre o ambiente local e o Multica, incluindo `SKILL.md` e arquivos de apoio, como scripts e documentação. Na página de gerenciamento global de Skills, clique em “Sincronizar SKILLs remotos”, conecte-se ao Multica e selecione um espaço de trabalho e um Skill remoto para baixá-lo e consultar o resultado da sincronização. Para importar um Skill local para o ambiente remoto, inicie a operação no Multica.

Corrigidos problemas que mantinham seleções antigas durante o recarregamento da lista remota e deixavam a recuperação incompleta após falhas de sincronização.

> [!CAUTION]
> **Baixar com substituição troca todo o diretório do Skill**
> Ao optar por substituir um Skill local, o conteúdo do diretório remoto passa a prevalecer. Arquivos locais que não existam no remoto também serão excluídos. Salve antes as alterações locais que deseja manter.

### 5. Atalho para abrir a busca de conversas

Pressione `Ctrl+F`, ou `Cmd+F` no macOS, para abrir a busca de conversas e colocar o foco no campo de pesquisa. Pressionar novamente seleciona o termo atual. Pressione `Esc` para fechar e devolver o foco ao controle anterior.

O editor de código mantém a prioridade de sua própria busca no arquivo. A busca de conversas não abre sobre outros diálogos, menus ou seletores que já estejam abertos.

### 6. Confirmação do destino antes de fazer push no Git

Antes de fazer push, o controle de código-fonte mostra o destino real no formato “branch local → remoto / branch de destino”, para você conferir para onde o código será enviado. Ao publicar um branch pela primeira vez, você pode escolher o remoto e definir o upstream. Branches que já tenham um upstream fazem push para o destino que realmente acompanham.

Branches sem upstream recebem o rótulo “Novo branch”, reduzindo a confusão entre branches ainda não publicados e branches já sincronizados.

## Melhorias de experiência

### 1. Leitura do histórico e retomada de sessões mais estáveis

Corrigido o problema em que a rolagem automática puxava você de volta ao final enquanto lia mensagens anteriores. Também é possível carregar mais histórico quando a página atual não preenche a tela. Ao voltar a uma sessão, perguntas já respondidas não retornam ao estado pendente por causa de dados antigos em cache.

Ao continuar uma sessão, são usados o modelo e as permissões realmente salvos nela, reduzindo falhas causadas por configurações antigas do fluxo de trabalho.

### 2. Anexos com o mesmo nome não sobrescrevem mais o histórico

Anexos com nomes iguais e conteúdos diferentes são salvos separadamente. Novos envios não sobrescrevem anexos anteriores nem suas prévias. O salvamento de nomes longos foi aprimorado, com uma mensagem clara quando o caminho é profundo demais para armazenar o arquivo com segurança.

### 3. Melhorias no Markdown e no espaço de trabalho de arquivos

1. Corrigidas as prévias de imagens locais e a abertura de links HTML locais nas conversas, preservando os parâmetros de consulta e as âncoras da página.
2. A prévia em tempo real de Markdown usa a largura disponível do painel, reduzindo margens laterais desnecessárias em telas largas.
3. O tamanho padrão da fonte do editor nos temas integrados passou para `13px`. Tamanhos personalizados existentes são mantidos.
4. Se ocorrer um erro de renderização em uma aba à direita, você pode tentar novamente nessa aba, sem afetar toda a conversa. Também foi melhorada a estabilidade quando várias árvores de arquivos estão presentes ao mesmo tempo.

### 4. Correção da contagem duplicada de Token

Corrigida a contagem duplicada do consumo de Token ao continuar uma conversa após um período ocioso. Duplicações identificáveis também são corrigidas ao ler registros existentes.

### 5. Outras melhorias

1. Corrigida a falha no carregamento das estatísticas de alterações ao visualizar commits com muitos arquivos ou commits de merge no Windows.
2. Aprimorada a detecção automática do idioma chinês: marcadores explícitos de chinês simplificado ou tradicional têm prioridade sobre a região. Escolhas manuais de idioma existentes são mantidas.
3. Completadas as traduções relacionadas a sessões mantidas ativas e corrigido o fechamento tardio de algumas dicas exibidas ao passar o mouse.
