use axum::{
	Router,
	extract::State,
	response::{IntoResponse, Response},
	routing::get,
};
use chrono::{DateTime, Utc};
use maud::{DOCTYPE, Markup, html};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};

// --- Modelos de Dados ---

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Noticia {
	id: String,
	titulo: String,
	resumo: String,
	url: String,
	data_publicacao: DateTime<Utc>,
	lida: bool,
}

// Estado agora guarda o Cliente e a URL base da API
struct AppState {
	http_client: Client,
	api_url: String,
}

struct AppError(anyhow::Error);

impl IntoResponse for AppError {
	fn into_response(self) -> Response {
		error!("Erro interno: {}", self.0);
		(
			axum::http::StatusCode::INTERNAL_SERVER_ERROR,
			format!("Erro no Dashboard: {}", self.0),
		)
			.into_response()
	}
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
	fn from(err: E) -> Self {
		Self(err.into())
	}
}

// --- Main ---

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	tracing_subscriber::fmt::init();

	// Configurações via ambiente
	let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
	// Se não houver variável, usa o localhost como padrão
	let api_url =
		std::env::var("FILTERFLOW_API_URL").unwrap_or_else(|_| "http://localhost:4000".to_string());

	info!("Iniciando Dash. Consumindo API em: {}", api_url);

	let shared_state = Arc::new(AppState {
		http_client: Client::new(),
		api_url,
	});

	let app = Router::new()
		.route("/", get(home_handler))
		.layer(TraceLayer::new_for_http())
		.layer(CompressionLayer::new())
		.with_state(shared_state);

	let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
	info!("Dashboard rodando em http://localhost:{}", port);

	axum::serve(listener, app).await?;
	Ok(())
}

// --- Handlers ---

async fn home_handler(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
	// Monta a URL final usando a string do estado
	let endpoint = format!("{}/api/noticias", state.api_url);

	let noticias: Vec<Noticia> = state
		.http_client
		.get(&endpoint)
		.send()
		.await?
		.json()
		.await?;

	let html_body = render_layout("Dash", render_home(noticias, 1, 10));

	Ok(html_body)
}

// --- Templates com Maud ---

fn render_layout(title: &str, content: Markup) -> Markup {
	html! {
		(DOCTYPE)
		html lang="pt-br" {
			head {
				meta charset="utf-8";
				meta name="viewport" content="width=device-width, initial-scale=1";
				title { (title) }
				script src="https://unpkg.com/htmx.org@1.9.10" {}
				script src="https://unpkg.com/htmx.org/dist/ext/json-enc.js" {}
				style { (css_styles()) }
			}
			body {
				header {
					h1 { "FilterFlow Dashboard" }
					p { "Monitoramento de notícias relevantes" }
				}
				main { (content) }
				footer {
					p { "Rodando em Rust e usando dados coletados pelo FilterFlow" }
				}
			}
		}
	}
}

fn render_home(noticias: Vec<Noticia>, current_page: usize, _total_pages: usize) -> Markup {
	// Note que removi o ponto e vírgula final para ser o retorno da função
	html! {
		div class="container"
			hx-get="/"
			hx-trigger="every 60s"
			hx-select=".container"
			hx-swap="outerHTML"
			hx-ext="json-enc"
		{
		@for noticia in &noticias {
			  details class={"card " @if noticia.lida { "read" } @else { "unread" }}
				 hx-post="http://localhost:4000/api/noticias/ler"
				 hx-ext="json-enc"
				 hx-trigger="toggle"
				 // Corrigido: Usando aspas simples externas para facilitar o JSON interno
				 hx-vals=(format!(r#"{{"id": "{}"}}"#, noticia.id))
				 hx-swap="none"
				 // Transição visual imediata no clique
				 onclick="this.classList.remove('unread'); this.classList.add('read'); let badge = this.querySelector('.badge-new'); if(badge) { badge.innerText='LIDA'; badge.style.background='#444'; }"
			  {
				 summary class="card-header" {
						div class="header-content" {
						   // Sempre renderiza o badge, mas muda o conteúdo/classe baseado no estado
						   span class="badge-new" {
							  @if noticia.lida { "Lida" } @else { "Nova" }
						   }

						   span class="date" { (noticia.data_publicacao
							  .with_timezone(&chrono::Local)
							  .format("%d/%m/%Y %H:%M")) }
						   strong { (noticia.titulo) }
						}
				 }
				 div class="card-body" {
						p { (noticia.resumo) }
						a href=(noticia.url) target="_blank" class="btn" { "Ler fonte original →" }
				 }
			  }
		   }

			div class="pagination" {
				@if current_page > 1 {
					a href={"?page=" (current_page - 1)} { "« Anterior" }
				}
				span { " Página " (current_page) " " }
				@if noticias.len() == 10 {
					a href={"?page=" (current_page + 1)} { "Próxima »" }
				}
			}
		}
	}
}

fn css_styles() -> String {
	r#"
    body { font-family: system-ui, -apple-system, sans-serif; background: #121212; color: #e0e0e0; margin: 0; padding: 20px; line-height: 1.6; }
    header { text-align: center; margin-bottom: 40px; border-bottom: 1px solid #333; padding-bottom: 20px; }
    h1 { color: #4fbcff; margin: 0; }
    .container { max-width: 900px; margin: 0 auto; }
    .card { background: #1e1e1e; border: 1px solid #333; border-radius: 8px; margin-bottom: 12px; transition: border-color 0.2s; }
    .card:hover { border-color: #4fbcff; }
    .card-header { padding: 15px; cursor: pointer; list-style: none; }
    .card-header::-webkit-details-marker { display: none; }
    .header-content { display: flex; flex-direction: column; }
    .date { font-size: 0.8rem; color: #888; margin-bottom: 5px; }
    .card-body { padding: 0 20px 20px 20px; border-top: 1px solid #333; margin-top: 10px; padding-top: 15px; }
    .btn { display: inline-block; margin-top: 15px; padding: 8px 16px; background: #4fbcff; color: #000; text-decoration: none; border-radius: 4px; font-weight: bold; font-size: 0.9rem; }
    .btn:hover { background: #7dd3ff; }
    .pagination { text-align: center; margin-top: 30px; }
    .pagination a { color: #4fbcff; text-decoration: none; padding: 0 10px; }
    /* Notícias Não Lidas: Destaque com borda azul à esquerda */
    .unread {
        border-left: 5px solid #4fbcff !important;
    }

    /* Notícias Lidas: Opacidade reduzida para tirar o foco visual */
    .read {
        opacity: 0.5;
        filter: grayscale(0.5);
        border-left: 5px solid #333 !important;
    }

    /* Estilo do Rótulo "Nova" */
    .badge-new {
        background: #4fbcff;
        color: #121212;
        padding: 2px 8px;
        border-radius: 4px;
        font-size: 0.7rem;
        font-weight: bold;
        margin-right: 12px;
        text-transform: uppercase;
        display: inline-block;
        vertical-align: middle;
    }

    /* Estilo quando a notícia for lida */
    .read .badge-new {
        background: #444 !important; /* Cinza escuro */
        color: #888 !important;
        display: inline-block !important; /* Garante que apareça como LIDA */
    }
    .card.read .badge-new {
        backgroud: #444 !important;
        color: #888 !important;
    }
    /* MÁGICA INSTANTÂNEA:
        Assim que você clica no details e ele abre, o badge some
        mesmo antes do refresh de 60s do HTMX */
    details[open] .badge-new {
        content: 'LIDA'
        display: none;
    }

    /* Opcional: faz o card parecer lido no momento da expansão */
    details[open] {
        opacity: 0.7;
        border-left: 5px solid #333 !important;
    }
    @media (min-width: 600px) {
        .header-content { flex-direction: row; align-items: center; gap: 20px; }
        .date { margin-bottom: 0; min-width: 140px; }
    }
    "#.to_string()
}
