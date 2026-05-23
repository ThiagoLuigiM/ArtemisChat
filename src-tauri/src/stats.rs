// Parsing puro (sem IA, sem regex) sobre `raw_input` das devolutivas aprovadas
// para detectar releases (vX.Y.Z — dd/mm/aaaa) e caminhos (A > B > C).
// Usado pelo #20 para propor atualizações ao `campos-padrao.md`.

use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ReleaseSuggestion {
    /// Versão proposta no formato "vX.Y.Z" + data "dd/mm/aaaa" ("v2.55.0 — 23/05/2026").
    pub proposed: String,
    /// O que está hoje em `**Release atual:**` no campos-padrao.md (se conseguimos detectar).
    pub current_in_file: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PathSuggestion {
    pub path: String,
    pub occurrences: u32,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct CamposSuggestions {
    pub release: Option<ReleaseSuggestion>,
    pub paths: Vec<PathSuggestion>,
    /// Quantas entries (raw_inputs) foram analisadas — informativo pra UI.
    pub analyzed_count: usize,
}

/// Extrai a primeira ocorrência de "vX.Y.Z — dd/mm/aaaa" do texto.
/// Aceita "v" maiúsculo ou minúsculo. Aceita "—" (em-dash) ou "-" (hífen) como separador.
/// Retorna a string original encontrada (formato canônico com em-dash).
pub fn extract_release(text: &str) -> Option<(String, (u32, u32, u32), (u32, u32, u32))> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Procura por 'v' ou 'V' seguido de dígito
        if (bytes[i] == b'v' || bytes[i] == b'V') && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            // Tenta parsear vX.Y.Z começando em i+1
            if let Some((semver, after_semver)) = parse_semver(bytes, i + 1) {
                // Pula whitespace
                let after_ws1 = skip_whitespace(bytes, after_semver);
                // Aceita '—' (em-dash UTF-8: E2 80 94) ou '-' como separador
                let after_dash = if after_ws1 + 3 <= bytes.len()
                    && bytes[after_ws1] == 0xE2
                    && bytes[after_ws1 + 1] == 0x80
                    && bytes[after_ws1 + 2] == 0x94
                {
                    after_ws1 + 3
                } else if after_ws1 < bytes.len() && bytes[after_ws1] == b'-' {
                    after_ws1 + 1
                } else {
                    i += 1;
                    continue;
                };
                let after_ws2 = skip_whitespace(bytes, after_dash);
                if let Some((date, date_tuple, after_date)) = parse_date(bytes, after_ws2) {
                    // Canoniza com em-dash, indiferente do separador encontrado
                    let canonical = format!("v{}.{}.{} — {}", semver.0, semver.1, semver.2, date);
                    let _ = after_date;
                    return Some((canonical, semver, date_tuple));
                }
            }
        }
        i += 1;
    }
    None
}

/// Parseia X.Y.Z começando em `start` (depois do 'v'). Retorna ((maj,min,pat), índice após Z).
fn parse_semver(bytes: &[u8], start: usize) -> Option<((u32, u32, u32), usize)> {
    let (maj, after_maj) = parse_uint(bytes, start)?;
    if after_maj >= bytes.len() || bytes[after_maj] != b'.' {
        return None;
    }
    let (min, after_min) = parse_uint(bytes, after_maj + 1)?;
    if after_min >= bytes.len() || bytes[after_min] != b'.' {
        return None;
    }
    let (pat, after_pat) = parse_uint(bytes, after_min + 1)?;
    Some(((maj, min, pat), after_pat))
}

/// Parseia dd/mm/aaaa. Retorna (string "dd/mm/aaaa", (d,m,a), índice após).
fn parse_date(bytes: &[u8], start: usize) -> Option<(String, (u32, u32, u32), usize)> {
    let (d, after_d) = parse_uint(bytes, start)?;
    if !(1..=31).contains(&d) || after_d >= bytes.len() || bytes[after_d] != b'/' {
        return None;
    }
    let (m, after_m) = parse_uint(bytes, after_d + 1)?;
    if !(1..=12).contains(&m) || after_m >= bytes.len() || bytes[after_m] != b'/' {
        return None;
    }
    let (y, after_y) = parse_uint(bytes, after_m + 1)?;
    if !(2000..=2100).contains(&y) {
        return None;
    }
    Some((format!("{:02}/{:02}/{:04}", d, m, y), (d, m, y), after_y))
}

fn parse_uint(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut i = start;
    let mut n: u32 = 0;
    let mut any = false;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        n = n * 10 + (bytes[i] - b'0') as u32;
        any = true;
        i += 1;
        if i - start > 9 {
            return None; // overflow guard
        }
    }
    if any {
        Some((n, i))
    } else {
        None
    }
}

fn skip_whitespace(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Extrai todos os caminhos "A > B > C" do texto. Aceita 2 ou mais segmentos.
///
/// Estratégia:
/// 1. Quebra cada linha em "chunks" por delimitadores que não podem aparecer em
///    paths (`. , ; : ( ) ! ? \t`).
/// 2. Para cada chunk que contém `>`, split por `>`, trim cada parte.
/// 3. Filtra: ≥ 2 partes não-vazias; cada parte só com chars válidos.
/// 4. Para o PRIMEIRO segmento (que pode incluir prefixo tipo "o caminho é Guardian"),
///    descarta palavras iniciais que não começam com letra maiúscula — preserva apenas
///    a partir da primeira palavra capitalizada (heurística: nomes de sistema/módulo
///    no projeto seguem convenção PascalCase).
pub fn extract_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        scan_line_for_paths(line, &mut out);
    }
    out
}

fn scan_line_for_paths(line: &str, out: &mut Vec<String>) {
    for chunk in line.split(|c: char| matches!(c, '.' | ',' | ';' | ':' | '(' | ')' | '!' | '?' | '\t' | '`' | '"')) {
        if !chunk.contains('>') {
            continue;
        }
        let parts: Vec<&str> = chunk.split('>').collect();
        if parts.len() < 2 {
            continue;
        }
        let mut trimmed: Vec<String> = parts.iter().map(|p| p.trim().to_string()).collect();
        if trimmed.iter().any(|s| s.is_empty()) {
            continue;
        }
        // Validação de chars em CADA segmento (rejeita lixo tipo "1+2 > x").
        if !trimmed.iter().all(|s| s.chars().all(is_segment_char)) {
            continue;
        }
        // Cleanup do primeiro segmento: pega a partir da primeira palavra
        // começando com maiúscula (descarta "o caminho é" antes de "Guardian").
        let first_cleaned = trim_to_capitalized_start(&trimmed[0]);
        if first_cleaned.is_empty() {
            continue;
        }
        trimmed[0] = first_cleaned;
        // Cleanup simétrico no ÚLTIMO segmento: corta prosa após o nome
        // (ex: "Emissão hoje" → "Emissão"). Aplica só ao último porque
        // segmentos do meio são delimitados por `>` em ambos lados.
        let last_idx = trimmed.len() - 1;
        let last_cleaned = trim_to_capitalized_end(&trimmed[last_idx]);
        if last_cleaned.is_empty() {
            continue;
        }
        trimmed[last_idx] = last_cleaned;
        out.push(trimmed.join(" > "));
    }
}

/// Retorna a substring começando na primeira palavra que inicia com letra maiúscula.
/// Se nenhuma palavra começa com maiúscula, retorna string vazia.
/// Exemplos: "o caminho é Guardian" → "Guardian"; "Guardian Servidor" → "Guardian Servidor".
fn trim_to_capitalized_start(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if w.chars().next().map_or(false, |c| c.is_uppercase()) {
            return words[i..].join(" ");
        }
    }
    String::new()
}

/// Mantém palavras até a primeira que começa com lowercase APÓS já termos visto
/// uma palavra capitalizada. Trata prosa anexada depois do path.
/// Exemplos: "Emissão hoje" → "Emissão"; "Notas Fiscais" → "Notas Fiscais";
/// "Compra/Venda" → "Compra/Venda".
fn trim_to_capitalized_end(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut end = words.len();
    let mut seen_cap = false;
    for (i, w) in words.iter().enumerate() {
        let starts_upper = w.chars().next().map_or(false, |c| c.is_uppercase());
        if seen_cap && !starts_upper {
            end = i;
            break;
        }
        if starts_upper {
            seen_cap = true;
        }
    }
    words[..end].join(" ")
}

fn is_segment_char(c: char) -> bool {
    c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '/'
}

/// Agrega caminhos por frequência. Filtra apenas os com `min_occurrences` ou mais.
/// Ordena por (-ocorrências, path) — mais frequentes primeiro, desempate alfabético.
pub fn rank_paths(paths: &[String], min_occurrences: u32) -> Vec<PathSuggestion> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for p in paths {
        // Normalização: trim + colapsar whitespace múltiplo
        let normalized = normalize_path(p);
        if normalized.is_empty() {
            continue;
        }
        *counts.entry(normalized).or_insert(0) += 1;
    }
    let mut v: Vec<PathSuggestion> = counts
        .into_iter()
        .filter(|(_, c)| *c >= min_occurrences)
        .map(|(path, occurrences)| PathSuggestion { path, occurrences })
        .collect();
    v.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    v
}

pub fn normalize_path(s: &str) -> String {
    s.split('>')
        .map(|seg| seg.trim().to_string())
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join(" > ")
}

/// Escolhe a release "atual" — maior versão semver dentre as encontradas.
/// Em caso de empate de versão, preserva a data passada (não há motivo pra mudar).
pub fn pick_latest_release(releases: &[(String, (u32, u32, u32), (u32, u32, u32))]) -> Option<String> {
    releases
        .iter()
        .max_by(|a, b| a.1.cmp(&b.1))
        .map(|(s, _, _)| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_release_em_dash() {
        let (canon, semver, date) = extract_release("liberado na v2.54.6 — 15/05/2026 ok").unwrap();
        assert_eq!(canon, "v2.54.6 — 15/05/2026");
        assert_eq!(semver, (2, 54, 6));
        assert_eq!(date, (15, 5, 2026));
    }

    #[test]
    fn extract_release_hifen() {
        let (canon, _, _) = extract_release("Release/versão: v3.0.1 - 01/01/2027").unwrap();
        // Canoniza pra em-dash
        assert_eq!(canon, "v3.0.1 — 01/01/2027");
    }

    #[test]
    fn extract_release_uppercase_v() {
        let (canon, _, _) = extract_release("V10.5.99 — 31/12/2099").unwrap();
        assert_eq!(canon, "v10.5.99 — 31/12/2099");
    }

    #[test]
    fn extract_release_picks_first() {
        // Encontra primeira; pra "última semver" usar pick_latest_release sobre agregação
        let (canon, _, _) = extract_release("v1.0.0 — 01/01/2026 e depois v2.0.0 — 01/02/2026").unwrap();
        assert_eq!(canon, "v1.0.0 — 01/01/2026");
    }

    #[test]
    fn extract_release_none() {
        assert!(extract_release("não tem versão aqui").is_none());
        assert!(extract_release("v2.54 — 15/05/2026").is_none()); // sem patch
        assert!(extract_release("v2.54.6 sem data").is_none());
        assert!(extract_release("v2.54.6 — 32/05/2026").is_none()); // dia inválido
        assert!(extract_release("v2.54.6 — 15/13/2026").is_none()); // mês inválido
    }

    #[test]
    fn extract_paths_simple() {
        let paths = extract_paths("o caminho é Guardian > Notas Fiscais > Emissão.");
        assert_eq!(paths, vec!["Guardian > Notas Fiscais > Emissão"]);
    }

    #[test]
    fn extract_paths_multiple_lines() {
        let text = "Caminho 1: Guardian > Cadastros > Compra/Venda\n\
                    Caminho 2: Artemis > Apurações > ICMS > Devolução";
        let paths = extract_paths(text);
        assert!(paths.iter().any(|p| p == "Guardian > Cadastros > Compra/Venda"));
        assert!(paths.iter().any(|p| p == "Artemis > Apurações > ICMS > Devolução"));
    }

    #[test]
    fn extract_paths_strips_prose_prefix() {
        // "o caminho é Guardian" → deve começar em "Guardian" (única palavra capitalizada)
        let paths = extract_paths("verifique o caminho é Guardian > Notas > Emissão hoje");
        assert!(paths.iter().any(|p| p == "Guardian > Notas > Emissão"));
        // Não deve incluir "verifique o caminho é Guardian > ..."
        assert!(paths.iter().all(|p| !p.starts_with("verifique")));
        assert!(paths.iter().all(|p| !p.starts_with("o caminho")));
    }

    #[test]
    fn extract_paths_handles_backticks_and_quotes() {
        let paths = extract_paths("o caminho é `Guardian > Notas > Emissão` e fim");
        assert!(paths.iter().any(|p| p == "Guardian > Notas > Emissão"));
    }

    #[test]
    fn extract_paths_skips_lowercase_only_chunks() {
        // Sem nenhuma palavra capitalizada, descarta (sinal fraco / lixo)
        let paths = extract_paths("foo > bar > baz");
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_paths_two_segments_minimum() {
        let paths = extract_paths("apenas Guardian sozinho");
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_paths_handles_extra_whitespace() {
        // Múltiplos espaços entre `>` colapsam para um; prosa lowercase no final é cortada.
        let paths = extract_paths("A   >   B   >  C extra");
        assert!(paths.iter().any(|p| p == "A > B > C"));
    }

    #[test]
    fn rank_paths_filters_below_threshold() {
        let paths = vec![
            "A > B".to_string(),
            "A > B".to_string(),
            "A > B".to_string(),
            "C > D".to_string(), // só 1, filtrado se min=2
        ];
        let ranked = rank_paths(&paths, 2);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].path, "A > B");
        assert_eq!(ranked[0].occurrences, 3);
    }

    #[test]
    fn rank_paths_orders_by_count_desc_then_alpha() {
        let paths = vec![
            "X > Y".to_string(),
            "X > Y".to_string(),
            "A > B".to_string(),
            "A > B".to_string(),
            "Z > W".to_string(),
            "Z > W".to_string(),
        ];
        let ranked = rank_paths(&paths, 2);
        // Todos têm 2 ocorrências → ordem alfabética
        assert_eq!(ranked[0].path, "A > B");
        assert_eq!(ranked[1].path, "X > Y");
        assert_eq!(ranked[2].path, "Z > W");
    }

    #[test]
    fn rank_paths_normalizes_whitespace_for_dedup() {
        let paths = vec![
            "A > B > C".to_string(),
            "A  >  B  >  C".to_string(),
            "  A > B > C  ".to_string(),
        ];
        let ranked = rank_paths(&paths, 1);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].occurrences, 3);
    }

    #[test]
    fn pick_latest_release_semver_order() {
        let releases = vec![
            ("v2.54.6 — 15/05/2026".to_string(), (2, 54, 6), (15, 5, 2026)),
            ("v2.55.0 — 23/05/2026".to_string(), (2, 55, 0), (23, 5, 2026)),
            ("v2.54.10 — 20/05/2026".to_string(), (2, 54, 10), (20, 5, 2026)),
        ];
        assert_eq!(pick_latest_release(&releases).unwrap(), "v2.55.0 — 23/05/2026");
    }

    #[test]
    fn pick_latest_release_empty_returns_none() {
        assert!(pick_latest_release(&[]).is_none());
    }
}
