use futures_util::StreamExt;
use reqwest_eventsource::{Event, EventSource};
use serde::{Deserialize, Serialize};

const API_URL: &str = "https://api.deepseek.com/v1/chat/completions";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

pub async fn stream_chat<F>(
    api_key: &str,
    messages: Vec<ChatMessage>,
    mut on_token: F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) + Send,
{
    let body = ChatRequest {
        model: "deepseek-chat",
        messages: &messages,
        stream: true,
        temperature: 0.3,
        max_tokens: 2048,
    };

    let client = reqwest::Client::new();
    let req = client
        .post(API_URL)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&body);

    let mut es = EventSource::new(req)
        .map_err(|e| anyhow::anyhow!("Falha ao abrir SSE: {}", e))?;

    while let Some(event) = es.next().await {
        match event {
            Ok(Event::Open) => {
                tracing::debug!("SSE stream aberto");
            }
            Ok(Event::Message(msg)) => {
                if msg.data == "[DONE]" {
                    break;
                }
                match serde_json::from_str::<StreamChunk>(&msg.data) {
                    Ok(chunk) => {
                        if let Some(choice) = chunk.choices.first() {
                            if let Some(content) = &choice.delta.content {
                                on_token(content);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Falha ao parsear chunk SSE: {} (data: {})", e, msg.data);
                    }
                }
            }
            Err(reqwest_eventsource::Error::StreamEnded) => break,
            Err(e) => {
                es.close();
                return Err(anyhow::anyhow!("Erro SSE: {}", e));
            }
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// classify — categorização leve em 1 palavra, não-streaming.
// Usado tanto na aprovação quanto antes da geração para escolher few-shot.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ClassifyRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct ClassifyResponse {
    choices: Vec<ClassifyChoice>,
}

#[derive(Deserialize)]
struct ClassifyChoice {
    message: ClassifyMessage,
}

#[derive(Deserialize)]
struct ClassifyMessage {
    content: String,
}

/// Classifica um texto de devolutiva em UMA palavra de categoria.
/// `existing_categories` ancora o modelo nas categorias já presentes para evitar fragmentação.
/// Retorna a categoria normalizada (lowercase, ASCII, kebab-case).
pub async fn classify(
    api_key: &str,
    user_input: &str,
    existing_categories: &[String],
) -> anyhow::Result<String> {
    let existing = if existing_categories.is_empty() {
        "(nenhuma ainda — proponha uma curta e específica)".to_string()
    } else {
        existing_categories.join(", ")
    };

    let prompt = format!(
        "Você classifica devolutivas técnicas de software ERP em UMA palavra de categoria.\n\n\
        Categorias já existentes neste sistema: {}\n\n\
        Texto a classificar:\n{}\n\n\
        Regras:\n\
        - Responda APENAS com UMA palavra de categoria, em minúsculas, sem pontuação ou explicação.\n\
        - Se o texto se encaixa em uma categoria existente, use essa EXATAMENTE (preserve a grafia).\n\
        - Crie nova categoria apenas se for genuinamente uma área diferente das existentes.\n\
        - Nomes curtos e específicos do domínio ERP (ex: fiscal, vendas, financeiro, estoque, promocao, relatorios).",
        existing, user_input
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let body = ClassifyRequest {
        model: "deepseek-chat",
        messages: &messages,
        stream: false,
        temperature: 0.1,
        max_tokens: 10,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(API_URL)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<ClassifyResponse>()
        .await?;

    let raw = resp
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .unwrap_or("geral");

    let category = raw
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("geral")
        .to_lowercase();

    Ok(slugify_category(&category))
}

// ─────────────────────────────────────────────────────────────────────────────
// analyze_edits — analisa pares (ai_raw, final) para detectar padrões de
// evitação. Retorna JSON parseado em Vec<EvitarSuggestion>.
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct EvitarSuggestion {
    pub expression: String,
    pub reason: String,
    pub occurrences: u32,
}

/// Cada par é (ai_raw_output, final_output) de uma entry edited+approved.
pub async fn analyze_edits(
    api_key: &str,
    pairs: &[(String, String)],
) -> anyhow::Result<Vec<EvitarSuggestion>> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }

    let mut pairs_text = String::with_capacity(pairs.len() * 800);
    for (i, (ai_raw, final_out)) in pairs.iter().enumerate() {
        pairs_text.push_str(&format!(
            "\n=== PAR {} ===\n\nVERSÃO ORIGINAL (gerada pela IA):\n{}\n\nVERSÃO EDITADA E APROVADA PELO USUÁRIO:\n{}\n",
            i + 1,
            ai_raw.trim(),
            final_out.trim()
        ));
    }

    let prompt = format!(
        "Você analisa edições humanas em devolutivas técnicas de software ERP para identificar padrões de evitação de linguagem.\n\n\
        Para cada par abaixo há uma versão original gerada pela IA e a versão editada e aprovada pelo usuário. Identifique:\n\
        - Palavras/expressões que o usuário REMOVEU consistentemente (presentes no original, ausentes no editado).\n\
        - Substituições sistemáticas (palavra X virou palavra Y em múltiplos casos).\n\
        - Construções estilísticas que o usuário evita (gerúndios, voz passiva, jargão burocrático, anglicismos, etc.).\n\n\
        Regras de saída:\n\
        - Responda APENAS com um array JSON, sem texto antes ou depois, sem fences markdown.\n\
        - Cada item tem: {{\"expression\": <string>, \"reason\": <string explicando o padrão>, \"occurrences\": <número>}}.\n\
        - Inclua apenas padrões com 2+ ocorrências OU claramente intencionais.\n\
        - Se nenhum padrão claro emergir, retorne [].\n\
        - Máximo 8 sugestões — priorize as mais frequentes/impactantes.\n\
        - `expression` deve ser a forma EXATA que aparece no original (a que deve ser evitada).\n\
        - `reason` em português, breve (uma frase).\n\n\
        Pares para análise:\n{}",
        pairs_text
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let body = ClassifyRequest {
        model: "deepseek-chat",
        messages: &messages,
        stream: false,
        temperature: 0.2,
        max_tokens: 1024,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(API_URL)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<ClassifyResponse>()
        .await?;

    let raw = resp
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .unwrap_or("[]");

    // Extrai o primeiro array JSON do conteúdo. Modelos às vezes incluem texto antes/depois.
    let json_slice = extract_json_array(raw).unwrap_or("[]");

    match serde_json::from_str::<Vec<EvitarSuggestion>>(json_slice) {
        Ok(suggestions) => Ok(suggestions),
        Err(e) => {
            tracing::warn!("falha ao parsear sugestões JSON: {} (raw: {:?})", e, raw);
            Ok(Vec::new())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cartilha didática — prompt builder para gerar conteúdo de cartilha HTML
// a partir do mesmo input do FormView, com tom apropriado ao público alvo.
// ─────────────────────────────────────────────────────────────────────────────

/// Monta as messages para gerar uma cartilha didática. Reusa `stream_chat`
/// (streaming SSE) — o comando Tauri controla os eventos emitidos.
///
/// `audience` aceita "suporte", "cliente" ou "interno". Default razoável para
/// valores desconhecidos: "suporte".
///
/// `image_captions` é apenas informativa pra IA mencionar "ver imagem N" onde
/// apropriado; o renderer HTML coloca todas as imagens numa galeria no fim.
pub fn build_cartilha_messages(
    form_input: &str,
    audience: &str,
    image_captions: &[String],
) -> Vec<ChatMessage> {
    let audience_block = match audience {
        "cliente" => "USUÁRIOS FINAIS do cliente (não técnicos). Linguagem acessível, sem jargão de programador. Foco em 'como usar' não em 'como funciona internamente'.",
        "interno" => "EQUIPE INTERNA da empresa (dev/suporte/QA). Pode usar termos técnicos sem explicar. Foco em estrutura e detalhes operacionais.",
        _ => "TIME DE SUPORTE N1. Português técnico mas explicativo — eles conhecem o sistema mas não os detalhes da implementação. Explique parâmetros novos e onde encontrá-los.",
    };

    let images_hint = if image_captions.is_empty() {
        String::new()
    } else {
        let mut s = String::from("\n\nO dev anexou as seguintes imagens (na ordem):\n");
        for (i, cap) in image_captions.iter().enumerate() {
            s.push_str(&format!("- Imagem {}: {}\n", i + 1, cap));
        }
        s.push_str("\nCite cada imagem onde for relevante no texto (ex: \"conforme a imagem 1\"). Todas serão renderizadas numa galeria no fim do documento.");
        s
    };

    let prompt = format!(
        "Você gera uma CARTILHA DIDÁTICA em português brasileiro a partir de uma especificação técnica.\n\n\
        Público-alvo: {audience}\n\n\
        Estruture o texto em seções, cada seção precedida por uma TAG ESPECIAL `[s]Título :[/s]` (parser equivalente ao `[n]` da devolutiva, mas para 'section'). Por exemplo:\n\
        `[s]Objetivo :[/s]`\n\n\
        Seções típicas (use as que fizerem sentido para o caso; omita as que não se aplicam):\n\
        - Objetivo (por que existe)\n\
        - O que mudou (resumo do delta)\n\
        - Pré-requisitos (versão mínima, permissões necessárias)\n\
        - Passo a passo (instruções numeradas ou em prosa)\n\
        - Observações (cuidados, limitações, dicas){images_hint}\n\n\
        REGRAS:\n\
        - NÃO use markdown (`#`, `**`, `*`). Exceção: linhas iniciadas com `- ` viram bullets no HTML final — use-as para enumerações e checklists.\n\
        - NÃO use as tags `[n]...[/n]` (essas são da devolutiva N1, formato diferente).\n\
        - Frases curtas, voz ativa, tom direto mas amigável.\n\
        - Se o input mencionar nova permissão/parâmetro/caminho, dedique uma seção a explicar passo a passo onde encontrar.\n\
        - Termine SEM despedidas ('Espero ter ajudado', etc.). A cartilha é referência, não conversa.\n\n\
        ═══ ESPECIFICAÇÃO TÉCNICA ═══\n\n{form_input}",
        audience = audience_block,
        images_hint = images_hint,
        form_input = form_input.trim(),
    );

    vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }]
}

// ─────────────────────────────────────────────────────────────────────────────
// Form de testes — sugestão de cenários via IA, baseada no input do FormView
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, Default)]
pub struct TestScenariosSuggestion {
    pub happy_path: String,
    pub edge_cases: String,
    pub negative_cases: String,
    pub acceptance_criteria: String,
    pub regression_areas: String,
    pub risks: String,
}

/// Pede à IA pra sugerir cenários de teste baseados no contexto do dev.
/// Não-streaming, retorna o objeto preenchido. Usuário pode revisar/editar
/// antes de incluir na saída final.
pub async fn suggest_test_scenarios(
    api_key: &str,
    form_input: &str,
) -> anyhow::Result<TestScenariosSuggestion> {
    if form_input.trim().is_empty() {
        anyhow::bail!("input vazio — nada pra sugerir");
    }

    let prompt = format!(
        "Você ajuda um dev a montar o formulário de testes para entregar para a equipe QA. Baseado na descrição da mudança abaixo, sugira:\n\n\
        - `happy_path`: passo a passo do caminho feliz que deve funcionar.\n\
        - `edge_cases`: cenários-limite (valores extremos, vazios, máximos, mínimos).\n\
        - `negative_cases`: cenários que devem ser BLOQUEADOS/REJEITADOS pelo sistema.\n\
        - `acceptance_criteria`: condições objetivas para QA aprovar (ex: 'mensagem X aparece após Y').\n\
        - `regression_areas`: outras funcionalidades correlatas que merecem re-teste.\n\
        - `risks`: pontos de atenção, dependências, limitações conhecidas.\n\n\
        REGRAS:\n\
        - APENAS JSON puro, sem texto antes ou depois, sem fences markdown.\n\
        - Cada campo é uma STRING (use `\\n` para múltiplas linhas; pode usar `- ` para listas dentro da string).\n\
        - Se um campo não for aplicável ao caso, devolva string vazia.\n\
        - Português brasileiro, tom técnico direto, sem floreios.\n\n\
        Estrutura JSON esperada:\n\
        {{\"happy_path\":\"...\",\"edge_cases\":\"...\",\"negative_cases\":\"...\",\"acceptance_criteria\":\"...\",\"regression_areas\":\"...\",\"risks\":\"...\"}}\n\n\
        ═══ DESCRIÇÃO DA MUDANÇA ═══\n\n{form_input}",
        form_input = form_input.trim(),
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let body = ClassifyRequest {
        model: "deepseek-chat",
        messages: &messages,
        stream: false,
        temperature: 0.4,
        max_tokens: 2048,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(API_URL)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<ClassifyResponse>()
        .await?;

    let raw = resp
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .unwrap_or("{}");

    let json_slice = extract_json_object(raw).unwrap_or("{}");
    match serde_json::from_str::<TestScenariosSuggestion>(json_slice) {
        Ok(s) => Ok(s),
        Err(e) => {
            tracing::warn!("falha parseando sugestão de cenários: {} (raw: {:?})", e, raw);
            Ok(TestScenariosSuggestion::default())
        }
    }
}

/// Versão de `extract_json_array` para objetos `{...}`. Mesma estratégia: encontra
/// o primeiro `{` balanceado, respeitando strings com escape.
fn extract_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_string {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&s[start..=i]);
            }
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// extract_phrase_templates — analisa amostras de final_output das aprovadas
// para extrair frases recorrentes como templates parametrizados. (#21)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct PhraseTemplate {
    pub situation: String,
    pub template: String,
    pub occurrences: u32,
}

/// Recebe N final_outputs aprovados + o campos-padrao.md atual e retorna
/// até 8 sugestões de novas frases-modelo que NÃO estão no arquivo atual.
pub async fn extract_phrase_templates(
    api_key: &str,
    samples: &[String],
    current_campos_md: &str,
) -> anyhow::Result<Vec<PhraseTemplate>> {
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let mut samples_text = String::with_capacity(samples.len() * 600);
    for (i, s) in samples.iter().enumerate() {
        samples_text.push_str(&format!("\n=== DEVOLUTIVA {} ===\n{}\n", i + 1, s.trim()));
    }

    let current_block = if current_campos_md.trim().is_empty() {
        "(arquivo vazio — qualquer frase recorrente é candidato válido)".to_string()
    } else {
        current_campos_md.trim().to_string()
    };

    let prompt = format!(
        "Você analisa devolutivas técnicas aprovadas de um sistema ERP para identificar frases-modelo recorrentes que podem virar templates parametrizados no arquivo `campos-padrao.md`.\n\n\
        Você recebe:\n\
        1. {n} devolutivas que o usuário APROVOU (são o output final aceito).\n\
        2. O conteúdo atual do `campos-padrao.md` (incluindo a seção 'Frases-modelo aprovadas' que já existe).\n\n\
        Sua tarefa: identificar frases que aparecem em ≥ 3 devolutivas (literalmente ou com variações pequenas como datas, versões, caminhos) e propor um TEMPLATE parametrizado para cada.\n\n\
        REGRAS:\n\
        - NÃO proponha frases que já estejam na seção 'Frases-modelo aprovadas' do arquivo atual (mesma situação ou template equivalente). Verifique cuidadosamente antes de sugerir.\n\
        - Para cada template: use `<placeholder>` nas partes variáveis (ex: `<vX.Y.Z — dd/mm/aaaa>`, `<caminho>`, `<nome do arquivo>`).\n\
        - `situation` é um título curto descrevendo QUANDO usar a frase (ex: 'Para correção com troca de executável', 'Para validação de cenário fiscal específico').\n\
        - `template` é o texto da frase em prosa, com placeholders. Mantenha o estilo direto do usuário.\n\
        - `occurrences` é sua estimativa de quantas das devolutivas casam com o template.\n\
        - Máximo 8 sugestões — priorize as MAIS frequentes e MAIS úteis (frases longas, com mais informação, valem mais que frases curtas óbvias).\n\
        - Se nenhuma frase claramente recorrente emergir, retorne `[]`.\n\n\
        FORMATO DE SAÍDA:\n\
        - APENAS um array JSON, sem texto antes ou depois, sem fences markdown.\n\
        - Cada item: `{{\"situation\": \"<string>\", \"template\": \"<string com placeholders>\", \"occurrences\": <número>}}`.\n\n\
        ═══ CAMPOS-PADRAO.MD ATUAL ═══\n\n{current}\n\n\
        ═══ DEVOLUTIVAS APROVADAS ═══\n{samples}",
        n = samples.len(),
        current = current_block,
        samples = samples_text
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let body = ClassifyRequest {
        model: "deepseek-chat",
        messages: &messages,
        stream: false,
        temperature: 0.2,
        max_tokens: 2048,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(API_URL)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<ClassifyResponse>()
        .await?;

    let raw = resp
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .unwrap_or("[]");

    let json_slice = extract_json_array(raw).unwrap_or("[]");
    match serde_json::from_str::<Vec<PhraseTemplate>>(json_slice) {
        Ok(templates) => Ok(templates),
        Err(e) => {
            tracing::warn!("falha ao parsear templates JSON: {} (raw: {:?})", e, raw);
            Ok(Vec::new())
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// synthesize_style — recebe o estilo.md atual + amostras (raw_input, final_output)
// e pede à IA uma versão refinada do estilo.md, preservando regras existentes.
// Retorna markdown puro pronto para gravar no vault (após preview pelo usuário).
// ─────────────────────────────────────────────────────────────────────────────

pub async fn synthesize_style(
    api_key: &str,
    current_style: &str,
    samples: &[(String, String)],
) -> anyhow::Result<String> {
    if samples.is_empty() {
        anyhow::bail!("nenhuma amostra para sintetizar");
    }

    let mut samples_text = String::with_capacity(samples.len() * 1024);
    for (i, (raw, final_out)) in samples.iter().enumerate() {
        samples_text.push_str(&format!(
            "\n=== AMOSTRA {} ===\n\nENTRADA BRUTA:\n{}\n\nDEVOLUTIVA APROVADA (sem edições — a IA acertou de cara):\n{}\n",
            i + 1,
            raw.trim(),
            final_out.trim()
        ));
    }

    let current_block = if current_style.trim().is_empty() {
        "(arquivo ainda não existe — gere uma primeira versão respeitando a estrutura padrão indicada nas regras)".to_string()
    } else {
        current_style.trim().to_string()
    };

    let prompt = format!(
        "Você refina o arquivo `estilo.md` de um sistema que redige devolutivas técnicas para suporte N1 de ERP.\n\n\
        Você recebe (1) o `estilo.md` atual escrito/mantido pelo usuário e (2) {n} amostras de devolutivas aprovadas pelo usuário SEM edição (sinais positivos puros — a IA acertou).\n\n\
        Sua tarefa: produzir uma nova versão refinada do `estilo.md` que sirva como instrução de estilo para um LLM gerar devolutivas alinhadas com este usuário.\n\n\
        REGRAS RÍGIDAS:\n\
        - PRESERVE todas as regras já presentes no estilo.md atual. Não remova, não relativize, não substitua. O usuário escolheu cada uma deliberadamente.\n\
        - ADICIONE padrões novos que você observar nas amostras (ex: novo vocabulário recorrente, novas convenções de formatação, frases-padrão).\n\
        - REFINE descrições vagas com evidência específica das amostras (ex: se o estilo atual diz 'voz ativa' e nas amostras a forma sempre é 'Corrigi X' não 'O sistema corrigiu X', explicite).\n\
        - MANTENHA a estrutura de seções do estilo atual (Tom geral, Vocabulário preferido, Formatação, O que NÃO fazer, ou outras se já existirem). Use os MESMOS títulos `##`.\n\
        - Se o estilo atual estiver vazio, use esta estrutura padrão: Tom geral / Vocabulário preferido / Formatação / O que NÃO fazer.\n\
        - Itens em lista (`-`), prosa curta. Pt-br. Sem emojis. Sem meta-comentários do tipo 'baseado nas amostras...'.\n\n\
        FORMATO DE SAÍDA:\n\
        - APENAS o conteúdo do arquivo markdown final, começando com `# ` no título.\n\
        - SEM preâmbulo ('Aqui está...', 'Segue...'), SEM fences markdown (```), SEM explicações posteriores.\n\
        - O texto que você retornar substituirá o estilo.md atual; ele será revisado por humano antes de aplicar.\n\n\
        ═══ ESTILO.MD ATUAL ═══\n\n{current}\n\n\
        ═══ AMOSTRAS APROVADAS SEM EDIÇÃO ═══\n{samples}",
        n = samples.len(),
        current = current_block,
        samples = samples_text
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let body = ClassifyRequest {
        model: "deepseek-chat",
        messages: &messages,
        stream: false,
        temperature: 0.3,
        max_tokens: 4096,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(API_URL)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json::<ClassifyResponse>()
        .await?;

    let raw = resp
        .choices
        .first()
        .map(|c| c.message.content.as_str())
        .unwrap_or("");

    let cleaned = extract_markdown_doc(raw);
    if cleaned.trim().is_empty() {
        anyhow::bail!("IA retornou conteúdo vazio após limpeza");
    }
    Ok(cleaned)
}

/// Limpa respostas LLM que ocasionalmente vêm com preâmbulo ou embrulhadas em
/// fences markdown. Estratégia:
/// 1. Remove fence inicial `\`\`\`markdown\n` ou `\`\`\`md\n` ou `\`\`\`\n` se presente.
/// 2. Procura o primeiro `# ` no início de linha — descarta tudo antes (preâmbulo).
/// 3. Remove fence final `\`\`\`` se presente.
/// Se nenhum `# ` for encontrado, devolve o texto original (trim apenas).
fn extract_markdown_doc(raw: &str) -> String {
    let trimmed = raw.trim();

    // Strip fence inicial
    let after_open_fence = if let Some(rest) = trimmed.strip_prefix("```markdown\n") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("```md\n") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("```\n") {
        rest
    } else {
        trimmed
    };

    // Strip fence final
    let body = after_open_fence
        .strip_suffix("\n```")
        .or_else(|| after_open_fence.strip_suffix("```"))
        .unwrap_or(after_open_fence);

    // Localiza o primeiro título `# ` no início de linha; descarta preâmbulo antes.
    if let Some(pos) = find_first_h1(body) {
        body[pos..].trim().to_string()
    } else {
        body.trim().to_string()
    }
}

fn find_first_h1(s: &str) -> Option<usize> {
    if s.starts_with("# ") {
        return Some(0);
    }
    s.find("\n# ").map(|p| p + 1)
}

/// Localiza o primeiro array JSON `[...]` balanceado no texto. Heurística suficiente
/// para extrair JSON de respostas LLM que ocasionalmente vêm com texto extra.
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_string {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'[' {
            depth += 1;
        } else if b == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(&s[start..=i]);
            }
        }
    }
    None
}

/// Normaliza uma string em slug ASCII kebab-case.
/// "Fiscal" → "fiscal"; "Notas Fiscais" → "notas-fiscais"; "Promoção" → "promocao".
pub fn slugify_category(s: &str) -> String {
    let lowered = s.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut prev_dash = true; // evita iniciar com '-'
    for c in lowered.chars() {
        let mapped = match c {
            'a'..='z' | '0'..='9' => Some(c),
            'á' | 'à' | 'â' | 'ã' | 'ä' => Some('a'),
            'é' | 'è' | 'ê' | 'ë' => Some('e'),
            'í' | 'ì' | 'î' | 'ï' => Some('i'),
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => Some('o'),
            'ú' | 'ù' | 'û' | 'ü' => Some('u'),
            'ç' => Some('c'),
            'ñ' => Some('n'),
            _ => None,
        };
        match mapped {
            Some(ch) => {
                out.push(ch);
                prev_dash = false;
            }
            None => {
                if !prev_dash && !out.is_empty() {
                    out.push('-');
                    prev_dash = true;
                }
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "geral".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic() {
        assert_eq!(slugify_category("fiscal"), "fiscal");
        assert_eq!(slugify_category("Fiscal"), "fiscal");
        assert_eq!(slugify_category("Notas Fiscais"), "notas-fiscais");
        assert_eq!(slugify_category("Promoção"), "promocao");
        assert_eq!(slugify_category("financeiro/contábil"), "financeiro-contabil");
    }

    #[test]
    fn slug_edge_cases() {
        assert_eq!(slugify_category(""), "geral");
        assert_eq!(slugify_category("   "), "geral");
        assert_eq!(slugify_category("---"), "geral");
        assert_eq!(slugify_category("FISCAL!"), "fiscal");
    }

    #[test]
    fn extract_markdown_strips_preamble() {
        let raw = "Aqui está o estilo refinado:\n\n# Estilo de escrita\n\n- Item 1";
        assert_eq!(extract_markdown_doc(raw), "# Estilo de escrita\n\n- Item 1");
    }

    #[test]
    fn extract_markdown_strips_fences() {
        let raw = "```markdown\n# Estilo\n\nConteúdo\n```";
        assert_eq!(extract_markdown_doc(raw), "# Estilo\n\nConteúdo");

        let raw2 = "```md\n# X\n```";
        assert_eq!(extract_markdown_doc(raw2), "# X");

        let raw3 = "```\n# Y\nlinha\n```";
        assert_eq!(extract_markdown_doc(raw3), "# Y\nlinha");
    }

    #[test]
    fn extract_markdown_strips_fences_and_preamble() {
        let raw = "Segue:\n\n```markdown\n# Título\n\nCorpo\n```\n\nEspero ter ajudado.";
        // Fence final só é removido se vier no fim — esta variante tem texto depois do fence,
        // então removemos preâmbulo + fence inicial mas o "Espero ter ajudado" fica no body.
        // Aceitável: o output ainda começa com # e é markdown válido.
        let out = extract_markdown_doc(raw);
        assert!(out.starts_with("# Título"));
        assert!(out.contains("Corpo"));
    }

    #[test]
    fn extract_markdown_no_heading_returns_trimmed() {
        let raw = "  conteúdo livre sem heading  ";
        assert_eq!(extract_markdown_doc(raw), "conteúdo livre sem heading");
    }

    #[test]
    fn extract_markdown_starts_already_with_h1() {
        let raw = "# Já começa bem\n\nResto";
        assert_eq!(extract_markdown_doc(raw), "# Já começa bem\n\nResto");
    }

    #[test]
    fn extract_json_object_basic() {
        let raw = "Aqui está: {\"a\":1,\"b\":\"foo\"} fim";
        assert_eq!(extract_json_object(raw), Some("{\"a\":1,\"b\":\"foo\"}"));
    }

    #[test]
    fn extract_json_object_handles_nested_braces() {
        let raw = "{\"outer\":{\"inner\":{\"x\":1}}}";
        assert_eq!(extract_json_object(raw), Some(raw));
    }

    #[test]
    fn extract_json_object_handles_braces_in_strings() {
        let raw = "{\"text\":\"{nested} braces inside string\",\"n\":2}";
        assert_eq!(extract_json_object(raw), Some(raw));
    }

    #[test]
    fn extract_json_object_none_when_absent() {
        assert!(extract_json_object("nada de json aqui").is_none());
    }
}
