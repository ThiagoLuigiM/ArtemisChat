# Artemis — Handoff técnico

> Documento de transição entre sessões/LLMs. **Leia inteiro antes de retomar o desenvolvimento.**

---

## ⚠️ REGRA OBRIGATÓRIA — APLICÁVEL A TODO AGENTE (humano ou LLM) QUE OPERAR NESTE CÓDIGO

**Ao final de CADA ciclo de alterações** (qualquer modificação que toque código, configuração, schema ou arquitetura), você **DEVE** atualizar este `HANDOFF.md` refletindo:

1. **Novos arquivos / módulos** adicionados, removidos ou renomeados
2. **Novas dependências** em `Cargo.toml` ou `package.json`
3. **Novos comandos Tauri** expostos ou removidos do `invoke_handler!`
4. **Novos eventos** emitidos do backend para o frontend
5. **Mudanças de arquitetura ou fluxo de dados** (especialmente **reversões** de decisões anteriores — documentar o porquê)
6. **Gotchas** novos descobertos durante a sessão
7. **Atualização** das seções "Estado das fases", "Próximo passo imediato" e "Última atividade"

**Critério:** considere uma tarefa **incompleta** enquanto o HANDOFF não estiver atualizado. Esta regra existe porque o projeto é colaborativo entre múltiplas sessões/agentes (Claude Opus, Sonnet, e outros LLMs) que precisam onboardar rapidamente sem ler o histórico todo.

**Como atualizar:** edite as seções relevantes. Não delete contexto histórico relevante (ex: rebrand DevReturn→Artemis, pivots arquiteturais) — mantém como rastro para entender o porquê das decisões. Se uma decisão foi revertida, explique a reversão.

---

> **Rebrand 2026-05-22:** Projeto renomeado de DevReturn para **Artemis**. Todos os arquivos, caminhos e identificadores foram atualizados. **A pasta raiz do projeto continua sendo `DevReturn/`** (só os identificadores internos viraram Artemis).

## O que é o projeto

App desktop **floating (FAB)** para o Thiago redigir devolutivas técnicas padronizadas para o time de suporte N1. Usa **API DeepSeek** com **streaming SSE** e **arquivos `.md` do Obsidian** como base de conhecimento adaptativa (estilo, expressões a evitar, valores recorrentes, **exemplos por categoria auto-curados**). A meta é que, com o uso, as edições manuais nas devolutivas geradas tendam a zero — mas o usuário sempre pode editar os `.md` do vault para influenciar diretamente o comportamento da IA.

## Stack escolhida (decisão definitiva)

| Camada | Escolha | Notas |
|---|---|---|
| Shell desktop | **Tauri 2.11** | ~43 MB RAM residente (vs ~300 MB Electron). Decisão firme do usuário: nada de Electron. |
| Frontend | **React 19 + Vite 7 + TypeScript 5.8** | Sem framework UI extra (CSS puro) |
| Backend | **Rust 1.95 (stable-msvc) + tokio + reqwest + reqwest-eventsource** | rustls-tls (sem OpenSSL) |
| File watcher | **notify 6 + notify-debouncer-mini 0.4** | Debounce 300ms |
| Histórico | **rusqlite 0.32 bundled + FTS5** | Sem dependência do SQLite do sistema |
| Datas | **chrono 0.4** (only `clock` feature) | Para timestamps formatados em pt-BR |
| Storage de config | **config.json em `%APPDATA%/Artemis/`** | Vide gotcha keyring abaixo |
| Folder picker | **tauri-plugin-dialog 2** | |

## Pré-requisitos no host (já instalados)

- Rust 1.95.0 (toolchain `stable-x86_64-pc-windows-msvc`)
- Visual Studio 2022 Build Tools (workload C++ + Windows 11 SDK 22621)
- Node 24 + npm 11
- WebView2 (vem com Windows 11)

## Como rodar

```powershell
cd E:\Projetos-Thiago\Space\WorkSpaceArtemis\DevReturn
npm run tauri dev
```

Vite sobe em `localhost:1420`, o Tauri compila o binário Rust e abre 2 janelas: FAB (72x72, canto inferior direito, always-on-top, transparente, **arrastável com mouseMove >4px**) + Chat (460x640, oculta até clicar no FAB).

**Hot reload:**
- Frontend (React/TS/CSS): Vite HMR aplica instantâneo
- Backend (Rust): Tauri watcher rebuilda automaticamente em ~15-30s

## Estrutura do projeto

```
DevReturn/                            ← pasta raiz (nome legado, não renomeado)
├── resources/
│   └── foguete.png                   # Ícone-fonte do app
├── public/
│   └── foguete.png                   # Cópia servida pelo Vite (favicon + FAB)
├── src/                              # Frontend React
│   ├── main.tsx                      # Roteador FAB/Chat via ?window=
│   ├── styles.css                    # CSS único; .form-*, .result-*, .devolutiva-header,
│   │                                 # .category-chip, .btn-danger
│   └── windows/
│       ├── FabWindow.tsx             # <img foguete.png>; drag via mouseMove >4px
│       └── ChatWindow.tsx            # FormView + ResultView (editável) + SettingsPanel
├── src-tauri/                        # Backend Rust
│   ├── Cargo.toml                    # name=artemis, lib name=artemis_lib
│   ├── tauri.conf.json               # productName=Artemis, identifier=com.artemis.app
│   ├── capabilities/default.json
│   ├── icons/                        # Gerados via `npx tauri icon resources/foguete.png`
│   └── src/
│       ├── main.rs                   # chama artemis_lib::run()
│       ├── lib.rs                    # tauri::Builder, setup, plugins, state managed
│       ├── commands.rs               # #[tauri::command] expostos ao frontend
│       ├── deepseek.rs               # Cliente SSE streaming + classify (1 palavra) + slugify
│       │                             # + analyze_edits (#18) + synthesize_style (#19)
│       ├── prompt.rs                 # PromptBuilder; output usa tags [n]...[/n]
│       ├── settings.rs               # Config JSON em %APPDATA%/Artemis/
│       ├── stats.rs                  # Parsing puro (#20): extract_release, extract_paths,
│       │                             # rank_paths, pick_latest_release. SEM regex/IA.
│       ├── vault.rs                  # VaultLoader (3 arquivos de regra) + auto-curadoria
│       │                             # per-categoria + leitura sob demanda + replace_estilo (#19)
│       │                             # + apply_campos_changes (#20)
│       └── history.rs                # SQLite (bundled) + FTS5; só histórico/search
│                                     # (NÃO é mais fonte de injeção — vide pivot 2026-05-23)
├── vault-template/
│   ├── estilo.md                     # Seed: tom de escrita
│   ├── evitar.md                     # Seed: expressões/gerúndios a evitar
│   └── campos-padrao.md              # Seed: versões/caminhos/frases recorrentes
│                                     # (exemplos-aprovados.md REMOVIDO do seed em 2026-05-23 —
│                                     #  agora exemplos são per-categoria, criados sob demanda
│                                     #  pela auto-curadoria)
└── docs/
    └── prompt-template.md            # Spec do prompt
```

**Arquivos gerados em runtime no vault do usuário:**
- `exemplos-{categoria}.md` — auto-curados pela aprovação. Categorias são descobertas pela IA (sem lista fixa). Exemplos: `exemplos-fiscal.md`, `exemplos-vendas.md`, `exemplos-promocao.md`.

**Arquivos em `%APPDATA%/Artemis/`:**
- `config.json` — `{ vault_path, api_key }` (plaintext — vide gotcha keyring)
- `history.db` — SQLite com a tabela `entries` + FTS5

## Comandos Tauri expostos

| Comando | Args | Retorno | Função |
|---|---|---|---|
| `open_chat` | — | — | Mostra a janela chat |
| `close_chat` | — | — | Esconde a janela chat |
| `get_api_key` | — | `Option<String>` | Lê de config.json |
| `set_api_key` | `key: String` | — | Grava em config.json + roundtrip verify |
| `get_vault_path` | — | `Option<String>` | |
| `set_vault_path` | `path: String` | `VaultStatus` | Salva, recarrega vault, reinicia watcher |
| `get_vault_status` | — | `VaultStatus` | |
| `seed_vault` | `path: String` | `Vec<String>` | Copia os 3 .md de regra de `vault-template/` |
| `stream_completion` | `user_input: String` | — | Classify → carrega exemplos da categoria → stream SSE |
| `approve_entry` | `raw_input, ai_raw_output, final_output` | `ApprovalResult { id, category, examples_file }` | Classifica → salva SQLite → appenda em `exemplos-{cat}.md` |
| `discard_entry` | `raw_input, ai_raw_output, final_output` | `i64` | Classifica → salva SQLite com approved=false |
| `list_history` | `limit?, approved_only?` | `Vec<Entry>` | |
| `search_history` | `query, limit?` | `Vec<Entry>` | FTS5 |
| `delete_history_entry` | `id: i64` | — | |
| `history_count` | `approved_only?` | `usize` | |
| `list_categories` | — | `Vec<String>` | Distinct categorias já vistas |
| `count_edited_approved` | — | `usize` | Quantas entries com `approved=1 AND edited=1` (para UI decidir se vale a pena analisar) |
| `analyze_edits` | — | `Vec<EvitarSuggestion>` | Chama DeepSeek com até 20 pares editados; retorna sugestões `{ expression, reason, occurrences }`. NÃO escreve em lugar nenhum |
| `apply_evitar_suggestions` | `suggestions: Vec<EvitarSuggestion>` | `String` (nome do arquivo escrito) | Appenda sugestões aceitas em `evitar.md` com marker `<!-- auto-aprendidos em DATA -->` |
| `count_approved_unedited` | — | `usize` | Quantas entries com `approved=1 AND edited=0` (acertos puros; UI desabilita botão de síntese se < 5) |
| `synthesize_style` | — | `String` (markdown da proposta) | Lê estilo.md atual + até 50 aprovadas-sem-edição; chama DeepSeek; retorna proposta refinada. NÃO escreve em lugar nenhum |
| `apply_style_synthesis` | `new_content: String` | `String` (nome do arquivo escrito) | Faz backup em `estilo.md.bak` (sobrescrito a cada vez) + substitui `estilo.md` + recarrega vault + emite `vault-changed` |
| `analyze_campos` | — | `CamposSuggestions` | Lê até 100 aprovadas; extrai release semver mais recente + caminhos `A > B > C` por frequência (parsing puro, sem IA). Filtra os já presentes no campos-padrao.md. NÃO escreve em lugar nenhum |
| `apply_campos_suggestions` | `release_accepted: Option<String>, paths_accepted: Vec<String>` | `String` (nome do arquivo escrito) | Faz backup em `campos-padrao.md.bak`; substitui linha "Release atual" via regex/parser; appenda caminhos na seção "Caminhos recorrentes" com marker `<!-- auto-aprendidos em DATA -->`; recarrega vault |
| `analyze_phrase_templates` | — | `Vec<PhraseTemplate>` | Lê até 80 aprovadas + campos-padrao atual; chama DeepSeek com prompt que pede JSON `[{situation, template, occurrences}]` evitando duplicar frases já presentes. NÃO escreve em lugar nenhum |
| `apply_phrase_templates` | `templates: Vec<PhraseTemplate>` | `String` (nome do arquivo escrito) | Faz backup em `campos-padrao.md.bak`; appenda templates aceitos na seção "Frases-modelo aprovadas" (formato `**<situation>:**\n> <template>`) sob marker; recarrega vault |
| `stream_cartilha` | `form_input, audience, image_captions` | — | Stream da geração de cartilha didática via DeepSeek; emite events `cartilha-token` / `cartilha-done` (separados dos da devolutiva pra UI não confundir) |
| `save_cartilha` | `title, content, release?, author?, images: Vec<CartilhaImageDto>` | `String` (path do `index.html`) | Cria `vault/cartilhas/YYYY-MM-DD-<slug>/index.html` + `imagens/NN.ext`. HTML self-contained (CSS inline). Imagens chegam como `Vec<u8>` direto, sem precisar base64 |
| `open_in_system` | `path: String` | — | Abre arquivo/pasta no app padrão do sistema (Windows: `cmd /c start`). Usado pra abrir cartilha gerada no navegador |
| `suggest_test_scenarios` | `form_input: String` | `TestScenariosSuggestion { happy_path, edge_cases, negative_cases, acceptance_criteria, regression_areas, risks }` | Pede à IA pra preencher cenários do form de testes a partir do contexto do FormView. JSON puro, parser via `extract_json_object`. Frontend pré-preenche os textareas |
| `get_autostart_enabled` | — | `bool` | Lê do registro do Windows se o app está configurado pra iniciar com o sistema |
| `set_autostart_enabled` | `enabled: bool` | — | Liga/desliga autostart via `tauri-plugin-autostart` |
| `check_for_update` | — | `UpdateInfo { available, current_version, new_version, release_notes }` | Consulta o endpoint do updater (GitHub releases) e retorna info da última versão. NÃO baixa |
| `download_and_install_update` | — | — | Baixa, verifica assinatura, instala MSI e reinicia o app. Erro se nenhuma update disponível |

## Eventos emitidos pelo backend

- `deepseek-token` (`{ content: String }`) — chunk de streaming
- `deepseek-done` — fim do stream
- `vault-changed` (`VaultStatus`) — quando notify detecta mudança nos `.md` de **regra** (estilo/evitar/campos-padrao). NÃO dispara para `exemplos-*.md` — esses são lidos sob demanda na próxima geração.
- `category-detected` (`{ category: String, examples_used: usize }`) — após classify, antes do stream começar
- `tray-open-settings` (vazio) — emitido pelo backend quando o usuário clica em "Configurações" no menu do tray; o `ChatWindow` escuta e abre direto o `SettingsPanel`
- `cartilha-token` (`{ content: String }`) / `cartilha-done` — stream da geração de cartilha HTML (#22). Eventos separados dos da devolutiva pra UI não embaralhar contextos.

## Arquitetura da UI (estado atual 2026-05-23)

```
ChatWindow  (view: "form" | "result" | "cartilha" | "testes";  showSettings, showLearning: boolean)
├── view=="form"   → FormView (3 botões: Cartilha · Form testes · Devolutiva N1)
│     └── 10 campos → compileForm() → string → invoke(stream_completion / stream_cartilha / suggest_test_scenarios)
├── view=="result" → ResultView
│     ├── Chip de categoria no topo: "categoria: fiscal · N exemplos"
│     │     (aparece após `category-detected`, antes do streaming terminar)
│     ├── Durante streaming: renderResult() com tags [n]...[/n] como <strong>
│     ├── Após streaming: <textarea editável> com o conteúdo (usuário pode ajustar)
│     ├── Footer 3 botões:
│     │     ├── "Descartar" (vermelho)         → invoke(discard_entry)
│     │     ├── "Nova devolutiva" (cinza)      → abandona sem registrar
│     │     └── "Copiar e aprovar" (azul)      → clipboard + invoke(approve_entry)
├── view=="cartilha" → CartilhaView (#22)
│     ├── Título + Autor + Audience (select: suporte/cliente/interno)
│     ├── Área de imagens: paste (Ctrl+V) + drag-and-drop + file picker
│     ├── Heurística "imagens obrigatórias" se campo `parametro` preenchido ou `correcao` contém palavras-chave de fluxo
│     ├── Botão "Gerar conteúdo" → stream_cartilha → eventos cartilha-token/done
│     ├── Textarea editável com proposta da IA
│     └── Botão "Salvar no vault" → save_cartilha → escreve vault/cartilhas/YYYY-MM-DD-<slug>/{index.html, imagens/}
├── view=="testes" → TestesView (#23)
│     ├── 6 seções: Identificação · Pré-requisitos · Cenários · Regressão · Riscos
│     ├── Campos prefilled do FormView (scripts, parametros, cenario, validacao → happyPath)
│     ├── Botão "Sugerir com IA" → suggest_test_scenarios → preenche cenários/regressão/riscos
│     └── Botão "Gerar e copiar" → compileTestesText() → clipboard com tags `[n]...[/n]`
├── showSettings   → SettingsPanel  (botão ⚙ no header)
│     ├── API key  (invoke set_api_key)
│     └── Vault picker (dialog → invoke set_vault_path / seed_vault)
└── showLearning   → LearningPanel  (botão 🧠 no header)
      ├── Tabs: [ evitar.md ] [ estilo.md ] [ campos-padrão.md ] [ frases-modelo ]
      ├── Tab "evitar" → EvitarTab (#18)
      │     ├── Contador de edições aprovadas disponíveis
      │     ├── Botão "Analisar minhas edições" → invoke(analyze_edits)
      │     ├── Lista de EvitarSuggestion com checkboxes
      │     └── Botão "Adicionar N ao evitar.md" → invoke(apply_evitar_suggestions)
      ├── Tab "estilo" → EstiloTab (#19)
      │     ├── Contador de aprovadas-sem-edição (botão desabilitado se < 5)
      │     ├── Botão "Sintetizar estilo.md" → invoke(synthesize_style)
      │     ├── Após retorno: textarea editável (50vh) com a proposta
      │     └── Botões "Cancelar" / "Substituir estilo.md" → invoke(apply_style_synthesis)
      ├── Tab "campos" → CamposTab (#20)
      │     ├── Botão "Analisar histórico" → invoke(analyze_campos) (sem IA, parsing puro)
      │     ├── Seção "Release atual": checkbox único com a release semver mais recente
      │     ├── Seção "Caminhos novos": lista de checkboxes + ocorrências
      │     └── Botão "Aplicar N mudanças" → invoke(apply_campos_suggestions)
      └── Tab "frases" → FrasesTab (#21)
            ├── Botão "Buscar frases-modelo" → invoke(analyze_phrase_templates) (~30-60s)
            ├── Lista de PhraseTemplate com checkboxes (situation, template, ocorrências)
            └── Botão "Adicionar N ao campos-padrao.md" → invoke(apply_phrase_templates)
```

**Formato de saída da IA** — tags `[n]Título :[/n]` parseadas por `renderResult()` em `<strong className="devolutiva-header">`. Sem markdown (`##`, `**`) na saída.

**Badge do vault no header do chat:** `vault · N/3 · agora` (3 arquivos de regra; exemplos NÃO contam aqui — são per-categoria sob demanda).

## Modelo de dados — auto-curadoria por categoria

```
┌──────────────────────────────────────────────────────────────────────┐
│                       FLUXO DE GERAÇÃO                                │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  1. Usuário preenche FormView → compileForm() → user_input            │
│                                                                       │
│  2. classify(user_input, existing_categories) → "fiscal"              │
│        (1 chamada DeepSeek, ~10 tokens output, custo ínfimo)          │
│        emite event "category-detected"                                │
│                                                                       │
│  3. Lê arquivo `exemplos-fiscal.md` INTEIRO do vault                  │
│        ├── Blocos `## Aprovado em ...` (auto-gerados)                 │
│        └── Qualquer texto manual que o usuário tenha escrito          │
│            (notas, instruções específicas, exemplos próprios)         │
│                                                                       │
│  4. PromptBuilder monta system prompt:                                │
│        template fixo                                                   │
│        + ═══ ESTILO DO USUÁRIO ═══ (estilo.md do vault)               │
│        + ═══ EXPRESSÕES A EVITAR ═══ (evitar.md do vault)             │
│        + ═══ VALORES FREQUENTES ═══ (campos-padrao.md do vault)       │
│        + ═══ EXEMPLOS E NOTAS DA CATEGORIA: FISCAL ═══                │
│              (conteúdo bruto de exemplos-fiscal.md)                   │
│                                                                       │
│  5. messages = [system, user_input] → stream SSE                      │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│                       FLUXO DE APROVAÇÃO                              │
├──────────────────────────────────────────────────────────────────────┤
│                                                                       │
│  1. Usuário clica "Copiar e aprovar"                                  │
│                                                                       │
│  2. clipboard.writeText(textoEditado)                                 │
│                                                                       │
│  3. invoke("approve_entry", { raw_input, ai_raw_output, final })      │
│        ├── classify(raw_input) → "fiscal"                             │
│        ├── SQLite: INSERT entries (id, ..., category, edited)         │
│        │     edited = ai_raw_output ≠ final_output                    │
│        └── vault::append_to_category_examples("fiscal", ...)          │
│              → appenda bloco em exemplos-fiscal.md                    │
│                                                                       │
│  4. resetToForm(clearForm=true)                                       │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

**Princípio crítico de design (aprendido na sessão 2026-05-23):**

Os arquivos `exemplos-{categoria}.md` no vault são a **fonte de verdade** da injeção de exemplos. O usuário pode:
- Adicionar notas/instruções específicas da categoria (ex: "nesta categoria sempre cite o decreto X")
- Editar/remover blocos auto-gerados
- Adicionar exemplos próprios manualmente
- Reordenar conteúdo
- ...e tudo isso é lido pela IA na próxima geração da mesma categoria.

O SQLite **não é fonte de injeção** — serve apenas como histórico bruto para search FTS5, futuras análises de diff edit (#18 → evitar.md), stats, etc.

## Schema SQLite

```sql
CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    raw_input TEXT NOT NULL,           -- o que compileForm produziu
    ai_raw_output TEXT NOT NULL,       -- o que a IA gerou (sem edição do usuário)
    final_output TEXT NOT NULL,        -- o que o usuário aprovou (potencialmente editado)
    approved INTEGER NOT NULL,         -- 1=aprovado, 0=descartado
    edited INTEGER NOT NULL,           -- 1 se ai_raw_output != final_output
    model TEXT NOT NULL,               -- "deepseek-chat"
    created_at INTEGER NOT NULL,       -- unix epoch
    category TEXT                      -- ex: "fiscal", NULL para rows pré-migração
);

CREATE INDEX idx_entries_approved ON entries(approved);
CREATE INDEX idx_entries_created_at ON entries(created_at DESC);
CREATE INDEX idx_entries_category ON entries(category, approved, created_at DESC);

CREATE VIRTUAL TABLE entries_fts USING fts5(
    raw_input, ai_raw_output, final_output,
    content='entries', content_rowid='id'
);

-- Triggers ai/ad mantém entries_fts em sincronia com entries
```

Migração da coluna `category`: `ensure_category_column()` verifica `pragma_table_info` e adiciona ALTER se ausente. Idempotente.

## Estado das fases do roadmap

### ✅ Fase 0 — Spike técnico (concluída)
- Tauri 2 bootstrap, FAB always-on-top com drag nativo, Chat compacto
- Cliente DeepSeek com streaming SSE funcionando
- Persistência de API key (após pivot do keyring para config.json — vide gotcha 1)

### ✅ Fase 0.5 — Correções e redesign (concluída 2026-05-22)
- Bug streaming dobrado corrigido: `StrictMode` removido, listeners com `Promise.all`+`active`, `chat.emit()` no backend, `sendingRef` no frontend
- UI: chat livre → **formulário estruturado** (10 campos, `compileForm()`, `FormView`, `ResultView`)
- Formato de saída: `[n]...[/n]` com `renderResult()` e `.devolutiva-header`
- Rebrand DevReturn → Artemis (todos os arquivos)
- Ícones: `npx tauri icon resources/foguete.png`; FAB usa `<img>`

### ✅ Fase 1 — Histórico SQLite com edit tracking (concluída 2026-05-23)
- `rusqlite 0.32 bundled` + `chrono 0.4` + FTS5 virtual table
- Schema completo com `category` (migração idempotente)
- Comandos: save/list/search/delete/count + list_categories
- `History` struct com Mutex<Connection>
- DB em `%APPDATA%/Artemis/history.db`

### ✅ Auto-curadoria com sinais explícitos (concluída 2026-05-23)
- `ResultView` editável: textarea após streaming, usuário ajusta
- 3 botões: **Descartar** / **Nova devolutiva** / **Copiar e aprovar**
- "Aprovar" copia para clipboard + salva no SQLite + escreve no vault
- "Descartar" salva como approved=false (sinal negativo, futura iteração 18)
- "Nova" abandona sem registro (sem viés)
- Auto-detecção de `edited` = `ai_raw_output ≠ final_output`

### ✅ Categorização auto-descoberta (concluída 2026-05-23)
- `deepseek::classify()` — chamada não-streaming retornando 1 palavra
- `slugify_category()` — normaliza para ASCII kebab-case (ex: "Promoção" → "promocao")
- Categorias **emergem organicamente** — não há lista fixa; a IA escolhe entre as existentes ou propõe nova
- Anchoragem: o prompt de classify recebe as categorias já vistas para evitar fragmentação
- `category-detected` event mostrado no chip da UI

### ✅ Pivot: vault como fonte de verdade (concluída 2026-05-23)
**Reversão deliberada de decisão anterior** (que tinha SQLite como fonte de few-shot via `list_approved_by_category`).

**Motivação do pivot** (push-back do usuário): "ler do SQLite tornava o usuário totalmente dependente das escolhas da IA — irreversível se ele quisesse acrescentar uma instrução em uma categoria específica".

**Implementação:**
- `vault::load_category_examples()` lê `exemplos-{slug}.md` sob demanda na geração
- `PromptBuilder::with_category(cat, examples)` injeta o arquivo **inteiro** como seção do system prompt (não parseia em pares user/assistant)
- Qualquer texto manual escrito pelo usuário no `.md` (notas, regras, exemplos próprios) chega à IA
- SQLite vira só histórico/search (método `list_approved_by_category` mantido com `#[allow(dead_code)]` para uso futuro)
- VaultLoader carrega só os 3 arquivos de regra (não mais o `exemplos-aprovados.md` monolítico — esse arquivo virou legado)

### ✅ #18 — Diff edit → sugestões para `evitar.md` (concluída 2026-05-23)
Loop de aprendizado **negativo** fechado. Quando o usuário edita o output da IA antes de aprovar (`edited=true`), o app pode analisar esses pares e sugerir adições ao `evitar.md`.

**Backend:**
- `History::list_edited_approved(limit)` + `count_edited_approved()`
- `deepseek::analyze_edits(api_key, pairs) -> Vec<EvitarSuggestion>` — chamada não-streaming com prompt instruindo JSON puro; parser robusto (`extract_json_array`) tolera resposta com texto extra
- `vault::append_to_evitar(path, suggestions)` — appenda com marker `<!-- auto-aprendidos em DATA -->`
- Comandos: `count_edited_approved`, `analyze_edits`, `apply_evitar_suggestions`

**Frontend:**
- Botão **🧠** no header do chat ao lado de ⚙
- `LearningPanel` (view completa, mesmo pattern de SettingsPanel)
- Mostra contador de edições disponíveis; analisa só se ≥ 2
- Lista de sugestões com checkboxes (todas pré-selecionadas)
- Botões "Selecionar todas / Nenhuma" e contador "X/Y selecionadas"
- Após aplicar: mensagem de confirmação + nome do arquivo escrito

**UX:** o app **propõe**, o usuário **revisa e aceita** — autonomia preservada. Sugestões marcadas explicitamente no `evitar.md` para o usuário diferenciar do que escreveu manualmente.

### ✅ #19 — Síntese de `estilo.md` (concluída 2026-05-23)
Loop de aprendizado **positivo** fechado. Quando o usuário aprova devolutivas sem editar (`approved=1 AND edited=0`), o app pode pegar até 50 dessas amostras + o `estilo.md` atual e pedir à IA uma versão refinada do `estilo.md`.

**Backend:**
- `History::list_approved_unedited(limit)` + `count_approved_unedited()`
- `deepseek::synthesize_style(api_key, current_style, samples) -> String` — chamada não-streaming, temperature 0.3, max_tokens 4096. Retorna markdown puro
- `deepseek::extract_markdown_doc(raw)` — parser defensivo que strip-a preâmbulos ("Aqui está...") e fences `\`\`\`markdown` que LLM às vezes adiciona
- `vault::replace_estilo(path, content)` — backup do atual em `estilo.md.bak` (sobrescrito a cada vez), depois substitui. Recusa conteúdo vazio
- Comandos: `count_approved_unedited`, `synthesize_style`, `apply_style_synthesis`

**Frontend:**
- `LearningPanel` reorganizado em 2 tabs: "Sugestões para evitar.md" (#18) e "Sintetizar estilo.md" (#19)
- `EstiloTab` mostra contador, botão "Sintetizar" (desabilitado se < 5), e após resposta da IA um textarea de 50vh com a proposta editável
- Botões "Cancelar" / "Substituir estilo.md" — usuário pode editar a proposta antes de aplicar

**UX:** padrão idêntico ao #18 — app **propõe**, usuário **revisa**, opcionalmente **edita**, **aceita**. Backup em `.bak` (apenas último estado — se quiser histórico completo, versionar vault com git).

**Por que separar #18 (edited) e #19 (unedited):** sinais diferentes precisam de tratamentos diferentes. Edições são "a IA errou aqui" → vira regra negativa (evitar.md). Acertos puros são "a IA bateu o tom" → vira refinamento do estilo positivo (estilo.md). Misturar diluiria os dois sinais.

### ✅ #20 — Stats em `campos-padrao.md` (concluída 2026-05-23)
Loop de aprendizado **factual** fechado (sem IA — parsing puro). Agrega o histórico de aprovadas e propõe atualizações ao `campos-padrao.md` em duas dimensões: **release atual** (max semver detectado nas devolutivas, formato `vX.Y.Z — dd/mm/aaaa`) e **caminhos** (sequências `A > B > C` mais frequentes que ainda não estão no arquivo).

**Backend:**
- Novo módulo `stats.rs` com parsers manuais (sem crate `regex` — evita adicionar 200KB de dep):
  - `extract_release(text) -> Option<(canonical, semver_tuple, date_tuple)>` — aceita `—` ou `-`, `v` ou `V`, valida dia 1-31/mês 1-12/ano 2000-2100
  - `extract_paths(text) -> Vec<String>` — split por delimitadores (`. , ; : ( ) ! ? \` "`), heurística PascalCase para descartar prosa antes/depois do path (`"o caminho é Guardian > Notas"` vira só `"Guardian > Notas"`)
  - `rank_paths(paths, min_occurrences) -> Vec<PathSuggestion>` — frequência, ordenação por count desc + alfabética
  - `pick_latest_release(releases) -> Option<String>` — max semver
- `vault::apply_campos_changes(path, release, new_paths)` — backup em `.bak`, substitui linha `**Release atual:**` via parser de linha (não regex), appenda caminhos na seção `## Caminhos recorrentes` ou cria nova seção com marker `<!-- auto-aprendidos em DATA -->`
- `vault::current_release_in_text` / `existing_paths_in_text` — usados para deduplicar antes de propor
- Comandos: `analyze_campos` (lê até 100 aprovadas, retorna `CamposSuggestions { release, paths, analyzed_count }`), `apply_campos_suggestions { release_accepted, paths_accepted }`

**Frontend:**
- `LearningPanel` agora com 3 tabs (evitar / estilo / campos-padrão)
- `CamposTab`: botão "Analisar histórico" (sem chamada de IA, é instantâneo); mostra seções "Release atual" (1 checkbox) e "Caminhos novos" (lista checkbox + ocorrências, top 10 com ≥ 2 ocorrências); botão "Aplicar N mudanças"

**Por que parsing puro em vez de IA:** os campos relevantes são **estruturados** (semver + path notation) e já vêm preenchidos pelo usuário em campos dedicados do formulário (`Release/versão`, `Caminho no sistema`). Parsing manual é gratuito, instantâneo, determinístico, sem dependência externa de rede. Frases-modelo (que SERIAM análise semântica) ficaram para iteração futura — se for prioritário, abrir como #21.

### ✅ Fase 4 — Polimento desktop (concluída 2026-05-23)
Pacote completo de integração com o sistema operacional Windows.

**Tray icon** (Tauri 2 core, feature `tray-icon`):
- Ícone na bandeja com tooltip "Artemis — devolutivas técnicas"
- Click esquerdo → abre o chat
- Menu (click direito): Abrir chat / Configurações / Sair
- "Configurações" emite event `tray-open-settings` que o ChatWindow escuta

**Global hotkey** (`tauri-plugin-global-shortcut`):
- `Ctrl+Shift+D` abre o chat de qualquer lugar do Windows (hardcoded nesta versão)
- Função `show_chat(app)` em `lib.rs` é a entrada comum compartilhada entre tray, menu e hotkey
- Configurabilidade via UI ficou para iteração futura (exige UI de captura de tecla + persistência + re-registro)

**Autostart** (`tauri-plugin-autostart`):
- Comandos `get_autostart_enabled` / `set_autostart_enabled`
- Toggle horizontal em SettingsPanel (`.toggle-row` no CSS)

**Auto-updater** (`tauri-plugin-updater`):
- Plugin no Builder + comandos `check_for_update` (não baixa, só consulta) e `download_and_install_update` (baixa, verifica assinatura, reinicia)
- Configuração em `tauri.conf.json`:
  - `bundle.createUpdaterArtifacts: true` (gera `.zip` + `.sig` ao buildar)
  - `plugins.updater.active: true`
  - `pubkey` inline (chave pública do par minisign gerado por `tauri signer generate`)
  - `endpoints: ["https://github.com/ThiagoLuigiM/ArtemisChat/releases/latest/download/latest.json"]`
- Chaves geradas em `.tauri/artemis.key` (privada, **gitignored**) + `.tauri/artemis.key.pub` (pública, redundante já que está inline)
- UI no SettingsPanel: botão "Verificar atualizações" + bloco com release notes e botão "Instalar e reiniciar"

**Workflow GitHub Actions** (`.github/workflows/release.yml`):
- Trigger: push de tag `v*` ou disparo manual
- Roda em `windows-latest`, instala Node 20 + Rust stable + cache
- Usa `tauri-apps/tauri-action@v0` para build + sign + criar Draft Release com MSI assinado e `latest.json` (manifest do updater)
- **Requer secrets configurados no repo GitHub** (vide próximos passos)

### ✅ #21 — Frases-modelo via IA (concluída 2026-05-23)
Última fronteira do aprendizado adaptativo. Analisa o `final_output` de até 80 aprovadas (editadas ou não) + o `campos-padrao.md` atual, e a IA extrai **templates parametrizados** de frases que se repetem (literalmente ou com variações de datas/versões/caminhos) em ≥ 3 devolutivas, evitando duplicar o que já está no arquivo.

**Backend:**
- `deepseek::extract_phrase_templates(api_key, samples, current_campos) -> Vec<PhraseTemplate>` — chamada não-streaming, temperature 0.2, max_tokens 2048. Prompt instrui a IA a (a) retornar JSON array puro, (b) usar `<placeholder>` nas partes variáveis, (c) verificar redundância contra o `campos-padrao.md` atual, (d) máximo 8 sugestões priorizando frequência e utilidade.
- Tipo `PhraseTemplate { situation, template, occurrences }` em `deepseek.rs`. Parser reusa `extract_json_array` (do #18).
- `vault::append_phrase_templates(path, templates)` — backup em `.bak`, encontra seção `## Frases-modelo aprovadas` (case-insensitive) ou cria nova no fim. Formato de cada item: `**<situation>:**\n> <template>` sob marker `<!-- auto-aprendidos em DATA -->`.
- Comandos: `analyze_phrase_templates` (mínimo 5 aprovadas), `apply_phrase_templates { templates }`.

**Frontend:**
- 4ª tab `FrasesTab` no LearningPanel (mesma estrutura do `EvitarTab`)
- Estilos `.phrase-situation` (destaque azul) e `.phrase-template` (blockquote com border-left)

**Tripleto+1 fechado:** `evitar.md` (#18 negativo) + `estilo.md` (#19 positivo) + `campos-padrao.md` em 2 dimensões: factual (#20 release/caminhos, parsing puro) + semântico (#21 frases-modelo via IA).

### ✅ #22 — Cartilha HTML didática (concluída 2026-05-23)
Geração automatizada de cartilhas a partir do mesmo input do FormView, com imagens obrigatórias quando há mudança de fluxo ou novo parâmetro.

**Backend:**
- `deepseek::build_cartilha_messages(form_input, audience, image_captions)` — prompt didático que estrutura saída em `[s]Título :[/s]` (variante do `[n]` da devolutiva, mas para "section"). Audience seleciona tom: suporte (técnico explicativo), cliente (acessível sem jargão), interno (técnico denso).
- `vault::save_cartilha(...)` + tipo `CartilhaImageInput { bytes, extension, caption }`. Cria `cartilhas/YYYY-MM-DD-<slug>/index.html` + `imagens/01.ext`, `02.ext`, ... Slug ASCII kebab-case, truncado a 60 chars pra não estourar limite de path do Windows. HTML self-contained com CSS inline (sem dependências externas, abre em qualquer browser ou offline).
- `vault::content_to_html(content)` — parser que converte `[s]...[/s]` em `<h2>`, quebras duplas em `<p>` separados, simples em `<br>`. Escape HTML em todos os campos pra evitar quebrar o template.
- Comandos Tauri: `stream_cartilha` (streaming SSE, eventos `cartilha-token`/`cartilha-done` separados), `save_cartilha` (recebe imagens como `Vec<u8>` direto, sem base64), `open_in_system` (Windows `cmd /c start`).

**Frontend:**
- `CartilhaView` componente: input para título + autor + audience (select), área de imagens com paste (`document.addEventListener("paste", ...)`) + drag-and-drop + file picker.
- Heurística `cartilhaImagesRequired(form)`: imagens obrigatórias se campo `parametro` preenchido OU `correcao` contém palavras-chave (`nova tela`, `novo botão`, `novo fluxo`, `nova permissão`, etc.). UI mostra badge vermelho com razão e desabilita "Gerar conteúdo".
- Stream em `<pre>` durante geração; `<textarea>` editável depois pra usuário ajustar antes de salvar.
- Após salvar: botão "Abrir no navegador" via `open_in_system`.

**Template HTML:** CSS inline (variáveis CSS pra tema), header com título+release+data+autor, conteúdo em `<main>`, galeria de imagens em `<figure>+<figcaption>` no fim, footer com referência ao Artemis, media query `@media print` para impressão.

**Tests:** 5 testes em `vault.rs` cobrindo `save_cartilha` (cria pasta+imagens+HTML, rejeita título/conteúdo vazios, slug truncado/sanitizado) e `content_to_html` (seções+parágrafos, escape XSS, tag não fechada).

### ✅ #23 — Formulário de testes para QA (concluída 2026-05-23)
Form complementar pra dev mandar pra equipe de testes; estrutura definida pelo Artemis (a equipe não forneceu diretriz).

**Estrutura (6 seções):**
1. **Identificação** — Ticket, data de entrega, tipo (Correção / Nova feature / Melhoria), release (do FormView)
2. **Pré-requisitos** — Ambiente (Homologação / Espelho / Dev), Scripts (prefill), Parâmetros (prefill), Dados de teste
3. **Cenários** — Caminho feliz, Edge cases, Cenários negativos, Critérios de aceitação
4. **Regressão** — Áreas afetadas que devem ser revalidadas
5. **Riscos** — Limitações, cenários não simulados (prefill)

**Backend:**
- `deepseek::suggest_test_scenarios(api_key, form_input) -> TestScenariosSuggestion` — chamada não-streaming, temp 0.4, max_tokens 2048. Prompt pede JSON com 6 campos string; parser `extract_json_object` (novo, análogo a `extract_json_array`).
- Tipo `TestScenariosSuggestion { happy_path, edge_cases, negative_cases, acceptance_criteria, regression_areas, risks }`.
- Comando Tauri: `suggest_test_scenarios { form_input }`.

**Frontend:**
- `TestesView` componente com 6 seções, prefill de campos do FormView (scripts/parametros/cenario/validacao).
- Botão "Sugerir com IA" no header da seção 3 (Cenários) → preenche 4 textareas (happy/edge/negative/criteria) + 2 (regression/risks).
- Botão "Gerar e copiar" → `compileTestesText()` monta texto usando mesmas tags `[n]...[/n]` da devolutiva pra consistência visual no ticket → `navigator.clipboard.writeText(...)`.

**Decisão deliberada:** sem persistência SQLite nesta versão. O form é descartável após copiar pro ticket. Se equipe QA pedir histórico depois, adicionar coluna `test_form: Option<String>` na `entries` é trivial (migration idempotente já tem padrão estabelecido).

### ⏳ Próximas fases pendentes
- **Configurabilidade de hotkey** — UI de captura + persistência em config.json + re-registro dinâmico do global shortcut
- **Cifrar API key com DPAPI** — substituir plaintext em `config.json` por `CryptProtectData` escopo current-user (vide gotcha #1)
- **Imagens inline na cartilha (opcional)** — hoje todas vão pra galeria no fim. Próxima iteração: IA gera placeholders `[img:01]` no texto e renderer substitui por `<figure>` inline

## Próximo passo imediato (status 2026-05-23 sessão 7)

**#22 (cartilha HTML) + #23 (form de testes)** entregues. Mesmo input do FormView agora alimenta 3 outputs paralelos: devolutiva N1 (já existia) + cartilha HTML salva no vault + form de testes copiado pro clipboard. 59/59 testes Rust passam (+10 novos: 5 cartilha + 5 deepseek), `cargo check` 0 warnings, `tsc --noEmit` 0 erros.

### Validação manual pendente do usuário:
1. **Cartilha:** preencher formulário com algo que inclua "novo botão" ou similar → clicar "Cartilha" → ver badge de imagens obrigatórias → colar/arrastar 1-2 imagens com legendas → "Gerar conteúdo" → revisar texto → "Salvar no vault" → conferir `vault/cartilhas/YYYY-MM-DD-<slug>/index.html` no Obsidian ou abrir no browser
2. **Form de testes:** preencher formulário com um cenário qualquer → clicar "Form de testes" → conferir prefill → "Sugerir com IA" e ver sugestões aparecerem → editar se quiser → "Gerar e copiar" → colar no Notepad pra verificar formato

### ⚠️ Setup pendente para o auto-updater funcionar (ação do usuário, ainda do roteiro anterior)

### ⚠️ Setup pendente para o auto-updater funcionar (ação do usuário)

O código está pronto, mas pra o updater funcionar end-to-end o **usuário precisa**:

1. **Push do código inicial** (`main` branch já configurada localmente, remote já adicionado):
   ```powershell
   cd E:\Projetos-Thiago\Space\WorkSpaceArtemis\DevReturn
   git push -u origin main
   ```

2. **Configurar 2 secrets no GitHub** (Settings → Secrets and variables → Actions → New repository secret):
   - `TAURI_SIGNING_PRIVATE_KEY` — conteúdo de `.tauri/artemis.key` (cole o arquivo inteiro)
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — string vazia (chave foi gerada sem senha; defina como `""` ou pule este secret)

3. **Criar a primeira release** (tag dispara o workflow):
   ```powershell
   git tag v0.1.0
   git push origin v0.1.0
   ```
   O workflow vai rodar (~10-15 min) e criar um **Draft Release** no GitHub com o `.msi` assinado + `latest.json`. **Publicar manualmente** o draft para liberar updates.

4. **Backup da chave privada** (`.tauri/artemis.key`): se perder, **nunca mais consegue assinar updates** para usuários atuais. Salve numa senha-gerente offline ou pendrive seguro.

### Validação manual local (sem precisar de release)

- Tray: ícone deve aparecer na bandeja; click esquerdo abre chat; menu funciona
- Hotkey: `Ctrl+Shift+D` em qualquer app foco do Windows → chat abre
- Autostart: marcar checkbox em Settings → reiniciar Windows → app deve subir
- Updater: clicar "Verificar atualizações" — deve falhar com "endpoint não respondeu" enquanto não houver release publicada (esperado)

### Após validação, opcionais:
- **#21** — frases-modelo via IA (se necessário)
- **Configurabilidade de hotkey** (UI + persistência + re-registro)
- **Build local de teste:** `npm run tauri build` gera MSI em `src-tauri/target/release/bundle/msi/` sem precisar de release no GitHub

## Gotchas descobertos (NÃO REPETIR)

### 1. keyring 3.6.3 está bugado no Windows 11 deste usuário
- `Entry::set_password()` retorna `Ok(())` mas a credencial **nunca chega ao Credential Manager**
- `cmdkey` (Wincred API direto) funciona — bug específico da crate
- **Solução adotada:** plain JSON em `%APPDATA%/Artemis/config.json`. Threat model equivalente para single-user desktop.
- **Se for tentar de novo no futuro:** SEMPRE incluir roundtrip verify (write → read → compare) antes de assumir sucesso.

### 2. Tauri 2 — permissões granulares
`core:default` **NÃO** inclui várias permissões interativas comuns. Adicionar explicitamente em `capabilities/default.json`:
- `core:window:allow-start-dragging` (para `startDragging()`)
- `core:window:allow-set-position`
- `core:window:allow-set-focus`
- `core:webview:allow-internal-toggle-devtools`
- `dialog:default` + `dialog:allow-open` (para folder picker)

Sintoma de falta de permissão: chamada Tauri JS falha silenciosamente.

### 3. PowerShell `Out-String` no Bash tool
`Out-String` bufferiza toda a saída até o processo terminar. Para logs streaming de processos longos (`npm run tauri dev`), redirecionar direto sem pipe pelo Out-String.

### 4. FAB drag: discriminar click vs drag
Padrão funciona: `mousedown` registra posição → `mousemove` com `e.buttons === 1` e movimento >4px chama `startDragging()` → `mouseup` invoca `open_chat` apenas se `draggedRef.current === false`. Listeners no `document`, não no botão (cobertura completa do 72x72).

### 5. PowerShell variável `$pid` é read-only
Não usar `$pid` como variável local em scripts — é reservado pelo PowerShell. Use `$processId` ou similar.

### 6. PowerShell `Set-Content` corrompe UTF-8 com caracteres não-ASCII
PS 5.1 lê arquivos com encoding do sistema (CP1252). Se o arquivo é UTF-8, bytes multi-byte viram dois chars CP1252. Re-gravar com `Set-Content -Encoding UTF8` causa dupla-codificação — `ç` vira `Ã§`.

**Solução:** usar sempre `[System.IO.File]::WriteAllText($f, $content, [System.Text.UTF8Encoding]::new($false))`.

### 7. Substituição parcial deixa código duplicado no final do arquivo
Se `Edit` substitui uma função mas não cobre o código antigo que segue, ambas as versões coexistem — gerando `Identifier 'X' has already been declared` no Vite.

**Detecção:** `grep -n "export default function X" arquivo.tsx` — se aparecer 2× há duplicata.

### 8. Cuidado ao usar SQLite como fonte de verdade quando há equivalente no vault
Tentei usar `History::list_approved_by_category` para alimentar few-shot na geração. O usuário corretamente apontou que isso torna o sistema "totalmente dependente das escolhas da IA — irreversível se quiser acrescentar uma instrução em uma categoria específica". 

**Lição:** se o vault tem um artefato visível ao usuário (arquivos `.md` no Obsidian), ele deve ser a fonte de verdade da leitura. Storage interno (SQLite) é registro/histórico, não input do prompt. Preservar autonomia do usuário sobre o comportamento do sistema.

### 9. `#[tauri::command]` exige tipos públicos no retorno
Se um comando retorna `Result<MyStruct, String>`, então `MyStruct` precisa ser `pub`. Caso contrário, erro `type is more private than the item` em compile.

### 10. Tauri dev watcher detecta Cargo.toml mas pode demorar
Mudanças em `src-tauri/Cargo.toml` (ex: adicionar `rusqlite`) disparam rebuild via Tauri's cargo watcher, MAS demoram mais que mudanças em `.rs` (precisa baixar/compilar a nova dep). Primeira compilação de `rusqlite bundled` leva ~3-5 min (compila SQLite em C). Subsequentes são cached.

### 11. Flakiness em `temp_history()` / `temp_vault()` dos testes
Helpers de teste criam diretório com `SystemTime::now().as_nanos() + process_id()`. Cargo roda testes em paralelo por default — colisão de nanos entre 2 testes simultâneos faz eles compartilharem o mesmo DB, contaminando resultados (já vi `count_approved_unedited_matches_list` esperar 2 e receber 5 numa run, depois passar na re-run sem mudança). Workaround: re-rodar `cargo test --lib`. Fix proposto (chip): trocar para `tempfile::TempDir`.

## Validação de qualidade de código

Antes de marcar uma task como completa, rodar:
- `cargo check --message-format=short` em `src-tauri/` → 0 erros, idealmente 0 warnings
- `npx tsc --noEmit` na raiz → 0 erros
- Para mudanças no `PromptBuilder`, `History`, `vault.rs`: rodar testes unitários:
  ```
  cd src-tauri && cargo test
  ```

## Memórias persistentes relacionadas

Em `C:\Users\Borge\.claude\projects\E--Projetos-Thiago-Space-WorkSpaceArtemis\memory\`:
- `project_devreturn.md` (nome legado — conteúdo atualizado para Artemis)
- `feedback_no_electron.md`
- `reference_tauri_capabilities.md`
- `feedback_keyring_broken_windows.md`

## Convenções do usuário

- Português brasileiro nos textos da UI e mensagens de log
- Não usar Electron jamais (RAM)
- Preferir resoluções pragmáticas a "fazer certo": pivotar quando uma lib não coopera (caso keyring)
- Validar implementações com o usuário antes de avançar fases
- **Autonomia do usuário sobre o comportamento da IA é princípio inegociável** — qualquer feature que reduza essa autonomia deve ser revertida (vide pivot 2026-05-23)
- **Atualizar este HANDOFF.md a cada ciclo de mudanças** (regra no topo)

---

**Última atividade:** Sessão Claude Opus 4.7 (sétima continuação) em 2026-05-23 BRT. Entregues nesta sessão: **#22 (cartilha HTML didática)** e **#23 (form de testes pra QA)**. Cartilha: `deepseek::build_cartilha_messages` constrói prompt com audience configurável (suporte/cliente/interno), stream via eventos `cartilha-token`/`cartilha-done`; `vault::save_cartilha` cria `cartilhas/YYYY-MM-DD-<slug>/index.html` + subpasta `imagens/`, template HTML self-contained com CSS inline; UI `CartilhaView` suporta paste (Ctrl+V) + drag-and-drop + file picker pra imagens, com heurística `cartilhaImagesRequired` que obriga imagens quando há `parametro` preenchido ou palavras-chave de fluxo no `correcao`. Form de testes: `deepseek::suggest_test_scenarios` retorna `TestScenariosSuggestion` JSON; novo parser `extract_json_object` análogo ao `extract_json_array`; UI `TestesView` com 6 seções (Identificação/Pré-req/Cenários/Regressão/Riscos), prefill do FormView, "Sugerir com IA", "Gerar e copiar" usando tags `[n]...[/n]`. FormView ganhou 3 botões na footer (Cartilha · Form testes · Devolutiva N1). 59/59 testes (+10 novos: 5 cartilha + 5 deepseek/extract_json_object), `cargo check` 0 warnings, `tsc --noEmit` 0 erros. Decisão: cartilha salva no vault (artefato histórico); form de testes só vai pro clipboard (descartável). Plus: novo comando `open_in_system` (sem plugin extra — usa `cmd /c start`) pra abrir cartilha no navegador padrão.
