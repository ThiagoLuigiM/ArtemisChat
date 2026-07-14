use notify::RecommendedWatcher;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

use crate::deepseek::slugify_category;

// Os 3 arquivos de "regras" são carregados no system prompt em toda geração.
// Exemplos aprovados NÃO entram aqui — eles ficam só no SQLite e são injetados
// como few-shot user/assistant pairs filtrados por categoria. Os arquivos
// `exemplos-{categoria}.md` no vault são apenas leitura humana no Obsidian.
const VAULT_RULE_FILES: usize = 3;

#[derive(Serialize, Clone, Default, Debug)]
pub struct VaultContext {
    pub estilo: String,
    pub evitar: String,
    pub campos_padrao: String,
}

impl VaultContext {
    pub fn total_chars(&self) -> usize {
        self.estilo.len() + self.evitar.len() + self.campos_padrao.len()
    }

    pub fn files_present(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.estilo.trim().is_empty() {
            out.push("estilo.md".into());
        }
        if !self.evitar.trim().is_empty() {
            out.push("evitar.md".into());
        }
        if !self.campos_padrao.trim().is_empty() {
            out.push("campos-padrao.md".into());
        }
        out
    }
}

#[derive(Serialize, Clone, Default, Debug)]
pub struct VaultStatus {
    pub path: Option<String>,
    pub last_loaded_ts: Option<u64>,
    pub files_present: Vec<String>,
    pub files_total: usize,
    pub total_chars: usize,
    pub error: Option<String>,
}

pub struct VaultLoader {
    path: Option<PathBuf>,
    context: VaultContext,
    status: VaultStatus,
}

impl VaultLoader {
    pub fn new() -> Self {
        Self {
            path: None,
            context: VaultContext::default(),
            status: VaultStatus::default(),
        }
    }

    pub fn set_path<P: Into<PathBuf>>(&mut self, path: P) {
        self.path = Some(path.into());
        self.reload();
    }

    pub fn reload(&mut self) {
        let Some(path) = self.path.clone() else {
            self.status = VaultStatus::default();
            self.context = VaultContext::default();
            return;
        };

        let mut ctx = VaultContext::default();
        let mut first_error: Option<String> = None;

        let files: [(&str, fn(&mut VaultContext, String)); 3] = [
            ("estilo.md", |c, s| c.estilo = s),
            ("evitar.md", |c, s| c.evitar = s),
            ("campos-padrao.md", |c, s| c.campos_padrao = s),
        ];

        for (name, set) in files {
            let file_path = path.join(name);
            if file_path.exists() {
                match fs::read_to_string(&file_path) {
                    Ok(content) => set(&mut ctx, content),
                    Err(e) => {
                        if first_error.is_none() {
                            first_error = Some(format!("erro lendo {}: {}", name, e));
                        }
                    }
                }
            }
        }

        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .ok();

        self.status = VaultStatus {
            path: Some(path.to_string_lossy().into_owned()),
            last_loaded_ts: ts,
            files_present: ctx.files_present(),
            files_total: VAULT_RULE_FILES,
            total_chars: ctx.total_chars(),
            error: first_error,
        };
        self.context = ctx;
    }

    pub fn context(&self) -> &VaultContext {
        &self.context
    }

    pub fn status(&self) -> &VaultStatus {
        &self.status
    }
}

pub struct Watcher {
    _debouncer: Debouncer<RecommendedWatcher>,
}

pub fn start_watcher(
    path: PathBuf,
    loader: Arc<RwLock<VaultLoader>>,
    app: AppHandle,
) -> anyhow::Result<Watcher> {
    let loader_for_handler = loader.clone();
    let app_for_handler = app.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        move |result: DebounceEventResult| {
            let relevant = match result {
                Ok(events) => events.iter().any(|e| {
                    // Reage apenas a mudanças nos arquivos de regras (estilo/evitar/campos-padrao).
                    // Mudanças em exemplos-*.md são ignoradas porque o sistema os trata como
                    // apenas-leitura-humana (a fonte de exemplos é o SQLite).
                    e.path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| matches!(n, "estilo.md" | "evitar.md" | "campos-padrao.md"))
                        .unwrap_or(false)
                }),
                Err(e) => {
                    tracing::warn!("vault watcher error: {:?}", e);
                    false
                }
            };
            if relevant {
                if let Ok(mut l) = loader_for_handler.write() {
                    l.reload();
                    let _ = app_for_handler.emit("vault-changed", l.status().clone());
                    tracing::info!("vault recarregado após mudança de arquivo de regras");
                }
            }
        },
    )?;

    debouncer
        .watcher()
        .watch(&path, notify::RecursiveMode::NonRecursive)?;

    Ok(Watcher {
        _debouncer: debouncer,
    })
}

/// Auto-curadoria por categoria.
/// Cria/appenda em `exemplos-{slug(category)}.md`. Arquivo cresce só na própria categoria,
/// evitando que toda a base vire um único monolito.
///
/// **A IA LÊ este arquivo inteiro** ao gerar nova devolutiva da mesma categoria — ele é
/// a fonte de verdade da injeção, não o SQLite. O usuário pode editar livremente no
/// Obsidian: adicionar exemplos manuais, comentários, instruções tipo "nesta categoria
/// sempre use formato X". O SQLite só guarda histórico bruto (para search/diff/etc).
pub fn append_to_category_examples(
    vault_path: &Path,
    category: &str,
    raw_input: &str,
    final_output: &str,
) -> anyhow::Result<PathBuf> {
    let slug = slugify_category(category);
    let filename = format!("exemplos-{}.md", slug);
    let file_path = vault_path.join(&filename);

    let date = chrono::Local::now().format("%d/%m/%Y %H:%M").to_string();
    let entry_block = format!(
        "\n\n---\n\n## Aprovado em {}\n\n**Entrada bruta:**\n\n```\n{}\n```\n\n**Devolutiva aprovada:**\n\n{}\n",
        date,
        raw_input.trim(),
        final_output.trim()
    );

    let existing = fs::read_to_string(&file_path).unwrap_or_default();
    let content = if existing.trim().is_empty() {
        format!(
            "# Exemplos aprovados — {}\n\n> Arquivo auto-curado pelo Artemis. A IA lê este arquivo INTEIRO ao gerar devolutivas desta categoria, incluindo qualquer texto que você adicionar manualmente (exemplos próprios, anotações, instruções específicas). Os blocos `## Aprovado em ...` são gerados automaticamente, mas você pode editar, remover, reordenar ou adicionar conteúdo livremente.\n{}",
            slug, entry_block
        )
    } else {
        let mut c = existing.trim_end().to_string();
        c.push_str(&entry_block);
        c
    };

    fs::write(&file_path, content)?;
    Ok(file_path)
}

/// Appenda sugestões aprovadas pelo usuário ao `evitar.md`.
/// Cada sugestão entra como `- ~~"expressão"~~ — motivo` (mesmo formato dos itens
/// que o usuário escreve manualmente). Um marker HTML `<!-- auto-aprendidos em DATA -->`
/// separa os blocos para que o usuário identifique facilmente o que foi auto-gerado
/// vs escrito por ele, e possa remover/editar livremente.
pub fn append_to_evitar(
    vault_path: &Path,
    suggestions: &[(String, String)], // (expression, reason)
) -> anyhow::Result<PathBuf> {
    let evitar_path = vault_path.join("evitar.md");
    let date = chrono::Local::now().format("%d/%m/%Y %H:%M").to_string();

    let mut block = format!("\n\n<!-- auto-aprendidos em {} -->\n", date);
    for (expr, reason) in suggestions {
        let safe_expr = expr.replace('"', "\\\"");
        block.push_str(&format!("- ~~\"{}\"~~ — {}\n", safe_expr, reason));
    }

    let existing = fs::read_to_string(&evitar_path).unwrap_or_default();
    let content = if existing.trim().is_empty() {
        format!("# Expressões a evitar\n\n> Adicione manualmente ou deixe o Artemis sugerir após você editar devolutivas antes de aprovar.\n{}", block)
    } else {
        let mut c = existing.trim_end().to_string();
        c.push_str(&block);
        c
    };

    fs::write(&evitar_path, content)?;
    Ok(evitar_path)
}

/// Lê o arquivo de exemplos de uma categoria específica. Retorna `None` se não existir.
/// Esta é a fonte de verdade da injeção de few-shot — o conteúdo bruto vai para o
/// system prompt, permitindo que o usuário inclua exemplos, notas ou instruções
/// específicas da categoria que serão lidas pela IA.
pub fn load_category_examples(vault_path: &Path, category: &str) -> Option<String> {
    let slug = slugify_category(category);
    let file_path = vault_path.join(format!("exemplos-{}.md", slug));
    fs::read_to_string(&file_path).ok()
}

/// Conta quantos blocos `## Aprovado em ...` existem no conteúdo. Útil para o chip de UI
/// indicar "X exemplos curados". Notas/instruções soltas adicionadas pelo usuário não
/// entram no count (mas estão visíveis para a IA).
pub fn count_example_blocks(content: &str) -> usize {
    content.matches("## Aprovado em").count()
}

/// Substitui o `estilo.md` no vault, fazendo backup do conteúdo anterior em
/// `estilo.md.bak`. O backup é SOBRESCRITO a cada chamada — guardamos apenas o
/// último estado (se o usuário quiser histórico completo, deve versionar o vault
/// com git). Não cria backup se o arquivo atual não existir.
///
/// Recusa silenciosamente conteúdo vazio (apenas whitespace) — chamador deve
/// validar antes. Este guard é defesa em profundidade contra IA retornando
/// resposta degenerada.
pub fn replace_estilo(vault_path: &Path, new_content: &str) -> anyhow::Result<PathBuf> {
    if new_content.trim().is_empty() {
        anyhow::bail!("conteúdo proposto está vazio — operação abortada");
    }

    let estilo_path = vault_path.join("estilo.md");
    let backup_path = vault_path.join("estilo.md.bak");

    if estilo_path.exists() {
        let current = fs::read_to_string(&estilo_path)?;
        fs::write(&backup_path, current)?;
    }

    fs::write(&estilo_path, new_content)?;
    Ok(estilo_path)
}

/// Aplica mudanças seletivas em `campos-padrao.md` (#20). Faz backup em
/// `campos-padrao.md.bak` (sobrescrito a cada chamada) antes de gravar.
///
/// - `new_release_line`: se Some, substitui a primeira linha que começar com
///   `- **Release atual:**` por uma nova linha apontando para essa release.
///   Se não houver tal linha no arquivo, ela é appendada ao fim numa seção
///   `## Versão atual em produção` criada automaticamente.
/// - `new_paths`: caminhos a appendar à seção `## Caminhos recorrentes`.
///   Se a seção não existir, é criada no fim do arquivo. Caminhos são appendados
///   sob um marker HTML `<!-- auto-aprendidos em DATA -->` (mesmo padrão de #18)
///   para que o usuário consiga distinguir o que veio da auto-curadoria.
pub fn apply_campos_changes(
    vault_path: &Path,
    new_release_line: Option<&str>,
    new_paths: &[String],
) -> anyhow::Result<PathBuf> {
    if new_release_line.is_none() && new_paths.is_empty() {
        anyhow::bail!("nenhuma mudança a aplicar");
    }

    let file_path = vault_path.join("campos-padrao.md");
    let backup_path = vault_path.join("campos-padrao.md.bak");

    let original = fs::read_to_string(&file_path).unwrap_or_default();

    if !original.is_empty() {
        fs::write(&backup_path, &original)?;
    }

    let mut content = original;

    if let Some(line) = new_release_line {
        content = replace_or_append_release_line(content, line);
    }

    if !new_paths.is_empty() {
        let date = chrono::Local::now().format("%d/%m/%Y %H:%M").to_string();
        content = append_paths_to_section(content, new_paths, &date);
    }

    fs::write(&file_path, content)?;
    Ok(file_path)
}

/// Lê o `campos-padrao.md` se existir, retorna string vazia caso contrário.
pub fn read_campos_padrao(vault_path: &Path) -> String {
    fs::read_to_string(vault_path.join("campos-padrao.md")).unwrap_or_default()
}

/// Extrai a linha de "Release atual" no formato `- **Release atual:** \`vX.Y.Z — dd/mm/aaaa\``
/// e devolve a parte interna entre crases. Retorna None se o padrão não estiver presente.
pub fn current_release_in_text(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("- **Release atual:**") {
            // Espera-se algo como ` \`v2.54.6 — 15/05/2026\``
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('`') {
                if let Some(end) = rest.find('`') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

/// Conjunto de caminhos já presentes no arquivo (qualquer linha que contenha `A > B`).
/// Usado para deduplicar sugestões antes de propor.
pub fn existing_paths_in_text(content: &str) -> std::collections::HashSet<String> {
    use crate::stats;
    stats::extract_paths(content)
        .into_iter()
        .map(|p| stats::normalize_path(&p))
        .collect()
}

fn replace_or_append_release_line(content: String, new_release: &str) -> String {
    let new_line = format!("- **Release atual:** `{}`", new_release);
    let mut replaced = false;
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count() + 1);
    for line in content.lines() {
        let trimmed = line.trim_start();
        if !replaced && trimmed.starts_with("- **Release atual:**") {
            // Preserva o indent do original
            let indent_len = line.len() - trimmed.len();
            out_lines.push(format!("{}{}", &line[..indent_len], new_line));
            replaced = true;
        } else {
            out_lines.push(line.to_string());
        }
    }
    let mut result = out_lines.join("\n");
    if !content.is_empty() && content.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    if !replaced {
        // Append no fim numa seção dedicada
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        if !result.ends_with("\n\n") && !result.is_empty() {
            result.push('\n');
        }
        result.push_str("## Versão atual em produção\n");
        result.push_str(&new_line);
        result.push('\n');
    }
    result
}

fn append_paths_to_section(content: String, new_paths: &[String], date: &str) -> String {
    let block_header = format!("<!-- auto-aprendidos em {} -->", date);
    let mut block = format!("\n{}\n", block_header);
    for p in new_paths {
        block.push_str(&format!("- `{}`\n", p));
    }

    // Procura uma seção "## Caminhos recorrentes" (case-sensitive, prefix match).
    let lines: Vec<&str> = content.lines().collect();
    let section_idx = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("## ") && t.to_lowercase().contains("caminhos recorrentes")
    });

    match section_idx {
        Some(start) => {
            // Encontra fim da seção: próxima linha que comece com "## " (depois do start)
            // ou EOF.
            let mut end = lines.len();
            for (i, l) in lines.iter().enumerate().skip(start + 1) {
                if l.trim_start().starts_with("## ") {
                    end = i;
                    break;
                }
            }
            // Reconstrói: lines[..end] + block + lines[end..]
            let mut out = String::with_capacity(content.len() + block.len());
            for (i, l) in lines.iter().enumerate() {
                out.push_str(l);
                out.push('\n');
                if i + 1 == end {
                    out.push_str(&block);
                }
            }
            // Se end == lines.len(), o for já passou por tudo e block ainda não foi appendado
            if end == lines.len() {
                out.push_str(&block);
            }
            out
        }
        None => {
            // Cria nova seção no fim
            let mut out = content.clone();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("## Caminhos recorrentes\n");
            out.push_str(&block);
            out
        }
    }
}

/// Appenda frases-modelo aprovadas pelo usuário em `campos-padrao.md`,
/// dentro da seção `## Frases-modelo aprovadas`. Cria a seção no fim do arquivo
/// se ela não existir. Faz backup em `campos-padrao.md.bak`.
///
/// Cada template entra como bloco `**<situation>:**\n> <template>\n` sob
/// um marker HTML `<!-- auto-aprendidos em DATA -->` (mesmo padrão de #18/#20).
pub fn append_phrase_templates(
    vault_path: &Path,
    templates: &[(String, String)], // (situation, template)
) -> anyhow::Result<PathBuf> {
    if templates.is_empty() {
        anyhow::bail!("nenhuma frase-modelo a aplicar");
    }

    let file_path = vault_path.join("campos-padrao.md");
    let backup_path = vault_path.join("campos-padrao.md.bak");
    let original = fs::read_to_string(&file_path).unwrap_or_default();

    if !original.is_empty() {
        fs::write(&backup_path, &original)?;
    }

    let date = chrono::Local::now().format("%d/%m/%Y %H:%M").to_string();
    let mut block = format!("\n<!-- auto-aprendidos em {} -->\n", date);
    for (situation, template) in templates {
        block.push_str(&format!("\n**{}:**\n> {}\n", situation.trim(), template.trim()));
    }

    let content = insert_into_phrases_section(&original, &block);

    fs::write(&file_path, content)?;
    Ok(file_path)
}

fn insert_into_phrases_section(content: &str, block: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let section_idx = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("## ") && t.to_lowercase().contains("frases-modelo")
    });

    match section_idx {
        Some(start) => {
            // Encontra fim da seção (próximo "## " ou EOF)
            let mut end = lines.len();
            for (i, l) in lines.iter().enumerate().skip(start + 1) {
                if l.trim_start().starts_with("## ") {
                    end = i;
                    break;
                }
            }
            let mut out = String::with_capacity(content.len() + block.len());
            for (i, l) in lines.iter().enumerate() {
                out.push_str(l);
                out.push('\n');
                if i + 1 == end {
                    out.push_str(block);
                }
            }
            if end == lines.len() {
                out.push_str(block);
            }
            out
        }
        None => {
            // Cria nova seção no fim
            let mut out = content.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("## Frases-modelo aprovadas\n");
            out.push_str(block);
            out
        }
    }
}

/// Imagem da cartilha — bytes brutos + extensão (sem ponto) + legenda.
/// A extensão é determinada no frontend pelo mime-type da imagem
/// (image/png → "png", image/jpeg → "jpg", etc.).
pub struct CartilhaImageInput<'a> {
    pub bytes: &'a [u8],
    pub extension: &'a str,
    pub caption: &'a str,
}

/// Cria a pasta `vault/cartilhas/YYYY-MM-DD-<slug>/` com:
/// - `imagens/01.<ext>`, `imagens/02.<ext>`, ... (preserva ordem do dev)
/// - `index.html` self-contained (CSS inline; sem dependências externas)
///
/// Retorna o caminho completo do `index.html` gerado.
pub fn save_cartilha(
    vault_path: &Path,
    title: &str,
    content: &str,
    release: Option<&str>,
    author: Option<&str>,
    images: &[CartilhaImageInput],
) -> anyhow::Result<PathBuf> {
    if title.trim().is_empty() {
        anyhow::bail!("título da cartilha está vazio");
    }
    if content.trim().is_empty() {
        anyhow::bail!("conteúdo da cartilha está vazio");
    }

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let slug = slug_from_title(title);
    let cartilha_dir = vault_path.join("cartilhas").join(format!("{}-{}", date, slug));
    let imagens_dir = cartilha_dir.join("imagens");
    fs::create_dir_all(&imagens_dir)?;

    // Salva cada imagem como NN.ext (zero-padded a 2 dígitos)
    let mut image_files: Vec<(String, String)> = Vec::with_capacity(images.len());
    for (i, img) in images.iter().enumerate() {
        let filename = format!("{:02}.{}", i + 1, sanitize_extension(img.extension));
        let path = imagens_dir.join(&filename);
        fs::write(&path, img.bytes)?;
        image_files.push((filename, img.caption.to_string()));
    }

    let date_display = chrono::Local::now().format("%d/%m/%Y").to_string();
    let html = render_cartilha_html(
        title,
        content,
        release.unwrap_or(""),
        author.unwrap_or(""),
        &date_display,
        &image_files,
    );

    let index_path = cartilha_dir.join("index.html");
    fs::write(&index_path, html)?;
    Ok(index_path)
}

/// Gera slug ASCII kebab-case a partir do título. Limita a 60 chars pra não estourar
/// limites de path no Windows (260 chars default).
fn slug_from_title(title: &str) -> String {
    let s = crate::deepseek::slugify_category(title);
    if s.len() > 60 {
        s[..60].trim_end_matches('-').to_string()
    } else {
        s
    }
}

/// Limita extensão a [a-z0-9]+ (rejeita lixo como `../../evil`). Default `png`.
fn sanitize_extension(ext: &str) -> String {
    let cleaned: String = ext
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .take(5)
        .collect();
    if cleaned.is_empty() {
        "png".to_string()
    } else {
        cleaned
    }
}

/// Escape mínimo de HTML pra evitar quebrar o template com `<`, `>`, `&`.
/// Usado em todos os campos de texto livre que vão pro HTML.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// CSS do template da cartilha — segue o padrão visual dos guias oficiais
/// (ex.: guia-usuario-conciliacao.html): sidebar azul fixa com índice, hero em
/// gradiente com chips de metadados, seções em cards brancos e screenshots com
/// barra de legenda. Self-contained: nenhuma dependência externa.
const CARTILHA_CSS: &str = r#"    :root {
      --azul: #072b73;
      --azul-2: #08388f;
      --azul-claro: #eef4ff;
      --texto: #1f2937;
      --muted: #64748b;
      --borda: #d8e1ec;
      --fundo: #f4f7fb;
      --card: #ffffff;
      --shadow: 0 16px 40px rgba(15, 23, 42, .10);
      --radius: 8px;
      --font: "Segoe UI", Arial, sans-serif;
    }

    * { box-sizing: border-box; }
    html { scroll-behavior: smooth; }
    body {
      margin: 0;
      background: var(--fundo);
      color: var(--texto);
      font-family: var(--font);
      font-size: 16px;
      line-height: 1.6;
    }

    a { color: inherit; text-decoration: none; }
    p { margin: 0 0 12px; }
    ul, ol { margin: 10px 0 12px; padding-left: 22px; }
    li { margin: 6px 0; }
    b { color: #111827; }

    .layout {
      display: grid;
      grid-template-columns: 286px minmax(0, 1fr);
      min-height: 100vh;
    }

    aside {
      position: sticky;
      top: 0;
      height: 100vh;
      overflow: auto;
      padding: 24px 18px;
      color: #fff;
      background: var(--azul);
      border-right: 1px solid rgba(255,255,255,.14);
    }

    .brand {
      display: flex;
      align-items: center;
      gap: 12px;
      padding-bottom: 18px;
      margin-bottom: 18px;
      border-bottom: 1px solid rgba(255,255,255,.18);
    }

    .brand .badge {
      display: grid;
      place-items: center;
      width: 48px;
      height: 48px;
      border-radius: 8px;
      background: #fff;
      color: var(--azul);
      font-size: 24px;
      font-weight: 700;
    }

    .brand strong { display: block; font-size: 16px; line-height: 1.2; }
    .brand span { display: block; margin-top: 4px; color: #bed3ff; font-size: 12px; }

    nav h2 {
      margin: 22px 0 8px;
      color: #a9c2ff;
      font-size: 11px;
      letter-spacing: .08em;
      text-transform: uppercase;
    }

    nav a {
      display: block;
      padding: 8px 10px;
      border-radius: 6px;
      color: #f4f7ff;
      font-size: 14px;
    }

    nav a:hover { background: rgba(255,255,255,.12); }

    main {
      width: 100%;
      max-width: 1220px;
      padding: 34px 44px 78px;
    }

    .hero {
      padding: 44px;
      border-radius: var(--radius);
      color: #fff;
      background:
        linear-gradient(110deg, rgba(7,43,115,.96), rgba(8,56,143,.88) 58%, rgba(23,107,135,.72)),
        linear-gradient(135deg, #072b73, #176b87);
      box-shadow: var(--shadow);
    }

    .hero h1 {
      max-width: 860px;
      margin: 0;
      font-size: clamp(32px, 4.5vw, 52px);
      line-height: 1.05;
    }

    .chips {
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin-top: 24px;
    }

    .chip {
      display: inline-flex;
      align-items: center;
      min-height: 31px;
      padding: 5px 11px;
      border: 1px solid rgba(255,255,255,.28);
      border-radius: 999px;
      background: rgba(255,255,255,.10);
      color: #fff;
      font-size: 13px;
    }

    section {
      margin-top: 34px;
      padding: 30px;
      border: 1px solid var(--borda);
      border-radius: var(--radius);
      background: var(--card);
      box-shadow: 0 4px 14px rgba(15, 23, 42, .04);
    }

    section h2 {
      margin: 0 0 12px;
      color: var(--azul);
      font-size: 28px;
      line-height: 1.22;
    }

    .screenshot {
      margin: 20px 0 8px;
      border: 1px solid #cbd7e6;
      border-radius: var(--radius);
      overflow: hidden;
      background: #fff;
      box-shadow: var(--shadow);
    }

    .screenshot img {
      display: block;
      width: 100%;
      height: auto;
    }

    .caption {
      margin: 0;
      padding: 10px 14px;
      border-top: 1px solid var(--borda);
      color: var(--muted);
      background: #f8fafc;
      font-size: 13px;
    }

    footer {
      padding: 24px 0 0;
      color: var(--muted);
      font-size: 13px;
    }

    @media (max-width: 980px) {
      .layout { grid-template-columns: 1fr; }
      aside { position: static; height: auto; }
      main { padding: 20px 14px 58px; }
      .hero { padding: 28px 22px; }
      section { padding: 22px 16px; }
    }

    @media print {
      aside { display: none; }
      .layout { display: block; }
      main { max-width: none; padding: 0; }
      .hero, section, .screenshot { box-shadow: none; break-inside: avoid; }
      body { background: #fff; }
    }"#;

/// Rótulo do índice lateral: título da seção sem o " :" decorativo do fim.
fn nav_label(heading: &str) -> String {
    heading.trim().trim_end_matches(':').trim().to_string()
}

/// Renderiza o HTML self-contained da cartilha no padrão dos guias oficiais:
/// sidebar com índice ancorado nas seções, hero com título + chips (release,
/// data, autor), cada seção `[s]...[/s]` vira um card `<section id=...>` e as
/// imagens entram numa seção final de screenshots com legenda. Imagens via
/// path relativo `imagens/NN.ext`. Funciona aberto no browser ou indexado
/// pelo Obsidian via plugin de HTML embed.
fn render_cartilha_html(
    title: &str,
    content: &str,
    release: &str,
    author: &str,
    date_display: &str,
    image_files: &[(String, String)],
) -> String {
    let sections = content_to_sections(content);

    let mut nav_links = String::new();
    let mut sections_html = String::new();
    for sec in &sections {
        match &sec.heading {
            Some(h) => {
                nav_links.push_str(&format!(
                    "        <a href=\"#{id}\">{label}</a>\n",
                    id = sec.id,
                    label = html_escape(&nav_label(h)),
                ));
                sections_html.push_str(&format!(
                    "      <section id=\"{id}\">\n        <h2>{h}</h2>\n{body}      </section>\n",
                    id = sec.id,
                    h = html_escape(h),
                    body = sec.body_html,
                ));
            }
            None => {
                sections_html.push_str(&format!(
                    "      <section>\n{body}      </section>\n",
                    body = sec.body_html,
                ));
            }
        }
    }

    let mut gallery = String::new();
    if !image_files.is_empty() {
        nav_links.push_str("        <a href=\"#imagens-referencia\">Imagens de referência</a>\n");
        gallery.push_str("      <section id=\"imagens-referencia\">\n        <h2>Imagens de referência</h2>\n");
        for (filename, caption) in image_files {
            gallery.push_str(&format!(
                "        <figure class=\"screenshot\">\n          <img src=\"imagens/{}\" alt=\"{}\" loading=\"lazy\">\n          <figcaption class=\"caption\">{}</figcaption>\n        </figure>\n",
                html_escape(filename),
                html_escape(caption),
                html_escape(caption)
            ));
        }
        gallery.push_str("      </section>\n");
    }

    let mut chips = String::new();
    if !release.trim().is_empty() {
        chips.push_str(&format!(
            "            <span class=\"chip\">Release {}</span>\n",
            html_escape(release.trim())
        ));
    }
    if !date_display.is_empty() {
        chips.push_str(&format!(
            "            <span class=\"chip\">Atualizado em {}</span>\n",
            html_escape(date_display)
        ));
    }
    if !author.trim().is_empty() {
        chips.push_str(&format!(
            "            <span class=\"chip\">{}</span>\n",
            html_escape(author.trim())
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="pt-BR">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title_esc}</title>
  <style>
{css}
  </style>
</head>
<body>
  <div class="layout">
    <aside>
      <div class="brand">
        <div class="badge">A</div>
        <div>
          <strong>Artemis</strong>
          <span>Cartilha didática</span>
        </div>
      </div>
      <nav aria-label="Índice da cartilha">
        <h2>Conteúdo</h2>
{nav}      </nav>
    </aside>
    <main>
      <header class="hero">
        <div>
          <h1>{title_esc}</h1>
          <div class="chips">
{chips}          </div>
        </div>
      </header>
{sections}{gallery}      <footer>
        <p>Cartilha gerada pelo Artemis em {date_esc}.</p>
      </footer>
    </main>
  </div>
</body>
</html>
"#,
        title_esc = html_escape(title),
        css = CARTILHA_CSS,
        nav = nav_links,
        chips = chips,
        sections = sections_html,
        gallery = gallery,
        date_esc = html_escape(date_display),
    )
}

/// Seção lógica da cartilha: uma tag `[s]Título :[/s]` + o texto até a próxima
/// tag. Texto antes da primeira tag vira uma seção sem heading (sem entrada no
/// índice lateral). O `id` alimenta a âncora `<section id>` + link da sidebar.
struct CartilhaSection {
    id: String,
    heading: Option<String>,
    body_html: String,
}

fn make_section(heading: Option<String>, body_html: String, index: usize) -> CartilhaSection {
    let id = heading
        .as_deref()
        .map(crate::deepseek::slugify_category)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("secao-{}", index + 1));
    CartilhaSection { id, heading, body_html }
}

/// Converte o texto bruto da IA em seções estruturadas:
/// - `[s]Título :[/s]` abre uma nova seção (mesmo padrão do `[n]` da devolutiva,
///   mas com `s` de "section" pra diferenciar)
/// - Quebras de linha duplas → novo parágrafo
/// - Parágrafo só de linhas `- item` → `<ul><li>`
/// - Quebras simples dentro de parágrafo → `<br>` (preserva passos numerados)
fn content_to_sections(content: &str) -> Vec<CartilhaSection> {
    let mut sections: Vec<CartilhaSection> = Vec::new();
    let mut heading: Option<String> = None;
    let mut body = String::new();

    for chunk in regex_split_sections(content) {
        match chunk {
            SectionChunk::Heading(h) => {
                if heading.is_some() || !body.trim().is_empty() {
                    let n = sections.len();
                    sections.push(make_section(heading.take(), std::mem::take(&mut body), n));
                }
                heading = Some(h.trim().to_string());
                body.clear();
            }
            SectionChunk::Text(t) => body.push_str(&text_to_paragraphs_html(t)),
        }
    }
    if heading.is_some() || !body.trim().is_empty() {
        let n = sections.len();
        sections.push(make_section(heading.take(), body, n));
    }

    // Garante ids únicos (headings repetidos ganham sufixo posicional)
    let mut seen = std::collections::HashSet::new();
    for (i, sec) in sections.iter_mut().enumerate() {
        if !seen.insert(sec.id.clone()) {
            sec.id = format!("{}-{}", sec.id, i + 1);
            seen.insert(sec.id.clone());
        }
    }
    sections
}

/// Corpo de uma seção: parágrafos separados por linha em branco; parágrafo
/// composto só de linhas `- item` vira lista `<ul>`.
fn text_to_paragraphs_html(text: &str) -> String {
    let mut out = String::new();
    for paragraph in text.split("\n\n") {
        let trimmed = paragraph.trim_matches('\n').trim();
        if trimmed.is_empty() {
            continue;
        }
        let lines: Vec<&str> = trimmed
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if !lines.is_empty() && lines.iter().all(|l| l.starts_with("- ")) {
            out.push_str("        <ul>\n");
            for l in &lines {
                out.push_str(&format!("          <li>{}</li>\n", html_escape(l[2..].trim())));
            }
            out.push_str("        </ul>\n");
        } else {
            let escaped = html_escape(trimmed).replace('\n', "<br>");
            out.push_str(&format!("        <p>{}</p>\n", escaped));
        }
    }
    out
}

/// Versão achatada (h2 + corpo, sem wrappers de card) usada pelos testes pra
/// validar o parsing sem depender do template completo.
#[cfg(test)]
fn content_to_html(content: &str) -> String {
    let mut out = String::new();
    for sec in content_to_sections(content) {
        if let Some(h) = &sec.heading {
            out.push_str(&format!("    <h2>{}</h2>\n", html_escape(h)));
        }
        out.push_str(&sec.body_html);
    }
    out
}

enum SectionChunk<'a> {
    Heading(&'a str),
    Text(&'a str),
}

/// Split simples baseado em [s]...[/s]. Não é regex de verdade — varredura linear.
fn regex_split_sections(s: &str) -> Vec<SectionChunk<'_>> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(start_rel) = s[cursor..].find("[s]") {
        let start = cursor + start_rel;
        if start > cursor {
            out.push(SectionChunk::Text(&s[cursor..start]));
        }
        let after_open = start + 3;
        if let Some(end_rel) = s[after_open..].find("[/s]") {
            let end = after_open + end_rel;
            out.push(SectionChunk::Heading(&s[after_open..end]));
            cursor = end + 4;
        } else {
            // [s] sem fechar — trata como texto cru pra não perder conteúdo
            out.push(SectionChunk::Text(&s[start..]));
            cursor = s.len();
            break;
        }
    }
    if cursor < s.len() {
        out.push(SectionChunk::Text(&s[cursor..]));
    }
    out
}

pub fn seed_vault(target_path: &Path) -> anyhow::Result<Vec<String>> {
    fs::create_dir_all(target_path)?;

    // Seed apenas os 3 arquivos de REGRAS. O 4o (exemplos-aprovados.md) era do
    // modelo antigo (aglutinado); agora os exemplos aprovados ficam em arquivos
    // por-categoria gerados sob demanda pelo append_to_category_examples.
    let templates: [(&str, &str); 3] = [
        ("estilo.md", include_str!("../../vault-template/estilo.md")),
        ("evitar.md", include_str!("../../vault-template/evitar.md")),
        (
            "campos-padrao.md",
            include_str!("../../vault-template/campos-padrao.md"),
        ),
    ];

    let mut created = Vec::new();
    for (name, content) in templates {
        let p = target_path.join(name);
        if !p.exists() {
            fs::write(&p, content)?;
            created.push(name.to_string());
        }
    }
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "artemis_vault_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn replace_estilo_creates_backup() {
        let dir = temp_vault();
        fs::write(dir.join("estilo.md"), "VERSAO ANTIGA").unwrap();

        let written = replace_estilo(&dir, "VERSAO NOVA").unwrap();

        assert_eq!(written, dir.join("estilo.md"));
        assert_eq!(fs::read_to_string(dir.join("estilo.md")).unwrap(), "VERSAO NOVA");
        assert_eq!(fs::read_to_string(dir.join("estilo.md.bak")).unwrap(), "VERSAO ANTIGA");
    }

    #[test]
    fn replace_estilo_overwrites_existing_bak() {
        let dir = temp_vault();
        fs::write(dir.join("estilo.md"), "v3").unwrap();
        fs::write(dir.join("estilo.md.bak"), "v1").unwrap();

        replace_estilo(&dir, "v4").unwrap();

        assert_eq!(fs::read_to_string(dir.join("estilo.md")).unwrap(), "v4");
        // bak contém v3 (o estado imediatamente anterior), v1 foi perdido — comportamento intencional.
        assert_eq!(fs::read_to_string(dir.join("estilo.md.bak")).unwrap(), "v3");
    }

    #[test]
    fn replace_estilo_works_when_no_existing_file() {
        let dir = temp_vault();

        replace_estilo(&dir, "primeira versao").unwrap();

        assert_eq!(fs::read_to_string(dir.join("estilo.md")).unwrap(), "primeira versao");
        assert!(!dir.join("estilo.md.bak").exists());
    }

    #[test]
    fn replace_estilo_rejects_empty_content() {
        let dir = temp_vault();
        fs::write(dir.join("estilo.md"), "conteudo original").unwrap();

        assert!(replace_estilo(&dir, "").is_err());
        assert!(replace_estilo(&dir, "   \n\t  \n").is_err());

        // Arquivo original deve permanecer intacto.
        assert_eq!(fs::read_to_string(dir.join("estilo.md")).unwrap(), "conteudo original");
        assert!(!dir.join("estilo.md.bak").exists());
    }

    #[test]
    fn current_release_in_text_basic() {
        let content = "# Header\n\n- **Release atual:** `v2.54.6 — 15/05/2026`\n\nresto";
        assert_eq!(
            current_release_in_text(content),
            Some("v2.54.6 — 15/05/2026".to_string())
        );
    }

    #[test]
    fn current_release_in_text_none_when_absent() {
        assert!(current_release_in_text("# só header\nnada de release").is_none());
        // Sem backticks também não casa
        assert!(current_release_in_text("- **Release atual:** v2.54.6 sem crases").is_none());
    }

    #[test]
    fn existing_paths_in_text_collects_normalized() {
        let content = "- `Guardian > Cadastros > Compra`\n- `Artemis > Apurações`\nlinha solta";
        let set = existing_paths_in_text(content);
        assert!(set.contains("Guardian > Cadastros > Compra"));
        assert!(set.contains("Artemis > Apurações"));
    }

    #[test]
    fn apply_campos_changes_replaces_release_line_preserving_rest() {
        let dir = temp_vault();
        let original = "# Header\n\n## Versão atual em produção\n- **Release atual:** `v2.54.6 — 15/05/2026`\n- **Próxima release prevista:** `v2.54.7 — 22/05/2026`\n\n## Outra seção\nconteúdo\n";
        fs::write(dir.join("campos-padrao.md"), original).unwrap();

        apply_campos_changes(&dir, Some("v2.55.0 — 23/05/2026"), &[]).unwrap();

        let new_content = fs::read_to_string(dir.join("campos-padrao.md")).unwrap();
        assert!(new_content.contains("- **Release atual:** `v2.55.0 — 23/05/2026`"));
        // Próxima release intocada
        assert!(new_content.contains("- **Próxima release prevista:** `v2.54.7 — 22/05/2026`"));
        // Outras seções intactas
        assert!(new_content.contains("## Outra seção"));
        // Backup criado
        assert_eq!(fs::read_to_string(dir.join("campos-padrao.md.bak")).unwrap(), original);
    }

    #[test]
    fn apply_campos_changes_appends_release_section_if_absent() {
        let dir = temp_vault();
        let original = "# Arquivo sem seção de release\n";
        fs::write(dir.join("campos-padrao.md"), original).unwrap();

        apply_campos_changes(&dir, Some("v1.0.0 — 01/01/2026"), &[]).unwrap();

        let new_content = fs::read_to_string(dir.join("campos-padrao.md")).unwrap();
        assert!(new_content.contains("## Versão atual em produção"));
        assert!(new_content.contains("- **Release atual:** `v1.0.0 — 01/01/2026`"));
    }

    #[test]
    fn apply_campos_changes_appends_paths_to_existing_section() {
        let dir = temp_vault();
        let original = "# Header\n\n## Caminhos recorrentes (exemplos)\n- `A > B > C`\n\n## Outra\nfim\n";
        fs::write(dir.join("campos-padrao.md"), original).unwrap();

        apply_campos_changes(
            &dir,
            None,
            &["Novo > Caminho".to_string(), "Outro > Novo".to_string()],
        )
        .unwrap();

        let new_content = fs::read_to_string(dir.join("campos-padrao.md")).unwrap();
        assert!(new_content.contains("- `A > B > C`"));
        assert!(new_content.contains("- `Novo > Caminho`"));
        assert!(new_content.contains("- `Outro > Novo`"));
        assert!(new_content.contains("<!-- auto-aprendidos em"));
        // Item appendado ANTES da próxima seção
        let idx_novo = new_content.find("Novo > Caminho").unwrap();
        let idx_outra_secao = new_content.find("## Outra").unwrap();
        assert!(idx_novo < idx_outra_secao);
    }

    #[test]
    fn apply_campos_changes_creates_paths_section_if_absent() {
        let dir = temp_vault();
        fs::write(dir.join("campos-padrao.md"), "# Vazio de caminhos\n").unwrap();

        apply_campos_changes(&dir, None, &["X > Y > Z".to_string()]).unwrap();

        let new_content = fs::read_to_string(dir.join("campos-padrao.md")).unwrap();
        assert!(new_content.contains("## Caminhos recorrentes"));
        assert!(new_content.contains("- `X > Y > Z`"));
    }

    #[test]
    fn apply_campos_changes_rejects_no_change_request() {
        let dir = temp_vault();
        fs::write(dir.join("campos-padrao.md"), "conteudo").unwrap();
        assert!(apply_campos_changes(&dir, None, &[]).is_err());
        // Arquivo intocado
        assert_eq!(fs::read_to_string(dir.join("campos-padrao.md")).unwrap(), "conteudo");
        assert!(!dir.join("campos-padrao.md.bak").exists());
    }

    #[test]
    fn append_phrase_templates_appends_to_existing_section() {
        let dir = temp_vault();
        let original = "# Header\n\n## Frases-modelo aprovadas\n\n**Existente:**\n> Template antigo\n\n## Outra seção\nfim\n";
        fs::write(dir.join("campos-padrao.md"), original).unwrap();

        let templates = vec![
            (
                "Para correção fiscal".to_string(),
                "A correção <X> foi liberada em <vY.Y.Z>.".to_string(),
            ),
        ];
        append_phrase_templates(&dir, &templates).unwrap();

        let new_content = fs::read_to_string(dir.join("campos-padrao.md")).unwrap();
        // Conteúdo antigo preservado
        assert!(new_content.contains("**Existente:**"));
        assert!(new_content.contains("> Template antigo"));
        assert!(new_content.contains("## Outra seção"));
        // Novo template aparece com marker
        assert!(new_content.contains("<!-- auto-aprendidos em"));
        assert!(new_content.contains("**Para correção fiscal:**"));
        assert!(new_content.contains("> A correção <X> foi liberada em <vY.Y.Z>."));
        // Novo template aparece ANTES da próxima seção
        let idx_novo = new_content.find("Para correção fiscal").unwrap();
        let idx_outra = new_content.find("## Outra seção").unwrap();
        assert!(idx_novo < idx_outra);
        // Backup criado
        assert_eq!(fs::read_to_string(dir.join("campos-padrao.md.bak")).unwrap(), original);
    }

    #[test]
    fn append_phrase_templates_creates_section_if_absent() {
        let dir = temp_vault();
        fs::write(dir.join("campos-padrao.md"), "# Apenas o título\n").unwrap();

        let templates = vec![
            ("Caso A".to_string(), "Frase A.".to_string()),
            ("Caso B".to_string(), "Frase B.".to_string()),
        ];
        append_phrase_templates(&dir, &templates).unwrap();

        let new_content = fs::read_to_string(dir.join("campos-padrao.md")).unwrap();
        assert!(new_content.contains("## Frases-modelo aprovadas"));
        assert!(new_content.contains("**Caso A:**"));
        assert!(new_content.contains("> Frase A."));
        assert!(new_content.contains("**Caso B:**"));
    }

    #[test]
    fn append_phrase_templates_rejects_empty_input() {
        let dir = temp_vault();
        fs::write(dir.join("campos-padrao.md"), "conteudo original").unwrap();
        assert!(append_phrase_templates(&dir, &[]).is_err());
        // Arquivo original intacto, sem backup
        assert_eq!(fs::read_to_string(dir.join("campos-padrao.md")).unwrap(), "conteudo original");
        assert!(!dir.join("campos-padrao.md.bak").exists());
    }

    #[test]
    fn save_cartilha_creates_directory_with_images_and_index_html() {
        let dir = temp_vault();
        let img_bytes = b"fake png bytes";
        let images = vec![
            CartilhaImageInput {
                bytes: img_bytes,
                extension: "png",
                caption: "Tela inicial do menu",
            },
            CartilhaImageInput {
                bytes: img_bytes,
                extension: "JPG", // será normalizado para lowercase
                caption: "Detalhe do novo botão",
            },
        ];
        let index = save_cartilha(
            &dir,
            "Nova permissão de cadastro de produto",
            "[s]Objetivo :[/s]\n\nConfigurar o acesso.\n\n[s]Passo a passo :[/s]\n\nAcesse o menu.",
            Some("v2.55.0 — 23/05/2026"),
            Some("Thiago"),
            &images,
        )
        .unwrap();

        assert!(index.exists());
        assert_eq!(index.file_name().unwrap(), "index.html");

        let html = fs::read_to_string(&index).unwrap();
        // Título no <h1> + no <title>
        assert!(html.contains("<h1>Nova permissão de cadastro de produto</h1>"));
        assert!(html.contains("<title>Nova permissão de cadastro de produto</title>"));
        // Meta line com release + data + autor
        assert!(html.contains("Release v2.55.0 — 23/05/2026"));
        assert!(html.contains("Thiago"));
        // Seções renderizadas como <h2>
        assert!(html.contains("<h2>Objetivo :</h2>"));
        assert!(html.contains("<h2>Passo a passo :</h2>"));
        // Galeria com paths relativos + ext normalizada
        assert!(html.contains("imagens/01.png"));
        assert!(html.contains("imagens/02.jpg"));
        assert!(html.contains("Tela inicial do menu"));
        assert!(html.contains("Detalhe do novo botão"));

        // Imagens salvas na pasta correta
        let cartilha_dir = index.parent().unwrap();
        assert!(cartilha_dir.join("imagens/01.png").exists());
        assert!(cartilha_dir.join("imagens/02.jpg").exists());
    }

    #[test]
    fn save_cartilha_rejects_empty_title_or_content() {
        let dir = temp_vault();
        let img = CartilhaImageInput { bytes: b"x", extension: "png", caption: "" };
        assert!(save_cartilha(&dir, "", "conteudo", None, None, &[img]).is_err());
        assert!(save_cartilha(&dir, "Titulo", "", None, None, &[]).is_err());
        assert!(save_cartilha(&dir, "Titulo", "  \n\t ", None, None, &[]).is_err());
    }

    #[test]
    fn save_cartilha_slug_handles_special_chars_and_truncates() {
        let dir = temp_vault();
        let huge_title = "Implementação de Cadastro de Produto com Permissões Específicas e Auditoria Completa do Sistema";
        let index = save_cartilha(
            &dir,
            huge_title,
            "[s]Resumo :[/s]\n\nteste",
            None,
            None,
            &[],
        )
        .unwrap();
        let folder_name = index.parent().unwrap().file_name().unwrap().to_str().unwrap();
        // Formato YYYY-MM-DD-slug; slug max 60 chars
        let parts: Vec<&str> = folder_name.splitn(2, '-').collect();
        let _date_part = parts[0]; // YYYY
        // O slug propriamente é tudo depois de "YYYY-MM-DD-"
        let after_date = &folder_name[11..]; // skip "YYYY-MM-DD-"
        assert!(after_date.len() <= 60, "slug {} excedeu 60 chars", after_date.len());
        assert!(folder_name.contains("implementacao"));
    }

    #[test]
    fn content_to_html_handles_sections_and_paragraphs() {
        let input = "[s]Objetivo :[/s]\n\nLinha 1.\nLinha 2.\n\nNovo parágrafo.";
        let html = content_to_html(input);
        assert!(html.contains("<h2>Objetivo :</h2>"));
        // Linha simples vira <br> dentro do mesmo <p>
        assert!(html.contains("<p>Linha 1.<br>Linha 2.</p>"));
        // Linha duplicada vira novo <p>
        assert!(html.contains("<p>Novo parágrafo.</p>"));
    }

    #[test]
    fn content_to_html_escapes_html_entities() {
        let input = "Texto com <script>alert('xss')</script> e &amp;";
        let html = content_to_html(input);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&amp;amp;")); // & original já era &amp; e é re-escapado
    }

    #[test]
    fn content_to_html_handles_unclosed_section_tag() {
        // [s] sem [/s] não deve fazer crash; vira texto cru escapado
        let input = "[s]Sem fim";
        let html = content_to_html(input);
        assert!(html.contains("[s]Sem fim") || html.contains("&#91;s&#93;Sem fim"));
    }
}
