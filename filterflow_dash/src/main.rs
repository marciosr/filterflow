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

// --- Modelos de Dados --- (Mantidos iguais)

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Noticia {
	id: String,
	titulo: String,
	resumo: String,
	url: String,
	data_publicacao: DateTime<Utc>,
	lida: bool,
}

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

// --- Main --- (Mantido igual)

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	tracing_subscriber::fmt::init();

	let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
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

// --- Handlers --- (Mantido igual)

async fn home_handler(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
	let endpoint = format!("{}/api/noticias", state.api_url);

	let noticias: Vec<Noticia> = state
		.http_client
		.get(&endpoint)
		.send()
		.await?
		.json()
		.await?;

	Ok(render_full_page(render_home(noticias, 1, 10)))
}

// --- Templates com Maud ---

fn render_full_page(content: Markup) -> Markup {
	html! {
		(DOCTYPE)
		html lang="pt-br" {
			head {
				meta charset="utf-8";
				title { "FilterFlow Dash com Animação" }
				style { (css_styles()) }
			}
			body {
				header { h1 { "🌊 FilterFlow Dashboard" } }
				(content)
				script src="https://unpkg.com/htmx.org@1.9.10" {}
				// Certifique-se de incluir esta linha se o backend esperar JSON
				script src="https://unpkg.com/htmx.org/dist/ext/json-enc.js" {}
			}
		}
	}
}

fn render_home(noticias: Vec<Noticia>, current_page: usize, _total_pages: usize) -> Markup {
	html! {
		div class="container"
			hx-get="/"
			hx-trigger="every 60s"
			hx-select=".container"
			hx-swap="outerHTML"
			hx-ext="json-enc"
		{
			@for noticia in &noticias {
				details
					class={"card " @if noticia.lida { "read" } @else { "unread" }}
					// MUDANÇA 1: Usamos 'click' em vez de 'toggle' para maior confiabilidade
					hx-post="http://localhost:4000/api/noticias/ler"
					hx-trigger="click consume"
					hx-vals=(format!(r#"{{"id": "{}"}}"#, noticia.id))
					hx-swap="none"
					// MUDANÇA 2: Melhoramos o JS para não conflitar com o HTMX
					onclick="
                        // 1. Fecha os outros (Efeito Acordeão)
                        document.querySelectorAll('details.card[open]').forEach(el => {
                            if (el !== this) el.removeAttribute('open');
                        });

                        // 2. Só executa a troca de classe se ainda não for lida
                        if (this.classList.contains('unread')) {
                            this.classList.remove('unread');
                            this.classList.add('read');
                            let badge = this.querySelector('.badge-new');
                            if(badge) {
                                badge.innerText='LIDA';
                                badge.style.background='#444';
                            }
                        }
                    "
				{
					summary class="card-header" {
						div class="header-content" {
							span class="badge-new" {
								@if noticia.lida { "Lida" } @else { "Nova" }
							}
							span class="date" { (noticia.data_publicacao
								.with_timezone(&chrono::Local)
								.format("%d/%m/%Y %H:%M")) }
							strong { (noticia.titulo) }
						}
					}
					div class="card-body-wrapper" {
						div class="card-body-content" {
							p { (noticia.resumo) }
							a href=(noticia.url) target="_blank" class="btn" { "Ler fonte original →" }
						}
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
    /* Estilos base mantidos */
    body { font-family: system-ui, -apple-system, sans-serif; background: #121212; color: #e0e0e0; margin: 0; padding: 20px; line-height: 1.6; }
    header { text-align: center; margin-bottom: 40px; border-bottom: 1px solid #333; padding-bottom: 20px; }
    h1 { color: #4fbcff; margin: 0; }
    .container { max-width: 900px; margin: 0 auto; }

    /* MODIFICAÇÃO CSS 1: Card Base */
    .card {
        background: #1e1e1e;
        border: 1px solid #333;
        border-radius: 8px;
        margin-bottom: 12px;
        transition: border-color 0.3s, box-shadow 0.3s, opacity 0.3s, transform 0.3s ease-out;
        overflow: hidden; /* Importante para a animação de deslize */
    }
    .card:hover { border-color: #4fbcff; box-shadow: 0 4px 12px rgba(79, 188, 255, 0.1); }

    .card-header { padding: 15px; cursor: pointer; list-style: none; }
    .card-header::-webkit-details-marker { display: none; }
    .header-content { display: flex; flex-direction: column; }
    .date { font-size: 0.8rem; color: #888; margin-bottom: 5px; }

    /* MODIFICAÇÃO CSS 2: Estrutura da Animação */
    /* O container externo (wrapper) controla a altura */
    .card-body-wrapper {
        display: grid;
        grid-template-rows: 0fr; /* Começa com altura zero */
        transition: grid-template-rows 0.4s ease-in-out, opacity 0.3s ease-in;
        opacity: 0;
        visibility: hidden;
        background: #181818;
        border-top: 1px solid transparent;
    }

    /* O container interno (content) guarda o padding e conteúdo real */
    .card-body-content {
        overflow: hidden; /* Impede que o texto apareça antes da hora */
        padding: 0 20px; /* Padding lateral constante */
        transition: padding 0.4s ease-in-out;
    }

    /* MÁGICA CSS 3: Estado Aberto */
    details[open] .card-body-wrapper {
        grid-template-rows: 1fr; /* Expande para a altura total do conteúdo */
        opacity: 1;
        visibility: visible;
        border-top: 1px solid #333;
    }

    details[open] .card-body-content {
        padding: 20px; /* Padding vertical aparece na expansão */
    }

    /* Botão e paginação mantidos */
    .btn { display: inline-block; margin-top: 15px; padding: 8px 16px; background: #4fbcff; color: #000; text-decoration: none; border-radius: 4px; font-weight: bold; font-size: 0.9rem; transition: background 0.2s; }
    .btn:hover { background: #7dd3ff; }
    .pagination { text-align: center; margin-top: 30px; }
    .pagination a { color: #4fbcff; text-decoration: none; padding: 0 10px; }

    /* Estados de Lido/Nova mantidos */
    .unread { border-left: 5px solid #4fbcff !important; }
    .read { opacity: 0.6; border-left: 5px solid #333 !important; }
    .badge-new { background: #4fbcff; color: #121212; padding: 2px 8px; border-radius: 4px; font-size: 0.7rem; font-weight: bold; margin-right: 12px; text-transform: uppercase; display: inline-block; vertical-align: middle; }
    .read .badge-new { background: #444 !important; color: #888 !important; }
    details[open] .badge-new { display: none; }

    /* Destaque visual do card aberto */
    details[open] { border-color: #4fbcff; opacity: 1; transform: translateY(-2px); box-shadow: 0 4px 15px rgba(79, 188, 255, 0.15); }

    @media (min-width: 600px) {
        .header-content { flex-direction: row; align-items: center; gap: 20px; }
        .date { margin-bottom: 0; min-width: 140px; }
    }
    "#.to_string()
}
