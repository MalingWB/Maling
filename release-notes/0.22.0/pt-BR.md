## Recursos novos e atualizados

> [!IMPORTANT]
> Esta versão migra as configurações locais. A versão 0.21.x não consegue ler as configurações migradas. Se precisar manter a opção de voltar a uma versão anterior, faça backup dos dados do app antes de atualizar.

### 1. Tente o nó atual novamente ou recomece de um nó que falhou no gráfico do fluxo de trabalho

Abra o gráfico de execução do fluxo de trabalho na conversa e clique com o botão direito em um nó elegível:

1. **Tentar o nó atual novamente**: se o nó atual foi pausado por um problema e o fluxo ainda não avançou, execute o nó de novo na mesma execução (Run). A conversa e os artefatos anteriores são preservados.
2. **Executar novamente a partir deste nó**: selecione um registro de execução encerrado com falha, inclusive um registro anterior elegível. Um novo Run começa nesse nó, usando as configurações mais recentes do fluxo de trabalho salvas na tarefa. O Run original continua disponível como histórico somente leitura.

> [!WARNING]
> Recomeçar de um nó que falhou interrompe o que ainda estiver em andamento no Run original. A operação não desfaz alterações no espaço de trabalho, commits do Git nem ações externas já realizadas. As duas opções se limitam a nós comuns do fluxo de trabalho e não se aplicam ao modo AUTO nem a nós internos de AI-DYNAMIC.

### 2. Continue ou recupere fluxos de trabalho pelo Feishu e pelo WeCom

Depois de vincular uma conversa privada no Feishu ou no WeCom, o aplicativo envia um cartão de ação quando um fluxo de trabalho é pausado em um estado elegível, inclusive se tiver sido interrompido manualmente no app para desktop:

1. **Continuar o fluxo de trabalho**: retoma o nó atual se ele ainda não tiver terminado.
2. **Recuperar o fluxo de trabalho**: se o nó atual terminou com sucesso, mas o fluxo ainda não avançou, usa o resultado existente para prosseguir sem executar o nó de novo.

As notificações vêm ativadas por padrão. Em **Configurações → Geral → Intervenção remota e notificações de IM**, você pode desativar **Solicitação para continuar o fluxo** por canal. Se o cartão indicar que o processamento ainda está em andamento logo após a interrupção, aguarde um pouco e tente novamente no mesmo cartão.

> [!NOTE]
> O app para desktop precisa estar online. Esta atualização não permite iniciar, interromper ou reexecutar fluxos de trabalho pelo IM. O app não envia cartões em massa para fluxos pausados antes da atualização.

### 3. Três novos Agents e uma configuração inicial mais flexível

- A seção **Agents** agora inclui DeepSeek Harness, ZCode e Qoder. O ZCode usa um adaptador da comunidade. A opção integrada do Qoder não está disponível no Windows ARM64. O nome de exibição padrão do CodeBuddy integrado passou a ser WorkBuddy; as configurações existentes são mantidas.
- Ao adicionar ou editar um Agent, consulte **Requisitos de conexão** para verificar os comandos locais e os métodos de autenticação necessários.
- A configuração inicial continua verificando Node.js, npm e npx, mas uma falha nessas verificações não impede mais a detecção de Agents locais. O resultado diferencia os que não estão instalados, os que não têm o ambiente de execução necessário e os que falharam no diagnóstico. Quando a detecção terminar ou falhar, escolha **Configurar depois** para abrir a tela principal e finalizar a configuração mais tarde, mesmo que não haja Agents disponíveis.

### 4. Temas Vista e Meadow, com papéis de parede separados para os modos claro e escuro

Escolha um dos dois novos temas em **Configurações → Aparência**:

- **Vista**: uma pintura panorâmica em aquarela no modo claro e picos nevados em preto e dourado sob um céu estrelado no modo escuro. Os tons de azul-petróleo e dourado de damasco vêm dos respectivos papéis de parede.
- **Meadow**: o modo claro usa um verde suave de broto no lugar do branco puro; o modo escuro lembra uma noite tranquila de primavera em tons de verde-pinho. As áreas de código e Diff têm um fundo mais claro para facilitar a leitura.

Gold Band, Vista e Meadow oferecem seis papéis de parede integrados ao todo. Escolha **Papel de parede do tema**, **Personalizado** ou **Desativar** separadamente em cada modo. Papéis de parede personalizados admitem **Preencher** e **Ajustar** e podem ser removidos de **Usado recentemente**.

### 5. Navegue pelos turnos da conversa e ajuste a altura do campo de mensagem

- Use a nova navegação por turnos à esquerda da conversa. Passe o ponteiro sobre uma marca para ver a prévia da mensagem ou clique para ir a esse turno, inclusive a turnos anteriores que ainda não aparecem na tela.
- Nas telas inicial e de conversa, arraste a borda superior do campo de mensagem para ajustar a altura. O tamanho é mantido ao trocar de conversa. Arraste a borda de volta para baixo para retornar ao tamanho automático.

### 6. Desinstale papéis e fluxos de trabalho locais pelo Multica

Depois de conectar, abra a aba **Runtime** na área de gerenciamento de papéis do Multica. Ali você pode ver os papéis e fluxos de trabalho locais no ambiente de execução atual e desinstalar itens que não sejam integrados. A desinstalação remove apenas a cópia local e não apaga o conteúdo do catálogo compartilhado do Multica. Quando você envia várias operações de recursos em sequência, o app para desktop processa as solicitações da fila com mais rapidez.

## Melhorias na experiência

### 1. Barra lateral e conversas mais fáceis de usar

- Em **Recentes**, você pode expandir o histórico de execuções, renomear ou fixar conversas. Use os botões no fim das listas da barra lateral para carregar mais itens. A seleção é preservada quando o histórico é atualizado, e o indicador de atividade desaparece quando a tarefa termina.
- O corpo das tabelas Markdown usa a cor de texto padrão do tema, facilitando a leitura tanto no tema claro quanto no escuro.

### 2. Inicialização e conexões com Agents mais estáveis

- Uma configuração de inicialização inválida em um Agent não impede mais o aplicativo de abrir. Corrigimos um problema em que o WorkBuddy podia trocar o método de inicialização em turnos posteriores e interromper uma conexão local que estava funcionando.
- Corrigimos algumas falhas de inicialização do WebView. Ambientes incompatíveis agora mostram instruções de atualização para a plataforma correspondente. Clientes MCP mais recentes descobrem com maior confiabilidade as ferramentas integradas de memória compartilhada.

### 3. Base de conhecimento e menu de downloads no site oficial

O site oficial e o Demo agora usam a marca MALING. Leia a base de conhecimento no site ou use o menu de downloads para escolher o instalador da sua plataforma.
