# 📚 FilterFlow: Agente Inteligente de Notícias em Rust

O FilterFlow é um agente de notícias assíncrono escrito em Rust que monitora feeds RSS e Sitemaps de forma contínua, filtra o conteúdo usando um LLM (Large Language Model) local e apresenta apenas as notícias relevantes e resumidas para o usuário. Ele utiliza o banco de dados `sled` para cache e evita reprocessar conteúdo.

## ⚙️ 1. Preparação do Ambiente (Fedora Silverblue + Toolbox)

Recomendamos utilizar o Toolbox no Fedora Silverblue para isolar o ambiente de desenvolvimento e compilação do Rust.

### 1.1. Configuração do Toolbox

1. **Crie e entre no Toolbox:**
   
   Bash
   
   ```
   toolbox create -c rust-dev
   toolbox enter rust-dev
   ```

2. **Instale as dependências básicas no Toolbox:**
   
   Bash
   
   ```
   # Atualize o sistema
   sudo dnf update -y
   # Instale dependências de compilação
   sudo dnf install -y clang make
   ```

### 1.2. Instalação do Rust

Dentro do Toolbox, instale o Rust usando o `rustup`:

Bash

```
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Selecione a opção '1' para instalação padrão.

# Carregue o ambiente
source $HOME/.cargo/env
```

### 1.3. Clonagem e Compilação do Projeto

1. **Clone o repositório:**
   
   Bash
   
   ```
   https://github.com/marciosr/filterflow.git
   cd filterflow
   ```

2. **Compile o projeto:**
   
   Bash
   
   ```
   cargo build --release
   ```

O executável compilado estará em `./target/release/filterflow`.

## 🛠️ 2. Bibliotecas Rust Necessárias

FilterFlow depende das bibliotecas abaixo:

| **Crate**         | **Função**                                                                                                         |
| ----------------- | ------------------------------------------------------------------------------------------------------------------ |
| `tokio`           | *Runtime* assíncrono para lidar com I/O concorrente (requisições HTTP e operações de DB).                          |
| `reqwest`         | Cliente HTTP assíncrono para baixar Feeds/Sitemaps e comunicar-se com o LLM.                                       |
| `sled`            | Banco de dados *key-value* embutido e de alto desempenho, usado para cache de irrelevância e notícias processadas. |
| `serde` & `toml`  | Desserialização de dados para leitura do arquivo de configuração `filterflow_config.toml`.                         |
| `async-recursion` | Atributo para habilitar a recursão em funções assíncronas (necessário para navegar em Índices de Sitemap).         |
| `rss` & `sitemap` | *Parsers* específicos para analisar e iterar sobre o conteúdo de Feeds RSS e arquivos Sitemap XML.                 |

## 🧠 3. Configuração do LLM (LM Studio)

O FilterFlow foi projetado para usar modelos de linguagem locais compatíveis com a API OpenAI (OpenAI-compatible local API). O LM Studio é uma excelente ferramenta para isso.

### 3.1. Download e Instalação

Faça o download e instale o **LM Studio** em seu sistema operacional (não no Toolbox).

### 3.2. Ativação do Servidor (Endpoint)

1. **Baixe um modelo:** No LM Studio, baixe e carregue um modelo no painel de **Chat/Servidor Local** (como Zephyr, Mistral, Llama, etc.).

2. **Inicie o Servidor:** Vá para a aba "Servidor Local" (o ícone de engrenagem) e clique em **"Start Server"**.

3. **Verifique o Endereço:** O endereço padrão do servidor é **`http://localhost:1234/v1/chat/completions`**. Este deve ser o valor configurado no campo `geral.endereco` no `filterflow_config.toml`.

## 📝 4. Arquivo de Configuração (`filterflow_config.toml`)

O FilterFlow é altamente configurável através deste arquivo.

| **Seção/Campo**                         | **Tipo**         | **Descrição**                                                                                                                                           |
| --------------------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`[geral].endereco`**                  | String           | **URL do endpoint da API do LLM.** (Ex: `http://localhost:1234/v1/chat/completions`).                                                                   |
| **`[geral].intervalo_minutos`**         | Inteiro          | Tempo de espera entre os ciclos completos de varredura.                                                                                                 |
| **`[geral].modelo_resumo`**             | String           | Nome do modelo (apenas para referência interna do LLM/LM Studio).                                                                                       |
| **`[geral].user_agent`**                | String           | **Importante!** O cabeçalho `User-Agent` usado nas requisições HTTP para evitar bloqueios `403 Forbidden` do servidor. Use um valor de navegador comum. |
| **`[filtro].palavras_chave`**           | Array            | Lista de termos que tornam a notícia **RELEVANTE** (Tópicos de INCLUSÃO).                                                                               |
| **`[filtro].indicadores_irrelevancia`** | Array            | Lista de termos que tornam a notícia **IRRELEVANTE** (Tópicos de EXCLUSÃO).                                                                             |
| **`[[feeds]]`**                         | Array de Tabelas | Nome e URL dos **Feeds RSS** a serem monitorados.                                                                                                       |
| **`[[sitemaps]]`**                      | Array de Tabelas | Nome e URL dos **Sitemaps (ou Sitemap Index)** a serem monitorados.                                                                                     |
| **`[proxy].usar_proxy`**                | Booleano         | `true` ou `false` para ativar o proxy para todas as requisições.                                                                                        |
| **`[proxy].endereco_proxy`**            | String           | Endereço completo do proxy HTTP/HTTPS.                                                                                                                  |

## 🚀 5. Uso do FilterFlow

Após configurar o `LM Studio` e o `filterflow_config.toml`, execute o agente:

Bash

```
# Dentro do Toolbox (após a compilação)
./target/release/filterflow
```

O FilterFlow iniciará e rodará em um *loop* contínuo.

- **Logs em Cores:** O agente utiliza códigos ANSI para destacar os logs e os resultados no terminal.

- **Novidade Relevante:** Quando uma notícia é considerada relevante, ela é exibida em destaque verde, seguida pelo resumo gerado pelo LLM.

- **Cache:** Notícias já processadas ou consideradas irrelevantes são armazenadas no banco de dados `sled` (`filterflow_data`) e não serão reavaliadas em ciclos futuros.

## 🤖 6. Como Funciona o Prompt de Filtragem

O coração da inteligência do FilterFlow está no `prompt` enviado ao LLM na função `call_llm_filter`.

O objetivo é forçar o LLM a atuar como um classificador binário (resposta `1` ou `0`), avaliando duas condições simultâneas com base nas suas configurações:

### Prompt Estruturado (Lógica Booleana)

O *prompt* instrui o LLM a tomar a decisão final usando a seguinte lógica:

1. **Condição de INCLUSÃO:** A notícia deve ser **principalmente** sobre um ou mais tópicos listados em `palavras_chave`.

2. **Condição de EXCLUSÃO:** A notícia **não** deve conter nenhum tópico listado em `indicadores_irrelevancia`.

A resposta final esperada do LLM é:

- **`1` (Relevante):** Se (INCLUSÃO for **VERDADEIRA**) **E** (EXCLUSÃO for **FALSA**).

- **`0` (Irrelevante):** Em qualquer outro caso (falha na inclusão OU presença de exclusão).

Essa filtragem em duas etapas garante que, por exemplo, uma notícia sobre "Mercado de Ações" (Inclusão) que também mencione "Celebridades" (Exclusão) seja corretamente descartada.
