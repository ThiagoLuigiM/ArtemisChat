# Template de prompt — DevReturn

Documento vivo. Toda alteração no prompt enviado à DeepSeek deve passar por
aqui e pelos testes de snapshot em `src-tauri/src/prompt/builder.rs`.

## Estrutura em 3 camadas

A requisição enviada à API é montada em três blocos. Os dois primeiros são
**estáveis entre chamadas** e marcados com `cache_control: ephemeral` para
ativar o prompt caching da DeepSeek.

```
┌──────────────────────────────────────────┐
│  Bloco 1 — SYSTEM (cacheable)             │
│  Identidade + template fixo + vault       │
├──────────────────────────────────────────┤
│  Bloco 2 — FEW-SHOT (cacheable)           │
│  3 exemplos aprovados mais recentes       │
├──────────────────────────────────────────┤
│  Bloco 3 — USER (não cacheable)           │
│  Descrição bruta da devolutiva atual      │
└──────────────────────────────────────────┘
```

## Bloco 1 — System prompt

```
Você é o assistente DevReturn, especializado em redigir devolutivas técnicas
para o time de suporte N1 de sistemas ERP. Sua saída é sempre em português
brasileiro, em prosa técnica direta, e respeita rigorosamente o template
abaixo. Omita campos que claramente não se aplicam ao caso descrito, mas
nunca invente conteúdo para preencher campos.

═══ TEMPLATE OBRIGATÓRIO ═══

Para cada devolutiva, contemple os campos aplicáveis na ordem abaixo,
usando títulos `##` em markdown:

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
11. Cenário não simulado (passo a passo dos testes realizados)

═══ ESTILO DO USUÁRIO ═══

{{ESTILO_MD}}

═══ EXPRESSÕES A EVITAR ═══

{{EVITAR_MD}}

═══ VALORES FREQUENTES ═══

{{CAMPOS_PADRAO_MD}}

═══ EXEMPLOS CURADOS ═══

{{EXEMPLOS_APROVADOS_MD}}
```

## Bloco 2 — Few-shot via histórico

Para cada exemplo aprovado (até 3 mais recentes), enviar como par
`{role: "user", content: raw_input}` e `{role: "assistant", content: final_output}`.

A seleção é feita por:
```sql
SELECT raw_input, final_output FROM entries
WHERE approved = 1
ORDER BY created_at DESC
LIMIT 3;
```

Quando o histórico passar de ~30 entradas aprovadas, migrar para seleção por
similaridade (embedding local). Premature otimizar antes disso.

## Bloco 3 — User message

```
Descrição bruta da solução:
{{DESCRICAO_USUARIO}}

Metadados conhecidos (opcionais, preencha apenas se mencionados):
- Release alvo: {{RELEASE}}
- Sistema/módulo: {{SISTEMA}}
- Tipo: {{TIPO}}  // paliativo | definitivo

Gere a devolutiva final em markdown, usando os campos do template que se
aplicam ao caso. Se algum campo crítico estiver faltando informação, liste
ao final em "Pendências ou observações" o que ainda precisa ser confirmado.
```

## Variáveis

| Variável | Origem | Quando vazia |
|---|---|---|
| `{{ESTILO_MD}}` | `vault/devreturn/estilo.md` | Omitir seção inteira |
| `{{EVITAR_MD}}` | `vault/devreturn/evitar.md` | Omitir seção inteira |
| `{{CAMPOS_PADRAO_MD}}` | `vault/devreturn/campos-padrao.md` | Omitir seção inteira |
| `{{EXEMPLOS_APROVADOS_MD}}` | `vault/devreturn/exemplos-aprovados.md` | Omitir seção inteira |
| `{{DESCRICAO_USUARIO}}` | Input do chat | **Erro — campo obrigatório** |
| `{{RELEASE}}` | Parser regex sobre input ou metadado | Omitir linha |
| `{{SISTEMA}}` | Parser regex sobre input ou metadado | Omitir linha |
| `{{TIPO}}` | Detectado por heurística ou metadado | Omitir linha |

## Parâmetros da API DeepSeek

```json
{
  "model": "deepseek-chat",
  "temperature": 0.3,
  "max_tokens": 2048,
  "stream": true
}
```

- `temperature: 0.3` — baixa para favorecer consistência de estilo
- `stream: true` — UX de chat token-a-token
- `max_tokens: 2048` — suficiente para devolutivas longas; ajustar se truncar

## Snapshot test

O `prompt::builder` deve ter ao menos um teste de snapshot:

```rust
#[test]
fn snapshot_prompt_completo() {
    let ctx = StyleContext::from_fixtures("tests/fixtures/vault/");
    let req = PromptBuilder::new(ctx)
        .with_user_input("corrigi bug de ICMS na NF de devolução")
        .build();
    insta::assert_snapshot!(req.system);
}
```

Qualquer alteração no system prompt vai falhar o snapshot — revisão consciente
obrigatória antes de aceitar a nova versão.
