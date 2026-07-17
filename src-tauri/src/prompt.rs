use crate::deepseek::ChatMessage;
use crate::vault::VaultContext;

const TEMPLATE_HEADER: &str = r#"Você é o assistente Artemis, especializado em redigir devolutivas técnicas para o time de suporte N1 de sistemas ERP. Sua saída é sempre em português brasileiro, em prosa técnica direta, e respeita rigorosamente o template abaixo. Omita campos que claramente não se aplicam ao caso descrito, mas nunca invente conteúdo para preencher campos.

═══ TEMPLATE OBRIGATÓRIO ═══

Formate cada campo com o título entre as tags [n] e [/n], seguido de uma linha em branco e o conteúdo. Deixe uma linha em branco entre os campos. Não use markdown (##, **, etc.).

Exemplo de formato:
[n]O que foi alterado/corrigido :[/n]

Descrição objetiva da correção realizada.

[n]Release/versão :[/n]

v1.0.0 — 22/05/2026

Campos disponíveis (inclua apenas os aplicáveis, nesta ordem):
1. O que foi alterado/corrigido
2. Caminho da solução (formato: Sistema > Menu > Submenu > Aba)
3. Novo parâmetro ou permissão (nome + caminho + uso correto)
4. Release/versão (formato: vX.Y.Z — dd/mm/aaaa)
5. Necessidade de atualização (versão / executável / nenhuma)
6. Forma de validação pelo suporte
7. Solução paliativa ou definitiva
8. Scripts ou ações manuais (SQL em bloco de código)
9. Documentação complementar
10. Pendências ou observações
11. Cenário não simulado (passo a passo dos testes realizados)"#;

/// Messages do modo REVISÃO: aplica as regras do vault (estilo/evitar/campos)
/// a uma devolutiva que o usuário escreveu à mão, sem reescrever do zero.
/// Não usa o TEMPLATE_HEADER da geração — o objetivo é preservar o texto,
/// não re-estruturá-lo no template.
pub fn build_revision_messages(vault: &VaultContext, user_text: &str) -> Vec<ChatMessage> {
    let system = format!(
        "Você REVISA devolutivas técnicas escritas manualmente por um dev para o time de suporte N1. \
        Português brasileiro, prosa técnica direta.\n\n\
        REGRAS DE REVISÃO:\n\
        - NÃO reescreva do zero: preserve a estrutura, a ordem e o conteúdo do texto original.\n\
        - Corrija apenas: expressões da lista de EVITAR, desvios do ESTILO do usuário, erros de gramática/pontuação e valores desatualizados conforme VALORES FREQUENTES (ex: release antiga).\n\
        - Mantenha as tags [n]...[/n] se o texto já as usar; NÃO as adicione se não existirem.\n\
        - NÃO adicione informações novas nem remova informações existentes.\n\
        - NÃO use markdown.\n\
        - Devolva APENAS o texto revisado, sem comentários sobre o que mudou e sem despedidas.\n\n\
        ═══ ESTILO DO USUÁRIO ═══\n\n{estilo}\n\n\
        ═══ EXPRESSÕES A EVITAR ═══\n\n{evitar}\n\n\
        ═══ VALORES FREQUENTES ═══\n\n{campos}",
        estilo = vault.estilo.trim(),
        evitar = vault.evitar.trim(),
        campos = vault.campos_padrao.trim(),
    );
    vec![
        ChatMessage {
            role: "system".to_string(),
            content: system,
        },
        ChatMessage {
            role: "user".to_string(),
            content: user_text.to_string(),
        },
    ]
}

pub struct PromptBuilder<'a> {
    vault: &'a VaultContext,
    category: Option<&'a str>,
    category_examples: Option<&'a str>,
}

impl<'a> PromptBuilder<'a> {
    pub fn new(vault: &'a VaultContext) -> Self {
        Self {
            vault,
            category: None,
            category_examples: None,
        }
    }

    pub fn with_category(mut self, category: &'a str, examples: Option<&'a str>) -> Self {
        self.category = Some(category);
        self.category_examples = examples;
        self
    }

    /// System prompt: template + 3 arquivos de regras + arquivo da categoria específica.
    /// O arquivo da categoria é injetado INTEIRO (não parseado em pares) para que
    /// qualquer instrução manual escrita pelo usuário no .md chegue à IA — exemplos
    /// formais, anotações livres, regras específicas da categoria, tudo.
    pub fn build_system_prompt(&self) -> String {
        let mut p = String::with_capacity(8192);
        p.push_str(TEMPLATE_HEADER);

        Self::append_section(&mut p, "ESTILO DO USUÁRIO", &self.vault.estilo);
        Self::append_section(&mut p, "EXPRESSÕES A EVITAR", &self.vault.evitar);
        Self::append_section(&mut p, "VALORES FREQUENTES", &self.vault.campos_padrao);

        if let (Some(cat), Some(examples)) = (self.category, self.category_examples) {
            let title = format!("EXEMPLOS E NOTAS DA CATEGORIA: {}", cat.to_uppercase());
            Self::append_section(&mut p, &title, examples);
        }

        p
    }

    pub fn build_messages(&self, user_input: &str) -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: "system".to_string(),
                content: self.build_system_prompt(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_input.to_string(),
            },
        ]
    }

    fn append_section(out: &mut String, title: &str, content: &str) {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return;
        }
        out.push_str("\n\n═══ ");
        out.push_str(title);
        out.push_str(" ═══\n\n");
        out.push_str(trimmed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_vault_no_category_examples() {
        let v = VaultContext::default();
        let s = PromptBuilder::new(&v).build_system_prompt();
        assert!(s.contains("TEMPLATE OBRIGATÓRIO"));
        assert!(!s.contains("ESTILO DO USUÁRIO"));
        assert!(!s.contains("EXEMPLOS E NOTAS"));
    }

    #[test]
    fn category_examples_appear_as_section() {
        let v = VaultContext {
            estilo: "Use voz ativa.".into(),
            ..Default::default()
        };
        let examples = "## Aprovado em 23/05/2026\n\n**Entrada:**\nteste\n\n**Saída:**\nresultado";
        let s = PromptBuilder::new(&v)
            .with_category("fiscal", Some(examples))
            .build_system_prompt();
        assert!(s.contains("EXEMPLOS E NOTAS DA CATEGORIA: FISCAL"));
        assert!(s.contains("Aprovado em 23/05/2026"));
        assert!(s.contains("resultado"));
    }

    #[test]
    fn category_without_examples_omits_section() {
        let v = VaultContext::default();
        let s = PromptBuilder::new(&v)
            .with_category("fiscal", None)
            .build_system_prompt();
        assert!(!s.contains("EXEMPLOS E NOTAS"));
    }

    #[test]
    fn build_messages_has_system_and_user_only() {
        let v = VaultContext::default();
        let msgs = PromptBuilder::new(&v).build_messages("input atual");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "input atual");
    }

    #[test]
    fn category_examples_include_manual_notes() {
        // Cenário: usuário editou o exemplos-fiscal.md adicionando uma nota livre
        // no topo, antes dos blocos auto-gerados. A nota deve chegar ao prompt.
        let v = VaultContext::default();
        let examples = "Nota manual: nesta categoria sempre use formato X.\n\n## Aprovado em 01/01/2026\n\nexemplo aqui";
        let s = PromptBuilder::new(&v)
            .with_category("fiscal", Some(examples))
            .build_system_prompt();
        assert!(s.contains("Nota manual: nesta categoria sempre use formato X"));
        assert!(s.contains("Aprovado em 01/01/2026"));
    }
}
