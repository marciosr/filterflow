# FilterFlow-Dash 📈

Interface web minimalista para visualizar as notícias processadas pelo FilterFlow.

## Configuração

O dashboard utiliza variáveis de ambiente para localização do banco de dados e porta:

- `FILTERFLOW_DB_PATH`: Caminho para a pasta do banco Sled (Padrão: `./filterflow_data`)
- `PORT`: Porta do servidor (Padrão: `3000`)

## Como Compilar e Executar

1. **Compilação**:
   ```bash
   cargo build --release


## Como definir a origem dos dados
Para rodar localmente como antes:
cargo run

Para rodar apontando para um servidor externo (exemplo):
FILTERFLOW_API_URL=http://192.168.1.50:4000 cargo run

🛠 Arquitetura do Sistema: FilterFlow Dashboard
1. Visão Geral

O sistema é composto por uma arquitetura desacoplada de microserviços em Rust:

    Backend (FilterFlow): Responsável pelo processamento de dados (Ingestion) e persistência em banco de dados chave-valor (Sled). Disponibiliza uma API JSON.

    Frontend (Dash): Um servidor web leve que consome a API do backend e renderiza interfaces dinâmicas usando HTMX.

2. Stack Tecnológica

    Linguagem: Rust (foco em segurança de memória e performance).

    Web Framework: Axum (baseado em Tokio/Tower), permitindo alta concorrência e uso de extratores tipados.

    Template Engine: Maud, gerando HTML compilado de alta velocidade e verificação de sintaxe em tempo de compilação.

    Interatividade: HTMX, permitindo atualizações parciais da página (AJAX) sem a necessidade de frameworks JavaScript complexos.

3. Fluxo de Sincronização de Notícias

Para resolver o problema de persistência de leitura (Status 422), foi implementado o seguinte fluxo:

    Gatilho: O usuário expande o elemento <details> no Dash.

    Requisição: O HTMX dispara um POST via extensão json-enc.

    Processamento: O FilterFlow utiliza a struct MarcarLidaRequest { id: String } para desserializar o JSON e atualizar o campo lida: true no Sled.

    Interface: O Dash aplica classes CSS (.read) instantaneamente para feedback visual (esmaecimento e troca de badge).

4. Otimizações de Performance

    Connection Pooling: O Dashboard utiliza um único reqwest::Client compartilhado via State do Axum para reutilizar conexões TCP.

    Desacoplamento de Banco: O Dashboard não acessa o arquivo Sled diretamente, evitando file locking e permitindo que os serviços escalem independentemente.

    Variáveis de Ambiente: A URL da API é configurável via FILTERFLOW_API_URL, seguindo as boas práticas de 12-Factor Apps.
