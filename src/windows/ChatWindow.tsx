import { useState, useEffect, useRef, useCallback } from "react";
import type React from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

interface TokenEvent {
  content: string;
}

interface VaultStatus {
  path: string | null;
  last_loaded_ts: number | null;
  files_present: string[];
  files_total: number;
  total_chars: number;
  error: string | null;
}

interface CategoryEvent {
  category: string;
  examples_used: number;
}

interface EvitarSuggestion {
  expression: string;
  reason: string;
  occurrences: number;
}

interface ReleaseSuggestion {
  proposed: string;
  current_in_file: string | null;
}

interface PathSuggestion {
  path: string;
  occurrences: number;
}

interface CamposSuggestions {
  release: ReleaseSuggestion | null;
  paths: PathSuggestion[];
  analyzed_count: number;
}

interface UpdateInfo {
  available: boolean;
  current_version: string;
  new_version: string | null;
  release_notes: string | null;
}

// ── Form data ────────────────────────────────────────────────────────────────

interface FormFields {
  correcao: string;
  caminho: string;
  parametro: string;
  release: string;
  atualizacao: string;
  validacao: string;
  solucaoTipo: string;
  scripts: string;
  pendencias: string;
  cenario: string;
}

const EMPTY_FORM: FormFields = {
  correcao: "",
  caminho: "",
  parametro: "",
  release: "",
  atualizacao: "",
  validacao: "",
  solucaoTipo: "",
  scripts: "",
  pendencias: "",
  cenario: "",
};

function compileForm(f: FormFields): string {
  const parts: string[] = [];

  const add = (label: string, value: string) => {
    if (value.trim()) parts.push(`${label}:\n${value.trim()}`);
  };
  const addLine = (label: string, value: string) => {
    if (value.trim()) parts.push(`${label}: ${value.trim()}`);
  };

  add("O que foi alterado/corrigido", f.correcao);
  addLine("Caminho no sistema", f.caminho);
  add("Novo parâmetro ou permissão", f.parametro);
  addLine("Release/versão", f.release);

  if (f.atualizacao) {
    const labels: Record<string, string> = {
      padrao: "Atualização padrão do sistema (sem troca de executável)",
      executavel: "Necessário atualizar o sistema e substituir o executável",
      nenhuma: "Nenhuma atualização necessária",
    };
    addLine("Necessidade de atualização", labels[f.atualizacao] ?? "");
  }

  add("Forma de validação pelo suporte", f.validacao);

  if (f.solucaoTipo) {
    addLine("Tipo de solução", f.solucaoTipo === "definitiva" ? "Definitiva" : "Paliativa");
  }

  if (f.scripts.trim()) {
    parts.push(`Scripts/ações manuais:\n\`\`\`sql\n${f.scripts.trim()}\n\`\`\``);
  }

  add("Pendências ou observações", f.pendencias);
  add("Cenário não simulado (testes realizados)", f.cenario);

  return parts.join("\n\n");
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function relativeTime(ts: number | null): string {
  if (!ts) return "—";
  const now = Math.floor(Date.now() / 1000);
  const diff = Math.max(0, now - ts);
  if (diff < 5) return "agora";
  if (diff < 60) return `${diff}s atrás`;
  if (diff < 3600) return `${Math.floor(diff / 60)}min atrás`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h atrás`;
  return `${Math.floor(diff / 86400)}d atrás`;
}

function VaultBadge({ status }: { status: VaultStatus | null }) {
  if (!status || !status.path) {
    return <span className="vault-badge vault-badge-empty">vault não configurado</span>;
  }
  if (status.error) {
    return (
      <span className="vault-badge vault-badge-error" title={status.error}>
        vault: erro
      </span>
    );
  }
  const filesCount = status.files_present.length;
  const total = status.files_total ?? 3;
  const ago = relativeTime(status.last_loaded_ts);
  return (
    <span
      className="vault-badge"
      title={`Vault: ${status.path}\n${filesCount}/${total} arquivos de regras · ${status.total_chars} chars\nExemplos aprovados vêm do SQLite (não do vault)\nÚltima leitura: ${ago}`}
    >
      vault · {filesCount}/{total} · {ago}
    </span>
  );
}

// ── FormView ──────────────────────────────────────────────────────────────────

function FormView({
  fields,
  onChange,
  onGenerate,
  disabled,
}: {
  fields: FormFields;
  onChange: (f: FormFields) => void;
  onGenerate: () => void;
  disabled: boolean;
}) {
  const set =
    <K extends keyof FormFields>(key: K) =>
    (e: React.ChangeEvent<HTMLTextAreaElement | HTMLInputElement | HTMLSelectElement>) =>
      onChange({ ...fields, [key]: e.target.value });

  const canGenerate = fields.correcao.trim().length > 0;

  return (
    <>
      <div className="form-body">

        <div className="form-field">
          <label>
            O que foi alterado/corrigido <span className="field-required">*</span>
          </label>
          <textarea
            rows={3}
            value={fields.correcao}
            onChange={set("correcao")}
            disabled={disabled}
            placeholder="Descreva a correção, melhoria ou ajuste realizado..."
            autoFocus
          />
        </div>

        <div className="form-row-2">
          <div className="form-field">
            <label>Release / versão</label>
            <input
              type="text"
              value={fields.release}
              onChange={set("release")}
              disabled={disabled}
              placeholder="v2.54.x — dd/mm/aaaa"
            />
          </div>
          <div className="form-field">
            <label>Tipo de solução</label>
            <select value={fields.solucaoTipo} onChange={set("solucaoTipo")} disabled={disabled}>
              <option value="">—</option>
              <option value="definitiva">Definitiva</option>
              <option value="paliativa">Paliativa</option>
            </select>
          </div>
        </div>

        <div className="form-field">
          <label>Necessidade de atualização</label>
          <select value={fields.atualizacao} onChange={set("atualizacao")} disabled={disabled}>
            <option value="">—</option>
            <option value="padrao">Atualização padrão (sem executável)</option>
            <option value="executavel">Atualização + troca de executável</option>
            <option value="nenhuma">Nenhuma</option>
          </select>
        </div>

        <div className="form-field">
          <label>
            Caminho no sistema <span className="field-optional">opcional</span>
          </label>
          <input
            type="text"
            value={fields.caminho}
            onChange={set("caminho")}
            disabled={disabled}
            placeholder="Ex: Guardian > Notas Fiscais > Emissão"
          />
        </div>

        <div className="form-field">
          <label>
            Novo parâmetro ou permissão <span className="field-optional">opcional</span>
          </label>
          <textarea
            rows={2}
            value={fields.parametro}
            onChange={set("parametro")}
            disabled={disabled}
            placeholder="Nome, caminho e uso do novo parâmetro ou permissão..."
          />
        </div>

        <div className="form-field">
          <label>
            Forma de validação <span className="field-optional">opcional</span>
          </label>
          <textarea
            rows={2}
            value={fields.validacao}
            onChange={set("validacao")}
            disabled={disabled}
            placeholder="Como o suporte deve reproduzir e validar a correção..."
          />
        </div>

        <div className="form-field">
          <label>
            Scripts / SQL <span className="field-optional">opcional</span>
          </label>
          <textarea
            rows={2}
            value={fields.scripts}
            onChange={set("scripts")}
            disabled={disabled}
            className="monospace"
            placeholder="SELECT ... / UPDATE ... / EXEC sp_..."
          />
        </div>

        <div className="form-field">
          <label>
            Pendências ou observações <span className="field-optional">opcional</span>
          </label>
          <textarea
            rows={2}
            value={fields.pendencias}
            onChange={set("pendencias")}
            disabled={disabled}
            placeholder="Ressalvas, próximos passos ou informações adicionais..."
          />
        </div>

        <div className="form-field">
          <label>
            Cenário não simulado <span className="field-optional">opcional</span>
          </label>
          <textarea
            rows={2}
            value={fields.cenario}
            onChange={set("cenario")}
            disabled={disabled}
            placeholder="Testes realizados e limitações do ambiente de desenvolvimento..."
          />
        </div>

      </div>

      <div className="form-footer">
        <button
          type="button"
          className="btn-secondary"
          onClick={() => onChange(EMPTY_FORM)}
          disabled={disabled}
        >
          Limpar
        </button>
        <button
          type="button"
          className="btn-primary"
          onClick={onGenerate}
          disabled={disabled || !canGenerate}
        >
          {disabled ? "Gerando..." : "Gerar devolutiva"}
        </button>
      </div>
    </>
  );
}

// ── renderResult ──────────────────────────────────────────────────────────────

function renderResult(text: string): React.ReactNode {
  const parts = text.split(/(\[n\][\s\S]*?\[\/n\])/);
  return parts.map((part, i) => {
    const match = part.match(/^\[n\]([\s\S]*?)\[\/n\]$/);
    if (match) {
      return <strong key={i} className="devolutiva-header">{match[1]}</strong>;
    }
    return <span key={i}>{part}</span>;
  });
}

// ── ResultView ────────────────────────────────────────────────────────────────
//
// Durante streaming: mostra rendered output (read-only, com [n] como <strong>).
// Depois do streaming: textarea editável (o "final_output" = o que o usuário aprova).
// 3 botões: Descartar (sinal negativo) · Nova (abandona sem registrar) · Copiar e aprovar (clipboard + persiste + auto-curadoria).

function ResultView({
  aiRawOutput,
  streaming,
  category,
  onNew,
  onApprove,
  onDiscard,
}: {
  aiRawOutput: string;
  streaming: boolean;
  category: CategoryEvent | null;
  onNew: () => void;
  onApprove: (finalText: string) => Promise<void>;
  onDiscard: (finalText: string) => Promise<void>;
}) {
  const [editedText, setEditedText] = useState(aiRawOutput);
  const [submitting, setSubmitting] = useState<"approve" | "discard" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  // Mantém o textarea sincronizado enquanto a IA está streaming;
  // quando streaming termina, o usuário "toma posse" do texto.
  useEffect(() => {
    if (streaming) setEditedText(aiRawOutput);
  }, [aiRawOutput, streaming]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [aiRawOutput]);

  const handleApprove = async () => {
    if (submitting || streaming || !editedText.trim()) return;
    setError(null);
    setSubmitting("approve");
    try {
      await navigator.clipboard.writeText(editedText);
      await onApprove(editedText);
    } catch (e) {
      setError(`Erro ao aprovar: ${String(e)}`);
      setSubmitting(null);
    }
  };

  const handleDiscard = async () => {
    if (submitting || streaming) return;
    setError(null);
    setSubmitting("discard");
    try {
      await onDiscard(editedText);
    } catch (e) {
      setError(`Erro ao descartar: ${String(e)}`);
      setSubmitting(null);
    }
  };

  return (
    <>
      {category && (
        <div className="category-chip" title="Categoria detectada pela IA + número de exemplos aprovados injetados no contexto">
          <span className="category-label">categoria:</span>{" "}
          <strong>{category.category}</strong>{" "}
          <span className="category-meta">· {category.examples_used} exemplo{category.examples_used === 1 ? "" : "s"}</span>
        </div>
      )}
      <div className="result-body">
        {streaming && !aiRawOutput && (
          <div className="result-waiting">
            Gerando devolutiva<span className="result-dots" />
          </div>
        )}
        {streaming ? (
          <div className="result-output">{renderResult(aiRawOutput)}</div>
        ) : (
          <textarea
            className="result-editor"
            value={editedText}
            onChange={(e) => setEditedText(e.target.value)}
            disabled={!!submitting}
            spellCheck
            placeholder="O texto gerado aparecerá aqui. Você pode ajustar antes de aprovar."
          />
        )}
        <div ref={bottomRef} />
        {error && <p className="result-error">{error}</p>}
      </div>

      <div className="result-footer">
        <button
          type="button"
          className="btn-danger"
          onClick={handleDiscard}
          disabled={streaming || !!submitting || !aiRawOutput}
          title="Marca como ruim. Não vai pro vault de exemplos aprovados."
        >
          {submitting === "discard" ? "Descartando..." : "Descartar"}
        </button>
        <button
          type="button"
          className="btn-secondary"
          onClick={onNew}
          disabled={streaming || !!submitting}
          title="Volta ao formulário sem registrar sinal."
        >
          Nova devolutiva
        </button>
        <button
          type="button"
          className="btn-primary"
          onClick={handleApprove}
          disabled={streaming || !!submitting || !editedText.trim()}
          title="Copia para o clipboard + salva no histórico + adiciona em exemplos-aprovados.md"
        >
          {submitting === "approve" ? "Salvando..." : "Copiar e aprovar"}
        </button>
      </div>
    </>
  );
}

// ── SettingsPanel ─────────────────────────────────────────────────────────────

interface SettingsPanelProps {
  initialKey: string;
  vaultStatus: VaultStatus | null;
  onSaveKey: (key: string) => Promise<void>;
  onClose?: () => void;
  onVaultChanged: (status: VaultStatus) => void;
}

function SettingsPanel({
  initialKey,
  vaultStatus,
  onSaveKey,
  onClose,
  onVaultChanged,
}: SettingsPanelProps) {
  const [key, setKey] = useState(initialKey);
  const [keySaving, setKeySaving] = useState(false);
  const [keySaved, setKeySaved] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [vaultPath, setVaultPath] = useState<string | null>(vaultStatus?.path ?? null);
  const [vaultBusy, setVaultBusy] = useState(false);
  const [vaultMessage, setVaultMessage] = useState<string | null>(null);

  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [autostartError, setAutostartError] = useState<string | null>(null);

  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);

  useEffect(() => {
    setVaultPath(vaultStatus?.path ?? null);
  }, [vaultStatus]);

  useEffect(() => {
    invoke<boolean>("get_autostart_enabled")
      .then(setAutostart)
      .catch((e) => {
        setAutostart(false);
        setAutostartError(`Não foi possível ler o estado de autostart: ${String(e)}`);
      });
  }, []);

  const handleToggleAutostart = async () => {
    if (autostart === null || autostartBusy) return;
    const next = !autostart;
    setAutostartBusy(true);
    setAutostartError(null);
    try {
      await invoke("set_autostart_enabled", { enabled: next });
      setAutostart(next);
    } catch (e) {
      setAutostartError(String(e));
    } finally {
      setAutostartBusy(false);
    }
  };

  const handleCheckUpdate = async () => {
    setUpdateChecking(true);
    setUpdateError(null);
    setUpdateInfo(null);
    try {
      const info = await invoke<UpdateInfo>("check_for_update");
      setUpdateInfo(info);
    } catch (e) {
      setUpdateError(String(e));
    } finally {
      setUpdateChecking(false);
    }
  };

  const handleInstallUpdate = async () => {
    setUpdateInstalling(true);
    setUpdateError(null);
    try {
      await invoke("download_and_install_update");
      // App reinicia automaticamente após instalar — esse código não roda
    } catch (e) {
      setUpdateError(String(e));
      setUpdateInstalling(false);
    }
  };

  const handleSaveKey = async () => {
    setKeyError(null);
    const trimmed = key.trim();
    if (!trimmed) {
      setKeyError("Cole a chave (formato sk-...) antes de salvar.");
      return;
    }
    setKeySaving(true);
    setKeySaved(false);
    try {
      await onSaveKey(trimmed);
      setKeySaved(true);
    } catch (e) {
      setKeyError(String(e));
    } finally {
      setKeySaving(false);
    }
  };

  const handlePickVault = async () => {
    try {
      const selected = await openDialog({
        directory: true,
        multiple: false,
        title: "Escolha a pasta do vault Obsidian (subpasta artemis/)",
      });
      if (typeof selected === "string") {
        await applyVaultPath(selected);
      }
    } catch (e) {
      setVaultMessage(`Erro ao escolher pasta: ${String(e)}`);
    }
  };

  const applyVaultPath = async (path: string) => {
    setVaultBusy(true);
    setVaultMessage(null);
    try {
      const status = await invoke<VaultStatus>("set_vault_path", { path });
      setVaultPath(path);
      onVaultChanged(status);
      if (status.files_present.length === 0) {
        setVaultMessage(
          "Pasta vazia. Clique em \"Inicializar com templates\" para criar os 4 arquivos base.",
        );
      } else {
        setVaultMessage(`${status.files_present.length} arquivo(s) carregado(s).`);
      }
    } catch (e) {
      setVaultMessage(`Erro: ${String(e)}`);
    } finally {
      setVaultBusy(false);
    }
  };

  const handleSeed = async () => {
    if (!vaultPath) return;
    setVaultBusy(true);
    setVaultMessage(null);
    try {
      const created = await invoke<string[]>("seed_vault", { path: vaultPath });
      if (created.length === 0) {
        setVaultMessage("Nenhum arquivo novo (todos já existiam).");
      } else {
        setVaultMessage(`Criados: ${created.join(", ")}`);
      }
      const status = await invoke<VaultStatus>("get_vault_status");
      onVaultChanged(status);
    } catch (e) {
      setVaultMessage(`Erro ao criar templates: ${String(e)}`);
    } finally {
      setVaultBusy(false);
    }
  };

  return (
    <div className="settings-panel">
      <header className="settings-header">
        <h2>Configurações</h2>
        {onClose && (
          <button className="secondary close" onClick={onClose} aria-label="Voltar">
            ✕
          </button>
        )}
      </header>

      <section className="settings-section">
        <label>
          <span className="label">DeepSeek API Key</span>
          <input
            type="password"
            value={key}
            onChange={(e) => {
              setKey(e.target.value);
              setKeySaved(false);
            }}
            placeholder="sk-..."
            autoFocus={!initialKey}
          />
        </label>
        <p className="help">
          Salva em <code>%APPDATA%/Artemis/config.json</code> (texto plano, apenas o
          seu usuário tem acesso). Nunca é enviada para nenhum servidor que não seja
          a API da DeepSeek.
        </p>
        <div className="row">
          <button type="button" onClick={handleSaveKey} disabled={keySaving}>
            {keySaving ? "Salvando..." : keySaved ? "Salvo ✓" : "Salvar chave"}
          </button>
        </div>
        {keyError && <p className="key-error">{keyError}</p>}
      </section>

      <hr className="divider" />

      <section className="settings-section">
        <label>
          <span className="label">Pasta do vault (Obsidian)</span>
          <div className="path-row">
            <input
              type="text"
              value={vaultPath ?? ""}
              readOnly
              placeholder="Nenhuma pasta selecionada"
            />
            <button type="button" className="secondary" onClick={handlePickVault} disabled={vaultBusy}>
              Escolher...
            </button>
          </div>
        </label>
        <p className="help">
          Os arquivos <code>estilo.md</code>, <code>evitar.md</code>, <code>campos-padrao.md</code>{" "}
          e <code>exemplos-aprovados.md</code> desta pasta são injetados no prompt. Edite-os no
          Obsidian e a IA usa as mudanças na próxima devolutiva.
        </p>
        {vaultPath && (
          <div className="row">
            <button type="button" className="secondary" onClick={handleSeed} disabled={vaultBusy}>
              Inicializar com templates
            </button>
          </div>
        )}
        {vaultMessage && <p className="vault-message">{vaultMessage}</p>}
        {vaultStatus && vaultStatus.path && (
          <div className="vault-summary">
            <strong>{vaultStatus.files_present.length}/4</strong> arquivos ·{" "}
            {vaultStatus.total_chars} chars ·{" "}
            {vaultStatus.files_present.length > 0
              ? vaultStatus.files_present.join(", ")
              : "vazio"}
          </div>
        )}
      </section>

      <hr className="divider" />

      <section className="settings-section">
        <label className="toggle-row">
          <span>
            <span className="label">Iniciar com o Windows</span>
            <p className="help" style={{ marginTop: 2 }}>
              Quando ativado, o Artemis sobe automaticamente no login. O FAB aparece
              no canto inferior direito e o chat fica oculto até você clicar.
            </p>
          </span>
          <input
            type="checkbox"
            checked={autostart ?? false}
            onChange={handleToggleAutostart}
            disabled={autostart === null || autostartBusy}
          />
        </label>
        {autostartError && <p className="key-error">{autostartError}</p>}

        <p className="help" style={{ marginTop: 8 }}>
          <strong>Hotkey global:</strong> <code>Ctrl+Shift+D</code> abre o chat de
          qualquer lugar do Windows. Atalho fixo nesta versão.
        </p>
      </section>

      <hr className="divider" />

      <section className="settings-section">
        <label>
          <span className="label">Atualizações</span>
        </label>
        <p className="help">
          O Artemis checa novas versões em{" "}
          <code>github.com/ThiagoLuigiM/ArtemisChat/releases</code>. A instalação é
          assinada e verificada — só releases publicadas pelo mantenedor são aceitas.
        </p>
        <div className="row">
          <button
            type="button"
            className="secondary"
            onClick={handleCheckUpdate}
            disabled={updateChecking || updateInstalling}
          >
            {updateChecking ? "Verificando..." : "Verificar atualizações"}
          </button>
        </div>
        {updateInfo && !updateInfo.available && (
          <p className="vault-message">
            ✓ Versão {updateInfo.current_version} é a mais recente.
          </p>
        )}
        {updateInfo && updateInfo.available && (
          <div className="vault-message">
            <div>
              ⬆ Nova versão disponível: <strong>{updateInfo.new_version}</strong>{" "}
              (atual: {updateInfo.current_version})
            </div>
            {updateInfo.release_notes && (
              <details style={{ marginTop: 6 }}>
                <summary style={{ cursor: "pointer", fontSize: 11 }}>
                  Notas da release
                </summary>
                <pre style={{
                  marginTop: 6,
                  fontSize: 11,
                  whiteSpace: "pre-wrap",
                  fontFamily: "inherit",
                }}>{updateInfo.release_notes}</pre>
              </details>
            )}
            <div className="row" style={{ marginTop: 8 }}>
              <button
                type="button"
                onClick={handleInstallUpdate}
                disabled={updateInstalling}
              >
                {updateInstalling ? "Baixando e instalando..." : "Instalar e reiniciar"}
              </button>
            </div>
          </div>
        )}
        {updateError && <p className="key-error">{updateError}</p>}
      </section>
    </div>
  );
}

// ── LearningPanel ─────────────────────────────────────────────────────────────
// Painel de aprendizado com três abas:
//  • "evitar" (#18) — analisa devolutivas EDITADAS e sugere adições ao evitar.md.
//  • "estilo" (#19) — sintetiza nova versão do estilo.md a partir das aprovadas
//    SEM edição (sinais positivos puros), com preview editável antes de aplicar.
//  • "campos" (#20) — agrega releases/caminhos das aprovadas (parsing puro, sem IA)
//    e propõe atualizações ao campos-padrao.md via checkbox.
// Princípio comum: o app PROPÕE, o usuário ACEITA — autonomia preservada.

type LearningTab = "evitar" | "estilo" | "campos";

function LearningPanel({
  vaultPath,
  onClose,
}: {
  vaultPath: string | null;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<LearningTab>("evitar");

  return (
    <div className="settings-panel">
      <header className="settings-header">
        <h2>Aprendizado</h2>
        <button className="secondary close" onClick={onClose} aria-label="Voltar">
          ✕
        </button>
      </header>

      <div className="tabs">
        <button
          type="button"
          className={`tab ${tab === "evitar" ? "active" : ""}`}
          onClick={() => setTab("evitar")}
        >
          evitar.md
        </button>
        <button
          type="button"
          className={`tab ${tab === "estilo" ? "active" : ""}`}
          onClick={() => setTab("estilo")}
        >
          estilo.md
        </button>
        <button
          type="button"
          className={`tab ${tab === "campos" ? "active" : ""}`}
          onClick={() => setTab("campos")}
        >
          campos-padrão.md
        </button>
      </div>

      {tab === "evitar" && <EvitarTab vaultPath={vaultPath} />}
      {tab === "estilo" && <EstiloTab vaultPath={vaultPath} />}
      {tab === "campos" && <CamposTab vaultPath={vaultPath} />}
    </div>
  );
}

function EvitarTab({ vaultPath }: { vaultPath: string | null }) {
  const [editCount, setEditCount] = useState<number | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [suggestions, setSuggestions] = useState<EvitarSuggestion[] | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const [appliedFile, setAppliedFile] = useState<string | null>(null);

  useEffect(() => {
    invoke<number>("count_edited_approved")
      .then(setEditCount)
      .catch(() => setEditCount(0));
  }, []);

  const handleAnalyze = async () => {
    setError(null);
    setSuggestions(null);
    setAppliedFile(null);
    setAnalyzing(true);
    try {
      const out = await invoke<EvitarSuggestion[]>("analyze_edits");
      setSuggestions(out);
      setSelected(new Set(out.map((_, i) => i))); // tudo selecionado por padrão
    } catch (e) {
      setError(String(e));
    } finally {
      setAnalyzing(false);
    }
  };

  const toggleSelected = (idx: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  const handleApply = async () => {
    if (!suggestions || selected.size === 0) return;
    setApplying(true);
    setError(null);
    try {
      const chosen = suggestions.filter((_, i) => selected.has(i));
      const file = await invoke<string>("apply_evitar_suggestions", { suggestions: chosen });
      setAppliedFile(file);
      setSuggestions(null);
      setSelected(new Set());
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  const canAnalyze = editCount !== null && editCount >= 2;

  return (
    <>
      <p className="help">
        Quando você edita uma devolutiva antes de aprovar, o Artemis registra a versão
        original e a final. Aqui ele analisa esses pares para sugerir expressões para
        adicionar ao seu <code>evitar.md</code>. Você revisa e escolhe quais aceitar.
      </p>

      {!vaultPath && (
        <p className="key-error">
          ⚠ Vault não configurado. Configure a pasta nas Configurações antes de aplicar sugestões.
        </p>
      )}

      <div className="edit-counter">
        <strong>{editCount ?? "…"}</strong> devolutiva(s) editada(s) e aprovada(s)
        disponível(eis) para análise
        {editCount !== null && editCount < 2 && (
          <div className="help" style={{ marginTop: 6 }}>
            Edite e aprove ao menos 2 devolutivas para gerar sugestões úteis.
          </div>
        )}
      </div>

      {!suggestions && !appliedFile && (
        <div className="row">
          <button
            type="button"
            onClick={handleAnalyze}
            disabled={analyzing || !canAnalyze}
          >
            {analyzing ? "Analisando..." : "Analisar minhas edições"}
          </button>
        </div>
      )}

      {error && <p className="key-error">{error}</p>}

      {appliedFile && (
        <p className="vault-message">
          ✓ Sugestões adicionadas em <code>{appliedFile}</code>. Confira no Obsidian — os
          itens auto-aprendidos vêm com um marker <code>&lt;!-- auto-aprendidos em DATA --&gt;</code>{" "}
          para você reconhecer.
        </p>
      )}

      {suggestions && suggestions.length === 0 && (
        <p className="help">
          Nenhum padrão claro emergiu das edições atuais. Edite e aprove mais devolutivas
          (especialmente removendo expressões consistentemente) e tente novamente.
        </p>
      )}

      {suggestions && suggestions.length > 0 && (
        <>
          <div className="selection-toolbar">
            <button
              type="button"
              className="secondary"
              onClick={() => setSelected(new Set(suggestions.map((_, i) => i)))}
            >
              Selecionar todas
            </button>
            <button
              type="button"
              className="secondary"
              onClick={() => setSelected(new Set())}
            >
              Nenhuma
            </button>
            <span className="counter">
              {selected.size}/{suggestions.length} selecionada(s)
            </span>
          </div>

          <div className="suggestions-list">
            {suggestions.map((s, i) => (
              <label key={i} className="suggestion-item">
                <input
                  type="checkbox"
                  checked={selected.has(i)}
                  onChange={() => toggleSelected(i)}
                />
                <div className="suggestion-body">
                  <div className="suggestion-expr">"{s.expression}"</div>
                  <div className="suggestion-reason">{s.reason}</div>
                  <div className="suggestion-meta">
                    {s.occurrences} ocorrência{s.occurrences === 1 ? "" : "s"}
                  </div>
                </div>
              </label>
            ))}
          </div>

          <div className="row">
            <button
              type="button"
              onClick={handleApply}
              disabled={applying || selected.size === 0 || !vaultPath}
            >
              {applying ? "Aplicando..." : `Adicionar ${selected.size} ao evitar.md`}
            </button>
          </div>
        </>
      )}
    </>
  );
}

const STYLE_MIN_SAMPLES = 5;

function EstiloTab({ vaultPath }: { vaultPath: string | null }) {
  const [unedited, setUnedited] = useState<number | null>(null);
  const [synthesizing, setSynthesizing] = useState(false);
  const [proposal, setProposal] = useState<string | null>(null);
  const [editedProposal, setEditedProposal] = useState("");
  const [applying, setApplying] = useState(false);
  const [appliedFile, setAppliedFile] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshCount = () => {
    invoke<number>("count_approved_unedited")
      .then(setUnedited)
      .catch(() => setUnedited(0));
  };

  useEffect(() => {
    refreshCount();
  }, []);

  const handleSynthesize = async () => {
    setError(null);
    setProposal(null);
    setAppliedFile(null);
    setSynthesizing(true);
    try {
      const out = await invoke<string>("synthesize_style");
      setProposal(out);
      setEditedProposal(out);
    } catch (e) {
      setError(String(e));
    } finally {
      setSynthesizing(false);
    }
  };

  const handleCancel = () => {
    setProposal(null);
    setEditedProposal("");
    setError(null);
  };

  const handleApply = async () => {
    if (!editedProposal.trim()) return;
    setApplying(true);
    setError(null);
    try {
      const file = await invoke<string>("apply_style_synthesis", { newContent: editedProposal });
      setAppliedFile(file);
      setProposal(null);
      setEditedProposal("");
      refreshCount();
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  const canSynthesize = !!vaultPath && unedited !== null && unedited >= STYLE_MIN_SAMPLES;

  return (
    <>
      <p className="help">
        O Artemis lê as últimas 50 devolutivas que você aprovou <strong>sem editar</strong>{" "}
        (sinais positivos puros — a IA acertou de cara) e propõe uma versão refinada do
        seu <code>estilo.md</code>. Você revisa, edita se quiser, e aplica.
      </p>

      {!vaultPath && (
        <p className="key-error">
          ⚠ Vault não configurado. Configure a pasta nas Configurações antes de sintetizar.
        </p>
      )}

      {!proposal && (
        <>
          <div className="edit-counter">
            <strong>{unedited ?? "…"}</strong> devolutiva(s) aprovada(s) sem edição
            disponível(eis) para análise
            {unedited !== null && unedited < STYLE_MIN_SAMPLES && (
              <div className="help" style={{ marginTop: 6 }}>
                Aprove ao menos {STYLE_MIN_SAMPLES} devolutivas <strong>sem editar</strong>{" "}
                para gerar uma síntese útil. Atual: {unedited}.
              </div>
            )}
          </div>

          {!appliedFile && (
            <div className="row">
              <button
                type="button"
                onClick={handleSynthesize}
                disabled={synthesizing || !canSynthesize}
                title="Pede à IA uma nova versão do estilo.md baseada nos seus acertos"
              >
                {synthesizing ? "Analisando até 50 devolutivas..." : "Sintetizar estilo.md"}
              </button>
            </div>
          )}

          {appliedFile && (
            <p className="vault-message">
              ✓ <code>{appliedFile}</code> substituído. Backup do anterior em{" "}
              <code>estilo.md.bak</code>. A IA usará o novo estilo na próxima devolutiva.
            </p>
          )}
        </>
      )}

      {error && <p className="key-error">{error}</p>}

      {proposal && (
        <>
          <p className="help">
            <strong>Proposta da IA</strong> — revise (e edite se quiser) antes de aplicar.
            O <code>estilo.md</code> atual será salvo em <code>estilo.md.bak</code>{" "}
            automaticamente (apenas o último backup é mantido).
          </p>
          <textarea
            className="style-proposal-textarea"
            value={editedProposal}
            onChange={(e) => setEditedProposal(e.target.value)}
            disabled={applying}
            spellCheck={false}
          />
          <div className="row">
            <button
              type="button"
              className="secondary"
              onClick={handleCancel}
              disabled={applying}
            >
              Cancelar
            </button>
            <button
              type="button"
              onClick={handleApply}
              disabled={applying || !editedProposal.trim() || !vaultPath}
            >
              {applying ? "Aplicando..." : "Substituir estilo.md"}
            </button>
          </div>
        </>
      )}
    </>
  );
}

const CAMPOS_MIN_SAMPLES = 5;

function CamposTab({ vaultPath }: { vaultPath: string | null }) {
  const [analyzing, setAnalyzing] = useState(false);
  const [data, setData] = useState<CamposSuggestions | null>(null);
  const [releaseChecked, setReleaseChecked] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<Set<number>>(new Set());
  const [applying, setApplying] = useState(false);
  const [appliedFile, setAppliedFile] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleAnalyze = async () => {
    setError(null);
    setData(null);
    setAppliedFile(null);
    setAnalyzing(true);
    try {
      const out = await invoke<CamposSuggestions>("analyze_campos");
      setData(out);
      setReleaseChecked(!!out.release);
      setSelectedPaths(new Set(out.paths.map((_, i) => i))); // todas marcadas por default
    } catch (e) {
      setError(String(e));
    } finally {
      setAnalyzing(false);
    }
  };

  const togglePath = (idx: number) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  const handleApply = async () => {
    if (!data) return;
    const release = releaseChecked && data.release ? data.release.proposed : null;
    const paths = data.paths.filter((_, i) => selectedPaths.has(i)).map((p) => p.path);
    if (!release && paths.length === 0) return;
    setApplying(true);
    setError(null);
    try {
      const file = await invoke<string>("apply_campos_suggestions", {
        releaseAccepted: release,
        pathsAccepted: paths,
      });
      setAppliedFile(file);
      setData(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  const totalSelected = (data?.release && releaseChecked ? 1 : 0) + selectedPaths.size;

  return (
    <>
      <p className="help">
        O Artemis lê suas aprovadas e extrai (sem chamar IA) a release mais recente
        usada e os caminhos `A &gt; B &gt; C` mais frequentes que ainda não estão no
        seu <code>campos-padrao.md</code>. Você marca o que aceitar e o app aplica.
      </p>

      {!vaultPath && (
        <p className="key-error">
          ⚠ Vault não configurado. Configure a pasta nas Configurações antes de analisar.
        </p>
      )}

      {!data && (
        <div className="row">
          <button
            type="button"
            onClick={handleAnalyze}
            disabled={analyzing || !vaultPath}
            title={`Lê até 100 aprovadas e procura releases + caminhos. Mínimo ${CAMPOS_MIN_SAMPLES} aprovadas.`}
          >
            {analyzing ? "Analisando..." : "Analisar histórico"}
          </button>
        </div>
      )}

      {error && <p className="key-error">{error}</p>}

      {appliedFile && (
        <p className="vault-message">
          ✓ <code>{appliedFile}</code> atualizado. Backup do anterior em{" "}
          <code>campos-padrao.md.bak</code>. A IA usará as novas referências na próxima
          devolutiva.
        </p>
      )}

      {data && (
        <>
          <div className="edit-counter">
            <strong>{data.analyzed_count}</strong> aprovada(s) analisada(s)
          </div>

          {data.release && (
            <div className="campos-section">
              <h3 className="campos-section-title">Release atual</h3>
              <label className="suggestion-item">
                <input
                  type="checkbox"
                  checked={releaseChecked}
                  onChange={(e) => setReleaseChecked(e.target.checked)}
                />
                <div className="suggestion-body">
                  <div className="suggestion-expr">{data.release.proposed}</div>
                  <div className="suggestion-reason">
                    {data.release.current_in_file
                      ? `Substituirá: ${data.release.current_in_file}`
                      : "Será adicionada (nenhuma release detectada no arquivo)"}
                  </div>
                </div>
              </label>
            </div>
          )}

          {data.paths.length > 0 && (
            <div className="campos-section">
              <h3 className="campos-section-title">
                Caminhos novos ({data.paths.length})
              </h3>
              <div className="selection-toolbar">
                <button
                  type="button"
                  className="secondary"
                  onClick={() => setSelectedPaths(new Set(data.paths.map((_, i) => i)))}
                >
                  Selecionar todos
                </button>
                <button
                  type="button"
                  className="secondary"
                  onClick={() => setSelectedPaths(new Set())}
                >
                  Nenhum
                </button>
                <span className="counter">
                  {selectedPaths.size}/{data.paths.length} selecionado(s)
                </span>
              </div>
              <div className="suggestions-list">
                {data.paths.map((p, i) => (
                  <label key={i} className="suggestion-item">
                    <input
                      type="checkbox"
                      checked={selectedPaths.has(i)}
                      onChange={() => togglePath(i)}
                    />
                    <div className="suggestion-body">
                      <div className="suggestion-expr">{p.path}</div>
                      <div className="suggestion-meta">
                        {p.occurrences} ocorrência{p.occurrences === 1 ? "" : "s"}
                      </div>
                    </div>
                  </label>
                ))}
              </div>
            </div>
          )}

          {!data.release && data.paths.length === 0 && (
            <p className="help">
              Nada novo a propor — o <code>campos-padrao.md</code> já cobre o que aparece
              nas suas aprovadas recentes.
            </p>
          )}

          {(data.release || data.paths.length > 0) && (
            <div className="row">
              <button
                type="button"
                onClick={handleApply}
                disabled={applying || totalSelected === 0 || !vaultPath}
              >
                {applying ? "Aplicando..." : `Aplicar ${totalSelected} mudança${totalSelected === 1 ? "" : "s"}`}
              </button>
            </div>
          )}
        </>
      )}
    </>
  );
}

// ── ChatWindow (main) ─────────────────────────────────────────────────────────

export default function ChatWindow() {
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [keyLoaded, setKeyLoaded] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [showLearning, setShowLearning] = useState(false);
  const [vaultStatus, setVaultStatus] = useState<VaultStatus | null>(null);

  const [view, setView] = useState<"form" | "result">("form");
  const [form, setForm] = useState<FormFields>(EMPTY_FORM);
  const [result, setResult] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [currentCategory, setCurrentCategory] = useState<CategoryEvent | null>(null);

  const streamingRef = useRef("");
  const sendingRef = useRef(false);

  useEffect(() => {
    invoke<string | null>("get_api_key").then((key) => {
      setApiKey(key);
      setKeyLoaded(true);
      if (!key) setShowSettings(true);
    });
    invoke<VaultStatus>("get_vault_status").then(setVaultStatus);
  }, []);

  useEffect(() => {
    let active = true;
    const unlisteners: UnlistenFn[] = [];

    Promise.all([
      listen<TokenEvent>("deepseek-token", (e) => {
        streamingRef.current += e.payload.content;
        setResult(streamingRef.current);
      }),
      listen("deepseek-done", () => {
        setStreaming(false);
      }),
      listen<VaultStatus>("vault-changed", (e) => {
        setVaultStatus(e.payload);
      }),
      listen<CategoryEvent>("category-detected", (e) => {
        setCurrentCategory(e.payload);
      }),
      listen("tray-open-settings", () => {
        setShowLearning(false);
        setShowSettings(true);
      }),
    ]).then((uls) => {
      if (!active) { uls.forEach((ul) => ul()); return; }
      unlisteners.push(...uls);
    });

    return () => {
      active = false;
      unlisteners.forEach((ul) => ul());
    };
  }, []);

  const handleGenerate = useCallback(async () => {
    if (streaming || sendingRef.current) return;
    const text = compileForm(form);
    if (!text.trim()) return;

    sendingRef.current = true;
    streamingRef.current = "";
    setResult("");
    setCurrentCategory(null);
    setStreaming(true);
    setView("result");

    try {
      await invoke("stream_completion", { userInput: text });
    } catch (e) {
      setResult(`Erro: ${String(e)}`);
      setStreaming(false);
    } finally {
      sendingRef.current = false;
    }
  }, [form, streaming]);

  const resetToForm = useCallback((clearForm: boolean) => {
    setView("form");
    setResult("");
    setCurrentCategory(null);
    streamingRef.current = "";
    if (clearForm) setForm(EMPTY_FORM);
  }, []);

  const handleNewDevolutiva = useCallback(() => {
    // Abandono silencioso: NÃO registra nada no histórico
    resetToForm(false);
  }, [resetToForm]);

  const handleApprove = useCallback(
    async (finalText: string) => {
      const rawInput = compileForm(form);
      const out = await invoke<{ id: number; category: string; examples_file: string | null }>(
        "approve_entry",
        { rawInput, aiRawOutput: result, finalOutput: finalText },
      );
      console.info(`[approve] #${out.id} categoria=${out.category} arquivo=${out.examples_file}`);
      resetToForm(true);
    },
    [form, result, resetToForm],
  );

  const handleDiscard = useCallback(
    async (finalText: string) => {
      const rawInput = compileForm(form);
      await invoke<number>("discard_entry", {
        rawInput,
        aiRawOutput: result,
        finalOutput: finalText,
      });
      resetToForm(true);
    },
    [form, result, resetToForm],
  );

  const saveApiKey = async (key: string) => {
    await invoke("set_api_key", { key });
    setApiKey(key);
  };

  if (!keyLoaded) {
    return <div className="loading">Carregando...</div>;
  }

  if (showSettings) {
    return (
      <SettingsPanel
        initialKey={apiKey ?? ""}
        vaultStatus={vaultStatus}
        onSaveKey={saveApiKey}
        onClose={apiKey ? () => setShowSettings(false) : undefined}
        onVaultChanged={setVaultStatus}
      />
    );
  }

  if (showLearning) {
    return (
      <LearningPanel
        vaultPath={vaultStatus?.path ?? null}
        onClose={() => setShowLearning(false)}
      />
    );
  }

  return (
    <div className="chat-window">
      <header className="chat-header">
        <div className="header-left">
          <span className="title">Artemis</span>
          <VaultBadge status={vaultStatus} />
        </div>
        <div className="actions">
          <button
            onClick={() => {
              setShowSettings(false);
              setShowLearning(true);
            }}
            title="Aprendizado: analisar edições e sugerir adições ao evitar.md"
            aria-label="Aprendizado"
          >
            🧠
          </button>
          <button
            onClick={() => {
              setShowLearning(false);
              setShowSettings(true);
            }}
            title="Configurações"
            aria-label="Configurações"
          >
            ⚙
          </button>
          <button
            onClick={() => invoke("close_chat").catch(console.error)}
            title="Fechar"
            aria-label="Fechar"
          >
            −
          </button>
        </div>
      </header>

      {view === "form" ? (
        <FormView
          fields={form}
          onChange={setForm}
          onGenerate={handleGenerate}
          disabled={streaming}
        />
      ) : (
        <ResultView
          aiRawOutput={result}
          streaming={streaming}
          category={currentCategory}
          onNew={handleNewDevolutiva}
          onApprove={handleApprove}
          onDiscard={handleDiscard}
        />
      )}
    </div>
  );
}
