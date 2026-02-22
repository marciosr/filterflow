use async_recursion::async_recursion;
use axum::{Json, Router, extract::State, response::IntoResponse, routing::get, routing::post};
use chrono::{DateTime, Duration, Local, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, Proxy};
use rss::{Channel, Item};
use serde::{Deserialize, Serialize};
use serde_json;
use sitemap::{
	reader::{SiteMapEntity, SiteMapReader},
	structs::LastMod,
};
use sled::Db;
use std::{
	error::Error,
	fs::{OpenOptions, create_dir_all},
	io::{BufReader, Error as io_error, Write},
	path::PathBuf,
	sync::Arc,
	time::Duration as StdDuration,
	time::Instant,
};
use tokio::time;
use tower_http::cors::Any;
use tower_http::cors::CorsLayer;
use url::Url;

// --- Constantes Globais ---
const CONFIG_FILE: &str = "filterflow_config.toml";
const DB_PATH: &str = "filterflow_data";
const IRRELEVANT_CACHE_TREE: &str = "irrelevant_cache";
static FIM_REGEX_LAZY: Lazy<Regex> =
	Lazy::new(|| Regex::new(r"(?s)Fim<\/th>.*?<td>(.*?)<\/td>").unwrap());

// Constantes ANSI para formatação de saída no terminal
const BOLD: &str = "\x1b[1m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const RESET: &str = "\x1b[0m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_YELLOW: &str = "\x1b[1;33m";

// --- Estruturas de Configuração (Lidas do TOML) ---

#[derive(Debug, Deserialize, Clone)]
struct FeedConfig {
	nome: String,
	url: String,
}

#[derive(Debug, Deserialize, Clone)]
struct SitemapConfig {
	nome: String,
	url: String,
}

#[derive(Debug, Deserialize, Clone)] // Clone necessário para o Arc
struct FiltroConfig {
	indicadores_relevancia: Vec<String>,
	indicadores_irrelevancia: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)] // Clone necessário para o Arc
struct GeralConfig {
	// PARÂMETROS EXISTENTES
	endereco: String,
	intervalo_minutos: u64,
	modelo_resumo: String,
	diretorio_arquivamento: String,
	user_agent: String,
	ocultar_latencia: Option<bool>,

	// NOVOS PARÂMETROS LLM (Sem Timeouts!)
	max_tokens_filtro: u32,
	temperature_filtro: f32,
	max_tokens_resumo: u32,
	temperature_resumo: f32,
	prompt_system_filtro: String,
	//prompt_system_resumo: String,
	prompt_user_resumo_template: String,
}

#[derive(Debug, Deserialize, Clone)]
struct ProxyConfig {
	pub usar_proxy: bool,
	pub endereco_proxy: String,
}

#[derive(Debug, Deserialize)]
struct Config {
	geral: GeralConfig,
	filtro: FiltroConfig,
	feeds: Vec<FeedConfig>,
	proxy: ProxyConfig,
	sitemaps: Vec<SitemapConfig>,
}

// --- Estruturas para Comunicação com a API OpenAI/LM Studio ---

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Message {
	role: String,
	content: String,
}

#[derive(Debug, serde::Serialize)]
struct ChatCompletionRequest {
	model: String,
	messages: Vec<Message>,
	max_tokens: u32,
	temperature: f32,
	stream: bool,
}

#[derive(Debug, serde::Deserialize)]
struct ChatCompletionResponse {
	choices: Vec<Choice>,
}

#[derive(Debug, serde::Deserialize)]
struct Choice {
	message: Message,
}

#[derive(Debug, Deserialize)]
struct RespostaProcessada {
	resumo: String,
	keywords: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Noticia {
	id: String,
	titulo: String,
	resumo: String,
	url: String,
	data_publicacao: DateTime<Utc>,
	lida: bool,
}

#[derive(Deserialize)]
pub struct MarcarLidaRequest {
	pub id: String,
}
// =================================================================
// FUNÇÕES DE COMUNICAÇÃO LLM (TIMEOUTS FIXOS REVERTIDOS)
// =================================================================

/// Função de resumo das notícias por llm
async fn resuma_noticia(
	client: &Client,
	title: &str,
	description: &str,
	config: Arc<GeralConfig>,
) -> Result<RespostaProcessada, Box<dyn std::error::Error>> {
	// Prompt para extrair JSON com resumo e palavras-chave
	let prompt_content = format!(
		"{} {} {}",
		&config.prompt_user_resumo_template, title, description
	);

	let request_body = ChatCompletionRequest {
		model: config.modelo_resumo.to_string(),
		messages: vec![
			Message {
				role: "system".to_string(),
				content: "Você é um extrator de dados JSON.".to_string(),
			},
			Message {
				role: "user".to_string(),
				content: prompt_content,
			},
		],
		max_tokens: config.max_tokens_resumo,
		temperature: config.temperature_resumo,
		stream: false,
	};

	let response = client
		.post(&config.endereco)
		.json(&request_body)
		.send()
		.await?;
	let response_json: ChatCompletionResponse = response.json().await?;

	if let Some(choice) = response_json.choices.into_iter().next() {
		let content = choice.message.content.trim();
		// Limpeza básica de blocos de código se o LLM os incluir
		let json_str = content
			.trim_start_matches("```json")
			.trim_start_matches("```")
			.trim_end_matches("```")
			.trim();
		let processada: RespostaProcessada = serde_json::from_str(json_str)?;
		return Ok(processada);
	}

	Err("Resposta do LLM vazia".into())
}

/// Filtro de relevância de notícias executado por llm
async fn filtre_noticia(
	client: &Client,
	title: &str,
	description: &str,
	filtro_config: Arc<FiltroConfig>, // Recebe a config de filtro
	geral_config: Arc<GeralConfig>,   // Recebe a config geral
) -> Result<bool, Box<dyn std::error::Error>> {
	// Acesso aos termos
	let termos1 = filtro_config.indicadores_relevancia.join(", ");
	let termos2 = filtro_config.indicadores_irrelevancia.join(", ");

	// 1. Injeção da variável no template
	let prompt_content = format!(
		"Avalie a relevância da notícia. Título: '{}' | Descrição: '{}'.\n\nCondições:\n1. A notícia é **principalmente** sobre um ou mais destes tópicos de INCLUSÃO: ({})\n2. A notícia **NÃO pode** estar relacionado a nenhum dos seguintes termos: ({}).\n\nSe AMBAS as condições forem satisfeitas, responda '1'. Caso contrário, responda '0'. Responda APENAS '1' ou '0'.",
		title, description, termos1, termos2
	);

	let request_body = ChatCompletionRequest {
		model: geral_config.modelo_resumo.to_string(),
		messages: vec![
			Message {
				role: "system".to_string(),
				content: geral_config.prompt_system_filtro.clone(),
			},
			Message {
				role: "user".to_string(),
				content: prompt_content,
			},
		],
		max_tokens: geral_config.max_tokens_filtro,
		temperature: geral_config.temperature_filtro,
		stream: false,
	};

	let start_time = Instant::now();

	// TIMEOUT FIXO REVERTIDO PARA 10s
	let response = client
		.post(&geral_config.endereco)
		.json(&request_body)
		.timeout(StdDuration::from_secs(15))
		.send()
		.await?;

	let duration = start_time.elapsed();
	if !geral_config.ocultar_latencia.unwrap_or(true) {
		eprintln!(
			"[LATÊNCIA FILTRO] Tempo LLM: {:.2?} (Tamanho da Resposta: {} bytes)",
			duration,
			response.content_length().unwrap_or(0)
		);
	}

	if !response.status().is_success() {
		let status = response.status();
		let error_body = response.text().await.unwrap_or_else(|_| "N/A".to_string());
		return Err(format!(
			"Erro de Status HTTP na Filtragem ({}): Status: {}. Corpo: {}",
			geral_config.endereco, status, error_body
		)
		.into());
	}

	let response_json: ChatCompletionResponse = response.json().await?;

	if let Some(choice) = response_json.choices.into_iter().next() {
		let llm_output_text = choice.message.content;
		let response_text = llm_output_text.trim();

		let is_relevant = match response_text {
			"1" => true,
			"0" => false,
			_ => {
				eprintln!(
					"🔥 ALERTA DE FORMATO LLM 🔥: LLM falhou ao retornar '1' ou '0'. Resposta: '{}'. Notícia ignorada.",
					response_text
				);
				false
			}
		};

		return Ok(is_relevant);
	}

	Ok(false)
}

fn caminho_relativo_diretorio(path: &str) -> String {
	if path.starts_with("~/") {
		if let Some(home) = dirs::home_dir() {
			return path.replacen("~", &home.to_string_lossy(), 1);
		}
	}
	path.to_string()
}

fn salvar_em_markdown(
	titulo: &str,
	link: &str,
	dados: &RespostaProcessada,
	archive_path: String,
) -> std::io::Result<()> {
	create_dir_all(archive_path.as_str())?;
	let data_atual = Local::now().format("%Y-%m-%d").to_string();
	let caminho = std::path::PathBuf::from(archive_path).join(format!("{}.md", data_atual));

	let mut file = OpenOptions::new().create(true).append(true).open(caminho)?;

	let tags = dados
		.keywords
		.iter()
		.map(|k| format!("#{}", k.to_lowercase().replace(' ', "_")))
		.collect::<Vec<String>>()
		.join(" ");

	let entrada = format!(
		"## {}\n\n**Link:** [{}]({})\n\n> {}\n\n**Tags:** {}\n\n*Hora: {}* \n\n---\n\n",
		titulo,
		titulo,
		link,
		dados.resumo,
		tags,
		Local::now().format("%H:%M:%S"),
	);

	file.write_all(entrada.as_bytes())
}

// =================================================================
// FUNÇÕES AUXILIARES E DB
// =================================================================

fn clean_html_content(html: &str) -> String {
	static HTML_TAG_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]*>").unwrap());
	let clean_text = HTML_TAG_REGEX.replace_all(html, " ").to_string();

	let clean_text = clean_text
		.replace('\n', " ")
		.replace('\r', " ")
		.replace("  ", " ")
		.replace("  ", " ")
		.replace("📎", "")
		.replace("https://", "")
		.replace("http://", "")
		.trim()
		.to_string();

	clean_text
}

fn db_init_trees(db_path: &str) -> Result<sled::Db, sled::Error> {
	let db = sled::open(db_path)?;
	let _irrelevant_cache_tree = db.open_tree(IRRELEVANT_CACHE_TREE)?;
	Ok(db)
}

fn db_is_irrelevant(db: &Db, link: &str) -> Result<bool, io_error> {
	let tree = db.open_tree(IRRELEVANT_CACHE_TREE)?;
	let exists = tree.contains_key(link.as_bytes())?;
	Ok(exists)
}

fn db_cache_as_irrelevant(db: &Db, link: &str) -> Result<(), io_error> {
	let tree = db.open_tree(IRRELEVANT_CACHE_TREE)?;
	tree.insert(link.as_bytes(), b"1")?;
	tree.flush()?;
	Ok(())
}

/// Verifica se o item de alerta do INMET expirou, usando o campo 'Fim' da tabela na descrição.
#[allow(unused)]
fn is_inmet_alert_expired(item: &Item) -> bool {
	let title = item.title().unwrap_or("[Sem Título]");
	let description = item.description().unwrap_or("");

	// ----------------------------------------------------
	// 1. Tentar extrair a data de FIM da DESCRIÇÃO
	// ----------------------------------------------------
	if let Some(caps) = FIM_REGEX_LAZY.captures(description) {
		if let Some(date_time_match) = caps.get(1) {
			let date_str_raw = date_time_match.as_str(); // Ex: "2025-10-28 10:00:00.0"

			let date_str_iso_prep = date_str_raw.trim().replace(' ', "T");
			let final_date_str = date_str_iso_prep.trim_end_matches(".0").to_string();

			match DateTime::parse_from_rfc3339(&format!("{}Z", final_date_str)) {
				Ok(expiration_dt) => {
					let now = Utc::now();
					let is_expired = expiration_dt.with_timezone(&Utc) < now;
					return is_expired;
				}
				Err(e) => {
					eprintln!(
						"⚠️ ERRO PARSE ⚠️: Falha ao analisar data '{}' da Descrição. Erro: {}",
						final_date_str, e
					);
					// Continua para o fallback pubDate se o parse falhar
				}
			}
		}
	}

	// ----------------------------------------------------
	// 2. FALLBACK: Tentar data de publicação (<pubDate>)
	// ----------------------------------------------------
	// Log de fallback MANTIDO para diagnosticar falha na FIM_REGEX_LAZY.
	eprintln!(
		"⚠️ INMET: Aviso '{}' sem 'Fim' na Descrição ou Erro de Parse. Usando <pubDate> como fallback.",
		title
	);

	if let Some(pub_date_str) = item.pub_date() {
		// pubDate usa o formato RFC2822 (Ex: Sun, 26 Oct 2025 07:00:00 -0300)
		match DateTime::parse_from_rfc2822(pub_date_str) {
			Ok(pub_dt) => {
				let now = Utc::now();
				// Assumir um alerta não pode ter mais de 72 horas (3 dias)
				let max_valid_duration = Duration::hours(72);

				let is_too_old = (now - pub_dt.with_timezone(&Utc)) > max_valid_duration;

				return is_too_old;
			}
			Err(_) => {
				eprintln!(
					"⚠️ ERRO PARSE ⚠️: Falha ao analisar <pubDate> '{}' para '{}'. Tratado como VÁLIDO.",
					pub_date_str, title
				);
			}
		}
	}

	// 3. Se tudo falhar ou estiver dentro do prazo de 72h, tratar como VÁLIDO
	false
}

/// Validação de URLs, checando host e esquema HTTP/HTTPS.
fn validate_url(url: &str) -> Result<(), Box<dyn Error>> {
	let parsed = Url::parse(url)?;

	if !parsed.has_host() {
		return Err("URL inválida: Endereço do host ausente.".into());
	}

	let scheme = parsed.scheme();
	if scheme != "http" && scheme != "https" {
		return Err(format!(
			"URL inválida: Apenas esquemas 'http' ou 'https' são permitidos, encontrado '{}'.",
			scheme
		)
		.into());
	}

	Ok(())
}

/// Validação semântica da configuração lida do TOML.
fn validate_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
	// 1. Validação de Intervalo
	if config.geral.intervalo_minutos < 2 {
		return Err("O intervalo de atualização não pode menor que 2 minutos.".into());
	}
	// 2. Validação da URL do LLM
	if let Err(e) = validate_url(&config.geral.endereco) {
		return Err(format!("Erro na URL do LLM ({}): {}", &config.geral.endereco, e).into());
	}

	// 3. Validação Condicional do Proxy
	if config.proxy.usar_proxy {
		if let Err(e) = validate_url(&config.proxy.endereco_proxy) {
			return Err(format!(
				"Erro na URL do Proxy ({}): {}",
				&config.proxy.endereco_proxy, e
			)
			.into());
		}
	}

	// 4. Validação das URLs dos Feeds
	for feed in &config.feeds {
		if let Err(e) = validate_url(&feed.url) {
			return Err(format!("Erro na URL do Feed '{}': {}", feed.nome, e).into());
		}
	}

	// 5. Validação das URLs dos Sitemaps
	for sitemap in &config.sitemaps {
		if let Err(e) = validate_url(&sitemap.url) {
			return Err(format!("Erro na URL do Sitemap '{}': {}", sitemap.nome, e).into());
		}
	}

	// 6. Validação dos Templates LLM
	let summary_template = &config.geral.prompt_user_resumo_template;
	if summary_template.split('{').count() - 1 != 2 {
		eprintln!(
			"⚠️ ALERTA ⚠️: prompt_user_resumo_template deve ter exatamente 2 placeholders {{}} (Título e Descrição). Atual: {}",
			summary_template
		);
	}

	Ok(())
}

/// Retorna o caminho para o arquivo de configuração TOML: ~/.config/filterflow/filterflow_config.toml
fn get_config_path() -> PathBuf {
	let mut config_path = dirs::config_dir().unwrap_or_else(|| {
		eprintln!(
			"[filterflow] Aviso: Não foi possível determinar o diretório de configuração. Usando ./"
		);
		PathBuf::from(".")
	});
	config_path.push("filterflow");
	let _ = create_dir_all(&config_path);
	config_path.push(CONFIG_FILE);
	config_path
}

/// Carregar a configuração
fn carregar_config() -> Result<Config, Box<dyn std::error::Error>> {
	let config_content = std::fs::read_to_string(get_config_path())?;
	let config: Config = toml::from_str(&config_content)?;
	validate_config(&config)?;
	Ok(config)
}

// =================================================================
// FUNÇÕES DE PROCESSAMENTO CENTRAL
// =================================================================

/// Lógica central de filtragem e resumo, usada por RSS e Sitemaps.
/// Retorna true se a notícia foi relevante e processada.
async fn process_single_item_logic(
	client: &Client,
	db: &Arc<sled::Db>, // Recebe Arc<Db>
	link: &str,
	title: &str,
	description: &str,
	filtro_config: Arc<FiltroConfig>,
	geral_config: Arc<GeralConfig>,
) -> Result<bool, Box<dyn Error>> {
	let db_key = link.as_bytes();

	// 1. Checagem de Duplicidade (Irrelevância e Processado)
	match db_is_irrelevant(db, link) {
		Ok(true) => return Ok(false), // Irrelevant, skip
		Err(e) => {
			eprintln!("Erro ao verificar cache de irrelevância: {}", e);
			return Err(e.into());
		}
		Ok(false) => {}
	}

	if db.contains_key(db_key)? {
		return Ok(false); // Already processed, skip
	}

	// 2. Filtragem Semântica (Fase 1: Rápida)
	let is_relevant = match filtre_noticia(
		client,
		title,
		description,
		Arc::clone(&filtro_config), // Propaga o Arc
		Arc::clone(&geral_config),  // Propaga o Arc
	)
	.await
	{
		Ok(result) => result,
		Err(e) => {
			eprintln!("\n[ERRO LLM] Falha na filtragem da notícia: {}", e);
			eprintln!(
				"Por favor, verifique se o LLM está rodando em {}",
				geral_config.endereco
			);
			return Ok(false); // Tratamos como irrelevante e continuamos.
		}
	};

	if is_relevant {
		// Notícia relevante! Passa para o resumo.
		println!(
			"\n\n{}[NOVA E RELEVANTE]{} Título: {}{}{}",
			BOLD_GREEN, RESET, BOLD, title, RESET
		);
		println!("{}Link:{} {}", BOLD, RESET, link);

		// 3. Fase 2: RESUMO E KEYWORDS
		match resuma_noticia(client, title, description, Arc::clone(&geral_config)).await {
			Ok(dados) => {
				// 1. Criar a struct Noticia com os dados reais
				let nova_noticia = Noticia {
					id: link.to_string(), // ou um hash
					titulo: title.to_string(),
					resumo: dados.resumo.clone(),
					url: link.to_string(),
					data_publicacao: Utc::now(),
					lida: false,
				};

				println!("\n{}Resumo:\n {}{}\n", BOLD_YELLOW, dados.resumo, RESET);
				println!(
					"{}Palavras-chave: {:?}{}",
					BOLD_YELLOW, dados.keywords, RESET
				);

				// 2. Converter para bytes JSON
				if let Ok(noticia_bytes) = serde_json::to_vec(&nova_noticia) {
					// 3. SALVAR A NOTÍCIA COMPLETA NO DB (em vez de apenas "processed")
					if let Err(e) = db.insert(db_key, noticia_bytes) {
						eprintln!("[ERRO DB] Falha ao salvar Noticia: {}", e);
					}
					let _ = db.flush(); // Garante a gravação no NVMe
				}

				// Salva notícias relevantes de forma estruturada em diretório prédeterminado
				if let Err(e) = salvar_em_markdown(
					title,
					link,
					&dados,
					caminho_relativo_diretorio(&geral_config.diretorio_arquivamento.as_str()),
				) {
					eprintln!("[ERRO] Falha ao arquivar Markdown: {}", e);
				}
			}
			Err(e) => {
				eprintln!("\n[ERRO LLM] Falha ao resumir/extrair tags: {}", e);
			}
		}

		// 4. Salvar no DB (apenas se for relevante e processada)
		//if let Err(e) = db.insert(db_key, b"processed") {
		//	eprintln!("[ERRO DB] Falha ao salvar na Árvore Principal: {}", e);
		//}
		return Ok(true); // Processed as relevant
	} else {
		// 5. Se irrelevante (LLM retornou '0'), salvar no cache
		if let Err(e) = db_cache_as_irrelevant(db, link) {
			eprintln!("[ERRO DB] Falha ao salvar no cache de irrelevância: {}", e);
		}
		return Ok(false); // Irrelevant
	}
}

// =================================================================
// FUNÇÕES DE PROCESSAMENTO DE FEEDS RSS
// =================================================================

async fn process_feed(
	client: &Client,
	db: &Arc<sled::Db>,
	feed: &FeedConfig,
	filtro_config: Arc<FiltroConfig>,
	geral_config: Arc<GeralConfig>,
) -> Result<(), Box<dyn std::error::Error>> {
	print!("--- Processando Fonte: {}{}{} ---", BOLD, feed.nome, RESET);

	// 1. Faz a requisição HTTP
	let response = match client
		.get(&feed.url)
		.timeout(StdDuration::from_secs(20))
		.send()
		.await
	{
		Ok(r) => r.bytes().await?,
		Err(e) => {
			eprintln!("{}Erro de requisição: {}{}", BOLD, e, RESET);
			return Ok(());
		}
	};

	// 2. Analisa o XML
	let channel = Channel::read_from(&response[..])?;
	let mut novas_noticias = 0;

	// 3. Itera sobre os itens (notícias)
	for item in channel.items() {
		let link = item.link().unwrap_or_default().to_string();
		if link.is_empty() {
			continue;
		}

		// --- FILTRAGEM DE DATA PARA ALERTAS (INMET) ---
		if feed.nome.contains("INMET") {
			if is_inmet_alert_expired(item) {
				if let Some(link_str) = item.link() {
					if let Err(e) = db_cache_as_irrelevant(db, link_str) {
						eprintln!("[ERRO DB] Falha ao salvar alerta expirado no cache: {}", e);
					}
				}
				continue;
			}
		}
		// --------------------------------------------------

		// --- EXTRAÇÃO DE DADOS ---
		let title = item.title().unwrap_or(&link).to_string();

		let description_raw = item
			.content()
			.or_else(|| item.description())
			.unwrap_or("")
			.to_string();

		let description = if description_raw.trim().starts_with("<ol>") {
			"".to_string()
		} else {
			clean_html_content(&description_raw)
		};
		// --------------------------------------------------

		// 4. Processamento Principal (LLM/DB)
		match process_single_item_logic(
			client,
			db,
			&link,
			&title,
			&description,
			Arc::clone(&filtro_config),
			Arc::clone(&geral_config),
		)
		.await
		{
			Ok(true) => novas_noticias += 1, // Relevante e processada
			Ok(false) => continue,           // Irrelevante ou já em cache
			Err(e) => {
				eprintln!(
					"[ERRO DE PROCESSAMENTO DE ITEM] Falha na lógica central para '{}': {}",
					title, e
				);
				continue;
			}
		}
	}

	if novas_noticias > 0 {
		println!(
			"\n{}*** {} NOVAS NOTÍCIAS RELEVANTES ENCONTRADAS ***{}",
			BOLD_GREEN, novas_noticias, RESET
		);
	} else {
		print!(" Atualizado ✅\n");
	}

	Ok(())
}

// =================================================================
// FUNÇÕES DE PROCESSAMENTO DE SITEMAPS
// =================================================================

/// Função auxiliar para download do conteúdo (GZIP-aware, com timeout e erro HTTP)
async fn fetch_sitemap_content(client: &Client, url: &str) -> Result<Vec<u8>, Box<dyn Error>> {
	// TIMEOUT FIXO REVERTIDO PARA 30s
	let response = client
		.get(url)
		.timeout(StdDuration::from_secs(30))
		.send()
		.await?;

	if !response.status().is_success() {
		return Err(format!(
			"Erro de Status HTTP ao baixar Sitemap ({}): {}",
			url,
			response.status()
		)
		.into());
	}

	// O .bytes() lida automaticamente com compressão GZIP (.xml.gz)
	response
		.bytes()
		.await
		.map(|b| b.to_vec())
		.map_err(|e| e.into())
}

/// Processa um Sitemap (ou Sitemap Index) de forma recursiva.
#[async_recursion]
async fn processar_sitemap(
	client: &Client,
	db: &Arc<sled::Db>,
	sitemap_config: &SitemapConfig,
	url_para_baixar: &str,
	filtro_config: Arc<FiltroConfig>,
	geral_config: Arc<GeralConfig>,
) -> Result<u32, Box<dyn Error>> {
	let mut urls_processadas = 0;

	print!("\n\n[INFO SITEMAP] Baixando: {}", url_para_baixar);

	// 1. Faz a requisição HTTP (Baixa o XML)
	let sitemap_data = match fetch_sitemap_content(client, url_para_baixar).await {
		Ok(data) => data,
		Err(e) => {
			eprintln!("[ERRO SITEMAP] Falha ao baixar {}: {}", url_para_baixar, e);
			return Ok(0);
		}
	};

	// 2. Analisa o XML
	let cursor = BufReader::new(sitemap_data.as_slice());
	let reader = SiteMapReader::new(cursor);

	for entity in reader {
		match entity {
			SiteMapEntity::Url(url_entry) => {
				let link = url_entry
					.loc
					.get_url()
					.map(|url| url.to_string())
					.unwrap_or_else(|| {
						eprintln!(
							"[ERRO SITEMAP] Entidade URL sem tag <loc> válida em {}",
							url_para_baixar
						);
						"".to_string()
					});

				if link.is_empty() {
					continue;
				}

				let last_modified_str = match &url_entry.lastmod {
					LastMod::DateTime(dt) => dt.to_string(),
					_ => "[N/A]".to_string(),
				};

				let title = format!("[Sitemap] {}", link);
				let description = format!("Última modificação: {}", last_modified_str);

				// 5. Processamento Principal (LLM/DB)
				match process_single_item_logic(
					client,
					db,
					&link,
					&title,
					&description,
					Arc::clone(&filtro_config),
					Arc::clone(&geral_config),
				)
				.await
				{
					Ok(true) => urls_processadas += 1,
					Ok(false) => continue,
					Err(e) => {
						eprintln!(
							"[ERRO SITEMAP/LLM] Falha na lógica central para '{}': {}",
							title, e
						);
						continue;
					}
				}
			}
			SiteMapEntity::SiteMap(sitemap_url) => {
				// RECURSÃO: Se for um Sitemap Index
				let sub_url = sitemap_url
					.loc
					.get_url()
					.map(|url| url.to_string())
					.unwrap_or_else(|| {
						eprintln!(
							"[ERRO SITEMAP] Sub-índice Sitemap sem tag <loc> válida em {}",
							url_para_baixar
						);
						"".to_string()
					});

				if sub_url.is_empty() {
					continue;
				}

				// Chamamos a função recursivamente para o novo arquivo Sitemap
				match processar_sitemap(
					client,
					db,
					sitemap_config,
					&sub_url,
					Arc::clone(&filtro_config),
					Arc::clone(&geral_config),
				)
				.await
				{
					Ok(count) => urls_processadas += count,
					Err(e) => eprintln!(
						"[ERRO SITEMAP/RECURSÃO] Falha ao processar sub-índice {}: {}",
						sub_url, e
					),
				}
			}
			// Catch-all para outras entidades (como Image, Video, etc.)
			_ => {
				// Ignorado: Entidade do Sitemap não é URL nem Sitemap Index.
			}
		}
	}

	Ok(urls_processadas)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	println!(
		"{}{}{}",
		BOLD, "--- FilterFlow: Agente de Notícias para LLMs locais ---", RESET
	);

	// 1. Inicialização de âncora (Carregar a config uma vez para iniciar o DB e logar)
	let initial_config = match carregar_config() {
		Ok(c) => c,
		Err(e) => {
			eprintln!(
				"[ERRO FATAL] Falha ao carregar configuração inicial '{}': {}",
				CONFIG_FILE, e
			);
			return Err(e);
		}
	};

	println!(
		"Configuração carregada. Modelo de Resumo: {}",
		initial_config.geral.modelo_resumo
	);
	println!(
		"Intervalo de Atualização: {} minutos",
		initial_config.geral.intervalo_minutos
	);
	println!(
		"\nIndicadores de relevância: \n{:?}",
		initial_config.filtro.indicadores_relevancia
	);
	println!(
		"\nIndicadores de irrelevância: \n{}{:?}{}",
		BOLD_RED, initial_config.filtro.indicadores_irrelevancia, RESET
	);

	// 2. Inicializar o Banco de Dados (sled) - DEVE SER ARC FORA DO LOOP
	let full_path = format!(
		"{}/{}",
		caminho_relativo_diretorio(&initial_config.geral.diretorio_arquivamento),
		DB_PATH
	);

	// Garante que a árvore de diretórios exista no SSD antes do sled abrir
	std::fs::create_dir_all(caminho_relativo_diretorio(
		&initial_config.geral.diretorio_arquivamento,
	))?;

	let db = db_init_trees(&full_path)?;
	let db_arc = Arc::new(db); // Empacota o DB em Arc para ser Thread-Safe
	println!("\nBanco de dados iniciado em: {}", DB_PATH);

	// Início da API de comunicação com o dash
	let db_api = Arc::clone(&db_arc);

	// Porta da API (pode colocar no seu config.toml depois)
	let api_port = "4000";

	tokio::spawn(async move {
		let cors = CorsLayer::new()
			.allow_origin(Any)
			.allow_methods(Any)
			.allow_headers(Any);

		let app = Router::new()
			.route("/api/noticias", get(api_listar_noticias))
			.route("/api/noticias/ler", post(api_marcar_como_lida))
			.with_state(db_api)
			.layer(cors);

		let listener = match tokio::net::TcpListener::bind(format!("0.0.0.0:{}", api_port)).await {
			Ok(l) => l,
			Err(e) => {
				eprintln!("[ERRO API] Não foi possível iniciar o servidor: {}", e);
				return;
			}
		};

		println!(
			"{}🚀 API de Notícias ativa em: http://localhost:{}/api/noticias{}",
			BOLD_GREEN, api_port, RESET
		);
		axum::serve(listener, app).await.unwrap();
	});
	// Fim da API
	let mut sleep_duration = StdDuration::from_secs(initial_config.geral.intervalo_minutos * 60);

	// --- Loop Principal de Atualização ---
	loop {
		let config = match carregar_config() {
			Ok(c) => c,
			Err(e) => {
				eprintln!(
					"[ERRO] Não foi possível recarregar o config: {}. Usando a configuração anterior.",
					e
				);
				time::sleep(sleep_duration).await;
				continue;
			}
		};

		// Recalcula o tempo de sleep se necessário
		let new_sleep_duration = StdDuration::from_secs(config.geral.intervalo_minutos * 60);
		if new_sleep_duration != sleep_duration {
			println!(
				"\n[INFO] Intervalo de atualização alterado para {} minutos.",
				config.geral.intervalo_minutos
			);
			sleep_duration = new_sleep_duration;
		}

		// 3. EMPACOTAMENTO EM ARC (Versão imutável desta iteração)
		let geral_config_arc = Arc::new(config.geral);
		let filtro_config_arc = Arc::new(config.filtro);
		let feeds_arc = Arc::new(config.feeds);
		let sitemaps_arc = Arc::new(config.sitemaps);

		// 4. Inicialização Condicional do Cliente HTTP (com Proxy)
		let mut client_builder = Client::builder();
		client_builder = client_builder.user_agent(&geral_config_arc.user_agent);

		if config.proxy.usar_proxy {
			match Proxy::https(&config.proxy.endereco_proxy) {
				Ok(proxy) => {
					eprintln!(
						"[INFO PROXY] Usando proxy em: {}",
						config.proxy.endereco_proxy
					);
					client_builder = client_builder.proxy(proxy);
				}
				Err(e) => {
					eprintln!(
						"\n[ERRO FATAL DE PROXY] Não foi possível configurar o proxy: {}. Verifique o formato.",
						e
					);
					time::sleep(sleep_duration).await;
					continue;
				}
			}
		}
		let client = client_builder.build().unwrap();

		// Bloco de logs do ciclo
		println!(
			"\n{}=================================================={}",
			BOLD, RESET
		);
		println!("{}        Iniciando ciclo de varredura...{}", BOLD, RESET);
		println!(
			"{}=================================================={}",
			BOLD, RESET
		);

		let agora = Local::now();
		println!(
			"      {}\n",
			agora.format("Data: %d/%m/%Y - Hora: %H:%M:%S")
		);

		let cycle_start_time = Instant::now();

		// 5. Processamento dos Feeds RSS
		for feed in feeds_arc.iter() {
			if let Err(e) = process_feed(
				&client,
				&db_arc, // Passando o Arc<Db>
				feed,
				Arc::clone(&filtro_config_arc),
				Arc::clone(&geral_config_arc),
			)
			.await
			{
				eprintln!("[ERRO] Falha ao processar feed '{}': {}", feed.nome, e);
			}
		}

		// 6. Processamento dos Sitemaps
		for sitemap_config in sitemaps_arc.iter() {
			print!(
				"--- Processando Fonte: {}{}{} ---",
				BOLD, sitemap_config.nome, RESET
			);

			let url_inicial = sitemap_config.url.to_string();

			match processar_sitemap(
				&client,
				&db_arc, // Passando o Arc<Db>
				sitemap_config,
				&url_inicial,
				Arc::clone(&filtro_config_arc),
				Arc::clone(&geral_config_arc),
			)
			.await
			{
				Ok(count) => {
					if count > 0 {
						println!(
							"\n{}*** {} NOVAS NOTÍCIAS RELEVANTES ENCONTRADAS PARA {} ***",
							BOLD_GREEN, count, sitemap_config.nome
						);
					} else {
						print!(" Atualizada ✅\n");
					}
				}
				Err(e) => {
					eprintln!(
						"[ERRO] Falha fatal ao processar sitemap '{}': {}",
						sitemap_config.nome, e
					);
				}
			}
		}

		let cycle_duration = cycle_start_time.elapsed();

		println!(
			"\n{} ***************** CICLO CONCLUÍDO *****************\n                  Tempo Total: {:.2?} {}",
			BOLD_GREEN, cycle_duration, RESET
		);

		// Taxa de utilização do tempo do sistema
		println!(
			"\n      {}Índice de utilização do sistema: {:.2}%{}",
			BOLD_YELLOW,
			(cycle_duration.as_secs_f32() / (geral_config_arc.intervalo_minutos as f32 * 60.0))
				* 100.0,
			RESET
		);

		let agora_final = Local::now();
		println!(
			"        {}\n",
			agora_final.format("     Data: %d/%m/%Y - Hora: %H:%M:%S")
		);

		// 7. Lógica de Espera
		println!(
			"\n{} [INFO] Aguardando {} minutos para a próxima checagem...{}",
			BOLD_GREEN, geral_config_arc.intervalo_minutos, RESET
		);

		time::sleep(sleep_duration).await;
	}
}

async fn api_listar_noticias(State(db): State<Arc<sled::Db>>) -> impl IntoResponse {
	// Itera diretamente no banco, filtrando apenas o que consegue converter com sucesso
	let mut noticias: Vec<Noticia> = db
		.iter()
		.filter_map(|item| item.ok()) // Descarta erros de leitura de hardware/disco
		.filter_map(|(_, v)| serde_json::from_slice::<Noticia>(&v).ok()) // Descarta dados malformados
		.collect();

	// Ordenação (opcional): se quiser as mais recentes primeiro
	noticias.sort_by(|a, b| b.data_publicacao.cmp(&a.data_publicacao));

	Json(noticias)
}

async fn api_marcar_como_lida(
	State(db): State<Arc<sled::Db>>,
	// 2. Mude de Json<String> para Json<MarcarLidaRequest>
	Json(payload): Json<MarcarLidaRequest>,
) -> impl IntoResponse {
	// 3. Use payload.id para acessar o valor
	let id = payload.id;

	if let Ok(Some(bytes)) = db.get(id.as_bytes()) {
		if let Ok(mut noticia) = serde_json::from_slice::<Noticia>(&bytes) {
			noticia.lida = true;

			if let Ok(noticia_bytes) = serde_json::to_vec(&noticia) {
				let _ = db.insert(id.as_bytes(), noticia_bytes);
				let _ = db.flush();
				return axum::http::StatusCode::OK;
			}
		}
	}
	axum::http::StatusCode::NOT_FOUND
}
