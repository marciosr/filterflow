# 🌊 FilterFlow: Ecossistema Inteligente de Notícias em Rust

O **FilterFlow** é um agente assíncrono de curadoria de conteúdo que monitora feeds RSS e Sitemaps, utiliza Inteligência Artificial local (LLM) para filtrar relevância e disponibiliza os resultados através de uma API robusta e um Dashboard reativo.

## 🏗️ 1. Arquitetura do Projeto

O projeto é organizado como um **Rust Workspace**, garantindo que o backend e o frontend compartilhem dependências de forma eficiente:

* **`filterflow_backend`**: O motor (Core). Responsável pelo crawling, filtragem via LLM, arquivamento em Markdown e exposição da API REST.
* **`filterflow_dash`**: O cliente (Frontend). Uma interface web construída com **Axum** e **HTMX** para visualização rápida das notícias filtradas.

---

## 🛠️ 2. Tecnologias Utilizadas

### Core & Backend

| Crate | Função |
| --- | --- |
| `tokio` | Runtime assíncrono de alta performance. |
| `axum` | Web framework para a API e o Servidor do Dashboard. |
| `sled` | Banco de dados *key-value* embutido para persistência ultra-rápida. |
| `reqwest` | Cliente HTTP para coleta de dados e comunicação com LLM. |
| `serde` | Serialização e desserialização de dados (JSON/TOML). |

### Dashboard & UI

| Tecnologia | Função |
| --- | --- |
| **HTMX** | Reatividade frontend sem a necessidade de frameworks JS pesados. |
| **Markdown** | Formato de arquivamento das notícias para leitura offline e portabilidade. |
| **Jin2 / Askama** | Motores de template para renderização de HTML no servidor. |

---

## 🚀 3. Novas Capacidades

### 📂 Arquivamento em Markdown

Agora, cada notícia considerada relevante é automaticamente convertida e salva em arquivos `.md`. Isso permite que você tenha um histórico permanente, pesquisável e compatível com ferramentas como Obsidian ou Notion.

### 🌐 API de Notícias

O backend agora expõe um endpoint `/noticias` que serve o conteúdo filtrado e resumido em formato JSON, permitindo que qualquer cliente (como o FilterFlow Dash) consuma as informações em tempo real.

---

## ⚙️ 4. Instalação e Uso

### 4.1. Preparação (Fedora Silverblue / Toolbox)

Se você utiliza ambientes imutáveis, recomendamos o uso do Toolbox:

```bash
toolbox create -c rust-dev
toolbox enter rust-dev
sudo dnf install clang make -y

```

### 4.2. Compilação Global

Na raiz do projeto (onde está o `Cargo.toml` do workspace), compile ambos os módulos:

```bash
cargo build --release

```

### 4.3. Executando os Módulos

Você deve rodar ambos simultaneamente para a experiência completa:

**Iniciar o Backend (API e Crawler):**

```bash
cargo run -p filterflow_backend

```

**Iniciar o Dashboard (Interface Web):**

```bash
cargo run -p filterflow_dash

```

*Acesse o dashboard em: `http://localhost:3000*`

---

## 📝 5. Configuração (`filterflow_config.toml`)

O arquivo de configuração agora gerencia o comportamento do agente e os parâmetros da API:

| Seção | Campo | Descrição |
| --- | --- | --- |
| `[geral]` | `endereco` | URL do LM Studio (Ex: `http://localhost:1234/v1`). |
| `[geral]` | `arquivar_md` | `true/false` para habilitar salvamento em Markdown. |
| `[filtro]` | `palavras_chave` | Tópicos de inclusão para o LLM. |
| `[api]` | `porta` | Porta onde o backend servirá os dados (Padrão: 4000). |

---

## 🤖 6. Inteligência de Filtragem (LLM)

O FilterFlow utiliza uma lógica booleana rigorosa no prompt para garantir alta precisão:

1. **Inclusão**: A notícia deve abordar os temas de interesse.
2. **Exclusão**: Se houver qualquer termo de irrelevância, a notícia é descartada mesmo que contenha palavras-chave.
3. **Resumo**: Somente após passar pelo filtro, o LLM gera um resumo executivo da notícia.

---

## 📂 7. Estrutura de Diretórios

```text
.
├── filterflow_backend/  # Crawler, API e Lógica de IA
├── filterflow_dash/     # Interface Web HTMX
├── filterflow_data/     # Persistência Sled (DB)
├── arquivados/          # Notícias salvas em .md
├── Cargo.toml           # Configuração do Workspace
└── filterflow_config.toml

```

---

## 🔌 8. Detalhamento da API (Backend)

O `filterflow_backend` não é apenas um crawler; ele atua como um servidor de dados robusto. A comunicação entre o backend e o dashboard é feita via JSON através de endpoints REST.

### Endpoints Principais

| Método | Endpoint | Descrição |
| --- | --- | --- |
| `GET` | `/noticias` | Retorna a lista completa de notícias filtradas e resumidas em formato JSON. |
| `GET` | `/status` | Retorna o estado do crawler (última varredura, quantidade de itens no DB). |
| `POST` | `/refresh` | Força um novo ciclo de varredura nos feeds e sitemaps. |

### Exemplo de Resposta JSON (`/noticias`)

```json
{
  "id": "2024-02-22-titulo-da-noticia",
  "titulo": "Avanços em Rust 1.76",
  "resumo": "A nova versão traz melhorias em performance e tipos...",
  "url": "https://exemplo.com/rust-news",
  "data_processamento": "2024-02-22T10:00:00Z"
}

```

---

## 🌍 9. Variáveis de Ambiente e Configuração Avançada

Embora o `filterflow_config.toml` seja o coração da configuração, o uso de **Variáveis de Ambiente** permite que o FilterFlow seja executado em containers Docker ou ambientes de CI/CD sem expor dados sensíveis.

O sistema busca automaticamente por um arquivo `.env` na raiz do projeto:

| Variável | Descrição | Exemplo |
| --- | --- | --- |
| `FILTERFLOW_PORT` | Porta do servidor Backend. | `4000` |
| `DASH_PORT` | Porta do Dashboard Frontend. | `3000` |
| `LLM_API_KEY` | Chave de API (se usar serviços externos como OpenAI). | `sk-xxxx...` |
| `RUST_LOG` | Nível de detalhamento dos logs. | `info`, `debug`, `error` |

### Exemplo de Arquivo `.env`

```env
# Configurações de Rede
FILTERFLOW_PORT=4000
DASH_PORT=3000

# Logs (Útil para depuração)
RUST_LOG=filterflow_backend=debug,filterflow_dash=info

```

---

## 🖥️ 10. Fluxo de Dados: Do Crawler ao Dashboard

Para entender como as novas capacidades de **arquivamento em Markdown** e **HTMX** interagem, veja o fluxo de vida de uma notícia:

1. **Coleta**: O `backend` lê os Sitemaps/RSS configurados.
2. **Filtragem**: O conteúdo é enviado ao LLM local. Se irrelevante, o ID vai para o `sled` (cache) e o processo para.
3. **Persistência**:
* O dado estruturado é salvo no `sled`.
* **Novo!** Um arquivo `.md` é gerado na pasta `/arquivados` com o resumo e links.


4. **Consumo**: O `filterflow_dash` solicita os dados ao endpoint `/noticias`.
5. **Renderização**: O **HTMX** recebe o HTML pré-renderizado pelo Dash e atualiza a página do usuário sem refresh completo.

---
