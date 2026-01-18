````markdown
# Interest - Rastreador de Investimentos B3 (Brasil)

Uma ferramenta de linha de comando para gerenciar investimentos na B3 (Bolsa de Valores do Brasil). O Interest cuida do seu fluxo completo de investimentos: importa transações a partir dos arquivos exportados pela B3, acompanha sua carteira em tempo real, calcula métricas de performance, gerencia eventos societários (splits, renomes, spin-offs) e gera relatórios fiscais compatíveis com as regras do IRPF (Imposto de Renda Pessoa Física).

**Principais recursos:**

- 📊 Acompanhamento de carteira em tempo real com atualização automática de preços
- 📈 Análises de performance (MTD, QTD, YTD, períodos customizados)
- 💰 Controle de rendimentos (dividendos, JCP, amortizações)
- 🧾 Cálculos fiscais brasileiros (swing trade, day trade, relatórios IRPF)
- 🔄 Gerenciamento de eventos societários (splits, renomes, fusões, spin-offs)
- 📥 Importação de planilhas Excel da B3/CEI (Negociação, Movimentação, PDFs de IRPF)
- 🎯 TUI interativa com histórico de comandos e autocompletar por tab

**Público-alvo:** Investidores brasileiros negociando na B3 que precisam de controle preciso do custo médio e geração de relatórios fiscais.

---

## Instalação

### Pré-requisitos

- **Rust 1.70+** para compilar a partir do código-fonte ([Instalar Rust](https://rustup.rs/))
- **SQLite 3.x** (geralmente já instalado no Linux/macOS)

### Compilando

```bash
git clone https://github.com/your-username/interest
cd interest
cargo build --release
```

O binário compilado ficará em `./target/release/interest`.

### Teste rápido

```bash
# Use o subcomando `interactive` para iniciar a TUI
./target/release/interest interactive

# Ou testar um comando
./target/release/interest help
```

Nota: os exemplos de comando neste README mantêm formatos ISO de data (`YYYY-MM-DD`) e notação decimal com ponto (ex.: `28.50`) para compatibilidade com a CLI.

---

## Primeiros passos: fluxo completo de configuração

Siga estes 6 passos para preparar o Interest com seus dados. Este fluxo cobre o caso comum em que você tem posições anteriores a 2020 (antes da B3 centralizar totalmente os registros digitais).

### Passo 1: Adicionar saldos iniciais

**Por quê:** As exportações de **Negociação** da B3 têm dados completos a partir de 2020. Para posições anteriores a 2020, adicione saldos iniciais manualmente.

**Escolha uma data de referência:** Use uma data em 2019 (ex.: `2019-12-31`) e mantenha-a consistente para todos os saldos iniciais.

**Adicione suas posições:**

```bash
# Sintaxe: interest transactions add <TICKER> buy <QUANTITY> <PRICE> <DATE>

# Exemplo: adicionar saldos iniciais para ações e FIIs
interest transactions add PETR4 buy 200 28.50 2019-12-31
interest transactions add VALE3 buy 150 52.30 2019-12-31
interest transactions add XPLG11 buy 50 120.00 2019-12-31
interest transactions add HGLG11 buy 75 135.50 2019-12-31
```

**Atenção:** o preço deve ser seu preço médio de aquisição, não o preço de mercado.

### Passo 2: Exportar dados da B3

**Como acessar o Portal do Investidor B3:**

1. Vá para https://www.investidor.b3.com.br/
2. Faça login com seu CPF e senha
3. Acesse **"Extratos e Informativos"** → **"Negociação de Ativos"**

**Exporte dois arquivos:**

**Arquivo 1: Negociação de Ativos** (Trades)

- Defina o intervalo: da data do saldo inicial (ex.: `2020-01-01`) até hoje
- Clique em **"Exportar"** e escolha formato **Excel**
- Salve como `negociacao.xlsx`

**Arquivo 2: Movimentação** (Eventos societários e rendimentos)

- Vá em **"Extratos e Informativos"** → **"Movimentação"**
- Use o mesmo intervalo de datas
- Clique em **"Exportar"** e escolha **Excel**
- Salve como `movimentacao.xlsx`

### Passo 3: Importar Negociação (Trades)

Importe primeiro as negociações para estabelecer o histórico de transações.

**Pré-visualizar (recomendado):**

```bash
interest import negociacao.xlsx --dry-run
```

**Importar de fato:**

```bash
interest import negociacao.xlsx
```

**O que é importado:**

- Compras/vendas
- Datas de negociação e liquidação
- Taxas e custos de corretagem
- Tipo de ativo (detectado automaticamente pelo sufixo do ticker)

**Detecção de duplicatas:** a ferramenta ignora automaticamente transações duplicadas, então é seguro reimportar o mesmo arquivo.

### Passo 4: Importar Movimentação (Eventos societários)

Agora importe ações corporativas, dividendos e outros eventos.

```bash
interest import movimentacao.xlsx
```

**O que é importado:**

- Dividendos e JCP (Juros sobre Capital Próprio)
- Splits e bonificações
- Direitos de subscrição e conversões
- Transferências e outros eventos

**Observação:** alguns eventos (ex.: conversões de subscrição sem custo) podem gerar **inconsistências** que você precisará resolver no próximo passo.

### Passo 5: Resolver inconsistências

Alguns eventos importados podem ter informações faltando. O Interest registra esses casos como "inconsistências" e você pode resolvê-las interativamente.

**Resolver com experiência guiada (recomendado):**

```bash
interest inconsistencies resolve
```

A ferramenta solicitará interativamente campos obrigatórios (preço, taxas, datas etc.). Isso costuma ser mais simples do que identificar manualmente quais campos faltam.

**Verificar questões em aberto:**

```bash
interest inconsistencies list --open
```

**Tipos comuns de problema:**

- **MissingCostBasis**: conversões de subscrição sem custo original
- **MissingPurchaseHistory**: vendas sem compras correspondentes (geralmente posições pré-2020)
- **InvalidTicker**: tickers que não foram detectados automaticamente

**Ver detalhes de um problema específico:**

```bash
interest inconsistencies show 42
```

**Definir campos diretamente (se souber):**

```bash
interest inconsistencies resolve 42 --set price_per_unit=18.75 --set fees=5.00
```

**Ignorar se não for relevante:**

```bash
interest inconsistencies ignore 42 --reason "Duplicate entry from old statement"
```

### Passo 6: Adicionar eventos societários manualmente (se necessário)

**Boas notícias:** a maioria dos eventos vem automaticamente nos arquivos da B3. Entrada manual costuma ser necessária apenas para **casos raros** que a B3 não registra bem.

**Casos comuns que exigem entrada manual:**

**Renomeações de ticker:**

```bash
# Ex.: Varejo virou Casas Bahia (VIIA3 → BHIA3)
interest actions rename add VIIA3 BHIA3 2023-01-15
```

**Spin-offs:**

```bash
# Ex.: GPA (Pão de Açúcar) desmembrou Assaí (ASAI3)
interest actions spinoff add PCAR3 ASAI3 2021-03-01 100 5000
```

**Fusões:**

```bash
interest actions merger add BTOW3 LAME3 2021-05-01 200 12000
interest actions merger add AMER3 LAME3 2021-05-01 150 8000
```

**Verificar listas:**

```bash
interest actions rename list
interest actions spinoff list
interest actions merger list
```

---

## Operações diárias

### Visualizar sua carteira

**Carteira completa com preços atuais:**

```bash
interest portfolio show
```

**Filtrar por tipo de ativo:**

```bash
interest portfolio show --asset-type fii
interest portfolio show --asset-type stock
interest portfolio show --asset-type fiagro
```

**Instantâneo histórico (carteira em uma data específica):**

```bash
interest portfolio show --at 2024-12-31
interest portfolio show --at 2024-06
interest portfolio show --at 2023
```

O output inclui:

- Quantidade atual e custo médio
- Preço de mercado atual
- Valor da posição e P&L não realizado (valor e %)
- Valor total da carteira e resumo por tipo de ativo

### Ver performance

**Períodos comuns:**

```bash
# Year-to-date
interest performance show YTD

# Month-to-date
interest performance show MTD

# Quarter-to-date
interest performance show QTD

# Últimos 12 meses
interest performance show 1Y

# Desde o início (primeira transação)
interest performance show ALL

# Ano específico
interest performance show 2024
```

**Intervalo customizado:**

```bash
interest performance show 2024-01-01:2024-12-31
interest performance show 2024-06:2024-12
```

As métricas incluem Time-Weighted Return (TWR), ganhos absolutos e breakdown por tipo de ativo.

### Ver rendimentos (Dividendos & JCP)

**Resumo por ativo:**

```bash
interest income show
interest income show 2024
```

**Eventos detalhados por ano:**

```bash
interest income detail 2024
```

**Filtrar por ativo:**

```bash
interest income detail 2024 --asset XPLG11
```

**Resumo mensal:**

```bash
interest income summary 2024
interest income summary
```

### Gerar relatórios fiscais

**Relatório anual IRPF:**

```bash
interest tax report 2024
```

Isso gera um relatório completo com:

- Cálculos mensais de imposto (swing trade)
- Controle de compensação de prejuízos
- Bens e Direitos (posições em 31/12)
- Rendimento recebido (dividendos, JCP)
- Resumo de transações

**Exportar para CSV:**

```bash
interest tax report 2024 --export
```

**Resumo rápido (visão condensada):**

```bash
interest tax summary 2024
```

---

## Operações comuns

### Gerenciar ativos

**Listar todos os ativos:**

```bash
interest assets list
```

**Filtrar por tipo:**

```bash
interest assets list --type fii
interest assets list --type stock
interest assets list --type bdr
```

**Mostrar detalhes de um ativo:**

```bash
interest assets show PETR4
```

**Definir/atualizar tipo de ativo:**

```bash
interest assets set-type XPLG11 fii
```

**Definir/atualizar nome do ativo:**

```bash
interest assets set-name XPLG11 "XP Logística FII"
```

**Sincronizar com registro Mais Retorno:**

```bash
# Pré-visualizar
interest assets sync-maisretorno --dry-run

# Sincronizar de fato
interest assets sync-maisretorno

# Sincronizar apenas um tipo
interest assets sync-maisretorno --type fii
```

### Atualizar registro de tickers

O registro de tickers armazena metadados sobre tickers B3. Ele é atualizado automaticamente, mas pode ser forçado.

**Ver status do cache:**

```bash
interest tickers status
```

**Forçar atualização:**

```bash
interest tickers refresh --force
```

**Listar tickers desconhecidos:**

```bash
interest tickers list-unknown
```

**Resolver manualmente um ticker:**

```bash
interest tickers resolve XPTO11 --type fii
```

### Importar preços históricos (COTAHIST da B3)

Para cálculos de performance históricos, importe o COTAHIST quando necessário e ele será cacheado.

**Importar ano específico:**

```bash
interest prices import-b3 2024
```

**Importar de arquivo local:**

```bash
interest prices import-b3-file ~/Downloads/COTAHIST_A2024.ZIP
```

**Limpar cache de preços:**

```bash
interest prices clear-cache 2024
```

---

## Referência de eventos societários

Resumo rápido dos tipos de ações corporativas. Lembre-se: a maioria dos splits vem automaticamente dos arquivos de Movimentação, então a entrada manual costuma ser necessária apenas para renomes, spin-offs e fusões.

### Splits & Reverse-Splits

**Adicionar split (quantidade aumenta):**

```bash
# Adiciona 100 ações por ação detida
interest actions split add PETR4 100 2022-03-15
```

**Adicionar reverse-split (quantidade diminui):**

```bash
# Reverse split 10:1 (1000→100, ajuste -900)
interest actions split add A1MD34 -900 2022-11-22
```

**Listar splits:**

```bash
interest actions split list
```

**Remover split:**

```bash
interest actions split remove 5
```

### Renomeações

**Adicionar renomeação de ticker:**

```bash
interest actions rename add VIIA3 BHIA3 2023-01-15
```

**Listar renomeações:**

```bash
interest actions rename list
```

**Remover renomeação:**

```bash
interest actions rename remove 3
```

### Bonificações

**Adicionar bonificação:**

```bash
# 10% bonificação (50 ações adicionais por 100)
interest actions bonus add ITSA4 50 2023-05-10 --notes "10% bonus declared"
```

**Remover bonificação:**

```bash
interest actions bonus remove 7
```

### Spin-offs & Fusões

**Adicionar spin-off:**

```bash
interest actions spinoff add PCAR3 ASAI3 2021-03-01 100 5000 --notes "Assaí spin-off"
```

**Adicionar fusão:**

```bash
interest actions merger add BTOW3 LAME3 2021-05-01 200 12000 --notes "B2W merger"
```

**Listar e remover:**

```bash
interest actions spinoff list
interest actions merger list
interest actions spinoff remove 8
interest actions merger remove 9
```

### Como os eventos societários funcionam

Os eventos são aplicados **automaticamente** durante cálculos de carteira e impostos. Ao gerar relatórios, o sistema:

1. Lê suas transações do banco (sem alterar)
2. Aplica ajustes (split/rename/merger) em ordem cronológica
3. Apresenta quantidades e preços ajustados

**Vantagens:**

- Não há etapa separada de "aplicar" — basta adicionar o evento
- Transações no banco permanecem inalteradas (auditável)
- Sem risco de aplicação dupla

---

## Arquivos & diretórios

### Local do banco de dados

```
~/.interest/data.db
```

Este banco SQLite contém:

- Transações
- Ativos (tickers, tipos, nomes)
- Eventos societários
- Histórico de preços
- Eventos de renda
- Snapshots de carteira
- Cálculos fiscais

**Backup regularmente:**

```bash
# Backup com timestamp
cp ~/.interest/data.db ~/.interest/data.db.backup-$(date +%Y%m%d)

# Antes de alterações grandes
cp ~/.interest/data.db ~/.interest/data.db.backup-pre-import
```

**Inspecionar com sqlite3:**

```bash
sqlite3 ~/.interest/data.db "SELECT * FROM assets LIMIT 10"
```

### Diretórios de cache

Local do cache segue padrões por plataforma (via `dir_spec`):

- **Linux**: `~/.cache/interest/`
- **macOS**: `~/Library/Caches/interest/`
- **Windows**: `%LOCALAPPDATA%\\interest\\cache\\`

**Subdirs:** `tickers/`, `cotahist/`, `tesouro/`

**Apagar cache (seguro):**

```bash
rm -rf ~/.cache/interest/
rm -rf ~/Library/Caches/interest/
```

Referência: https://docs.rs/dir_spec/latest/dir_spec/fn.cache_home.html

---

## Solução de problemas

### Erro "Insufficient Purchase History"

**Mensagem:**

```
Error: PETR4: Insufficient purchase history: Selling 100 units but only 50 available.
```

**Causas:**

1. Falta de transações pré-2020
2. Evento societário não registrado
3. Direitos de subscrição/transferências não importados
4. Dados pré-CEI não informados

**Soluções:**

**Adicionar compras históricas:**

```bash
interest transactions add PETR4 buy 100 25.50 2018-06-15
```

**Verificar eventos registrados:**

```bash
interest actions split list PETR4
```

**Ver inconsistências:**

```bash
interest inconsistencies list --open --asset PETR4
```

### Erro "Unknown Ticker"

**Mensagem:**

```
Error: Unknown ticker: XPTO11
```

**Soluções:**

```bash
interest tickers refresh --force
interest tickers resolve XPTO11 --type fii
interest assets add XPTO11 --type fii --name "XPTO Fundo Imobiliário"
```

### Falha ao buscar preço

**Aviso:**

```
Warning: Failed to fetch price for PETR4: 404 Not Found
```

**Ações:**

```bash
interest portfolio show
interest prices import-b3 2024
```

### Inconsistência não resolve

Se faltar um campo obrigatório (ex.: `price_per_unit`), veja detalhes e use a resolução guiada:

```bash
interest inconsistencies show 42
interest inconsistencies resolve 42
```

Ou passe todos os campos:

```bash
interest inconsistencies resolve 42 \\
  --set price_per_unit=18.75 \\
  --set fees=12.34 \\
  --set trade_date=2023-08-02
```

### Detecção de duplicatas ao importar

Mensagem:

```
Skipped 15 duplicate transactions
```

Comportamento normal — duplicatas são ignoradas com base em ticker, data, tipo e quantidade.

---

## Uso avançado

### Modo TUI interativo

```bash
# Quando instalado
interest interactive
# ou via cargo
cargo run -- interactive
```

**Recursos:** histórico de comandos, autocompletar, indicadores de progresso.

### Saída JSON para scripts

Quase todos os comandos aceitam `--json`:

```bash
interest portfolio show --json > portfolio.json
```

Parse com `jq`:

```bash
interest portfolio show --json | jq '.positions[] | select(.asset_type == "FII")'
```

### Modo dry-run

Pré-visualize mudanças:

```bash
interest import negociacao.xlsx --dry-run
interest assets sync-maisretorno --dry-run
```

### Análise de fluxos de caixa

```bash
interest cash-flow show 2024
interest cash-flow show YTD
interest cash-flow show ALL
interest cash-flow show 2024-01:2024-06
interest cash-flow stats YTD
```

---

## Dicas & boas práticas

1. Use `--dry-run` em importações grandes
2. Faça backup do banco regularmente
3. Resolva inconsistências rapidamente
4. Mantenha eventos societários atualizados
5. Atenção a mudanças fiscais (ex.: regras de FII/FIAGRO em 2026)
6. Verifique a carteira após importações
7. Gere relatórios fiscais com antecedência
8. Use saída JSON para automação

---

## Obter ajuda

```bash
interest help
```

No modo interativo:

```
help
?
```

Reportar issues:

- GitHub Issues: https://github.com/your-username/interest/issues

---

## O que não está neste guia

Este README é focado no uso. Para desenvolvedores, veja `CLAUDE.md` para arquitetura, padrões, esquema do DB e estratégia de testes.

---

## Licença

MIT

---

## Créditos

Desenvolvido por [Gustavo Noronha Silva](https://github.com/kov) com auxílio de:
Claude Code (Anthropic)
Codex (OpenAI)
````
